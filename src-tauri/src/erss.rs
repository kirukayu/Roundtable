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

/// The password the author publishes beside the download.
///
/// Some releases go out AES-encrypted and the post that links them prints the
/// password two lines further down — it is there to stop reuploaders and
/// scanners, not the person who just downloaded the file. Trying it first is
/// what that person would do; the field is still there when a release changes
/// it.
const PUBLISHED_PASSWORD: &str = "huutaiii";

/// Executable versions the mod loads against: game 1.16, 1.14 and 1.13.
const SUPPORTED_BUILDS: &[&str] = &["2.6", "2.4", "2.3"];

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
    /// The one that will be installed: the newest release found.
    pub release: Option<PathBuf>,
    /// True when the game folder has ReShade, so the stub is wanted.
    pub reshade: bool,
    /// True when that release is encrypted. The published password is tried
    /// automatically, so this is a note rather than a demand.
    pub locked: bool,
    /// Reasons the mod will not work until they are dealt with.
    pub blockers: Vec<String>,
    /// Its own settings, once it has run once and written them.
    pub settings: Vec<Setting>,
    /// What its log says happened the last time the game ran.
    pub last_launch: Vec<String>,
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

/// Every archive of each kind, newest first, as things to try in order.
///
/// A list rather than one choice, because whether an archive is any use cannot
/// be told by looking at it. Releases are usually published in the clear, but
/// some go out with every file inside AES-encrypted — 4.14.1 did — and a
/// half-finished download looks exactly like a good one until it is unpacked.
/// Predicting which is which means being wrong quietly; trying the newest and
/// falling back means being right.
pub fn candidates(archives: &[PathBuf]) -> Vec<(Kind, Vec<PathBuf>)> {
    let mut out: Vec<(Kind, Vec<PathBuf>)> = Vec::new();
    for path in archives {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let Some(kind) = kind_of(&name) else {
            continue;
        };
        // `find_archives` already sorted newest first, so pushing keeps that.
        match out.iter_mut().find(|(seen, _)| *seen == kind) {
            Some((_, paths)) => paths.push(path.clone()),
            None => out.push((kind, vec![path.clone()])),
        }
    }
    out
}

/// The one that will be installed, which is simply the newest of its kind.
pub fn chosen(archives: &[PathBuf], kind: Kind) -> Option<PathBuf> {
    candidates(archives)
        .into_iter()
        .find(|(seen, _)| *seen == kind)
        .and_then(|(_, paths)| paths.into_iter().next())
}

/// True when ReShade is installed in the game folder.
fn has_reshade(game_dir: &Path) -> bool {
    ["ReShade.ini", "reshade-shaders", "ReShade64.dll"]
        .iter()
        .any(|name| game_dir.join(name).exists())
}

/// Everything the mod's own instructions insist on, checked before it is blamed.
///
/// Each of these is in the author's post as a prerequisite or a known issue, and
/// each of them produces a symptom that looks like something else: no overlay,
/// tearing, a crash on minimise, a game that will not start. Reading them off
/// the machine beats reading them off a page.
pub fn preflight(
    game_dir: &Path,
    build: Option<&str>,
    has_eac: bool,
    eac_bypassed: bool,
    hags: bool,
    screen_mode: Option<&str>,
    game_res: Option<(u32, u32)>,
    display_res: Option<(u32, u32)>,
) -> Vec<String> {
    let mut notes = Vec::new();

    if has_eac && !eac_bypassed {
        notes.push(
            "The anti-cheat has to be off — the mod cannot load past it, and online play \
             with it loaded is a ban."
                .to_string(),
        );
    }

    // "Expect screen tearing and stutters while using framegen with HwSch
    // disabled" — the author's words, and it is a prerequisite not a suggestion.
    if !hags {
        notes.push(
            "Hardware-accelerated GPU scheduling is off. Frame generation tears and \
             stutters without it. Optimise turns it on; Windows needs a restart after."
                .to_string(),
        );
    }

    // "Requires ELDEN RING version 1.16, 1.14 or 1.13", in the author's words.
    // The executable reports the internal number rather than the one on the
    // patch notes, and the two run six apart — 1.16.1 is file version 2.6.1.0,
    // verified on this machine. So the three supported releases are 2.6, 2.4 and
    // 2.3, and anything else is worth saying before the mod is blamed.
    if let Some(build) = build {
        if !SUPPORTED_BUILDS.iter().any(|ok| build.starts_with(ok)) {
            notes.push(format!(
                "This mod supports ELDEN RING 1.16, 1.14 and 1.13, and this copy reports \
                 build {build}. It will most likely refuse to load."
            ));
        }
    }

    // "Set Resolution in the game's Graphics menu to your target resolution" —
    // upscaling from a resolution below the desktop is upscaling twice.
    if let (Some((gw, gh)), Some((dw, dh))) = (game_res, display_res) {
        if gw != dw || gh != dh {
            notes.push(format!(
                "The game renders at {gw}x{gh} and your screen is {dw}x{dh}. Set the \
                 game to your screen's resolution or the upscaler works from the wrong \
                 size — Optimise does this."
            ));
        }
    }

    // "Minimizing while FSR-FG is enabled crashes the game", and in full screen
    // "the game will repeatedly try to enter exclusive full-screen mode causing
    // screen flashes". Borderless is the answer to both, and to HDR.
    if screen_mode.is_some_and(|mode| mode.eq_ignore_ascii_case("FULLSCREEN")) {
        notes.push(
            "The game is in exclusive fullscreen. The mod's overlay can be invisible \
             there, HDR will not work, and FSR frame generation flashes. Borderless \
             fixes all three."
                .to_string(),
        );
    }

    // The loader stands in for D3D12, and so does every other mod loader.
    for rival in ["dinput8.dll", "SpecialK64.dll", "modengine2", "me3"] {
        if game_dir.join(rival).exists() {
            notes.push(format!(
                "{rival} is in the game folder. Two loaders both standing in for D3D12 \
                 is the usual reason neither loads — the mod's post covers the order \
                 they need."
            ));
        }
    }

    notes
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
    let release = chosen(&archives, Kind::Main);
    // Encrypted, which is not the same as needing anything typed — the password
    // the author publishes is tried on its own, and proving here that it works
    // would mean decompressing seventy megabytes on every refresh. So this says
    // the release is locked and the field stays an override for the day a
    // release changes it.
    let locked = release.as_deref().is_some_and(is_locked);

    ErssStatus {
        installed,
        loader,
        version,
        release,
        frame_time_addon: game_dir
            .join("ERSS2")
            .join("addons")
            .join("RemoveFrameTimeConstraint.dll")
            .is_file(),
        reshade: has_reshade(game_dir),
        locked,
        archives,
        blockers,
        settings: settings(game_dir),
        last_launch: last_launch(game_dir),
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

    let wanted = candidates(archives);
    if !wanted.iter().any(|(kind, _)| *kind == Kind::Main) {
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

    for (kind, releases) in &wanted {
        // The alternative-toggle download is not files, it is three settings —
        // and it puts its `ERSS-FG.toml` in the game root, where the mod does
        // not look, contradicting the real one in `ERSS2` for anybody who later
        // goes reading. Roundtable writes those same settings itself and they
        // are editable in the launcher, so the archive is left where it is.
        if *kind == Kind::Overlay {
            continue;
        }
        let into = staging.join(match kind {
            Kind::Main => "main",
            Kind::FrameTime => "addon",
            Kind::Overlay => "overlay",
        });

        // Newest first, and down the list until one comes out whole. A locked
        // release, a download that stopped halfway, a file half-eaten by a
        // scanner — all of them fail here and none of them should be the end of
        // it while an older release sits in the same folder.
        let mut last: Option<Error> = None;
        let mut opened = None;
        for archive in releases {
            let name = archive
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().to_string());
            std::fs::remove_dir_all(&into).ok();
            std::fs::create_dir_all(&into).at(&into)?;

            match unpack(archive, &into, password) {
                Ok(()) => {
                    opened = Some(name);
                    break;
                }
                Err(error) => last = Some(error),
            }
        }

        let Some(name) = opened else {
            // The addon and the overlay config are extras; only the mod itself
            // failing is worth stopping for. The half-written folder goes either
            // way so nothing from it reaches the game.
            std::fs::remove_dir_all(&into).ok();
            if *kind == Kind::Main {
                std::fs::remove_dir_all(&staging).ok();
                return Err(last.unwrap_or_else(|| {
                    Error::msg("no ERSS release archive could be opened".to_string())
                }));
            }
            continue;
        };

        if *kind == Kind::Main {
            version = version_of(&name);
        }
        done.push(name);
    }

    let reshade = has_reshade(game_dir);
    let mut written: Vec<String> = Vec::new();
    // Kept apart from the report lines so the read-back below has real paths to
    // check rather than sentences.
    let mut files: Vec<PathBuf> = Vec::new();

    for source in [staging.join("main"), staging.join("addon")] {
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
            files.push(target);
        }
    }

    // A config in the game root is inert — the mod reads the one in `ERSS2` —
    // and two files of the same name saying different things is worse than one
    // saying nothing. Earlier versions of Roundtable put one here.
    let stray = game_dir.join("ERSS-FG.toml");
    if stray.is_file() {
        std::fs::remove_file(&stray).ok();
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

    // Written is not the same as still there.
    //
    // The author warns that Defender takes these for malware, and a real-time
    // scanner deletes a file seconds after it lands. Reporting a successful
    // install and leaving the player to find out at the title screen is the
    // worst of both, so every file this install copied is read back — the files
    // themselves rather than a list of names, which would go stale the first
    // time a release drops one.
    // Looked at twice, a moment apart.
    //
    // A real-time scanner holds a file it is examining, and a file called
    // `D3D12.dll` that has just appeared is exactly what it examines hardest —
    // so a single check reports it missing while it is merely busy. A file
    // Defender has actually taken is gone and stays gone, which is what the
    // second look distinguishes. Without this the check fired at random on
    // files that were perfectly fine.
    let mut missing = absent(game_dir, &files);
    if !missing.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(700));
        missing = absent(game_dir, &files);
    }

    let mut report = vec![format!(
        "ERSS {} installed from {}",
        version.as_deref().unwrap_or("(unknown version)"),
        done.join(", ")
    )];
    report.push(format!("{} files written", written.len()));
    if !reshade {
        report.push("ReShade stub skipped — no ReShade here".into());
    }
    if seed(game_dir).unwrap_or(false) {
        report.push("settings started, so they can be changed before the first launch".into());
    }

    if !missing.is_empty() {
        return Err(Error::msg(format!(
            "{} went missing right after being written — Defender takes these for \
             malware. Exclude the game folder in Windows Security and install again.",
            missing.join(", ")
        )));
    }

    Ok(report)
}

/// What the mod reported doing the last time the game ran.
///
/// Its settings say what was asked for; this says what happened. They come
/// apart often enough to matter — a generator that is set in the config but not
/// supported by the card is simply not enabled, and the config still says it is.
pub fn last_launch(game_dir: &Path) -> Vec<String> {
    let Ok(log) = std::fs::read_to_string(game_dir.join("ERSS2").join("ERSS-FG.log")) else {
        return Vec::new();
    };

    let mut said = Vec::new();
    for line in log.lines() {
        let Some((_, text)) = line.split_once(" - ") else {
            continue;
        };
        let text = text.trim();
        let lower = text.to_ascii_lowercase();
        let worth = lower.starts_with("enabling")
            || lower.starts_with("adapter:")
            || lower.contains("is supported")
            || lower.contains("not supported")
            || lower.contains("failed")
            || lower.contains("monitor refresh");
        if worth && !said.contains(&text.to_string()) {
            said.push(text.to_string());
        }
    }
    said.truncate(6);
    said
}

/// Which of these are not on disk, named the short way.
fn absent(game_dir: &Path, files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .filter(|path| !path.is_file())
        .map(|path| {
            path.strip_prefix(game_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string()
        })
        .collect()
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

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Where the mod keeps its settings, once it has run once.
fn config_path(game_dir: &Path) -> PathBuf {
    game_dir.join("ERSS2").join("ERSS-FG.toml")
}

/// A setting Roundtable knows how to present, addressed the way the file holds it.
///
/// `path` is `Section.Key`, or a bare key for the handful that sit at the top of
/// the file. That distinction is the whole reason this pane did not work: the
/// mod keeps almost everything in sections — `[Renderer]`, `[DLSS]`,
/// `[FrameGeneration]` — and the first version of this code read and wrote only
/// top-level keys. So the pane showed four settings that did not matter, and
/// writing "DLSSMode" created a stray top-level key beside the real
/// `[DLSS] DLSSMode`, which the mod went on ignoring. The player changed things
/// in the launcher and nothing happened, which is exactly what they reported.
struct Known {
    path: &'static str,
    title: &'static str,
    detail: &'static str,
    /// Value and label, for a setting that is one of a set.
    choices: &'static [(&'static str, &'static str)],
    /// True when the game has to be restarted for it to take effect.
    restart: bool,
}

/// The upscaler, named as the file's own section names name it.
///
/// `[Renderer] ScalingMode` is a string and the sections beneath it are `[NIS]`,
/// `[DLSS]`, `[FSR3U]` and `[XESS]` — a config almost always names its sections
/// after the values that select them, and these are the four the mod's own post
/// lists as its upscalers.
const UPSCALERS: &[(&str, &str)] = &[
    ("None", "Off"),
    ("DLSS", "DLSS 4"),
    ("FSR3U", "FSR 3.1"),
    ("XESS", "XeSS"),
];

/// DLSS quality, as the raw NVIDIA NGX value the mod stores.
///
/// Not a string: the mod writes `NVSDK_NGX_PerfQuality_Value` straight out, so 0
/// is Performance and 2 is Quality — which reads backwards to anybody editing
/// the file by hand, and is the whole reason these are buttons with names.
const DLSS_MODES: &[(&str, &str)] = &[
    ("5", "DLAA"),
    ("2", "Quality"),
    ("1", "Balanced"),
    ("0", "Performance"),
    ("3", "Ultra performance"),
];

/// Which frame generator, if any.
///
/// 2 is DLSS, which is not a guess: the player picked DLSS frame generation in
/// the mod's own overlay, the config came back `FrameGenMode = 2`, and the log
/// line for that launch reads "Enabling DLSS-G". This table had it the other way
/// round and the launcher would have named the wrong generator.
const FRAMEGEN: &[(&str, &str)] = &[("0", "Off"), ("1", "FSR 3"), ("2", "DLSS")];

/// How many frames it makes from each real one.
const MULTIPLIER: &[(&str, &str)] = &[("1", "×2"), ("2", "×3"), ("3", "×4")];

/// Reflex, which is what gives the latency back that generation costs.
const LATENCY: &[(&str, &str)] = &[("0", "Off"), ("1", "On"), ("2", "On + boost")];

/// The settings worth putting in front of somebody, in the order they matter.
const KNOWN: &[Known] = &[
    Known {
        path: "Renderer.ScalingMode",
        title: "Upscaler",
        detail: "Renders below your resolution and reconstructs. Separate from frame \
                 generation — you can have one without the other.",
        choices: UPSCALERS,
        restart: false,
    },
    Known {
        path: "DLSS.DLSSMode",
        title: "DLSS quality",
        detail: "How far below your resolution it renders. DLAA does not scale down at \
                 all — no speed, the best picture, and nothing to argue about.",
        choices: DLSS_MODES,
        restart: false,
    },
    Known {
        path: "FrameGeneration.FrameGenMode",
        title: "Frame generation",
        detail: "Inserts generated frames between the real ones. Needs hardware GPU \
                 scheduling, and a steady base rate to look right.",
        choices: FRAMEGEN,
        restart: true,
    },
    Known {
        path: "DLSS-G.NumGenFrames",
        title: "Generated frames",
        detail: "How many frames are made from each real one. More is smoother and adds \
                 latency; ×2 is the safe one.",
        choices: MULTIPLIER,
        restart: true,
    },
    Known {
        path: "FrameGeneration.GIGlitchMitigation",
        title: "Global illumination fix",
        detail: "The lighting flickers in shaded areas with frames being generated. This \
                 is the mod author's own workaround and it belongs on.",
        choices: &[("0", "Off"), ("1", "On"), ("2", "Strong")],
        restart: false,
    },
    Known {
        path: "Renderer.LatencyReductionMode",
        title: "Latency reduction",
        detail: "Reflex on NVIDIA, Anti-Lag on AMD. Generation adds a frame of delay and \
                 this is what takes it back off.",
        choices: LATENCY,
        restart: false,
    },
    Known {
        path: "Renderer.MaxFPS",
        title: "Frame limit",
        detail: "Counted in finished frames, so with generation on a limit of 60 means 60 \
                 on screen and 30 rendered. Zero is no limit.",
        choices: &[],
        restart: false,
    },
    Known {
        path: "Renderer.RemoveFPSLimit",
        title: "Remove the sixty cap",
        detail: "The game's own limit, lifted. Leave this on — it is what the mod is for.",
        choices: &[],
        restart: false,
    },
    Known {
        path: "DLSS.Sharpness",
        title: "Sharpening",
        detail: "Applied after upscaling. Low or none: past halfway it draws a bright \
                 outline around everything, which is an artefact of its own.",
        choices: &[],
        restart: false,
    },
    Known {
        path: "Renderer.IsHDR",
        title: "HDR",
        detail: "Works in borderless, which is where this game should be anyway. Only \
                 useful on a display that has it.",
        choices: &[],
        restart: false,
    },
    Known {
        path: "SwapChain.Force10Bit",
        title: "Force 10-bit output",
        detail: "For a display that supports HDR but that the game hands 8 bits anyway. \
                 Without it the mod's HDR has nothing to write into.",
        choices: &[],
        restart: true,
    },
    Known {
        path: "OverlayToggleKey",
        title: "Its overlay key",
        detail: "Opens the mod's own settings in game. Everything worth changing is on \
                 this page, so this is mostly for turning it off.",
        choices: &[],
        restart: false,
    },
    Known {
        path: "ImGuiUseGamepadToggle",
        title: "Both sticks open it",
        detail: "Clicking both thumbsticks opens the mod's overlay. Worth turning off if \
                 you never want to see it, since it is easy to hit by accident.",
        choices: &[],
        restart: false,
    },
];

/// Settings that stop the mod announcing itself.
///
/// The player's complaint is exact: they do not want the people they hand this
/// to knowing there is a mod, and they certainly do not want a "press Home to
/// close" banner over the game. None of that is necessary any more — everything
/// the overlay offers is on this page — so the overlay can simply be shut.
///
/// `DatePopup` is the mod's own record of when it last showed its notice.
/// Setting it forward is how the mod itself remembers that the notice has been
/// seen, which is why this is a date and not a flag.
const QUIET: &[(&str, &str)] = &[
    // F24 exists in the virtual-key table and on almost no keyboard, so there
    // is nothing to press by accident.
    //
    // "None" was tried first and the mod rejected it: the file came back with
    // the player's previous key restored, which is what a parse failure looks
    // like here. A key it can parse and a person cannot reach is the way to do
    // this.
    ("OverlayToggleKey", "F24"),
    ("ImGuiUseGamepadToggle", "false"),
    ("ShowMetricsWindow", "false"),
    ("ShowAdvancedSettingsWindow", "false"),
    ("bIsFPSUnlockWarningAccepted", "true"),
];

/// Shuts the mod's overlay and its notices.
///
/// Returns what it changed. A setting the installed version does not have is
/// skipped rather than created — a key the mod never reads is exactly the mess
/// this pane was built to clear up.
pub fn quieten(game_dir: &Path) -> Vec<String> {
    let mut done = Vec::new();
    for (key, value) in QUIET {
        if set_setting(game_dir, key, value).is_ok() {
            done.push((*key).to_string());
        }
    }

    // The notice is dated rather than flagged: the mod shows it when its own
    // record is older than the build. Today's date is what "already seen" looks
    // like to it.
    let today = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    if set_setting(game_dir, "DatePopup", &today).is_ok() {
        done.push("DatePopup".into());
    }
    done
}

fn described(path: &str) -> Option<usize> {
    KNOWN.iter().position(|k| k.path.eq_ignore_ascii_case(path))
}

/// One value a setting can take, and what to call it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    /// `Section.Key`, or a bare key for the top of the file.
    pub key: String,
    pub title: String,
    pub detail: String,
    /// The value as TOML holds it: `true`, `0.5`, `DLSS`.
    pub value: String,
    /// `bool`, `number` or `text`, so the interface knows what to draw.
    pub kind: String,
    pub choices: Vec<Choice>,
    /// False when Roundtable has nothing to say about this key beyond its name.
    pub described: bool,
    /// True when the game has to be restarted for it to take effect.
    pub restart: bool,
}

/// Reads the mod's settings, sections and all.
pub fn settings(game_dir: &Path) -> Vec<Setting> {
    let Ok(text) = std::fs::read_to_string(config_path(game_dir)) else {
        return Vec::new();
    };
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut take = |path: String, value: &toml_edit::Value| {
        let known = described(&path).map(|at| &KNOWN[at]);
        let kind = if value.is_bool() {
            "bool"
        } else if value.is_integer() || value.is_float() {
            "number"
        } else if value.is_datetime() {
            // A timestamp the mod keeps for itself. There is nothing sensible
            // to offer a player here, so it is not shown — but it is still
            // writable, because one of them is how the mod remembers that its
            // startup notice has been seen.
            return;
        } else {
            "text"
        };

        out.push(Setting {
            title: known.map_or_else(|| spaced(&path), |k| k.title.to_string()),
            detail: known.map_or_else(String::new, |k| k.detail.to_string()),
            value: value.to_string().trim().trim_matches('"').to_string(),
            kind: kind.to_string(),
            choices: known
                .map(|k| {
                    k.choices
                        .iter()
                        .map(|(value, label)| Choice {
                            value: (*value).to_string(),
                            label: (*label).to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            described: known.is_some(),
            restart: known.is_some_and(|k| k.restart),
            key: path,
        });
    };

    for (key, item) in doc.as_table().iter() {
        match item {
            toml_edit::Item::Value(value) => take(key.to_string(), value),
            toml_edit::Item::Table(table) => {
                for (inner, item) in table.iter() {
                    if let Some(value) = item.as_value() {
                        take(format!("{key}.{inner}"), value);
                    }
                }
            }
            _ => {}
        }
    }

    // Described settings first and in the order above, which is the order
    // somebody would change them in. The rest keep the file's order.
    out.sort_by_key(|s| described(&s.key).unwrap_or(usize::MAX));
    out
}

/// The item at a dotted path, if the file has one.
fn at<'a>(doc: &'a toml_edit::DocumentMut, path: &str) -> Option<&'a toml_edit::Value> {
    match path.split_once('.') {
        Some((section, key)) => doc.get(section)?.as_table()?.get(key)?.as_value(),
        None => doc.get(path)?.as_value(),
    }
}

/// Writes a value at a dotted path, refusing to create anything new.
///
/// Refusing matters. The mod writes this file itself and reads only the keys it
/// knows; a key invented here would sit in the file forever doing nothing, which
/// is precisely how the stray top-level `DLSSMode` came to exist beside the real
/// `[DLSS] DLSSMode` and precisely why nothing the player changed took effect.
fn put(doc: &mut toml_edit::DocumentMut, path: &str, value: toml_edit::Value) -> Result<()> {
    match path.split_once('.') {
        Some((section, key)) => {
            let table = doc
                .get_mut(section)
                .and_then(|item| item.as_table_mut())
                .ok_or_else(|| Error::msg(format!("its settings have no [{section}] section")))?;
            if !table.contains_key(key) {
                return Err(Error::msg(format!("{path} is not one of its settings")));
            }
            table[key] = toml_edit::value(value);
        }
        None => {
            if !doc.as_table().contains_key(path) {
                return Err(Error::msg(format!("{path} is not one of its settings")));
            }
            doc[path] = toml_edit::value(value);
        }
    }
    Ok(())
}

/// Writes one setting back, leaving the rest of the file exactly as it was.
///
/// `toml_edit` rather than a parse-and-serialise round trip: the mod writes this
/// file too, and handing it back reformatted with its comments stripped is how a
/// launcher earns a bug report it did not cause.
pub fn set_setting(game_dir: &Path, key: &str, value: &str) -> Result<String> {
    let path = config_path(game_dir);
    let text = std::fs::read_to_string(&path).map_err(|_| {
        Error::msg("the mod has not written its settings yet — run the game once".to_string())
    })?;

    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| Error::msg(format!("its settings file will not parse: {e}")))?;

    // Typed the way the existing value is typed, so a boolean does not become
    // the string "true" and get ignored, and a float does not become an integer
    // the mod refuses to read.
    let existing = at(&doc, key)
        .ok_or_else(|| Error::msg(format!("{key} is not one of its settings")))?
        .clone();

    let wanted: toml_edit::Value = if existing.is_bool() {
        matches!(value.to_ascii_lowercase().as_str(), "true" | "1" | "on" | "yes").into()
    } else if existing.is_integer() {
        value
            .parse::<i64>()
            .map_err(|_| Error::msg(format!("{key} takes a whole number")))?
            .into()
    } else if existing.is_float() {
        value
            .parse::<f64>()
            .map_err(|_| Error::msg(format!("{key} takes a number")))?
            .into()
    } else if existing.is_datetime() {
        // The mod keeps a few timestamps and reads them as timestamps. Written
        // back as a quoted string they parse as nothing and the mod resets
        // them, which for the notice it dates means showing the notice again.
        toml_edit::Value::from(
            value
                .parse::<toml_edit::Datetime>()
                .map_err(|_| Error::msg(format!("{key} takes a date")))?,
        )
    } else {
        value.into()
    };

    put(&mut doc, key, wanted)?;

    // The original once, so a bad guess is recoverable.
    let backup = path.with_extension("toml.roundtable-bak");
    if !backup.exists() {
        let _ = std::fs::copy(&path, &backup);
    }
    std::fs::write(&path, doc.to_string()).at(&path)?;

    // Read back rather than trust the write. This is the setting the player
    // said did nothing, and "saved" is worth nothing next to "is now that".
    let confirmed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|doc| at(&doc, key).map(|v| v.to_string().trim().trim_matches('"').to_string()));

    match confirmed {
        Some(now) => Ok(format!("{key} is now {now}")),
        None => Err(Error::msg(format!("{key} did not stick"))),
    }
}

/// `RemoveFPSLimit` reads better as `Remove FPS limit`, and `DLSS.Sharpness` as
/// `DLSS · sharpness`.
fn spaced(path: &str) -> String {
    let (section, key) = match path.split_once('.') {
        Some((section, key)) => (Some(section), key),
        None => (None, path),
    };

    let mut out = String::with_capacity(key.len() + 4);
    for (index, ch) in key.char_indices() {
        if index > 0 && ch.is_uppercase() {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    match section {
        Some(section) => format!("{section} · {out}"),
        None => out,
    }
}


/// Writes the three settings that can be chosen before the game has ever run.
///
/// The mod generates its own config the first time it loads, which would leave
/// this pane empty until then — and the one thing somebody wants beforehand is
/// the key that opens it. The author publishes an optional three-line
/// `ERSS-FG.toml` meant to be dropped in ahead of the mod, which is the proof
/// that a partial file is read and filled in rather than rejected.
///
/// Those three lines and no more. An earlier version added `DLSSMode = 2` here
/// on the reasoning that DLSS quality is worth choosing early, and that was
/// wrong in a way worth remembering: the mod keeps DLSS quality at
/// `[DLSS] DLSSMode`, so what this created was a stray top-level key that the
/// mod never read and that sat in the file looking authoritative.
///
/// An existing file is never touched — those are the player's choices.
pub fn seed(game_dir: &Path) -> Result<bool> {
    let path = config_path(game_dir);
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    std::fs::write(&path, SEED).at(&path)?;
    Ok(true)
}

/// Home to open it, and a controller able to as well.
const SEED: &str = "OverlayToggleKey = \"Home\"
                    ImGuiUseGamepadNav = true
                    ImGuiUseGamepadToggle = true
";

/// Keys an earlier Roundtable wrote that the mod does not read.
///
/// Left alone they are harmless and confusing: a top-level `DLSSMode` sits in
/// the file next to the real `[DLSS] DLSSMode` and disagrees with it, and
/// anybody reading the file to work out why a setting will not change finds two
/// of it.
const STRAY: &[&str] = &["DLSSMode"];

/// Takes those back out. Returns what it removed.
pub fn tidy(game_dir: &Path) -> Vec<String> {
    let path = config_path(game_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };

    let mut gone = Vec::new();
    for key in STRAY {
        // Only when the real one exists in its section, so this can never take
        // away the only copy of a setting.
        let real = doc
            .iter()
            .any(|(_, item)| item.as_table().is_some_and(|t| t.contains_key(key)));
        if real && doc.as_table().contains_key(key) {
            doc.as_table_mut().remove(key);
            gone.push((*key).to_string());
        }
    }

    if !gone.is_empty() {
        let _ = std::fs::write(&path, doc.to_string());
    }
    gone
}

// ---------------------------------------------------------------------------
// Artefacts
// ---------------------------------------------------------------------------

/// One thing to change in the game's own config, and the artefact it removes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub key: &'static str,
    pub value: &'static str,
    pub why: &'static str,
}

/// Everything in the game's own config that shows up as an artefact once frames
/// are being generated.
///
/// Frame generation does not invent detail; it interpolates between two frames
/// it is given. So anything that changes a pixel for reasons the motion vectors
/// cannot explain — a flickering light, a blur pass, a reflection that only
/// exists on screen — is guessed at, and the guess is the artefact. Every entry
/// here is one of those, and the first two come from the author's own list of
/// known issues rather than from theory.
pub const ARTEFACT_FIXES: &[Fix] = &[
    Fix {
        key: "GIDataQuality",
        value: "LOW",
        // The author's own words: "Setting Global Illumination to Low can
        // reduce the intensity" of the flickering this mod is known for.
        why: "Flickering light in shaded rooms is this mod's one real artefact, and \
              turning global illumination down is the author's own workaround for it",
    },
    Fix {
        key: "RaytracingQuality",
        value: "DISABLE",
        why: "The same flicker is worst with ray tracing on, and in this game it buys \
              reflections nobody sees in motion",
    },
    Fix {
        key: "MotionBlur",
        value: "OFF",
        why: "Blur drawn along last frame's motion, then interpolated again — it smears \
              twice and hides the frame rate you just paid for",
    },
    Fix {
        key: "DepthOfField",
        value: "LOW",
        why: "A full-screen blur that moves with the camera and not with the geometry, \
              which is exactly what an interpolator gets wrong",
    },
    Fix {
        key: "ScreenMode",
        value: "BORDERLESS",
        why: "In exclusive fullscreen the mod's overlay is invisible, HDR does not work, \
              and FSR frame generation flashes the screen",
    },
    Fix {
        key: "AutoDetectBestRenderingSettings",
        value: "OFF",
        why: "Otherwise the game puts all of this back on the next start",
    },
];

/// One of the mod's settings by what it does, whatever the release calls it.
///
/// The titles come from the table above and are stable; the keys behind them are
/// the mod's own and are not. Asking for "Frame limit" and getting whichever key
/// turned out to spell it is the only way to touch a setting whose name cannot
/// be known ahead of time.
pub fn setting_titled(game_dir: &Path, title: &str) -> Option<Setting> {
    settings(game_dir).into_iter().find(|s| s.title == title)
}

/// True when one of the mod's settings reads as switched on.
pub fn switched_on(setting: &Setting) -> bool {
    let value = setting.value.to_ascii_lowercase();
    !matches!(value.as_str(), "false" | "0" | "off" | "none" | "disabled" | "")
}

/// The DLSS quality worth running, in the mod's own numbering.
///
/// Upscaling artefacts and frame generation artefacts compound: the generator is
/// handed a reconstructed frame and interpolates its mistakes. The way to no
/// artefacts is therefore to scale down as little as the card allows, which is
/// DLAA — full resolution, no upscale at all — wherever there are frames to
/// spare, and one step down from there when there are not.
///
/// Frame generation is what buys the room to do this. It roughly doubles the
/// finished rate, so a card that could only hold the panel at Quality can hold it
/// at DLAA once the generator is carrying half the frames.
pub fn best_dlss_mode(tier: crate::perf::Tier, pixels: u64, framegen: bool) -> (&'static str, &'static str) {
    use crate::perf::Tier;

    // Four megapixels is 1440p. Past it the reconstruction is doing real work.
    let heavy = pixels >= 3_500_000;
    let very_heavy = pixels >= 7_000_000; // 4K and up.

    match (tier, very_heavy, heavy) {
        // Nothing to reconstruct from, so take the frames.
        (Tier::Weak, _, _) => ("0", "Performance"),
        (Tier::Modest, true, _) => ("0", "Performance"),
        (Tier::Modest, _, true) => ("1", "Balanced"),
        (Tier::Modest, _, _) if framegen => ("2", "Quality"),
        (Tier::Modest, _, _) => ("2", "Quality"),
        (Tier::Strong, true, _) => ("1", "Balanced"),
        (Tier::Strong, _, true) if framegen => ("5", "DLAA"),
        (Tier::Strong, _, true) => ("2", "Quality"),
        (Tier::Strong, _, _) => ("5", "DLAA"),
        (Tier::Ample, true, _) if framegen => ("5", "DLAA"),
        (Tier::Ample, true, _) => ("2", "Quality"),
        (Tier::Ample, _, _) => ("5", "DLAA"),
    }
}

/// Roundtable's own frame-cap patch has to stand down while this is installed.
///
/// Since v4.7.0 the mod removes the sixty cap unconditionally — the author took
/// the option away and shipped the changelog line "fixed a conflict with other
/// FPS unlocking mods" in the same release. Two things rewriting the same value
/// in a running process is not a race that resolves; it is the tearing and the
/// uneven pointer that gets blamed on the monitor.
pub fn owns_the_frame_cap(game_dir: &Path) -> bool {
    game_dir.join(CORE).is_file()
}

/// Opens one archive, whatever it is locked with.
///
/// A typed password first, then the one the author prints beside the download.
/// Neither is stored — used for this unpack and then gone.
fn unpack(archive: &Path, into: &Path, password: Option<&str>) -> Result<()> {
    let name = archive
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().to_ascii_lowercase());
    if name.ends_with(".zip") {
        return unzip(archive, into);
    }

    let mut result = match password {
        Some(password) => sevenz_rust2::decompress_file_with_password(
            archive,
            into,
            sevenz_rust2::Password::from(password),
        ),
        None => sevenz_rust2::decompress_file(archive, into),
    };

    if matches!(
        result,
        Err(sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_))
    ) && password != Some(PUBLISHED_PASSWORD)
    {
        std::fs::remove_dir_all(into).ok();
        std::fs::create_dir_all(into).at(into)?;
        result = sevenz_rust2::decompress_file_with_password(
            archive,
            into,
            sevenz_rust2::Password::from(PUBLISHED_PASSWORD),
        );
    }

    result.map_err(|error| match error {
        sevenz_rust2::Error::PasswordRequired => Error::msg(
            "This release is locked and the usual password did not open it. Paste the \
             one from the post you downloaded it from."
                .to_string(),
        ),
        sevenz_rust2::Error::MaybeBadPassword(_) => {
            Error::msg("That password did not open the archive.".to_string())
        }
        other => Error::Archive(other.to_string()),
    })
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

    /// A scratch folder under `target`, not under the system temp.
    ///
    /// These tests write files called `D3D12.dll` and `nvngx_dlss.dll`, and a
    /// file with one of those names appearing in the Windows temp folder is
    /// precisely what a real-time scanner examines hardest. It holds the file
    /// while it looks, the post-install read-back finds it absent, and the test
    /// fails — not every run, which is worse than always.
    ///
    /// `target` is already excluded from scanning on any machine that builds
    /// this often enough to notice, and it is the conventional place for build
    /// scratch anyway.
    ///
    /// The name has to be unique per test. Two of these tests were passed
    /// "stray" and ran in parallel, and since this wipes the folder before it
    /// uses it, one deleted the other's files mid-run — which showed up as an
    /// install failing on a file that had been there a moment earlier, about
    /// one run in four.
    fn temp(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-scratch")
            .join(format!("erss-{name}"));
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

        let main = chosen(&sorted, Kind::Main).unwrap();
        assert!(main.to_string_lossy().contains("4.14.1"), "got {main:?}");
        assert!(chosen(&sorted, Kind::FrameTime).is_some());
        assert!(chosen(&sorted, Kind::Overlay).is_none());

        // And the older ones stay behind it, to fall back to.
        let (_, releases) = candidates(&sorted)
            .into_iter()
            .find(|(kind, _)| *kind == Kind::Main)
            .unwrap();
        let order: Vec<String> = releases
            .iter()
            .map(|p| version_of(&p.file_name().unwrap().to_string_lossy()).unwrap())
            .collect();
        assert_eq!(order, ["4.14.1", "4.13.0", "4.12.0"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_release_that_will_not_open_falls_back_to_one_that_does() {
        // Locked, half-downloaded, half-eaten by a scanner — all the same here,
        // and none of them should be the end of it while an older release sits
        // in the same folder.
        let dir = temp("fallback");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();

        let broken = dir.join("ERSS-FG-v4.14.1-Release.zip");
        std::fs::write(&broken, b"this is not an archive").unwrap();
        let good = dir.join("ERSS-FG-v4.14.0-Release.zip");
        release(&good, &["D3D12.dll", "ERSS-FG.dll"]);

        let report = install(&game, &[broken.clone(), good], false, None).unwrap();
        assert!(game.join("ERSS-FG.dll").is_file());
        assert!(
            report[0].contains("4.14.0"),
            "installed the one that opened: {report:?}"
        );

        // With nothing to fall back to, it says so instead of half-installing.
        let empty = dir.join("Game2");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(install(&empty, &[broken], false, None).is_err());
        assert!(!empty.join("ERSS-FG.dll").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_alternative_toggle_download_is_settings_and_not_files() {
        // Its `ERSS-FG.toml` goes in the game root, which the mod does not read
        // — leaving two files of that name saying different things.
        let dir = temp("toggle");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll"]);
        let toggle = dir.join("ERSS-FG-controller-overlay-toggle.zip");
        release(&toggle, &["ERSS-FG.toml", "README.txt"]);

        let report = install(&game, &[main, toggle], false, None).unwrap();
        assert!(
            !game.join("ERSS-FG.toml").exists(),
            "nothing inert left in the game folder"
        );
        assert!(!report.iter().any(|line| line.contains("overlay-toggle")));

        // And the settings it carries are written where the mod does look.
        let seeded = std::fs::read_to_string(config_path(&game)).unwrap();
        assert!(seeded.contains("OverlayToggleKey"));
        assert!(seeded.contains("ImGuiUseGamepadToggle = true"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stray_config_from_an_earlier_install_is_cleared() {
        let dir = temp("stray");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("ERSS-FG.toml"), "OverlayToggleKey = \"T\"\n").unwrap();

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll"]);
        install(&game, &[main], false, None).unwrap();

        assert!(!game.join("ERSS-FG.toml").exists());
        assert!(config_path(&game).is_file(), "and the real one is there");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_broken_addon_does_not_stop_the_mod_going_in() {
        // The addon and the overlay config are extras.
        let dir = temp("extra-broken");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll"]);
        let addon = dir.join("ERSS-Addon-RemoveFrameTimeConstraint-v0.1.0-Release.zip");
        std::fs::write(&addon, b"truncated").unwrap();

        install(&game, &[main, addon], false, None).unwrap();
        assert!(game.join("ERSS-FG.dll").is_file(), "the mod is in");
        assert!(!status(&game, true, true, true).frame_time_addon, "the extra is not");

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

        install(&game, std::slice::from_ref(&main), false, None).unwrap();
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

    /// The config as the mod actually writes it, trimmed to the parts that
    /// matter. Taken from a real install after a real launch — the shape is the
    /// whole point, so inventing one would test nothing.
    const REAL: &str = r#"# Version: 4.14.1-0 (86498e0) Release

OverlayToggleKey = "Delete"
DateFirstLaunch = 2026-08-07T16:08:06.000000Z
DatePopup = 1970-01-01T00:00:00.000000Z
ImGuiUseGamepadNav = true
ImGuiUseGamepadToggle = true
DLSSMode = 2

[Renderer]
ScalingMode = "None"
IsHDR = false
LatencyReductionMode = 0
RemoveFPSLimit = true
MaxFPS = 0.0

[DLSS]
DLSSPreset = 0
DLSSMode = 5
Sharpness = 0.5

[FrameGeneration]
FrameGenMode = 0
GIGlitchMitigation = 1
EnableFrameGen = true

[DLSS-G]
LimitFrameRate = true
NumGenFrames = 1
"#;

    fn with_config(name: &str) -> PathBuf {
        let dir = temp(name);
        let game = dir.join("Game");
        std::fs::create_dir_all(game.join("ERSS2")).unwrap();
        std::fs::write(game.join("ERSS2").join("ERSS-FG.toml"), REAL).unwrap();
        game
    }

    #[test]
    fn the_settings_the_player_wants_are_the_ones_in_sections() {
        // Live, this pane showed four top-level settings nobody cares about and
        // none of the ones that do anything, because it never looked inside a
        // section. The player changed things and nothing happened.
        let game = with_config("sections");
        let found = settings(&game);

        let by = |key: &str| found.iter().find(|s| s.key == key);
        assert!(by("Renderer.ScalingMode").is_some(), "the upscaler: {found:#?}");
        assert!(by("FrameGeneration.FrameGenMode").is_some(), "frame generation");
        assert!(by("DLSS-G.NumGenFrames").is_some(), "a section name with a dash in it");

        // DLSS quality is the one in the section, not the stray at the top.
        let dlss = by("DLSS.DLSSMode").expect("DLSS quality");
        assert_eq!(dlss.value, "5");
        assert_eq!(dlss.title, "DLSS quality");
        assert_eq!(
            dlss.choices.iter().find(|c| c.value == "5").map(|c| c.label.as_str()),
            Some("DLAA")
        );

        // A timestamp the mod keeps for itself is not a setting.
        assert!(by("DateFirstLaunch").is_none(), "no datetimes in the pane");

        // The ones that matter lead.
        assert_eq!(found[0].key, "Renderer.ScalingMode");

        std::fs::remove_dir_all(game.parent().unwrap()).ok();
    }

    #[test]
    fn the_upscaler_and_the_generator_move_separately() {
        // "if I want DLSS but no frame generation, I am stuck" — they are two
        // settings in two sections and always were; the pane could not reach
        // either of them.
        let game = with_config("independent");

        set_setting(&game, "Renderer.ScalingMode", "DLSS").unwrap();
        set_setting(&game, "FrameGeneration.FrameGenMode", "0").unwrap();

        let found = settings(&game);
        let value = |key: &str| found.iter().find(|s| s.key == key).map(|s| s.value.clone());
        assert_eq!(value("Renderer.ScalingMode").as_deref(), Some("DLSS"));
        assert_eq!(value("FrameGeneration.FrameGenMode").as_deref(), Some("0"));

        // And the other way round.
        set_setting(&game, "Renderer.ScalingMode", "None").unwrap();
        set_setting(&game, "FrameGeneration.FrameGenMode", "1").unwrap();
        let found = settings(&game);
        let value = |key: &str| found.iter().find(|s| s.key == key).map(|s| s.value.clone());
        assert_eq!(value("Renderer.ScalingMode").as_deref(), Some("None"));
        assert_eq!(value("FrameGeneration.FrameGenMode").as_deref(), Some("1"));

        std::fs::remove_dir_all(game.parent().unwrap()).ok();
    }

    #[test]
    fn a_config_the_player_already_has_is_left_alone() {
        let dir = temp("seed-keeps");
        let game = dir.join("Game");
        let folder = game.join("ERSS2");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("ERSS-FG.toml"), "OverlayToggleKey = \"T\"\n").unwrap();

        assert!(!seed(&game).unwrap(), "their choice, not ours");
        assert_eq!(
            settings(&game).iter().find(|s| s.key == "OverlayToggleKey").unwrap().value,
            "T"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_mod_can_be_made_to_stop_announcing_itself() {
        // The player hands this to friends and does not want them to know there
        // is a mod, let alone see "press Home to close" over the game. Nothing
        // is lost by shutting it: every setting the overlay offers is in the
        // launcher.
        let game = with_config("quiet");
        let done = quieten(&game);

        assert!(done.contains(&"OverlayToggleKey".to_string()), "the key: {done:?}");
        assert!(done.contains(&"DatePopup".to_string()), "the notice: {done:?}");

        let text = std::fs::read_to_string(config_path(&game)).unwrap();
        assert!(text.contains("OverlayToggleKey = \"F24\""));
        // The date has to stay a date, or the mod cannot read it and shows the
        // notice again.
        assert!(
            !text.contains("DatePopup = \""),
            "a quoted date is not a date: {text}"
        );
        assert!(
            text.contains("DatePopup = 20"),
            "and it has to actually be this century: {text}"
        );

        // Settings this build of the mod does not have are skipped, never
        // invented.
        assert!(!done.contains(&"ShowMetricsWindow".to_string()));

        std::fs::remove_dir_all(game.parent().unwrap()).ok();
    }

    #[test]
    fn a_value_keeps_the_type_the_mod_gave_it() {
        // A float written as an integer, or a boolean written as the string
        // "true", is a value the mod reads and discards.
        let game = with_config("types");

        set_setting(&game, "Renderer.MaxFPS", "144").unwrap();
        set_setting(&game, "Renderer.IsHDR", "true").unwrap();
        set_setting(&game, "DLSS.Sharpness", "0.2").unwrap();
        set_setting(&game, "OverlayToggleKey", "Home").unwrap();

        let text = std::fs::read_to_string(config_path(&game)).unwrap();
        assert!(text.contains("MaxFPS = 144.0"), "a float stays a float: {text}");
        assert!(text.contains("IsHDR = true"), "not the string: {text}");
        assert!(text.contains("Sharpness = 0.2"));
        assert!(text.contains("OverlayToggleKey = \"Home\""), "a string stays quoted");

        // The comment the mod wrote at the top survives a round trip.
        assert!(text.starts_with("# Version: 4.14.1-0"));

        // A key the mod does not have is refused rather than invented — this is
        // how the stray top-level DLSSMode got there in the first place.
        assert!(set_setting(&game, "Renderer.MadeUp", "1").is_err());
        assert!(set_setting(&game, "NoSuchSection.Thing", "1").is_err());
        assert!(set_setting(&game, "AlsoMadeUp", "1").is_err());

        std::fs::remove_dir_all(game.parent().unwrap()).ok();
    }

    #[test]
    fn a_stray_key_an_earlier_version_wrote_is_taken_back_out() {
        // Roundtable itself put `DLSSMode` at the top of the file, where the mod
        // never reads it, next to the real `[DLSS] DLSSMode` that disagrees.
        let game = with_config("stray-key");
        assert!(settings(&game).iter().any(|s| s.key == "DLSSMode"));

        assert_eq!(tidy(&game), vec!["DLSSMode".to_string()]);

        let found = settings(&game);
        assert!(!found.iter().any(|s| s.key == "DLSSMode"), "the stray is gone");
        assert_eq!(
            found.iter().find(|s| s.key == "DLSS.DLSSMode").map(|s| s.value.as_str()),
            Some("5"),
            "and the real one is untouched"
        );

        // Nothing left to do the second time, and never the only copy.
        assert!(tidy(&game).is_empty());

        std::fs::remove_dir_all(game.parent().unwrap()).ok();
    }

    #[test]
    fn the_settings_that_matter_live_in_sections_and_are_reached_there() {
        // The bug this exists to stop coming back. The mod keeps almost
        // everything in `[Renderer]`, `[DLSS]`, `[FrameGeneration]`; the first
        // version of this pane read and wrote only the top of the file, so it
        // showed four settings nobody cares about and every change the player
        // made landed on a key the mod does not read.
        let sectioned = KNOWN.iter().filter(|k| k.path.contains('.')).count();
        assert!(sectioned >= 10, "only {sectioned} of the known settings are in sections");

        for known in KNOWN {
            assert!(!known.title.is_empty());
            assert!(known.detail.len() > 30, "{} needs a reason", known.path);
        }

        // The two the player specifically could not separate.
        assert!(described("Renderer.ScalingMode").is_some(), "the upscaler");
        assert!(described("FrameGeneration.FrameGenMode").is_some(), "frame generation");
        assert_ne!(
            described("Renderer.ScalingMode"),
            described("FrameGeneration.FrameGenMode"),
            "one without the other has to be possible"
        );
    }

    #[test]
    fn an_unknown_setting_is_still_shown_and_still_editable() {
        // A launcher that hides half a config is worse than no launcher.
        let dir = temp("unknown");
        let game = dir.join("Game");
        let folder = game.join("ERSS2");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("ERSS-FG.toml"),
            "DLSSMode = 2\nSomeFutureThing = 3\n",
        )
        .unwrap();

        let found = settings(&game);
        let future = found.iter().find(|s| s.key == "SomeFutureThing").unwrap();
        assert_eq!(future.title, "Some future thing", "spaced out, not hidden");
        assert!(!future.described);
        assert!(future.choices.is_empty());
        // Described ones lead, because they are the ones anybody would change.
        assert_eq!(found[0].key, "DLSSMode");

        set_setting(&game, "SomeFutureThing", "1").unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_artefact_fixes_are_the_ones_the_author_names() {
        let at = |key| ARTEFACT_FIXES.iter().find(|f| f.key == key).unwrap().value;

        assert_eq!(at("GIDataQuality"), "LOW", "the author's own workaround");
        assert_eq!(at("RaytracingQuality"), "DISABLE");
        assert_eq!(at("MotionBlur"), "OFF");
        assert_eq!(
            at("ScreenMode"),
            "BORDERLESS",
            "exclusive fullscreen kills the overlay, HDR and FSR frame generation"
        );
        assert!(ARTEFACT_FIXES.iter().all(|f| !f.why.is_empty()), "each says why");

        // And the optimiser has to agree with every one of them, or the two
        // buttons take turns undoing each other.
        for fix in ARTEFACT_FIXES {
            assert_eq!(
                crate::perf::preset_value(crate::perf::Tier::Ample, fix.key),
                Some(fix.value),
                "the optimiser disagrees about {} on a strong card",
                fix.key
            );
        }
    }

    #[test]
    fn a_setting_reads_as_on_or_off_however_the_mod_spells_it() {
        let of = |value: &str| Setting {
            key: "x".into(),
            title: "x".into(),
            detail: String::new(),
            value: value.into(),
            kind: "text".into(),
            choices: Vec::new(),
            described: false,
            restart: false,
        };
        for off in ["false", "0", "Off", "None", "Disabled", ""] {
            assert!(!switched_on(&of(off)), "{off}");
        }
        for on in ["true", "1", "DLSSG", "FSR3"] {
            assert!(switched_on(&of(on)), "{on}");
        }
    }

    #[test]
    fn how_far_it_upscales_follows_the_card_and_the_panel() {
        use crate::perf::Tier;
        let hd = 1920 * 1080;
        let qhd = 2560 * 1440;
        let uhd = 3840 * 2160;

        // Reconstruction artefacts and generated frames compound, so the answer
        // is to reconstruct as little as the card allows.
        assert_eq!(best_dlss_mode(Tier::Ample, qhd, true).1, "DLAA");
        assert_eq!(best_dlss_mode(Tier::Strong, hd, true).1, "DLAA");
        // Frame generation is what buys the room for DLAA at 1440p.
        assert_eq!(best_dlss_mode(Tier::Strong, qhd, true).1, "DLAA");
        assert_eq!(best_dlss_mode(Tier::Strong, qhd, false).1, "Quality");
        // And at 4K even a strong card has to give something up.
        assert_eq!(best_dlss_mode(Tier::Strong, uhd, true).1, "Balanced");
        assert_eq!(best_dlss_mode(Tier::Weak, hd, true).1, "Performance");

        // Every answer is one the mod will accept.
        for tier in [Tier::Weak, Tier::Modest, Tier::Strong, Tier::Ample] {
            for pixels in [hd, qhd, uhd] {
                for framegen in [true, false] {
                    let (value, _) = best_dlss_mode(tier, pixels, framegen);
                    assert!(DLSS_MODES.iter().any(|(v, _)| *v == value), "{value}");
                }
            }
        }
    }

    #[test]
    fn the_mod_owns_the_frame_cap_once_it_is_in() {
        // Since 4.7.0 it lifts the sixty cap unconditionally, and the author
        // fixed a conflict with other unlockers in the same release. Roundtable
        // patching the same value in the live process is the tearing.
        let dir = temp("cap-owner");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        assert!(!owns_the_frame_cap(&game));

        let main = dir.join("ERSS-FG-v4.14.1-Release.zip");
        release(&main, &["D3D12.dll", "ERSS-FG.dll"]);
        install(&game, &[main], false, None).unwrap();
        assert!(owns_the_frame_cap(&game));

        uninstall(&game).unwrap();
        assert!(!owns_the_frame_cap(&game), "and hands it back");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_game_versions_it_loads_against_are_the_three_the_author_lists() {
        let dir = temp("builds");
        let game = dir.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        let check = |build: &str| {
            preflight(&game, Some(build), false, true, true, None, None, None)
                .iter()
                .any(|note| note.contains("1.16, 1.14 and 1.13"))
        };

        // 1.16.1 reports 2.6.1.0, verified against the executable.
        assert!(!check("2.6.1.0"), "1.16");
        assert!(!check("2.4.0.0"), "1.14");
        assert!(!check("2.3.0.0"), "1.13");
        assert!(check("2.0.0.0"), "1.10 is too old and should be said so");

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
