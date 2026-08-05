//! Mod library, profiles, conflict detection and deployment.
//!
//! Mods are kept outside the game folder in Roundtable's own library. Both
//! supported loaders accept absolute paths, so a normal profile puts *nothing*
//! into the game directory — the loader is told where to look instead.
//!
//! Some older tools still insist on a literal `mod` folder next to the game. For
//! those, a profile can be deployed as an NTFS junction, which is a directory link
//! that needs no administrator rights and no file copying.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::games::Game;

/// Folders and files that mark a directory as an ELDEN RING asset override.
const ASSET_MARKERS: &[&str] = &[
    "regulation.bin",
    "parts",
    "chr",
    "map",
    "msg",
    "menu",
    "sfx",
    "sound",
    "event",
    "script",
    "action",
    "font",
    "movie",
    "param",
    "other",
    "asset",
    "cutscene",
    "expression",
    "shader",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModKind {
    /// Replaces game assets; goes in `packages` / `mods`.
    Assets,
    /// A DLL; goes in `natives` / `external_dlls`.
    Native,
    /// Ships both.
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRecord {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    /// Nexus Mods id, when the mod came from there.
    pub nexus_mod_id: Option<u32>,
    pub game: Game,
    pub kind: ModKind,
    /// Folder inside the library holding the mod's files.
    pub path: PathBuf,
    /// DLLs shipped by this mod, relative to `path`.
    pub natives: Vec<String>,
    pub file_count: usize,
    pub size_bytes: u64,
    pub installed_at: String,
    /// Set when the mod bundles its own loader, which is how big overhauls ship.
    pub bundled_loader: Option<String>,
}

/// Where the library lives.
pub fn library_dir(app_data: &Path, game: Game) -> PathBuf {
    app_data.join("mods").join(game.appdata_folder())
}

pub fn profiles_dir(app_data: &Path, game: Game) -> PathBuf {
    app_data.join("profiles").join(game.appdata_folder())
}

fn record_path(app_data: &Path, game: Game, id: &str) -> PathBuf {
    library_dir(app_data, game).join(format!("{id}.mod.json"))
}

pub fn list_mods(app_data: &Path, game: Game) -> Vec<ModRecord> {
    let dir = library_dir(app_data, game);
    let Ok(children) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut mods: Vec<ModRecord> = children
        .flatten()
        .filter(|c| {
            c.file_name()
                .to_string_lossy()
                .ends_with(".mod.json")
        })
        .filter_map(|c| std::fs::read(c.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<ModRecord>(&bytes).ok())
        .filter(|m| m.path.is_dir())
        .collect();

    mods.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    mods
}

pub fn save_record(app_data: &Path, record: &ModRecord) -> Result<()> {
    let dir = library_dir(app_data, record.game);
    std::fs::create_dir_all(&dir).at(&dir)?;
    let path = record_path(app_data, record.game, &record.id);
    std::fs::write(&path, serde_json::to_vec_pretty(record)?).at(&path)?;
    Ok(())
}

pub fn delete_mod(app_data: &Path, game: Game, id: &str) -> Result<()> {
    let record_file = record_path(app_data, game, id);
    if let Ok(bytes) = std::fs::read(&record_file) {
        if let Ok(record) = serde_json::from_slice::<ModRecord>(&bytes) {
            // Only ever delete inside our own library.
            if record.path.starts_with(library_dir(app_data, game)) && record.path.is_dir() {
                std::fs::remove_dir_all(&record.path).at(&record.path)?;
            }
        }
    }
    if record_file.exists() {
        std::fs::remove_file(&record_file).at(&record_file)?;
    }
    Ok(())
}

/// Turns an arbitrary folder of extracted mod files into a library entry.
///
/// Archives are packed inconsistently: some have the assets at the root, some wrap
/// everything in one folder, some bury them under `ModEngine-2.1.0/mod`. This walks
/// down until it finds the layer that actually holds the game assets.
pub fn analyse_layout(root: &Path) -> LayoutAnalysis {
    let mut current = root.to_path_buf();
    let mut depth = 0usize;

    loop {
        if looks_like_asset_root(&current) {
            break;
        }
        // Descend through single-child wrapper folders.
        let Ok(children) = std::fs::read_dir(&current) else {
            break;
        };
        let entries: Vec<_> = children.flatten().collect();
        let dirs: Vec<_> = entries.iter().filter(|e| e.path().is_dir()).collect();
        let files: Vec<_> = entries.iter().filter(|e| e.path().is_file()).collect();

        // A bundled loader keeps its assets in a `mod` subfolder.
        if let Some(mod_dir) = dirs
            .iter()
            .find(|d| d.file_name().to_string_lossy().eq_ignore_ascii_case("mod"))
        {
            if looks_like_asset_root(&mod_dir.path()) {
                current = mod_dir.path();
                break;
            }
        }

        if dirs.len() == 1 && files.iter().all(|f| is_ignorable_file(&f.path())) && depth < 6 {
            current = dirs[0].path();
            depth += 1;
            continue;
        }
        break;
    }

    let natives = collect_natives(&current);
    let has_assets = looks_like_asset_root(&current);
    let bundled_loader = detect_bundled_loader(root);

    let kind = match (has_assets, natives.is_empty()) {
        (true, false) => ModKind::Mixed,
        (true, true) => ModKind::Assets,
        (false, false) => ModKind::Native,
        // Nothing recognisable; treat it as assets so the user can still use it.
        (false, true) => ModKind::Assets,
    };

    LayoutAnalysis {
        asset_root: current,
        kind,
        recognised: has_assets || !natives.is_empty(),
        natives,
        bundled_loader,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutAnalysis {
    pub asset_root: PathBuf,
    pub kind: ModKind,
    pub natives: Vec<String>,
    pub bundled_loader: Option<String>,
    pub recognised: bool,
}

fn looks_like_asset_root(dir: &Path) -> bool {
    let Ok(children) = std::fs::read_dir(dir) else {
        return false;
    };
    children.flatten().any(|child| {
        let name = child.file_name().to_string_lossy().to_ascii_lowercase();
        ASSET_MARKERS.contains(&name.as_str())
    })
}

fn is_ignorable_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    name.ends_with(".txt")
        || name.ends_with(".md")
        || name.ends_with(".pdf")
        || name.ends_with(".url")
        || name.ends_with(".nfo")
        || name.ends_with(".jpg")
        || name.ends_with(".png")
        || name == "thumbs.db"
}

fn collect_natives(root: &Path) -> Vec<String> {
    walkdir::WalkDir::new(root)
        .max_depth(3)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        })
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            // The loaders' own DLLs are not mods.
            if name == "modengine2.dll" || name.starts_with("me3_") {
                return None;
            }
            e.path()
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
        })
        .collect()
}

fn detect_bundled_loader(root: &Path) -> Option<String> {
    let mut found = None;
    for entry in walkdir::WalkDir::new(root)
        .max_depth(3)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name == "modengine2_launcher.exe" {
            found = Some("ModEngine 2".to_string());
            break;
        }
        if name.ends_with(".me3") {
            found = Some("me3".to_string());
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileMod {
    pub mod_id: String,
    pub enabled: bool,
}

/// A named combination of mods plus the settings needed to launch it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub game: Game,
    /// Load order, first entry wins on a conflict for ModEngine 2 and loses for
    /// me3, which is normalised when the loader config is generated.
    pub mods: Vec<ProfileMod>,
    pub seamless_coop: bool,
    /// Isolated save name, so a modded run cannot damage a vanilla character.
    pub savefile: Option<String>,
    pub skip_logos: bool,
    pub disable_arxan: bool,
    pub mem_patch: bool,
    pub start_online: bool,
    pub created: String,
    pub last_played: Option<String>,
    pub notes: Option<String>,
}

impl Profile {
    pub fn new(game: Game, name: &str) -> Profile {
        Profile {
            id: slugify(name),
            name: name.to_string(),
            game,
            mods: Vec::new(),
            seamless_coop: false,
            savefile: None,
            skip_logos: true,
            disable_arxan: true,
            mem_patch: true,
            start_online: false,
            created: Local::now().to_rfc3339(),
            last_played: None,
            notes: None,
        }
    }

    pub fn enabled_mod_ids(&self) -> Vec<&str> {
        self.mods
            .iter()
            .filter(|m| m.enabled)
            .map(|m| m.mod_id.as_str())
            .collect()
    }
}

pub fn slugify(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "profile".to_string()
    } else {
        collapsed
    }
}

pub fn list_profiles(app_data: &Path, game: Game) -> Vec<Profile> {
    let dir = profiles_dir(app_data, game);
    let Ok(children) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut profiles: Vec<Profile> = children
        .flatten()
        .filter(|c| c.path().extension().is_some_and(|e| e == "json"))
        .filter_map(|c| std::fs::read(c.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<Profile>(&bytes).ok())
        .collect();

    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    profiles
}

pub fn save_profile(app_data: &Path, profile: &Profile) -> Result<()> {
    let dir = profiles_dir(app_data, profile.game);
    std::fs::create_dir_all(&dir).at(&dir)?;
    let path = dir.join(format!("{}.json", profile.id));
    std::fs::write(&path, serde_json::to_vec_pretty(profile)?).at(&path)?;
    Ok(())
}

pub fn delete_profile(app_data: &Path, game: Game, id: &str) -> Result<()> {
    let path = profiles_dir(app_data, game).join(format!("{id}.json"));
    if path.exists() {
        std::fs::remove_file(&path).at(&path)?;
    }
    Ok(())
}

/// Copies a profile under a new name, which is the safe way to try a change.
pub fn clone_profile(app_data: &Path, game: Game, id: &str, new_name: &str) -> Result<Profile> {
    let source = list_profiles(app_data, game)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::msg(format!("profile '{id}' was not found")))?;

    let mut copy = source.clone();
    copy.name = new_name.to_string();
    copy.id = unique_profile_id(app_data, game, &slugify(new_name));
    copy.created = Local::now().to_rfc3339();
    copy.last_played = None;
    save_profile(app_data, &copy)?;
    Ok(copy)
}

pub fn unique_profile_id(app_data: &Path, game: Game, base: &str) -> String {
    let existing: Vec<String> = list_profiles(app_data, game)
        .into_iter()
        .map(|p| p.id)
        .collect();
    if !existing.contains(&base.to_string()) {
        return base.to_string();
    }
    for suffix in 2..1000 {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", Local::now().timestamp())
}

// ---------------------------------------------------------------------------
// Conflict detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileConflict {
    /// Path relative to the mod root, e.g. `parts/am_m_0000.partsbnd.dcx`.
    pub relative_path: String,
    /// Mod names that all provide this file, in load order.
    pub providers: Vec<String>,
    /// The mod that actually wins once the loader is done.
    pub winner: String,
    /// True for `regulation.bin`, where a plain override throws away the other
    /// mod's balance changes entirely and merging is the only real fix.
    pub mergeable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictReport {
    pub conflicts: Vec<FileConflict>,
    pub total_files: usize,
    pub regulation_providers: Vec<String>,
}

/// Builds the file map for a set of mods and reports every overlap.
///
/// `mods` must already be in load order, highest priority first.
pub fn detect_conflicts(mods: &[ModRecord]) -> ConflictReport {
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for record in mods {
        for entry in walkdir::WalkDir::new(&record.path)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let Ok(relative) = entry.path().strip_prefix(&record.path) else {
                continue;
            };
            let key = relative.to_string_lossy().replace('\\', "/").to_lowercase();
            owners.entry(key).or_default().push(record.name.clone());
        }
    }

    let total_files = owners.len();
    let mut conflicts: Vec<FileConflict> = owners
        .into_iter()
        .filter(|(_, providers)| providers.len() > 1)
        .map(|(relative_path, providers)| {
            let mergeable = relative_path.ends_with("regulation.bin");
            FileConflict {
                winner: providers[0].clone(),
                mergeable,
                relative_path,
                providers,
            }
        })
        .collect();

    // Surface the ones that actually change gameplay first.
    conflicts.sort_by(|a, b| {
        b.mergeable
            .cmp(&a.mergeable)
            .then(b.providers.len().cmp(&a.providers.len()))
            .then(a.relative_path.cmp(&b.relative_path))
    });

    let regulation_providers = conflicts
        .iter()
        .find(|c| c.mergeable)
        .map(|c| c.providers.clone())
        .unwrap_or_default();

    ConflictReport {
        conflicts,
        total_files,
        regulation_providers,
    }
}

// ---------------------------------------------------------------------------
// Junction deployment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkState {
    /// Nothing at the link path.
    Absent,
    /// A junction created by Roundtable, pointing where it should.
    Linked,
    /// A junction pointing somewhere else.
    LinkedElsewhere,
    /// A real directory. Roundtable will not touch it.
    RealDirectory,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub state: LinkState,
    pub link: PathBuf,
    pub target: Option<PathBuf>,
}

#[cfg(windows)]
pub fn link_status(link: &Path, expected_target: Option<&Path>) -> LinkStatus {
    if !link.exists() && std::fs::symlink_metadata(link).is_err() {
        return LinkStatus {
            state: LinkState::Absent,
            link: link.to_path_buf(),
            target: None,
        };
    }

    match junction::get_target(link) {
        Ok(target) => {
            let matches = expected_target.is_some_and(|expected| same_path(&target, expected));
            LinkStatus {
                state: if expected_target.is_none() || matches {
                    LinkState::Linked
                } else {
                    LinkState::LinkedElsewhere
                },
                link: link.to_path_buf(),
                target: Some(target),
            }
        }
        Err(_) => LinkStatus {
            state: LinkState::RealDirectory,
            link: link.to_path_buf(),
            target: None,
        },
    }
}

#[cfg(not(windows))]
pub fn link_status(link: &Path, expected_target: Option<&Path>) -> LinkStatus {
    match std::fs::read_link(link) {
        Ok(target) => LinkStatus {
            state: if expected_target.is_none_or(|expected| same_path(&target, expected)) {
                LinkState::Linked
            } else {
                LinkState::LinkedElsewhere
            },
            link: link.to_path_buf(),
            target: Some(target),
        },
        Err(_) if link.is_dir() => LinkStatus {
            state: LinkState::RealDirectory,
            link: link.to_path_buf(),
            target: None,
        },
        Err(_) => LinkStatus {
            state: LinkState::Absent,
            link: link.to_path_buf(),
            target: None,
        },
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// Points `link` at `target`, replacing an existing junction.
///
/// A real directory at `link` is never deleted; the user is told to move it first.
/// Wiping someone's mod folder because it happened to be in the way is not an
/// acceptable failure mode.
pub fn deploy_link(link: &Path, target: &Path) -> Result<LinkStatus> {
    if !target.is_dir() {
        return Err(Error::msg(format!(
            "{} does not exist, so there is nothing to link to",
            target.display()
        )));
    }

    let status = link_status(link, Some(target));
    match status.state {
        LinkState::Linked => return Ok(status),
        LinkState::RealDirectory => {
            return Err(Error::Conflict(format!(
                "{} is a real folder, not a link. Move or rename it first so Roundtable does not delete your files.",
                link.display()
            )));
        }
        LinkState::LinkedElsewhere => {
            remove_link(link)?;
        }
        LinkState::Absent => {}
    }

    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }

    create_link(link, target)?;
    Ok(link_status(link, Some(target)))
}

#[cfg(windows)]
fn create_link(link: &Path, target: &Path) -> Result<()> {
    junction::create(target, link).at(link)
}

#[cfg(not(windows))]
fn create_link(link: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link).at(link)
}

/// Removes a junction, and only a junction.
pub fn remove_link(link: &Path) -> Result<()> {
    let status = link_status(link, None);
    match status.state {
        LinkState::Absent => Ok(()),
        LinkState::RealDirectory => Err(Error::Conflict(format!(
            "{} is a real folder; Roundtable will not delete it",
            link.display()
        ))),
        LinkState::Linked | LinkState::LinkedElsewhere => {
            // Removing the link itself leaves the target untouched.
            std::fs::remove_dir(link).at(link)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-mods-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_mod(root: &Path, name: &str, files: &[&str]) -> ModRecord {
        let path = root.join(name);
        for file in files {
            let full = path.join(file);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, name.as_bytes()).unwrap();
        }
        ModRecord {
            id: slugify(name),
            name: name.to_string(),
            version: None,
            author: None,
            summary: None,
            nexus_mod_id: None,
            game: Game::EldenRing,
            kind: ModKind::Assets,
            path,
            natives: Vec::new(),
            file_count: files.len(),
            size_bytes: 0,
            installed_at: Local::now().to_rfc3339(),
            bundled_loader: None,
        }
    }

    #[test]
    fn slugs_are_stable_and_never_empty() {
        assert_eq!(slugify("The Convergence"), "the-convergence");
        assert_eq!(slugify("  Elden Ring: Reforged  "), "elden-ring-reforged");
        assert_eq!(slugify("!!!"), "profile");
        assert_eq!(slugify("v1.2.3"), "v1-2-3");
    }

    #[test]
    fn conflicts_list_every_provider_and_name_the_winner() {
        let dir = scratch("conflicts");
        let a = make_mod(&dir, "Convergence", &["regulation.bin", "parts/a.dcx"]);
        let b = make_mod(&dir, "Reforged", &["regulation.bin", "parts/b.dcx"]);

        let report = detect_conflicts(&[a, b]);
        assert_eq!(report.total_files, 3);
        assert_eq!(report.conflicts.len(), 1);

        let clash = &report.conflicts[0];
        assert_eq!(clash.relative_path, "regulation.bin");
        assert_eq!(clash.providers, vec!["Convergence", "Reforged"]);
        assert_eq!(clash.winner, "Convergence");
        assert!(clash.mergeable);
        assert_eq!(report.regulation_providers, vec!["Convergence", "Reforged"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_overlapping_mods_report_no_conflicts() {
        let dir = scratch("no-conflicts");
        let a = make_mod(&dir, "Textures", &["parts/a.dcx"]);
        let b = make_mod(&dir, "Sounds", &["sound/b.fsb"]);

        let report = detect_conflicts(&[a, b]);
        assert!(report.conflicts.is_empty());
        assert_eq!(report.total_files, 2);
        assert!(report.regulation_providers.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn conflict_matching_ignores_path_case_and_separators() {
        let dir = scratch("case");
        let a = make_mod(&dir, "First", &["Parts/Armor.dcx"]);
        let b = make_mod(&dir, "Second", &["parts/armor.dcx"]);

        let report = detect_conflicts(&[a, b]);
        assert_eq!(report.conflicts.len(), 1, "Windows paths are case-insensitive");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn layout_analysis_descends_through_wrapper_folders() {
        let dir = scratch("layout-wrapped");
        let inner = dir.join("TheConvergence v2.2").join("mod");
        std::fs::create_dir_all(inner.join("parts")).unwrap();
        std::fs::write(inner.join("regulation.bin"), b"x").unwrap();
        std::fs::write(dir.join("readme.txt"), b"notes").unwrap();

        let analysis = analyse_layout(&dir);
        assert_eq!(analysis.asset_root, inner);
        assert_eq!(analysis.kind, ModKind::Assets);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn layout_analysis_finds_assets_at_the_root() {
        let dir = scratch("layout-flat");
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        std::fs::write(dir.join("regulation.bin"), b"x").unwrap();

        let analysis = analyse_layout(&dir);
        assert_eq!(analysis.asset_root, dir);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn layout_analysis_classifies_dll_only_mods_as_native() {
        let dir = scratch("layout-dll");
        std::fs::write(dir.join("ItemRandomiser.dll"), b"MZ").unwrap();

        let analysis = analyse_layout(&dir);
        assert_eq!(analysis.kind, ModKind::Native);
        assert_eq!(analysis.natives, vec!["ItemRandomiser.dll"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bundled_modengine_is_reported() {
        let dir = scratch("layout-bundled");
        std::fs::create_dir_all(dir.join("mod")).unwrap();
        std::fs::write(dir.join("mod").join("regulation.bin"), b"x").unwrap();
        std::fs::write(dir.join("modengine2_launcher.exe"), b"MZ").unwrap();

        let analysis = analyse_layout(&dir);
        assert_eq!(analysis.bundled_loader.as_deref(), Some("ModEngine 2"));
        assert_eq!(analysis.asset_root, dir.join("mod"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_loaders_own_dlls_are_not_treated_as_mods() {
        let dir = scratch("layout-loader-dll");
        std::fs::write(dir.join("modengine2.dll"), b"MZ").unwrap();
        std::fs::write(dir.join("CoolMod.dll"), b"MZ").unwrap();

        let analysis = analyse_layout(&dir);
        assert_eq!(analysis.natives, vec!["CoolMod.dll"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_junction_can_be_created_repointed_and_removed() {
        let dir = scratch("junction");
        let target_a = dir.join("profile-a");
        let target_b = dir.join("profile-b");
        let link = dir.join("game").join("mod");
        std::fs::create_dir_all(&target_a).unwrap();
        std::fs::create_dir_all(&target_b).unwrap();
        std::fs::write(target_a.join("marker-a"), b"a").unwrap();
        std::fs::write(target_b.join("marker-b"), b"b").unwrap();

        let status = deploy_link(&link, &target_a).unwrap();
        assert_eq!(status.state, LinkState::Linked);
        assert!(link.join("marker-a").exists(), "the link must expose the target");

        // Re-deploying the same target is a no-op.
        assert_eq!(deploy_link(&link, &target_a).unwrap().state, LinkState::Linked);

        // Switching profiles repoints the link.
        let status = deploy_link(&link, &target_b).unwrap();
        assert_eq!(status.state, LinkState::Linked);
        assert!(link.join("marker-b").exists());
        assert!(!link.join("marker-a").exists());

        remove_link(&link).unwrap();
        assert_eq!(link_status(&link, None).state, LinkState::Absent);
        // Removing the link must leave both real folders intact.
        assert!(target_a.join("marker-a").exists());
        assert!(target_b.join("marker-b").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_real_folder_in_the_way_is_never_deleted() {
        let dir = scratch("junction-guard");
        let target = dir.join("profile");
        let link = dir.join("mod");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&link).unwrap();
        std::fs::write(link.join("precious.bin"), b"user data").unwrap();

        assert!(deploy_link(&link, &target).is_err());
        assert!(remove_link(&link).is_err());
        assert!(
            link.join("precious.bin").exists(),
            "existing user files must survive"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removing_an_absent_link_is_not_an_error() {
        let dir = scratch("junction-absent");
        assert!(remove_link(&dir.join("nothing-here")).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn profile_ids_stay_unique() {
        let dir = scratch("profile-ids");
        let profile = Profile::new(Game::EldenRing, "Co-op Run");
        save_profile(&dir, &profile).unwrap();

        let next = unique_profile_id(&dir, Game::EldenRing, &slugify("Co-op Run"));
        assert_eq!(next, "co-op-run-2");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cloning_a_profile_keeps_the_mods_but_not_the_history() {
        let dir = scratch("profile-clone");
        let mut profile = Profile::new(Game::EldenRing, "Base");
        profile.mods.push(ProfileMod {
            mod_id: "convergence".into(),
            enabled: true,
        });
        profile.last_played = Some("2026-01-01T00:00:00+00:00".into());
        save_profile(&dir, &profile).unwrap();

        let copy = clone_profile(&dir, Game::EldenRing, &profile.id, "Experiment").unwrap();
        assert_eq!(copy.name, "Experiment");
        assert_eq!(copy.id, "experiment");
        assert_eq!(copy.mods.len(), 1);
        assert!(copy.last_played.is_none());

        // The original must be untouched.
        let stored = list_profiles(&dir, Game::EldenRing);
        assert_eq!(stored.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn enabled_mods_respect_the_toggle() {
        let mut profile = Profile::new(Game::EldenRing, "Mixed");
        profile.mods = vec![
            ProfileMod { mod_id: "a".into(), enabled: true },
            ProfileMod { mod_id: "b".into(), enabled: false },
            ProfileMod { mod_id: "c".into(), enabled: true },
        ];
        assert_eq!(profile.enabled_mod_ids(), vec!["a", "c"]);
    }
}
