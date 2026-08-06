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
    /// Builds an `Installation` from any folder at or near the game.
    pub fn probe(game: Game, folder: &Path) -> Result<Installation> {
        let (root, game_dir) = resolve_layout(game, folder).ok_or_else(|| {
            Error::msg(format!(
                "No {} install under {}. Looked for {} and for the game's data files.",
                game.display_name(),
                folder.display(),
                game.executable()
            ))
        })?;

        let executable = executable_in(game, &game_dir);
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

/// Works out where the game actually is, given any folder near it.
///
/// People point at whatever they recognise: the repack's outer folder, a
/// launcher directory two levels above the game, the `Game` folder itself, or
/// something inside it. Repacks nest arbitrarily —
/// `ELDEN RING (2013)\ELDEN RING (2022)\Elden Ring\Game` is a real example — so
/// checking one fixed layout rejects installs that are plainly there.
///
/// The search goes down first, taking the shallowest match, then up through the
/// ancestors. Both are bounded, and only the picked subtree is walked, so this
/// never wanders off across the disk.
fn resolve_layout(game: Game, folder: &Path) -> Option<(PathBuf, PathBuf)> {
    let exe = game.executable();

    // The two common shapes, checked directly so the usual case costs nothing.
    if folder.join(exe).is_file() {
        return Some((parent_or_self(folder), folder.to_path_buf()));
    }
    let nested = folder.join("Game");
    if nested.join(exe).is_file() {
        return Some((folder.to_path_buf(), nested));
    }

    // Downward, shallowest first: the outer folder of a nested repack.
    let mut candidates = deep_scan(game, folder, 5);
    if !candidates.is_empty() {
        candidates.sort_by_key(|path| path.components().count());
        let game_dir = candidates.remove(0);
        return Some((parent_or_self(&game_dir), game_dir));
    }

    // Upward: they picked something inside the game, like SeamlessCoop or mod.
    let mut level = folder;
    for _ in 0..4 {
        let Some(parent) = level.parent() else { break };
        if parent.join(exe).is_file() {
            return Some((parent_or_self(parent), parent.to_path_buf()));
        }
        let sibling = parent.join("Game");
        if sibling.join(exe).is_file() {
            return Some((parent.to_path_buf(), sibling));
        }
        level = parent;
    }

    // Last resort: find the game by its data files rather than its executable.
    // A repack can rename or repackage the launcher, but the archives have to
    // keep their names or the game does not start.
    let mut by_data = scan_by_signature(game, folder, 5);
    if !by_data.is_empty() {
        by_data.sort_by_key(|path| path.components().count());
        let game_dir = by_data.remove(0);
        return Some((parent_or_self(&game_dir), game_dir));
    }

    None
}

/// True when a folder holds the game's data archives.
fn has_signature(game: Game, folder: &Path) -> bool {
    let signature = game.signature_files();
    !signature.is_empty() && signature.iter().all(|name| folder.join(name).is_file())
}

/// Finds folders holding the game's data archives.
fn scan_by_signature(game: Game, root: &Path, max_depth: usize) -> Vec<PathBuf> {
    if game.signature_files().is_empty() {
        return Vec::new();
    }
    walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_noise(entry.file_name().to_string_lossy().as_ref()))
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_dir())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| has_signature(game, path))
        .collect()
}

/// Folders never worth walking into.
pub fn is_noise(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "windows"
            | "$recycle.bin"
            | "system volume information"
            | "appdata"
            | "programdata"
            | "node_modules"
            | ".git"
    )
}

/// The executable to launch from a known game folder.
///
/// Normally the one the game ships under its own name. When a repack has
/// renamed it, the largest executable that is not a known helper is the game:
/// ELDEN RING's is around 80 MB and everything else in the folder is under ten.
pub fn executable_in(game: Game, game_dir: &Path) -> PathBuf {
    let named = game_dir.join(game.executable());
    if named.is_file() {
        return named;
    }

    let helpers = game.helper_executables();
    let mut best: Option<(u64, PathBuf)> = None;

    if let Ok(entries) = std::fs::read_dir(game_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.ends_with(".exe") || helpers.contains(&name.as_str()) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            // A game executable is tens of megabytes. Anything smaller is a
            // helper this list happens not to name.
            if size < 8 * 1024 * 1024 {
                continue;
            }
            if best.as_ref().is_none_or(|(largest, _)| size > *largest) {
                best = Some((size, path));
            }
        }
    }

    // Falling back to the expected name keeps the error message honest when
    // there is genuinely nothing to run.
    best.map(|(_, path)| path).unwrap_or(named)
}

/// The parent folder, or the folder itself when it is a drive root.
fn parent_or_self(folder: &Path) -> PathBuf {
    folder
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| folder.to_path_buf())
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
///
/// Three passes, cheapest first, each only running when the last found nothing:
/// the Steam registry, then the folder names installers use, then a bounded walk
/// of every fixed drive. The walk exists because repacks land in folders named
/// whatever their owner felt like — `pleasedonttakeofmymask` is a real one — and
/// a list of expected names will never cover that.
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

    if found.is_empty() {
        for candidate in sweep_drives(game, 4, |_| true) {
            push(candidate, &mut found);
        }
    }

    found
}

/// Walks every fixed drive to the end, looking for the game anywhere on it.
///
/// The shallow pass covers where installers put things. This covers where
/// people put things, which is anywhere at all, and is what runs when the
/// launcher would otherwise have to say it could not find a game the user can
/// see with their own eyes.
///
/// `progress` is called with each folder as it is entered and returns false to
/// stop, so the interface can show where it is rather than a frozen spinner.
pub fn deep_discover(game: Game, progress: impl FnMut(&Path) -> bool) -> Vec<Installation> {
    let mut found: Vec<Installation> = Vec::new();
    for candidate in sweep_drives(game, usize::MAX, progress) {
        if let Ok(install) = Installation::probe(game, &candidate) {
            if !found.iter().any(|i| paths_equal(&i.root, &install.root)) {
                found.push(install);
            }
        }
    }
    found
}

/// Walks every fixed drive looking for the game, by name or by its data files.
///
/// Directory names are compared as strings, which is nearly free; the on-disk
/// check for the game's archives only runs when the name misses. Pruning is
/// what keeps a full walk survivable — Windows, package caches and the like are
/// never entered.
fn sweep_drives(
    game: Game,
    max_depth: usize,
    mut progress: impl FnMut(&Path) -> bool,
) -> Vec<PathBuf> {
    let needle = normalise(game.display_name());
    let short = normalise(game.short_name());
    let mut hits = Vec::new();
    let mut seen = 0usize;

    for drive in fixed_drives() {
        let mut walker = walkdir::WalkDir::new(&drive)
            .follow_links(false);
        if max_depth != usize::MAX {
            walker = walker.max_depth(max_depth);
        }

        for entry in walker
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0 || !is_noise(entry.file_name().to_string_lossy().as_ref())
            })
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_dir())
        {
            let path = entry.path();

            // Reporting every folder would spend more time locking shared state
            // than walking; every few hundred is enough to look alive.
            seen += 1;
            if seen % 400 == 0 && !progress(path) {
                return hits;
            }

            let name = normalise(&entry.file_name().to_string_lossy());
            if (!needle.is_empty() && name.contains(&needle))
                || (!short.is_empty() && short.len() > 4 && name.contains(&short))
                || has_signature(game, path)
            {
                hits.push(path.to_path_buf());
                if hits.len() >= 40 {
                    return hits;
                }
            }
        }
    }

    hits
}

/// Lowercase, letters and digits only, so spacing and punctuation stop mattering.
fn normalise(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
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
pub fn fixed_drives() -> Vec<PathBuf> {
    ('A'..='Z')
        .map(|letter| PathBuf::from(format!("{letter}:\\")))
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(not(windows))]
pub fn fixed_drives() -> Vec<PathBuf> {
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

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-find-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A game folder as it exists on disk, optionally with the executable
    /// renamed the way some repacks do.
    fn lay_out(game_dir: &Path, exe: &str) {
        std::fs::create_dir_all(game_dir).unwrap();
        for name in Game::EldenRing.signature_files() {
            std::fs::write(game_dir.join(name), b"x").unwrap();
        }
        // Over the eight megabyte floor that separates a game from a helper.
        std::fs::write(game_dir.join(exe), vec![0u8; 9 * 1024 * 1024]).unwrap();
        std::fs::write(game_dir.join("start_protected_game.exe"), vec![0u8; 9 * 1024 * 1024])
            .unwrap();
    }

    #[test]
    fn the_folder_holding_the_executable_is_accepted() {
        let base = temp("direct");
        let game_dir = base.join("Game");
        lay_out(&game_dir, "eldenring.exe");

        let found = Installation::probe(Game::EldenRing, &game_dir).unwrap();
        assert_eq!(found.game_dir, game_dir);
        assert_eq!(found.executable, game_dir.join("eldenring.exe"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn the_install_root_above_it_is_accepted() {
        let base = temp("root");
        let root = base.join("ELDEN RING");
        lay_out(&root.join("Game"), "eldenring.exe");

        let found = Installation::probe(Game::EldenRing, &root).unwrap();
        assert_eq!(found.game_dir, root.join("Game"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_repack_nested_several_levels_deep_is_found() {
        // The shape that made a user pick the folder by hand and still be told
        // the game was not there.
        let base = temp("nested");
        let deep = base
            .join("ELDEN RING ( 2013 )")
            .join("ELDEN RING (2022)")
            .join("Elden Ring")
            .join("Game");
        lay_out(&deep, "eldenring.exe");

        let found = Installation::probe(Game::EldenRing, &base).unwrap();
        assert_eq!(found.game_dir, deep, "the outer folder must resolve inward");

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn picking_a_folder_inside_the_game_still_resolves() {
        let base = temp("inside");
        let game_dir = base.join("Game");
        lay_out(&game_dir, "eldenring.exe");
        let inner = game_dir.join("SeamlessCoop");
        std::fs::create_dir_all(&inner).unwrap();

        let found = Installation::probe(Game::EldenRing, &inner).unwrap();
        assert_eq!(found.game_dir, game_dir);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_renamed_executable_is_found_by_the_games_data_files() {
        let base = temp("renamed");
        let game_dir = base.join("Game");
        lay_out(&game_dir, "eldenring_launcher_patched.exe");
        std::fs::remove_file(game_dir.join("start_protected_game.exe")).ok();

        let found = Installation::probe(Game::EldenRing, &base).unwrap();
        assert_eq!(found.game_dir, game_dir, "the data archives identify the folder");
        assert_eq!(
            found.executable.file_name().unwrap(),
            "eldenring_launcher_patched.exe",
            "and the largest non-helper executable is the game"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn helpers_are_never_mistaken_for_the_game() {
        let base = temp("helpers");
        let game_dir = base.join("Game");
        std::fs::create_dir_all(&game_dir).unwrap();
        for name in Game::EldenRing.signature_files() {
            std::fs::write(game_dir.join(name), b"x").unwrap();
        }
        // Only helpers, all of them large.
        for name in ["start_protected_game.exe", "ersc_launcher.exe", "me3.exe"] {
            std::fs::write(game_dir.join(name), vec![0u8; 20 * 1024 * 1024]).unwrap();
        }
        std::fs::write(game_dir.join("game.exe"), vec![0u8; 9 * 1024 * 1024]).unwrap();

        let picked = executable_in(Game::EldenRing, &game_dir);
        assert_eq!(picked.file_name().unwrap(), "game.exe", "got {picked:?}");

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn small_executables_are_not_candidates() {
        let base = temp("small");
        let game_dir = base.join("Game");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("readme_tool.exe"), b"tiny").unwrap();

        // Nothing plausible, so it reports the name it expected rather than
        // pointing the loader at a two kilobyte helper.
        let picked = executable_in(Game::EldenRing, &game_dir);
        assert_eq!(picked.file_name().unwrap(), "eldenring.exe");

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn folder_names_are_matched_past_spacing_and_case() {
        // The forms a repack actually uses.
        for name in [
            "ELDEN RING",
            "Elden Ring",
            "elden_ring",
            "ELDEN RING (2022)",
            "EldenRing.v1.16",
        ] {
            assert!(
                normalise(name).contains(&normalise("ELDEN RING")),
                "{name} should match"
            );
        }
        assert!(!normalise("Dark Souls III").contains(&normalise("ELDEN RING")));
    }

    #[test]
    fn a_folder_holding_the_archives_is_recognised_whatever_it_is_called() {
        let base = temp("signature");
        let odd = base.join("pleasedonttakeofmymask").join("g");
        lay_out(&odd, "eldenring.exe");

        assert!(has_signature(Game::EldenRing, &odd));
        // And probing the nonsense-named parent still resolves inward.
        let found = Installation::probe(Game::EldenRing, &base.join("pleasedonttakeofmymask"))
            .expect("resolved");
        assert_eq!(found.game_dir, odd);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn an_unrelated_folder_is_still_rejected() {
        let base = temp("nothing");
        std::fs::create_dir_all(base.join("Documents")).unwrap();
        std::fs::write(base.join("notes.txt"), b"x").unwrap();

        let result = Installation::probe(Game::EldenRing, &base);
        assert!(result.is_err(), "an empty folder must not resolve to a game");

        std::fs::remove_dir_all(&base).ok();
    }

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
