//! DLSS, frame generation and Reflex in a game that has none of them.
//!
//! ELDEN RING renders at whatever the settings say and caps at sixty. It has no
//! upscaler, so the usual wrappers — the ones that swap an existing DLSS for a
//! newer one — have nothing to hook. huutaiii's ERSS is a different thing: it
//! sits in front of D3D12 itself and brings the whole stack with it, DLSS 4 and
//! DLAA, DLSS frame generation, FSR 3.1, XeSS and Reflex.
//!
//! What it needs around it is the part people get wrong, and all of it is
//! already Roundtable's business: the anti-cheat off, hardware GPU scheduling
//! on, and ray tracing down when the global illumination flickers. So the mod is
//! installed here rather than by hand, in the shape it expects:
//!
//! ```text
//! Game\D3D12.dll                                 the loader
//! Game\ERSS-FG.dll                               the mod
//! Game\ERSS2\bin\*.dll                           DLSS, Streamline, FSR, XeSS
//! Game\ERSS2\addons\RemoveFrameTimeConstraint.dll  optional, from its own archive
//! Game\ERSSReShadeStub.addon                     only when ReShade is present
//! ```
//!
//! The loader is named `D3D12.dll` because that is the export table it stands
//! in for. That name also collides with the Steam overlay, and the mod's own
//! answer is to rename it `dxgi.dll` — which is why that is a choice here rather
//! than a fixed decision.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};

/// The loader, under both names it is allowed to have.
const LOADERS: &[&str] = &["D3D12.dll", "dxgi.dll"];
/// The mod itself, which is the honest test of whether it is installed.
const CORE: &str = "ERSS-FG.dll";
/// Only wanted when ReShade is there; it does nothing otherwise and confuses
/// anybody reading the folder.
const RESHADE_STUB: &str = "ERSSReShadeStub.addon";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErssStatus {
    pub installed: bool,
    /// Which name the loader is under, when it is there.
    pub loader: Option<String>,
    /// What the folder holds, read from the mod's own files.
    pub version: Option<String>,
    /// True when the frame-time addon is in as well.
    pub frame_time_addon: bool,
    /// Archives found on this machine, newest first.
    pub archives: Vec<PathBuf>,
    /// True when the game folder has ReShade, so the stub is wanted.
    pub reshade: bool,
    /// True when the release archive is locked and a password has to be typed in.
    pub needs_password: bool,
    /// Reasons the mod will not work until they are dealt with.
    pub blockers: Vec<String>,
}

/// Pulls `4.14.1` out of `ERSS-FG-v4.14.1-Release.7z`.
fn version_of(name: &str) -> Option<String> {
    let after = name.split("-v").nth(1)?;
    let digits: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (!digits.is_empty()).then(|| digits.trim_end_matches('.').to_string())
}

/// Orders `4.14.1` above `4.9.0`, which a string compare gets backwards.
fn version_key(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0))
        .collect()
}

/// What an archive on disk turns out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The mod itself.
    Main,
    /// The addon that lifts the frame-time constraint.
    FrameTime,
    /// The config that puts the overlay toggle on a controller.
    Overlay,
}

fn kind_of(name: &str) -> Option<Kind> {
    let lower = name.to_ascii_lowercase();
    if !lower.starts_with("erss") {
        return None;
    }
    if lower.contains("removeframetimeconstraint") {
        return Some(Kind::FrameTime);
    }
    if lower.contains("overlay-toggle") || lower.contains("overlay_toggle") {
        return Some(Kind::Overlay);
    }
    if lower.contains("erss-fg-v") || lower.contains("erss-fg_v") {
        return Some(Kind::Main);
    }
    None
}

/// Every ERSS archive sitting where a browser leaves downloads.
///
/// Newest first, and only one of each kind is ever needed — but all of them are
/// reported, because picking a version is the user's to make when a new one
/// turns out to be worse.
pub fn find_archives() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        folders.push(home.join("Downloads"));
        folders.push(home.join("Desktop"));
    }
    if let Some(downloads) = dirs::download_dir() {
        folders.push(downloads);
    }
    folders.sort();
    folders.dedup();

    let mut found: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let is_archive = [".7z", ".zip"]
                .iter()
                .any(|ext| name.to_ascii_lowercase().ends_with(ext));
            if !is_archive || kind_of(&name).is_none() {
                continue;
            }
            let key = version_of(&name).map(|v| version_key(&v)).unwrap_or_default();
            found.push((key, path));
        }
    }

    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, path)| path).collect()
}

/// The newest archive of each kind, which is what an install actually uses.
pub fn best_of_each(archives: &[PathBuf]) -> Vec<(Kind, PathBuf)> {
    let mut out: Vec<(Kind, PathBuf)> = Vec::new();
    for path in archives {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let Some(kind) = kind_of(&name) else {
            continue;
        };
        // `find_archives` already sorted newest first, so the first of a kind
        // wins and the rest are older releases kept around.
        if !out.iter().any(|(seen, _)| *seen == kind) {
            out.push((kind, path.clone()));
        }
    }
    out
}

/// True when ReShade is installed in the game folder.
fn has_reshade(game_dir: &Path) -> bool {
    ["ReShade.ini", "reshade-shaders", "ReShade64.dll"]
        .iter()
        .any(|name| game_dir.join(name).exists())
}

/// What is in the folder now.
pub fn status(game_dir: &Path, has_eac: bool, eac_bypassed: bool, hags: bool) -> ErssStatus {
    let loader = LOADERS
        .iter()
        .find(|name| game_dir.join(name).is_file())
        .map(|name| (*name).to_string());
    let installed = game_dir.join(CORE).is_file();

    let version = std::fs::read_dir(game_dir.join("ERSS2"))
        .ok()
        .and_then(|_| version_from_folder(game_dir));

    let mut blockers = Vec::new();
    if has_eac && !eac_bypassed {
        blockers.push(
            "The anti-cheat has to be off — the mod cannot load past it, and online play with \
             it loaded is a ban."
                .to_string(),
        );
    }
    if !hags {
        blockers.push(
            "Hardware-accelerated GPU scheduling has to be on, or DLSS frame generation will \
             not start."
                .to_string(),
        );
    }

    let archives = find_archives();
    let needs_password = best_of_each(&archives)
        .iter()
        .filter(|(kind, _)| *kind == Kind::Main)
        .any(|(_, path)| is_locked(path));

    ErssStatus {
        installed,
        loader,
        version,
        frame_time_addon: game_dir
            .join("ERSS2")
            .join("addons")
            .join("RemoveFrameTimeConstraint.dll")
            .is_file(),
        reshade: has_reshade(game_dir),
        needs_password,
        archives,
        blockers,
    }
}

/// True when the archive's contents are encrypted.
///
/// Reading the header is enough and costs nothing: the file names are in the
/// clear even when every stream behind them is not, which is why the listing
/// looks fine right up until an unpack fails.
fn is_locked(archive: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(archive) else {
        return false;
    };
    let Ok(read) = sevenz_rust2::Archive::read(&mut file, &sevenz_rust2::Password::empty()) else {
        // A header that will not open without a password is itself the answer.
        return true;
    };
    read.blocks.iter().any(|block| {
        block
            .coders
            .iter()
            .any(|coder| coder.encoder_method_id() == sevenz_rust2::EncoderMethod::ID_AES256_SHA256)
    })
}

/// Reads the installed version out of the marker Roundtable leaves.
///
/// The mod's own files carry no version, so the one that was installed is
/// written down at install time. Guessing from file sizes would be worse than
/// saying nothing.
fn version_from_folder(game_dir: &Path) -> Option<String> {
    std::fs::read_to_string(game_dir.join("ERSS2").join(".roundtable-version"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Unpacks the mod into the game folder.
///
/// `steam_overlay` renames the loader, because `D3D12.dll` and the Steam overlay
/// cannot both have that name and the mod's answer is to stand in for `dxgi`
/// instead.
pub fn install(
    game_dir: &Path,
    archives: &[PathBuf],
    steam_overlay: bool,
    password: Option<&str>,
) -> Result<Vec<String>> {
    if !game_dir.is_dir() {
        return Err(Error::msg("the game folder is not there".to_string()));
    }

    let chosen = best_of_each(archives);
    if !chosen.iter().any(|(kind, _)| *kind == Kind::Main) {
        return Err(Error::msg(
            "no ERSS release archive found. Download one and it will be picked up here."
                .to_string(),
        ));
    }

    // Unpacked to one side first: the game folder is 45 GB of somebody's install
    // and a half-written mod in it is worse than no mod.
    //
    // The path carries a counter as well as the process, because two installs at
    // once sharing a scratch folder means one deletes what the other is reading.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let staging = std::env::temp_dir().join(format!(
        "roundtable-erss-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::remove_dir_all(&staging).ok();
    std::fs::create_dir_all(&staging).at(&staging)?;

    let mut done = Vec::new();
    let mut version = None;

    for (kind, archive) in &chosen {
        let name = archive
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().to_string());
        let into = staging.join(match kind {
            Kind::Main => "main",
            Kind::FrameTime => "addon",
            Kind::Overlay => "overlay",
        });
        std::fs::create_dir_all(&into).at(&into)?;

        if name.to_ascii_lowercase().ends_with(".zip") {
            unzip(archive, &into)?;
        } else {
            // The releases are locked: every file inside is AES-encrypted and the
            // password comes with the post they were downloaded from. Nothing
            // here stores it — it is used for this unpack and then gone.
            let result = match password {
                Some(password) => sevenz_rust2::decompress_file_with_password(
                    archive,
                    &into,
                    sevenz_rust2::Password::from(password),
                ),
                None => sevenz_rust2::decompress_file(archive, &into),
            };
            result.map_err(|error| match error {
                sevenz_rust2::Error::PasswordRequired => Error::msg(
                    "This release is locked. Paste the password from the post you \
                     downloaded it from."
                        .to_string(),
                ),
                sevenz_rust2::Error::MaybeBadPassword(_) => {
                    Error::msg("That password did not open the archive.".to_string())
                }
                other => Error::Archive(other.to_string()),
            })?;
        }

        if *kind == Kind::Main {
            version = version_of(&name);
        }
        done.push(name);
    }

    let reshade = has_reshade(game_dir);
    let mut written: Vec<String> = Vec::new();

    for source in [
        staging.join("main"),
        staging.join("addon"),
        staging.join("overlay"),
    ] {
        if !source.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&source)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            let Ok(relative) = entry.path().strip_prefix(&source) else {
                continue;
            };
            if relative.as_os_str().is_empty() {
                continue;
            }
            let leaf = relative
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().to_string());

            // The stub is only meaningful with ReShade, and a stray .addon file
            // in the folder is the kind of thing that gets blamed for a crash a
            // year later.
            if leaf == RESHADE_STUB && !reshade {
                continue;
            }
            // The mod's own readme is not wanted in a 45 GB game folder.
            if leaf.eq_ignore_ascii_case("README.txt") {
                continue;
            }

            let mut target = game_dir.join(relative);
            if steam_overlay && leaf.eq_ignore_ascii_case("D3D12.dll") {
                target = game_dir.join("dxgi.dll");
            }

            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target).at(&target)?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            std::fs::copy(entry.path(), &target).at(&target)?;
            written.push(
                target
                    .strip_prefix(game_dir)
                    .unwrap_or(&target)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }

    // Both loader names at once means the game loads one and the other sits
    // there shadowing it.
    let other = if steam_overlay { "D3D12.dll" } else { "dxgi.dll" };
    let stale = game_dir.join(other);
    if stale.is_file() && written.iter().any(|w| w.eq_ignore_ascii_case(other)) {
        // Only if this install did not just write it.
    } else if stale.is_file() {
        std::fs::remove_file(&stale).at(&stale)?;
        written.push(format!("removed the other loader, {other}"));
    }

    if let Some(version) = &version {
        let marker = game_dir.join("ERSS2").join(".roundtable-version");
        if let Some(parent) = marker.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        let _ = std::fs::write(&marker, version);
    }

    std::fs::remove_dir_all(&staging).ok();

    let mut report = vec![format!(
        "ERSS {} installed from {}",
        version.as_deref().unwrap_or("(unknown version)"),
        done.join(", ")
    )];
    report.push(format!("{} files written", written.len()));
    if !reshade {
        report.push("ReShade stub skipped — no ReShade here".into());
    }
    Ok(report)
}

/// Takes it back out.
pub fn uninstall(game_dir: &Path) -> Result<Vec<String>> {
    let mut removed = Vec::new();

    for name in LOADERS {
        let path = game_dir.join(name);
        // Only the mod's loader, never a real system DLL somebody put here.
        if path.is_file() && game_dir.join(CORE).is_file() {
            std::fs::remove_file(&path).at(&path)?;
            removed.push((*name).to_string());
        }
    }
    for name in [CORE, RESHADE_STUB, "ERSS-FG.toml"] {
        let path = game_dir.join(name);
        if path.is_file() {
            std::fs::remove_file(&path).at(&path)?;
            removed.push(name.to_string());
        }
    }
    let folder = game_dir.join("ERSS2");
    if folder.is_dir() {
        std::fs::remove_dir_all(&folder).at(&folder)?;
        removed.push("ERSS2".to_string());
    }

    if removed.is_empty() {
        return Err(Error::msg("ERSS is not installed here".to_string()));
    }
    Ok(removed)
}

fn unzip(archive: &Path, destination: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).at(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Archive(e.to_string()))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| Error::Archive(e.to_string()))?;
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).at(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        let mut out = std::fs::File::create(&target).at(&target)?;
        std::io::copy(&mut entry, &mut out).at(&target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-erss-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Builds the archive in the shape the real release has.
    fn release(at: &Path, files: &[&str]) {
        let file = std::fs::File::create(at).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for name in files {
            zip.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut zip, name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn a_release_name_gives_up_its_version() {
        assert_eq!(version_of("ERSS-FG-v4.14.1-Release.7z").as_deref(), Some("4.14.1"));
        assert_eq!(version_of("ERSS-FG-v4.12.0-Release.7z").as_deref(), Some("4.12.0"));
        assert_eq!(
            version_of("ERSS-Addon-RemoveFrameTimeConstraint-v0.1.0-Release.7z").as_deref(),
            Some("0.1.0")
        );
        assert_eq!(version_of("ERSS-FG-controller-overlay-toggle.zip"), None);
    }

    #[test]
    fn versions_order_by_number_and_not_by_text() {
        // 4.9 sorts above 4.14 as text, which would install the older release.
        assert!(version_key("4.14.1") > version_key("4.9.0"));
        assert!(version_key("4.14.1") > version_key("4.14.0"));
        assert!(version_key("4.13.0") > version_key("4.12.0"));
    }

    #[test]
    fn each_archive_is_recognised_for_what_it_is() {
        assert_eq!(kind_of("ERSS-FG-v4.14.1-Release.7z"), Some(Kind::Main));
        assert_eq!(
            kind_of("ERSS-Addon-RemoveFrameTimeConstraint-v0.1.0-Release.7z"),
            Some(Kind::FrameTime)
        );
        assert_eq!(kind_of("ERSS-FG-controller-overlay-toggle.zip"), Some(Kind::Overlay));
        assert_eq!(kind_of("ConvergenceER 3.0.1.zip"), None);
        assert_eq!(kind_of("seamless-coop.zip"), None);
    }

    #[test]
    fn the_newest_of_each_kind_is_the_one_installed() {
        // The user keeps every release they have downloaded.
        let dir = temp("pick");
        let names = [
            "ERSS-FG-v4.12.0-Release.7z",
            "ERSS-FG-v4.14.1-Release.7z",
            "ERSS-FG-v4.13.0-Release.7z",
            "ERSS-Addon-RemoveFrameTimeConstraint-v0.1.0-Release.7z",
        ];
        let mut paths: Vec<(Vec<u32>, PathBuf)> = names
            .iter()
            .map(|n| (version_key(&version_of(n).unwrap()), dir.join(n)))
            .collect();
        paths.sort_by(|a, b| b.0.cmp(&a.0));
        let sorted: Vec<PathBuf> = paths.into_iter().map(|(_, p)| p).collect();

        let best = best_of_each(&sorted);
        let main = best.iter().find(|(k, _)| *k == Kind::Main).unwrap();
        assert!(
            main.1.to_string_lossy().contains("4.14.1"),
            "got {:?}",
            main.1
        );
        assert!(best.iter().any(|(k, _)| *k == Kind::FrameTime));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_mod_lands_in_the_shape_the_loader_expects() {
        let dir = temp("install");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(
            &main,
            &[
                "D3D12.dll",
                "ERSS-FG.dll",
                "ERSSReShadeStub.addon",
                "ERSS2/bin/nvngx_dlss.dll",
                "ERSS2/bin/sl.reflex.dll",
            ],
        );
        let addon = dir.join("ERSS-Addon-RemoveFrameTimeConstraint-v0.1.0-Release.zip");
        release(&addon, &["ERSS2/addons/RemoveFrameTimeConstraint.dll"]);

        install(&game, &[main, addon], false, None).unwrap();

        assert!(game.join("D3D12.dll").is_file(), "the loader");
        assert!(game.join("ERSS-FG.dll").is_file(), "the mod");
        assert!(game.join("ERSS2").join("bin").join("nvngx_dlss.dll").is_file());
        assert!(game
            .join("ERSS2")
            .join("addons")
            .join("RemoveFrameTimeConstraint.dll")
            .is_file());
        assert!(
            !game.join(RESHADE_STUB).exists(),
            "the stub does nothing without ReShade and only confuses the folder"
        );

        let found = status(&game, true, true, true);
        assert!(found.installed);
        assert_eq!(found.loader.as_deref(), Some("D3D12.dll"));
        assert_eq!(found.version.as_deref(), Some("4.14.1"));
        assert!(found.frame_time_addon);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_reshade_stub_goes_in_when_reshade_is_there() {
        let dir = temp("reshade");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("ReShade.ini"), b"x").unwrap();

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll", "ERSSReShadeStub.addon"]);
        install(&game, &[main], false, None).unwrap();

        assert!(game.join(RESHADE_STUB).is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_loader_is_renamed_when_the_steam_overlay_is_wanted() {
        // Both names at once means one shadows the other.
        let dir = temp("overlay");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll"]);
        install(&game, &[main], true, None).unwrap();

        assert!(game.join("dxgi.dll").is_file(), "renamed for the overlay");
        assert!(!game.join("D3D12.dll").exists(), "and not left behind as well");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn switching_loader_names_does_not_leave_the_old_one() {
        let dir = temp("switch");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll"]);

        install(&game, &[main.clone()], false, None).unwrap();
        assert!(game.join("D3D12.dll").is_file());

        install(&game, &[main], true, None).unwrap();
        assert!(game.join("dxgi.dll").is_file());
        assert!(
            !game.join("D3D12.dll").exists(),
            "the old name has to go or the game loads the wrong one"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn what_stops_it_working_is_said_before_it_is_installed() {
        let dir = temp("blockers");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();

        let armed = status(&game, true, false, false);
        assert_eq!(armed.blockers.len(), 2, "anti-cheat and GPU scheduling");
        assert!(armed.blockers.iter().any(|b| b.contains("anti-cheat")));
        assert!(armed.blockers.iter().any(|b| b.contains("scheduling")));

        let ready = status(&game, true, true, true);
        assert!(ready.blockers.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn taking_it_out_leaves_the_folder_as_it_was() {
        let dir = temp("uninstall");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        // Something of the game's own, which must survive.
        std::fs::write(game.join("eldenring.exe"), b"game").unwrap();

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll", "ERSS2/bin/nvngx_dlss.dll"]);
        install(&game, &[main], false, None).unwrap();

        uninstall(&game).unwrap();
        assert!(!game.join("D3D12.dll").exists());
        assert!(!game.join("ERSS-FG.dll").exists());
        assert!(!game.join("ERSS2").exists());
        assert!(game.join("eldenring.exe").is_file(), "the game is untouched");
        assert!(!status(&game, true, true, true).installed);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_install_with_no_release_archive_is_refused() {
        let dir = temp("nothing");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        let addon = dir.join("ERSS-Addon-RemoveFrameTimeConstraint-v0.1.0-Release.zip");
        release(&addon, &["ERSS2/addons/RemoveFrameTimeConstraint.dll"]);

        // The addon on its own does nothing without the mod it adds to.
        assert!(install(&game, &[addon], false, None).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
