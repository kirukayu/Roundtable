//! Prints every save the launcher can see and the characters it reads out of
//! each, so "the level is wrong" can be pinned on a file rather than guessed at.
//!
//! `cargo run --example slots`

use roundtable_lib::games::Game;
use roundtable_lib::saves;

fn main() {
    let game = Game::EldenRing;

    let folders = saves::discover(game, None);
    if folders.is_empty() {
        println!("no save folders found");
        return;
    }

    for folder in &folders {
        println!("\n{}", folder.path.display());
        println!(
            "  id {:?}  account {:?}  cracked {}",
            folder.folder_id, folder.account_name, folder.likely_cracked
        );

        for entry in &folder.entries {
            println!(
                "\n  {}  [{:?}]  {:.1} MB  modified {}",
                entry.file_name,
                entry.flavour,
                entry.size_bytes as f64 / 1e6,
                entry.modified.as_deref().unwrap_or("?")
            );

            match saves::inspect(&entry.path) {
                Ok(summary) => {
                    let active: Vec<_> = summary.slots.iter().filter(|s| s.active).collect();
                    if active.is_empty() {
                        println!("      no active slots");
                    }
                    for slot in active {
                        println!(
                            "      slot {}: {:?}  level {}  played {}h",
                            slot.index,
                            slot.name,
                            slot.level,
                            slot.seconds_played / 3600
                        );
                    }
                }
                Err(error) => println!("      could not parse: {error}"),
            }
        }
    }

    // What the launcher itself would hand the assistant.
    let newest = folders
        .iter()
        .flat_map(|folder| folder.entries.iter())
        .filter(|entry| entry.flavour != saves::SaveFlavour::GameBackup)
        .max_by(|a, b| a.modified.cmp(&b.modified));
    println!(
        "\nthe launcher would read: {}",
        newest.map_or("nothing".to_string(), |e| e.file_name.clone())
    );

    // And what the running game says, for comparison.
    match roundtable_lib::live::read(game) {
        Some(live) => println!(
            "the running game says: {:?}  level {}  runes {}",
            live.name, live.level, live.runes
        ),
        None => println!("the game is not running (or could not be read)"),
    }
}
