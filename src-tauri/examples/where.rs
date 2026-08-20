//! Where the player is, by the chain The Grand Archives maintain.
//!
//! [[WorldChrMan + 0x10EF8] + 0] + 0x6C0 is a block holding X, Z, Y and the map
//! id — the same one their table reads, which is why this is a lookup rather
//! than a hunt.

use roundtable_lib::games::Game;
use roundtable_lib::unlock::win::Process;

const WORLD_CHR_MAN: &str = "48 8B 05 ?? ?? ?? ?? 48 85 C0 74 0F 48 39 88";

fn main() {
    let Some(pid) = roundtable_lib::unlock::running_pid(Game::EldenRing.executable()) else {
        println!("the game is not running");
        return;
    };
    let Ok(process) = Process::open(pid) else {
        println!("could not open it — needs administrator");
        return;
    };
    let Ok((base, size)) = process.main_module() else { return };

    let image = process.read(base, size);
    let Some(found) =
        roundtable_lib::unlock::find_only(&image, &roundtable_lib::unlock::parse(WORLD_CHR_MAN))
    else {
        println!("WorldChrMan did not match once");
        return;
    };
    let displacement = i32::from_le_bytes(image[found + 3..found + 7].try_into().unwrap());
    let slot = (base + found + 7).wrapping_add_signed(displacement as isize);

    let deref = |at: usize| -> usize {
        usize::from_le_bytes(process.read(at, 8).try_into().unwrap())
    };

    let world = deref(slot);
    println!("WorldChrMan {world:#x}");
    let player = deref(world + 0x10EF8);
    println!("  player     {player:#x}");
    if player < 0x10000 {
        println!("  (nothing there — not loaded into a world?)");
        return;
    }
    let inner = deref(player);
    println!("  inner      {inner:#x}");

    let block = process.read(inner + 0x6C0, 0x20);
    let f = |o: usize| f32::from_le_bytes(block[o..o + 4].try_into().unwrap());
    let map = u32::from_le_bytes(block[0x10..0x14].try_into().unwrap());

    println!(
        "\n  x {:.2}   z {:.2}   y {:.2}",
        f(0x00),
        f(0x04),
        f(0x08)
    );
    println!(
        "  map {:#010x}  = m{:02}_{:02}_{:02}_{:02}",
        map,
        (map >> 24) & 0xff,
        (map >> 16) & 0xff,
        (map >> 8) & 0xff,
        map & 0xff
    );
    println!("\n  raw: {block:02x?}");
}
