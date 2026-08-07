//! A model with the wiki as a tool.
//!
//! Not a search box with a model bolted on the end. The launcher holds both
//! wikis on disk and exposes them as two functions — look up titles, read an
//! article — and the model decides what to look up, reads what comes back,
//! looks up something else if that was not it, and answers when it has enough.
//!
//! That is the whole reason this file was rewritten. It used to guess, on the
//! player's behalf, what a question was about: a table of Russian verbs mapped
//! to English article names, a list of words considered too common to search
//! for, a rule about greetings not being worth a lookup. Every one of those was
//! a hand-written fragment of a job a model does properly, and every one of
//! them was wrong in some language nobody had thought about. There are no
//! keyword tables left here.
//!
//! What is left is machinery: an index over the mirrored titles, ranked by how
//! rare each word is rather than by a list of words to ignore, and a loop that
//! carries the model's tool calls back and forth. Both wikis are searched, and
//! which edition is installed only decides which one wins a tie.
//!
//! Retrieval stays on this machine. Only a question and the passages the model
//! actually asked for ever leave it, which is what keeps a handful of free
//! tiers stretching across everybody using this.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where the model lives. No key: the service holds those.
const SERVICE: &str = "https://roundtable-ask.roundtable-launcher.workers.dev/chat";

/// How many times the model may call a tool before it has to answer.
///
/// Enough to look something up, find it was the wrong article, look up the
/// right one, and then check a claim against the other wiki — which is the
/// round that matters, because the mod changes numbers the base game's wiki
/// still prints. Past this it is going in circles, and a player waiting in a
/// boss arena would rather have an honest "I could not find it".
const MAX_ROUNDS: usize = 4;

/// How much of an article comes back from one read.
///
/// The single number that decides whether the pool works. Free tiers meter
/// tokens per minute rather than requests, and every round re-sends the whole
/// conversation — so this is multiplied by the number of reads and again by the
/// number of rounds. At nine thousand characters, which is where it started, a
/// single question pushed something like thirty thousand tokens through one
/// provider in a few seconds and earned a 429 from every lane in turn; the
/// answer then came from whichever model was left, which was the worst one.
///
/// Twenty-six hundred was the number while the pool was one account per
/// provider and every question was a minute of somebody's allowance. It is
/// twenty-five accounts now, and the constraint moved: at twenty-six hundred a
/// question about a boss got the top of the page and came back "there was not
/// enough information", which is a worse failure than a slow answer.
///
/// Four thousand is a whole strategy section, and it is chosen as the section
/// that matches rather than the first one, so it is nearly always the part that
/// holds the answer.
const ARTICLE: usize = 4000;

/// How much of an older tool result is kept when the conversation is re-sent.
///
/// The model has already read it and acted on it; what it needs on the next
/// round is a reminder of what it learned, not the text again. Only the most
/// recent results stay whole.
const STALE: usize = 600;
/// How many tool results stay at full length.
const FRESH: usize = 1;

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// Words out of a piece of text, as the index holds them.
fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2)
        .map(str::to_string)
        .collect()
}

/// Cyrillic respelled in Latin.
///
/// The model writes its searches in English, because it is told the wikis are
/// English, so this is a safety net rather than the mechanism it used to be: if
/// a name comes through in the alphabet the player typed it in, it still finds
/// the article instead of coming back empty.
fn translit(word: &str) -> Option<String> {
    if !word.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)) {
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
            'і' => "i", 'ї' => "yi", 'є' => "ye", 'ґ' => "g", 'ў' => "u",
            'ђ' => "dj", 'ј' => "j", 'љ' => "lj", 'њ' => "nj", 'ћ' => "c",
            'џ' => "dz", 'ѓ' => "g", 'ќ' => "k", 'ѕ' => "dz",
            other => {
                out.push(other);
                continue;
            }
        });
    }
    Some(out)
}

/// Whether two words are the same word.
///
/// A prefix is not enough on its own and an exact match is too much. The model
/// searches in English and the titles are English, so most of the time this is
/// a plain prefix — but a name that came through the alphabet the player typed
/// it in arrives bent: "Малению" transliterates to "maleniyu" and the wiki
/// writes "Malenia", which share six letters and neither of which is a prefix
/// of the other.
///
/// So: a prefix, or a long enough shared beginning. Long enough is five letters
/// and most of the shorter word, which is the line between "maleniyu" meeting
/// "malenia" and "radahn" meeting "radagon" — two different demigods four
/// letters apart.
fn same_word(part: &str, word: &str) -> bool {
    if part.starts_with(word) {
        return true;
    }
    let shared = part
        .chars()
        .zip(word.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let shorter = part.chars().count().min(word.chars().count());
    shared >= 5 && shared * 10 >= shorter * 7
}

/// The titles of one wiki, with how common each word in them is.
///
/// The word counts are what replaced the list of words to ignore. "The" was on
/// that list and "of" was on it and "sword" was not, which meant a search for a
/// sword matched four hundred articles equally. Counting instead of listing
/// gets that right without anybody deciding anything: a word in half the titles
/// carries almost no weight, a word in three carries a great deal, and it
/// works the same in a language nobody thought about.
pub struct Index {
    titles: Vec<String>,
    /// How many titles each word appears in.
    seen: HashMap<String, u32>,
}

impl Index {
    fn build(titles: Vec<String>) -> Self {
        let mut seen: HashMap<String, u32> = HashMap::new();
        for title in &titles {
            let mut once: Vec<String> = tokens(title);
            once.sort();
            once.dedup();
            for word in once {
                *seen.entry(word).or_insert(0) += 1;
            }
        }
        Self { titles, seen }
    }

    /// How much a word is worth: rarer is worth more.
    fn weight(&self, word: &str) -> f32 {
        let total = self.titles.len().max(1) as f32;
        // Prefix, not exact: "radahn" should be worth what "radahn" is worth
        // even when the index knows it as part of longer words.
        let count = self
            .seen
            .get(word)
            .copied()
            .unwrap_or_else(|| {
                self.seen
                    .iter()
                    .filter(|(known, _)| same_word(known, word))
                    .map(|(_, count)| *count)
                    .sum()
            })
            .max(1) as f32;
        (total / count).ln().max(0.0)
    }

    /// Titles worth reading for this query, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(f32, &String)> {
        let mut wanted: Vec<String> = Vec::new();
        for word in tokens(query) {
            if let Some(latin) = translit(&word) {
                if !wanted.contains(&latin) {
                    wanted.push(latin);
                }
            }
            if !wanted.contains(&word) {
                wanted.push(word);
            }
        }
        if wanted.is_empty() {
            return Vec::new();
        }

        let weights: Vec<(String, f32)> =
            wanted.iter().map(|w| (w.clone(), self.weight(w))).collect();

        let mut hits: Vec<(f32, &String)> = self
            .titles
            .iter()
            .filter_map(|title| {
                let lower = title.to_lowercase();
                let parts: Vec<&str> = lower
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| !w.is_empty())
                    .collect();
                if parts.is_empty() {
                    return None;
                }

                // Where the phrase's head sits: the last word before a comma,
                // or the last word of the title. English puts the head of a
                // noun phrase at the end — "Radahn Soldier Set" is a set, and
                // "Starscourge Radahn" is Radahn. Rewarding the *first* word,
                // which this used to do, answered a question about the boss
                // with his soldiers' armour and left his own page out entirely.
                let head = lower
                    .split(',')
                    .next()
                    .map(|main| {
                        main.split(|c: char| !c.is_alphanumeric())
                            .filter(|w| !w.is_empty())
                            .count()
                            .saturating_sub(1)
                    })
                    .unwrap_or(0);

                let mut score = 0.0f32;
                let mut matched = 0usize;

                for (word, weight) in &weights {
                    if *weight <= 0.0 {
                        continue;
                    }
                    let at = parts.iter().position(|part| same_word(part, word));
                    let Some(at) = at else {
                        if lower.contains(word.as_str()) {
                            score += weight * 0.25;
                        }
                        continue;
                    };
                    matched += 1;
                    score += weight;

                    if at == head {
                        score += weight * 0.8;
                    }
                    // Her armour, not her: "Malenia's Armor".
                    if parts.get(at + 1).is_some_and(|next| *next == "s") {
                        score -= weight * 0.7;
                    }
                    // Her weapon, not her: "Hand of Malenia", where the name
                    // does end the phrase and would otherwise be paid for it.
                    if at > 0 && parts.get(at - 1).is_some_and(|prev| *prev == "of") {
                        score -= weight * 0.8;
                    }
                }

                if matched == 0 && score <= 0.0 {
                    return None;
                }
                // Every word matched is worth more than the sum of its parts:
                // a title that covers the whole query is what was asked for.
                score *= 1.0 + 0.25 * (matched.saturating_sub(1) as f32);

                // A subpage is a fragment of an article, not an article. A page
                // of raw dialogue lines answers almost nothing on its own.
                if lower.contains('/') {
                    score *= 0.4;
                }
                // Words the query did not ask about are noise in the title.
                let spare = parts.len().saturating_sub(matched) as f32;
                score /= 1.0 + spare * 0.12;

                (score > 0.0).then_some((score, title))
            })
            .collect();

        hits.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
        hits.truncate(limit);
        hits
    }
}

/// The index for one wiki, built once and kept.
///
/// Rebuilt when the mirror grows, which is what the title count is for — it is
/// not a hash, but a wiki does not change size without changing.
fn index_for(app_data: &Path, source: &'static crate::wiki::WikiSource) -> Arc<Index> {
    /// One built index and the title count it was built from.
    type Cached = (usize, Arc<Index>);
    static CACHE: OnceLock<Mutex<HashMap<&'static str, Cached>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    let titles = crate::wiki::titles(app_data, source.id);
    let mut held = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((count, index)) = held.get(source.id) {
        if *count == titles.len() {
            return Arc::clone(index);
        }
    }
    let built = Arc::new(Index::build(titles));
    held.insert(source.id, (built.titles.len(), Arc::clone(&built)));
    built
}

/// The wikis to search, the installed edition's first.
///
/// Both, always. Somebody playing The Convergence still asks about vanilla
/// bosses, and somebody playing vanilla still hears about a Convergence spell
/// from a friend. The mod keeps most of the base game, so the two overlap
/// heavily and neither alone is the right answer.
fn wikis(edition: Option<&str>) -> Vec<&'static crate::wiki::WikiSource> {
    let first = crate::wiki::for_edition(edition);
    let mut out = vec![first];
    for other in crate::wiki::SOURCES {
        if other.id != first.id {
            out.push(other);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The tools
// ---------------------------------------------------------------------------

/// What the model is told it can do.
///
/// Two functions, described plainly, because a model that is told what a tool
/// is for uses it better than one handed a list of parameters. The descriptions
/// are the only place the shape of the wikis is explained, and they are written
/// for the model rather than for a reader of this file.
fn tool_schemas() -> serde_json::Value {
    serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "search_wiki",
                "description":
                    "Search the ELDEN RING wikis for article titles. Two wikis are searched at \
                     once: the base game's, and The Convergence mod's. Write the query in \
                     ENGLISH — the wikis are written in English — using the names the game \
                     uses, whatever language the player wrote in. Call it again with different \
                     wording if the first attempt found the wrong thing. Returns titles and \
                     which wiki each is from; it does not return their contents.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description":
                                "English search words, e.g. 'Starscourge Radahn boss' or \
                                 'flask upgrade golden seed' or 'arcane attribute scaling'."
                        }
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "item_stats",
                "description":
                    "Look up the game's own numbers for a weapon, armour piece, talisman, \
                     spell, incantation, ash of war, spirit, boss or creature. These come from \
                     a structured database rather than from wiki prose, so they are exact: \
                     attack values, scaling letters, stat requirements, weight, resistances, \
                     boss drops and locations. Faster and more reliable than reading an \
                     article when what you need is a figure. Names are in English.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "English item or boss name, e.g. 'Rivers of Blood'."
                        },
                        "kind": {
                            "type": "string",
                            "description":
                                "Optional filter: weapons, armors, shields, talismans, \
                                 sorceries, incantations, ashes, spirits, ammos, items, \
                                 bosses, creatures, npcs, locations, classes."
                        }
                    },
                    "required": ["name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "player_status",
                "description":
                    "What this player's own game looks like right now: their characters and \
                     levels from the save file, which game version they are on, which total \
                     conversion is installed, and which mods are enabled. Use it when the \
                     answer depends on where they actually are — whether something is worth \
                     it at their level, whether they already have the mod a thing needs, why \
                     something in a guide does not match their game. Takes no arguments.",
                // Empty rather than absent: a tool with no arguments still has
                // to look like every other tool, and some providers reject a
                // schema that leaves either of these out.
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_article",
                "description":
                    "Read a wiki article. Use a title exactly as search_wiki returned it. \
                     Articles run to tens of thousands of words, so say in 'about' what you \
                     want from it and the section that covers it is returned. When the same \
                     article exists in both wikis, reading both is how you check a number: \
                     the mod rebalances the base game, so the two often disagree and the \
                     disagreement is itself the answer.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "The article title, exactly as search_wiki gave it."
                        },
                        "about": {
                            "type": "string",
                            "description":
                                "What you want from it, in English: 'how to beat her, second \
                                 phase', 'where to find it', 'scaling and requirements'."
                        },
                        "wiki": {
                            "type": "string",
                            "enum": ["base", "convergence"],
                            "description":
                                "Which wiki to read it from. Omit to get whichever has it. \
                                 Name one to check a claim against the other."
                        }
                    },
                    "required": ["title"]
                }
            }
        }
    ])
}

/// What one wiki turned up for a search: the wiki, and its best titles.
type Found = (&'static crate::wiki::WikiSource, Vec<String>);

/// A tool the model asked to use.
#[derive(Debug, Clone, Deserialize)]
struct ToolCall {
    id: String,
    #[serde(default)]
    function: ToolFunction,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ToolFunction {
    #[serde(default)]
    name: String,
    /// JSON, as a string, which is how the API carries it.
    #[serde(default)]
    arguments: String,
}

/// What running a tool produced, and what to tell the player it was.
struct Ran {
    /// What goes back to the model.
    output: String,
    /// What the interface shows: "Searching …" or an article title.
    note: Option<String>,
    /// An article that was actually read, for the sources under the answer.
    source: Option<String>,
}

/// What the tools are allowed to know about this machine.
///
/// Passed in rather than reached for, so the whole of this file can be tested
/// without a game installed and so nothing here can wander into state it was
/// not given.
#[derive(Default)]
pub struct Player {
    /// The game's own version, from the executable.
    pub version: Option<String>,
    /// The total conversion in use, by name.
    pub edition: Option<String>,
    /// Characters in the save: name, level, hours.
    pub characters: Vec<(String, u32, u32)>,
    /// Mods switched on in the active profile.
    pub mods: Vec<String>,
    /// True when the frame-generation mod is installed.
    pub framegen: bool,
}

async fn run_tool(
    http: &reqwest::Client,
    app_data: &Path,
    edition: Option<&str>,
    player: &Player,
    call: &ToolCall,
) -> Ran {
    let args: serde_json::Value =
        serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);

    match call.function.name.as_str() {
        "search_wiki" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
            if query.trim().is_empty() {
                return Ran {
                    output: "No query given.".into(),
                    note: None,
                    source: None,
                };
            }

            // Taken in turn rather than in order, so the second wiki is always
            // represented. Straight concatenation meant the first filled every
            // slot and the other might as well not have been searched.
            let mut per_wiki: Vec<Found> = Vec::new();
            for source in wikis(edition) {
                let index = index_for(app_data, source);
                let hits: Vec<String> = index
                    .search(query, 6)
                    .into_iter()
                    .map(|(_, title)| title.clone())
                    .collect();
                per_wiki.push((source, hits));
            }

            let mut lines = Vec::new();
            let deepest = per_wiki.iter().map(|(_, h)| h.len()).max().unwrap_or(0);
            for rank in 0..deepest {
                for (source, hits) in &per_wiki {
                    if let Some(title) = hits.get(rank) {
                        // Whether the other wiki carries it too, said as a fact
                        // rather than as an instruction. A title in both is
                        // where the mod's numbers and the base game's numbers
                        // can be put side by side, and that is worth knowing
                        // before deciding what to read.
                        let both = per_wiki
                            .iter()
                            .filter(|(other, hits)| {
                                other.id != source.id
                                    && hits.iter().any(|t| t.eq_ignore_ascii_case(title))
                            })
                            .count()
                            > 0;
                        lines.push(format!(
                            "{title}  [{}]{}",
                            source.name,
                            if both { "  — also in the other wiki" } else { "" }
                        ));
                    }
                }
            }
            // Each title once, however many wikis it turned up in: the line
            // already says where it can be read.
            let mut seen: Vec<String> = Vec::new();
            lines.retain(|line| {
                let key = line.split("  [").next().unwrap_or(line).to_lowercase();
                let fresh = !seen.contains(&key);
                if fresh {
                    seen.push(key);
                }
                fresh
            });
            lines.truncate(10);

            if lines.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing in either wiki matched \"{query}\". Try different English \
                         words, or answer from your own knowledge and say so."
                    ),
                    note: Some(format!("Searching · {query}")),
                    source: None,
                };
            }
            Ran {
                output: format!("Articles found:\n{}", lines.join("\n")),
                note: Some(format!("Searching · {query}")),
                source: None,
            }
        }

        "read_article" => {
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or_default();
            let about = args.get("about").and_then(|v| v.as_str()).unwrap_or(title);
            let which = args.get("wiki").and_then(|v| v.as_str());
            if title.trim().is_empty() {
                return Ran {
                    output: "No title given.".into(),
                    note: None,
                    source: None,
                };
            }

            // Either the one asked for, or whichever has it — the installed
            // edition's first, since that is the game actually being played.
            let mut order = wikis(edition);
            if let Some(which) = which {
                let wants_mod = which.eq_ignore_ascii_case("convergence");
                order.sort_by_key(|source| {
                    let is_mod = source.id.contains("converg");
                    u8::from(is_mod != wants_mod)
                });
            }

            for source in &order {
                let Ok(page) = crate::wiki::page(http, app_data, source, title, false).await else {
                    continue;
                };
                let text = to_text(&page.html);
                if text.chars().count() < 80 {
                    continue;
                }
                let labelled = format!("{} · {}", page.title, source.name);

                // Whether the same page exists on the other side, so a claim
                // out of this one can be checked without a search first.
                let elsewhere: Vec<&str> = order
                    .iter()
                    .filter(|other| {
                        other.id != source.id
                            && index_for(app_data, other)
                                .search(&page.title, 1)
                                .first()
                                .is_some_and(|(_, found)| found.eq_ignore_ascii_case(&page.title))
                    })
                    .map(|other| other.name)
                    .collect();

                let also = if elsewhere.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\n(This article also exists in: {}. Read it there to check any \
                         number or mechanic, since the mod rebalances the base game.)",
                        elsewhere.join(", ")
                    )
                };

                return Ran {
                    output: format!("{labelled}\n\n{}{also}", best_window(&text, about, ARTICLE)),
                    note: Some(format!("Reading · {} · {}", page.title, source.name)),
                    source: Some(labelled),
                };
            }

            Ran {
                output: format!(
                    "No article called \"{title}\". Search again with different words."
                ),
                note: Some(format!("Reading · {title}")),
                source: None,
            }
        }

        "item_stats" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let kind = args.get("kind").and_then(|v| v.as_str()).filter(|k| !k.is_empty());
            if name.trim().is_empty() {
                return Ran { output: "No name given.".into(), note: None, source: None };
            }

            let entries = crate::codex::load(app_data);
            if entries.is_empty() {
                return Ran {
                    output: "The item database has not been downloaded on this machine. Use \
                             the wiki instead."
                        .into(),
                    note: None,
                    source: None,
                };
            }

            let hits = crate::codex::search(&entries, name, kind, 4);
            if hits.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing called \"{name}\" in the item database. It covers the base \
                         game, so a mod-only item will not be there — search the wiki for it."
                    ),
                    note: Some(format!("Looking up · {name}")),
                    source: None,
                };
            }

            // Every fact it has, unabridged. This is a database row, not an
            // article: it is already short, and cutting it is how a launcher
            // ends up answering a question about scaling without the scaling.
            let mut out = String::new();
            for entry in &hits {
                out.push_str(&format!(
                    "{} ({})\n",
                    entry.name,
                    crate::codex::label_for(&entry.kind)
                ));
                if let Some(description) = &entry.description {
                    out.push_str(&format!("{description}\n"));
                }
                for fact in &entry.facts {
                    out.push_str(&format!("  {}: {}\n", fact.label, fact.value));
                }
                out.push('\n');
            }
            out.push_str(
                "(These are the base game's numbers. If a total conversion is installed, \
                 check its wiki before quoting a figure.)",
            );

            Ran {
                output: out,
                note: Some(format!("Looking up · {}", hits[0].name)),
                source: Some(format!("{} · game data", hits[0].name)),
            }
        }

        "player_status" => {
            let mut out = String::new();

            out.push_str(&format!(
                "Game version: {}\n",
                player.version.as_deref().unwrap_or("unknown")
            ));
            match &player.edition {
                Some(name) => out.push_str(&format!("Total conversion installed: {name}\n")),
                None => out.push_str("No total conversion — the base game.\n"),
            }

            if player.characters.is_empty() {
                out.push_str("No save file found, so no characters to report.\n");
            } else {
                out.push_str("Characters:\n");
                for (name, level, seconds) in &player.characters {
                    out.push_str(&format!(
                        "  {name} — level {level}, {} hours played\n",
                        seconds / 3600
                    ));
                }
            }

            if player.mods.is_empty() {
                out.push_str("No mods enabled.\n");
            } else {
                out.push_str(&format!("Mods enabled: {}\n", player.mods.join(", ")));
            }
            if player.framegen {
                out.push_str("DLSS and frame generation are installed.\n");
            }

            Ran {
                output: out,
                note: Some("Checking your game".into()),
                source: None,
            }
        }

        other => Ran {
            output: format!("There is no tool called {other}."),
            note: None,
            source: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Reading an article
// ---------------------------------------------------------------------------

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
/// An article can run to twenty thousand words. The text is walked in
/// overlapping windows and the one densest in the words the model said it
/// wanted wins — for "how to beat her" that is the strategy section, not the
/// lore at the top.
fn best_window(text: &str, about: &str, size: usize) -> String {
    let wanted: Vec<String> = tokens(about);
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

// ---------------------------------------------------------------------------
// The conversation
// ---------------------------------------------------------------------------

/// A turn of the conversation, kept so the next question can refer to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub question: String,
    pub answer: String,
}

/// Something that happened on the way to an answer.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// What the model is doing right now, in its own words: the search it chose
    /// to run, the article it chose to open.
    Doing { note: String },
    /// The articles the answer was actually built from.
    Sources { sources: Vec<String> },
    /// A piece of the answer, as the model writes it.
    Delta { text: String },
    Done { lane: Option<String>, ms: Option<u64> },
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub answer: String,
    pub sources: Vec<String>,
    pub lane: Option<String>,
    pub ms: Option<u64>,
}

/// What one call to the service came back with.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    #[serde(default)]
    lane: Option<String>,
    #[serde(default)]
    ms: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

/// Asks, letting the model do the looking.
///
/// The loop is here rather than in the service because the wikis are here: the
/// model says what it wants, this fetches it off the disk beside the game, and
/// only the passages it asked for are sent on. The last turn is streamed, so
/// the answer appears as it is written.
pub async fn answer_stream<F>(
    http: &reqwest::Client,
    app_data: &Path,
    edition: Option<&str>,
    player: &Player,
    question: &str,
    history: &[Turn],
    mut emit: F,
) where
    F: FnMut(Event),
{
    let question = question.trim();
    if question.is_empty() {
        emit(Event::Failed { error: "ask something".into() });
        return;
    }

    let mut messages: Vec<serde_json::Value> = Vec::new();
    for turn in history.iter().rev().take(4).rev() {
        messages.push(serde_json::json!({ "role": "user", "content": turn.question }));
        messages.push(serde_json::json!({ "role": "assistant", "content": turn.answer }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": question }));

    let tools = tool_schemas();
    let mut sources: Vec<String> = Vec::new();
    let mut steps: Vec<String> = Vec::new();

    for round in 0..MAX_ROUNDS {
        // The last round is forced to answer: a model still calling tools at
        // the limit would otherwise return nothing at all.
        let last = round + 1 == MAX_ROUNDS;
        if last {
            // Taking the tools away is not enough on its own. Llama, handed no
            // tools and still wanting one, writes the call out as prose —
            // `<function=read_article>{"title":…}</function>` arrived as an
            // answer, which is worse than a wrong answer because it is not even
            // in a language. Saying so plainly stops it.
            messages.push(serde_json::json!({
                "role": "user",
                "content": "You have no tools left for this question. Answer now, in prose, \
                            from what you found. If it was not enough, say what you could not \
                            find and answer from your own knowledge.",
            }));
        }

        let body = serde_json::json!({
            "messages": trimmed(&messages),
            "tools": if last { serde_json::Value::Null } else { tools.clone() },
            "edition": edition,
            "stream": false,
        });

        let reply = match post_chat(http, &body).await {
            Ok(reply) => reply,
            Err(error) => {
                emit(Event::Failed { error });
                return;
            }
        };

        if let Some(error) = reply.error {
            emit(Event::Failed { error });
            return;
        }

        if reply.tool_calls.is_empty() {
            // Nothing more it wants. Ask again with streaming on, so the answer
            // arrives as it is written rather than in one piece at the end.
            if !sources.is_empty() {
                emit(Event::Sources { sources: sources.clone() });
            }
            stream_final(http, &messages, edition, question, &mut emit).await;
            return;
        }

        // The assistant turn that made the calls has to go back verbatim, or
        // the tool results below have nothing to attach to.
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": reply.content.unwrap_or_default(),
            "tool_calls": reply.tool_calls.iter().map(|call| serde_json::json!({
                "id": call.id,
                "type": "function",
                "function": { "name": call.function.name, "arguments": call.function.arguments },
            })).collect::<Vec<_>>(),
        }));

        // All at once when it asked for several. Reading an article that is not
        // cached yet is a web request, and two of those in a row is two of
        // those in a row — which is most of the wait when the model checks a
        // number against the other wiki.
        let ran: Vec<Ran> = futures_util::future::join_all(
            reply
                .tool_calls
                .iter()
                .map(|call| run_tool(http, app_data, edition, player, call)),
        )
        .await;

        for (call, ran) in reply.tool_calls.iter().zip(ran) {
            if let Some(note) = ran.note {
                // The same step twice is the model going round in a circle, and
                // showing it twice makes the wait look longer than it is.
                if !steps.contains(&note) {
                    steps.push(note.clone());
                    emit(Event::Doing { note });
                }
            }
            if let Some(source) = ran.source {
                if !sources.contains(&source) {
                    sources.push(source);
                }
            }
            messages.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": ran.output,
            }));
        }
    }

    if !sources.is_empty() {
        emit(Event::Sources { sources });
    }
    stream_final(http, &messages, edition, question, &mut emit).await;
}

/// The conversation with the parts nobody needs in full any more cut down.
///
/// Every round re-sends everything, and the free tiers meter tokens per minute
/// — so an article read four rounds ago is four times the cost of reading it
/// once, for no benefit. The model has already acted on it; what it needs now
/// is the reminder, not the text.
///
/// The most recent results stay whole, because those are the ones it is
/// actually reasoning over. Nothing is dropped: a truncated result still says
/// which article it was and what the first of it said, so the model can ask for
/// it again if it turns out to matter.
fn trimmed(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let results: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .map(|(at, _)| at)
        .collect();
    let keep_from = results.len().saturating_sub(FRESH);
    let whole: Vec<usize> = results[keep_from..].to_vec();

    messages
        .iter()
        .enumerate()
        .map(|(at, message)| {
            if message.get("role").and_then(|r| r.as_str()) != Some("tool") || whole.contains(&at) {
                return message.clone();
            }
            let content = message.get("content").and_then(|c| c.as_str()).unwrap_or("");
            if content.chars().count() <= STALE {
                return message.clone();
            }
            let mut short: String = content.chars().take(STALE).collect();
            short.push_str("\n… (the rest of this is no longer being carried; read it again if you need it)");
            let mut copy = message.clone();
            copy["content"] = serde_json::Value::String(short);
            copy
        })
        .collect()
}

/// Whether waiting a moment and asking again is likely to work.
///
/// A free tier meters tokens per minute, so "every lane refused" usually means
/// the accounts are a few seconds over their allowance rather than down. The
/// pool cools a refused account for six seconds; sleeping for about that long
/// and asking once more turns most of these into an answer that took a little
/// longer, instead of a failure the player has to retype their question after.
fn worth_retrying(error: &str) -> bool {
    error.contains("every lane refused")
        || error.contains("no lane has any allowance left")
        || error.contains("503")
}

async fn post_chat(
    http: &reqwest::Client,
    body: &serde_json::Value,
) -> std::result::Result<Reply, String> {
    match post_once(http, body).await {
        Ok(reply) => match reply.error.as_deref() {
            Some(error) if worth_retrying(error) => {}
            _ => return Ok(reply),
        },
        Err(error) if !worth_retrying(&error) => return Err(error),
        Err(_) => {}
    }

    tokio::time::sleep(std::time::Duration::from_millis(7000)).await;
    post_once(http, body).await
}

async fn post_once(
    http: &reqwest::Client,
    body: &serde_json::Value,
) -> std::result::Result<Reply, String> {
    let reply = http
        .post(SERVICE)
        .json(body)
        .send()
        .await
        .map_err(|error| format!("could not reach the answering service: {error}"))?;

    if reply.status().as_u16() == 429 {
        return Err("That is a lot of questions at once. Give it a minute.".into());
    }

    // Read as text first, so a response that is not JSON can be reported as
    // what it actually was. "error decoding response body" told nobody
    // anything; the body underneath it was a gateway error page.
    let status = reply.status();
    let body = reply
        .text()
        .await
        .map_err(|error| format!("the answer did not arrive: {error}"))?;

    serde_json::from_str::<Reply>(&body).map_err(|_| {
        let snippet: String = body.chars().take(160).collect();
        if snippet.trim().is_empty() {
            format!("the answering service returned nothing ({status})")
        } else {
            format!("the answering service returned {status}: {snippet}")
        }
    })
}

/// The answer itself, a few words at a time.
async fn stream_final<F>(
    http: &reqwest::Client,
    messages: &[serde_json::Value],
    edition: Option<&str>,
    question: &str,
    emit: &mut F,
) where
    F: FnMut(Event),
{
    // A reminder of which language to write in, as an instruction rather than
    // as a message.
    //
    // "Reply in the language the player used" is in the system prompt and is
    // followed most of the time. Most of the time is not enough: a Japanese
    // question came back in English after four rounds of reading English wiki
    // pages, because by then the English was all the model could see. Showing
    // it the original words again fixes that, and needs no language detection
    // at all — the thing to match is right there.
    //
    // It has to arrive as a system instruction. The first attempt put the
    // question back as a *user* message, and short inputs were then echoed
    // straight back: "asdfgh" was answered with "asdfgh". A model handed a
    // user turn will try to respond to it, and the only sensible response to a
    // repeated question is the question.
    let mut messages = trimmed(messages);
    messages.push(serde_json::json!({
        "role": "system",
        "content": format!(
            "Write your reply in the same language the player used. For reference, this \
             is what they asked, which you must answer rather than repeat:\n\n{question}"
        ),
    }));

    let body = serde_json::json!({
        "messages": messages,
        "edition": edition,
        "stream": true,
    });

    let sent = http.post(SERVICE).json(&body).send().await;
    let reply = match sent {
        Ok(reply) => reply,
        Err(error) => {
            emit(Event::Failed {
                error: format!("could not reach the answering service: {error}"),
            });
            return;
        }
    };

    if reply.status().as_u16() == 429 {
        emit(Event::Failed {
            error: "That is a lot of questions at once. Give it a minute.".into(),
        });
        return;
    }

    let streamed = reply
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|kind| kind.contains("text/event-stream"));

    // A service that has not been redeployed yet answers in one piece, and that
    // still has to work — a launcher whose answers depend on which day the
    // worker was last pushed is not one anybody can rely on.
    if !streamed {
        match reply.json::<Reply>().await {
            Ok(body) => match body.content.filter(|text| !text.trim().is_empty()) {
                Some(answer) => {
                    emit(Event::Delta { text: answer });
                    emit(Event::Done { lane: body.lane, ms: body.ms });
                }
                None => emit(Event::Failed {
                    error: body.error.unwrap_or_else(|| "no answer came back".into()),
                }),
            },
            Err(error) => emit(Event::Failed {
                error: format!("the answering service said something odd: {error}"),
            }),
        }
        return;
    }

    if read_events(reply, emit).await == Outcome::WorthRetrying {
        tokio::time::sleep(std::time::Duration::from_millis(7000)).await;
        match http.post(SERVICE).json(&body).send().await {
            Ok(second) => {
                if read_events(second, emit).await == Outcome::WorthRetrying {
                    emit(Event::Failed {
                        error: "every model is busy right now. Ask again in a moment.".into(),
                    });
                }
            }
            Err(error) => emit(Event::Failed {
                error: format!("could not reach the answering service: {error}"),
            }),
        }
    }
}

/// How a stream ended.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Something was said, or the failure is not one waiting helps with.
    Done,
    /// Every lane was over its allowance, which lasts seconds.
    WorthRetrying,
}

/// Server-sent events from the service, turned into ours.
async fn read_events<F>(reply: reqwest::Response, emit: &mut F) -> Outcome
where
    F: FnMut(Event),
{
    use futures_util::StreamExt;

    #[derive(Deserialize)]
    struct Chunk {
        delta: Option<String>,
        lane: Option<String>,
        ms: Option<u64>,
        done: Option<bool>,
        error: Option<String>,
    }

    let mut stream = reply.bytes_stream();
    let mut buffer = String::new();
    let mut lane: Option<String> = None;
    let mut said = false;
    let mut ended = false;

    while let Some(chunk) = stream.next().await {
        let Ok(bytes) = chunk else { break };
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        // A chunk can end mid-line, so only whole lines are taken and the rest
        // is carried into the next one.
        while let Some(cut) = buffer.find('\n') {
            let line = buffer[..cut].trim().to_string();
            buffer.drain(..=cut);
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                ended = true;
                break;
            }
            let Ok(chunk) = serde_json::from_str::<Chunk>(payload) else {
                continue;
            };
            if let Some(error) = chunk.error {
                if !said && worth_retrying(&error) {
                    return Outcome::WorthRetrying;
                }
                emit(Event::Failed { error });
                return Outcome::Done;
            }
            if chunk.lane.is_some() {
                lane = chunk.lane.clone();
            }
            if let Some(text) = chunk.delta {
                if !text.is_empty() {
                    said = true;
                    emit(Event::Delta { text });
                }
            }
            if chunk.done == Some(true) {
                emit(Event::Done { lane: lane.clone(), ms: chunk.ms });
                return Outcome::Done;
            }
        }
        if ended {
            break;
        }
    }

    if said {
        emit(Event::Done { lane, ms: None });
        Outcome::Done
    } else {
        Outcome::WorthRetrying
    }
}

/// The whole answer in one piece, for callers with nowhere to put a stream.
pub async fn answer(
    http: &reqwest::Client,
    app_data: &Path,
    edition: Option<&str>,
    player: &Player,
    question: &str,
) -> Result<Answer> {
    let mut text = String::new();
    let mut sources = Vec::new();
    let mut lane = None;
    let mut ms = None;
    let mut failure = None;

    answer_stream(http, app_data, edition, player, question, &[], |event| match event {
        Event::Delta { text: piece } => text.push_str(&piece),
        Event::Sources { sources: found } => sources = found,
        Event::Done { lane: which, ms: took } => {
            lane = which;
            ms = took;
        }
        Event::Failed { error } => failure = Some(error),
        Event::Doing { .. } => {}
    })
    .await;

    if let Some(error) = failure {
        return Err(Error::msg(error));
    }
    Ok(Answer { answer: text, sources, lane, ms })
}

/// Which articles a question matches, for anything that wants a list without
/// spending a model call on it.
pub fn matching_titles(app_data: &Path, edition: Option<&str>, query: &str, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for source in wikis(edition) {
        for (_, title) in index_for(app_data, source).search(query, limit) {
            out.push(PathBuf::from(title));
        }
    }
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(titles: &[&str]) -> Index {
        Index::build(titles.iter().map(|s| (*s).to_string()).collect())
    }

    fn best<'a>(index: &'a Index, query: &str) -> Option<&'a String> {
        index.search(query, 5).first().map(|(_, title)| *title)
    }

    fn wiki() -> Index {
        index(&[
            "Malenia, Blade of Miquella",
            "Malenia, Blade of Miquella/dialogue",
            "Malenia's Armor",
            "Malenia's Greaves",
            "Hand of Malenia",
            "Starscourge Radahn",
            "Radahn Soldier Set",
            "Radahn Soldier",
            "Radahn Soldier Ashes",
            "Radahn Festival",
            "Radahn's Great Rune",
            "Waterfowl Dance",
            "Scarlet Rot",
            "Rot Pot",
            "Rotten Breath",
            "Vigor",
            "Invigorating Cured Meat",
            "Arcane",
            "Arcane-Knot Crystal Tear",
            "Limgrave",
            "Rivers of Blood",
            "Bloodhound's Fang",
            "Flask Upgrades",
            "Golden Seed",
            "Sacred Tear",
        ])
    }

    #[test]
    fn the_page_about_the_thing_beats_the_pages_that_mention_it() {
        // Live, "how do I beat Radahn" came back with his soldiers' armour set,
        // his soldiers' ashes and the festival named after him — and not his
        // own page at all. English puts the head of a noun phrase last:
        // "Radahn Soldier Set" is a set, "Starscourge Radahn" is Radahn.
        let all = wiki();
        assert_eq!(best(&all, "Starscourge Radahn boss").map(String::as_str), Some("Starscourge Radahn"));
        assert_eq!(best(&all, "Radahn").map(String::as_str), Some("Starscourge Radahn"));
    }

    #[test]
    fn the_boss_beats_her_own_wardrobe() {
        // The tiebreak used to be title length, and "Malenia's Armor" is
        // shorter than "Malenia, Blade of Miquella".
        let all = wiki();
        assert_eq!(
            best(&all, "Malenia boss strategy").map(String::as_str),
            Some("Malenia, Blade of Miquella")
        );
        // A page of raw dialogue lines is not an article.
        let hits = all.search("Malenia", 3);
        let boss = hits.iter().position(|(_, t)| *t == "Malenia, Blade of Miquella");
        let talk = hits.iter().position(|(_, t)| t.contains("/dialogue"));
        assert!(boss.is_some(), "{hits:?}");
        assert!(talk.is_none() || talk > boss, "{hits:?}");
    }

    #[test]
    fn a_common_word_is_worth_less_than_a_rare_one_without_a_list_of_common_words() {
        // This is what replaced the hand-written list of words to ignore. The
        // list had "the" on it and not "soldier", so a search for a soldier's
        // armour matched every soldier page equally.
        let all = wiki();
        assert!(
            all.weight("radahn") < all.weight("starscourge"),
            "a word in five titles must be worth less than a word in one"
        );
        assert!(all.weight("festival") > all.weight("radahn"));
    }

    #[test]
    fn a_whole_word_outranks_a_fragment() {
        let all = wiki();
        assert_eq!(best(&all, "scarlet rot").map(String::as_str), Some("Scarlet Rot"));
        assert_eq!(best(&all, "vigor").map(String::as_str), Some("Vigor"));
        assert_eq!(best(&all, "arcane").map(String::as_str), Some("Arcane"));
    }

    #[test]
    fn a_query_matching_nothing_returns_nothing() {
        // Better to say so than to hand over five irrelevant articles.
        let all = wiki();
        assert!(all.search("wifi password", 4).is_empty());
        assert!(all.search("", 4).is_empty());
    }

    #[test]
    fn cyrillic_still_finds_the_article_when_the_model_echoes_it() {
        // The model is told to search in English and does. This is the net
        // under that, not the mechanism it used to be.
        let all = wiki();
        assert_eq!(best(&all, "Малению").map(String::as_str), Some("Malenia, Blade of Miquella"));
        assert_eq!(translit("малению").as_deref(), Some("maleniyu"));
        assert_eq!(translit("malenia"), None);
    }

    #[test]
    fn the_tools_are_described_in_a_shape_the_api_expects() {
        let schemas = tool_schemas();
        let list = schemas.as_array().expect("an array of tools");
        for tool in list {
            assert_eq!(tool["type"], "function");
            let function = &tool["function"];
            assert!(function["name"].as_str().is_some_and(|n| !n.is_empty()));
            assert!(function["description"].as_str().is_some_and(|d| d.len() > 40));
            assert_eq!(function["parameters"]["type"], "object");
            assert!(function["parameters"]["required"].as_array().is_some());
        }
        let names: Vec<&str> = list
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        for wanted in ["search_wiki", "read_article", "item_stats", "player_status"] {
            assert!(names.contains(&wanted), "{wanted} is not offered: {names:?}");
        }

        // Every name the model can send has an arm that answers it. A tool
        // described but not implemented is a round wasted on "there is no tool
        // called that", which the player waits through.
        let unhandled: Vec<&&str> = names
            .iter()
            .filter(|name| {
                !matches!(
                    **name,
                    "search_wiki" | "read_article" | "item_stats" | "player_status"
                )
            })
            .collect();
        assert!(unhandled.is_empty(), "described but not implemented: {unhandled:?}");
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
    fn the_passage_chosen_is_the_one_the_model_asked_about() {
        // The answer is buried a long way into a long article. Taking the first
        // characters would miss it entirely.
        let lore = "Long ago in the Lands Between there was an age of plenty. ".repeat(60);
        let answer = "Waterfowl Dance is a three-stage flurry; run from the first stage.";
        let tail = "Unrelated trivia about the soundtrack. ".repeat(60);
        let article = format!("{lore}{answer}{tail}");

        let window = best_window(&article, "waterfowl dance flurry", 600);
        assert!(window.contains("three-stage flurry"), "got {window:?}");
    }

    #[test]
    fn a_short_article_is_returned_whole() {
        let text = "Malenia heals on every hit.";
        assert_eq!(best_window(text, "malenia", 2000), text);
    }
}
