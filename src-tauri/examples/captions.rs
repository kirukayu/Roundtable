//! Which text table actually answers for a given id, so the caption tables can
//! be trusted rather than assumed.
//!
//! `cargo run --example captions`

use roundtable_lib::games::Game;
use roundtable_lib::text::Text;

fn main() {
    let Some(pid) = roundtable_lib::unlock::running_pid(Game::EldenRing.executable()) else {
        println!("the game is not running");
        return;
    };
    let process = roundtable_lib::unlock::win::Process::open(pid).expect("open");
    let (base, size) = process.main_module().expect("module");
    let image = process.read(base, size);
    let text = Text::open(&process, &image, base).expect("text store");

    // Every table that has anything to say about these two, so the mapping is
    // read rather than guessed.
    let ids: &[(u32, &str)] = &[
        (1040000, "Reduvia — a dagger"),
        (34160000, "the seal in their left hand"),
        (681000, "the hood they are wearing"),
        (1000, "talisman id 1000"),
    ];

    for (id, what) in ids {
        println!("\n=== {id} ({what})");
        for table in 0..500u32 {
            if let Some(found) = text.get(table, *id) {
                let short: String = found.chars().take(72).collect();
                let short = short.replace('\n', " / ");
                println!("  table {table:>3}: {short}");
            }
        }
    }
}
