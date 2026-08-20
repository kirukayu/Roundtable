//! Reads a weapon out of the installed regulation, mod and all.
//!
//! `cargo run --example weapon` — no game needed, only the files on disk.

use roundtable_lib::formats::regulation::Regulation;

/// Offsets inside a row of `EquipParamWeapon`, worked out from the field
/// definitions and then checked against a weapon whose numbers are known from
/// three other places. See `assets/param-layout.md`.
mod at {
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
    pub const NEEDS_STRENGTH: usize = 0x0f2;
    pub const NEEDS_DEXTERITY: usize = 0x0f3;
    pub const NEEDS_INTELLIGENCE: usize = 0x0f4;
    pub const NEEDS_FAITH: usize = 0x0f5;
    pub const NEEDS_ARCANE: usize = 0x195;
    pub const REGAIN_HP: usize = 0x242;
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(first) = args.next() else {
        eprintln!(
            "usage: cargo run --example weapon -- <regulation.bin> [another one] [id]\n\
             \n\
             Point it at the regulation the game actually loads — a mod's, and the \
             base game's beside it, to see what the mod changed."
        );
        return;
    };

    let mut paths = vec![first];
    let mut wanted: i64 = 1_040_000;
    for arg in args {
        match arg.parse::<i64>() {
            Ok(id) => wanted = id,
            Err(_) => paths.push(arg),
        }
    }

    for (index, path) in paths.iter().enumerate() {
        let label = if index == 0 { "the first" } else { "the next" };
        println!("\n=== {label}");
        let started = std::time::Instant::now();
        let regulation = match Regulation::open(std::path::Path::new(path)) {
            Ok(regulation) => regulation,
            Err(error) => {
                println!("  could not read it: {error}");
                continue;
            }
        };
        println!(
            "  {} tables in {:?}",
            regulation.names().count(),
            started.elapsed()
        );

        let Some(weapons) = regulation.table("EquipParamWeapon") else {
            println!("  no weapon table");
            continue;
        };
        println!("  {} weapons", weapons.len());

        if !weapons.has(wanted) {
            println!("  {wanted} is not one of them");
            continue;
        }

        let f = |at| weapons.f32(wanted, at).unwrap_or(f32::NAN);
        let u = |at| weapons.u16(wanted, at).unwrap_or(0);
        let b = |at| weapons.u8(wanted, at).unwrap_or(0);

        println!("  weapon {wanted}, weight {:.1}", f(at::WEIGHT));
        println!(
            "    damage    physical {}  magic {}  fire {}  lightning {}  holy {}",
            u(at::PHYSICAL),
            u(at::MAGIC),
            u(at::FIRE),
            u(at::LIGHTNING),
            u(at::HOLY)
        );
        println!(
            "    scaling   str {:.0}  dex {:.0}  int {:.0}  fth {:.0}  arc {:.0}",
            f(at::SCALE_STRENGTH),
            f(at::SCALE_DEXTERITY),
            f(at::SCALE_INTELLIGENCE),
            f(at::SCALE_FAITH),
            f(at::SCALE_ARCANE)
        );
        println!(
            "    requires  str {}  dex {}  int {}  fth {}  arc {}",
            b(at::NEEDS_STRENGTH),
            b(at::NEEDS_DEXTERITY),
            b(at::NEEDS_INTELLIGENCE),
            b(at::NEEDS_FAITH),
            b(at::NEEDS_ARCANE)
        );
        println!("    heals on hit {}", u(at::REGAIN_HP));
    }
}
