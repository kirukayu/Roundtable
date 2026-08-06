use serde::{Deserialize, Serialize};

/// The FromSoftware catalogue.
///
/// Two of these never came to PC. They are listed anyway because the launcher is
/// about the studio's work, not only about what it can start; the interface marks
/// them plainly rather than pretending they are installable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Game {
    EldenRing,
    Nightreign,
    DarkSoulsRemastered,
    DarkSouls2,
    DarkSouls3,
    Sekiro,
    ArmoredCore6,
    Bloodborne,
    DemonsSouls,
}

impl Game {
    /// Release order, which is how the grid reads.
    pub const ALL: [Game; 9] = [
        Game::EldenRing,
        Game::Nightreign,
        Game::Sekiro,
        Game::DarkSouls3,
        Game::DarkSouls2,
        Game::DarkSoulsRemastered,
        Game::ArmoredCore6,
        Game::Bloodborne,
        Game::DemonsSouls,
    ];

    /// Titles the launcher can actually manage.
    pub fn is_playable(self) -> bool {
        !matches!(self, Game::Bloodborne | Game::DemonsSouls)
    }

    pub fn steam_app_id(self) -> u32 {
        match self {
            Game::EldenRing => 1_245_620,
            Game::Nightreign => 2_622_380,
            Game::DarkSoulsRemastered => 570_940,
            Game::DarkSouls2 => 335_300,
            Game::DarkSouls3 => 374_320,
            Game::Sekiro => 814_380,
            Game::ArmoredCore6 => 1_888_160,
            // Console exclusives with no Steam entry.
            Game::Bloodborne | Game::DemonsSouls => 0,
        }
    }

    /// Short id understood by `me3 launch --game <id>`.
    pub fn me3_id(self) -> Option<&'static str> {
        match self {
            Game::EldenRing => Some("eldenring"),
            Game::Nightreign => Some("nightreign"),
            Game::DarkSouls3 => Some("darksouls3"),
            Game::Sekiro => Some("sekiro"),
            Game::ArmoredCore6 => Some("armoredcore6"),
            _ => None,
        }
    }

    /// Short id understood by `modengine2_launcher -t <id>`.
    pub fn me2_id(self) -> Option<&'static str> {
        match self {
            Game::EldenRing => Some("er"),
            Game::DarkSouls3 => Some("ds3"),
            Game::ArmoredCore6 => Some("ac6"),
            _ => None,
        }
    }

    pub fn me2_config_name(self) -> Option<&'static str> {
        match self {
            Game::EldenRing => Some("config_eldenring.toml"),
            Game::DarkSouls3 => Some("config_darksouls3.toml"),
            Game::ArmoredCore6 => Some("config_armoredcore6.toml"),
            _ => None,
        }
    }

    pub fn executable(self) -> &'static str {
        match self {
            Game::EldenRing => "eldenring.exe",
            Game::Nightreign => "nightreign.exe",
            Game::DarkSoulsRemastered => "DarkSoulsRemastered.exe",
            Game::DarkSouls2 => "DarkSoulsII.exe",
            Game::DarkSouls3 => "DarkSoulsIII.exe",
            Game::Sekiro => "sekiro.exe",
            Game::ArmoredCore6 => "armoredcore6.exe",
            Game::Bloodborne | Game::DemonsSouls => "",
        }
    }

    /// Files that identify the game's own folder, whatever the executable is
    /// called.
    ///
    /// Repacks rename and repackage the launcher freely, but none of them can
    /// touch the data archives — the game will not load without them, under
    /// exactly these names. So when the executable cannot be found by name,
    /// these are what say "the game is here".
    pub fn signature_files(self) -> &'static [&'static str] {
        match self {
            Game::EldenRing => &["Data0.bdt", "Data0.bhd", "regulation.bin"],
            Game::Nightreign => &["Data0.bdt", "Data0.bhd", "regulation.bin"],
            Game::ArmoredCore6 => &["Data0.bdt", "Data0.bhd", "regulation.bin"],
            Game::Sekiro => &["data1.bdt", "data1.bhd"],
            Game::DarkSouls3 => &["Data1.bdt", "Data1.bhd"],
            Game::DarkSouls2 => &["GameDataEbl.bdt"],
            Game::DarkSoulsRemastered => &["dvdbnd0.bhd", "dvdbnd0.bdt"],
            Game::Bloodborne | Game::DemonsSouls => &[],
        }
    }

    /// Executables in a game folder that are not the game.
    ///
    /// A repack ships several: the anti-cheat shim, a language picker, the
    /// co-op launcher, whatever the cracker added. When the real executable has
    /// been renamed, these are the ones to rule out before guessing.
    pub fn helper_executables(self) -> &'static [&'static str] {
        &[
            "start_protected_game.exe",
            "ersc_launcher.exe",
            "language selector.exe",
            "modengine2_launcher.exe",
            "me3.exe",
            "me3-launcher.exe",
            "launcher.exe",
            "unins000.exe",
            "vcredist_x64.exe",
            "dxsetup.exe",
            "crashpad_handler.exe",
            "steamerrorreporter.exe",
            "steamerrorreporter64.exe",
        ]
    }

    /// Easy Anti-Cheat shim that Steam launches. Starting the real executable
    /// directly is what "anti-cheat off" means for these titles.
    pub fn eac_executable(self) -> Option<&'static str> {
        match self {
            Game::EldenRing | Game::Nightreign | Game::ArmoredCore6 => {
                Some("start_protected_game.exe")
            }
            _ => None,
        }
    }

    pub fn appdata_folder(self) -> &'static str {
        match self {
            Game::EldenRing => "EldenRing",
            Game::Nightreign => "Nightreign",
            Game::DarkSoulsRemastered => "DarkSoulsRemastered",
            Game::DarkSouls2 => "DarkSoulsII",
            Game::DarkSouls3 => "DarkSoulsIII",
            Game::Sekiro => "Sekiro",
            Game::ArmoredCore6 => "ArmoredCore6",
            Game::Bloodborne => "Bloodborne",
            Game::DemonsSouls => "DemonsSouls",
        }
    }

    pub fn save_file(self) -> &'static str {
        match self {
            Game::EldenRing => "ER0000.sl2",
            Game::Nightreign => "NR0000.sl2",
            Game::DarkSoulsRemastered => "DRAKS0005.sl2",
            Game::DarkSouls2 => "DS2SOFS0000.sl2",
            Game::DarkSouls3 => "DS30000.sl2",
            Game::Sekiro => "S0000.sl2",
            Game::ArmoredCore6 => "AC60000.sl2",
            Game::Bloodborne | Game::DemonsSouls => "",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Game::EldenRing => "Elden Ring",
            Game::Nightreign => "Elden Ring Nightreign",
            Game::DarkSoulsRemastered => "Dark Souls Remastered",
            Game::DarkSouls2 => "Dark Souls II",
            Game::DarkSouls3 => "Dark Souls III",
            Game::Sekiro => "Sekiro: Shadows Die Twice",
            Game::ArmoredCore6 => "Armored Core VI",
            Game::Bloodborne => "Bloodborne",
            Game::DemonsSouls => "Demon's Souls",
        }
    }

    /// Fits a narrow row without truncating.
    pub fn short_name(self) -> &'static str {
        match self {
            Game::EldenRing => "Elden Ring",
            Game::Nightreign => "Nightreign",
            Game::DarkSoulsRemastered => "Dark Souls",
            Game::DarkSouls2 => "Dark Souls II",
            Game::DarkSouls3 => "Dark Souls III",
            Game::Sekiro => "Sekiro",
            Game::ArmoredCore6 => "Armored Core VI",
            Game::Bloodborne => "Bloodborne",
            Game::DemonsSouls => "Demon's Souls",
        }
    }

    pub fn year(self) -> u16 {
        match self {
            Game::DemonsSouls => 2009,
            Game::DarkSoulsRemastered => 2011,
            Game::DarkSouls2 => 2014,
            Game::Bloodborne => 2015,
            Game::DarkSouls3 => 2016,
            Game::Sekiro => 2019,
            Game::EldenRing => 2022,
            Game::ArmoredCore6 => 2023,
            Game::Nightreign => 2025,
        }
    }

    /// One line of context, shown under the title on a card.
    pub fn note(self) -> &'static str {
        match self {
            Game::EldenRing => "Full mod, co-op and save support",
            Game::Nightreign => "Mods and saves",
            Game::DarkSoulsRemastered => "Saves and system tools",
            Game::DarkSouls2 => "Saves and system tools",
            Game::DarkSouls3 => "Mods and saves",
            Game::Sekiro => "Mods and saves",
            Game::ArmoredCore6 => "Mods and saves",
            Game::Bloodborne => "PlayStation exclusive",
            Game::DemonsSouls => "PlayStation exclusive",
        }
    }

    pub fn supports_seamless_coop(self) -> bool {
        matches!(self, Game::EldenRing)
    }

    pub fn save_slot_count(self) -> usize {
        10
    }

    pub fn from_steam_app_id(id: u32) -> Option<Game> {
        if id == 0 {
            return None;
        }
        Game::ALL.into_iter().find(|g| g.steam_app_id() == id)
    }

    pub fn from_executable(name: &str) -> Option<Game> {
        let lower = name.to_ascii_lowercase();
        Game::ALL
            .into_iter()
            .filter(|g| g.is_playable())
            .find(|g| g.executable().to_ascii_lowercase() == lower)
    }

    /// Vertical poster from Steam's own library art.
    pub fn cover_url(self) -> Option<String> {
        self.is_playable().then(|| {
            format!(
                "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/library_600x900.jpg",
                self.steam_app_id()
            )
        })
    }

    pub fn hero_url(self) -> Option<String> {
        self.is_playable().then(|| {
            format!(
                "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/library_hero.jpg",
                self.steam_app_id()
            )
        })
    }

    pub fn logo_url(self) -> Option<String> {
        self.is_playable().then(|| {
            format!(
                "https://cdn.cloudflare.steamstatic.com/steam/apps/{}/logo.png",
                self.steam_app_id()
            )
        })
    }
}

/// Serialisable description handed to the interface so it never hardcodes ids.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub id: Game,
    pub name: &'static str,
    pub short: &'static str,
    pub year: u16,
    pub note: &'static str,
    pub playable: bool,
    pub steam_app_id: u32,
    pub executable: &'static str,
    pub save_file: &'static str,
    pub supports_seamless_coop: bool,
    pub supports_modengine2: bool,
    pub supports_me3: bool,
    pub cover_url: Option<String>,
    pub hero_url: Option<String>,
    pub logo_url: Option<String>,
}

impl From<Game> for GameInfo {
    fn from(id: Game) -> Self {
        GameInfo {
            id,
            name: id.display_name(),
            short: id.short_name(),
            year: id.year(),
            note: id.note(),
            playable: id.is_playable(),
            steam_app_id: id.steam_app_id(),
            executable: id.executable(),
            save_file: id.save_file(),
            supports_seamless_coop: id.supports_seamless_coop(),
            supports_modengine2: id.me2_id().is_some(),
            supports_me3: id.me3_id().is_some(),
            cover_url: id.cover_url(),
            hero_url: id.hero_url(),
            logo_url: id.logo_url(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_in_release_order() {
        let years: Vec<u16> = Game::ALL.into_iter().map(Game::year).collect();
        // The list is curated rather than sorted, but every entry must have a
        // plausible year attached.
        assert!(years.iter().all(|y| (2009..=2026).contains(y)));
    }

    #[test]
    fn console_exclusives_are_marked_and_carry_no_art() {
        for game in [Game::Bloodborne, Game::DemonsSouls] {
            assert!(!game.is_playable());
            assert_eq!(game.steam_app_id(), 0);
            assert!(game.cover_url().is_none());
            assert!(game.executable().is_empty());
        }
    }

    #[test]
    fn every_playable_title_has_art_and_an_executable() {
        for game in Game::ALL.into_iter().filter(|g| g.is_playable()) {
            assert!(game.steam_app_id() > 0, "{} has no app id", game.display_name());
            assert!(!game.executable().is_empty());
            assert!(!game.save_file().is_empty());
            let cover = game.cover_url().expect("playable titles have covers");
            assert!(cover.contains(&game.steam_app_id().to_string()));
        }
    }

    #[test]
    fn executable_lookup_ignores_console_entries() {
        assert_eq!(Game::from_executable("eldenring.exe"), Some(Game::EldenRing));
        assert_eq!(Game::from_executable("DarkSoulsII.exe"), Some(Game::DarkSouls2));
        // An empty name must not match the console titles' empty executable.
        assert_eq!(Game::from_executable(""), None);
    }

    #[test]
    fn app_id_lookup_rejects_zero() {
        assert_eq!(Game::from_steam_app_id(0), None);
        assert_eq!(Game::from_steam_app_id(1_245_620), Some(Game::EldenRing));
    }

    #[test]
    fn save_file_names_are_distinct() {
        let mut seen = Vec::new();
        for game in Game::ALL.into_iter().filter(|g| g.is_playable()) {
            let file = game.save_file();
            assert!(!seen.contains(&file), "duplicate save file {file}");
            seen.push(file);
        }
    }

    #[test]
    fn only_elden_ring_claims_seamless_coop() {
        let with_coop: Vec<_> = Game::ALL
            .into_iter()
            .filter(|g| g.supports_seamless_coop())
            .collect();
        assert_eq!(with_coop, vec![Game::EldenRing]);
    }
}
