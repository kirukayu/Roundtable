# Map markers

What the player pins on the world map, and how to read and write it without
touching the running game.

## Where they live

`CSMenuMarkersSaveData`. In memory the object keeps

| offset | meaning |
| ------ | ------- |
| `+0x08` | pointer to the array |
| `+0x10` | capacity |
| `+0x40` | how many are in use |
| `+0x48` | the counter that hands out ids |

and in the save only the array survives — **110 records of 16 bytes**.

```text
i32 id      the counter's value, or negative for a slot nobody is using
f32 x       map coordinates, not world ones
f32 y
u8  icon    which pin; 0 is the plain one
u8  one     always 1, in used and free records alike
u16 --      never seen as anything but zero
```

## Finding the array

The two slots of one save held their arrays at **different offsets**
(`slot0+0x19a91`, `slot1+0x19cdc`), so there is no constant to hardcode. It is
found by the sixteen bytes immediately in front of it, which occur exactly once
per character:

```text
00 0b 00 00  fb 0b 00 00  fc 0b 00 00  fd 0b 00 00
```

and confirmed by the word just past record 110, which is always `-2`. A slot
that fails the second test is not an array, however well the anchor matched.

## What the game does

`sub_140818FC0` adds one. It walks the array sixteen bytes at a time for the
first `id` that is negative, writes the record there, and gives it `++counter`.
With no free slot it asserts at line 200 of `CSMenuMarkersSaveData.cpp`.

`sub_140819070` removes one, writing back exactly

```text
ff ff ff ff  00 00 00 00  00 00 00 00  00 01 00 00
```

which is the free record this launcher writes too, byte for byte.

The counter is nowhere in the save — not within four kilobytes of the array, and
what follows the array is filler. The game therefore rebuilds it on load, so a
marker written here with `max(id) + 1` is the same number the game would have
chosen, and the next pin the player plants follows it rather than colliding.

## Verified

Read against the player's own save, five markers in slot 1:

| id | x | y |
| -- | - | - |
| 94 | 4318.86 | 8166.39 |
| 99 | 2114.86 | 5451.24 |
| 100 | 2352.59 | 5569.01 |
| 101 | 1865.75 | 5344.60 |
| 93 | 4354.01 | 8199.27 |

`markers::place` then `markers::erase` leaves the slot byte for byte as found,
and `recompute_checksums` keeps the file loadable. There is a test that does
this against every save on the machine.

## World to map

```text
map_x = global_x − 7680          30 grid squares
map_y = 16640 − global_z         65 grid squares, and the axis reversed
global = grid square * 256 + the position within it
```

One map unit is one world unit. The map's corner is the corner of grid square
(30, 65), and its y grows southward where the world's z grows north.

**How it was solved**, since an earlier fit from two pins the player had planted
gave `1.019·gx − 7203` and `−0.948·gz + 16245` — wrong by two to five per cent,
with two points fitting two parameters and nothing left over to check them.

Three tables, all in `regulation.bin`:

- `WorldMapPieceParam` — 34 rows, one per map fragment, each an
  `openTravelAreaLeft/Right/Top/Bottom` rectangle **in marker coordinates**.
- `WorldMapPointParam` — every named point on the map, in world coordinates.
- `WorldMapPlaceNameParam` — the pairing. Each row carries both a
  `worldMapPieceId` and a world position, so that place must land inside *that*
  rectangle rather than merely somewhere on the map.

Four of those pairs are in the overworld. Written as `map_x = a·gx + b` and
`map_y = −a·gz + c` each pair is four linear inequalities, and the three
unknowns come out of the intersection: `a` between 0.8955 and 1.0990, which
contains exactly one round answer.

Then the checks. A scale of one with the corner at (30, 65) satisfies all four
pairs and places **383 of 383** other map points inside the map — and none of
those 383 were used to find it. Moving the corner one square either way fails
the pairs outright. `markers.rs` keeps the four pairs as a fixture and tests
both facts, so a wrong corner cannot pass unnoticed.

## Names, and the player who has no mod

`WorldMapPointParam` holds the positions and up to eight text ids each; the
names behind those ids come from the message archives. A total conversion ships
those loose, so `formats::fmg` reads them straight off the disk.

A clean installation does not. Its text is inside `Data0.bdt` and its fellows,
eleven gigabytes apiece, which nothing here opens — while `regulation.bin` sits
out in the open, which is why the weapon figures work for everybody and the
names did not.

So `places::everywhere` asks in three places, in this order:

1. the loose message archives, which is the copy the game will load;
2. a table written down last time, kept per installation and language beside
   the launcher's settings;
3. the running game, through `text::every_name` — and then written down, so the
   next question does not need the game open.

Markers can only be written with the game shut, and on a clean install the names
can only be read with it open, so without the third step and the writing down
the two never meet. Checked on this machine with the mod folder taken out of the
picture: 229 places read out of the process, Church of Elleh at the same
(3031, 7345) it has under the mod, and 85 KB kept for afterwards.

**A trap worth remembering.** Oodle is found once and the answer was cached with
a `OnceLock` — including the failure. Anything that asked for a decompression
before a game had been registered set that answer to "no library", and every
archive stayed unreadable for the rest of the run. It surfaced as an English
question answered with Russian names, because the file read failed and the
fallback asked the running game instead. Only success is remembered now.

## What is left

Reading the packed archives themselves. `Data0.bhd`'s header is RSA-encrypted;
the keys are published and the format is documented. That would need no prior
launch at all, and would give the launcher every file the game ships.
