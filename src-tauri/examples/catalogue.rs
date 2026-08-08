//! Everything the loaded game knows how to name.
//!
//! `cargo run --example catalogue` — the game has to be open with a save loaded.

use roundtable_lib::games::Game;
use roundtable_lib::text::{Kind, Text};

fn main() {
    let Some(pid) = roundtable_lib::unlock::running_pid(Game::EldenRing.executable()) else {
        println!("the game is not running");
        return;
    };
    let Ok(process) = roundtable_lib::unlock::win::Process::open(pid) else {
        println!("could not open the process");
        return;
    };
    let Ok((base, size)) = process.main_module() else {
        println!("no module");
        return;
    };
    let image = process.read(base, size);

    let started = std::time::Instant::now();
    let Some(text) = Text::open(&process, &image, base) else {
        println!("no text store — is a save loaded?");
        return;
    };
    println!("opened the text store in {:?}\n", started.elapsed());

    for kind in [Kind::Weapon, Kind::Armour, Kind::Talisman, Kind::Goods] {
        let at = std::time::Instant::now();
        let all = text.index(kind);
        println!("{kind:?}: {} entries in {:?}", all.len(), at.elapsed());
        for (id, name) in all.iter().take(4) {
            println!("    {id:>10}  {name}");
        }
        if all.len() > 4 {
            println!("    …");
            for (id, name) in all.iter().rev().take(2) {
                println!("    {id:>10}  {name}");
            }
        }
    }

    println!("\nthe game's text is in: {}", text.language());

    // Searching by name, which is what the assistant will do.
    for query in ["Редувия", "янтар", "Кинжал"] {
        println!("\nsearch {query:?}:");
        for hit in text.find(query, 4) {
            println!("  [{}] {} ({})", hit.kind.what(), hit.name, hit.id);
            if let Some(effect) = &hit.effect {
                println!("      does: {effect}");
            }
            if let Some(caption) = &hit.caption {
                let short: String = caption.chars().take(120).collect();
                println!("      {short}");
            }
        }
    }
}
