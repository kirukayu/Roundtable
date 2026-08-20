//! Named places, and where they are on the map.
//!
//! The join that makes "put a marker on Stormveil" a pair of numbers:
//!
//! - `WorldMapPointParam`, out of the installed `regulation.bin`, holds every
//!   point the map draws — a grid square, a position within it, and up to eight
//!   text ids.
//! - `PlaceName`, out of the installed message archives, holds what those ids
//!   say, in whichever language the player installed.
//! - [`crate::markers::from_world`] turns the position into map coordinates.
//!
//! All of it off the disk, so it works with the game closed — which it has to
//! be, because that is when a marker can be written.
//!
//! A total conversion replaces both halves, so a place the mod renamed comes
//! back under its new name and a place it moved comes back where it moved to.

use std::path::Path;

use serde::Serialize;

use crate::formats::{fmg, regulation};

/// Field offsets in `WorldMapPointParam`, from the paramdef's field order and
/// the way a `dummy8` with a bit size shares a byte with what came before it.
mod at {
    pub const AREA: usize = 0x20;
    pub const GRID_X: usize = 0x21;
    pub const GRID_Z: usize = 0x22;
    pub const POS_X: usize = 0x24;
    pub const POS_Z: usize = 0x2c;
    /// Eight of them, four bytes apart with two flag words between each.
    pub const TEXT: [usize; 8] = [0x30, 0x3c, 0x48, 0x54, 0x60, 0x6c, 0x78, 0x84];
}

/// The overworld. The underground and the DLC are drawn on their own maps with
/// their own coordinates, and a marker written from one onto the other lands
/// nowhere sensible.
const OVERWORLD: u8 = 60;

/// One place the map knows the name of.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub name: String,
    /// Where a marker goes, already in the save's coordinates.
    pub map_x: f32,
    pub map_y: f32,
}

/// Which installation is being asked about, and what may be kept.
///
/// A struct rather than five arguments because the last one is easy to get
/// wrong and the wrong one is silent: without somewhere to keep it, a table
/// read out of the running game is thrown away and the player is told there are
/// no places the moment they quit.
pub struct Where<'a> {
    pub game: crate::games::Game,
    /// The folder the game runs out of.
    pub game_dir: &'a Path,
    /// A total conversion's folder, when one is installed.
    pub mod_dir: Option<&'a Path>,
    /// The message folder the game reads, e.g. `rusru`.
    pub language: &'a str,
    /// Where a table read out of the running game may be written down.
    pub keep_in: Option<&'a Path>,
}

/// Where a table read out of the running game is kept.
///
/// Named after the installation and language it came from, so a second game or
/// a second language does not overwrite the first, and so a moved installation
/// simply reads nothing rather than reading somebody else's.
fn remembered_at(at: &Where<'_>) -> Option<std::path::PathBuf> {
    let root = at.mod_dir.unwrap_or(at.game_dir);
    let mut tag = String::new();
    for ch in root.to_string_lossy().chars() {
        tag.push(if ch.is_ascii_alphanumeric() { ch } else { '-' });
    }
    // Long paths make long names; the tail is the part that differs.
    let tail: String = tag.chars().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
    Some(
        at.keep_in?
            .join("places")
            .join(format!("{tail}.{}.json", at.language)),
    )
}

fn remembered_names(at: &Where<'_>) -> Option<fmg::Strings> {
    let path = remembered_at(at)?;
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn remember_names(at: &Where<'_>, names: &fmg::Strings) {
    let Some(path) = remembered_at(at) else {
        return;
    };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    if let Ok(bytes) = serde_json::to_vec(names) {
        let _ = std::fs::write(path, bytes);
    }
}

/// Every named place on the overworld map.
///
/// Empty rather than an error when the pieces are not all there: a vanilla
/// install keeps its text inside packed archives this does not open, and that
/// is a reason to fall back on the running game rather than to fail.
pub fn everywhere(at: Where<'_>) -> Vec<Place> {
    use std::sync::{Mutex, OnceLock};

    let (game_dir, mod_dir, language) = (at.game_dir, at.mod_dir, at.language);

    // Building this means decrypting the regulation and expanding six megabytes
    // of message archive. That is a second the first time and nothing after,
    // but only if it is remembered — and it is asked for on every question that
    // mentions a place.
    type Key = (std::path::PathBuf, String, Option<std::time::SystemTime>);
    type Kept = std::collections::HashMap<Key, std::sync::Arc<Vec<Place>>>;
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(Kept::new()));

    // The tables' own timestamp is part of the key, so installing a mod or
    // patching one is a different answer rather than a stale one.
    let root = mod_dir.unwrap_or(game_dir);
    let stamp = std::fs::metadata(root.join("regulation.bin"))
        .and_then(|meta| meta.modified())
        .ok();
    let key = (root.to_path_buf(), language.to_string(), stamp);
    if let Ok(kept) = kept.lock() {
        if let Some(had) = kept.get(&key) {
            return had.as_ref().clone();
        }
    }

    let found = gather(&at);
    if let Ok(mut kept) = kept.lock() {
        kept.insert(key, std::sync::Arc::new(found.clone()));
    }
    found
}

/// The work behind [`everywhere`], done once per installation and language.
fn gather(at: &Where<'_>) -> Vec<Place> {
    crate::formats::oodle::register(at.game_dir);

    let Some(regulation) = regulation::installed(at.game, at.game_dir, at.mod_dir) else {
        return Vec::new();
    };
    let Some(points) = regulation.table("WorldMapPointParam") else {
        return Vec::new();
    };
    let (game_dir, mod_dir) = (at.game_dir, at.mod_dir);

    // Off the disk first, since that is the copy the game will load. Failing
    // that, whatever was written down last time. Failing that, the game itself
    // if it happens to be running — and then written down, so the next time it
    // does not have to be.
    let mut names = place_names(game_dir, mod_dir, at.language);
    if names.is_empty() {
        names = remembered_names(at).unwrap_or_default();
    }
    if names.is_empty() {
        if let Some(read) = crate::text::every_name(at.game, crate::text::Kind::Place) {
            names = read.into_iter().collect();
            remember_names(at, &names);
        }
    }
    if names.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<Place> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for id in points.ids() {
        if points.u8(id, at::AREA) != Some(OVERWORLD) {
            continue;
        }
        let (Some(grid_x), Some(grid_z), Some(x), Some(z)) = (
            points.u8(id, at::GRID_X),
            points.u8(id, at::GRID_Z),
            points.f32(id, at::POS_X),
            points.f32(id, at::POS_Z),
        ) else {
            continue;
        };
        let (map_x, map_y) = crate::markers::from_world(grid_x, grid_z, x, z);

        for slot in at::TEXT {
            let Some(text) = points.i32(id, slot) else {
                continue;
            };
            if text <= 0 {
                continue;
            }
            let Ok(text) = u32::try_from(text) else {
                continue;
            };
            let Some(name) = names.get(&text).map(|raw| plain(raw)) else {
                continue;
            };
            if !worth_naming(&name) {
                continue;
            }
            // One place, one entry: the same name turns up on several points
            // where a region has more than one icon.
            if seen.insert((name.clone(), map_x.round() as i32, map_y.round() as i32)) {
                out.push(Place { name, map_x, map_y });
            }
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The `PlaceName` tables out of the installed message archives.
/// The map labels, off the disk.
///
/// This used to walk `msg/<language>` itself, which found a mod's loose text
/// and the game's loose text and nothing else. An untouched installation keeps
/// its own inside `Data0.bhd`, so with the game shut the whole map went
/// nameless: the moment the player's game crashed, `places` started answering
/// with nothing at all, and every question about where something is went with
/// it.
///
/// `library::tables_for` looks in all three places and already applies the rule
/// this needed — a mod that merges the DLC ships `PlaceName_dlc02` holding the
/// same ids as the base table but in English, even inside a translated folder,
/// so the base table must land first and the first name for an id wins. It
/// sorts the table names by length and inserts without overwriting, which is
/// exactly that.
fn place_names(game_dir: &Path, mod_dir: Option<&Path>, language: &str) -> fmg::Strings {
    crate::library::tables_for(game_dir, mod_dir, language)
        .get("PlaceName")
        .cloned()
        .unwrap_or_default()
}

/// The game colours some of its map labels, so the words arrive wrapped in
/// markup. The player is shown the words.
fn plain(raw: &str) -> String {
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
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a label names a place at all.
///
/// Several map points carry a state rather than a name — a defeated one reads
/// "Dead" — and a list of two hundred identical entries is worse than a shorter
/// list of real ones.
fn worth_naming(name: &str) -> bool {
    if name.chars().count() < 3 {
        return false;
    }
    !matches!(
        name.to_lowercase().as_str(),
        "dead" | "?" | "???" | "%null%" | "dummytext" | "(dummytext)"
    )
}

/// The place whose name best matches what was asked for.
///
/// Exact first, then a whole-word match, then anything containing it — so
/// "Stormveil" finds "Stormveil Castle" without "Castle" finding every castle
/// in the Lands Between ahead of it.
/// What the game itself calls the nearest place to a point, in the language it
/// is played in.
///
/// The launcher carries its own survey of the overworld — three hundred and
/// eighty lines of somebody's careful work — and it is in English whoever is
/// playing. Asked in Russian where they were standing, an answer named "Weeping
/// Peninsula - Castle Morne Rampart" to somebody whose armour and items came
/// back in Russian in the same breath, which reads as the launcher not knowing
/// their language rather than as a label it cannot translate.
///
/// The game names its own places and those names carry map coordinates, so the
/// nearest one is a name off their own map. `None` when nothing is close: a
/// point in open country belongs to its region and not to a landmark two
/// hundred metres away, and inventing an association is worse than the English.
///
/// `map` is the game's own id for the square, `m60_35_44_00`, whose two middle
/// numbers are the overworld grid.
pub fn nearest_named(
    game_dir: &Path,
    mod_dir: Option<&Path>,
    language: &str,
    map: &str,
    x: f32,
    z: f32,
) -> Option<String> {
    // Only the overworld grid: a legacy dungeon has no tile and its coordinates
    // are its own, so converting them would land somewhere in a field.
    let parts: Vec<&str> = map.trim_start_matches('m').split('_').collect();
    if parts.len() != 4 || parts[0] != "60" {
        return None;
    }
    let grid_x: u8 = parts[1].parse().ok()?;
    let grid_z: u8 = parts[2].parse().ok()?;
    let (map_x, map_y) = crate::markers::from_world(grid_x, grid_z, x, z);

    let places = everywhere(Where { game: crate::games::Game::EldenRing, game_dir, mod_dir, language, keep_in: None });

    // Close enough to be where they are. The overworld is about ten thousand
    // units across and a named point covers a good deal of ground, so this is
    // generous — but not so generous that a name from the next region wins.
    const NEAR: f32 = 300.0;
    places
        .iter()
        .map(|place| {
            let (dx, dy) = (place.map_x - map_x, place.map_y - map_y);
            (dx.hypot(dy), place)
        })
        .filter(|(away, _)| *away < NEAR)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, place)| place.name.clone())
}

pub fn find<'a>(places: &'a [Place], wanted: &str) -> Option<&'a Place> {
    let wanted = wanted.trim().to_lowercase();
    if wanted.is_empty() {
        return None;
    }

    let mut best: Option<(u8, usize, &Place)> = None;
    for place in places {
        let name = place.name.to_lowercase();
        let rank = if name == wanted {
            0
        } else if name.split_whitespace().any(|word| word == wanted) {
            1
        } else if name.starts_with(&wanted) {
            2
        } else if name.contains(&wanted) {
            3
        } else {
            continue;
        };
        // Among equals the shorter name is the more specific one.
        let better = match best {
            None => true,
            Some((had, length, _)) => (rank, name.len()) < (had, length),
        };
        if better {
            best = Some((rank, name.len(), place));
        }
    }
    best.map(|(_, _, place)| place)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some() -> Vec<Place> {
        ["Stormveil Castle", "Castle Morne", "Stormhill", "Castle Ensis"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Place {
                name: name.to_string(),
                map_x: index as f32 * 100.0,
                map_y: index as f32 * 50.0,
            })
            .collect()
    }

    #[test]
    fn a_whole_word_beats_a_word_inside_another() {
        let places = some();
        // "Stormveil" is a word in one name and part of nothing else.
        assert_eq!(find(&places, "Stormveil").unwrap().name, "Stormveil Castle");
        // "Storm" is inside two, and the shorter one wins.
        assert_eq!(find(&places, "Storm").unwrap().name, "Stormhill");
    }

    #[test]
    fn an_exact_name_wins_over_a_shorter_one_containing_it() {
        let places = some();
        assert_eq!(find(&places, "Castle Morne").unwrap().name, "Castle Morne");
        assert_eq!(find(&places, "castle morne").unwrap().name, "Castle Morne");
    }

    /// The whole point of writing the names down.
    ///
    /// A clean installation keeps its text inside archives nothing here opens,
    /// so the names can only be read while the game is up — and a marker can
    /// only be written while it is down. If what was read is not kept, the two
    /// never meet and the feature does not exist for anyone without a mod.
    #[test]
    fn a_table_read_from_the_game_survives_the_game_closing() {
        let dir = std::env::temp_dir().join("roundtable-places-kept");
        std::fs::remove_dir_all(&dir).ok();

        let at = Where {
            game: crate::games::Game::EldenRing,
            game_dir: Path::new("Z:\\no game here"),
            mod_dir: None,
            language: "engus",
            keep_in: Some(&dir),
        };
        assert!(remembered_names(&at).is_none(), "nothing has been kept yet");

        let mut names = fmg::Strings::new();
        names.insert(10_000, "Stormveil Castle".to_string());
        names.insert(10_010, "Церковь Элле".to_string());
        remember_names(&at, &names);

        let back = remembered_names(&at).expect("what was kept comes back");
        assert_eq!(back.get(&10_000).map(String::as_str), Some("Stormveil Castle"));
        assert_eq!(back.get(&10_010).map(String::as_str), Some("Церковь Элле"));

        // A different language is a different table, not the same one twice.
        let other = Where { language: "rusru", ..at };
        assert!(remembered_names(&other).is_none());

        // And with nowhere to keep it, nothing is written and nothing breaks.
        let nowhere = Where { keep_in: None, ..at };
        remember_names(&nowhere, &names);
        assert!(remembered_names(&nowhere).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nothing_matches_nothing() {
        let places = some();
        assert!(find(&places, "").is_none());
        assert!(find(&places, "   ").is_none());
        assert!(find(&places, "Anor Londo").is_none());
    }

    /// The installed game, if this machine has one.
    #[test]
    fn the_installed_game_gives_places_where_the_map_is() {
        let Some((game, installed)) =
            crate::testing::installed(crate::games::Game::EldenRing)
        else {
            return;
        };

        for language in ["engus", "rusru"] {
            let places = everywhere(Where {
                game: crate::games::Game::EldenRing,
                game_dir: &game,
                mod_dir: Some(&installed),
                language,
                keep_in: None,
            });
            if places.is_empty() {
                continue;
            }
            assert!(places.len() > 50, "only {} places", places.len());

            // Every one has to be somewhere a marker could go. The map runs
            // roughly nine thousand units each way; anything far outside means
            // the corner or the grid arithmetic is wrong.
            for place in &places {
                assert!(
                    (-2000.0..12000.0).contains(&place.map_x)
                        && (-2000.0..12000.0).contains(&place.map_y),
                    "{} is at ({}, {})",
                    place.name,
                    place.map_x,
                    place.map_y
                );
                assert!(!place.name.trim().is_empty());
            }

            // One place everybody knows, under the name that language calls it
            // — and at the same spot in both, which is what says the names
            // were joined onto the right points rather than merely found.
            let wanted = if language == "rusru" {
                "Храм Элле"
            } else {
                "Church of Elleh"
            };
            let found = find(&places, wanted)
                .unwrap_or_else(|| panic!("{language} has no {wanted}: {:?}", &places[..5]));
            assert!((found.map_x - 3031.0).abs() < 2.0, "{}", found.map_x);
            assert!((found.map_y - 7345.0).abs() < 2.0, "{}", found.map_y);

            // A translated install must come back translated. The trap: a mod
            // that merges the DLC ships an untranslated table with the same
            // ids, and letting it win turns the whole map back to English.
            if language == "rusru" {
                assert!(
                    found.name.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)),
                    "the Russian install answered {:?}",
                    found.name
                );
            }

            // And no markup reaches the player.
            assert!(
                !places.iter().any(|place| place.name.contains('<')),
                "markup survived into a name"
            );
        }
    }
}
