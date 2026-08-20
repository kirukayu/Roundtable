//! Pins a named place on a save, end to end, for checking the whole chain on a
//! copy rather than on somebody's own game.
//!
//!     cargo run --example pin -- <game dir> <mod dir> <language> <save> <slot> <place>

use roundtable_lib::{markers, places, saves};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, installed, language, save, slot, place] = args.as_slice() else {
        eprintln!("usage: pin <game> <mod> <language> <save> <slot> <place>");
        return;
    };
    let slot: usize = slot.parse().unwrap_or(0);

    let found = places::everywhere(places::Where {
        game: roundtable_lib::games::Game::EldenRing,
        game_dir: std::path::Path::new(game),
        mod_dir: Some(std::path::Path::new(installed)),
        language,
        keep_in: None,
    });
    let Some(want) = places::find(&found, place) else {
        println!("no place called {place:?} among {}", found.len());
        return;
    };
    println!("{} is at ({:.2}, {:.2})", want.name, want.map_x, want.map_y);

    let scratch = std::env::temp_dir().join("roundtable-pin");
    let save = std::path::Path::new(save);
    match saves::add_marker(
        &scratch,
        roundtable_lib::games::Game::EldenRing,
        save,
        slot,
        want.map_x,
        want.map_y,
        0,
    ) {
        Ok(marker) => println!("wrote marker {}", marker.id),
        Err(problem) => {
            println!("failed: {problem}");
            return;
        }
    }

    match saves::read_markers(save) {
        Ok(slots) => {
            for one in slots.iter().filter(|s| !s.name.trim().is_empty()) {
                println!("\nslot {} — {}", one.slot, one.name);
                for marker in &one.markers {
                    let (gx, gz) = markers::to_global(marker.x, marker.y);
                    println!(
                        "  id={:<4} ({:.2}, {:.2})   world ({:.0}, {:.0})",
                        marker.id, marker.x, marker.y, gx, gz
                    );
                }
            }
        }
        Err(problem) => println!("reading back failed: {problem}"),
    }
}
