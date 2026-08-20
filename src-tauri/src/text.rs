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
use std::path::PathBuf;

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
    /// A stable name for the file this kind is written down in. Spelled out
    /// rather than derived, so renaming the variant does not silently orphan
    /// every catalogue already on disk.
    fn folder(self) -> &'static str {
        match self {
            Kind::Weapon => "weapons",
            Kind::Armour => "armour",
            Kind::Talisman => "talismans",
            Kind::Goods => "goods",
            Kind::Place => "places",
            Kind::Npc => "npcs",
        }
    }

    /// What the packed archives call this kind's table of names.
    ///
    /// Read off a real `item.msgbnd.dcx` rather than recalled — the probe that
    /// listed them is `formats::fmg::tests::show_the_packed_tables`. They are
    /// nothing like the numbers the running game uses for the same tables.
    fn table(self) -> &'static str {
        match self {
            Kind::Weapon => "WeaponName",
            Kind::Armour => "ProtectorName",
            Kind::Talisman => "AccessoryName",
            Kind::Goods => "GoodsName",
            Kind::Place => "PlaceName",
            Kind::Npc => "NpcName",
        }
    }

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

/// Every name of one kind the running game has loaded, by text id.
///
/// For the things the launcher has to know about with the game shut. A clean
/// installation keeps its text inside eleven-gigabyte archives that nothing
/// here opens, so the only chance to read it is while the game is up — this is
/// how that chance is taken, once, and written down.
pub fn every_name(game: crate::games::Game, kind: Kind) -> Option<Vec<(u32, String)>> {
    let pid = crate::unlock::running_pid(game.executable())?;
    let process = Process::open(pid).ok()?;
    let (base, size) = process.main_module().ok()?;
    let image = process.read(base, size);
    let found = Text::open(&process, &image, base)?.index(kind);
    (!found.is_empty()).then_some(found)
}

/// Writes down every kind of name the running game has loaded.
///
/// One read of the process for all six, because the expensive part is copying
/// the game's main module out of another process — a hundred megabytes — and
/// doing that once per kind would cost six times what it needs to.
///
/// Best-effort and silent. The game not being up is the ordinary case, not a
/// failure, and a catalogue that could not be written is one that will be
/// written the next time they play.
pub fn write_catalogue(app_data: &std::path::Path, game: crate::games::Game) -> usize {
    let Some(pid) = crate::unlock::running_pid(game.executable()) else {
        return 0;
    };
    let Ok(process) = Process::open(pid) else {
        return 0;
    };
    let Ok((base, size)) = process.main_module() else {
        return 0;
    };
    let image = process.read(base, size);
    let Some(text) = Text::open(&process, &image, base) else {
        return 0;
    };

    let mut written = 0;
    for kind in
        [Kind::Weapon, Kind::Armour, Kind::Talisman, Kind::Goods, Kind::Place, Kind::Npc]
    {
        let found = text.index(kind);
        if found.is_empty() {
            continue;
        }
        let at = kept_at(app_data, game, kind);
        if let Some(parent) = at.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(&found) {
            if std::fs::write(&at, bytes).is_ok() {
                written += found.len();
            }
        }
    }
    written
}

/// Where a kind's names are written down between runs.
fn kept_at(app_data: &std::path::Path, game: crate::games::Game, kind: Kind) -> PathBuf {
    app_data
        .join("catalogue")
        .join(game.appdata_folder())
        .join(format!("{}.json", kind.folder()))
}

/// The names of one kind, from the running game when it is up and from what was
/// written down when it is not.
///
/// The comment above `every_name` has always said the chance is "taken once and
/// written down". It was not: the read happened and nothing was kept, so a
/// player on a plain installation had a catalogue of exactly nothing the moment
/// they shut the game — measured, 0 names against a modded installation's
/// 10,922. Everything built on those names went with it: the talisman list, the
/// upgrade materials, every lookup by name.
///
/// Three sources, in the order they are worth having. The running game is best:
/// it is the text on the player's own screen, mod and all. What was written
/// down last time is the same text, one launch stale. The game's own archives
/// are last — they are whatever shipped rather than what is installed, but they
/// are there on an installation nobody has played yet, where the other two have
/// nothing at all.
/// One line of the game's own writing, and what kind of writing it is.
#[derive(Debug, Clone)]
pub struct Written {
    /// In the player's words, not the file's: "a tutorial", "a menu entry".
    pub sort: &'static str,
    /// The table it came out of, kept for anyone debugging this.
    pub table: String,
    pub id: u32,
    pub said: String,
}

/// What a table of strings actually is, said the way a player would say it.
///
/// `None` for the ones nothing good comes of searching. `BloodMsg` is the
/// largest table in the game — 9,692 strings — and every one of them is a
/// message some stranger scrawled on the ground out of a fixed vocabulary;
/// searching it for "bleed" returns four hundred people writing "try bleed
/// ahead". It would drown everything worth finding.
fn what_sort(table: &str) -> Option<&'static str> {
    Some(match table {
        "WeaponName" => "a weapon",
        "WeaponCaption" | "WeaponEffect" => "a weapon's description",
        "ProtectorName" => "a piece of armour",
        "ProtectorCaption" | "ProtectorInfo" => "armour's description",
        "AccessoryName" => "a talisman",
        "AccessoryCaption" | "AccessoryInfo" => "a talisman's description",
        "GoodsName" => "an item",
        "GoodsCaption" | "GoodsInfo" | "GoodsInfo2" | "GoodsDialog" => "an item's description",
        "GemName" => "an ash of war",
        "GemCaption" | "GemInfo" => "an ash of war's description",
        "ArtsName" => "a skill",
        "ArtsCaption" => "a skill's description",
        "PlaceName" => "a place",
        "NpcName" => "somebody's name",
        "TutorialTitle" | "TutorialBody" => "a tutorial",
        "GR_MenuText" | "GR_LineHelp" | "GR_KeyGuide" | "ActionButtonText" => "a menu entry",
        "GR_Dialogues" | "GR_System_Message_win64" => "something the game says",
        "TalkMsg" | "EventTextForTalk" => "something somebody says",
        "EventTextForMap" => "a note on the map",
        "LoadingTitle" | "LoadingText" => "a loading screen",
        _ => return None,
    })
}

/// Everything the installed game has written down, searched.
///
/// The launcher used to read six tables — the ones holding NAMES — out of the
/// forty-four the game ships, and throw the rest away after loading them. That
/// is 12,000 strings kept out of 54,178, and the missing four fifths are the
/// part that explains anything: every item's description, every tutorial, every
/// menu entry, everything an NPC says.
///
/// What that cost, exactly. A player standing at a grace, looking at
/// "Трансмогрификация брони" on their own screen, asked what it was. The answer
/// was that no article covered it and they should try a forum. Their
/// installation holds a tutorial explaining it in full, under
/// `TutorialBody`, and a matching item description — both in Russian, both
/// written by whoever built the conversion, both invisible to every tool here.
///
/// Matched on a lowercase substring rather than a word, because these languages
/// inflect and a whole word finds nothing: searching the Russian for "вера"
/// misses "веру" and "веры", which is the failure that shaped the talisman
/// search too.
pub fn search(
    game_dir: &std::path::Path,
    mod_dir: Option<&std::path::Path>,
    language: &str,
    wanted: &str,
) -> Vec<Written> {
    let asked = wanted.trim().to_lowercase();
    if asked.chars().count() < 2 {
        return Vec::new();
    }
    let tables = crate::library::tables_for(game_dir, mod_dir, language);

    // The word as asked, then its stem, then a shorter stem.
    //
    // Told to search a fragment, a model searches a whole word — and a whole
    // word misses its own plural. Asked to explain "Красители доспехов", one
    // searched "Краситель" and got nothing: the game writes "Красители", and
    // "краситель" is not inside "красители" because the last letter differs.
    // One letter, and the tutorial explaining the thing stayed invisible.
    //
    // So the trimming happens here rather than being asked for. Only when the
    // whole word found nothing, and never below four characters, where a stem
    // stops being a word and starts being a syllable that matches everything.
    for cut in 0..=2 {
        let wanted: String = {
            let letters: Vec<char> = asked.chars().collect();
            // The guard is on what TRIMMING would leave, never on what was
            // asked for: a two-character query is theirs to make and this
            // refusing it made the whole search return nothing.
            if cut > 0 && letters.len().saturating_sub(cut) < 4 {
                break;
            }
            letters[..letters.len().saturating_sub(cut)].iter().collect()
        };
        let found = matching(&tables, &wanted);
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// The same search, saying whether it had to shorten the word to find anything.
///
/// The trimming above is right and it has a cost that was invisible. A word
/// this game does not contain gets cut down until some OTHER word shares the
/// stem, and those come back looking like an answer: searching the Russian for
/// "костер" — bonfire, which is Dark Souls and is not in this game — finds
/// nothing, is trimmed to "косте", and returns twenty-five hits about BONES
/// ("Болт из Костей Заразы") and containers ("Ёмкостей при себе").
///
/// So "nothing here" quietly became "here are twenty-five things", and a model
/// reading them has every reason to believe the thing exists. That is fuel for
/// exactly the invention this launcher spends its time catching.
///
/// `Some(stem)` when the hits came from a shortened form, so the caller can say
/// so. `None` when the word was found as asked.
pub fn search_saying_how(
    game_dir: &std::path::Path,
    mod_dir: Option<&std::path::Path>,
    language: &str,
    wanted: &str,
) -> (Vec<Written>, Option<String>) {
    let asked = wanted.trim().to_lowercase();
    if asked.chars().count() < 2 {
        return (Vec::new(), None);
    }
    let tables = crate::library::tables_for(game_dir, mod_dir, language);
    for cut in 0..=2 {
        let shortened: String = {
            let letters: Vec<char> = asked.chars().collect();
            if cut > 0 && letters.len().saturating_sub(cut) < 4 {
                break;
            }
            letters[..letters.len().saturating_sub(cut)].iter().collect()
        };
        let found = matching(&tables, &shortened);
        if !found.is_empty() {
            return (found, (cut > 0).then_some(shortened));
        }
    }
    (Vec::new(), None)
}

/// One pass of the search, over every table worth reading.
fn matching(
    tables: &std::collections::HashMap<String, crate::formats::fmg::Strings>,
    wanted: &str,
) -> Vec<Written> {
    let mut found: Vec<Written> = Vec::new();
    for (table, strings) in tables {
        let Some(sort) = what_sort(table) else {
            continue;
        };
        for (id, said) in strings.iter() {
            if said.to_lowercase().contains(wanted) {
                found.push(Written { sort, table: table.clone(), id: *id, said: said.clone() });
            }
        }
    }
    // A name before the paragraph describing it, and the shortest first inside
    // each — a hit in a four-word menu entry is about that entry, where a hit
    // in a page of dialogue may be an aside.
    found.sort_by(|a, b| {
        let rank = |sort: &str| usize::from(sort.contains("description") || sort.contains("says"));
        rank(a.sort).cmp(&rank(b.sort)).then_with(|| a.said.len().cmp(&b.said.len()))
    });
    found
}

pub fn names(
    app_data: &std::path::Path,
    game: crate::games::Game,
    game_dir: Option<&std::path::Path>,
    mod_dir: Option<&std::path::Path>,
    kind: Kind,
) -> Vec<(u32, String)> {
    let write_down = |found: &Vec<(u32, String)>| {
        let at = kept_at(app_data, game, kind);
        if let Some(parent) = at.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_vec(found) {
            let _ = std::fs::write(&at, text);
        }
    };

    if let Some(fresh) = every_name(game, kind) {
        // Written every time rather than only when missing: a mod can be
        // installed or removed between runs, and the newest read is the one
        // that matches what they will see on screen.
        write_down(&fresh);
        return fresh;
    }

    let kept: Vec<(u32, String)> = std::fs::read(kept_at(app_data, game, kind))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if !kept.is_empty() {
        return kept;
    }

    // Nothing running and nothing written down, which is every installation
    // nobody has played yet. The text is still on the disk: a total conversion
    // keeps its own loose, an untouched game keeps the original packed, and
    // `library` knows the order to try them in. Reading only the packed ones
    // would have given a modded player the base game's names, which is worse
    // than none — so the mod case was skipped entirely, and that left it with
    // nothing at all.
    let Some(game_dir) = game_dir else {
        return Vec::new();
    };
    let found = from_the_disk(game_dir, mod_dir, kind);
    if !found.is_empty() {
        write_down(&found);
    }
    found
}

/// One kind's names, off the disk: the mod's own text, the game's, or the
/// archives the game keeps its own inside.
///
/// The last resort and the slowest, because a packed index has to be unlocked
/// before anything can be found in it — but it is the only one that answers on
/// an installation nobody has launched, where there is no process to read and
/// nothing was ever written down.
fn from_the_disk(
    game_dir: &std::path::Path,
    mod_dir: Option<&std::path::Path>,
    kind: Kind,
) -> Vec<(u32, String)> {
    // Whatever the copy is set to. A repack keeps it in an emulator's config,
    // which is readable with the game shut; a Steam copy keeps it in Steam,
    // which is not, and English is the game's own default.
    let locale = crate::language::status(game_dir)
        .current
        .as_deref()
        .and_then(crate::language::locale_folder)
        .unwrap_or("engus");

    let tables = crate::library::tables_for(game_dir, mod_dir, locale);
    let Some(found) = tables.get(kind.table()) else {
        return Vec::new();
    };
    // The same filter the catalogue uses, because these tables are full of the
    // studio's own scaffolding and the process read never shows it: reading off
    // the disk turned up an entry numbered 0 called "DLC dummy", and a bestiary
    // that names something that is not a thing is worse than a quiet one.
    let mut out: Vec<(u32, String)> = found
        .iter()
        .map(|(id, text)| (*id, crate::library::plain(text)))
        .filter(|(id, name)| *id > 0 && crate::library::worth_naming(name))
        .collect();
    out.sort();
    out
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

    /// A word this game does not have must not come back looking like it does.
    ///
    /// The case that exposed it, and it is a real one. An answer offered a
    /// player "костры" — bonfires — as places that refill flasks. There are no
    /// bonfires in ELDEN RING; they are Dark Souls. Searching for the word
    /// finds nothing, so the search trims it to "косте" and returns
    /// twenty-five lines about BONES ("Болт из Костей Заразы") and containers
    /// ("Ёмкостей при себе"). Handed over bare those read as confirmation.
    ///
    /// The trimming itself is right and must stay — it is what finds
    /// "Красители" when a model searched "Краситель". What was missing is the
    /// caller being told which of the two happened.
    #[test]
    fn a_shortened_word_says_that_it_was_shortened() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");

        // Only meaningful on a Russian copy; elsewhere the word is absent
        // outright, which is also a correct answer and not this test's business.
        let (found, shortened) =
            search_saying_how(&game_dir, mod_dir.as_deref(), language, "костер");
        if !found.is_empty() {
            assert_eq!(
                shortened.as_deref(),
                Some("косте"),
                "hits for a word this game lacks must name the stem they really matched"
            );
        }

        // And a word the game DOES contain comes back with nothing to declare.
        // Whichever language this copy is in, one of these is in it.
        for real in ["рун", "rune"] {
            let (found, shortened) =
                search_saying_how(&game_dir, mod_dir.as_deref(), language, real);
            if !found.is_empty() {
                assert_eq!(
                    shortened, None,
                    "{real:?} is in the game, so nothing should have been shortened"
                );
            }
        }
    }

    /// Do the words from a DIFFERENT FromSoft game appear in this one?
    ///
    /// Run with `--ignored --nocapture`. Written because an answer offered a
    /// player "костры, святилища, NPC" as places that refill flasks. There are
    /// no bonfires in ELDEN RING — they are Dark Souls — and none of the checks
    /// in `ask.rs` can see it: `ungrounded_names` looks for capitalised item
    /// names, and these are lowercase common nouns.
    ///
    /// Before writing a check around a list like that, the list has to be
    /// EARNED rather than recalled. This asks the installation itself. A word
    /// that genuinely appears in the game's own writing must never go on such
    /// a list, however sure anybody is.
    #[test]
    #[ignore = "a survey, not an assertion"]
    fn show_whether_another_game_s_words_are_in_this_one() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");

        // Both alphabets, because the installation may be in either.
        for word in [
            "костёр", "костер", "bonfire", "эстус", "estus", "человечность",
            "humanity", "ember", "уголёк", "прозрение", "insight", "эхо крови",
            "blood echo", "святилищ", "shrine",
        ] {
            let found = search(&game_dir, mod_dir.as_deref(), language, word);
            println!("  {word:<14} {} hits", found.len());
            for one in found.iter().take(4) {
                println!("        {} · {}", one.sort, one.said.chars().take(70).collect::<String>());
            }
        }
    }

    /// The game's own writing is searchable, all of it.
    ///
    /// Pinned on the question that exposed the hole. A player standing at a
    /// grace, reading "Трансмогрификация брони" off their own screen, asked
    /// what it was and was told no article covered it and to try a forum. Their
    /// installation holds a tutorial explaining it and an item description
    /// mentioning it, and neither was reachable because only the six NAME
    /// tables were ever looked at.
    #[test]
    fn the_game_s_own_writing_answers_for_itself() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");

        // Whatever language this copy is in, one of these pairs is common in
        // it. A single letter would do but is refused on purpose — one
        // character matches most of the game and is never a real question.
        let anything = ["ра", "er", "an", "ой", "in", "の", "de"]
            .iter()
            .map(|probe| search(&game_dir, mod_dir.as_deref(), language, probe))
            .max_by_key(Vec::len)
            .unwrap_or_default();
        assert!(
            anything.len() > 500,
            "the commonest pair in this language matched only {}",
            anything.len()
        );

        // The kinds have to be named in the player's terms, not the file's.
        assert!(
            anything.iter().any(|one| one.sort.contains("description")),
            "no descriptions came back, so only the name tables are being read"
        );

        // And the noisiest table in the game stays out: 9,692 messages
        // scrawled on the ground would drown every real answer.
        assert!(
            !anything.iter().any(|one| one.table == "BloodMsg"),
            "the ground messages got in"
        );

        // Too short a query finds nothing rather than everything.
        assert!(search(&game_dir, mod_dir.as_deref(), language, "e").is_empty());
        assert!(search(&game_dir, mod_dir.as_deref(), language, "").is_empty());

        // The question itself, when this installation is the one it was asked
        // on. Skipped elsewhere, because a plain copy has no such mod.
        let transmog = search(&game_dir, mod_dir.as_deref(), language, "рансмогрификаци");
        if !transmog.is_empty() {
            assert!(
                transmog.iter().any(|one| one.sort == "a tutorial"),
                "the tutorial explaining it did not come back: {transmog:#?}"
            );

            // And the word as somebody would actually type it, in the singular
            // the game does not use. "Краситель" is not inside "Красители" —
            // one letter — and that one letter hid the tutorial explaining it
            // from a question that asked for it by name.
            let singular = search(&game_dir, mod_dir.as_deref(), language, "Краситель");
            assert!(
                !singular.is_empty(),
                "the singular found nothing, so the stem is not being tried"
            );
        }
    }

    /// Every text table the game ships, and how much is in each.
    ///
    /// Six of them are read — the six that hold NAMES. Everything else the game
    /// has written down is loaded into memory by `tables_for` and then thrown
    /// away, which is why asking what a menu entry means gets nowhere: the
    /// player was looking at "Трансмогрификация брони" on their own screen,
    /// asked what it was, and the answer was that no article covered it.
    ///
    /// `cargo test --lib show_text_tables -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_menu_words_for_resistance() {
        // The four resistances the menu shows are Immunity, Robustness, Focus
        // and Vitality, and a player asks for them by the word their own menu
        // uses. Guessing those words is how "Robustheit" nearly got filed as
        // poise; German poise is Haltung and Robustness is bleed and frost.
        // So: ask the game.
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        println!("\n  language: {language}\n");
        let tables = crate::library::tables_for(&game_dir, mod_dir.as_deref(), language);
        let menu = tables.get("GR_MenuText").expect("the menu has words");
        for id in [10395u32..10410, 10530..10560, 11180..11210] {
            for one in id {
                if let Some(said) = menu.get(&one) {
                    let flat = said.replace('\n', " / ");
                    if !flat.trim().is_empty() && flat.chars().count() < 70 {
                        println!("  #{one:<7} {flat}");
                    }
                }
            }
            println!();
        }

        // Which of the four is which. The menu gives the words in the order the
        // equipment screen shows them, and that order is settled by the four
        // above them: the game lists physical, strike, slash, pierce, while the
        // table stores physical, slash, strike, pierce. So the ids follow the
        // SCREEN, not the table — which fixes the four resistances as immunity,
        // robustness, focus, vitality in id order. The descriptions below are
        // the second opinion: whatever ailment a word is quoted beside is the
        // ailment it covers.
        for word in ["Иммунитет", "Живучесть", "Концентрац", "Физ. мощь", "Баланс"] {
            println!("\n  === {word}");
            for hit in search(&game_dir, mod_dir.as_deref(), language, word)
                .iter()
                .filter(|hit| hit.said.chars().count() > word.chars().count() + 8)
                .take(6)
            {
                let said: String =
                    hit.said.replace('\n', " ").chars().take(150).collect();
                println!("      [{}] {said}", hit.sort);
            }
        }
    }

    #[test]
    #[ignore = "a probe, not a check"]
    fn show_text_tables() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");

        let tables = crate::library::tables_for(&game_dir, mod_dir.as_deref(), language);
        let read: Vec<&str> =
            [Kind::Weapon, Kind::Armour, Kind::Talisman, Kind::Goods, Kind::Place, Kind::Npc]
                .iter()
                .map(|kind| kind.table())
                .collect();

        let mut sizes: Vec<(usize, &String)> =
            tables.iter().map(|(name, strings)| (strings.len(), name)).collect();
        sizes.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
        let whole: usize = sizes.iter().map(|(size, _)| size).sum();
        println!("\n  {} tables, {whole} strings in all, in {language}\n", sizes.len());
        for (size, name) in &sizes {
            let mark = if read.contains(&name.as_str()) { " <- read" } else { "" };
            println!("  {size:>6}  {name}{mark}");
        }

        // And the thing that started this: where the words on the player's own
        // screen actually live.
        for word in ["рансмогрификаци", "ransmogrif"] {
            println!("\n  --- \"{word}\" ---");
            for (name, strings) in &tables {
                for (id, said) in strings.iter() {
                    if said.contains(word) {
                        println!("    {name} [{id}] {}", said.chars().take(90).collect::<String>());
                    }
                }
            }
        }
    }

    /// The names an installation nobody has played still has.
    ///
    /// What makes this worth more than "something came back" is the second
    /// half: the ids out of the archive have to be ids the running game knows.
    /// A wrong table, a wrong locale or a misread FMG all produce a list of
    /// plausible strings under numbers that match nothing.
    #[test]
    fn the_archives_carry_names_under_the_game_s_own_numbers() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        crate::formats::oodle::register(&game_dir);
        if !crate::formats::oodle::available() {
            return;
        }

        let packed = from_the_disk(&game_dir, None, Kind::Npc);
        if packed.is_empty() {
            // An installation whose archives have been unpacked has none.
            return;
        }
        assert!(packed.len() > 100, "only {} names in the archive", packed.len());

        // And with a mod, which is the case that used to give nothing at all:
        // the archive path was skipped so the base game's names could not be
        // passed off as the mod's, and skipping it left the mod with none.
        if let Some(mod_dir) = crate::testing::mod_dir(game) {
            let modded = from_the_disk(&game_dir, Some(&mod_dir), Kind::Npc);
            assert!(
                modded.len() > 100,
                "a modded installation read {} names off the disk",
                modded.len()
            );
            for (id, name) in &modded {
                assert!(*id > 0 && !name.trim().is_empty(), "id {id} is {name:?}");
            }
        }
        for (id, name) in &packed {
            assert!(*id > 0, "a name under id {id}");
            assert!(!name.trim().is_empty(), "id {id} has an empty name");
        }

        // Against the process, which is a different reader of different bytes.
        // A total conversion renames things; it does not renumber them, so the
        // names may differ and the numbers may not.
        let Some(live) = every_name(game, Kind::Npc) else {
            return;
        };
        let known: std::collections::HashSet<u32> = live.iter().map(|(id, _)| *id).collect();
        let shared = packed.iter().filter(|(id, _)| known.contains(id)).count();
        assert!(
            shared * 2 > packed.len(),
            "only {shared} of {} archive ids are ids the game knows",
            packed.len()
        );
    }

    /// A name map keyed by id is keyed by id.
    ///
    /// One of these was built as `(0u32, name)` for every entry — every name
    /// under the same key, so the map held exactly one and no spell could be
    /// named. It was invisible while the game was running, because that branch
    /// never ran, and total the moment it was not. What catches that shape is
    /// counting the keys against the entries, which no assertion about "did we
    /// get something back" would.
    #[test]
    fn a_map_of_names_has_as_many_keys_as_names() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        crate::formats::oodle::register(&game_dir);
        let mod_dir = crate::testing::mod_dir(game);
        let app_data = crate::testing::app_data().unwrap_or_default();

        for kind in [Kind::Goods, Kind::Weapon, Kind::Armour, Kind::Talisman] {
            let read = names(&app_data, game, Some(&game_dir), mod_dir.as_deref(), kind);
            if read.is_empty() {
                continue;
            }
            let keys: std::collections::HashSet<u32> = read.iter().map(|(id, _)| *id).collect();
            assert!(read.len() > 20, "{kind:?} came back with only {}", read.len());
            // Not one key for everything, and not a handful either: ids are
            // what the tables join on and duplicates lose entries silently.
            assert_eq!(
                keys.len(),
                read.len(),
                "{kind:?} has {} names under {} ids",
                read.len(),
                keys.len()
            );
            assert!(!keys.contains(&0), "{kind:?} has something under id 0");
        }
    }

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
