//! Editions: total conversions large enough to be their own game.
//!
//! The Convergence is eight gigabytes, ships its own copy of me3, its own mod
//! profiles and its own save file. Treating it as an entry in a mod list would
//! mean copying all of that into Roundtable's library and then rebuilding the
//! loader config it already came with. So it is not a mod here. It is a second
//! edition of ELDEN RING that you switch to, with its own launch chain.
//!
//! The thing that actually breaks for people is narrower than it looks. The
//! mod's `Start_Convergence.bat` runs:
//!
//! ```text
//! me3.exe launch --auto-detect -p ".\me3\convergence - seamless.me3"
//! ```
//!
//! `--auto-detect` resolves the game through Steam. On a repack there is no
//! Steam to ask, `eldenring.exe` never starts, the batch file's thirty-second
//! wait expires, and its self-diagnosis prints "Steam is not running. Elden
//! Ring needs to be legitimately owned on Steam." The mod is fine; the lookup
//! is what failed. me3 takes `--exe` for an explicit executable and
//! `--skip-steam-init` to stop the launcher waiting on a client, and that pair
//! is the whole fix.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::game::{InstallKind, Installation};
use crate::games::Game;
use crate::launch::{LaunchPlan, LaunchRoute, Notice, PatchReport};

/// A total conversion Roundtable knows how to drive.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionSpec {
    pub id: &'static str,
    pub game: Game,
    pub name: &'static str,
    pub short: &'static str,
    pub note: &'static str,
    pub site: &'static str,
    /// Lowercase fragment a candidate folder's name has to contain.
    #[serde(skip)]
    pub folder_hint: &'static str,
    /// Relative paths that must all exist before a folder counts as an install.
    #[serde(skip)]
    pub markers: &'static [&'static str],
    /// Loader profile to use without co-op, relative to the edition root.
    #[serde(skip)]
    pub profile: &'static str,
    /// Loader profile to use with Seamless Co-op.
    #[serde(skip)]
    pub profile_coop: &'static str,
    /// me3 executable the edition ships with.
    #[serde(skip)]
    pub me3_exe: &'static str,
    pub savefile: &'static str,
    pub savefile_coop: &'static str,
}

pub const CONVERGENCE: EditionSpec = EditionSpec {
    id: "convergence",
    game: Game::EldenRing,
    name: "ELDEN RING: The Convergence",
    short: "The Convergence",
    note: "Total conversion",
    site: "https://convergencemod.com/",
    folder_hint: "convergence",
    // regulation.bin is the one file every version of the mod has shipped, at
    // the same place, since it is what rewrites the game's parameters.
    markers: &["mod/regulation.bin"],
    profile: "me3/convergence.me3",
    profile_coop: "me3/convergence - seamless.me3",
    me3_exe: "me3/Windows/me3.exe",
    savefile: "ER0000.cnv",
    savefile_coop: "ER0000.cnv.co2",
};

pub fn specs() -> &'static [EditionSpec] {
    &[CONVERGENCE]
}

pub fn spec(id: &str) -> Option<&'static EditionSpec> {
    specs().iter().find(|s| s.id == id)
}

pub fn for_game(game: Game) -> Vec<&'static EditionSpec> {
    specs().iter().filter(|s| s.game == game).collect()
}

/// An edition found on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionInstall {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub version: Option<String>,
    /// The me3 the edition ships with. Its absence is a blocker.
    pub me3: Option<PathBuf>,
    pub profile: Option<PathBuf>,
    pub profile_coop: Option<PathBuf>,
    /// `<root>/SeamlessCoop/ersc.dll`, which the co-op profile expects.
    pub coop_dll: Option<PathBuf>,
    pub size_bytes: Option<u64>,
    /// True when the folder sits under the game's own `Game` directory, which
    /// the mod refuses to run from.
    pub inside_game_dir: bool,
}

impl EditionInstall {
    pub fn supports_coop(&self) -> bool {
        self.profile_coop.is_some()
    }
}

/// Reads a folder and decides whether an edition lives in it.
pub fn probe(spec: &EditionSpec, dir: &Path) -> Option<EditionInstall> {
    if !dir.is_dir() {
        return None;
    }
    if !spec.markers.iter().all(|m| dir.join(m).exists()) {
        return None;
    }

    let optional = |relative: &str| {
        let path = dir.join(relative);
        path.is_file().then_some(path)
    };

    Some(EditionInstall {
        id: spec.id.to_string(),
        name: spec.name.to_string(),
        version: version_from_name(dir),
        me3: optional(spec.me3_exe),
        profile: optional(spec.profile),
        profile_coop: optional(spec.profile_coop),
        coop_dll: optional(crate::loader::SEAMLESS_COOP_DLL),
        size_bytes: None,
        inside_game_dir: looks_like_game_dir(dir),
        root: dir.to_path_buf(),
    })
}

/// Pulls a version out of a folder name like `ConvergenceER 3.0.1`.
fn version_from_name(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_string_lossy().to_string();
    name.split_whitespace()
        .rev()
        .find(|token| token.starts_with(|c: char| c.is_ascii_digit()) && token.contains('.'))
        .map(str::to_string)
}

/// True when the path sits under a game's own `Game` folder.
///
/// This mirrors the mod's own check, which looks for the literal `ELDEN RING\Game`
/// in its working directory and refuses to start. Matching a bare `Game`
/// component would be stricter than the mod itself and would wrongly block a
/// perfectly good `D:\Games\Convergence`, so the parent has to name the title too.
fn looks_like_game_dir(dir: &Path) -> bool {
    let parts: Vec<String> = dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();

    parts.windows(2).any(|pair| {
        pair[1] == "game" && pair[0].contains("elden") && pair[0].contains("ring")
    })
}

/// Looks for an edition around a game installation.
///
/// The mod belongs in its own folder rather than inside the game, so the search
/// walks outward: the install root first, then its siblings, then the siblings
/// of each parent for a couple of levels. That covers the shape people actually
/// end up with, where the mod sits next to the game on the same drive.
pub fn discover(spec: &EditionSpec, install: &Installation) -> Vec<EditionInstall> {
    let mut found: Vec<EditionInstall> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    let consider = |dir: &Path, found: &mut Vec<EditionInstall>, seen: &mut Vec<PathBuf>| {
        let key = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        if seen.contains(&key) {
            return;
        }
        seen.push(key);
        if let Some(edition) = probe(spec, dir) {
            found.push(edition);
        }
    };

    // The install itself, in case the mod was dropped straight into it.
    consider(&install.root, &mut found, &mut seen);
    consider(&install.game_dir, &mut found, &mut seen);

    let mut level = install.root.as_path();
    for _ in 0..3 {
        let Some(parent) = level.parent() else { break };
        let Ok(entries) = std::fs::read_dir(parent) else {
            level = parent;
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.contains(spec.folder_hint) {
                consider(&path, &mut found, &mut seen);
            }
        }
        level = parent;
    }

    found
}

/// Builds the launch for an edition.
///
/// This is deliberately not the general planner in `launch.rs`. That one starts
/// from a Roundtable profile and writes a loader config; here the edition
/// already shipped a correct config, and the only thing missing is a game to
/// point it at.
pub fn plan(
    spec: &EditionSpec,
    edition: &EditionInstall,
    install: &Installation,
    coop: bool,
    steam_running: bool,
) -> Result<LaunchPlan> {
    let standalone = install.kind == InstallKind::Standalone;
    let mut notices = Vec::new();
    let mut steps = Vec::new();
    let mut writes = Vec::new();

    if edition.inside_game_dir {
        notices.push(Notice::blocker(
            "Installed in the wrong folder",
            format!(
                "{} sits inside the game's own directory. The mod refuses to run from there — move it to its own folder on the same drive.",
                edition.root.display()
            ),
        ));
    }

    let Some(me3) = edition.me3.clone() else {
        notices.push(Notice::blocker(
            "This copy has no loader",
            format!(
                "{} was expected inside the edition folder. Re-extract the archive without leaving anything out.",
                edition.root.join(spec.me3_exe).display()
            ),
        ));
        return Ok(blocked(install, notices));
    };

    let profile = if coop {
        edition.profile_coop.clone()
    } else {
        edition.profile.clone()
    };
    let Some(profile) = profile else {
        notices.push(Notice::blocker(
            "Missing loader profile",
            format!(
                "{} was not found in the edition folder.",
                if coop { spec.profile_coop } else { spec.profile }
            ),
        ));
        return Ok(blocked(install, notices));
    };

    // The co-op profile refers to `./../SeamlessCoop/ersc.dll`, relative to
    // itself, so the DLL has to be beside the edition rather than beside the
    // game. Patching copies it across.
    if coop && edition.coop_dll.is_none() {
        notices.push(Notice::warn(
            "Co-op is not wired up yet",
            format!(
                "The co-op profile loads {}, which is not there. Press Patch and Roundtable will copy Seamless Co-op across from the game folder.",
                edition.root.join(crate::loader::SEAMLESS_COOP_DLL).display()
            ),
        ));
    }

    if standalone {
        notices.push(Notice::info(
            "Non-Steam installation",
            "The mod's own batch file calls me3 with --auto-detect, which looks the game up through Steam and fails here. Roundtable passes the executable directly instead.",
        ));
    } else if !steam_running {
        notices.push(Notice::warn(
            "Steam is not running",
            "This is a Steam copy. Start Steam first, or the game may close on launch.",
        ));
    }

    let mut args = vec!["launch".to_string()];

    if let Some(me3_id) = install.game.me3_id() {
        args.push("--game".to_string());
        args.push(me3_id.to_string());
    }

    // The two flags that replace --auto-detect.
    args.push("--exe".to_string());
    args.push(install.executable.to_string_lossy().to_string());
    steps.push(format!(
        "Point me3 at {} instead of asking Steam",
        install.executable.display()
    ));

    if standalone {
        args.push("--skip-steam-init".to_string());
        args.push("true".to_string());
        steps.push("Pass --skip-steam-init so the launcher does not wait for a Steam client".into());
    }

    args.push("--profile".to_string());
    args.push(profile.to_string_lossy().to_string());
    steps.push(format!(
        "Load the edition's own profile, {}",
        profile
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    ));

    steps.push(format!(
        "Saves go to {}",
        if coop { spec.savefile_coop } else { spec.savefile }
    ));

    if standalone {
        writes.push(install.game_dir.join("steam_appid.txt"));
    }
    if coop && edition.coop_dll.is_none() {
        writes.push(edition.root.join(crate::loader::SEAMLESS_COOP_DLL));
    }

    let mut env = std::collections::BTreeMap::new();
    env.insert(
        "SteamAppId".to_string(),
        install.game.steam_app_id().to_string(),
    );
    env.insert(
        "SteamGameId".to_string(),
        install.game.steam_app_id().to_string(),
    );

    Ok(LaunchPlan {
        route: LaunchRoute::Me3,
        program: me3,
        args,
        // me3 resolves the relative paths inside the profile against the profile
        // itself, but the edition's own launcher runs from its root and some of
        // its native DLLs expect that too.
        working_dir: edition.root.clone(),
        env,
        steps,
        notices,
        writes,
        coop_enabled: coop,
        skip_steam_init: standalone,
    })
}

/// A plan that cannot run, carrying only the reason why.
fn blocked(install: &Installation, notices: Vec<Notice>) -> LaunchPlan {
    LaunchPlan {
        route: LaunchRoute::Direct,
        program: install.executable.clone(),
        args: Vec::new(),
        working_dir: install.game_dir.clone(),
        env: std::collections::BTreeMap::new(),
        steps: Vec::new(),
        notices,
        writes: Vec::new(),
        coop_enabled: false,
        skip_steam_init: false,
    }
}

/// Makes the edition launchable: copies Seamless Co-op across and writes the
/// app id file a repack's emulator reads.
pub fn patch(
    edition: &EditionInstall,
    install: &Installation,
    coop: bool,
    plan: &LaunchPlan,
) -> Result<PatchReport> {
    let mut written = Vec::new();
    let mut changes = Vec::new();

    if coop && edition.coop_dll.is_none() {
        let source = install.game_dir.join("SeamlessCoop");
        let target = edition.root.join("SeamlessCoop");
        if source.is_dir() {
            let count = copy_tree(&source, &target)?;
            changes.push(format!(
                "Copied Seamless Co-op into the edition folder ({count} file(s))"
            ));
            written.push(target.join("ersc.dll"));
        } else {
            changes.push(format!(
                "Could not copy Seamless Co-op: {} does not exist. Install it from the Co-op tab first.",
                source.display()
            ));
        }
    }

    if install.kind == InstallKind::Standalone {
        match crate::steam::write_appid_file(&install.game_dir, install.game.steam_app_id()) {
            Ok(path) => {
                changes.push(format!(
                    "Wrote steam_appid.txt so the emulator reports app {}",
                    install.game.steam_app_id()
                ));
                written.push(path);
            }
            Err(err) => changes.push(format!("Could not write steam_appid.txt: {err}")),
        }
    }

    if changes.is_empty() {
        changes.push("Nothing to change — this edition is already set up.".into());
    }

    Ok(PatchReport {
        route: plan.route,
        written,
        changes,
        notices: plan.notices.clone(),
    })
}

/// Recursive copy that reports how many files it wrote.
fn copy_tree(source: &Path, destination: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in walkdir::WalkDir::new(source)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let Ok(relative) = entry.path().strip_prefix(source) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).at(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            std::fs::copy(entry.path(), &target).at(&target)?;
            count += 1;
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Installing from the downloaded archive
// ---------------------------------------------------------------------------

/// Live state of an extraction, polled by the interface.
///
/// The Convergence archive is over eight gigabytes and takes minutes to unpack.
/// A request that simply blocks until it finishes tells the user nothing, so the
/// work runs on its own thread and writes its position here.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionJob {
    pub edition: String,
    pub running: bool,
    pub done: bool,
    pub message: String,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub destination: Option<PathBuf>,
    pub error: Option<String>,
}

impl EditionJob {
    pub fn percent(&self) -> f32 {
        if self.bytes_total == 0 {
            return 0.0;
        }
        (self.bytes_done as f32 / self.bytes_total as f32) * 100.0
    }
}

/// Where an edition should be unpacked, given the game it belongs to.
///
/// Beside the game rather than inside it: the mod's launcher checks for
/// `ELDEN RING\Game` in its own path and refuses to start from there.
pub fn default_destination(install: &Installation, spec: &EditionSpec) -> PathBuf {
    let base = install
        .root
        .parent()
        .unwrap_or(install.root.as_path())
        .to_path_buf();
    base.join(match spec.id {
        "convergence" => "ConvergenceER",
        other => other,
    })
}

/// Total size of an archive once unpacked, read from its own directory.
///
/// Worth knowing before starting: The Convergence unpacks to more than ten
/// gigabytes, and running out of room half way leaves a folder that looks
/// installed and is not.
pub fn unpacked_size(archive: &Path) -> Result<u64> {
    let file = std::fs::File::open(archive).at(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Archive(e.to_string()))?;
    let mut total = 0u64;
    for index in 0..zip.len() {
        if let Ok(entry) = zip.by_index_raw(index) {
            total += entry.size();
        }
    }
    Ok(total)
}

/// Free bytes on the volume holding `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    let mut probe = path.to_path_buf();
    // The destination may not exist yet; walk up to something that does.
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|d| probe.starts_with(d.mount_point()))
        // The deepest mount point that contains the path is the right volume.
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(sysinfo::Disk::available_space)
}

/// Unpacks an edition archive, reporting progress through `report`.
///
/// Archives from Nexus wrap everything in a single versioned folder. That folder
/// becomes the edition root, so `destination` is treated as the parent and the
/// wrapper is kept — the version in its name is how the interface knows which
/// release is installed.
pub fn extract_archive(
    archive: &Path,
    destination: &Path,
    mut report: impl FnMut(usize, usize, u64, u64),
) -> Result<PathBuf> {
    let file = std::fs::File::open(archive).at(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Archive(e.to_string()))?;

    let total_files = zip.len();
    let mut total_bytes = 0u64;
    for index in 0..total_files {
        if let Ok(entry) = zip.by_index_raw(index) {
            total_bytes += entry.size();
        }
    }

    let mut done_files = 0usize;
    let mut done_bytes = 0u64;
    let mut root: Option<PathBuf> = None;

    for index in 0..total_files {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| Error::Archive(e.to_string()))?;

        // `enclosed_name` rejects `..`, which is what stops a hostile archive
        // writing outside the destination.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };

        if root.is_none() {
            if let Some(std::path::Component::Normal(first)) = relative.components().next() {
                root = Some(destination.join(first));
            }
        }

        let target = destination.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).at(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            let mut out = std::io::BufWriter::new(std::fs::File::create(&target).at(&target)?);

            // Copied in chunks rather than with `io::copy`, because a few of the
            // files in this archive are gigabytes on their own and a per-file
            // counter sits frozen the whole time one of them is being written.
            let mut buffer = vec![0u8; 1 << 20];
            let mut since_report = 0u64;
            loop {
                let read = std::io::Read::read(&mut entry, &mut buffer).at(&target)?;
                if read == 0 {
                    break;
                }
                std::io::Write::write_all(&mut out, &buffer[..read]).at(&target)?;
                done_bytes += read as u64;
                since_report += read as u64;
                if since_report >= 32 << 20 {
                    since_report = 0;
                    report(done_files, total_files, done_bytes, total_bytes);
                }
            }
            std::io::Write::flush(&mut out).at(&target)?;
        }

        done_files += 1;
        // Locking the shared state for every small file would cost more than the
        // extra precision is worth.
        if done_files % 25 == 0 || done_files == total_files {
            report(done_files, total_files, done_bytes, total_bytes);
        }
    }

    Ok(root.unwrap_or_else(|| destination.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-edition-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A folder shaped like Convergence 3.0.1 as it comes out of the archive.
    fn lay_out(root: &Path, with_coop: bool) {
        std::fs::create_dir_all(root.join("mod")).unwrap();
        std::fs::write(root.join("mod/regulation.bin"), b"x").unwrap();
        std::fs::create_dir_all(root.join("me3/Windows")).unwrap();
        std::fs::write(root.join("me3/Windows/me3.exe"), b"x").unwrap();
        std::fs::write(root.join("me3/convergence.me3"), b"x").unwrap();
        std::fs::write(root.join("me3/convergence - seamless.me3"), b"x").unwrap();
        if with_coop {
            std::fs::create_dir_all(root.join("SeamlessCoop")).unwrap();
            std::fs::write(root.join("SeamlessCoop/ersc.dll"), b"x").unwrap();
        }
    }

    fn installation(kind: InstallKind, root: PathBuf) -> Installation {
        let game_dir = root.join("Game");
        Installation {
            game: Game::EldenRing,
            executable: game_dir.join("eldenring.exe"),
            game_dir,
            root,
            kind,
            version: None,
            has_eac: false,
            eac_bypassed: false,
            has_seamless_coop: true,
            seamless_coop_version: None,
            size_bytes: None,
            markers: Vec::new(),
        }
    }

    #[test]
    fn a_folder_without_the_marker_is_not_an_edition() {
        let dir = temp("empty");
        assert!(probe(&CONVERGENCE, &dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn probing_reads_the_loader_and_both_profiles() {
        let dir = temp("probe").join("ConvergenceER 3.0.1");
        lay_out(&dir, false);

        let found = probe(&CONVERGENCE, &dir).expect("edition detected");
        assert_eq!(found.version.as_deref(), Some("3.0.1"));
        assert!(found.me3.is_some());
        assert!(found.profile.is_some());
        assert!(found.supports_coop());
        assert!(found.coop_dll.is_none());

        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn an_edition_beside_the_game_is_discovered() {
        let base = temp("beside");
        let game_root = base.join("ELDEN RING");
        std::fs::create_dir_all(game_root.join("Game")).unwrap();
        lay_out(&base.join("ConvergenceER 3.0.1"), false);

        let install = installation(InstallKind::Standalone, game_root);
        let found = discover(&CONVERGENCE, &install);
        assert_eq!(found.len(), 1, "expected one edition, got {found:?}");
        assert!(!found[0].inside_game_dir);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_repack_gets_the_exe_and_skip_steam_init_that_the_batch_file_lacks() {
        let base = temp("repack");
        let game_root = base.join("ELDEN RING");
        std::fs::create_dir_all(game_root.join("Game")).unwrap();
        let edition_root = base.join("ConvergenceER 3.0.1");
        lay_out(&edition_root, true);

        let install = installation(InstallKind::Standalone, game_root);
        let edition = probe(&CONVERGENCE, &edition_root).unwrap();
        let plan = plan(&CONVERGENCE, &edition, &install, true, false).unwrap();

        assert!(plan.is_runnable());
        assert!(plan.skip_steam_init);
        let line = plan.args.join(" ");
        assert!(line.contains("--skip-steam-init true"), "got {line}");
        assert!(line.contains("--exe"), "got {line}");
        assert!(!line.contains("--auto-detect"), "the flag that breaks repacks");
        assert!(line.contains("convergence - seamless.me3"), "got {line}");
        assert_eq!(plan.working_dir, edition_root);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_steam_copy_is_not_told_to_skip_steam() {
        let base = temp("steam");
        let game_root = base.join("ELDEN RING");
        std::fs::create_dir_all(game_root.join("Game")).unwrap();
        let edition_root = base.join("ConvergenceER 3.0.1");
        lay_out(&edition_root, true);

        let install = installation(InstallKind::Steam, game_root);
        let edition = probe(&CONVERGENCE, &edition_root).unwrap();
        let plan = plan(&CONVERGENCE, &edition, &install, false, true).unwrap();

        assert!(!plan.skip_steam_init);
        assert!(!plan.args.join(" ").contains("--skip-steam-init"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn co_op_without_the_dll_beside_the_edition_warns_but_still_runs() {
        let base = temp("nocoop");
        let game_root = base.join("ELDEN RING");
        std::fs::create_dir_all(game_root.join("Game")).unwrap();
        let edition_root = base.join("ConvergenceER 3.0.1");
        lay_out(&edition_root, false);

        let install = installation(InstallKind::Standalone, game_root);
        let edition = probe(&CONVERGENCE, &edition_root).unwrap();
        let plan = plan(&CONVERGENCE, &edition, &install, true, false).unwrap();

        assert!(plan.is_runnable(), "a missing DLL is fixable, not fatal");
        assert!(plan
            .notices
            .iter()
            .any(|n| n.title.contains("Co-op is not wired up")));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn patching_copies_seamless_coop_next_to_the_edition() {
        let base = temp("patch");
        let game_root = base.join("ELDEN RING");
        let game_dir = game_root.join("Game");
        std::fs::create_dir_all(game_dir.join("SeamlessCoop")).unwrap();
        std::fs::write(game_dir.join("SeamlessCoop/ersc.dll"), b"dll").unwrap();
        std::fs::write(game_dir.join("SeamlessCoop/ersc_settings.ini"), b"ini").unwrap();
        let edition_root = base.join("ConvergenceER 3.0.1");
        lay_out(&edition_root, false);

        let install = installation(InstallKind::Standalone, game_root);
        let edition = probe(&CONVERGENCE, &edition_root).unwrap();
        let plan = plan(&CONVERGENCE, &edition, &install, true, false).unwrap();
        let report = patch(&edition, &install, true, &plan).unwrap();

        assert!(edition_root.join("SeamlessCoop/ersc.dll").is_file());
        assert!(edition_root.join("SeamlessCoop/ersc_settings.ini").is_file());
        assert!(report.changes.iter().any(|c| c.contains("Seamless Co-op")));

        // Re-probing now sees co-op, so the warning goes away.
        let again = probe(&CONVERGENCE, &edition_root).unwrap();
        assert!(again.coop_dll.is_some());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn an_edition_inside_the_game_folder_is_blocked_with_the_reason() {
        let base = temp("wrongplace");
        let game_root = base.join("ELDEN RING");
        let edition_root = game_root.join("Game").join("ConvergenceER 3.0.1");
        lay_out(&edition_root, true);

        let install = installation(InstallKind::Standalone, game_root);
        let edition = probe(&CONVERGENCE, &edition_root).unwrap();
        assert!(edition.inside_game_dir);

        let plan = plan(&CONVERGENCE, &edition, &install, true, false).unwrap();
        assert!(!plan.is_runnable());
        assert!(plan
            .notices
            .iter()
            .any(|n| n.title.contains("wrong folder")));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_copy_missing_me3_is_blocked_rather_than_launched() {
        let base = temp("nome3");
        let game_root = base.join("ELDEN RING");
        std::fs::create_dir_all(game_root.join("Game")).unwrap();
        let edition_root = base.join("ConvergenceER 3.0.1");
        std::fs::create_dir_all(edition_root.join("mod")).unwrap();
        std::fs::write(edition_root.join("mod/regulation.bin"), b"x").unwrap();

        let install = installation(InstallKind::Standalone, game_root);
        let edition = probe(&CONVERGENCE, &edition_root).unwrap();
        let plan = plan(&CONVERGENCE, &edition, &install, false, false).unwrap();

        assert!(!plan.is_runnable());
        assert!(plan.notices.iter().any(|n| n.title.contains("no loader")));

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn the_default_destination_is_beside_the_game_not_inside_it() {
        let install = installation(
            InstallKind::Standalone,
            PathBuf::from("D:\\Games\\ELDEN RING"),
        );
        let target = default_destination(&install, &CONVERGENCE);
        assert_eq!(target, PathBuf::from("D:\\Games\\ConvergenceER"));
        assert!(!looks_like_game_dir(&target));
    }

    #[test]
    fn only_the_games_own_game_folder_counts_as_the_wrong_place() {
        // What the mod actually refuses.
        assert!(looks_like_game_dir(Path::new(
            "C:\\x\\ELDEN RING\\Game\\ConvergenceER"
        )));
        assert!(looks_like_game_dir(Path::new(
            "C:\\x\\elden ring (2022)\\game\\ConvergenceER"
        )));
        // A folder that merely happens to be called Game is fine.
        assert!(!looks_like_game_dir(Path::new("D:\\Game\\ConvergenceER")));
        assert!(!looks_like_game_dir(Path::new(
            "C:\\x\\ELDEN RING (2022)\\ConvergenceER 3.0.1"
        )));
    }

    #[test]
    fn versions_are_read_off_the_folder_name() {
        assert_eq!(
            version_from_name(Path::new("C:\\x\\ConvergenceER 3.0.1")).as_deref(),
            Some("3.0.1")
        );
        assert_eq!(version_from_name(Path::new("C:\\x\\Convergence")), None);
    }
}
