//! A model with the wiki as a tool.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Where the model lives. No key: the service holds those.
const SERVICE: &str = "https://roundtable-ask.roundtable-launcher.workers.dev/chat";

/// Tool calls allowed before it must answer.
const MAX_ROUNDS: usize = 4;

/// Characters returned from one article read.
const ARTICLE: usize = 4000;

/// Characters of a web page's opening always sent, on top of the match.
const OPENING: usize = 1400;

/// How much of an older tool result is kept when the conversation is re-sent.
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

/// Cyrillic respelled in Latin. A safety net for names typed in the wrong alphabet.
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

/// Whether two words are the same word: prefix, or a long enough shared start.
fn same_word(part: &str, word: &str) -> bool {
    if part == word {
        return true;
    }
    // A short word has to match outright, or "to" matches "Torrent".
    if word.chars().count() >= MEANINGFUL && part.starts_with(word) {
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

/// The letter grade the game prints for a scaling value. Bands pinned to the stat screen.
fn grade(value: f32) -> &'static str {
    match value {
        v if v >= 175.0 => "S",
        v if v >= 140.0 => "A",
        v if v >= 90.0 => "B",
        v if v >= 50.0 => "C",
        v if v >= 25.0 => "D",
        v if v > 0.0 => "E",
        _ => "-",
    }
}

/// What is left of a title's score when the query named something and the title
const UNNAMED: f32 = 0.3;

/// The words a query wrote with a capital letter, lowercased.
fn capitalised(query: &str) -> Vec<String> {
    let mut found = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        if word.chars().count() < 2 {
            continue;
        }
        if !word.chars().next().is_some_and(char::is_uppercase) {
            continue;
        }
        let lower = word.to_lowercase();
        if let Some(latin) = translit(&lower) {
            if !found.contains(&latin) {
                found.push(latin);
            }
        }
        if !found.contains(&lower) {
            found.push(lower);
        }
    }
    found
}

/// How many single-letter changes turn one word into the other.
fn apart(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let most = 4usize;
    if a.len().abs_diff(b.len()) > most {
        return most;
    }

    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, from) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, to) in b.iter().enumerate() {
            let cost = usize::from(from != to);
            let next = (row[j] + 1).min(row[j + 1] + 1).min(previous + cost);
            previous = row[j + 1];
            row[j + 1] = next;
        }
        if row.iter().min().copied().unwrap_or(most) > most {
            return most;
        }
    }
    row[b.len()].min(most)
}

/// What two titles share when they name the same article.
fn one_thing(title: &str) -> String {
    let mut key: String = title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    if key.len() > 3 && key.ends_with('s') {
        key.pop();
    }
    key
}

/// The length at which a word is long enough to stand for what follows it.
const MEANINGFUL: usize = 4;

/// The titles of one wiki, with how common each word in them is.
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

    /// Titles whose words are nearly the one given, for a name that matched
    pub fn nearest(&self, word: &str, limit: usize) -> Vec<String> {
        let needle = word.to_lowercase();
        if needle.chars().count() < MEANINGFUL {
            return Vec::new();
        }
        // Two letters in a seven-letter name, which is what "Relanna" is away
        // from "Rellana". Generous on purpose: this only runs for a word that
        // matched nothing at all, and what it returns is offered to the model
        // as a guess rather than used as an answer.
        let room = ((needle.chars().count() + 2) / 4).clamp(1, 3);

        let mut close: Vec<(usize, &String)> = Vec::new();
        for title in &self.titles {
            let best = title
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|part| !part.is_empty())
                .map(|part| apart(part, &needle))
                .min();
            if best.is_some_and(|distance| distance <= room) {
                close.push((best.unwrap_or(room), title));
            }
        }
        close.sort_by_key(|(distance, title)| (*distance, title.chars().count()));

        let mut out: Vec<String> = Vec::new();
        for (_, title) in close {
            if out.iter().any(|kept| one_thing(kept) == one_thing(title)) {
                continue;
            }
            out.push(title.clone());
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    /// The names in a query that appear in no title at all.
    pub fn unmatched(&self, query: &str) -> Vec<String> {
        let mut missing = Vec::new();
        let words: Vec<&str> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        for word in words.into_iter().skip(1) {
            let lower = word.to_lowercase();
            if lower.chars().count() < MEANINGFUL {
                continue;
            }
            if !word.chars().next().is_some_and(char::is_uppercase) {
                continue;
            }
            let forms: Vec<String> = translit(&lower).into_iter().chain([lower]).collect();
            let known = forms.iter().any(|form| {
                self.seen.keys().any(|known| same_word(known, form))
                    || self.titles.iter().any(|t| t.to_lowercase().contains(form.as_str()))
            });
            if !known && !missing.contains(&word.to_string()) {
                missing.push(word.to_string());
            }
        }
        missing
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

        // What the reader capitalised is what they are asking about. Rarity
        // gets this backwards: "boss" is worth 6.9 against "radahn" at 5.5,
        // because a demigod with a dozen articles looks common while a word
        // confined to index pages looks rare — so "Radahn boss" was answered
        // with the list of every boss in the game.
        let named = capitalised(query);
        let weights: Vec<(String, f32)> =
            wanted.iter().map(|w| (w.clone(), self.weight(w))).collect();
        // What the whole question is worth, to judge how much of it a title
        // actually answers.
        let asked: f32 = weights.iter().map(|(_, w)| w.max(0.0)).sum::<f32>().max(1.0);

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
                let mut covered = 0.0f32;
                let mut has_a_name = false;

                for (word, weight) in &weights {
                    if *weight <= 0.0 {
                        continue;
                    }
                    let at = parts.iter().position(|part| same_word(part, word));
                    let Some(at) = at else {
                        // Buried inside a longer word, which is worth something
                        // for a real word and nothing at all for a fragment:
                        // the Russian "за" is two letters that fall inside
                        // Salza, Zamor and Wakizashi, and a question asked in
                        // Russian came back as a list of names ending in -za.
                        if word.chars().count() >= MEANINGFUL && lower.contains(word.as_str()) {
                            score += weight * 0.25;
                        }
                        continue;
                    };
                    matched += 1;
                    covered += weight;
                    score += weight;
                    if named.contains(word) {
                        has_a_name = true;
                    }

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
                // Covering the query is worth more than the sum of its parts —
                // but measured by what was covered, not by how many words.
                //
                // Counting them rewarded a title for matching junk: "what does
                // arcane do" put "What Do You Want?" above "Arcane", two empty
                // words beating the one that carried the question.
                score *= 1.0 + 0.5 * (covered / asked);

                // A subpage is a fragment of an article, not an article. A page
                // of raw dialogue lines answers almost nothing on its own.
                if lower.contains('/') {
                    score *= 0.4;
                }
                // Nothing the query named. When a query names something, a
                // title that does not mention it is about something else, and
                // no amount of matching the ordinary words around it changes
                // that.
                if !named.is_empty() && !has_a_name {
                    score *= UNNAMED;
                }

                // Words the query did not ask about are noise in the title.
                let spare = parts.len().saturating_sub(matched) as f32;
                score /= 1.0 + spare * 0.12;

                (score > 0.0).then_some((score, title))
            })
            .collect();

        hits.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));

        // One thing, once.
        //
        // The index carries the wiki's redirects so that a search for a bleed
        // build reaches blood loss, and the cost of that is families of near
        // identical names: "Sacred Flask", "Sacred Flasks" and "Sacred flasks"
        // are one article under three titles, and left alone they took three of
        // the six answers and pushed "Sacred Tear" — the page actually being
        // asked for — off the end.
        let mut already: Vec<String> = Vec::new();
        hits.retain(|(_, title)| {
            let key = one_thing(title);
            if already.contains(&key) {
                return false;
            }
            already.push(key);
            true
        });

        hits.truncate(limit);
        hits
    }
}

/// The index for one wiki, built once and kept.
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
fn wikis(edition: Option<&str>) -> Vec<&'static crate::wiki::WikiSource> {
    reading(edition, None)
}

/// The two-letter code for a language the game names in full.
fn short_code(language: &str) -> Option<&'static str> {
    let plain = language.to_lowercase();
    [
        ("rus", "ru"),
        ("ukrain", "uk"),
        ("polish", "pl"),
        ("german", "de"),
        ("french", "fr"),
        ("spanish", "es"),
        ("italian", "it"),
        ("portug", "pt"),
        ("japan", "ja"),
        ("korean", "ko"),
        ("chinese", "zh"),
    ]
    .into_iter()
    .find(|(needle, _)| plain.contains(needle))
    .map(|(_, code)| code)
}

/// The wikis worth searching, given what language the game is in.
fn reading(
    edition: Option<&str>,
    language: Option<&str>,
) -> Vec<&'static crate::wiki::WikiSource> {
    let first = crate::wiki::for_edition(edition);
    let mut out = vec![first];
    for other in crate::wiki::SOURCES {
        if other.id == first.id {
            continue;
        }
        if let Some(needs) = crate::wiki::spoken_in(other) {
            let theirs = language.unwrap_or_default().to_lowercase();
            if !theirs.contains(needs) {
                continue;
            }
        }
        out.push(other);
    }
    out
}

// ---------------------------------------------------------------------------
// The tools
// ---------------------------------------------------------------------------

/// What the model is told it can do.
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
                    "This player's own game. When it is running: their level, runes, health, all \
                     eight attributes, the region of the map they are standing in, and \
                     EVERYTHING THEY HAVE EQUIPPED — the weapons in their hands, every armour \
                     piece by slot, and their talismans, under the names their own screen shows. \
                     All of it as of this second, read out of the game itself. Otherwise the same \
                     from their save file, plus the game version, which total conversion is \
                     installed and which mods are on.\n\
                     \n\
                     Use it whenever the answer depends on where they actually are or on what \
                     they are carrying: whether a boss is worth attempting at their level, \
                     whether they meet a weapon's requirements, what their armour or talismans \
                     are, what is near them, why a guide does not match their game. Never ask \
                     them to type out what they have equipped — call this instead. No arguments.",
                // Empty rather than absent: a tool with no arguments still has
                // to look like every other tool, and some providers reject a
                // schema that leaves either of these out.
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "game_item",
                "description":
                    "Look something up in THIS player's running game, in the game's own words — \
                     items, weapons, armour, talismans, PLACES and characters. Returns what it \
                     is called on their screen, the line the menu prints for what it does, and \
                     its description, read out of the game's memory. Right even for things a \
                     total conversion invented or renamed, which no wiki and no shipped table \
                     can be. It needs the game to be open.\n\
                     \n\
                     Use it before naming any place or item in your answer when the player is \
                     not reading English. Translating an English wiki name yourself produces a \
                     name that is not on their screen: the arena the wiki calls the Wailing \
                     Dunes is printed in a Russian copy as \"Воющие дюны\", and a sensible \
                     translation gave \"Стонущие дюны\", which the player had never seen. Names \
                     come back in the language the game is installed in, which player_status \
                     reports — search with a word in that language, or part of a name you were \
                     already given.\n\
                     \n\
                     It says what a thing IS and never WHERE IT IS. Nothing in the game's text \
                     carries a location, so an answer about where to find something has to come \
                     out of a wiki article you have actually read. Asked where a katana was, a \
                     model called this, got the flavour text, and placed the weapon in a \
                     \"Ruined Sellian Temple in Soap Valley\" — neither of which is a place in \
                     any version of this game.\n\
                     \n\
                     When the mod's own wiki has no location for something, open the BASE \
                     GAME's article for that item and read it — both wikis are mirrored here \
                     and the base game's is usually where a location is written down. Recalling \
                     one instead is how the same katana ended up in the Erdtree Church, which \
                     is not where it is either. Read the item's own page, not a page listing \
                     locations, and say which game's the answer is for.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "Part of the item's name, in the game's own language. Part of \
                                 one is enough and often better: a stem matches the whole \
                                 family, where a full name with the wrong ending matches nothing."
                        }
                    },
                    "required": ["name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "spell_numbers",
                "description":
                    "What a sorcery or incantation costs in THIS player's game: the FP, the \
                     stamina, how many memory slots it takes and what it asks of their \
                     attributes — read out of the tables their installation runs on.\n\
                     \n\
                     Use it for any question about a spell's cost or requirements, in preference \
                     to item_stats and the wikis. Those are the base game: a total conversion \
                     rewrites these numbers and invents hundreds of spells of its own, and \
                     answering from the base game's figures is answering about a different \
                     game.\n\
                     \n\
                     Give the name the way their game names it — player_status lists what they \
                     have, and game_item finds the game's own spelling.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "The spell, as the game names it. Part of the name is enough."
                        },
                        "faith": {
                            "type": "integer",
                            "description":
                                "Instead of a name: list what this much faith would open. Use it \
                                 for \"what would raising faith to 30 give me\" — the answer is \
                                 the spells between what they have now and this, out of their \
                                 own tables, and never a list from memory. It also answers \
                                 \"which need at least this much\": both lists come back, the \
                                 ones that open (needing no more than this) and the ones that \
                                 need this or MORE, so use the one the question asked. With no \
                                 name and no attribute at all, it answers what they can cast as \
                                 they stand and how many spells this game has altogether."
                        },
                        "intelligence": {
                            "type": "integer",
                            "description": "The same for intelligence."
                        },
                        "arcane": {
                            "type": "integer",
                            "description": "The same for arcane."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "game_text",
                "description":
                    "Search EVERYTHING the installed game has written down, in the language it \
                     is played in: item and weapon and armour descriptions, tutorials, menu \
                     entries, what characters say, notes on the map. Fifty-four thousand lines \
                     of it, and all of it written by whoever built this installation, so for a \
                     total conversion it is the only place its own mechanics are explained at \
                     all.\n\
                     \n\
                     REACH FOR THIS BEFORE THE WIKI when the question is about a word or an \
                     idea rather than a number — a mechanic, a menu entry, a term they read \
                     somewhere, what something is FOR. A wiki is the base game written by \
                     strangers; this is their own copy explaining itself.\n\
                     \n\
                     It is what the launcher was missing. Somebody standing at a grace, reading \
                     \"Трансмогрификация брони\" off the screen in front of them, asked what it \
                     was and was told no article covered it and to try a forum. Their game holds \
                     a tutorial explaining it in full and an item that removes it, both in their \
                     language, and nothing here could see either.\n\
                     \n\
                     Match on part of a word, not a whole one: these languages inflect and a \
                     full word misses its own endings. Two characters minimum.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "words": {
                            "type": "string",
                            "description":
                                "Part of a word, in the language their game is in — the block \
                                 above says which. A word in any other language matches nothing."
                        }
                    },
                    "required": ["words"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "catalogue",
                "description":
                    "Everything this installation names, searched by name AND by what the thing \
                     does. For the kinds without a tool of their own: ashes of war, consumables \
                     and crafting items, weapon skills.\n\
                     \n\
                     Weapons, armour, spells and talismans have their own tools with real \
                     figures — use those for those. Come here for anything else, and come here \
                     before naming one: asked which ashes of war cause bleed, a model that did \
                     not went to a wiki and offered English names it had guessed the meaning \
                     of, while 253 ashes sat named and described in this game's own text.\n\
                     \n\
                     Search by a word for the effect — an ailment, a stat, a kind of damage — \
                     in the language their game is in. Endings do not matter. Narrow it with a \
                     kind when you know which you want.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            // Built from the list the shelf itself is built
                            // from, so a kind that is offered here is one that
                            // exists. Spelling one that does not searched
                            // nothing and read as an answer.
                            "description": format!(
                                "One of exactly these: {}. Nothing else is a kind — anything \
                                 without a tool of its own, spirit summons included, is an \
                                 \"item\". Leave it out to search everything.",
                                crate::library::kinds()
                            )
                        },
                        "search": {
                            "type": "string",
                            "description":
                                "A word from the name or from what it does. Leave out to list \
                                 the whole kind."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "upgrade_path",
                "description":
                    "What upgrading a weapon actually costs in THIS installation: every level, \
                     the material each one wants and how many, under the names their own game \
                     prints.\n\
                     \n\
                     Call it for any question about getting a weapon to +anything, about \
                     smithing stones, about what a smith needs. There is no other source: a \
                     total conversion rewrites the materials and the levels, and the base \
                     game's are not theirs. Asked how to reach +10 without this, a model \
                     invented the material every single time — \"Somerset Stone\", \
                     \"Somingesite Stones\", a blacksmith who does not exist — and a player \
                     goes looking for those.\n\
                     \n\
                     Give the weapon's name, or leave it out for whatever they are holding. It \
                     says how far the weapon goes, too: a path that stops at ten is a weapon \
                     that stops at ten, and that is the answer to \"can I take this further\".",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "The weapon, as their game names it. Leave out for what they hold."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "whos_here",
                "description":
                    "The named things standing on a map — bosses, NPCs, anything the game has a \
                     name for — with how many runes each gives, WHAT KIND OF DAMAGE HURTS IT and \
                     by how much, and where the tables hold it, how much health. Read out of the \
                     map files and the installation's own tables, whether the maps are loose or \
                     still packed in the game's own archives. GIVE IT A BOSS'S NAME and it \
                     searches the whole world for it — you do NOT need a map id, and this is \
                     the ONLY place a boss's name and its rune reward sit together, so a rune \
                     figure quoted without calling this is one you made up. Asked what the Red \
                     Wolf of Radagon gives, a model read a wiki and reported the BASE game's \
                     reward as theirs.\n\
                     \n\
                     The damage figures make this the answer to \"what is this boss weak to\" \
                     and \"what should I bring\". Nothing else here reads them and a wiki's are \
                     the base game's, which a total conversion rewrites.\n\
                     \n\
                     Call it with no argument for the map they are standing on: that is the \
                     answer to \"what boss is here\", \"what is nearby\" and \"what is this \
                     fight worth\". Give a map id like m60_35_44_00 to ask about another.\n\
                     \n\
                     Or GIVE IT A NAME — anything that is not a map id is searched for across \
                     the whole world, and what comes back says which map each one stands on. \
                     That is how to answer \"where is X\" and \"what should I use against X\" \
                     when nobody has said which map. Told to find a boss it had no map id for, \
                     a model went to a wiki instead and came back about a different creature; \
                     the name was right here. Use the name their game prints, or part of it.\n\
                     \n\
                     It is the ONLY source for a boss's name and its reward together. Nothing \
                     else in this launcher has them: the area table carries runes and a \
                     position and no name at all, so a rune figure quoted without this is one \
                     you made up. Richest first, so the top of the list is usually the boss and \
                     the rest are the named people standing around it — there is no flag that \
                     says which is which, so do not claim one is a boss beyond that.\n\
                     \n\
                     It answers one boss or one map, never the whole game at once: \"which boss \
                     gives the most runes\" cannot be answered, because the table that holds \
                     every reward holds no names to rank. Say that plainly and offer to look up \
                     a boss they name — do NOT reach for the base game's answer and pass it off \
                     as this installation's.\n\
                     \n\
                     For what can be GOT on a map rather than who is on it, use \
                     what_drops_here. Named things almost never drop anything — a boss's reward \
                     is scripted rather than rolled — so an empty drop line here is not the \
                     answer to \"what does this give\".",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "map": {
                            "type": "string",
                            "description":
                                "A NAME or a map id, despite what this argument is called. A \
                                 boss's or an NPC's name — part of it is enough, as their game \
                                 prints it — is searched for across the whole world and comes \
                                 back saying which map it stands on: that is how to answer \
                                 \"what does X give\" and \"what hurts X\" with no map id in \
                                 hand. A map id like m60_35_44_00 asks about that map, and \
                                 left out it is the map they are standing on."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "what_drops_here",
                "description":
                    "What the things standing on a map give, and at what odds, out of the \
                     installation's own drop tables. The answer to \"what can I farm here\", \
                     \"where do I get X on this map\" and \"is it worth clearing this\".\n\
                     \n\
                     A different question from whos_here, and the tables answer them \
                     differently: the named things almost never drop anything, because a boss's \
                     reward is scripted rather than rolled. The nameless soldiers and beasts \
                     around them carry every drop there is.\n\
                     \n\
                     Ordered by how much a clear actually yields, so the top of the list IS the \
                     answer to \"what falls most often\". Each line also carries the best odds \
                     any ONE thing gives it at and how many things there give it — a 3% off \
                     forty of them is a farm, a 3% off one is not. All of it comes from the \
                     table's own weights and is worked out already: never add the figures up, \
                     never restate them as \"often\".\n\
                     \n\
                     Call it with no argument for where they are standing. A wiki's drop list is \
                     the base game's and a total conversion rewrites them, so this is the only \
                     right answer for their installation.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "map": {
                            "type": "string",
                            "description":
                                "A map id like m60_35_44_00. Leave out for where they are."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "talismans",
                "description":
                    "Every talisman this installation actually has, under the names their own \
                     game prints, with what each one weighs.\n\
                     \n\
                     Call it before naming a talisman, always. There is no other way to know \
                     what this game calls them: a total conversion renames and replaces them \
                     wholesale, and the base game's names are not theirs. Asked which talismans \
                     would suit them, a model that did not call this offered four that exist in \
                     no version of the game at all, and a player will go looking for them.\n\
                     \n\
                     What a talisman DOES is not in here — that lives in its own description, \
                     which game_item reads. So a recommendation goes: take the list from here, \
                     pick the names that look right, then read each one with game_item before \
                     saying what it is for. Never guess an effect from a name.\n\
                     \n\
                     ALWAYS pass a word — a stat, an ailment, a kind of damage, a mechanic, in \
                     the language their game is in. It matches what each talisman does as well \
                     as what it is called, and endings do not matter. Called with no word it \
                     returns only how many there are, on purpose: a page of names with no \
                     effects is not something to draw a conclusion from, and a model that got \
                     one concluded from it that the game had no talisman for faith. Search \
                     twice with different words before saying there is none.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "Part of a name, to narrow the list. Leave it out for all of them."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "spells_by_cost",
                "description":
                    // Two paragraphs on purpose. `tools_in_brief` sends only the
                    // FIRST, so what the tool does goes above the blank line and
                    // the case law below it — where it stays in the source for
                    // whoever maintains this and never crosses the wire.
                    //
                    // Written as one blob to begin with, which defeated the
                    // trimming entirely: measured at 1,003 characters sent
                    // against 1,003 whole, the third heaviest tool in the list.
                    "Every spell in this installation ranked by what it costs in FP, both ends \
                     of the list, with what each asks for. Call it for \"which spell is \
                     cheapest\", \"the most expensive incantation\", \"a cheap one for my faith \
                     build\". Pass \"sorcery\" or \"incantation\" in `school` to narrow it, or \
                     part of a name.\n\
                     \n\
                     WITHOUT this there was no way to rank spells at all, and the result was not \
                     a refusal but a falsehood: asked in Spanish for the cheapest faith spell, an \
                     answer said it could not read the spell table and told the player to open \
                     the game. Both halves were untrue — the tables read fine with the game shut. \
                     Both ends are printed because showing only the cheapest produced \"the \
                     dearest incantation is Династическое таинство, 0 FP\".",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "school": { "type": "string", "description":
                            "\"sorcery\" for the ones that ask for intelligence, \"incantation\" \
                             for the ones that ask for faith. Left out, both." },
                        "name": { "type": "string", "description":
                            "Part of a name, to narrow it. In the game's own language." }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "starting_classes",
                "description":
                    // First paragraph only is sent — see `spells_by_cost` for
                    // why this is split rather than run together.
                    "The classes THIS installation starts you as, with the level each begins at \
                     and all eight attributes, read from its own tables. Call it for anything \
                     about picking a class, starting stats, or which start suits a build — \
                     \"which class is strongest\", \"what should I start as for faith\", \"what \
                     level does the Wretch begin at\".\n\
                     \n\
                     A total conversion is free to rebalance and renumber every one of them, so \
                     memory is about a different game: asked which start was strongest, an answer \
                     said they are all about equal \"in the base game\" and named two, having \
                     called nothing at all.",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "ashes_of_war",
                "description":
                    // First paragraph only is sent. The count is gone from it
                    // deliberately: the listing prints the real number, and a
                    // hardcoded one is a figure that can drift into an answer.
                    "Every ash of war in this installation with WHAT ITS SKILL COSTS IN FP, by \
                     button, under the names their own game prints. Call it for any question \
                     about an ash, a weapon art, or what a skill costs: \"which ash is \
                     cheapest\", \"what does this one cost\", \"a cheap skill for my build\". \
                     Pass part of a name to narrow it.\n\
                     \n\
                     An ash is NOT a weapon and not an item, so gear_numbers and game_item will \
                     both miss it — asked which ash was cheapest, a model looked five of them up \
                     in the weapon table, got nothing five times, and answered from memory.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "Part of an ash's name, to narrow the list. Leave it out for \
                                 all of them."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "physick_tears",
                "description":
                    "Every crystal tear that can go into the Flask of Wondrous Physick in this \
                     installation — 60 of them, under the names their own game prints, and for \
                     about half of them WHAT THE TEAR ACTUALLY DOES as a figure: dexterity +10, \
                     max FP +10%, equip load +350%. Call it for any question about the physick, \
                     what to mix, or how to get a stat up for one fight. Asked twice what could \
                     go in the flask, the launcher found nothing at all and said so, while all \
                     of this was sitting readable. Where a tear has no figure the tables carry \
                     none for it: say so rather than supplying one, and never name a tear that \
                     is not in this list.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "Part of a name or of a stat, to narrow the list. Leave it out \
                                 for all of them."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "spirit_ashes",
                "description":
                    "Every spirit ash this installation has, under the names their own game \
                     prints, and for each one WHETHER IT UPGRADES, WHAT THAT COSTS IN RUNES and \
                     WHICH MATERIAL IT CONSUMES.\n\
                     \n\
                     So call it for upgrading as well as for naming — \"how do I upgrade a \
                     spirit ash\", \"what does levelling one need\", \"which material\". Asked \
                     exactly that, a model searched the game's prose instead, found the words \
                     \"Пепел Войны\" — ashes of WAR, a different thing entirely — and guessed \
                     they might be related. The answer was in this tool and it was never \
                     called.\n\
                     \n\
                     Call it before naming a summon, always. Asked which spirit ashes there \
                     were and which was best, a model called nothing that worked and then \
                     listed five out of memory of the BASE game. A total conversion adds and \
                     removes them, so a name that is not in this list is a name the player \
                     cannot use.\n\
                     \n\
                     WHAT IS NOT IN HERE, and must not be supplied from memory: how strong a \
                     summon is, what it costs in FP, its health, or what it does. The cost \
                     field reads -1 on every ash in this installation — the conversion took the \
                     figure out of it — and a summon's own strength is not read at all. So \
                     \"which is best\" cannot be answered from the tables: say so, give the \
                     names, and search the wiki for opinion if they want one. What IS here is \
                     the name, whether it upgrades, and what upgrading costs.\n\
                     \n\
                     Pass a word to narrow it — part of a name, in the language their game is \
                     in. Endings do not matter.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "Part of a name, to narrow the list. Leave it out for all of them."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "search_web",
                "description":
                    "Search the open web. The right tool whenever the answer is newer than a \
                     mirror or was never written down as a wiki page: a patch note, a release, a \
                     build somebody posted, a mod too new to have a wiki, an argument about \
                     whether something is worth doing.\n\
                     \n\
                     Prefer the wikis for anything they hold, because a wiki page is written by \
                     people who play the game and a search result is whatever ranked. But when \
                     they come back with nothing, search here BEFORE answering from memory — an \
                     answer that begins \"I could not find it, but…\" with this tool untouched is \
                     the one mistake it exists to prevent.\n\
                     \n\
                     Returns titles, addresses and the engine's own summaries. It does not open \
                     the pages: pass an address to read_page for that, and do, because a summary \
                     is one sentence and the answer usually is not.\n\
                     \n\
                     Two things to hold onto. This leaves the player's machine, so it is one of \
                     the two tools that send anything anywhere. And a result is a claim by a \
                     stranger: say where something came from when you use it, and never let it \
                     outrank the game's own tables.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description":
                                "What to search for. Include the game's name — 'Elden Ring' or \
                                 the mod's — since half these words mean something else on their \
                                 own."
                        }
                    },
                    "required": ["query"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "map_markers",
                "description":
                    "What the player has pinned on their own map, and taking a pin off.\n\
                     \n\
                     Reading is free — use it whenever they ask what they have marked, or before \
                     pinning something so you can say whether it is already there. Each pin comes \
                     back with the nearest place the game has a name for, since a pair of \
                     coordinates means nothing to anybody.\n\
                     \n\
                     Give `remove` an id to take one off, or the word `all` to clear the map. \
                     That part writes, so only on a plain request, and the game has to be closed \
                     for it exactly as for placing one. Clearing a map cannot be undone from in \
                     here — the save is snapshotted first, but say what you are about to do \
                     before you do it.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "character": {
                            "type": "string",
                            "description":
                                "Whose map. Pass it whenever the player has named a character, \
                                 even in passing. Leave it out only when they have not, and if \
                                 the save turns out to hold several, what comes back names them \
                                 so you can ask."
                        },
                        "remove": {
                            "type": "string",
                            "description":
                                "A marker's id, or `all`. Leave it out to only read the map."
                        }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "place_marker",
                "description":
                    "Put a pin on the player's own map, at a place named in the game they have \
                     installed. It appears where they would have put it by hand, and they can \
                     remove it the same way.\n\
                     \n\
                     This is the one tool that changes something. Everything else reads. So: only \
                     when they have asked for a marker, one place at a time, and say afterwards \
                     what was pinned and where. Do not pin a place to illustrate an answer, and \
                     do not pin several because a route has several stops unless they asked for \
                     the route to be marked.\n\
                     \n\
                     The game has to be shut down to the desktop — it keeps the map in memory \
                     while it runs and would write over this. Going back to the main menu does \
                     NOT count and telling them it does wastes their time; the process has to be \
                     gone. If it is open, say so and offer to do it once they have quit. The save \
                     is backed up first, every time.\n\
                     \n\
                     Name the place the way their game names it, in their language, and check the \
                     name with a tool first rather than translating it yourself. Legacy dungeons \
                     — Stormveil, Leyndell, Raya Lucaria — are drawn on their own maps and cannot \
                     be pinned on the overworld; somewhere just outside can.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "place": {
                            "type": "string",
                            "description":
                                "The place to pin, as the game names it. Part of the name is \
                                 enough."
                        },
                        "character": {
                            "type": "string",
                            "description":
                                "Whose map, when the save holds more than one character. Leave it \
                                 out first: if it matters, what comes back names them and you can \
                                 ask which."
                        }
                    },
                    "required": ["place"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_page",
                "description":
                    "Open one web page and read it. The other half of search_web: that tool finds \
                     the mod's own changelog, this one is how you find out what is in it.\n\
                     \n\
                     Only for addresses a search already returned — do not invent a URL, since a \
                     guessed address either fails or lands somewhere with nothing to do with the \
                     question. Give the topic as well as the address and the part of the page \
                     about that topic comes back rather than the top of it, which on a long \
                     changelog is the difference between the answer and a menu.\n\
                     \n\
                     What comes back is a stranger's page, not a wiki and not the game: quote it \
                     as what a page said, name it, and let the tables win over it every time.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The address, exactly as search_web gave it."
                        },
                        "about": {
                            "type": "string",
                            "description":
                                "What you are looking for on the page, a few words. Used to pick \
                                 which part of a long page to return."
                        }
                    },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "gear_numbers",
                "description":
                    "The damage, scaling and requirements of a weapon, read out of the tables the \
                     player's own installation balances itself with. These are the figures the \
                     game is really using, and under a total conversion they are nothing like \
                     the wiki's: The Convergence gives Reduvia no physical damage at all, 82 \
                     fire, and faith scaling, where every wiki prints 79 physical and no faith.\n\
                     \n\
                     Use it before quoting any weapon figure, in preference to item_stats and to \
                     the wiki — both of those are the base game and are simply wrong here.\n\
                     \n\
                     It also names the SKILL on a weapon — the ash of war, or the art it cannot \
                     be parted from — and what pressing it costs in FP. That is the only place \
                     either is readable: a wiki says which ash a weapon shipped with in the base \
                     game, which a total conversion has moved.\n\
                     \n\
                     Armour too: give it a piece's name and it answers with that piece's weight, \
                     its damage negation as the menu shows it, and its resistances. Name a SET \
                     and it answers with every piece of it and what they weigh together, already \
                     added up. Ask it rather than looking a set up anywhere else.\n\
                     \n\
                     Call it with NO arguments to get whatever they are holding, which is what \
                     \"my weapon\" means. Give a name only to ask about something else.\n\
                     \n\
                     With the game open it also gives the figure on their own stat screen — the \
                     weapon upgraded, plus what their attributes add. Use that one when talking \
                     about their weapon; the base is what it is worth to somebody else.\n\
                     \n\
                     And it answers \"what would more of an attribute do\". Pass the attribute \
                     set to the value they are asking about and it works the damage out at that \
                     value, exactly, off the game's own scaling curves. Pass ONLY what is \
                     changing — every attribute you leave out keeps the value they have now. \
                     Watch the arithmetic: ten more faith on twenty-two is thirty-two, not \
                     twenty-three. This is the one way to answer such a question; never \
                     estimate it.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description":
                                "A weapon other than the one in their hands, named as the game \
                                 names it; part of the name is enough. Leave it out for what \
                                 they are holding."
                        },
                        "armour_against": {
                            "type": "string",
                            "description":
                                "Rank EVERY piece of armour in the game — 913 of them — by any \
                                 ONE of these five families, whichever the player named, in any \
                                 language: (0) WEIGHT, for \"what is the heaviest armour\" and \
                                 \"what is the lightest\" — pass \"weight\" and it comes back \
                                 heaviest first per slot; (1) a DAMAGE KIND it stops — physical, \
                                 slash, \
                                 strike, pierce, magic, fire, lightning, holy; (2) POISE, which \
                                 is not damage but is what heavy armour is worn for, so pass it \
                                 for anything about being staggered and never turn that into \
                                 \"physical\"; (3) one of the four the equipment screen shows — \
                                 IMMUNITY, ROBUSTNESS, FOCUS, VITALITY — which are how long an \
                                 ailment bar takes to fill, and note that ROBUSTNESS IS NOT \
                                 POISE, it is bleed and frost; (4) an ATTRIBUTE the armour \
                                 GRANTS — faith, strength, arcane, intelligence, dexterity, \
                                 endurance, vigor, mind — because armour here really does give \
                                 attributes and 836 of the 913 carry an effect. Use it for \
                                 \"what should I wear against X\" and \"what should a faith \
                                 build wear\" alike. Nothing else is needed alongside it.\n\
                                 \n\
                                 \"poise\" is accepted here too. It is not damage — it is what \
                                 heavy armour is worn for — and it ranks the same way. Pass it \
                                 for any question about holding up or not being staggered, and \
                                 never turn such a question into \"physical\", which one answer \
                                 did before labelling the defence percentages as poise.\n\
                                 \n\
                                 So is an ATTRIBUTE — faith, strength, arcane, intelligence, \
                                 dexterity, endurance, vigor, mind. Armour in this installation \
                                 GRANTS attributes: 836 of its 841 pieces carry an effect, and \
                                 this is how to ask which ones. Use it for any \"what should a \
                                 faith build wear\" question — asked in French for the lightest \
                                 armour for a faith build, a model ranked by physical negation \
                                 and answered without reference to faith at all, because there \
                                 was no way to ask. Only pieces that actually grant it come \
                                 back, so the list is short and every row on it counts.\n\
                                 \n\
                                 So are the four the equipment screen shows: IMMUNITY (poison \
                                 and rot), ROBUSTNESS (bleed and frost), FOCUS (sleep and \
                                 madness) and VITALITY (death blight). Pass whichever the \
                                 player named, in any language — Robustheit, Живучесть — or the \
                                 ailment itself, and it will be matched. These are not damage \
                                 and are never a percentage.\n\
                                 \n\
                                 ROBUSTNESS IS NOT POISE. It is the bar that fills before you \
                                 bleed or freeze; poise is whether a hit staggers you. Asked in \
                                 German for the helm with the most Robustheit, a model decided \
                                 that meant poise, passed \"poise\", and answered a question \
                                 about bleed and frost with a number about stagger. German \
                                 poise is HALTUNG. Pass the player's own word and let it be \
                                 matched here rather than translating it yourself."
                        },
                        "weapons_building": {
                            "type": "string",
                            "description":
                                "Rank EVERY weapon in the game by how much of one ailment it \
                                 builds up per hit: poison, rot, bleed, curse, frost, sleep or \
                                 madness. Use it for \"what should I use for bleed\" and \
                                 anything like it.\n\
                                 \n\
                                 It exists because the alternative was searching names and \
                                 checking hits one at a time — seventeen calls and fifty-three \
                                 seconds for one question, and only working at all because this \
                                 conversion happens to put the word in the names. All SEVEN are \
                                 read here — frost, sleep and madness used to be unreadable and \
                                 no longer are."
                        },
                        "armour_set": {
                            "type": "string",
                            "description":
                                "A whole armour SET, by part of any one piece's name as their \
                                 own game prints it. Gives every piece the set has HERE, each \
                                 one's weight, poise, negation and resistances, and the totals \
                                 added up in code — plus how the set sits against what they can \
                                 carry.\n\
                                 \n\
                                 Use it for \"what does the X set weigh\", \"is X worth \
                                 wearing\", \"what does X protect against\". Asked what the \
                                 bandit set weighed, a model searched a WIKI, looked one piece \
                                 up, and answered with four pieces totalling 11.8. This \
                                 installation's bandit set has THREE pieces and weighs 8.2 — \
                                 there is no bandit mask here at all, and the three weights it \
                                 gave for the pieces that do exist were wrong too. A set is not \
                                 always four pieces and a wiki is about a different game." },
                        "weapons_of_sort": {
                            "type": "string",
                            "description":
                                "List a whole CLASS of weapon, ranked. Give the class in \
                                 English — greatshield, medium shield, small shield, katana, \
                                 great katana, dagger, straight sword, greatsword, colossal \
                                 sword, curved sword, curved greatsword, twinblade, thrusting \
                                 sword, heavy thrusting sword, axe, greataxe, hammer, great \
                                 hammer, flail, spear, great spear, halberd, reaper, fist, \
                                 claw, whip, colossal weapon, bow, greatbow, crossbow, \
                                 ballista, glintstone staff, sacred seal, torch, perfume \
                                 bottle, thrusting shield, throwing blade, backhand blade, \
                                 light greatsword, beast claw, hand-to-hand. \"shield\" on its \
                                 own means all four shield classes; \"sword\" means all the \
                                 sword classes; and \"weapon\" means EVERY weapon class at \
                                 once, which is what a question about a BUILD wants.\n\
                                 \n\
                                 Ask for \"weapon\" once rather than naming the classes one at \
                                 a time: a question about a dexterity build did exactly that, \
                                 twenty-eight calls, and ran out of lanes before it could \
                                 answer.\n\
                                 \n\
                                 Use it for ANY \"which X should I use\" where X is a class \
                                 rather than a named weapon. Asked which greatshield holds \
                                 magic best, a model reached for armour_against instead and \
                                 answered with a HELMET. Asked in English which shield suits a \
                                 faith build, another searched the name catalogue, found \
                                 nothing — the names in this installation are Russian — and \
                                 listed four shields from memory instead. Both were one call \
                                 of this away." },
                        "sorted_by": {
                            "type": "string",
                            "description":
                                "What to rank the class on, alongside weapons_of_sort. A \
                                 damage kind (physical, magic, fire, lightning, holy) — which \
                                 means the BLOCK percentage for shields and the damage for \
                                 everything else; \"guard\" for guard boost, which is what \
                                 decides whether a block staggers; \"weight\" for the lightest \
                                 first and \"heaviest\" for the heaviest first; or poison, rot, \
                                 bleed or curse. Left out, shields rank by physical block and \
                                 everything else by total damage." },
                        "strength": { "type": "integer", "description":
                            "Work the damage out at this strength instead of theirs." },
                        "dexterity": { "type": "integer", "description": "The same for dexterity." },
                        "intelligence": { "type": "integer", "description": "The same for intelligence." },
                        "faith": { "type": "integer", "description": "The same for faith." },
                        "arcane": { "type": "integer", "description": "The same for arcane." }
                    },
                    "required": []
                }
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
    /// Why the frame rate is what it is, when the launcher can say.
    pub frames: Option<String>,
    /// How much of each thing this installation's tables hold.
    pub holdings: Option<String>,
    /// Where the anti-cheat stands, which decides whether it is safe to play.
    pub safety: Option<String>,
    /// The snapshots the launcher itself is holding.
    pub backups: Option<String>,
    /// Which wikis are mirrored onto this machine, and how much of the game's
    pub mirrors: Option<String>,
    /// Which of the games it manages this player has actually pointed it at.
    pub set_up: Option<String>,
    /// The named things standing on a map, richest first.
    pub dwellers: Standing,
    /// What everything on a map gives, best odds first.
    pub haul: Farming,
    /// One weapon, by table id, with nothing about the running game read.
    pub weigh: Box<dyn Fn(i64) -> Option<crate::formats::regulation::Weapon> + Send + Sync>,
    /// The skill on one weapon, by table id.
    pub skill_on: Box<dyn Fn(i64) -> Option<Skill> + Send + Sync>,
    /// Anything the installation names, searched by name and by what it does.
    pub catalogue_of: Shelf,
    /// True when Seamless Co-op is installed.
    pub seamless: bool,
    /// The language the game is set to.
    pub language: Option<String>,
    /// The character as the running game has them, read on demand.
    pub live: Option<Box<dyn Fn() -> Option<crate::live::Live> + Send + Sync>>,
    /// A weapon's real figures out of the installed tables, asked by name.
    pub weapon: Lookup<Armed>,
    /// The same for a piece of armour, asked by name.
    pub armour: Lookup<Dressed>,
    /// Every piece ranked by one kind of damage resisted.
    pub armoury: Ranked,
    /// Every weapon that builds up one ailment, most first. See [`Arsenal`].
    pub arsenal: Arsenal,
    /// Every weapon of one class, ranked.
    pub armed: Armoury,
    /// How far weapons upgrade: the ceiling, and the step count.
    pub upgrades_to: Vec<(u8, usize)>,
    /// The game's own word for each attribute.
    pub attribute_words: Vec<(String, String)>,
    /// A whole armour set, totalled in code. See [`Suited`].
    pub suit: Suit,
    /// The question as the player typed it, before any model touched it.
    pub asked: String,
    /// What one talisman does, in figures, by its id.
    pub charm: Box<dyn Fn(u32) -> Option<crate::formats::regulation::Charm> + Send + Sync>,
    /// A sorcery or incantation, asked by name.
    pub spell: Lookup<Cast>,
    /// Which spells a set of attributes opens — `(intelligence, faith, arcane)`
    pub spells_at: Box<dyn Fn(u8, u8, u8) -> Vec<Cast> + Send + Sync>,
    /// Every ash of war, with the FP its skill costs. See `regulation::gem`.
    pub ashes: Box<dyn Fn() -> Vec<WarAsh> + Send + Sync>,
    /// The ten classes the game starts you as. See `regulation::class`.
    pub classes: Box<dyn Fn() -> Vec<StartingClass> + Send + Sync>,
    /// Every crystal tear that goes in the wondrous physick, with its figures.
    pub tears: Box<dyn Fn() -> Vec<Tear> + Send + Sync>,
    /// Every spirit ash this installation has, with what is readable of it.
    pub spirits: Box<dyn Fn() -> Vec<Ash> + Send + Sync>,
    /// Every talisman this installation has: the name their game prints, what
    pub talismans: Box<dyn Fn() -> Vec<Charm> + Send + Sync>,
    /// What upgrading a weapon costs, step by step, named the way their game
    pub upgrading: Climbing,
    /// The same weapon at different attributes.
    pub what_if: WhatIf,
    /// The named places closest to a point on the map, nearest first.
    pub nearby: Nearby,
    /// The running game's own catalogue, asked by name.
    pub catalogue: Lookup<Catalogued>,
    /// Everything the game has written, searched.
    pub written: Reading,
    /// The most they can carry at a given endurance.
    pub load: Box<dyn Fn(u32) -> Option<f32> + Send + Sync>,
    /// What they are carrying now and the most they could.
    pub carrying: Box<dyn Fn() -> Option<(f32, f32)> + Send + Sync>,
    /// Puts a pin on the player's map, or says why it could not.
    pub mark: Marker,
    /// What is pinned, and taking a pin off. `(character, what to remove)`.
    pub pins: Pins,
}

/// Something the launcher can be asked about by name.
pub type Lookup<T> = Box<dyn Fn(&str) -> Option<Vec<T>> + Send + Sync>;

/// Searching everything the game has written down.
pub type Reading = Box<dyn Fn(&str) -> (Vec<crate::text::Written>, Option<String>) + Send + Sync>;

/// Pinning a place, by name, on a named character's map.
pub type Marker =
    Box<dyn Fn(&str, Option<&str>) -> std::result::Result<String, String> + Send + Sync>;

/// The named places closest to a point on the map, and how far each one is.
pub type Nearby = Box<dyn Fn(f32, f32) -> Vec<(String, f64)> + Send + Sync>;

/// What is standing on a map, richest first. An empty name means their own.
pub type Standing = Box<dyn Fn(&str) -> Vec<crate::bestiary::Dweller> + Send + Sync>;

/// What a map's inhabitants give, best odds first. An empty name means theirs.
pub type Farming = Box<dyn Fn(&str) -> Vec<crate::bestiary::Haul> + Send + Sync>;

/// Everything the installation names, narrowed by kind and searched by word.
pub type Shelf = Box<dyn Fn(&str, &str) -> Vec<Named> + Send + Sync>;

/// The same weapon at attributes they do not have yet, in the STR, DEX, INT,
pub type WhatIf = Box<dyn Fn(&str, [Option<u32>; 5]) -> Vec<Armed> + Send + Sync>;

/// A weapon's upgrade path, asked by name. Empty name means what they hold.
pub type Climbing = Box<dyn Fn(&str) -> Vec<Climb> + Send + Sync>;

/// Reading the map, and clearing it. Given nothing to remove it only reads.
pub type Pins = Box<
    dyn Fn(Option<&str>, Option<&str>) -> std::result::Result<String, String> + Send + Sync,
>;

/// Where a standing character is on the world map.
fn on_the_map(place: &crate::live::Place) -> Option<(f32, f32)> {
    let mut parts = place.map.split('_');
    let area = parts.next()?.trim_start_matches('m');
    if area != "60" {
        return None;
    }
    let across: u8 = parts.next()?.parse().ok()?;
    let down: u8 = parts.next()?.parse().ok()?;
    Some(crate::markers::from_world(across, down, place.x, place.z))
}

/// How many spells are named before the rest are counted instead.
const SPELLS_LISTED: usize = 12;

/// A handful of spells, hardest first, and a count for the rest.
fn listing(spells: &[&Cast]) -> String {
    let mut out = String::new();
    for cast in spells.iter().take(SPELLS_LISTED) {
        let needs: Vec<String> = cast
            .spell
            .needs
            .iter()
            .map(|(what, value)| format!("{what} {value}"))
            .collect();
        out.push_str(&format!(
            "  {} — {} FP, needs {}\n",
            cast.name,
            cast.spell.fp,
            needs.join(", ")
        ));
    }
    if spells.len() > SPELLS_LISTED {
        out.push_str(&format!(
            "  …and {} more, which are easier than these\n",
            spells.len() - SPELLS_LISTED
        ));
    }
    out
}

/// Whether they can hold the thing, decided here rather than in a sentence.
fn short_of(needs: &[(String, u8)], theirs: Option<&[(String, u32)]>) -> String {
    let Some(theirs) = theirs else {
        return String::new();
    };
    if needs.is_empty() {
        return String::new();
    }

    // The names match because both sides are written the same way — "faith
    // (FTH)" — and the short form is what makes that safe across languages.
    let held = |what: &str| -> Option<u32> {
        theirs
            .iter()
            .find(|(mine, _)| mine.eq_ignore_ascii_case(what))
            .map(|(_, value)| *value)
    };

    let missing: Vec<String> = needs
        .iter()
        .filter_map(|(what, wanted)| {
            let have = held(what)?;
            (have < u32::from(*wanted)).then(|| format!("{what} {have} of {wanted}"))
        })
        .collect();

    if missing.is_empty() {
        "  They meet every requirement for this one.\n".to_string()
    } else {
        format!(
            "  THEY CANNOT USE THIS PROPERLY: short on {}. Say so plainly — wielding it under \
             the requirement costs most of the damage.\n",
            missing.join(", ")
        )
    }
}

/// A sorcery or incantation found in the installed tables.
#[derive(Debug, Clone)]
pub struct Cast {
    pub name: String,
    pub spell: crate::formats::regulation::Spell,
    pub modded: bool,
}

/// One weapon's whole upgrade path, materials named.
#[derive(Debug, Clone)]
pub struct Climb {
    pub weapon: String,
    /// Level reached, and what that step costs: material and how many.
    pub steps: Vec<(u8, Vec<(String, i8)>)>,
    pub modded: bool,
}

/// One thing the installation names, whatever kind it is.
#[derive(Debug, Clone)]
pub struct Named {
    /// "ash of war", "item", "skill", "talisman", "weapon", "armour".
    pub what: String,
    pub name: String,
    /// The line the menu prints for what it does, when there is one.
    pub effect: Option<String>,
}

/// A talisman found in the installed tables.
#[derive(Debug, Clone)]
pub struct WarAsh {
    /// The skill it grants, as the game names it — which is what a player
    pub name: String,
    /// What a press costs, by button.
    pub costs: Vec<(String, u16)>,
}

/// A class the game will start you as, named and with its figures.
#[derive(Debug, Clone)]
pub struct StartingClass {
    pub name: String,
    pub level: i16,
    /// The eight, in the order the character screen lists them.
    pub attributes: Vec<(String, u8)>,
    /// What it starts holding, as names where they could be resolved.
    pub gear: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Tear {
    pub name: String,
    /// What the game says it does, where there is a line for it.
    pub effect: Option<String>,
    /// What it actually does, read off the effect it applies.
    pub gives: Vec<(String, i32)>,
    pub changes: Vec<(String, f32)>,
    pub adds: Vec<(String, i32)>,
}

#[derive(Debug, Clone)]
pub struct Ash {
    pub name: String,
    /// What the game says it does, where the text tables have a line for it.
    pub effect: Option<String>,
    /// What upgrading it consumes, by name.
    pub material: Option<String>,
    pub summon: crate::formats::regulation::Summon,
}

#[derive(Debug, Clone)]
pub struct Charm {
    pub name: String,
    pub weight: f32,
    /// The game's own one-line summary of what it does.
    pub effect: Option<String>,
    /// The same thing in figures, out of the effect it applies. `None` where
    pub figures: Option<crate::formats::regulation::Charm>,
}

/// One piece, ranked: where it is worn, its name, its figure.
pub type Shielding = (&'static str, String, f32, f32);

/// Every piece against one kind of damage, best first.
pub type Ranked = Box<dyn Fn(&str) -> Vec<Shielding> + Send + Sync>;

/// Every weapon that builds up one ailment, most first: what it is called, how
pub type Arsenal = Box<dyn Fn(&str) -> Vec<(String, i32, f32, String)> + Send + Sync>;

/// One weapon in a listing of its own class.
#[derive(Debug, Clone)]
pub struct OfSort {
    pub name: String,
    /// Which class this one is, for the answers that span several — "shield"
    pub sort: String,
    pub weight: f32,
    /// What it asks for before it can be held at all.
    pub needs: Vec<(String, u8)>,
    /// The figure it was ranked on, and what that figure is.
    pub figure: f32,
    /// The five block percentages, for the classes carried to block.
    pub blocks: Vec<(String, f32)>,
    pub boost: Option<i16>,
    /// Base damage before scaling, added across the kinds it deals.
    pub damage: u16,
    pub ailments: Vec<(String, i32)>,
}

/// Every weapon of one class, ranked by one figure.
#[derive(Debug, Clone)]
pub struct Sorted {
    /// What the class is called in the player's own game, and in English.
    pub called: String,
    pub english: String,
    /// What it was ranked on, in words.
    pub by: String,
    /// How many are in the class altogether, against however many are shown.
    pub all: usize,
    pub best: Vec<OfSort>,
}

/// A whole armour set, totalled in code.
#[derive(Debug, Clone)]
pub struct Suited {
    /// What the set is called, taken from the longest run of words its pieces
    pub called: String,
    pub pieces: Vec<crate::formats::regulation::Armour>,
    /// The piece names, in the same order.
    pub names: Vec<String>,
    /// Added up here rather than in a sentence.
    pub weight: f32,
    pub poise: f32,
    /// What they can carry, when it can be read, so "will it fit" is answerable
    pub carrying: Option<(f32, f32)>,
}

pub type Suit = Box<dyn Fn(&str) -> Option<Suited> + Send + Sync>;

/// Every weapon of a named class — greatshields, katanas — ranked.
pub type Armoury = Box<dyn Fn(&str, &str) -> Option<Sorted> + Send + Sync>;

/// A piece of armour found in the installed tables.
#[derive(Debug, Clone)]
pub struct Dressed {
    pub name: String,
    pub armour: crate::formats::regulation::Armour,
    pub modded: bool,
}

/// A weapon's skill, as the player's own menu names it, and what one press
pub type Skill = (String, Vec<(String, u16)>);

/// A weapon found in the installed tables.
#[derive(Debug, Clone)]
pub struct Armed {
    /// What the game calls it, which is what the player sees.
    pub name: String,
    pub weapon: crate::formats::regulation::Weapon,
    /// What it hits for in THEIR hands: the base after upgrading, and what
    pub hits: Vec<crate::formats::regulation::Damage>,
    /// The same weapon at the attributes they have now.
    pub now: Vec<crate::formats::regulation::Damage>,
    /// The skill on it, named, and what a press costs in FP.
    pub skill: Option<Skill>,
    /// True when the figures came from a total conversion's tables.
    pub modded: bool,
}

/// One entry as the running game describes it.
#[derive(Debug, Clone)]
pub struct Catalogued {
    pub name: String,
    /// Its row, so a figure keyed on the id can be joined to it.
    pub id: u32,
    /// Weapon, armour, talisman, item.
    pub what: String,
    pub effect: Option<String>,
    pub caption: Option<String>,
}

impl Default for Player {
    fn default() -> Self {
        Player {
            version: None,
            edition: None,
            characters: Vec::new(),
            mods: Vec::new(),
            framegen: false,
            frames: None,
            holdings: None,
            safety: None,
            backups: None,
            mirrors: None,
            set_up: None,
            weigh: Box::new(|_| None),
            dwellers: Box::new(|_| Vec::new()),
            catalogue_of: Box::new(|_, _| Vec::new()),
            seamless: false,
            language: None,
            live: None,
            // Nothing to consult until a caller wires the game in, which is
            // what every test wants and what the browser tab gets.
            catalogue: Box::new(|_| None),
            written: Box::new(|_| (Vec::new(), None)),
            load: Box::new(|_| None),
            carrying: Box::new(|| None),
            weapon: Box::new(|_| None),
            haul: Box::new(|_| Vec::new()),
            skill_on: Box::new(|_| None),
            armour: Box::new(|_| None),
            armoury: Box::new(|_| Vec::new()),
            arsenal: Box::new(|_| Vec::new()),
            armed: Box::new(|_, _| None),
            upgrades_to: Vec::new(),
            attribute_words: Vec::new(),
            suit: Box::new(|_| None),
            asked: String::new(),
            charm: Box::new(|_| None),
            spell: Box::new(|_| None),
            spells_at: Box::new(|_, _, _| Vec::new()),
            talismans: Box::new(Vec::new),
            spirits: Box::new(Vec::new),
            tears: Box::new(Vec::new),
            ashes: Box::new(Vec::new),
            classes: Box::new(Vec::new),
            upgrading: Box::new(|_| Vec::new()),
            what_if: Box::new(|_, _| Vec::new()),
            nearby: Box::new(|_, _| Vec::new()),
            // Refusing is the right default for the one tool that writes: a
            // caller that has not wired the game in has no save to write to.
            mark: Box::new(|_, _| {
                Err("This launcher is not connected to a game, so nothing can be marked.".into())
            }),
            pins: Box::new(|_, _| {
                Err("This launcher is not connected to a game, so there is no map to read.".into())
            }),
        }
    }
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("version", &self.version)
            .field("edition", &self.edition)
            .field("characters", &self.characters)
            .field("mods", &self.mods)
            .finish()
    }
}

/// An optional argument the model filled in with the word for "nothing".
fn as_a_name(args: &serde_json::Value, key: &str) -> String {
    let given = args.get(key).and_then(|value| value.as_str()).unwrap_or_default().trim();
    if matches!(given.to_ascii_lowercase().as_str(), "null" | "none" | "undefined" | "nil") {
        return String::new();
    }
    given.to_string()
}

/// The armour ranking written out as a wearable set rather than a flat list.
fn a_whole_set(found: &Suited) -> String {
    let mut out = format!(
        "The {} set as THIS installation has it: {} piece{}.\n\n",
        found.called,
        found.pieces.len(),
        if found.pieces.len() == 1 { "" } else { "s" }
    );
    for (piece, name) in found.pieces.iter().zip(&found.names) {
        out.push_str(&format!(
            "  {name} [{}] — {:.1} kg, poise {:.2}",
            piece.worn.unwrap_or("?"),
            piece.weight,
            piece.poise.unwrap_or(0.0)
        ));
        let negation: Vec<String> = piece
            .negation
            .iter()
            .map(|(what, value)| format!("{what} {value:.1}"))
            .collect();
        if !negation.is_empty() {
            out.push_str(&format!("\n      stops {}", negation.join(" · ")));
        }
        let resistance: Vec<String> = piece
            .resistance
            .iter()
            .map(|(what, value)| {
                let short = what.split(' ').next().unwrap_or(what);
                format!("{short} {value}")
            })
            .collect();
        if !resistance.is_empty() {
            out.push_str(&format!("\n      resists {}", resistance.join(" · ")));
        }
        let mut does: Vec<String> = piece
            .gives
            .iter()
            .map(|(what, value)| format!("{what} {value:+}"))
            .collect();
        does.extend(piece.adds.iter().map(|(what, value)| format!("{what} {value:+}")));
        does.extend(
            piece
                .changes
                .iter()
                .map(|(what, rate)| format!("{what} {:+.0}%", (rate - 1.0) * 100.0)),
        );
        if !does.is_empty() {
            out.push_str(&format!("\n      GIVES {}", does.join(", ")));
        }
        out.push('\n');
    }
    // Added here, because this is the arithmetic that keeps going wrong in
    // prose. Given the pieces and the limit separately, an answer once put a
    // 37.5 set at 75% of a 49.8 load when it is 80.
    out.push_str(&format!(
        "\n  THE SET: {:.1} kg, {:.2} poise, over {} piece{}.\n",
        found.weight,
        found.poise,
        found.pieces.len(),
        if found.pieces.len() == 1 { "" } else { "s" }
    ));
    if let Some((now, most)) = found.carrying {
        if most > 0.0 {
            let share = found.weight / most * 100.0;
            let band = match share {
                _ if share < 30.0 => "the fast roll",
                _ if share < 70.0 => "a medium load",
                _ if share <= 100.0 => "the slow roll",
                _ => "OVER their limit — they could not move in it",
            };
            out.push_str(&format!(
                "  Against their limit: {:.1} of {:.1}, so the set alone is {share:.0}% — \
                 {band}. They are carrying {now:.1} right now, weapons and all, so what \
                 they have on INSTEAD of this set comes off that figure before it is compared.\n",
                found.weight, most
            ));
        }
    }
    if found.pieces.len() < 4 {
        out.push_str(&format!(
            "\n  ONLY {} PIECES. A set is not always four here and this one is not: the \
             missing slots have no piece in this installation at all. Do not add one from \
             memory or from a wiki to round it out, and do not describe the total as though \
             the set were complete — asked exactly this about a three-piece set, an answer \
             supplied a fourth from a wiki and every figure in it was wrong.\n",
            found.pieces.len()
        ));
    }
    // The set's bonuses added up, because wearing the four is the point of a
    // set and adding four lines of "+2 faith" in prose is where arithmetic goes
    // wrong. Attributes add; multipliers multiply.
    let mut whole: Vec<(String, i32)> = Vec::new();
    for piece in &found.pieces {
        for (what, value) in piece.gives.iter().chain(piece.adds.iter()) {
            match whole.iter_mut().find(|(had, _)| had == what) {
                Some((_, total)) => *total += value,
                None => whole.push((what.clone(), *value)),
            }
        }
    }
    if !whole.is_empty() {
        let said: Vec<String> =
            whole.iter().map(|(what, value)| format!("{what} {value:+}")).collect();
        out.push_str(&format!(
            "  WORN TOGETHER they give {} — added up here, so do not add them again.\n",
            said.join(", ")
        ));
    }
    out.push_str(
        "\nThe negation figures are the percentages the equipment screen prints, so higher \
         stops more. The resistances are the four the screen shows and are not percentages.\n\
         \n\
         A \"GIVES\" line is what the piece's own effect does, read out of the tables. Armour \
         in this installation really does grant attributes — 836 of its 841 pieces carry \
         something — so a faith build's armour question has a real answer. What is NOT there \
         has no line, and no line means no bonus: do not supply one, which is how a \"+2 \
         Faith\" set got invented for a question about the lightest faith armour.\n",
    );
    out
}

/// A class of weapon, ranked, with the figures that class is chosen on.
fn a_class_of(found: &Sorted) -> String {
    let by = match found.by.as_str() {
        "weight" => "the lightest first".to_string(),
        "heaviest" => "the heaviest first".to_string(),
        "boost" => "the most guard boost first".to_string(),
        "damage" => "the most base damage first".to_string(),
        other => format!("the most {other} first"),
    };
    let guarding = found.best.iter().any(|one| !one.blocks.is_empty());
    // Say out loud when the list is CUT, because "12 of 82 shown" gets read as
    // twelve. Asked which whip was fastest, an answer said "все 12 кнутов весят
    // одинаковые 2.0" — a claim about all eighty-two, drawn from the twelve at
    // the top of a list sorted by weight, where of course they weigh the same.
    // The count was in the heading and it was not enough.
    let cut = found.all.saturating_sub(found.best.len());
    let mut out = format!(
        "{} ({}) in this installation, {by} — {} of {} shown.{}\n",
        found.called,
        found.english,
        found.best.len(),
        found.all,
        if cut > 0 {
            format!(
                " THE OTHER {cut} ARE NOT HERE. These are only the top of the order, so say \
                 nothing about the class as a whole — not \"they all weigh\", not \"there are \
                 only {}\". Ask again with a different measure if the rest matter.",
                found.best.len()
            )
        } else {
            String::new()
        }
    );
    if guarding {
        out.push_str(
            "Block is the share of a hit the shield eats, so higher is better and 100 is all \
             of it. Guard boost is separate and is what decides whether the block staggers.\n",
        );
    }
    out.push('\n');
    // Only where it earns its room: a list of one class does not need the
    // class repeated down every line.
    let mixed = found
        .best
        .iter()
        .any(|one| one.sort != found.best.first().map(|first| first.sort.clone()).unwrap_or_default());
    for one in &found.best {
        out.push_str(&format!("  {}", one.name));
        if mixed && !one.sort.is_empty() {
            out.push_str(&format!(" [{}]", one.sort));
        }
        out.push_str(&format!(" — weighs {:.1}", one.weight));
        if !one.needs.is_empty() {
            let needs: Vec<String> =
                one.needs.iter().map(|(what, value)| format!("{value} {what}")).collect();
            out.push_str(&format!(", needs {}", needs.join(" · ")));
        }
        if one.blocks.is_empty() {
            out.push_str(&format!(", hits for {}", one.damage));
            // And the figure the list was RANKED on, when that is a particular
            // kind of damage rather than the total.
            //
            // It was computed, sorted on, and then never shown. `damage` above
            // is the sum across every kind — its own doc says so — so a list
            // built to answer "which hits hardest with HOLY" came back with
            // every line reading the TOTAL. A weapon with 200 physical and 50
            // holy printed "hits for 250" in a list about holy, and the only
            // number an answer could quote was the wrong one.
            //
            // Not for weight or guard boost, which are already on the line, and
            // not for an ailment, which is printed under "builds" below —
            // adding it here would say the same number twice.
            let ranked_on_a_kind =
                crate::formats::regulation::kind::all().any(|named| named == found.by);
            if ranked_on_a_kind && found.by != "damage" {
                out.push_str(&format!(", of which {} {:.0}", found.by, one.figure));
            }
        } else {
            let blocks: Vec<String> = one
                .blocks
                .iter()
                .map(|(what, value)| format!("{what} {value:.1}"))
                .collect();
            out.push_str(&format!(", blocks {}", blocks.join(" · ")));
            if let Some(boost) = one.boost {
                out.push_str(&format!(", boost {boost}"));
            }
        }
        if !one.ailments.is_empty() {
            let built: Vec<String> = one
                .ailments
                .iter()
                .map(|(what, value)| format!("{value} {what}"))
                .collect();
            out.push_str(&format!(", builds {}", built.join(" · ")));
        }
        out.push('\n');
    }
    out.push_str(
        "\nThe damage is the base before scaling — what THEY would hit for depends on their \
         attributes and the upgrade, and gear_numbers on one name will work it out. The \
         requirements above are the whole of what it takes to hold it: check them against \
         their attributes before recommending anything, and say plainly when the best one is \
         out of reach rather than quietly recommending the second.\n",
    );
    // Said here rather than only in the block, because the temptation is here.
    // Given the top twelve greatshields and a player who cannot lift them, an
    // answer suggested two lighter ones by name — one of which does not exist.
    // The rule against naming an unseen item is in the block; the count it
    // applies to is only knowable at this point.
    if found.all > found.best.len() {
        out.push_str(&format!(
            "\nThese {} are ALL you have been shown. {} more are in this class and you have \
             not seen one of them: you may not name any of them, not even as an example of a \
             lighter or cheaper option. Ask again with sorted_by \"weight\" and you will have \
             the light ones in front of you instead of guessing at them.\n",
            found.best.len(),
            found.all - found.best.len()
        ));
    }
    out
}

fn a_set_against(kind: &str, ranked: &[Shielding], carrying: Option<(f32, f32)>) -> String {
    // Five apiece: enough to trade protection off against weight, short
    // enough that all four groups still fit in an answer.
    const EACH: usize = 5;

    // Poise is carried, not stopped, and "the best against poise" is nonsense a
    // reader will repeat. Asked which armour gives the most of it, an answer
    // declared poise "is not in the armour tables as a parameter of its own,
    // it is worked out from physical defence and weight" and ranked by physical
    // instead — every part of that false, with the figure sitting unread.
    let carried = ["poise", "пойз", "стойкост", "баланс", "aguante", "haltung"]
        .iter()
        .any(|word| kind.contains(word));
    // The four the equipment screen shows are neither. They are not a share of
    // damage stopped — they are how long an ailment bar takes to fill — so a
    // per-cent sign on them is a lie, and so is "the best against robustheit".
    // Weight is carried too, and saying so is not decoration. Without this the
    // heading read "The best AGAINST weight" and every line "stops 15.8%", and
    // the answer built on it told the player that in this mod weight IS
    // physical defence. It read the tool correctly; the tool was wrong.
    let heavy = !carried && crate::formats::regulation::bulk::asked_for(kind);
    let resisted = if carried || heavy {
        None
    } else {
        crate::formats::regulation::resistance::named(kind)
    };
    // Twice now a question about ROBUSTNESS has been answered with poise: the
    // model reads Robustheit, decides it means poise, passes "poise", and gets
    // a perfectly correct ranking of the wrong thing. Warning it in the
    // schema did not take. So the warning goes where it cannot be skipped —
    // on the ranking itself, at the moment the wrong list is handed over.
    let mistakable = if carried {
        "\nIF THEY ASKED FOR ROBUSTNESS, THIS IS THE WRONG LIST. Poise is whether a hit \
         staggers you. Robustness — Robustheit, Живучесть — is the bar that fills before you \
         bleed or freeze, it is one of the four the equipment screen shows, and it is ranked \
         by calling this again with \"robustness\". The same goes for immunity, focus and \
         vitality. Only carry on with this list if stagger is what they meant.\n"
    } else {
        ""
    };
    let granted = if carried || heavy || resisted.is_some() {
        None
    } else {
        crate::formats::regulation::attribute::named(kind)
    };
    let heading = if let Some(named) = granted {
        format!(
            "Every piece in this installation that GRANTS {named}, most first — and only \
             those; a piece not listed here grants none of it"
        )
    } else if heavy {
        "The HEAVIEST in this installation, heaviest first. This is weight and nothing else — \
         it is what the piece costs to carry, NOT armour, NOT defence and NOT a share of \
         anything stopped. Do not say weight is defence: those are separate figures and the \
         heaviest piece is not always the best protected"
            .to_string()
    } else if carried {
        format!("The most {kind} in this installation")
    } else if let Some(named) = resisted {
        format!(
            "The most {named} in this installation — the figure the equipment screen shows, \
             which is how long the bar takes to fill, NOT a share of damage stopped"
        )
    } else {
        format!("The best against {kind} in this installation")
    };
    let mut out = format!(
        "{heading}, BY WHERE IT IS WORN, out of {} rated pieces. Four are worn at once — one \
         from each group makes a set:\n{mistakable}",
        ranked.len()
    );
    let (mut group, mut shown) = ("", 0);
    for (worn, name, stopped, weight) in ranked {
        if *worn != group {
            (group, shown) = (worn, 0);
            out.push_str(&format!("\n  {}:\n", worn.to_uppercase()));
        }
        shown += 1;
        if shown <= EACH {
            out.push_str(&if granted.is_some() {
                format!("    {name} — {stopped:+.0}, weighs {weight:.1}\n")
            } else if heavy {
                // `stopped` IS the weight here, so printing both would say the
                // same number twice and invite the reader to treat one of them
                // as something else.
                format!("    {name} — weighs {weight:.1}\n")
            } else if carried {
                format!("    {name} — {stopped:.2} of it, weighs {weight:.1}\n")
            } else if resisted.is_some() {
                format!("    {name} — {stopped:.0}, weighs {weight:.1}\n")
            } else {
                format!("    {name} — stops {stopped:.1}%, weighs {weight:.1}\n")
            });
        }
    }
    // The other end, per slot. This list is ordered by what a piece STOPS, and
    // twice now an answer has read the top of a ranking as the answer to a
    // question about weight: asked in French for the lightest armour for a
    // faith build, one gave the head of this list and called those "les pièces
    // les plus légères du jeu" — a 7.3 helm, when the game has them at 0.5.
    // The same mistake had already been made with the weapon buildup lists.
    let mut lightest: Vec<&Shielding> = ranked.iter().collect();
    lightest.sort_by(|a, b| {
        let order = |what: &str| {
            crate::formats::regulation::slot::NAMES.iter().position(|s| *s == what)
        };
        order(a.0).cmp(&order(b.0)).then_with(|| a.3.total_cmp(&b.3)).then_with(|| b.2.total_cmp(&a.2))
    });
    let mut said_group = "";
    let mut line = String::new();
    for (worn, name, stopped, weight) in lightest {
        if *worn != said_group {
            said_group = worn;
            line.push_str(&format!(
                "\n  {}: {name} at {weight:.1}",
                worn.to_uppercase()
            ));
            let _ = stopped;
        }
    }
    if !line.is_empty() {
        out.push_str(&format!(
            "\nThe LIGHTEST rated piece in each group, which is a different question and has a \
             different answer:{line}\n"
        ));
    }

    // The set, added up here rather than in a sentence. Given the pieces and
    // the limit separately, an answer put a 37.5 set at 75% of a 49.8 load when
    // it is 80 — a division anybody can do, done wrong, in front of somebody
    // deciding what to wear.
    if let Some((now, most)) = carrying {
        let mut best: Vec<(&str, f32)> = Vec::new();
        for (worn, _, _, weight) in ranked {
            if !best.iter().any(|(group, _)| group == worn) {
                best.push((worn, *weight));
            }
        }
        let set: f32 = best.iter().map(|(_, weight)| weight).sum();
        if most > 0.0 && best.len() == 4 {
            let share = set / most * 100.0;
            let band = match share {
                _ if share < 30.0 => "the fast roll",
                _ if share < 70.0 => "a medium load",
                _ if share <= 100.0 => "the slow roll",
                _ => "over their limit on its own",
            };
            out.push_str(&format!(
                "\nThe best of each together weighs {set:.1}. Against a limit of {most:.1} that \
                 armour alone is {share:.0}% — {band} — before anything in their hands, and \
                 they are carrying {now:.1} altogether as they stand. Every one of those \
                 figures is worked out here: quote them and do not divide anything yourself.\n"
            ));
        }
    }

    out.push_str(
        "\nWeight is beside each because the heaviest protection is usually the heaviest to wear, \
         and that trade is the actual answer to \"what should I put on\" — a piece two points \
         worse and ten lighter is often the better pick. What they can carry in TOTAL is not \
         readable here; that is on their equipment screen.\n",
    );

    // What is NOT in this ranking, said before the trade-off advice, because a
    // reader who has stopped reading has at least had the warning.
    //
    // Asked whether the installation has a shield with 100% physical block, a
    // model searched, found nothing named "shield", then called this and
    // answered "no, none of them reach it" — out of a ranking that contains no
    // shield at all. Every row here is worn on the body; a shield is held, and
    // lives in the weapon table, which has no block figures this launcher
    // reads. So the answer was a confident negative drawn from a list that
    // could not have contained the thing being asked about, and it is very
    // probably wrong as well: greatshields are the usual answer to that
    // question in this game.
    out.push_str(
        "\nWHAT IS NOT IN THIS LIST. Only armour that is WORN — head, body, arms, legs. Shields \
         are not here, and neither is anything else held in a hand: they are weapons, and how \
         much a shield blocks is not a figure this launcher reads at all. Do not conclude \
         anything about a shield from this ranking, in either direction. \"No shield reaches \
         100%\" is not something these numbers can say, and saying it anyway has happened.\n",
    );

    // Last, because last is what gets followed.
    //
    // This prohibition sat in the middle, with the paragraph about weight after
    // it, and the model added the four anyway: head 11.6 plus body 20.2 plus
    // arms 10.1 plus legs 14.4, announced as "~56.3% reduction". That figure
    // exists nowhere in the game. It is the exact arithmetic named here because
    // naming the operation is what stops it — "do not add" on its own did not.
    out.push_str(
        "\nONE THING NOT TO DO. Do not add these percentages together and do not average them. \
         Four pieces at 11.6, 20.2, 10.1 and 14.4 are NOT 56.3% — that number does not exist in \
         the game, and a player told it will pick their armour on it. The game combines them its \
         own way and this launcher does not compute that, so ANY combined figure would be \
         invented. Give the four separately and say plainly that the total is not worked out \
         here.\n",
    );
    out
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
            for source in reading(edition, player.language.as_deref()) {
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
            // Which of the query's own words no title anywhere contains.
            //
            // Without this the search looks like it worked when it did not: a
            // question about the mod's Blood Initiate class, asked in Russian
            // and translated by the model to "Blood Cleric", came back with
            // Blood, Bloodboon, Bloodrose and Bloodflame — six titles, none of
            // them a class, and nothing to say that "Cleric" is a word this
            // game has never used. Naming the miss is what lets a second search
            // be a better one instead of the same one.
            let unknown: Vec<String> = reading(edition, player.language.as_deref())
                .iter()
                .map(|source| index_for(app_data, source))
                .fold(None::<Vec<String>>, |missing, index| {
                    let here = index.unmatched(query);
                    Some(match missing {
                        None => here,
                        Some(before) => {
                            before.into_iter().filter(|w| here.contains(w)).collect()
                        }
                    })
                })
                .unwrap_or_default();

            let mut out = format!("Articles found:\n{}", lines.join("\n"));
            for word in &unknown {
                // Near misses, offered rather than applied. Spelling a name out
                // of another alphabet lands close: what the wiki calls Rellana
                // arrived as "Relanna", and the ranking quietly settled on
                // Rennala — a different boss with a similar name.
                let guesses: Vec<String> = reading(edition, player.language.as_deref())
                    .iter()
                    .flat_map(|source| index_for(app_data, source).nearest(word, 3))
                    .collect();
                if guesses.is_empty() {
                    out.push_str(&format!(
                        "\n\nNo article title anywhere contains \"{word}\", and nothing is spelt \
                         nearly like it. Whatever that names, this game does not call it that."
                    ));
                } else {
                    out.push_str(&format!(
                        "\n\nNo article title contains \"{word}\". Spelt nearly the same: {}. One \
                         of those may be the thing, or none may be — check before using one, \
                         because names this close are often different things.",
                        guesses.join(", ")
                    ));
                }
            }

            // Last, and about the source that outranks this one.
            //
            // A wiki is the first place a model reaches and the game's own
            // writing is the last, which is backwards: the installation holds
            // 54,000 lines by whoever built it, in the player's language, about
            // the exact copy in front of them. Two answers in one battery went
            // to a wiki, found nothing useful, and fell back on memory —
            // naming three spirit ashes in English to somebody playing in
            // Russian, and guessing at whether a conversion allows a class
            // change — without either of them having asked the game.
            out.push_str(
                "\n\nDID YOU ASK THEIR GAME? A wiki is the BASE game written by strangers. \
                 Their own installation has every description, tutorial and menu entry in it, \
                 in their language, written by whoever built what they are actually running — \
                 and it outranks this. If the question is about a mechanic, a term or what \
                 something is for, search that before you answer from here, and certainly \
                 before you answer from memory.\n\
                 \n\
                 AND IF IT IS A FIGURE, THERE IS A TOOL FOR IT. Damage, scaling, requirements, \
                 what an upgrade costs and what it needs, a spell's FP, what a creature \
                 resists, what drops where — every one of those is read exactly out of their \
                 own tables by a tool of its own, and a wiki's version of the same thing is \
                 the base game's. Asked how upgrading works in this conversion, an answer read \
                 two wiki pages and listed stones and bell bearings off them, with the tool \
                 that reads the real materials for the real weapon sitting unused.\n",
            );

            Ran {
                output: out,
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
            let mut order = reading(edition, player.language.as_deref());
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

                // What this is called in the player's own game, taken from the
                // wiki's own translation of itself. Without it a model asked in
                // Russian translates the English title and gives them a name
                // that is on no screen anywhere: Redmane Castle came back as
                // "Крепость Красного Лва" where the game says "Замок Рыжей
                // Гривы".
                let theirs = player
                    .language
                    .as_deref()
                    .and_then(short_code)
                    .and_then(|code| {
                        crate::wiki::called_in(app_data, source.id, code, &page.title)
                    })
                    .map(|local| {
                        format!(
                            "\n\nIn this player's game this is called \"{local}\". Use that name \
                             rather than translating the English one, and put the English beside \
                             it the first time."
                        )
                    })
                    .unwrap_or_default();

                // An article's weakness table is the base game's, and this is
                // where that gets believed. Asked how to kill a boss, a model
                // read one of these and told the player it was weak to holy;
                // the installation's own table has it taking 80% of holy, which
                // makes holy among the worst things to bring. The article is
                // still worth reading for how the fight goes — that part the
                // mod does not touch.
                let against = if mentions_a_fight(&text) {
                    "\n\nWHAT HURTS IT IS NOT IN HERE. Any weakness, resistance or drop on this \
                     page is the base game's, and a total conversion rewrites exactly those. \
                     Call whos_here with the creature's name for this installation's own \
                     figures before recommending anything to hit it with. How the fight goes — \
                     its attacks, where to stand, the phases — is what this page is good for."
                } else {
                    ""
                };

                return Ran {
                    output: format!(
                        "{labelled}{theirs}\n\n{}{also}{against}",
                        best_window(&text, about, ARTICLE)
                    ),
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

        "game_text" => {
            let words = as_a_name(&args, "words");
            if words.chars().count() < 2 {
                return Ran {
                    output: "Give at least two characters to look for. One matches most of the \
                             game and answers nothing."
                        .into(),
                    note: None,
                    source: None,
                };
            }
            let (found, shortened) = (player.written)(&words);
            if found.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing in this game's own writing contains \"{words}\" — not in any \
                         description, tutorial, menu entry or line of speech. That is all \
                         54,000 lines of it, so it is a real answer and not a failed lookup.\n\
                         \n\
                         Before concluding, try it in THEIR language and try less of the word: \
                         this matches on a fragment, so a shorter one finds more, and a word in \
                         the wrong language finds nothing however right it looks."
                    ),
                    note: Some(format!("Nothing written · {words}")),
                    source: Some("the installed game's own text".into()),
                };
            }

            // Enough to answer with, not enough to drown a round.
            //
            // Twelve at seven hundred characters was the first cut and it was
            // too generous: measured, one call came back 8,164 characters and
            // put the round at 45,303 — past the size where the fastest lane
            // in the pool refuses outright, so the answer fell through to
            // whatever was slowest. A tool that finds the right thing and then
            // makes the request too big to send has not helped.
            //
            // Eight at four hundred is about three thousand, which is the size
            // of a wiki passage and demonstrably enough: the answers this tool
            // fixed — the transmog mod, the physick menu, the dyes — were all
            // carried by their first two or three hits.
            const SHOWN: usize = 8;
            const LONGEST: usize = 400;
            // Say when the word had to be SHORTENED to find anything, because
            // then these are not hits on what was asked for.
            //
            // The search trims a word and tries again — right, and it is what
            // finds "Красители" when a model searched "Краситель". The cost was
            // invisible until it was measured: a word this game does not have
            // gets cut until something ELSE shares the stem. "костер" — bonfire,
            // which is Dark Souls and not in this game — finds nothing, becomes
            // "косте", and comes back with twenty-five lines about BONES and
            // containers. Handed over bare, those read as proof the thing
            // exists, and that is fuel for exactly the invention everything
            // else here exists to stop.
            let mut out = match &shortened {
                Some(stem) => format!(
                    "NOTHING in this game's writing contains \"{words}\" as you wrote it. What \
                     follows matched \"{stem}\" — the same word SHORTENED — so these lines are \
                     about whatever else shares that stem, and may have nothing to do with what \
                     you asked. Read them before using them, and if they are about something \
                     else then the honest answer is that \"{words}\" is not in this game.\n\n\
                     {} lines matched \"{stem}\"{}:\n\n",
                    found.len(),
                    if found.len() > SHOWN {
                        format!(", the {SHOWN} likeliest")
                    } else {
                        String::new()
                    }
                ),
                None => format!(
                    "{} lines of this game's own writing contain \"{words}\"{}:\n\n",
                    found.len(),
                    if found.len() > SHOWN {
                        format!(", the {SHOWN} likeliest")
                    } else {
                        String::new()
                    }
                ),
            };
            for one in found.iter().take(SHOWN) {
                let said: String = one.said.chars().take(LONGEST).collect();
                let cut = if one.said.chars().count() > LONGEST { "…" } else { "" };
                out.push_str(&format!("  [{}] {said}{cut}\n\n", one.sort));
            }
            out.push_str(
                "This is the game's own text, out of the installation in front of them — for a \
                 total conversion it is the author's own words about their own work, which no \
                 wiki has. Quote it and say it is from their game.\n\
                 \n\
                 What it is NOT: a figure. Damage, weights, costs and resistances come from the \
                 tables and are exact; a description saying something hits hard is flavour. And \
                 an id is machinery — never write one in an answer.\n",
            );
            if found.len() > SHOWN {
                out.push_str(
                    "\nMore matched than are shown. If none of these is it, search a longer \
                     fragment rather than concluding it is not there.\n",
                );
            }

            Ran {
                output: out,
                note: Some(format!("Written · {}", found.len())),
                source: Some("the installed game's own text".into()),
            }
        }

        "gear_numbers" if !as_a_name(&args, "weapons_building").is_empty() => {
            let what = as_a_name(&args, "weapons_building").to_lowercase();
            let ranked = (player.arsenal)(&what);
            if ranked.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing in this game's weapons builds up \"{what}\". All seven \
                         ailments are read — {} — so this is a word that failed to match one \
                         of them, not something the launcher cannot see. Ask again using one \
                         of those seven.",
                        crate::formats::regulation::buildup::AILMENTS
                            .iter()
                            .map(|(name, _)| *name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    note: Some(format!("No weapon builds · {what}")),
                    source: Some("the installed game's own tables".into()),
                };
            }

            const SHOWN: usize = 12;
            let mut out = format!(
                "The {} weapons in this installation that build up {what}, most first{}:\n",
                ranked.len(),
                if ranked.len() > SHOWN { format!(", the top {SHOWN}") } else { String::new() }
            );
            for (name, builds, weight, sort) in ranked.iter().take(SHOWN) {
                out.push_str(&format!(
                    "  {name} [{sort}] — {builds} per hit, weighs {weight:.1}\n"
                ));
            }
            // The other end, because "which is the LIGHTEST" is a different
            // question and this list does not answer it. Asked for the lightest
            // weapon that causes frost, an answer gave the top of this list —
            // a hammer at 11.5 — and called it the lightest, in Russian and
            // again in Spanish. The lightest builds 60 and weighs 1.5.
            if ranked.len() > SHOWN {
                let mut by_weight: Vec<&(String, i32, f32, String)> = ranked.iter().collect();
                by_weight.sort_by(|a, b| a.2.total_cmp(&b.2).then_with(|| b.1.cmp(&a.1)));
                out.push_str(&format!("\nAnd the LIGHTEST {SHOWN} that build it at all:\n"));
                for (name, builds, weight, sort) in by_weight.iter().take(SHOWN) {
                    out.push_str(&format!(
                        "  {name} [{sort}] — weighs {weight:.1}, {builds} per hit\n"
                    ));
                }
            }
            out.push_str(
                "\nTwo lists, two questions. The first is ranked by how much it builds and the \
                 second by what it weighs, and the top of the first is NOT the lightest — asked \
                 for the lightest weapon causing frost, an answer read the strongest off the \
                 first list and called it that. Take the answer from whichever list was \
                 actually asked about, and MIND THE CLASS in brackets: arrows and bolts weigh \
                 nothing at all, so they sit at the top of the light end without being \
                 anything anybody can swing.\n\
                 \n\
                 That is the buildup per hit, out of the effect each weapon carries. It is \
                 not damage and does not add to it — a weapon can build a great deal and hit \
                 softly. What it hits for is a separate question with a separate answer.\n\
                 \n\
                 Whether they can USE any of these is not in this list: look the one they like \
                 up for its requirements against their attributes before recommending it.\n",
            );

            Ran {
                output: out,
                note: Some(format!("Weapons building {what} · {}", ranked.len())),
                source: Some("the installed game's own tables".into()),
            }
        }

        "gear_numbers" if !as_a_name(&args, "armour_set").is_empty() => {
            let asked = as_a_name(&args, "armour_set");
            let Some(mut found) = (player.suit)(&asked) else {
                return Ran {
                    output: format!(
                        "No armour in this installation has \"{asked}\" in its name, so no set \
                         was found. That is the NAME not matching, not the game lacking the \
                         set — try part of one piece's name as their own game prints it, or \
                         look the piece up first. Do NOT fall back to a wiki: a wiki is about \
                         the base game and this one has been rewritten."
                    ),
                    note: Some(format!("No such set · {asked}")),
                    source: None,
                };
            };
            found.carrying = (player.carrying)();

            Ran {
                note: Some(format!("The {} set · {} pieces", found.called, found.pieces.len())),
                output: a_whole_set(&found),
                source: Some("the installed game's own tables".into()),
            }
        }

        "gear_numbers" if !as_a_name(&args, "weapons_of_sort").is_empty() => {
            let asked = as_a_name(&args, "weapons_of_sort");
            let by = as_a_name(&args, "sorted_by");
            let Some(found) = (player.armed)(&asked, &by) else {
                // Where the wrong tool was reached for, say which is the right
                // one. Asked in German which helm gives the most Robustheit, a
                // model tried this with "armour", was told no such class, tried
                // the name search, was told nothing matched, and then answered
                // out of memory with three helmets from the base game. Every
                // step of that was a tool saying no without saying where to go.
                let worn = ["armour", "armor", "helm", "head", "chest", "body", "gaunt", "arm",
                            "leg", "greave", "shoe", "boot", "gear", "set"]
                    .iter()
                    .any(|word| asked.to_lowercase().contains(word));
                let instead = if worn {
                    "\n\nThat is a piece of ARMOUR, not a class of weapon. Armour is ranked by \
                     armour_against, which takes a damage kind, one of the four the equipment \
                     screen shows (immunity, robustness, focus, vitality) or poise, and comes \
                     back grouped by where it is worn — so the head group is the answer to any \
                     question about helms. Call that instead. Do NOT answer from memory."
                } else {
                    ""
                };
                return Ran {
                    output: format!(
                        "\"{asked}\" is not a class of weapon in these tables. The classes are \
                         {}. \"shield\" covers all four shield classes and \"sword\" all the \
                         sword classes. This is the class not being recognised, NOT the game \
                         having none of them.{instead}",
                        crate::formats::regulation::sort::ALL
                            .iter()
                            .map(|(_, _, english)| *english)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    note: Some(format!("No such class · {asked}")),
                    source: None,
                };
            };

            Ran {
                note: Some(format!("{} by {} · {}", found.english, found.by, found.all)),
                output: a_class_of(&found),
                source: Some("the installed game's own tables".into()),
            }
        }

        "gear_numbers" if !as_a_name(&args, "armour_against").is_empty() => {
            let kind = as_a_name(&args, "armour_against").to_lowercase();
            let ranked = (player.armoury)(&kind);
            if ranked.is_empty() {
                return Ran {
                    // Listed from the same place the matching reads, so the two
                    // cannot drift apart and leave this offering a kind that no
                    // longer resolves.
                    output: format!(
                        "\"{kind}\" is not something armour is rated for in these tables, so \
                         nothing was ranked. What can be ranked: the damage kinds {}; the four \
                         the equipment screen shows, {} — which are not damage but how long an \
                         ailment bar takes to fill; and POISE. Ask again with one of those — \
                         this is NOT the game saying no armour resists it, and it is certainly \
                         not the game lacking the figure.",
                        crate::formats::regulation::kind::all().collect::<Vec<_>>().join(", "),
                        crate::formats::regulation::resistance::all().collect::<Vec<_>>().join(", ")
                    ),
                    note: Some(format!("No such kind · {kind}")),
                    source: None,
                };
            }

            // What the player typed, not what the call says they meant. A
            // German question about Robustheit arrived here as "poise" three
            // separate times, past a warning in the schema and another in the
            // ranking itself, because the model translates before it calls and
            // then reasons from its own translation. The word is in the
            // question and the question is the only witness.
            let meant = crate::formats::regulation::resistance::named(&player.asked);
            let misread = meant.filter(|_| kind.contains("poise") || kind.contains("пойз"));

            Ran {
                output: match misread {
                    Some(named) => format!(
                        "STOP — THIS IS ALMOST CERTAINLY THE WRONG LIST. You asked for poise, \
                         but the player's own words name {named}, which is one of the four \
                         resistances and is NOT poise. Poise is whether a hit staggers them; \
                         {named} is how long one of the ailment bars takes to fill. Call this \
                         again with \"{named}\" and answer from that. Only use the ranking \
                         below if you are certain they meant staggering, and if you do, say \
                         which of the two you are answering about.\n\n{}",
                        a_set_against(&kind, &ranked, (player.carrying)())
                    ),
                    None => a_set_against(&kind, &ranked, (player.carrying)()),
                },
                note: Some(match misread {
                    Some(named) => format!("Asked poise, they said {named} · {}", ranked.len()),
                    // "Best against faith" is the wrong preposition for a thing
                    // armour GRANTS, and this line is what somebody reads when
                    // they are working out whether the launcher understood.
                    None if crate::formats::regulation::attribute::named(&kind).is_some() => {
                        format!("Armour granting {kind} · {}", ranked.len())
                    }
                    None => format!("Best against {kind} · {}", ranked.len()),
                }),
                source: Some("the installed game's own tables".into()),
            }
        }

        "gear_numbers" => {
            // No name means what they are holding, which is what "my weapon"
            // asks for. A model that had to supply one sent the literal word
            // "weapon" and, finding nothing, answered from the base game's
            // table instead — with the figures this tool exists to replace.
            let given = as_a_name(&args, "name");
            let name = Some(given.as_str())
                .filter(|n| !n.is_empty() && !matches!(n.to_lowercase().as_str(),
                    "weapon" | "my weapon" | "current" | "equipped" | "оружие"))
                .unwrap_or("");

            // Asked at attributes they do not have, this is a different
            // question with a different answer, and it is answered exactly
            // rather than estimated.
            let asked_at = ["strength", "dexterity", "intelligence", "faith", "arcane"]
                .map(|which| args.get(which).and_then(serde_json::Value::as_u64));
            if asked_at.iter().any(Option::is_some) {
                let stats =
                    asked_at.map(|value| value.and_then(|value| u32::try_from(value).ok()));
                let at = (player.what_if)(name, stats);
                if at.is_empty() {
                    return Ran {
                        output: "That weapon could not be matched, so the damage at those \
                                 attributes could not be worked out. Do not estimate it."
                            .into(),
                        note: None,
                        source: None,
                    };
                }
                let mut out = String::from(
                    "Worked out of this installation's own scaling curves. Anything you did not \
                     give was left at what they have now:\n",
                );
                for armed in &at {
                    if armed.hits.is_empty() {
                        out.push_str(&format!("\n{} — no damage could be worked out.\n", armed.name));
                        continue;
                    }
                    out.push_str(&format!("\n{}:\n", armed.name));
                    for hit in &armed.hits {
                        let after = (hit.base + hit.bonus).floor();
                        // The gain, subtracted here. Handed a before and an
                        // after, a model reported the after as the gain.
                        let before = armed
                            .now
                            .iter()
                            .find(|was| was.kind == hit.kind)
                            .map(|was| (was.base + was.bonus).floor());
                        match before {
                            Some(before) => out.push_str(&format!(
                                "  {}: {:.0} now, {after:.0} then — a GAIN OF {:.0}. That last \
                                 number is what the change is worth; the others are totals.\n",
                                hit.kind,
                                before,
                                after - before
                            )),
                            None => out.push_str(&format!("  {}: {after:.0}\n", hit.kind)),
                        }
                    }
                }
                out.push_str(
                    "\nGive them the gain. Do not report a total as though it were the \
                     improvement, and do not add the gains of two different attributes together \
                     — they are alternatives, not a sum.\n",
                );
                return Ran {
                    output: out,
                    note: Some(format!("At other attributes · {}", at[0].name)),
                    source: Some("the installed game's own tables".into()),
                };
            }

            let Some(found) = (player.weapon)(name) else {
                return Ran {
                    output: "The installed tables could not be read, so there are no figures for \
                             this game. Say the wiki's are the base game's if a total conversion \
                             is installed."
                        .into(),
                    note: None,
                    source: None,
                };
            };
            // Armour lives in its own table, so a question about a robe reaches
            // here having matched no weapon. Asked what their armour weighed, a
            // model took that for "nothing found", looked each piece up four
            // more ways and then searched the web twice before failing.
            if found.is_empty() && !name.is_empty() {
                if let Some(worn) = (player.armour)(name).filter(|worn| !worn.is_empty()) {
                    let mut out = String::from("Out of the tables this installation runs on:\n");
                    for piece in &worn {
                        out.push_str(&format!("\n{} — {:.1} weight\n", piece.name, piece.armour.weight));
                        if !piece.armour.negation.is_empty() {
                            let parts: Vec<String> = piece
                                .armour
                                .negation
                                .iter()
                                .map(|(kind, stopped)| format!("{kind} {stopped:.1}%"))
                                .collect();
                            out.push_str(&format!("  Damage negation: {}\n", parts.join(", ")));
                        }
                        if !piece.armour.resistance.is_empty() {
                            let parts: Vec<String> = piece
                                .armour
                                .resistance
                                .iter()
                                .map(|(what, value)| format!("{what} {value}"))
                                .collect();
                            out.push_str(&format!("  Resistance: {}\n", parts.join(", ")));
                        }
                        // The figure heavy armour is actually worn for, and the
                        // one the launcher could not read until now.
                        if let Some(poise) = piece.armour.poise {
                            out.push_str(&format!(
                                "  Poise: {poise:.2} — the four worn pieces add up, and the \
                                 screen rounds the total. Give this as read; do not round it \
                                 yourself and do not compare it to a number from a wiki, which \
                                 is the base game's.\n"
                            ));
                        }
                        // What the piece's own effect grants. Armour in this
                        // conversion gives attributes — 836 of 841 pieces carry
                        // something — and none of it was read until now, which
                        // is how an answer came to invent a "+2 Faith" set.
                        let mut does: Vec<String> = piece
                            .armour
                            .gives
                            .iter()
                            .chain(piece.armour.adds.iter())
                            .map(|(what, value)| format!("{what} {value:+}"))
                            .collect();
                        does.extend(
                            piece.armour.changes.iter().map(|(what, rate)| {
                                format!("{what} {:+.0}%", (rate - 1.0) * 100.0)
                            }),
                        );
                        if does.is_empty() {
                            out.push_str(
                                "  Gives nothing beyond the figures above: its effect slots are \
                                 empty. Say so rather than supplying a bonus.\n",
                            );
                        } else {
                            out.push_str(&format!("  Gives: {}\n", does.join(", ")));
                        }
                    }
                    // Added here rather than left to the model. Asked what a
                    // whole set weighs, it fetched the four pieces one at a
                    // time and then had to add them itself, which is the step
                    // that goes wrong and the one the tables can do exactly.
                    if worn.len() > 1 {
                        let total: f32 = worn.iter().map(|piece| piece.armour.weight).sum();
                        // The lookup hands back at most four, which is a set,
                        // so this is the set total whenever the name was one.
                        out.push_str(&format!(
                            "\nThe {} pieces above weigh {total:.1} together.\n",
                            worn.len()
                        ));
                    }
                    out.push_str(
                        "\nNegation is the percentage stopped, as the menu shows it, and it is \
                         one piece's. Do NOT add the negations up or average them: pieces do not \
                         combine that way and the combined figure is not worked out here. Their \
                         equipment screen shows it.\n\
                         \n\
                         The weight total above is already added up — use it as it stands rather \
                         than adding anything again. Do NOT go on to say what they can carry, how \
                         much is left, or what fraction of the limit that is: the maximum is not \
                         readable anywhere, and there is no formula for it here. Asked this exact \
                         question, a model added the four pieces correctly and then announced \
                         that endurance 11 gives a limit of 52 and they were at 17% of it, all \
                         three of which it made up. Say what it weighs and that the limit is on \
                         their equipment screen.",
                    );
                    if worn.iter().any(|piece| piece.modded) {
                        out.push_str(
                            " They came from the total conversion's own tables, so they override \
                             every wiki.",
                        );
                    }
                    let note = worn
                        .iter()
                        .map(|piece| piece.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Ran {
                        output: out,
                        note: Some(format!("Numbers · {note}")),
                        source: Some("the installed game's own tables".into()),
                    };
                }
            }

            // A talisman is gear and gets asked for here, but what one does is
            // not a number in a table — it is a special effect, and the game's
            // own words for it are the description. Answering "no figures" sent
            // the model round the wikis and the web and it never came back.
            if found.is_empty() && !name.is_empty() {
                if let Some(known) = (player.catalogue)(name).filter(|found| !found.is_empty()) {
                    let mut out = String::from(
                        "Not in the weapon or armour tables — but this is what the game itself \
                         says about it, in the player's own language and with whatever a total \
                         conversion changed:\n",
                    );
                    for item in known.iter().take(3) {
                        out.push_str(&format!("\n{} ({})\n", item.name, item.what));
                        if let Some(effect) = &item.effect {
                            out.push_str(&format!("  {effect}\n"));
                        }
                        if let Some(caption) = &item.caption {
                            out.push_str(&format!("  {caption}\n"));
                        }
                    }
                    out.push_str(
                        "\nThat is the game's own text, so it is right for their installation. \
                         There are no attack or weight figures for this kind — do not go looking \
                         for them elsewhere and do not supply any.",
                    );
                    return Ran {
                        output: out,
                        note: Some(format!("In the game · {name}")),
                        source: Some("the running game".into()),
                    };
                }
            }

            if found.is_empty() {
                let what = if name.is_empty() {
                    "Nothing they are holding has a row in the weapon tables — a torch or a \
                     seal will not. Ask player_status what they have and name it, or say you \
                     could not find figures rather than supplying any."
                        .to_string()
                } else {
                    // Before saying it is not in the game, look for it under
                    // the name the game actually prints. Asked which of two
                    // katanas hits harder, a model translated both into English
                    // to ask for figures, got nothing twice, and answered from
                    // their flavour text — while the very same names in Russian
                    // found both items immediately. "Not in this game" was
                    // false and it was the reason the answer gave up.
                    let nearby: Vec<String> = (player.catalogue)(name)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|hit| hit.what == "weapon" || hit.what == "armour")
                        .map(|hit| hit.name)
                        .take(4)
                        .collect();
                    if nearby.is_empty() {
                        // Why it missed, when the reason is visible. Asked which
                        // of two katanas hits harder, a model translated both
                        // names into English, missed twice, and concluded the
                        // "rotten" versions "do not exist in the base game" —
                        // about two weapons that are in this one, under names
                        // written in a different alphabet.
                        let asked_in_latin =
                            name.chars().any(|c| c.is_ascii_alphabetic());
                        let game_is_not = player
                            .language
                            .as_deref()
                            .is_some_and(|tongue| !tongue.eq_ignore_ascii_case("english"));
                        let script = if asked_in_latin && game_is_not {
                            format!(
                                " \n\nAND LOOK AT WHY. Their game is in {}, so every name in \
                                 these tables is written in it — and \"{name}\" is in the Latin \
                                 alphabet. That is almost certainly the whole problem, not a \
                                 missing item. Find the name their game prints (the catalogue \
                                 and their own equipment will both give it) and ask again with \
                                 THAT. Do not tell them the item does not exist on the strength \
                                 of a search in the wrong language.",
                                player.language.as_deref().unwrap_or("another language")
                            )
                        } else {
                            String::new()
                        };
                        format!(
                            "Nothing called \"{name}\" in this installation's weapon or armour \
                             tables, and nothing like it in the catalogue either. If they are \
                             wearing or holding it, use the name player_status gave.\n\
                             \n\
                             AND CHECK IT IS THE RIGHT TABLE. This one holds weapons and \
                             armour and nothing else. A SPELL is not in here however it is \
                             spelled — spell_numbers has the sorceries and incantations with \
                             their FP and their stamina — nor is a talisman, a spirit ash, a \
                             crystal tear or a consumable, and each of those has a tool of its \
                             own. Asked what a lightning incantation cost in stamina, a model \
                             asked this twice, in two languages, and got nothing both \
                             times.{script}"
                        )
                    } else {
                        format!(
                            "Nothing is called \"{name}\" in the weapon or armour tables — but \
                             the catalogue has {}. That is almost certainly the same thing \
                             under the name their game prints, and the tables are keyed on \
                             THAT. Ask again with it, exactly as written. Do NOT conclude the \
                             item is missing and do NOT answer from its description: the \
                             figures are there under the right name.",
                            nearby
                                .iter()
                                .map(|near| format!("\"{near}\""))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                };
                return Ran {
                    output: what,
                    note: Some(format!("No figures · {name}")),
                    source: None,
                };
            }

            let listed = |what: &str, of: &[(String, String)]| -> String {
                if of.is_empty() {
                    String::new()
                } else {
                    let parts: Vec<String> = of.iter().map(|(k, v)| format!("{k} {v}")).collect();
                    format!("  {what}: {}\n", parts.join(", "))
                }
            };

            // Their own attributes, so whether they can hold the thing is
            // settled here rather than left to arithmetic in a sentence. Asked
            // to compare two weapons, a model read "requires dexterity 18",
            // read "dexterity 14" a line above it, and told the player they met
            // the requirement — then recommended the weapon.
            let reading = player.live.as_ref().and_then(|read| read());
            let theirs = reading.as_ref().map(|live| live.stats.clone());
            // What they are actually carrying, so a question about "my sword"
            // can be checked against it. Asked why "my Blasphemous Blade" hit
            // so softly, the model looked the weapon up, found the figures, and
            // explained the player's scaling on a greatsword they do not own —
            // three lanes in a row, because the tool handed over real numbers
            // and nothing alongside them said whose they were.
            let held: Vec<String> = reading
                .as_ref()
                .and_then(|live| live.gear.as_ref())
                .map(|gear| gear.weapons.iter().map(|name| name.to_lowercase()).collect())
                .unwrap_or_default();

            let modded = found.iter().any(|armed| armed.modded);

            // The standing notes go FIRST and the figures last, and that
            // ordering is the whole of the fix. A weak lane finished its answer
            // by translating the tail of this output into Russian and handing
            // it to the player — "это то, что они видят на своём экране;
            // скажите это..." — because the last thing it read was an
            // instruction rather than a number. Notes that belong BESIDE a
            // figure still sit beside it; only this closing paragraph moved,
            // and now the output ends in data.
            let mut out = String::new();
            out.push_str(
                "Notes before the figures, none of which is for the player to see:\n\
                 - These are the base figures at no upgrade. Their stat screen shows higher, \
                 because the game multiplies by the upgrade level and adds what the scaling \
                 contributes — present them as the base, not as what they will see.\n",
            );
            if modded {
                out.push_str(
                    "- They come from the total conversion's own tables, so they override every \
                     wiki. Where the two disagree, this is what their game does.\n",
                );
            }
            out.push_str("\nOut of the tables this installation runs on:\n");
            for armed in &found {
                let numbers = |of: &[(String, u16)]| -> Vec<(String, String)> {
                    of.iter().map(|(k, v)| (k.clone(), v.to_string())).collect()
                };
                let needs = |of: &[(String, u8)]| -> Vec<(String, String)> {
                    of.iter().map(|(k, v)| (k.clone(), v.to_string())).collect()
                };
                // Scaling is kept in hundredths and shown as a letter, so give
                // both — a player told "faith 65" has nothing to compare.
                let scaling: Vec<(String, String)> = armed
                    .weapon
                    .scaling
                    .iter()
                    .map(|(what, value)| (what.clone(), format!("{value:.0} ({})", grade(*value))))
                    .collect();

                out.push_str(&format!("\n{}\n", armed.name));
                // Said before the figures, because the figures are what carries
                // the assumption. Only when the game is open and did tell us
                // what is in their hands — with it closed, `held` is empty and
                // nothing is claimed either way.
                if !held.is_empty() {
                    let lowered = armed.name.to_lowercase();
                    let carrying = held
                        .iter()
                        .any(|mine| mine.contains(&lowered) || lowered.contains(mine.as_str()));
                    if !carrying {
                        out.push_str(&format!(
                            "  THEY ARE NOT HOLDING THIS. In their hands right now: {}. If they \
                             called it theirs, say that first — it is in storage, or they mean \
                             another character, or they have the wrong name. Do not explain \
                             their scaling on it as though they were swinging it.\n",
                            held.join(", ")
                        ));
                    }
                }
                out.push_str(&listed("Damage", &numbers(&armed.weapon.damage)));
                // What it actually hits for in their hands. The line above is
                // the table's base and does not match their screen: this
                // weapon's row says 82 fire and the player is looking at
                // 106 + 49. Quoting the base at somebody reading the other
                // number is how a right figure becomes a wrong answer.
                if !armed.hits.is_empty() {
                    let parts: Vec<String> = armed
                        .hits
                        .iter()
                        .map(|hit| {
                            format!(
                                "{} {:.0} + {:.0}",
                                hit.kind,
                                hit.base.floor(),
                                hit.bonus.floor()
                            )
                        })
                        .collect();
                    // Stated, not ordered. A sentence phrased as an instruction
                    // to the answerer is a sentence that gets answered with: a
                    // weak lane translated this one into Russian and handed it
                    // to the player as the last line of its reply. The same
                    // fact, in the indicative, is nothing to copy.
                    out.push_str(&format!(
                        "  ON THEIR SCREEN, at their attributes: {}\n\
                             (first number: the weapon upgraded. Second: what their own \
                         attributes add. This pair is the one on their screen; the Damage line \
                         above is what the weapon is worth to anybody.)\n",
                        parts.join(", ")
                    ));
                }
                out.push_str(&listed("Scaling", &scaling));
                out.push_str(&listed("Requires", &needs(&armed.weapon.needs)));
                out.push_str(&short_of(&armed.weapon.needs, theirs.as_deref()));
                out.push_str(&format!("  Weight: {:.1}\n", armed.weapon.weight));
                if let Some(hp) = armed.weapon.regain {
                    out.push_str(&format!("  Returns {hp} health on a hit\n"));
                }
                // What it stops when raised. Here because it was nowhere, and
                // the gap got filled in: asked whether the installation has a
                // shield blocking 100% of physical, a model answered "no" out
                // of the armour ranking, which holds no shields. It has 334.
                if let Some(blocks) = &armed.weapon.blocks {
                    let listed: Vec<String> =
                        blocks.iter().map(|(what, value)| format!("{what} {value:.0}%")).collect();
                    out.push_str(&format!(
                        "  Blocking with it stops: {}. Those are the five their own menu prints \
                         under blocking, read from the tables and exact.\n\
                         How much STAMINA a blocked hit costs is the other half of choosing a \
                         shield and is not read here — that is guard boost, and their menu shows \
                         it. Do not give a figure for it.\n",
                        listed.join(", ")
                    ));
                }
                // Said once, beside the figures, because its absence is what
                // gets filled in. Asked what inflicted frostbite, a model
                // answered that their dagger did fifty of it — the tables carry
                // no such number and the dagger carries no such status.
                // Four of the seven are read now, through the effect the weapon
                // hangs on itself. The other three are not, and saying which is
                // which is the whole point — "buildup is not read" was true of
                // all seven and is now a lie about four of them.
                if armed.weapon.ailments.is_empty() {
                    out.push_str(
                        "  It builds up no poison, rot, bleed or curse — those four are read \
                         and this weapon carries none of them.\n",
                    );
                } else {
                    let listed: Vec<String> = armed
                        .weapon
                        .ailments
                        .iter()
                        .map(|(what, value)| format!("{what} {value}"))
                        .collect();
                    out.push_str(&format!("  Builds up per hit: {}\n", listed.join(", ")));
                }
                out.push_str(
                    "  All seven ailments are read — poison, rot, bleed, curse, frost, sleep, \
                     madness — so a weapon with none of them listed above builds none of them. \
                     That was not true until recently: three of the seven could not be read and \
                     an answer had to say so. Anything claiming this launcher cannot see frost \
                     is out of date.\n",
                );
                if let Some((skill, costs)) = &armed.skill {
                    let priced: Vec<String> =
                        costs.iter().map(|(button, fp)| format!("{button} {fp} FP")).collect();
                    out.push_str(&format!("  Skill: {skill}"));
                    if priced.is_empty() {
                        // Said rather than left blank: a skill with no cost is
                        // a real answer and an invited guess otherwise.
                        out.push_str(" (costs no FP)\n");
                    } else {
                        out.push_str(&format!(" — {}\n", priced.join(", ")));
                    }
                }
            }
            // The figures are the answer, not the raw material for an opinion.
            // Asked which of two katanas hits harder, a model fetched both —
            // correctly, after being redirected to their Russian names — and
            // then wrote that one had "higher base damage and slightly better
            // scaling", with no number in the sentence, and finished with
            // requirements it had not read. Both sets of figures were sitting
            // in front of it.
            out.push_str(
                "\nQUOTE THESE NUMBERS. If they asked which of two things is better, give both \
                 figures and the difference — \"143 against 128\" — rather than saying one is \
                 higher, and take the requirements from the lines above rather than from what \
                 a weapon of that name usually asks for. An answer that has the numbers and \
                 describes them instead has thrown away the only thing here a wiki could not \
                 have told them.\n",
            );

            let note = found
                .iter()
                .map(|armed| armed.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Ran {
                output: out,
                note: Some(format!("Numbers · {note}")),
                source: Some("the installed game's own tables".into()),
            }
        }

        "catalogue" => {
            let asked_for = as_a_name(&args, "kind");
            // "armor" and "talismans" are the same request as the words on the
            // list; refusing them is a right answer to the wrong question.
            let kind = crate::library::as_a_kind(&asked_for).unwrap_or(asked_for.as_str());
            let searching = as_a_name(&args, "search");
            let search = searching.as_str();

            // A kind nobody has matches nothing, and nothing reads as "the game
            // does not have that". Asked which spirit summons the player had, a
            // model narrowed by a kind that is not one and was told there were
            // none — the game files them under items, and there are dozens.
            if !kind.is_empty() && !crate::library::is_a_kind(kind) {
                return Ran {
                    output: format!(
                        "\"{kind}\" is not one of the kinds this can narrow by, so nothing was \
                         searched at all — this is NOT the game saying it has no such thing. \
                         The kinds are: {}. Anything without a kind of its own is filed under \
                         \"item\", spirit summons among them. Ask again with one of those, or \
                         with no kind at all and just a word.",
                        crate::library::kinds()
                    ),
                    note: Some(format!("No such kind · {kind}")),
                    source: None,
                };
            }

            let found = (player.catalogue_of)(kind, search);
            if found.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing in this installation's own text matches \"{search}\"{}.\n\
                         \n\
                         That is one word, and one word is not enough to conclude anything. \
                         SEARCH AGAIN with a different one before you say the game has no such \
                         thing: the game is in its own language and its word may not be the one \
                         you translated. Asked which spirit summons the player had, a model \
                         searched the Russian for \"spirit\", found nothing, and gave up — the \
                         game calls them ashes. Try the other word for the same idea, and try \
                         part of a word rather than a whole one.\n\
                         \n\
                         IN THEIR LANGUAGE, WHICH IS THE ONE NAMED IN THE BLOCK ABOVE. Every \
                         name in these tables is written the way their copy writes it, so a word \
                         from another language matches nothing here however right it looks. Do \
                         not hand them one either: an answer told a player with a Russian game \
                         to look for names containing \"Bloody\" and \"Hemorrhage\", neither of \
                         which appears anywhere in their installation.\n\
                         \n\
                         If a second word finds nothing either, then it is not in their game \
                         under any of them — say that, rather than reaching for a wiki's English \
                         name and guessing at what it means. And do not name a thing that was \
                         not in a result: asked for bleed recipes, a model was handed a hundred \
                         and fifteen real matches, decided they were not enough, and finished by \
                         inventing three item names out of nothing.\n\
                         \n\
                         AND THE THING THIS SEARCH CANNOT DO. It matches NAMES and the game's \
                         own description text. It cannot find things by a property, because the \
                         properties are not in what it searches: nothing here can list \"weapons \
                         that cause bleed\", or frost, poison, rot, sleep or madness, because \
                         this launcher does not read status buildup at all — not from the \
                         weapon's row, where it measurably is not, and not from the table it is \
                         actually kept in, which is not opened. Finding nothing for one of those \
                         is this launcher being blind to it, NOT the game lacking it.\n\
                         \n\
                         Say that, in those words. Asked which weapons in this installation are \
                         good for bleed, an answer reported that the catalogue gave no results \
                         and then filled the silence with remembered Dark Souls, a recommendation \
                         to read a forum, and a weapon that does not exist. Their own menu shows \
                         buildup on every weapon; that is the honest place to send them.",
                        if kind.is_empty() { String::new() } else { format!(" among the {kind}s") }
                    ),
                    note: Some(format!("Nothing named · {search}")),
                    source: Some("the installed game's own text".into()),
                };
            }

            // Long lists are the enemy of a good answer, and an unnarrowed
            // search over eleven thousand things is a long list.
            let most = 40;
            let shown = found.len().min(most);
            let mut out = format!(
                "{} of this installation's own things match{}. This is the CATALOGUE — what the \
                 game contains, not what they own. Their inventory is not readable anywhere in \
                 this launcher, so never say they have these, never count them as theirs, and \
                 never open with \"in your build you have\": asked which spirit summons they \
                 had, a model listed four real ones as though they were in their pouch.\n",
                found.len(),
                if search.is_empty() { String::new() } else { format!(" \"{search}\"") }
            );
            for one in found.iter().take(most) {
                out.push_str(&format!("  [{}] {}", one.what, one.name));
                if let Some(says) = &one.effect {
                    out.push_str(&format!(" — {says}"));
                }
                out.push('\n');
            }
            if found.len() > shown {
                out.push_str(&format!(
                    "\nThat is the first {shown} of {}. Search a narrower word rather than \
                     guessing at the rest.\n",
                    found.len()
                ));
            }
            out.push_str(
                "\nThese are names and the line the menu prints, out of their own game. Do not \
                 add an effect that is not written here, and do not translate a name — that is \
                 what is on their screen.\n\
                 \n\
                 NO FIGURE IS IN HERE. A name and the one line the menu prints beside it, and \
                 nothing else — no FP cost, no damage, no weight, no requirement. Anything of \
                 that shape has a tool of its own that reads it exactly. Asked which healing \
                 incantation they could use, an answer took a catalogue hit and wrote \"20 FP, \
                 from the mod's tables\"; the tables say 8, and the tables were never asked. \
                 Attributing a figure to them makes it worse, not better.\n\
                 \n\
                 READ THE WHOLE NAME, not the word that matched. One word names unrelated \
                 things and the kind in brackets will not separate them, because both are \
                 items: asked which SPIRIT ASH suited them, an answer searched the word for \
                 \"ash\", got back a hundred and twenty-seven matches and recommended an ASH OF \
                 WAR — a skill that goes on a weapon, which is not a summon and cannot be one. \
                 If the matches are a mixture, say which kind you are picking from and pick \
                 from that kind only.\n",
            );
            out.push_str(&onward_from(found.iter().take(most).map(|one| one.what.as_str())));

            Ran {
                output: out,
                note: Some(format!("Catalogue · {} match", found.len())),
                source: Some("the installed game's own text".into()),
            }
        }

        "upgrade_path" => {
            let asked = as_a_name(&args, "name");
            let name = asked.as_str();

            let found = (player.upgrading)(name);
            if found.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing called \"{name}\" could be matched to a weapon in this \
                         installation, so its upgrade path could not be read. Do not answer \
                         from memory — the materials in a total conversion are not the base \
                         game's. Ask which weapon they mean, or say it could not be found."
                    ),
                    note: Some(format!("No upgrade path · {name}")),
                    source: None,
                };
            }

            let mut out = String::from("Out of the tables this installation runs on:\n");
            for climb in &found {
                if climb.steps.is_empty() {
                    out.push_str(&format!(
                        "\n{} cannot be upgraded at all — it has no path in the tables. That is \
                         the answer, not a failure to find one.\n",
                        climb.weapon
                    ));
                    continue;
                }
                let top = climb.steps.last().map_or(0, |step| step.0);
                out.push_str(&format!("\n{} goes to +{top}:\n", climb.weapon));
                // The count is always written, even when it is one. Left off,
                // the model read the number in a material's own name as the
                // quantity — "Кузнечный камень мрака [3]" became "x3 per level"
                // and "[5]" became "x5", so a player was told to bring three
                // and five times what a level actually costs. There is no gap
                // to fill if the count is already there.
                for (level, costs) in &climb.steps {
                    let parts: Vec<String> = costs
                        .iter()
                        .map(|(item, count)| format!("{count} x {item}"))
                        .collect();
                    out.push_str(&format!("  +{level} needs: {}\n", parts.join(", ")));
                }
            }
            if found.iter().any(|climb| climb.modded) {
                out.push_str(
                    "\nThese came from the total conversion's own tables, so they override every \
                     wiki and every memory of the base game.\n",
                );
            }
            out.push_str(
                "\nEach line is ONE level and the number before the material is how many of it \
                 that single level takes. A number inside a material's name is part of the name, \
                 not a quantity. Levels that want the same thing may be given as a range, but \
                 never multiply the count by how many levels are in it — say what one level \
                 costs.\n\
                 \n\
                 Runes are also charged per level and are NOT in here — do not state a rune \
                 cost. Where each material is found is not in here either: that is the wiki's, \
                 and say so if you go there for it.\n",
            );

            Ran {
                output: out,
                note: Some(format!("Upgrading · {}", found[0].weapon)),
                source: Some("the installed game's own tables".into()),
            }
        }

        "what_drops_here" => {
            let held = as_a_name(&args, "map");
            let map = held.as_str();
            let haul = (player.haul)(map);
            if haul.is_empty() {
                return Ran {
                    output: "Nothing standing on that map drops anything the tables give odds \
                             for — either there is no such map, or the things on it give runes \
                             and nothing else. Say that. A wiki's drop list is the base game's \
                             and a total conversion rewrites them, so it is not the answer here."
                        .into(),
                    note: None,
                    source: None,
                };
            }

            let mut out = format!(
                "What the things standing on {} give, out of this installation's own tables. \
                 MOST PLENTIFUL FIRST — this order already answers \"what drops most often\":\n",
                if map.is_empty() { "the map they are on" } else { map }
            );
            for one in haul.iter().take(20) {
                out.push_str(&format!(
                    "  {} [{}] — about {:.1} per clear, best odds {:.0}% off {} of them\n",
                    one.what, one.kind, one.expect, one.chance, one.from
                ));
            }
            if haul.len() > 20 {
                out.push_str(&format!("  … and {} more kinds\n", haul.len() - 20));
            }
            out.push_str(
                "\n\"Per clear\" is how many to expect from killing everything on the map once: \
                 every source's own odds added up. It is the figure that answers what falls \
                 most, and it is NOT the percentage — three things at 100% off one creature \
                 each are rarer than a feather at 62% off thirty-nine, which was exactly the \
                 wrong answer given before this line existed.\n\
                 \n\
                 The percentage is the best any ONE of them gives it at. Quote both as they \
                 stand: never add them together, never turn a 3% into \"fairly often\", and \
                 never call a per-clear figure a chance.\n\
                 \n\
                 These are the odds for THEIR installation. Do not check them against a wiki and \
                 do not mention the base game's, which are about a different game once a total \
                 conversion is installed.\n",
            );

            Ran {
                output: out,
                note: Some(format!("Drops here · {}", haul.len())),
                source: Some("the installed game's own tables".into()),
            }
        }

        "whos_here" => {
            let held = as_a_name(&args, "map");
            let map = held.as_str();
            let here = (player.dwellers)(map);
            if here.is_empty() {
                return Ran {
                    output: "No named thing could be read for that map — either there is no such \
                             map, or nothing on it has a name of its own. Say the names could \
                             not be read; do not supply one from memory, because a total \
                             conversion renames its bosses."
                        .into(),
                    note: None,
                    source: None,
                };
            }

            // What they are carrying, so the multiplication can be done here
            // rather than described. A resistance is only half an answer: told
            // a thing takes 60% of fire, a reader still has to know their
            // weapon deals fire and work out what that leaves, and that is the
            // step that goes wrong.
            let swinging: Vec<(String, Vec<(String, f32)>)> = player
                .live
                .as_ref()
                .and_then(|read| read())
                .and_then(|live| live.gear)
                .map(|gear| {
                    gear.weapon_ids
                        .iter()
                        .filter_map(|(name, id)| {
                            let weapon = (player.weigh)(*id)?;
                            let deals: Vec<(String, f32)> = weapon
                                .damage
                                .iter()
                                .map(|(kind, value)| (kind.clone(), f32::from(*value)))
                                .collect();
                            (!deals.is_empty()).then(|| (name.clone(), deals))
                        })
                        .collect()
                })
                .unwrap_or_default();

            let by_name = !map.is_empty() && !crate::bestiary::is_a_map(map);
            let mut out = if by_name {
                format!(
                    "Everything the world calls something like \"{map}\", out of the map files \
                     and this installation's own tables. The map each one stands on is given, \
                     and it is the answer to \"where is it\":\n"
                )
            } else {
                // "Standing on", not "enemies on". The list is everything the
                // map holds that has a name, and the framing is set here
                // because it is read before any of the names are: said at the
                // foot of this result instead, the warning was followed by one
                // lane and ignored by another, which opened with "dangerous
                // enemies nearby" and gave a merchant's health and how much the
                // player's dagger would take off it.
                format!(
                    // Lower case, and said as a sentence rather than a label.
                    // Shouted as "Everything NAMED standing on…", the word came
                    // straight back out in a Russian answer — "Из Named
                    // противников поблизости" — because a capitalised word in
                    // the middle of a result reads as a term to quote.
                    "Everything that has a name, standing on {}, out of the map files and this \
                     installation's own tables, the richest first. This is not a list of \
                     enemies: merchants, questgivers and bystanders are in here on the same \
                     footing, and nothing in these tables says which of them is hostile. The \
                     nameless things standing about are not in here either.\n",
                    if map.is_empty() { "the map they are on" } else { map }
                )
            };
            for one in here.iter().take(12) {
                let whereabouts =
                    if by_name { format!(" on {}", one.map) } else { String::new() };
                match one.hp {
                    Some(hp) => out.push_str(&format!(
                        "  {}{whereabouts} — {} runes, {hp} HP\n",
                        one.name, one.runes
                    )),
                    // Said plainly rather than left out, or the next reader
                    // fills the gap from memory, which is the whole problem.
                    None => out.push_str(&format!(
                        "  {}{whereabouts} — {} runes, health not in the tables\n",
                        one.name, one.runes
                    )),
                }
                if !one.takes.is_empty() {
                    // Which way round, said in words. A bare "magic 60%" was
                    // read as the thing's weakest point when it is its
                    // strongest: asked what it holds up worst against, a model
                    // answered magic, which is the one kind it shrugs off.
                    let parts: Vec<String> = one
                        .takes
                        .iter()
                        .map(|(kind, pc)| {
                            let how = if *pc > 100.0 { "WEAK TO" } else { "resists" };
                            format!("{how} {kind} ({pc:.0}% of it lands)")
                        })
                        .collect();
                    out.push_str(&format!("      {}\n", parts.join("; ")));
                    out.push_str(
                        "      every other kind lands in full — that is the ordinary amount and \
                         NOT a weakness, though it still beats anything on the list it resists\n",
                    );
                    // And what that leaves of what they are actually swinging,
                    // multiplied here. The base damage each kind does, times
                    // this thing's own rate for that kind.
                    for (weapon, deals) in &swinging {
                        let against: Vec<String> = deals
                            .iter()
                            .map(|(kind, base)| {
                                let rate = one
                                    .takes
                                    .iter()
                                    .find(|(what, _)| what == kind)
                                    .map_or(100.0, |(_, pc)| *pc);
                                format!("{kind} {:.0}", base * rate / 100.0)
                            })
                            .collect();
                        out.push_str(&format!(
                            "        their {weapon} would land {}\n",
                            against.join(", ")
                        ));
                    }
                }
                // Nearly always empty, and worth printing on the rare occasion
                // it is not: a named thing that really does roll for a drop.
                for (what, count, chance) in one.drops.iter().take(6) {
                    let many = if *count > 1 { format!(" ×{count}") } else { String::new() };
                    out.push_str(&format!("      drops {what}{many} — {chance:.0}%\n"));
                }
            }
            out.push_str(
                "\nRichest first is the only ordering there is: nothing in the tables marks one \
                 of these as the boss. The top of the list usually is one, and saying \"the \
                 biggest thing here\" is honest where calling it the area boss is a guess.\n\
                 \n\
                 These are the names their own game prints, so use them exactly. Where each one \
                 stands beyond the map is not worth quoting — the numbers are the world's own \
                 units and mean nothing to a player.\n\
                 \n\
                 \"Resists X\" and \"WEAK TO X\" are worked out here, from that thing's own \
                 rates, and the percentage beside each is how much of that kind LANDS. Read the \
                 word, not the number: 60% of it landing is a thing shrugging magic off, and it \
                 was once reported as the thing's weakest point. Anything not listed lands in \
                 full, which makes an unlisted kind a better choice than anything it resists. \
                 This is the ONLY place any of it is readable, and a wiki's weakness table is \
                 the base game's.\n\
                 \n\
                 \"Their WEAPON would land\" is that rate already applied to the weapon's own \
                 base damage — the arithmetic is done, so read it off rather than working \
                 anything out. It is the answer to \"what should I hit this with\": the biggest \
                 figure wins. Do not recommend a kind of damage they are not carrying, and do \
                 not call something a weakness because it sounds like one — a thing that takes \
                 60% of fire is the WORST thing to bring fire to, however much the name \
                 suggests otherwise.\n\
                 \n\
                 Nothing listed under a name is the ordinary case, not a gap: a boss's reward is \
                 scripted rather than rolled, and only 2 named things in the whole world roll \
                 for a drop at all. So do NOT read a bare name as \"drops nothing\" and do not \
                 reach for a wiki to fill it. What can be got on a map comes from \
                 what_drops_here, off the nameless things standing around.\n\
                 \n\
                 \"Health not in the tables\" means exactly that. Those are the human characters, \
                 whose rows all share one template figure that cannot be the health of every one \
                 of them, and the game works out the real one somewhere this launcher does not \
                 read. Say it is not known. A number for one of them is invented however \
                 confident it feels.\n\
                 \n\
                 NOT EVERYTHING STANDING HERE IS AN ENEMY, and nothing in these tables says \
                 which are. Merchants, questgivers and bystanders have health and resistances \
                 like anything else, and reading a list of them as a list of threats has \
                 happened: asked who was dangerous nearby, an answer opened \"dangerous enemies \
                 nearby\" and named a Nomadic Merchant, with its health and the damage their \
                 dagger would do to it. In this game killing a merchant is permanent and takes \
                 the shop with it. So when a name reads as a trader or a character rather than a \
                 creature, say what it is instead of how to fight it — and if they asked what is \
                 DANGEROUS, say plainly that hostility is not something these tables record. \
                 Say that in your own words: an answer that told the player which TOOL had no \
                 note of it named the tool, and they have never heard of it.\n",
            );

            Ran {
                output: out,
                note: Some(format!("Standing here · {}", here.len())),
                source: Some("the game's own map files".into()),
            }
        }

        "spells_by_cost" => {
            // Every spell, by asking the reach lister for more attributes than
            // any character has. It already reads name, FP and requirements —
            // what was missing was only the ranking, and a second reader would
            // have been a second thing to keep in step.
            let all = (player.spells_at)(99, 99, 99);
            if all.is_empty() {
                return Ran {
                    output: "The spell tables could not be read. Say that plainly — do NOT say \
                             the game has to be running, because it does not, and do not name \
                             any spell from memory."
                        .into(),
                    note: Some("No spells read".into()),
                    source: None,
                };
            }
            let school = as_a_name(&args, "school").to_lowercase();
            let wanted = as_a_name(&args, "name").to_lowercase();
            // Which school a spell belongs to is not a field; it is what the
            // spell asks for. One that wants intelligence is a sorcery and one
            // that wants faith is an incantation, which is also how the player
            // thinks of them.
            let asks_for = |cast: &Cast, what: &str| {
                cast.spell.needs.iter().any(|(need, _)| need.contains(what))
            };
            let mut matching: Vec<&Cast> = all
                .iter()
                .filter(|cast| {
                    if school.starts_with("sorc") || school.contains("закл") || school.contains("int") {
                        asks_for(cast, "intelligence")
                    } else if school.starts_with("incant") || school.contains("молит") || school.contains("faith") {
                        asks_for(cast, "faith")
                    } else {
                        true
                    }
                })
                .filter(|cast| wanted.is_empty() || cast.name.to_lowercase().contains(&wanted))
                .collect();
            if matching.is_empty() {
                return Ran {
                    output: format!(
                        "None of this installation's {} spells matches that. Say so rather than \
                         naming one that is not here.",
                        all.len()
                    ),
                    note: Some(format!("No spell · {school}{wanted}")),
                    source: Some("the installed game's own tables".into()),
                };
            }
            matching.sort_by_key(|cast| cast.spell.fp);
            // BOTH ENDS, and this was a real fault rather than a nicety.
            //
            // The listing was the cheapest twenty-five and nothing else, on the
            // reasoning that "which is cheapest" was the question it was built
            // for. Asked instead for the most EXPENSIVE incantation, a model
            // read the only list it had and answered "Сифон Духа, 10 FP" — the
            // dearest of the twenty-five cheapest, with real conviction, while
            // the same battery had already seen a 90 FP spell. A tool that
            // answers half the question and looks like it answered all of it is
            // worse than one that refuses.
            //
            // Fifteen each way rather than twenty-five one way: the same size,
            // and it makes the top end reachable without a second call. The
            // middle is dropped on purpose — nobody asks which spell is
            // thirtieth cheapest, and saying the list is cut is what keeps that
            // honest.
            const EACH_END: usize = 15;
            let describe = |cast: &Cast| {
                let needs: Vec<String> = cast
                    .spell
                    .needs
                    .iter()
                    .map(|(what, value)| format!("{what} {value}"))
                    .collect();
                format!(
                    "  {} — {} FP, {} slot{}, asks {}\n",
                    cast.name,
                    cast.spell.fp,
                    cast.spell.slots,
                    if cast.spell.slots == 1 { "" } else { "s" },
                    if needs.is_empty() { "nothing".into() } else { needs.join(", ") }
                )
            };

            // Lead with the end they asked about.
            //
            // Both ends being present was not enough. Asked for the DEAREST
            // incantation, a model read the first section it met and answered
            // "Династическое таинство, которое стоит 0 FP" — calling the
            // cheapest spell in the game the most expensive, in one
            // self-contradicting sentence. The data was right there under the
            // next heading.
            //
            // So the order follows the question, the way the armour ranking
            // already reads `player.asked` rather than trusting what the model
            // passed. Getting this wrong costs nothing: both ends are printed
            // either way, so a missed word means a worse ordering and never a
            // missing answer.
            let asked = player.asked.to_lowercase();
            let wants_dearest = [
                "дорог", "дорож", "больше всего fp", "самая больш",
                "most expensive", "dearest", "priciest", "highest fp",
                "mas caro", "más caro", "mais caro", "plus cher", "teuerste",
            ]
            .iter()
            .any(|word| asked.contains(word));

            let mut out =
                format!("{} spells in this installation, {} matching.\n\n", all.len(), matching.len());
            let cheapest: Vec<&&Cast> = matching.iter().take(EACH_END).collect();
            let dearest: Vec<&&Cast> = matching.iter().rev().take(EACH_END).collect();
            let missing = matching.len().saturating_sub(EACH_END * 2);
            let gap = if missing > 0 {
                format!(" — the {missing} in between are not shown")
            } else {
                String::new()
            };

            let mut section = |title: &str, spells: &[&&Cast]| {
                out.push_str(title);
                for cast in spells {
                    out.push_str(&describe(cast));
                }
            };
            if wants_dearest && matching.len() > EACH_END {
                section(
                    &format!("DEAREST in FP — this is the end you were asked about{gap}:\n"),
                    &dearest,
                );
                section("\nCHEAPEST in FP, the other end of the same list:\n", &cheapest);
            } else {
                section("CHEAPEST in FP:\n", &cheapest);
                if matching.len() > EACH_END {
                    section(
                        &format!("\nDEAREST in FP, the other end of the same list{gap}:\n"),
                        &dearest,
                    );
                }
            }
            return Ran {
                output: out,
                note: Some(format!("Spells by cost · {} of {}", matching.len(), all.len())),
                source: Some("the installed game's own tables".into()),
            };
        }

        "starting_classes" => {
            let all = (player.classes)();
            if all.is_empty() {
                return Ran {
                    output: "The starting classes could not be read — the game's tables are not \
                             open. Say so; do not name any from memory, and do not answer from \
                             the base game."
                        .into(),
                    note: Some("No classes read".into()),
                    source: None,
                };
            }
            let mut out = format!(
                "The {} classes this installation offers, in the order its menu shows them. \
                 Every class is given the same number of points — the level IS the total, less \
                 the {} everybody starts with — so no start is stronger than another overall, \
                 and what differs is where the points sit and what it is holding. Say that \
                 rather than picking a winner.\n",
                all.len(),
                crate::formats::regulation::class::POINTS_AT_LEVEL_ZERO,
            );
            for class in &all {
                let stats: Vec<String> = class
                    .attributes
                    .iter()
                    .map(|(what, value)| format!("{what} {value}"))
                    .collect();
                out.push_str(&format!(
                    "  {} — level {}: {}\n",
                    class.name,
                    class.level,
                    stats.join(", ")
                ));
                if !class.gear.is_empty() {
                    out.push_str(&format!("      starts with: {}\n", class.gear.join(", ")));
                }
            }
            return Ran {
                output: out,
                note: Some(format!("Starting classes · {}", all.len())),
                source: Some("the installed game's own tables".into()),
            };
        }

        "ashes_of_war" => {
            let wanted = as_a_name(&args, "name").to_lowercase();
            let all = (player.ashes)();
            if all.is_empty() {
                return Ran {
                    output: "The ashes of war could not be read — the game's tables are not \
                             open. Say so; do not name any from memory."
                        .into(),
                    note: Some("No ashes read".into()),
                    source: None,
                };
            }
            let matching: Vec<&WarAsh> = all
                .iter()
                .filter(|ash| wanted.is_empty() || ash.name.to_lowercase().contains(&wanted))
                .collect();
            if matching.is_empty() {
                return Ran {
                    output: format!(
                        "No ash of war is NAMED \"{wanted}\", and this tool searches ash names \
                         only. Whether an ash can go on a given weapon is NOT in this data. If \
                         \"{wanted}\" is a kind of weapon, that is why it missed — do not say that \
                         weapon takes no ash of war; search an effect or an ash's own name. {} \
                         ashes in all.",
                        all.len()
                    ),
                    note: Some(format!("No ash · {wanted}")),
                    source: Some("the installed game's own tables".into()),
                };
            }

            // Cheapest first: the question this exists for is "which is
            // cheapest", and a list in name order does not answer it.
            let mut ranked: Vec<&WarAsh> = matching.clone();
            ranked.sort_by_key(|ash| {
                ash.costs.iter().map(|(_, fp)| *fp).min().unwrap_or(u16::MAX)
            });
            let mut out = format!(
                "{} ashes of war in this installation, {} matching, cheapest first:\n",
                all.len(),
                matching.len()
            );
            let mut free = 0;
            for ash in ranked.iter().take(30) {
                if ash.costs.is_empty() {
                    free += 1;
                    out.push_str(&format!("  {} — costs no FP\n", ash.name));
                } else {
                    let priced: Vec<String> =
                        ash.costs.iter().map(|(button, fp)| format!("{button} {fp} FP")).collect();
                    out.push_str(&format!("  {} — {}\n", ash.name, priced.join(", ")));
                }
            }
            if ranked.len() > 30 {
                out.push_str(&format!("  … and {} more\n", ranked.len() - 30));
            }
            out.push_str(
                "\nThe FP is per press, and a skill with two buttons costs differently for \
                 each. WHICH WEAPONS AN ASH FITS is not in this list and is not read: do not \
                 say an ash can or cannot go on something. Neither is where to find one.\n",
            );
            if free > 0 {
                out.push_str(
                    "An ash listed as costing no FP has no cost in the tables — that is the \
                     figure, not a gap.\n",
                );
            }

            Ran {
                output: out,
                note: Some(format!("Ashes of war · {} of {}", matching.len(), all.len())),
                source: Some("the installed game's own tables".into()),
            }
        }

        "physick_tears" => {
            let wanted = as_a_name(&args, "name").to_lowercase();
            let all = (player.tears)();
            if all.is_empty() {
                return Ran {
                    output: "The crystal tears could not be read — the game's tables are not \
                             open. Say so; do not name any from memory."
                        .into(),
                    note: Some("No tears read".into()),
                    source: None,
                };
            }

            let figures = |tear: &Tear| -> Vec<String> {
                let mut said: Vec<String> = tear
                    .gives
                    .iter()
                    .chain(tear.adds.iter())
                    .map(|(what, value)| format!("{what} {value:+}"))
                    .collect();
                said.extend(
                    tear.changes
                        .iter()
                        .map(|(what, rate)| format!("{what} {:+.0}%", (rate - 1.0) * 100.0)),
                );
                said
            };
            let matching: Vec<&Tear> = all
                .iter()
                .filter(|tear| {
                    wanted.is_empty()
                        || tear.name.to_lowercase().contains(&wanted)
                        || tear.effect.as_deref().is_some_and(|says| {
                            says.to_lowercase().contains(&wanted)
                        })
                        // By the stat it raises, in English, because the names
                        // and descriptions are in the player's language and the
                        // question often is not.
                        || crate::formats::regulation::attribute::named(&wanted)
                            .is_some_and(|attribute| {
                                tear.gives.iter().any(|(what, _)| what == attribute)
                            })
                })
                .collect();
            if matching.is_empty() {
                return Ran {
                    output: format!(
                        "None of this game's {} crystal tears matches \"{wanted}\" — not by \
                         name, not by what it says, and not by the stat it raises. Try another \
                         word before concluding there is none.",
                        all.len()
                    ),
                    note: Some(format!("No tear · {wanted}")),
                    source: Some("the installed game's own tables".into()),
                };
            }

            let mut out = format!(
                "{} crystal tears go in the wondrous physick here. {}:\n",
                all.len(),
                if wanted.is_empty() { "All of them".into() } else { format!("{} match", matching.len()) }
            );
            let mut blank = 0;
            for tear in matching.iter().take(40) {
                let said = figures(tear);
                out.push_str(&format!("  {}", tear.name));
                if said.is_empty() {
                    blank += 1;
                    out.push_str(" — no figure in the tables");
                } else {
                    out.push_str(&format!(" — {}", said.join(", ")));
                }
                out.push('\n');
            }
            if matching.len() > 40 {
                out.push_str(&format!("  … and {} more\n", matching.len() - 40));
            }
            out.push_str(
                "\nTwo tears go in at once and the mixture lasts until they rest or die. A per \
                 cent is against normal, so \"damage taken +15%\" is a PRICE and not a benefit \
                 — read the sign before recommending anything.\n",
            );
            if blank > 0 {
                out.push_str(
                    "Where a tear says \"no figure in the tables\", the launcher genuinely \
                     cannot read what it does — its own description is all there is. Say that; \
                     do not fill the gap from memory of the base game.\n",
                );
            }

            Ran {
                output: out,
                note: Some(format!("Tears · {} of {}", matching.len(), all.len())),
                source: Some("the installed game's own tables".into()),
            }
        }

        "spirit_ashes" => {
            let wanted = as_a_name(&args, "name").to_lowercase();
            let all = (player.spirits)();
            if all.is_empty() {
                return Ran {
                    output: "The spirit ashes could not be read — the game's tables are not \
                             open. Say so; do NOT name any from memory, because a total \
                             conversion adds and removes them and the base game's list is not \
                             theirs."
                        .into(),
                    note: Some("No spirit ashes read".into()),
                    source: None,
                };
            }

            let matching: Vec<&Ash> = all
                .iter()
                .filter(|ash| {
                    wanted.is_empty()
                        || ash.name.to_lowercase().contains(&wanted)
                        || ash
                            .effect
                            .as_deref()
                            .is_some_and(|says| says.to_lowercase().contains(&wanted))
                })
                .collect();
            if matching.is_empty() {
                return Ran {
                    output: format!(
                        "None of this game's {} spirit ashes matches \"{wanted}\", by name or by \
                         what it does. Try a different word before concluding there is none — \
                         and if there is none, say that rather than naming one from the base \
                         game.",
                        all.len()
                    ),
                    note: Some(format!("No spirit ash · {wanted}")),
                    source: Some("the installed game's own tables".into()),
                };
            }

            let mut out = format!(
                "This installation has {} spirit ashes. {} of them{}:\n",
                all.len(),
                matching.len(),
                if wanted.is_empty() { "" } else { " match" }
            );
            for ash in matching.iter().take(40) {
                out.push_str(&format!("  {}", ash.name));
                if ash.summon.upgrades {
                    if let Some(price) = ash.summon.price {
                        out.push_str(&format!(" — upgradable, {price} runes a level"));
                    } else {
                        out.push_str(" — upgradable");
                    }
                } else {
                    out.push_str(" — cannot be upgraded");
                }
                // What upgrading it consumes. Read all along and never shown;
                // see `Ash::material` for why that gap matters.
                if let Some(material) = &ash.material {
                    out.push_str(&format!(", using {material}"));
                }
                if let Some(says) = &ash.effect {
                    out.push_str(&format!("\n      {says}"));
                }
                out.push('\n');
            }
            if matching.len() > 40 {
                out.push_str(&format!("  … and {} more\n", matching.len() - 40));
            }
            out.push_str(
                "\nHOW STRONG ANY OF THESE IS, IS NOT IN HERE, and neither is what it costs to \
                 summon. The cost field reads -1 on every ash in this installation — the \
                 conversion took the figure out of it — and a summon's health and damage are \
                 not read at all. So \"which is the best\" has no answer in these tables. Say \
                 that plainly, give the names, and search the wiki if they want somebody's \
                 opinion. Do NOT rank them from memory: asked exactly this, a model listed five \
                 out of the base game as though they were here.\n",
            );

            Ran {
                output: out,
                note: Some(format!("Spirit ashes · {} of {}", matching.len(), all.len())),
                source: Some("the installed game's own tables".into()),
            }
        }

        "talismans" => {
            let wanted = as_a_name(&args, "name").to_lowercase();

            let all = (player.talismans)();
            if all.is_empty() {
                return Ran {
                    output: "The installed tables could not be read, so there is no list of \
                             talismans. Say so — do not name any from memory, because the ones \
                             you remember belong to the base game and this player may not be \
                             running it."
                        .into(),
                    note: None,
                    source: None,
                };
            }

            // Matched against what it does as well as what it is called. "Which
            // of these helps my faith" is unanswerable from names — a model
            // handed only names went back to memory and invented two, which is
            // the failure this tool exists to stop.
            //
            // Matched on a stem, not the whole word, because the languages this
            // game is played in inflect. Searching the Russian for "вера" found
            // nothing and the answer was that the game has no faith talisman:
            // the descriptions say "веру" and "веры", and a plain substring
            // search misses every one of them.
            //
            // The first attempt at this only trimmed words of five characters
            // or more, which left "вера" — four — untouched and failed in
            // exactly the case that prompted it. The short stat words are the
            // ones people search by, so they are the ones that have to work:
            // one character off anything from four, two off anything from six.
            let length = wanted.chars().count();
            let trim = match length {
                0..=3 => 0,
                4..=5 => 1,
                _ => 2,
            };
            let stem: String = wanted.chars().take(length - trim).collect();
            let mentions = |text: &str| text.to_lowercase().contains(&stem);
            // What the player asked for, as an attribute, if it is one. The
            // name and the description are in THEIR language and the query
            // often is not: asked in Russian what raises arcane, a model
            // searched "arcane" and "sorcery" against Russian text, found
            // nothing twice, and told the player that nothing in the game
            // raises it. Two talismans give arcane +5 and +6, and the figures
            // were being read the whole time — just never matched against.
            let asked_for = crate::formats::regulation::attribute::named(&wanted);
            // What the figures are LABELLED, as well as what they grant. The
            // attribute path fixed "what raises arcane"; this fixes the same
            // question one class over. Asked in Portuguese for the best bleed
            // talisman, a model searched "sangramento" and then "bleed", and
            // was told there is no such talisman in these tables — because the
            // names and descriptions are Russian and the only figures being
            // matched were the nine attributes.
            let also = crate::formats::regulation::resistance::named(&wanted);
            let grants = |charm: &Charm| -> bool {
                charm.figures.as_ref().is_some_and(|figures| {
                    if let Some(attribute) = asked_for {
                        if figures
                            .gives
                            .iter()
                            .any(|(what, value)| what == attribute && *value > 0)
                        {
                            return true;
                        }
                    }
                    // Every figure carries an English label — "bleed resist",
                    // "fire taken", "casting cost" — so match the query against
                    // those directly, and against whatever the four
                    // resistances understood it as.
                    let labelled = |what: &str| {
                        what.contains(&wanted)
                            || also.is_some_and(|named| what.contains(named))
                    };
                    figures.adds.iter().any(|(what, _)| labelled(what))
                        || figures.changes.iter().any(|(what, _)| labelled(what))
                        || figures.gives.iter().any(|(what, _)| labelled(what))
                })
            };
            let matching: Vec<&Charm> = if wanted.is_empty() {
                all.iter().collect()
            } else {
                all.iter()
                    .filter(|charm| {
                        mentions(&charm.name)
                            || charm.effect.as_deref().is_some_and(mentions)
                            || grants(charm)
                    })
                    .collect()
            };

            if matching.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing among this game's {} talismans mentions \"{wanted}\" (matched \
                         loosely, on \"{stem}\", so word endings are not the reason), in its \
                         name, in what it does, or in the figures it grants{}. Try one more \
                         word for the same idea before concluding anything — a stat has a short \
                         form as well as a name. If that finds nothing either, there is no such \
                         talisman here: say so rather than offering one that is not.",
                        all.len(),
                        match asked_for {
                            // Said out loud, because this is the one case where
                            // "nothing found" is a real answer rather than a
                            // failed search: the attribute was understood, the
                            // figures were read, and none of them grants it.
                            Some(attribute) => format!(
                                " — and \"{wanted}\" WAS understood as {attribute}, so the \
                                 figures were checked properly and none of them raises it"
                            ),
                            None => String::new(),
                        }
                    ),
                    note: Some(format!("No talisman · {wanted}")),
                    source: Some("the installed game's own tables".into()),
                };
            }

            // An unfiltered call gets no list at all, only the count and how to
            // search. Returning all 210 names without their effects looked
            // harmless and was the worst of the three things this tool has
            // done: the model called it with no word, got a wall of names it
            // could not reason about, and concluded from that wall that the
            // game has no talisman for faith — saying so in as many words,
            // including that the list "contains only names and weights". A
            // dead end that looks like an answer is worse than no answer, so
            // there is no longer a way to reach one.
            if wanted.is_empty() {
                // The ends of the range, because "which is the lightest" is a
                // question about a number and there was no way to ask it. A
                // model searched the word for "weight" in a language the game
                // is not in, found nothing twice, and answered from memory of
                // the base game — with all 210 weights sitting in the table.
                let mut by_weight: Vec<&Charm> = all.iter().collect();
                by_weight.sort_by(|a, b| {
                    a.weight.total_cmp(&b.weight).then_with(|| a.name.cmp(&b.name))
                });
                let listed = |some: &[&Charm]| -> String {
                    some.iter()
                        .map(|charm| format!("{} {:.1}", charm.name, charm.weight))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let ends = 5.min(by_weight.len());
                let lightest = listed(&by_weight[..ends]);
                let heaviest = listed(
                    &by_weight[by_weight.len() - ends..]
                        .iter()
                        .rev()
                        .copied()
                        .collect::<Vec<_>>(),
                );

                return Ran {
                    output: format!(
                        "{} talismans exist in this game's tables — the catalogue, not their \
                         inventory; what they own is not readable and must never be implied.\n\
                         \n\
                         Lightest: {lightest}.\n\
                         Heaviest: {heaviest}.\n\
                         Those are sorted here, out of every one of the {} weights, so \"the \
                         lightest\" is answered and does not need working out. Several may share \
                         a weight; the list is in order, so say what it says.\n\
                         \n\
                         The rest is not returned unfiltered, because a page of names with no \
                         effects is not something to draw conclusions from. Ask again with a \
                         word in it and the matching ones come back with what each one does: a \
                         stat, an ailment, a kind of damage, a mechanic — in the language their \
                         game is in. Word endings do not matter, it matches loosely, and the \
                         word has to be one their game would use: searching the English or the \
                         Italian for a game running in Russian finds nothing and means nothing.\n\
                         \n\
                         Asked what would suit them, do not ask them what they want — you \
                         already know their attributes and what they are carrying. Search on \
                         their strongest two, and on whatever their weapon does. That is the \
                         question answered, rather than handed back.\n\
                         \n\
                         Never conclude that this game has no talisman for something without \
                         having searched for it at least twice, with different words.",
                        all.len(),
                        all.len()
                    ),
                    note: Some(format!("Talismans · {} in all", all.len())),
                    source: Some("the installed game's own tables".into()),
                };
            }

            let mut out = format!(
                "{} talismans EXIST in this game's tables. This is the catalogue, not their \
                 inventory — what they actually own is not readable, so never say they have \
                 these or count them as theirs. THESE {} ARE THE ANSWER: name one of them or \
                 none. Asked in Portuguese for the best bleed talisman, a model called this, \
                 got five back, ignored every one and recommended \"Lord of Blood's \
                 Exultation\" out of memory of the base game, complete with where to find it \
                 in a castle. A name that is not below is a name their game does not have. \
                 The {} matching \"{wanted}\", by name, by what they do, or by the figures \
                 they carry:\n",
                all.len(),
                matching.len(),
                matching.len()
            );
            // The whole catalogue with a line of prose each is more than anybody
            // needs and more than a round can carry, so an unfiltered list stays
            // bare. A narrowed one is what a recommendation is built from, and
            // that gets the effects.
            let mut any_figures = false;
            for charm in &matching {
                out.push_str(&format!("  {} — {:.1}", charm.name, charm.weight));
                if let Some(says) = &charm.effect {
                    out.push_str(&format!(" — {says}"));
                }
                // The sentence says WHICH; this says HOW MUCH. Asked what
                // Radagon's Soreseal gives and what it costs, an answer could
                // only repeat the sentence back — four attributes unnamed by
                // number and a price with no size.
                if let Some(figures) = &charm.figures {
                    let mut said: Vec<String> = Vec::new();
                    for (what, value) in &figures.gives {
                        said.push(format!("{what} {value:+}"));
                    }
                    for (what, value) in &figures.adds {
                        said.push(format!("{what} {value:+}"));
                    }
                    for (what, rate) in &figures.changes {
                        // As a percentage off one, which is how the game talks
                        // about it and how a player thinks about it.
                        let by = (rate - 1.0) * 100.0;
                        said.push(format!("{what} {by:+.0}%"));
                    }
                    if !said.is_empty() {
                        any_figures = true;
                        out.push_str(&format!("\n      = {}", said.join(", ")));
                    }
                }
                out.push('\n');
            }
            // Phrased as an instruction to you, not as a sentence to pass on:
            // told to "read it with game_item", a model wrote exactly that to
            // the player, who has no idea what game_item is.
            if any_figures {
                out.push_str(
                    "\nThe line beginning \"=\" is what the talisman DOES, read out of the \
                     effect it applies, and it is the answer to \"how much\". A per cent is \
                     against normal: \"physical taken +15%\" means fifteen per cent MORE damage \
                     taken, which is a price and not a benefit — read the sign before you \
                     recommend anything. An attribute is a flat number of points.\n\
                     \n\
                     Where a talisman has no \"=\" line, the tables carry nothing readable for \
                     it and its sentence is all there is. Say so; do not fill the gap.\n",
                );
            } else {
                out.push_str(
                    "\nThose are names and weights and nothing else. Before you tell them what \
                     any of these is FOR, look that one up and read its own description; an \
                     effect guessed from a name is an invention. None of this machinery is \
                     theirs to hear about — never write the name of a tool in your answer.\n",
                );
            }
            // Last, and about the half of the question this tool cannot answer.
            //
            // Asked which talisman raises stamina AND WHERE IT IS, a model
            // named a real one and then placed it on the body of a boss in a
            // forest, with the +2 beside an altar in a cave. None of those
            // exist. The name was right and everything after it was invented,
            // which is the worst shape an answer can have — the true part
            // vouches for the false one.
            //
            // Where a thing IS is genuinely readable in one case only, so the
            // instruction has to say which, or "look it up" sends them round in
            // circles.
            out.push_str(
                "\nWHERE ONE IS FOUND IS NOT IN HERE. These tables give a name, a weight and an \
                 effect, and nothing about location. Asked where something is, do not answer \
                 from memory: a model asked exactly that named a real talisman and then put it \
                 on a boss in a forest and its upgrade beside an altar in a cave, none of which \
                 are in this game.\n\
                 \n\
                 What CAN be established: whether a creature drops it. Look that up — for a map \
                 with what_drops_here, for a named creature with whos_here — WITHOUT naming \
                 either of those to the player, who has never heard of them. If it is not a \
                 drop, its place in the world is not readable by this launcher — say so plainly. A wiki may have it, \
                 and a wiki is the BASE game: a total conversion moves things, so offer it as \
                 the base game's answer or not at all.\n",
            );

            Ran {
                output: out,
                note: Some(format!("Talismans · {}", matching.len())),
                source: Some("the installed game's own tables".into()),
            }
        }

        "spell_numbers" => {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or_default();
            // No name, but an attribute: what that much of it would open.
            if name.is_empty() {
                let raised = |what: &str| -> Option<u8> {
                    args.get(what).and_then(|v| v.as_u64()).and_then(|v| u8::try_from(v).ok())
                };
                let theirs = |what: &str| -> u8 {
                    player
                        .live
                        .as_ref()
                        .and_then(|read| read())
                        .and_then(|live| {
                            live.stats
                                .iter()
                                .find(|(name, _)| name.to_lowercase().starts_with(what))
                                .map(|(_, value)| *value)
                        })
                        .and_then(|value| u8::try_from(value).ok())
                        .unwrap_or(0)
                };

                let (int, fth, arc) = (theirs("intel"), theirs("faith"), theirs("arcane"));
                let (want_int, want_fth, want_arc) = (
                    raised("intelligence").unwrap_or(int).max(int),
                    raised("faith").unwrap_or(fth).max(fth),
                    raised("arcane").unwrap_or(arc).max(arc),
                );
                let now = (player.spells_at)(int, fth, arc);

                // No attribute raised: they are asking what they can cast as
                // they stand, which is a fair question and was being answered
                // from memory because this refused it.
                if (want_int, want_fth, want_arc) == (int, fth, arc) {
                    if now.is_empty() {
                        return Ran {
                            output: "The installed tables could not be read, so there is no list. \
                                     Do not name spells from memory."
                                .into(),
                            note: None,
                            source: None,
                        };
                    }
                    // The whole table, for the times the question is "how many
                    // are there" rather than "what can I cast" — asked that, a
                    // model read a number off a wiki page instead.
                    let every = (player.spells_at)(99, 99, 99);
                    let mut out = format!(
                        "This game has {} spells in its own tables. Their attributes are INT \
                         {int}, FTH {fth}, ARC {arc}, and {} of those ask no more than that. The \
                         hardest of the ones within reach:\n",
                        every.len(),
                        now.len()
                    );
                    out.push_str(&listing(&now.iter().collect::<Vec<_>>()));
                    out.push_str(
                        "\nThat is what their attributes allow, not what they own — which spells \
                         they have found or memorised is not readable. Say it that way. And the \
                         total above is this installation's own count: use it rather than a \
                         number off a wiki.\n\
                         \n\
                         WHAT IS IN THEIR HANDS IS NOT A SPELL. Asked in Polish what their spell \
                         cost in FP, an answer named the DAGGER they were holding and reported \
                         its skill's stamina cost as the answer. A weapon's skill is not a \
                         spell; a staff or a sacred seal is what CASTS spells and is not a spell \
                         either. If they say \"my spell\" and nothing here says which one, that \
                         is the one thing worth handing back — ask which, or give them this list \
                         and say the game does not tell the launcher what they have memorised.",
                    );
                    return Ran {
                        output: out,
                        note: Some(format!("Within reach · {} spells", now.len())),
                        source: Some("the installed game's own tables".into()),
                    };
                }

                let later = (player.spells_at)(want_int, want_fth, want_arc);
                let opened: Vec<&Cast> = later
                    .iter()
                    .filter(|cast| !now.iter().any(|had| had.name == cast.name))
                    .collect();

                // "At least X" off every spell, not the delta: the threshold
                // set. Off the raw value asked, not one maxed with current.
                let req = |cast: &Cast, key: &str| -> u8 {
                    cast.spell
                        .needs
                        .iter()
                        .find(|(what, _)| what.to_lowercase().starts_with(key))
                        .map(|(_, value)| *value)
                        .unwrap_or(0)
                };
                let every = (player.spells_at)(99, 99, 99);
                let mut threshold = String::new();
                for (key, label, given) in [
                    ("intel", "INT", raised("intelligence")),
                    ("faith", "FTH", raised("faith")),
                    ("arcane", "ARC", raised("arcane")),
                ] {
                    let Some(floor) = given else { continue };
                    let mut at_least: Vec<&Cast> =
                        every.iter().filter(|cast| req(cast, key) >= floor).collect();
                    at_least.sort_by(|a, b| req(b, key).cmp(&req(a, key)));
                    if at_least.is_empty() {
                        threshold.push_str(&format!(
                            "No spell in this game needs {label} {floor} or more.\n"
                        ));
                    } else {
                        threshold.push_str(&format!(
                            "{} spells need {label} {floor} or greater. This is the FULL set — \
                             every spell whose {label} requirement is {floor} or above, the ones \
                             needing more than {floor} included, NOT only those at exactly \
                             {floor}. Hardest first:\n",
                            at_least.len()
                        ));
                        threshold.push_str(&listing(&at_least));
                    }
                }

                if opened.is_empty() && threshold.is_empty() {
                    return Ran {
                        output: format!(
                            "Nothing in this game's tables opens up between what they have \
                             (INT {int}, FTH {fth}, ARC {arc}) and INT {want_int}, FTH \
                             {want_fth}, ARC {want_arc}. Say so — do not name spells that are \
                             not in the list."
                        ),
                        note: Some("Nothing opens".into()),
                        source: Some("the installed game's own tables".into()),
                    };
                }

                let has_live = int > 0 || fth > 0 || arc > 0;
                let mut out = String::new();
                if !threshold.is_empty() {
                    out.push_str(&threshold);
                }
                // Opens-up needs real stats to rise from; without a live read it
                // is just "everything up to X" and muddies the at-least list.
                if !opened.is_empty() && has_live {
                    out.push_str(&format!(
                        "\nA DIFFERENT QUESTION — raising to INT {want_int}, FTH {want_fth}, ARC \
                         {want_arc} from INT {int}, FTH {fth}, ARC {arc} newly opens these, which \
                         need NO MORE than that:\n"
                    ));
                    out.push_str(&listing(&opened));
                }
                out.push_str(
                    "\n\"At least X\" / \"X or more\" / \"минимум X\" / \"mindestens X\" means X AND \
                     everything above it — report every spell in the list, not only those that \
                     need exactly X. Never say a game has no spell above a value unless the list \
                     says none.\n",
                );
                return Ran {
                    output: out,
                    note: Some(if threshold.is_empty() {
                        format!("Opens up · {} spells", opened.len())
                    } else {
                        "Spells by requirement".into()
                    }),
                    source: Some("the installed game's own tables".into()),
                };
            }

            let Some(found) = (player.spell)(name).filter(|found| !found.is_empty()) else {
                return Ran {
                    output: format!(
                        "No spell called \"{name}\" in this installation's tables. If they have \
                         it, use the name player_status or game_item gave; otherwise say it is \
                         not in this game rather than quoting the base game's numbers for it."
                    ),
                    note: Some(format!("No figures · {name}")),
                    source: None,
                };
            };

            let mut out = String::from("Out of the tables this installation runs on:\n");
            for cast in &found {
                out.push_str(&format!("\n{}\n", cast.name));
                out.push_str(&format!("  {} FP", cast.spell.fp));
                if let Some(held) = cast.spell.fp_held {
                    out.push_str(&format!(", or {held} held"));
                }
                out.push_str(&format!(
                    ", {} stamina, {} memory slot{}\n",
                    cast.spell.stamina,
                    cast.spell.slots,
                    if cast.spell.slots == 1 { "" } else { "s" }
                ));
                if cast.spell.needs.is_empty() {
                    out.push_str("  Asks nothing of their attributes.\n");
                } else {
                    let parts: Vec<String> = cast
                        .spell
                        .needs
                        .iter()
                        .map(|(what, value)| format!("{what} {value}"))
                        .collect();
                    out.push_str(&format!("  Needs: {}\n", parts.join(", ")));
                }
            }
            if found.iter().any(|cast| cast.modded) {
                out.push_str(
                    "\nThese came from the total conversion's own tables, so they override every \
                     wiki. Where the two disagree, this is what their game does.",
                );
            }

            let note = found
                .iter()
                .map(|cast| cast.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Ran {
                output: out,
                note: Some(format!("Numbers · {note}")),
                source: Some("the installed game's own tables".into()),
            }
        }

        "search_web" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
            if query.trim().is_empty() {
                return Ran { output: "No query given.".into(), note: None, source: None };
            }

            let found = crate::web::search(http, query, 6).await;
            if found.is_empty() {
                return Ran {
                    output: format!(
                        "The web returned nothing for \"{query}\", or could not be reached. \
                         Answer from what is on this machine, and say what you could not find."
                    ),
                    note: Some(format!("The web · {query}")),
                    source: None,
                };
            }

            let mut out = String::from(
                "From the open web. These are strangers' pages, not the wikis and not the game's \
                 own data — weigh them accordingly and say where a claim came from. Open the one \
                 that looks right with read_page rather than answering from these summaries:\n",
            );
            for hit in &found {
                out.push_str(&format!("\n{}\n  {}\n", hit.title, hit.url));
                if !hit.summary.is_empty() {
                    let short: String = hit.summary.chars().take(300).collect();
                    out.push_str(&format!("  {short}\n"));
                }
            }

            // Last, because the temptation is at the end of reading these.
            //
            // Every name on the open web is the BASE game's, in English, written
            // by somebody playing a different installation from theirs. Asked
            // which spirit ash suited them, an answer searched the web and came
            // back with "Spirit Jellyfish", "Latenna the Albinauric" and
            // "Soldjars of Fortune" — to a player whose game is in Russian and
            // whose conversion may not have any of the three. Their own
            // catalogue holds the ashes, under the names their menu prints, and
            // it was never asked.
            out.push_str(
                "\nEVERY NAME ABOVE IS SOMEBODY ELSE'S GAME. These pages are the base game in \
                 English, written by people who are not running what this player is running. \
                 Before any name from here goes into an answer, look it up in their own \
                 installation and use the name THEIR menu prints — and if it is not there, it \
                 is not theirs to be told about. The same goes for a number: a figure off a web \
                 page is the base game's and a total conversion rewrites exactly those.\n",
            );

            Ran {
                output: out,
                note: Some(format!("The web · {query}")),
                source: Some("the open web".into()),
            }
        }

        "map_markers" => {
            let character = args
                .get("character")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty());
            let remove = args
                .get("remove")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|what| !what.is_empty());

            let note = match remove {
                Some(what) => format!("Map · removing {what}"),
                None => "Map · what is pinned".to_string(),
            };
            match (player.pins)(character, remove) {
                Ok(said) => Ran {
                    output: said,
                    note: Some(note),
                    source: Some("their own save".into()),
                },
                Err(why) => Ran {
                    output: format!(
                        "{why}\n\nSay this as it is rather than describing a map you could not \
                         read."
                    ),
                    note: Some(note),
                    source: None,
                },
            }
        }

        "place_marker" => {
            let place = args
                .get("place")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim();
            if place.is_empty() {
                return Ran {
                    output: "No place given. Ask them which one.".into(),
                    note: None,
                    source: None,
                };
            }

            let character = args
                .get("character")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty());

            match (player.mark)(place, character) {
                Ok(done) => Ran {
                    output: format!(
                        "{done}\n\nTell them it is on the map and that they can take it off the \
                         same way they would any pin."
                    ),
                    note: Some(format!("Marking · {place}")),
                    source: Some("their own save".into()),
                },
                Err(why) => Ran {
                    output: format!(
                        "Nothing was written. {why}\n\nSay this plainly rather than claiming the \
                         marker is there."
                    ),
                    note: Some(format!("Marking · {place}")),
                    source: None,
                },
            }
        }

        "read_page" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default().trim();
            let about = args.get("about").and_then(|v| v.as_str()).unwrap_or_default();
            if url.is_empty() {
                return Ran { output: "No address given.".into(), note: None, source: None };
            }

            // The host alone, for the note and the citation: an address is too
            // long to read at a glance, and the player watches these go past.
            let host = url
                .split("//")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .unwrap_or(url)
                .trim_start_matches("www.")
                .to_string();

            match crate::web::fetch(http, url).await {
                Ok(html) => {
                    let text = to_text(&html);
                    let mut out = format!(
                        "From {host} — a page on the open web, not a wiki and not the game's own \
                         data. Name it when you use it.\n\nThe page opens with this. On a page of \
                         dated entries the newest is the one at the top, so this is what \"the \
                         latest\" means here:\n{}",
                        text.chars().take(OPENING).collect::<String>()
                    );

                    if !about.trim().is_empty() {
                        let part = best_window(&text, about, ARTICLE);
                        // Only when it is somewhere else. On a short page the
                        // match is the opening, and printing it twice spends
                        // the budget saying the same thing.
                        let head: String = part.chars().take(150).collect();
                        if !text.chars().take(OPENING).collect::<String>().contains(&head) {
                            out.push_str(&format!(
                                "\n\nAnd this is the part of the page about \"{about}\", from \
                                 further down — check which entry it belongs to before calling it \
                                 recent:\n{part}"
                            ));
                        }
                    }

                    Ran {
                        output: out,
                        note: Some(format!("Opening · {host}")),
                        source: Some(host),
                    }
                }
                Err(why) => Ran {
                    output: format!(
                        "{host} could not be read. {why} Try another result, or answer from what \
                         is on this machine and say what you could not open."
                    ),
                    note: Some(format!("Opening · {host}")),
                    source: None,
                },
            }
        }

        "game_item" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            if name.trim().is_empty() {
                return Ran { output: "No name given.".into(), note: None, source: None };
            }

            let Some(hits) = (player.catalogue)(name) else {
                return Ran {
                    output: "The game is not open, so its own text cannot be read. Use the \
                             wiki, and say that the figures are the base game's if a total \
                             conversion is installed."
                        .into(),
                    note: None,
                    source: None,
                };
            };

            if hits.is_empty() {
                return Ran {
                    output: format!(
                        "Nothing the running game NAMES is called \"{name}\" — no weapon, no \
                         piece of armour, no talisman, no item. The names are in the language \
                         the game is installed in, so try a shorter piece of the word, or a \
                         word in that language.\n\
                         \n\
                         THIS IS NOT \"NOT IN THE GAME\", and taking it for that has already \
                         gone wrong. Only things with names are in here. A menu entry, a \
                         mechanic, a tutorial, a term somebody read on a screen — none of those \
                         are named things and none of them would show up. Asked what a grace \
                         menu entry did, an answer took this miss for proof and told the player \
                         the entry \"is not a real option in the game\", about words they were \
                         reading off their own screen at that moment. Search the game's own \
                         writing before concluding anything: 54,000 lines of description, \
                         tutorial, menu and speech, and a hit there settles it whatever this \
                         said.\n\
                         \n\
                         When BOTH have found nothing, then \"{name}\" is not in this game and \
                         must not appear in your answer. Not as a suggestion, not hedged with \
                         \"if it is in this version\", not as the thing they should go and look \
                         for. Asked how to redistribute their points, a model was told three \
                         separate times that a name was not in the game and then built its \
                         whole answer around it anyway, sending the player after a merchant and \
                         an item that do not exist. Say what you could not find and stop there."
                    ),
                    // Said as a miss, because echoing the query reads as a hit:
                    // "In the game · Blood Cleric" looked in the log exactly
                    // like the game having such a thing, and it does not.
                    note: Some(format!("Nothing named · {name}")),
                    source: None,
                };
            }

            let mut out = String::from(
                "From the running game itself — these are this player's own copy, mod and all. \
                 EVERYTHING BELOW EXISTS IN THEIR GAME. That is what a result here means, so \
                 never answer that one of them is missing, is from a different game, or is \
                 something you do not recognise. Asked where to find a crimson rose, an answer \
                 was handed the item by name and replied that it is from Bloodborne and is not \
                 in ELDEN RING at all — then recommended two spells instead. The launcher had \
                 just read it out of their installation.\n",
            );
            let mut figured = false;
            for hit in &hits {
                out.push_str(&format!("\n{} ({})\n", hit.name, hit.what));
                if let Some(effect) = &hit.effect {
                    out.push_str(&format!("  Does: {effect}\n"));
                }
                // For a talisman the sentence above says WHICH and this says
                // HOW MUCH. Both here, rather than one here and one behind
                // another call, because a figure that lives in only one tool
                // gets invented by whichever path the model happens to take —
                // the third time this launcher has been caught by exactly that.
                if hit.what == "talisman" {
                    if let Some(figures) = (player.charm)(hit.id) {
                        let mut said: Vec<String> = Vec::new();
                        for (what, value) in &figures.gives {
                            said.push(format!("{what} {value:+}"));
                        }
                        for (what, value) in &figures.adds {
                            said.push(format!("{what} {value:+}"));
                        }
                        for (what, rate) in &figures.changes {
                            said.push(format!("{what} {:+.0}%", (rate - 1.0) * 100.0));
                        }
                        if !said.is_empty() {
                            figured = true;
                            out.push_str(&format!(
                                "  Exactly: {} — and it weighs {:.1}\n",
                                said.join(", "),
                                figures.weight
                            ));
                        }
                    }
                }
                // A weapon's damage, here, for the same reason as the talisman.
                // Asked Reduvia's stats, the model took this description, saw no
                // numbers, and concluded the weapon "has no parameters" — while
                // gear_numbers reads 82 fire off the same row. Put the figure
                // where the description is so it cannot be missed.
                if hit.what == "weapon" {
                    if let Some(armed) = (player.weapon)(&hit.name)
                        .into_iter()
                        .flatten()
                        .next()
                    {
                        let damage: Vec<String> = armed
                            .weapon
                            .damage
                            .iter()
                            .filter(|(_, value)| *value > 0)
                            .map(|(kind, value)| format!("{value} {kind}"))
                            .collect();
                        if !damage.is_empty() {
                            figured = true;
                            out.push_str(&format!(
                                "  Damage: {} — weighs {:.1}. Fuller scaling and requirements are \
                                 gear_numbers'.\n",
                                damage.join(", "),
                                armed.weapon.weight
                            ));
                        }
                    }
                }
                if let Some(caption) = &hit.caption {
                    out.push_str(&format!("  {caption}\n"));
                }
            }
            if figured {
                out.push_str(
                    "\nThe \"Exactly\" line is read out of the effect the talisman applies and \
                     is what \"how much\" gets answered with — never \"noticeably\" or \
                     \"significantly\" while the number is sitting right there. A per cent is \
                     against normal, so \"physical taken +15%\" means fifteen per cent MORE \
                     damage taken: a price, not a benefit. Read the sign before recommending.\n",
                );
            }

            out.push_str(&onward_from(hits.iter().map(|hit| hit.what.as_str())));

            let note = format!("In the game · {}", hits[0].name);
            Ran { output: out, note: Some(note), source: Some("the running game".into()) }
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
            // Said hard, because this table is the base game and a modded
            // player is reading something else. Asked what their weapon does,
            // a model went here and reported 79 physical damage and dexterity
            // scaling for a weapon the mod had made a fire weapon with no
            // physical damage at all.
            out.push_str(if player.edition.is_some() {
                "(THESE ARE THE BASE GAME'S NUMBERS AND THIS PLAYER IS NOT PLAYING THE BASE \
                 GAME. A total conversion is installed and it rewrites exactly these figures. \
                 Do not quote anything above for a weapon — call gear_numbers, which reads \
                 their own installation. WHAT SOMETHING DROPS is rewritten too and is not a \
                 mechanic: what_drops_here reads their own drop tables. Where a boss's reward \
                 is concerned nothing here can read it at all — those are scripted rather than \
                 rolled — so say it is not readable rather than repeating the base game's, and \
                 never guess at what a named set contains. Names, locations and how a thing \
                 WORKS are still worth having from here.)"
            } else {
                "(These are the base game's numbers. gear_numbers reads the figures this \
                 installation actually uses, and what_drops_here their own drop tables, which \
                 are better where the two could differ. A boss's reward is scripted and is not \
                 readable here at all — say so rather than guessing at it.)"
            });

            Ran {
                output: out,
                note: Some(format!("Looking up · {}", hits[0].name)),
                source: Some(format!("{} · game data", hits[0].name)),
            }
        }

        "player_status" => {
            let mut out = String::new();

            // The running game first, because it is the only source that is
            // current. The save is whichever slot was written last and whatever
            // it held at the last grace.
            let live = player.live.as_ref().and_then(|read| read());
            if let Some(live) = &live {
                out.push_str(
                    "The game is open and this is the character in it, right now. The first \
                     thing on the line is the NAME THEY CHOSE, not a class — the game does not \
                     record which class a character started as, and there is nothing here that \
                     says. Asked what class they were, a model read the character's name off \
                     this line and answered with that. If they ask, say it is not recorded and \
                     that their gear and attributes are what can be seen.\n",
                );
                out.push_str(&format!(
                    "  {} — level {}, {} runes held, {} earned all told\n",
                    live.name, live.level, live.runes, live.runes_ever
                ));
                // What the next level costs is NOT here, and asked for it a
                // model produced 108,030 runes for level 35 — out by a factor
                // of fifty. A total conversion can rewrite the curve as well,
                // so even the base game's formula would be the wrong answer.
                out.push_str(
                    "  What the next level costs is not readable, and this game's curve may not \
                     be the base game's anyway. Do not state a figure for it — say it is on \
                     their own level-up screen.\n",
                );
                out.push_str(&format!(
                    "  {}/{} HP, {}/{} FP, {}/{} stamina\n",
                    live.hp, live.hp_max, live.fp, live.fp_max, live.stamina, live.stamina_max
                ));
                // Poise, here as well as in the block, because this is the tool
                // the model reaches for when asked about the player. Without it
                // "how much poise have I" came back as their stamina twice and
                // as the runes in their pocket once — three different numbers
                // off the same screen, none of them the one asked for.
                if let Some(gear) = live.gear.as_ref() {
                    let carried: f32 = gear
                        .armour
                        .iter()
                        .filter_map(|(_, name)| {
                            (player.armour)(name)?.first()?.armour.poise
                        })
                        .sum();
                    if carried > 0.0 {
                        out.push_str(&format!(
                            "  {carried:.2} poise, added up from the armour they have on. That \
                             is what the screen calls Poise — in Russian \"Баланс\" — and it is \
                             NOT their stamina, NOT their endurance and NOT their runes, all of \
                             which are on this same line and have all three been given as the \
                             answer to it.\n"
                        ));
                    }
                }
                // With the game's own word beside each, where it is known.
                // The labels already end in the English abbreviation — "Faith
                // (FTH)" — and an answer still rendered Faith into Russian as
                // "Фея", a fairy, while quoting their real 22. Given the word
                // there is nothing left to translate.
                let words = &player.attribute_words;
                let listed = |stats: &[(String, u32)]| -> String {
                    stats
                        .iter()
                        .map(|(what, value)| {
                            match words.iter().find(|(english, _)| what.starts_with(english)) {
                                Some((_, theirs)) => {
                                    format!("{what} {value} — their game calls it {theirs}")
                                }
                                None => format!("{what} {value}"),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                out.push_str(&format!("  {}\n", listed(&live.stats)));
                // Their equip load is not in here. Asked how much they had
                // left, a model added up the six pieces correctly — those come
                // from the tables — and then supplied a maximum out of its own
                // memory, which is the half of that answer nobody can check.
                out.push_str(
                    "  Their equip load — the maximum they can carry — is not readable. \
                     gear_numbers gives what each piece weighs and those can be added up, but \
                     do not state the maximum or how much is left: say what it all weighs and \
                     that the limit is on their equipment screen.\n",
                );
                if let Some(spent) = &live.spent {
                    // Equipment has moved something, so both numbers are worth
                    // having: the first is what their screen says and what any
                    // requirement is met against, the second is what they spent
                    // and what they would keep if they took the item off.
                    out.push_str(&format!(
                        "  Those are the numbers on their stat screen. Without their equipment, \
                         the points they have actually spent are: {}\n",
                        listed(spent)
                    ));
                }
                if let Some(place) = &live.place {
                    out.push_str(&format!(
                        "  Standing in: {} — map {}, at {:.0}, {:.0}, {:.0}\n",
                        place.name.as_deref().unwrap_or("an unnamed part of the map"),
                        place.map,
                        place.x,
                        place.y,
                        place.z
                    ));

                    // What is actually around them. Asked where to go next, a
                    // model that had only the region's name invented a boss for
                    // it; these are places the game itself names, at distances
                    // measured from where they are standing.
                    if let Some((map_x, map_y)) = on_the_map(place) {
                        let near = (player.nearby)(map_x, map_y);
                        if !near.is_empty() {
                            out.push_str("  Nearest places the map names, from where they stand:\n");
                            for (name, away) in near.iter().take(6) {
                                out.push_str(&format!("    {name} — {away:.0} away\n"));
                            }
                            out.push_str(
                                "    (Those distances are the world's own units. Use these for \
                                 \"where do I go\" rather than naming somewhere from memory, and \
                                 read the wiki before saying what is in one.)\n\
                                 \n\
                                     These are the labels drawn on the map and NOTHING ELSE. \
                                 There is not one site of grace among the 452 of them, so where \
                                 the nearest grace is cannot be answered from here — asked, a \
                                 model picked a castle off this list and called it the nearest \
                                 grace, which it is not. Say the map in their game shows those \
                                 and this does not.\n",
                            );
                        }
                    }
                }

                // Named by the game rather than by a table, so these are the
                // words on the player's own screen — including anything a total
                // conversion renamed, and in the language they installed.
                if let Some(gear) = &live.gear {
                    if !gear.weapons.is_empty() {
                        out.push_str(&format!("  Holding: {}\n", gear.weapons.join(", ")));
                    }
                    if !gear.armour.is_empty() {
                        let worn: Vec<String> = gear
                            .armour
                            .iter()
                            .map(|(slot, piece)| format!("{slot} {piece}"))
                            .collect();
                        out.push_str(&format!("  Wearing: {}\n", worn.join(", ")));
                    }
                    out.push_str(&match gear.talismans.is_empty() {
                        true => "  No talismans equipped.\n".to_string(),
                        false => format!("  Talismans: {}\n", gear.talismans.join(", ")),
                    });
                    // Equipped is not the same as unlocked, and the difference
                    // was being filled in with invention. Asked why their
                    // second talisman slot was locked, an answer named the
                    // "Талисман Путника (Traveler's Talisman)" as the item that
                    // opens it in The Convergence, told them to check their
                    // inventory for it, and offered to go and find where it
                    // lies. No tool returned that name. The battery before it,
                    // the same question produced a great rune requirement
                    // instead — so it is the QUESTION that reliably produces
                    // invention, not one model having a bad turn, and the
                    // premise was false both times.
                    // Said out loud because its absence was being filled in.
                    // Asked whether they could cast everything they had
                    // memorised, a model answered "yes, all of them" — it had
                    // no list, and nothing here told it so.
                    out.push_str(
                        "  Which spells they have memorised is NOT readable and is not listed \
                         above. Do not say what they can or cannot cast: ask them which spell \
                         they mean, then use spell_numbers on it. Do not OFFER to look either — \
                         \"I can check what you have memorised\" is a promise of the one thing \
                         that cannot be done, and they will say yes.\n",
                    );
                    out.push_str(
                        "  (Those names are the game's own, so they are in the language it is \
                         installed in and account for any mod. game_item looks up more about \
                         any of them.)\n",
                    );
                }
                out.push('\n');
            }

            // Out here rather than beside the equipped talismans, because the
            // case that needs it most is the one where there is no live game to
            // read — and that is the ordinary case, the launcher being a thing
            // you use before you press play.
            //
            // Equipped is not the same as unlocked, and the difference was
            // being filled in with invention. Asked why their second talisman
            // slot was locked, an answer named the "Талисман Путника
            // (Traveler's Talisman)" as the item that opens it in The
            // Convergence, told them to go and check their inventory for it,
            // and offered to find where it lies. No tool returned that name.
            // The battery before it, the same question produced a great rune
            // requirement instead — so it is the QUESTION that reliably
            // produces invention rather than one model having a bad turn, and
            // the premise was false both times.
            out.push_str(
                "How many talisman SLOTS are unlocked is NOT readable, and neither is anything \
                 else about their progress: no inventory, no event flags, no list of what they \
                 have found or where they have been. What is WORN can be read, and only while \
                 the game is running. So do not explain why a slot is locked, do not name the \
                 item or the boss that opens one, and do not take it as true that a slot IS \
                 locked because the question says so — say that this cannot be seen from here, \
                 and ask what their screen actually shows.\n",
            );

            out.push_str(&format!(
                "Game version: {}\n",
                player.version.as_deref().unwrap_or("unknown")
            ));
            match &player.edition {
                Some(name) => {
                    out.push_str(&format!("Total conversion installed: {name}\n"));
                    // What this line does NOT say, said where the line is
                    // handed over. The same shape that stopped the invented
                    // talisman slot: naming the limit at the point the data
                    // arrives works where a rule in the block does not.
                    //
                    // Asked where the bonfires were, an answer called this tool
                    // and nothing else, then wrote "в The Convergence они стоят
                    // на тех же местах карты — мод не перемещает их и не
                    // удаляет". That is a claim about what the conversion
                    // CHANGES, and this line only says it is installed.
                    out.push_str(
                        "  That is all this says about it: that it is installed. What it \
                         CHANGES — where anything is, what a weapon does, whether a mechanic \
                         was altered or removed — is NOT in here, and it is not in your memory \
                         either, because this conversion rewrote it. Read the game's own text \
                         or its tables before saying what it does or does not do. \"The mod \
                         does not change X\" is a claim like any other and needs looking up.\n",
                    );
                }
                None => out.push_str("No total conversion — the base game.\n"),
            }

            if player.characters.is_empty() {
                out.push_str("No save file found, so no characters to report.\n");
            } else {
                out.push_str(if live.is_some() {
                    "In the save, as of their last rest. A save holds several slots; the one \
                     marked below is the character in the running game. The others are old \
                     characters and are not who the question is about:\n"
                } else {
                    "Characters, from the save file — the game is not running, so this is \
                     where they were when they last rested:\n"
                });
                for (name, level, seconds) in &player.characters {
                    // Which slot they are actually in is the thing the save
                    // cannot say: it holds every character they ever made, and
                    // answering about a retired one reads as the launcher not
                    // knowing them at all.
                    let playing = live
                        .as_ref()
                        .is_some_and(|live| live.name.trim().eq_ignore_ascii_case(name.trim()));
                    out.push_str(&format!(
                        "  {name} — level {level}, {} hours played{}\n",
                        seconds / 3600,
                        if playing { "   <- playing this one" } else { "" }
                    ));
                }
            }

            // "No mods enabled" on its own is true of the mod PROFILE and
            // false as an impression. Asked what mod profiles they had, an
            // answer said "none, 0 mods installed" — to somebody running a
            // total conversion and Seamless Co-op, because an edition is
            // managed separately from the profile list and this line did not
            // know that. Whatever it says, it now says it in context.
            if player.mods.is_empty() {
                let alongside: Vec<&str> = [
                    player.edition.as_deref(),
                    player.seamless.then_some("Seamless Co-op"),
                ]
                .into_iter()
                .flatten()
                .collect();
                if alongside.is_empty() {
                    out.push_str("No mods enabled, and no edition either — this is the game as \
                                  it shipped.\n");
                } else {
                    out.push_str(&format!(
                        "The mod PROFILE is empty — no loose mods enabled. That is not the same \
                         as an unmodified game: {} {} installed and managed separately from the \
                         profile list. Never answer \"you have no mods\" on the strength of the \
                         empty profile alone.\n",
                        alongside.join(" and "),
                        if alongside.len() == 1 { "is" } else { "are" }
                    ));
                }
            } else {
                out.push_str(&format!("Mods enabled: {}\n", player.mods.join(", ")));
            }
            if player.framegen {
                out.push_str("DLSS and frame generation are installed.\n");
            }
            if let Some(frames) = &player.frames {
                out.push_str(frames);
            }
            if player.seamless {
                out.push_str(
                    "Seamless Co-op is installed, so playing together works nothing like the \
                     base game's: both players stay in one world for the whole run, bosses and \
                     progress included, and there are no summon signs and no Furlcalling Finger \
                     Remedy in it. They join by setting the same password in the mod's settings \
                     — the launcher writes that — and starting through its own launcher. Answer \
                     any question about playing with somebody that way, not the vanilla way.\n",
                );
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
    Doing { note: String },
    /// The articles the answer was actually built from.
    Sources { sources: Vec<String> },
    /// A piece of the answer, as the model writes it.
    Delta { text: String },
    /// `cut` means the answer stops early.
    Done { lane: Option<String>, ms: Option<u64>, cut: bool },
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
    /// Prompt tokens the winning lane counted, and how many were cached.
    #[serde(default)]
    prompt: Option<u64>,
    #[serde(default)]
    cached: Option<u64>,
    /// Why each lane declined, when they all did.
    #[serde(default)]
    tried: Vec<Declined>,
}

/// One lane's refusal.
#[derive(Debug, Clone, Deserialize)]
struct Declined {
    #[serde(default)]
    lane: String,
    #[serde(default)]
    why: String,
}

/// What is installed, when it changes the answer rather than decorating it.
fn could_be_about_the_launcher(asked: &str) -> bool {
    let said = asked.trim().to_lowercase();
    if said.is_empty() {
        return true;
    }
    // English, Russian, German, Spanish and French, because the launcher has
    // already been asked about in four of the five.
    const WORDS: &[&str] = &[
        "launcher", "лаунчер", "лаунчере", "программ", "app", "starter",
        "mod", "мод", "моды", "мода", "модов", "modding",
        "save", "сейв", "сохран", "spielstand", "guardado", "sauvegarde",
        "backup", "бэкап", "бекап", "snapshot", "снапшот", "снимок",
        "profile", "профил", "profil",
        "coop", "co-op", "кооп", "мультиплеер", "multiplayer",
        "password", "пароль", "passwort", "contraseña",
        "install", "установ", "поставить", "ставить", "instalar",
        "fps", "кадр", "frame", "framerate", "dlss", "fsr", "upscal",
        "апскейл", "график", "graphic", "grafik", "разрешение", "resolution",
        "performance", "производительн", "лаг", "lag", "stutter", "фриз",
        "crash", "краш", "вылет", "ошибк", "error",
        "folder", "папк", "path", "путь", "файл", "file", "directory",
        "button", "кнопк", "tab", "вкладк", "screen", "экран", "интерфейс",
        "interface", "overlay", "оверлей", "settings", "настройк",
        "запуст", "launch", "steam", "gpu", "видеокарт",
        "wiki", "вики", "online", "онлайн", "ban", "бан", "античит", "anticheat",
        "eac", "версия", "version", "update", "обнов",
    ];
    WORDS.iter().any(|word| said.contains(word))
}

fn setup_worth_knowing(player: &Player) -> Option<String> {
    // What the launcher itself can do for them. Asked whether a save could be
    // rolled back, the assistant searched two wikis and the open web and
    // answered that it could not find out — while sitting inside the program
    // that takes a snapshot before every launch and can put one back.
    let mut said = String::from(
        "# Roundtable\n\
         \n\
         You are part of Roundtable, the launcher this player is running. A question about what \
         can be done is often about it rather than about the game. Everything below is the \
         launcher telling you what it knows; none of it is text to repeat, and each rule is here \
         because an answer went wrong without it.\n\
         \n\
         AND A COMPLAINT ABOUT THE LAUNCHER IS STILL YOURS. Asked \"why did the launcher delete \
         my mods\", an answer said that is a question about the launcher and not about the game, \
         and handed it back — having called nothing. It is the launcher asking you; there is no \
         one else to pass it to. player_status says what is enabled RIGHT NOW, which either \
         contradicts the premise or narrows it, and one call beats asking them to describe their \
         own screen. Note the shape: the same subject asked helpfully — how do I back up a save \
         — was answered at once and exactly. A question phrased as a complaint is not out of \
         scope, it is the one most worth looking into.\n\
         \n\
         ## Where an answer may come from\n\
         \n\
         In this order, always. A lower source may not contradict a higher one, and where they \
         disagree the higher one is what their game does.\n\
         \n\
         1. THE INSTALLED TABLES — gear_numbers, spell_numbers, talismans, upgrade_path, \
         whos_here, what_drops_here, catalogue. These are read out of the files their game \
         loads. Nothing outranks them.\n\
         2. THE GAME'S OWN WRITING — game_text. Fifty-four thousand lines: every description, \
         every tutorial, every menu entry, everything anyone says, in their language. The \
         tables give the NUMBERS and this gives the WORDS, so it is the first place to look for \
         a mechanic, a term, or what a thing is FOR — and for a total conversion it is the \
         author explaining their own work, which no wiki has. Somebody reading a menu entry off \
         their own screen was told no article covered it and to try a forum; the tutorial \
         explaining it was sitting in their installation.\n\
         3. THE RUNNING GAME — player_status, and the line about them further down. What is \
         true this second.\n\
         4. THE MIRRORED WIKIS — search_wiki and read_article. Written by people who play the \
         BASE game. Good for how a fight goes, where a place is, what a quest wants. Not for a \
         number, a weakness, or a drop, all of which a total conversion rewrites.\n\
         5. THE OPEN WEB — search_web and read_page. For what is newer than a mirror: a patch \
         note, a release, an argument.\n\
         6. YOUR OWN MEMORY — last, and only for how the game WORKS: what poise is, what a \
         parry does, how scaling reads. Never for a number, never for a name, never for what \
         something drops or resists. Every invented answer this launcher has caught came from \
         here being reached for first.\n\
         \n\
         A SEQUENCE OF STEPS IS A LIST OF NAMES, and that rule covers it. Asked which step of a \
         questline they were on, an answer said honestly that it was going from memory and then \
         gave the whole chain — a tower, three servants by name, a well, a city below — every \
         one of them out of the base game and handed to somebody running a conversion that \
         rewrites exactly those. Saying \"from memory\" first does not make it theirs. Where a \
         quest stands is not readable from anything here: say so, and if a wiki has the chain, \
         give it as the wiki's and the base game's, with that said out loud.\n\
         \n\
         When a lower source is all there is, say which one it was. \"According to the wiki, in \
         the base game\" is an answer; the same sentence without that clause is a mistake.\n\
         \n\
         A FIND BEATS A MISS. Each of these looks at a different thing, so one of them coming \
         back empty says only that the answer is not in ITS part — it is not evidence of \
         anything. If one tool finds it and another does not, it exists and the second was \
         looking in the wrong place. Asked what a grace menu entry did, an answer took a miss \
         from the item catalogue as proof and told the player the entry \"is not a real option \
         in the game\" — words they were reading off their own screen — while the search that \
         did find it sat in the same conversation, ignored.\n\
         \n\
         \"It is not in the game\" is the strongest claim available here and it needs EVERY \
         source to have missed, not one. Until then the honest sentence is that you could not \
         find it, which is about you and not about their game.\n\
         \n\
         DO IT, DO NOT OFFER TO DO IT. You have these sources in your hands right now and the \
         player has none of them. \"I could look that up if you like\", \"I can search the wiki \
         for you\", \"tell me and I will check\" — every one of those spends their turn and \
         gives them nothing, and they have to ask again for the thing they already asked for. \
         Asked who a character was and where to find them, an answer said it was a character in \
         ELDEN RING, that it did not know where they were, and that it could look for an \
         article if that was wanted. It had four ways to find out and used none of them.\n\
         \n\
         The only question worth handing back is one you cannot answer even in principle — what \
         they OWN, which is not readable, or which of two things they meant when it changes the \
         answer. Everything else: look, and then say what you found.\n\
         \n\
         ## What the launcher does\n\
         \n\
         It snapshots their save before every launch and before anything that writes, and can \
         restore one; it moves a character between saves and between accounts, and converts \
         between the ordinary save and the co-op one; it installs and orders mods and keeps \
         profiles of them; it writes the Seamless Co-op settings including the shared password; \
         it installs DLSS, frame generation and Reflex and changes them while the game runs; it \
         raises the frame cap and tunes the graphics settings; it mirrors both wikis onto this \
         machine; and it can pin places on their map.\n\
         \n\
         Say what it does rather than sending them to look elsewhere, and do not claim it does \
         something not in that list. Asked whether a save could be rolled back, a model searched \
         two wikis and the open web and answered that it could not find out — from inside the \
         program that takes the snapshot.\n\
         \n\
         It keeps the newest twenty automatic snapshots and prunes the rest; a snapshot taken by \
         hand is never pruned. If asked why old snapshots vanish, that is the answer — not an \
         invented setting. There is no field called \"Max snapshots\" and no separate Saves card \
         under System; the snapshots live on the Saves tab.\n\
         \n\
         ## The two controls you may name\n\
         \n\
         Both are on the Saves tab, on the character itself. A \"Move\" button, which asks which \
         SAVE FILE to move them into and rebinds the account as it goes. And a \"Convert\" \
         button, which is the real answer to \"how do I get this character into co-op\".\n\
         \n\
         Asked that, a model said characters cannot be moved across and talked about passwords \
         instead; asked how to move one to another account, another described a button that asks \
         you to pick an account, which is not what the dialog asks.\n\
         \n\
         ",
    );

    // The controls themselves only where they could possibly be wanted. See
    // `could_be_about_the_launcher`: this is 2,459 characters that a question
    // about a weapon can never use, and the round is well over the size the
    // pool was measured to accept. What does NOT go is the rule that stops the
    // names being invented when they are absent — that stays either way,
    // because the failure it prevents is worse than the one it costs.
    if could_be_about_the_launcher(&player.asked) {
        said.push_str(
            "## The interface, by its real names\n\
         \n\
         These are the actual controls. Use them and no others — a name that is not on this \
         list is one you are inventing, and it sends somebody hunting a button that is not \
         there. This list used to say \"you have never seen the interface, do not describe \
         it\", which was ignored: asked how to back a save up, a model invented a \"Create \
         backup\" button, and asked how to install a mod, another invented an \"Install mod\" \
         button, drag-to-reorder and a \"Save and launch\". Being vague did not stop the \
         inventing; it only made the right answers vaguer too. So here they are.\n\
         \n\
         The tabs down the side: Play, Mods, Saves, Co-op, Codex, Wiki, System. Mods is hidden \
         while an edition is open, Co-op only appears for games that support seamless co-op, \
         and Codex and Wiki only for the game they were built for.\n\
         \n\
         SAVES. Each character has \"Snapshot\", which takes one there and then. \"Snapshots\" \
         opens the list of them, and each has \"Restore\". \"Move\" asks which SAVE FILE to \
         move the character into and rebinds the account as it goes. \"Convert\" is the real \
         answer to \"how do I get this character into co-op\". There is also \"Markers\", and \
         \"Pin a place\" for putting one on the map.\n\
         \n\
         MODS. \"Add mod\" offers \"Add an archive\" and \"Add a folder\". The list is the \
         \"Load order\" card, ordered with move-up and move-down buttons — the list is \
         deliberately drag-free — and \"Installed\" is everything on disk, in use or not. There \
         is \"Options\", \"Conflicts\" for what overlaps what, and \"Folder\" to open it. There \
         is NO save-and-apply button: the order is kept the moment it changes.\n\
         \n\
         CO-OP. The \"Session\" card holds \"Password\", with a button to generate one and \
         another to copy it. Everyone in the session needs the same password.\n\
         \n\
         THE OVERLAY, at SHIFT+F1, is this assistant over the running game. It also carries \
         \"Snapshot the save\", which takes one without leaving the game — that is the answer \
         to \"how do I back up before this boss\", and it beats sending somebody to the Saves \
         tab at the moment they least want to alt-tab out. And \"Picture\", which opens the \
         upscaler and frame-generation settings. And \"Co-op password\", which shows the one \
         this session is using with a button to copy it — read-only there, because the mod \
         reads that file when the game starts, so changing it mid-game would do nothing. \
         Restoring, moving and converting a character are NOT in the overlay: those live on \
         the Saves tab, because they are decisions rather than one safe click.\n\
         \n\
         Where the control you want is not on this list, describe what to do instead of naming \
         a button — \"add it in the mods screen, then put it in order\" is right and useful \
         without inventing anything.\n\
         \n\
         ",
        );
    } else {
        // The guard-rail without the facts. Left out to keep the request
        // inside what the model pool will take; if this turns out to be a
        // launcher question after all, saying so costs a sentence and
        // inventing a button costs somebody a hunt through a screen.
        said.push_str(
            "## The interface\n\
             \n\
             The list of the launcher's actual screens and controls is not in front of you for \
             this question, because it did not look like one about the launcher. So do NOT \
             name a button, a tab or a card: every control name you produce from memory is \
             invented, and models have invented \"Create backup\", \"Install mod\" and \"Save \
             and launch\", none of which exist. Describe what to do instead — \"add it in the \
             mods screen, then put it in order\" — or say plainly that you would need to \
             check.\n\
             \n\
             ",
        );
    }

    said.push_str(
        "## Never name a tool, and never repeat what one says to you\n\
         \n\
         The tools you call are machinery and the player has never heard of them. Asked what \
         was interesting in a region, an answer told them to \"check the damage and drop \
         figures through gear_numbers / what_drops_here\" — two words that mean nothing to \
         anybody outside this program, in the middle of a sentence that was otherwise fine. \
         Call the tool, use what comes back, and write as though you simply knew it. If you \
         need to say where something came from, say \"their own game's tables\" or \"the \
         wiki\", never the name of the thing you called.\n\
         \n\
         AND THE PROSE A TOOL RETURNS IS ADDRESSED TO YOU, NOT TO THEM. Everything a tool says \
         after its figures — what to try next, what a previous answer got wrong, why a search \
         may have missed — is instruction for you and is written in the second person because \
         of it. It is not a draft to hand on. Asked what could go into the physick flask, an \
         answer TRANSLATED the tool's own coaching and printed it: \"that is one word and one \
         word is not enough to conclude anything, search again with another word... when a \
         player asked what spirit summons they had, the model searched for 'дух', found \
         nothing and gave up\". The player was reading a note about a different player and a \
         different question, in their own language, as though it were the answer.\n\
         \n\
         So: take the FIGURES and the FACTS out of what a tool returns. Do what its \
         instructions tell you. Never quote them, never translate them, never summarise them, \
         and never mention that a previous answer got something wrong — those notes exist to \
         steer you and are invisible to everyone else.\n\
         \n\
         ## Answer in their language, but keep the game's names in the game's\n\
         \n\
         Write the answer in whatever language they asked in. An ITEM NAME is different: it \
         has to be the string printed on their own screen, because that is what they will read \
         off a menu or type into a search. Never translate one, however obvious the \
         translation looks.\n\
         \n\
         These come apart the moment somebody asks in a language their game is not in, which \
         happens constantly. Asked in Italian for the lightest shield that blocks fire, an \
         answer gave the figures correctly and then called the shield \"Scudo con volto\" — a \
         name that appears nowhere in their installation, for an item they now cannot find. \
         The names come back from the tools already in the right language; pass them through \
         untouched. If a name would be opaque to them, put your translation in brackets AFTER \
         the real one and never instead of it.\n",
    );
    // What it manages, built from the list itself so the two cannot drift.
    // Asked whether the launcher worked for Sekiro, a model answered that it
    // was for ELDEN RING only — to a player using a program that has managed
    // Sekiro since before this assistant existed.
    let managed: Vec<&str> = crate::games::Game::ALL
        .iter()
        .filter(|game| game.is_playable())
        .map(|game| game.display_name())
        .collect();
    said.push_str(&format!(
        "\n## The games it manages\n\
         \n\
         The launcher is not only for this game. It manages {}, all of them the same way. Do \
         not say it is for ELDEN RING alone.\n\
         \n\
         It does not read all of them the same way, though, and the difference matters. Saves, \
         mods, profiles, backups, graphics and the wiki mirrors work for every one of those \
         titles. The FIGURES — a weapon's damage, an armour's negation, a spell's cost, what \
         drops where, who is standing on a map — are read out of the game's own tables, and \
         those are only read for ELDEN RING: the layout was worked out against it and checked \
         against a second reader of it, and the other titles' files were never available to \
         check. Asked for a number about one of them, say the launcher does not read that \
         game's tables. Do not supply one from memory and do not let a wiki's stand in for \
         theirs.\n",
        managed.join(", ")
    ));
    // Handled and set up are different, and running them together produced the
    // wrong half of both: asked how many characters they had in Sekiro, a model
    // answered that the launcher was for ELDEN RING alone and did not manage
    // Sekiro saves. It does; there is simply no Sekiro here to read.
    if let Some(here) = &player.set_up {
        said.push_str(here);
    }
    // What YOU can do, as against what the program can. These are different and
    // were being conflated in the worst possible place: asked to delete every
    // save, a model replied "confirm and I will" — of an action it cannot take,
    // on the one thing that cannot be got back.
    said.push_str(
        "\n## What YOU can do, as against the launcher\n\
         \n\
         Be exact about who does what. The launcher does all of the above; YOU, answering, can \
         do exactly one thing to their machine, which is put a pin on their map. You cannot \
         delete, restore, move or convert a save, install or remove a mod, change a setting or \
         start the game. For any of those, say what they should do and where — never \"confirm \
         and I will\", never \"I have done that\". Asked to delete every save, a model replied \
         \"confirm and I will\", of an action it cannot take, on the one thing that cannot be \
         got back. The honest answer is that you cannot delete anything, followed by what the \
         launcher keeps and where the restore lives.\n\
         \n\
         The overlay they are talking to opens and closes with SHIFT+F1, anywhere, including \
         over the running game. That is the only key worth telling them and it is not a setting \
         to switch on.\n",
    );
    said.push_str(
        "\n## What is never invented\n\
         \n\
         BEFORE YOU NAME A THING, LOOK IT UP. Every fabrication this launcher has caught has \
         the same shape — a name written down without a tool having returned it. A talisman \
         placed on a boss in a forest and its upgrade beside an altar in a cave, none of which \
         exist. A \"Misericorde of Bloodflame\" that is not in any version of this game. Three \
         spirit ashes invented whole. A weapon recommended by a name remembered from a \
         different game. The rule is not \"try to be careful\": it is that a name you have not \
         seen come back from a tool in THIS conversation does not go in the answer.\n\
         \n\
         AND NEVER CLAIM TO HAVE LOOKED. \"I could not find it in this installation's files\" is \
         a statement about what you DID, and it is a lie if you did not call anything. Asked \
         what a boss gives, an answer said exactly that and then supplied the base game's \
         figure — having made no call at all. It reads as diligence and it is the opposite: a \
         player who is told the files were searched will not go and check. If you did not \
         look, either look or say you have not; \"I have not checked their tables for this\" is \
         honest and takes the same breath. Blaming the tools is the same lie in a different \
         coat: \"none of my tools could read this boss\" was sent after calling none of them, \
         and a tool that was never called did not fail.\n\
         \n\
         AND CHECK YOURSELF BEFORE YOU FINISH. Read back what you are about to send and ask of \
         every specific in it: which tool gave me this? A number, a name, a place, a threshold, \
         a percentage — each one either came back from a call or it did not, and the ones that \
         did not come out. This costs nothing and it is the difference between an answer and a \
         confident guess. Where two sources disagree, say which you are using and why; where \
         you could not establish something, say that plainly, because \"I could not find this\" \
         is a real answer and a plausible substitute is not.\n\
         \n\
         The only names you may use are the ones written out for you in this block. \
         Everything else: describe the action.\n\
         \n\
         What you are deliberately not given: where anything is installed, their save's file \
         path, their Steam id, their account name, their co-op password. That is on purpose — \
         none of it changes an answer and it is nobody else's business. Asked for any of them, \
         say the launcher does not hand them over and that the interface shows it. The launcher \
         holds all of it and chooses not to pass it on, which is not the same as not having it: \
         \"I have no access to that\" is a claim about the launcher and it is false. Say it in \
         their language and in your own words — a phrase lifted out of this block lands in the \
         middle of a Russian answer in English, which one did.\n\
         \n\
         \
         Do NOT produce a plausible one: asked where their game was, a model answered \
         `C:\\Program Files (x86)\\Steam\\steamapps\\common\\ELDEN RING`, which was not the \
         folder, and a made-up path is worse than none because it looks checkable.\n\
         \n\
         Never name an item you have not been shown by a tool or read in an article this turn — \
         and that includes naming one as an EXAMPLE. \"For instance the Meteorite Staff\", \
         \"a shield like the Brass Shield\", \"something such as X\": every one of those is the \
         same invention wearing a hedge, and a player reads it as a thing to go and look for. \
         Asked what their dearest spell cost, a model could not read their spells and named two \
         out of memory to illustrate; asked what a shield would block, another asked which \
         shield and offered two made-up ones to choose between. If you need an example and have \
         not been given one, describe the KIND of thing instead: \"a light shield\", \"a staff \
         that scales with intelligence\". \
         \n\
         \n\
         AND A MECHANIC IS A THING. The worst answer this launcher has produced was to \
         \"why can you not wear two talismans in Convergence?\", and it was: \"there are no \
         talismans in Convergence at all — they were replaced by amulets, and you can only \
         wear one. That is part of the mod's rework.\" Every clause of that is invented, it \
         was written without a single tool call, and it is worse than a wrong number because \
         it sounds like inside knowledge of the conversion.\n\
         \n\
         So: a claim that THIS conversion added, removed, renamed or reworked a mechanic is a \
         claim about the installed files, and it is either read or it is not made. The player \
         will build on it. If the question takes a change for granted and you have not read \
         one, the honest answer is that the premise does not hold as far as you can see, and \
         then what you actually found — not a fuller version of their assumption. A false \
         premise wants agreeing with; agreeing is the failure.\n\
         \n\
         AND REJECTING A FALSE PREMISE DOES NOT LICENCE INVENTING THE TRUE ONE. Asked why the \
         mod allows only two spell slots, an answer correctly said it does not — and then \
         explained that the number comes from intelligence and faith, and that their seal does \
         not scale with faith. Neither of those was read and both are wrong. Half an answer \
         being right is what makes the other half believed. Say the premise does not hold, say \
         what you actually read, and stop: \"how that number is arrived at is not something I \
         can see here\" is a finished answer.\n\
         \n\
         AND WHEN THE PREMISE IS ABOUT AN ATTRIBUTE AND DAMAGE, IT IS TESTABLE — SO TEST IT. \
         Asked why strength no longer affects damage in this conversion, an answer explained \
         at length why it might not: the weapon's scaling could be low, an affinity might have \
         removed it, mods rework these things. Every word of that was reasoning, and none of \
         it was their weapon. The figure was one call away and then one subtraction: work the \
         damage out at the strength they have, work it out again ten higher, and say what \
         changed. If the difference is zero, they were right and now you can say WHY, from \
         their own scaling. If it is not zero, say the number. Never argue with arithmetic you \
         could have done.\n\
         \n\
         Asked how to get a weapon to +10, a model answered out of memory and called the \
         material a \"Somerset Stone\", which is not an item in this game or in any other — and \
         it did not look anything up before saying so. game_item finds what this installation \
         actually calls a thing, gear_numbers and spell_numbers give its figures, and talismans \
         covers those. If a check turns nothing up, say the name is not in their game. An \
         invented item name sends somebody hunting for something that does not exist, which is \
         worse than being told you do not know.\n",
    );
    // A heading of its own, and the same for everything below it that is about
    // one installation rather than about the work.
    //
    // These used to be pushed on bare, which meant each landed inside whichever
    // rule section came last and was read as part of it. Weighing the block
    // showed what that costs: "What is never invented" measures 2,462
    // characters of rules for a bare install and 6,022 for a real one, and
    // "What can be worked out" goes from 860 to 4,193 — the two sections a
    // reader has to follow most closely are the two most buried in somebody's
    // circumstances. Nothing here moves a rule relative to another rule; it
    // only stops a fact about one player pretending to be one.
    if player.edition.is_some() || player.language.as_deref() == Some("russian") {
        said.push_str("\n## This particular installation\n\n");
    }
    if let Some(edition) = &player.edition {
        said.push_str(&format!(
            "This player is running {edition}, a total conversion. It rewrites weapons, spells, \
             classes and enemies, so the base game's figures and the wikis' are not theirs — \
             gear_numbers reads their own installation. Say when something you are telling \
             them is the base game's.\n\
             AND IF THEY SAY OTHERWISE, THEY ARE MISTAKEN AND IT IS WORTH SAYING. Told \"I have \
             a clean game, no mods, what can you even help with?\", an answer listed what it \
             could do and then described their character — never mentioning that {edition} is \
             installed and that everything it was about to tell them came from it. The \
             launcher can SEE what is installed; that is not a matter of opinion, and somebody \
             who thinks they are on the base game will read every figure you give them against \
             the wrong game. Say it plainly and in one sentence, then answer the question.\n\
             AND NEVER SAY IT YOURSELF. \"Since you are on the base game…\" is a sentence an \
             answer has already written, unprompted, to this player — while recommending an \
             item out of the base game to go with it. They are not on the base game. There is \
             no question to which \"you are playing vanilla\" is a true premise here, so if you \
             find yourself writing it, the reasoning that led there is wrong too.\n"
        ));
    }
    // Here rather than behind a tool call, because this is a fact that gets
    // INVENTED. Asked why weapons in this conversion cannot pass +10, a model
    // agreed with the premise and supplied a mechanism for it — the mod
    // "removes the ordinary smithing stones and the somber stones and replaces
    // them with its own materials, which stop at +10" — with no tool call
    // behind any of it. Nearly everything here goes to +15.
    if !player.upgrades_to.is_empty() {
        let ladders: Vec<String> = player
            .upgrades_to
            .iter()
            .map(|(ceiling, how_many)| format!("+{ceiling} for {how_many}"))
            .collect();
        said.push_str(&format!(
            "How far weapons upgrade HERE, walked out of their own installed tables: {}. Two \
             things in the files agree on it — the reinforce ladder has that many steps, and \
             there is a buyable set of materials for every one of them — and the figure is \
             the same for almost every weapon, where the base game splits into two different \
             ceilings. This game stores no upgraded copies of a weapon at all; the levels are \
             worked out from that ladder, so the ladder is the ceiling.\n\
             \n\
             BUT THE MOD'S OWN WIKI SAYS +10, and a model reading it has already answered +10 \
             twice. That disagreement is real and it is not settled. Do not pick a side \
             silently: say what their installed files hold, say the published documentation \
             says otherwise, and let them know it is worth checking at a smith. Never invent a \
             reason for either figure — asked why the cap was +10, an answer explained that the \
             mod \"removes the ordinary and somber stones and replaces them with its own\", \
             which is not in the files and was made up whole.\n",
            ladders.join(", ")
        ));
    }
    // What their game calls each attribute, in THEIR language, read out of the
    // game rather than translated.
    //
    // This is the general form of the Russian paragraph below, and it exists
    // because that paragraph only helped one player. Every localisation names
    // the attributes its own way and none of them map straight across: a
    // German player asking about Geschick, a Portuguese one about Vigor, get
    // nothing, and the model translates — which is how "Faith (FTH) 22" was
    // once rendered into Russian as "Фея", a fairy, beside the correct 22.
    //
    // Nothing here is memory. `attribute_words` comes from GR_MenuText in the
    // installed game, and those entries carry their own proof: the game writes
    // "Стойкость(END)" and "Мудрость(INT)", so the parenthetical says which
    // attribute each word IS. That works for a language nobody here speaks,
    // which is the whole point — the alternative was a hand-written paragraph
    // per language, each one a chance to get somebody's stats wrong from
    // memory.
    //
    // Empty when the game is in English, because then there is nothing to say.
    if !player.attribute_words.is_empty() {
        let pairs: Vec<String> = player
            .attribute_words
            .iter()
            .map(|(english, theirs)| format!("{english} is \"{theirs}\""))
            .collect();
        said.push_str(&format!(
            "\nTheir game names the attributes in its own language, and these are the game's \
             own words for them, not a translation: {}. Map what they wrote to the English \
             name before calling any tool that takes an attribute, and give the figure back \
             using THEIR word. Sending somebody to level the wrong stat costs them the runes.\n",
            pairs.join(", ")
        ));
    }
    // The Russian localisation does not name the attributes the way a
    // translation would, and getting it wrong sends points into the wrong
    // stat. The player themselves corrected this once: "колдовство" is arcane,
    // and intelligence is "мудрость". Kept alongside the general line above
    // because it carries two things GR_MenuText does not: poise and the
    // carrying figure, both asked about constantly and both mis-answered.
    if player.language.as_deref() == Some("russian") {
        said.push_str(
            "Their game is in Russian, which names the attributes in a way that does not \
             translate straight across, and confusing two of them means telling somebody to \
             spend points on the wrong one. In this game: \"колдовство\" is ARCANE, \"мудрость\" \
             is INTELLIGENCE, \"интеллект\" is MIND (the FP stat), \"вера\" is FAITH, \
             \"выносливость\" is ENDURANCE, \"сила\" is STRENGTH, \"ловкость\" is DEXTERITY and \
             \"здоровье\"/\"живучесть\" is VIGOR. Map what they wrote to those before using any \
             tool that takes an attribute.\n\
             Two more off the same screen, because they are asked about constantly and the \
             words do not line up either: \"баланс\" and \"пойз\" are POISE, which is on their \
             line below and is not stamina; \"вес снаряжения\" is the carrying figure, also \
             below. Asked in Russian how much poise they had, an answer replied with their \
             stamina, and another with how many runes they were holding — both numbers off the \
             same screen and neither the one asked for.\n",
        );
    }
    // Who they are, in one line, without being asked for it.
    //
    // This used to sit behind `player_status` on the reasoning that it cost
    // nothing until it was wanted. It cost a great deal. "What level am I" is
    // about the commonest question there is, and behind a tool it takes two
    // round-trips to a model instead of one: measured, 59 seconds to produce
    // eleven characters, against about three for a question the block already
    // answers. The read itself is a handful of bytes out of the running game.
    //
    // Only what is short and constantly asked for goes here. Equipment,
    // talismans, what is near them and the rest stay behind the tool, where
    // they belong — this is a line, not a copy of the character screen.
    if let Some(live) = player.live.as_ref().and_then(|read| read()) {
        said.push_str("\n## This player, as of this second\n\n");
        let listed = live
            .stats
            .iter()
            .map(|(what, value)| format!("{what} {value}"))
            .collect::<Vec<_>>()
            .join(", ");
        // The caveat rides beside the name, not in a paragraph below it. Below
        // it, it does not work: this character is called "Way Of Life", which
        // reads like a build, and asked what class they were the model opened
        // with "You are Way Of Life" and only then said the class is not
        // recorded. A warning three sentences away loses to a word in place.
        said.push_str(&format!(
            "Their character as of this second, read out of the running game: the name they \
             typed for it is \"{}\" — a name they chose, NEVER a class, however much it may read \
             like one; which class they started as is recorded nowhere and cannot be worked out. \
             Level {}, {} runes held, {}/{} HP, {}/{} FP, {}/{} stamina, {}.",
            live.name,
            live.level,
            live.runes,
            live.hp,
            live.hp_max,
            live.fp,
            live.fp_max,
            live.stamina,
            live.stamina_max,
            listed,
        ));
        if let Some(place) = &live.place {
            // The caveat sits with the place name, because the place name is
            // what gets pressed into service. Asked how far the nearest site of
            // grace was, a model took this label and answered that the grace
            // was the place they were standing in. Not one of the 452 names the
            // launcher can see is a grace; they are map labels.
            said.push_str(&format!(
                " Standing in {} — that is a label drawn on the map, not a site of grace. Where \
                 graces are is not readable at all, here or anywhere in this launcher: asked, \
                 say their own map shows them and do not offer a nearest one. NOT ONE, and not \
                 by name — told there were no graces in a region, an answer correctly said \
                 there were and then placed two of them, at a church door and by a bridge, out \
                 of memory of a game that is not the one they are running. Correcting somebody \
                 is not permission to invent the correction.",
                place.name.as_deref().unwrap_or("an unnamed part of the map")
            ));
            // And whose words those are, which is not the game's.
            //
            // The label comes from a survey table this launcher carries, and it
            // is in English whoever is playing. Asked in Russian where they
            // were standing, an answer came back naming "Weeping Peninsula -
            // Castle Morne Rampart" to somebody whose game prints that in
            // Russian and whose armour and items came back in Russian in the
            // same breath — so it reads as the launcher not knowing their
            // language rather than as a name it cannot translate.
            if place.name.is_some() {
                said.push_str(
                    " That name is the launcher's own, in English, and their game prints it in \
                     whatever language they play in. Give it as they wrote their question and \
                     say it is the region rather than quoting it as what their screen shows.",
                );
            }
        }

        // What their armour weighs, added up here rather than by the model.
        //
        // It is a common question and it used to cost five tool calls — the
        // equipment, then every piece separately — which is slow enough that
        // asked it alongside something else the model answered the other thing
        // and told the player to come back about this one. The tables are
        // already open and the addition is four numbers.
        //
        // Only when every worn piece is found. A total with a piece missing
        // from it is worse than no total, because it reads as complete.
        // What is actually in their hands, so "my sword" can be checked rather
        // than taken on trust. Asked why "my Blasphemous Blade" hit so softly,
        // a model looked the weapon up and explained their scaling on it — to
        // somebody carrying a seal and a dagger and no greatsword at all. The
        // useful answer was that they are not holding one.
        // Everything in their hands, read by id — no live call, which is what
        // made this expensive once.
        let held: Vec<(String, crate::formats::regulation::Weapon)> = live
            .gear
            .as_ref()
            .map(|gear| {
                gear.weapon_ids
                    .iter()
                    .filter_map(|(name, id)| Some((name.clone(), (player.weigh)(*id)?)))
                    .collect()
            })
            .unwrap_or_default();

        if let Some(gear) = &live.gear {
            if !gear.weapons.is_empty() {
                said.push_str(&format!(
                    " In their hands right now: {}. When they call something \"my\" weapon and \
                     it is not on that list, say so before anything else — they are either \
                     asking about something in storage or thinking of another character, and \
                     explaining their scaling on a weapon they do not carry answers nobody.",
                    gear.weapons.join(", ")
                ));
                // What each one actually deals, beside its name. The names
                // alone were not enough: asked whether to punch instead, a
                // model read this list, decided from memory of the base game
                // that the dagger on it was a curved sword dealing magic, and
                // built its advice on both. It is a dagger and it deals fire,
                // and either fact was one call away and never fetched.
                let dealing: Vec<String> = held
                    .iter()
                    .filter(|(_, weapon)| !weapon.damage.is_empty())
                    .map(|(name, weapon)| {
                        let kinds: Vec<String> = weapon
                            .damage
                            .iter()
                            .map(|(kind, value)| format!("{kind} {value}"))
                            .collect();
                        // The skill by name, because the name gets invented
                        // otherwise: a model called this dagger's "Кровь
                        // королей" without asking, where the tables say "Кров.
                        // клинок Редувии".
                        let art = (player.skill_on)(weapon.id)
                            .map(|(named, _)| format!(", skill \"{named}\""))
                            .unwrap_or_default();
                        // And what it builds up, because without it the figure
                        // gets filled in: asked how much bleed their dagger
                        // put on, an answer said 55 with nothing on this line
                        // to read and the tables saying 82.
                        let builds = if weapon.ailments.is_empty() {
                            String::new()
                        } else {
                            format!(
                                ", builds {}",
                                weapon
                                    .ailments
                                    .iter()
                                    .map(|(what, value)| format!("{what} {value}"))
                                    .collect::<Vec<_>>()
                                    .join(" and ")
                            )
                        };
                        format!("{name} deals {}{builds}{art}", kinds.join(" and "))
                    })
                    .collect();
                if !dealing.is_empty() {
                    said.push_str(&format!(
                        " What those deal, out of this installation's own tables: {}. What is \
                         listed after \"builds\" is all SEVEN ailments — poison, rot, bleed, \
                         curse, frost, sleep, madness — so what is missing from that line the \
                         weapon genuinely does not build. Three of the seven could not be read \
                         until recently and answers had to say so; that caveat is now wrong and \
                         repeating it would refuse a figure that is right there. A total \
                         conversion changes the damage a weapon does and even what kind of \
                         weapon it is, so never describe one of theirs from memory of the base \
                         game — these figures are the base at no upgrade, and gear_numbers gives \
                         the rest.",
                        dealing.join("; ")
                    ));
                }
            }
            let found: Vec<(String, f32, Option<f32>)> = gear
                .armour
                .iter()
                .filter_map(|(_, name)| {
                    let matched = (player.armour)(name)?;
                    let piece = matched.first()?;
                    Some((name.clone(), piece.armour.weight, piece.armour.poise))
                })
                .collect();
            // The weapons weigh something too, and leaving them out made "what
            // does all my gear weigh" unanswerable: the model gave the armour
            // total and said the weapons' weights were "not in the data", when
            // the same tables carry them and a separate question gets 2.5 for
            // the dagger without trouble.
            let carried: Vec<(String, f32)> =
                held.iter().map(|(name, weapon)| (name.clone(), weapon.weight)).collect();

            if !gear.armour.is_empty() && found.len() == gear.armour.len() {
                let total: f32 = found.iter().map(|(_, weight, _)| weight).sum();
                let each: Vec<String> = found
                    .iter()
                    .map(|(name, weight, _)| format!("{name} {weight:.1}"))
                    .collect();
                // Poise, added here for the same reason the weight is: it is
                // asked constantly and the block is where a short question
                // gets answered. Left to the tool, "how much poise have I" came
                // back "97 — that is your stamina", which is a different
                // number off a different line of their screen.
                let poise: f32 = found.iter().filter_map(|(_, _, poise)| *poise).sum();
                if poise > 0.0 {
                    said.push_str(&format!(
                        " Their poise is {poise:.2}, being the four worn pieces added up — the \
                         screen rounds that to a whole number, so say what this says and let \
                         them see their own rounding. It is NOT their stamina and NOT their \
                         endurance; those are different lines and both are above.\n\
                         HOW MUCH POISE IS ENOUGH IS NOT READABLE. The figure above is; the \
                         threshold at which a particular attack stops staggering them is not \
                         in any table this launcher reads. Asked how much they needed, an \
                         answer gave their real {poise:.2} and then \"about 50 to 60 for \
                         stability, 100 or more against everything\" as though it were the \
                         same kind of fact. It is folklore about a different game, and this \
                         conversion rewrote the numbers it came from. Give what is read, say \
                         the threshold is not something you can see, and leave it there.",
                    ));
                }
                // The prohibition travels with the number, because the number
                // is what invites it. Handed the total on its own, a model
                // offered to work out how much more they could carry "at 70% of
                // endurance" — a rule that does not exist, from a maximum that
                // is not readable anywhere.
                // And what that weight MEANS, which is the half nobody could
                // answer. Their endurance is on the live line above; the
                // maximum comes off the game's own curve.
                let endurance = live
                    .stats
                    .iter()
                    .find(|(what, _)| what.starts_with("Endurance"))
                    .map(|(_, value)| *value);
                let carrying: f32 =
                    total + carried.iter().map(|(_, weight)| weight).sum::<f32>();
                if let Some(most) = endurance.and_then(|end| (player.load)(end)) {
                    let share = if most > 0.0 { carrying / most * 100.0 } else { 0.0 };
                    let band = match share {
                        _ if share < 30.0 => "a light load, which is the fast roll",
                        _ if share < 70.0 => "a medium load",
                        _ if share <= 100.0 => "a heavy load, which is the slow roll",
                        _ => "OVERLOADED, which is barely able to move",
                    };
                    said.push_str(&format!(
                        " Their armour weighs {total:.1} all together — {}. With what they are \
                         holding that is {carrying:.1} of {most:.1}, {share:.0}% — {band}. The \
                         maximum is worked out from their endurance off the game's own curve \
                         and is the figure their equipment screen shows; the bands are 30% and \
                         70%. All of that is read, so give it as read and do not round it into \
                         something else.\n\
                         What that leaves them is {:.1}. Say what a piece would cost against \
                         that rather than guessing whether it fits.",
                        each.join(", "),
                        (most - carrying).max(0.0)
                    ));
                    // What LEVELLING it would give, off the same curve. Asked
                    // whether endurance was worth taking to 40, an answer said
                    // each point adds "roughly 1.2 to 1.5, the exact figure is
                    // read from the game's tables and not from a wiki" — from
                    // inside the program that reads the curve. It is not a
                    // straight line and it cannot be guessed at; here it is.
                    if let Some(here) = endurance {
                        let landmarks: Vec<String> = [here + 5, here + 10, 25, 30, 40, 50, 60, 99]
                            .into_iter()
                            .filter(|point| *point > here)
                            .collect::<std::collections::BTreeSet<u32>>()
                            .into_iter()
                            .filter_map(|point| {
                                let then = (player.load)(point)?;
                                Some(format!("{point} → {then:.1}"))
                            })
                            .collect();
                        if !landmarks.is_empty() {
                            said.push_str(&format!(
                                "\nWhat RAISING endurance would give them, off the same curve, \
                                 from {here} at {most:.1} now: {}. The curve is not a straight \
                                 line and the gain per point changes along it, so work any \
                                 \"is it worth levelling\" question off these figures and \
                                 never off a rate per point.",
                                landmarks.join(", ")
                            ));
                        }
                    }
                } else {
                    said.push_str(&format!(
                    " Their armour weighs {total:.1} all together — {}. That total is added up \
                     from this installation's own tables; use it as it stands and do not ask \
                     which piece they meant. What they can carry in TOTAL is not readable and \
                     there is no formula for it here: do not state a maximum, a percentage, a \
                     roll threshold or how much they have left, and do not offer to work any of \
                     them out. Nor may you say this total IS a limit, or is near one, or that \
                     nothing heavier will fit — asked whether to wear something heavier, a model \
                     announced that their 9.0 was the game's maximum and that heavier was \
                     physically impossible, which invents the same number by describing it \
                     instead of printing it. Say what it weighs; the limit is on their \
                     equipment screen and only they can see it.\n\
                     Nor may you offer one as a test. Asked whether a greatshield was worth \
                     carrying, an answer said \"check your limit — if it is 23.0 or lower the \
                     shield will overload you\". Their limit was 49.8. A guessed threshold does \
                     not stop being a guess by being phrased as a question for them, and this \
                     one was wrong by half and the advice built on it was wrong with it.",
                        each.join(", ")
                    ));
                }
                // Everything at once, for "what does all my gear weigh". Added
                // up here, and only when every piece and every weapon was
                // found — a total missing an item reads as complete.
                if !carried.is_empty() && carried.len() == gear.weapon_ids.len() {
                    let all: f32 = total + carried.iter().map(|(_, weight)| weight).sum::<f32>();
                    let held: Vec<String> = carried
                        .iter()
                        .map(|(name, weight)| format!("{name} {weight:.1}"))
                        .collect();
                    said.push_str(&format!(
                        " In their hands: {}. EVERYTHING they are wearing and holding comes to \
                         {all:.1} — use that for \"all my gear\" rather than the armour figure \
                         on its own.",
                        held.join(", ")
                    ));
                    // Read rather than assumed. Talismans weigh something and
                    // are not in that total; saying they have none when they
                    // have four would be a wrong total dressed as a complete
                    // one.
                    if gear.talismans.is_empty() {
                        said.push_str(" They have no talismans on, so nothing is missing from it.");
                    } else {
                        said.push_str(&format!(
                            " Their talismans — {} — are NOT in that total and do weigh \
                             something; say the figure is gear without talismans.",
                            gear.talismans.join(", ")
                        ));
                    }
                }
            }
        }
        said.push_str(
            " Those three pairs are POOLS — how much health, focus and stamina they have and \
             have left. None of them is the cost of anything: asked what their most expensive \
             incantation cost, a model took the 113 out of their FP pool and reported it as the \
             spell's price, on a spell it had also got wrong. A cost comes from the spell's own \
             row and nowhere else.\n\
             Answer anything about their level, runes, health or attributes straight from this \
             line. Calling player_status for them returns these same numbers and costs the \
             player most of a minute. Asked what class they are, say it is not recorded and that \
             their attributes and gear are what can be seen — do not answer with their name. \
             What the next level costs is not readable either: say it is on their level-up \
             screen rather than giving a figure. player_status is for what they have EQUIPPED, \
             what is near them, and anything not on this line.\n",
        );
    }
    // Everything from here down is this player's circumstances, not the work.
    // One heading for the lot, for the reason given at `## This particular
    // installation` above: without it these land inside "What can be worked
    // out, and what cannot" and are read as part of it.
    // The frame-rate story is the longest thing in this section and is only
    // ever wanted by a question about the machine. What is NOT gated is the
    // count of what the mod holds, just below: "how many weapons does this mod
    // have" is a question about the GAME and matches none of the launcher
    // words, and it is exactly the question that was once answered "127 and
    // 214" off no count at all.
    let machine = player.frames.as_ref().filter(|_| could_be_about_the_launcher(&player.asked));
    if machine.is_some() || player.holdings.is_some() {
        said.push_str("\n## What is true of this machine right now\n");
    }
    if let Some(frames) = machine {
        said.push_str(frames);
    }
    // In front of the model rather than behind player_status, because "how many
    // weapons does this mod have" does not look like a question about their
    // character and the tool never got called. Answered from memory it came
    // back "127 and 214, from your installation", counted from nothing.
    if let Some(holdings) = &player.holdings {
        said.push_str(holdings);
    }
    // Every character in the save, not just the one loaded into the game.
    //
    // Asked what had become of their second character, the assistant answered
    // that there was only one — to somebody whose save holds two. It was
    // reading the line above, which is whichever character the running game has
    // open, and taking that for the whole of it. The save has always been read;
    // the list simply never reached here.
    if !player.characters.is_empty() {
        let listed: Vec<String> = player
            .characters
            .iter()
            .map(|(name, level, seconds)| {
                // Hours worked out here. Handed a count of seconds, a model
                // divides it in prose and the answer is whatever it divided by.
                let hours = *seconds / 3600;
                let minutes = (*seconds % 3600) / 60;
                format!("\"{name}\" at level {level}, {hours}h {minutes}m played")
            })
            .collect();
        said.push_str(&format!(
            "Their save holds {} character{}: {}. That is all of them — if they ask about \
             another one, there is no other one. Hours played are exact, out of the save; do \
             not recompute them.\n\
             THIS GAME'S SAVE, and no other. Asked how long they had played across all their \
             games, an answer added these two together and gave the total as though it were \
             the answer — the other titles the launcher manages have their own saves and this \
             says nothing at all about them. Adding up the characters of one game is fine; \
             calling that sum \"all your games\" is not.\n",
            listed.len(),
            if listed.len() == 1 { "" } else { "s" },
            listed.join("; ")
        ));
    }
    // What everything here IS, by version. All three were asked for and all
    // three were answered with a shrug or a guess: the game's build came back
    // as "the Convergence version", and the launcher's own as something it
    // could not check.
    {
        let mut versions =
            format!("Roundtable itself is version {}.", env!("CARGO_PKG_VERSION"));
        if let Some(game) = &player.version {
            versions.push_str(&format!(" The game's own build is {game}."));
        }
        versions.push_str(
            " Whether a newer Roundtable exists is checked in the launcher, not from here — say \
             it is on its own screen rather than guessing either way.\n",
        );
        said.push_str(&versions);
    }
    // What an attribute DOES is not readable, and saying so is the whole fix.
    //
    // Two questions kept coming back wrong however the prompt was worded: what
    // endurance gives beyond stamina (answered with poise, which comes from
    // armour, and with physical damage reduction, which it does not give), and
    // how much a weapon would gain from ten more faith. Both were answered
    // confidently and from memory. Nothing here can contradict them, which is
    // why prompt pressure alone kept failing — the launcher reads what a weapon
    // has, not what a point spent on it would buy.
    said.push_str(
        "\n## What can be worked out, and what cannot\n\
         \n\
         What an attribute does to a WEAPON'S DAMAGE is readable, exactly, and must be read \
         rather than estimated: gear_numbers takes the attributes to work it out at, runs the \
         game's own scaling curves, and gives the figure their stat screen would show. Asked \
         what ten more faith would buy, ask it twice — once at what they have, once at what they \
         are considering — and give the difference. Never a guess, never \"about forty\". Pass \
         only the attribute being changed; every one you leave out stays at what they have.\n\
         \n\
         Two more ARE readable now and were not before. What endurance lets them CARRY comes \
         off the game's own curve and is on their line above with the share and the roll band; \
         so does the POISE on any piece of armour, which adds up across the four worn. Both are \
         read, so give them as read.\n\
         \n\
         Everything ELSE an attribute does is not readable from here: what endurance gives \
         beyond carrying and stamina, how a stat changes anything that is not weapon damage. \
         Say what you know and mark the rest as how the base game generally works, which under \
         a total conversion may not be theirs at all.\n\
         \n\
         DO NOT ASK THEM FOR WHAT THIS BLOCK ALREADY SAYS. Their level, their attributes, what \
         they are wearing and holding, which mods they have and where they are standing are all \
         above — asking for any of it wastes their turn and reads as not having looked. Asked \
         which ash of war suited their build, an answer said it would need to know more about \
         their build, to somebody whose attributes were in front of it. If a recommendation \
         needs narrowing, narrow it on what is here and say what you narrowed on.\n\
         \n\
         A LONG LIST IS AN ANSWER, NOT A FAILURE. Handed a hundred and twenty-seven matches, an \
         answer reported that it could not find enough information. Pick from them, on their \
         attributes, and say why those. \"Too many results\" is a thing to sort, and the sorting \
         is the work.\n",
    );
    // The last three are this installation's again, and this is the last
    // section, so without a heading of their own they fall inside "What can be
    // worked out, and what cannot" — which measured 860 characters of rules
    // bare and 4,193 live, nearly all of it these.
    // The state of their backups and which wikis are mirrored are launcher
    // facts and go the same way as the rest. What STAYS whatever the question
    // is, is `safety`: whether the anti-cheat shim is in place decides whether
    // playing online gets their account banned, the question can be phrased
    // without a single word this gate knows, and being confidently wrong about
    // it costs more than a long prompt ever will.
    let about = could_be_about_the_launcher(&player.asked);
    let backups = player.backups.as_ref().filter(|_| about);
    let mirrors = player.mirrors.as_ref().filter(|_| about);
    if player.safety.is_some() || backups.is_some() || mirrors.is_some() {
        said.push_str("\n## What is kept here, and where\n");
    }
    if let Some(safety) = &player.safety {
        said.push_str(safety);
    }
    if let Some(backups) = backups {
        said.push_str(backups);
    }
    if let Some(mirrors) = mirrors {
        said.push_str(mirrors);
    }
    if player.seamless {
        said.push_str(
            "They have Seamless Co-op installed. Playing together works nothing like the base \
             game's: both players stay in one world for the whole run, bosses and progress \
             included, with no summon signs and no Furlcalling Finger Remedy involved. They join \
             by sharing a password in the mod's settings — which this launcher writes for them — \
             and starting through its own launcher. Any question about playing with somebody is \
             about that, not about the vanilla system. There is no Host button and no Join \
             button — a model invented both. Whoever loads their world first is the host, and \
             the only thing either of them sets is the shared password.\n\
             \n\
             That password lives on the launcher's Co-op tab, on the \"Session\" card, in the \
             field called \"Password\", with a button beside it to generate one and another to \
             copy it. Send them there and nowhere else. Asked where it was, a model sent them to \
             the upscaling card three times running and eventually told them the field was on \
             it, because that was the only screen it had ever been told the name of. You are not \
             given the password itself, only where they can see it.\n\
             \n\
             How many people a session holds is NOT readable — the launcher writes the mod's \
             settings but does not surface that one. Asked, say it is set in the mod's own \
             configuration and you cannot see it; do not answer \"two\", which a model did and \
             which would stop somebody inviting a third.\n\
             \n\
             Answer the co-op question from that line and nothing else. Asked how to call a \
             friend in, one model spent fifty seconds describing golden summon signs and the \
             Furlcalling Finger Remedy — the system this mod replaces — and sent the player \
             hunting for an item that does nothing for them. Summon signs, remedies and \
             fingers do not come into it at all. If you catch yourself writing any of those \
             words, the answer is wrong.\n",
        );
    }

    // The last thing in the block, because last is what gets followed. That
    // has been true every time it was tested here: a prohibition moved to the
    // end started being obeyed, and the same words in the middle did not.
    //
    // This one earns the place because it is the failure that wastes the
    // player's turn outright. Asked whether to take a bow and which, an answer
    // listed their attributes, said it would need to check what bows the game
    // had, wrote "let me look" — and stopped, with the catalogue untouched and
    // three rounds still available to it.
    said.push_str(
        "\n---\n\
         \n\
         Before you send: are you ASKING them for something, or telling them you are about to \
         look? Both are the same mistake. You hold every source and they hold none, so a turn \
         that ends in \"let me check\" or \"tell me which one and I will look\" has spent their \
         question and returned nothing. Look first, then write. The only thing worth asking \
         back is what genuinely cannot be read — what they own, or which of two things they \
         meant when it changes the answer — and even then, answer for both and say so.\n",
    );

    Some(said)
}

/// Asks, letting the model do the looking.
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

    // The handful of facts that change what a correct answer even is, in front
    // of the model from the first word rather than behind a tool it has to
    // think to call. Asked how to play with a friend, it explained summon signs
    // to somebody running Seamless Co-op — it never looked, because nothing
    // about the question suggested their installation mattered.
    let built = std::time::Instant::now();
    if let Some(setup) = setup_worth_knowing(player) {
        // Its size as well as its cost. Every fix that lands here makes it
        // longer, and a block that has quietly grown past what a model will
        // hold in mind is a block whose oldest rules stop being followed —
        // which looks exactly like a new bug and is not one.
        note_timing(&format!("setup_worth_knowing ({} chars)", setup.len()), built);
        if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
            for (size, name) in setup_by_section(&setup).iter().take(8) {
                eprintln!("[block] {size:>6}  {name}");
            }
        }
        messages.push(serde_json::json!({ "role": "system", "content": setup }));
    } else {
        note_timing("setup_worth_knowing (nothing to say)", built);
    }

    for turn in history.iter().rev().take(4).rev() {
        messages.push(serde_json::json!({ "role": "user", "content": turn.question }));
        messages.push(serde_json::json!({ "role": "assistant", "content": turn.answer }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": question }));

    let tools = tool_schemas();
    let mut sources: Vec<String> = Vec::new();
    let mut steps: Vec<String> = Vec::new();
    // How many tools have actually run. Counted rather than taken from `steps`,
    // which only grows when a tool returns a note and several do not.
    let mut called = 0usize;
    // Whether the model has already been sent back to look. Once only — if it
    // will not call anything twice, the second answer is what there is.
    let mut told_to_look = false;
    // The lane that answered the last round, so the next round can ask it
    // first and land on a cache it has already paid for. See the `prefer`
    // field below.
    //
    // Starts from whoever answered the LAST QUESTION rather than from nothing,
    // because the head of the request is the same for every question this
    // installation asks — so a lane that served the last one is already warm
    // for this one, and round 0 is otherwise always cold. See LAST_GOOD_LANE.
    let mut warm_lane: Option<String> = lane_from_last_time();
    // Whether a whole article or page has been read. See where it is set.
    let mut read_prose = false;

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

        // The language reminder rides on every round, because any round can be
        // the one that answers — see `answer_in_their_language`. Added to the
        // request rather than to `messages`, so it stays last instead of being
        // buried under the next round's tool results.
        let mut sending = trimmed(&messages);
        sending.push(answer_in_their_language(question));

        // In brief on every round, including the first.
        //
        // The full form was kept for round zero on the reasoning that it is
        // where the choosing happens and the argument is worth its length. It
        // is — but the round it made was 39,323 characters, about ten thousand
        // tokens, and that is more than the best lane in the pool will take.
        // Measured against the live service rather than reasoned about:
        //
        //   groq/gpt-oss-120b   413 Request too large for model
        //   groq/llama-3.3-70b  429 Rate limit reached for model
        //   nvidia/minimax      429 Too Many Requests
        //   nvidia/llama-3.3-70b answered, in 18.9 seconds
        //
        // Every fast lane refused and the slowest one served it. That is the
        // whole reason questions had been failing with "every model is busy":
        // seven of eight in one battery, while a 354-byte probe through the
        // same pool answered in under a second. The pool was never the problem.
        //
        // Brief throughout puts a round at about 28,000 characters, which the
        // same lanes accept. A tool description that persuades nobody because
        // the request was rejected persuades nobody.
        let offered = if last { serde_json::Value::Null } else { tools_in_brief(&tools) };

        let body = serde_json::json!({
            "messages": sending,
            "tools": offered,
            "edition": edition,
            "stream": false,
            // Which lane answered the round before this one. The service cannot
            // know — each round is its own request and it keeps nothing between
            // them — but this side does, and telling it is worth a great deal.
            //
            // A prefix cache belongs to one provider AND one account, and the
            // pool deliberately spreads work across eighty-eight lanes so that
            // ten accounts are really ten. Right across questions, wrong within
            // one: measured, two rounds of a single question went to
            // nvidia/llama-3.3-70b and then mistral/medium#2, each warming a
            // cache the next round threw away. The one lane that did get a
            // second visit served 13,824 of 14,067 prompt tokens from cache.
            "prefer": warm_lane,
        });

        // What is actually going over the wire, per round.
        //
        // Everything here is bounded except one thing, and the only way to know
        // whether that matters is to weigh it: the setup block reports its own
        // size, an article is cut at `ARTICLE`, an older result at `STALE` —
        // and the previous turns of the conversation are carried whole. A round
        // that has quietly grown to a size no free tier will take looks, from
        // the outside, exactly like the pool being down.
        note_size(round, &body);

        let asked_at = std::time::Instant::now();
        let reply = match post_chat(http, &body).await {
            Ok(reply) => {
                // Remember who answered, so the next round can ask them first
                // and land on a prefix they have already paid to compute.
                if reply.lane.is_some() {
                    warm_lane.clone_from(&reply.lane);
                    remember_lane(&reply.lane);
                }
                reply
            }
            Err(error) => {
                // The words the service actually used, where somebody watching
                // can see them. A question that dies here never reaches the
                // per-lane breakdown below, so without this the only thing
                // recorded is the sentence written for the player — and that
                // sentence is deliberately the same for several different
                // causes. Two wrong diagnoses in one afternoon came of it.
                if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                    eprintln!("[failed] round {round}, nothing came back: {error}");
                }
                emit(Event::Failed { error: in_plain_words(&error) });
                return;
            }
        };
        note_timing(&format!("round {round} at the service"), asked_at);

        if let Some(error) = reply.error {
            // The reasons go into the log rather than at the player, who cannot
            // act on "context length exceeded" — but somebody reading a bug
            // report can, and without them a total failure says nothing at all.
            if !reply.tried.is_empty() {
                let why: Vec<String> = reply
                    .tried
                    .iter()
                    .map(|one| format!("{}: {}", one.lane, one.why))
                    .collect();
                tracing::warn!("every lane refused — {}", why.join(" | "));
                // And where somebody is actually watching. `tracing` has no
                // subscriber here — by choice, this launcher keeps no log —
                // so without this the one line that says WHY a question failed
                // goes nowhere at all, and a size problem, a rate limit and an
                // outage are indistinguishable from the outside. Measured
                // rather than guessed is the whole discipline; this is what
                // makes it possible on the path that matters most.
                if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                    eprintln!("[lanes] round {round} refused by {}:", reply.tried.len());
                    for one in &reply.tried {
                        eprintln!("        {} — {}", one.lane, one.why);
                    }
                }
            }
            emit(Event::Failed { error: in_plain_words(&error) });
            return;
        }

        if reply.tool_calls.is_empty() {
            // Nothing has been called all question, and the question is about
            // the GAME. Whatever is about to be said is memory, and memory is
            // about a different game than the one installed here.
            //
            // Twice on the same question, batteries 46 and 47: "сколько
            // великих рун в этом моде?" answered "В базовой игре их 8, но в
            // The Convergence эта механика убрана", and then, on the retest,
            // "их нет в файлах установленной версии" — a claim about FILES
            // that nothing had opened. `ungrounded_names` cannot see either:
            // there is no invented name, only a number and a sentence. Also
            // "что падает с Морготта?" — "я не смог найти информацию", with
            // nothing called to fail.
            //
            // Gated on the question NOT being about the launcher, because the
            // launcher's own answers are the ones that legitimately need no
            // tool: "Нажми Snapshot на вкладке Saves... в оверлее (Shift+F1)"
            // is right, needs nothing looked up, and carries a digit — so any
            // rule built on digits alone would have broken it.
            //
            // The round this replaces is the CHEAP kind: no tool ran, so there
            // is no tool output in it. One cheap round spent on exactly the
            // answers most likely to be wrong.
            // The exemption for launcher questions has one hole, and it was
            // measured rather than reasoned about. Asked why the launcher had
            // deleted their mods, the answer was "это вопрос про лаунчер, а не
            // про игру… это к разработчикам Roundtable" with nothing called —
            // and a rule spelled out in the block did NOT change it on the
            // retest. Third time this session that written prose lost to a
            // check, so this is a check.
            //
            // The exemption is right for the ordinary case: "how do I back up a
            // save" is answered correctly out of the block, needs no tool, and
            // must not be sent back. What is not right is an answer that HANDS
            // THE QUESTION BACK — that is not an answer at all, and the
            // launcher can always say what is enabled right now.
            // Read from the reply rather than from `already`, which is not in
            // scope until further down — this runs before the answer is
            // released, which is the whole point of it.
            let handed_back = called == 0
                && reply
                    .content
                    .as_deref()
                    .is_some_and(pushed_the_question_away);
            if !last
                && !told_to_look
                && called == 0
                && (!could_be_about_the_launcher(question) || handed_back)
            {
                told_to_look = true;
                if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                    eprintln!("[unlooked] answered with no tool call, sending it back");
                }
                messages.push(serde_json::json!({
                    "role": "system",
                    "content":
                        "STOP. That answer was not sent. You have not called a single tool for \
                         this question, so everything in it is memory — and this installation is \
                         not the base game, so memory is about a different one. Its numbers, its \
                         item names and what it does or does not contain are all in ITS tables, \
                         and you have not opened them. Look now. If nothing you have can answer \
                         it, say plainly that you have not checked and what you would need — \
                         that is a real answer and a plausible one is not. Never say a thing is \
                         absent from their files unless you looked in them. Do not mention this \
                         correction.",
                }));
                continue;
            }

            if !sources.is_empty() {
                emit(Event::Sources { sources: sources.clone() });
            }

            // The answer is already here.
            //
            // This round asked "do you want a tool?", the model said no, and
            // said no by writing the answer instead. Asking a second time, with
            // streaming on, used to be how the answer got out — and it meant
            // every question that needed no tool at all was paid for twice.
            // Measured on "what level am I", which needs nothing looked up: the
            // streamed round takes about a second and a half, and the player
            // waited nearly eight, because the first round had already produced
            // the whole answer and it was thrown away.
            //
            // So it is used. Nothing is lost but the typing effect, and an
            // answer that arrives in one piece four seconds sooner is the
            // better trade — these are the short ones, where there was barely
            // anything to watch appear.
            //
            // Only when it is real prose. An empty reply, or one that is the
            // model writing a tool call out as text, goes the long way round,
            // where the guard for that lives.
            let already = reply.content.as_deref().map(str::trim).unwrap_or_default();
            if !already.is_empty() && !leaked_a_tool_call(already) {
                // Check it against what the tools actually said before it goes
                // anywhere. This is the path both recorded inventions took: the
                // talisman that opens a second slot came after one call to
                // player_status, and the "all classes are about equal in the
                // base game" answer came after none at all. In both the model
                // stopped calling and wrote, which is exactly here.
                //
                // Worth doing HERE and not after streaming, because here the
                // answer still exists as one string that nobody has seen. Once
                // it is streamed it cannot be taken back. See `ungrounded_names`
                // for why names and not numbers.
                // A reason invented for something the answer itself says it
                // could not find. Checked before the rest because it is the
                // most confident-sounding failure there is: it survives every
                // other check here by calling tools first and inventing after.
                if !told_to_look && reasoned_past_its_own_doubt(question, already) {
                    told_to_look = true;
                    if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                        eprintln!("[unfounded] a reason given for something it did not find");
                    }
                    messages.push(serde_json::json!({
                        "role": "system",
                        "content":
                            "STOP. That answer was not sent. It explains WHY something is so, \
                             and in the same breath says it could not find anything that says \
                             it IS so. Then the explanation is yours, not the game's. Their \
                             question assumed it; that assumption is the thing to check, and if \
                             nothing confirms it the answer is that it could not be confirmed — \
                             say that FIRST, plainly, before anything else. Do not offer a \
                             reason for something you have not established. Do not mention this \
                             correction.",
                    }));
                    continue;
                }

                // The same fault about a QUANTITY: it says the number cannot be
                // read and then supplies one. Sharing `told_to_look` with the
                // check above on purpose — one correction per answer, or two
                // that disagree can push it round the loop twice.
                if !told_to_look
                    && figure_it_said_it_could_not_read(
                        question,
                        already,
                        &facts_so_far(&messages),
                    )
                {
                    told_to_look = true;
                    if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                        eprintln!("[unread] a figure given for something it could not read");
                    }
                    messages.push(serde_json::json!({
                        "role": "system",
                        "content":
                            "STOP. That answer was not sent. It says the figure cannot be read \
                             here, and then gives one. Wherever that number came from it was not \
                             from this installation, and the player cannot tell the difference — \
                             it arrives looking exactly like the numbers the tables really did \
                             give them. If you could not establish it, say only that, plainly. A \
                             figure from the base game is not an answer about THEIR game unless a \
                             tool in this conversation returned it. Do not mention this \
                             correction.",
                    }));
                    continue;
                }

                // A tab that does not exist. Same fault, aimed at the launcher
                // rather than the game, and worse for the player: they go and
                // look for it.
                if !told_to_look {
                    if let Some(tab) = names_a_tab_that_is_not_there(already) {
                        told_to_look = true;
                        if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                            eprintln!("[notab] {tab}");
                        }
                        messages.push(serde_json::json!({
                            "role": "system",
                            "content": format!(
                                "STOP. That answer was not sent. It sends them to a tab called \
                                 \"{tab}\", and there is no such tab. There are seven and they \
                                 are named in your instructions: Play, Mods, Saves, Co-op, Codex, \
                                 Wiki, System. Upscaling and frame generation are under System, \
                                 and the overlay carries them too. Write the answer again naming \
                                 a real one. Do not mention this correction."
                            ),
                        }));
                        continue;
                    }
                }

                // A tool's name in the answer is the same class of fault as an
                // invented one — something the player cannot act on — and it is
                // caught in the same place, while the answer is still unsent.
                if let Some(named) = names_a_tool(already) {
                    if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                        eprintln!("[toolname] {named}");
                    }
                    messages.push(serde_json::json!({
                        "role": "system",
                        "content": format!(
                            "STOP. That answer was not sent. It says \"{named}\" to the player. \
                             That is the name of one of your own tools and means nothing to \
                             them — they cannot call it, look it up, or find it in the \
                             launcher. Write the answer again saying what you can DO in plain \
                             words: not \"I can check with {named}\" but \"I can look that up in \
                             your game's tables\". Do not mention this correction."
                        ),
                    }));
                    stream_final(http, &messages, edition, question, &mut emit).await;
                    return;
                }
                // Not after an article has been read — see `read_prose`. The
                // names in that case are usually the article's own, translated
                // into the player's language, and blocking them cost a correct
                // answer outright.
                let invented = if read_prose {
                    Vec::new()
                } else {
                    ungrounded_names(already, &facts_so_far(&messages), question)
                };
                if invented.is_empty() {
                    emit(Event::Delta { text: already.to_string() });
                    emit(Event::Done { lane: reply.lane.clone(), ms: reply.ms, cut: false });
                    return;
                }
                if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
                    eprintln!("[ungrounded] {invented:?}");
                }
                // One correction, naming what was wrong, and then the answer is
                // written again. Not a refusal: the question is usually
                // answerable and only the invented part has to go.
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": format!(
                        "STOP. That answer was not sent. It names {} which NO tool in this \
                         conversation returned: {}. Either it does not exist in this \
                         installation or you have not looked it up — and you cannot tell which \
                         from memory, so neither can the player. Write the answer again without \
                         it. If a name is the whole answer, look it up first; if it cannot be \
                         found, say plainly that this installation has no such thing. Do not \
                         mention this correction.",
                        if invented.len() == 1 { "something" } else { "things" },
                        invented.join(", ")
                    ),
                }));
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

        // What the lane said about caching, which decides whether the 45,000
        // stable characters at the head of every request cost anything against
        // the per-minute ceiling. See `Reply::cached`.
        if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
            // The LANE matters as much as the number. A prefix cache belongs to
            // one provider and one model, so a pool that hedges across
            // eighty-eight lanes only ever warms a cache it then walks away
            // from. If consecutive rounds of one question keep landing on
            // different lanes, that alone explains nought per cent and no
            // amount of reordering the block will change it.
            let which = reply.lane.as_deref().unwrap_or("?");
            match (reply.prompt, reply.cached) {
                (Some(whole), Some(hit)) => eprintln!(
                    "[cache]  round {round}: {hit} of {whole} prompt tokens cached ({}%) · {which}",
                    if whole > 0 { hit * 100 / whole } else { 0 }
                ),
                (Some(whole), None) => {
                    eprintln!("[cache]  round {round}: {whole} prompt tokens, {which} said nothing about caching");
                }
                _ => eprintln!("[cache]  round {round}: {which} reported no usage at all"),
            }
        }
        called += reply.tool_calls.len();
        // Whether a whole ARTICLE or PAGE was read this round, as against a
        // search that returns titles. It decides whether the name check may
        // block, and the two cases that settled it are exact opposites:
        //
        //   battery 55, "что находится в Кладбище Пепла?" — searched, read
        //   NOTHING, and wrote Iudex Gundyr, Firelink Shrine, Судия Гундир.
        //   Those are Dark Souls and came out of memory. Correctly held back.
        //
        //   battery 57, "что находится в Айнсел-Ривер?" — READ the article and
        //   wrote "Руины Дворца Уль", "Озеро Гнили", "Лунный Алтарь". Those are
        //   real ELDEN RING places; the article is in English and the answer is
        //   in Russian, so the names are TRANSLATIONS and appear nowhere in the
        //   tool output. Held back wrongly, and the rewrite came back empty —
        //   the player got silence instead of a good answer.
        //
        // A check that costs a correct answer is worse than one that misses, so
        // once prose has been read the name check stops blocking. Everything it
        // has ever caught for real — the invented talisman, Somerset Stone, the
        // Bull-Goat set, the Dark Souls names — was written WITHOUT reading an
        // article, which is exactly when it keeps working.
        // A SEARCH counts as prose too, not just a read.
        //
        // Its results are the titles of real articles, and the model answers in
        // the player's language, so what comes back as "Caelid Catacombs" and
        // "Minor Erdtree" is written as "Катакомбы Каэлида" and "Малое
        // Эрдерево". The name check compares letters, so it called both
        // inventions — twice running, on "что находится в Caelid?".
        //
        // Transliterating does not rescue this: those are TRANSLATIONS, and
        // "katakomby" never meets "Catacombs". And the rewrite the check then
        // demands is impossible, because an answer about a region with every
        // place name stripped out has nothing left to say — so the model went
        // quiet and the player got a blank screen both times.
        //
        // The trade is the same one already accepted for `read_article`: after
        // prose has been handed over, names in the answer stop being checkable
        // and the check stands down rather than blocking a correct answer.
        read_prose |= reply.tool_calls.iter().any(|call| {
            matches!(
                call.function.name.as_str(),
                "read_article" | "read_page" | "search_wiki" | "search_web"
            )
        });
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

/// The tools with their reasoning cut away.
fn tools_in_brief(tools: &serde_json::Value) -> serde_json::Value {
    let Some(all) = tools.as_array() else {
        return tools.clone();
    };
    serde_json::Value::Array(
        all.iter()
            .map(|one| {
                let mut short = one.clone();
                let Some(said) = one["function"]["description"].as_str() else {
                    return short;
                };
                // The first paragraph, which is always what the tool does.
                let opening = said.split("\n\n").next().unwrap_or(said);
                short["function"]["description"] = serde_json::Value::String(opening.to_string());

                // And the same for each argument, which is where the weight
                // moved once the tool descriptions were cut: measured, 5,573
                // characters of argument prose against 4,710 describing the
                // tools themselves. The NAME, the TYPE and whether it is
                // required are untouched — a shortened argument description
                // costs an argument used clumsily, where a shortened parameter
                // block costs a call that cannot be made at all.
                if let Some(properties) =
                    short["function"]["parameters"]["properties"].as_object_mut()
                {
                    for (_, argument) in properties.iter_mut() {
                        let Some(about) = argument["description"].as_str() else {
                            continue;
                        };
                        let first = about.split("\n\n").next().unwrap_or(about);
                        argument["description"] = serde_json::Value::String(first.to_string());
                    }
                }
                short
            })
            .collect(),
    )
}

/// The conversation with what nobody needs in full any more cut down.
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
            let role = message.get("role").and_then(|r| r.as_str());
            if role != Some("tool") || whole.contains(&at) {
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

/// The always-present block with one player's circumstances taken back out,
#[allow(dead_code)]
fn rules_only(block: &str) -> (String, usize) {
    const CIRCUMSTANCE: [&str; 4] = [
        "## This player, as of this second",
        "## What is true of this machine right now",
        "## What is kept here, and where",
        "## This particular installation",
    ];

    let mut kept = String::with_capacity(block.len());
    let mut dropped = 0;
    let mut skipping = false;
    for line in block.lines() {
        if line.starts_with("## ") || line.starts_with("# ") {
            skipping = CIRCUMSTANCE.iter().any(|heading| line.trim() == *heading);
            if skipping {
                kept.push_str(line);
                kept.push('\n');
                kept.push_str(
                    "(said in full on the first round of this question and unchanged since; ask \
                     player_status if you need it again)\n",
                );
                continue;
            }
        }
        if skipping {
            dropped += line.len() + 1;
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    (kept, dropped)
}

/// Whether waiting a moment and asking again is likely to work.
fn worth_retrying(error: &str) -> bool {
    // The request never arrived: a dropped connection, a refused socket, a name
    // that would not resolve for a second. A question three rounds into looking
    // things up should not die on that.
    error.contains("could not reach the answering service")
        || error.contains("the answer did not arrive")
}

/// A pool that has nothing left to give, as against a connection that dropped.
fn the_pool_is_spent(error: &str) -> bool {
    error.contains("every lane refused")
        || error.contains("no lane has any allowance left")
        || error.contains("503")
        || the_service_hit_its_own_ceiling(error)
}

/// The answering service ran out of room, as against the models being busy.
fn the_service_hit_its_own_ceiling(error: &str) -> bool {
    error.contains("1102")
}

/// How long to wait before trying again, per attempt.
const RETRY_AFTER_MS: [u64; 2] = [3_000, 9_000];

/// The lane that answered last, remembered BETWEEN questions.
static LAST_GOOD_LANE: Mutex<Option<String>> = Mutex::new(None);

fn remember_lane(lane: &Option<String>) {
    if let Some(id) = lane {
        if let Ok(mut held) = LAST_GOOD_LANE.lock() {
            *held = Some(id.clone());
        }
    }
}

fn lane_from_last_time() -> Option<String> {
    LAST_GOOD_LANE.lock().ok().and_then(|held| held.clone())
}

async fn post_chat(
    http: &reqwest::Client,
    body: &serde_json::Value,
) -> std::result::Result<Reply, String> {
    let mut last = match post_once(http, body).await {
        Ok(reply) => match reply.error.as_deref() {
            Some(error) if worth_retrying(error) => Ok(reply),
            _ => return Ok(reply),
        },
        Err(error) if !worth_retrying(&error) => return Err(error),
        Err(error) => Err(error),
    };

    for wait in RETRY_AFTER_MS {
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        match post_once(http, body).await {
            Ok(reply) => match reply.error.as_deref() {
                Some(error) if worth_retrying(error) => last = Ok(reply),
                // An answer, or a refusal that trying again will not change.
                _ => return Ok(reply),
            },
            Err(error) if !worth_retrying(&error) => return Err(error),
            Err(error) => last = Err(error),
        }
    }
    last
}

/// What to say when what came back was not an answer.
fn facts_so_far(messages: &[serde_json::Value]) -> String {
    messages
        .iter()
        .filter(|turn| turn["role"] == "tool" || turn["role"] == "system")
        .filter_map(|turn| turn["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Names in an answer that no tool put there.
fn ungrounded_names(answer: &str, facts: &str, asked: &str) -> Vec<String> {
    // A word is "known" if it, or a long enough stem of it, turns up in the
    // facts or in the question. Stemmed because Russian declines: the table
    // says "Доспех овцебыка" and an answer may say "Доспеха овцебыка", and
    // those are the same armour.
    let facts_lower = facts.to_lowercase();
    let asked_lower = asked.to_lowercase();
    let known = |word: &str| -> bool {
        let word = word.to_lowercase();
        let latin = translit(&word);
        let stem: String = word.chars().take(word.chars().count().saturating_sub(2)).collect();
        let needle = if stem.chars().count() >= 4 { stem } else { word };
        if facts_lower.contains(&needle) || asked_lower.contains(&needle) {
            return true;
        }
        // The answer can be in a different alphabet from the facts. The wikis
        // are English and the player is not, so a name the search really did
        // return comes back through the model translated, and the letters no
        // longer match.
        //
        // Transliteration fixes only the half of that which is a NAME rather
        // than a translation: "Каэлида" -> "kaelida" finds "Caelid", where
        // "Катакомбы" -> "katakomby" will never find "Catacombs" and
        // "Эрдерево" -> "erderevo" will never find "Erdtree". So this is not
        // the whole answer to the problem — see where `read_prose` is set for
        // the part that is.
        let Some(latin) = latin else {
            return false;
        };
        let head: String = latin.chars().take(4).collect();
        let tail: String = latin.chars().skip(1).take(4).collect();
        (head.chars().count() >= 4 && (facts_lower.contains(&head) || asked_lower.contains(&head)))
            || (tail.chars().count() >= 4
                && (facts_lower.contains(&tail) || asked_lower.contains(&tail)))
    };

    let capitalised = |word: &str| {
        word.chars().next().is_some_and(char::is_uppercase) && word.chars().count() > 1
    };

    // What is capitalised, appears in no tool result, and still is not an
    // invention: the game, the conversion, the launcher and its parts. These
    // come from the always-present block rather than from a tool, so by the
    // letter of the check they are ungrounded, and by any sense they are not.
    const SAFE: [&str; 12] = [
        "elden", "ring", "convergence", "seamless", "roundtable", "erdtree",
        "shadow", "steam", "windows", "dlss", "fsr", "co-op",
    ];
    // A determiner is grammar, not part of a name. German capitalises every
    // noun, so an answer beginning "Diese Namen sind…" — "these names are" —
    // reads as a two-word capitalised run and was reported as an invented item.
    // It cost a round on a German talisman question that was otherwise right.
    //
    // Only words that CANNOT open an item name go here. "Der", "Die" and "Das"
    // can ("Der Riese"), so they are absent on purpose.
    const GRAMMAR: [&str; 14] = [
        "diese", "dieser", "dieses", "jene", "jener", "welche", "welcher", "welches",
        "these", "those", "which", "эти", "этот", "эта",
    ];
    // A short word in capitals is an abbreviation, not a name. STR, DEX, INT,
    // FTH, ARC, FP, HP — the stat shorthand every answer uses, and none of it
    // comes back from a tool because the tables say "strength" and the answer
    // says "Силы (STR)".
    //
    // This is not hypothetical tidying: it is the only thing the check has
    // ever got wrong. Battery 47 held back two perfectly good answers over
    // ["Силы STR"] and ["SIŁY STR"] — the Russian and Polish for strength,
    // capitalised mid-sentence, with the abbreviation after them. Each cost a
    // round, and rounds are the scarce thing (see the per-minute ceiling).
    // A false positive teaches everybody to switch the check off.
    let shorthand = |word: &str| {
        word.chars().count() <= 4
            && word.chars().any(char::is_alphabetic)
            && word.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase)
    };
    let safe = |word: &str| {
        if shorthand(word) {
            return true;
        }
        let word = word.to_lowercase();
        SAFE.iter().any(|allowed| word == *allowed)
    };

    // The first word of a sentence is capitalised because it is the first word
    // of a sentence. It still belongs to the run — "Талисман Путника открывает"
    // is a name and has to be reported whole — but it cannot be the evidence
    // that the run is invented. Without this, "Надень Доспеха Овцебыка" was
    // flagged over the imperative verb while both halves of the armour's real
    // name sat right there in the tool output.
    let mut found: Vec<String> = Vec::new();
    // A run of two or more capital-initial words, which is what an item name
    // looks like when a model writes one out. One capital on its own is a
    // sentence start or, in German, every noun there is — flagging those would
    // drown the real thing.
    // Each entry is a word and whether it opened a sentence.
    let mut run: Vec<(&str, bool)> = Vec::new();
    let finish = |run: &mut Vec<(&str, bool)>, found: &mut Vec<String>| {
        // EVERY word has to be accounted for, not merely one of them. `any` was
        // tried first and let the worst case straight through: "Талисманом
        // Путника (Traveler's Talisman)" contains the word "Talisman", the
        // facts contained "Talismans: ...", and one true word licensed three
        // invented ones. A name that really came from a tool has ALL of its
        // words in the tool's output — that is what being from there means.
        let accounted =
            run.iter().all(|(word, opened)| *opened || known(word) || safe(word));
        // A run that OPENS with a determiner is a sentence, not a name. German
        // capitalises its nouns, so "Diese Namen sind…" — "these names are" —
        // is an ordinary clause wearing the shape this looks for, and it was
        // reported as an invented item on an otherwise correct answer.
        //
        // Checked on the first word only: "Der Riese" can name something, and
        // a determiner in the MIDDLE of a name is part of it.
        let opens_with_grammar = run
            .first()
            .is_some_and(|(word, _)| GRAMMAR.contains(&word.to_lowercase().as_str()));
        if run.len() >= 2 && !accounted && !opens_with_grammar {
            let name = run.iter().map(|(word, _)| *word).collect::<Vec<_>>().join(" ");
            if !found.contains(&name) {
                found.push(name);
            }
        }
        run.clear();
    };

    // Line by line, because a NEWLINE is a sentence boundary and this could not
    // see one. `opening` was set from a token ending in '\n', and
    // `split_whitespace` has already eaten the newline — so the first word of
    // every line looked like the middle of the previous sentence.
    //
    // Two false positives came of it in one battery: ["Штормовой Кнут Если"]
    // and ["Поскольку Каменное"], where "Если" and "Поскольку" are the
    // conjunctions "if" and "since" beginning a new line and being glued to the
    // word beside them. A check that holds back a correct answer is worse than
    // one that misses, so the boundary has to be real.
    for line in answer.lines() {
        let mut opening = true;
        for raw in line.split_whitespace() {
        // Punctuation is not part of a name, and a full stop ENDS a run: the
        // last word of one sentence and the first of the next are not a name
        // together, and that pairing is the obvious false positive.
        let ends = raw.ends_with(['.', '!', '?', ':', ';', ',', ')', '»', '"']);
        // An aside in brackets is its own thing and does not belong to the
        // words before it. "требует 20 Силы (STR)" is a stat and its
        // abbreviation, not an item called "Силы STR" — and reading it as one
        // held back two good answers in battery 47, in Russian and in Polish.
        // Meanwhile "Талисманом Путника (Traveler's Talisman)" still breaks
        // into two runs and the first of them, both words unknown, is still
        // caught. The bracket is the boundary in both.
        if raw.starts_with(['(', '«', '"', '[']) {
            finish(&mut run, &mut found);
        }
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'');
        if word.is_empty() {
            finish(&mut run, &mut found);
            continue;
        }
        if capitalised(word) {
            run.push((word, opening));
            if ends {
                finish(&mut run, &mut found);
            }
        } else {
            finish(&mut run, &mut found);
        }
            // Only a full stop and its kin start a new sentence; a comma does
            // not. A newline is handled by the loop above.
            opening = raw.ends_with(['.', '!', '?', ':']);
        }
        // A line ending closes whatever was open: a name does not run across
        // one, and treating it as though it did is what glued a conjunction on.
        finish(&mut run, &mut found);
    }
    found
}

fn not_an_answer(status: reqwest::StatusCode, body: &str) -> String {
    let snippet: String = body.trim().chars().take(160).collect();
    if snippet.is_empty() {
        format!("the answering service returned nothing ({status})")
    } else if snippet.starts_with('<') {
        format!("the answering service sent a page rather than an answer ({status})")
    } else {
        // Anything that is actually words is worth quoting — `error code: 1010`
        // is how a banned client signature announces itself, and hiding it
        // would have cost an afternoon.
        format!("the answering service returned {status}: {snippet}")
    }
}

/// An error with what actually caused it, not just its polite top layer.
fn all_the_way_down(error: &(dyn std::error::Error + 'static)) -> String {
    let mut said = error.to_string();
    let mut cause = error.source();
    while let Some(deeper) = cause {
        let next = deeper.to_string();
        // Chains repeat themselves; only add a link that says something new.
        if !said.contains(&next) {
            said.push_str(": ");
            said.push_str(&next);
        }
        cause = deeper.source();
    }
    said
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
        .map_err(|error| {
            format!(
                "could not reach the answering service: {}",
                all_the_way_down(&error)
            )
        })?;

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

    serde_json::from_str::<Reply>(&body).map_err(|_| not_an_answer(status, &body))
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
    let mut messages = trimmed(messages);
    messages.push(answer_in_their_language(question));

    let body = serde_json::json!({
        "messages": messages,
        "edition": edition,
        "stream": true,
    });

    // The connection itself is worth a second go. A question that has already
    // spent three rounds looking things up should not die because one socket
    // was refused, and the player has no way to tell that from a real failure.
    let mut sent = http.post(SERVICE).json(&body).send().await;
    for wait in RETRY_AFTER_MS {
        if sent.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        sent = http.post(SERVICE).json(&body).send().await;
    }
    let reply = match sent {
        Ok(reply) => reply,
        Err(error) => {
            emit(Event::Failed {
                error: in_plain_words(&format!("could not reach the answering service: {error}")),
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
                    emit(Event::Done { lane: body.lane, ms: body.ms, cut: false });
                }
                None => emit(Event::Failed {
                    error: in_plain_words(
                        body.error.as_deref().unwrap_or("no answer came back"),
                    ),
                }),
            },
            Err(error) => emit(Event::Failed {
                error: format!("the answering service said something odd: {error}"),
            }),
        }
        return;
    }

    if read_events(reply, emit).await == Outcome::Done {
        return;
    }
    // Nothing reached the player, so the whole thing can be asked again — ONCE.
    // A stream that says nothing is usually a lane that leaked a tool call, and
    // a second ask fixes that. It is sometimes the whole pool being spent, and
    // there a second ask is one more request it has to refuse: measured, nine
    // in a row refused over four minutes. One attempt splits the difference
    // without turning a busy minute into a busier one.
    tokio::time::sleep(std::time::Duration::from_millis(RETRY_AFTER_MS[0])).await;
    if let Ok(again) = http.post(SERVICE).json(&body).send().await {
        if read_events(again, emit).await == Outcome::Done {
            return;
        }
    }
    emit(Event::Failed {
        error: "every model is busy right now. Ask again in a minute.".into(),
    });
}

/// How a finished stream ended: silence is not success.
///
/// `done` arriving with nothing said is the shape that reached a player — the
/// name check caught three invented places in an answer about Caelid, asked for
/// the answer again, and the rewrite came back empty. It is worth retrying
/// rather than reporting, because nothing retries a success.
fn ended_well(said: bool) -> Outcome {
    if said { Outcome::Done } else { Outcome::WorthRetrying }
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
        cut: Option<bool>,
        error: Option<String>,
    }

    let mut stream = reply.bytes_stream();
    let mut buffer = String::new();
    let mut lane: Option<String> = None;
    let mut said = false;
    let mut ended = false;
    // The opening of the answer, held back until there is enough of it to tell
    // prose from a leaked tool call. See `leaked_a_tool_call`.
    let mut opening = String::new();
    let mut opening_held = true;

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
                emit(Event::Failed { error: in_plain_words(&error) });
                return Outcome::Done;
            }
            if chunk.lane.is_some() {
                lane = chunk.lane.clone();
            }
            if let Some(text) = chunk.delta {
                if !text.is_empty() {
                    if opening_held {
                        opening.push_str(&text);
                        if leaked_a_tool_call(&opening) {
                            // Nothing has reached the player yet, so this can
                            // be thrown away and asked again.
                            return Outcome::WorthRetrying;
                        }
                        if opening.chars().count() < OPENING_HELD {
                            continue;
                        }
                        opening_held = false;
                        said = true;
                        emit(Event::Delta { text: std::mem::take(&mut opening) });
                    } else {
                        said = true;
                        emit(Event::Delta { text });
                    }
                }
            }
            if chunk.done == Some(true) {
                if opening_held && !opening.is_empty() {
                    said = true;
                    emit(Event::Delta { text: std::mem::take(&mut opening) });
                }
                // A clean `done` that carried no words is not an answer. It
                // reached a player once: the name check caught three invented
                // places in an answer about Caelid, asked for it again, and the
                // rewrite came back empty — so the question that had just cost
                // three rounds displayed nothing at all. Ending "successfully"
                // with silence is the one outcome worse than an error, because
                // nothing retries it and nothing explains it.
                if ended_well(said) == Outcome::WorthRetrying {
                    return Outcome::WorthRetrying;
                }
                emit(Event::Done {
                    lane: lane.clone(),
                    ms: chunk.ms,
                    cut: chunk.cut.unwrap_or(false),
                });
                return Outcome::Done;
            }
        }
        if ended {
            break;
        }
    }

    // A stream that ended before the opening was long enough to judge: it never
    // tripped the guard, so it is an answer and it goes out.
    if opening_held && !opening.is_empty() {
        emit(Event::Delta { text: opening });
        emit(Event::Done { lane, ms: None, cut: false });
        return Outcome::Done;
    }
    if said {
        // The stream stopped without ever saying it was done. Something ended
        // it early — which is the whole of what `cut` claims.
        emit(Event::Done { lane, ms: None, cut: true });
        Outcome::Done
    } else {
        Outcome::WorthRetrying
    }
}

/// Whether an article is about something that gets fought.
fn mentions_a_fight(text: &str) -> bool {
    let said = text.to_lowercase();
    ["weak to", "weakness", "resistan", "immune", "boss", "phase", "drops"]
        .iter()
        .filter(|word| said.contains(**word))
        .count()
        >= 2
}

/// Where to go next, when a name search has landed on something with figures.
fn onward_from<'a>(kinds: impl Iterator<Item = &'a str>) -> String {
    let mut weapon = false;
    let mut armour = false;
    let mut creature = false;
    for kind in kinds {
        weapon |= kind == "weapon";
        armour |= kind == "armour";
        creature |= kind == "character";
    }
    let mut out = String::new();
    if creature {
        out.push_str(
            "\nFor a CREATURE OR CHARACTER above, call whos_here with its name — not with a map \
             id, the name. That gives where it stands, what it is worth, and WHAT KIND OF \
             DAMAGE HURTS IT, out of their own tables. Asked how to kill one of these, a model \
             skipped that, read a wiki, and told the player it was weak to holy; this \
             installation's own table has it taking 80% of holy, which makes holy one of the \
             worst things to bring. The wiki is the base game and the mod rewrote it.\n",
        );
    }
    if weapon {
        out.push_str(
            "\nFor a WEAPON above, the figures are not here and are not the wiki's: call \
             gear_numbers with its name exactly as printed. That is the only place its damage, \
             its scaling, its requirements, and the SKILL on it — the ash of war, and what a \
             press of it costs in FP — can be read for this installation.\n",
        );
    }
    if armour {
        out.push_str(
            "\nFor a PIECE OF ARMOUR above, call gear_numbers with its name for the weight and \
             the damage negation. Name the set rather than a piece and it adds the set up.\n",
        );
    }
    out
}

/// Where the time went, for anybody trying to find out.
pub fn note_timing(what: &str, since: std::time::Instant) {
    if std::env::var_os("ROUNDTABLE_TIMING").is_some() {
        eprintln!("[timing] {:>6} ms  {what}", since.elapsed().as_millis());
    }
}

/// The always-present block split into the sections it is written in, each
fn setup_by_section(block: &str) -> Vec<(usize, String)> {
    let mut sections: Vec<(usize, String)> = Vec::new();
    let mut name = String::from("(before the first heading)");
    let mut size = 0usize;
    for line in block.lines() {
        if line.starts_with("## ") || line.starts_with("# ") {
            if size > 0 {
                sections.push((size, std::mem::take(&mut name)));
            }
            name = line.trim_start_matches('#').trim().to_string();
            size = line.len() + 1;
        } else {
            size += line.len() + 1;
        }
    }
    if size > 0 {
        sections.push((size, name));
    }
    sections.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
    sections
}

/// How big a round is, broken down by what is taking the room.
fn note_size(round: usize, body: &serde_json::Value) {
    if std::env::var_os("ROUNDTABLE_TIMING").is_none() {
        return;
    }
    let whole = body.to_string().len();
    let mut by_role: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        for message in messages {
            let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("?");
            let size = message.get("content").and_then(|c| c.as_str()).unwrap_or("").len();
            *by_role.entry(role).or_default() += size;
        }
    }
    let tools = body.get("tools").map(|t| t.to_string().len()).unwrap_or(0);
    let parts: Vec<String> =
        by_role.iter().map(|(role, size)| format!("{role} {size}")).collect();
    eprintln!("[size]  round {round}: {whole} total · {} · tools {tools}", parts.join(" · "));
}

/// The question again, so the reply comes back in the language it was asked in.
fn answer_in_their_language(question: &str) -> serde_json::Value {
    serde_json::json!({
        "role": "system",
        // The names are the exception, and it has to be said here because this
        // is where the language is decided. Asked in Dutch, a model wrote the
        // player's weapon as "Pechat' voennogo khirurga" — their game prints
        // "Печать военного хирурга", and no menu, no wiki and no search box
        // anywhere contains the Latin version. Translating a name was already
        // forbidden; spelling it out in another alphabet is the same harm and
        // was not.
        "content": format!(
            "{}Write your reply in the same language the player used — except for the names of \
             things in the game, which are copied EXACTLY as this launcher gives them, letter \
             for letter and alphabet for alphabet. Never translate one and never transliterate \
             one, however foreign it looks beside the rest of your sentence: it is what is \
             printed on their screen and what they will type into a search. For reference, this \
             is what they asked, which you must answer rather than repeat:\n\n{question}",
            // Named outright where it can be, because "the same language they
            // used" loses to two kilobytes of English tool output on a weaker
            // lane: a Russian question about armour came back wholly in
            // English, after a tool whose every word was English. An
            // instruction that names the language does not depend on the model
            // still remembering what the question looked like.
            in_what_language(question)
        ),
    })
}

/// The language a question is written in, named, when its script says so.
fn in_what_language(question: &str) -> String {
    let named = question.chars().find_map(|ch| match ch {
        'а'..='я' | 'А'..='Я' | 'ё' | 'Ё' => Some("Russian"),
        'α'..='ω' | 'Α'..='Ω' => Some("Greek"),
        '\u{0590}'..='\u{05ff}' => Some("Hebrew"),
        '\u{0600}'..='\u{06ff}' => Some("Arabic"),
        '\u{3040}'..='\u{30ff}' => Some("Japanese"),
        '\u{ac00}'..='\u{d7af}' => Some("Korean"),
        '\u{4e00}'..='\u{9fff}' => Some("Chinese"),
        _ => None,
    });
    match named {
        Some(language) => format!(
            "THE PLAYER WROTE IN {}. Your whole reply is in {} — every sentence of it, however \
             much English you have just read in a tool's output. ",
            language.to_uppercase(),
            language
        ),
        None => String::new(),
    }
}

/// The answering service's own words, turned into the player's.
fn in_plain_words(error: &str) -> String {
    let said = error.to_ascii_lowercase();
    if said.contains("every lane refused") || said.contains("no lane has any allowance left") {
        return "Every model the launcher can reach is busy or over its limit for the moment. \
                Nothing is wrong with the question — give it a minute and ask again."
            .into();
    }
    if said.contains("no answer came back") {
        return "The answering service replied with nothing at all. Ask again; if it keeps \
                happening, it is the service rather than this machine."
            .into();
    }
    if said.contains("could not reach the answering service")
        || said.contains("the answer did not arrive")
    {
        return "The launcher could not reach the answering service — it has already tried \
                again. That is the connection or the service, not the question. Check the \
                network if it keeps happening; otherwise ask again in a moment."
            .into();
    }
    // Before the general case, because it is the one that used to be described
    // as something it is not. See `the_service_hit_its_own_ceiling`.
    if the_service_hit_its_own_ceiling(&said) {
        return "The answering service ran out of room handling that one — it is the service \
                itself, not any of the models behind it and not this machine. Asking again \
                straight away makes it worse. Give it a minute; a shorter question, or a new \
                conversation instead of a long one, usually goes through."
            .into();
    }
    // An error PAGE with a 5xx, before the pool case, because the pool case
    // claims something that did not happen. Measured 12 Aug: across three
    // batteries every single failure was this, and the log recorded ZERO
    // `[lanes]` lines — that line is printed whenever the worker answers with
    // its per-lane breakdown, which is what a genuinely exhausted pool looks
    // like. So no model was ever asked, and the player was being told "the
    // service tried each of them" about requests that never reached one. It
    // sent the maintainer after the pool for two sessions as well.
    //
    // This does NOT make it retryable: whether the worker fell over briefly or
    // hit a wall the same request would hit again cannot be told apart from
    // here, and there is a measurement against retrying. It only stops the
    // launcher naming the wrong culprit.
    if said.contains("sent a page rather than an answer (5") {
        return "The answering service did not respond — the launcher got an error page back \
                instead of an answer. That is the service itself: no model was asked, so it is \
                not the models being busy and it is nothing to do with the question. Ask again \
                in a moment."
            .into();
    }
    if the_pool_is_spent(&said) {
        return "Every model the launcher can reach is busy — the service tried each of them \
                and ran out of time doing it. That is not the question and not this machine, \
                and asking again straight away makes it worse. Give it a minute or two."
            .into();
    }
    error.to_string()
}

/// How much of the opening is held back before any of it is shown.
const OPENING_HELD: usize = 100;

/// Whether what is arriving is the model writing a tool call out as prose.
fn reasoned_past_its_own_doubt(question: &str, answer: &str) -> bool {
    let asked = question.to_lowercase();
    const WHY: [&str; 8] =
        ["почему", "зачем", "why ", "why?", "por que", "porque ", "warum", "pourquoi"];
    if !WHY.iter().any(|word| asked.starts_with(word) || asked.contains(word)) {
        return false;
    }

    let said = answer.to_lowercase();
    const BECAUSE: [&str; 8] = [
        "потому что",
        "поэтому это",
        "because it",
        "because the",
        "porque é",
        "porque o",
        "weil es",
        "parce que",
    ];
    const NOT_FOUND: [&str; 10] = [
        "не нашёл",
        "не нашел",
        "не удалось найти",
        "не объясня",
        "не подтвержда",
        "could not find",
        "did not find",
        "no encontré",
        "não encontrei",
        "nicht gefunden",
    ];
    BECAUSE.iter().any(|word| said.contains(word))
        && NOT_FOUND.iter().any(|word| said.contains(word))
}

/// A figure supplied for something the answer has just called unreadable.
/// A tab of the launcher that the launcher does not have.
///
/// The block already lists them — "The tabs down the side: Play, Mods, Saves,
/// Co-op, Codex, Wiki, System" — and an answer still sent a player to a tab
/// called "DLSS и генерация кадров", which does not exist. DLSS lives under
/// System. That is the worst kind of invention about the launcher, because the
/// player goes looking and the thing is not there.
///
/// `ungrounded_names` CANNOT catch this. It looks for runs of two or more
/// capitalised words, which is what an item name looks like in English and in
/// Russian item text — but a Russian UI label is capitalised once and then
/// lower case ("генерация кадров"), so the invented tab is not a run at all.
/// Verified by splitting that exact sentence: zero runs of two.
///
/// The tabs are a closed set of seven, so this does not need to guess. It looks
/// for the word for "tab" followed by a quoted name, and checks that name
/// against the seven. Only quoted names: an answer that says "on the System tab"
/// without quotes is already right, and prose about tabs in general is not a
/// claim about a specific one.
fn names_a_tab_that_is_not_there(answer: &str) -> Option<String> {
    const TABS: [&str; 7] = ["play", "mods", "saves", "co-op", "codex", "wiki", "system"];
    // The word for "tab" in the languages this has been asked in, plus English.
    const TAB_WORD: [&str; 7] =
        ["вкладк", "tab", "zakładk", "zakladk", "reiter", "onglet", "pestaña"];

    let said = answer.to_lowercase();
    if !TAB_WORD.iter().any(|word| said.contains(word)) {
        return None;
    }

    // Every quoted run in the answer, in the quote marks these languages use.
    let mut quoted: Vec<String> = Vec::new();
    let mut held: Option<String> = None;
    for character in answer.chars() {
        match character {
            '"' | '«' | '»' | '“' | '”' => match held.take() {
                Some(name) => quoted.push(name),
                None => held = Some(String::new()),
            },
            other => {
                if let Some(name) = held.as_mut() {
                    name.push(other);
                }
            }
        }
    }

    quoted.into_iter().find(|name| {
        let name = name.trim().to_lowercase();
        // Short quoted things are buttons and settings, not tab names, and the
        // tabs themselves are all one word.
        name.chars().count() >= 3
            && name.split_whitespace().count() <= 5
            && !TABS.iter().any(|tab| name.contains(tab))
    })
}

fn figure_it_said_it_could_not_read(question: &str, answer: &str, facts: &str) -> bool {
    let asked = question.to_lowercase();
    const HOW_MUCH: [&str; 9] = [
        "сколько", "how much", "how many", "cuánto", "cuanto", "quanto", "wie viel",
        "combien", "ile ",
    ];
    if !HOW_MUCH.iter().any(|word| asked.contains(word)) {
        return false;
    }

    let said = answer.to_lowercase();
    // Only an admission about READING it. "There is no such weapon" is a real
    // answer about a thing that does not exist and must pass untouched; this is
    // about a thing that exists whose number was not obtainable.
    const UNREADABLE: [&str; 12] = [
        "не записано",
        "не могу прочитать",
        "не может его прочитать",
        "не удалось прочитать",
        "нельзя прочитать",
        "не установлено",
        "не указано",
        "not recorded",
        "cannot be read",
        "could not be read",
        "no está registrado",
        "não está registrado",
    ];
    if !UNREADABLE.iter().any(|word| said.contains(word)) {
        return false;
    }

    // Separators come off BOTH sides before comparing, so "15 000" in the answer
    // matches "15000" in a tool result. Stripping them from the facts as well
    // can only ever join two numbers into one and make a match where there was
    // none — that direction fails to flag, which is the safe way round for a
    // check whose false positives cost a round.
    let bare = |text: &str| -> String {
        text.chars().filter(|c| !matches!(c, ' ' | ',' | '\u{a0}' | '\u{202f}' | '\'')).collect()
    };
    let grounded = bare(facts);

    let mut digits = String::new();
    let mut numbers: Vec<String> = Vec::new();
    for character in bare(&said).chars().chain(std::iter::once(' ')) {
        if character.is_ascii_digit() || (character == '.' && !digits.is_empty()) {
            digits.push(character);
        } else {
            if !digits.is_empty() {
                // A full stop that ends the sentence gets swallowed by the
                // branch above, and "15 000." then matches nothing in facts
                // holding "15000" — the check flagged a correctly quoted
                // figure. The decimal point survives because a digit follows
                // it; only a trailing one is punctuation.
                let number = digits.trim_end_matches('.').to_string();
                if !number.is_empty() {
                    numbers.push(number);
                }
                digits.clear();
            }
        }
    }

    numbers.iter().any(|number| {
        let whole = number.split('.').next().unwrap_or(number);
        whole.len() >= 3 && !grounded.contains(number.as_str())
    })
}

fn pushed_the_question_away(answer: &str) -> bool {
    let said = answer.to_lowercase();
    const AWAY: [&str; 12] = [
        "не про игру",
        "не ко мне",
        "к разработчикам",
        "обратись к разработ",
        "это вне моей",
        "я не занимаюсь",
        "not about the game",
        "not something i",
        "contact the develop",
        "ask the develop",
        "outside my",
        "no es sobre el juego",
    ];
    AWAY.iter().any(|phrase| said.contains(phrase))
}

/// A tool's name, said out loud to the player.
fn names_a_tool(answer: &str) -> Option<String> {
    let said = answer.to_lowercase();
    tool_schemas()
        .as_array()?
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .find(|name| said.contains(&name.to_lowercase()))
        .map(str::to_string)
}

fn leaked_a_tool_call(opening: &str) -> bool {
    const MARKERS: [&str; 6] = [
        "<tool_call>",
        "<invoke name=",
        "<function=",
        "<|tool",
        "]<]minimax[>[",
        "<function_calls>",
    ];
    let head: String = opening.chars().take(OPENING_HELD * 2).collect();
    MARKERS.iter().any(|marker| head.contains(marker))
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
        Event::Done { lane: which, ms: took, cut: _ } => {
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

/// What one word is worth to the ranking, for working out why a search ranked
pub fn word_weight(app_data: &Path, word: &str) -> f32 {
    wikis(None)
        .first()
        .map_or(0.0, |source| index_for(app_data, source).weight(word))
}

/// The names in a query that no wiki has, each with what is spelt nearly like
pub fn unknown_words(
    app_data: &Path,
    edition: Option<&str>,
    query: &str,
) -> Vec<(String, Vec<String>)> {
    let sources = wikis(edition);
    let missing = sources
        .iter()
        .map(|source| index_for(app_data, source).unmatched(query))
        .fold(None::<Vec<String>>, |all, here| {
            Some(match all {
                None => here,
                Some(before) => before.into_iter().filter(|w| here.contains(w)).collect(),
            })
        })
        .unwrap_or_default();

    missing
        .into_iter()
        .map(|word| {
            let close = sources
                .iter()
                .flat_map(|source| index_for(app_data, source).nearest(&word, 3))
                .collect();
            (word, close)
        })
        .collect()
}

/// Which articles a question matches, for anything that wants a list without
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

    /// A tab the launcher does not have is caught; the seven real ones are not.
    #[test]
    fn an_invented_tab_is_caught() {
        // Live, from battery 67. DLSS is under System, and this sent the player
        // hunting for a tab that does not exist.
        assert_eq!(
            names_a_tab_that_is_not_there(
                "DLSS настраивается на вкладке \"DLSS и генерация кадров\" в лаунчере."
            )
            .as_deref(),
            Some("DLSS и генерация кадров"),
            "no such tab"
        );

        // A real tab, quoted, in either language.
        assert_eq!(
            names_a_tab_that_is_not_there("Откройте вкладку \"System\" и включите DLSS."),
            None
        );
        assert_eq!(names_a_tab_that_is_not_there("Open the \"Saves\" tab."), None);

        // No mention of a tab at all, and quoted things that are buttons.
        assert_eq!(names_a_tab_that_is_not_there("Нажмите \"Snapshot\"."), None);
        assert_eq!(
            names_a_tab_that_is_not_there("Кинжал \"Редувия\" наносит 82 огня."),
            None,
            "an item in quotes is not a tab claim"
        );
    }

    /// A weapon-type ash search must not read as "no ashes for that weapon".
    #[tokio::test]
    async fn a_weapon_type_is_not_a_missing_ash() {
        let player = Player {
            ashes: Box::new(|| {
                vec![
                    WarAsh { name: "Lion's Claw".into(), costs: vec![("R2".into(), 15)] },
                    WarAsh { name: "Bloody Slash".into(), costs: vec![("R2".into(), 12)] },
                ]
            }),
            ..Player::default()
        };
        let named = |wanted: &str| ToolCall {
            id: "1".into(),
            function: ToolFunction {
                name: "ashes_of_war".into(),
                arguments: format!("{{\"name\":\"{wanted}\"}}"),
            },
        };
        let http = reqwest::Client::new();

        let miss = run_tool(&http, Path::new("."), None, &player, &named("whip")).await;
        let said = miss.output.to_lowercase();
        assert!(said.contains("do not say"), "must forbid the 'no ashes' claim: {said}");
        assert!(said.contains("ash names only"), "must say it searches by name: {said}");
        assert!(said.contains("kind of weapon"), "must name the weapon-type case: {said}");

        // A real ash name still lists that ash.
        let hit = run_tool(&http, Path::new("."), None, &player, &named("lion")).await;
        assert!(hit.output.contains("Lion's Claw"), "a named match still lists: {}", hit.output);
    }

    /// "At least X int" must not be answered with the opens-up list (the opposite).
    #[tokio::test]
    async fn at_least_a_stat_is_not_the_opens_up_list() {
        let player = Player {
            spells_at: Box::new(|int, _faith, _arcane| {
                let mk = |name: &str, need: u8| Cast {
                    name: name.into(),
                    spell: crate::formats::regulation::Spell {
                        id: 0,
                        fp: 20,
                        fp_held: None,
                        stamina: 0,
                        slots: 1,
                        needs: vec![("intelligence (INT)".into(), need)],
                    },
                    modded: false,
                };
                [("Pebble", 40u8), ("Comet", 60), ("Dark Moon", 70)]
                    .into_iter()
                    .map(|(name, need)| mk(name, need))
                    .filter(|cast| cast.spell.needs.iter().all(|(_, value)| *value <= int))
                    .collect()
            }),
            ..Player::default()
        };
        let call = ToolCall {
            id: "1".into(),
            function: ToolFunction {
                name: "spell_numbers".into(),
                arguments: "{\"intelligence\":60}".into(),
            },
        };
        let http = reqwest::Client::new();
        let ran = run_tool(&http, Path::new("."), None, &player, &call).await;

        // Must carry both the 60 and the 70 spell; the 70 is what got dropped.
        assert!(ran.output.contains("or greater"), "must give the at-least list: {}", ran.output);
        assert!(ran.output.contains("Dark Moon"), "a 70-INT spell must show for >=60: {}", ran.output);
        assert!(ran.output.contains("Comet"), "the 60-INT spell shows too: {}", ran.output);
        // And it must forbid narrowing "at least" to "exactly".
        assert!(
            ran.output.to_lowercase().contains("exactly"),
            "must warn against the exact-only read: {}",
            ran.output
        );
    }

    /// A determiner is grammar, not the first word of an invented name.
    #[test]
    fn a_german_determiner_is_not_an_item() {
        // Live, from a German talisman answer that was otherwise correct:
        // "Diese Namen" is "these names", and it cost the answer a round.
        assert_eq!(
            ungrounded_names("Diese Namen sind korrekt.", "Radagon's Scarseal", ""),
            Vec::<String>::new(),
            "a determiner cannot open an item name"
        );

        // "Der" can open one, so it is not on the list and this still fires.
        assert!(
            !ungrounded_names("Der Riese Wolfgang wartet.", "Radagon's Scarseal", "").is_empty(),
            "Der Riese is a name shape and must still be checked"
        );
    }

    /// A name written in the other alphabet is matched, and an invention is not.
    #[test]
    fn a_name_in_another_alphabet_is_still_grounded() {
        let facts = "Caelid Catacombs, Minor Erdtree, Sellia Hideaway, Redmane Castle";

        // Transliteration covers the part that is a NAME spelled out rather
        // than translated: "Селлия Каэлида" is "Sellia" and "Caelid" wearing
        // Cyrillic, letter for letter.
        assert_eq!(
            ungrounded_names("Селлия Каэлида.", facts, ""),
            Vec::<String>::new(),
            "Селлия/Каэлида are Sellia/Caelid in another alphabet"
        );

        // It does NOT cover translation. "Катакомбы Гнилых Земель" is a real
        // place written in Russian, and every word of it is a translation, so
        // the letters reach nothing and the check calls it invented. That is
        // the case `read_prose` exists to stand down for after a search.
        assert!(
            !ungrounded_names("Катакомбы Гнилых Земель.", facts, "").is_empty(),
            "translated words cannot be matched by letters; read_prose covers it"
        );

        // And the check still has teeth: a place from a different game is
        // caught in any alphabet.
        assert!(
            !ungrounded_names("Храм Огня и Судия Гундир.", facts, "").is_empty(),
            "Dark Souls names are in these facts in no alphabet"
        );
    }

    /// A finished stream that said nothing is retried, not reported as done.
    #[test]
    fn silence_is_not_a_finished_answer() {
        // The Caelid answer: three invented names caught, a rewrite asked for,
        // and the rewrite arrived empty. `done` was still true, so the player
        // got a blank where three rounds of looking had just happened.
        assert_eq!(ended_well(false), Outcome::WorthRetrying);
        assert_eq!(ended_well(true), Outcome::Done);
    }

    /// The step from "what is this" to "what are its numbers".
    #[test]
    fn a_name_search_says_where_the_figures_are() {
        let said = onward_from(["weapon", "item"].into_iter());
        assert!(said.contains("gear_numbers"), "{said}");
        assert!(said.contains("ash of war"), "the ash is what sent it to a wiki: {said}");

        // A creature is the same shape of miss and cost more: asked how to kill
        // one, a model read a wiki and called it weak to holy where this
        // installation's own table has it shrugging holy off.
        let living = onward_from(["character"].into_iter());
        assert!(living.contains("whos_here"), "{living}");
        assert!(!living.contains("gear_numbers"), "a creature was sent to the weapon tool");

        // And a wiki page about a fight carries the same warning, because that
        // is where the base game's weakness table gets believed. A page about
        // a merchant does not, or nobody reads it on the page that matters.
        assert!(mentions_a_fight(
            "The Death Rite Bird is a boss. It is weak to Holy damage and resistant to Fire."
        ));
        assert!(mentions_a_fight("In the second phase it drops to the ground; it is immune to rot."));
        assert!(!mentions_a_fight(
            "Kale is a merchant found at the Church of Elleh who sells crafting kits."
        ));
        assert!(!mentions_a_fight("A cookbook containing recipes for throwing pots."));

        let dressed = onward_from(["armour"].into_iter());
        assert!(dressed.contains("gear_numbers"), "{dressed}");
        assert!(dressed.contains("set"), "the set total went unmentioned: {dressed}");
        assert!(!dressed.contains("ash of war"), "armour was told about ashes: {dressed}");

        // Nothing with figures, nothing said.
        assert!(onward_from(["item", "talisman", "skill", "place"].into_iter()).is_empty());
        assert!(onward_from(std::iter::empty()).is_empty());
    }

    /// The pool's own vocabulary must not reach the player.
    #[test]
    fn the_pool_s_own_words_do_not_reach_the_player() {
        for said in ["every lane refused", "no lane has any allowance left"] {
            let shown = in_plain_words(said);
            assert!(!shown.contains("lane"), "{said} came through as {shown}");
            assert!(a_way_forward(&shown), "{said} gave no way forward: {shown}");
        }

        // The edge answering instead of the service, which is what a burst of
        // questions provokes. Fed the real page, byte for byte, because the
        // leak was in the reading of it and not in the wording afterwards.
        let real = "<!DOCTYPE html>\n<!--[if lt IE 7]> <html class=\"no-js ie6 oldie\" \
                    lang=\"en-US\"> <![endif]-->\n<head>\n<title>roundtable-ask.workers.dev | \
                    503: Service unavailable</title>";
        let page = not_an_answer(reqwest::StatusCode::SERVICE_UNAVAILABLE, real);
        assert!(!page.contains('<'), "markup survived the reading: {page}");
        // And it must NOT be asked again. Measured: once the pool starts
        // refusing it refuses nine times running over four minutes, so every
        // retry is one more request it has to turn down. Retrying these is what
        // turned a handful of failures into a whole battery of them.
        assert!(!worth_retrying(&page), "a spent pool was asked again");
        assert!(the_pool_is_spent(&page), "a spent pool went unrecognised: {page}");
        let shown = in_plain_words(&page);
        assert!(!shown.contains('<'), "markup reached the player: {shown}");
        assert!(!shown.contains("503"), "a status code reached the player: {shown}");
        assert!(a_way_forward(&shown), "no way forward given: {shown}");

        // Words, though, are worth keeping: this one named the real cause.
        let banned = not_an_answer(reqwest::StatusCode::FORBIDDEN, "error code: 1010");
        assert!(banned.contains("1010"), "a plain reason was swallowed: {banned}");

        // The service over its own limit is NOT the models being busy, and
        // saying so was a real wrong answer: after a run of these the pool was
        // reported as exhausted, and it had 123,419 requests left for the day
        // with every NVIDIA lane ready. Blaming the models sends whoever reads
        // it to check accounts that were never the problem.
        let ceiling = not_an_answer(reqwest::StatusCode::SERVICE_UNAVAILABLE, "error code: 1102");
        assert!(the_service_hit_its_own_ceiling(&ceiling), "1102 went unrecognised: {ceiling}");
        assert!(!worth_retrying(&ceiling), "the service was asked again over its own ceiling");
        let shown = in_plain_words(&ceiling);
        assert!(!shown.contains("1102"), "a status code reached the player: {shown}");
        assert!(a_way_forward(&shown), "no way forward given: {shown}");
        assert!(
            !shown.to_lowercase().contains("every model"),
            "the models were blamed for the service's own limit: {shown}"
        );
        assert!(
            shown.to_lowercase().contains("service"),
            "the real cause went unnamed: {shown}"
        );
        // And the two must not have collapsed into one message.
        let refused = in_plain_words("every lane refused");
        assert_ne!(shown, refused, "two different failures say the same thing");

        // A request that never arrived. This went unretried entirely, and one
        // question in eight of a battery died on it — a dropped socket after
        // three rounds of looking things up, thrown away.
        let dropped = "could not reach the answering service: error sending request for url";
        assert!(worth_retrying(dropped), "a dropped connection was not retried");
        let shown = in_plain_words(dropped);
        assert!(!shown.contains("url"), "the raw error reached the player: {shown}");
        assert!(a_way_forward(&shown), "no way forward given: {shown}");

        // What it must NOT say, which took three batteries and a log to see.
        // The pool sentence claims "the service tried each of them", and for an
        // error page that is false: the worker never answered, so no lane was
        // ever asked. The proof is in the launcher's own diagnostics — the
        // `[lanes]` line is printed whenever the worker returns its per-lane
        // breakdown, and across every failure of three batteries there were
        // five `[failed]` lines and ZERO `[lanes]`.
        let told = in_plain_words(&page);
        assert!(
            !told.contains("tried each of them"),
            "an error page is still being reported as the model pool refusing: {told}"
        );
        assert!(
            told.contains("did not respond") || told.contains("no model was asked"),
            "an error page no longer says what actually happened: {told}"
        );

        // A 5xx error page was NEARLY made retryable, on the theory that it is
        // the edge briefly falling over rather than the pool refusing — two
        // questions in one battery died on it, both after their tool calls had
        // already succeeded, and both went unretried. The theory may even be
        // right for those two. It was dropped because the error text is the
        // SAME string the measurement above is about, so nothing here can tell
        // the two apart, and flipping a measured decision on an untestable
        // hypothesis is how the four-minute refusal storm gets rediscovered.
        // If it is ever worth revisiting, the evidence has to come from the
        // worker's own logs, not from this string.

        // And what must NOT be retried: a refusal that trying again cannot
        // change. Retrying those is how one slow question becomes three.
        assert!(!worth_retrying("context length exceeded"));
        assert!(!worth_retrying("That is a lot of questions at once."));
        // The pool saying it has nothing left, in its own words. This is the
        // case the bare `contains("503")` was there for, and it has to keep
        // working now that the bare match is gone.
        for spent in [
            "every lane refused",
            "no lane has any allowance left",
            "the answering service returned 503: every lane refused",
        ] {
            assert!(the_pool_is_spent(spent), "{spent} is no longer read as an empty pool");
            assert!(!worth_retrying(spent), "{spent} would now be retried, which adds load");
        }

        // Anything unrecognised is passed through rather than swallowed: a
        // message nobody planned for is still better read than hidden.
        let odd = "the roof fell in";
        assert_eq!(in_plain_words(odd), odd);
    }

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
    fn a_short_word_does_not_stand_for_every_word_it_begins() {
        // Measured against the real mirror: "Malenia boss fight how to beat"
        // came back "Bosses, Tools, Torch, Torches, Torrent, Torchpole". Every
        // one of those begins with "to", each counted as a word matched, and
        // six junk matches buried the one title with her name in it.
        assert!(!same_word("torch", "to"));
        assert!(!same_word("torrent", "to"));
        assert!(!same_word("gethsemane", "get"));
        // Four letters is still enough to reach a plural or a possessive.
        assert!(same_word("bosses", "boss"));
        assert!(same_word("malenias", "malenia"));
        // And a word always matches itself, however short.
        assert!(same_word("to", "to"));
        assert!(same_word("of", "of"));
    }

    #[test]
    fn a_name_outranks_a_common_word_it_loses_to_on_frequency() {
        // The case rarity gets backwards. Against the real mirror "boss" scores
        // 6.9 and "radahn" 5.5 — not because "boss" says more, but because a
        // demigod with a dozen articles looks common while a word confined to a
        // few index pages looks rare. So "Radahn boss" was answered with the
        // list of every boss in the game.
        // Sized like a wiki. In a nine-title index a demigod owns two thirds of
        // the corpus and his name is worth nothing at all — which is not the
        // situation being tested, and a rule tuned to it would be tuned to a
        // fiction. The real mirror is five thousand titles with twenty about
        // him and five about bosses in general.
        let mut titles: Vec<String> = (0..60).map(|n| format!("Some Other Page {n}")).collect();
        for title in [
            "Bosses",
            "Remembrance Bosses",
            "Wormface (boss)",
            "Starscourge Radahn",
            "Radahn Soldier Set",
            "Radahn Soldier",
            "Radahn Soldier Ashes",
            "Radahn Festival",
            "Radahn's Great Rune",
        ] {
            titles.push(title.to_string());
        }
        let all = Index::build(titles);
        assert!(
            all.weight("boss") > all.weight("radahn"),
            "the premise: frequency alone prefers the common word"
        );
        assert_eq!(
            best(&all, "Radahn boss").map(String::as_str),
            Some("Starscourge Radahn"),
            "the capital letter is what says which word was the subject"
        );
        // Written without the capital there is nothing to go on, and the old
        // answer stands — this adds a signal, it does not invent one.
        assert_eq!(best(&all, "radahn boss").map(String::as_str), Some("Bosses"));
    }

    #[test]
    fn the_words_a_query_capitalised_are_found_in_either_alphabet() {
        let found = capitalised("How do I beat Malenia");
        assert!(found.contains(&"malenia".to_string()), "{found:?}");
        // Sentence case is unavoidable and harmless: "how" is short and worth
        // almost nothing, so boosting it changes no ranking.
        assert!(found.contains(&"how".to_string()));
        assert!(!found.contains(&"beat".to_string()));

        // A name typed in Cyrillic arrives as both, so it can meet an English
        // title.
        let russian = capitalised("Как убить Малению");
        assert!(russian.iter().any(|w| w.starts_with("malen")), "{russian:?}");
    }

    #[test]
    fn a_scaling_number_carries_the_letter_the_screen_shows() {
        // The tables store hundredths; the player sees a grade. Telling them
        // "faith 65" gives them nothing to check against their own screen.
        // Pinned to a photograph of the stat screen: Reduvia under the mod
        // shows Str E, Dex C, Fth C, Arc C on scaling of 5, 50, 65 and 65. The
        // first attempt put the D-to-C line at sixty and printed Dex as a D.
        assert_eq!(grade(5.0), "E", "strength");
        assert_eq!(grade(50.0), "C", "dexterity — the one that caught it");
        assert_eq!(grade(65.0), "C", "faith and arcane");
        assert_eq!(grade(0.0), "-", "an attribute it does not scale on");
        // Ordered, and no band swallows its neighbour.
        let bands = [200.0, 150.0, 100.0, 65.0, 30.0, 10.0, 0.0];
        let letters: Vec<&str> = bands.iter().map(|v| grade(*v)).collect();
        assert_eq!(letters, ["S", "A", "B", "C", "D", "E", "-"]);
    }

    #[test]
    fn a_word_this_game_has_never_used_is_named_as_missing() {
        // The failure this exists for. Asked in Russian about the mod's Blood
        // Initiate class, the model translated it to "Blood Cleric" — a word
        // the game does not use — and the search answered with Blood,
        // Bloodboon, Bloodrose: six titles, no class, and nothing to say the
        // name was wrong. A second search then repeated the first.
        let all = index(&[
            "Blood Initiate",
            "Bloodboon",
            "Bloodrose",
            "Classes",
            "Starscourge Radahn",
        ]);
        assert_eq!(all.unmatched("Blood Cleric class"), vec!["Cleric"]);
        // A name the wiki does use is not reported.
        assert!(all.unmatched("Blood Initiate").is_empty());
        // Nor are the short words that carry no meaning either way.
        assert!(all.unmatched("what is the Blood Initiate").is_empty());
    }

    #[test]
    fn a_name_spelt_nearly_right_gets_the_near_ones_offered() {
        // Live: "Реллана" was transliterated to "Relanna", which no title
        // contains — and the ranking quietly settled on "Rennala", a different
        // boss. The answer that followed was about the wrong character.
        let all = index(&[
            "Rellana, Twin Moon Knight",
            "Rennala, Queen of the Full Moon",
            "Renna's Rise",
            "Starscourge Radahn",
            "Radagon of the Golden Order",
        ]);
        assert_eq!(all.unmatched("Where is Relanna"), vec!["Relanna"]);

        let close = all.nearest("Relanna", 3);
        assert!(close.iter().any(|t| t.contains("Rellana")), "{close:?}");

        // Rennala is offered too. Two letters is all that separates the two
        // names, and choosing between them is the model's job — which is why
        // they are handed over labelled as guesses rather than matched.
        assert!(close.len() >= 2, "{close:?}");

        // A name the wiki does use never reaches this path: it matched, so
        // there is nothing to guess at. That is what keeps Radahn from being
        // quietly answered with Radagon, which is also two letters away.
        assert!(all.unmatched("Where is Radahn").is_empty());
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

    /// The inventions this launcher actually produced, against the tool results
    #[test]
    fn a_name_no_tool_returned_is_caught_and_a_real_one_is_not() {
        // ---- must be caught ----

        // Battery 43. One call to player_status, then a named item that opens
        // a talisman slot. No tool returned it; the item does not exist.
        let invented = "В ELDEN RING: The Convergence второй слот под талисман открывается не \
                        уровнем, а специальным предметом — Талисманом Путника (Traveler's \
                        Talisman). Без него второй слот остаётся заблокирован.";
        let facts = "Playing as Way Of Life, level 34. Mods enabled: Seamless Co-op. \
                     Talismans: Кровавый амулет. Game version: 1.16";
        let caught = ungrounded_names(invented, facts, "почему у меня заблокирован второй слот?");
        assert!(
            caught.iter().any(|name| name.contains("Путника")),
            "the invented talisman should be caught, got {caught:?}"
        );

        // The upgrade material invented before any of this was written.
        let stone = "Чтобы довести оружие до +10, нужен Somerset Stone.";
        assert!(
            !ungrounded_names(stone, "Ancient Dragon Smithing Stone, Somber Smithing Stone", "как прокачать оружие")
                .is_empty(),
            "an invented upgrade material should be caught"
        );

        // ---- must NOT be caught ----

        // Battery 44, and correct: every name came back from the ranking.
        let real = "Самая тяжёлая по слотам: Голова — Шлем Тыквоголового 7.3, Тело — Доспех \
                    овцебыка 15.8, Руки — Перчатки овцебыка 5.2. Итого 38.1.";
        let armoury = "HEAD: Шлем Тыквоголового — weighs 7.3\n\
                       BODY: Доспех овцебыка — weighs 15.8\n\
                       ARMS: Перчатки овцебыка — weighs 5.2\n";
        assert_eq!(
            ungrounded_names(real, armoury, "какая броня самая тяжёлая?"),
            Vec::<String>::new(),
            "names the ranking returned must not be flagged"
        );

        // The same, declined. The tool says "Доспех" and the answer says
        // "Доспеха", which is the same armour and must not read as invented.
        let declined = "Надень Доспеха Овцебыка и Перчатки Овцебыка.";
        assert_eq!(
            ungrounded_names(declined, armoury, ""),
            Vec::<String>::new(),
            "a declined form of a name the tool returned is still that name"
        );

        // Battery 44 again: the launcher's own earlier invention, correctly
        // refused. The name is in the QUESTION, so repeating it to say the game
        // has no such thing must be allowed — otherwise the refusal cannot name
        // what it is refusing.
        let refusing = "В твоей игре такого нет: среди 210 талисманов ни одного с «traveler». \
                        В базовой вики есть Talisman of the Traveler, но не здесь.";
        assert_eq!(
            ungrounded_names(refusing, "210 talismans, none matching traveler", "где найти Талисман Путника?"),
            Vec::<String>::new(),
            "a name the player themselves typed is theirs to repeat"
        );

        // German capitalises every noun. Two of them in a row is ordinary
        // grammar there, not an item name, and this is the case most likely to
        // drown the check in noise.
        let german = "Die beste Robustheit pro Teil: Kopf und Rumpf zusammen geben 145.";
        assert_eq!(
            ungrounded_names(german, "robustness ranking: head 46, body 99", "welche rüstung hat die beste robustheit?"),
            Vec::<String>::new(),
            "German nouns are capitalised and must not read as invented names"
        );

        // A reason invented for something the answer could not confirm. The
        // positive is transcribed from battery 53; every negative is a real
        // answer from a real battery, which is the only way to know the check
        // separates them rather than merely firing.
        assert!(
            reasoned_past_its_own_doubt(
                "почему в этом моде благодати не восстанавливают фляги?",
                "В The Convergence благодати фляги не восстанавливают, потому что это \
                 осознанное изменение мода: вся система лечения переделана. Я открыл статью \
                 про Sites of Grace, но она не объясняет напрямую, почему именно отключено \
                 восстановление фляг — конкретного абзаца я там не нашёл."
            ),
            "a reason given for something it admits it could not find"
        );

        // Admitting it found nothing, WITHOUT inventing a cause. The whole
        // point of the check is that this stays untouched.
        assert!(!reasoned_past_its_own_doubt(
            "почему у меня заблокирован второй слот под талисман?",
            "Не вижу из файлов игры, сколько слотов у тебя открыто. Посмотри на экран."
        ));
        // A cause with no admission — an ordinary explanation.
        assert!(!reasoned_past_its_own_doubt(
            "почему лаунчер удалил мои моды?",
            "Лаунчер не удаляет моды сам по себе, потому что он ничего не стирает без команды. \
             У тебя установлен The Convergence и Seamless Co-op."
        ));
        // Not a why-question at all: an admission and a cause together are
        // perfectly ordinary anywhere else.
        assert!(!reasoned_past_its_own_doubt(
            "какое оружие сильнее всего накапливает кровотечение?",
            "Не нашёл талисмана с таким эффектом, потому что в таблицах их нет."
        ));

        // A figure given for something the answer has just called unreadable.
        // Transcribed from battery 63, whole, including the caveat after it —
        // the caveat is the point: the answer knows it cannot read the number.
        assert!(
            figure_it_said_it_could_not_read(
                "сколько здоровья у Радана?",
                "Его здоровье в файлах не записано числом, и ни один инструмент не может его \
                 прочитать. В базовой игре у Радана 15 000 HP, но в The Convergence эта цифра \
                 изменена.",
                "In the game · Копьё Радана"
            ),
            "a figure supplied for something it just said it could not read"
        );

        // The same admission with NO figure after it. This is the honest answer
        // the check exists to leave alone, and it must survive untouched.
        assert!(!figure_it_said_it_could_not_read(
            "сколько здоровья у Радана?",
            "Его здоровье в файлах не записано числом. Сказать точную цифру не могу.",
            "In the game · Копьё Радана"
        ));

        // The figure IS in the tool results, merely spaced differently. Reading
        // it out is the whole job and must not be punished for the space.
        assert!(!figure_it_said_it_could_not_read(
            "сколько здоровья у босса?",
            "Точное значение в файлах не записано, но таблица даёт 15 000.",
            "boss health: 15000"
        ));

        // Arithmetic, hedged, and exact — the Spanish poise answer from the same
        // battery. Nothing here was called unreadable, so the sums pass, and
        // this is the answer a hedge-word check would have destroyed.
        assert!(!figure_it_said_it_could_not_read(
            "cuanto aguante tiene esa armadura?",
            "Total aproximado: ~102.7 de poise por 26.7 de peso.",
            "Доспех ветерана 48.1 11.2 | Перчатки 10.4 3.7 | Поножи 28.6 7.0 | Шлем 15.6 4.8"
        ));

        // Under a hundred: counts and levels live here and would swamp the
        // check. The admission is present and the number is ungrounded, and it
        // still must not fire.
        assert!(!figure_it_said_it_could_not_read(
            "сколько у меня бэкапов?",
            "Точное число в файлах не записано, но у тебя 1 бэкап.",
            "backups listed"
        ));

        // Not a how-much question. An admission and a number together are
        // ordinary everywhere else.
        assert!(!figure_it_said_it_could_not_read(
            "какая коса бьёт сильнее всего?",
            "Могильная коса — 215 физического урона. Точный урон под твои статы не записано.",
            ""
        ));

        // Handing the question back, both shapes, transcribed from the answer
        // that survived a rule written into the block.
        assert!(pushed_the_question_away(
            "Я не знаю — это вопрос про лаунчер, а не про игру."
        ));
        assert!(pushed_the_question_away(
            "Почему что-то пропало — это к разработчикам Roundtable или к логам."
        ));
        // Uncertainty is NOT pushing back. These must pass untouched, or the
        // check punishes the honesty it exists to protect.
        assert!(!pushed_the_question_away(
            "Не вижу из файлов игры, сколько слотов у тебя открыто. Скажи, что на экране."
        ));
        assert!(!pushed_the_question_away(
            "В этой установке нет оружия под названием «Ордовикская глефа»."
        ));
        assert!(!pushed_the_question_away("Нажми Snapshot на вкладке Saves."));

        // The figure a list was ranked on has to appear in it.
        //
        // It did not, for months: `figure` was computed, sorted on, and never
        // printed, while every line showed `damage`, which is the sum across
        // every kind. A list built to answer "which hits hardest with holy"
        // showed the total, and the total was the only number an answer could
        // quote. Sibling of the `blocks` fault fixed in 360ced5 — the tool
        // failing to carry the thing that was asked about.
        let mixed_damage = Sorted {
            called: "greatsword".into(),
            english: "greatsword".into(),
            by: "holy".into(),
            all: 221,
            best: vec![OfSort {
                name: "Меч".into(),
                sort: "greatsword".into(),
                weight: 9.0,
                needs: Vec::new(),
                figure: 50.0,
                blocks: Vec::new(),
                boost: None,
                damage: 250,
                ailments: Vec::new(),
            }],
        };
        let listed = a_class_of(&mixed_damage);
        assert!(listed.contains("hits for 250"), "the total is still there: {listed}");
        assert!(
            listed.contains("holy 50"),
            "the figure it was RANKED on must be in the list: {listed}"
        );

        // Ranked by weight, which is already on every line — the figure must
        // not be repeated as though it were damage.
        let by_weight = Sorted { by: "weight".into(), ..mixed_damage.clone() };
        let listed = a_class_of(&by_weight);
        assert!(
            !listed.contains("of which weight"),
            "weight is already printed and must not be repeated: {listed}"
        );

        // A tool's name said to the player, twice seen live.
        assert_eq!(
            names_a_tool("Запусти игру — тогда я смогу показать список через upgrade_path."),
            Some("upgrade_path".to_string()),
            "a tool named to the player must be caught"
        );
        assert_eq!(
            names_a_tool("gear_numbers ничего не нашёл."),
            Some("gear_numbers".to_string())
        );
        // And an ordinary answer is not a tool name.
        assert_eq!(names_a_tool("Самый тяжёлый — Доспех овцебыка, 15.8."), None);
        assert_eq!(names_a_tool("Нажми Snapshot на вкладке Saves."), None);

        // The only two false positives the check has ever produced, both from
        // battery 47, both transcribed exactly. A stat abbreviation after a
        // capitalised stat name is not an item, and holding a good answer back
        // over it costs a round for nothing.
        let stats = "Медузий щит, весит 8.0, требует 20 Силы (STR) и 14 Ловкости (DEX).";
        assert_eq!(
            ungrounded_names(stats, "Медузий щит — physical 100.0, weighs 8.0", ""),
            Vec::<String>::new(),
            "STR and DEX are shorthand, not names"
        );
        let polish = "Najlepszy greatshield to Медузий щит, wymaga 20 SIŁY (STR) i 14 DEX.";
        assert_eq!(
            ungrounded_names(polish, "Медузий щит — greatshield", ""),
            Vec::<String>::new(),
            "the same in Polish"
        );

        // A NEWLINE is a sentence boundary too, and it was not being seen:
        // `split_whitespace` eats it, so the first word of every line looked
        // like the middle of the previous sentence. Both of these are real
        // answers from battery 61, held back over a conjunction — "Если" (if)
        // and "Поскольку" (since) — glued to the word beside them.
        assert_eq!(
            ungrounded_names(
                "Самый быстрый — Штормовой Кнут\nЕсли важен темп, смотри катаны.",
                "Штормовой Кнут — weighs 2.0",
                ""
            ),
            Vec::<String>::new(),
            "a conjunction opening a LINE is not part of the name above it"
        );
        assert_eq!(
            ungrounded_names(
                "Каменное скопление улучшается за руны.\nПоскольку Каменное скопление у тебя есть, начни с него.",
                "Каменное скопление — upgradable, 280 runes a level",
                ""
            ),
            Vec::<String>::new(),
            "and neither is one after a full stop at the end of a line"
        );

        // A sentence boundary is not a name. "...заблокирован. Смотри..." must
        // not pair the last word of one sentence with the first of the next.
        let across = "Слот заблокирован. Смотри, что показывает экран.";
        assert_eq!(
            ungrounded_names(across, "", ""),
            Vec::<String>::new(),
            "a full stop ends a name"
        );
    }

    /// What each tool costs to describe, as it actually goes over the wire.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn show_what_each_tool_costs() {
        let full = tool_schemas();
        let brief = tools_in_brief(&full);
        let mut rows: Vec<(usize, usize, String)> = brief
            .as_array()
            .expect("an array")
            .iter()
            .zip(full.as_array().expect("an array"))
            .filter_map(|(short, long)| {
                let name = short["function"]["name"].as_str()?.to_string();
                Some((short.to_string().len(), long.to_string().len(), name))
            })
            .collect();
        rows.sort_by_key(|(sent, _, _)| std::cmp::Reverse(*sent));

        let sent: usize = rows.iter().map(|(sent, _, _)| sent).sum();
        let whole: usize = rows.iter().map(|(_, whole, _)| whole).sum();
        println!("\n  {} tools · {sent} characters sent, {whole} before trimming\n", rows.len());
        for (sent, whole, name) in &rows {
            println!("  {sent:>6}  (of {whole:>6})  {name}");
        }
        println!("\n  the ten heaviest come to {}", rows.iter().take(10).map(|(s, _, _)| s).sum::<usize>());

        // And inside the heaviest tool, argument by argument. `gear_numbers`
        // alone is a fifth of everything sent, and it is nearly all arguments —
        // so this says which of them to look at rather than guessing.
        let heaviest = &rows.first().expect("at least one tool").2;
        let Some(tool) = brief
            .as_array()
            .expect("an array")
            .iter()
            .find(|one| one["function"]["name"].as_str() == Some(heaviest.as_str()))
        else {
            return;
        };
        println!("\n  inside {heaviest}:");
        if let Some(properties) = tool["function"]["parameters"]["properties"].as_object() {
            let mut args: Vec<(usize, &String)> = properties
                .iter()
                .map(|(name, spec)| (spec.to_string().len(), name))
                .collect();
            args.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
            for (size, name) in args {
                println!("  {size:>6}  {name}");
            }
        }
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
        for wanted in [
            "search_wiki",
            "read_article",
            "item_stats",
            "player_status",
            "game_item",
            "gear_numbers",
            "search_web",
            "read_page",
            "place_marker",
            "map_markers",
            "spell_numbers",
            "talismans",
            "upgrade_path",
            "catalogue",
            "whos_here",
            "what_drops_here",
        ] {
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
                    "search_wiki"
                        | "read_article"
                        | "item_stats"
                        | "player_status"
                        | "game_item"
                        | "gear_numbers"
                        | "search_web"
                        | "read_page"
                        | "place_marker"
                        | "map_markers"
                        | "spell_numbers"
                        | "talismans"
                        | "spirit_ashes"
                        | "physick_tears"
                        | "ashes_of_war"
                        | "starting_classes"
                        | "spells_by_cost"
                        | "upgrade_path"
                        | "catalogue"
                        | "game_text"
                        | "whos_here"
                        | "what_drops_here"
                )
            })
            .collect();
        assert!(unhandled.is_empty(), "described but not implemented: {unhandled:?}");
    }

    /// Whether a failure told the player what to do next.
    fn a_way_forward(shown: &str) -> bool {
        let said = shown.to_lowercase();
        ["ask again", "asking again", "tried again", "a minute", "check the network"]
            .iter()
            .any(|hint| said.contains(hint))
    }

    /// A language wiki is searched for whoever plays in it and nobody else.
    #[test]
    fn a_wiki_in_one_language_is_read_by_that_language() {
        use crate::wiki::SOURCES;

        // Everybody gets the two that are written for everybody.
        for language in [None, Some("russian"), Some("german"), Some("japanese")] {
            let names: Vec<&str> = reading(None, language).iter().map(|one| one.id).collect();
            assert!(names.contains(&"eldenring"), "{language:?} lost the English wiki");
            assert!(names.contains(&"convergence"), "{language:?} lost the mod's wiki");
        }

        // And their own, only theirs.
        for (language, theirs) in [
            ("russian", "eldenring-ru"),
            ("german", "eldenring-de"),
            ("spanish", "eldenring-es"),
        ] {
            let mine: Vec<&str> =
                reading(None, Some(language)).iter().map(|one| one.id).collect();
            assert!(mine.contains(&theirs), "{language} did not get {theirs}: {mine:?}");
            for (_, other) in [
                ("russian", "eldenring-ru"),
                ("german", "eldenring-de"),
                ("spanish", "eldenring-es"),
            ] {
                if other != theirs {
                    assert!(!mine.contains(&other), "{language} was handed {other}");
                }
            }
        }

        // Somebody playing in English gets no language wiki at all.
        let plain: Vec<&str> = reading(None, None).iter().map(|one| one.id).collect();
        assert_eq!(plain, vec!["eldenring", "convergence"], "an English player got extras");

        // The mod's own wiki leads for somebody running it, whatever order the
        // list is written in — it used to be picked by position.
        assert_eq!(crate::wiki::for_edition(Some("convergence")).id, "convergence");
        assert_eq!(crate::wiki::for_edition(None).id, "eldenring");
        assert_eq!(reading(Some("convergence"), None).first().map(|one| one.id), Some("convergence"));

        // Every language wiki names the language it is for, or it can never be
        // reached; every general one names none, or nobody can reach it.
        for source in SOURCES {
            let gated = crate::wiki::spoken_in(source).is_some();
            let language_wiki = source.id.contains('-');
            assert_eq!(gated, language_wiki, "{} is gated wrongly", source.id);
        }
    }

    /// A script that settles the language gets it named.
    #[test]
    fn the_language_is_named_when_the_script_says_which() {
        for (question, expected) in [
            ("какая броня лучше держит молнию?", "RUSSIAN"),
            ("сколько весит Редувия", "RUSSIAN"),
            ("いま装備している防具の重量は?", "JAPANESE"),
            ("내 무기는 무엇입니까", "KOREAN"),
        ] {
            let said = in_what_language(question);
            assert!(said.contains(expected), "{question:?} was not named: {said:?}");
        }

        // Latin is not a language. Guessing between Dutch, Norwegian, Czech and
        // English would be worse than the general instruction that follows.
        for latin in [
            "Hvor mye veier rustningen min?",
            "Welk wapen doet het meeste schade?",
            "what does my dagger do",
            "",
        ] {
            assert!(in_what_language(latin).is_empty(), "{latin:?} was guessed at");
        }

        // And a name in the game's own alphabet inside an English question is
        // still that alphabet — this is about the player's own words, so a
        // Cyrillic weapon name in an English sentence names Russian. That is
        // the accepted cost of a rule this cheap, and it errs toward the
        // language their game is in.
        assert!(!in_what_language("what does Редувия do").is_empty());
    }

    /// The word for "nothing" is not a name to search for.
    #[test]
    fn the_word_for_nothing_is_treated_as_nothing() {
        for empty in ["null", "NULL", "none", "None", "undefined", "nil", "  null  ", ""] {
            let args = serde_json::json!({ "name": empty });
            assert_eq!(as_a_name(&args, "name"), "", "{empty:?} was searched for");
        }
        // A missing argument, and one that is not a string at all.
        assert_eq!(as_a_name(&serde_json::json!({}), "name"), "");
        assert_eq!(as_a_name(&serde_json::json!({ "name": 7 }), "name"), "");
        assert_eq!(as_a_name(&serde_json::Value::Null, "name"), "");

        // And a real name still goes through, including one that merely
        // contains the word.
        assert_eq!(as_a_name(&serde_json::json!({ "name": " Reduvia " }), "name"), "Reduvia");
        assert_eq!(as_a_name(&serde_json::json!({ "name": "Nullstone" }), "name"), "Nullstone");
    }

    /// A later round keeps every rule and drops one player's circumstances.
    #[test]
    fn a_question_about_the_game_leaves_the_launcher_paperwork_behind() {
        let full = |asked: &str| {
            let mut player = Player {
                asked: asked.to_string(),
                ..Default::default()
            };
            // The launcher-facing sections only exist when there is something
            // to say, so give them something.
            player.frames = Some("Their panel runs at 165 Hz and the game is capped.\n".into());
            player.backups = Some("Eleven snapshots, the newest an hour ago.\n".into());
            player.mirrors = Some("Both wikis are mirrored on this machine.\n".into());
            player.safety = Some("The anti-cheat shim is in place.\n".into());
            player.holdings = Some("4,510 weapons and 901 pieces of armour.\n".into());
            setup_worth_knowing(&player).unwrap_or_default()
        };

        let about_a_weapon = full("какое оружие даёт больше всего кровотечения?");
        let about_the_launcher = full("как поставить мод рядом с Convergence?");

        // The saving, and it has to be worth having.
        assert!(
            about_a_weapon.len() + 1_500 < about_the_launcher.len(),
            "the gate saved almost nothing: {} against {}",
            about_a_weapon.len(),
            about_the_launcher.len()
        );

        // What comes out.
        for gone in ["Add mod", "Load order", "Snapshot the save", "165 Hz", "Eleven snapshots"] {
            assert!(!about_a_weapon.contains(gone), "{gone} survived a question about a weapon");
            assert!(about_the_launcher.contains(gone), "{gone} is missing from a launcher question");
        }

        // What must NEVER come out. The anti-cheat state decides whether
        // playing online gets them banned and can be asked about in words this
        // gate does not know; the count of what the mod holds is a question
        // about the GAME that once got answered "127 and 214" off no count.
        for kept in ["anti-cheat shim", "4,510 weapons"] {
            assert!(about_a_weapon.contains(kept), "{kept} was dropped and must not be");
        }

        // And the guard-rail in place of the facts.
        assert!(
            about_a_weapon.contains("do NOT")
                && about_a_weapon.contains("Create backup"),
            "the shortened block stopped forbidding invented control names"
        );

        // An empty question is a caller who has not said, not a caller who
        // does not need it. Those get everything.
        assert!(full("").contains("Add mod"));
    }

    /// Routing guidance has to survive the shortening, or it is never sent.
    #[test]
    fn a_first_paragraph_carries_what_the_model_must_choose_on() {
        let brief = tools_in_brief(&tool_schemas());
        let all = brief.as_array().expect("the tools are a list");
        let find = |name: &str| -> serde_json::Value {
            all.iter()
                .find(|one| one["function"]["name"] == name)
                .unwrap_or_else(|| panic!("no tool called {name}"))
                .clone()
        };
        // Lowercased, because whether a word is shouted is a matter of
        // emphasis and this test is about whether it is THERE.
        let argument = |tool: &str, arg: &str| -> String {
            find(tool)["function"]["parameters"]["properties"][arg]["description"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
        };

        // Every family armour_against accepts. Each of these three was added
        // in a later paragraph and silently never delivered.
        let against = argument("gear_numbers", "armour_against");
        for must in [
            "physical", "poise", "robustness", "immunity", "focus", "vitality", "faith",
            "strength", "arcane", "attribute",
        ] {
            assert!(
                against.contains(must),
                "armour_against no longer offers \"{must}\" where a model can see it — check it \
                 has not been pushed past a blank line, which is the same as deleting it"
            );
        }
        // And the distinction that was got wrong three times running.
        assert!(
            against.contains("robustness is not poise"),
            "the robustness/poise warning is not in the part that gets sent"
        );

        // The other arguments whose whole point is in one word.
        assert!(argument("gear_numbers", "weapons_of_sort").contains("greatshield"));
        assert!(argument("gear_numbers", "armour_set").contains("set"));
        for ailment in ["poison", "bleed", "frost", "madness"] {
            assert!(
                argument("gear_numbers", "weapons_building").contains(ailment),
                "weapons_building no longer names {ailment} where it can be seen"
            );
        }

        // whos_here, where the same bug was found a second time. "GIVE IT A
        // NAME" sat in the FOURTH paragraph and its argument said only "a map
        // id like m60_35_44_00", so nothing the model could see said a boss's
        // name would work. Asked what the Red Wolf of Radagon gives, it read a
        // wiki and reported the base game's reward as theirs.
        let standing = find("whos_here")["function"]["description"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        for must in ["name", "rune"] {
            assert!(
                standing.contains(must),
                "whos_here no longer says \"{must}\" where a model can see it"
            );
        }
        let by_name = argument("whos_here", "map");
        assert!(
            by_name.contains("name"),
            "the whos_here argument says nothing about taking a NAME, so a question about a \
             boss with no map id has no way in: {by_name}"
        );

        // And a guard on the shortening itself: if it ever stops trimming,
        // this test would pass for the wrong reason.
        let whole = tool_schemas();
        assert!(
            brief.to_string().len() < whole.to_string().len(),
            "the brief form is no longer shorter, so this test proves nothing"
        );
    }

    /// Every control the block names must still exist in the interface.
    #[test]
    fn no_prose_carries_a_flattened_line_continuation() {
        // A Rust string continued with a trailing backslash swallows the
        // newline AND the indentation. Edit one of these through a tool that
        // eats the backslash and the two lines join with thirty spaces between
        // them — it still compiles, and the player reads a sentence with a
        // hole in it. This has happened three times, twice in text that goes
        // straight to a model and once in an assertion message.
        //
        // Nothing legitimate puts a run of twenty-plus spaces in the MIDDLE
        // of a sentence, so that is what to look for. Ten was tried first and
        // caught a println! that pads its columns, which is a real thing to do.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut scars: Vec<String> = Vec::new();
        let mut walk: Vec<std::path::PathBuf> = vec![src];
        while let Some(here) = walk.pop() {
            let Ok(entries) = std::fs::read_dir(&here) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk.push(path);
                    continue;
                }
                if path.extension().is_none_or(|kind| kind != "rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for (at, line) in text.lines().enumerate() {
                    // Anywhere after the first quote on the line. An earlier
                    // version of this SKIPPED lines beginning with a quote,
                    // which is most string literals in this file — so it found
                    // nothing but a false positive, and only proving it against
                    // an injected scar showed that up. Twenty spaces, not ten:
                    // a println! column header pads legitimately and those runs
                    // are short, while a flattened continuation carries the
                    // whole indentation of the next line, thirty or more.
                    let trimmed = line.trim_start();
                    let Some(quoted) = trimmed.find('"') else { continue };
                    let rest = &trimmed[quoted..];
                    let Some(hole) = rest.find("                    ") else { continue };
                    let before = rest[..hole].trim_end();
                    let after = rest[hole..].trim_start();
                    // Words on BOTH sides of the run: leading indentation
                    // inside a continued string is how this file is written and
                    // is not a scar.
                    if before.ends_with(|c: char| c.is_alphanumeric() || c == ',' || c == '.')
                        && after.starts_with(|c: char| c.is_alphabetic())
                    {
                        scars.push(format!(
                            "{}:{}",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            at + 1
                        ));
                    }
                }
            }
        }
        assert!(
            scars.is_empty(),
            "these lines have a run of spaces in the middle of a sentence, which is what a \
             flattened line continuation looks like: {scars:#?}"
        );
    }

    #[test]
    fn no_test_has_quietly_lost_its_attribute() {
        // A test with no #[test] above it compiles, is never run, and looks
        // exactly like a test. This has happened three times in this codebase,
        // every time from inserting a function immediately before an existing
        // one and taking its attribute with it — and twice the silently dead
        // test was the only thing guarding a figure.
        //
        // Cheap to catch: read the sources, find every `fn name()` inside a
        // `mod tests`, and check something attribute-shaped sits above it. A
        // parser is not needed and would not be more convincing.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut orphans: Vec<String> = Vec::new();
        let mut walk: Vec<std::path::PathBuf> = vec![src];
        while let Some(here) = walk.pop() {
            let Ok(entries) = std::fs::read_dir(&here) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk.push(path);
                    continue;
                }
                if path.extension().is_none_or(|kind| kind != "rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                let lines: Vec<&str> = text.lines().collect();
                let mut in_tests = false;
                for (at, line) in lines.iter().enumerate() {
                    if line.trim_start().starts_with("mod tests") {
                        in_tests = true;
                    }
                    if !in_tests {
                        continue;
                    }
                    let trimmed = line.trim_start();
                    // Only no-argument functions: a helper taking arguments
                    // cannot be a test and is a normal thing to have here.
                    if !(trimmed.starts_with("fn ") && trimmed.contains("() {")) {
                        continue;
                    }
                    // Walk back over doc comments and attributes, counting the
                    // #[test]s. TWO is as much a symptom as none: both come
                    // from inserting a function immediately before an existing
                    // one, and a doubled attribute registers the test twice,
                    // which is how the problem keeps being noticed late — by a
                    // filter that matches one name reporting "running 2 tests".
                    let mut above = at;
                    let mut attributed = 0;
                    let mut tests = 0;
                    while above > 0 {
                        let before = lines[above - 1].trim_start();
                        if before.starts_with("#[") {
                            attributed += 1;
                            if before.starts_with("#[test]") {
                                tests += 1;
                            }
                        } else if !(before.starts_with("///") || before.starts_with("//")) {
                            break;
                        }
                        above -= 1;
                    }
                    if tests > 1 {
                        let name = trimmed.trim_start_matches("fn ");
                        let name = name.split('(').next().unwrap_or(name);
                        orphans.push(format!(
                            "{}:{} {name} has {tests} #[test] attributes, which means an edit \
                             took one from the function below it",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            at + 1
                        ));
                    }
                    if attributed == 0 {
                        let name = trimmed.trim_start_matches("fn ");
                        let name = name.split('(').next().unwrap_or(name);
                        orphans.push(format!(
                            "{}:{} {name}",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            at + 1
                        ));
                    }
                }
            }
        }
        assert!(
            orphans.is_empty(),
            "these look like tests but have no attribute above them, so they never run: {orphans:#?}"
        );
    }

    #[test]
    fn every_control_the_block_names_is_still_in_the_interface() {
        // From src-tauri, where the tests run, to the interface beside it. A
        // relative path, so nothing here depends on where the repo is checked
        // out.
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/pages");
        if !ui.exists() {
            // A source build without the interface beside it. Nothing to check
            // against, and failing here would be about the checkout, not the
            // block.
            return;
        }

        for (pane, names) in [
            (
                "panes/SavesPane.tsx",
                &["Snapshot", "Snapshots", "Restore", "Move", "Convert", "Markers",
                  "Pin a place"][..],
            ),
            (
                "panes/ModsPane.tsx",
                &["Add mod", "Add an archive", "Add a folder", "Load order", "Installed",
                  "Options", "Conflicts", "Folder"][..],
            ),
            ("panes/CoopPane.tsx", &["Session", "Password"][..]),
            ("Overlay.tsx", &["Snapshot the save", "Picture", "Co-op password"][..]),
            ("Stage.tsx", &["Play", "Mods", "Saves", "Co-op", "Codex", "Wiki", "System"][..]),
        ] {
            let path = ui.join(pane);
            let Ok(source) = std::fs::read_to_string(&path) else {
                panic!("the block names controls in {pane}, which is not there any more");
            };
            for name in names {
                assert!(
                    source.contains(name),
                    "the block tells the player about \"{name}\" and {pane} no longer has it — \
                     either the control was renamed, in which case fix the block, or it was \
                     removed, in which case take it out of the block"
                );
            }
        }

        // And the block really does name them, so this test cannot pass by
        // checking an interface the block has stopped describing.
        let block = crate::ask::setup_worth_knowing(&Player::default()).unwrap_or_default();
        for name in
            ["Snapshot", "Add mod", "Load order", "Convert", "Session", "SHIFT+F1", "Picture"]
        {
            assert!(block.contains(name), "the block no longer names {name}");
        }
    }

    #[test]
    fn ageing_the_block_takes_the_circumstances_and_leaves_the_rules() {
        let one = Player {
            set_up: Some("\n## Installed here\n\nELDEN RING, with two mods.\n".into()),
            holdings: Some("\nThey have 40 saves.\n".into()),
            language: Some("Russian".into()),
            seamless: true,
            ..Default::default()
        };
        let Some(block) = setup_worth_knowing(&one) else {
            return;
        };
        let (rules, dropped) = rules_only(&block);

        // Every heading survives, so nothing has silently vanished — a
        // circumstance section keeps its heading and loses its body.
        for (_, name) in setup_by_section(&block) {
            assert!(
                rules.contains(&name) || name.starts_with('('),
                "the section \"{name}\" disappeared entirely"
            );
        }

        // The rules themselves, whole. Checked against what the block actually
        // has rather than against a list written here — a phrase this test
        // expects and the block does not carry fails for the wrong reason, and
        // one the block gains later would go unchecked.
        for rule in ["BEFORE YOU NAME A THING", "A FIND BEATS A MISS", "What is never invented"] {
            if block.contains(rule) {
                assert!(rules.contains(rule), "a rule went with the circumstances: {rule}");
            }
        }
        // And this is the assertion that stopped the whole idea. Every line
        // that is not about THIS player has to survive — and one does not:
        //
        //   "Roundtable itself is version 0.2.3. Whether a newer Roundtable
        //    exists is checked in the launcher, not from here — say it is on
        //    its own screen rather than guessing either way."
        //
        // A fact about this installation and a rule about how to answer, in
        // one sentence, inside a section the cut removes. The same is true of
        // the backup count ("do not produce a count of your own") and the
        // character list ("hours played are exact; do not recompute them").
        //
        // So the check is kept and inverted: what it proves is that cutting by
        // section takes rules with it, and `rules_only` is written, tested and
        // deliberately unused because of this.
        let lost: Vec<&str> = block
            .lines()
            .filter(|line| line.trim().len() > 40)
            .filter(|line| !rules.contains(line.trim()))
            .collect();
        assert!(
            lost.iter().any(|line| line.contains("say it is on its own screen")),
            "the rule this test exists to catch is no longer in the block; if the \
             circumstance sections have been cleared of rules, `rules_only` can be used \
             after all — check the others first"
        );

        // And it has to save something, or it is risk for nothing.
        assert!(dropped > 0, "nothing was dropped, so this does nothing");
        assert!(rules.len() < block.len(), "the shortened block is not shorter");

        // A bare installation has no circumstances to drop and must come back
        // untouched rather than mangled.
        if let Some(bare) = setup_worth_knowing(&Player::default()) {
            let (same, none) = rules_only(&bare);
            assert_eq!(none, 0, "a bare block had circumstances taken out of it");
            assert_eq!(same.trim_end(), bare.trim_end(), "a bare block was changed anyway");
        }
    }

    /// A tool shortened for a later round is still a tool that can be called.
    #[test]
    fn a_shortened_tool_can_still_be_called() {
        let whole = tool_schemas();
        let brief = tools_in_brief(&whole);

        let all = whole.as_array().expect("the schemas are a list");
        let cut = brief.as_array().expect("and so is the short form");
        assert_eq!(all.len(), cut.len(), "a tool went missing in the shortening");

        for (long, short) in all.iter().zip(cut) {
            let name = long["function"]["name"].as_str().expect("every tool is named");
            assert_eq!(short["function"]["name"].as_str(), Some(name), "a tool was renamed");

            // How to call it must survive untouched, or the call cannot be made.
            // How to call it survives: same arguments, same types, same
            // required list. Only the prose inside each may be shorter.
            let names = |schema: &serde_json::Value| -> Vec<String> {
                schema["properties"]
                    .as_object()
                    .map(|all| all.keys().cloned().collect())
                    .unwrap_or_default()
            };
            assert_eq!(
                names(&long["function"]["parameters"]),
                names(&short["function"]["parameters"]),
                "{name} lost an argument"
            );
            assert_eq!(
                long["function"]["parameters"]["required"],
                short["function"]["parameters"]["required"],
                "{name} changed which arguments are required"
            );
            for argument in names(&long["function"]["parameters"]) {
                let was = &long["function"]["parameters"]["properties"][&argument];
                let now = &short["function"]["parameters"]["properties"][&argument];
                assert_eq!(was["type"], now["type"], "{name}.{argument} changed type");
                if let Some(about) = now["description"].as_str() {
                    assert!(
                        was["description"].as_str().unwrap_or("").starts_with(about),
                        "{name}.{argument} was reworded rather than shortened"
                    );
                }
            }

            // And what it IS must survive: an empty description is a tool the
            // model has no reason to pick.
            let said = short["function"]["description"].as_str().unwrap_or("");
            assert!(said.len() > 40, "{name} was cut down to nothing: {said:?}");
            assert!(
                long["function"]["description"].as_str().unwrap_or("").starts_with(said),
                "{name} was reworded rather than shortened"
            );
        }

        // It has to save enough to be worth the risk of shortening at all.
        //
        // A third, not a half. The bar was "more than half" and it has been hit
        // three times by tools and arguments being added — legitimately, each
        // time — which is a threshold measuring the wrong thing: whether the
        // schemas have grown, rather than whether the shortening works. It
        // saves a little under half as this is written (24,992 to 12,623) and
        // that is a good saving; a third is the point below which it stops
        // paying for the risk that a trimmed description costs a call.
        let before = whole.to_string().len();
        let after = brief.to_string().len();
        assert!(
            after * 3 < before * 2,
            "the shortening saved almost nothing: {before} became {after}"
        );
    }

    /// What each part of the always-present block costs, largest first.
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_setup_sections() {
        let Some(block) = setup_worth_knowing(&Player::default()) else {
            println!("  nothing to say");
            return;
        };
        let sections = setup_by_section(&block);
        let whole = block.len();
        println!("\n  {whole} chars, on every round, for everybody\n");
        for (size, name) in &sections {
            println!("  {size:>6}  {:>3}%  {name}", size * 100 / whole.max(1));
        }
    }

    /// How much of the always-present block is the same for everybody.
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_setup_prefix() {
        // Two installations with nothing in common but the rules.
        let one = Player {
            set_up: Some("\n## Installed here\n\nELDEN RING, with two mods.\n".into()),
            holdings: Some("\nThey have 40 saves.\n".into()),
            language: Some("Russian".into()),
            seamless: true,
            ..Default::default()
        };
        let two = Player {
            set_up: Some("\n## Installed here\n\nDARK SOULS III, unmodded.\n".into()),
            holdings: Some("\nThey have 2 saves.\n".into()),
            language: Some("German".into()),
            ..Default::default()
        };

        let (Some(first), Some(second)) =
            (setup_worth_knowing(&one), setup_worth_knowing(&two))
        else {
            println!("  nothing to say for either");
            return;
        };

        let shared = first
            .as_bytes()
            .iter()
            .zip(second.as_bytes())
            .take_while(|(a, b)| a == b)
            .count();
        // And from the other end: what is the same AFTER they diverge, which is
        // what moving the varying part to the end would add to the prefix.
        let tail = first
            .as_bytes()
            .iter()
            .rev()
            .zip(second.as_bytes().iter().rev())
            .take_while(|(a, b)| a == b)
            .count();

        println!("\n  {} chars for one, {} for the other", first.len(), second.len());
        println!("  {shared} shared at the front  ({}%)", shared * 100 / first.len().max(1));
        println!("  {tail} shared at the back");
        println!(
            "  {} could be a prefix if the varying part moved last  ({}%)",
            shared + tail,
            (shared + tail) * 100 / first.len().max(1)
        );
        println!("\n  --- where they first differ ---");
        let from = shared.saturating_sub(90);
        println!("  ...{}", &first[from..(shared + 90).min(first.len())]);

        // And the ceiling: the block with no player in it at all is the part
        // every installation on earth would share, which is what a prefix could
        // be worth if the varying parts all moved to the end.
        if let Some(bare) = setup_worth_knowing(&Player::default()) {
            println!("\n  {} chars with no player wired in at all", bare.len());
            println!(
                "  so reordering could take the prefix from {shared} to about {}",
                bare.len()
            );
        }
        println!("  tool schemas, shared by everyone already: {}", tool_schemas().to_string().len());
    }

    /// What each tool costs to offer, largest first.
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_tool_sizes() {
        let schemas = tool_schemas();
        let mut each: Vec<(usize, String)> = schemas
            .as_array()
            .map(|all| {
                all.iter()
                    .map(|one| {
                        let name = one["function"]["name"].as_str().unwrap_or("?").to_string();
                        (one.to_string().len(), name)
                    })
                    .collect()
            })
            .unwrap_or_default();
        each.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
        let whole = schemas.to_string().len();
        let brief = tools_in_brief(&schemas).to_string().len();
        println!("\n  {whole} chars over {} tools on the first round", each.len());
        println!("  {brief} chars on every round after it\n");

        // Where the brief form's weight sits: what a tool IS against how it is
        // called. Trimming the wrong one of those costs a call that cannot be
        // made at all.
        let short = tools_in_brief(&schemas);
        let mut split: Vec<(usize, usize, String)> = short
            .as_array()
            .map(|all| {
                all.iter()
                    .map(|one| {
                        let name = one["function"]["name"].as_str().unwrap_or("?").to_string();
                        let said = one["function"]["description"].to_string().len();
                        let how = one["function"]["parameters"].to_string().len();
                        (said, how, name)
                    })
                    .collect()
            })
            .unwrap_or_default();
        split.sort_by_key(|(said, how, _)| std::cmp::Reverse(said + how));
        println!("  brief form, what it is / how to call it:");
        for (said, how, name) in &split {
            println!("    {:>5} = {said:>5} + {how:>5}  {name}", said + how);
        }
        let (words, calls): (usize, usize) =
            split.iter().fold((0, 0), |(w, c), (said, how, _)| (w + said, c + how));
        println!("\n    {words} describing them, {calls} saying how to call them");
        for (size, name) in &each {
            println!("  {size:>6}  {:>4}%  {name}", size * 100 / whole.max(1));
        }
    }

    /// Armour is ranked as a set of clothes, not as a leaderboard.
    #[test]
    fn the_armour_ranking_covers_every_place_a_piece_is_worn() {
        // Deliberately in the order the server hands them over: grouped, best
        // first inside each group. Six in one group, to prove the cut.
        let mut ranked: Vec<Shielding> = Vec::new();
        for (at, worn) in crate::formats::regulation::slot::NAMES.iter().enumerate() {
            for nth in 0..6 {
                ranked.push((
                    worn,
                    format!("{worn} piece {nth}"),
                    20.0 - nth as f32,
                    3.0 + at as f32,
                ));
            }
        }
        let shown = a_set_against("fire", &ranked, Some((11.5, 49.8)));

        // Every place appears, with its heading and its best piece.
        for worn in crate::formats::regulation::slot::NAMES {
            assert!(
                shown.contains(&worn.to_uppercase()),
                "nothing worn on the {worn} was offered:\n{shown}"
            );
            assert!(shown.contains(&format!("{worn} piece 0")), "{worn} lost its best");
        }
        // Five apiece, so the sixth is left out and the answer stays readable.
        assert!(!shown.contains("piece 5"), "a sixth piece got through:\n{shown}");
        assert_eq!(shown.matches("stops ").count(), 20, "not five in each of four");

        // The count is of everything rated, not of what is printed — a player
        // told "out of 20" would think the game had twenty pieces of armour.
        assert!(shown.contains("out of 24 rated"), "the total was miscounted:\n{shown}");

        // The set is added up here, not in a sentence. Given the pieces and
        // the limit separately, an answer put a 37.5 set at 75% of a 49.8
        // limit when it is 80 — a division anybody can do, done wrong, in
        // front of somebody deciding what to wear.
        assert!(shown.contains("together weighs"), "the set was not added up:\n{shown}");
        assert!(
            shown.contains("do not divide anything yourself"),
            "nothing stops the model doing the sum again:\n{shown}"
        );
        // And with no game open there is no limit, so none is offered.
        let blind = a_set_against("fire", &ranked, None);
        assert!(
            !blind.contains("together weighs"),
            "a limit was quoted with no game to read it from:\n{blind}"
        );

        // Poise reads as carried, not as stopped. Asked which armour gives the
        // most of it, an answer said poise was not in the tables at all and
        // ranked by physical defence instead; the figure was there and the
        // ranking now takes it, so the words have to follow.
        let poise = a_set_against("poise", &ranked, None);
        assert!(poise.contains("The most poise"), "poise is being ranked as a defence:\n{poise}");
        assert!(!poise.contains("stops"), "poise is something you have, not something it stops");
        assert!(poise.contains("of it, weighs"), "the per-piece line still reads as a percentage");

        // A shield is not in here and a negative about one must not be drawn
        // from it. One was: "no shield in this build blocks 100% physical",
        // out of a ranking of 851 worn pieces and no shields whatever.
        assert!(shown.contains("Shields are not here"), "shields went unmentioned:\n{shown}");
        assert!(shown.contains("100%"), "the exact wrong claim went unnamed:\n{shown}");

        // And the thing the model must not do: add the four together. It did,
        // and announced the sum as a percentage — so what is pinned here is not
        // just that the prohibition exists but that it comes LAST. It used to
        // sit in the middle with advice about weight after it, and the advice
        // is what got followed.
        assert!(shown.contains("Do not add"), "nothing stops it summing them:\n{shown}");
        assert!(shown.contains("56.3"), "the wrong arithmetic went unnamed:\n{shown}");
        let tail_starts = shown.len().saturating_sub(500);
        assert!(
            shown[tail_starts..].contains("Do not add"),
            "the prohibition is buried where the last one was ignored:\n{shown}"
        );
    }

    /// The kinds offered are the kinds that exist.
    #[test]
    fn every_kind_the_catalogue_offers_is_one_it_knows() {
        let schemas = tool_schemas();
        let offered = schemas
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["function"]["name"] == "catalogue")
            .map(|tool| tool["function"]["parameters"]["properties"]["kind"]["description"].clone())
            .expect("the catalogue takes a kind");
        let offered = offered.as_str().expect("a description");

        for kind in ["weapon", "armour", "talisman", "item", "ash of war", "skill"] {
            assert!(offered.contains(kind), "{kind} is not offered: {offered}");
            assert!(crate::library::is_a_kind(kind), "{kind} is offered and not known");
        }
        // And the ones that are not kinds are refused rather than searched.
        for made_up in ["spirit", "spirit ash", "summon", "boss", "place", "npc"] {
            assert!(!crate::library::is_a_kind(made_up), "{made_up} passed as a kind");
        }

        // The same request spelled another way is the same request. "armor"
        // was refused outright, which is a right answer to the wrong question.
        for (asked, meant) in [
            ("armor", "armour"),
            ("Armour", "armour"),
            ("talismans", "talisman"),
            ("weapons", "weapon"),
            ("gem", "ash of war"),
            ("consumable", "item"),
        ] {
            assert_eq!(
                crate::library::as_a_kind(asked),
                Some(meant),
                "{asked} did not reach {meant}"
            );
        }
    }

    /// Every rule the standing block has earned is still in it.
    #[test]
    fn the_standing_block_still_carries_every_rule_it_earned() {
        let bare = Player::default();
        let said = setup_worth_knowing(&bare).expect("there is always something to say");

        // Each entry: what the rule is for, and a phrase that has to appear.
        // The phrases are deliberately short and central, so rewording a
        // sentence does not break the test but losing the point does.
        for (why, must) in [
            ("the launcher restores saves", "snapshot"),
            ("moving a character is a real control", "Move"),
            ("converting to co-op is a real control", "Convert"),
            ("the interface must not be described", "never seen the interface"),
            ("it is not only for one game", "not only for this game"),
            ("only one game's tables are read", "only read for ELDEN RING"),
            ("the answerer is not the launcher", "YOU, answering"),
            ("the overlay's key", "SHIFT+F1"),
            ("no file paths", "made-up path"),
            ("no invented items", "Somerset Stone"),
            ("the assistant cannot act", "never \"I have done that\""),
            ("what is withheld is withheld on purpose", "nobody else's business"),
            // The one rule the whole launcher exists to enforce, and the one
            // that used to be scattered across a dozen paragraphs instead of
            // stated once: where an answer is allowed to come from, in order.
            ("the source ladder exists", "Where an answer may come from"),
            ("the tables outrank everything", "Nothing outranks them"),
            ("memory is the last resort", "YOUR OWN MEMORY"),
        ] {
            assert!(
                said.contains(must),
                "the block no longer says anything about {why} — looked for {must:?}"
            );
        }

        // And it is one block, not a wall: the sections are what make a rule
        // findable rather than buried in the middle of a paragraph.
        let headings = said.lines().filter(|line| line.starts_with("## ")).count();
        assert!(headings >= 5, "the block has {headings} sections and reads as one wall");
    }

    /// How much the descriptions cost, watched rather than capped.
    #[test]
    fn the_tool_descriptions_have_not_quietly_doubled() {
        let schemas = tool_schemas();
        let list = schemas.as_array().expect("an array of tools");

        let mut sizes: Vec<(usize, &str)> = list
            .iter()
            .filter_map(|tool| {
                let function = &tool["function"];
                Some((
                    function["description"].as_str()?.len(),
                    function["name"].as_str()?,
                ))
            })
            .collect();
        sizes.sort_by_key(|(size, _)| std::cmp::Reverse(*size));

        let whole = schemas.to_string().len();
        println!("{} tools, {whole} characters of schema", list.len());
        for (size, name) in sizes.iter().take(6) {
            println!("  {size:>5}  {name}");
        }

        // 20,942 at the time of writing, across sixteen tools.
        assert!(whole < 42_000, "the tool list has reached {whole} characters");
    }

    #[test]
    fn a_requirement_they_do_not_meet_is_stated_rather_than_computed() {
        // The real case: Rivers of Blood wants dexterity 18 and this character
        // has 14. A model was handed both numbers and said the requirement was
        // met, then recommended the weapon.
        let theirs = vec![
            ("strength (STR)".to_string(), 10u32),
            ("dexterity (DEX)".to_string(), 14),
            ("faith (FTH)".to_string(), 22),
            ("arcane (ARC)".to_string(), 26),
        ];
        let rivers = vec![
            ("strength (STR)".to_string(), 10u8),
            ("dexterity (DEX)".to_string(), 18),
            ("faith (FTH)".to_string(), 20),
            ("arcane (ARC)".to_string(), 20),
        ];
        let said = short_of(&rivers, Some(&theirs));
        assert!(said.contains("CANNOT"), "{said}");
        assert!(said.contains("dexterity (DEX) 14 of 18"), "{said}");
        // And only the one they are short on.
        assert!(!said.contains("faith"), "{said}");

        // Reduvia, which they do meet.
        let reduvia = vec![
            ("strength (STR)".to_string(), 5u8),
            ("dexterity (DEX)".to_string(), 8),
            ("faith (FTH)".to_string(), 13),
            ("arcane (ARC)".to_string(), 13),
        ];
        assert!(short_of(&reduvia, Some(&theirs)).contains("meet every requirement"));

        // With the game shut there are no attributes, and saying nothing beats
        // guessing at whether they qualify.
        assert!(short_of(&rivers, None).is_empty());
        assert!(short_of(&[], Some(&theirs)).is_empty());
    }

    #[test]
    fn a_tool_call_written_out_as_prose_is_caught() {
        // All of these reached a player. The last one is verbatim from a lane
        // that answered a question about bleed builds with its own scaffolding.
        for leak in [
            "<tool_call>{\"name\":\"search_wiki\"}",
            "<function=read_article>{\"title\":\"Rellana\"}</function>",
            "]<]minimax[>[<tool_call>]<]minimax[>[<invoke name=\"search_wiki\">",
            "Sure! <invoke name=\"item_stats\">",
        ] {
            assert!(leaked_a_tool_call(leak), "let through: {leak}");
        }

        // And an answer that merely mentions the machinery is still an answer.
        for real in [
            "Reduvia deals 106 fire damage.",
            "Метки на карте: 5.",
            "The launcher can restore a save from before the launch.",
            "",
        ] {
            assert!(!leaked_a_tool_call(real), "swallowed: {real}");
        }
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
