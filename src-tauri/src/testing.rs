//! Finding a real installation, for the checks that need one.
//!
//! Several checks are only worth anything against the game as it is actually
//! installed — the weapon figures, the message archives, the markers in a save.
//! Those used to carry one machine's paths written into them, which is wrong
//! twice over: it puts somebody's folder names into a public repository, and on
//! every other machine the check quietly passes without having checked
//! anything.
//!
//! So it asks the launcher instead. The launcher already knows where the game
//! is — that is most of what its settings are for — and when it does not know,
//! the check skips and says nothing.
//!
//! `ROUNDTABLE_TEST_GAME` and `ROUNDTABLE_TEST_MOD` override both, for running
//! the same checks against an installation the launcher has never seen.

#![cfg(test)]

use std::path::PathBuf;

use crate::games::Game;

/// Where the launcher keeps its settings, and the catalogue of names it wrote
/// down the last time the game was open.
pub fn app_data() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("app.roundtable.launcher"))
}

/// The game's own folder — the one with the executable in it — as the launcher
/// has it, or nothing when no game has been located.
pub fn game_dir(game: Game) -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("ROUNDTABLE_TEST_GAME") {
        let dir = PathBuf::from(named);
        return dir.is_dir().then_some(dir);
    }

    let settings = crate::settings::Settings::load(&app_data()?);
    let root = settings
        .installations
        .iter()
        .find(|saved| saved.game == game && saved.is_default)
        .or_else(|| settings.installations.iter().find(|saved| saved.game == game))
        .map(|saved| saved.root.clone())?;

    crate::game::Installation::probe(game, &root)
        .ok()
        .map(|install| install.game_dir)
        .filter(|dir| dir.is_dir())
}

/// A total conversion's folder, when one is installed — the folder holding
/// `regulation.bin`, whichever way that edition was packaged.
pub fn mod_dir(game: Game) -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("ROUNDTABLE_TEST_MOD") {
        let dir = PathBuf::from(named);
        return dir.join("regulation.bin").is_file().then_some(dir);
    }

    let settings = crate::settings::Settings::load(&app_data()?);
    let wanted: Vec<&str> = crate::edition::for_game(game)
        .into_iter()
        .map(|spec| spec.id)
        .collect();

    settings
        .editions
        .iter()
        .filter(|(id, _)| wanted.contains(&id.as_str()))
        .flat_map(|(_, root)| [root.join("mod"), root.clone()])
        .find(|dir| dir.join("regulation.bin").is_file())
}

/// Both at once, for the checks that want the modded game or nothing.
pub fn installed(game: Game) -> Option<(PathBuf, PathBuf)> {
    Some((game_dir(game)?, mod_dir(game)?))
}

/// The regulation the player's game actually loads.
pub fn regulation(game: Game) -> Option<PathBuf> {
    let path = match mod_dir(game) {
        Some(dir) => dir.join("regulation.bin"),
        None => game_dir(game)?.join("regulation.bin"),
    };
    path.is_file().then_some(path)
}
