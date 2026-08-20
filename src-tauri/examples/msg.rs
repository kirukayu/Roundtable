//! Unwraps one of the game's message archives, to prove Oodle is reachable.
//!
//!     cargo run --example msg -- "<game>\Game" "<mod>\msg\engus\item.msgbnd.dcx"

use roundtable_lib::formats::{dcx, oodle};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(game), Some(file)) = (args.next(), args.next()) else {
        eprintln!("usage: cargo run --example msg -- <game dir> <msgbnd.dcx>");
        return;
    };

    oodle::register(std::path::Path::new(&game));
    match oodle::library_path() {
        Some(path) => println!("oodle: {}", path.display()),
        None => {
            println!("oodle: not found in {game}");
            return;
        }
    }

    let bytes = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(problem) => {
            eprintln!("{file}: {problem}");
            return;
        }
    };
    println!("packed: {} bytes", bytes.len());

    match dcx::expand(&bytes, "message archive") {
        Ok(out) => {
            println!("expanded: {} bytes", out.len());
            println!(
                "starts with: {:?}",
                String::from_utf8_lossy(out.get(..4).unwrap_or_default())
            );
        }
        Err(problem) => println!("failed: {problem}"),
    }

    match roundtable_lib::formats::fmg::archive(std::path::Path::new(&file)) {
        Ok(tables) => {
            let mut names: Vec<&String> = tables.keys().collect();
            names.sort();
            println!("\n{} tables:", names.len());
            for name in names {
                let table = &tables[name];
                let sample = table
                    .iter()
                    .filter(|(_, text)| text.chars().count() > 2)
                    .min_by_key(|(id, _)| **id)
                    .map(|(id, text)| {
                        let short: String = text.chars().take(40).collect();
                        format!("{id} = {short:?}")
                    })
                    .unwrap_or_default();
                println!("  {name:<34} {:>6} strings   {sample}", table.len());
            }
        }
        Err(problem) => println!("tables: {problem}"),
    }
}
