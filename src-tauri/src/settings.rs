//! Persisted application settings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IoContext, Result};
use crate::games::Game;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub selected_game: Game,
    /// Install roots the user has confirmed, keyed by game.
    pub installations: Vec<SavedInstall>,
    /// Titles pinned to the top of the library.
    pub favourites: Vec<Game>,
    pub active_profile: Option<String>,
    /// Stored locally and never sent anywhere except api.nexusmods.com.
    pub nexus_api_key: Option<String>,
    pub discord_presence: bool,
    pub auto_backup_on_launch: bool,
    pub auto_backup_keep: usize,
    pub theme: String,
    pub accent: String,
    pub ui_scale: f32,
    pub reduce_motion: bool,
    pub language: String,
    /// Deploy profiles as a `mod` junction in the game folder for tools that
    /// require it. Off by default: the loaders take absolute paths.
    pub use_junction_deploy: bool,
    pub confirm_destructive: bool,
    pub download_connections: usize,
    pub download_dir: Option<PathBuf>,
    pub torrent_port: u16,
    pub use_doh: bool,
    pub first_run_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedInstall {
    pub game: Game,
    pub root: PathBuf,
    pub is_default: bool,
    pub label: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            selected_game: Game::EldenRing,
            installations: Vec::new(),
            favourites: Vec::new(),
            active_profile: None,
            nexus_api_key: None,
            discord_presence: false,
            auto_backup_on_launch: true,
            auto_backup_keep: 20,
            theme: "gilded-dark".into(),
            accent: "erdtree".into(),
            ui_scale: 1.0,
            reduce_motion: false,
            language: "en".into(),
            use_junction_deploy: false,
            confirm_destructive: true,
            download_connections: 8,
            download_dir: None,
            torrent_port: 6881,
            use_doh: true,
            first_run_complete: false,
        }
    }
}

impl Settings {
    pub fn path(app_data: &Path) -> PathBuf {
        app_data.join("settings.json")
    }

    pub fn load(app_data: &Path) -> Settings {
        let path = Settings::path(app_data);
        std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, app_data: &Path) -> Result<()> {
        std::fs::create_dir_all(app_data).at(app_data)?;
        let path = Settings::path(app_data);
        std::fs::write(&path, serde_json::to_vec_pretty(self)?).at(&path)?;
        Ok(())
    }

    pub fn install_for(&self, game: Game) -> Option<&SavedInstall> {
        self.installations
            .iter()
            .find(|i| i.game == game && i.is_default)
            .or_else(|| self.installations.iter().find(|i| i.game == game))
    }

    /// Adds or replaces an install, keeping exactly one default per game.
    pub fn remember_install(&mut self, game: Game, root: PathBuf, make_default: bool) {
        self.installations
            .retain(|i| !(i.game == game && i.root == root));

        if make_default {
            for existing in self.installations.iter_mut().filter(|i| i.game == game) {
                existing.is_default = false;
            }
        }

        let is_only = !self.installations.iter().any(|i| i.game == game);
        self.installations.push(SavedInstall {
            game,
            root,
            is_default: make_default || is_only,
            label: None,
        });
    }

    pub fn forget_install(&mut self, game: Game, root: &Path) {
        self.installations
            .retain(|i| !(i.game == game && i.root == root));
        // Promote another install so the game is never left without a default.
        if !self.installations.iter().any(|i| i.game == game && i.is_default) {
            if let Some(first) = self.installations.iter_mut().find(|i| i.game == game) {
                first.is_default = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let settings = Settings::default();
        assert_eq!(settings.selected_game, Game::EldenRing);
        assert!(settings.auto_backup_on_launch);
        assert!(!settings.discord_presence, "telemetry-ish features stay off");
        assert!(!settings.use_junction_deploy);
        assert!(settings.nexus_api_key.is_none());
    }

    #[test]
    fn settings_round_trip_through_disk() {
        let dir = std::env::temp_dir().join("roundtable-settings-roundtrip");
        std::fs::remove_dir_all(&dir).ok();

        let mut settings = Settings::default();
        settings.download_connections = 12;
        settings.theme = "ashen".into();
        settings.save(&dir).unwrap();

        let loaded = Settings::load(&dir);
        assert_eq!(loaded.download_connections, 12);
        assert_eq!(loaded.theme, "ashen");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_yields_defaults_rather_than_an_error() {
        let loaded = Settings::load(Path::new("Z:\\nowhere-at-all"));
        assert_eq!(loaded.theme, "gilded-dark");
    }

    #[test]
    fn a_corrupt_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join("roundtable-settings-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Settings::path(&dir), b"{ not json").unwrap();

        assert_eq!(Settings::load(&dir).selected_game, Game::EldenRing);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_fields_in_an_older_file_do_not_break_loading() {
        let dir = std::env::temp_dir().join("roundtable-settings-partial");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Settings::path(&dir), br#"{"theme":"ashen"}"#).unwrap();

        let loaded = Settings::load(&dir);
        assert_eq!(loaded.theme, "ashen");
        // Everything absent must come from Default, not be dropped.
        assert_eq!(loaded.download_connections, 8);
        assert!(loaded.auto_backup_on_launch);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exactly_one_install_stays_default_per_game() {
        let mut settings = Settings::default();
        settings.remember_install(Game::EldenRing, PathBuf::from("D:\\steam\\ER"), true);
        settings.remember_install(Game::EldenRing, PathBuf::from("E:\\repack\\ER"), true);
        settings.remember_install(Game::DarkSouls3, PathBuf::from("D:\\steam\\DS3"), false);

        let defaults: Vec<_> = settings
            .installations
            .iter()
            .filter(|i| i.game == Game::EldenRing && i.is_default)
            .collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].root, PathBuf::from("E:\\repack\\ER"));

        // The first install of a game becomes its default even without asking.
        assert!(settings.install_for(Game::DarkSouls3).unwrap().is_default);
    }

    #[test]
    fn re_adding_the_same_path_does_not_duplicate_it() {
        let mut settings = Settings::default();
        let path = PathBuf::from("D:\\steam\\ER");
        settings.remember_install(Game::EldenRing, path.clone(), true);
        settings.remember_install(Game::EldenRing, path.clone(), true);
        assert_eq!(settings.installations.len(), 1);
    }

    #[test]
    fn forgetting_the_default_promotes_another_install() {
        let mut settings = Settings::default();
        settings.remember_install(Game::EldenRing, PathBuf::from("D:\\a"), true);
        settings.remember_install(Game::EldenRing, PathBuf::from("D:\\b"), false);

        settings.forget_install(Game::EldenRing, Path::new("D:\\a"));

        let remaining = settings.install_for(Game::EldenRing).unwrap();
        assert_eq!(remaining.root, PathBuf::from("D:\\b"));
        assert!(remaining.is_default);
    }
}
