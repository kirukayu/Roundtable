# Reading the mod's own numbers

What a weapon actually does under a total conversion is in `regulation.bin`, and
nothing else has it — every wiki prints the base game's figures.

**Done.** `formats::regulation` reads the file: AES-CBC, a DCX wrapper
compressed with Zstandard, a BND4 archive, then the PARAM tables. Half a second
for all 194 of them, no running game, and it reads the mod's regulation and the
base game's alike. The fixture below is a test.

Two things were wrong on the way and both are worth remembering. The layers are
not what they looked like from the outside — the file opens with random bytes,
which reads as encryption but says nothing about what is under it, and the
compression turned out to be Zstandard rather than the Oodle that would have
needed the game's own library. And the field offsets came out one byte long,
because a `dummy8` carrying a bit size pads somebody else's byte rather than
taking one of its own; every field after it read the tail of the one before.

The offsets below are the corrected ones, and they are checked on every test
run against a weapon whose numbers are known from three independent places.

## Proven

The data is right, checked three ways for Reduvia (`EquipParamWeapon` row
1040000) on The Convergence 3.0.1:

| field | vanilla | mod | on the stat screen |
|---|---|---|---|
| `attackBasePhysics` | 79 | 0 | Физическ. 0 |
| `attackBaseFire` | 0 | 82 | Огонь 106 + 49 |
| `correctStrength` | 10 | 5 | Str E |
| `correctAgility` | 40 | 50 | Dex C |
| `correctFaith` | 0 | 65 | Fth C |
| `correctLuck` (Arcane) | 55 | 65 | Arc C |
| `properAgility` | 13 | 8 | Dex 8 |
| `properFaith` | 0 | 13 | Fth 13 |
| `weight` | 2.5 | 2.5 | Вес 2.5 |

The mod turns a physical dexterity dagger into a fire faith/arcane one, which is
why the Blood Initiate starts with it. Use this table as the fixture: any reader
that produces these numbers for this row is correct.

## Field offsets, verified

Every one of these is exercised by the test in `formats::regulation`. A weapon
row is 664 bytes.

```
+0x010  weight                  f32      +0x0f2  properStrength    u8
+0x024  correctStrength         f32      +0x0f3  properAgility     u8
+0x028  correctAgility          f32      +0x0f4  properMagic       u8
+0x02c  correctMagic            f32      +0x0f5  properFaith       u8
+0x030  correctFaith            f32      +0x195  properLuck        u8
+0x19c  correctLuck             f32      +0x242  wepRegainHp       u16
+0x0c8  attackBasePhysics       u16      +0x198  swordArtsParamId  s32
+0x0ca  attackBaseMagic         u16      +0x22c  attackElementCorrectId s32
+0x0cc  attackBaseFire          u16      +0x0da  reinforceTypeId   s16
+0x0ce  attackBaseThunder       u16      +0x18c  attackBaseDark    u16
+0x0d0  attackBaseStamina       u16
```

`correctLuck` and `properLuck` are arcane — the tables still call it luck, from
an older game.

## The container

Read off the bytes, and each part checks itself: a file's offset plus its size
lands on the next one, and the entry stride is written in the header at `+0x20`.

```
BND4 header      +0x0c  file count        +0x20  bytes per entry (36)
entry            +0x00  flags             +0x18  where the data starts
                 +0x08  size              +0x1c  index
                 +0x10  size again        +0x20  where the name starts
PARAM            +0x0a  row count, then rows of 24 from +0x40:
                        +0x00 id (i64)    +0x08 where the row's data starts
```

## What is left

Only the wiring: a tool that hands these figures to the assistant, saying
plainly that they are the installed mod's rather than the base game's. The
reading itself is done.

## The upgrade curve

`reinforceTypeId` at `+0x0da` — **two bytes, not four**; read as an `i32` it
picks up the field behind it and lands on a row that does not exist, which looks
exactly like a weapon that cannot be upgraded. The row in
`ReinforceParamWeapon` is `reinforceTypeId + level`, and an item id carries its
level in its last two digits: `1040003` is Reduvia +3, row `1040000`.

```text
ReinforceParamWeapon, 128 bytes a row
+0x00  physicsAtkRate   f32      +0x1c  correctStrengthRate     f32
+0x04  magicAtkRate     f32      +0x20  correctAgilityRate      f32  (dexterity)
+0x08  fireAtkRate      f32      +0x24  correctMagicRate        f32  (intelligence)
+0x0c  thunderAtkRate   f32      +0x28  correctFaithRate        f32
+0x58  darkAtkRate      f32      +0x60  correctLuckRate         f32  (arcane)
```

**The 82-against-106 puzzle, solved.** It was never the upgrade level. The
player's Reduvia is `1040000`, which is +0. Its `reinforceTypeId` is 2200, and
in this mod row 2200 already carries ×1.3 where the base game's first row is
×1.0. 82 × 1.3 floors to 106 — the number on their screen, exactly.

`maxReinforceLevel` at `+0x57` is not a maximum level. Across rows 2200-2203 it
reads 0, 2, 3, 5, which is something else; nothing is reported from it rather
than reporting a guess.

## Armour

`EquipParamProtector`, 250 fields, 416 bytes a row.

```text
+0x024  weight                f32     +0x0c4  immunity   u16  (poison, rot)
+0x0e4  neutralDamageCutRate  f32     +0x0c8  robustness u16  (bleed, frost)
+0x0e8  slashDamageCutRate    f32     +0x00c  focus      u16  (sleep, madness)
+0x0ec  blowDamageCutRate     f32     +0x0ca  vitality   u16  (death blight)
+0x0f0  thrustDamageCutRate   f32
+0x0f4  magicDamageCutRate    f32
+0x0f8  fireDamageCutRate     f32
+0x0fc  thunderDamageCutRate  f32
+0x11c  darkDamageCutRate     f32
```

**Two things a reader gets wrong.** The cut rates are stored the opposite way
round from the menu: `0.9` means nine tenths of the damage gets through, and the
screen shows ten per cent negation. Quoting the stored number would report a rag
as near-impervious. And the table keeps a field per ailment where the game shows
four groups — poison and rot both hold the immunity figure, bleed and frost the
robustness one — so reading all seven prints the same numbers under different
names. One of each group is taken.

**Poise is deliberately not reported.** `toughnessDamageCutRate` reads 1.0 for
every piece and `toughnessCorrectRate` reads 0, so neither is what the stat
screen shows. A number that cannot be stood behind is worse than none, because
the player will quote it.

## Bosses, and what this table will not tell you

`GameAreaParam`, 216 rows, 96 bytes each.

```text
+0x04  bonusSoul_single  u32     runes for killing it alone
+0x38  foundBossTextId   s32     not the name — see below
+0x48  bossPosX/Y/Z      f32
+0x54  bossMapAreaNo     u8      \
+0x55  bossMapBlockNo    u8      | mAA_BB_CC_00
+0x56  bossMapMapNo      u8      /
```

Verified: the row ids are map-encoded and come out as real maps, and the
rewards run from a thousand to five hundred thousand runes with a median of
fifteen thousand and Stormveil's boss at twelve — a shape a misread field does
not have.

**`foundBossTextId` is not the boss's name.** It sounds like it, and it is not:
its values resolve to weapon and armour names in the message tables, so it
indexes something else. A boss's name belongs to the enemy placed on the map,
which lives in the map files rather than in any param.

So this table can say what a fight is worth and where it is, and nothing about
who it is — and **none of it is given to the assistant**. That is deliberate. A
rune figure with no name attached is an invitation to supply the name, and
supplying names nobody looked up is the exact failure this was meant to close:
asked where to go next, a model named the boss of Castle Morne and got it
wrong.

**Where the names actually are, and what is still missing.** Checked against the
game's own field definition rather than guessed at: `GAME_AREA_PARAM_ST` has no
reference to an enemy at all — flags, runes, a position and a map, and nothing
else — so no offset in it will ever produce a name. `foundBossTextId` is
発見時テキストID, the line shown *when the boss is discovered*, which is why its
values landed in a different message table.

The names themselves are within reach: `NpcParam` is in the same regulation this
module already reads, 7039 rows, and it carries a `nameId` that indexes the
`NpcName` message table. What is missing is only the join — which NPC row
belongs to which of these 216 areas — and that lives in the map files.

Two warnings for whoever picks this up. The offset of `nameId` is **not** worked
out by walking the field list naively; that was tried here and produced `0x00d`
for a four-byte field, which is misaligned and therefore wrong. It is the
`dummy8`-with-a-bit-size trap described at the top of this file, and it has to be
handled the same way the weapon offsets were.

And the map files are only loose for a total conversion — 634 of them under
`map/mapstudio/*.msb.dcx`, next to the regulation. A player on the plain game has
them inside `Data0.bhd`, behind an RSA-encrypted archive header, so doing this
for one player is not doing it for everybody.

## The scaling bonus, and what reading it would take

**Done since:** talismans. `EquipParamAccessory`, 157 rows in the base game and
210 under The Convergence, 96 bytes each, `weight` at `+0x0c`. What a talisman
*does* is still not in its row — that is four `residentSpEffectId` fields and a
chain this module does not follow — so the effect comes from the item's own
description text instead, which is the better source anyway and reads in the
player's own language. Checked against SmithBox, which reads the same file with
FromSoftware's definition: row 1010 weighs 0.3, and so does ours.

**Still to do:** the bonus the stat screen adds on top of a weapon's attack —
the `+ 49` in "Огонь 106 + 49". That is the one number a player can see and this
cannot, and it is why questions about what another ten points would buy are
still answered from memory and still get it wrong.

It is four tables, not one, and all four are in the regulation already parsed:

```text
EquipParamWeapon.attackElementCorrectId  -> AttackElementCorrectParam
                 correctStrength/Agility/Magic/Faith/Luck   the scaling percentages
                 reinforceTypeId + level                    -> ReinforceParamWeapon
                                                               correct*Rate per stat
                 correctType_* (offsets not yet worked out) -> CalcCorrectGraph row
```

```text
AttackElementCorrectParam, 184 rows, 128 bytes each
  +0x00  25 one-bit flags, five stats x five damage types, in this order:
         physical STR DEX INT FTH ARC, then magic, then FIRE, then lightning,
         then dark. Fire's five are therefore bits 10 to 14.
  +0x04  25 x s16  overwrite<Stat>CorrectRate_by<Type>
  +0x36  25 x s16  Influence<Stat>CorrectRate_by<Type>

CalcCorrectGraph, 77 rows, 80 bytes each — all f32
  +0x00  stageMaxVal0..4       the five threshold points
  +0x14  stageMaxGrowVal0..4   the correction at each threshold
  +0x28  adjPt_maxGrowVal0..4  the curvature between them
```

The curve between two thresholds is not linear: with `r` the position between
them, the growth is `r ^ adj` when `adj` is positive and `1 - (1 - r) ^ |adj|`
when it is negative, and the result is interpolated between the two
`stageMaxGrowVal`s either side.

## The map files, as far as they have been read

Boss names live with the enemy placed in the map, and a total conversion leaves
those loose: 634 of them under `map/mapstudio/*.msb.dcx`. Everything below was
read off the real bytes of `m60_35_44_00.msb.dcx` and each step checks itself.

**They need no new unpacking.** `formats::dcx` opens them as they are — 35 KB
packed, 845 KB plain — so the RSA-and-BHD5 problem does not arise for a modded
player at all. It arises only for somebody on the plain game, whose maps are
inside `Data2.bhd`; that is solved further down, under "The packed archives".

```text
+0x00  "MSB " magic, then int version (1), then int headerSize (0x10)

Each block, starting at 0x10 and chained by the last field:
  +0x00  int   version        (73 in this build)
  +0x04  int   offsetCount    N
  +0x08  long  nameOffset     absolute; the block's name in UTF-16
  +0x10  long  entry[N-1]     absolute offsets, one per entry
         long  nextBlock      0 on the last
```

The layout proves itself: `0x18 + 8 + (94-1)*8 + 8` lands exactly on `0x310`,
which is the value `nameOffset` holds. The blocks come in a fixed order —
`MODEL_PARAM_ST`, `EVENT_PARAM_ST`, `POINT_PARAM_ST`, `ROUTE_PARAM_ST`,
`LAYER_PARAM_ST`, `PARTS_PARAM_ST` — and the last is the one that matters, with
619 entries in that tile, 59 of them named like a character.

```text
Each entry in PARTS_PARAM_ST:
  +0x00  long  nameOffset     RELATIVE TO THE ENTRY, not the file. Reading it
                              as absolute gives a plausible-looking string and
                              the wrong one; that cost a round to spot.
  +0x08  int   instanceId
  +0x0c  int   type           2 is the enemy-shaped one; 0 is a map piece
  +0x10  int   typeIndex
  +0x14  int   modelIndex
  +0x20  f32   x, y, z        where it stands
  +0x50  long  slot[8]        offsets, again relative, to the parts of the
                              entry that differ by type. Slot 4 is the
                              type-specific block.
```

**Slot 3, not slot 4.** Slot 4 reads -1, -1 and zeros for every enemy in the
tile, which looked like the type block and is not one. Slot 3 is:

```text
Enemy data, at entry + slot[3]:
  +0x00  int   unk
  +0x04  int   unk
  +0x08  int   thinkParamId
  +0x0c  int   npcParamId     <- this one
  +0x10  int   talkId
```

`+0x0c` was not guessed. Every four-byte word in the block was tested against
the installed `NpcParam`'s real row ids across all 54 ordinary enemies in the
tile, and `+0x0c` is a live row in **54 of 54**. `+0x08` hits 53, which is
`thinkParamId` colliding by coincidence — the kind of near-miss that a single
hand-picked example would have sold as the answer.

**The chain runs end to end.**

```text
map part (type 2) -> npcParamId -> NpcParam row
                                     +0x00c  nameId   -> NpcName message table
                                     +0x024  hp
                                     +0x02c  getSoul  = the runes it is worth
NpcParam is 7819 rows here, 736 bytes each.
```

Checked on the real files: every enemy in `m60_35_44_00` resolves to a row with
plausible figures — 169 HP and 111 runes for a small one, 438 and 1270 for
something bigger — and `c0000_9001` comes out as "Белоликий Варрэ", a real
character, in Russian, out of the running game's own text.

Most enemies resolve to no name at all, and that is correct rather than a
failure: a soldier or a rat has no `NpcName` entry, only named characters and
bosses do. A reader must treat the empty case as "this one has no name" and not
go looking for one.

**This also retires `GameAreaParam` as the route to boss rewards.** `getSoul`
sits on the NPC itself, next to its name, so runes and names arrive together and
the "a rune figure with no name attached" problem above never arises. There is
no longer any need to match an enemy to one of the 216 areas by position, which
was the join with a threshold in it that could silently name the wrong thing.

**One figure out of that row must not be repeated, and it took a second look to
see it.** Every human character is built on the `c0000` model and their
`NpcParam` rows all carry the same `hp`, so the reader reported Sir Gideon Ofnir
and a wandering knight as equally hard to kill. No outside source is needed to
know that is wrong — one number cannot be the health of all of them. The game
works the real one out somewhere this does not read, so `hp` is now given only
where the map's own name for the part does not begin `c0000`, and the answer for
the rest is that it is not known.

**The fixture is already known and it is exact.** This character has FTH 22, ARC
26, DEX 14, STR 10, and their Reduvia shows 106 fire and a bonus of 49. Any
implementation that produces 49 for those inputs is right; one that produces
anything else is not, and must not be wired to the assistant, because a scaling
figure that is confidently wrong is worse than the honest "not readable" the
assistant now gives.

## The packed archives

What a player who has installed nothing has instead of loose files. Seven of
them beside the executable — `Data0`–`Data3`, `DLC`, `sd`, `sd_dlc02` — each a
`.bhd` index and a `.bdt` of contents, and the maps are in **`Data2`**.

Four layers, and every one was settled by making it prove itself.

```text
Data2.bhd   RSA: 256 bytes in, 255 out, plain modular exponentiation
  └ BHD5    buckets of 40-byte records, files named only by a hash
Data2.bdt   the contents, at the offsets the records give
  └ AES     128-bit, ECB, over the ranges the record lists
    └ DCX   Kraken, so the game's own oo2core is needed
```

**The key needs no label.** There are 29 of them and none says which archive it
belongs to. None has to: the right key is the one whose first block comes out
`BHD5`, and no wrong key can fake four bytes of a 2048-bit result. Every archive
is opened by trying them in turn. The encryption is bare modPow with no padding
scheme, and the plaintext is 255 bytes with a zero byte in front — so the right
key gives back exactly 255 and they start with the magic.

```text
Header:
  +0x00  "BHD5", then ff, 01, 00 00, then an int that is always 1
  +0x0c  int   headerSize
  +0x10  int   bucketCount
  +0x14  int   bucketsOffset
  +0x18  int   saltLength, then the salt itself ("GR_other", "_map", "GR_asset")

Each bucket, at bucketsOffset + n*8:
  +0x00  int   recordCount
  +0x04  int   recordsOffset

Each record, 40 bytes:
  +0x00  long  nameHash
  +0x08  int   paddedSize
  +0x0c  int   size        0 means it was never padded, so paddedSize is real.
                           Taking the 0 literally truncates the file to nothing.
  +0x10  long  offset into the .bdt
  +0x18  long  shaOffset   into the header
  +0x20  long  aesOffset   into the header

At aesOffset:
  +0x00  byte[16]  the key
  +0x10  int       how many ranges
  +0x14  (long, long) * n   the ranges of the file that are encrypted
```

**The record's shape came out of the spacing, not out of memory.** The gap
between one record's `aesOffset` and the next is 36 bytes on some and 100 on
others — which is exactly `16 + 4 + 16` and `16 + 4 + 5*16`, one range and five.
The AES entry was read as a key only after its own layout had been predicted
that way and held.

**The name hash**, and the one check worth more than any hand-picked example:

```text
lowercase the path, turn '\' into '/', put a '/' in front if there is none,
then fold: value = value * 0x85 + character   (wrapping, 64-bit)
```

Every record in an archive must sit in bucket `hash % bucketCount`. It does — in
**all ~120,000 records across all seven archives**. That single check covers the
fold, the 40-byte stride and the bucket table at once, and it would collapse
rather than degrade if any of the three were wrong.

**ECB, not CBC, and the first block cannot tell you.** A zero IV makes CBC's
first block identical to ECB's, so the `DCX` magic appears either way and proves
nothing. What separates them is further in: ECB leaves `KRAK` in the DCX method
field where CBC leaves noise.

**The index keeps no names, so the list of maps is rebuilt.** Map ids have one
shape, `mAA_BB_CC_DD`, so the whole space is asked about instead — and the fold
being a left fold is what makes that affordable: every candidate shares
`/map/mapstudio/m`, so only the digits are new work. Sweeping it finds **864
maps**, and the spread is the game's own: 782 overworld tiles, 21 catacombs, 19
caves, 8 tunnels, and the legacy dungeons one apiece.

Opening a whole index is several thousand exponentiations, so it is not done to
find out which archive to use. `glance` unlocks the header, the bucket table and
the one bucket a name falls in — about ten blocks — and only the archive that
answers is opened in full.

**End to end, measured:** a plain installation gives 339 named things across 121
maps in 3.2 seconds, which is faster than the 634 loose files take, because most
maps hold nothing named and an archive read is cheaper than a file open.

## The skill on a weapon

`swordArtsParamId` was noted here as "at +0x198, unverified — the
`foundBossTextId` trap all over again, so it was left alone rather than
guessed". It is verified now, and by two independent things rather than one.

**The offset is deduced, and the deduction is checkable.** `NEEDS_ARCANE` is one
byte at 0x195 and `SCALE_ARCANE` is four at 0x19c, so a four-byte field at 0x198
ends exactly where the latter begins — there is nowhere else it fits. That is an
argument. The check is that every weapon which names a skill names one the skill
table really has, across the whole table; shift the offset by four and it
collapses at once.

```text
EquipParamWeapon +0x198  swordArtsParamId -> SwordArtsParam row (32 bytes)
                                              +0x0c  textId -> ArtsName
                                              +0x10  useMagicPoint_L1
                                              +0x12  useMagicPoint_L2
                                              +0x14  useMagicPoint_R1
                                              +0x16  useMagicPoint_R2
```

**Signed, all four costs.** A button the skill does not answer to carries -1, and
read unsigned that is 65535 FP.

**`textId` is not the row id.** It usually is, which is exactly why assuming it
would have gone unnoticed: skill 8000 takes its name from entry 4200.

**The `dummy8` trap is in this row too.** `reserve2` is a `dummy8` with a bit
size, so it pads the byte the four grey-out flags share rather than taking one
of its own. Give it a byte and everything from `textId` onward reads one late.

Checked against the installation: 304 distinct skills across the weapons, named
in the player's own language, with costs that read like the game's — Пинок free,
Ливень Радана 42 FP on R1, Топот 8/8/10 across three buttons.

## What things drop

`NpcParam +0x30` is `itemLotId_enemy`, four bytes past the `getSoul` already
established at 0x2c. It points at `ItemLotParam_enemy`, whose rows are 184 bytes
of flat arrays — eight of everything:

```text
+0x00  s32[8]  lotItemId       0 is an empty slot
+0x20  s32[8]  lotItemCategory  which table the id is in
+0x40  u16[8]  lotItemBasePoint a weight, NOT a percentage
+0x8a  u8[8]   lotItemNum       how many at once
```

The odds are a slot's weight over the row's total. The first slot is almost
always empty with a large weight — that is the "nothing happens" outcome, and
leaving it out of the total turns a one-in-thirty into a certainty.

**The categories were derived, not recalled.** Every (category, id) pair in the
table was asked of all five equipment tables, because a single lookup proves
nothing when the id spaces overlap:

```text
category 1  3974 ids  Goods 99%   (Weapon 9%, Protector 33% — coincidence)
category 2   295 ids  Weapon 100%, everything else 0%
category 3    23 ids  Protector 95%  (Weapon 39% — coincidence)
category 4    19 ids  Accessory 100% AND Goods 100%
category 5     1 id   Gem 100% AND Goods 100%
```

The last two are each a single id that two tables both have, so the table could
not settle them and the game's own text did: id 6070 reads "Sacrificial Twig" as
a talisman, id 20900 reads "Ash of War: Gravitas" as a gem, and the alternative
readings are unrelated goods. The structure agrees — distinct categories exist
to name distinct tables, and goods was already category 1.

**A real id at zero weight means it does not drop.** A total conversion leaves
plenty: the empty outcome keeps its 960 and the item is zeroed. Listing it at
0% reads as though it might, so those slots are left out.

**The useful question is not the obvious one.** Only **2 of 537** named things
in the world drop anything at all — a boss's reward is scripted rather than
rolled. All 3,974 drops belong to the nameless soldiers and beasts, so what is
worth answering is what can be got on a map, folded across everything standing
there. Every one of the 3,974 resolves to a name in the player's own language.

**And the obvious ordering is the wrong one.** Sorted by the best single roll,
"what drops most often here" came back as three things at 100% off one creature
each — leaving out a feather at 62% off thirty-nine, which a player sees twenty
times as often. What answers the question is every source's own odds added up
and multiplied by how many it gives at once: 23.4 poison flowers per clear of
an overworld tile, against 0.8 dragonfly heads. Both figures are reported, and
the prompt says which is which, because a per-clear count read as a percentage
is its own kind of wrong.

## What hurts a thing

`NpcParam +0x1a4`, eight consecutive floats: physical, slash, strike, pierce,
magic, fire, lightning, holy. A rate is what gets THROUGH — 0.6 means it takes
six tenths of that kind, 1.4 means half again as much, 0 means nothing at all —
the same convention the armour reader was already verified against.

**Found by fingerprint, not by counting.** Walking three hundred fields down a
definition list is what produced two wrong offsets already. Instead: a second
reader gave one row's eight values — 34702468 reads 1.1, 1.1, 0.9, 1.1, 1.0,
0.6, 1.2, 1.4 — and exactly one place in the row holds those floats in that
order. The same row's `hp` 247 and `getSoul` 2044 agreed with the offsets
already established, which is what made it safe to trust the rest of the row.

**A shape check would not have been enough, and nearly wasn't.** The first
probe looked for eight plausible multipliers and found `partsDamageRate1..8`
instead, which is eight 1.0s followed by a 1.5. And the float immediately
before the real ones — `toughnessRecoverCorrection` — is just as plausible a
rate, so a four-byte shift passes every sanity check there is. The test pins
the fingerprint for that reason; shifting the offset makes it read 40 where the
other reader says 110.

**These are the creature's own, unlike the health.** 277 distinct patterns
across 7,631 rows, the commonest covering under a tenth. That is the check that
had to be made before reporting any of it: every human character shares one
`hp` value, and that number had to stop being quoted.

Only the kinds that differ from the ordinary amount are reported. A list of
eight hundreds is not an answer, and a reader would have to hunt the two that
matter inside it.

## How it could have been done

1. **From memory.** Abandoned. The accessors thunk into the anti-tamper segment,
   and it would have needed the game running for something the file gives freely.
2. **Parse `regulation.bin`.** This is what was built.
3. **Export a snapshot.** Quickest and worst: goes stale when the mod updates,
   and SmithBox's bulk row listing returns names only, so it would be one call
   per row across 4,745 weapons.
