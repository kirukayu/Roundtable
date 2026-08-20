//! What the launcher thinks the map's named places are.
//!
//!     cargo run --example places -- <game dir> <mod dir> <language> [search]

use roundtable_lib::places;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game), Some(installed), Some(language)) = (args.next(), args.next(), args.next())
    else {
        eprintln!("usage: cargo run --example places -- <game> <mod> <language> [search]");
        return;
    };
    let wanted = args.next();

    // A single dash for the mod stands for "no mod at all", which is the case
    // worth checking: a clean game has no loose text, so the names have to come
    // out of the running process and be written down.
    let kept = std::env::temp_dir().join("roundtable-places-example");
    let found = places::everywhere(places::Where {
        game: roundtable_lib::games::Game::EldenRing,
        game_dir: std::path::Path::new(&game),
        mod_dir: (installed != "-").then(|| std::path::Path::new(&installed)),
        language: &language,
        keep_in: Some(&kept),
    });
    println!("(anything read from the game is kept in {})", kept.display());
    println!("{} places\n", found.len());

    match wanted {
        Some(wanted) => {
            let lower = wanted.to_lowercase();
            for place in found.iter().filter(|p| p.name.to_lowercase().contains(&lower)) {
                println!("  {:<44} ({:.0}, {:.0})", place.name, place.map_x, place.map_y);
            }
            match places::find(&found, &wanted) {
                Some(best) => println!("\nbest: {} ({:.0}, {:.0})", best.name, best.map_x, best.map_y),
                None => println!("\nno match for {wanted:?}"),
            }
        }
        None => {
            for place in found.iter().take(40) {
                println!("  {:<44} ({:.0}, {:.0})", place.name, place.map_x, place.map_y);
            }
        }
    }
}
