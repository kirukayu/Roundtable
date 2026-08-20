//! The markers in a real save, for checking the reader against the file the
//! player actually plays.
//!
//!     cargo run --example markers -- "%APPDATA%\EldenRing\<id>\ER0000.sl2"

use roundtable_lib::formats::save::{SaveFile, SLOT_COUNT};
use roundtable_lib::markers;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run --example markers -- <save file>");
        return;
    };

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(problem) => {
            eprintln!("{path}: {problem}");
            return;
        }
    };
    let save = match SaveFile::from_bytes(bytes) {
        Ok(save) => save,
        Err(problem) => {
            eprintln!("{path}: {problem}");
            return;
        }
    };

    println!("checksums {}", if save.verify_checksums() { "good" } else { "BAD" });

    for index in 0..SLOT_COUNT {
        let Ok(slot) = save.slot_data(index) else {
            continue;
        };
        let found = markers::read(slot);
        let active = save.is_slot_active(index).unwrap_or(false);
        let name = save
            .slot_summary(index)
            .map(|summary| summary.name)
            .unwrap_or_default();

        if !active && found.is_empty() {
            continue;
        }
        println!("\nslot {index}  {name}  ({} markers)", found.len());
        for marker in found {
            println!(
                "  id={:<5} x={:<10.2} y={:<10.2} icon={}",
                marker.id, marker.x, marker.y, marker.icon
            );
        }
    }
}
