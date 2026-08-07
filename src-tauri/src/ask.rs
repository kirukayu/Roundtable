//! Answering a question out of the wiki.
//!
//! The model is not asked to know anything about ELDEN RING. It is handed the
//! passages that actually match the question and told to answer from those, so
//! what comes back is the wiki in a sentence rather than a guess that sounds
//! like one. A wrong answer about a boss costs somebody a run, and a plausible
//! wrong answer is worse than an admission.
//!
//! Retrieval happens here, on the player's machine, for three reasons: every
//! article title is already mirrored so it costs nothing, it keeps what leaves
//! the machine down to a question and four paragraphs, and small requests are
//! the only reason a handful of free tiers covers a whole userbase.
//!
//! Only the titles are held locally in full. The two or three articles a
//! question actually needs are fetched on demand and cached, so the second
//! question about a boss is instant.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where the answers come from. No key: the service holds those.
const SERVICE: &str = "https://roundtable-ask.roundtable-launcher.workers.dev/ask";
/// Where a question is turned into the words the wiki uses.
const PLANNER: &str = "https://roundtable-ask.roundtable-launcher.workers.dev/terms";

/// Asks a model what to search for.
///
/// The glossary below still exists and still runs, but it is the fallback now.
/// It only ever knew the Russian words somebody had thought to add, and a table
/// like that is wrong in a new way every week — it knew "убить" and not "как
/// задавить", and nothing at all in Polish. A model reads the question and
/// names the articles, in any language, for the price of one call to the
/// fastest lane.
///
/// Returns nothing when the service is unreachable or slow, and the local
/// glossary carries the question on its own. Being offline should cost accuracy,
/// not the feature.
async fn planned_terms(http: &reqwest::Client, question: &str) -> Vec<String> {
    let Ok(reply) = http
        .post(PLANNER)
        .json(&serde_json::json!({ "question": question }))
        .timeout(std::time::Duration::from_secs(9))
        .send()
        .await
    else {
        return Vec::new();
    };

    #[derive(Deserialize)]
    struct Terms {
        #[serde(default)]
        terms: Vec<String>,
    }

    reply
        .json::<Terms>()
        .await
        .map(|body| body.terms)
        .unwrap_or_default()
}

/// Words that match everything and therefore mean nothing, in both languages
/// the launcher is used in.
const NOISE: &[&str] = &[
    // English
    "the", "a", "an", "is", "are", "was", "were", "be", "to", "of", "in", "on", "at", "for",
    "with", "and", "or", "but", "how", "what", "where", "when", "why", "who", "which", "do",
    "does", "did", "can", "could", "should", "would", "i", "you", "it", "this", "that", "my",
    "me", "get", "got", "best", "good", "vs", "from", "about", "there", "any", "all",
    // Russian
    "как", "что", "где", "когда", "почему", "кто", "какой", "какая", "какие", "чем", "чего",
    "это", "этот", "эта", "эти", "мне", "меня", "мой", "моя", "тут", "там", "он", "она", "они",
    "и", "в", "на", "с", "по", "за", "из", "до", "от", "у", "о", "об", "для", "не", "ли", "же",
    "бы", "ну", "вот", "если", "или", "а", "но", "то", "так", "уже", "ещё", "еще", "надо",
    "нужно", "можно", "лучше", "самый", "всё", "все",
    // The names of the game and the mod. Somebody asking "как качаться в
    // Convergence" is telling us which wiki to read, not what to read in it —
    // and left in, "convergence" prefix-matched "Convert Corruption" and
    // "Converted Tower" and pushed the real answer out of every slot.
    "elden", "ring", "convergence", "элден", "ринг", "конвергенс", "конверг",
    "игре", "игры", "игру", "игра", "game",
];

/// Splits a question into the words worth searching for.
fn terms(question: &str) -> Vec<String> {
    let noise: HashSet<&str> = NOISE.iter().copied().collect();
    question
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|word| word.chars().count() > 2 && !noise.contains(word))
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .take(12)
        .collect()
}

/// A prefix long enough to survive Russian inflection.
///
/// "Малению", "Малении" and "Маления" are the same boss, and a launcher that
/// only finds the nominative is a launcher nobody uses twice. Comparing the
/// first several characters catches every case ending without needing a
/// stemmer for two languages.
fn stem_to(word: &str, len: usize) -> String {
    word.chars().take(len).collect()
}

#[cfg(test)]
fn stem(word: &str) -> String {
    stem_to(word, 5)
}

/// Cyrillic spelled the way the wiki spells it.
///
/// Both wikis are written in English, and somebody playing in Russian types
/// "Малению". No amount of prefix matching bridges two alphabets, so the word
/// is respelled in Latin first: "Малению" becomes "maleniyu", which shares a
/// stem with "Malenia" and finds the article. Without this, every question
/// asked in Russian about a boss came back with no sources at all.
fn translit(word: &str) -> Option<String> {
    if !word.chars().any(|c| ('а'..='я').contains(&c) || c == 'ё') {
        return None;
    }
    let mut out = String::with_capacity(word.len());
    for ch in word.chars() {
        out.push_str(match ch {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d",
            'е' => "e", 'ё' => "e", 'ж' => "zh", 'з' => "z", 'и' => "i",
            'й' => "y", 'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n",
            'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t",
            'у' => "u", 'ф' => "f", 'х' => "h", 'ц' => "ts", 'ч' => "ch",
            'ш' => "sh", 'щ' => "sch", 'ъ' => "", 'ы' => "y", 'ь' => "",
            'э' => "e", 'ю' => "yu", 'я' => "ya",
            other => {
                out.push(other);
                continue;
            }
        });
    }
    Some(out)
}

/// The handful of Russian words that carry the intent of a question.
///
/// Transliteration gets a proper noun across — "Малению" finds "Malenia" —
/// but it does nothing for the verb. "Убить" becomes "ubit", which appears
/// nowhere in an English article, so a question about *how to kill* a boss
/// scored every paragraph of her page equally and settled on the lore at the
/// top. The answer that came back was "the passages do not say".
///
/// Matched by prefix, so one entry covers a word's whole declension. A word may
/// appear more than once: "убить" is answered by the paragraph headed
/// "Strategy" as often as by one containing "kill", and both are worth looking
/// for.
const GLOSS: &[(&str, &str)] = &[
    ("убит", "kill"), ("убит", "strategy"), ("убит", "defeat"),
    ("убива", "kill"), ("убива", "strategy"),
    ("побед", "defeat"), ("побед", "strategy"),
    ("босс", "boss"), ("держ", "fight"), ("бить", "fight"), ("бой", "fight"),
    ("сраж", "fight"), ("такти", "strategy"), ("страт", "strategy"),
    ("урон", "damage"), ("дамаг", "damage"), ("слаб", "weakness"),
    ("уязв", "weakness"), ("сопрот", "resistance"), ("имму", "immunity"),
    ("найти", "location"), ("найд", "location"), ("где", "location"),
    ("локац", "location"), ("мест", "location"), ("получ", "acquisition"),
    ("взять", "acquisition"), ("дроп", "drops"), ("выпад", "drops"),
    ("оруж", "weapon"), ("меч", "sword"), ("щит", "shield"), ("брон", "armor"),
    ("доспе", "armor"), ("шлем", "helm"), ("талисм", "talisman"),
    ("закли", "incantation"), ("магия", "sorcery"), ("маги", "sorcery"),
    ("пепел", "ashes"), ("призв", "summon"), ("фаза", "phase"), ("фазы", "phase"),
    ("хил", "heal"), ("леч", "heal"), ("парир", "parry"), ("уклон", "dodge"),
    ("блок", "block"), ("билд", "build"), ("квест", "quest"), ("конц", "ending"),
    // Levelling. Neither wiki has a page called "Leveling" — the answer lives
    // in "Attributes", "Stats and Attributes" and "Runes" — and glossing this
    // to "level" was worse than nothing, because it matched "Gaol Lower Level
    // Key". The gloss has to name the article, not translate the word.
    ("прокач", "attributes"), ("прокач", "runes"),
    ("качат", "attributes"), ("качат", "runes"), ("качат", "stats"),
    ("качаю", "attributes"), ("качая", "attributes"),
    ("уровен", "attributes"), ("уровн", "attributes"), ("лвл", "attributes"),
    ("руна", "runes"), ("руны", "runes"), ("рун", "runes"),
    ("фарм", "farming"), ("гринд", "farming"),
    ("класс", "class"), ("стат", "stats"), ("характер", "stats"),
    ("сила", "strength"), ("ловк", "dexterity"), ("интел", "intelligence"),
    ("вера", "faith"), ("выносл", "endurance"), ("здоров", "vigor"),
    ("старт", "starting"), ("начал", "starting"), ("нович", "beginner"),
];

/// Every spelling of a question word worth searching for.
fn variants(word: &str, len: usize) -> Vec<String> {
    let mut out = vec![stem_to(word, len)];

    if let Some(latin) = translit(word) {
        let stemmed = stem_to(&latin, len);
        if !out.contains(&stemmed) {
            out.push(stemmed);
        }
    }

    // The English word for what was asked, where there is one. A proper noun
    // needs transliteration; a verb needs translating.
    for (russian, english) in GLOSS {
        if word.starts_with(russian) {
            let stemmed = stem_to(english, len);
            if !out.contains(&stemmed) {
                out.push(stemmed);
            }
        }
    }

    out
}

/// Titles worth reading, best first.
///
/// A title scores for every question word it contains. Short titles win ties,
/// because "Malenia" is far more likely to be what was meant than
/// "Malenia, Blade of Miquella/dialogue".
pub fn rank_titles<'a>(titles: &'a [String], question: &str, limit: usize) -> Vec<&'a String> {
    let words = terms(question);
    if words.is_empty() {
        return Vec::new();
    }

    // Five characters is the right prefix for most names. Where it finds
    // nothing, four is tried: transliteration is not exact, and "Радан" only
    // reaches "Radahn" once the comparison is loose enough to forgive the h.
    for len in [5usize, 4] {
        let wanted: Vec<Vec<String>> = words.iter().map(|w| variants(w, len)).collect();
        let hits = score_titles(titles, &wanted, limit);
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}

fn score_titles<'a>(
    titles: &'a [String],
    wanted: &[Vec<String>],
    limit: usize,
) -> Vec<&'a String> {
    let mut hits: Vec<(u32, usize, &String)> = titles
        .iter()
        .filter_map(|title| {
            let lower = title.to_lowercase();
            let parts: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
            let mut score = 0u32;

            // One score per question word, however many spellings it has, so a
            // word that happens to transliterate two ways is not worth double.
            for spellings in wanted {
                let at = parts
                    .iter()
                    .position(|part| spellings.iter().any(|want| part.starts_with(want.as_str())));

                let Some(at) = at else {
                    // Not a whole word anywhere, but present inside one.
                    if spellings.iter().any(|want| lower.contains(want.as_str())) {
                        score += 3;
                    }
                    continue;
                };

                // A whole-word match beats a fragment: "rot" should not drag in
                // "Rotten" ahead of "Scarlet Rot".
                score += 10;

                // Is the name what the page is *about*, or something the page
                // merely belongs to? "Malenia, Blade of Miquella" is the boss;
                // "Malenia's Armor" and "Hand of Malenia" are her belongings,
                // and a question about fighting her wants the first. Asking
                // whether the match heads the title, is possessive, or sits
                // after an "of" separates them — sorting on title length, which
                // is what this did before, put her gauntlets first.
                if at == 0 {
                    score += 6;
                }
                if parts.get(at + 1).is_some_and(|next| *next == "s") {
                    score = score.saturating_sub(6);
                }
                if at > 0 && parts.get(at - 1).is_some_and(|prev| *prev == "of") {
                    score = score.saturating_sub(4);
                }
            }

            if score == 0 {
                return None;
            }
            // A subpage is a fragment of an article, not an article: a page of
            // raw dialogue lines answers almost nothing on its own, and it
            // would otherwise crowd out the page it belongs to.
            if lower.contains('/') {
                score = score.saturating_sub(8);
            }
            (score > 0).then_some((score, title.chars().count(), title))
        })
        .collect();

    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    hits.into_iter().take(limit).map(|(_, _, t)| t).collect()
}

/// Strips the article back to readable prose.
fn to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut last_space = true;

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                // A tag boundary is a word boundary, or headings weld onto the
                // paragraph beneath them.
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            '>' => in_tag = false,
            _ if in_tag => {}
            c if c.is_whitespace() => {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            c => {
                out.push(c);
                last_space = false;
            }
        }
    }

    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// The most relevant window of an article, rather than its opening.
///
/// An article can run to twenty thousand words and the model is given two
/// thousand characters of it, so which two thousand decides whether the answer
/// is right. The text is walked in overlapping windows and the one densest in
/// question words wins — for "waterfowl dance" that is the paragraph about the
/// attack, not the lore at the top.
fn best_window(text: &str, question: &str, size: usize) -> String {
    // The same spellings the titles were matched on, so a Russian question
    // lands on the paragraph about the fight rather than the lore at the top.
    let wanted: Vec<String> = terms(question)
        .iter()
        .flat_map(|word| variants(word, 5))
        .collect();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= size || wanted.is_empty() {
        return chars.into_iter().take(size).collect();
    }

    let step = size / 3;
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let mut best = 0usize;
    let mut best_score = -1i32;

    let mut at = 0usize;
    while at + size <= chars.len() {
        let window: String = lower[at..at + size].iter().collect();
        let mut score = 0i32;
        for want in &wanted {
            score += window.matches(want.as_str()).count() as i32;
        }
        // All else equal, earlier is better: the top of an article is usually
        // its summary.
        let adjusted = score * 10 - (at / size) as i32;
        if adjusted > best_score {
            best_score = adjusted;
            best = at;
        }
        at += step;
    }

    chars[best..(best + size).min(chars.len())].iter().collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passage {
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub answer: String,
    /// Which articles it was drawn from, so it can be checked.
    pub sources: Vec<String>,
    /// Which model answered, and how long it took.
    pub lane: Option<String>,
    pub ms: Option<u64>,
}

#[derive(Deserialize)]
struct ServiceReply {
    answer: Option<String>,
    lane: Option<String>,
    ms: Option<u64>,
    error: Option<String>,
}

/// Gathers the passages a question needs.
///
/// Public so the interface can show what it found while the model is still
/// thinking — waiting is easier when something is happening.
pub async fn gather(
    http: &reqwest::Client,
    app_data: &Path,
    edition: Option<&str>,
    question: &str,
    want: usize,
) -> Vec<Passage> {
    // Both wikis, not one.
    //
    // Somebody playing The Convergence still asks about vanilla bosses, and
    // somebody playing vanilla still hears about a Convergence spell from a
    // friend. The mod keeps most of the base game, so the two overlap heavily
    // and neither alone is the right answer. Whichever edition is loaded is
    // searched first and its articles win ties, but the other is there.
    let first = crate::wiki::for_edition(edition);
    let mut wikis = vec![first];
    for other in crate::wiki::SOURCES {
        if other.id != first.id {
            wikis.push(other);
        }
    }

    // What a model thinks the wiki calls this, appended to the question. The
    // ranking sees both, so a term the planner gets wrong costs nothing and a
    // term it gets right is usually the article name itself.
    let planned = planned_terms(http, question).await;
    let query = if planned.is_empty() {
        question.to_string()
    } else {
        format!("{question} {}", planned.join(" "))
    };

    let ranked: Vec<(&'static crate::wiki::WikiSource, Vec<String>)> = wikis
        .iter()
        .map(|source| {
            let titles = crate::wiki::titles(app_data, source.id);
            let hits = rank_titles(&titles, &query, want)
                .into_iter()
                .cloned()
                .collect();
            (*source, hits)
        })
        .collect();

    // Taken in turn rather than in order, so the second wiki is always
    // represented. Straight concatenation meant the first wiki filled every
    // slot and the other might as well not have been searched.
    let mut wanted: Vec<(&'static crate::wiki::WikiSource, &String)> = Vec::new();
    let deepest = ranked.iter().map(|(_, hits)| hits.len()).max().unwrap_or(0);
    for rank in 0..deepest {
        for (source, hits) in &ranked {
            if let Some(title) = hits.get(rank) {
                wanted.push((source, title));
            }
        }
    }

    let mut passages = Vec::new();
    for (source, title) in wanted {
        if passages.len() >= want {
            break;
        }
        // Cached articles are free; the rest are fetched once and then are too.
        let Ok(page) = crate::wiki::page(http, app_data, source, title, false).await else {
            continue;
        };
        let text = to_text(&page.html);
        if text.chars().count() < 120 {
            continue;
        }

        // Which wiki said it, in the title the model reads. The Convergence
        // rebalances most of the base game, so the same boss has two sets of
        // numbers — an answer that quietly mixes them is worse than none.
        let labelled = format!("{} · {}", page.title, source.name);
        if passages.iter().any(|p: &Passage| p.title == labelled) {
            continue;
        }

        // The best-matching article gets far more room than the rest.
        //
        // Picking the right two thousand characters out of a twenty-thousand
        // word article is guesswork, and it kept guessing wrong: a question
        // about how to fight a boss landed on her lore and the answer came back
        // "the passages do not say". The models here take a hundred thousand
        // tokens, so the honest fix is to stop guessing and send the section
        // that matters along with everything around it.
        let size = if passages.is_empty() { 7000 } else { 1800 };
        passages.push(Passage {
            title: labelled,
            text: best_window(&text, &query, size),
        });
    }
    passages
}

/// Asks, and says where the answer came from.
pub async fn answer(
    http: &reqwest::Client,
    app_data: &Path,
    edition: Option<&str>,
    question: &str,
) -> Result<Answer> {
    let question = question.trim();
    if question.is_empty() {
        return Err(Error::msg("ask something".to_string()));
    }

    let passages = gather(http, app_data, edition, question, 4).await;
    let sources: Vec<String> = passages.iter().map(|p| p.title.clone()).collect();

    let reply = http
        .post(SERVICE)
        .json(&serde_json::json!({ "question": question, "passages": passages }))
        .send()
        .await
        .map_err(|e| Error::msg(format!("could not reach the answering service: {e}")))?;

    let status = reply.status();
    let body: ServiceReply = reply
        .json()
        .await
        .map_err(|e| Error::msg(format!("the answering service said something odd: {e}")))?;

    if let Some(answer) = body.answer {
        return Ok(Answer { answer, sources, lane: body.lane, ms: body.ms });
    }

    // 429 is the one refusal worth explaining, because it is not a fault.
    if status.as_u16() == 429 {
        return Err(Error::msg(
            "That is a lot of questions at once. Give it a minute.".to_string(),
        ));
    }
    Err(Error::msg(
        body.error.unwrap_or_else(|| "no answer came back".to_string()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles() -> Vec<String> {
        [
            "Malenia, Blade of Miquella",
            "Malenia, Blade of Miquella/dialogue",
            "Malenia's Armor",
            "Malenia's Greaves",
            "Hand of Malenia",
            "Waterfowl Dance",
            "Scarlet Rot",
            "Rot Pot",
            "Rotten Breath",
            "Limgrave",
            "Rivers of Blood",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect()
    }

    #[test]
    fn a_question_keeps_only_the_words_that_narrow_it() {
        let out = terms("How do I beat Malenia in Elden Ring?");
        assert!(out.contains(&"beat".to_string()));
        assert!(out.contains(&"malenia".to_string()));
        // These match every article ever written.
        assert!(!out.contains(&"how".to_string()));
        assert!(!out.contains(&"the".to_string()));
    }

    #[test]
    fn russian_noise_words_go_too() {
        let out = terms("Как убить Малению?");
        assert!(!out.contains(&"как".to_string()));
        assert!(out.iter().any(|w| w.starts_with("малени")));
    }

    #[test]
    fn a_russian_case_ending_still_finds_the_article() {
        // "Малению" is accusative; the article is titled with the nominative.
        // Matching on a prefix is what makes these the same word.
        assert_eq!(stem("малению"), stem("малении"));
        assert_eq!(stem("малению"), stem("маления"));
    }

    #[test]
    fn a_question_in_russian_reaches_an_english_wiki() {
        // Both wikis are written in English. Somebody playing in Russian types
        // "Малению", and no amount of prefix matching bridges two alphabets —
        // this went out with no sources at all until the word was respelled.
        let all = titles();
        let hits = rank_titles(&all, "Как убить Малению?", 3);
        assert!(
            hits.iter().any(|t| t.contains("Malenia")),
            "a Russian question has to find the article: {hits:?}"
        );
    }

    #[test]
    fn transliteration_matches_how_the_wiki_spells_it() {
        assert_eq!(translit("малению").as_deref(), Some("maleniyu"));
        assert_eq!(translit("скарлет").as_deref(), Some("skarlet"));
        // Latin words are left exactly as they are.
        assert_eq!(translit("malenia"), None);
    }

    #[test]
    fn a_loose_spelling_is_tried_when_the_exact_one_finds_nothing() {
        // "Радан" transliterates to "radan", and the wiki writes "Radahn". Five
        // characters miss it; four do not, and the widening only happens when
        // the first pass came back empty.
        let all = vec!["Starscourge Radahn".to_string(), "Limgrave".to_string()];
        let hits = rank_titles(&all, "как убить Радана", 3);
        assert_eq!(hits.first().map(|t| t.as_str()), Some("Starscourge Radahn"), "got {hits:?}");
    }

    #[test]
    fn widening_does_not_drag_in_everything() {
        // The looser pass must only run when the strict one failed, or a
        // four-letter prefix starts matching half the wiki.
        let all2 = titles();
        let hits = rank_titles(&all2, "what is scarlet rot", 4);
        assert_eq!(hits[0].as_str(), "Scarlet Rot", "got {hits:?}");
    }

    #[test]
    fn the_boss_beats_her_own_wardrobe() {
        // Live, a Russian question about fighting Malenia came back with her
        // armour, her greaves and her gauntlets, because the tiebreak was title
        // length and "Malenia's Armor" is shorter than "Malenia, Blade of
        // Miquella". Whether the name heads the title is the signal that works.
        let all = titles();
        for question in ["how do I beat Malenia", "Как убить Малению?"] {
            let hits = rank_titles(&all, question, 4);
            assert_eq!(
                hits.first().map(|t| t.as_str()),
                Some("Malenia, Blade of Miquella"),
                "{question} got {hits:?}"
            );
        }
    }

    #[test]
    fn the_boss_beats_her_own_dialogue_page() {
        // Which of "Malenia, Blade of Miquella" and "Malenia's Armor" comes
        // first is a coin toss from the title alone, and it does not matter:
        // several passages are sent and the model picks. What does matter is
        // that a page of raw dialogue lines never displaces a real article.
        let all = titles();
        let hits = rank_titles(&all, "how do I beat Malenia", 3);
        let boss = hits.iter().position(|t| *t == "Malenia, Blade of Miquella");
        let talk = hits.iter().position(|t| *t == "Malenia, Blade of Miquella/dialogue");
        assert!(boss.is_some(), "the boss page has to be there: {hits:?}");
        assert!(
            talk.is_none() || talk > boss,
            "a dialogue subpage must not outrank the article: {hits:?}"
        );
    }

    #[test]
    fn a_whole_word_outranks_a_fragment() {
        // "rot" appears inside "Rotten Breath" but is a word in "Scarlet Rot".
        let all = titles();
        let hits = rank_titles(&all, "what is scarlet rot", 3);
        assert_eq!(hits[0].as_str(), "Scarlet Rot", "got {hits:?}");
    }

    #[test]
    fn the_name_of_the_game_is_not_a_search_term() {
        // "Как качаться в Convergence" was answered with "Convert Corruption"
        // and "Converted Tower": the mod's own name prefix-matched them and
        // filled every slot. It says which wiki, not what to look for in it.
        // These are the titles the wikis actually carry — there is no page
        // called "Leveling", which is why the gloss names the article rather
        // than translating the verb.
        let all = vec![
            "Convert Corruption".to_string(),
            "Converted Tower".to_string(),
            "Gaol Lower Level Key".to_string(),
            "Stats and Attributes".to_string(),
            "Attributes".to_string(),
            "Runes".to_string(),
        ];
        let hits = rank_titles(&all, "Как качаться в Convergence?", 3);
        assert!(
            hits.iter().any(|t| t.contains("Attributes")),
            "levelling means attributes and runes: {hits:?}"
        );
        assert!(
            !hits.iter().any(|t| t.starts_with("Convert")),
            "the mod's name must not drag these in: {hits:?}"
        );
        assert!(
            !hits.iter().any(|t| t.contains("Gaol")),
            "a key with Level in its name is not how you level up: {hits:?}"
        );
    }

    #[test]
    fn a_question_matching_nothing_returns_nothing() {
        // Better to say so than to hand over four irrelevant articles.
        assert!(rank_titles(&titles(), "what is the wifi password", 4).is_empty());
        assert!(rank_titles(&titles(), "how do I", 4).is_empty());
    }

    #[test]
    fn markup_comes_out_as_prose() {
        let html = "<h2>Notes</h2><p>She heals on <b>every</b> hit.</p><p>Even&nbsp;blocked.</p>";
        let text = to_text(html);
        assert!(text.contains("She heals on every hit."), "got {text:?}");
        assert!(text.contains("Even blocked."), "got {text:?}");
        assert!(!text.contains('<'));
        // A heading must not weld onto the sentence under it.
        assert!(text.contains("Notes She heals"), "got {text:?}");
    }

    #[test]
    fn the_passage_chosen_is_the_one_about_the_question() {
        // The answer is buried a long way into a long article. Taking the first
        // two thousand characters would miss it entirely.
        let lore = "Long ago in the Lands Between there was an age of plenty. ".repeat(60);
        let answer = "Waterfowl Dance is a three-stage flurry; run from the first stage.";
        let tail = "Unrelated trivia about the soundtrack. ".repeat(60);
        let article = format!("{lore}{answer}{tail}");

        let window = best_window(&article, "что делать на Waterfowl Dance", 600);
        assert!(window.contains("three-stage flurry"), "got {window:?}");
    }

    #[test]
    fn a_russian_verb_finds_the_english_paragraph() {
        // Transliteration carries a name across but not a verb: "убить" becomes
        // "ubit", which is in no English article. Live, this meant a question
        // about how to kill a boss picked the lore at the top of her page and
        // the model answered "the passages do not say".
        let lore = "Long ago in the Lands Between there was an age of plenty. ".repeat(50);
        let fight = "Strategy: she heals on every hit, so never trade. Dodge into the flurry.";
        let trivia = "The soundtrack was recorded in Tokyo. ".repeat(50);
        let article = format!("{lore}{fight}{trivia}");

        let window = best_window(&article, "Как убить Малению?", 500);
        assert!(window.contains("never trade"), "got {window:?}");
    }

    #[test]
    fn a_short_article_is_returned_whole() {
        let text = "Malenia heals on every hit.";
        assert_eq!(best_window(text, "malenia", 2000), text);
    }
}
