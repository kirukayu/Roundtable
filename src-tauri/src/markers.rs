//! The pins the player puts on the world map, read and written in the save.
//!
//! Markers live in `CSMenuMarkersSaveData`. In memory the object keeps its
//! capacity at `+0x10`, how many are in use at `+0x40` and a counter at `+0x48`
//! that hands out ids; in the save all that survives is the array itself, one
//! hundred and ten records of sixteen bytes.
//!
//! ```text
//! i32 id      the game's own counter, or negative for a slot in use by nobody
//! f32 x       map coordinates, not world ones
//! f32 y
//! u8  icon    which pin; 0 is the plain one
//! u8  one     always 1, in used and free records alike
//! u16 --      never seen as anything but zero
//! ```
//!
//! Everything here was taken from the game rather than guessed at. The add
//! function walks the array sixteen bytes at a time looking for the first `id`
//! that is negative, writes the record there and gives it `++counter` as its
//! id. The remove function writes back exactly `ff ff ff ff` then zeros then
//! `00 01 00 00`, which is the free record this module writes too.
//!
//! Written while the game is closed. The launcher does not touch the running
//! process for this: a save is a file, and a file is something a save manager
//! is allowed to edit.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// One pin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    /// Which pin the map draws. Zero is the plain one, which is what the player
    /// gets by clicking.
    pub icon: u8,
}

/// Sixteen bytes each.
const RECORD: usize = 16;

/// How many the array holds. Measured in a real save rather than assumed, and
/// checked again every time the array is located.
pub const MOST: usize = 110;

/// The sixteen bytes that sit immediately before the array.
///
/// The two slots of one save had their arrays at different offsets, so there is
/// no constant to hardcode — the array has to be found. This is the last row of
/// the table in front of it, and it occurs exactly once per character.
const ANCHOR: [u8; RECORD] = [
    0x00, 0x0b, 0x00, 0x00, 0xfb, 0x0b, 0x00, 0x00, 0xfc, 0x0b, 0x00, 0x00, 0xfd, 0x0b, 0x00, 0x00,
];

/// What the game writes into a slot it is finished with.
const FREE: [u8; RECORD] = [
    0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x01, 0x00, 0x00,
];

/// The word just past the last record.
const TERMINATOR: i32 = -2;

// ---------------------------------------------------------------------------
// World to map
// ---------------------------------------------------------------------------

/// How wide one square of the overworld grid is, in world units.
pub const TILE: f32 = 256.0;

/// The corner the map measures from, as a grid square.
///
/// Solved rather than fitted. `WorldMapPlaceNameParam` says which map piece each
/// named place belongs to and where it is in the world, and
/// `WorldMapPieceParam` gives every piece as a rectangle in the coordinates a
/// marker is stored in — so each place must land inside its own rectangle. Four
/// such pairs leave one answer: a scale of exactly one, and this corner. The
/// grid square either side of it fails the pairs outright, and 383 other map
/// points, none of which were used to find it, all land on the map.
const ORIGIN_X: f32 = 30.0 * TILE;
const ORIGIN_Z: f32 = 65.0 * TILE;

/// Where a place in the world falls on the world map.
///
/// Takes the position the way the game's own tables hold it: the grid square,
/// then the position within it. A map coordinate is a world coordinate with the
/// corner moved and the second axis reversed — the map runs south as y grows,
/// where the world runs north as z grows.
pub fn from_world(grid_x: u8, grid_z: u8, x: f32, z: f32) -> (f32, f32) {
    let global_x = f32::from(grid_x) * TILE + x;
    let global_z = f32::from(grid_z) * TILE + z;
    from_global(global_x, global_z)
}

/// The same, for a position already added up.
pub fn from_global(global_x: f32, global_z: f32) -> (f32, f32) {
    (global_x - ORIGIN_X, ORIGIN_Z - global_z)
}

/// And back, for saying where a pin the player planted actually is.
pub fn to_global(map_x: f32, map_y: f32) -> (f32, f32) {
    (map_x + ORIGIN_X, ORIGIN_Z - map_y)
}

/// The overworld square a point on the map falls in, packed the way the game
/// names its maps: `m60_44_33_00` is `0x3c2c2100`.
///
/// With this a marker can be given the name of the region it is in, which is
/// what a player recognises — "in the Weeping Peninsula" says more than any
/// pair of coordinates does.
pub fn map_id(map_x: f32, map_y: f32) -> Option<u32> {
    let (global_x, global_z) = to_global(map_x, map_y);
    let square = |value: f32| -> Option<u32> {
        let tile = (value / TILE).floor();
        (0.0..256.0).contains(&tile).then_some(tile as u32)
    };
    Some(60 << 24 | square(global_x)? << 16 | square(global_z)? << 8)
}

/// Where one character's marker array begins.
///
/// Returns `None` for a slot with no character in it, which has no array to
/// find and is not an error — the launcher shows ten slots and most are empty.
fn locate(slot: &[u8]) -> Option<usize> {
    let at = slot
        .windows(RECORD)
        .position(|window| window == ANCHOR)?
        + RECORD;

    // Refuse anything that does not end where an array of this size ends. A
    // wrong offset that happens to parse would write markers into whatever
    // structure really lives there, and that is a corrupted character.
    let end = at + MOST * RECORD;
    if end + 4 > slot.len() || word(slot, end) != TERMINATOR {
        return None;
    }
    Some(at)
}

/// Every marker the character has planted.
pub fn read(slot: &[u8]) -> Vec<Marker> {
    let Some(at) = locate(slot) else {
        return Vec::new();
    };

    (0..MOST)
        .filter_map(|index| {
            let base = at + index * RECORD;
            let id = word(slot, base);
            if id < 0 {
                return None;
            }
            Some(Marker {
                id,
                x: float(slot, base + 4),
                y: float(slot, base + 8),
                icon: slot[base + 12],
            })
        })
        .collect()
}

/// Plants one, and gives back the marker as the game will read it.
///
/// The id continues the character's own numbering. The game recomputes its
/// counter from what it loads, so the next pin the player plants by hand
/// follows this one rather than colliding with it.
pub fn place(slot: &mut [u8], x: f32, y: f32, icon: u8) -> Result<Marker> {
    let at = locate(slot)
        .ok_or_else(|| Error::msg("this save slot has no map to mark".to_string()))?;

    let mut free = None;
    let mut highest = 0i32;
    for index in 0..MOST {
        let id = word(slot, at + index * RECORD);
        if id < 0 {
            free.get_or_insert(index);
        } else {
            highest = highest.max(id);
        }
    }

    let Some(index) = free else {
        return Err(Error::msg(format!(
            "the map already holds {MOST} markers, which is all it can hold"
        )));
    };

    let marker = Marker {
        id: highest + 1,
        x,
        y,
        icon,
    };
    let base = at + index * RECORD;
    slot[base..base + 4].copy_from_slice(&marker.id.to_le_bytes());
    slot[base + 4..base + 8].copy_from_slice(&x.to_le_bytes());
    slot[base + 8..base + 12].copy_from_slice(&y.to_le_bytes());
    slot[base + 12] = icon;
    slot[base + 13] = 1;
    slot[base + 14] = 0;
    slot[base + 15] = 0;
    Ok(marker)
}

/// Pulls one out, and says whether there was one to pull.
pub fn erase(slot: &mut [u8], id: i32) -> bool {
    let Some(at) = locate(slot) else {
        return false;
    };
    for index in 0..MOST {
        let base = at + index * RECORD;
        if word(slot, base) == id {
            slot[base..base + RECORD].copy_from_slice(&FREE);
            return true;
        }
    }
    false
}

/// Takes every marker off the map.
pub fn erase_all(slot: &mut [u8]) -> usize {
    let Some(at) = locate(slot) else {
        return 0;
    };
    let mut gone = 0;
    for index in 0..MOST {
        let base = at + index * RECORD;
        if word(slot, base) >= 0 {
            slot[base..base + RECORD].copy_from_slice(&FREE);
            gone += 1;
        }
    }
    gone
}

/// An empty array with its anchor and terminator, for tests elsewhere that need
/// a save a character could really own.
#[cfg(test)]
pub(crate) fn blank() -> Vec<u8> {
    let mut out = Vec::with_capacity(RECORD * (MOST + 1) + 4);
    out.extend_from_slice(&ANCHOR);
    for _ in 0..MOST {
        out.extend_from_slice(&FREE);
    }
    out.extend_from_slice(&TERMINATOR.to_le_bytes());
    out
}

fn word(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

fn float(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot shaped the way a real one is: junk, the anchor, the array, the
    /// terminator, more junk.
    fn slot_with(markers: &[(i32, f32, f32)]) -> Vec<u8> {
        let mut out = vec![0x77u8; 512];
        out.extend_from_slice(&ANCHOR);
        for index in 0..MOST {
            match markers.get(index) {
                Some(&(id, x, y)) => {
                    out.extend_from_slice(&id.to_le_bytes());
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                    out.extend_from_slice(&[0, 1, 0, 0]);
                }
                None => out.extend_from_slice(&FREE),
            }
        }
        out.extend_from_slice(&TERMINATOR.to_le_bytes());
        out.extend_from_slice(&[0xffu8; 128]);
        out
    }

    /// The five the player had planted, exactly as they were found in the save.
    const REAL: [(i32, f32, f32); 5] = [
        (94, 4318.86, 8166.39),
        (99, 2114.86, 5451.24),
        (100, 2352.59, 5569.01),
        (101, 1865.75, 5344.60),
        (93, 4354.01, 8199.27),
    ];

    /// The four places the game itself pairs with a map piece, and that piece's
    /// rectangle: `(world x, world z, left, right, top, bottom)`.
    ///
    /// Straight out of `WorldMapPlaceNameParam` and `WorldMapPieceParam` in the
    /// installed regulation. This is the whole evidence the transform rests on,
    /// so it belongs in a test rather than in a comment.
    const PAIRED: [(f32, f32, f32, f32, f32, f32); 4] = [
        (10376.65, 11872.48, 1392.0, 2784.0, 3954.0, 5834.0),
        (10054.4, 14695.7, 1354.0, 3194.0, 1694.0, 3701.0),
        (12543.99, 8997.84, 4724.0, 7194.0, 6002.0, 7956.0),
        (13800.64, 13119.25, 5719.0, 8704.0, 519.0, 4356.0),
    ];

    #[test]
    fn every_named_place_lands_inside_its_own_piece_of_the_map() {
        for (global_x, global_z, left, right, top, bottom) in PAIRED {
            let (map_x, map_y) = from_global(global_x, global_z);
            assert!(
                (left..=right).contains(&map_x),
                "x {map_x} outside {left}..{right}"
            );
            assert!(
                (top..=bottom).contains(&map_y),
                "y {map_y} outside {top}..{bottom}"
            );
        }
    }

    #[test]
    fn a_grid_square_either_side_of_the_corner_would_not_do() {
        // What makes the corner an answer rather than a guess: move it one
        // square and the pairs stop holding. If this ever fails, the corner is
        // no longer pinned and the constant needs solving again.
        for shift in [-TILE, TILE] {
            let missed = PAIRED.iter().any(|&(global_x, _, left, right, ..)| {
                let map_x = global_x - (ORIGIN_X + shift);
                !(left..=right).contains(&map_x)
            });
            assert!(missed, "shifting x by {shift} still satisfied every pair");
        }
    }

    #[test]
    fn the_map_runs_the_other_way_up() {
        // The thing that took two attempts to notice: the map's y grows
        // southward while the world's z grows northward.
        let (_, further_north) = from_world(40, 50, 0.0, 0.0);
        let (_, further_south) = from_world(40, 40, 0.0, 0.0);
        assert!(
            further_north < further_south,
            "north {further_north} should be above south {further_south}"
        );

        // And one world unit is one map unit, in both directions.
        let (near, _) = from_world(40, 40, 0.0, 0.0);
        let (far, _) = from_world(41, 40, 0.0, 0.0);
        assert!((far - near - TILE).abs() < 0.01);
        let (_, up) = from_world(40, 41, 0.0, 0.0);
        assert!((further_south - up - TILE).abs() < 0.01);
    }

    #[test]
    fn a_position_survives_the_trip_out_and_back() {
        for (global_x, global_z, ..) in PAIRED {
            let (map_x, map_y) = from_global(global_x, global_z);
            let (back_x, back_z) = to_global(map_x, map_y);
            assert!((back_x - global_x).abs() < 0.01);
            assert!((back_z - global_z).abs() < 0.01);
        }
        // And the grid-square form agrees with the added-up form.
        let (a, b) = from_world(41, 46, -119.35, 96.48);
        let (c, d) = from_global(41.0 * TILE - 119.35, 46.0 * TILE + 96.48);
        assert!((a - c).abs() < 0.01 && (b - d).abs() < 0.01);
    }

    #[test]
    fn the_players_own_pins_are_somewhere_on_the_map() {
        // Not a check of where they are — only they know that — but of the
        // magnitudes: a pin that back-solves to a grid square outside the
        // overworld would mean the corner is wrong.
        for (_, x, y) in REAL {
            let (global_x, global_z) = to_global(x, y);
            let tile_x = global_x / TILE;
            let tile_z = global_z / TILE;
            assert!(
                (30.0..70.0).contains(&tile_x) && (30.0..70.0).contains(&tile_z),
                "({x}, {y}) is grid square {tile_x:.1}_{tile_z:.1}"
            );
        }
    }

    #[test]
    fn a_marker_knows_which_square_of_the_world_it_is_in() {
        // The player was standing in m60_44_33_00 with two of these pinned
        // nearby, so they have to land in that neighbourhood rather than
        // somewhere across the map.
        let (_, x, y) = REAL[4]; // id 93
        let id = map_id(x, y).expect("a pin on the map has a square");
        assert_eq!(id >> 24 & 0xff, 60, "the overworld");
        let (across, down) = (id >> 16 & 0xff, id >> 8 & 0xff);
        assert!((44..=48).contains(&across), "square x {across}");
        assert!((31..=35).contains(&down), "square z {down}");

        // And the packing is the game's own: m60_44_33_00 is 0x3c2c2100.
        let (middle_x, middle_y) = from_world(44, 33, 128.0, 128.0);
        assert_eq!(map_id(middle_x, middle_y), Some(0x3c2c_2100));
    }

    #[test]
    fn the_players_own_markers_come_back() {
        let slot = slot_with(&REAL);
        let found = read(&slot);
        assert_eq!(found.len(), 5, "{found:?}");
        assert_eq!(found[0].id, 94);
        assert_eq!(found[4].id, 93);
        assert!((found[0].x - 4318.86).abs() < 0.01);
        assert!((found[3].y - 5344.60).abs() < 0.01);
    }

    #[test]
    fn a_new_marker_takes_the_first_free_slot_and_the_next_id() {
        let mut slot = slot_with(&REAL);
        let planted = place(&mut slot, 1234.5, 6789.0, 0).expect("there is room");
        // The game's counter continues past the highest in use, not past the
        // last one in the array — 93 sits after 101.
        assert_eq!(planted.id, 102);

        let found = read(&slot);
        assert_eq!(found.len(), 6);
        assert!(found.iter().any(|m| m.id == 102 && m.x == 1234.5));
        // And the five that were already there are untouched.
        for (id, _, _) in REAL {
            assert!(found.iter().any(|m| m.id == id), "lost {id}");
        }
    }

    #[test]
    fn a_freed_slot_is_filled_before_the_end() {
        let mut slot = slot_with(&REAL);
        assert!(erase(&mut slot, 99));
        let planted = place(&mut slot, 1.0, 2.0, 0).expect("there is room");

        // Sixth record still free, because the hole 99 left was used.
        let at = locate(&slot).expect("the array is there");
        assert_eq!(word(&slot, at + RECORD), planted.id);
        assert_eq!(&slot[at + 5 * RECORD..at + 6 * RECORD], &FREE);
    }

    #[test]
    fn erasing_writes_what_the_game_writes() {
        // Taken from the remove function: id -1, zeroed position, then 00 01.
        // A free record of any other shape is one the game may not recognise.
        let mut slot = slot_with(&REAL);
        let at = locate(&slot).expect("the array is there");
        assert!(erase(&mut slot, 100));
        assert_eq!(&slot[at + 2 * RECORD..at + 3 * RECORD], &FREE);
        assert!(!read(&slot).iter().any(|m| m.id == 100));
        // And erasing what is not there says so rather than corrupting a slot.
        assert!(!erase(&mut slot, 100));
        assert!(!erase(&mut slot, 5000));
    }

    #[test]
    fn a_full_map_is_refused_rather_than_overflowing() {
        let full: Vec<(i32, f32, f32)> = (0..MOST as i32).map(|n| (n, 1.0, 2.0)).collect();
        let mut slot = slot_with(&full);
        assert_eq!(read(&slot).len(), MOST);
        let refused = place(&mut slot, 1.0, 1.0, 0);
        assert!(refused.is_err(), "{refused:?}");
        // Nothing beyond the array was written over.
        let at = locate(&slot).expect("the array is there");
        assert_eq!(word(&slot, at + MOST * RECORD), TERMINATOR);
    }

    #[test]
    fn a_slot_without_a_character_has_no_markers() {
        let empty = vec![0u8; 4096];
        assert!(read(&empty).is_empty());
        assert!(place(&mut empty.clone(), 1.0, 2.0, 0).is_err());
        assert!(!erase(&mut empty.clone(), 1));
        assert_eq!(erase_all(&mut empty.clone()), 0);
    }

    #[test]
    fn an_anchor_without_an_array_behind_it_is_not_an_array() {
        // The failure worth guarding: those sixteen bytes turning up somewhere
        // else in a slot. Writing markers there would be writing them into
        // whatever structure actually lives at that address.
        let mut lookalike = vec![0x33u8; 256];
        lookalike.extend_from_slice(&ANCHOR);
        lookalike.extend_from_slice(&[0x33u8; 4096]);
        assert!(locate(&lookalike).is_none());
        assert!(read(&lookalike).is_empty());
    }

    /// The player's own save, if this machine has one.
    ///
    /// Everything above is a slot this file built, which proves the code agrees
    /// with itself. This is the one that proves it agrees with ELDEN RING.
    /// Nothing is written to disk: the round trip happens on a copy in memory.
    #[test]
    fn a_real_save_round_trips() {
        let Some(home) = std::env::var_os("APPDATA") else {
            return;
        };
        let root = std::path::Path::new(&home).join("EldenRing");
        let Ok(accounts) = std::fs::read_dir(&root) else {
            return;
        };

        let mut saves: Vec<std::path::PathBuf> = Vec::new();
        for account in accounts.flatten() {
            let Ok(files) = std::fs::read_dir(account.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(extension, "sl2" | "co2") {
                    saves.push(path);
                }
            }
        }
        if saves.is_empty() {
            return;
        }

        for path in saves {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(mut save) = crate::formats::save::SaveFile::from_bytes(bytes) else {
                continue;
            };
            for index in 0..crate::formats::save::SLOT_COUNT {
                let Ok(slot) = save.slot_data(index) else {
                    continue;
                };
                if locate(slot).is_none() {
                    continue;
                }

                let before = read(slot);
                let slot = save.slot_data_mut(index).expect("the slot is there");
                let planted = place(slot, 1234.5, 6789.25, 0).expect("a real slot has room");

                let after = read(slot);
                assert_eq!(after.len(), before.len() + 1, "{path:?} slot {index}");
                assert!(after.iter().any(|m| m.id == planted.id
                    && m.x == 1234.5
                    && m.y == 6789.25
                    && m.icon == 0));
                for old in &before {
                    assert!(after.contains(old), "{path:?} lost {old:?}");
                }

                // And back out again, leaving the slot byte-for-byte as found.
                let untouched = save.slot_bytes(index).expect("the slot is there");
                let slot = save.slot_data_mut(index).expect("the slot is there");
                assert!(erase(slot, planted.id));
                assert_eq!(read(slot), before, "{path:?} slot {index}");

                // The game refuses a slot whose MD5 no longer matches, so a
                // write path that forgets this produces a save that will not
                // load.
                save.recompute_checksums();
                assert!(save.verify_checksums());
                assert_ne!(untouched.len(), 0);
            }
        }
    }

    #[test]
    fn clearing_the_map_leaves_it_readable_and_writable() {
        let mut slot = slot_with(&REAL);
        assert_eq!(erase_all(&mut slot), 5);
        assert!(read(&slot).is_empty());
        // And the array still works afterwards, numbering from the start again.
        let planted = place(&mut slot, 7.0, 8.0, 0).expect("there is room");
        assert_eq!(planted.id, 1);
    }
}
