<div align="center">

<img src="docs/media/mark.png" width="80" alt="">

# roundtable

mod, co-op and save manager for elden ring and the other fromsoftware games.
runs in your browser off a local server.

windows · no account · no telemetry · mit

[![release](https://img.shields.io/github/v/release/kirukayu/Roundtable?style=flat-square&labelColor=0d0d0d&color=d6d6d6)](../../releases/latest)
[![license](https://img.shields.io/badge/license-mit-d6d6d6?style=flat-square&labelColor=0d0d0d)](LICENSE)

<img src="docs/media/demo.gif" width="880" alt="the roundtable interface">

</div>

---

## what it does

- **mods manager** - manage and install your mods just by dropping them into the window
- **seamless co-op** - full seamless co-op support, and just like with mods you can manage all your co-op settings easily
- **saves** - manage your game saves, check a save's info, back it up and more
- **AI** - roundtable has an in-game ai assistant (shift + f1). ask it anything about elden ring and it'll help with builds, items and a lot more
- **graphics** - set up dlss, fsr, nvidia image scaling and xess upscaling, plus frame generation, straight from the launcher

## games

| title | year | support |
| --- | --- | --- |
| elden ring | 2022 | mods, seamless co-op, saves, anti-cheat |
| elden ring nightreign | 2025 | mods and saves |
| armored core vi | 2023 | mods and saves |
| sekiro | 2019 | mods and saves |
| dark souls iii | 2016 | mods and saves |
| dark souls ii | 2014 | saves and system tools |
| dark souls: remastered | 2011 | saves and system tools |
| bloodborne · demon's souls | | listed only, playstation exclusive |

## install

grab the installer from [releases](../../releases/latest). installs per-user, no admin
prompt. leave the window open, the browser side needs it for the folder pickers.

## build

rust 1.82+, node 20+, msvc build tools.

```powershell
npm install
npm run app          # development
npm run app:build    # installer
```

## credits

`modengine2` and `me3` for the loader configs, `eldenringsavecopier` and `er-save-editor`
for the save format, seamless co-op by lukeyui. russian convergence text by s1rbi
([nexus](https://www.nexusmods.com/eldenring/mods/4697)). cover art is fromsoftware's.

half of this is vibecoded, i didn't know much typescript going in.
