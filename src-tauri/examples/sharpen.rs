//! One weapon at every upgrade level, for checking the curve against the
//! numbers on the player's own screen.
//!
//!     cargo run --example sharpen -- <regulation.bin> 1040000

use roundtable_lib::formats::regulation::Regulation;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(id)) = (args.next(), args.next()) else {
        eprintln!("usage: cargo run --example sharpen -- <regulation.bin> <id>");
        return;
    };
    let base: i64 = id.parse().unwrap_or(0);

    let regulation = match Regulation::open(std::path::Path::new(&path)) {
        Ok(regulation) => regulation,
        Err(problem) => {
            eprintln!("{path}: {problem}");
            return;
        }
    };

    // What the tables actually say, before any of it is interpreted.
    if let (Some(weapons), Some(curve)) = (
        regulation.table("EquipParamWeapon"),
        regulation.table("ReinforceParamWeapon"),
    ) {
        let kind = weapons.u16(base, 0x0da);
        println!("reinforceTypeId = {kind:?}   ({} rows in the curve)", curve.len());
        if let Some(kind) = kind {
            for step in 0..4 {
                let row = i64::from(kind) + step;
                println!(
                    "  row {row}: exists {}  physical {:?}  magic {:?}  fire {:?}  most {:?}",
                    curve.has(row),
                    curve.f32(row, 0x00),
                    curve.f32(row, 0x04),
                    curve.f32(row, 0x08),
                    curve.u8(row, 0x57),
                );
            }
        }
        println!();
    }

    for level in 0..=25 {
        let Some(weapon) = regulation.weapon(base + level) else {
            continue;
        };
        let damage: Vec<String> = weapon
            .damage
            .iter()
            .map(|(kind, value)| format!("{value} {kind}"))
            .collect();
        let scaling: Vec<String> = weapon
            .scaling
            .iter()
            .map(|(what, value)| format!("{} {value:.0}", what.split_whitespace().next().unwrap_or(what)))
            .collect();
        println!(
            "+{:<3} {:<28} {}",
            weapon.level,
            damage.join(", "),
            scaling.join(", ")
        );
    }
}
