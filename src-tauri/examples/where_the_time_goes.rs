//! Times each piece of the work done before a question can even be sent.
//!
//! Run against a real installation:
//!   cargo run --release --example where_the_time_goes

use std::time::Instant;

fn timed<T>(what: &str, run: impl FnOnce() -> T) -> T {
    let began = Instant::now();
    let out = run();
    println!("{:>8} ms  {what}", began.elapsed().as_millis());
    out
}

fn main() {
    let game = roundtable_lib::games::Game::EldenRing;
    let app_data = match dirs::config_dir() {
        Some(dir) => dir.join("app.roundtable.launcher"),
        None => {
            println!("no config directory; nothing to measure against");
            return;
        }
    };
    let settings = roundtable_lib::settings::Settings::load(&app_data);
    let Some(saved) = settings
        .installations
        .iter()
        .find(|one| one.game == game && one.is_default)
        .or_else(|| settings.installations.iter().find(|one| one.game == game))
    else {
        println!("no installation remembered; nothing to measure against");
        return;
    };
    let Ok(install) = roundtable_lib::game::Installation::probe(game, &saved.root) else {
        println!("that installation could not be probed");
        return;
    };
    let game_dir = install.game_dir.clone();
    let mod_dir = settings
        .editions
        .values()
        .map(|root| root.join("mod"))
        .find(|dir| dir.join("regulation.bin").is_file());

    println!("game  {}", game_dir.display());
    match &mod_dir {
        Some(dir) => println!("mod   {}", dir.display()),
        None => println!("mod   none"),
    }
    println!();

    // Twice each: the first pays for whatever is cached, the second is what a
    // question actually costs once the launcher has been running a while.
    for pass in 1..=2 {
        println!("--- pass {pass}");
        timed("active install", || {
            roundtable_lib::game::Installation::probe(game, &saved.root).ok()
        });
        timed("language::status", || roundtable_lib::language::status(&game_dir));
        timed("erss::settings", || roundtable_lib::erss::settings(&game_dir));
        timed("perf::status", || {
            roundtable_lib::perf::status(game, std::slice::from_ref(&game_dir), false)
        });
        let folders = timed("saves::discover", || roundtable_lib::saves::discover(game, None));
        let newest = folders
            .iter()
            .flat_map(|folder| folder.entries.iter())
            .filter(|entry| entry.flavour != roundtable_lib::saves::SaveFlavour::GameBackup)
            .max_by(|a, b| a.modified.cmp(&b.modified))
            .map(|entry| entry.path.clone());
        if let Some(path) = &newest {
            timed("saves::inspect", || roundtable_lib::saves::inspect(path).ok());
        }
        timed("mods::list_mods", || roundtable_lib::mods::list_mods(&app_data, game));
        timed("mods::list_profiles", || {
            roundtable_lib::mods::list_profiles(&app_data, game)
        });
        timed("regulation::installed", || {
            roundtable_lib::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())
        });
        timed("live::read (the running game)", || {
            roundtable_lib::live::read(game).is_some()
        });
        println!();
    }
}
