//! The character the game has open right now.
//!
//! Not the save, which holds every character ever made and only as of the last
//! grace — reading it is how the assistant told somebody they were level 12
//! while they stood in the world at 34.
//!
//! Every offset here was read off a running game, and each is checked before it
//! is believed: the attributes must be in range and add up to the level. When
//! the checks fail this reports nothing, because a wrong level is worse than no
//! level — the advice built on it sounds equally certain.

use serde::Serialize;

use crate::games::Game;

/// The instruction that loads the pointer to the player's data.
///
/// A code pattern rather than an address: the code moves far less than the data
/// does, and `find_only` refuses a pattern that matches twice.
const GAME_DATA_MAN: &str = "48 8B 05 ?? ?? ?? ?? 48 85 C0 74 05 48 8B 40 58 C3 C3";

/// The same, for the pointer to the world and everything standing in it.
const WORLD_CHR_MAN: &str = "48 8B 05 ?? ?? ?? ?? 48 85 C0 74 0F 48 39 88";

/// Where the player is, and which map they are on.
///
/// `[[WorldChrMan + 0x10EF8] + 0] + 0x6C0` is a block of four values: X, Z, Y
/// and the map id. This is the chain The Grand Archives' table reads, which is
/// the reason it is a lookup here and not a hunt — two afternoons of watching
/// what moved while somebody walked found the camera and a lot of scenery.
mod world {
    pub const PLAYER: usize = 0x10EF8;
    pub const BLOCK: usize = 0x6C0;
    pub const X: usize = 0x00;
    pub const Z: usize = 0x04;
    pub const Y: usize = 0x08;
    pub const MAP: usize = 0x10;
}

/// Map ids and the places they are, one per line: hex id, tab, name.
///
/// Kept as data rather than code — it is three hundred and eighty lines of
/// somebody else's careful survey work, and it belongs in a file that can be
/// replaced when the next area is added.
const MAP_NAMES: &str = include_str!("../assets/map-names.tsv");

/// What to call the map the player is standing on.
///
/// The overworld is a grid of tiles and only the ones worth naming are named,
/// so a tile with no entry falls back to its neighbours: being told "Liurnia of
/// the Lakes" from one tile over is right, and being told nothing is not.
pub fn place(map: u32) -> Option<String> {
    let named = |id: u32| -> Option<&'static str> {
        MAP_NAMES.lines().find_map(|line| {
            let (key, name) = line.split_once('\t')?;
            (u32::from_str_radix(key, 16).ok()? == id).then_some(name)
        })
    };

    if let Some(name) = named(map) {
        return Some(name.to_string());
    }

    // The overworld only. A legacy dungeon with no entry is not a tile and has
    // no neighbours to borrow from.
    if (map >> 24) & 0xff != 60 {
        return None;
    }
    let (x, y) = ((map >> 16) & 0xff, (map >> 8) & 0xff);
    for ring in 1..=2i64 {
        for dx in -ring..=ring {
            for dy in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if !(0..=255).contains(&nx) || !(0..=255).contains(&ny) {
                    continue;
                }
                let near = (60 << 24) | ((nx as u32) << 16) | ((ny as u32) << 8);
                if let Some(name) = named(near) {
                    // Only the region, since the landmark belongs to the tile
                    // next door rather than to this one.
                    let region = name.split(" - ").next().unwrap_or(name);
                    return Some(format!("near {region}"));
                }
            }
        }
    }
    None
}

/// What they are wearing and holding, as offsets into the player's block.
///
/// Found by asking the game to name every number in the block and keeping the
/// ones it recognised — a run of six that name weapons, then four that name
/// armour, then four that name talismans, with the empty slots reading as -1
/// between them. A layout that names itself is a layout that cannot be off by
/// one.
mod gear {
    pub const WEAPONS: usize = 0x398;
    pub const WEAPON_SLOTS: usize = 6;
    pub const ARMOUR: usize = 0x3c8;
    pub const TALISMANS: usize = 0x3d8;
    pub const TALISMAN_SLOTS: usize = 4;
    /// The id the game uses for a hand with nothing in it.
    pub const BARE: u32 = 110_000;
}

/// Head, body, arms, legs — in that order, and the ids say so themselves: a set
/// is one number with 000, 100, 200 and 300 added to it.
const ARMOUR_SLOTS: [&str; 4] = ["Head", "Body", "Arms", "Legs"];

/// Verified against a live 1.16.1 (executable 2.6.1.0).
mod at {
    pub const HP: usize = 0x10;
    pub const HP_MAX: usize = 0x14;
    pub const FP: usize = 0x1c;
    pub const FP_MAX: usize = 0x20;
    pub const STAMINA: usize = 0x2c;
    pub const STAMINA_MAX: usize = 0x30;
    /// The attributes as the save holds them: eight, Vigor first, Arcane last.
    ///
    /// These are the points the player spent, which is what the level is a sum
    /// of — so they are what the level check below is done against. They are
    /// not what the stat screen prints.
    pub const STATS: usize = 0x3c;

    /// The attributes the game prints, which is base plus whatever equipment
    /// adds.
    ///
    /// Nine slots here rather than eight: the fourth is Vitality, unused since
    /// Dark Souls and always zero. Reading the eight from the save instead
    /// reported Faith 17 and Arcane 21 at somebody whose screen said 22 and 26,
    /// because a seal was giving five of each — the sort of confidently wrong
    /// number that makes every answer built on it wrong too.
    pub const SHOWN: usize = 0x288;
    pub const SHOWN_SLOTS: usize = 9;
    /// The one that is not an attribute.
    pub const VITALITY: usize = 3;
    pub const LEVEL: usize = 0x68;
    pub const RUNES: usize = 0x6c;
    pub const RUNES_EVER: usize = 0x70;
    /// UTF-16, sixteen characters at most.
    pub const NAME: usize = 0x9c;
}

/// Each attribute with the short form the game prints beside it.
///
/// The short form is the part that cannot be misread. A Russian copy labels
/// Mind "Интеллект(FP)" and Intelligence "Мудрость(INT)", so an answer that
/// says "Intelligence 9" against a screen reading "Интеллект 13" is talking
/// about a different attribute and sounds like a mistake. FTH is FTH in every
/// language the game ships.
const STAT_NAMES: [&str; 8] = [
    "Vigor (VIG)",
    "Mind (MND/FP)",
    "Endurance (END)",
    "Strength (STR)",
    "Dexterity (DEX)",
    "Intelligence (INT)",
    "Faith (FTH)",
    "Arcane (ARC)",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    pub name: String,
    pub level: u32,
    pub runes: u32,
    pub runes_ever: u32,
    pub hp: u32,
    pub hp_max: u32,
    pub fp: u32,
    pub fp_max: u32,
    pub stamina: u32,
    pub stamina_max: u32,
    /// The attributes as the stat screen prints them: what they spent, plus
    /// whatever they are wearing.
    pub stats: Vec<(String, u32)>,
    /// The points actually spent, given only when equipment has changed them.
    pub spent: Option<Vec<(String, u32)>>,
    /// Where they are standing, when the game has a world open.
    pub place: Option<Place>,
    /// What they are holding and wearing, named by the game itself.
    pub gear: Option<Gear>,
}

/// Equipment, in the game's own words.
///
/// Names rather than numbers, and the game's names rather than a table's: a
/// total conversion renames things, and the player is reading its names off
/// their screen while they ask.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gear {
    /// Everything in hand, empty hands left out.
    pub weapons: Vec<String>,
    /// Slot and what is in it, missing pieces left out.
    pub armour: Vec<(String, String)>,
    pub talismans: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    /// `m60_35_44_00`, as the game's own files name it.
    pub map: String,
    /// What that is, in words.
    pub name: Option<String>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub fn read(game: Game) -> Option<Live> {
    let pid = crate::unlock::running_pid(game.executable())?;
    let process = crate::unlock::win::Process::open(pid).ok()?;
    let (base, size) = process.main_module().ok()?;

    let image = process.read(base, size);
    let found = crate::unlock::find_only(&image, &crate::unlock::parse(GAME_DATA_MAN))?;

    // RIP-relative: four bytes of displacement at +3, from the end at +7.
    let displacement = i32::from_le_bytes(image.get(found + 3..found + 7)?.try_into().ok()?);
    let slot = (base + found + 7).checked_add_signed(displacement as isize)?;

    let manager = pointer(&process, slot)?;
    let data = pointer(&process, manager + 0x08)?;
    // Far enough to reach the printed attributes, which sit well past the ones
    // the save holds.
    let block = process.read(data, 0x300);

    let mut live = parse_block(&block)?;
    live.place = where_they_are(&process, &image, base);
    live.gear = crate::text::Text::open(&process, &image, base)
        .and_then(|text| what_they_carry(&process, &text, data));
    Some(live)
}

/// What is in their hands and on their back.
///
/// Every id is put to the game to be named, and one that does not name is left
/// out rather than printed as a number — a slot the layout got wrong then shows
/// up as nothing, which is the failure worth having.
fn what_they_carry(
    process: &crate::unlock::win::Process,
    text: &crate::text::Text,
    data: usize,
) -> Option<Gear> {
    use crate::text::Kind;

    let block = process.read(data + gear::WEAPONS, 0x50);
    let id = |at: usize| -> Option<u32> {
        let value = u32::from_le_bytes(block.get(at..at + 4)?.try_into().ok()?);
        (value != 0 && value != u32::MAX && value != gear::BARE).then_some(value)
    };
    let from = |base: usize| base - gear::WEAPONS;

    let mut carried = Gear::default();

    for slot in 0..gear::WEAPON_SLOTS {
        if let Some(name) = id(slot * 4).and_then(|value| text.name(Kind::Weapon, value)) {
            // Both hands can hold the same weapon, and saying it twice reads as
            // a mistake rather than as two of them.
            if !carried.weapons.contains(&name) {
                carried.weapons.push(name);
            }
        }
    }

    for (slot, what) in ARMOUR_SLOTS.iter().enumerate() {
        if let Some(name) =
            id(from(gear::ARMOUR) + slot * 4).and_then(|value| text.name(Kind::Armour, value))
        {
            carried.armour.push(((*what).to_string(), name));
        }
    }

    for slot in 0..gear::TALISMAN_SLOTS {
        if let Some(name) =
            id(from(gear::TALISMANS) + slot * 4).and_then(|value| text.name(Kind::Talisman, value))
        {
            carried.talismans.push(name);
        }
    }

    let empty = carried.weapons.is_empty() && carried.armour.is_empty() && carried.talismans.is_empty();
    (!empty).then_some(carried)
}

/// The map and the coordinates, when a world is loaded.
///
/// Optional all the way down: at the title screen and during a load there is no
/// player and no map, and the right answer then is silence.
fn where_they_are(
    process: &crate::unlock::win::Process,
    image: &[u8],
    base: usize,
) -> Option<Place> {
    let found = crate::unlock::find_only(image, &crate::unlock::parse(WORLD_CHR_MAN))?;
    let displacement = i32::from_le_bytes(image.get(found + 3..found + 7)?.try_into().ok()?);
    let slot = (base + found + 7).checked_add_signed(displacement as isize)?;

    let world = pointer(process, slot)?;
    let player = pointer(process, world + world::PLAYER)?;
    let inner = pointer(process, player)?;

    let block = process.read(inner + world::BLOCK, 0x20);
    let float = |at: usize| -> Option<f32> {
        let value = f32::from_le_bytes(block.get(at..at + 4)?.try_into().ok()?);
        value.is_finite().then_some(value)
    };
    let map = u32::from_le_bytes(block.get(world::MAP..world::MAP + 4)?.try_into().ok()?);
    if map == 0 {
        return None;
    }

    Some(Place {
        map: format!(
            "m{:02}_{:02}_{:02}_{:02}",
            (map >> 24) & 0xff,
            (map >> 16) & 0xff,
            (map >> 8) & 0xff,
            map & 0xff
        ),
        name: place(map),
        x: float(world::X)?,
        y: float(world::Y)?,
        z: float(world::Z)?,
    })
}

/// The same, from bytes, so the checks can be tested without a game.
fn parse_block(block: &[u8]) -> Option<Live> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(block.get(at..at + 4)?.try_into().ok()?))
    };

    let level = word(at::LEVEL)?;
    if !(1..=713).contains(&level) {
        return None;
    }

    let mut stats = Vec::with_capacity(8);
    for index in 0..8 {
        let value = word(at::STATS + index * 4)?;
        if !(1..=99).contains(&value) {
            return None;
        }
        stats.push(value);
    }

    // The check that makes the rest trustworthy. A character's level is the sum
    // of their attributes less what their class began with, and every class
    // begins with between 79 and 81 points. Anything outside that is eight
    // numbers that happened to be small.
    let total: u32 = stats.iter().sum();
    if total < level + 70 || total > level + 90 {
        return None;
    }

    let hp_max = word(at::HP_MAX)?;
    if hp_max == 0 || hp_max > 10_000 {
        return None;
    }

    // What the stat screen prints, when it can be read. Equipment moves these
    // and the player is looking at the moved numbers, so an answer built on the
    // spent points is answering about a character they do not have.
    let shown = worn_in(block).unwrap_or(stats.clone());

    Some(Live {
        name: name(block),
        level,
        runes: word(at::RUNES)?,
        runes_ever: word(at::RUNES_EVER)?,
        hp: word(at::HP)?,
        hp_max,
        fp: word(at::FP)?,
        fp_max: word(at::FP_MAX)?,
        stamina: word(at::STAMINA)?,
        stamina_max: word(at::STAMINA_MAX)?,
        stats: STAT_NAMES
            .iter()
            .zip(shown.iter().copied())
            .map(|(what, value)| ((*what).to_string(), value))
            .collect(),
        // Only worth saying when equipment has actually moved something.
        spent: (shown != stats).then(|| {
            STAT_NAMES
                .iter()
                .zip(stats)
                .map(|(what, value)| ((*what).to_string(), value))
                .collect()
        }),
        place: None,
        gear: None,
    })
}

/// The attributes with equipment counted in, as the stat screen prints them.
///
/// Nine slots with Vitality in the middle, which the game has kept and not used
/// since Dark Souls. Refused rather than guessed at when the shape is wrong:
/// falling back to the spent points is a number that is merely incomplete,
/// where a misread one is a number that is false.
fn worn_in(block: &[u8]) -> Option<Vec<u32>> {
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(block.get(at..at + 4)?.try_into().ok()?))
    };

    let mut shown = Vec::with_capacity(8);
    for slot in 0..at::SHOWN_SLOTS {
        let value = word(at::SHOWN + slot * 4)?;
        if slot == at::VITALITY {
            // The unused one. Anything in it means this is not the block.
            if value != 0 {
                return None;
            }
            continue;
        }
        if !(1..=198).contains(&value) {
            return None;
        }
        shown.push(value);
    }
    Some(shown)
}

fn name(block: &[u8]) -> String {
    let mut units = Vec::new();
    let mut cursor = at::NAME;
    while cursor + 1 < block.len() && units.len() < 16 {
        let unit = u16::from_le_bytes([block[cursor], block[cursor + 1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
        cursor += 2;
    }
    String::from_utf16_lossy(&units).trim().to_string()
}

fn pointer(process: &crate::unlock::win::Process, at: usize) -> Option<usize> {
    let bytes = process.read(at, 8);
    let value = usize::from_le_bytes(bytes.try_into().ok()?);
    (value > 0x10000 && value < 0x7fff_ffff_ffff).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block as a running game had it: level 34, "Way Of Life", 643 of 690,
    /// wearing a seal worth five Faith and five Arcane.
    ///
    /// Both sets of attributes, because the difference between them is the bug
    /// this file exists to have got wrong once: the save says 17 and 21, the
    /// stat screen said 22 and 26, and the launcher reported the save.
    fn real() -> Vec<u8> {
        let mut block = vec![0u8; 0x300];
        let mut put = |at: usize, value: u32| {
            block[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        // What the screen prints, with the unused Vitality slot left at zero.
        for (slot, value) in [18u32, 13, 11, 0, 10, 14, 9, 22, 26].iter().enumerate() {
            put(at::SHOWN + slot * 4, *value);
        }
        put(at::HP, 643);
        put(at::HP_MAX, 690);
        put(at::FP, 113);
        put(at::FP_MAX, 113);
        put(at::STAMINA, 97);
        put(at::STAMINA_MAX, 97);
        for (index, value) in [18u32, 13, 11, 10, 14, 9, 17, 21].iter().enumerate() {
            put(at::STATS + index * 4, *value);
        }
        put(at::LEVEL, 34);
        put(at::RUNES, 3501);
        put(at::RUNES_EVER, 107_760);
        for (index, unit) in "Way Of Life".encode_utf16().enumerate() {
            let cursor = at::NAME + index * 2;
            block[cursor..cursor + 2].copy_from_slice(&unit.to_le_bytes());
        }
        block
    }

    #[test]
    fn the_character_reads_back_as_the_game_had_them() {
        let live = parse_block(&real()).expect("the real block has to parse");
        assert_eq!(live.name, "Way Of Life");
        assert_eq!(live.level, 34);
        assert_eq!(live.runes, 3501);
        assert_eq!((live.hp, live.hp_max), (643, 690));
        assert_eq!(live.stats[0].1, 18);
        assert_eq!(live.stats[7].1, 26, "the screen said 26, not the save's 21");
    }

    #[test]
    fn the_attributes_reported_are_the_ones_on_the_screen() {
        // A seal was giving five Faith and five Arcane, so the save said 17 and
        // 21 while the player was looking at 22 and 26. Reporting the save made
        // every answer built on it wrong, and sounded just as certain.
        let live = parse_block(&real()).unwrap();
        let by = |what: &str| live.stats.iter().find(|(n, _)| n.contains(what)).unwrap().1;
        assert_eq!(by("FTH"), 22);
        assert_eq!(by("ARC"), 26);

        // The points they actually spent are kept, since that is what the level
        // is a sum of and what respeccing would move.
        let spent = live.spent.expect("equipment moved them, so both are known");
        let base = |what: &str| spent.iter().find(|(n, _)| n.contains(what)).unwrap().1;
        assert_eq!(base("FTH"), 17);
        assert_eq!(base("ARC"), 21);
        assert_eq!(base("VIG"), 18, "an attribute nothing touched reads the same");
    }

    #[test]
    fn attributes_nothing_has_moved_are_reported_once() {
        // No equipment bonus, so there is no second set to explain.
        let mut plain = real();
        for (slot, value) in [18u32, 13, 11, 0, 10, 14, 9, 17, 21].iter().enumerate() {
            plain[at::SHOWN + slot * 4..at::SHOWN + slot * 4 + 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        let live = parse_block(&plain).unwrap();
        assert!(live.spent.is_none());
        assert_eq!(live.stats[6].1, 17);
    }

    #[test]
    fn a_short_read_falls_back_to_the_points_that_were_spent() {
        // Incomplete beats false: a block that stops before the printed
        // attributes gives the save's, which are at least a character's.
        let short = real()[..0x100].to_vec();
        let live = parse_block(&short).expect("the save's attributes are in reach");
        assert_eq!(live.stats[7].1, 21);
        assert!(live.spent.is_none());
    }

    #[test]
    fn every_attribute_says_which_one_it_is_in_any_language() {
        // A Russian copy labels Mind "Интеллект(FP)" and Intelligence
        // "Мудрость(INT)". Answering "Intelligence 9" against a screen reading
        // "Интеллект 13" names a different attribute and reads as an error, so
        // each carries the short form, which does not translate.
        for short in ["VIG", "MND", "END", "STR", "DEX", "INT", "FTH", "ARC"] {
            assert!(
                STAT_NAMES.iter().any(|name| name.contains(short)),
                "no attribute is marked {short}"
            );
        }
    }

    #[test]
    fn a_block_that_is_not_a_character_is_refused() {
        // The point of the checks. Reporting a level read out of the wrong
        // structure is worse than reporting nothing, because the advice built
        // on it is just as confident.
        assert!(parse_block(&[0u8; 0x100]).is_none(), "all zeroes");
        assert!(parse_block(&[0xffu8; 0x100]).is_none(), "all ones");
        assert!(parse_block(&[0u8; 0x20]).is_none(), "too short");

        // Eight plausible attributes that do not add up to the level.
        let mut wrong = real();
        wrong[at::LEVEL..at::LEVEL + 4].copy_from_slice(&300u32.to_le_bytes());
        assert!(parse_block(&wrong).is_none(), "level 300 on 113 points");

        // An attribute the game could not produce.
        let mut mad = real();
        mad[at::STATS..at::STATS + 4].copy_from_slice(&255u32.to_le_bytes());
        assert!(parse_block(&mad).is_none());
    }

    #[test]
    fn a_map_id_becomes_somewhere_a_person_knows() {
        // The id the running game gave while the player stood in Liurnia.
        assert_eq!(
            place(0x3c232c00).as_deref(),
            Some("Liurnia of the Lakes - Far West Gate Town, North Rose Church")
        );

        // Legacy dungeons are named outright.
        assert_eq!(place(0x0a000000).as_deref(), Some("Stormveil Castle"));
        assert_eq!(place(0x0b0a0000).as_deref(), Some("Roundtable Hold"));

        // An overworld tile nobody surveyed borrows its region from next door,
        // and says that it is doing so.
        let borrowed = place(0x3c232d00);
        assert!(
            borrowed.as_deref().is_none_or(|name| name.starts_with("near ")
                || name.starts_with("Liurnia")),
            "got {borrowed:?}"
        );

        // Nothing invented for an id that is not a map.
        assert!(place(0).is_none());
        assert!(place(0xffffffff).is_none());
    }

    #[test]
    fn every_line_of_the_map_table_parses() {
        let mut count = 0;
        for line in MAP_NAMES.lines().filter(|l| !l.trim().is_empty()) {
            let (key, name) = line.split_once('\t').unwrap_or_else(|| panic!("no tab: {line}"));
            u32::from_str_radix(key, 16).unwrap_or_else(|_| panic!("not hex: {key}"));
            assert!(!name.trim().is_empty(), "no name: {line}");
            count += 1;
        }
        assert!(count > 300, "only {count} maps");
    }

    #[test]
    fn the_save_is_not_consulted() {
        // It used to be: the name and level were taken from the save and used
        // to find the fields. That reported level 12 from a slot nobody was
        // playing while the live character stood at 34.
        let live = parse_block(&real()).unwrap();
        assert_eq!(live.level, 34);
    }
}
