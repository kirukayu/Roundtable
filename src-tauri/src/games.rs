use serde::{Deserialize, Serialize};

/// The FromSoftware titles Roundtable knows how to manage.
///
/// Steam app ids and me3 short names are taken from the me3 mod-profile schema
/// (`schemas/mod-profile.md`), which is the authoritative list both loaders agree on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Game {
    EldenRing,
    Nightreign,
    DarkSouls3,
    Sekiro,
    ArmoredCore6,
}

impl Game {
    pub const ALL: [Game; 5] = [
        Game::EldenRing,
        Game::Nightreign,
        Game::DarkSouls3,
        Game::Sekiro,
        Game::ArmoredCore6,
    ];

    pub fn steam_app_id(self) -> u32 {
        match self {
            Game::EldenRing => 1_245_620,
            Game::Nightreign => 2_622_380,
            Game::DarkSouls3 => 374_320,
            Game::Sekiro => 814_380,
            Game::ArmoredCore6 => 1_888_160,
        }
    }

    /// Short id understood by `me3 launch --game <id>`.
    pub fn me3_id(self) -> &'static str {
        match self {
            Game::EldenRing => "eldenring",
            Game::Nightreign => "nightreign",
            Game::DarkSouls3 => "darksouls3",
            Game::Sekiro => "sekiro",
            Game::ArmoredCore6 => "armoredcore6",
        }
    }

    /// Short id understood by `modengine2_launcher -t <id>`. ModEngine 2 never
    /// shipped Sekiro or Nightreign support, so those return `None`.
    pub fn me2_id(self) -> Option<&'static str> {
        match self {
            Game::EldenRing => Some("er"),
            Game::DarkSouls3 => Some("ds3"),
            Game::ArmoredCore6 => Some("ac6"),
            Game::Sekiro | Game::Nightreign => None,
        }
    }

    /// Name of the ModEngine 2 config file for this game.
    pub fn me2_config_name(self) -> Option<&'static str> {
        match self {
            Game::EldenRing => Some("config_eldenring.toml"),
            Game::DarkSouls3 => Some("config_darksouls3.toml"),
            Game::ArmoredCore6 => Some("config_armoredcore6.toml"),
            Game::Sekiro | Game::Nightreign => None,
        }
    }

    /// The real game binary, which lives in the `Game` subfolder of the install root.
    pub fn executable(self) -> &'static str {
        match self {
            Game::EldenRing => "eldenring.exe",
            Game::Nightreign => "nightreign.exe",
            Game::DarkSouls3 => "DarkSoulsIII.exe",
            Game::Sekiro => "sekiro.exe",
            Game::ArmoredCore6 => "armoredcore6.exe",
        }
    }

    /// Easy Anti-Cheat shim that Steam actually launches. Starting the real
    /// executable directly is what "anti-cheat off" means for these games.
    pub fn eac_executable(self) -> Option<&'static str> {
        match self {
            Game::EldenRing | Game::Nightreign | Game::ArmoredCore6 => {
                Some("start_protected_game.exe")
            }
            Game::DarkSouls3 | Game::Sekiro => None,
        }
    }

    /// Folder under `%APPDATA%` holding this game's save files.
    pub fn appdata_folder(self) -> &'static str {
        match self {
            Game::EldenRing => "EldenRing",
            Game::Nightreign => "Nightreign",
            Game::DarkSouls3 => "DarkSoulsIII",
            Game::Sekiro => "Sekiro",
            Game::ArmoredCore6 => "ArmoredCore6",
        }
    }

    /// Default save file name and its vanilla extension.
    pub fn save_file(self) -> &'static str {
        match self {
            Game::EldenRing => "ER0000.sl2",
            Game::Nightreign => "NR0000.sl2",
            Game::DarkSouls3 => "DS30000.sl2",
            Game::Sekiro => "S0000.sl2",
            Game::ArmoredCore6 => "AC60000.sl2",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Game::EldenRing => "Elden Ring",
            Game::Nightreign => "Elden Ring Nightreign",
            Game::DarkSouls3 => "Dark Souls III",
            Game::Sekiro => "Sekiro: Shadows Die Twice",
            Game::ArmoredCore6 => "Armored Core VI",
        }
    }

    /// Fits a narrow sidebar row without truncating.
    pub fn short_name(self) -> &'static str {
        match self {
            Game::EldenRing => "Elden Ring",
            Game::Nightreign => "Nightreign",
            Game::DarkSouls3 => "Dark Souls III",
            Game::Sekiro => "Sekiro",
            Game::ArmoredCore6 => "Armored Core VI",
        }
    }

    /// Year of release, shown on library tiles.
    pub fn year(self) -> u16 {
        match self {
            Game::EldenRing => 2022,
            Game::Nightreign => 2025,
            Game::DarkSouls3 => 2016,
            Game::Sekiro => 2019,
            Game::ArmoredCore6 => 2023,
        }
    }

    /// Seamless Co-op only exists for ELDEN RING.
    pub fn supports_seamless_coop(self) -> bool {
        matches!(self, Game::EldenRing)
    }

    /// Number of character slots in the save container.
    pub fn save_slot_count(self) -> usize {
        match self {
            Game::EldenRing => 10,
            _ => 10,
        }
    }

    pub fn from_steam_app_id(id: u32) -> Option<Game> {
        Game::ALL.into_iter().find(|g| g.steam_app_id() == id)
    }

    pub fn from_executable(name: &str) -> Option<Game> {
        let lower = name.to_ascii_lowercase();
        Game::ALL
            .into_iter()
            .find(|g| g.executable().to_ascii_lowercase() == lower)
    }

    /// Steam movie id for this title's best cinematic trailer.
    ///
    /// The store API that lists these is often unreachable, so the ids are pinned.
    /// The CDN path they resolve to is a plain mp4, which a webview can play with
    /// no extra machinery.
    fn trailer_id(self) -> u64 {
        match self {
            // "Rise, Tarnished" launch trailer.
            Game::EldenRing => 256_875_477,
            // Reveal gameplay trailer.
            Game::Nightreign => 257_084_452,
            // "Kingdom Fall".
            Game::DarkSouls3 => 256_663_135,
            Game::Sekiro => 256_745_081,
            Game::ArmoredCore6 => 256_966_031,
        }
    }

    pub fn trailer_url(self) -> String {
        format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/movie480.mp4",
            self.trailer_id()
        )
    }

    /// The colour of buttons and current state while this title is selected.
    pub fn accent(self) -> &'static str {
        match self {
            Game::EldenRing => "#d8b25c",
            Game::Nightreign => "#8f7ee6",
            Game::DarkSouls3 => "#d4703c",
            Game::Sekiro => "#d4553f",
            Game::ArmoredCore6 => "#3fb5c9",
        }
    }

    /// The two colours of the aurora behind everything.
    ///
    /// Held apart on the wheel so the blooms read as a gradient rather than one
    /// flat wash: gold over deep bronze for Elden Ring, ember over blood for
    /// Sekiro, and so on. This is what makes each title feel like its own place.
    pub fn aurora(self) -> (&'static str, &'static str) {
        match self {
            // Erdtree gold falling into bronze shadow.
            Game::EldenRing => ("#8a6a1e", "#3d2f14"),
            // Night sky: violet against deep indigo.
            Game::Nightreign => ("#4a3d8f", "#1e2352"),
            // Ember and ash.
            Game::DarkSouls3 => ("#8a3c1c", "#3a1f14"),
            // Lacquer red over cold slate.
            Game::Sekiro => ("#8c2f22", "#2a3038"),
            // Rubicon: coolant blue against hazard rust.
            Game::ArmoredCore6 => ("#1c6272", "#5a3418"),
        }
    }

    /// Official Steam library art, used for cards and the dashboard hero.
    pub fn cover_url(self) -> String {
        format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/library_600x900.jpg",
            self.steam_app_id()
        )
    }

    pub fn hero_url(self) -> String {
        format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/library_hero.jpg",
            self.steam_app_id()
        )
    }

    pub fn logo_url(self) -> String {
        format!(
            "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/logo.png",
            self.steam_app_id()
        )
    }
}

/// Serialisable description handed to the UI so the frontend never hardcodes ids.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub id: Game,
    pub name: &'static str,
    pub short: &'static str,
    pub year: u16,
    pub steam_app_id: u32,
    pub executable: &'static str,
    pub save_file: &'static str,
    pub supports_seamless_coop: bool,
    pub supports_modengine2: bool,
    pub cover_url: String,
    pub hero_url: String,
    pub logo_url: String,
    pub trailer_url: String,
    pub accent: &'static str,
    /// First and second aurora bloom colours.
    pub aurora: [&'static str; 2],
}

impl From<Game> for GameInfo {
    fn from(id: Game) -> Self {
        GameInfo {
            id,
            name: id.display_name(),
            short: id.short_name(),
            year: id.year(),
            steam_app_id: id.steam_app_id(),
            executable: id.executable(),
            save_file: id.save_file(),
            supports_seamless_coop: id.supports_seamless_coop(),
            supports_modengine2: id.me2_id().is_some(),
            cover_url: id.cover_url(),
            hero_url: id.hero_url(),
            logo_url: id.logo_url(),
            trailer_url: id.trailer_url(),
            accent: id.accent(),
            aurora: [id.aurora().0, id.aurora().1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_title_has_distinct_art_and_an_accent() {
        let mut accents = Vec::new();
        for game in Game::ALL {
            let info = GameInfo::from(game);
            assert!(info.cover_url.contains(&game.steam_app_id().to_string()));
            assert!(info.trailer_url.ends_with(".mp4"));
            // A shared accent would defeat the point of recolouring per title.
            assert!(!accents.contains(&info.accent), "duplicate accent");
            accents.push(info.accent);
        }
    }

    #[test]
    fn accents_are_valid_hex() {
        for game in Game::ALL {
            let accent = game.accent();
            assert_eq!(accent.len(), 7);
            assert!(accent.starts_with('#'));
            assert!(accent[1..].chars().all(|c| c.is_ascii_hexdigit()));
        }
    }
}
