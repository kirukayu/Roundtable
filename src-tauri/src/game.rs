//! Finding and describing game installations, whether they came from Steam or not.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::games::Game;
use crate::steam;

/// How a copy of the game got onto the disk. This drives the launch strategy:
/// a repack needs `--skip-steam-init` and a `steam_appid.txt`, a Steam copy does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallKind {
    Steam,
    /// A repack or cracked copy: Steamworks is emulated rather than real.
    Standalone,
    Unknown,
}

/// Files a repack leaves behind. Finding any of them means Steamworks is emulated,
/// so the launcher must not expect a running Steam client.
const EMULATOR_MARKERS: &[&str] = &[
    "steam_emu.ini",
    "SmartSteamEmu.ini",
    "ColdClientLoader.ini",
    "steam_settings",
    "SteamClient64.dll",
    "steamclient_loader.exe",
    "hlm.ini",
    "valve.ini",
    "cream_api.ini",
    "CreamAPI.ini",
    "ALI213.ini",
    "SteamOverlay64.dll",
    "launcher.ini",
    "OnlineFix.ini",
    "OnlineFix64.dll",
    "winmm.dll",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    pub game: Game,
    /// Install root: the folder that contains `Game/`.
    pub root: PathBuf,
    /// Folder holding the executable, normally `<root>/Game`.
    pub game_dir: PathBuf,
    pub executable: PathBuf,
    pub kind: InstallKind,
    /// File version of the game binary, e.g. `1.16.0.0`.
    pub version: Option<String>,
    /// Present when Easy Anti-Cheat's shim is installed next to the game.
    pub has_eac: bool,
    /// True when the anti-cheat shim has been bypassed by this launcher.
    pub eac_bypassed: bool,
    pub has_seamless_coop: bool,
    pub seamless_coop_version: Option<String>,
    pub size_bytes: Option<u64>,
    /// Emulator markers found in the game folder, shown so the user can see why
    /// Roundtable classified the install the way it did.
    pub markers: Vec<String>,
}

impl Installation {
    /// Builds an `Installation` from a folder that may be either the install root
    /// or the inner `Game` folder, since users pick both.
    pub fn probe(game: Game, folder: &Path) -> Result<Installation> {
        let (root, game_dir) = resolve_layout(game, folder).ok_or_else(|| {
            Error::msg(format!(
                "{} was not found under {}",
                game.executable(),
                folder.display()
            ))
        })?;

        let executable = game_dir.join(game.executable());
        let markers: Vec<String> = EMULATOR_MARKERS
            .iter()
            .filter(|marker| game_dir.join(marker).exists())
            .map(|marker| (*marker).to_string())
            .collect();

        let kind = classify(&root, game, &markers);
        let has_eac = game
            .eac_executable()
            .is_some_and(|exe| game_dir.join(exe).exists());
        let eac_bypassed = game_dir.join(crate::eac::BACKUP_NAME).exists();

        let coop_dll = game_dir.join("SeamlessCoop").join("ersc.dll");
        let has_seamless_coop = coop_dll.exists();

        Ok(Installation {
            game,
            version: file_version(&executable),
            executable,
            kind,
            has_eac,
            eac_bypassed,
            has_seamless_coop,
            seamless_coop_version: has_seamless_coop
                .then(|| file_version(&coop_dll))
                .flatten(),
            size_bytes: None,
            markers,
            root,
            game_dir,
        })
    }

    /// Refreshes the volatile fields without redoing discovery.
    pub fn refresh(&mut self) -> Result<()> {
        *self = Installation::probe(self.game, &self.root)?;
        Ok(())
    }

    pub fn appdata_dir(&self) -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join(self.game.appdata_folder()))
    }
}

/// Accepts either `<root>` or `<root>/Game` and returns both.
fn resolve_layout(game: Game, folder: &Path) -> Option<(PathBuf, PathBuf)> {
    let exe = game.executable();

    // Given the install root.
    let nested = folder.join("Game");
    if nested.join(exe).is_file() {
        return Some((folder.to_path_buf(), nested));
    }

    // Given the folder holding the executable.
    if folder.join(exe).is_file() {
        let root = folder
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| folder.to_path_buf());
        return Some((root, folder.to_path_buf()));
    }

    // Some repacks flatten the layout entirely.
    if folder.join(exe).is_file() {
        return Some((folder.to_path_buf(), folder.to_path_buf()));
    }

    None
}

fn classify(root: &Path, game: Game, markers: &[String]) -> InstallKind {
    if !markers.is_empty() {
        return InstallKind::Standalone;
    }
    if let Some(steam_dir) = steam::app_install_dir(game.steam_app_id()) {
        if paths_equal(&steam_dir, root) {
            return InstallKind::Steam;
        }
    }
    // A Steam copy always sits under `steamapps/common`.
    let lossy = root.to_string_lossy().to_ascii_lowercase();
    if lossy.contains("steamapps") {
        return InstallKind::Steam;
    }
    InstallKind::Unknown
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_ascii_lowercase()
            .trim_end_matches('\\')
            .to_string()
    };
    norm(a) == norm(b)
}

/// Discovers every installation Roundtable can find without asking the user.
pub fn discover(game: Game) -> Vec<Installation> {
    let mut found: Vec<Installation> = Vec::new();
    let push = |folder: PathBuf, out: &mut Vec<Installation>| {
        if let Ok(install) = Installation::probe(game, &folder) {
            if !out.iter().any(|i| paths_equal(&i.root, &install.root)) {
                out.push(install);
            }
        }
    };

    if let Some(dir) = steam::app_install_dir(game.steam_app_id()) {
        push(dir, &mut found);
    }

    for candidate in common_install_locations(game) {
        push(candidate, &mut found);
    }

    found
}

/// Folders repacks land in often enough to be worth checking without a full scan.
fn common_install_locations(game: Game) -> Vec<PathBuf> {
    let folder_names = install_folder_names(game);
    let mut roots: Vec<PathBuf> = Vec::new();

    for drive in fixed_drives() {
        for parent in [
            "Games",
            "Game",
            "SteamLibrary\\steamapps\\common",
            "Program Files",
            "Program Files (x86)",
            "Repacks",
            "Torrents",
            "",
        ] {
            let base = if parent.is_empty() {
                drive.clone()
            } else {
                drive.join(parent)
            };
            if !base.is_dir() {
                continue;
            }
            for name in &folder_names {
                roots.push(base.join(name));
            }
        }
    }

    roots.retain(|p| p.is_dir());
    roots
}

fn install_folder_names(game: Game) -> Vec<String> {
    let base = match game {
        Game::EldenRing => vec!["ELDEN RING", "Elden Ring", "EldenRing"],
        Game::Nightreign => vec!["ELDEN RING NIGHTREIGN", "Nightreign"],
        Game::DarkSoulsRemastered => {
            vec!["DARK SOULS REMASTERED", "Dark Souls Remastered", "DARK SOULS"]
        }
        Game::DarkSouls2 => vec![
            "DARK SOULS II Scholar of the First Sin",
            "Dark Souls II",
            "DarkSoulsII",
        ],
        Game::DarkSouls3 => vec!["DARK SOULS III", "Dark Souls III", "DarkSoulsIII"],
        Game::Sekiro => vec!["Sekiro", "Sekiro Shadows Die Twice"],
        Game::ArmoredCore6 => vec!["ARMORED CORE VI FIRES OF RUBICON", "Armored Core VI"],
        // Never installed on this platform.
        Game::Bloodborne | Game::DemonsSouls => vec![],
    };
    base.into_iter().map(str::to_string).collect()
}

#[cfg(windows)]
fn fixed_drives() -> Vec<PathBuf> {
    ('A'..='Z')
        .map(|letter| PathBuf::from(format!("{letter}:\\")))
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(not(windows))]
fn fixed_drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

/// Walks a folder tree looking for the game executable. Used by the optional deep
/// scan, which the user starts explicitly because it touches a lot of the disk.
pub fn deep_scan(game: Game, root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let exe = game.executable().to_ascii_lowercase();
    let skip = [
        "windows",
        "$recycle.bin",
        "system volume information",
        "appdata",
        "programdata",
        "node_modules",
        ".git",
    ];

    walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            !skip.contains(&name.as_str())
        })
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry.file_type().is_file()
                && entry.file_name().to_string_lossy().to_ascii_lowercase() == exe
        })
        .filter_map(|entry| entry.path().parent().map(Path::to_path_buf))
        .collect()
}

/// Reads the `FileVersion` field from a Windows executable's version resource.
#[cfg(windows)]
pub fn file_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };

    if !path.is_file() {
        return None;
    }

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }

        let mut buffer = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) == 0 {
            return None;
        }

        let sub_block: Vec<u16> = "\\".encode_utf16().chain(std::iter::once(0)).collect();
        let mut value: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut value_len: u32 = 0;
        if VerQueryValueW(
            buffer.as_ptr().cast(),
            sub_block.as_ptr(),
            &mut value,
            &mut value_len,
        ) == 0
            || value.is_null()
            || (value_len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }

        let info = &*(value as *const VS_FIXEDFILEINFO);
        Some(format!(
            "{}.{}.{}.{}",
            (info.dwFileVersionMS >> 16) & 0xFFFF,
            info.dwFileVersionMS & 0xFFFF,
            (info.dwFileVersionLS >> 16) & 0xFFFF,
            info.dwFileVersionLS & 0xFFFF,
        ))
    }
}

#[cfg(not(windows))]
pub fn file_version(_path: &Path) -> Option<String> {
    None
}

/// Total size of a folder tree, used for the install cards.
pub fn folder_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_kind_follows_emulator_markers() {
        let markers = vec!["steam_emu.ini".to_string()];
        assert_eq!(
            classify(Path::new("D:\\Games\\ELDEN RING"), Game::EldenRing, &markers),
            InstallKind::Standalone
        );
    }

    #[test]
    fn steamapps_path_reads_as_a_steam_install() {
        assert_eq!(
            classify(
                Path::new("D:\\SteamLibrary\\steamapps\\common\\ELDEN RING"),
                Game::EldenRing,
                &[]
            ),
            InstallKind::Steam
        );
    }

    #[test]
    fn unmarked_install_outside_steam_is_unknown() {
        assert_eq!(
            classify(Path::new("D:\\Games\\ELDEN RING"), Game::EldenRing, &[]),
            InstallKind::Unknown
        );
    }

    #[test]
    fn path_comparison_ignores_case_and_trailing_separator() {
        assert!(paths_equal(
            Path::new("D:\\Games\\ELDEN RING\\"),
            Path::new("d:\\games\\elden ring")
        ));
        assert!(!paths_equal(
            Path::new("D:\\Games\\ELDEN RING"),
            Path::new("D:\\Games\\ELDEN RING 2")
        ));
    }

    #[test]
    fn layout_resolution_accepts_root_and_game_folder() {
        let temp = std::env::temp_dir().join("roundtable-layout-test");
        let game_dir = temp.join("Game");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("eldenring.exe"), b"stub").unwrap();

        let from_root = resolve_layout(Game::EldenRing, &temp).unwrap();
        assert_eq!(from_root.1, game_dir);

        let from_game = resolve_layout(Game::EldenRing, &game_dir).unwrap();
        assert_eq!(from_game.1, game_dir);
        assert_eq!(from_game.0, temp);

        std::fs::remove_dir_all(&temp).ok();
    }

    #[test]
    fn layout_resolution_fails_cleanly_when_exe_is_absent() {
        let temp = std::env::temp_dir().join("roundtable-layout-empty");
        std::fs::create_dir_all(&temp).unwrap();
        assert!(resolve_layout(Game::EldenRing, &temp).is_none());
        std::fs::remove_dir_all(&temp).ok();
    }
}
