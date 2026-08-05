//! Anti-cheat handling.
//!
//! ELDEN RING ships two executables: `start_protected_game.exe`, which boots Easy
//! Anti-Cheat and then the game, and `eldenring.exe`, the game itself. Every mod
//! loader already starts the second one directly, which is why modded play is
//! offline play.
//!
//! The toggle here goes one step further and makes *Steam's own* Play button skip
//! EAC too, by standing the real executable in for the launcher shim. That keeps a
//! modded profile from accidentally booting into an anti-cheat session, which is the
//! situation that gets accounts banned.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, IoContext, Result};
use crate::games::Game;

/// Where the original shim is parked while the bypass is active.
pub const BACKUP_NAME: &str = "start_protected_game.exe.roundtable-backup";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EacState {
    /// The shim is intact; Steam's Play button boots anti-cheat.
    Active,
    /// The shim has been replaced; every launch path is anti-cheat free.
    Bypassed,
    /// This title has no anti-cheat shim at all.
    NotPresent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EacStatus {
    pub state: EacState,
    pub shim: Option<PathBuf>,
    pub backup: Option<PathBuf>,
    /// Plain-language explanation shown next to the toggle.
    pub detail: String,
}

pub fn status(game: Game, game_dir: &Path) -> EacStatus {
    let Some(shim_name) = game.eac_executable() else {
        return EacStatus {
            state: EacState::NotPresent,
            shim: None,
            backup: None,
            detail: format!("{} does not use Easy Anti-Cheat.", game.display_name()),
        };
    };

    let shim = game_dir.join(shim_name);
    let backup = game_dir.join(BACKUP_NAME);

    if backup.exists() {
        EacStatus {
            state: EacState::Bypassed,
            shim: Some(shim),
            backup: Some(backup),
            detail: "Anti-cheat is bypassed. Every launch, including Steam's Play button, starts the game directly. Stay offline.".into(),
        }
    } else if shim.exists() {
        EacStatus {
            state: EacState::Active,
            shim: Some(shim),
            backup: None,
            detail: "Anti-cheat is active. Mod loaders still bypass it for their own launches, but Steam's Play button does not.".into(),
        }
    } else {
        EacStatus {
            state: EacState::NotPresent,
            shim: None,
            backup: None,
            detail: "No anti-cheat shim found in this installation.".into(),
        }
    }
}

/// Replaces the anti-cheat shim with the real executable.
pub fn disable(game: Game, game_dir: &Path) -> Result<EacStatus> {
    let shim_name = game
        .eac_executable()
        .ok_or_else(|| Error::msg("this title has no anti-cheat shim to disable"))?;

    let shim = game_dir.join(shim_name);
    let backup = game_dir.join(BACKUP_NAME);
    let real = game_dir.join(game.executable());

    if backup.exists() {
        return Ok(status(game, game_dir));
    }
    if !shim.exists() {
        return Err(Error::msg(format!(
            "{} was not found, so there is nothing to bypass",
            shim.display()
        )));
    }
    if !real.exists() {
        return Err(Error::msg(format!(
            "{} is missing; refusing to touch the anti-cheat shim",
            real.display()
        )));
    }

    // Park the original first. If the copy below fails the install is still intact
    // and `restore` can put everything back.
    std::fs::rename(&shim, &backup).at(&shim)?;
    if let Err(err) = std::fs::copy(&real, &shim) {
        std::fs::rename(&backup, &shim).ok();
        return Err(Error::Io {
            path: shim,
            source: err,
        });
    }

    Ok(status(game, game_dir))
}

/// Puts the original anti-cheat shim back.
pub fn enable(game: Game, game_dir: &Path) -> Result<EacStatus> {
    let shim_name = game
        .eac_executable()
        .ok_or_else(|| Error::msg("this title has no anti-cheat shim to restore"))?;

    let shim = game_dir.join(shim_name);
    let backup = game_dir.join(BACKUP_NAME);

    if !backup.exists() {
        return Ok(status(game, game_dir));
    }

    if shim.exists() {
        std::fs::remove_file(&shim).at(&shim)?;
    }
    std::fs::rename(&backup, &shim).at(&backup)?;

    Ok(status(game, game_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-eac-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stage(dir: &Path) {
        std::fs::write(dir.join("eldenring.exe"), b"real game").unwrap();
        std::fs::write(dir.join("start_protected_game.exe"), b"eac shim").unwrap();
    }

    #[test]
    fn reports_active_then_bypassed_then_active_again() {
        let dir = scratch("cycle");
        stage(&dir);

        assert_eq!(status(Game::EldenRing, &dir).state, EacState::Active);

        let after = disable(Game::EldenRing, &dir).unwrap();
        assert_eq!(after.state, EacState::Bypassed);
        assert_eq!(
            std::fs::read(dir.join("start_protected_game.exe")).unwrap(),
            b"real game"
        );
        assert_eq!(std::fs::read(dir.join(BACKUP_NAME)).unwrap(), b"eac shim");

        let restored = enable(Game::EldenRing, &dir).unwrap();
        assert_eq!(restored.state, EacState::Active);
        assert_eq!(
            std::fs::read(dir.join("start_protected_game.exe")).unwrap(),
            b"eac shim"
        );
        assert!(!dir.join(BACKUP_NAME).exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disabling_twice_does_not_clobber_the_backup() {
        let dir = scratch("idempotent");
        stage(&dir);

        disable(Game::EldenRing, &dir).unwrap();
        disable(Game::EldenRing, &dir).unwrap();

        // The parked shim must still be the genuine one, not a second copy of the game.
        assert_eq!(std::fs::read(dir.join(BACKUP_NAME)).unwrap(), b"eac shim");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refuses_when_the_real_executable_is_missing() {
        let dir = scratch("no-exe");
        std::fs::write(dir.join("start_protected_game.exe"), b"eac shim").unwrap();

        assert!(disable(Game::EldenRing, &dir).is_err());
        // The shim must be left exactly where it was.
        assert!(dir.join("start_protected_game.exe").exists());
        assert!(!dir.join(BACKUP_NAME).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restoring_without_a_backup_is_harmless() {
        let dir = scratch("no-backup");
        stage(&dir);
        let state = enable(Game::EldenRing, &dir).unwrap();
        assert_eq!(state.state, EacState::Active);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn titles_without_a_shim_report_not_present() {
        let dir = scratch("ds3");
        assert_eq!(
            status(Game::DarkSouls3, &dir).state,
            EacState::NotPresent
        );
        assert!(disable(Game::DarkSouls3, &dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
