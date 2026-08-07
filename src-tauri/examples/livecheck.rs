//! Calls exactly what the launcher calls, so a difference between this and the
//! raw probe is a bug in the launcher's own path rather than in the offsets.

use roundtable_lib::games::Game;

fn main() {
    let started = std::time::Instant::now();
    match roundtable_lib::live::read(Game::EldenRing) {
        Some(live) => {
            println!("{} — level {}, {} runes", live.name, live.level, live.runes);
            println!("  {}/{} HP", live.hp, live.hp_max);
            match &live.place {
                Some(place) => println!(
                    "  {} ({}) at {:.0}, {:.0}, {:.0}",
                    place.name.as_deref().unwrap_or("unnamed"),
                    place.map,
                    place.x,
                    place.y,
                    place.z
                ),
                None => println!("  no place"),
            }
        }
        None => println!("read returned nothing"),
    }
    println!("took {:?}", started.elapsed());
}
