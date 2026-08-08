//! The game's own words for things.
//!
//! Reading names out of the running game rather than a shipped table settles
//! two problems at once: a total conversion that renames half the game is
//! answered correctly, and the answer comes back in the language the player is
//! reading off their screen. A flat list of ids was wrong both ways — `1040000`
//! is a dagger in one table and a helmet in another, and a modded game's ids
//! are in no table at all.
//!
//! Reads only. The game has a function for this, but a launcher that runs code
//! inside a running game can crash it and looks, to anything watching, like a
//! cheat. The structure behind that function is walked here instead; the result
//! was checked against the function's own answer on sixteen lookups first.

use std::collections::HashMap;

use crate::unlock::win::Process;

/// The code that loads the text store, keeps it, and names it when it is
/// missing.
///
/// The name is the point. Several hundred places in the game load a singleton
/// this way and the shape alone cannot tell them apart, so each candidate is
/// only accepted once the string it would have printed reads `MsgRepository`.
/// That check costs nothing and survives the next patch moving everything.
const GETTER: &str = "48 8B 0D ?? ?? ?? ?? 48 85 C9 75 ?? 4C 8D 0D";

/// Bytes from the start of the match to the end of the `lea`.
const GETTER_LEN: usize = 19;

const NAMED: &[u8] = b"MsgRepository";

/// What sort of thing an id belongs to.
///
/// The same number means different things in different tables, so the kind is
/// not a hint — it is half of the lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Weapon,
    Armour,
    Talisman,
    /// Everything carried that is not worn or held, which in this game includes
    /// the spells: a sorcery is an item in the pouch, not a table of its own.
    /// The table that sounds like it holds spells holds two placeholders.
    Goods,
    Place,
    Npc,
}

impl Kind {
    /// The tables the game itself tries, in the order it tries them.
    ///
    /// Taken from the game's own lookup helpers rather than chosen: each helper
    /// walks a list and falls back to the next, which is how the base game, the
    /// two downloadable chapters and the patched-in entries end up in separate
    /// tables that answer to one question.
    fn names(self) -> &'static [u32] {
        match self {
            Kind::Weapon => &[115, 11, 310, 410],
            Kind::Armour => &[117, 12, 313, 413],
            Kind::Talisman => &[113, 13, 316, 416],
            Kind::Goods => &[111, 10, 319, 419],
            Kind::Place => &[120, 19, 329, 429],
            Kind::Npc => &[119, 18, 328, 428],
        }
    }

    /// The tables holding the paragraph of flavour under the name.
    fn captions(self) -> &'static [u32] {
        match self {
            Kind::Weapon => &[106, 25, 312, 412],
            Kind::Armour => &[108, 26, 315, 415],
            Kind::Talisman => &[109, 27, 318, 418],
            Kind::Goods => &[100, 24, 321, 421],
            _ => &[],
        }
    }

    /// The one line saying what the thing does, as the menu prints it.
    ///
    /// Shorter and more useful than the description: a talisman's is "raises
    /// maximum HP" where its description is three sentences about amber.
    fn effects(self) -> &'static [u32] {
        match self {
            Kind::Weapon => &[114, 21, 311, 411],
            Kind::Armour => &[116, 22, 314, 414],
            Kind::Talisman => &[112, 23, 317, 417],
            Kind::Goods => &[110, 20, 320, 420],
            _ => &[],
        }
    }
}

/// What the game writes into a slot nobody filled in.
///
/// Unused ids are not absent — they hold a marker, and a catalogue that keeps
/// them offers the player a choice of four thousand items called `[ERROR]`.
fn is_placeholder(text: &str) -> bool {
    let plain = text.trim();
    plain.is_empty()
        || plain.starts_with("[ERROR]")
        || plain.eq_ignore_ascii_case("dummy")
        || plain.eq_ignore_ascii_case("DLC dummy")
        || plain.chars().all(|c| c == '%' || c == '?' || c.is_whitespace())
}

/// The menu's own markup, taken back out.
///
/// Descriptions carry colour tags and underlines for the menu to render. Left
/// in, they arrive in the answer as literal `<font color="#FFA500">`, which is
/// noise in a sentence and wasted room in a narrow window.
fn without_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for ch in text.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    // The menu breaks lines to fit its own box, which is not this box.
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One thing the game knows about, as the game describes it.
#[derive(Debug, Clone)]
pub struct Found {
    pub kind: Kind,
    pub id: u32,
    pub name: String,
    /// The line the menu prints for what it does.
    pub effect: Option<String>,
    /// The paragraph under it.
    pub caption: Option<String>,
}

impl Kind {
    pub fn what(self) -> &'static str {
        match self {
            Kind::Weapon => "weapon",
            Kind::Armour => "armour",
            Kind::Talisman => "talisman",
            Kind::Goods => "item",
            Kind::Place => "place",
            Kind::Npc => "character",
        }
    }
}

/// The text store of a running game.
pub struct Text<'a> {
    process: &'a Process,
    /// Each table, by the number the game knows it as.
    tables: HashMap<u32, usize>,
    /// Answers already paid for. A set of armour is four lookups of the same
    /// shape and an inventory is hundreds.
    seen: std::cell::RefCell<HashMap<(u32, u32), Option<String>>>,
}

impl<'a> Text<'a> {
    /// Finds the store in a running game, or reports that it is not there yet.
    ///
    /// At the title screen the pointer is null and stays null until a save is
    /// loaded, which is a normal state and not a failure.
    pub fn open(process: &'a Process, image: &[u8], base: usize) -> Option<Self> {
        let slot = find_store(process, image, base)?;
        let repo = pointer(process, slot)?;

        let head = process.read(repo, 0x18);
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(head.get(at..at + 4)?.try_into().ok()?))
        };
        let languages = word(0x10)?;
        let count = word(0x14)?;
        if languages == 0 || count == 0 || count > 4096 {
            return None;
        }

        // One language is loaded — whichever the player installed. Asking for
        // the second would be asking for a language this copy does not have.
        let groups = pointer(process, repo + 0x08)?;
        let group = pointer(process, groups)?;

        let raw = process.read(group, count as usize * 8);
        let mut tables = HashMap::new();
        for index in 0..count as usize {
            let Some(bytes) = raw.get(index * 8..index * 8 + 8) else {
                break;
            };
            let at = usize::from_le_bytes(bytes.try_into().ok()?);
            if at > 0x10000 && at < 0x7fff_ffff_ffff {
                tables.insert(index as u32, at);
            }
        }
        (!tables.is_empty()).then_some(Text {
            process,
            tables,
            seen: std::cell::RefCell::new(HashMap::new()),
        })
    }

    /// What the game calls this thing, in the player's own language.
    pub fn name(&self, kind: Kind, id: u32) -> Option<String> {
        kind.names().iter().find_map(|table| self.get(*table, id))
    }

    /// The description printed under it, which is where a weapon says what it
    /// actually does.
    pub fn caption(&self, kind: Kind, id: u32) -> Option<String> {
        kind.captions()
            .iter()
            .find_map(|table| self.get(*table, id))
            .map(|found| without_markup(&found))
            .filter(|found| !found.is_empty())
    }

    /// The single line the menu prints for what it does.
    pub fn effect(&self, kind: Kind, id: u32) -> Option<String> {
        kind.effects()
            .iter()
            .find_map(|table| self.get(*table, id))
            .map(|found| without_markup(&found))
            .filter(|found| !found.is_empty())
    }

    /// Everything of one kind the loaded game knows about.
    ///
    /// The point of reading this rather than shipping a list: under a total
    /// conversion this is that conversion's catalogue, including the items it
    /// invented, and it is in the player's language. A list shipped with the
    /// launcher is the base game in English and is wrong twice over.
    pub fn index(&self, kind: Kind) -> Vec<(u32, String)> {
        let mut found: Vec<(u32, String)> = Vec::new();
        let mut have: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for table in kind.names() {
            for (id, name) in self.whole(*table) {
                // The tables are tried in the game's own order, so the first
                // answer for an id is the one the game would have given.
                if have.insert(id) {
                    found.push((id, name));
                }
            }
        }
        found
    }

    /// Items whose name contains what was asked for.
    ///
    /// Matched against the game's own names, which are in the language the game
    /// was installed in — so a search reaches whatever the player is reading on
    /// their screen, including the things a total conversion invented.
    pub fn find(&self, query: &str, limit: usize) -> Vec<Found> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }

        let mut found = Vec::new();
        // Places and characters as well as things carried. A model asked where
        // a boss is will otherwise translate the English name itself: the arena
        // the game calls "Воющие дюны" came back as "Стонущие дюны", which is a
        // reasonable translation of the English and not what is on the screen.
        for kind in [
            Kind::Weapon,
            Kind::Armour,
            Kind::Talisman,
            Kind::Goods,
            Kind::Place,
            Kind::Npc,
        ] {
            for (id, name) in self.index(kind) {
                let lower = name.to_lowercase();
                if !lower.contains(&needle) {
                    continue;
                }
                // An exact name beats a name that merely contains it, so
                // "Crimson Amber Medallion" is not buried under its own +1.
                let rank = if lower == needle { 0 } else { name.chars().count() };
                found.push((rank, Found { kind, id, name, effect: None, caption: None }));
            }
        }

        found.sort_by_key(|(rank, item)| (*rank, item.name.clone()));
        found
            .into_iter()
            .take(limit)
            .map(|(_, mut item)| {
                item.effect = self.effect(item.kind, item.id);
                item.caption = self.caption(item.kind, item.id);
                item
            })
            .collect()
    }

    /// Which language this copy of the game is in, judged by its own words.
    ///
    /// Worth reporting because everything else here comes back in it, and an
    /// answer that quotes an item name should quote the one on the screen.
    pub fn language(&self) -> &'static str {
        let sample: String = self
            .index(Kind::Goods)
            .iter()
            .take(60)
            .map(|(_, name)| name.as_str())
            .collect();
        let script = |range: std::ops::RangeInclusive<char>| {
            sample.chars().filter(|c| range.contains(c)).count()
        };
        let cyrillic = script('\u{0400}'..='\u{04ff}');
        let cjk = script('\u{3040}'..='\u{9fff}');
        let latin = script('a'..='z') + script('A'..='Z');
        if cyrillic > latin && cyrillic > cjk {
            "Russian"
        } else if cjk > latin {
            "Japanese or Chinese"
        } else {
            "English"
        }
    }

    /// One table, start to finish.
    ///
    /// Three reads rather than one per entry: the runs, then the whole offset
    /// array, then the whole block the strings live in. Walking a table of six
    /// thousand items across a process boundary one string at a time took long
    /// enough to feel like the launcher had stopped.
    fn whole(&self, table: u32) -> Vec<(u32, String)> {
        let Some(&fmg) = self.tables.get(&table) else {
            return Vec::new();
        };

        let head = self.process.read(fmg, 0x20);
        let Some(groups) = head.get(0x0c..0x10).and_then(four) else {
            return Vec::new();
        };
        let Some(offsets) = head.get(0x18..0x20).and_then(eight) else {
            return Vec::new();
        };
        if groups == 0 || groups > 100_000 || offsets < 0x10000 {
            return Vec::new();
        }

        let runs = self.process.read(fmg + 0x28, groups as usize * 16);
        let mut wanted: Vec<(u32, u32)> = Vec::new(); // index, id
        let mut highest = 0u32;
        for run in 0..groups as usize {
            let at = run * 16;
            let (Some(base), Some(first), Some(last)) = (
                runs.get(at..at + 4).and_then(four),
                runs.get(at + 4..at + 8).and_then(four),
                runs.get(at + 8..at + 12).and_then(four),
            ) else {
                continue;
            };
            if last < first || last - first > 1_000_000 {
                continue;
            }
            for id in first..=last {
                let index = base + (id - first);
                highest = highest.max(index);
                wanted.push((index, id));
            }
        }
        if wanted.is_empty() || highest > 2_000_000 {
            return Vec::new();
        }

        let table_bytes = self.process.read(offsets, (highest as usize + 1) * 8);
        let mut places: Vec<(u32, usize)> = Vec::with_capacity(wanted.len());
        let mut furthest = 0usize;
        for (index, id) in wanted {
            let at = index as usize * 8;
            let Some(offset) = table_bytes.get(at..at + 8).and_then(eight) else {
                continue;
            };
            if offset == 0 {
                continue;
            }
            furthest = furthest.max(offset);
            places.push((id, offset));
        }
        if places.is_empty() {
            return Vec::new();
        }

        // The strings sit inside the same allocation, at offsets from its start,
        // so one read covers all of them. The tail is for the last string.
        let span = furthest + 2048;
        if span > 64 * 1024 * 1024 {
            return Vec::new();
        }
        let blob = self.process.read(fmg, span);

        let mut out = Vec::with_capacity(places.len());
        for (id, offset) in places {
            match read_wide(&blob, offset) {
                Some(text) if !is_placeholder(&text) => out.push((id, text)),
                _ => {}
            }
        }
        out
    }

    /// One lookup, in one table.
    pub fn get(&self, table: u32, id: u32) -> Option<String> {
        if let Some(known) = self.seen.borrow().get(&(table, id)) {
            return known.clone();
        }
        let found = self.lookup(table, id).filter(|text| !is_placeholder(text));
        self.seen.borrow_mut().insert((table, id), found.clone());
        found
    }

    fn lookup(&self, table: u32, id: u32) -> Option<String> {
        let fmg = *self.tables.get(&table)?;

        // A header, then a run of {where the strings start, first id, last id}
        // sorted by id, then a table of offsets. Ids are dense inside a run and
        // absent between runs, which is why the game bisects rather than indexes.
        let head = self.process.read(fmg, 0x20);
        let groups = u32::from_le_bytes(head.get(0x0c..0x10)?.try_into().ok()?);
        let offsets = usize::from_le_bytes(head.get(0x18..0x20)?.try_into().ok()?);
        if groups == 0 || groups > 100_000 || offsets < 0x10000 {
            return None;
        }

        let runs = self.process.read(fmg + 0x28, groups as usize * 16);
        let word = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(runs.get(at..at + 4)?.try_into().ok()?))
        };

        let (mut low, mut high) = (0i64, groups as i64 - 1);
        while low <= high {
            let middle = (low + high) / 2;
            let at = middle as usize * 16;
            let first = word(at + 4)?;
            let last = word(at + 8)?;
            if id > last {
                low = middle + 1;
                continue;
            }
            if id < first {
                high = middle - 1;
                continue;
            }
            let index = word(at)?.checked_add(id - first)?;
            let entry = self.process.read(offsets + index as usize * 8, 8);
            let offset = usize::from_le_bytes(entry.try_into().ok()?);
            if offset == 0 {
                return None;
            }
            return self.wide(fmg + offset);
        }
        None
    }

    /// A string, from where it starts to its terminator.
    ///
    /// Read in one go and cut, rather than two bytes at a time: crossing a
    /// process boundary for each character of an item description would be
    /// thousands of calls for one answer.
    fn wide(&self, at: usize) -> Option<String> {
        let raw = self.process.read(at, 1024);
        if raw.len() < 2 {
            return None;
        }
        let mut units = Vec::new();
        for pair in raw.chunks_exact(2) {
            let unit = u16::from_le_bytes([pair[0], pair[1]]);
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        if units.is_empty() {
            return None;
        }
        let text = String::from_utf16_lossy(&units).trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// Ask the running game about something, by name.
///
/// Opens the process, answers, and lets go. Everything comes back owned because
/// the store cannot outlive the handle it was read through, and the caller is a
/// tool that will be turned into a sentence long after this returns.
pub fn look_up(game: crate::games::Game, query: &str, limit: usize) -> Option<Vec<Found>> {
    let pid = crate::unlock::running_pid(game.executable())?;
    let process = Process::open(pid).ok()?;
    let (base, size) = process.main_module().ok()?;
    let image = process.read(base, size);
    let text = Text::open(&process, &image, base)?;
    Some(text.find(query, limit))
}

/// Which language the running game is in, when it is running.
pub fn language(game: crate::games::Game) -> Option<&'static str> {
    let pid = crate::unlock::running_pid(game.executable())?;
    let process = Process::open(pid).ok()?;
    let (base, size) = process.main_module().ok()?;
    let image = process.read(base, size);
    Some(Text::open(&process, &image, base)?.language())
}

/// Where the game keeps the pointer to its text store.
fn find_store(process: &Process, image: &[u8], base: usize) -> Option<usize> {
    let pattern = crate::unlock::parse(GETTER);
    let mut at = 0;
    while at + GETTER_LEN <= image.len() {
        let matches = pattern
            .iter()
            .zip(&image[at..])
            .all(|(want, got)| want.is_none_or(|byte| byte == *got));
        if !matches {
            at += 1;
            continue;
        }

        // The string this would have printed. Only `MsgRepository` is ours.
        let named = i32::from_le_bytes(image.get(at + 15..at + 19)?.try_into().ok()?);
        let target = (base + at + GETTER_LEN).checked_add_signed(named as isize);
        if let Some(target) = target {
            let label = process.read(target, NAMED.len() + 1);
            if label.starts_with(NAMED) && label.get(NAMED.len()) == Some(&0) {
                let slot = i32::from_le_bytes(image.get(at + 3..at + 7)?.try_into().ok()?);
                return (base + at + 7).checked_add_signed(slot as isize);
            }
        }
        at += 1;
    }
    None
}

fn four(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn eight(bytes: &[u8]) -> Option<usize> {
    Some(usize::from_le_bytes(bytes.try_into().ok()?))
}

/// A terminated string out of a block already read.
fn read_wide(blob: &[u8], at: usize) -> Option<String> {
    let rest = blob.get(at..)?;
    let mut units = Vec::new();
    for pair in rest.chunks_exact(2) {
        let unit = u16::from_le_bytes([pair[0], pair[1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
        if units.len() > 4096 {
            break;
        }
    }
    if units.is_empty() {
        return None;
    }
    let text = String::from_utf16_lossy(&units).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn pointer(process: &Process, at: usize) -> Option<usize> {
    let bytes = process.read(at, 8);
    let value = usize::from_le_bytes(bytes.try_into().ok()?);
    (value > 0x10000 && value < 0x7fff_ffff_ffff).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_getter_pattern_is_the_shape_the_game_compiles() {
        // Sixteen bytes of pattern, and the four after the `lea` opcode carry
        // the displacement that names it — so a match has to be long enough to
        // hold them.
        let parsed = crate::unlock::parse(GETTER);
        assert_eq!(parsed.len(), 15);
        assert!(GETTER_LEN > parsed.len(), "the name is read past the match");

        // The bytes that must be exact are the instructions, not the addresses.
        assert_eq!(parsed[0], Some(0x48));
        assert_eq!(parsed[1], Some(0x8b));
        assert_eq!(parsed[2], Some(0x0d));
        assert_eq!(parsed[3], None, "the displacement is free");
        assert_eq!(parsed[7], Some(0x48), "test rcx, rcx");
        assert_eq!(parsed[12], Some(0x4c), "lea r9");
    }

    #[test]
    fn every_kind_knows_where_to_look() {
        for kind in [
            Kind::Weapon,
            Kind::Armour,
            Kind::Talisman,
            Kind::Goods,
            Kind::Place,
            Kind::Npc,
        ] {
            let names = kind.names();
            assert!(!names.is_empty(), "{kind:?} has nowhere to look");
            // Each kind reads its own tables. Two kinds sharing one would be a
            // transcription slip, and it would name armour as weapons.
            for other in [Kind::Weapon, Kind::Armour, Kind::Talisman, Kind::Goods] {
                if other != kind {
                    assert!(
                        !names.iter().any(|t| other.names().contains(t)),
                        "{kind:?} and {other:?} share a table"
                    );
                }
            }
        }
    }

    #[test]
    fn the_menus_own_markup_does_not_reach_the_answer() {
        let raw = "Защита <u>Меньшее</u> сродство\nФизическое - <font color=\"#FFA500\">Умеренно</font>";
        let plain = without_markup(raw);
        assert!(!plain.contains('<'), "got {plain:?}");
        assert!(!plain.contains("font"), "got {plain:?}");
        assert!(plain.contains("Меньшее сродство"), "got {plain:?}");
        // The menu's line breaks belong to the menu's box, not to a sentence.
        assert!(!plain.contains('\n'));
        assert!(plain.contains("сродство Физическое"), "got {plain:?}");
    }

    #[test]
    fn the_slots_nobody_filled_in_are_not_offered_as_items() {
        // Unused ids hold a marker rather than nothing, and a catalogue that
        // keeps them is four thousand items called `[ERROR]`.
        assert!(is_placeholder("[ERROR]"));
        assert!(is_placeholder("[ERROR] something"));
        assert!(is_placeholder("dummy"));
        assert!(is_placeholder("DLC dummy"));
        assert!(is_placeholder("   "));
        assert!(is_placeholder("%%%"));

        assert!(!is_placeholder("Редувия"));
        assert!(!is_placeholder("Dagger"));
        // A real name that merely mentions one of the words stays.
        assert!(!is_placeholder("Dummy's Talisman"));
    }

    #[test]
    fn the_things_a_player_can_hold_have_descriptions() {
        for kind in [Kind::Weapon, Kind::Armour, Kind::Talisman, Kind::Goods] {
            assert!(!kind.captions().is_empty(), "{kind:?} has no description");
        }
        // A place is a name and nothing else, and asking for its paragraph
        // should return an empty list rather than a wrong table.
        assert!(Kind::Place.captions().is_empty());
    }
}
