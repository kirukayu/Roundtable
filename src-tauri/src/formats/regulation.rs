//! What the installed mod actually did to the game's numbers.
//!
//! `regulation.bin` holds every table the game balances itself with — a
//! weapon's damage, its scaling, what it asks of you before you can hold it. A
//! total conversion ships its own, which is why every wiki figure is wrong for
//! somebody playing one: The Convergence turns Reduvia from a physical dagger
//! that scales on dexterity into a fire one that scales on faith, and no page
//! anywhere says so.
//!
//! Read from the file rather than from the running game. The game keeps the
//! same tables in memory, but reaching them means going through the anti-tamper
//! layer the accessors sit behind; the file needs nothing running at all.
//!
//! Four layers, and each was settled against real bytes before it was written:
//!
//! ```text
//! regulation.bin  AES-256-CBC, the first sixteen bytes are the IV
//!   └ DCX         a wrapper, compressed with Zstandard
//!     └ BND4      an archive of one file per table
//!       └ PARAM   a header, a list of rows, and the rows
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use crate::error::{Error, IoContext, Result};

/// The key ELDEN RING encrypts its regulation with.
///
/// Public, and the same in every copy of the game — it is in SoulsFormats,
/// which is what every mod tool uses. Quoted from there rather than recalled:
/// a key remembered wrong decrypts to noise, and noise reads exactly like a bug
/// in the parser for an hour before the key is suspected.
const KEY: [u8; 32] = [
    0x99, 0xBF, 0xFC, 0x36, 0x6A, 0x6B, 0xC8, 0xC6, 0xF5, 0x82, 0x7D, 0x09, 0x36, 0x02, 0xD6, 0x76,
    0xC4, 0x28, 0x92, 0xA0, 0x1C, 0x20, 0x7F, 0xB0, 0x24, 0xD3, 0xAF, 0x4E, 0x49, 0x3F, 0xEF, 0x99,
];

/// One table, as the archive stores it.
pub struct Table {
    /// Where each row's data begins, by the row's id.
    rows: HashMap<i64, usize>,
    bytes: Vec<u8>,
}

impl Table {
    pub fn ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.rows.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn has(&self, id: i64) -> bool {
        self.rows.contains_key(&id)
    }

    /// A field of one row, by the offset it sits at.
    ///
    /// Everything returns `None` rather than a default: a field read past the
    /// end of a row would otherwise arrive as a zero and be quoted as a fact.
    pub fn f32(&self, id: i64, at: usize) -> Option<f32> {
        let start = *self.rows.get(&id)? + at;
        let bytes = self.bytes.get(start..start + 4)?;
        Some(f32::from_le_bytes(bytes.try_into().ok()?))
    }

    pub fn u16(&self, id: i64, at: usize) -> Option<u16> {
        let start = *self.rows.get(&id)? + at;
        let bytes = self.bytes.get(start..start + 2)?;
        Some(u16::from_le_bytes(bytes.try_into().ok()?))
    }

    pub fn u8(&self, id: i64, at: usize) -> Option<u8> {
        let start = *self.rows.get(&id)? + at;
        self.bytes.get(start).copied()
    }

    /// Signed, for the fields where -1 means "none" rather than 65535.
    pub fn i16(&self, id: i64, at: usize) -> Option<i16> {
        let start = *self.rows.get(&id)? + at;
        let bytes = self.bytes.get(start..start + 2)?;
        Some(i16::from_le_bytes(bytes.try_into().ok()?))
    }

    pub fn i32(&self, id: i64, at: usize) -> Option<i32> {
        let start = *self.rows.get(&id)? + at;
        let bytes = self.bytes.get(start..start + 4)?;
        Some(i32::from_le_bytes(bytes.try_into().ok()?))
    }
}

/// Where each thing a weapon does sits inside its row.
///
/// Worked out from the game's own field definitions and then checked against a
/// weapon known from three other places — see `assets/param-layout.md`. The
/// tables still call arcane "luck", from an older game.
pub mod weapon {
    pub const WEIGHT: usize = 0x010;
    pub const SCALE_STRENGTH: usize = 0x024;
    pub const SCALE_DEXTERITY: usize = 0x028;
    pub const SCALE_INTELLIGENCE: usize = 0x02c;
    pub const SCALE_FAITH: usize = 0x030;
    pub const SCALE_ARCANE: usize = 0x19c;
    pub const PHYSICAL: usize = 0x0c8;
    pub const MAGIC: usize = 0x0ca;
    pub const FIRE: usize = 0x0cc;
    pub const LIGHTNING: usize = 0x0ce;
    pub const HOLY: usize = 0x18c;
    pub const STAMINA: usize = 0x0d0;
    pub const NEEDS_STRENGTH: usize = 0x0f2;
    pub const NEEDS_DEXTERITY: usize = 0x0f3;
    pub const NEEDS_INTELLIGENCE: usize = 0x0f4;
    pub const NEEDS_FAITH: usize = 0x0f5;
    pub const NEEDS_ARCANE: usize = 0x195;
    pub const REGAIN_HP: usize = 0x242;
    /// `staminaGuardDef`, the guard boost the equipment screen prints. See
    /// [`super::Weapon::boost`] for how it was settled.
    pub const GUARD_BOOST: usize = 0x0d8;
    pub const REINFORCE_TYPE: usize = 0x0da;
    /// The base a reinforce row's own `materialSetId` is added to. See `mtrl`.
    pub const MATERIAL_SET: usize = 0x05c;
    /// Which row of `AttackElementCorrectParam` says what corrects what.
    pub const ELEMENT_CORRECT: usize = 0x22c;
    /// Which row of `SwordArtsParam` is the skill on it — an ash of war, or the
    /// weapon's own art where it cannot be changed.
    ///
    /// It sits in the only gap the layout leaves: `NEEDS_ARCANE` is one byte at
    /// 0x195, `SCALE_ARCANE` is four at 0x19c, and a four-byte field at 0x198
    /// ends exactly where the latter begins.
    pub const SKILL: usize = 0x198;
    /// Scaling per stat, in the order every one of these tables keeps: STR,
    /// DEX, INT, FTH, ARC. The tables still call arcane "luck".
    pub const CORRECT: [usize; 5] = [0x024, 0x028, 0x02c, 0x030, 0x19c];
    /// Each damage type: what it is called, its base, the reinforce rate that
    /// multiplies it, and the `correctType_*` picking its curve.
    pub const DAMAGE: [(&str, usize, usize, usize); 5] = [
        ("physical", 0x0c8, 0x00, 0x0ec),
        ("magic", 0x0ca, 0x04, 0x17d),
        ("fire", 0x0cc, 0x08, 0x17e),
        ("lightning", 0x0ce, 0x0c, 0x17f),
        ("holy", 0x18c, 0x58, 0x18e),
    ];
}

/// What the upgrade level multiplies a weapon's scaling by, per stat, in the
/// same STR, DEX, INT, FTH, ARC order as `weapon::CORRECT`.
pub mod reinforce {
    pub const CORRECT_RATE: [usize; 5] = [0x1c, 0x20, 0x24, 0x28, 0x60];
}

/// What killing something gives, in `ItemLotParam_enemy`. Rows are 184 bytes.
///
/// Eight slots, each a flat array: the ids first, then the categories, then the
/// weights, and the counts much later. A slot with id 0 is an empty one, and
/// the first slot of almost every row is exactly that — it is the "nothing"
/// outcome, and its weight is how likely nothing is.
pub mod lot {
    pub const SLOTS: usize = 8;
    pub const ITEM: usize = 0x00;
    pub const CATEGORY: usize = 0x20;
    /// Not a percentage. Its share of the row's total is.
    pub const WEIGHT: usize = 0x40;
    pub const COUNT: usize = 0x8a;
}

/// Which table a lot's `category` points into.
///
/// Not recalled — derived. Every (category, id) pair in the whole lot table was
/// asked of all five equipment tables, because one lookup proves nothing when
/// the id spaces overlap. Three settled themselves outright: category 2 is a
/// live weapon 100% of the time and nothing else 0%, category 1 is goods 99%,
/// category 3 protectors 95%.
///
/// The last two could not be settled that way — each is a single id that two
/// tables both have — so the game's own text settled them instead. Id 6070 is
/// "Sacrificial Twig" as a talisman and something unrelated as goods; id 20900
/// is "Ash of War: Gravitas" as a gem. Both readings are corroborated by the
/// structure: distinct category numbers exist to name distinct tables, and
/// goods was already taken by category 1.
pub fn category_table(category: i32) -> Option<&'static str> {
    Some(match category {
        1 => "EquipParamGoods",
        2 => "EquipParamWeapon",
        3 => "EquipParamProtector",
        4 => "EquipParamAccessory",
        5 => "EquipParamGem",
        _ => return None,
    })
}

/// The word the rest of the launcher uses for what a table holds.
fn what_a(table: &str) -> &'static str {
    match table {
        "EquipParamWeapon" => "weapon",
        "EquipParamProtector" => "armour",
        "EquipParamAccessory" => "talisman",
        // The same words `crate::library` uses, so a caller can join a drop to
        // a name without a second mapping to keep in step.
        "EquipParamGem" => "ash of war",
        _ => "item",
    }
}

/// Where the drop and the figures sit in an `NpcParam` row. The rest of them
/// are in `crate::bestiary`, which is what reads this table for names.
mod npc {
    /// `itemLotId_enemy`, four bytes past `getSoul` at 0x2c.
    pub const DROPS: usize = 0x30;

    /// The eight damage-cut rates, in the order the row keeps them.
    ///
    /// Found by fingerprint rather than by counting three hundred fields down
    /// a list, which is how two offsets went wrong before: a second reader gave
    /// row 34702468 as 1.1, 1.1, 0.9, 1.1, 1.0, 0.6, 1.2, 1.4, and exactly one
    /// place in the row holds those eight floats in that order.
    ///
    /// A rate is what gets THROUGH, the same convention the armour reader was
    /// verified against — 0.6 means it takes six tenths of that kind and 1.4
    /// means it takes half again as much.
    pub const TAKES: [(&str, usize); 8] = [
        ("physical", 0x1a4),
        ("slash", 0x1a8),
        ("strike", 0x1ac),
        ("pierce", 0x1b0),
        ("magic", 0x1b4),
        ("fire", 0x1b8),
        ("lightning", 0x1bc),
        ("holy", 0x1c0),
    ];
}

/// One thing something can drop.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drop {
    /// The row in whichever table `kind` names.
    pub id: i64,
    /// "weapon", "armour", "talisman", "item" — the word the launcher uses
    /// elsewhere, so a caller can join it to a name without a second mapping.
    pub kind: String,
    /// How many at once.
    pub count: u8,
    /// Out of a hundred, worked out here because a share of a total is exactly
    /// the arithmetic that goes wrong when it is left to be described.
    pub chance: f32,
}

/// A skill, in `SwordArtsParam`. Rows are 32 bytes.
///
/// Worked out from the game's own field definition and checked against it:
/// four bytes of `disableParam` flags, then four one-byte fields, then four
/// more — with `reserve2` a `dummy8` carrying a bit size, which pads the byte
/// the grey-out flags are in rather than taking one of its own. Miss that and
/// everything from `textId` on reads one byte late, which is the same trap that
/// cost two readings elsewhere in this file.
pub mod skill {
    pub const TEXT: usize = 0x0c;
    /// What one press costs, by the button it is on: L1, L2, R1, R2. A skill
    /// that cannot be used that way carries -1, which is not a cost.
    pub const COSTS: [(&str, usize); 4] =
        [("L1", 0x10), ("L2", 0x12), ("R1", 0x14), ("R2", 0x16)];
}

/// What upgrading multiplies, in `ReinforceParamWeapon`.
///
/// One row per level: the row for a weapon at +N is its `reinforceTypeId` plus
/// N. The names in the table are the old ones from Dark Souls — agility for
/// dexterity, magic for intelligence, luck for arcane, dark for holy — and are
/// given here under the names this game uses.
pub mod sharpen {
    pub const PHYSICAL: usize = 0x00;
    pub const MAGIC: usize = 0x04;
    pub const FIRE: usize = 0x08;
    pub const LIGHTNING: usize = 0x0c;
    pub const HOLY: usize = 0x58;
    pub const SCALE_STRENGTH: usize = 0x1c;
    pub const SCALE_DEXTERITY: usize = 0x20;
    pub const SCALE_INTELLIGENCE: usize = 0x24;
    pub const SCALE_FAITH: usize = 0x28;
    pub const SCALE_ARCANE: usize = 0x60;
}

/// Field offsets in `EquipParamProtector`, worked out the same way.
///
/// This game does not give armour flat defence — `defensePhysics` and its
/// fellows are left over from the older ones and read zero. What the menu shows
/// as damage negation is a multiplier: `0.9` means nine-tenths of the damage
/// gets through, so the number on screen is a tenth off. That inversion is the
/// whole reason these are read here rather than quoted raw.
pub mod armour {
    pub const WEIGHT: usize = 0x024;
    pub const PHYSICAL: usize = 0x0e4;
    pub const SLASH: usize = 0x0e8;
    pub const STRIKE: usize = 0x0ec;
    pub const PIERCE: usize = 0x0f0;
    pub const MAGIC: usize = 0x0f4;
    pub const FIRE: usize = 0x0f8;
    pub const LIGHTNING: usize = 0x0fc;
    pub const HOLY: usize = 0x11c;
    // The four the menu shows. The table keeps a field per ailment and the
    // members of a group hold the same number — poison and rot both read the
    // immunity figure — so one of each is taken and the rest would be the same
    // number under four more names.
    //
    // Checked against SmithBox's field list, which names them: these four land
    // on `resistPoison`, `resistBlood`, `resistSleep` and `resistCurse`, in that
    // order, and their partners `resistDisease`, `resistFreeze`, `resistMadness`
    // sit alongside. That is the grouping the menu shows.
    /// The three `SpEffectParam` rows a piece carries. Same machinery as a
    /// talisman's, and in this conversion 836 of 841 pieces use it.
    pub const RESIDENT: [usize; 3] = [0x028, 0x02c, 0x030];
    pub const IMMUNITY: usize = 0x0c4;
    pub const ROBUSTNESS: usize = 0x0c8;
    pub const FOCUS: usize = 0x00c;
    pub const VITALITY: usize = 0x0ca;
}

/// What a talisman actually does, in numbers.
///
/// Asked what Radagon's Soreseal gives and what it costs, the launcher had its
/// name, its weight and the sentence under it — "raises vigour, endurance,
/// strength and dexterity, and raises damage taken" — and not one figure. Which
/// four, by how much, and how much more damage, is the entire question. All of
/// it was in the tables and none of it was read.
///
/// `EquipParamAccessory` holds the talismans; `refId` points at the
/// `SpEffectParam` row that carries the arithmetic.
///
/// The offsets come from walking the paramdefs rather than hunting bytes —
/// see `scratchpad/paramwalk.py` and the note on [`sort`] for why the walk can
/// now be trusted. But the walk only says where a field IS, not which field
/// matters, and that distinction cost the afternoon its one real scare: the
/// obvious candidates are `changeStrengthPoint` and its four fellows, they are
/// exactly what the name suggests, and on Radagon's Soreseal every one of them
/// reads zero. The attributes a talisman grants live in `addLifeForceStatus`
/// and its eight neighbours, a byte apiece, four hundred bytes further on.
/// Reading the plausible field would have reported that the soreseal grants
/// nothing and costs 15% more damage — a confident, checkable, wrong answer.
/// The spirit ashes, in `EquipParamGoods`.
///
/// Asked which spirit ashes exist and which was best, the launcher searched,
/// found nothing it could use, and answered entirely from memory of the BASE
/// game — naming five that this conversion may not contain at all. It could
/// have read every name in the table instead.
///
/// The counts, because three different ones are true of three different things
/// and mixing them up is how a wrong figure gets written down: 1,232 rows carry
/// the flag, 217 of those are base rows rather than upgraded copies, and 114 of
/// THOSE have a name the launcher can resolve. 114 is the number a player would
/// recognise; the rest are unnamed rows the tables carry and nobody sees.
///
/// [`SUMMONS`] is the discriminator, and it is not a guess: 759 rows carry
/// `goodsType` 7 and 473 carry 8, which is 1,232, and exactly 1,232 rows have
/// this flag set. The two types are the two families of ash; the flag is their
/// union.
///
/// The offsets were hand-walked with the padding rule from
/// `scratchpad/paramwalk.py`, then confirmed byte for byte against a row read
/// by name. Row 231000 reads `58 02 14 01` at 0x70 and SmithBox says
/// maxRepositoryNum 600, sortGroupId 20; it reads
/// `59 86 03 00  10 27 00 00  52 03 00 00` at 0x84 and SmithBox says
/// reinforceGoodsId 231001, reinforceMaterialId 10000, reinforcePrice 850.
/// Six fields agreeing across sixteen bytes is not luck.
///
/// WHAT IS DELIBERATELY NOT HERE. `consumeMP` is what a summon costs in FP, and
/// in THIS installation it reads -1 on all 112 while vanilla holds 88 for the
/// very same row. That is not a bad offset — the bytes on either side of it
/// were just confirmed — it is the conversion having taken the cost out of the
/// field. So the launcher names them and prices their upgrades, and says
/// plainly that it cannot give a summon's cost or its strength.
pub mod spirit {
    /// `useLimitSummonBuddy`: set on every spirit ash and on nothing else.
    pub const SUMMONS: usize = 0x079;
    /// `goodsType`: 7 and 8 are the two families of ash.
    pub const SORT: usize = 0x03e;
    /// `goodsType` 10: the crystal tears that go into the wondrous physick.
    ///
    /// 60 rows headed by the crimson crystal tear, which is what a tear is.
    /// Asked twice what could go into the flask, the launcher found nothing —
    /// while 31 of these read a real effect off the same route a talisman
    /// uses: dexterity +10, max FP +10%, equip load +350%.
    pub const TEAR: u8 = 10;
    /// `refId_default`: the `SpEffectParam` row a Good applies.
    pub const EFFECT: usize = 0x004;
    /// `consumeMP`. Reads -1 throughout this installation; see above.
    pub const FP: usize = 0x080;
    /// `reinforceGoodsId`: the next level up, or -1 at the top.
    pub const NEXT: usize = 0x084;
    /// `reinforceMaterialId`: what upgrading one costs.
    pub const MATERIAL: usize = 0x088;
    /// `reinforcePrice`: and how many runes it takes.
    pub const PRICE: usize = 0x08c;
}

/// Ashes of war, which the game files call GEMS.
///
/// That naming is the whole reason they were unreachable. An ash is not a
/// weapon and not a Good: it is a row in `EquipParamGem`, 242 of them, and it
/// points at the `SwordArtsParam` row carrying the skill and its FP — the very
/// rows already read for the skill sitting on a weapon. Asked which ash was
/// cheapest, the launcher looked five of them up in the WEAPON table, missed
/// five times, and then answered from memory.
///
/// Offset hand-walked with the padding rule, and the walk proves itself again:
/// the paramdef contains a field the developers left called
/// `reserved_canMountWep_0x3d_4`, and the walk puts it at exactly 0x3d.
pub mod gem {
    /// `swordArtsParamId`: the skill this ash puts on a weapon.
    pub const SKILL: usize = 0x18;
}

/// The classes a player picks at the start, which are two tables joined.
///
/// Reached by NOT guessing at the name. `CharaInitParam` has 3,240 rows and
/// only ten of them are classes; the rest are every NPC in the game. What says
/// which ten, and in what order the menu shows them, is a separate table with a
/// name nothing about "class" or "keepsake" would find —
/// `BaseChrSelectMenuParam`, fifteen rows. Rows 1000-1004 are leftovers; the
/// shipped ten are 2000-2009, and each points at its stats through
/// [`STATS`] and at its name through [`NAME`].
///
/// The vanilla spacing is regular — row 2000+n has stats 3100+2n and name
/// 288100+n, checked at both ends and in the middle. It is NOT used. A mod is
/// free to renumber these and this launcher exists to serve modded
/// installations, so the join is read from the table every time.
///
/// KEEPSAKES ARE NOT HERE, and that is worth writing down because it cost a
/// look. `CharaInitParam` has `item_01`..`item_10`, which is where a starting
/// gift would sit, and on every class row in this installation all ten read
/// -1. The keepsake is applied elsewhere.
pub mod class {
    /// In `BaseChrSelectMenuParam`: which `CharaInitParam` row holds the stats.
    pub const STATS: usize = 0x04;
    /// In `BaseChrSelectMenuParam`: the menu text carrying the class's name.
    pub const NAME: usize = 0x10;

    /// In `CharaInitParam`: the level it starts at, in the DLC layout.
    ///
    /// Read through [`super::Regulation::classes`], which picks the layout
    /// rather than trusting this — see [`SHIFTS`].
    pub const LEVEL: usize = 0x0c4;

    /// How far back everything after 0x54 sits on a pre-DLC regulation.
    ///
    /// Shadow of the Erdtree added `scadutreeBlessing` and
    /// `reveredSpiritAshBlessing` at 0x54, two bytes plus two of padding, and
    /// every field after them moved four bytes later. A regulation from before
    /// the DLC does not have them.
    ///
    /// This was not reasoned out, it was walked into: reading Vagabond at the
    /// DLC offsets gave 13, 9, 9, 7 for its first four attributes, which are
    /// its LAST four — dexterity, intelligence, faith, arcane — and a level of
    /// 3,595. Four bytes late, exactly.
    ///
    /// Both are tried and the one that is self-consistent wins, because this
    /// launcher is for modded installations and cannot assume a vintage.
    pub const SHIFTS: [usize; 2] = [0, 4];

    /// What the eight attributes must add up to, less the level.
    ///
    /// The game gives every class the same total: Vagabond is level 9 with 88
    /// points, Astrologer level 6 with 85. That is the check that says which
    /// [`SHIFTS`] reading is the real one — a wrong offset gives eight bytes
    /// that do not add up, and being off by one here is otherwise invisible.
    pub const POINTS_AT_LEVEL_ZERO: i64 = 79;

    /// The eight attributes, under the names on the player's own screen.
    ///
    /// The table's names are older than the game, the same way they are for a
    /// talisman: life force is vigour, willpower is mind, magic is intelligence
    /// and luck is arcane. Kept in the order the character screen lists them,
    /// which is also the order they sit in the row.
    pub const ATTRIBUTES: [(&str, usize); 8] = [
        ("vigour", 0x0c6),
        ("mind", 0x0c7),
        ("endurance", 0x0c8),
        ("strength", 0x0c9),
        ("dexterity", 0x0ca),
        ("intelligence", 0x0cb),
        ("faith", 0x0cc),
        ("arcane", 0x0cd),
    ];

    /// What it starts holding. Ids, for looking up like any other item.
    ///
    /// `equip_Subwep_Right` and the left-hand pair sit between these at 0x14
    /// and 0x1c; they are read too, because a class that starts with a bow in
    /// its off hand is exactly the sort of thing somebody asks about.
    pub const GEAR: [(&str, usize); 8] = [
        ("right hand", 0x010),
        ("right hand, second", 0x014),
        ("left hand", 0x018),
        ("left hand, second", 0x01c),
        ("head", 0x020),
        ("chest", 0x024),
        ("hands", 0x028),
        ("legs", 0x02c),
    ];

    /// The spells it knows, if any. Seven slots, -1 where empty.
    pub const SPELLS: [usize; 7] = [0x064, 0x068, 0x06c, 0x070, 0x074, 0x078, 0x07c];
}

pub mod charm {
    /// In `EquipParamAccessory`: which `SpEffectParam` row it applies.
    pub const EFFECT: usize = 0x004;
    /// In `EquipParamAccessory`: what it weighs.
    pub const WEIGHT: usize = 0x00c;

    /// The nine attributes, in the order the table keeps them, under the names
    /// the player's own screen uses. The table's names are older than the game:
    /// life force is vigour, willpower is mind, endure is endurance, magic is
    /// intelligence and luck is arcane. `addVitalityStatus` belongs to no
    /// attribute this game shows and is here to keep the run of bytes honest.
    pub const ATTRIBUTES: [(&str, usize); 9] = [
        ("vigor", 0x31e),
        ("mind", 0x31f),
        ("endurance", 0x320),
        ("vitality", 0x321),
        ("strength", 0x322),
        ("dexterity", 0x323),
        ("intelligence", 0x324),
        ("faith", 0x325),
        ("arcane", 0x326),
    ];

    /// Everything else worth reading, as `(what it is, where, what it reads
    /// when it does nothing)`. A rate of 1.0 and a point of 0 both mean the
    /// talisman leaves that alone, and only what it does NOT leave alone is
    /// worth an answer.
    pub const RATES: [(&str, usize, f32); 40] = [
        ("max HP", 0x010, 1.0),
        ("max FP", 0x014, 1.0),
        ("max stamina", 0x018, 1.0),
        ("equip load", 0x0e4, 1.0),
        ("runes gained", 0x0e0, 1.0),
        ("stamina spent", 0x1f4, 1.0),
        ("physical attack", 0x048, 1.0),
        ("magic attack", 0x04c, 1.0),
        ("fire attack", 0x050, 1.0),
        ("lightning attack", 0x054, 1.0),
        ("holy attack", 0x1e0, 1.0),
        ("physical taken", 0x028, 1.0),
        ("slash taken", 0x01c, 1.0),
        ("strike taken", 0x020, 1.0),
        ("pierce taken", 0x024, 1.0),
        ("magic taken", 0x02c, 1.0),
        ("fire taken", 0x030, 1.0),
        ("lightning taken", 0x034, 1.0),
        ("holy taken", 0x1d0, 1.0),
        ("poise damage taken", 0x1b4, 1.0),
        // 0x134 and 0x2e8, and they were 0x0cc and 0x37c for one build. Those
        // two were the only entries in this table not taken from the walk — a
        // guess each, and both landed on padding that reads zero, which the
        // answer then reported as "blocking and casting cost NOTHING". Nobody
        // would have believed a talisman that did that, but a smaller lie in
        // the same place would have gone straight through.
        ("stamina spent guarding", 0x134, 1.0),
        ("HP healed", 0x120, 1.0),
        ("casting cost", 0x2e8, 1.0),
        ("skill cost", 0x2e4, 1.0),
        ("fall damage", 0x0dc, 1.0),
        // A second family of damage-dealt multipliers, applied at a different
        // stage from the `attack power` ones above. Both are real and a
        // talisman may use either, so they are named apart rather than merged.
        ("physical damage dealt", 0x038, 1.0),
        ("magic damage dealt", 0x03c, 1.0),
        ("fire damage dealt", 0x040, 1.0),
        ("lightning damage dealt", 0x044, 1.0),
        ("holy damage dealt", 0x1dc, 1.0),
        // And damage taken split by WHO dealt it. Fifteen talismans here use
        // the first set and thirteen the second, and merging them into plain
        // "damage taken" would tell a player their PvP talisman helps against
        // the boss in front of them.
        ("physical taken from players", 0x260, 1.0),
        ("magic taken from players", 0x264, 1.0),
        ("fire taken from players", 0x268, 1.0),
        ("lightning taken from players", 0x26c, 1.0),
        ("holy taken from players", 0x270, 1.0),
        ("physical taken from enemies", 0x274, 1.0),
        ("magic taken from enemies", 0x278, 1.0),
        ("fire taken from enemies", 0x27c, 1.0),
        ("lightning taken from enemies", 0x280, 1.0),
        ("holy taken from enemies", 0x284, 1.0),
    ];

    /// The flat additions, which are points rather than multipliers.
    ///
    /// RESISTING an ailment and INFLICTING one are opposite things and both are
    /// in here, named so they cannot be confused. Asked in Portuguese for the
    /// best talisman for bleed DAMAGE, the launcher had only the resist figure
    /// to offer — bleed resistance is what stops YOU bleeding — and the
    /// language fix that let it understand "sangramento" only made it confident
    /// about the wrong one. Both are now read, and the labels say which.
    ///
    /// The inflict offsets are [`super::buildup::AILMENTS`], the same seven
    /// this file already reads on a weapon and proves end to end: Reduvia's
    /// effect gives 82 at `0x0d4` and its own menu prints "Накапливает
    /// кровотечение (82)". A talisman's effect is read through the same
    /// `SpEffectParam` row as a weapon's, so nothing new had to be found —
    /// only listed.
    pub const POINTS: [(&str, usize); 14] = [
        ("poison resist", 0x1fc),
        ("rot resist", 0x200),
        ("bleed resist", 0x204),
        ("curse resist", 0x208),
        ("frost resist", 0x20c),
        ("sleep resist", 0x348),
        ("madness resist", 0x34c),
        // What it makes YOU inflict, which is what a bleed build is asking for.
        ("poison inflicted", 0x0cc),
        ("rot inflicted", 0x0d0),
        ("bleed inflicted", 0x0d4),
        ("curse inflicted", 0x0d8),
        ("frost inflicted", 0x1a8),
        ("sleep inflicted", 0x338),
        ("madness inflicted", 0x33c),
    ];
}

/// What sort of weapon a weapon is: dagger, katana, greatshield.
///
/// The gap this closes was the plainest in a whole battery. Asked which
/// GREATSHIELD holds magic best, the launcher ranked ARMOUR and answered with a
/// helmet — because nothing could ask for a class of weapon, only a name. And
/// an English question about shields on this Russian installation found nothing
/// at all, because the search matches names and no name here contains the
/// English word.
///
/// `wepType` is the discriminator, at 0x1a6. That offset is not counted down a
/// field list — that is the trap that has produced two wrong offsets in this
/// file. It comes from walking the paramdef and measuring the walk's error
/// against twenty offsets already established: every one out by exactly one
/// byte, and the walked row 665 against the true 664. A uniform error is a
/// usable one. Reduvia reads 1, the dagger.
///
/// The names are the GAME's, not a translation: `GR_MenuText` keeps all 47 at
/// #60010 to #60187, so a Russian player's "катана" and a German's "Katana"
/// both land without a word of it being written here. The English column below
/// is for the model, which asks in English whatever the player typed.
pub mod sort {
    /// `wepType` in `EquipParamWeapon`, a u16.
    pub const AT: usize = 0x1a6;

    /// Every sort in the installed game: the value, the game's own menu id for
    /// its name, and the English word for it.
    ///
    /// The pairing was checked group by group against the weapons actually in
    /// each — 4,510 of them — not assumed from the order. It could not be
    /// assumed: the ids are hand-authored and run out of step twice over, with
    /// whips at #60155 sitting after greatshields, and colossal weapons at
    /// #60102 before light bows.
    ///
    /// Two do not pair. `wepType` 33 is a single row the game calls "no
    /// weapon", which is empty hands rather than a class of anything, so it has
    /// no menu name; and #60125 names a class this installation has no weapons
    /// in. Neither is an error and neither belongs in the list.
    pub const ALL: [(u16, u32, &str); 47] = [
        (1, 60010, "dagger"),
        (3, 60015, "straight sword"),
        (5, 60020, "greatsword"),
        (7, 60025, "colossal sword"),
        (9, 60030, "curved sword"),
        (11, 60035, "curved greatsword"),
        (13, 60040, "katana"),
        (14, 60043, "twinblade"),
        (15, 60045, "thrusting sword"),
        (16, 60046, "heavy thrusting sword"),
        (17, 60050, "axe"),
        (19, 60055, "greataxe"),
        (21, 60060, "hammer"),
        (23, 60065, "great hammer"),
        (24, 60067, "flail"),
        (25, 60070, "spear"),
        (28, 60077, "great spear"),
        (29, 60080, "halberd"),
        (31, 60085, "reaper"),
        (33, 0, "bare hands"),
        (35, 60090, "fist"),
        (37, 60100, "claw"),
        (39, 60155, "whip"),
        (41, 60102, "colossal weapon"),
        (50, 60104, "light bow"),
        (51, 60105, "bow"),
        (53, 60110, "greatbow"),
        (55, 60115, "crossbow"),
        (56, 60117, "ballista"),
        (57, 60120, "glintstone staff"),
        (61, 60130, "sacred seal"),
        (65, 60140, "small shield"),
        (67, 60145, "medium shield"),
        (69, 60150, "greatshield"),
        (81, 60160, "arrow"),
        (83, 60165, "greatarrow"),
        (85, 60170, "bolt"),
        (86, 60171, "great bolt"),
        (87, 60175, "torch"),
        (88, 60180, "hand-to-hand"),
        (89, 60181, "perfume bottle"),
        (90, 60182, "thrusting shield"),
        (91, 60183, "throwing blade"),
        (92, 60184, "backhand blade"),
        (93, 60185, "light greatsword"),
        (94, 60186, "great katana"),
        (95, 60187, "beast claw"),
    ];

    /// What the English word is, for a value out of the table.
    pub fn english(sort: u16) -> Option<&'static str> {
        ALL.iter().find(|(value, _, _)| *value == sort).map(|(_, _, word)| *word)
    }

    /// Where the game keeps its own name for it.
    pub fn menu_id(sort: u16) -> Option<u32> {
        ALL.iter()
            .find(|(value, _, _)| *value == sort)
            .map(|(_, id, _)| *id)
            .filter(|id| *id != 0)
    }

    /// Whether a sort is one a shield's guard figures are the point of.
    ///
    /// Four classes block: the three shields and the DLC's thrusting shield.
    /// Everything else has guard rates too and they are mostly beside the
    /// point, which is why a list of katanas should not lead with them.
    pub fn blocks(sort: u16) -> bool {
        matches!(sort, 65 | 67 | 69 | 90)
    }

    /// Which sorts a player meant, in English. Several, where the word covers
    /// several: "shield" is four classes and answering with one of them is how
    /// a question about greatshields got a buckler.
    pub fn named(word: &str) -> Vec<u16> {
        let said = word.trim().to_lowercase();
        let said = said.strip_suffix('s').unwrap_or(&said);
        if said.len() < 3 {
            return Vec::new();
        }

        // Every class that is a weapon, for the question that is not about a
        // class at all — "what is the best weapon for a dexterity build".
        //
        // There was no way to ask that, and the cost was measured rather than
        // guessed at. In French, live: the model asked for katana, then curved
        // sword, then twinblade, then thrusting sword, dagger, curved
        // greatsword, greatsword, light greatsword, claw, fist, whip, spear,
        // great spear, halberd, reaper, axe, greataxe, hammer, great hammer,
        // flail, bow, greatbow, crossbow, ballista — and then started again by
        // physical. TWENTY-EIGHT calls, each re-sending 45,000 characters of
        // rules and tool schemas, well over a megabyte on the wire for one
        // question, and it still ended with every lane spent and no answer.
        //
        // Shields, arrows, bolts and the torch are left out: somebody asking
        // for the best weapon is not asking for a buckler, and the shield
        // families are one word away for when they are.
        let every_weapon = || -> Vec<u16> {
            const NOT_A_WEAPON: [u16; 9] = [65, 67, 69, 90, 81, 83, 85, 86, 87];
            let mut out: Vec<u16> = ALL
                .iter()
                .map(|(value, _, _)| *value)
                .filter(|value| !NOT_A_WEAPON.contains(value))
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        };
        // A trailing "s" is already gone by here, so "all weapons" arrives as
        // "all weapon". Exact matches, so "colossal weapon" is untouched.
        if matches!(said, "weapon" | "all" | "any" | "all weapon" | "any weapon" | "everything") {
            return every_weapon();
        }

        // The words that mean a family rather than one class, checked FIRST.
        // Several of them are also the exact name of one member — "bow" is both
        // the family and the middle class of it, "spear" both the family and
        // the shorter half. Somebody asking which bow to use means bows, so the
        // family wins, and the listing labels each row with its own class so
        // the wider answer is not a vaguer one.
        for (family, members) in [
            ("shield", &[65u16, 67, 69, 90][..]),
            ("sword", &[3, 5, 7, 9, 11, 13, 15, 16, 93, 94][..]),
            ("bow", &[50, 51, 53][..]),
            ("catalyst", &[57, 61][..]),
            ("staff", &[57][..]),
            ("seal", &[61][..]),
            ("ammunition", &[81, 83, 85, 86][..]),
            ("ammo", &[81, 83, 85, 86][..]),
            ("polearm", &[25, 28, 29, 31][..]),
            ("spear", &[25, 28][..]),
            ("greatshield", &[69][..]),
            ("shortsword", &[3][..]),
            ("great sword", &[5][..]),
            ("ultra greatsword", &[7][..]),
            ("colossal", &[7, 41][..]),
            ("scythe", &[31][..]),
            ("pick", &[21][..]),
            ("mace", &[21][..]),
            ("club", &[21][..]),
            ("rapier", &[15][..]),
            ("estoc", &[16][..]),
            ("buckler", &[65][..]),
            ("fist weapon", &[35][..]),
            ("nagakiba", &[13][..]),
        ] {
            if said == family {
                return members.to_vec();
            }
        }

        // Then the exact class names. "greatshield" has to beat the "shield"
        // family, which is why this runs before the loose containment below
        // and why the family list above is exact-match only.
        let exact: Vec<u16> = ALL
            .iter()
            .filter(|(_, _, english)| *english == said)
            .map(|(value, _, _)| *value)
            .collect();
        if !exact.is_empty() {
            return exact;
        }

        // Then anything the word is inside, longest first, so "great katana"
        // is not answered with every katana.
        let mut near: Vec<(usize, u16)> = ALL
            .iter()
            .filter(|(_, _, english)| english.contains(said) || said.contains(*english))
            .map(|(value, _, english)| (english.len(), *value))
            .collect();
        near.sort_by_key(|(length, _)| std::cmp::Reverse(*length));
        near.into_iter().map(|(_, value)| value).collect()
    }
}

/// The nine attributes, by the words a player uses for them.
///
/// For ranking armour by what it GRANTS. Armour in this conversion gives
/// attributes — 836 of 841 pieces carry an effect — and "quelle armure est la
/// plus légère pour un build foi" cannot be answered from a ranking that only
/// knows about negation. Asked exactly that, an answer gave the lightest armour
/// in the game with no reference to faith at all, having no way to ask.
///
/// Checked AFTER poise and the four resistances, never before, so that nothing
/// which already worked changes meaning.
///
/// Two words are deliberately absent. Spanish *aguante* already means poise
/// here and Spanish is not consistent about it; German *Vitalität* already
/// means the death-blight resistance. Both would be ambiguous and an ambiguous
/// stem silently ranks by the wrong column, which is worse than not matching.
/// The Russian names are the ones the game actually prints, which do NOT
/// translate straight across: мудрость is INTELLIGENCE and колдовство is
/// ARCANE, and getting those two round the wrong way sends somebody's points
/// into the wrong stat.
pub mod attribute {
    pub(super) const SAID: [(&str, &[&str]); 9] = [
        ("vigor", &["vigor", "vigour", "жизненн", "vigueur", "lebenskraft"]),
        ("mind", &["mind", "интеллект", "esprit", "verstand", "mente"]),
        ("endurance", &["endurance", "выносливост", "ausdauer", "resistencia"]),
        ("vitality", &["vitality"]),
        ("strength", &["strength", "сила", "силу", "силы", "force", "stärke", "kraft", "fuerza"]),
        ("dexterity", &["dexterity", "ловкост", "dextérité", "dexterite", "geschick", "destreza"]),
        ("intelligence", &["intelligence", "мудрост", "intelligenz", "inteligencia"]),
        ("faith", &["faith", "вера", "веру", "веры", "foi", "glaube", "fe"]),
        ("arcane", &["arcane", "колдовств", "arcan", "arkan", "esoterismo"]),
    ];

    pub fn all() -> impl Iterator<Item = &'static str> {
        SAID.iter().map(|(name, _)| *name)
    }

    /// Where the game keeps its OWN word for each, in `GR_MenuText`.
    ///
    /// So the player's line can carry it and nothing has to be translated on
    /// the way out. The stat labels already end in the English abbreviation —
    /// "Faith (FTH)" — for exactly this reason, and an answer STILL rendered
    /// Faith into Russian as "Фея", which is a fairy. Give it the word and
    /// there is nothing left to get wrong.
    ///
    /// The ids are read off the game, not guessed, and they carry their own
    /// proof: the entries are "Стойкость(END)", "Интеллект(FP)" and
    /// "Мудрость(INT)", so the parenthetical says which attribute each one is.
    /// That settles the two that do not translate straight across — мудрость is
    /// INTELLIGENCE and интеллект is MIND.
    pub const MENU: [(&str, u32); 8] = [
        ("Vigor", 10400),
        ("Mind", 10402),
        ("Endurance", 10401),
        ("Strength", 10403),
        ("Dexterity", 10404),
        ("Intelligence", 10406),
        ("Faith", 10407),
        ("Arcane", 10409),
    ];

    /// Which attribute a player meant, if any. Longest match wins.
    ///
    /// A short stem is matched as a WHOLE WORD, a long one anywhere inside.
    /// Spanish faith is *fe*, two letters, and it lives inside "defense",
    /// "fear" and "perfumer"; matching it loosely would rank armour by faith
    /// for a question about defence. Long stems stay loose so that declensions
    /// and compounds still land — "выносливост" has half a dozen endings.
    pub fn named(word: &str) -> Option<&'static str> {
        const SHORT: usize = 4;
        let said = word.trim().to_lowercase();
        if said.len() < 2 {
            return None;
        }
        let words: Vec<&str> =
            said.split(|c: char| !c.is_alphanumeric()).filter(|part| !part.is_empty()).collect();
        let mut best: Option<(usize, &'static str)> = None;
        for (name, stems) in SAID {
            for stem in stems {
                let hit = if stem.len() <= SHORT {
                    words.iter().any(|part| part == stem)
                } else {
                    said.contains(stem)
                        || (stem.len() > said.len() && stem.starts_with(&said) && said.len() > 2)
                };
                if hit && best.is_none_or(|(had, _)| stem.len() > had) {
                    best = Some((stem.len(), name));
                }
            }
        }
        best.map(|(_, name)| name)
    }
}

/// The four resistances the equipment screen shows, by the words a player uses.
///
/// Separate from `kind` because these are not damage and are not negated: they
/// are how long an ailment bar takes to fill. Asked "welcher Helm gibt mir am
/// meisten Robustheit?" the launcher said Robustheit is not a kind of damage
/// and offered poise instead. Both halves of that were wrong. Robustness is a
/// real column in the armour table, it was already being read, and German poise
/// is *Haltung* — filing Robustheit under poise would have answered a question
/// about bleed and frost with a number about stagger.
///
/// The mapping is not recalled, it is what the installed game says on its own
/// loading screens: Иммунитет is quoted beside the poison and rot bars,
/// Живучесть beside bleed and frostbite, Концентрация beside madness and sleep.
/// The fourth, Физ. мощь, has no loading screen of its own and is placed by the
/// order of the menu ids — an order the game itself settles, because the four
/// damage kinds above it read physical, strike, slash, pierce, which is the
/// screen's order and not the table's.
/// Whether a ranking was asked for by WEIGHT.
///
/// Its own module so the ranking and the wording of the ranking cannot drift
/// apart. They did, within one battery: the ranking learned weight, the wording
/// did not, and the heading came out as "the best AGAINST weight" with each
/// line reading "stops 15.8%". The model then told the player that in this mod
/// weight IS physical defence — a sentence it had every reason to believe,
/// because the tool had just said so.
pub mod bulk {
    /// "light" is deliberately absent: it is a prefix of "lightning", a real
    /// damage kind. "heavy" too — "best armour against heavy attacks" is not a
    /// weight question. The superlatives are unambiguous and are what gets
    /// typed.
    const SAID: [&str; 14] = [
        "weight", "heaviest", "lightest", "вес", "тяжёл", "тяжел", "легч",
        "peso", "pesad", "poids", "gewicht", "waga", "ciężk", "schwerste",
    ];

    pub fn asked_for(word: &str) -> bool {
        let said = word.trim().to_lowercase();
        SAID.iter().any(|stem| said.contains(stem))
    }
}

pub mod resistance {
    /// What each is called, then the words that mean it. Stems, not whole
    /// words, so declensions and the odd abbreviation still land.
    ///
    /// SPANISH IS NOT PORTUGUESE, and a stem that forgets it is a stem that
    /// does nothing. `named` asks whether the QUERY contains the stem, so
    /// "sangrado" can never match "sangramento" — the two words share only
    /// their first six letters. Asked in Portuguese for the best bleed
    /// talisman, the launcher searched, understood nothing, and told the player
    /// there is no such talisman in this game. There are several. The stems
    /// below are cut back to the part the languages share for exactly that
    /// reason, and every one of them is checked by
    /// `a_resistance_is_recognised_in_every_language_it_is_asked_in`.
    pub(super) const SAID: [(&str, &[&str]); 4] = [
        (
            "immunity",
            &[
                "immunit", "иммунит", "immunität", "inmunidad", "imunidad",
                "immunité",
                // What it actually covers, because that is how it gets asked.
                "poison", "яд", "отравл", "gift", "veneno", "envenen", "rot",
                "гнил", "scarlet", "seuche",
            ],
        ),
        (
            "robustness",
            &[
                "robust", "живуч", "robustez", "robustesse", "hardiness",
                // "sangr" and not "sangrado": it has to reach the Portuguese
                // sangramento and sangrar as well as the Spanish sangrado, and
                // nothing but blood begins that way in any of them.
                "bleed", "кровот", "кровоп", "hemorrhage", "blut", "sangr",
                "frost", "обморож", "мороз", "freeze", "kälte", "congel",
            ],
        ),
        (
            "focus",
            &[
                "focus", "концентрац", "konzentration", "concentraci",
                "sleep", "сон", "сонлив", "schlaf", "sueño", "sono",
                // Spanish locura, Portuguese loucura — one letter apart and it
                // is in the middle, so neither stem covers the other.
                "madness", "безум", "wahnsinn", "locura", "loucur",
            ],
        ),
        (
            "vitality",
            &[
                // "vitalid" for the Spanish vitalidad and the Portuguese
                // vitalidade both; "vitalit" stays for the English.
                "vitalit", "vitalid", "физ. мощь", "физмощь", "физ мощь",
                // German ER prints Lebenskraft for this one, confirmed against
                // the German UI alongside Immunität, Robustheit and Fokus.
                "lebenskraft", "death blight", "смертельн", "мор", "tod",
                "muerte", "morte",
            ],
        ),
    ];

    pub fn all() -> impl Iterator<Item = &'static str> {
        SAID.iter().map(|(name, _)| *name)
    }

    /// Which of the four a player meant, if any.
    ///
    /// The longest match wins, but a word that is actually PRESENT beats a
    /// longer word the query merely begins. Without that ordering "мор" — death
    /// blight, and vitality — landed on robustness, because "мороз" is frost,
    /// starts with it, and is two letters longer. Both readings are real; the
    /// one the player typed is the one that counts.
    pub fn named(word: &str) -> Option<&'static str> {
        let said = word.trim().to_lowercase();
        if said.len() < 3 {
            return None;
        }
        let mut best: Option<(usize, &'static str)> = None;
        for (name, words) in SAID {
            for stem in words {
                let strength = if said.contains(stem) {
                    stem.len() * 2
                } else if stem.len() > said.len() && stem.starts_with(&said) {
                    stem.len()
                } else {
                    continue;
                };
                if best.is_none_or(|(had, _)| strength > had) {
                    best = Some((strength, name));
                }
            }
        }
        best.map(|(_, name)| name)
    }
}

/// Poise, which is the figure heavy armour is worn for.
///
/// Stored per piece as a rate — the number on the screen is a thousand times
/// what the table holds — and the four worn pieces add up.
///
/// Anchored and then confirmed by shape, because one anchor can match by luck.
/// The anchor: this installation's screen reads 12 while a surgeon's robe set
/// is on, and those four pieces sum to 11.70 here, which is what 12 looks like
/// after the screen rounds it. The shape: sorted by this field, the top of the
/// game is 61.10 at 15.8 weight, then 52.00, 52.00, 48.10 — every one of them a
/// row ending in 100, which is a body piece, and body pieces are where poise
/// lives. A field matching by coincidence does not order itself by weight and
/// put the breastplates at the top.
///
/// What is NOT claimed: that 11.70 is exactly 12. The screen rounds and this
/// does not, so an answer should say what it reads rather than pretend to the
/// screen's own arithmetic.
pub mod poise {
    /// `EquipParamProtector`, as a rate.
    pub const AT: usize = 0x014;
    /// What the rate is multiplied by to become the number on the screen.
    pub const SCALE: f32 = 1000.0;
}

/// How much they can carry, and what a load does to their roll.
///
/// The curve is found by fingerprint rather than counted or recalled: of the
/// game's 87 curves, exactly one puts endurance 11 at 49.8, which is the figure
/// this installation's own equipment screen shows. See `show_equip_curve`.
///
/// The bands are measured the same way, from two screens of the same
/// character: 14.0 of 49.8 is 28.1% and it reads light, 20.0 of 49.8 is 40.2%
/// and it reads medium. Two points, two bands, both from the game's own words.
pub mod carrying {
    /// `CalcCorrectGraph` row: endurance in, maximum load out.
    pub const CURVE: i64 = 220;
    /// Below this share of the maximum, the roll is the fast one.
    pub const LIGHT: f32 = 30.0;
    pub const MEDIUM: f32 = 70.0;
    /// At or below this they still move; above it they are overloaded.
    pub const HEAVY: f32 = 100.0;
}

/// What a weapon builds up on the thing it hits, which is kept somewhere else.
///
/// Not in the weapon's row — that was swept, every u16 slot, two bytes at a
/// time, and the one field holding Reduvia's 82 is its fire damage. The game
/// hangs an ailment off a weapon by reference: an id in the weapon's row, into
/// `SpEffectParam`, which carries all seven side by side.
///
/// Both ends established rather than counted. SmithBox reads these params with
/// the names the game ships and gave the shape in one call; the offsets came
/// from fingerprints against it, because counting down a field order is what
/// produced two wrong offsets in this file before — a `dummy8` carrying a bit
/// size pads somebody else's byte rather than taking one.
///
/// The id: `0x044` is the only field in the row that holds -1 or a row that
/// really exists on all 4,510 named weapons while pointing somewhere for 1,995
/// of them. Following Reduvia's through it lands on effect 105075, whose blood
/// figure is **82** — exactly what this installation's menu prints for it under
/// "Накапливает кровотечение". That is the whole chain confirmed end to end,
/// from a byte to the player's screen.
///
/// The figure: `0x0d4`, because vanilla effect 6410 has 50 there, SmithBox
/// says that row's `bloodAttackPower` is 50, and no other offset in the row
/// holds it.
///
/// Only bleed is named on the weapon. All seven — poison, rot, bleed, curse,
/// frost, sleep, madness — sit in `SpEffectParam` under `poizonAttackPower` and
/// its fellows, and all seven are now read.
pub mod buildup {
    /// The effect ids a weapon hangs on itself, in `EquipParamWeapon`.
    pub const EFFECTS: [usize; 3] = [0x044, 0x048, 0x04c];

    /// All seven, in `SpEffectParam`.
    ///
    /// The first four were fingerprinted, one at a time, against a second
    /// reader: `0x0cc` because row 6504 reads 84 there and SmithBox says that
    /// row's `poizonAttackPower` is 84 with the other three at zero, and
    /// `0x0d4` because row 6410 reads 50 and SmithBox says `bloodAttackPower`
    /// is 50 — two fixed points two apart in a run of consecutive `s32`.
    ///
    /// The last three could not be found that way and were refused for weeks
    /// rather than guessed. Frost had one line of evidence — it is the first
    /// `s32` after a run of seven `-1` vfx slots, which puts it at `0x1a8` —
    /// and failed a second test, and one line of evidence pointing at a field
    /// while another declines to is exactly when a wrong offset gets written
    /// down. So it stayed unread, and answers said so.
    ///
    /// What settled it was not a better sweep. It was fixing the paramdef walk
    /// — `dummy8` with a bit size is padding inside the open bit pool, not a
    /// byte of its own — after which the whole layout computes exactly. The
    /// walk puts frost at `0x1a8`, which is where the structure had pointed all
    /// along, and sleep and madness at `0x338` and `0x33c`. It also reproduces
    /// all four of the fingerprinted offsets above, which is the reason to
    /// believe the three it found on its own.
    ///
    /// And then the second line of evidence the first attempt never had, out of
    /// `show_all_seven_buildups`: what the weapons carrying each are CALLED.
    /// Every one of the 156 highest in madness is named Безумный — mad. The
    /// top of frost is Ледяной and Драконья гроза великанов, all ice. Sleep
    /// belongs to the Мистический weapons, which is what this conversion calls
    /// the sleep affinity. Their ranges — 42 to 130 — sit in the same band as
    /// the four that were already proven. An offset landing on the wrong field
    /// still reads numbers; it does not sort the game's ice weapons to the top
    /// of a column and its mad ones to the top of another.
    pub const AILMENTS: [(&str, usize); 7] = [
        ("poison", 0x0cc),
        ("rot", 0x0d0),
        ("bleed", 0x0d4),
        ("curse", 0x0d8),
        ("frost", 0x1a8),
        ("sleep", 0x338),
        ("madness", 0x33c),
    ];

    /// Bleed alone, kept for the reader that only wants it.
    pub const BLOOD: usize = 0x0d4;
}

/// How much a shield stops when it is raised, out of `EquipParamWeapon`.
///
/// A shield is not armour and is not in `EquipParamProtector`; it is held, so
/// it is a weapon, and its block is the one number anybody picks one on. Not
/// reading it had a cost: asked whether this installation has a shield with
/// 100% physical block, a model answered "no" out of the armour ranking, which
/// contains no shield at all. It does have them — 334 of them.
///
/// Found by shape and confirmed by names. At `0x034` the shields average 89.1
/// against 51.1 for everything else, 334 of them sit at exactly 100.0, and the
/// top of the list is shields all the way down. At `0x038` the highest in the
/// game is the Silver Mirrorshield, which is the shield that blocks magic —
/// that is the confirmation, not the arithmetic.
///
/// All five settled against the game's own menu, which is the only second
/// reader that could do it. Names could not: fire and lightning are both
/// topped by the Fingerprint Stone Shield, which is best at everything and
/// therefore separates nothing.
///
/// The Exile Knight Shield's "Сопротивление в блоке" pane reads physical
/// 100.0, magic 49.0, fire 57.0, lightning 31.0, holy 48.0 — and each of those
/// five matched exactly one offset in its row. Four sit in a run and **holy
/// does not**, which is why the run looked one short of the menu and why
/// guessing the order would have put fire's figure under lightning's name.
///
/// Guard boost — 55 on that shield, the number that decides how much stamina a
/// blocked hit costs — matched no float at all, so it is stored as an integer
/// somewhere and is not read here.
pub mod shield {
    pub const PHYSICAL: usize = 0x034;
    pub const MAGIC: usize = 0x038;
    pub const FIRE: usize = 0x03c;
    pub const LIGHTNING: usize = 0x040;
    /// Away from the other four, on its own.
    pub const HOLY: usize = 0x188;

    /// The five as the menu lists them, in the menu's order.
    pub const ALL: [(&str, usize); 5] = [
        ("physical", PHYSICAL),
        ("magic", MAGIC),
        ("fire", FIRE),
        ("lightning", LIGHTNING),
        ("holy", HOLY),
    ];
}

/// The eight kinds of damage, under the words a player might use for them.
///
/// The tables name them in English and a question does not arrive in English.
/// Asked "Welche Rüstung schützt am besten vor Blitz?", a model passed `blitz`
/// through as the kind, was told there is no such kind, and spent a whole round
/// — nine and a half seconds — working out that the launcher wanted the English
/// word. It got there; a slower model would have told the player that nothing
/// in the game resists lightning, which is the failure this prevents.
///
/// The languages covered are the ones the launcher already commits to
/// elsewhere: its wiki mirrors are English, Russian, German and Spanish. These
/// are words, not figures out of the game — nothing here asserts anything about
/// how much of anything is stopped.
pub mod kind {
    /// English first in each row: that is what the row is called, and what
    /// every caller gets back.
    pub(super) const SAID: [&[&str]; 8] = [
        &["physical", "физическ", "физ", "physisch", "físico", "fisico"],
        &["slash", "cut", "рубящ", "рассекающ", "разрез", "schnitt", "hieb", "corte", "tajo"],
        &["strike", "blunt", "дробящ", "ударн", "schlag", "stumpf", "golpe", "contundente"],
        &["pierce", "thrust", "колющ", "прокал", "прокол", "stich", "perforac", "estocada"],
        &["magic", "sorcery", "магическ", "магии", "магия", "маг", "magie", "magisch", "mágico",
          "magico"],
        &["fire", "огненн", "огня", "огонь", "feuer", "fuego"],
        &["lightning", "молни", "электр", "blitz", "rayo", "relámpago", "relampago"],
        &["holy", "священн", "святой", "святог", "свят", "heilig", "sagrado", "santo"],
    ];

    /// The eight as the tables name them.
    pub fn all() -> impl Iterator<Item = &'static str> {
        SAID.iter().filter_map(|row| row.first().copied())
    }

    /// The English name for whatever the player called it, if it is one of
    /// these at all.
    ///
    /// Matched on prefix in both directions, because a word arrives declined
    /// ("физического"), compounded ("Blitzschaden") or shortened ("маг"). The
    /// stems are chosen short enough to survive that and long enough not to
    /// collide — the check that proves they do not is in the tests.
    pub fn named(word: &str) -> Option<&'static str> {
        let said = word.trim().to_lowercase();
        if said.len() < 3 {
            return None;
        }
        SAID.iter().find_map(|row| {
            let hit = row
                .iter()
                .any(|form| said.starts_with(form) || (said.len() >= 4 && form.starts_with(&said)));
            hit.then(|| row[0])
        })
    }
}

/// Which of the four places a piece is worn, and the byte that says so.
///
/// Found by shape and proved by the game's own words rather than counted down
/// a field list: exactly one byte splits the 913 wearable pieces into four
/// groups, and the names within each group are all one kind of thing — helms
/// with helms, greaves with greaves. 263 head, 296 body, 175 arms, 179 legs,
/// which is every one of them and nothing left over.
///
/// It matters because without it "what protects me best" answers with four
/// chest pieces, and a player wears one of those. A model handed such a list
/// noticed the problem itself: every one of them was a breastplate.
pub mod slot {
    pub const WHERE: usize = 0x0d6;
    pub const NAMES: [&str; 4] = ["head", "body", "arms", "legs"];

    pub fn called(value: u8) -> Option<&'static str> {
        NAMES.get(value as usize).copied()
    }

    /// Which pieces belong to one SET.
    ///
    /// Read off the ids rather than assumed: the bull-goat's four are 140000,
    /// 140100, 140200 and 140300, and the bandit's are 931100, 931200 and
    /// 931300. So a set is `id / 1000`, and the hundreds digit is the slot in
    /// the order [`NAMES`] keeps them. Grouping by `id / 100` instead — the
    /// obvious guess, and the first thing tried — puts every piece in a set of
    /// its own and finds no sets at all.
    ///
    /// A SET IS NOT ALWAYS FOUR. Of the 331 sets here, 162 have four or more,
    /// 13 have three, 33 have two, and 123 are a single piece. The bandit's is
    /// three, because this installation has no bandit mask. Asked what that set
    /// weighs, an answer read a wiki and gave four pieces totalling 11.8: the
    /// mask does not exist here and the other three weigh 4.6, 1.0 and 2.6, so
    /// every figure in it was wrong, including how many there were.
    pub fn set_of(id: u32) -> u32 {
        id / 1000
    }
}

/// Field offsets in `Magic` — sorceries and incantations both.
pub mod spell {
    /// Arcane, under the name the older games used for it.
    pub const NEEDS_ARCANE: usize = 0x0e;
    pub const FP: usize = 0x10;
    pub const STAMINA: usize = 0x12;
    pub const SLOTS: usize = 0x21;
    pub const NEEDS_INTELLIGENCE: usize = 0x22;
    pub const NEEDS_FAITH: usize = 0x23;
    /// What a held cast costs, when it can be held.
    pub const FP_HELD: usize = 0x74;
}

/// What upgrading costs, which is two tables joined by a sum.
///
/// A weapon carries a base `materialSetId`, each reinforce row carries an
/// addition — the definition calls it 素材ID加算値, an *addition* value, not an
/// id — and the row in `EquipMtrlSetParam` is the two added together. Getting
/// that wrong reads a real row belonging to a different weapon, which is the
/// worst kind of wrong because it looks like an answer.
///
/// This exists because there was nothing to look up. Asked how to get a weapon
/// to +10, the assistant invented the materials every single time, on every
/// lane and through every wording of the prompt: "Somerset Stone", "Somingesite
/// Stones", "sombre stones", a "goat blacksmith at Roundtable Hold". None of
/// those is an item or a person in any version of this game.
/// The bonus the stat screen adds on top of a weapon's attack.
///
/// This is the `+ 49` in "Огонь 106 + 49" — the one number a player can see
/// that the launcher could not, and the reason questions about what another ten
/// points would buy were answered from memory and got it wrong.
///
/// Four tables meet here. The weapon gives a base per damage type, its scaling
/// per stat, and a `correctType_*` picking a curve; the reinforce row multiplies
/// both; `AttackElementCorrectParam` says which stats touch which damage type
/// at all; and `CalcCorrectGraph` turns a stat value into a percentage along a
/// five-point curve. The bonus is the base times the sum, over every stat that
/// applies, of scaling × curve.
///
/// Worked by hand against this character before a line of it was written, and
/// the check is exact: Reduvia at 82 fire × 1.3 is 106.6; faith 22 on curve 0
/// gives 21.875 and arcane 26 gives 26.0; scaling 65 × 1.2 is 78 for each; with
/// dexterity and strength that sums to 0.4603, and 106.6 × 0.4603 is 49.07. The
/// screen says 49.
pub mod curve {
    pub const MAX: [usize; 5] = [0x00, 0x04, 0x08, 0x0c, 0x10];
    pub const GROW: [usize; 5] = [0x14, 0x18, 0x1c, 0x20, 0x24];
    /// The exponent between two points. Negative bends the other way.
    pub const ADJUST: [usize; 5] = [0x28, 0x2c, 0x30, 0x34, 0x38];
}

/// One kind of damage a weapon does, split into what it has and what the
/// character adds.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Damage {
    /// "physical", "magic", "fire", "lightning", "holy".
    pub kind: String,
    /// After the upgrade multiplier, before the character.
    pub base: f32,
    /// What their attributes add on top.
    pub bonus: f32,
}

/// A step can reuse the step before it, and the names can look wrong and be
/// right. The Convergence charges item 10160 for +1, +2 and +3 alike, then
/// 10164 for two levels, then 10165 — it has collapsed the base game's ladder
/// of numbered stones into tiers. Its English text renames them to match
/// ("Somber Stone", "Large Somber Stone", "Great Somber Stone"); its Russian
/// text is the base game's, so the same ids come back as "Кузнечный камень
/// мрака [3]", "[5]" and "[7]". That is not a misread. The player's game reads
/// the same text file, so the name here is the name on their screen, and the
/// two languages disagreeing is the mod's doing rather than this reader's.
pub mod mtrl {
    /// Up to six ingredients a step, most of them unused.
    pub const MATERIAL_ID: [usize; 6] = [0x00, 0x04, 0x08, 0x0c, 0x10, 0x14];
    pub const ITEM_COUNT: [usize; 6] = [0x20, 0x21, 0x22, 0x23, 0x24, 0x25];
    /// `materialSetId` in a reinforce row: a u8 added to the weapon's own.
    pub const SET_FROM_LEVEL: usize = 0x56;
}

/// One step of upgrading: which level it reaches, and what it costs.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub level: u8,
    /// Item id and how many, in the order the smith's menu lists them.
    pub costs: Vec<(i64, i8)>,
}

/// Field offsets in `EquipParamAccessory`, which is the talismans.
///
/// 157 rows, 96 bytes each, and every offset here was computed from the game's
/// own definition of `EQUIP_PARAM_ACCESSORY_ST` rather than found by looking at
/// the bytes.
///
/// **What a talisman does is not in this table.** The row carries four
/// `residentSpEffectId` fields and the effect lives in whichever `SpEffect`
/// rows those point at — a chain this module does not follow, and one where a
/// half-followed link would produce a confident wrong answer about what an item
/// is for. The item's own description says what it does in the player's own
/// language, and `game_item` already reads that.
///
/// So this is deliberately only two things: which talismans this installation
/// actually has, and what each one weighs. That is enough for the failure it
/// exists to stop — asked which talismans would suit them, with nothing to look
/// up, a model produced "Символ веры", "Священный медальон", "Медальон
/// колдовства" and "Знак древних", none of which is an item in any version of
/// this game.
pub mod talisman {
    pub const WEIGHT: usize = 0x0c;
}

/// A talisman, as the installed game has it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Talisman {
    pub id: i64,
    pub weight: f32,
}

/// A sorcery or incantation, as the installed game has it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spell {
    pub id: i64,
    pub fp: i16,
    /// What a charged cast costs, when it differs.
    pub fp_held: Option<i16>,
    pub stamina: i16,
    /// Memory slots it takes up.
    pub slots: u8,
    pub needs: Vec<(String, u8)>,
}

/// Field offsets in `GameAreaParam`, which is the game's own list of bosses.
///
/// 216 rows, 96 bytes each. What is in here is verified: the row ids are
/// map-encoded and come out as real maps, and the rune rewards run from a
/// thousand to five hundred thousand with Stormveil's boss at twelve — the
/// shape real rewards have.
///
/// Every offset below was checked against the game's own field definition for
/// `GAME_AREA_PARAM_ST` rather than worked out from the data, and all of them
/// were already right — so the missing name is not a misread field, and moving
/// the offsets around will not find it.
///
/// **The name is not in here, and cannot be.** `foundBossTextId` sounds like
/// it; the definition calls it 発見時テキストID — the id of the line shown *when
/// the boss is discovered*. It indexes a different message table, which is why
/// resolving it against item names produced weapons and armour. Nor is there
/// any way through: the row carries flags, runes, a position and a map, and no
/// reference to the enemy at all. A boss's name belongs to the enemy placed in
/// the map file, and that is the only place it exists.
///
/// So this table can say what a fight is worth and where it is, and nothing
/// about who it is — which is why nothing here is given to the assistant yet. A
/// rune figure with no name attached is an invitation to supply one, and
/// supplying one is the mistake this was meant to stop.
///
/// Reading those map files is a job of its own. A total conversion leaves them
/// loose — 634 of them, `map/mapstudio/*.msb.dcx`, next to the regulation this
/// module already reads — but a player on the plain game has them inside
/// `Data0.bhd`, behind an encrypted archive header, so doing it for one player
/// is not doing it for everybody.
pub mod boss {
    pub const RUNES: usize = 0x04;
    /// `foundBossTextId`. Kept for completeness; see above for why it is not a
    /// name and must not be resolved as one.
    pub const NAME_TEXT: usize = 0x38;
    pub const X: usize = 0x48;
    pub const Y: usize = 0x4c;
    pub const Z: usize = 0x50;
    pub const AREA: usize = 0x54;
    pub const GRID_X: usize = 0x55;
    pub const GRID_Z: usize = 0x56;
}

/// A boss, as the game's own table lists it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Boss {
    pub id: i64,
    /// The id the table calls its "found" text. Kept because it is what the
    /// table holds, not because it has been shown to name anything.
    pub name_text: u32,
    /// Runes for killing it, alone.
    pub runes: u32,
    /// `mAA_BB_CC_00`, the map it stands on.
    pub map: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// One piece of armour, as the installed game has it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Armour {
    pub id: i64,
    pub weight: f32,
    /// Head, body, arms or legs. `None` for the rows that are not worn.
    ///
    /// Without it the answer to "what protects me best" is four chest pieces,
    /// and a player wears one of those. See [`slot`].
    pub worn: Option<&'static str>,
    /// Damage negation as the menu prints it: a percentage, higher is better.
    pub negation: Vec<(String, f32)>,
    /// Immunity, robustness, focus and vitality, only where there is any.
    pub resistance: Vec<(String, u16)>,
    /// Poise, as the stat screen counts it. `None` for a piece with none.
    ///
    /// It used to be absent and the note here said why: the two fields that
    /// sound like poise are useless, one reading 1.0 for every piece and the
    /// other 0. That was right about those two and wrong about the conclusion —
    /// the figure is at `0x014`, kept as a rate, and it was found by adding the
    /// worn pieces up against a screen that read 12. See [`poise`].
    pub poise: Option<f32>,
    /// Attributes this piece grants, out of the effects it carries.
    ///
    /// In this conversion 836 of 841 pieces carry one. Reading it is the
    /// difference between "the launcher has no such figure" and a robe that
    /// really does give faith +4 — and between those two sits an answer that
    /// invented a "+2 Faith" set because the mechanic was real and the reader
    /// was blind to it. See [`Regulation::what_an_effect_does`].
    pub gives: Vec<(String, i32)>,
    /// What it multiplies, as rates. Above one is more of the thing.
    pub changes: Vec<(String, f32)>,
    /// Flat additions, in points.
    pub adds: Vec<(String, i32)>,
}

/// A weapon as the installed game has it, in words rather than field names.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weapon {
    pub id: i64,
    /// How far it has been upgraded, taken from the id the player is holding.
    pub level: u8,
    pub weight: f32,
    /// Poison, rot, bleed and curse, where it builds any. Frost, sleep and
    /// madness are not read; see [`buildup`].
    pub ailments: Vec<(String, i32)>,
    /// What raising it stops, by kind, as the percentages the menu prints.
    /// `None` for a row that carries no such figure.
    ///
    /// On everything held, not only shields, because a sword blocks too and
    /// badly — the difference between 45 and 100 is the whole reason anybody
    /// carries a shield. See [`shield`] for how each offset was settled.
    pub blocks: Option<Vec<(String, f32)>>,
    /// Damage by kind, only the kinds it deals, with this weapon's upgrade
    /// already applied. What the stat screen shows on top of this is the
    /// scaling bonus, which depends on the character rather than the weapon.
    pub damage: Vec<(String, u16)>,
    /// What it scales on, as the hundredths the table stores.
    pub scaling: Vec<(String, f32)>,
    /// What it asks for before it can be held.
    pub needs: Vec<(String, u8)>,
    /// Health returned on a hit, when it returns any.
    pub regain: Option<u16>,
    /// What sort of thing it is — dagger, katana, greatshield — as `wepType`
    /// and the English word for it. See [`sort`].
    pub sort: Option<(u16, &'static str)>,
    /// Guard boost: how much of a blocked hit's force the arm keeps. The figure
    /// the equipment screen shows beside the five block percentages, and the
    /// one that decides whether a block staggers.
    ///
    /// Confirmed by shape, not by counting: sorted by this field the top of the
    /// game is eight greatshields, headed by the fingerprint stone family, and
    /// nothing of another class is near them.
    pub boost: Option<i16>,
}

/// Add a figure to a running list, or start it off.
fn add_up(running: &mut Vec<(String, i32)>, what: &str, value: i32) {
    match running.iter_mut().find(|(had, _)| had == what) {
        Some((_, total)) => *total += value,
        None => running.push((what.to_string(), value)),
    }
}

/// A class the player can start as, as this installation has it. See [`class`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartingClass {
    /// Its row in `BaseChrSelectMenuParam`, which is also its place in the menu.
    pub id: i64,
    /// The `CharaInitParam` row the stats were read from.
    pub stats_row: i64,
    /// The menu text holding its name, for whatever resolves text.
    pub name: u32,
    pub level: i16,
    /// The eight attributes, in the order the character screen lists them.
    pub attributes: Vec<(String, u8)>,
    /// What it starts holding: where it sits, and the item id.
    pub gear: Vec<(String, i64)>,
    /// The spells it starts knowing. Empty for most of them.
    pub spells: Vec<i64>,
}

/// A spirit ash as the installed game has it. See [`spirit`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summon {
    pub id: i64,
    /// Which of the two families it belongs to, as the table numbers them.
    pub sort: u8,
    /// FP to summon it. `None` throughout this installation — the conversion
    /// took the figure out of the field, and saying so is the honest answer.
    pub fp: Option<i16>,
    /// Whether it can be upgraded at all.
    pub upgrades: bool,
    /// The item id upgrading it consumes, and the runes it costs.
    pub material: Option<i32>,
    pub price: Option<i32>,
}

/// What a talisman does, read rather than described. See [`charm`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Charm {
    pub id: i64,
    pub weight: f32,
    /// The `SpEffectParam` row it applies, kept so an answer can be checked.
    pub effect: i64,
    /// Attributes it grants, or takes: `("strength", 5)`.
    pub gives: Vec<(String, i32)>,
    /// What it multiplies: `("physical taken", 1.15)`. Above one is more, below
    /// one is less, and which of those is GOOD depends on the field — 1.15 of
    /// damage taken is the price of a talisman, not its benefit.
    pub changes: Vec<(String, f32)>,
    /// Flat additions, in points rather than multipliers.
    pub adds: Vec<(String, i32)>,
}

/// The skill on a weapon: an ash of war, or the art it cannot be parted from.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    /// The `SwordArtsParam` row.
    pub id: i64,
    /// Which entry of the game's `ArtsName` table names it. The name itself is
    /// text rather than a number, so it is joined outside this reader.
    pub text: u32,
    /// What a press costs in FP, by button. Empty for the skills that cost
    /// nothing, which is most of the plain weapon arts.
    pub costs: Vec<(String, u16)>,
}

/// Every table in an installed regulation, by the name the archive gives it.
pub struct Regulation {
    tables: HashMap<String, Table>,
}

impl Regulation {
    /// Reads and unpacks a regulation from disk.
    pub fn open(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path).at(path)?;
        Self::from_bytes(&raw)
    }

    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let plain = decrypt(raw)?;
        let inner = uncompress(&plain)?;
        let files = unpack(&inner)?;

        let mut tables = HashMap::new();
        for (name, bytes) in files {
            // The archive names them by their path on somebody's build machine:
            // `N:\GR\data\Param\param\GameParam\merged\DLC02\EquipParamWeapon.param`.
            let short = name
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or(&name)
                .trim_end_matches(".param")
                .to_string();
            if let Some(table) = read_param(&bytes) {
                tables.insert(short, table);
            }
        }

        if tables.is_empty() {
            return Err(Error::Parse {
                what: "regulation".into(),
                detail: "unpacked, but no table inside it could be read".into(),
            });
        }
        Ok(Regulation { tables })
    }

    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.get(name)
    }

    /// One weapon, as the installed game has it.
    ///
    /// Zeroes are left out rather than reported: a weapon that deals no
    /// lightning damage should not be described as dealing zero lightning
    /// damage, and a list of eight numbers of which two matter reads as noise.
    /// What killing one of these gives, likeliest first.
    ///
    /// Empty when it drops nothing, when the tables have no lot for it, or when
    /// the installation has no lot table at all. The "nothing happens" slot is
    /// left out of the list but counted in the odds, so a one-in-thirty item
    /// reads as one in thirty rather than as a certainty.
    pub fn drops_from(&self, npc_id: i64) -> Vec<Drop> {
        let Some(npcs) = self.table("NpcParam") else {
            return Vec::new();
        };
        let Some(lots) = self.table("ItemLotParam_enemy") else {
            return Vec::new();
        };
        let Some(row) = npcs.i32(npc_id, npc::DROPS).map(i64::from).filter(|at| *at > 0) else {
            return Vec::new();
        };
        if !lots.has(row) {
            return Vec::new();
        }

        let weight = |slot: usize| f32::from(lots.u16(row, lot::WEIGHT + slot * 2).unwrap_or(0));
        let total: f32 = (0..lot::SLOTS).map(weight).sum();
        if total <= 0.0 {
            return Vec::new();
        }

        let mut out: Vec<Drop> = (0..lot::SLOTS)
            .filter_map(|slot| {
                let id = i64::from(lots.i32(row, lot::ITEM + slot * 4)?);
                if id <= 0 {
                    return None;
                }
                // A slot can name a real item at no weight at all, and a total
                // conversion leaves plenty of them: the empty outcome keeps its
                // 960 and the item is zeroed. Per the installed tables that
                // thing does not drop, and listing it at 0% reads as though it
                // might.
                let share = weight(slot);
                if share <= 0.0 {
                    return None;
                }
                let kind = category_table(lots.i32(row, lot::CATEGORY + slot * 4)?)?;
                Some(Drop {
                    id,
                    kind: what_a(kind).to_string(),
                    count: lots.u8(row, lot::COUNT + slot).unwrap_or(1).max(1),
                    chance: share / total * 100.0,
                })
            })
            .collect();
        out.sort_by(|a, b| b.chance.total_cmp(&a.chance));
        out
    }

    /// What a kind of damage does to something, where it is not the ordinary
    /// amount.
    ///
    /// A percentage of what the same hit would do to anything else: 60 means it
    /// shrugs most of that off, 140 means it hurts half again as much. Only the
    /// kinds that differ, because a list of eight hundreds is not an answer and
    /// a reader would have to find the two that matter in it.
    ///
    /// Empty for the great majority, which take everything as it comes.
    pub fn damage_taken_by(&self, npc_id: i64) -> Vec<(String, f32)> {
        let Some(npcs) = self.table("NpcParam") else {
            return Vec::new();
        };
        npc::TAKES
            .iter()
            .filter_map(|(kind, at)| {
                let rate = npcs.f32(npc_id, *at)?;
                // A rate outside this is not a rate: it is the wrong offset, or
                // a row the game does not use.
                if !rate.is_finite() || !(0.0..=5.0).contains(&rate) {
                    return None;
                }
                ((rate - 1.0).abs() > 0.005).then(|| ((*kind).to_string(), rate * 100.0))
            })
            .collect()
    }

    /// The skill a weapon carries, and what using it costs.
    ///
    /// `None` where the weapon has none, where the row it names is not one, or
    /// where the installation has no skill table at all — a wiki's answer for
    /// this is about a different game once a total conversion has moved the
    /// ashes around, so nothing is better than that.
    /// What raising this stops, by kind, as the percentages the menu prints.
    ///
    /// Every weapon has these, not only shields — a sword blocks too, worse.
    /// The caller decides whether the answer is worth giving; this only reads.
    /// See [`shield`] for how each offset was settled.
    pub fn blocks(&self, weapon_id: i64) -> Option<Vec<(String, f32)>> {
        let weapons = self.table("EquipParamWeapon")?;
        // Upgraded ids end in their level; the table is keyed on the base.
        let id = if weapons.has(weapon_id) { weapon_id } else { weapon_id - weapon_id % 100 };
        // A row with nothing in it reads as zero everywhere, which is not
        // "blocks none of it" but "this was never filled in".
        if weapons.f32(id, shield::PHYSICAL)? <= 0.0 {
            return None;
        }
        Some(
            shield::ALL
                .iter()
                .filter_map(|(what, at)| Some(((*what).to_string(), weapons.f32(id, *at)?)))
                .collect(),
        )
    }

    /// How much bleed this weapon builds up per hit, or `None` when it builds
    /// none.
    ///
    /// Followed through the effect the weapon hangs on itself; see [`buildup`]
    /// for how both ends of that were established and for the six ailments
    /// that are beside it and deliberately unnamed.
    pub fn bleeds(&self, weapon_id: i64) -> Option<i32> {
        self.ailments(weapon_id)
            .into_iter()
            .find(|(what, _)| *what == "bleed")
            .map(|(_, value)| value)
    }

    /// How far weapons upgrade here, and how many take each ceiling.
    ///
    /// A fact worth having in front of an answer rather than behind a tool
    /// call, because it is one that gets invented. Asked why weapons in this
    /// conversion cannot go past +10, a model agreed at length and made up a
    /// mechanism for it: the mod "removes the ordinary smithing stones and the
    /// somber stones and replaces them with its own materials, which stop at
    /// +10". It called no tool. The premise was false, and a false premise
    /// about a mod is the easiest thing in the world to agree with.
    ///
    /// Walked, not recalled: for each weapon's reinforce type, follow
    /// `ReinforceParamWeapon` from `kind + 1` until the row runs out. That is
    /// the same walk `upgrades` does to price each step, so the ceiling here
    /// and the ladder there cannot disagree.
    pub fn upgrade_ceilings(&self) -> Vec<(u8, usize)> {
        let (Some(weapons), Some(levels)) =
            (self.table("EquipParamWeapon"), self.table("ReinforceParamWeapon"))
        else {
            return Vec::new();
        };
        let mut tally: std::collections::BTreeMap<u8, usize> = Default::default();
        let mut seen: std::collections::HashMap<u16, u8> = Default::default();
        for id in weapons.ids().filter(|id| id % 100 == 0) {
            let Some(kind) = weapons.u16(id, weapon::REINFORCE_TYPE) else { continue };
            let ceiling = *seen.entry(kind).or_insert_with(|| {
                let mut highest = 0;
                for level in 1..=25u8 {
                    if !levels.has(i64::from(kind) + i64::from(level)) {
                        break;
                    }
                    highest = level;
                }
                highest
            });
            if ceiling > 0 {
                *tally.entry(ceiling).or_default() += 1;
            }
        }
        let mut out: Vec<(u8, usize)> = tally.into_iter().collect();
        out.sort_by_key(|(_, how_many)| std::cmp::Reverse(*how_many));
        out
    }

    /// Every crystal tear that can go into the wondrous physick.
    ///
    /// Returns the row id and what the tear does, read through the same
    /// effect route a talisman uses. About half carry nothing readable and
    /// come back with empty figures rather than being dropped, so an answer
    /// can name them and say their effect is not in the tables.
    pub fn tears(&self) -> Vec<(i64, Vec<(String, i32)>, Vec<(String, f32)>, Vec<(String, i32)>)> {
        let Some(table) = self.table("EquipParamGoods") else {
            return Vec::new();
        };
        table
            .ids()
            .filter(|id| table.u8(*id, spirit::SORT) == Some(spirit::TEAR))
            .map(|id| {
                let effect = i64::from(table.i32(id, spirit::EFFECT).unwrap_or(-1));
                let (gives, changes, adds) = self.what_an_effect_does(&[effect]);
                (id, gives, changes, adds)
            })
            .collect()
    }

    /// Every class the player can start as, in the order the menu shows them.
    ///
    /// Two tables joined — see [`class`] for which, and for why the join is
    /// read rather than computed from the vanilla numbering.
    pub fn classes(&self) -> Vec<StartingClass> {
        let (Some(menu), Some(stats)) =
            (self.table("BaseChrSelectMenuParam"), self.table("CharaInitParam"))
        else {
            return Vec::new();
        };
        let mut found: Vec<StartingClass> = menu
            .ids()
            .filter_map(|id| {
                let row = i64::from(menu.i32(id, class::STATS)?);
                // Rows 1000-1004 point at archetypes that are not in the table
                // in a shipped build. Letting them fall out here means the list
                // is the classes and nothing else, without hardcoding 2000.
                if !stats.has(row) {
                    return None;
                }

                // Which layout this regulation uses, decided by whether the
                // numbers add up rather than by any version stamp. See
                // `class::SHIFTS`.
                let (shift, level, attributes) = class::SHIFTS.iter().find_map(|shift| {
                    let level = stats.i16(row, class::LEVEL - shift)?;
                    if level <= 0 {
                        return None;
                    }
                    let attributes: Vec<(String, u8)> = class::ATTRIBUTES
                        .iter()
                        .filter_map(|(what, at)| {
                            Some(((*what).to_string(), stats.u8(row, at - shift)?))
                        })
                        .collect();
                    if attributes.len() != class::ATTRIBUTES.len() {
                        return None;
                    }
                    let spent: i64 =
                        attributes.iter().map(|(_, value)| i64::from(*value)).sum();
                    (spent - class::POINTS_AT_LEVEL_ZERO == i64::from(level))
                        .then_some((*shift, level, attributes))
                })?;

                Some(StartingClass {
                    id,
                    stats_row: row,
                    name: u32::try_from(menu.i32(id, class::NAME).unwrap_or(-1)).ok()?,
                    level,
                    attributes,
                    // What it is holding sits BEFORE the fields the DLC added,
                    // so these offsets do not move.
                    gear: class::GEAR
                        .iter()
                        .filter_map(|(where_, at)| {
                            // -1 is an empty hand or an empty slot, and every
                            // class has several. Only what is really there.
                            let item = stats.i32(row, *at).filter(|item| *item > 0)?;
                            Some(((*where_).to_string(), i64::from(item)))
                        })
                        .collect(),
                    spells: class::SPELLS
                        .iter()
                        .filter_map(|at| stats.i32(row, at - shift).filter(|spell| *spell > 0))
                        .map(i64::from)
                        .collect(),
                })
            })
            .collect();

        // `ids()` comes back in whatever order the table is stored in, which is
        // not the menu's. Sorting by row id restores it — the first class the
        // game offers is the lowest-numbered — and the test pins that, because
        // "the first class" is how anybody will refer to it.
        found.sort_by_key(|class| class.id);
        found
    }

    /// What a `reinforceMaterialId` actually asks for: item ids and how many.
    ///
    /// The field does NOT name an item. It names a row of `EquipMtrlSetParam`,
    /// and that row holds up to six ingredients — the same indirection the
    /// weapon upgrade path already goes through, and the reason `mtrl` exists.
    ///
    /// Treating it as an item id directly is wrong twice over, and both wrongs
    /// were measured rather than reasoned about. Looked up in the merged
    /// catalogue, material 10000 came back "Пепел Войны: Коготь льва" — an ash
    /// of WAR, because ids are unique only within a table and the catalogue
    /// keys six tables on the id alone. Narrowed to goods, the same id came
    /// back "Осколок стекла", a real item and still not the answer. Either
    /// would have been printed to a player as the thing to go and find.
    pub fn ingredients(&self, set: i64) -> Vec<(i64, i8)> {
        let Some(sets) = self.table("EquipMtrlSetParam") else {
            return Vec::new();
        };
        if set <= 0 || !sets.has(set) {
            return Vec::new();
        }
        (0..6)
            .filter_map(|at| {
                let item = sets.i32(set, mtrl::MATERIAL_ID[at])?;
                // -1 and 0 are both "no ingredient here".
                (item > 0).then(|| {
                    let count = sets.u8(set, mtrl::ITEM_COUNT[at]).unwrap_or(1) as i8;
                    (i64::from(item), count.max(1))
                })
            })
            .collect()
    }

    /// Every spirit ash this installation has, base rows only.
    ///
    /// The ids, for joining to whatever holds the names. See [`spirit`] for how
    /// they are told apart and for the one figure that is NOT readable here.
    pub fn spirits(&self) -> Vec<Summon> {
        let Some(table) = self.table("EquipParamGoods") else {
            return Vec::new();
        };
        table
            .ids()
            // Base rows only: +1 to +10 are the same ash, upgraded, and they
            // outnumber the ashes ten to one.
            .filter(|id| id % 100 == 0)
            .filter(|id| table.u8(*id, spirit::SUMMONS).is_some_and(|flag| flag > 0))
            .map(|id| Summon {
                id,
                sort: table.u8(id, spirit::SORT).unwrap_or(0),
                // -1 everywhere in this installation, and kept as an Option so
                // that stays visible rather than being printed as a cost.
                fp: table.i16(id, spirit::FP).filter(|cost| *cost > 0),
                upgrades: table.i32(id, spirit::NEXT).is_some_and(|next| next > 0),
                material: table.i32(id, spirit::MATERIAL).filter(|item| *item > 0),
                price: table.i32(id, spirit::PRICE).filter(|runes| *runes > 0),
            })
            .collect()
    }

    /// What a talisman does, in figures, and what it weighs.
    ///
    /// Only what it actually changes: a talisman leaves almost every field
    /// alone, and listing forty untouched multipliers of 1.0 would bury the two
    /// that matter. See [`charm`] for the fields and for the near-miss in
    /// choosing them.
    pub fn charm(&self, id: i64) -> Option<Charm> {
        let charms = self.table("EquipParamAccessory")?;
        let weight = charms.f32(id, charm::WEIGHT)?;
        let effect = i64::from(charms.i32(id, charm::EFFECT)?);
        let (gives, changes, adds) = self.what_an_effect_does(&[effect]);
        Some(Charm { id, weight, effect, gives, changes, adds })
    }

    /// What one or more `SpEffectParam` rows actually do, added together.
    ///
    /// Split out of the talisman reader when it turned out ARMOUR uses the same
    /// machinery: `EquipParamProtector` has three resident effect slots, and in
    /// this conversion 836 of its 841 pieces carry something real — faith +4 on
    /// one robe, arcane +6 on another, casting cost down two per cent. None of
    /// it was being read, and an answer asked for the lightest armour for a
    /// faith build INVENTED a "+2 Faith" set. The mechanic was real, the figure
    /// was not, and the two were a shared function apart.
    ///
    /// Several rows are summed because a piece may carry three. Only what is
    /// not idle comes back; see [`charm`] for the fields and for why choosing
    /// them mattered more than finding them.
    pub fn what_an_effect_does(
        &self,
        rows: &[i64],
    ) -> (Vec<(String, i32)>, Vec<(String, f32)>, Vec<(String, i32)>) {
        let (mut gives, mut changes, mut adds) = (Vec::new(), Vec::new(), Vec::new());
        let Some(effects) = self.table("SpEffectParam") else {
            return (gives, changes, adds);
        };
        for row in rows.iter().copied().filter(|row| *row > 0) {
            for (what, at) in charm::ATTRIBUTES {
                // s8, and taking an attribute away is a real thing to do.
                let Some(byte) = effects.u8(row, at) else { continue };
                let value = i32::from(byte as i8);
                if value != 0 {
                    add_up(&mut gives, what, value);
                }
            }
            for (what, at, idle) in charm::RATES {
                let Some(value) = effects.f32(row, at) else { continue };
                if (value - idle).abs() > 0.0005 {
                    // Multipliers multiply; two effects at 1.02 are 1.0404.
                    match changes.iter_mut().find(|(had, _)| had == what) {
                        Some((_, running)) => *running *= value,
                        None => changes.push((what.to_string(), value)),
                    }
                }
            }
            for (what, at) in charm::POINTS {
                let Some(value) = effects.i32(row, at) else { continue };
                if value != 0 {
                    add_up(&mut adds, what, value);
                }
            }
        }
        (gives, changes, adds)
    }

    /// Every ailment this weapon builds up, and how much of it per hit.
    ///
    /// Followed through the effects the weapon hangs on itself — up to three,
    /// and the ailment can be on any of them. See [`buildup`] for how the
    /// offsets were established and for the three that are deliberately not
    /// read.
    pub fn ailments(&self, weapon_id: i64) -> Vec<(&'static str, i32)> {
        let Some(weapons) = self.table("EquipParamWeapon") else {
            return Vec::new();
        };
        let Some(effects) = self.table("SpEffectParam") else {
            return Vec::new();
        };
        // Upgraded ids end in their level; the table is keyed on the base.
        let id = if weapons.has(weapon_id) { weapon_id } else { weapon_id - weapon_id % 100 };
        let hung: Vec<i64> = buildup::EFFECTS
            .iter()
            .filter_map(|at| weapons.i32(id, *at))
            .filter(|effect| *effect > 0)
            .map(i64::from)
            .collect();

        buildup::AILMENTS
            .iter()
            .filter_map(|(what, at)| {
                let most = hung
                    .iter()
                    .filter_map(|effect| effects.i32(*effect, *at))
                    .filter(|value| *value > 0)
                    .max()?;
                Some((*what, most))
            })
            .collect()
    }

    /// How much poise a piece of armour carries, as the screen counts it.
    ///
    /// See [`poise`] for how the field was found and what is deliberately not
    /// claimed about it.
    pub fn poise_of(&self, armour_id: i64) -> Option<f32> {
        let table = self.table("EquipParamProtector")?;
        let rate = table.f32(armour_id, poise::AT)?;
        (rate > 0.0).then_some(rate * poise::SCALE)
    }

    /// The most they can carry, worked out from their endurance.
    ///
    /// The missing half of every armour answer. The launcher could say a set
    /// weighs 34 and not whether they could wear it, which is the only part
    /// anybody asks — and an answer that tried to help invented a threshold of
    /// 23.0 where the real figure was 49.8, advice wrong by half.
    ///
    /// Curve 220, found by fingerprint against this installation's own screen:
    /// at endurance 11 it gives 49.8, which is what the equipment screen shows,
    /// and it is the only one of the game's 87 curves that does. Its shape is
    /// the right shape too — 45.0 at 1, 72.0 at 25, 160.0 at 99 — where a curve
    /// matching by luck would be flat or wild.
    ///
    /// A conversion may move this. It is read rather than remembered for that
    /// reason: whatever row 220 says in the installed regulation is what their
    /// game is using.
    pub fn can_carry(&self, endurance: u32) -> Option<f32> {
        self.along_curve(carrying::CURVE, endurance as f32)
    }

    /// How a load reads on the equipment screen: light, medium, heavy, or over.
    ///
    /// The bands are measured rather than recalled, off two screenshots of this
    /// installation: 14.0 of 49.8 is 28% and the screen says light; 20.0 of
    /// 49.8 is 40% and it says medium. Everything above that follows the same
    /// scheme and the last one is not a band but a state.
    ///
    /// `None` when the maximum is not known, because a band without a maximum
    /// is the guess this exists to stop.
    pub fn how_laden(&self, endurance: u32, carrying: f32) -> Option<(&'static str, f32)> {
        let most = self.can_carry(endurance)?;
        if most <= 0.0 {
            return None;
        }
        let share = carrying / most * 100.0;
        let band = match share {
            _ if share < carrying::LIGHT => "light",
            _ if share < carrying::MEDIUM => "medium",
            _ if share <= carrying::HEAVY => "heavy",
            _ => "overloaded",
        };
        Some((band, share))
    }

    pub fn skill_of(&self, weapon_id: i64) -> Option<Skill> {
        let weapons = self.table("EquipParamWeapon")?;
        // Upgraded ids end in their level; the table is keyed on the base.
        let id = if weapons.has(weapon_id) { weapon_id } else { weapon_id - weapon_id % 100 };
        let named = i64::from(weapons.i32(id, weapon::SKILL)?);
        if named <= 0 {
            return None;
        }

        self.skill_at(named)
    }

    /// One `SwordArtsParam` row, read directly.
    ///
    /// Split out of `skill_of` so an ASH OF WAR can use it. An ash is not a
    /// weapon and not a Good — it lives in `EquipParamGem` and points at one of
    /// these rows through [`gem::SKILL`] — so before this there was no way in
    /// by the ash's own name. Asked which ash was cheapest in FP, the launcher
    /// looked five of them up in the weapon table and missed five times.
    pub fn skill_at(&self, arts_row: i64) -> Option<Skill> {
        let arts = self.table("SwordArtsParam")?;
        if arts_row <= 0 || !arts.has(arts_row) {
            return None;
        }
        Some(Skill {
            id: arts_row,
            text: u32::try_from(arts.i32(arts_row, skill::TEXT).unwrap_or(-1)).ok()?,
            costs: skill::COSTS
                .iter()
                .filter_map(|(button, at)| {
                    // Signed, and -1 means the button does nothing. Read
                    // unsigned it becomes 65535 FP.
                    let cost = arts.i16(arts_row, *at)?;
                    (cost > 0).then(|| ((*button).to_string(), cost as u16))
                })
                .collect(),
        })
    }

    /// Every ash of war in this installation, with the skill it grants.
    ///
    /// The row id is the ash's own, for joining to whatever holds the names.
    pub fn ashes_of_war(&self) -> Vec<(i64, Option<Skill>)> {
        let Some(gems) = self.table("EquipParamGem") else {
            return Vec::new();
        };
        gems.ids()
            .map(|id| {
                let arts = gems.i32(id, gem::SKILL).unwrap_or(-1);
                (id, self.skill_at(i64::from(arts)))
            })
            .collect()
    }

    pub fn weapon(&self, id: i64) -> Option<Weapon> {
        let table = self.table("EquipParamWeapon")?;

        // An item id carries its upgrade in its last two digits: +5 Reduvia is
        // 1040005, and the table is keyed on 1040000. Passing either works, and
        // passing the one the player is holding is what gets their figures
        // rather than a shop's.
        let (id, level) = if table.has(id) {
            (id, 0u8)
        } else {
            let base = id - id % 100;
            let level = u8::try_from(id % 100).ok()?;
            if !table.has(base) || level > 25 {
                return None;
            }
            (base, level)
        };

        let some = |name: &str, value: u16| (value > 0).then(|| (name.to_string(), value));
        let scale = |name: &str, value: f32| (value.abs() > 0.01).then(|| (name.to_string(), value));
        let need = |name: &str, value: u8| (value > 0).then(|| (name.to_string(), value));

        // What this level multiplies by. A weapon with no upgrade path, or a
        // level the tables do not carry, keeps its own figures.
        let curve = self
            .table("ReinforceParamWeapon")
            // Two bytes, not four: read as an i32 it picks up the field behind
            // it and lands on a row that does not exist, which looks exactly
            // like a weapon that cannot be upgraded.
            .zip(table.u16(id, weapon::REINFORCE_TYPE))
            .map(|(curve, kind)| (curve, i64::from(kind) + i64::from(level)))
            .filter(|(curve, row)| curve.has(*row));
        let rate = |at: usize| -> f32 {
            curve
                .as_ref()
                .and_then(|(curve, row)| curve.f32(*row, at))
                .unwrap_or(1.0)
        };
        let raise = |base: u16, at: usize| -> u16 {
            let grown = f32::from(base) * rate(at);
            // The game floors these; rounding up would print one more than the
            // screen does on almost every weapon.
            grown.max(0.0).floor() as u16
        };
        Some(Weapon {
            id,
            level,
            ailments: self
                .ailments(id)
                .into_iter()
                .map(|(what, value)| (what.to_string(), value))
                .collect(),
            weight: table.f32(id, weapon::WEIGHT)?,
            blocks: (table.f32(id, shield::PHYSICAL).unwrap_or(0.0) > 0.0).then(|| {
                shield::ALL
                    .iter()
                    .filter_map(|(what, at)| Some(((*what).to_string(), table.f32(id, *at)?)))
                    .collect()
            }),
            damage: [
                some("physical", raise(table.u16(id, weapon::PHYSICAL)?, sharpen::PHYSICAL)),
                some("magic", raise(table.u16(id, weapon::MAGIC)?, sharpen::MAGIC)),
                some("fire", raise(table.u16(id, weapon::FIRE)?, sharpen::FIRE)),
                some(
                    "lightning",
                    raise(table.u16(id, weapon::LIGHTNING)?, sharpen::LIGHTNING),
                ),
                some("holy", raise(table.u16(id, weapon::HOLY)?, sharpen::HOLY)),
            ]
            .into_iter()
            .flatten()
            .collect(),
            // Named with the short form the game prints, which does not
            // translate. Given plain English, a model rendered arcane into
            // Russian as "Тьма" — a word no attribute has.
            scaling: [
                scale(
                    "strength (STR)",
                    table.f32(id, weapon::SCALE_STRENGTH)? * rate(sharpen::SCALE_STRENGTH),
                ),
                scale(
                    "dexterity (DEX)",
                    table.f32(id, weapon::SCALE_DEXTERITY)? * rate(sharpen::SCALE_DEXTERITY),
                ),
                scale(
                    "intelligence (INT)",
                    table.f32(id, weapon::SCALE_INTELLIGENCE)? * rate(sharpen::SCALE_INTELLIGENCE),
                ),
                scale(
                    "faith (FTH)",
                    table.f32(id, weapon::SCALE_FAITH)? * rate(sharpen::SCALE_FAITH),
                ),
                scale(
                    "arcane (ARC)",
                    table.f32(id, weapon::SCALE_ARCANE)? * rate(sharpen::SCALE_ARCANE),
                ),
            ]
            .into_iter()
            .flatten()
            .collect(),
            needs: [
                need("strength (STR)", table.u8(id, weapon::NEEDS_STRENGTH)?),
                need("dexterity (DEX)", table.u8(id, weapon::NEEDS_DEXTERITY)?),
                need("intelligence (INT)", table.u8(id, weapon::NEEDS_INTELLIGENCE)?),
                need("faith (FTH)", table.u8(id, weapon::NEEDS_FAITH)?),
                need("arcane (ARC)", table.u8(id, weapon::NEEDS_ARCANE)?),
            ]
            .into_iter()
            .flatten()
            .collect(),
            regain: table.u16(id, weapon::REGAIN_HP).filter(|hp| *hp > 0),
            sort: table
                .u16(id, sort::AT)
                .and_then(|kind| Some((kind, sort::english(kind)?))),
            boost: table.i16(id, weapon::GUARD_BOOST).filter(|value| *value > 0),
        })
    }

    /// One piece of armour out of the installed tables.
    ///
    /// The negation figures are turned the right way up on the way out: the
    /// table stores how much damage gets through and the game shows how much
    /// is stopped, so quoting the stored number would be quoting its opposite.
    pub fn armour(&self, id: i64) -> Option<Armour> {
        let table = self.table("EquipParamProtector")?;
        if !table.has(id) {
            return None;
        }
        // The three resident effect slots. Same machinery as a talisman, and in
        // this conversion nearly every piece uses it.
        let carried: Vec<i64> = armour::RESIDENT
            .iter()
            .filter_map(|at| table.i32(id, *at))
            .filter(|effect| *effect > 0)
            .map(i64::from)
            .collect();
        let (gives, changes, adds) = self.what_an_effect_does(&carried);
        let cut = |name: &str, at: usize| -> Option<(String, f32)> {
            let through = table.f32(id, at)?;
            // A rate of exactly 1 stops nothing, and a list of zeroes is noise.
            let stopped = (1.0 - through) * 100.0;
            (stopped.abs() > 0.05).then(|| (name.to_string(), stopped))
        };
        let resist = |name: &str, at: usize| -> Option<(String, u16)> {
            table.u16(id, at).filter(|value| *value > 0).map(|value| (name.to_string(), value))
        };

        Some(Armour {
            id,
            weight: table.f32(id, armour::WEIGHT)?,
            worn: table.u8(id, slot::WHERE).and_then(slot::called),
            poise: table.f32(id, poise::AT).filter(|rate| *rate > 0.0).map(|rate| rate * poise::SCALE),
            gives,
            changes,
            adds,
            negation: [
                cut("physical", armour::PHYSICAL),
                cut("slash", armour::SLASH),
                cut("strike", armour::STRIKE),
                cut("pierce", armour::PIERCE),
                cut("magic", armour::MAGIC),
                cut("fire", armour::FIRE),
                cut("lightning", armour::LIGHTNING),
                cut("holy", armour::HOLY),
            ]
            .into_iter()
            .flatten()
            .collect(),
            resistance: [
                resist("immunity — poison and rot", armour::IMMUNITY),
                resist("robustness — bleed and frost", armour::ROBUSTNESS),
                resist("focus — sleep and madness", armour::FOCUS),
                resist("vitality — death blight", armour::VITALITY),
            ]
            .into_iter()
            .flatten()
            .collect(),
        })
    }

    /// A sorcery or incantation out of the installed tables.
    pub fn spell(&self, id: i64) -> Option<Spell> {
        let table = self.table("Magic")?;
        if !table.has(id) {
            return None;
        }
        let need = |name: &str, at: usize| -> Option<(String, u8)> {
            table.u8(id, at).filter(|value| *value > 0).map(|value| (name.to_string(), value))
        };
        let fp = table.u16(id, spell::FP)? as i16;
        let held = table.u16(id, spell::FP_HELD).map(|value| value as i16);

        Some(Spell {
            id,
            fp,
            // Only when a held cast costs something different; otherwise it is
            // the same number twice and reads as if there were two costs.
            fp_held: held.filter(|value| *value > 0 && *value != fp),
            stamina: table.u16(id, spell::STAMINA)? as i16,
            slots: table.u8(id, spell::SLOTS)?,
            needs: [
                need("intelligence (INT)", spell::NEEDS_INTELLIGENCE),
                need("faith (FTH)", spell::NEEDS_FAITH),
                need("arcane (ARC)", spell::NEEDS_ARCANE),
            ]
            .into_iter()
            .flatten()
            .collect(),
        })
    }

    /// Where a stat value lands on one of the game's five-point curves.
    ///
    /// Returns a percentage. Between two points it is not a straight line: the
    /// exponent at the lower point bends it, and a negative exponent bends it
    /// the other way — `1 - (1 - r)^|adj|` rather than `r^adj`.
    fn along_curve(&self, row: i64, value: f32) -> Option<f32> {
        let graph = self.table("CalcCorrectGraph")?;
        let max: Vec<f32> = curve::MAX.iter().filter_map(|at| graph.f32(row, *at)).collect();
        let grow: Vec<f32> = curve::GROW.iter().filter_map(|at| graph.f32(row, *at)).collect();
        let adjust: Vec<f32> =
            curve::ADJUST.iter().filter_map(|at| graph.f32(row, *at)).collect();
        if max.len() < 5 || grow.len() < 5 || adjust.len() < 5 {
            return None;
        }

        // Below the first point and above the last, the curve is flat.
        if value <= max[0] {
            return Some(grow[0]);
        }
        if value >= max[4] {
            return Some(grow[4]);
        }
        let at = (0..4).find(|at| value <= max[at + 1])?;
        let span = max[at + 1] - max[at];
        if span <= 0.0 {
            return Some(grow[at]);
        }
        let ratio = (value - max[at]) / span;
        let bent = if adjust[at] >= 0.0 {
            ratio.powf(adjust[at])
        } else {
            1.0 - (1.0 - ratio).powf(adjust[at].abs())
        };
        Some(grow[at] + (grow[at + 1] - grow[at]) * bent)
    }

    /// What a weapon does in these hands: the base after upgrading, and what
    /// the character's attributes add on top of it.
    ///
    /// `stats` is strength, dexterity, intelligence, faith, arcane, in that
    /// order — the order every one of these tables keeps them in.
    pub fn attack_with(&self, weapon_id: i64, stats: [u32; 5]) -> Vec<Damage> {
        let Some(weapons) = self.table("EquipParamWeapon") else {
            return Vec::new();
        };
        let base_id = if weapons.has(weapon_id) { weapon_id } else { weapon_id - weapon_id % 100 };
        let level = u8::try_from(weapon_id - base_id).unwrap_or(0);
        let Some(kind) = weapons.u16(base_id, weapon::REINFORCE_TYPE) else {
            return Vec::new();
        };
        let row = i64::from(kind) + i64::from(level);
        let (Some(levels), Some(elements)) =
            (self.table("ReinforceParamWeapon"), self.table("AttackElementCorrectParam"))
        else {
            return Vec::new();
        };
        let Some(which) = weapons.i32(base_id, weapon::ELEMENT_CORRECT).map(i64::from) else {
            return Vec::new();
        };
        // Twenty-five one-bit flags, five stats for each of five damage types,
        // in the order the definition lists them.
        let flags = elements.i32(which, 0x00).unwrap_or(0) as u32;

        // Scaling per stat, already multiplied by what the upgrade does to it.
        let scaling: Vec<f32> = weapon::CORRECT
            .iter()
            .zip(reinforce::CORRECT_RATE.iter())
            .map(|(at, rate)| {
                weapons.f32(base_id, *at).unwrap_or(0.0) * levels.f32(row, *rate).unwrap_or(1.0)
            })
            .collect();

        weapon::DAMAGE
            .iter()
            .enumerate()
            .filter_map(|(kind_at, (name, base_at, rate_at, curve_at))| {
                let raw = f32::from(weapons.u16(base_id, *base_at)?);
                if raw <= 0.0 {
                    return None;
                }
                let base = raw * levels.f32(row, *rate_at).unwrap_or(1.0);
                let graph = i64::from(weapons.u8(base_id, *curve_at)?);

                let share: f32 = (0..5)
                    .filter(|stat_at| flags & (1 << (kind_at * 5 + stat_at)) != 0)
                    .filter_map(|stat_at| {
                        let along =
                            self.along_curve(graph, stats[stat_at] as f32)?;
                        Some(scaling[stat_at] / 100.0 * along / 100.0)
                    })
                    .sum();

                Some(Damage { kind: (*name).to_string(), base, bonus: base * share })
            })
            .collect()
    }

    /// Every step of upgrading a weapon, and what each one costs.
    ///
    /// Empty when the weapon has no upgrade path at all, which is a real answer
    /// and not a failure — some things cannot be reinforced.
    pub fn upgrade_steps(&self, weapon_id: i64) -> Vec<Step> {
        let weapons = match self.table("EquipParamWeapon") {
            Some(table) => table,
            None => return Vec::new(),
        };
        // The id may carry a level; the tables are keyed on the base.
        let base_id = if weapons.has(weapon_id) { weapon_id } else { weapon_id - weapon_id % 100 };
        let Some(base_set) = weapons.i32(base_id, weapon::MATERIAL_SET) else {
            return Vec::new();
        };
        let Some(kind) = weapons.u16(base_id, weapon::REINFORCE_TYPE) else {
            return Vec::new();
        };
        let (Some(levels), Some(sets)) =
            (self.table("ReinforceParamWeapon"), self.table("EquipMtrlSetParam"))
        else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for level in 1..=25u8 {
            let row = i64::from(kind) + i64::from(level);
            if !levels.has(row) {
                break;
            }
            let Some(added) = levels.u8(row, mtrl::SET_FROM_LEVEL) else {
                break;
            };
            let set = i64::from(base_set) + i64::from(added);
            if !sets.has(set) {
                continue;
            }
            let costs: Vec<(i64, i8)> = (0..6)
                .filter_map(|at| {
                    let item = sets.i32(set, mtrl::MATERIAL_ID[at])?;
                    // -1 and 0 are both "no ingredient here".
                    (item > 0).then(|| {
                        let count = sets.u8(set, mtrl::ITEM_COUNT[at]).unwrap_or(1) as i8;
                        (i64::from(item), count.max(1))
                    })
                })
                .collect();
            if !costs.is_empty() {
                out.push(Step { level, costs });
            }
        }
        out
    }

    /// Every talisman this installation has, by table id.
    ///
    /// Unnamed here on purpose: the ids are joined to the game's own text
    /// elsewhere, so this stays a reader of one file and the names stay in the
    /// player's language rather than a table's.
    pub fn talismans(&self) -> Vec<Talisman> {
        let Some(table) = self.table("EquipParamAccessory") else {
            return Vec::new();
        };
        table
            .ids()
            .filter_map(|id| Some(Talisman { id, weight: table.f32(id, talisman::WEIGHT)? }))
            .collect()
    }

    /// Every boss the game's own table knows, with where it stands.
    ///
    /// Rows with no name are the ones the table keeps for bookkeeping — an
    /// arena with no fight in it — and are left out.
    pub fn bosses(&self) -> Vec<Boss> {
        let Some(table) = self.table("GameAreaParam") else {
            return Vec::new();
        };
        let mut out: Vec<Boss> = table
            .ids()
            .filter_map(|id| {
                let text = table.i32(id, boss::NAME_TEXT)?;
                let name_text = u32::try_from(text).ok().filter(|value| *value > 0)?;
                Some(Boss {
                    id,
                    name_text,
                    runes: u32::try_from(table.i32(id, boss::RUNES)?).unwrap_or(0),
                    map: format!(
                        "m{:02}_{:02}_{:02}_00",
                        table.u8(id, boss::AREA)?,
                        table.u8(id, boss::GRID_X)?,
                        table.u8(id, boss::GRID_Z)?
                    ),
                    x: table.f32(id, boss::X)?,
                    y: table.f32(id, boss::Y)?,
                    z: table.f32(id, boss::Z)?,
                })
            })
            .collect();
        out.sort_by_key(|found| found.id);
        out
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }
}

/// Undoes the encryption, leaving the wrapper.
fn decrypt(raw: &[u8]) -> Result<Vec<u8>> {
    use aes::cipher::{BlockDecryptMut, KeyIvInit};

    if raw.len() < 32 {
        return Err(Error::Parse {
            what: "regulation".into(),
            detail: format!("only {} bytes, too short to hold anything", raw.len()),
        });
    }
    // Whole blocks only. The tail past the last one is padding the game does
    // not write meaningfully.
    let body = &raw[16..];
    let whole = body.len() / 16 * 16;

    let mut out = body[..whole].to_vec();
    let cipher = cbc::Decryptor::<aes::Aes256>::new(&KEY.into(), raw[..16].into());
    cipher
        .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut out)
        .map_err(|e| Error::Parse {
            what: "regulation".into(),
            detail: format!("could not decrypt: {e}"),
        })?;
    Ok(out)
}

/// Unwraps the DCX and decompresses what is inside it.
fn uncompress(plain: &[u8]) -> Result<Vec<u8>> {
    if !super::dcx::wraps(plain) {
        // Worth saying plainly here: at this point the bytes have already been
        // through the cipher, so anything other than a container means the key
        // was wrong rather than the file.
        return Err(Error::Parse {
            what: "regulation".into(),
            detail: format!(
                "decrypted to {:?} rather than a container — the key is wrong",
                plain.get(..4)
            ),
        });
    }
    super::dcx::expand(plain, "regulation")
}

/// Every file in the BND4 archive, as name and bytes.
///
/// The entry layout was read off the bytes rather than assumed, and it checks
/// itself: each file's offset plus its size lands on the next one, and the
/// stride is written in the header at `+0x20` — thirty-six here.
///
/// Shared with the message archives, which are the same container: it lives
/// here because this is where it was worked out and proven.
pub(crate) fn unpack(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let fail = |detail: String| Error::Parse { what: "regulation".into(), detail };
    if data.get(..4) != Some(b"BND4") {
        return Err(fail(format!("not an archive: {:?}", data.get(..4))));
    }

    let word = |at: usize| -> Option<i32> {
        Some(i32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    let long = |at: usize| -> Option<i64> {
        Some(i64::from_le_bytes(data.get(at..at + 8)?.try_into().ok()?))
    };

    let count = word(0x0c).unwrap_or(0);
    let stride = long(0x20).unwrap_or(0);
    if !(0..10_000).contains(&count) || !(16..256).contains(&stride) {
        return Err(fail(format!("{count} files on a stride of {stride}")));
    }

    let mut out = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let at = 0x40 + index * stride as usize;
        let (Some(size), Some(offset), Some(name_at)) =
            (long(at + 0x08), word(at + 0x18), word(at + 0x20))
        else {
            continue;
        };
        let (Ok(size), Ok(offset), Ok(name_at)) =
            (usize::try_from(size), usize::try_from(offset), usize::try_from(name_at))
        else {
            continue;
        };
        let Some(bytes) = data.get(offset..offset + size) else {
            continue;
        };
        out.push((wide_at(data, name_at), bytes.to_vec()));
    }
    Ok(out)
}

/// A UTF-16 name from the archive's string table.
pub(crate) fn wide_at(data: &[u8], at: usize) -> String {
    let mut units = Vec::new();
    let mut cursor = at;
    while let Some(pair) = data.get(cursor..cursor + 2) {
        let unit = u16::from_le_bytes([pair[0], pair[1]]);
        if unit == 0 || units.len() > 512 {
            break;
        }
        units.push(unit);
        cursor += 2;
    }
    String::from_utf16_lossy(&units)
}

/// The row index of one PARAM table.
///
/// Only where each row starts is kept, not the fields — which field means what
/// is a separate question, answered by the offsets the caller passes in.
fn read_param(bytes: &[u8]) -> Option<Table> {
    let count = u16::from_le_bytes(bytes.get(0x0a..0x0c)?.try_into().ok()?) as usize;
    if count == 0 || count > 200_000 {
        return None;
    }

    let mut rows = HashMap::with_capacity(count);
    for index in 0..count {
        let at = 0x40 + index * 24;
        let id = i64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?);
        let data_at = i64::from_le_bytes(bytes.get(at + 8..at + 16)?.try_into().ok()?);
        let Ok(data_at) = usize::try_from(data_at) else {
            continue;
        };
        if data_at < 0x40 || data_at >= bytes.len() {
            continue;
        }
        rows.insert(id, data_at);
    }
    (!rows.is_empty()).then(|| Table { rows, bytes: bytes.to_vec() })
}

/// Whether this game's tables are laid out the way everything in this file
/// assumes.
///
/// Every offset here was settled against ELDEN RING and checked against a
/// second reader of the same game. Three other titles the launcher manages ship
/// a `regulation.bin` under the same name — Nightreign and Armored Core VI at
/// least — and their rows are not this shape. Nothing stops the reader parsing
/// one: it would find tables, find rows, and report a weight from whatever four
/// bytes happen to sit at 0x010. Numbers like that are worse than none, because
/// they look like an answer.
///
/// Neither of those games is installed on the machine this was written on, so
/// whether they happen to match COULD NOT BE ESTABLISHED, and a guess in either
/// direction is the wrong kind. They are excluded until somebody can check.
///
/// The game is a parameter rather than a note in a comment so that the compiler
/// asks the question at every call site.
pub fn laid_out_like_this(game: crate::games::Game) -> bool {
    matches!(game, crate::games::Game::EldenRing)
}

/// The regulation the game will actually load, read once and kept.
///
/// Which one that is depends on the mod: a total conversion ships its own and
/// the loader puts it in front of the game's, so the mod's wins where there is
/// one. Parsing costs half a second and sixty-seven megabytes of decompression,
/// which is nothing once and too much per question.
///
/// `None` for a title whose layout has not been verified — see
/// [`laid_out_like_this`].
pub fn installed(
    game: crate::games::Game,
    game_dir: &Path,
    mod_dir: Option<&Path>,
) -> Option<Arc<Regulation>> {
    if !laid_out_like_this(game) {
        return None;
    }
    /// One file, as it stood when it was read, and what came out of it.
    type Kept = HashMap<PathBuf, (SystemTime, Arc<Regulation>)>;
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(Kept::new()));

    let path = mod_dir
        .map(|dir| dir.join("regulation.bin"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| game_dir.join("regulation.bin"));
    if !path.is_file() {
        return None;
    }
    // Re-read when the file changes, so editing the mod does not require
    // restarting the launcher to see it.
    let touched = std::fs::metadata(&path).ok()?.modified().ok()?;

    let mut held = kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((was, regulation)) = held.get(&path) {
        if *was == touched {
            return Some(Arc::clone(regulation));
        }
    }
    let regulation = Arc::new(Regulation::open(&path).ok()?);
    held.insert(path, (touched, Arc::clone(&regulation)));
    Some(regulation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_read_out_of_something_that_is_not_a_regulation() {
        // Each layer says which one it was, because "could not read it" over a
        // four-stage format is not a diagnosis.
        assert!(Regulation::from_bytes(&[]).is_err());

        // Sixty-four bytes of nothing decrypt to nothing in particular, which
        // must be caught at the container rather than parsed as one.
        let Err(noise) = Regulation::from_bytes(&[7u8; 64]) else {
            panic!("sixty-four bytes of nothing must not read as a regulation");
        };
        assert!(
            format!("{noise}").contains("container") || format!("{noise}").contains("decrypt"),
            "unhelpful: {noise}"
        );
    }

    #[test]
    fn a_row_is_never_read_past_its_end() {
        // A field asked for past the end of the data has to be absent rather
        // than zero: a zero would be quoted as a weapon that deals no damage.
        let mut bytes = vec![0u8; 8];
        bytes[0..2].copy_from_slice(&1234u16.to_le_bytes());
        bytes[4..8].copy_from_slice(&2.5f32.to_le_bytes());
        let table = Table { rows: HashMap::from([(1i64, 0usize)]), bytes };

        assert_eq!(table.u16(1, 0), Some(1234));
        assert_eq!(table.f32(1, 4), Some(2.5));
        assert_eq!(table.f32(1, 6), None, "half a float past the end");
        assert_eq!(table.u8(1, 99), None);
        assert_eq!(table.u16(2, 0), None, "no such row");
    }

    /// The whole chain, against the file this machine actually has.
    ///
    /// Skipped where the game is not installed, since the regulation is the
    /// game's own file and cannot live in the repository. The numbers are the
    /// fixture from `assets/param-layout.md`: Reduvia under The Convergence,
    /// agreed on by the mod's files, the vanilla files and the stat screen.
    #[test]
    fn the_installed_regulation_reads_the_weapon_it_should() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let weapons = regulation
            .table("EquipParamWeapon")
            .expect("it holds a weapon table");
        assert!(weapons.len() > 3000, "only {} weapons", weapons.len());

        let reduvia = 1_040_000;
        assert_eq!(weapons.u16(reduvia, 0x0c8), Some(0), "physical damage");
        assert_eq!(weapons.u16(reduvia, 0x0cc), Some(82), "fire damage");
        assert_eq!(weapons.f32(reduvia, 0x030), Some(65.0), "faith scaling");
        assert_eq!(weapons.f32(reduvia, 0x19c), Some(65.0), "arcane scaling");
        assert_eq!(weapons.f32(reduvia, 0x028), Some(50.0), "dexterity scaling");
        assert_eq!(weapons.u8(reduvia, 0x0f5), Some(13), "faith required");
        assert_eq!(weapons.u8(reduvia, 0x0f3), Some(8), "dexterity required");
        assert_eq!(weapons.f32(reduvia, 0x010), Some(2.5), "weight");
    }

    /// "Weapon" means every weapon, and every other word still means what it did.
    ///
    /// The second half is the point. Asking for all of them is one exact-match
    /// word added in front of a list that several real questions depend on, and
    /// "colossal weapon" is a class whose name ENDS in the new word — so the
    /// risk is not that the new case fails, it is that it quietly swallows an
    /// old one.
    #[test]
    fn asking_for_a_weapon_gets_all_of_them_and_a_class_still_gets_itself() {
        use super::sort;

        let every = sort::named("weapon");
        assert!(every.len() > 20, "every weapon class, got {}", every.len());
        for same in ["all", "any", "weapons", "all weapons", "everything"] {
            assert_eq!(sort::named(same), every, "{same:?} should mean every weapon");
        }
        // Not a weapon and deliberately absent: shields have their own family,
        // and nobody asking for the best weapon means an arrow or a torch.
        for (id, what) in [(65u16, "small shield"), (69, "greatshield"), (81, "arrow"), (87, "torch")] {
            assert!(!every.contains(&id), "{what} is not a weapon");
        }
        // In and expected — the classes the French question walked one by one.
        for (id, what) in [(13u16, "katana"), (3, "dagger"), (41, "colossal weapon"), (50, "bow")] {
            assert!(every.contains(&id), "{what} should be in every weapon");
        }

        // And the words that already worked still do.
        assert_eq!(sort::named("colossal weapon"), vec![41], "the class, not the family");
        assert_eq!(sort::named("shield").len(), 4, "shield is still four classes");
        assert!(sort::named("greatshield") == vec![69], "greatshield still beats shield");
        assert!(sort::named("katana").contains(&13));
        assert!(sort::named("sword").len() >= 8, "sword is still the sword family");
    }

    /// Inflicting an ailment and resisting one are read separately.
    ///
    /// The offsets are shared with what a weapon carries, so this is really
    /// asking whether the two lists have been kept apart: fourteen entries, no
    /// offset used twice, and every "inflicted" one matching the weapon table
    /// it was proven against. A talisman that raised bleed RESISTANCE offered
    /// as the answer to "best talisman for bleed damage" is the failure this
    /// prevents, and the two differ by one word in the label.
    #[test]
    fn what_a_thing_inflicts_is_not_what_it_resists() {
        use super::{buildup, charm};

        assert_eq!(charm::POINTS.len(), 14, "seven resisted and seven inflicted");

        // No offset may appear twice: one byte cannot be two figures.
        let mut seen: Vec<usize> = charm::POINTS.iter().map(|(_, at)| *at).collect();
        seen.sort_unstable();
        let mut unique = seen.clone();
        unique.dedup();
        assert_eq!(seen, unique, "an offset is claimed by two labels");

        // Every inflict entry must be the offset the weapon reader proved.
        for (what, at) in buildup::AILMENTS {
            let label = format!("{what} inflicted");
            let found = charm::POINTS.iter().find(|(name, _)| *name == label);
            assert_eq!(
                found.map(|(_, offset)| *offset),
                Some(at),
                "{label} must read the same byte the weapon does"
            );
        }

        // And the pairs must not collide — bleed resist and bleed inflicted are
        // different bytes, which is the whole point.
        for (what, _) in buildup::AILMENTS {
            let resist = charm::POINTS.iter().find(|(n, _)| *n == format!("{what} resist"));
            let inflict = charm::POINTS.iter().find(|(n, _)| *n == format!("{what} inflicted"));
            if let (Some((_, a)), Some((_, b))) = (resist, inflict) {
                assert_ne!(a, b, "{what}: resisting and inflicting cannot be one byte");
            }
        }
    }

    /// Every resistance, in every language somebody might ask in.
    ///
    /// Written after a Portuguese question for a bleed talisman came back with
    /// "there is no such talisman in this game". The stem was "sangrado", which
    /// is Spanish; the player wrote "sangramento", which is Portuguese; and
    /// `named` asks whether the query CONTAINS the stem, so the two never met.
    /// The launcher then reported the absence as a fact about the game.
    ///
    /// Kept as a table rather than a handful of asserts because the failure is
    /// silent: a stem that matches nothing looks exactly like a stem that
    /// matches, right up until somebody asks in that language.
    #[test]
    fn a_resistance_is_recognised_in_every_language_it_is_asked_in() {
        let asked: [(&str, &str); 24] = [
            // The one that was broken, and its neighbours.
            ("sangramento", "robustness"),
            ("sangrado", "robustness"),
            ("sangrar", "robustness"),
            ("bleed", "robustness"),
            ("кровотечение", "robustness"),
            ("robustez", "robustness"),
            ("congelamento", "robustness"),
            ("обморожение", "robustness"),
            // Immunity, and what it covers.
            ("immunity", "immunity"),
            ("imunidade", "immunity"),
            ("inmunidad", "immunity"),
            ("иммунитет", "immunity"),
            ("veneno", "immunity"),
            ("envenenamento", "immunity"),
            ("отравление", "immunity"),
            // Focus.
            ("focus", "focus"),
            ("concentración", "focus"),
            ("loucura", "focus"),
            ("locura", "focus"),
            ("безумие", "focus"),
            ("sono", "focus"),
            // Vitality.
            ("vitalidade", "vitality"),
            ("vitalidad", "vitality"),
            ("morte", "vitality"),
        ];
        for (word, wanted) in asked {
            assert_eq!(
                super::resistance::named(word),
                Some(wanted),
                "{word:?} should be understood as {wanted}"
            );
        }

        // And the reading that was nearly lost to the scoring: "мор" is death
        // blight, not "мороз", which is frost and two letters longer.
        assert_eq!(super::resistance::named("мор"), Some("vitality"));
        assert_eq!(super::resistance::named("мороз"), Some("robustness"));

        // A word that means none of them stays none of them. Without this the
        // test would pass just as well if `named` returned a resistance for
        // everything it was handed.
        for nothing in ["greatsword", "меч", "espada", "runes", "poise"] {
            assert_eq!(
                super::resistance::named(nothing),
                None,
                "{nothing:?} is not a resistance"
            );
        }
    }

    /// The starting classes, against the one everybody can recite.
    ///
    /// Same fixture discipline as the spell below: read the BASE game, where
    /// Vagabond is level 9 with 15 vigour, 10 mind, 11 endurance, 14 strength,
    /// 13 dexterity, 9 intelligence, 9 faith and 7 arcane, and starts holding a
    /// Longsword. Nine numbers and an item, all of them checkable by anybody
    /// who has opened the game, and every one of them arrives through the
    /// two-table join rather than from the row a mod might have moved.
    ///
    /// What this is really guarding is the join and the ORDER. Eight one-byte
    /// attributes in a row are the easiest thing in this file to read one byte
    /// off: get it wrong and Vagabond has 10 vigour and 11 mind, which is still
    /// a plausible-looking class and would be quietly served to a player.
    #[test]
    fn the_starting_classes_are_the_ones_the_game_ships() {
        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&game.join("regulation.bin")) else {
            return;
        };
        let classes = regulation.classes();
        if classes.is_empty() {
            return;
        }

        assert_eq!(classes.len(), 10, "the game ships ten classes");

        // The menu order is NOT the stats order, and assuming it was would have
        // been easy: 2000 through 2005 step neatly by two, and then Confessor
        // at 2008 points back at 3112 while Samurai at 2006 points at 3114.
        assert_eq!(
            classes.iter().map(|c| c.stats_row).collect::<Vec<_>>(),
            vec![3100, 3102, 3104, 3106, 3108, 3110, 3114, 3116, 3112, 3118],
            "the join is read from the table, not computed from the row number"
        );

        let vagabond = &classes[0];
        assert_eq!(vagabond.stats_row, 3100, "the join lands on Vagabond's row");
        assert_eq!(vagabond.level, 9, "Vagabond starts at level 9");
        assert_eq!(
            vagabond.attributes,
            vec![
                ("vigour".to_string(), 15),
                ("mind".to_string(), 10),
                ("endurance".to_string(), 11),
                ("strength".to_string(), 14),
                ("dexterity".to_string(), 13),
                ("intelligence".to_string(), 9),
                ("faith".to_string(), 9),
                ("arcane".to_string(), 7),
            ],
            "Vagabond's eight attributes, in the order the screen lists them"
        );
        assert_eq!(
            vagabond.gear.iter().find(|(where_, _)| where_ == "right hand"),
            Some(&("right hand".to_string(), 2_000_000)),
            "and it starts with the Longsword in its right hand"
        );

        // The level is the attributes, less the eight the game starts you with
        // and plus one. It holds for every class in the game and it is the
        // cheapest possible check that no row was read off by one.
        for class in &classes {
            let spent: u32 = class.attributes.iter().map(|(_, value)| u32::from(*value)).sum();
            assert_eq!(
                spent as i64 - 79,
                i64::from(class.level),
                "class {} at level {} carries attributes summing to {spent}",
                class.id,
                class.level
            );
        }
    }

    /// The gap between the table and the stat screen, closed.
    ///
    /// The table says 82 fire and the player's screen says 106, and for a long
    /// while that was written off as "the upgrade multiplier" without anybody
    /// checking. It is not the level: their Reduvia is `1040000`, which is +0.
    /// It is that this mod's reinforce curve starts at 1.3 rather than at 1,
    /// and 82 × 1.3 floors to exactly the 106 they are looking at.
    /// Spells, against a cost everybody knows.
    ///
    /// Read from the base game's own regulation rather than a mod's, because
    /// the point is the fixture: row 4000 is Glintstone Pebble, and Glintstone
    /// Pebble costs 7 FP and asks for 10 intelligence. If those two numbers
    /// come out of the bytes, the offsets are right.
    #[test]
    fn a_spell_costs_what_everybody_knows_it_costs() {
        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&game.join("regulation.bin")) else {
            return;
        };
        let Some(pebble) = regulation.spell(4000) else {
            return;
        };

        assert_eq!(pebble.fp, 7, "Glintstone Pebble costs 7 FP");
        assert_eq!(pebble.slots, 1);
        assert_eq!(
            pebble.needs,
            vec![("intelligence (INT)".to_string(), 10)],
            "and asks for 10 intelligence and nothing else"
        );

        // And the table as a whole reads like a table of spells rather than
        // like whatever happens to sit at those offsets.
        let all: Vec<Spell> = regulation
            .table("Magic")
            .into_iter()
            .flat_map(|table| table.ids().collect::<Vec<_>>())
            .filter_map(|id| regulation.spell(id))
            .collect();
        assert!(all.len() > 200, "only {} spells", all.len());
        let costing: Vec<i16> = all.iter().map(|one| one.fp).filter(|fp| *fp > 0).collect();
        assert!(costing.len() > 200, "only {} of them cost anything", costing.len());
        assert!(
            costing.iter().all(|fp| *fp < 200),
            "something costs {:?} FP",
            costing.iter().max()
        );
        assert!(all.iter().all(|one| one.slots <= 4), "a spell wants more than four slots");
    }

    /// The number on the player's own stat screen.
    ///
    /// Their Reduvia reads "Огонь 106 + 49" — 82 fire times the mod's 1.3
    /// upgrade rate, and 49 added by strength 10, dexterity 14, faith 22 and
    /// arcane 26. Both halves are checked, because base and bonus fail in
    /// different ways and a wrong one hiding behind a right one is the whole
    /// risk of a four-table join.
    #[test]
    fn a_weapon_hits_for_what_the_stat_screen_says() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&path) else {
            return;
        };

        let theirs = [10u32, 14, 0, 22, 26];
        let done = regulation.attack_with(1_040_000, theirs);
        let fire = done.iter().find(|one| one.kind == "fire").expect("Reduvia does fire damage");

        assert_eq!(fire.base.floor() as i32, 106, "base was {}", fire.base);
        assert_eq!(fire.bonus.floor() as i32, 49, "bonus was {}", fire.bonus);

        // Nothing else on this weapon: a join that leaks would light up a
        // damage type the weapon does not have.
        assert_eq!(done.len(), 1, "it also reads {:?}", done.iter().map(|d| &d.kind).collect::<Vec<_>>());

        // And more faith is more damage, which is the question this exists for.
        let more = regulation.attack_with(1_040_000, [10, 14, 0, 32, 26]);
        let after = more.iter().find(|one| one.kind == "fire").expect("still fire");
        assert!(after.bonus > fire.bonus, "ten more faith changed nothing");
    }

    /// The scaling maths, over every weapon rather than the one it was built
    /// against.
    ///
    /// Reduvia proved the formula; it cannot prove the formula generalises. A
    /// four-table join can be right for one row and wrong for a whole class —
    /// a weapon with two damage types, one with no scaling at all, one whose
    /// curve row is not the linear zero. So this runs the length of the table
    /// and asserts only what must hold for all of them, which is the shape of
    /// the answer rather than any one number.
    #[test]
    fn the_scaling_maths_holds_for_every_weapon() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&path) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamWeapon") else {
            return;
        };

        let theirs = [10u32, 14, 9, 22, 26];
        let mut looked = 0usize;
        let mut with_damage = 0usize;
        for id in table.ids() {
            // Base rows only: the +1..+25 variants are the same weapon.
            if id % 100 != 0 {
                continue;
            }
            looked += 1;
            for hit in regulation.attack_with(id, theirs) {
                with_damage += 1;
                assert!(hit.base.is_finite(), "{id} {} base is {}", hit.kind, hit.base);
                assert!(hit.bonus.is_finite(), "{id} {} bonus is {}", hit.kind, hit.bonus);
                assert!(hit.base > 0.0, "{id} {} kept a base of {}", hit.kind, hit.base);
                assert!(hit.bonus >= 0.0, "{id} {} scales to {}", hit.kind, hit.bonus);
                // Scaling adds to a weapon; it does not replace it several
                // times over. Anything past this is a misread curve, not a
                // strong weapon.
                assert!(
                    hit.bonus <= hit.base * 4.0,
                    "{id} {} adds {} on top of {}",
                    hit.kind,
                    hit.bonus,
                    hit.base
                );
            }
        }

        assert!(looked > 300, "only {looked} weapons were looked at");
        assert!(with_damage > 300, "only {with_damage} of them did any damage");
    }

    /// Upgrade paths, over the whole table rather than one weapon.
    #[test]
    fn every_upgrade_path_reads_like_one() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&path) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamWeapon") else {
            return;
        };

        let mut with_a_path = 0usize;
        let mut tops = std::collections::BTreeSet::new();
        for id in table.ids().filter(|id| id % 100 == 0) {
            let steps = regulation.upgrade_steps(id);
            if steps.is_empty() {
                continue;
            }
            with_a_path += 1;
            tops.insert(steps.last().map_or(0, |step| step.level));
            for (at, step) in steps.iter().enumerate() {
                assert_eq!(step.level as usize, at + 1, "{id} runs its levels out of order");
                assert!(!step.costs.is_empty(), "{id} +{} costs nothing", step.level);
                for (item, count) in &step.costs {
                    assert!(*item > 0, "{id} +{} wants item {item}", step.level);
                    assert!(
                        (1..=99).contains(count),
                        "{id} +{} wants {count} of {item}",
                        step.level
                    );
                }
            }
        }

        assert!(with_a_path > 200, "only {with_a_path} weapons can be upgraded");
        // Every path stops somewhere sensible. A join gone wrong walks off the
        // end and every weapon reads as +25.
        assert!(
            tops.iter().all(|top| (1..=25).contains(top)),
            "paths end at {tops:?}"
        );
    }

    /// The catalogue the game carries in memory, written down and read back.
    ///
    /// This is what a player without a mod has instead of the packed archives.
    /// Only runs with the game up, because that is the only moment the text
    /// exists outside sixty gigabytes of encrypted container.
    #[test]
    fn the_game_s_own_names_survive_being_written_down() {
        let game = crate::games::Game::EldenRing;
        if crate::text::every_name(game, crate::text::Kind::Talisman).is_none() {
            return; // the game is not running; nothing to catch
        }

        let scratch = std::env::temp_dir().join("roundtable-catalogue-check");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("a place to write");

        let written = crate::text::write_catalogue(&scratch, game);
        assert!(written > 5000, "only {written} names were written down");

        // Read back with the same call the launcher uses, and the same call
        // must work whether the game is up or not.
        for (kind, fewest) in [
            (crate::text::Kind::Weapon, 500usize),
            (crate::text::Kind::Talisman, 100),
            (crate::text::Kind::Goods, 500),
        ] {
            let back = crate::text::names(&scratch, game, None, None, kind);
            assert!(back.len() >= fewest, "{kind:?} came back with {}", back.len());
            assert!(
                back.iter().all(|(id, name)| *id > 0 && !name.trim().is_empty()),
                "{kind:?} has an empty name"
            );
        }

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The catalogue offers nothing the game does not name.
    ///
    /// These tables carry the studio's scaffolding alongside the real entries,
    /// and a listing of the ashes of war came back with "[ERROR]" among them —
    /// which is the launcher offering the player an item that does not exist.
    #[test]
    fn the_catalogue_holds_nothing_the_game_did_not_name() {
        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let dir = crate::testing::mod_dir(crate::games::Game::EldenRing);
        let shelf = crate::library::everything(&game, dir.as_deref(), "rusru");
        if shelf.is_empty() {
            return;
        }
        for one in shelf.iter() {
            let lower = one.name.to_lowercase();
            assert!(!lower.starts_with("[error]"), "{:?} is scaffolding", one.name);
            assert!(!lower.contains("%null%"), "{:?} is scaffolding", one.name);
            assert!(!one.name.trim().is_empty(), "an entry has no name at all");
        }
    }

    /// The gap the catalogue exists to close, stated as a check.
    ///
    /// This used to assert the opposite: a plain installation has no loose text
    /// at all — measured, nought names against a modded installation's ten
    /// thousand — because it keeps every one of them inside the packed
    /// archives. That was true and it was the bug, pinned as though it were a
    /// property. The archives open now, so the assertion is inverted, which is
    /// what landing the fix looks like from here.
    #[test]
    fn a_plain_installation_still_names_its_own_things() {
        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        crate::formats::oodle::register(&game);
        if !crate::formats::oodle::available() {
            return;
        }

        let bare = crate::library::everything(&game, None, "engus");
        assert!(bare.len() > 1000, "a plain installation named only {} things", bare.len());

        // Names alone would be half of it. What the catalogue is for is the
        // line that says what a thing does, and that is in the same archive.
        let described = bare.iter().filter(|one| one.effect.is_some()).count();
        assert!(described > 100, "only {described} of {} had an effect line", bare.len());
        // Every kind, not just whichever table happened to come out first.
        for want in ["weapon", "armour", "talisman", "item", "skill"] {
            assert!(
                bare.iter().any(|one| one.what == want),
                "nothing of kind {want} came out of the archives"
            );
        }

        let places = crate::places::everywhere(crate::places::Where {
            game: crate::games::Game::EldenRing,
            game_dir: &game,
            mod_dir: None,
            language: "engus",
            keep_in: None,
        });
        assert!(places.len() > 100, "only {} places without a mod", places.len());
    }

    /// The installed regulation's NPC table, for the map probe to check ids
    /// against. `None` when there is no installation to read.
    fn regulation_for_probe() -> Option<std::collections::BTreeSet<i64>> {
        let path = crate::testing::regulation(crate::games::Game::EldenRing)?;
        let regulation = Regulation::open(&path).ok()?;
        Some(regulation.table("NpcParam")?.ids().collect())
    }

    /// Walks a map file as far as this has got, and prints what it finds.
    ///
    /// Kept rather than deleted: it is the tool the next step needs, and
    /// re-deriving the block chain and the relative name offsets from scratch
    /// costs an afternoon. What it has established is written up in
    /// `assets/param-layout.md` under "The map files".
    ///
    /// Ignored because it prints rather than asserts — nothing here is settled
    /// enough to fail a build over. Run it with
    /// `cargo test --lib show_map_header -- --ignored --nocapture`.
    #[test]
    #[ignore = "a probe, not a check — see param-layout.md"]
    fn show_map_header() {
        let Some(dir) = crate::testing::mod_dir(crate::games::Game::EldenRing) else {
            println!("no mod");
            return;
        };
        let maps = dir.join("map").join("mapstudio");
        let Ok(entries) = std::fs::read_dir(&maps) else {
            println!("no map/mapstudio at {}", maps.display());
            return;
        };
        let mut names: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".msb.dcx"))
            .collect();
        names.sort();
        println!("{} msb files", names.len());
        let Some(one) = names.iter().find(|p| p.to_string_lossy().contains("m60_35_44")) else {
            println!("first is {:?}", names.first());
            return;
        };
        let raw = std::fs::read(one).unwrap();
        println!("{} — {} bytes packed, dcx={}", one.file_name().unwrap().to_string_lossy(), raw.len(), crate::formats::dcx::wraps(&raw));
        let plain = crate::formats::dcx::expand(&raw, "msb").unwrap();
        println!("{} bytes plain", plain.len());
        println!("first 64: {:02x?}", &plain[..64.min(plain.len())]);
        let text: String = plain[..96.min(plain.len())]
            .iter()
            .map(|b| if b.is_ascii_graphic() { *b as char } else { '.' })
            .collect();
        println!("as text: {text}");

        // Walk the blocks. Each is: version, offsetCount, nameOffset, then
        // offsetCount-1 entry offsets, then where the next block starts.
        let u32_at = |at: usize| -> u32 {
            u32::from_le_bytes(plain[at..at + 4].try_into().unwrap())
        };
        let u64_at = |at: usize| -> u64 {
            u64::from_le_bytes(plain[at..at + 8].try_into().unwrap())
        };
        let utf16_at = |at: usize| -> String {
            let mut out = String::new();
            let mut go = at;
            while go + 1 < plain.len() {
                let ch = u16::from_le_bytes([plain[go], plain[go + 1]]);
                if ch == 0 {
                    break;
                }
                out.push(char::from_u32(u32::from(ch)).unwrap_or('?'));
                go += 2;
            }
            out
        };

        let mut at = 0x10usize;
        for _ in 0..8 {
            if at + 16 > plain.len() {
                break;
            }
            let version = u32_at(at);
            let count = u32_at(at + 4) as usize;
            let name_at = u64_at(at + 8) as usize;
            if count == 0 || name_at >= plain.len() {
                break;
            }
            let next = u64_at(at + 8 + count * 8) as usize;
            println!(
                "  block at {at:#x}: version {version}, {} entries, name {:?}, next {next:#x}",
                count - 1,
                utf16_at(name_at)
            );
            if utf16_at(name_at) == "PARTS_PARAM_ST" {
                let entries: Vec<usize> = (0..count - 1)
                    .map(|n| u64_at(at + 16 + n * 8) as usize)
                    .collect();
                // The name offset is relative to the entry, not the file.
                let named = |one: usize| utf16_at(one + u64_at(one) as usize);
                println!("  first 6 part names:");
                for one in entries.iter().take(6) {
                    println!(
                        "    {one:#x} {:?}  head {:02x?}",
                        named(*one),
                        &plain[*one + 8..*one + 40]
                    );
                }
                let chrs: Vec<&usize> =
                    entries.iter().filter(|one| named(**one).starts_with('c')).collect();
                println!("  {} entries named like a character", chrs.len());
                // Which offset inside the type-specific block holds an NPC id,
                // found by asking the regulation rather than by guessing: the
                // right offset is the one whose value is a real NpcParam row
                // for enemy after enemy. A guessed offset agrees with one.
                let npcs = regulation_for_probe();
                let real = |value: u32| -> bool {
                    value > 0 && npcs.as_ref().is_some_and(|ids| ids.contains(&i64::from(value)))
                };
                let mut agrees: std::collections::BTreeMap<usize, usize> =
                    std::collections::BTreeMap::new();
                let mut looked = 0usize;
                for one in &chrs {
                    let start = **one;
                    // The player's own model and the system placeholder carry
                    // nothing; they were the first two in the list and sent the
                    // last attempt looking at a block of zeros.
                    let name = named(start);
                    if name.starts_with("c0000") || name.starts_with("c1000") {
                        continue;
                    }
                    let rel = u64_at(start + 0x50 + 3 * 8) as usize;
                    if rel == 0 || start + rel + 0x80 > plain.len() {
                        continue;
                    }
                    looked += 1;
                    for step in 0..32 {
                        let at = start + rel + step * 4;
                        if real(u32_at(at)) {
                            *agrees.entry(step * 4).or_default() += 1;
                        }
                    }
                }
                println!("  {looked} ordinary enemies looked at, npc table {:?} rows",
                    npcs.as_ref().map(std::collections::BTreeSet::len));
                for (at, hits) in agrees.iter().filter(|(_, hits)| **hits >= 3) {
                    println!("    +{at:#04x} is a real NpcParam row in {hits} of them");
                }
                // The whole chain: the enemy placed here, the NPC row it points
                // at, the name that row's text id resolves to, and what it is
                // worth. This is what the map files were opened for.
                let path = crate::testing::regulation(crate::games::Game::EldenRing);
                let reg = path.and_then(|p| Regulation::open(&p).ok());
                let npc_names: std::collections::HashMap<u32, String> =
                    crate::text::every_name(crate::games::Game::EldenRing, crate::text::Kind::Npc)
                        .map(|from| from.into_iter().collect())
                        .unwrap_or_default();
                println!("  {} npc names from the running game", npc_names.len());
                if let Some(reg) = &reg {
                    if let Some(table) = reg.table("NpcParam") {
                        let mut seen = std::collections::BTreeSet::new();
                        for one in &chrs {
                            let start = **one;
                            let rel = u64_at(start + 0x50 + 3 * 8) as usize;
                            if rel == 0 || start + rel + 0x10 > plain.len() {
                                continue;
                            }
                            let npc = i64::from(u32_at(start + rel + 0x0c));
                            if !table.has(npc) || !seen.insert(npc) {
                                continue;
                            }
                            let name = table
                                .i32(npc, 0x00c)
                                .and_then(|text| u32::try_from(text).ok())
                                .and_then(|text| npc_names.get(&text).cloned());
                            println!(
                                "    {:<14} npc {npc:>9}  hp {:>7?}  runes {:>7?}  {:?}",
                                named(start),
                                table.i32(npc, 0x024),
                                table.i32(npc, 0x02c),
                                name
                            );
                        }
                    }
                }
            }
            if next == 0 || next <= at {
                break;
            }
            at = next;
        }
    }

    /// What upgrading costs, against a second reader of the same file.
    ///
    /// The join is a sum — the weapon's own `materialSetId` plus the one on the
    /// reinforce row — and a sum is the kind of thing that lands on a real row
    /// belonging to something else when it is wrong, which reads as an answer.
    /// So it is checked against SmithBox, which reads these same bytes with
    /// FromSoftware's own definition: `EquipMtrlSetParam` row 2201 holds
    /// material 10160, one of it, and nothing else. Reduvia's first step must
    /// come out as exactly that.
    #[test]
    fn upgrading_costs_what_another_reader_says_it_does() {
        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&game.join("regulation.bin")) else {
            return;
        };

        let steps = regulation.upgrade_steps(1_040_000);
        assert!(steps.len() >= 10, "only {} steps", steps.len());
        assert_eq!(steps[0].level, 1);
        assert_eq!(steps[0].costs, vec![(10160, 1)], "the first step of Reduvia");

        // And the path reads like an upgrade path rather than like whatever
        // happens to sit at those offsets: one level after another, and nobody
        // is asked for a thousand of anything.
        for (at, step) in steps.iter().enumerate() {
            assert_eq!(step.level as usize, at + 1, "levels run out of order");
            for (item, count) in &step.costs {
                assert!(*item > 0, "step {} wants item {item}", step.level);
                assert!((1..=50).contains(count), "step {} wants {count} of {item}", step.level);
            }
        }
    }

    /// Talismans, against a second reader of the same file.
    ///
    /// There is no talisman whose weight everybody knows the way everybody
    /// knows what Glintstone Pebble costs, so the fixture is borrowed instead:
    /// SmithBox reads this same base-game file with FromSoftware's own field
    /// definition, and says row 1010 weighs 0.3. Two unrelated readers landing
    /// on the same number out of the same bytes is the check.
    #[test]
    fn a_talisman_weighs_what_another_reader_says_it_does() {
        let Some(game) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&game.join("regulation.bin")) else {
            return;
        };

        let all = regulation.talismans();
        assert!(all.len() > 100, "only {} talismans", all.len());

        let one = all.iter().find(|one| one.id == 1010).expect("row 1010 is there");
        assert!((one.weight - 0.3).abs() < 0.001, "row 1010 weighs {}", one.weight);

        // And the column reads like weights rather than like whatever happens
        // to sit at that offset: talismans are light, and none of them is free
        // of weight entirely in the base game.
        assert!(
            all.iter().all(|one| one.weight >= 0.0 && one.weight < 10.0),
            "a talisman weighs {:?}",
            all.iter().map(|one| one.weight).fold(f32::NAN, f32::max)
        );
    }

    /// The boss table, as far as it has been shown to be right.
    ///
    /// What is pinned is what was checked: real maps out of the row ids, and
    /// rewards of a shape only real rewards have. The names are pointedly not
    /// pinned, because they are not in this table.
    #[test]
    fn the_boss_table_gives_real_maps_and_real_rewards() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let found = regulation.bosses();
        if found.is_empty() {
            return;
        }
        assert!(found.len() > 100, "only {} bosses", found.len());

        for one in &found {
            assert!(
                one.map.starts_with('m') && one.map.len() == 12,
                "{} is not a map name",
                one.map
            );
            assert!(one.runes < 5_000_000, "{} runes for row {}", one.runes, one.id);
        }

        // Rewards spread across the game rather than sitting on one number,
        // which is what a wrongly-read field looks like.
        let paying = found.iter().filter(|one| one.runes > 0).count();
        assert!(paying > 100, "only {paying} of them pay anything");
        let most = found.iter().map(|one| one.runes).max().unwrap_or(0);
        let least = found.iter().filter(|o| o.runes > 0).map(|o| o.runes).min().unwrap_or(0);
        assert!(most > 100_000, "the biggest reward is only {most}");
        assert!(least < 10_000, "the smallest reward is already {least}");

        // The legacy dungeons are in there under their own maps.
        assert!(
            found.iter().any(|one| one.map.starts_with("m10_00")),
            "nothing is placed in Stormveil"
        );
    }

    /// What the skills come out as, for reading rather than asserting.
    ///
    /// `cargo test --lib show_skills -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_skills() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let shelf = crate::library::everything(&game_dir, mod_dir.as_deref(), language);
        let named: HashMap<u32, &str> = shelf
            .iter()
            .filter(|one| one.what == "skill")
            .map(|one| (one.id, one.name.as_str()))
            .collect();
        println!("{} skills named in {language}", named.len());

        let Some(weapons) = regulation.table("EquipParamWeapon") else {
            return;
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut shown = 0;
        for id in weapons.ids() {
            let Some(skill) = regulation.skill_of(id) else {
                continue;
            };
            if !seen.insert(skill.id) {
                continue;
            }
            let costs: Vec<String> =
                skill.costs.iter().map(|(b, fp)| format!("{b} {fp}FP")).collect();
            if shown < 25 {
                println!(
                    "  weapon {id:>8} -> skill {:>4} text {:>5}  {:<34} {}",
                    skill.id,
                    skill.text,
                    named.get(&skill.text).copied().unwrap_or("<unnamed>"),
                    costs.join(", ")
                );
                shown += 1;
            }
        }
        println!("{} distinct skills across the weapons", seen.len());
    }

    /// Which table each lot category points into, decided by the whole table.
    ///
    /// One lookup proves nothing: the id spaces overlap, so a goods id can also
    /// be a live weapon row by coincidence. What settles it is asking every
    /// (category, id) pair in the lot table and seeing which of the four
    /// equipment tables holds nearly all of a category's ids and the others
    /// hold few.
    ///
    /// `cargo test --lib show_lot_categories -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_lot_categories() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let Some(lots) = regulation.table("ItemLotParam_enemy") else {
            println!("no lot table");
            return;
        };
        const TABLES: [&str; 5] = [
            "EquipParamGoods",
            "EquipParamWeapon",
            "EquipParamProtector",
            "EquipParamAccessory",
            "EquipParamGem",
        ];

        let mut seen: std::collections::BTreeMap<i32, Vec<i64>> = Default::default();
        for row in lots.ids() {
            for slot in 0..lot::SLOTS {
                let (Some(id), Some(kind)) = (
                    lots.i32(row, lot::ITEM + slot * 4),
                    lots.i32(row, lot::CATEGORY + slot * 4),
                ) else {
                    continue;
                };
                if id > 0 {
                    seen.entry(kind).or_default().push(i64::from(id));
                }
            }
        }

        println!("{} rows in the lot table", lots.len());
        for (kind, ids) in &seen {
            print!("  category {kind:>2}: {:>6} ids ", ids.len());
            for name in TABLES {
                let Some(table) = regulation.table(name) else {
                    continue;
                };
                // A weapon in a lot is given at +0, but check both so an
                // upgraded id does not read as a miss.
                let hits = ids
                    .iter()
                    .filter(|id| table.has(**id) || table.has(*id - *id % 100))
                    .count();
                print!("{name} {:>3}%  ", hits * 100 / ids.len().max(1));
            }
            println!();
            // The small categories are the ambiguous ones, so show their ids.
            if ids.len() <= 25 {
                let mut sample: Vec<i64> = ids.clone();
                sample.sort_unstable();
                sample.dedup();
                println!("      ids: {sample:?}");
            }
        }

        // Where a table cannot decide it, the game's own text can: whichever
        // name table has the id is the kind the player is shown.
        let Some(game_dir) = crate::testing::game_dir(crate::games::Game::EldenRing) else {
            return;
        };
        crate::formats::oodle::register(&game_dir);
        let mod_dir = crate::testing::mod_dir(crate::games::Game::EldenRing);
        let shelf = crate::library::everything(&game_dir, mod_dir.as_deref(), "engus");
        println!("\n  what the text calls the ambiguous ones:");
        for id in [6070u32, 20900] {
            let named: Vec<String> = shelf
                .iter()
                .filter(|one| one.id == id)
                .map(|one| format!("[{}] {}", one.what, one.name))
                .collect();
            println!("    {id}: {}", if named.is_empty() { "nothing".into() } else { named.join(" | ") });
        }
    }

    /// What things drop, over every enemy in the tables.
    ///
    /// Three separate things have to be right and each fails differently. The
    /// id in `NpcParam` has to point at a real lot row. Every category has to
    /// name a table that really has its ids — which is how the mapping was
    /// derived, and it collapses if any offset moved. And the odds have to be
    /// odds: a share of the row's own total, never over a hundred, never a
    /// certainty for something the game gives one time in thirty.
    #[test]
    fn what_things_drop_is_a_real_item_at_real_odds() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let (Some(npcs), Some(lots)) =
            (regulation.table("NpcParam"), regulation.table("ItemLotParam_enemy"))
        else {
            return;
        };
        assert!(lots.len() > 100, "only {} lots", lots.len());

        let mut dropping = 0usize;
        let mut items = 0usize;
        for npc in npcs.ids() {
            let found = regulation.drops_from(npc);
            if found.is_empty() {
                continue;
            }
            dropping += 1;
            let mut share = 0.0f32;
            for one in &found {
                items += 1;
                // The kind has to be a kind, and the id has to be a row of it.
                let table = match one.kind.as_str() {
                    "weapon" => "EquipParamWeapon",
                    "armour" => "EquipParamProtector",
                    "talisman" => "EquipParamAccessory",
                    "ash of war" => "EquipParamGem",
                    "item" => "EquipParamGoods",
                    other => panic!("npc {npc} drops a {other}"),
                };
                if let Some(rows) = regulation.table(table) {
                    assert!(
                        rows.has(one.id) || rows.has(one.id - one.id % 100),
                        "npc {npc} drops {} {}, which is not a row of {table}",
                        one.kind,
                        one.id
                    );
                }
                assert!(one.count >= 1, "npc {npc} drops {} of something", one.count);
                assert!(
                    one.chance > 0.0 && one.chance <= 100.0,
                    "npc {npc} drops {} at {}%",
                    one.id,
                    one.chance
                );
                share += one.chance;
            }
            // The empty slot is left out of the list but counted in the odds,
            // so the listed shares add up to at most everything.
            assert!(share <= 100.5, "npc {npc}'s drops add up to {share}%");
        }

        assert!(dropping > 50, "only {dropping} things drop anything at all");
        assert!(items > dropping, "no enemy had more than one thing to give");
        // Something in the world has to be a rarity, or the odds are not odds.
        let rare = npcs
            .ids()
            .flat_map(|npc| regulation.drops_from(npc))
            .any(|one| one.chance < 20.0);
        assert!(rare, "every drop in the game is likelier than one in five");
    }

    /// Ranking armour by a kind of damage gives a different answer per kind.
    ///
    /// The check that matters is the last one. A ranking that ignored the kind
    /// and sorted by overall protection would look perfectly reasonable — the
    /// heavy plate would top every list — and would answer "what should I wear
    /// against fire" with the same thing it answers for lightning. What says it
    /// is really keyed on the kind is that the lists differ, and that the piece
    /// topping fire is one whose own name is about fire.
    #[test]
    fn armour_ranks_differently_for_different_damage() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamProtector") else {
            return;
        };

        let ranked = |kind: &str| -> Vec<(i64, f32)> {
            let mut out: Vec<(i64, f32)> = table
                .ids()
                .filter_map(|id| {
                    let piece = regulation.armour(id)?;
                    if piece.weight <= 0.0 {
                        return None;
                    }
                    let stopped =
                        piece.negation.iter().find(|(what, _)| what == kind).map(|(_, v)| *v)?;
                    Some((id, stopped))
                })
                .collect();
            out.sort_by(|a, b| b.1.total_cmp(&a.1));
            out
        };

        let fire = ranked("fire");
        let lightning = ranked("lightning");
        assert!(fire.len() > 100, "only {} pieces rated against fire", fire.len());
        assert_eq!(fire.len(), lightning.len(), "the kinds cover different pieces");

        for list in [&fire, &lightning] {
            let mut best = f32::INFINITY;
            for (id, stopped) in list.iter() {
                assert!(
                    (-60.0..95.0).contains(stopped),
                    "{id} stops {stopped}% — not a percentage anybody wears"
                );
                assert!(*stopped <= best + 0.01, "{id} is out of order at {stopped}");
                best = *stopped;
            }
        }

        // The two orders must not be the same list. If they were, the kind is
        // being ignored and every question gets the heaviest plate.
        let fire_top: Vec<i64> = fire.iter().take(20).map(|(id, _)| *id).collect();
        let lightning_top: Vec<i64> = lightning.iter().take(20).map(|(id, _)| *id).collect();
        assert_ne!(fire_top, lightning_top, "every kind ranks the same, so the kind is ignored");
    }

    /// Which field in a piece of armour is its poise.
    ///
    /// The last figure the equipment screen shows that this launcher does not
    /// read. Asked how much poise they had, the honest answer is that it is not
    /// known — which is right, and is also the answer to the question everybody
    /// asks about heavy armour.
    ///
    /// Anchored the way the carrying curve was: this installation's screen
    /// reads "Баланс 12" while the four worn pieces are on. So the field is one
    /// whose four values add to 12, or to something a fixed scale turns into
    /// 12 — the game stores poise as a rate in some titles and as a number in
    /// others, and which of those it is here is exactly what this settles.
    ///
    /// `cargo test --lib show_poise -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, and it needs the game running"]
    fn show_poise() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamProtector") else {
            return;
        };
        let Some(live) = crate::live::read(game) else {
            println!("  the game is not running, so there is nothing to add up");
            return;
        };
        let Some(gear) = live.gear.as_ref() else {
            return;
        };
        // The live read gives armour by name, not by id — only weapons carry
        // their numbers. So the names go back through the catalogue.
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: HashMap<String, u32> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "armour")
                .map(|one| (one.name.clone(), one.id))
                .collect();
        let worn: Vec<i64> = gear
            .armour
            .iter()
            .filter_map(|(_, name)| named.get(name).map(|id| i64::from(*id)))
            .collect();
        println!("\n  wearing {:?}", gear.armour);
        println!("  resolved to {worn:?}");
        if worn.is_empty() {
            return;
        }

        // What the screen says while those are on.
        const POISE: f32 = 12.0;
        println!("  looking for fields adding to {POISE}\n");

        let mut near: Vec<(f32, String)> = Vec::new();
        for at in (0..416).step_by(2) {
            // Floats on their alignment, and whole numbers on theirs, because
            // the game stores poise as a rate in some titles and a count in
            // others and which it is here is the question.
            let mut readings: Vec<(&str, Vec<f32>)> = Vec::new();
            if at % 4 == 0 {
                let floats: Vec<f32> = worn.iter().filter_map(|id| table.f32(*id, at)).collect();
                if floats.len() == worn.len() {
                    readings.push(("f32", floats));
                }
            }
            let words: Vec<f32> =
                worn.iter().filter_map(|id| table.u16(*id, at)).map(f32::from).collect();
            if words.len() == worn.len() {
                readings.push(("u16", words));
            }

            for (kind, values) in readings {
                let total: f32 = values.iter().sum();
                if total <= 0.0 {
                    continue;
                }
                // Straight, or through the scales a rate is kept in.
                for (scale, how) in
                    [(1.0, "as-is"), (100.0, "×100"), (1000.0, "×1000"), (0.001, "÷1000")]
                {
                    let got = total * scale;
                    let off = (got - POISE).abs();
                    if off < 0.05 {
                        println!(
                            "    0x{at:03x} {kind} {how}: {} = {got:.2}",
                            values
                                .iter()
                                .map(|v| format!("{v:.3}"))
                                .collect::<Vec<_>>()
                                .join(" + ")
                        );
                    } else if off < 1.5 {
                        near.push((off, format!("0x{at:03x} {kind} {how} = {got:.2}")));
                    }
                }
            }
        }
        // And the shape of the best candidate, because 11.70 against a screen
        // reading 12 is either rounding or a coincidence. If this really is
        // poise, the heaviest pieces carry far more of it than a surgeon's
        // robe does, and the order follows weight.
        const CANDIDATE: usize = 0x014;
        let mut heavy: Vec<(f32, f32, i64)> = table
            .ids()
            .filter_map(|id| {
                let piece = regulation.armour(id)?;
                let value = table.f32(id, CANDIDATE)? * 1000.0;
                (piece.weight > 0.0).then_some((value, piece.weight, id))
            })
            .collect();
        heavy.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("\n  0x014 ×1000, largest first, against weight:");
        for (value, weight, id) in heavy.iter().take(5) {
            println!("    {value:6.2}  weighs {weight:5.1}  row {id}");
        }
        let worn_sum: f32 =
            worn.iter().filter_map(|id| table.f32(*id, CANDIDATE)).sum::<f32>() * 1000.0;
        println!("    {worn_sum:6.2}  what they are wearing, screen says {POISE}");

        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.dedup_by(|a, b| a.1 == b.1);
        if !near.is_empty() {
            println!("\n  nearest misses:");
            for (off, what) in near.iter().take(8) {
                println!("    {what}  ({off:.2} away)");
            }
        }
    }

    /// Which curve turns endurance into how much they can carry.
    ///
    /// The missing half of every armour answer. The launcher reads what each
    /// piece weighs and what they are wearing, so it can say a set weighs 34 —
    /// and cannot say whether they can carry it, which is the only part they
    /// asked. Every armour answer ends with "that is on your equipment screen",
    /// and one that tried to help invented a threshold of 23.0 when the real
    /// figure was 49.8, which is advice wrong by half.
    ///
    /// One anchor is enough to CHECK a curve even though it is not enough to
    /// guess one: this installation, at endurance 11, shows a maximum of 49.8.
    /// So every curve in the game is evaluated at 11 and anything landing on
    /// 49.8 is a candidate — and then the one that is really it has to keep
    /// working at other values, which is what the second half prints.
    ///
    /// `cargo test --lib show_equip_curve -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_equip_curve() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(graph) = regulation.table("CalcCorrectGraph") else {
            return;
        };

        const ENDURANCE: f32 = 11.0;
        const CARRIES: f32 = 49.8;
        println!("\n  {} curves; looking for one that puts {ENDURANCE} at {CARRIES}",
                 graph.ids().count());

        let mut hits = 0;
        for row in graph.ids() {
            let Some(at) = regulation.along_curve(row, ENDURANCE) else {
                continue;
            };
            if (at - CARRIES).abs() > 0.05 {
                continue;
            }
            hits += 1;
            // What it does either side, so a flat curve that happens to sit at
            // 49.8 everywhere is told apart from one that really climbs.
            let shape: Vec<String> = [1.0, 11.0, 25.0, 40.0, 60.0, 99.0]
                .iter()
                .filter_map(|end| Some(format!("{end:.0}→{:.1}", regulation.along_curve(row, *end)?)))
                .collect();
            println!("    row {row}: {}", shape.join("  "));
        }
        if hits == 0 {
            println!("\n  Nothing lands on it, so the maximum is not a curve read this way —");
            println!("  it may be a base plus a curve, or somewhere else entirely.");
            // The closest few, to say which direction to look next.
            let mut near: Vec<(f32, i64)> = graph
                .ids()
                .filter_map(|row| Some(((regulation.along_curve(row, ENDURANCE)? - CARRIES).abs(), row)))
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (away, row) in near.iter().take(5) {
                let value = regulation.along_curve(*row, ENDURANCE).unwrap_or(0.0);
                println!("    row {row} gives {value:.2}, {away:.2} away");
            }
        }
    }

    /// Where bleed, frost, poison, rot, sleep and madness live.
    ///
    /// Not reading them is a hole with a shape: asked which weapons in this
    /// installation are good for bleed, the answer was "my searches of the
    /// weapon catalogue gave no results, and I could not find any daggers or
    /// knives matching bleed criteria" — followed by a paragraph of general
    /// Dark Souls recollection and a suggestion to go and read a forum. The
    /// tables hold the figure; nothing here looks at it.
    ///
    /// **It is not in the weapon's own row, and that is now measured.** Every
    /// u16 slot was swept, two bytes at a time so none was skipped, for a field
    /// carried by a minority of weapons at a buildup-sized number. With the
    /// fields this file already reads excluded, nothing matches at all.
    ///
    /// Then settled outright, with a number off the game's own screen. Reduvia
    /// in this installation reads "Накапливает кровотечение (82)" under passive
    /// effects. **Exactly one field in its entire row holds 82, and that field
    /// is `weapon::FIRE`.** So the buildup is not in the row at any offset,
    /// under any alignment — it is hung off the weapon by reference, through
    /// the SpEffect ids in its row and into `SpEffectParam`, a second table
    /// this launcher does not open. That is where the next attempt belongs.
    ///
    /// A coincidence worth naming, because it nearly cost an afternoon: this
    /// weapon's BASE fire damage is also 82 — the 106 its menu shows is that
    /// figure with the upgrade applied — so bleed and fire collide on the one
    /// weapon anybody would reach for to test bleed. What separated them was
    /// the company `0x0cc` keeps: the weapons at the top of it are named for
    /// lava, for flame, and for a fire knight.
    ///
    /// ## Where it actually is
    ///
    /// Read out of SmithBox, which gives these params with the field names the
    /// game ships and settled in one call what sweeping bytes could not:
    ///
    ///   `EquipParamWeapon.spEffectBehaviorId0` → a row of `SpEffectParam`,
    ///   which carries all seven ailments side by side as s32:
    ///   `poizonAttackPower`, `diseaseAttackPower`, `bloodAttackPower`,
    ///   `curseAttackPower`, and further along `freezeAttackPower`,
    ///   `sleepAttackPower`, `madnessAttackPower`.
    ///
    /// **`bloodAttackPower` is at `0x0d4`**, established by fingerprint the
    /// same way the guard rates were: vanilla row 6410 has 50 there, SmithBox
    /// says that row's `bloodAttackPower` is 50, and 0x0d4 is the ONLY offset
    /// in the row holding it. `SpEffectParam` has 16,002 rows and the launcher
    /// already opens it.
    ///
    /// Still wanted, and it is one probe: the offset of `spEffectBehaviorId0`
    /// in the weapon row. Vanilla Reduvia points at 6410; this installation's
    /// does NOT, because a total conversion rewrote the weapon and hung its own
    /// effect on it — the one carrying 82. So the id cannot be found by looking
    /// for 6410 in the mod's row, and the fingerprint has to come from a weapon
    /// whose effect the mod left alone.
    ///
    /// Do not count the offset down SmithBox's field order to get there. That
    /// is what produced two wrong offsets in this file before, because a
    /// `dummy8` carrying a bit size pads somebody else's byte rather than
    /// taking one.
    ///
    /// Two traps sprung on the way, both worth keeping:
    ///
    /// The first sweep offered `0x0cc` with a bleed-sounding weapon at the top
    /// and `0x18c` with a greataxe, and they looked like a status and a poise
    /// figure respectively — carried by a minority, round numbers, the right
    /// size. They are FIRE and HOLY damage, already read a hundred lines up.
    /// The 82 on Reduvia that made `0x0cc` look right is the fire figure this
    /// project pins by rule, which is the rule doing exactly its job.
    ///
    /// And Reduvia is the wrong reference for bleed here anyway: this
    /// installation is a total conversion that made it a fire and faith weapon
    /// with no physical damage at all. A dagger famous for bleeding in the base
    /// game does not have to bleed in this one.
    ///
    /// `cargo test --lib show_status_buildup -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_status_buildup() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamWeapon") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "weapon")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        // Base rows only — an upgraded copy is the same weapon and would count
        // its buildup five times over.
        let rows: Vec<(i64, &str)> = table
            .ids()
            .filter(|id| id % 100 == 0)
            .filter_map(|id| Some((id, named.get(&u32::try_from(id).ok()?)?.as_str())))
            .collect();
        println!("\n  {} base weapons with names", rows.len());

        // Everything this file already reads out of a weapon row. Without
        // this the sweep hands back the damage fields and they look exactly
        // like a status: carried by a minority, a round number, tens to low
        // hundreds. It offered 0x0cc with a bleed-sounding weapon at the top
        // and 0x18c with a greataxe, and those are FIRE and HOLY damage —
        // the 82 on Reduvia is the fire figure this project pins by rule.
        let known: [usize; 12] = [
            weapon::PHYSICAL,
            weapon::MAGIC,
            weapon::FIRE,
            weapon::LIGHTNING,
            weapon::HOLY,
            weapon::STAMINA,
            weapon::WEIGHT,
            shield::PHYSICAL,
            shield::MAGIC,
            shield::FIRE,
            shield::LIGHTNING,
            shield::HOLY,
        ];
        // Two at a time, not four. Stepping by four checks only half the
        // u16 slots in the row and skips every field on an odd pair —
        // `weapon::LIGHTNING` sits at 0x0ce and would never be looked at. The
        // first sweep found nothing once the known fields were excluded, and
        // this is why.
        for at in (0..664).step_by(2) {
            if known.contains(&at) {
                continue;
            }
            let values: Vec<(u16, &str)> = rows
                .iter()
                .filter_map(|(id, name)| Some((table.u16(*id, at)?, *name)))
                .collect();
            if values.len() < rows.len() / 2 {
                continue;
            }
            let carrying = values.iter().filter(|(v, _)| *v > 0).count();
            // A status is on a minority and never on nobody, and the numbers
            // are the tens and hundreds a buildup bar is measured in.
            let share = carrying * 100 / values.len().max(1);
            if !(2..=40).contains(&share) {
                continue;
            }
            let biggest = values.iter().map(|(v, _)| *v).max().unwrap_or(0);
            if !(30..=200).contains(&biggest) {
                continue;
            }
            let mut top: Vec<(u16, &str)> = values.into_iter().filter(|(v, _)| *v > 0).collect();
            top.sort_by_key(|(value, _)| std::cmp::Reverse(*value));
            top.dedup_by(|a, b| a.1 == b.1);
            println!("\n  0x{at:03x}: {carrying} weapons carry it ({share}%), highest {biggest}");
            for (value, name) in top.iter().take(5) {
                println!("      {value:4}  {name}");
            }
        }

        // Names could not separate a status from a poise figure — the top of
        // 0x0cc was a bleed-sounding weapon and then four greataxes, three
        // points apart. A dagger against a greataxe can: bleed is carried by
        // light quick weapons and poise damage by heavy slow ones, so the two
        // readings point opposite ways.
        //
        // Reduvia is row 1040000, pinned by this project's own rule, and is a
        // bleed dagger. Every u16 it carries, beside the same field on the
        // heaviest thing in the game that has no reputation for bleeding.
        let heavy = rows
            .iter()
            .find(|(_, name)| name.to_lowercase().contains("грейтакс"))
            .or_else(|| rows.iter().find(|(id, _)| *id > 3_000_000));
        // Every place Reduvia carries the number its own menu prints for
        // bleed. Its "Пассивные эффекты" line reads "Накапливает кровотечение
        // (82)", and its BASE fire is also 82 — the upgraded 106 on screen is
        // that 82 with the upgrade applied — so the two collide on this one
        // weapon and the offset has to be told apart another way. 0x0cc is
        // fire: the weapons at the top of it are named for lava, flame and a
        // fire knight.
        // A row that carries the field before blood and not blood itself, so
        // SmithBox can be asked what that field is called. The four are said
        // to be poison, rot, blood and curse in that order; said is not shown.
        if let Some(effects) = regulation.table("SpEffectParam") {
            for (at, guess) in [(0x0cc_usize, "poison?"), (0x0d0, "rot?"), (0x0d8, "curse?")] {
                let example = effects.ids().find(|row| {
                    effects.i32(*row, at).is_some_and(|v| v > 0)
                        && effects.i32(*row, buildup::BLOOD) == Some(0)
                });
                match example {
                    Some(row) => println!(
                        "  0x{at:03x} ({guess}): row {row} has {}, and no blood — ask SmithBox \
                         what that field is called",
                        effects.i32(row, at).unwrap_or(0)
                    ),
                    None => println!("  0x{at:03x} ({guess}): nothing carries it alone"),
                }
            }
        }

        // Base against upgraded. The answer for the live weapon came back 55
        // where the menu says 82, so one of the two rows disagrees with the
        // screen and it matters which.
        // The three still unnamed. A buildup field is zero for nearly every
        // effect in the game and a round number for a few, and the four known
        // ones share that shape exactly — so anything else with it is a
        // candidate, and one row carrying several settles several at once.
        if let Some(effects) = regulation.table("SpEffectParam") {
            let rows: Vec<i64> = effects.ids().collect();
            let known: Vec<usize> = buildup::AILMENTS.iter().map(|(_, at)| *at).collect();
            println!("\n  --- fields shaped like a buildup, beyond the four ---");
            let mut candidates: Vec<usize> = Vec::new();
            for at in (0..1024).step_by(4) {
                if known.contains(&at) {
                    continue;
                }
                let values: Vec<i32> = rows.iter().filter_map(|row| effects.i32(*row, at)).collect();
                if values.len() < rows.len() / 2 {
                    continue;
                }
                let carrying: Vec<i32> = values.iter().copied().filter(|v| *v > 0).collect();
                let share = carrying.len() * 1000 / values.len().max(1);
                // A handful in a thousand, and the sizes a buildup bar uses.
                if !(1..=60).contains(&share) {
                    continue;
                }
                let biggest = carrying.iter().copied().max().unwrap_or(0);
                if !(30..=900).contains(&biggest) {
                    continue;
                }
                candidates.push(at);
                println!("    0x{at:03x}: {} of {} rows, largest {biggest}", carrying.len(), values.len());
            }
            // Narrowed by what SURROUNDS them, which the field list gives for
            // free. In the declared order `freezeAttackPower` follows seven
            // `vfxId` slots, and row 6410 has every one of those at -1 — so
            // the freeze field is the first s32 after a run of seven -1s.
            // `sleepAttackPower` and `madnessAttackPower` are a pair, and the
            // float before them reads 1.0.
            let sevens: Vec<usize> = (0..1024)
                .step_by(4)
                .filter(|at| {
                    *at >= 28
                        && (1..=7).all(|back| effects.i32(6410, at - back * 4) == Some(-1))
                })
                .collect();
            println!("\n  --- first s32 after seven -1s in row 6410 ---");
            for at in &sevens {
                println!(
                    "    0x{at:03x}  candidate: {}",
                    if candidates.contains(at) { "yes, and it is shaped like a buildup" } else { "no" }
                );
            }

            // A row carrying more than one of them is worth one lookup.
            if candidates.len() > 1 {
                let together = rows.iter().find(|row| {
                    candidates.iter().filter(|at| effects.i32(**row, **at).is_some_and(|v| v > 0)).count() > 1
                });
                match together {
                    Some(row) => println!("\n    row {row} carries several — ask SmithBox for it"),
                    None => println!("\n    no row carries two of them; one lookup each"),
                }
            }
        }

        println!("\n  --- Reduvia by upgrade level ---");
        for level in 0..=10 {
            let id = 1_040_000 + level;
            if !table.has(id) {
                continue;
            }
            let effects: Vec<i32> = buildup::EFFECTS
                .iter()
                .filter_map(|at| table.i32(id, *at))
                .filter(|e| *e > 0)
                .collect();
            println!(
                "      +{level} (row {id}): effects {effects:?} -> {:?}",
                regulation.ailments(id)
            );
        }

        println!("\n  --- every 82 in Reduvia's row ---");
        for at in (0..664).step_by(2) {
            if table.u16(1_040_000, at) == Some(82) {
                let what = if at == weapon::FIRE { "  <- weapon::FIRE" } else { "" };
                println!("      0x{at:03x}{what}");
            }
        }

        // The link out. SmithBox, reading the same param with the field names
        // the game ships, gives Reduvia `spEffectBehaviorId0 = 6410` and that
        // row's `bloodAttackPower` — which is what the menu's "Накапливает
        // кровотечение" comes from. So the two offsets wanted are: whichever
        // i32 in the weapon row holds the effect id, and whichever i32 in the
        // effect row holds the buildup.
        // Looking for 6410 in this installation's Reduvia finds nothing — the
        // conversion rewrote the weapon and hung its own effect on it. So the
        // field is found by how it BEHAVES instead: an effect id is either -1
        // or a row that exists, on every weapon in the game, and almost no
        // other field in this row can say that of itself.
        if let Some(effects) = regulation.table("SpEffectParam") {
            println!("\n  --- fields that only ever hold -1 or a real effect id ---");
            for at in (0..664).step_by(4) {
                let seen: Vec<i32> =
                    rows.iter().filter_map(|(id, _)| table.i32(*id, at)).collect();
                if seen.len() < rows.len() / 2 {
                    continue;
                }
                let good = seen.iter().filter(|v| **v == -1 || effects.has(i64::from(**v))).count();
                if good != seen.len() {
                    continue;
                }
                // A field that is -1 everywhere says nothing; one that points
                // somewhere is the candidate.
                let pointing = seen.iter().filter(|v| **v != -1).count();
                if pointing == 0 {
                    continue;
                }
                // And what the bleed reads through it, for the weapon whose
                // menu says 82.
                let mine = table.i32(1_040_000, at).unwrap_or(-1);
                let bleeds = (mine != -1)
                    .then(|| effects.i32(i64::from(mine), 0x0d4))
                    .flatten()
                    .unwrap_or(0);
                println!(
                    "      0x{at:03x}  {pointing} weapons point somewhere; \
                     Reduvia -> {mine}, bleed {bleeds}"
                );
            }
        }
        if let Some(effects) = regulation.table("SpEffectParam") {
            println!("\n  SpEffectParam has {} rows", effects.ids().count());
            // 82 is what this installation's menu prints for Reduvia; vanilla
            // is 50, so the mod raised it and the mod's own row is the one
            // this launcher reads.
            for wanted in [82_i32, 50] {
                let hits: Vec<String> = (0..1024)
                    .step_by(4)
                    .filter(|at| effects.i32(6410, *at) == Some(wanted))
                    .map(|at| format!("0x{at:03x}"))
                    .collect();
                println!("      {wanted} in row 6410 at: {}", hits.join(" "));
            }
        } else {
            println!("\n  SpEffectParam is not in this regulation");
        }

        if let Some((heavy_id, heavy_name)) = heavy {
            println!("\n  --- Reduvia (1040000) against {heavy_name} ---");
            println!("      offset   Reduvia   {heavy_name}");
            for at in (0..664).step_by(2) {
                let (Some(mine), Some(theirs)) =
                    (table.u16(1_040_000, at), table.u16(*heavy_id, at))
                else {
                    continue;
                };
                // Only where one of them has something and the numbers are the
                // size a buildup bar is measured in.
                if (mine == 0 && theirs == 0) || mine > 400 || theirs > 400 {
                    continue;
                }
                println!("      0x{at:03x}    {mine:6}   {theirs:6}");
            }
        }
    }

    /// How much a shield blocks, which is the one figure a shield is chosen on.
    ///
    /// Asked whether the installation has a shield with 100% physical block, a
    /// model answered "no" — out of the armour ranking, which contains no
    /// shield at all. The prohibition against that is in place; this is the
    /// other half, which is being able to answer the question instead.
    ///
    /// Shields live in `EquipParamWeapon`, so the rate is somewhere in a 664
    /// byte row. Found by shape and proved by names rather than counted down a
    /// field list: the right field puts shields near 100 and everything held in
    /// two hands well below it, and the greatshields come out at exactly 100 if
    /// any do.
    ///
    /// `cargo test --lib show_guard_rates -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_guard_rates() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamWeapon") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "weapon")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        // A shield calls itself one, in whatever language the game is in.
        let is_shield = |name: &str| {
            let said = name.to_lowercase();
            ["shield", "щит", "buckler", "баклер"].iter().any(|word| said.contains(word))
        };
        let rows: Vec<(i64, &str)> = table
            .ids()
            .filter_map(|id| Some((id, named.get(&u32::try_from(id).ok()?)?.as_str())))
            .collect();
        let shields: Vec<(i64, &str)> =
            rows.iter().filter(|(_, name)| is_shield(name)).copied().collect();
        let others: Vec<(i64, &str)> =
            rows.iter().filter(|(_, name)| !is_shield(name)).copied().collect();
        println!("\n  {} named weapons, {} of them shields", rows.len(), shields.len());
        if shields.len() < 5 || others.len() < 20 {
            println!("  not enough to tell them apart");
            return;
        }

        // A guard rate is a percentage: mostly between 20 and 100, with the
        // shields at the top of it. Anything that does not separate the two
        // groups is not it.
        let mean = |of: &[(i64, &str)], at: usize| -> Option<f32> {
            let seen: Vec<f32> = of.iter().filter_map(|(id, _)| table.f32(*id, at)).collect();
            (seen.len() > of.len() / 2).then(|| seen.iter().sum::<f32>() / seen.len() as f32)
        };
        for at in (0..664).step_by(4) {
            let (Some(shield), Some(other)) = (mean(&shields, at), mean(&others, at)) else {
                continue;
            };
            if !(20.0..=100.0).contains(&shield) || shield < other + 15.0 {
                continue;
            }
            let hundreds = shields
                .iter()
                .filter(|(id, _)| table.f32(*id, at) == Some(100.0))
                .count();
            println!("\n  0x{at:03x}: shields mean {shield:.1}, everything else {other:.1}, \
                      {hundreds} shields at exactly 100");
            let mut best: Vec<(f32, &str)> = shields
                .iter()
                .filter_map(|(id, name)| Some((table.f32(*id, at)?, *name)))
                .collect();
            best.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (value, name) in best.iter().take(4) {
                println!("      {value:6.1}  {name}");
            }
            for (id, name) in others.iter().take(2) {
                if let Some(value) = table.f32(*id, at) {
                    println!("      {value:6.1}  {name}  (not a shield)");
                }
            }
        }

        // One shield, every float in the row, against the five numbers its own
        // menu prints. This is the second reader the offsets needed: names
        // could not separate fire from lightning because the shield that tops
        // both is the one that is best at everything.
        //
        // Read off the screen for the Exile Knight Shield, "Сопротивление в
        // блоке": physical 100.0, magic 49.0, fire 57.0, lightning 31.0, holy
        // 48.0, and guard boost 55.
        let wanted: [(&str, f32); 6] = [
            ("physical", 100.0),
            ("magic", 49.0),
            ("fire", 57.0),
            ("lightning", 31.0),
            ("holy", 48.0),
            ("guard boost", 55.0),
        ];
        if let Some((id, name)) = shields
            .iter()
            .find(|(_, name)| name.to_lowercase().contains("изгнанник"))
        {
            println!("\n  --- {name}, every float that matches its menu ---");
            for (what, screen) in wanted {
                let hits: Vec<String> = (0..664)
                    .step_by(4)
                    .filter(|at| table.f32(*id, *at).is_some_and(|v| (v - screen).abs() < 0.01))
                    .map(|at| format!("0x{at:03x}"))
                    .collect();
                println!("      {what:<12} {screen:6.1}  at {}", hits.join(" "));
            }
        }

        // The two after magic, by the names at the top of each. Whether one is
        // fire and the other lightning is not something to assume from the
        // order they happen to sit in — a shield that says what it is for in
        // its own name is the proof, the same standard the slot byte was held
        // to. Printed even though the sweep above filters them out: they do
        // not separate shields from everything else nearly as sharply, which
        // is itself a fact about them.
        for at in [0x03c_usize, 0x040] {
            let mut best: Vec<(f32, &str)> = shields
                .iter()
                .filter_map(|(id, name)| Some((table.f32(*id, at)?, *name)))
                .collect();
            best.sort_by(|a, b| b.0.total_cmp(&a.0));
            best.dedup_by(|a, b| a.1 == b.1);
            println!("\n  --- 0x{at:03x}, the shields that top it ---");
            for (value, name) in best.iter().take(8) {
                println!("      {value:6.1}  {name}");
            }
        }

        // The run around the physical rate, for one shield that is not the
        // same in every kind. A row of five is what the menu shows; reading
        // one of them and guessing the order of the rest is how an offset
        // gets quoted wrong.
        let mirror = shields
            .iter()
            .find(|(_, name)| name.to_lowercase().contains("зеркал"))
            .or_else(|| shields.first());
        if let Some((id, name)) = mirror {
            println!("\n  --- {name} across 0x030..0x050 ---");
            for at in (0x030..0x050).step_by(4) {
                if let Some(value) = table.f32(*id, at) {
                    println!("      0x{at:03x}  {value:8.2}");
                }
            }
        }
    }

    /// Which byte says head, body, arms or legs.
    ///
    /// Counting down the field list is what produced two wrong offsets before,
    /// so this looks for the shape and then proves it with the game's own
    /// words: the right byte splits the table into a handful of values, and the
    /// pieces under each share a kind of name. A helmet is called a helmet.
    ///
    /// `cargo test --lib show_armour_slots -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_armour_slots() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamProtector") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "armour")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        // Only the pieces somebody can wear; the placeholders have no weight
        // and would drown the counts.
        let real: Vec<i64> = table
            .ids()
            .filter(|id| regulation.armour(*id).is_some_and(|piece| piece.weight > 0.0))
            .collect();
        println!("  {} wearable pieces", real.len());

        for at in 0..416usize {
            let values: std::collections::BTreeMap<u8, usize> =
                real.iter().filter_map(|id| table.u8(*id, at)).fold(
                    std::collections::BTreeMap::new(),
                    |mut seen, value| {
                        *seen.entry(value).or_default() += 1;
                        seen
                    },
                );
            // Four slots, so four values sharing the table roughly evenly. A
            // flag byte has two, an id byte has hundreds.
            if !(3..=5).contains(&values.len()) {
                continue;
            }
            let fewest = values.values().copied().min().unwrap_or(0);
            if fewest * 8 < real.len() {
                continue;
            }
            println!("\n  0x{at:03x}: {values:?}");
            for value in values.keys() {
                let sample: Vec<&str> = real
                    .iter()
                    .filter(|id| table.u8(**id, at) == Some(*value))
                    .filter_map(|id| named.get(&u32::try_from(*id).ok()?))
                    .map(String::as_str)
                    .take(3)
                    .collect();
                println!("    {value} -> {sample:?}");
            }
        }
    }

    /// Ranking every piece against one kind, for reading.
    ///
    /// `cargo test --lib show_best_armour -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_best_armour() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamProtector") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "armour")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        for kind in ["lightning", "fire", "physical"] {
            let mut ranked: Vec<(String, f32, f32)> = table
                .ids()
                .filter_map(|id| {
                    let piece = regulation.armour(id)?;
                    if piece.weight <= 0.0 {
                        return None;
                    }
                    let name = named.get(&u32::try_from(id).ok()?)?;
                    let stopped =
                        piece.negation.iter().find(|(what, _)| what == kind).map(|(_, v)| *v)?;
                    Some((name.clone(), stopped, piece.weight))
                })
                .collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.2.total_cmp(&b.2)));
            ranked.dedup_by(|a, b| a.0 == b.0);
            println!("\n  best against {kind}, of {}:", ranked.len());
            for (name, stopped, weight) in ranked.iter().take(5) {
                println!("    {name:<44} {stopped:>5.1}%  {weight:>5.1}");
            }
        }
    }

    /// A title whose layout was never checked reads nothing rather than
    /// something.
    ///
    /// Three of the games this launcher manages ship a `regulation.bin` under
    /// exactly the same name, and nothing about the file says which game wrote
    /// it. The reader would happily parse one and report a weight off whatever
    /// four bytes sit at 0x010 — a number that looks like an answer and is not.
    /// Neither of those games is installed here, so whether they match could
    /// not be established, and the honest position is to refuse.
    #[test]
    fn only_the_game_whose_rows_were_checked_is_read() {
        use crate::games::Game;

        assert!(laid_out_like_this(Game::EldenRing));
        for other in Game::ALL.into_iter().filter(|game| *game != Game::EldenRing) {
            assert!(
                !laid_out_like_this(other),
                "{} is read with ELDEN RING's offsets and nobody has checked it",
                other.display_name()
            );
        }

        // And the refusal is at the door, not left to each caller to remember.
        let Some(game_dir) = crate::testing::game_dir(Game::EldenRing) else {
            return;
        };
        assert!(
            installed(Game::EldenRing, &game_dir, None).is_some(),
            "the game that was checked stopped reading"
        );
        // The same real regulation, asked for as another title. It parses fine;
        // that is the point — the file is not what makes it wrong.
        assert!(
            installed(Game::Nightreign, &game_dir, None).is_none(),
            "another title's numbers came out of ELDEN RING's tables"
        );
    }

    /// Where the eight damage-cut rates sit in an `NpcParam` row.
    ///
    /// Counting down a field list of three hundred entries is what produced two
    /// wrong offsets already, so this looks for the shape instead: eight
    /// consecutive floats that are all a plausible multiplier, that are not all
    /// the same across the table, and that sit near 1.0 on average — a rate of
    /// 1.0 changes nothing and most things resist nothing.
    ///
    /// Which weapon is which sort of weapon, and what the game calls each sort.
    ///
    /// The gap this fills: asked for the best GREATSHIELD against magic, the
    /// launcher ranked helmets, because there was no way to ask for a class of
    /// weapon at all — only a name. An English player asking about "shields" on
    /// a Russian installation got nothing for the same reason, since the search
    /// matches names and no name contains the English word.
    ///
    /// `wepType` is the discriminator. Its offset is 0x1a6, taken from a field
    /// walk of the paramdef whose drift was measured against twenty offsets
    /// already established here — every one of them exactly one byte, and the
    /// walked row size 665 against the true 664, so the walk is uniformly long
    /// by one and nothing else. Reduvia reads 1, which is the dagger.
    ///
    /// `cargo test --lib show_weapon_sorts -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_weapon_sorts() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamWeapon") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "weapon")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        const WEP_TYPE: usize = 0x1a6;
        println!("
  Reduvia's wepType: {:?}", table.u16(1_040_000, WEP_TYPE));

        let mut sorts: std::collections::BTreeMap<u16, Vec<String>> = Default::default();
        for id in table.ids() {
            if id % 100 != 0 {
                continue;
            }
            let Some(sort) = table.u16(id, WEP_TYPE) else {
                continue;
            };
            let Some(name) = u32::try_from(id).ok().and_then(|id| named.get(&id)) else {
                continue;
            };
            sorts.entry(sort).or_default().push(name.clone());
        }
        let tables = crate::library::tables_for(&game_dir, mod_dir.as_deref(), language);
        let menu = tables.get("GR_MenuText").expect("the menu has words");
        println!(
            "  {} sorts over {} base rows
",
            sorts.len(),
            sorts.values().map(Vec::len).sum::<usize>()
        );
        println!("  type    n  the game's word        english             three of them");
        let mut unpaired = Vec::new();
        for (kind, mut names) in sorts {
            names.sort();
            let show: Vec<&str> = names.iter().take(3).map(String::as_str).collect();
            let english = sort::english(kind).unwrap_or("—");
            let theirs = sort::menu_id(kind)
                .and_then(|id| menu.get(&id))
                .map_or("—".to_string(), |said| said.trim().to_string());
            if sort::english(kind).is_none() {
                unpaired.push(kind);
            }
            println!(
                "  {kind:>4} {:>4}  {theirs:<22} {english:<19} {}",
                names.len(),
                show.join(" · ")
            );
        }
        assert!(unpaired.is_empty(), "sorts with no name: {unpaired:?}");

        // Guard boost, by shape: if 0x0d8 is the figure the menu calls that,
        // greatshields own the top of it and nothing else comes close.
        const GUARD_BOOST: usize = 0x0d8;
        let mut boost: Vec<(i16, u16, String)> = table
            .ids()
            .filter(|id| id % 100 == 0)
            .filter_map(|id| {
                let name = u32::try_from(id).ok().and_then(|id| named.get(&id))?;
                Some((table.i16(id, GUARD_BOOST)?, table.u16(id, sort::AT)?, name.clone()))
            })
            .collect();
        boost.sort_by_key(|(value, _, _)| std::cmp::Reverse(*value));
        println!("
  most guard boost at 0x{GUARD_BOOST:03x}:");
        for (value, kind, name) in boost.iter().take(8) {
            println!("      {value:>4}  {:<18} {name}", sort::english(*kind).unwrap_or("—"));
        }
    }

    /// What a talisman actually DOES, in numbers.
    ///
    /// The gap: asked what Radagon's Soreseal gives and what it costs, the
    /// launcher could return its name, its weight and the sentence under it —
    /// "raises vigour, endurance, strength and dexterity, and raises damage
    /// taken" — and not one figure. Which four, by how much, and how much more
    /// damage, are the whole question, and they are all sitting in the tables.
    ///
    /// `EquipParamAccessory` holds the talismans and points at `SpEffectParam`
    /// through `refId`; the effect row carries the arithmetic. Offsets here
    /// come from walking the paramdefs with the padding rule that makes the
    /// walk exact — see `scratchpad/paramwalk.py` — and the walk is checked
    /// against a weight the launcher already reads another way, so a wrong
    /// offset shows up as a wrong weight rather than silently.
    ///
    /// Do the spirit ashes actually name an upgrade material?
    ///
    /// Run with `--ignored --nocapture`. The id was being read and dropped, and
    /// before surfacing it the question is whether it points anywhere: an id
    /// that resolves to nothing would put "using " with a blank after it into
    /// every line, which is worse than the silence it replaces.
    ///
    /// `cargo test --lib show_what_an_ash_upgrades_with -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_what_an_ash_upgrades_with() {
        let game = crate::games::Game::EldenRing;
        let Some(path) = crate::testing::regulation(game) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&path) else {
            return;
        };
        let spirits = regulation.spirits();
        let with = spirits.iter().filter(|one| one.material.is_some()).count();
        println!("  {} spirit ashes, {with} naming an upgrade material", spirits.len());

        // And whether the id RESOLVES TO A NAME, which is the half that
        // matters. Reading the id is worthless if the catalogue has no entry
        // for it: the tool prints nothing, the model has no material, and it
        // fills the gap — asked what upgrading a spirit ash needs, one answered
        // "Корневой прах (Root Resin)", which is a DARK SOULS II item.
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        // GOODS ONLY. Ids repeat across the six tables the catalogue merges, so
        // looking one up without saying which table it belongs to returns
        // whatever was read last: id 10000 came back as an ash of WAR out of
        // the gem table when it should be an upgrade material out of goods.
        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, crate::testing::mod_dir(game).as_deref(), language)
                .iter()
                .filter(|one| one.what == "item")
                .map(|one| (one.id, one.name.clone()))
                .collect();
        let mut resolved = 0;
        for one in spirits.iter().filter(|one| one.material.is_some()).take(6) {
            // Through the material SET, which is what the field really names.
            let parts: Vec<String> = regulation
                .ingredients(i64::from(one.material.unwrap_or(0)))
                .into_iter()
                .filter_map(|(item, count)| {
                    let called = named.get(&u32::try_from(item).ok()?)?;
                    Some(format!("{called} x{count}"))
                })
                .collect();
            resolved += usize::from(!parts.is_empty());
            println!("      {}: set {:?} -> {:?}", one.id, one.material, parts);
        }
        let all = spirits
            .iter()
            .filter_map(|one| one.material)
            .filter_map(|id| u32::try_from(id).ok())
            .filter(|id| named.contains_key(id))
            .count();
        println!("\n  {resolved} of the first six resolve to a name; {all} of {with} overall");
    }

    /// What a whole weapon class really reads, damage by damage.
    ///
    /// Run with `--ignored --nocapture`. Written because an answer said "в этой
    /// сборке большие молоты переделаны в щиты — урон не прописан, только
    /// блок", which would be a serious data fault if true and an invention if
    /// not. Neither should be taken on trust.
    ///
    /// `cargo test --lib show_what_a_class_reads -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_what_a_class_reads() {
        let game = crate::games::Game::EldenRing;
        let Some(path) = crate::testing::regulation(game) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&path) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamWeapon") else {
            return;
        };
        // 23 is `great hammer`, the class the answer was about.
        let mut seen = 0;
        let mut with_damage = 0;
        for id in table.ids().filter(|id| id % 100 == 0) {
            let Some(kind) = table.u16(id, sort::AT) else {
                continue;
            };
            if kind != 23 {
                continue;
            }
            let Some(weapon) = regulation.weapon(id) else {
                continue;
            };
            let total: u16 = weapon.damage.iter().map(|(_, value)| *value).sum();
            seen += 1;
            with_damage += usize::from(total > 0);
            if seen <= 6 {
                println!("  {id}: damage {:?} · blocks {:?}", weapon.damage, weapon.blocks);
            }
        }
        println!("\n  {seen} great hammers, {with_damage} of them with damage above zero");
    }

    /// Which talismans raise an ailment YOU inflict, if any do.
    ///
    /// Written because the honest answer to "best talisman for bleed damage"
    /// depends on a fact nobody had checked. It has now been run and the answer
    /// is NONE: not one talisman in this installation carries an outgoing
    /// figure. So the launcher's job there is to say there is no such talisman
    /// rather than to offer bleed RESISTANCE, which is the opposite thing and
    /// is what it used to reach for.
    ///
    /// `cargo test --lib show_which_talismans_inflict -- --ignored --nocapture`
    #[test]
    #[ignore = "a survey, not an assertion"]
    fn show_which_talismans_inflict() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamAccessory") else {
            return;
        };
        let mut found = 0;
        for id in table.ids() {
            let Some(charm) = regulation.charm(id) else {
                continue;
            };
            let inflicts: Vec<&(String, i32)> =
                charm.adds.iter().filter(|(what, _)| what.ends_with("inflicted")).collect();
            if !inflicts.is_empty() {
                found += 1;
                println!("  {id}: {inflicts:?}");
            }
        }
        println!("\n  {found} talismans carry an outgoing ailment figure");
    }

    /// What a talisman actually DOES, in numbers.
    ///
    /// The gap this was written for: asked what Radagon's Soreseal gives and
    /// what it costs, the launcher could return its name, its weight and the
    /// sentence under it — "raises vigour, endurance, strength and dexterity,
    /// and raises damage taken" — and not one figure. Which four, by how much,
    /// and how much more damage, are the whole question.
    ///
    /// `cargo test --lib show_talisman_effects -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_talisman_effects() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(charms) = regulation.table("EquipParamAccessory") else {
            return;
        };
        let Some(effects) = regulation.table("SpEffectParam") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "talisman")
                .map(|one| (one.id, one.name.clone()))
                .collect();
        println!(
            "
  {} talisman rows, {} named, {} effect rows
",
            charms.ids().count(),
            named.len(),
            effects.ids().count()
        );

        // Through the shipped reader, not a copy of its offsets. A probe that
        // keeps its own constants stops testing the thing that ships the moment
        // one of them is corrected.
        let mut shown = 0;
        let mut silent = 0;
        for (id, name) in &named {
            let Some(figures) = regulation.charm(i64::from(*id)) else { continue };
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
            if said.is_empty() {
                silent += 1;
                continue;
            }
            shown += 1;
            if shown <= 20 {
                println!("  {name}  ({:.1} kg)
      {}", figures.weight, said.join(", "));
            }
        }
        println!("
  {shown} talismans read something, {silent} read nothing at all");

        // For the silent ones: which SpEffect slots are they actually using?
        // Tally every four-byte slot that is neither 0 nor 1.0 across all of
        // them, so the fields worth adding come out ranked by how many
        // talismans need them, rather than picked by guesswork.
        let mine: std::collections::HashSet<usize> = charm::RATES
            .iter()
            .map(|(_, at, _)| *at)
            .chain(charm::POINTS.iter().map(|(_, at)| *at))
            .collect();
        let mut wanted: std::collections::BTreeMap<usize, usize> = Default::default();
        for (id, _) in &named {
            let Some(figures) = regulation.charm(i64::from(*id)) else { continue };
            if !figures.gives.is_empty() || !figures.adds.is_empty() || !figures.changes.is_empty()
            {
                continue;
            }
            for at in (0..912usize - 4).step_by(4) {
                if mine.contains(&at) {
                    continue;
                }
                let whole = effects.i32(figures.effect, at).unwrap_or(0);
                let real = effects.f32(figures.effect, at).unwrap_or(0.0);
                let idle = whole == 0
                    || whole == -1
                    || (real - 1.0).abs() < 0.0005
                    || !real.is_finite()
                    || real.abs() > 1e9
                    || (real != 0.0 && real.abs() < 1e-6);
                if !idle {
                    *wanted.entry(at).or_default() += 1;
                }
            }
        }
        let mut ranked: Vec<(usize, usize)> = wanted.into_iter().collect();
        ranked.sort_by_key(|(_, how_many)| std::cmp::Reverse(*how_many));
        println!("
  slots the silent ones use, most-wanted first:");
        for (at, how_many) in ranked.iter().take(18) {
            println!("      0x{at:03x}  {how_many} talismans");
        }
    }

    /// Does the reinforce ladder agree with the MATERIALS ladder?
    ///
    /// `upgrade_ceilings` walks ReinforceParamWeapon and says +15. The mod's
    /// own wiki says +10. One of them is wrong and it matters, because the
    /// figure went into the always-present block as a fact.
    ///
    /// The discriminator: a level you cannot BUY is not a level. `upgrades`
    /// requires a material set for each step. If the reinforce rows run to 15
    /// but the materials stop at 10, the cap is 10 and the walk is counting
    /// rows the game never offers.
    ///
    /// `cargo test --lib show_upgrade_reach -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_upgrade_reach() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(weapons) = regulation.table("EquipParamWeapon") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "weapon")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        // Three things per weapon: how far the reinforce rows go, how far the
        // MATERIALS go, and how many +N copies of the weapon itself exist.
        let mut agree: std::collections::BTreeMap<(u8, u8, u8), usize> = Default::default();
        let mut examples: std::collections::BTreeMap<(u8, u8, u8), String> = Default::default();
        for id in weapons.ids().filter(|id| id % 100 == 0) {
            let Some(name) = u32::try_from(id).ok().and_then(|id| named.get(&id)) else {
                continue;
            };
            let Some(kind) = weapons.u16(id, weapon::REINFORCE_TYPE) else { continue };
            let levels = regulation.table("ReinforceParamWeapon");
            let rows = levels.map_or(0, |levels| {
                let mut highest = 0;
                for level in 1..=25u8 {
                    if !levels.has(i64::from(kind) + i64::from(level)) {
                        break;
                    }
                    highest = level;
                }
                highest
            });
            let materials =
                regulation.upgrade_steps(id).iter().map(|step| step.level).max().unwrap_or(0);
            let mut copies = 0u8;
            for level in 1..=25u8 {
                if weapons.has(id + i64::from(level)) {
                    copies = level;
                } else {
                    break;
                }
            }
            *agree.entry((rows, materials, copies)).or_default() += 1;
            examples.entry((rows, materials, copies)).or_insert_with(|| name.clone());
        }
        println!("
  rows  materials  copies   weapons   e.g.");
        for ((rows, materials, copies), how_many) in &agree {
            println!(
                "  +{rows:<4} +{materials:<9} +{copies:<7} {how_many:<9} {}",
                examples[&(*rows, *materials, *copies)]
            );
        }

        // The discriminator, spelt out: which +N rows actually EXIST for one
        // weapon. A level the game has no row for is not a level, whatever the
        // reinforce table holds.
        println!("
  total rows in EquipParamWeapon: {}", weapons.ids().count());
        for base in [1_040_000i64, 2_000_000, 1_000_000] {
            if !weapons.has(base) {
                println!("  {base}: no such base row");
                continue;
            }
            let present: Vec<u8> =
                (0..=25u8).filter(|level| weapons.has(base + i64::from(*level))).collect();
            let name = u32::try_from(base).ok().and_then(|id| named.get(&id));
            println!(
                "  {base} ({}): +N rows present = {:?}",
                name.map_or("unnamed", String::as_str),
                present
            );
        }
    }

    /// Does any armour here carry an effect, the way a talisman does?
    ///
    /// Asked in French for the lightest armour for a faith build, an answer
    /// said a set gave "+2 a la Foi". The reader has no such figure and never
    /// did, so that was invented. But `EquipParamProtector` has three
    /// `residentSpEffectId` slots at 0x28, 0x2c and 0x30, and a conversion is
    /// exactly the sort of thing that would use them.
    ///
    /// `cargo test --lib show_armour_effects -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_armour_effects() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamProtector") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "armour")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        const RESIDENT: [usize; 3] = [0x28, 0x2c, 0x30];
        let mut carrying = 0;
        let mut shown = 0;
        for id in table.ids() {
            let Some(name) = u32::try_from(id).ok().and_then(|id| named.get(&id)) else {
                continue;
            };
            let effects: Vec<i32> = RESIDENT
                .iter()
                .filter_map(|at| table.i32(id, *at))
                .filter(|effect| *effect > 0)
                .collect();
            if effects.is_empty() {
                continue;
            }
            carrying += 1;
            // Reuse the talisman reader: it takes an accessory id, so read the
            // effect row directly through the same field tables instead.
            let mut said: Vec<String> = Vec::new();
            for effect in &effects {
                let row = i64::from(*effect);
                for (what, at) in charm::ATTRIBUTES {
                    if let Some(byte) = regulation.table("SpEffectParam").and_then(|e| e.u8(row, at))
                    {
                        let value = byte as i8;
                        if value != 0 {
                            said.push(format!("{what} {value:+}"));
                        }
                    }
                }
                for (what, at, idle) in charm::RATES {
                    if let Some(value) =
                        regulation.table("SpEffectParam").and_then(|e| e.f32(row, at))
                    {
                        if (value - idle).abs() > 0.0005 {
                            said.push(format!("{what} {:+.0}%", (value - 1.0) * 100.0));
                        }
                    }
                }
            }
            if said.is_empty() {
                continue;
            }
            shown += 1;
            if shown <= 16 {
                println!("  {name}  (effects {effects:?})
      {}", said.join(", "));
            }
        }
        println!("
  {carrying} pieces carry an effect id, {shown} of them read something");
    }

    /// Are the physick tears goodsType 10, and can their effects be read?
    ///
    /// Asked twice what can go into the wondrous physick, the launcher found
    /// nothing. `goodsType` 10 holds 60 rows headed by the crimson crystal
    /// tear, which is exactly what a tear is.
    ///
    /// `cargo test --lib show_physick_tears -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_physick_tears() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(table) = regulation.table("EquipParamGoods") else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .map(|one| (one.id, one.name.clone()))
                .collect();

        const TEAR: u8 = 10;
        let mut shown = 0;
        let mut silent = 0;
        for id in table.ids() {
            if table.u8(id, spirit::SORT) != Some(TEAR) {
                continue;
            }
            let Some(name) = u32::try_from(id).ok().and_then(|id| named.get(&id)) else {
                continue;
            };
            // Same route as a talisman: the goods row points at an effect.
            let effect = i64::from(table.i32(id, 0x04).unwrap_or(-1));
            let (gives, changes, adds) = regulation.what_an_effect_does(&[effect]);
            let mut said: Vec<String> = gives
                .iter()
                .chain(adds.iter())
                .map(|(what, value)| format!("{what} {value:+}"))
                .collect();
            said.extend(
                changes.iter().map(|(what, rate)| format!("{what} {:+.0}%", (rate - 1.0) * 100.0)),
            );
            if said.is_empty() {
                silent += 1;
                continue;
            }
            shown += 1;
            if shown <= 14 {
                println!("  {name}
      {}", said.join(", "));
            }
        }
        println!("
  {shown} tears read something, {silent} read nothing");
    }

    /// Do the ashes of war read, and how many carry an FP cost?
    ///
    /// `cargo test --lib show_ashes_of_war -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_ashes_of_war() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let tables = crate::library::tables_for(&game_dir, mod_dir.as_deref(), language);
        let arts_names = tables.get("ArtsName");

        let ashes = regulation.ashes_of_war();
        let with = ashes.iter().filter(|(_, skill)| skill.is_some()).count();
        let priced = ashes
            .iter()
            .filter(|(_, skill)| skill.as_ref().is_some_and(|one| !one.costs.is_empty()))
            .count();
        println!(
            "
  {} gem rows, {with} reach a skill, {priced} of those cost FP",
            ashes.len()
        );
        let mut shown = 0;
        for (id, skill) in &ashes {
            let Some(skill) = skill else { continue };
            if skill.costs.is_empty() {
                continue;
            }
            shown += 1;
            if shown > 12 {
                break;
            }
            let called = arts_names
                .and_then(|names| names.get(&skill.text))
                .map_or("(unnamed)".to_string(), |said| said.trim().to_string());
            let priced: Vec<String> =
                skill.costs.iter().map(|(button, fp)| format!("{button} {fp} FP")).collect();
            println!("  {id:>7}  {called:<34} {}", priced.join(", "));
        }
    }

    /// `cargo test --lib show_cut_rates -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, not a check"]
    fn show_cut_rates() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let Some(npcs) = regulation.table("NpcParam") else {
            return;
        };

        // A fingerprint, out of a second reader of the same file: row 34702468
        // is neutral 1.1, slash 1.1, blow 0.9, thrust 1.1, magic 1.0, fire 0.6,
        // lightning 1.2, holy 1.4. Eight floats in that order appear once.
        const WANT: [f32; 8] = [1.1, 1.1, 0.9, 1.1, 1.0, 0.6, 1.2, 1.4];
        let known = 34_702_468i64;
        if npcs.has(known) {
            println!("  row {known}: hp {:?}, runes {:?}", npcs.i32(known, 0x24), npcs.i32(known, 0x2c));
            for at in (0..736 - 32).step_by(4) {
                let got: Vec<f32> =
                    (0..8).filter_map(|n| npcs.f32(known, at + n * 4)).collect();
                if got.len() == 8
                    && got.iter().zip(WANT).all(|(had, want)| (had - want).abs() < 0.001)
                {
                    println!("  the eight cut rates start at 0x{at:03x}");
                }
            }
        }

        // Are they their own, or a shared template like the health? Every
        // human character is built on one model and their rows carry the same
        // hp, which had to stop being reported. If the rates are the same too,
        // they cannot be reported either.
        let mut how_many: std::collections::BTreeMap<String, usize> = Default::default();
        let mut differing = 0usize;
        for id in npcs.ids() {
            let taken = regulation.damage_taken_by(id);
            if taken.is_empty() {
                continue;
            }
            differing += 1;
            let shape: Vec<String> =
                taken.iter().map(|(kind, pc)| format!("{kind} {pc:.0}")).collect();
            *how_many.entry(shape.join(", ")).or_default() += 1;
        }
        println!("\n  {differing} rows take something differently, in {} patterns", how_many.len());
        let mut common: Vec<(&String, &usize)> = how_many.iter().collect();
        common.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for (shape, count) in common.iter().take(6) {
            println!("    {count:>4} rows: {shape}");
        }
    }

    /// What hurts a thing, over the whole table.
    ///
    /// The offset was found by fingerprint — a second reader gave one row's
    /// eight rates and exactly one place holds those floats in that order — so
    /// what this checks is everything else: that they are rates rather than
    /// whatever else could sit there, and that they belong to the creature
    /// rather than to a template. The health taught that lesson: every human
    /// character shares one number and it had to stop being reported.
    #[test]
    fn what_hurts_a_thing_is_its_own_and_is_a_rate() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let Some(npcs) = regulation.table("NpcParam") else {
            return;
        };

        // The fingerprint, first, because nothing else catches a four-byte
        // shift: the float before these is a recovery correction and is just as
        // plausible a rate. A second reader gives row 34702468 exactly this.
        let known = 34_702_468i64;
        if npcs.has(known) {
            let want: Vec<(String, f32)> = [
                ("physical", 110.0),
                ("slash", 110.0),
                ("strike", 90.0),
                ("pierce", 110.0),
                ("fire", 60.0),
                ("lightning", 120.0),
                ("holy", 140.0),
            ]
            .into_iter()
            .map(|(kind, pc)| (kind.to_string(), pc))
            .collect();
            let got = regulation.damage_taken_by(known);
            assert_eq!(got.len(), want.len(), "row {known} came back as {got:?}");
            for ((kind, pc), (wanted, should)) in got.iter().zip(&want) {
                assert_eq!(kind, wanted, "row {known} came back as {got:?}");
                assert!((pc - should).abs() < 0.5, "row {known} takes {pc} of {kind}, not {should}");
            }
        }

        let mut differing = 0usize;
        let mut patterns: std::collections::HashSet<String> = Default::default();
        for id in npcs.ids() {
            let taken = regulation.damage_taken_by(id);
            if taken.is_empty() {
                continue;
            }
            differing += 1;
            let mut shape = String::new();
            for (kind, percent) in &taken {
                // A multiplier, not a number of hit points. Anything outside
                // this is the wrong four bytes. Zero is in range and is real:
                // some things take no physical damage at all, which is the
                // sort of thing a player most wants told.
                assert!(
                    (0.0..=500.0).contains(percent),
                    "npc {id} takes {percent} per cent of {kind}"
                );
                // And never the ordinary amount: those are left out on purpose,
                // because a list of eight hundreds hides the two that matter.
                assert!((percent - 100.0).abs() > 0.4, "npc {id} lists {kind} at {percent}");
                shape.push_str(&format!("{kind}{percent:.0} "));
            }
            patterns.insert(shape);
        }

        assert!(differing > 500, "only {differing} things take anything differently");
        // Their own, not a template. Under a hundred patterns across thousands
        // of rows would mean these are shared and unreportable, the way the
        // health is.
        assert!(
            patterns.len() > 100,
            "only {} distinct patterns across {differing} rows, which reads like a template",
            patterns.len()
        );
    }

    /// The weapon-to-skill chain, over every weapon there is.
    ///
    /// The offset was not measured, it was deduced: `NEEDS_ARCANE` is one byte
    /// at 0x195 and `SCALE_ARCANE` is four at 0x19c, so a four-byte field at
    /// 0x198 is the only thing that fits. That is an argument, not a check —
    /// this is the check. Every weapon that names a skill must name one the
    /// skill table really has, and a wrong offset lands in the middle of a
    /// float and hits nothing.
    #[test]
    fn every_weapon_s_skill_is_a_skill_the_game_has() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let (Some(weapons), Some(arts)) =
            (regulation.table("EquipParamWeapon"), regulation.table("SwordArtsParam"))
        else {
            return;
        };
        assert!(arts.len() > 50, "only {} skills", arts.len());

        let mut named = 0usize;
        let mut costed = 0usize;
        for id in weapons.ids() {
            let Some(skill) = regulation.skill_of(id) else {
                continue;
            };
            named += 1;
            assert!(arts.has(skill.id), "weapon {id} names skill {}, which is no row", skill.id);
            for (button, cost) in &skill.costs {
                // The dearest thing in the game is well under a hundred; a
                // misread signed field comes back in the tens of thousands.
                assert!(*cost < 300, "skill {} costs {cost} FP on {button}", skill.id);
                costed += 1;
            }
        }
        // Every weapon has some art, even if it is only the plain one.
        assert!(
            named * 2 > weapons.len(),
            "only {named} of {} weapons resolved a skill",
            weapons.len()
        );
        assert!(costed > 0, "not one skill cost anything, so the cost fields are wrong");

        // And the level suffix must not break it: the ids a player holds carry
        // their upgrade, and a +5 weapon has the same skill as a +0 one.
        let Some(base) = weapons.ids().find(|id| regulation.skill_of(*id).is_some()) else {
            return;
        };
        let held = regulation.skill_of(base + 5).map(|found| found.id);
        assert_eq!(held, regulation.skill_of(base).map(|found| found.id));
    }

    /// Armour, read out of the installed tables.
    ///
    /// Poise is a rate, and body pieces carry the most of it.
    ///
    /// The anchor for this lives in a running game, so what is pinned here is
    /// the shape — which is what told the field apart from a coincidence in the
    /// first place, and which holds for any installation.
    #[test]
    fn poise_follows_the_armour_it_is_worn_on() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let Some(table) = regulation.table("EquipParamProtector") else {
            return;
        };

        let mut carrying: Vec<(f32, f32, i64)> = table
            .ids()
            .filter_map(|id| {
                let piece = regulation.armour(id)?;
                let value = regulation.poise_of(id)?;
                (piece.weight > 0.0).then_some((value, piece.weight, id))
            })
            .collect();
        assert!(carrying.len() > 100, "only {} pieces carry poise", carrying.len());
        carrying.sort_by(|a, b| b.0.total_cmp(&a.0));

        // Numbers of the right size. Poise on one piece is tens, not hundreds
        // and not thousandths — the scale being wrong is the way this breaks.
        for (value, _, id) in &carrying {
            assert!((0.1..=200.0).contains(value), "row {id} has {value} of poise");
        }
        assert!(carrying[0].0 > 30.0, "the best piece in the game has only {}", carrying[0].0);

        // And the top of the list is body pieces, which is what separated this
        // field from a coincidence. Ids end in 100 for a torso.
        let torsos = carrying.iter().take(5).filter(|(_, _, id)| id % 1000 == 100).count();
        assert!(torsos >= 4, "the most poise is not on body pieces: {:?}", &carrying[..5]);

        // Heavier armour carries more of it, on the whole.
        let heavy: f32 = carrying.iter().take(20).map(|(_, weight, _)| weight).sum::<f32>() / 20.0;
        let light: f32 =
            carrying.iter().rev().take(20).map(|(_, weight, _)| weight).sum::<f32>() / 20.0;
        assert!(heavy > light, "the most poise is not on the heaviest: {heavy:.1} against {light:.1}");
    }

    /// What they can carry, against the figure their own screen shows.
    ///
    /// Pinned on the one number this was found with, because both ways of
    /// getting it wrong are silent: a curve that matched by luck would drift
    /// at other values, and a band whose thresholds are recalled rather than
    /// measured would put somebody in the wrong roll without saying so.
    #[test]
    fn what_they_can_carry_matches_their_equipment_screen() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        if regulation.table("CalcCorrectGraph").is_none() {
            return;
        }

        // Off the screen: endurance 11 shows a maximum of 49.8.
        let Some(most) = regulation.can_carry(11) else {
            return;
        };
        assert!(
            (most - 49.8).abs() < 0.05,
            "endurance 11 should carry 49.8 and this says {most:.2}"
        );

        // It has to climb, and keep climbing. A flat curve sitting on 49.8
        // would pass the check above and be useless.
        let mut last = 0.0;
        for endurance in [1u32, 11, 25, 40, 60, 99] {
            let Some(now) = regulation.can_carry(endurance) else {
                panic!("no answer at endurance {endurance}");
            };
            assert!(now > last, "carrying did not rise from {last:.1} at endurance {endurance}");
            assert!((20.0..=400.0).contains(&now), "{now:.1} is not a carrying capacity");
            last = now;
        }

        // The bands, against the two the screen has been seen to print.
        let (light, share) = regulation.how_laden(11, 14.0).expect("a band at 14.0");
        assert_eq!(light, "light", "14.0 of 49.8 is {share:.1}% and reads light on the screen");
        let (medium, share) = regulation.how_laden(11, 20.0).expect("a band at 20.0");
        assert_eq!(medium, "medium", "20.0 of 49.8 is {share:.1}% and reads medium");
        // And the ends, which follow from the same scheme.
        assert_eq!(regulation.how_laden(11, 45.0).map(|(b, _)| b), Some("heavy"));
        assert_eq!(regulation.how_laden(11, 60.0).map(|(b, _)| b), Some("overloaded"));
    }

    /// Bleed comes back, and it is the number the game's own menu prints.
    ///
    /// Pinned against the screen: this installation shows Reduvia as
    /// "Накапливает кровотечение (82)", and that 82 is what the chain must
    /// produce. Two things could break it silently — the effect id offset
    /// moving, or the blood offset moving — and either would give a plausible
    /// wrong figure rather than nothing.
    #[test]
    fn bleed_is_read_through_the_effect_the_weapon_carries() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        if regulation.table("SpEffectParam").is_none() {
            return;
        }

        // The one weapon this project pins by rule, and the figure off its own
        // stat screen.
        assert_eq!(
            regulation.bleeds(1_040_000),
            Some(82),
            "Reduvia's bleed is 82 on the game's own screen"
        );

        // And it is not saying that about everything: a weapon with no ailment
        // must come back with none, or "does it bleed" is answered yes forever.
        let Some(weapons) = regulation.table("EquipParamWeapon") else {
            return;
        };
        let mut bleeding = 0;
        let mut dry = 0;
        for id in weapons.ids().filter(|id| id % 100 == 0) {
            match regulation.bleeds(id) {
                Some(value) => {
                    assert!(
                        (1..=1000).contains(&value),
                        "row {id} builds {value} of bleed, which is not a buildup"
                    );
                    bleeding += 1;
                }
                None => dry += 1,
            }
        }
        assert!(bleeding > 10, "only {bleeding} weapons bleed, which cannot be right");
        assert!(dry > bleeding, "more weapons bleed than do not: {bleeding} against {dry}");
    }

    /// A shield's block comes back, and it is a percentage.
    ///
    /// Pinned because the question that exposed the gap was answered wrongly
    /// with real confidence: "no shield in this build blocks 100% physical",
    /// out of a table with no shields in it. There are 334 that do.
    #[test]
    fn a_shield_blocks_something_and_it_is_a_percentage() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let Some(weapons) = regulation.table("EquipParamWeapon") else {
            return;
        };

        // Whatever is installed, a percentage is a percentage.
        let mut read = 0;
        let mut at_a_hundred = 0;
        for id in weapons.ids() {
            let Some(blocks) = regulation.blocks(id) else {
                continue;
            };
            read += 1;
            assert_eq!(blocks.len(), 5, "row {id} came back with {} kinds, not five", blocks.len());
            for (what, value) in &blocks {
                assert!(
                    (0.0..=100.0).contains(value),
                    "row {id} blocks {value} of {what}, which is not a percentage"
                );
            }
            // Physical leads, because the menu lists it first and every caller
            // reads it that way.
            assert_eq!(blocks[0].0, "physical", "row {id} does not lead with physical");
            if blocks[0].1 == 100.0 {
                at_a_hundred += 1;
            }
        }
        assert!(read > 100, "only {read} rows had a block figure at all");
        // The claim that started this. If an installation really had none, the
        // right answer would be to say so — but this one has them, and a test
        // that passes either way would not have caught anything.
        assert!(at_a_hundred > 0, "nothing reaches 100% physical, which is what was claimed");

        // And the reading is not the whole row shifted: a weapon that is not a
        // shield blocks a little, never a lot. If this offset were somebody
        // else's field the two groups would not separate.
        let low = weapons
            .ids()
            .filter_map(|id| regulation.blocks(id))
            .filter(|blocks| blocks[0].1 < 60.0)
            .count();
        assert!(low > 50, "every row blocks like a greatshield, so this is the wrong field");
    }

    /// A kind of damage is recognised in the language it was asked in.
    ///
    /// The failure this exists for was measured: a German question passed
    /// `blitz` and got "there is no such kind", which cost a round and could as
    /// easily have been answered "nothing in the game resists lightning".
    #[test]
    fn a_kind_of_damage_is_known_by_what_the_player_calls_it() {
        // The eight resolve to themselves, whatever the case.
        for english in kind::all() {
            assert_eq!(kind::named(english), Some(english));
            assert_eq!(kind::named(&english.to_uppercase()), Some(english));
        }
        assert_eq!(kind::all().count(), 8, "there are eight kinds");

        // The word that started this, and one of each other language.
        for (said, wanted) in [
            ("blitz", "lightning"),
            ("Blitzschaden", "lightning"),
            ("молния", "lightning"),
            ("молнии", "lightning"),
            ("rayo", "lightning"),
            ("физического", "physical"),
            ("physisch", "physical"),
            ("огонь", "fire"),
            ("Feuer", "fire"),
            ("fuego", "fire"),
            ("священный", "holy"),
            ("heilig", "holy"),
            ("магия", "magic"),
            ("магический", "magic"),
            ("колющий", "pierce"),
            ("дробящий", "strike"),
            ("рубящий", "slash"),
            ("corte", "slash"),
        ] {
            assert_eq!(kind::named(said), Some(wanted), "{said} was not understood");
        }

        // Nothing that is not a kind of damage.
        for nonsense in ["", "  ", "a", "xyz", "greatsword", "Reduvia", "поножи", "null"] {
            assert_eq!(kind::named(nonsense), None, "{nonsense:?} was taken for a damage kind");
        }

        // And the stems must not reach into each other. Every form of every
        // kind has to come back as ITS kind and no other — a stem short enough
        // to survive a declension is a stem long enough to collide, and a
        // collision here silently ranks armour against the wrong thing.
        for row in kind::SAID {
            let mine = row[0];
            for form in row {
                assert_eq!(kind::named(form), Some(mine), "{form} landed on the wrong kind");
            }
        }
    }

    /// A talisman's figures, against a row read by a second reader.
    ///
    /// Radagon's Soreseal, accessory 1051, applies effect 310510, and that row
    /// reads — by name, out of SmithBox — `addLifeForceStatus` 5,
    /// `addEndureStatus` 5, `addStrengthStatus` 5, `addDexterityStatus` 5, and
    /// 1.15 on all eight damage-taken multipliers. Four attributes and a
    /// fifteen per cent price, which is exactly what the sentence under it in
    /// the game says in words.
    ///
    /// This is pinned because the near-miss was silent. `changeStrengthPoint`
    /// is the field anybody would reach for, it is four hundred bytes away from
    /// the right one, and it reads zero — so the wrong reading does not fail,
    /// it reports a talisman that grants nothing and costs 15% more damage.
    #[test]
    fn a_talisman_reads_what_the_game_says_it_does() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let Some(soreseal) = regulation.charm(1051) else {
            return;
        };

        assert_eq!(soreseal.effect, 310_510, "the soreseal points somewhere else now");
        assert!(
            (soreseal.weight - 0.8).abs() < 0.001,
            "the weight reads {} — the offsets have moved",
            soreseal.weight
        );

        let gives: std::collections::HashMap<&str, i32> =
            soreseal.gives.iter().map(|(what, value)| (what.as_str(), *value)).collect();
        assert_eq!(gives.len(), 4, "four attributes, not {}: {:?}", gives.len(), soreseal.gives);
        for attribute in ["vigor", "endurance", "strength", "dexterity"] {
            assert_eq!(gives.get(attribute), Some(&5), "{attribute} is not +5");
        }
        // And not the four it does not touch. The failure this guards against
        // reads a neighbouring byte and reports mind or arcane by accident.
        for attribute in ["mind", "intelligence", "faith", "arcane"] {
            assert!(!gives.contains_key(attribute), "{attribute} should be untouched");
        }

        // The price, on every kind of damage there is.
        let taken: Vec<&(String, f32)> =
            soreseal.changes.iter().filter(|(what, _)| what.ends_with("taken")).collect();
        assert_eq!(taken.len(), 8, "eight kinds take more, not {}: {taken:?}", taken.len());
        for (what, rate) in &taken {
            assert!((rate - 1.15).abs() < 0.001, "{what} reads {rate}, not 1.15");
        }

        // And NOTHING else. This half matters as much as the half above and it
        // was added after the half above let a real fault through: two of the
        // rate offsets were guessed rather than walked, landed on padding that
        // reads zero, and a zero against an idle of 1.0 is reported as "-100%".
        // The answer told a player the talisman made blocking and casting
        // free. Every attribute assertion still passed.
        //
        // So: an offset that has drifted onto padding shows up here, because a
        // talisman with a real effect leaves nearly everything alone and the
        // list of what it touches is short and knowable.
        assert_eq!(
            soreseal.changes.len(),
            8,
            "the soreseal changes only the eight damage-taken rates, and this read {:?}",
            soreseal.changes
        );
        assert!(
            soreseal.adds.is_empty(),
            "the soreseal adds no resistance points, and this read {:?}",
            soreseal.adds
        );
    }

    /// The four matchers on `armour_against` must not steal from each other.
    ///
    /// Poise, then the four resistances, then the nine attributes, then the
    /// damage kinds — four families checked in order, each added in a different
    /// session, and until this test nothing checked that an earlier one does
    /// not swallow a word belonging to a later one. That failure would not look
    /// like a failure: it would rank by the wrong column and read as a perfectly
    /// good answer to a question nobody asked.
    #[test]
    fn the_four_matchers_do_not_steal_from_each_other() {
        // `armour_against` now tries four things in order — poise, then the
        // four resistances, then the nine attributes, then the damage kinds —
        // and each was added in a different session. Nothing until now checked
        // that an earlier one does not swallow a word belonging to a later one,
        // which would silently rank by the wrong column and look like a fine
        // answer to the wrong question.
        for kind in kind::all() {
            assert_eq!(
                resistance::named(kind),
                None,
                "the damage kind {kind} is being taken for a resistance, which runs first"
            );
            assert_eq!(
                attribute::named(kind),
                None,
                "the damage kind {kind} is being taken for an attribute, which runs first"
            );
            assert_eq!(kind::named(kind), Some(kind), "{kind} no longer finds itself");
        }

        // And the other direction, with ONE deliberate exception. "vitality"
        // is both: it is one of the four the equipment screen shows, and it is
        // also the name of a table slot — addVitalityStatus — that maps to no
        // stat this game displays. Resistance runs first and therefore wins,
        // which is right, because a player who types it means the one they can
        // see. Every other attribute must be untouched by the resistances.
        for attribute in attribute::all().filter(|name| *name != "vitality") {
            assert_eq!(
                resistance::named(attribute),
                None,
                "the attribute {attribute} is being taken for a resistance, which runs first"
            );
        }
        assert_eq!(resistance::named("vitality"), Some("vitality"));
    }

    /// Classes of weapon, by the English word a model will send.
    ///
    /// The specific reading has to beat the general one: "greatshield" contains
    /// "shield", and answering a question about greatshields with the whole
    /// shield table is how the ranking ends up leading with a buckler.
    #[test]
    fn a_class_of_weapon_is_found_by_its_english_name() {
        for (said, wanted) in [
            ("greatshield", vec![69]),
            ("Greatshields", vec![69]),
            ("katana", vec![13]),
            ("great katana", vec![94]),
            ("dagger", vec![1]),
            ("halberd", vec![29]),
            ("colossal sword", vec![7]),
            ("sacred seal", vec![61]),
        ] {
            assert_eq!(sort::named(said), wanted, "{said} was not found");
        }

        // The words that cover a family. All four shield classes, or the
        // question "which shield" is answered out of one of them.
        assert_eq!(sort::named("shield"), vec![65, 67, 69, 90]);
        assert_eq!(sort::named("shields"), vec![65, 67, 69, 90]);
        assert!(sort::named("sword").len() > 5);
        assert_eq!(sort::named("bow"), vec![50, 51, 53]);
        // Both a family and the name of one member. The family wins, and the
        // listing labels each row so the wider answer is not a vaguer one.
        assert_eq!(sort::named("spear"), vec![25, 28]);

        // Nothing that is not a class.
        for nonsense in ["", "  ", "xy", "Reduvia", "fire", "poise", "lightning"] {
            assert!(sort::named(nonsense).is_empty(), "{nonsense:?} was taken for a class");
        }

        // Every class answers to its own name and lands on itself alone, and
        // every one has a name and a place to read the game's own word for it.
        for (kind, menu, english) in sort::ALL {
            assert!(sort::named(english).contains(&kind), "{english} does not find itself");
            assert_eq!(sort::english(kind), Some(english));
            // Bare hands is the one row the game keeps for empty hands. It is
            // not a class and has no menu entry; everything else has both.
            if kind == 33 {
                assert_eq!(menu, 0);
                assert_eq!(sort::menu_id(kind), None);
            } else {
                assert!(menu >= 60_010 && menu <= 60_187, "{english} has a stray menu id");
                assert_eq!(sort::menu_id(kind), Some(menu));
            }
        }

        // The four that block, and only those four.
        let blocking: Vec<u16> =
            sort::ALL.iter().map(|(kind, _, _)| *kind).filter(|kind| sort::blocks(*kind)).collect();
        assert_eq!(blocking, vec![65, 67, 69, 90]);
    }

    /// The spirit ashes are found, and the figure that is NOT there stays out.
    ///
    /// Pinned because the whole feature rests on one flag being the right one,
    /// and because the honest half — that a summon's cost is unreadable HERE —
    /// is exactly the sort of caveat that quietly disappears when somebody
    /// later "fixes" an offset that was never broken.
    /// The ashes of war read, and their FP with them.
    ///
    /// Pinned because the whole thing hangs on one link — EquipParamGem's
    /// swordArtsParamId at 0x18 reaching the SwordArtsParam rows already read
    /// for a weapon's skill. If that offset ever drifts, every ash comes back
    /// with no skill and the tool goes quiet rather than wrong, which is the
    /// sort of failure nobody notices.
    #[test]
    fn the_ashes_of_war_reach_their_skill() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let ashes = regulation.ashes_of_war();
        assert!(
            (150..=400).contains(&ashes.len()),
            "{} gem rows, which is not the shape of this table",
            ashes.len()
        );

        // Nearly all of them reach a skill. A handful do not and that is real —
        // but if the link breaks it is ALL of them, so the bar is high.
        let reached = ashes.iter().filter(|(_, skill)| skill.is_some()).count();
        assert!(
            reached * 10 > ashes.len() * 9,
            "only {reached} of {} ashes reach a skill — swordArtsParamId has moved",
            ashes.len()
        );

        // And the FP, which is the figure the tool exists for.
        let priced: Vec<u16> = ashes
            .iter()
            .filter_map(|(_, skill)| skill.as_ref())
            .flat_map(|skill| skill.costs.iter().map(|(_, fp)| *fp))
            .collect();
        assert!(priced.len() > 100, "only {} FP figures across every ash", priced.len());
        assert!(
            priced.iter().all(|fp| *fp > 0 && *fp < 300),
            "an FP cost is outside anything believable: {:?}",
            priced.iter().max()
        );
    }

    #[test]
    fn the_physick_tears_are_found_and_read() {
        // Shipped without a test, unlike the spirit ashes beside them, and the
        // whole feature rests on one goodsType being the right one. If 10 ever
        // stops meaning "crystal tear" this should be what says so, rather than
        // an answer confidently listing the wrong sixty items.
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let tears = regulation.tears();

        assert!(
            (40..=120).contains(&tears.len()),
            "{} crystal tears, which is not the shape of this group",
            tears.len()
        );

        // About half carry a readable effect and about half do not. Both halves
        // matter: the readable ones are the answer, and the unreadable ones are
        // why the tool says so out loud instead of filling the gap.
        let readable = tears
            .iter()
            .filter(|(_, gives, changes, adds)| {
                !gives.is_empty() || !changes.is_empty() || !adds.is_empty()
            })
            .count();
        assert!(
            readable > 20,
            "only {readable} tears read anything — the effect route has broken"
        );
        assert!(
            readable < tears.len(),
            "every tear now reads something, so the tool's \"no figure in the tables\" \
             wording is stale and has to be revisited"
        );

        // A tear that grants a flat attribute is what the group is FOR, and it
        // is the shape most easily lost if the effect fields drift.
        let attributes: Vec<&(String, i32)> =
            tears.iter().flat_map(|(_, gives, _, _)| gives.iter()).collect();
        assert!(
            attributes.iter().any(|(_, value)| *value >= 5),
            "no tear grants an attribute any more: {attributes:?}"
        );
    }

    #[test]
    fn the_spirit_ashes_are_read_and_their_cost_is_not() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        crate::formats::oodle::register(&game_dir);
        let Some(regulation) = installed(game, &game_dir, mod_dir.as_deref()) else {
            return;
        };
        let ashes = regulation.spirits();

        // Base rows only, and there are enough of them to be a real list.
        // 217 base rows carry the flag; 114 of them have a name the launcher
        // can resolve, and that is the number a player would recognise. Pinned
        // loosely because a conversion update may add or drop a few, and
        // tightly enough that a broken flag shows up at once.
        assert!(
            (150..=300).contains(&ashes.len()),
            "{} spirit ashes found, which is not the shape of this table",
            ashes.len()
        );
        assert!(ashes.iter().all(|ash| ash.id % 100 == 0), "an upgraded copy got in");

        // Two families, and only two.
        let mut families: Vec<u8> = ashes.iter().map(|ash| ash.sort).collect();
        families.sort_unstable();
        families.dedup();
        assert_eq!(families, vec![7, 8], "the goodsType families have changed");

        // The flag is the union of those two families, which is what makes it
        // the right discriminator rather than a plausible one.
        if let Some(table) = regulation.table("EquipParamGoods") {
            let flagged = table
                .ids()
                .filter(|id| table.u8(*id, spirit::SUMMONS).is_some_and(|flag| flag > 0))
                .count();
            let typed = table
                .ids()
                .filter(|id| table.u8(*id, spirit::SORT).is_some_and(|sort| sort == 7 || sort == 8))
                .count();
            assert_eq!(flagged, typed, "the summon flag and the two goodsTypes have diverged");
        }

        // And the caveat. Every one of them reads no FP cost in this
        // installation; vanilla holds 88 for row 231000. If this ever starts
        // passing a figure through, the answer's wording has to change with it.
        assert!(
            ashes.iter().all(|ash| ash.fp.is_none()),
            "a spirit ash now has a readable FP cost — the tool still tells the player it \
             cannot give one, and that text has to be fixed with this"
        );

        // What IS readable, so the list is worth printing at all.
        assert!(
            ashes.iter().filter(|ash| ash.upgrades).count() > 50,
            "almost nothing upgrades, which would mean the reinforce offsets moved"
        );
    }

    /// The nine attributes, and the two words deliberately left out of them.
    #[test]
    fn the_menu_ids_for_the_attributes_are_the_ones_the_game_prints() {
        // These eight ids put the game's OWN word on the player's stat line, so
        // nothing has to be translated on the way out — an answer turned
        // "Faith (FTH) 22" into "Фея 22", a fairy, while quoting the right
        // number. Read against the installed game rather than asserted, because
        // the whole value is that they are the game's words and not mine.
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
        let Some(menu) = tables.get("GR_MenuText") else {
            return;
        };

        for (english, at) in attribute::MENU {
            let word = menu.get(&at).map(|said| said.trim().to_string());
            assert!(
                word.as_ref().is_some_and(|word| !word.is_empty()),
                "the game has no word at #{at}, which is where {english} was supposed to be"
            );
        }

        // The two that do NOT translate straight across, and getting them the
        // wrong way round sends somebody's levels into the wrong stat. The
        // game's own entries prove which is which by carrying the English
        // abbreviation: "Мудрость(INT)" and "Интеллект(FP)".
        let word = |english: &str| -> String {
            let at = attribute::MENU
                .iter()
                .find(|(name, _)| *name == english)
                .map(|(_, at)| *at)
                .expect("every attribute has an id");
            menu.get(&at).map(|said| said.trim().to_string()).unwrap_or_default()
        };
        let intelligence = word("Intelligence");
        let mind = word("Mind");
        assert_ne!(intelligence, mind, "two attributes are reading the same entry");
        if intelligence.contains('(') {
            assert!(
                intelligence.contains("INT"),
                "#{} was taken for Intelligence and the game calls it {intelligence}",
                attribute::MENU.iter().find(|(n, _)| *n == "Intelligence").unwrap().1
            );
            assert!(
                mind.contains("FP"),
                "#{} was taken for Mind and the game calls it {mind}",
                attribute::MENU.iter().find(|(n, _)| *n == "Mind").unwrap().1
            );
        }
    }

    #[test]
    fn an_attribute_answers_to_the_word_the_game_prints() {
        for (said, wanted) in [
            ("faith", "faith"),
            ("вера", "faith"),
            ("build foi", "faith"),
            ("Glaube", "faith"),
            ("strength", "strength"),
            ("сила", "strength"),
            ("fuerza", "strength"),
            ("dexterity", "dexterity"),
            ("ловкость", "dexterity"),
            ("endurance", "endurance"),
            ("выносливость", "endurance"),
            ("vigor", "vigor"),
            ("mind", "mind"),
        ] {
            assert_eq!(attribute::named(said), Some(wanted), "{said} was not understood");
        }

        // The Russian names do NOT translate straight across, and swapping
        // these two sends somebody's levels into the wrong stat.
        assert_eq!(attribute::named("мудрость"), Some("intelligence"));
        assert_eq!(attribute::named("колдовство"), Some("arcane"));
        assert_eq!(attribute::named("интеллект"), Some("mind"));

        // Left out on purpose, because each already means something else here
        // and an ambiguous stem silently ranks by the wrong column. "aguante"
        // is on the poise list; "Vitalität" is the death-blight resistance.
        assert_ne!(attribute::named("aguante"), Some("endurance"));
        assert_ne!(attribute::named("Vitalität"), Some("vigor"));

        // Spanish faith is two letters and lives inside other words. Matched
        // as a whole word it works; matched loosely it would rank armour by
        // faith for a question about defence or a perfumer.
        assert_eq!(attribute::named("build de fe"), Some("faith"));
        assert_eq!(attribute::named("fe"), Some("faith"));
        for elsewhere in ["defense", "fear", "perfumer", "fetish"] {
            assert_ne!(
                attribute::named(elsewhere),
                Some("faith"),
                "{elsewhere} was read as Spanish faith"
            );
        }

        // And nothing that is not an attribute at all.
        for nonsense in ["", " ", "fire", "lightning", "Reduvia", "greatshield"] {
            assert_eq!(attribute::named(nonsense), None, "{nonsense:?} was taken for an attribute");
        }

        // Every listed word finds its own row and no other.
        for (mine, words) in attribute::SAID {
            for word in words {
                assert_eq!(
                    attribute::named(word),
                    Some(mine),
                    "{word} landed on the wrong attribute"
                );
            }
        }
    }

    /// The four the equipment screen shows, by whatever the player calls them.
    ///
    /// Worth its own check because the near-miss was expensive: "Robustheit"
    /// went to the ranking, came back "no such kind", and the answer told a
    /// German player that armour has no such figure. It has one, it was read,
    /// and the fix was very nearly to file Robustheit under poise — which would
    /// have answered a question about bleed and frost with one about stagger.
    #[test]
    fn the_four_resistances_answer_to_their_own_names() {
        for (said, wanted) in [
            ("robustness", "robustness"),
            ("Robustheit", "robustness"),
            ("живучесть", "robustness"),
            ("живучести", "robustness"),
            // The ailments themselves, which is how it actually gets asked.
            ("кровотечение", "robustness"),
            ("frostbite", "robustness"),
            ("обморожение", "robustness"),
            ("immunity", "immunity"),
            ("Immunität", "immunity"),
            ("иммунитет", "immunity"),
            ("poison resistance", "immunity"),
            ("красная гниль", "immunity"),
            ("focus", "focus"),
            ("концентрация", "focus"),
            ("madness", "focus"),
            ("безумие", "focus"),
            ("vitality", "vitality"),
            ("физ. мощь", "vitality"),
            ("death blight", "vitality"),
        ] {
            assert_eq!(resistance::named(said), Some(wanted), "{said} was not understood");
        }

        // The trap this was written against: "мор" is death blight and "мороз"
        // is frost, and the short one must not swallow the long one.
        assert_eq!(resistance::named("мороз"), Some("robustness"));

        // And nothing that is not one of the four. A damage kind especially —
        // the two rankings mean different things and reading a percentage as a
        // bar fill rate would be a wrong number, not a missing one.
        for nonsense in ["", "  ", "a", "xyz", "lightning", "physical", "poise", "Reduvia"] {
            assert_eq!(resistance::named(nonsense), None, "{nonsense:?} was taken for a resistance");
        }

        // Every listed word lands on its own row and no other.
        for (mine, words) in resistance::SAID {
            for word in words {
                assert_eq!(
                    resistance::named(word),
                    Some(mine),
                    "{word} landed on the wrong resistance"
                );
            }
        }
    }

    /// The values themselves belong to whatever is installed, so what is pinned
    /// here is the shape they have to have. The one that would go wrong
    /// silently is the negation: the table stores how much damage gets through
    /// and the menu shows how much is stopped, so a reader that forgot to turn
    /// it round would report ninety per cent protection for a rag.
    #[test]
    fn armour_comes_back_the_way_the_menu_shows_it() {
        let Some(path) = crate::testing::regulation(crate::games::Game::EldenRing) else {
            return;
        };
        let regulation = Regulation::open(&path).expect("the installed regulation reads");
        let Some(table) = regulation.table("EquipParamProtector") else {
            return;
        };
        assert!(table.len() > 100, "only {} pieces", table.len());

        let mut real = 0;
        for id in table.ids() {
            let Some(piece) = regulation.armour(id) else {
                continue;
            };
            // The rows with no weight are the game's own placeholders.
            if piece.weight <= 0.0 {
                continue;
            }
            real += 1;

            assert!(piece.weight < 60.0, "{id} weighs {}", piece.weight);
            for (kind, stopped) in &piece.negation {
                assert!(
                    (-50.0..95.0).contains(stopped),
                    "{id} stops {stopped}% of {kind}, which is not a percentage anybody wears"
                );
            }
            for (what, value) in &piece.resistance {
                assert!(*value < 2000, "{id} has {value} {what}");
            }
        }
        assert!(real > 100, "only {real} pieces had any weight");
    }

    #[test]
    fn the_upgrade_curve_gives_the_number_on_the_screen() {
        // Only the mod's tables carry these figures; the base game's Reduvia is
        // a different weapon entirely.
        let Some(dir) = crate::testing::mod_dir(crate::games::Game::EldenRing) else {
            return;
        };
        let Ok(regulation) = Regulation::open(&dir.join("regulation.bin")) else {
            return;
        };

        let Some(held) = regulation.weapon(1_040_000) else {
            return; // Another total conversion, which will not have this row.
        };
        assert_eq!(held.level, 0);
        assert_eq!(
            held.damage,
            vec![("fire".to_string(), 106)],
            "the screen says 106 fire"
        );

        // An id carrying its level is understood, and more upgrade is more
        // damage. Anything else means the row is being picked wrongly.
        let sharper = regulation.weapon(1_040_003).expect("+3 is the same weapon");
        assert_eq!(sharper.level, 3);
        assert_eq!(sharper.id, 1_040_000, "it is still the same row");
        let fire = |weapon: &Weapon| {
            weapon
                .damage
                .iter()
                .find(|(kind, _)| kind == "fire")
                .map(|(_, value)| *value)
                .unwrap_or(0)
        };
        assert!(
            fire(&sharper) > fire(&held),
            "+3 deals {} where +0 deals {}",
            fire(&sharper),
            fire(&held)
        );

        // Scaling grows with the upgrade too, and both letters stay letters.
        let faith = |weapon: &Weapon| {
            weapon
                .scaling
                .iter()
                .find(|(what, _)| what.starts_with("faith"))
                .map(|(_, value)| *value)
                .unwrap_or(0.0)
        };
        assert!(faith(&sharper) > faith(&held), "faith scaling did not grow");
    }
}
