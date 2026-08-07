//! The character as the running game has them, not as the save file left them.
//!
//! Level, runes, stats, health, where they are standing. A question like "should
//! I go for Malenia now" turns on the answer, and the save on disk is whatever
//! it was at the last grace — twenty levels ago in a long session.
//!
//! Offsets into another program's memory move with its patches, so none are
//! trusted here. The structure is found by pattern, and then *calibrated*: the
//! save file already says what the character is called and what level they are,
//! so the fields are located by looking for those known values rather than by
//! counting bytes from a table that was right for some other version. When the
//! calibration does not find them, this reports nothing at all — a wrong level
//! is worse than no level, because the answer built on it sounds just as sure.

use serde::Serialize;

use crate::games::Game;

/// Where the game keeps the pointer to its own player data.
///
/// A code pattern, not an address: the instruction that loads the pointer is
/// stable across versions in a way the pointer's address is not. The `48 8B 05`
/// is a RIP-relative load, so the four bytes after it are a displacement from
/// the end of the instruction.
const GAME_DATA_MAN: &str = "48 8B 05 ?? ?? ?? ?? 48 85 C0 74 05 48 8B 40 58 C3 C3";

/// How far into the block to look for the fields being calibrated.
const BLOCK: usize = 0x400;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    pub name: String,
    pub level: u32,
    pub runes: u32,
    /// Vigor, Mind, Endurance, Strength, Dexterity, Intelligence, Faith, Arcane.
    pub stats: Vec<(String, u32)>,
}

/// The eight, in the order the game stores them.
const STATS: &[&str] = &[
    "Vigor",
    "Mind",
    "Endurance",
    "Strength",
    "Dexterity",
    "Intelligence",
    "Faith",
    "Arcane",
];

/// Reads the running game, given what the save says to expect.
///
/// `expect` is a character from the save: the name is the anchor and the level
/// is the check. Without one there is nothing to calibrate against and this
/// returns nothing, which is the right answer rather than a guess.
pub fn read(game: Game, expect: &[(String, u32)]) -> Option<Live> {
    if expect.is_empty() {
        return None;
    }
    let pid = crate::unlock::running_pid(game.executable())?;
    let process = crate::unlock::win::Process::open(pid).ok()?;
    let (base, size) = process.main_module().ok()?;

    let image = process.read(base, size);
    let at = crate::unlock::find_only(&image, &crate::unlock::parse(GAME_DATA_MAN))?;

    // RIP-relative: the displacement is the four bytes at +3, relative to the
    // instruction's end at +7.
    let displacement = i32::from_le_bytes(image.get(at + 3..at + 7)?.try_into().ok()?);
    let slot = (base + at + 7).checked_add_signed(displacement as isize)?;

    let manager = pointer(&process, slot)?;
    let data = pointer(&process, manager + 0x08)?;
    let block = process.read(data, BLOCK);

    // The name anchors everything. It is UTF-16 and it is one of the names the
    // save knows, which is what makes finding it proof rather than a guess.
    let (name, name_at) = find_name(&block, expect)?;
    let level = *expect.iter().find(|(who, _)| *who == name).map(|(_, l)| l)?;

    // The level is a small integer somewhere before the name. Which one it is,
    // is settled by it matching the save — and by the eight stats sitting where
    // they should relative to it, so a coincidental match is not enough.
    let (level_at, live_level) = find_level(&block, name_at, level)?;
    let stats = read_stats(&block, level_at)?;

    Some(Live {
        name,
        level: live_level,
        runes: word(&block, level_at + 0x04).unwrap_or(0),
        stats: STATS
            .iter()
            .zip(stats)
            .map(|(what, value)| ((*what).to_string(), value))
            .collect(),
    })
}

fn pointer(process: &crate::unlock::win::Process, at: usize) -> Option<usize> {
    let bytes = process.read(at, 8);
    let value = usize::from_le_bytes(bytes.try_into().ok()?);
    (value > 0x10000).then_some(value)
}

fn word(block: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(block.get(at..at + 4)?.try_into().ok()?))
}

/// The character's name, and where in the block it starts.
fn find_name(block: &[u8], expect: &[(String, u32)]) -> Option<(String, usize)> {
    for (who, _) in expect {
        let wide: Vec<u8> = who
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        if wide.is_empty() {
            continue;
        }
        if let Some(at) = block.windows(wide.len()).position(|w| w == wide) {
            return Some((who.clone(), at));
        }
    }
    None
}

/// Where the level sits, confirmed by the stats that follow it.
///
/// A block this size holds plenty of small integers and several will happen to
/// equal the level. The one that is the level has eight plausible attributes a
/// fixed distance behind it and a rune count in front, and only one candidate
/// satisfies all of that.
fn find_level(block: &[u8], name_at: usize, level: u32) -> Option<(usize, u32)> {
    (0..name_at.min(BLOCK))
        .step_by(4)
        .filter(|at| word(block, *at) == Some(level))
        .find_map(|at| {
            read_stats(block, at).map(|_| (at, level))
        })
}

/// The eight attributes, which sit immediately before the level.
///
/// Returns nothing unless all eight are in range and add up to something a
/// character of this level could have — which is what makes the level candidate
/// above a match rather than a coincidence.
fn read_stats(block: &[u8], level_at: usize) -> Option<Vec<u32>> {
    let start = level_at.checked_sub(0x20)?;
    let mut out = Vec::with_capacity(8);
    for index in 0..8 {
        let value = word(block, start + index * 4)?;
        // The game's own floor is 1 and its ceiling is 99.
        if !(1..=99).contains(&value) {
            return None;
        }
        out.push(value);
    }

    // A character's level is the total of their attributes less the eight they
    // started with, give or take the starting class. Anything wildly off is a
    // run of numbers that happened to be in range.
    let level = word(block, level_at)?;
    let total: u32 = out.iter().sum();
    let expected = level + 70;
    (total.abs_diff(expected) <= 25).then_some(out)
}
