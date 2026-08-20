//! The enemies a map file places, and which NPC row each one is.
//!
//! A boss's name is not in any param — `GameAreaParam` has its rune reward and
//! its position and no reference to the enemy at all. The name belongs to the
//! thing standing there, which lives in `map/mapstudio/*.msb.dcx`, and this
//! reads those.
//!
//! The files are DCX like everything else the game ships, so `formats::dcx`
//! opens them: 35 KB packed, 845 KB plain. A total conversion leaves all 634 of
//! them loose next to the regulation; a plain game keeps 864 of them inside
//! `Data2.bhd`, which [`super::bhd5`] opens. Either way what arrives here is
//! bytes, and this does not care which it was.
//!
//! Every offset here was read off the real bytes and each one checks itself —
//! see `assets/param-layout.md` under "The map files" for how, and for the two
//! places a plausible-looking wrong answer was available.

use std::path::Path;

/// One enemy, where it stands.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placed {
    /// What the map calls it — `c4460_9000`, a model and an instance. Useful
    /// for telling two of the same creature apart, not for showing anybody.
    pub tag: String,
    /// The `NpcParam` row it points at, which carries its name and its worth.
    pub npc: i64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// A part that is an enemy rather than scenery, a collision or the player.
const ENEMY: u32 = 2;

/// Where the block chain starts, past `"MSB "`, the version and the header size.
const FIRST_BLOCK: usize = 0x10;

/// In a part: the name, relative to the part; the type; where it stands; and
/// the table of offsets to whatever differs by type.
mod part {
    pub const NAME: usize = 0x00;
    pub const TYPE: usize = 0x0c;
    pub const X: usize = 0x20;
    pub const SLOTS: usize = 0x50;
    /// The enemy block is the fourth. The fifth looks like it and is all -1.
    pub const ENEMY_SLOT: usize = 3;
}

/// In an enemy block, relative to it.
mod enemy {
    /// Found by testing every word in the block against the installed
    /// `NpcParam`'s real rows: this one is a live row in 54 of 54 enemies,
    /// where `thinkParamId` four bytes earlier hits 53 by coincidence.
    pub const NPC: usize = 0x0c;
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// A UTF-16 string, to its first zero.
fn text_at(bytes: &[u8], at: usize) -> String {
    let mut out = String::new();
    let mut go = at;
    while let Some(pair) = bytes.get(go..go + 2) {
        let ch = u16::from_le_bytes([pair[0], pair[1]]);
        if ch == 0 {
            break;
        }
        out.push(char::from_u32(u32::from(ch)).unwrap_or('\u{fffd}'));
        go += 2;
    }
    out
}

/// Every enemy one map file places.
///
/// An unreadable file is an empty answer rather than an error: there are 634 of
/// them and one being odd should cost that map's enemies, not the feature.
pub fn enemies(path: &Path) -> Vec<Placed> {
    let Ok(packed) = std::fs::read(path) else {
        return Vec::new();
    };
    let what = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().to_string());
    enemies_in(&packed, &what)
}

/// The same, for a map handed over as bytes rather than named on disk.
///
/// Which is how it arrives out of `Data2.bhd`, where the plain game keeps its
/// maps. The bytes may be wrapped or not: the packed ones always are, and a
/// total conversion's loose ones can be either.
pub fn enemies_in(bytes: &[u8], what: &str) -> Vec<Placed> {
    if !crate::formats::dcx::wraps(bytes) {
        return parts_of(bytes);
    }
    match crate::formats::dcx::expand(bytes, what) {
        Ok(plain) => parts_of(&plain),
        Err(_) => Vec::new(),
    }
}

/// Walks the block chain to `PARTS_PARAM_ST` and reads the enemies out of it.
fn parts_of(plain: &[u8]) -> Vec<Placed> {
    if plain.get(..4) != Some(b"MSB ") {
        return Vec::new();
    }

    let mut at = FIRST_BLOCK;
    // Six blocks is the whole file; the bound is against a corrupt chain
    // pointing at itself rather than against any real map.
    for _ in 0..8 {
        let Some(count) = u32_at(plain, at + 4).map(|count| count as usize) else {
            return Vec::new();
        };
        let Some(name_at) = u64_at(plain, at + 8).map(|off| off as usize) else {
            return Vec::new();
        };
        if count == 0 {
            return Vec::new();
        }
        let is_parts = text_at(plain, name_at) == "PARTS_PARAM_ST";
        if is_parts {
            return (0..count - 1)
                .filter_map(|n| u64_at(plain, at + 0x10 + n * 8).map(|off| off as usize))
                .filter_map(|entry| one_part(plain, entry))
                .collect();
        }
        let Some(next) = u64_at(plain, at + 0x10 + (count - 1) * 8).map(|off| off as usize) else {
            return Vec::new();
        };
        if next <= at || next >= plain.len() {
            return Vec::new();
        }
        at = next;
    }
    Vec::new()
}

fn one_part(plain: &[u8], entry: usize) -> Option<Placed> {
    if u32_at(plain, entry + part::TYPE)? != ENEMY {
        return None;
    }
    // Relative to the part, not to the file. Read as absolute it lands on a
    // real string somewhere else in the map, which is the worst kind of wrong.
    let name_at = entry + u64_at(plain, entry + part::NAME)? as usize;
    let slot = u64_at(plain, entry + part::SLOTS + part::ENEMY_SLOT * 8)? as usize;
    if slot == 0 {
        return None;
    }
    let npc = i64::from(u32_at(plain, entry + slot + enemy::NPC)?);
    // Zero is "no NPC", which the player's own placeholder carries.
    if npc <= 0 {
        return None;
    }
    Some(Placed {
        tag: text_at(plain, name_at),
        npc,
        x: f32_at(plain, entry + part::X)?,
        y: f32_at(plain, entry + part::X + 4)?,
        z: f32_at(plain, entry + part::X + 8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a real map, with the figures that were checked by hand.
    ///
    /// `m60_35_44_00` is an overworld tile with 619 parts in it, of which 54
    /// are ordinary enemies. Every one of them must point at a live `NpcParam`
    /// row — that is the check that the offset is the right offset, and it is
    /// the same check that found it.
    #[test]
    fn a_map_places_enemies_that_are_all_real() {
        let Some(dir) = crate::testing::mod_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let map = dir.join("map").join("mapstudio").join("m60_35_44_00.msb.dcx");
        if !map.is_file() {
            return;
        }
        let placed = enemies(&map);
        assert!(placed.len() > 40, "only {} enemies", placed.len());

        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = crate::formats::regulation::Regulation::open(&path) else {
            return;
        };
        let Some(npcs) = regulation.table("NpcParam") else {
            return;
        };

        for one in &placed {
            assert!(npcs.has(one.npc), "{} points at npc {}, which is not a row", one.tag, one.npc);
            assert!(!one.tag.is_empty(), "an enemy with no tag at {}", one.npc);
            // The overworld is big but not unbounded; a misread position runs
            // to millions or to NaN.
            for (which, value) in [("x", one.x), ("y", one.y), ("z", one.z)] {
                assert!(value.is_finite(), "{} has {which} of {value}", one.tag);
                assert!(value.abs() < 100_000.0, "{} stands at {which} {value}", one.tag);
            }
        }
    }

    /// Every map file in the installation, for the shapes one file cannot show.
    #[test]
    fn every_map_reads_or_says_nothing() {
        let Some(dir) = crate::testing::mod_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir.join("map").join("mapstudio")) else {
            return;
        };
        let maps: Vec<_> = entries
            .flatten()
            .map(|one| one.path())
            .filter(|one| one.to_string_lossy().ends_with(".msb.dcx"))
            .collect();
        assert!(maps.len() > 100, "only {} maps", maps.len());

        let mut with_enemies = 0usize;
        let mut total = 0usize;
        for map in &maps {
            let placed = enemies(map);
            if !placed.is_empty() {
                with_enemies += 1;
            }
            total += placed.len();
            for one in &placed {
                assert!(one.npc > 0);
                assert!(one.x.is_finite() && one.y.is_finite() && one.z.is_finite());
            }
        }
        assert!(with_enemies > 50, "only {with_enemies} maps had any enemies");
        assert!(total > 1000, "only {total} enemies in the whole world");
    }
}
