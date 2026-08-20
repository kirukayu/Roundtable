//! What the running game says the player is holding, wearing and standing in.
//!
//! `cargo run --example gear` — needs the game open and an elevated shell.

use roundtable_lib::games::Game;

fn main() {
    let Some(live) = roundtable_lib::live::read(Game::EldenRing) else {
        println!("no character — the game is closed, at the title, or unreadable");
        return;
    };

    println!("{} — level {}, {} runes", live.name, live.level, live.runes);
    println!(
        "  {}/{} HP, {}/{} FP, {}/{} stamina",
        live.hp, live.hp_max, live.fp, live.fp_max, live.stamina, live.stamina_max
    );
    let stats: Vec<String> = live
        .stats
        .iter()
        .map(|(what, value)| format!("{what} {value}"))
        .collect();
    println!("  {}", stats.join(", "));

    match &live.place {
        Some(place) => println!(
            "  standing in {} ({}) at {:.0}, {:.0}, {:.0}",
            place.name.as_deref().unwrap_or("somewhere unnamed"),
            place.map,
            place.x,
            place.y,
            place.z
        ),
        None => println!("  no world loaded"),
    }

    match &live.gear {
        Some(gear) => {
            println!("\n  holding:");
            for weapon in &gear.weapons {
                println!("    {weapon}");
            }
            println!("  their table rows:");
            for (name, id) in &gear.weapon_ids {
                println!("    {id:>10}  {name}");
            }
            println!("  wearing:");
            for (slot, piece) in &gear.armour {
                println!("    {slot:<5} {piece}");
            }
            if gear.talismans.is_empty() {
                println!("  no talismans");
            } else {
                println!("  talismans:");
                for talisman in &gear.talismans {
                    println!("    {talisman}");
                }
            }
        }
        None => println!("\n  could not read the equipment"),
    }
}
