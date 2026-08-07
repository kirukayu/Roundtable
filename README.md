<div align="center">

<img src="docs/media/mark.png" width="80" alt="">

# Roundtable

**Mod, co-op and save management for ELDEN RING and the rest of FromSoftware's catalogue.**

Windows · no account · no telemetry · MIT

[![Release](https://img.shields.io/github/v/release/kirukayu/Roundtable?style=flat-square&labelColor=0d0d0d&color=d6d6d6)](../../releases/latest)
[![License](https://img.shields.io/badge/license-MIT-d6d6d6?style=flat-square&labelColor=0d0d0d)](LICENSE)

<img src="docs/media/demo.gif" width="880" alt="The Roundtable interface">

</div>

---

Three things that should be simple are not.

**Running a big overhaul with Seamless Co-op.** Roundtable writes `ersc.dll` into
whichever loader you use, as part of the same profile as the overhaul.

**Launching a modded copy that did not come from Steam.** It detects the emulator
files, passes `--skip-steam-init` and writes `steam_appid.txt`. That is the fix for
*trying to find steam*.

**Moving a character between two installs with different account ids.** The id
lives inside the `.sl2` container, so a plain file copy fails. Roundtable rewrites
every occurrence of it and recomputes the MD5 checksums the game verifies on load.

## Install

Take the installer from [Releases](../../releases/latest). It installs for the
current user, so Windows raises no administrator prompt.

The app opens a small window, starts a server on `127.0.0.1`, and hands the address
to your browser. Keep the window open: a browser cannot show a folder picker and
the desktop side can. The port comes from the OS, every request carries a session
key minted at startup, and anything without it gets a 401.

## What it does

**Loaders.** Finds installs through the Steam registry, `libraryfolders.vdf` and the
usual standalone paths. Detects ModEngine 2 and me3 anywhere, including a copy that
a mod bundles with itself, and prefers me3 when both exist.

**Mods.** They live in Roundtable's library, never in the game folder. Reads `zip`
and `7z` as downloaded and finds the layer holding the assets. Conflict detection
names every shared file and its winner. When two mods both ship `regulation.bin` it
says so, because only the first one takes effect.

**Co-op.** Every `ersc_settings.ini` option with its documentation, written in place
with comments intact. Password generator on a keystroke. It refuses a save extension
of `sl2`, the one setting that makes co-op overwrite a solo character.

**Saves.** Character transfer, `.sl2` ↔ `.co2` conversion, snapshots before anything
that writes, duplicate detection across folders.

**Tools.** Shader cache clearing for NVIDIA, AMD, Intel and DirectX, with paths
checked against a whitelist. Anti-cheat toggle that keeps the original file.

## Catalogue

| Title | Year | Support |
| --- | --- | --- |
| ELDEN RING | 2022 | Mods, Seamless Co-op, saves, anti-cheat |
| ELDEN RING NIGHTREIGN | 2025 | Mods and saves |
| ARMORED CORE VI | 2023 | Mods and saves |
| Sekiro | 2019 | Mods and saves |
| DARK SOULS III | 2016 | Mods and saves |
| DARK SOULS II | 2014 | Saves and system tools |
| DARK SOULS: Remastered | 2011 | Saves and system tools |
| Bloodborne · Demon's Souls | | Listed only, PlayStation exclusive |

## Safety

Snapshots precede every save write, and the write itself is atomic. Junction removal
refuses a real directory. Cache clearing refuses paths outside a known cache.
Archive extraction drops entries that escape the destination.

```
cargo test    145 passed
```

## Building

Rust 1.82+, Node 20+, MSVC build tools.

```powershell
npm install
npm run app          # development
npm run app:build    # installer
```

`cargo build` alone gives you a blank window: Tauri embeds the frontend during
`tauri build`.

## Layout

```
src/                  the interface, served to your browser
src-tauri/src/
  server.rs           the loopback server and its session key
  dialog.rs           native folder and file pickers
  game.rs             install discovery and classification
  steam.rs            registry, VDF parsing, local accounts
  loader.rs           ModEngine 2 and me3 configuration
  launch.rs           the launch planner
  coop.rs             Seamless Co-op settings
  saves.rs            discovery, backup, transfer, conversion
  formats/save.rs     the .sl2 container
  mods.rs             library, profiles, conflicts, junctions
  eac.rs              anti-cheat toggle
  sys.rs              shader caches and system reporting
```

## Credits

`ModEngine2` and `me3` define the loader configs. `EldenRingSaveCopier` and
`ER-Save-Editor` documented the save container. Seamless Co-op is by LukeYui.

The Russian text for The Convergence is
[ConvergenceER RU Translation](https://www.nexusmods.com/eldenring/mods/4697) by
S1RBI, redistributed under its upload permission and installed on request.

Cover art belongs to FromSoftware and Bandai Namco, shown for identification.
