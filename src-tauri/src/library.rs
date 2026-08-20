//! What the game calls its things, read off the disk.
//!
//! [`crate::text`] answers the same question out of the running process, and
//! while the game is up that is the better source — it is the copy the player
//! is looking at, mods and all. This is for the rest of the time, which is most
//! of the time: the launcher is open before the game starts and after it quits,
//! and "I cannot tell you what that is until you launch the game" is not an
//! answer anybody wants.
//!
//! The message archives name everything in the same shape — `WeaponName` beside
//! `WeaponCaption` and `WeaponInfo`, and so on for each kind — so one pass over
//! them gives names, the flavour text under an item and the line that says what
//! it does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::formats::fmg;

/// The kinds worth answering about, and the prefix each one's tables carry.
const KINDS: [(&str, &str); 6] = [
    ("Weapon", "weapon"),
    ("Protector", "armour"),
    ("Accessory", "talisman"),
    ("Goods", "item"),
    ("Gem", "ash of war"),
    ("Arts", "skill"),
];

/// The words a caller may narrow by, comma-separated.
///
/// Taken from the list itself so the tool's description, the check that a
/// caller used a real one, and what the shelf actually holds cannot drift. A
/// kind nobody has silently matched nothing, and "nothing matches" reads as
/// "the game has no such thing": asked which spirit summons the player had, a
/// model narrowed by a kind that does not exist and was told, in effect, that
/// there are none. There are; the game files them under items.
pub fn kinds() -> String {
    KINDS.map(|(_, what)| what).join(", ")
}

/// A caller's narrowing word as one of ours, where it plainly means one.
///
/// The list is spelled one way and a caller reaching for it will not always
/// guess which: "armor" was refused outright, which is a right answer to the
/// wrong question. A plural and the other spelling are the same request, and
/// refusing them teaches nothing. Anything that is genuinely not a kind still
/// is refused, because silently searching everything would hide the mistake.
pub fn as_a_kind(what: &str) -> Option<&'static str> {
    let lowered = what.trim().to_lowercase();
    let asked = lowered.strip_suffix('s').unwrap_or(&lowered);
    if let Some(known) = KINDS.iter().map(|(_, known)| *known).find(|known| *known == asked) {
        return Some(known);
    }
    match asked {
        "armor" => Some("armour"),
        "ashe" | "ash" | "war ash" | "gem" => Some("ash of war"),
        "good" | "consumable" | "material" => Some("item"),
        "accessory" | "charm" => Some("talisman"),
        "art" => Some("skill"),
        _ => None,
    }
}

/// Whether a caller's narrowing word is one of them.
pub fn is_a_kind(what: &str) -> bool {
    as_a_kind(what).is_some()
}

/// One thing the game has words for.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// Weapon, armour, talisman, item — as a word, for the answer.
    pub what: String,
    /// The row it came from, so a name read off the disk can be joined to the
    /// same row in the tables. Without it the two halves of "which talismans
    /// does this installation have, and what do they weigh" could only be put
    /// together while the game was running.
    pub id: u32,
    pub name: String,
    /// The line under it that says what it does.
    pub effect: Option<String>,
    /// The description, which in this game is where the lore lives.
    pub caption: Option<String>,
}

/// Everything named, kept per installation and language.
type Shelf = Arc<Vec<Found>>;

/// Which installation, in which language, as it stood when it was read. The
/// timestamp is in the key so installing a mod is a different shelf rather than
/// a stale one.
type Which = (PathBuf, String, Option<std::time::SystemTime>);

fn shelves() -> &'static Mutex<HashMap<Which, Shelf>> {
    static SHELVES: OnceLock<Mutex<HashMap<Which, Shelf>>> = OnceLock::new();
    SHELVES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Everything the installed game has a name for.
///
/// Reads the loose text a mod leaves on disk, and failing that the archives an
/// untouched installation keeps its own inside. Empty only when neither is
/// there, which is a reason to fall back on the running game, not a failure.
pub fn everything(game_dir: &Path, mod_dir: Option<&Path>, language: &str) -> Shelf {
    let root = mod_dir.unwrap_or(game_dir);
    let stamp = std::fs::metadata(root)
        .and_then(|meta| meta.modified())
        .ok();
    let key = (root.to_path_buf(), language.to_string(), stamp);

    if let Ok(shelves) = shelves().lock() {
        if let Some(had) = shelves.get(&key) {
            return Arc::clone(had);
        }
    }

    let built = Arc::new(gather(game_dir, mod_dir, language));
    if let Ok(mut shelves) = shelves().lock() {
        shelves.insert(key, Arc::clone(&built));
    }
    built
}

/// Every message table an installation has, by the name the archive gives it.
///
/// Three places to look, in the order they are worth having: the mod's loose
/// text, the game's loose text, and the game's own packed archives. Public
/// because the tables carry more than this module surfaces — `NpcName` and
/// `PlaceName` are in the same file as the weapons — and a second reader that
/// only knew about one of the three sources is how a modded installation that
/// had never been launched ended up with no names at all.
pub fn tables_for(
    game_dir: &Path,
    mod_dir: Option<&Path>,
    language: &str,
) -> HashMap<String, fmg::Strings> {
    crate::formats::oodle::register(game_dir);

    // Every table from every archive of this language, merged with the base
    // tables winning: a mod that merges the DLC ships an untranslated copy of
    // them under the same ids, and letting it land last undoes the translation.
    let mut tables: HashMap<String, fmg::Strings> = HashMap::new();
    for root in [mod_dir, Some(game_dir)].into_iter().flatten() {
        let dir = root.join("msg").join(language);
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut archives: Vec<PathBuf> = files
            .flatten()
            .map(|file| file.path())
            .filter(|path| path.to_string_lossy().ends_with(".msgbnd.dcx"))
            .collect();
        archives.sort();

        for path in archives {
            let Ok(read) = fmg::archive(&path) else {
                continue;
            };
            let mut names: Vec<String> = read.keys().cloned().collect();
            names.sort_by_key(|name| (name.len(), name.clone()));
            for name in names {
                let base = name
                    .split("_dlc")
                    .next()
                    .unwrap_or(&name)
                    .to_string();
                let into = tables.entry(base).or_default();
                for (id, text) in &read[&name] {
                    into.entry(*id).or_insert_with(|| text.clone());
                }
            }
        }
        if !tables.is_empty() {
            break;
        }
    }
    // Nothing loose, which is every untouched installation. The same archives
    // are in the game's own packed files, and reading them is the difference
    // between the whole catalogue and none of it.
    if tables.is_empty() {
        tables = crate::formats::fmg::packed(game_dir, language);
    }
    tables
}

fn gather(game_dir: &Path, mod_dir: Option<&Path>, language: &str) -> Vec<Found> {
    let tables = tables_for(game_dir, mod_dir, language);
    if tables.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for (prefix, what) in KINDS {
        let Some(names) = tables.get(&format!("{prefix}Name")) else {
            continue;
        };
        let captions = tables.get(&format!("{prefix}Caption"));
        let effects = tables.get(&format!("{prefix}Info"));

        for (id, name) in names {
            let name = plain(name);
            if !worth_naming(&name) {
                continue;
            }
            out.push(Found {
                what: what.to_string(),
                id: *id,
                name,
                effect: effects.and_then(|table| table.get(id)).map(|t| plain(t)),
                caption: captions.and_then(|table| table.get(id)).map(|t| plain(t)),
            });
        }
    }
    out
}

/// The things whose name contains what was asked for, best first.
pub fn look_up(shelf: &[Found], query: &str, most: usize) -> Vec<Found> {
    let wanted = query.trim().to_lowercase();
    if wanted.is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<(u8, usize, &Found)> = shelf
        .iter()
        .filter_map(|item| {
            let name = item.name.to_lowercase();
            let rank = if name == wanted {
                0
            } else if name.split_whitespace().any(|word| word == wanted) {
                1
            } else if name.starts_with(&wanted) {
                2
            } else if name.contains(&wanted) {
                3
            } else {
                return None;
            };
            Some((rank, name.len(), item))
        })
        .collect();

    hits.sort_by_key(|(rank, length, _)| (*rank, *length));
    hits.into_iter()
        .take(most)
        .map(|(_, _, item)| item.clone())
        .collect()
}

/// Markup out, words left.
pub fn plain(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut inside = false;
    for ch in raw.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if inside => {}
            _ => out.push(ch),
        }
    }
    out.trim().to_string()
}

/// Whether an entry names something real.
///
/// These tables are full of the studio's own scaffolding, and answering a
/// question with "test gem 1" is worse than answering it with nothing.
pub fn worth_naming(name: &str) -> bool {
    let lower = name.to_lowercase();
    if name.trim().is_empty() || lower.starts_with("test") || lower.starts_with("dummy") {
        return false;
    }
    // What the game writes into a row nobody filled in. These were getting
    // through: a listing of the ashes of war came back with "[ERROR]" among
    // the real names, which is the catalogue offering the player an item that
    // is not one.
    if lower.starts_with("[error]") || lower.starts_with("%null%") {
        return false;
    }
    !matches!(lower.as_str(), "?" | "???" | "%null%" | "(dummytext)" | "dlc dummy")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a word actually matches in the installed catalogue.
    ///
    /// `cargo test --lib show_catalogue_matches -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_catalogue_matches() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let shelf = everything(&game_dir, mod_dir.as_deref(), "rusru");
        println!("{} things named", shelf.len());

        let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
        for one in shelf.iter() {
            *kinds.entry(one.what.as_str()).or_default() += 1;
        }
        println!("by kind: {kinds:?}");

        for word in ["пепел", "прах", "дух", "призыв", "гравитас"] {
            let hits = look_up(&shelf, word, 6);
            println!("\n  {word:?}: {} shown", hits.len());
            for one in &hits {
                println!("    [{}] {}", one.what, one.name);
            }
        }
    }

    fn shelf() -> Vec<Found> {
        [
            ("weapon", "Reduvia"),
            ("weapon", "Reduvia Blood Blade"),
            ("talisman", "Radagon's Scarseal"),
            ("item", "Rune Arc"),
        ]
        .into_iter()
        .enumerate()
        .map(|(at, (what, name))| Found {
            what: what.to_string(),
            id: at as u32,
            name: name.to_string(),
            effect: None,
            caption: None,
        })
        .collect()
    }

    #[test]
    fn an_exact_name_comes_before_the_longer_one_containing_it() {
        let found = look_up(&shelf(), "Reduvia", 5);
        assert_eq!(found[0].name, "Reduvia");
        assert_eq!(found[1].name, "Reduvia Blood Blade");
    }

    #[test]
    fn a_word_inside_a_name_still_finds_it() {
        assert_eq!(look_up(&shelf(), "scarseal", 5)[0].name, "Radagon's Scarseal");
        assert_eq!(look_up(&shelf(), "arc", 5)[0].name, "Rune Arc");
        assert!(look_up(&shelf(), "Moonveil", 5).is_empty());
        assert!(look_up(&shelf(), "  ", 5).is_empty());
    }

    #[test]
    fn the_studios_scaffolding_is_not_offered_to_the_player() {
        // These are real entries in the real tables.
        for junk in ["test gem 1", "DLC dummy", "%null%", "", "  "] {
            assert!(!worth_naming(junk), "{junk:?} got through");
        }
        for real in ["Reduvia", "Рунная дуга", "Longsword"] {
            assert!(worth_naming(real), "{real:?} was dropped");
        }
    }

    #[test]
    fn markup_does_not_reach_the_player() {
        assert_eq!(plain("<font color=\"#928977\">Dead</font>"), "Dead");
        assert_eq!(plain("Reduvia"), "Reduvia");
    }

    /// The installed game, if this machine has one.
    #[test]
    fn the_installed_game_names_its_own_things() {
        let Some((game, installed)) =
            crate::testing::installed(crate::games::Game::EldenRing)
        else {
            return;
        };

        let shelf = everything(&game, Some(&installed), "rusru");
        if shelf.is_empty() {
            return;
        }
        assert!(shelf.len() > 2000, "only {} things named", shelf.len());

        // The dagger the rest of this work was checked against, under the name
        // the player's own installation gives it.
        let found = look_up(&shelf, "Редувия", 3);
        assert!(!found.is_empty(), "the game does not name Редувия");
        assert_eq!(found[0].what, "weapon");
        assert!(
            found[0].caption.as_deref().is_some_and(|text| text.len() > 20),
            "no description came with it: {:?}",
            found[0].caption
        );

        // And nothing offered to the player is scaffolding or markup.
        for item in shelf.iter().take(500) {
            assert!(!item.name.contains('<'), "markup in {:?}", item.name);
            assert!(worth_naming(&item.name), "{:?} should have been dropped", item.name);
        }
    }
}
