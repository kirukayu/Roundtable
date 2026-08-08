//! Asks the running game about something by name, the way the assistant does.
//!
//! `cargo run --example lookup -- "Blood Cleric"`

use roundtable_lib::games::Game;

fn main() {
    let query: Vec<String> = std::env::args().skip(1).collect();
    if query.is_empty() {
        println!("give it something to look for");
        return;
    }

    for word in query {
        println!("=== {word:?}");
        match roundtable_lib::text::look_up(Game::EldenRing, &word, 8) {
            None => println!("   the game is not open"),
            Some(found) if found.is_empty() => println!("   nothing"),
            Some(found) => {
                for hit in found {
                    println!("   [{}] {} ({})", hit.kind.what(), hit.name, hit.id);
                    if let Some(effect) = hit.effect {
                        println!("        does: {effect}");
                    }
                    if let Some(caption) = hit.caption {
                        let short: String = caption.chars().take(150).collect();
                        println!("        {short}");
                    }
                }
            }
        }
    }
}
