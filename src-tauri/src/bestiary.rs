//! Every named thing standing in the world, and what killing it is worth.
//!
//! The maps place the enemies and the regulation says what each one is; neither
//! half is an answer on its own. This joins them, keeps only the ones the game
//! has a name for — a soldier or a rat has no `NpcName` entry, and that is a
//! real answer rather than a gap — and remembers the result, because walking
//! the world takes seconds and nothing about it changes between questions.
//!
//! It reads both kinds of installation. A total conversion leaves its maps
//! loose on disk; a plain game keeps them inside `Data2.bhd`, and that case is
//! the one that matters most, since a player who has installed nothing has no
//! other way to find out what is standing in front of them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Something with a name, where it stands, and what it gives.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dweller {
    /// As the player's own game prints it.
    pub name: String,
    /// `m60_35_44_00`, the map it stands on.
    pub map: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// What the table has, where that figure is really this one's own.
    ///
    /// `None` for the player-shaped characters — invaders, questline NPCs,
    /// anything built on the `c0000` model. Their rows all carry the same
    /// number, and one number cannot be the health of all of them: Sir Gideon
    /// Ofnir and a wandering knight are not equally hard to kill. The game
    /// scales those elsewhere and nothing here reads it, so saying nothing is
    /// the honest answer and repeating the template is not.
    pub hp: Option<u32>,
    /// Runes for killing it, out of the NPC's own row.
    pub runes: u32,
    /// What a kind of damage does to it, as a percentage of the ordinary
    /// amount, for the kinds where it is not ordinary. 60 means it shrugs most
    /// of that off; 140 means it hurts half again as much.
    ///
    /// Empty for anything that takes everything as it comes, which is most
    /// people — and unlike the health, these are genuinely each creature's own:
    /// 277 distinct patterns across the table, the commonest covering under a
    /// tenth of it.
    pub takes: Vec<(String, f32)>,
    /// What it gives, named, likeliest first. Empty where it gives nothing —
    /// which most named characters do, since a merchant is not a drop table.
    ///
    /// The odds are worked out in the tables rather than described, because a
    /// share of a total is exactly the arithmetic that goes wrong in prose.
    pub drops: Vec<(String, u8, f32)>,
}

/// Where the name and the figures live in an `NpcParam` row. Worked out from
/// the game's own field definition, with the `dummy8` padding handled — see
/// `assets/param-layout.md`, which also records what the naive walk produced.
mod npc {
    pub const NAME_TEXT: usize = 0x00c;
    pub const HP: usize = 0x024;
    pub const RUNES: usize = 0x02c;
}

/// Something a map's inhabitants give, and how readily.
///
/// Named things almost never drop anything: 2 of 537 across the whole world,
/// because a boss's reward is scripted rather than rolled. The soldiers and
/// beasts standing around them are where the 3,974 drops in the tables live,
/// and they have no names at all — so the useful question is not "what does
/// this creature give" but "what can be got here", which is this.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Haul {
    /// As the player's own game prints it.
    pub what: String,
    /// "weapon", "armour", "talisman", "item", "ash of war".
    pub kind: String,
    /// The best odds any one thing on this map gives it at.
    pub chance: f32,
    /// How many of the things standing here drop it, which is the difference
    /// between a rare roll off one creature and a steady farm off thirty.
    pub from: usize,
    /// How many to expect from clearing the map once — every source's own odds
    /// added up, not the best one multiplied.
    ///
    /// This is the answer to "what drops most often", and sorting by `chance`
    /// got that wrong: asked what falls most on a map, it named three things at
    /// 100% off one creature each and left out the feather at 62% off
    /// thirty-nine, which a player sees twenty times as often.
    pub expect: f32,
}

/// One installation's world: what is named, and what can be got where.
#[derive(Debug, Default)]
pub struct World {
    pub dwellers: Vec<Dweller>,
    /// By map. Best odds first.
    pub hauls: HashMap<String, Vec<Haul>>,
}

type Kept = HashMap<PathBuf, (Option<std::time::SystemTime>, Arc<World>)>;

/// Where the maps live, whichever kind of installation this is.
enum Maps {
    /// Left loose by a total conversion, already named and already expanded.
    Loose(Vec<PathBuf>),
    /// Inside the game's own archive, with the names swept back out of it.
    Packed(Box<crate::formats::bhd5::Archive>, Vec<String>),
}

/// Where every map is, in either kind of installation.
const UNDER: &str = "/map/mapstudio/";
const ENDING: &str = ".msb.dcx";

/// The model every human character shares, and the map names them after it.
const PLAYER_SHAPED: &str = "c0000";

/// Everything named, for one installation.
///
/// Empty only when there is genuinely nothing to read: no regulation, no game
/// text to name things in, or an installation whose maps are neither loose nor
/// in any archive that opens. That is a real answer too, and the caller must
/// say so rather than filling the silence.
pub fn everyone(
    app_data: &Path,
    game: crate::games::Game,
    game_dir: &Path,
    mod_dir: Option<&Path>,
) -> Arc<World> {
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(Kept::new()));

    let maps = mod_dir
        .map(|dir| dir.join("map").join("mapstudio"))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| game_dir.join("map").join("mapstudio"));

    // Re-read when the folder changes, so swapping a mod does not need the
    // launcher restarted to see it.
    let touched = std::fs::metadata(&maps).ok().and_then(|meta| meta.modified().ok());

    let mut held = kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((was, found)) = held.get(&maps) {
        if *was == touched {
            return Arc::clone(found);
        }
    }

    let found = Arc::new(walk(app_data, game, game_dir, mod_dir, &maps));
    held.insert(maps, (touched, Arc::clone(&found)));
    found
}

fn walk(
    app_data: &Path,
    game: crate::games::Game,
    game_dir: &Path,
    mod_dir: Option<&Path>,
    maps: &Path,
) -> World {
    let Some(regulation) = crate::formats::regulation::installed(game, game_dir, mod_dir) else {
        return World::default();
    };
    let Some(npcs) = regulation.table("NpcParam") else {
        return World::default();
    };
    // Names come from the running game when it is up and off the catalogue
    // written the last time it was, so the answer is the same either way and in
    // their own language. This used to read the process only, which meant the
    // whole bestiary vanished the moment they shut the game — and shut is how
    // somebody asking their launcher what a boss is worth usually has it.
    let named: HashMap<u32, String> =
        crate::text::names(app_data, game, Some(game_dir), mod_dir, crate::text::Kind::Npc)
            .into_iter()
            .collect();
    if named.is_empty() {
        return World::default();
    }

    // Everything the installation has a word for, so a dropped id becomes the
    // name on the player's own screen. Cached, so this costs one read.
    let language = crate::language::status(game_dir)
        .current
        .as_deref()
        .and_then(crate::language::locale_folder)
        .unwrap_or("engus");
    let shelf = crate::library::everything(game_dir, mod_dir, language);
    let call_it = |kind: &str, id: i64| -> Option<String> {
        let id = u32::try_from(id).ok()?;
        shelf
            .iter()
            .find(|one| one.what == kind && one.id == id)
            .map(|one| one.name.clone())
    };

    // No entry in the text means this one has no name, which most do not.
    // Skipping is the answer; inventing one is what this whole module replaces.
    let dweller = |map: &str, placed: &crate::formats::msb::Placed| -> Option<Dweller> {
        let text = npcs
            .i32(placed.npc, npc::NAME_TEXT)
            .and_then(|text| u32::try_from(text).ok())?;
        Some(Dweller {
            name: named.get(&text)?.clone(),
            map: map.to_string(),
            x: placed.x,
            y: placed.y,
            z: placed.z,
            // The map names a part after its model — `c0000_9001` — which is
            // the only thing that says whether the row's health is its own.
            hp: (!placed.tag.starts_with(PLAYER_SHAPED))
                .then(|| npcs.i32(placed.npc, npc::HP).unwrap_or(0).max(0) as u32),
            takes: regulation.damage_taken_by(placed.npc),
            // Only the ones there is a name for. An id on its own is not an
            // answer, and printing "item 20753" invites somebody to guess what
            // it is — which is the whole failure this launcher exists to stop.
            drops: regulation
                .drops_from(placed.npc)
                .into_iter()
                .filter_map(|one| {
                    Some((call_it(&one.kind, one.id)?, one.count, one.chance))
                })
                .collect(),
            runes: npcs.i32(placed.npc, npc::RUNES).unwrap_or(0).max(0) as u32,
        })
    };

    let mut world = World::default();
    // Everything standing on a map, named or not, folded into what can be got
    // there. Keyed on the name so two hundred soldiers dropping the same thing
    // read as one steady source rather than two hundred entries.
    let mut gather = |map: &str, placed: &[crate::formats::msb::Placed]| {
        let mut here: HashMap<(String, String), (f32, usize, f32)> = HashMap::new();
        for one in placed {
            for got in regulation.drops_from(one.npc) {
                let Some(what) = call_it(&got.kind, got.id) else {
                    continue;
                };
                let tally = here.entry((what, got.kind)).or_insert((0.0, 0, 0.0));
                tally.0 = tally.0.max(got.chance);
                tally.1 += 1;
                // Each source's own odds, times however many it gives at once.
                tally.2 += got.chance / 100.0 * f32::from(got.count);
            }
        }
        if here.is_empty() {
            return;
        }
        let mut haul: Vec<Haul> = here
            .into_iter()
            .map(|((what, kind), (chance, from, expect))| Haul { what, kind, chance, from, expect })
            .collect();
        // By how much of it a clear actually yields, which is what "drops most
        // often" means to somebody standing there. Ordering by the best single
        // roll put three one-off certainties above a feather off thirty-nine.
        haul.sort_by(|a, b| b.expect.total_cmp(&a.expect).then_with(|| a.what.cmp(&b.what)));
        world.hauls.insert(map.to_string(), haul);
    };

    match where_the_maps_are(game_dir, maps) {
        Maps::Loose(files) => {
            for file in files {
                // `m60_35_44_00.msb.dcx` — the map's own name, minus what it is
                // packed in, which is how the rest of the launcher names a map.
                let map = file
                    .file_name()
                    .map(|name| name.to_string_lossy().trim_end_matches(ENDING).to_string())
                    .unwrap_or_default();
                let placed = crate::formats::msb::enemies(&file);
                world.dwellers.extend(placed.iter().filter_map(|one| dweller(&map, one)));
                gather(&map, &placed);
            }
        }
        Maps::Packed(archive, names) => {
            for map in names {
                let Some(bytes) = archive.read(&format!("{UNDER}{map}{ENDING}")) else {
                    continue;
                };
                let placed = crate::formats::msb::enemies_in(&bytes, &map);
                world.dwellers.extend(placed.iter().filter_map(|one| dweller(&map, one)));
                gather(&map, &placed);
            }
        }
    }
    world
}

/// Loose if any are, packed otherwise.
fn where_the_maps_are(game_dir: &Path, loose: &Path) -> Maps {
    let mut files: Vec<PathBuf> = std::fs::read_dir(loose)
        .into_iter()
        .flatten()
        .flatten()
        .map(|one| one.path())
        .filter(|one| one.to_string_lossy().ends_with(ENDING))
        .collect();
    if !files.is_empty() {
        files.sort();
        return Maps::Loose(files);
    }

    // Two maps every copy of the game has, to tell which archive the maps are
    // in without opening the six that they are not in.
    let known = [
        crate::formats::bhd5::hash(&format!("{UNDER}m10_00_00_00{ENDING}")),
        crate::formats::bhd5::hash(&format!("{UNDER}m11_00_00_00{ENDING}")),
    ];
    match crate::formats::bhd5::holding(game_dir, &known) {
        Some(archive) => {
            let names = every_map_in(&archive);
            Maps::Packed(Box::new(archive), names)
        }
        None => Maps::Loose(Vec::new()),
    }
}

/// The maps an archive holds, found by asking after every name one could have.
///
/// The index keeps hashes and not names, so there is nothing in it to list.
/// Map ids have exactly one shape — `mAA_BB_CC_DD` — so the whole of that space
/// is asked about instead. It is only affordable because the fold is a left
/// fold: every candidate shares the path in front of it, so all that is new
/// work is the digits.
fn every_map_in(archive: &crate::formats::bhd5::Archive) -> Vec<String> {
    use crate::formats::bhd5::fold;

    let pairs: Vec<String> = (0..100).map(|n| format!("{n:02}")).collect();
    let head = fold(fold(0, UNDER), "m");

    let mut found = Vec::new();
    for aa in &pairs {
        let a = fold(fold(head, aa), "_");
        for bb in &pairs {
            let b = fold(fold(a, bb), "_");
            for cc in &pairs {
                let c = fold(fold(b, cc), "_");
                // The last pair is 00 on every map the game ships, but sweeping
                // a few more costs nothing and assumes less.
                for dd in &pairs[..10] {
                    if archive.holds(fold(fold(c, dd), ENDING)) {
                        found.push(format!("m{aa}_{bb}_{cc}_{dd}"));
                    }
                }
            }
        }
    }
    found
}

/// The named things on one map, richest first.
///
/// Ordered by what they are worth because that is what separates a boss from a
/// named merchant standing next to it, and there is no flag in the tables that
/// does.
pub fn on_map(world: &World, map: &str) -> Vec<Dweller> {
    let mut here: Vec<Dweller> =
        world.dwellers.iter().filter(|one| one.map == map).cloned().collect();
    here.sort_by(|a, b| b.runes.cmp(&a.runes).then_with(|| a.name.cmp(&b.name)));
    here.dedup_by(|a, b| a.name == b.name);
    here
}

/// What everything standing on one map gives, best odds first.
pub fn haul_on<'a>(world: &'a World, map: &str) -> &'a [Haul] {
    world.hauls.get(map).map_or(&[], Vec::as_slice)
}

/// Whether a word is one of the game's map ids rather than the name of
/// something.
///
/// `m60_35_44_00`. A player says "Тень Погибели" and a model says "Weeping
/// Peninsula"; neither is a map id, and the launcher could only be asked by id.
/// Told to find a boss it had no id for, a model went to a wiki and answered
/// about a different creature entirely.
pub fn is_a_map(word: &str) -> bool {
    let word = word.trim();
    word.len() >= 3
        && word.starts_with('m')
        && word[1..].starts_with(|c: char| c.is_ascii_digit())
}

/// Everything the world has a name for that matches these words, richest
/// first.
///
/// Matched loosely and on either side, because the player types what their
/// screen says and a model types what a wiki says.
pub fn called<'a>(world: &'a World, wanted: &str) -> Vec<&'a Dweller> {
    let looking = wanted.trim().to_lowercase();
    if looking.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<&Dweller> = world
        .dwellers
        .iter()
        .filter(|one| {
            let name = one.name.to_lowercase();
            name.contains(&looking) || looking.contains(&name)
        })
        .collect();
    found.sort_by(|a, b| b.runes.cmp(&a.runes).then_with(|| a.name.cmp(&b.name)));
    found.dedup_by(|a, b| a.name == b.name && a.map == b.map);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the names written down last time the game was open are kept.
    fn data() -> PathBuf {
        crate::testing::app_data().unwrap_or_default()
    }

    /// Reward rows joined to the nearest same-map name, to test if boss rewards
    /// can be named. Noisy: 75/186 unmatched, matches out to a kilometre.
    ///
    /// `cargo test --lib show_boss_rewards -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_boss_rewards() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let Some(regulation) =
            crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())
        else {
            return;
        };
        let bosses = regulation.bosses();
        let world = everyone(&data(), game, &game_dir, mod_dir.as_deref());
        println!("{} reward rows, {} dwellers", bosses.len(), world.dwellers.len());

        let mut joined: Vec<(u32, String, f32, String)> = Vec::new();
        let mut no_map = 0;
        for boss in &bosses {
            let nearest = world
                .dwellers
                .iter()
                .filter(|d| d.map == boss.map)
                .map(|d| {
                    let dist = ((d.x - boss.x).powi(2)
                        + (d.y - boss.y).powi(2)
                        + (d.z - boss.z).powi(2))
                    .sqrt();
                    (dist, d)
                })
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            match nearest {
                Some((dist, d)) => joined.push((boss.runes, d.name.clone(), dist, boss.map.clone())),
                None => no_map += 1,
            }
        }
        joined.sort_by_key(|(runes, ..)| std::cmp::Reverse(*runes));
        println!("{no_map} reward rows had no dweller on their map");
        println!("richest 25 by reward, nearest same-map name and its distance:");
        for (runes, name, dist, map) in joined.iter().take(25) {
            println!("  {runes:>8} runes  ~{dist:>8.0}u  {name}  [{map}]");
        }
    }

    /// Who the launcher would name, for reading rather than asserting.
    ///
    /// `cargo test --lib show_the_world -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_the_world() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let world = everyone(&data(), game, &game_dir, mod_dir.as_deref());
        let all = &world.dwellers;
        println!("{} named things across the world", all.len());

        let mut by_map: std::collections::BTreeMap<&str, usize> = Default::default();
        for one in all.iter() {
            *by_map.entry(one.map.as_str()).or_default() += 1;
        }
        println!("{} maps have one", by_map.len());

        let mut richest: Vec<&Dweller> = all.iter().collect();
        richest.sort_by_key(|one| std::cmp::Reverse(one.runes));
        richest.dedup_by(|a, b| a.name == b.name);
        println!("  the ten richest:");
        for one in richest.iter().take(10) {
            println!("    {:>8} runes, {:>12} — {} on {}", one.runes, said(one.hp), one.name, one.map);
            if !one.takes.is_empty() {
                let parts: Vec<String> =
                    one.takes.iter().map(|(k, pc)| format!("{k} {pc:.0}%")).collect();
                println!("            takes {}", parts.join(", "));
            }
            for (what, count, chance) in one.drops.iter().take(4) {
                println!("            drops {what} ×{count} at {chance:.1}%");
            }
        }
        let giving = all.iter().filter(|one| !one.drops.is_empty()).count();
        println!("  {giving} of {} give something named", all.len());

        for map in ["m60_35_44_00", "m11_10_00_00"] {
            let haul = haul_on(&world, map);
            println!("  what {map} gives, {} kinds:", haul.len());
            for one in haul.iter().take(8) {
                println!(
                    "    {:<40} {:>6.1} per clear   best {:>5.1}% off {}",
                    one.what, one.expect, one.chance, one.from
                );
            }
        }
        for map in ["m60_35_44_00", "m60_34_43_00"] {
            let here = on_map(&world, map);
            println!("  on {map}: {}", here.len());
            for one in here.iter().take(5) {
                println!("    {} — {} runes", one.name, one.runes);
            }
        }
    }

    /// The world a player who has installed nothing sees.
    ///
    /// Asking with no mod directory is exactly that player's path: no loose
    /// maps anywhere, so everything has to come out of the game's own archive.
    /// It used to return nothing at all, and nothing is what this is here to
    /// stop coming back.
    #[test]
    #[ignore = "reads eleven gigabytes of archive; run it deliberately"]
    fn the_packed_world_reads_too() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        crate::formats::oodle::register(&game_dir);
        if !crate::formats::oodle::available() {
            return;
        }

        let began = std::time::Instant::now();
        let world = everyone(&data(), game, &game_dir, None);
        let all = &world.dwellers;
        println!("{} named things in {:.1}s", all.len(), began.elapsed().as_secs_f32());
        assert!(all.len() > 50, "the plain game gave back {}", all.len());

        let mut maps: std::collections::BTreeSet<&str> = Default::default();
        for one in all.iter() {
            assert!(!one.name.trim().is_empty(), "a nameless entry on {}", one.map);
            assert!(one.runes < 1_000_000, "{} is worth {} runes", one.name, one.runes);
            maps.insert(one.map.as_str());
        }
        println!("across {} maps", maps.len());

        let mut richest: Vec<&Dweller> = all.iter().collect();
        richest.sort_by_key(|one| std::cmp::Reverse(one.runes));
        richest.dedup_by(|a, b| a.name == b.name);
        for one in richest.iter().take(8) {
            println!("  {:>8} runes, {:>12} — {} on {}", one.runes, said(one.hp), one.name, one.map);
        }
    }

    /// A health figure, or the fact that there is not one.
    fn said(hp: Option<u32>) -> String {
        hp.map_or_else(|| "no hp given".into(), |hp| format!("{hp} hp"))
    }

    /// The template figure must not go out as anybody's health.
    ///
    /// Every human character's row carries the same number, so quoting it named
    /// Sir Gideon Ofnir and a wandering knight as equally hard to kill. Both
    /// halves matter: the human ones have to go quiet, and everything else has
    /// to keep answering, or the fix is just a feature removed.
    #[test]
    fn the_template_health_is_not_passed_off_as_anybody_s() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let world = everyone(&data(), game, &game_dir, mod_dir.as_deref());
        let all = &world.dwellers;
        if all.is_empty() {
            return;
        }

        let quiet = all.iter().filter(|one| one.hp.is_none()).count();
        let told = all.len() - quiet;
        assert!(
            quiet > 0,
            "all {} reported health, so the shared template is still being passed off",
            all.len()
        );
        assert!(told > 0, "none of {} reported health at all", all.len());
        for one in all.iter() {
            if let Some(hp) = one.hp {
                assert!(hp < 1_000_000, "{} is said to have {hp} health", one.name);
            }
        }
    }

    /// What a map gives, over every map that gives anything.
    ///
    /// The shape is what can be checked, and each part of it fails on its own:
    /// a name that is a name rather than an id, odds that are odds, and a count
    /// of sources that cannot exceed what is standing there. The figure worth
    /// guarding is the 100% — a guaranteed drop is real (thirty-one poison
    /// flowers each give their flower) and a 100% on everything would mean the
    /// weights were being read as the total.
    #[test]
    fn what_a_map_gives_is_named_and_at_real_odds() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let world = everyone(&data(), game, &game_dir, mod_dir.as_deref());
        if world.dwellers.is_empty() {
            return;
        }
        assert!(!world.hauls.is_empty(), "not one map gives anything");

        let mut certain = 0usize;
        let mut total = 0usize;
        for (map, haul) in &world.hauls {
            assert!(!haul.is_empty(), "{map} has an empty haul rather than none");
            let mut best = f32::INFINITY;
            let mut most = f32::INFINITY;
            for one in haul {
                assert!(!one.what.trim().is_empty(), "something unnamed on {map}");
                // An id that never found a name would come through as digits.
                assert!(
                    one.what.chars().any(|c| c.is_alphabetic()),
                    "{map} gives {:?}, which is not a name",
                    one.what
                );
                assert!(
                    one.chance > 0.0 && one.chance <= 100.0,
                    "{} on {map} at {}%",
                    one.what,
                    one.chance
                );
                assert!(one.from > 0, "{} on {map} comes off nothing", one.what);
                // Most plentiful first, because that is what "drops most" means
                // to somebody standing there. Ordering by the best single roll
                // put three one-off certainties above a feather off thirty-nine.
                assert!(one.expect > 0.0, "{} on {map} yields nothing per clear", one.what);
                assert!(one.expect <= most + 0.01, "{map} is out of order at {}", one.what);
                most = one.expect;
                // A clear cannot yield more of a thing than there are things
                // giving it, and cannot yield less than the best single roll.
                assert!(
                    one.expect <= one.from as f32 * 99.0,
                    "{} on {map} yields {} from {}",
                    one.what,
                    one.expect,
                    one.from
                );
                assert!(
                    one.expect >= one.chance / 100.0 - 0.01,
                    "{} on {map} yields {} but rolls at {}%",
                    one.what,
                    one.expect,
                    one.chance
                );
                best = best.min(one.chance);
                total += 1;
                certain += usize::from(one.chance >= 99.9);
            }
        }
        assert!(total > 50, "only {total} things can be got in the whole world");
        // Guaranteed drops exist and are the minority. Both halves matter: none
        // would mean the weights are misread, all would mean the share is.
        assert!(certain > 0, "nothing anywhere is a certain drop");
        assert!(certain * 2 < total, "{certain} of {total} drops are certainties");
    }

    /// Finding something by its name rather than by the map it stands on.
    ///
    /// A player names a boss and a model names a region; neither is a map id,
    /// and until this the launcher could only be asked by id. Told to find one
    /// it had no id for, a model went to a wiki and answered about a different
    /// creature entirely.
    #[test]
    fn a_thing_can_be_found_by_what_it_is_called() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let world = everyone(&data(), game, &game_dir, mod_dir.as_deref());
        if world.dwellers.is_empty() {
            return;
        }

        // A map id is a map id and must never be taken for a name.
        assert!(is_a_map("m60_35_44_00"));
        assert!(is_a_map("m10_00_00_00"));
        for word in ["Тень Погибели", "Weeping Peninsula", "mimic", "", "m", "malenia"] {
            assert!(!is_a_map(word), "{word:?} was taken for a map id");
        }

        // Whatever the richest thing in the world is called, asking for it by
        // name finds it — and says where it stands.
        let mut richest: Vec<&Dweller> = world.dwellers.iter().collect();
        richest.sort_by_key(|one| std::cmp::Reverse(one.runes));
        let Some(known) = richest.first() else {
            return;
        };
        let found = called(&world, &known.name);
        assert!(!found.is_empty(), "{} could not be found by its own name", known.name);
        assert!(found.iter().any(|one| one.map == known.map), "found it nowhere it stands");

        // Part of a name is enough, and matching is not case-bound.
        let part: String = known.name.chars().take(5).collect();
        assert!(!called(&world, &part).is_empty(), "{part:?} matched nothing");
        assert!(!called(&world, &known.name.to_uppercase()).is_empty(), "case defeated it");

        // And nothing is not something.
        assert!(called(&world, "").is_empty());
        assert!(called(&world, "   ").is_empty());
    }

    /// The join, over the whole world.
    ///
    /// What can be asserted is the shape: every name is a real string, every
    /// rune figure is a plausible reward, and there are enough of them that the
    /// walk found the maps rather than an empty folder. The one figure checked
    /// by hand is in `param-layout.md` — `c0000_9001` in `m60_35_44_00` comes
    /// out as Белоликий Варрэ, worth 500.
    #[test]
    fn the_world_has_named_things_in_it() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let world = everyone(&data(), game, &game_dir, mod_dir.as_deref());
        let all = &world.dwellers;
        if all.is_empty() {
            // No loose maps and no running game to read names from. Nothing to
            // check, and saying nothing is the point of the empty case.
            return;
        }

        assert!(all.len() > 50, "only {} named things in the world", all.len());
        for one in all.iter() {
            assert!(!one.name.trim().is_empty(), "a nameless entry on {}", one.map);
            assert!(!one.map.is_empty(), "{} stands nowhere", one.name);
            assert!(one.x.is_finite() && one.y.is_finite() && one.z.is_finite());
            // Half a million is Elden Beast money; past that is a misread.
            assert!(one.runes < 1_000_000, "{} is worth {} runes", one.name, one.runes);
        }

        // And the second call is the cache rather than another eight seconds.
        let again = everyone(&data(), game, &game_dir, mod_dir.as_deref());
        assert_eq!(all.len(), again.dwellers.len());
    }
}
