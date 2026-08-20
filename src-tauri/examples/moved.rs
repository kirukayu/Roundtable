//! Finds the player by watching what changes while they run.
//!
//! Coordinates are three floats that move together and keep moving. Nothing
//! else in the block does, so two samples taken while somebody walks identify
//! them without a table of offsets to be wrong about.
//!
//! `moved.exe <seconds>`

use roundtable_lib::games::Game;
use roundtable_lib::unlock::win::Process;

const WORLD_CHR_MAN: &str = "48 8B 05 ?? ?? ?? ?? 48 85 C0 74 0F 48 39 88";
const SPAN: usize = 0x20000;

fn main() {
    let wait: u64 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(6);

    let Some(pid) = roundtable_lib::unlock::running_pid(Game::EldenRing.executable()) else {
        println!("the game is not running");
        return;
    };
    let Ok(process) = Process::open(pid) else {
        println!("could not open it — run this as administrator");
        return;
    };
    let Ok((base, size)) = process.main_module() else { return };

    let image = process.read(base, size);
    let Some(found) = roundtable_lib::unlock::find_only(&image, &roundtable_lib::unlock::parse(WORLD_CHR_MAN)) else {
        println!("WorldChrMan pattern did not match once");
        return;
    };
    let displacement = i32::from_le_bytes(image[found + 3..found + 7].try_into().unwrap());
    let slot = (base + found + 7).wrapping_add_signed(displacement as isize);
    let world = usize::from_le_bytes(process.read(slot, 8).try_into().unwrap());
    println!("WorldChrMan {world:#x}");

    // Every pointer in the block, and every pointer one hop further, so a
    // player two levels down is still reached.
    let mut targets: Vec<usize> = Vec::new();
    let mut route: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    let block = process.read(world, SPAN);
    for at in (0..block.len() - 8).step_by(8) {
        let p = usize::from_le_bytes(block[at..at + 8].try_into().unwrap());
        if (0x10000..0x7fff_ffff_ffff).contains(&p) {
            targets.push(p);
            route.entry(p).or_insert_with(|| format!("world+{at:#x}"));
        }
    }
    // One hop further: the player's physics usually hangs off the character
    // rather than off the manager, so the coordinates are two pointers deep.
    let mut deeper: Vec<usize> = Vec::new();
    for target in targets.iter().take(2500) {
        let inner = process.read(*target, 0x400);
        for at in (0..inner.len() - 8).step_by(8) {
            let p = usize::from_le_bytes(inner[at..at + 8].try_into().unwrap());
            if (0x10000..0x7fff_ffff_ffff).contains(&p) {
                deeper.push(p);
                let via = route.get(target).cloned().unwrap_or_default();
                route.entry(p).or_insert_with(|| format!("{via} -> +{at:#x}"));
            }
        }
    }
    targets.extend(deeper);
    targets.sort_unstable();
    targets.dedup();
    targets.truncate(60_000);
    println!("{} candidate structures", targets.len());

    let sample = |targets: &[usize]| -> Vec<Vec<u8>> {
        targets.iter().map(|p| process.read(*p, 0x400)).collect()
    };

    println!("walk around for {wait} seconds…");
    let before = sample(&targets);
    std::thread::sleep(std::time::Duration::from_secs(wait));
    let after = sample(&targets);

    // Three consecutive floats that all moved, all sane, and moved together.
    for (index, target) in targets.iter().enumerate() {
        let (a, b) = (&before[index], &after[index]);
        for at in (0..a.len() - 12).step_by(4) {
            let f = |buf: &[u8], o: usize| f32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
            let (x0, y0, z0) = (f(a, at), f(a, at + 4), f(a, at + 8));
            let (x1, y1, z1) = (f(b, at), f(b, at + 4), f(b, at + 8));
            let sane = |v: f32| v.is_finite() && v.abs() > 0.5 && v.abs() < 3000.0;
            if !(sane(x0) && sane(y0) && sane(z0) && sane(x1) && sane(y1) && sane(z1)) {
                continue;
            }
            let moved = (x1 - x0).abs() + (z1 - z0).abs();
            if moved > 2.0 && moved < 400.0 {
                println!(
                    "  +{at:#05x}  ({x0:.0}, {y0:.0}, {z0:.0}) -> ({x1:.0}, {y1:.0}, {z1:.0})   {}",
                    route.get(target).map(String::as_str).unwrap_or("?")
                );
            }
        }
    }
}
