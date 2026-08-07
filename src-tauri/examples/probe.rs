//! Dumps what the running game keeps about the player, so the offsets in
//! `live.rs` are read off the real thing rather than remembered.
//!
//! `cargo run --example probe`

use roundtable_lib::games::Game;

fn main() {
    let Some(pid) = roundtable_lib::unlock::running_pid(Game::EldenRing.executable()) else {
        println!("the game is not running");
        return;
    };
    println!("pid {pid}");

    let process = match roundtable_lib::unlock::win::Process::open(pid) {
        Ok(process) => process,
        Err(error) => {
            println!("could not open it: {error}");
            return;
        }
    };
    let (base, size) = match process.main_module() {
        Ok(pair) => pair,
        Err(error) => {
            println!("no module: {error}");
            return;
        }
    };
    println!("module at {base:#x}, {:.0} MB", size as f64 / 1e6);

    let image = process.read(base, size);

    // Every pattern worth trying, so a version that moved one is obvious.
    let patterns: &[(&str, &str)] = &[
        ("GameDataMan", "48 8B 05 ?? ?? ?? ?? 48 85 C0 74 05 48 8B 40 58 C3 C3"),
        ("WorldChrMan", "48 8B 05 ?? ?? ?? ?? 48 85 C0 74 0F 48 39 88"),
        ("GameMan", "48 8B 05 ?? ?? ?? ?? 80 B8 ?? ?? ?? ?? 00 0F 85"),
    ];

    for (name, pattern) in patterns {
        let parsed = roundtable_lib::unlock::parse(pattern);
        let hits = count(&image, &parsed);
        let Some(at) = roundtable_lib::unlock::find_only(&image, &parsed) else {
            println!("{name}: {hits} matches — no single hit");
            continue;
        };
        let Ok(bytes) = image[at + 3..at + 7].try_into() else {
            continue;
        };
        let displacement = i32::from_le_bytes(bytes);
        let Some(slot) = (base + at + 7).checked_add_signed(displacement as isize) else {
            continue;
        };
        let value = read_ptr(&process, slot);
        println!("{name}: pattern at {:#x}, slot {slot:#x} -> {value:#x}", base + at);

        if *name == "GameDataMan" && value != 0 {
            dump_player(&process, value);
        }
        if *name == "WorldChrMan" && value != 0 {
            dump_world(&process, value);
        }
    }
}

fn count(haystack: &[u8], pattern: &[Option<u8>]) -> usize {
    let mut found = 0;
    let mut at = 0;
    while at + pattern.len() <= haystack.len() {
        if pattern
            .iter()
            .zip(&haystack[at..])
            .all(|(want, got)| want.is_none_or(|byte| byte == *got))
        {
            found += 1;
        }
        at += 1;
    }
    found
}

fn read_ptr(process: &roundtable_lib::unlock::win::Process, at: usize) -> usize {
    let bytes = process.read(at, 8);
    bytes
        .try_into()
        .map(usize::from_le_bytes)
        .unwrap_or(0)
}

/// The block behind GameDataMan, printed so the fields can be picked out.
fn dump_player(process: &roundtable_lib::unlock::win::Process, manager: usize) {
    let data = read_ptr(process, manager + 0x08);
    println!("\n  PlayerGameData at {data:#x}");
    if data < 0x10000 {
        println!("  (not a pointer)");
        return;
    }

    let block = process.read(data, 0x300);

    // Any UTF-16 run that reads like a name.
    for at in (0..block.len() - 8).step_by(2) {
        let mut chars = Vec::new();
        let mut cursor = at;
        while cursor + 1 < block.len() {
            let unit = u16::from_le_bytes([block[cursor], block[cursor + 1]]);
            if unit == 0 {
                break;
            }
            if !(0x20..0x2000).contains(&unit) {
                chars.clear();
                break;
            }
            chars.push(unit);
            cursor += 2;
        }
        if chars.len() >= 3 {
            if let Ok(text) = String::from_utf16(&chars) {
                println!("  name-like at +{at:#05x}: {text:?}");
            }
        }
    }

    // Equipment sits past the character block as a run of item ids, which are
    // large and distinctive where everything around them is small.
    let wide = process.read(data, 0x800);
    println!("\n  large ids past +0x100:");
    for at in (0x100..wide.len() - 4).step_by(4) {
        let value = u32::from_le_bytes(wide[at..at + 4].try_into().unwrap());
        if (1_000_000..90_000_000).contains(&value) {
            println!("    +{at:#05x}  {value}");
        }
    }

    println!("\n  words, four per line:");
    for row in (0..0x100).step_by(16) {
        let words: Vec<String> = (0..4)
            .map(|i| {
                let at = row + i * 4;
                let value = u32::from_le_bytes([
                    block[at],
                    block[at + 1],
                    block[at + 2],
                    block[at + 3],
                ]);
                format!("{value:>11}")
            })
            .collect();
        println!("  +{row:#05x}  {}", words.join(" "));
    }
}

/// Hunting the player's own character instance inside WorldChrMan.
///
/// Its offset moves between versions, so rather than trust one, every pointer
/// in the first part of the block is followed and judged: the player's
/// character has a physics module a fixed distance in, and that module holds
/// three floats that look like somewhere in a world and a map id that looks
/// like a map id.
fn dump_world(process: &roundtable_lib::unlock::win::Process, world: usize) {
    println!("
  WorldChrMan at {world:#x} — looking for the player");
    let block = process.read(world, 0x20000);

    for at in (0..block.len() - 8).step_by(8) {
        let candidate = u64::from_le_bytes(block[at..at + 8].try_into().unwrap()) as usize;
        if candidate < 0x10000 || candidate > 0x7fff_ffff_ffff {
            continue;
        }
        // ChrIns -> +0x190 physics module -> coordinates.
        let physics_ptr = process.read(candidate + 0x190, 8);
        let physics = u64::from_le_bytes(physics_ptr.try_into().unwrap()) as usize;
        if physics < 0x10000 || physics > 0x7fff_ffff_ffff {
            continue;
        }
        let m = process.read(physics, 0x90);
        let f = |o: usize| f32::from_le_bytes(m[o..o + 4].try_into().unwrap());
        let (x, y, z) = (f(0x70), f(0x74), f(0x78));
        let sane = |v: f32| v.is_finite() && v.abs() > 0.01 && v.abs() < 5000.0;
        if sane(x) && sane(y) && sane(z) {
            let map = u32::from_le_bytes(m[0x6c..0x70].try_into().unwrap());
            println!(
                "    +{at:#07x}: chr {candidate:#x}  map {:02}_{:02}_{:02}_{:02}  x {x:.1} y {y:.1} z {z:.1}",
                (map >> 24) & 0xff, (map >> 16) & 0xff, (map >> 8) & 0xff, map & 0xff
            );
        }
    }
}
