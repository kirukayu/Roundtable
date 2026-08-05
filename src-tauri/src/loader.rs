//! Mod loader support: ModEngine 2 and me3 (ModEngine 3).
//!
//! ModEngine 2 is discontinued upstream but still ships with most large mods, so
//! Roundtable has to speak both dialects:
//!
//! * ModEngine 2 — `config_eldenring.toml` with `[modengine] external_dlls` for DLL
//!   mods and `[extension.mod_loader] mods = [{ enabled, name, path }]` for assets.
//!   Launched via `modengine2_launcher.exe -t er -c <config>`.
//! * me3 — a `.me3` profile with `[[natives]]` for DLLs and `[[packages]]` for assets.
//!   Launched via `me3 launch -p <profile>`, and it is the only one with
//!   `--skip-steam-init`, which is what makes cracked copies work.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

use crate::error::{Error, IoContext, Result};
use crate::games::Game;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoaderKind {
    ModEngine2,
    Me3,
}

impl LoaderKind {
    pub fn label(self) -> &'static str {
        match self {
            LoaderKind::ModEngine2 => "ModEngine 2",
            LoaderKind::Me3 => "me3",
        }
    }

    /// Only me3 can skip Steam initialisation, which is the whole reason cracked
    /// copies fail with "trying to find steam" under ModEngine 2.
    pub fn supports_skip_steam_init(self) -> bool {
        matches!(self, LoaderKind::Me3)
    }
}

/// A loader installation found on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderInstall {
    pub kind: LoaderKind,
    /// The binary to run.
    pub executable: PathBuf,
    /// Folder that holds the loader and, for ModEngine 2, its config files.
    pub directory: PathBuf,
    pub version: Option<String>,
    /// ModEngine 2 config discovered next to the launcher, if any.
    pub config: Option<PathBuf>,
}

/// Finds every loader Roundtable can see: bundled with a mod, next to the game,
/// or installed system-wide.
pub fn discover(game: Game, game_root: Option<&Path>) -> Vec<LoaderInstall> {
    let mut found: Vec<LoaderInstall> = Vec::new();

    for dir in me2_search_roots(game_root) {
        if let Some(install) = probe_modengine2(game, &dir) {
            if !found.iter().any(|l| l.executable == install.executable) {
                found.push(install);
            }
        }
    }

    for exe in me3_candidates() {
        if exe.is_file() {
            let directory = exe
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| exe.clone());
            let install = LoaderInstall {
                kind: LoaderKind::Me3,
                version: crate::game::file_version(&exe),
                executable: exe,
                directory,
                config: None,
            };
            if !found.iter().any(|l| l.executable == install.executable) {
                found.push(install);
            }
        }
    }

    found
}

fn me2_search_roots(game_root: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = game_root {
        roots.push(root.to_path_buf());
        roots.push(root.join("ModEngine"));
        roots.push(root.join("ModEngine2"));
        // Mods commonly unpack a `ModEngine-2.x.x-win64` folder beside the game.
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with("modengine") && entry.path().is_dir() {
                    roots.push(entry.path());
                }
            }
        }
        if let Some(parent) = root.parent() {
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    if name.starts_with("modengine") && entry.path().is_dir() {
                        roots.push(entry.path());
                    }
                }
            }
        }
    }
    roots
}

fn probe_modengine2(game: Game, dir: &Path) -> Option<LoaderInstall> {
    let exe = dir.join("modengine2_launcher.exe");
    if !exe.is_file() {
        return None;
    }
    let config = game
        .me2_config_name()
        .map(|name| dir.join(name))
        .filter(|p| p.is_file());
    Some(LoaderInstall {
        kind: LoaderKind::ModEngine2,
        version: crate::game::file_version(&exe),
        executable: exe,
        directory: dir.to_path_buf(),
        config,
    })
}

fn me3_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(local) = dirs::data_local_dir() {
        candidates.push(local.join("Programs").join("me3").join("bin").join("me3.exe"));
        candidates.push(local.join("Programs").join("me3").join("me3.exe"));
        candidates.push(local.join("me3").join("bin").join("me3.exe"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local").join("bin").join("me3.exe"));
        candidates.push(home.join(".local").join("bin").join("me3"));
    }
    for base in ["C:\\Program Files\\me3", "C:\\Program Files (x86)\\me3"] {
        candidates.push(PathBuf::from(base).join("bin").join("me3.exe"));
        candidates.push(PathBuf::from(base).join("me3.exe"));
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            candidates.push(dir.join("me3.exe"));
        }
    }

    candidates
}

/// me3's own configuration directory, where profiles and `me3.toml` live.
pub fn me3_config_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("garyttierney").join("me3").join("config"))
}

pub fn me3_profile_dir() -> Option<PathBuf> {
    me3_config_dir().map(|d| d.join("profiles"))
}

// ---------------------------------------------------------------------------
// ModEngine 2 config
// ---------------------------------------------------------------------------

/// One asset-override entry in ModEngine 2's `mods` array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me2Mod {
    pub enabled: bool,
    pub name: String,
    pub path: String,
}

/// The parts of a ModEngine 2 config Roundtable reads and writes. Editing goes
/// through `toml_edit` so the extensive comments in the stock file survive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me2Config {
    pub debug: bool,
    pub external_dlls: Vec<String>,
    pub mod_loader_enabled: bool,
    pub loose_params: bool,
    pub mods: Vec<Me2Mod>,
    pub scylla_hide: bool,
}

pub fn read_me2_config(path: &Path) -> Result<Me2Config> {
    let text = std::fs::read_to_string(path).at(path)?;
    parse_me2_config(&text)
}

pub fn parse_me2_config(text: &str) -> Result<Me2Config> {
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| Error::parse("ModEngine 2 config", e))?;

    let external_dlls = doc
        .get("modengine")
        .and_then(|m| m.get("external_dlls"))
        .and_then(Item::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mods = doc
        .get("extension")
        .and_then(|e| e.get("mod_loader"))
        .and_then(|m| m.get("mods"))
        .and_then(Item::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_inline_table)
                .map(|t| Me2Mod {
                    enabled: t.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                    name: t
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    path: t
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Me2Config {
        debug: bool_at(&doc, &["modengine", "debug"]).unwrap_or(false),
        external_dlls,
        mod_loader_enabled: bool_at(&doc, &["extension", "mod_loader", "enabled"]).unwrap_or(true),
        loose_params: bool_at(&doc, &["extension", "mod_loader", "loose_params"]).unwrap_or(false),
        mods,
        scylla_hide: bool_at(&doc, &["extension", "scylla_hide", "enabled"]).unwrap_or(false),
    })
}

fn bool_at(doc: &DocumentMut, keys: &[&str]) -> Option<bool> {
    let mut item: &Item = doc.as_item();
    for key in keys {
        item = item.get(key)?;
    }
    item.as_bool()
}

/// Rewrites a ModEngine 2 config in place, preserving comments and formatting.
pub fn write_me2_config(path: &Path, config: &Me2Config) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .parse()
            .map_err(|e| Error::parse("ModEngine 2 config", e))?
    };

    let modengine = ensure_table(doc.as_table_mut(), "modengine");
    modengine["debug"] = value(config.debug);
    let mut dlls = Array::new();
    for dll in &config.external_dlls {
        dlls.push(dll.as_str());
    }
    // Keep the original one-per-line shape when the list is long enough to need it.
    if config.external_dlls.len() > 1 {
        for item in dlls.iter_mut() {
            item.decor_mut().set_prefix("\n    ");
        }
        dlls.set_trailing("\n");
    }
    modengine["external_dlls"] = value(dlls);

    let extension = ensure_table(doc.as_table_mut(), "extension");
    let mod_loader = ensure_table(extension, "mod_loader");
    mod_loader["enabled"] = value(config.mod_loader_enabled);
    mod_loader["loose_params"] = value(config.loose_params);

    let mut mods = Array::new();
    for entry in &config.mods {
        let mut table = InlineTable::new();
        table.insert("enabled", Value::from(entry.enabled));
        table.insert("name", Value::from(entry.name.clone()));
        table.insert("path", Value::from(entry.path.clone()));
        mods.push(Value::InlineTable(table));
    }
    for item in mods.iter_mut() {
        item.decor_mut().set_prefix("\n    ");
    }
    if !config.mods.is_empty() {
        mods.set_trailing("\n");
    }
    mod_loader["mods"] = value(mods);

    let scylla = ensure_table(extension, "scylla_hide");
    scylla["enabled"] = value(config.scylla_hide);

    std::fs::write(path, doc.to_string()).at(path)?;
    Ok(())
}

fn ensure_table<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
    if !parent.contains_key(key) {
        let mut table = Table::new();
        table.set_implicit(false);
        parent.insert(key, Item::Table(table));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .expect("table was just inserted")
}

// ---------------------------------------------------------------------------
// me3 profiles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me3Native {
    pub path: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me3Package {
    pub path: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// A `.me3` mod profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me3Profile {
    pub game: Game,
    pub natives: Vec<Me3Native>,
    pub packages: Vec<Me3Package>,
    /// Alternative save file name; me3 copies the base save if it does not exist.
    /// This is the cleanest way to keep a modded run away from a vanilla one.
    pub savefile: Option<String>,
    /// me3 blocks official matchmaking by default. Seamless Co-op does not need
    /// this turned on, so it stays off unless the user asks for a private server.
    pub start_online: Option<bool>,
    pub disable_arxan: Option<bool>,
    pub mem_patch: Option<bool>,
}

impl Me3Profile {
    pub fn new(game: Game) -> Self {
        Me3Profile {
            game,
            natives: Vec::new(),
            packages: Vec::new(),
            savefile: None,
            start_online: None,
            disable_arxan: None,
            mem_patch: None,
        }
    }

    pub fn to_toml(&self) -> String {
        let mut doc = DocumentMut::new();
        doc["profileVersion"] = value("v1");

        if let Some(savefile) = &self.savefile {
            doc["savefile"] = value(savefile.as_str());
        }
        if let Some(online) = self.start_online {
            doc["start_online"] = value(online);
        }
        if let Some(arxan) = self.disable_arxan {
            doc["disable_arxan"] = value(arxan);
        }
        if let Some(patch) = self.mem_patch {
            doc["mem_patch"] = value(patch);
        }

        let mut supports = ArrayOfTables::new();
        let mut support = Table::new();
        support["game"] = value(self.game.me3_id());
        supports.push(support);
        doc["supports"] = Item::ArrayOfTables(supports);

        let mut packages = ArrayOfTables::new();
        for package in self.packages.iter().filter(|p| p.enabled) {
            let mut table = Table::new();
            if let Some(id) = &package.id {
                table["id"] = value(id.as_str());
            }
            table["path"] = value(package.path.as_str());
            packages.push(table);
        }
        if !packages.is_empty() {
            doc["packages"] = Item::ArrayOfTables(packages);
        }

        let mut natives = ArrayOfTables::new();
        for native in self.natives.iter().filter(|n| n.enabled) {
            let mut table = Table::new();
            table["path"] = value(native.path.as_str());
            if native.optional {
                table["optional"] = value(true);
            }
            natives.push(table);
        }
        if !natives.is_empty() {
            doc["natives"] = Item::ArrayOfTables(natives);
        }

        doc.to_string()
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        let doc: DocumentMut = text.parse().map_err(|e| Error::parse("me3 profile", e))?;

        let game = doc
            .get("supports")
            .and_then(Item::as_array_of_tables)
            .and_then(|tables| tables.iter().next())
            .and_then(|t| t.get("game"))
            .and_then(Item::as_str)
            .and_then(me3_id_to_game)
            .unwrap_or(Game::EldenRing);

        let natives = doc
            .get("natives")
            .and_then(Item::as_array_of_tables)
            .map(|tables| {
                tables
                    .iter()
                    .filter_map(|t| {
                        Some(Me3Native {
                            path: t.get("path")?.as_str()?.to_string(),
                            enabled: t.get("enabled").and_then(Item::as_bool).unwrap_or(true),
                            optional: t.get("optional").and_then(Item::as_bool).unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let packages = doc
            .get("packages")
            .and_then(Item::as_array_of_tables)
            .map(|tables| {
                tables
                    .iter()
                    .filter_map(|t| {
                        Some(Me3Package {
                            path: t.get("path")?.as_str()?.to_string(),
                            id: t.get("id").and_then(Item::as_str).map(str::to_string),
                            enabled: t.get("enabled").and_then(Item::as_bool).unwrap_or(true),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Me3Profile {
            game,
            natives,
            packages,
            savefile: doc.get("savefile").and_then(Item::as_str).map(str::to_string),
            start_online: doc.get("start_online").and_then(Item::as_bool),
            disable_arxan: doc.get("disable_arxan").and_then(Item::as_bool),
            mem_patch: doc.get("mem_patch").and_then(Item::as_bool),
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        std::fs::write(path, self.to_toml()).at(path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).at(path)?;
        Me3Profile::from_toml(&text)
    }
}

fn me3_id_to_game(id: &str) -> Option<Game> {
    match id {
        "eldenring" | "er" | "elden-ring" => Some(Game::EldenRing),
        "nightreign" | "nr" | "nightrein" => Some(Game::Nightreign),
        "darksouls3" | "ds3" => Some(Game::DarkSouls3),
        "sekiro" | "sdt" => Some(Game::Sekiro),
        "armoredcore6" | "ac6" => Some(Game::ArmoredCore6),
        _ => None,
    }
}

/// Writes per-game defaults into me3's own `me3.toml`, which is how a cracked copy
/// gets `skip_steam_init` and a custom executable path without passing flags every
/// time the game starts.
pub fn write_me3_game_defaults(
    game: Game,
    skip_steam_init: bool,
    exe: Option<&Path>,
    skip_logos: Option<bool>,
) -> Result<PathBuf> {
    let dir = me3_config_dir()
        .ok_or_else(|| Error::msg("could not locate the me3 configuration folder"))?;
    std::fs::create_dir_all(&dir).at(&dir)?;
    let path = dir.join("me3.toml");

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: DocumentMut = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing.parse().map_err(|e| Error::parse("me3.toml", e))?
    };

    let game_table = ensure_table(doc.as_table_mut(), "game");
    game_table.set_dotted(false);
    let entry = ensure_table(game_table, game.me3_id());
    entry["skip_steam_init"] = value(skip_steam_init);
    if let Some(exe) = exe {
        entry["exe"] = value(exe.to_string_lossy().to_string());
    }
    if let Some(skip) = skip_logos {
        entry["skip_logos"] = value(skip);
    }

    std::fs::write(&path, doc.to_string()).at(&path)?;
    Ok(path)
}

/// Path a Seamless Co-op DLL is referenced by, relative to the game folder.
pub const SEAMLESS_COOP_DLL: &str = "SeamlessCoop/ersc.dll";

/// Adds the Seamless Co-op DLL to a ModEngine 2 config if it is not already there.
/// Returns true when the config changed.
pub fn me2_add_coop_dll(config: &mut Me2Config, dll: &str) -> bool {
    let normalised = dll.replace('/', "\\");
    let already = config
        .external_dlls
        .iter()
        .any(|existing| existing.replace('/', "\\").eq_ignore_ascii_case(&normalised));
    if already {
        return false;
    }
    config.external_dlls.push(normalised);
    true
}

/// Same for a me3 profile.
pub fn me3_add_coop_native(profile: &mut Me3Profile, dll: &str) -> bool {
    let normalised = dll.replace('\\', "/");
    let already = profile
        .natives
        .iter()
        .any(|n| n.path.replace('\\', "/").eq_ignore_ascii_case(&normalised));
    if already {
        return false;
    }
    profile.natives.push(Me3Native {
        path: normalised,
        enabled: true,
        optional: false,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stock config shipped by ModEngine 2, trimmed of its comment blocks.
    const STOCK_ME2: &str = r#"
[modengine]
debug = false
external_dlls = []

[extension.mod_loader]
enabled = true
loose_params = false
mods = [
    { enabled = true, name = "default", path = "mod" }
]

[extension.scylla_hide]
enabled = false
"#;

    #[test]
    fn reads_the_stock_modengine2_config() {
        let config = parse_me2_config(STOCK_ME2).unwrap();
        assert!(!config.debug);
        assert!(config.external_dlls.is_empty());
        assert!(config.mod_loader_enabled);
        assert!(!config.loose_params);
        assert_eq!(config.mods.len(), 1);
        assert_eq!(config.mods[0].name, "default");
        assert_eq!(config.mods[0].path, "mod");
        assert!(config.mods[0].enabled);
        assert!(!config.scylla_hide);
    }

    #[test]
    fn me2_round_trips_through_a_file_and_keeps_comments() {
        let dir = std::env::temp_dir().join("roundtable-me2-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config_eldenring.toml");
        let with_comment = format!("# keep me\n{STOCK_ME2}");
        std::fs::write(&path, &with_comment).unwrap();

        let mut config = read_me2_config(&path).unwrap();
        assert!(me2_add_coop_dll(&mut config, SEAMLESS_COOP_DLL));
        write_me2_config(&path, &config).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"), "comments must survive a rewrite");

        let reread = read_me2_config(&path).unwrap();
        assert_eq!(reread.external_dlls, vec!["SeamlessCoop\\ersc.dll"]);
        assert_eq!(reread.mods.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn adding_the_coop_dll_twice_is_a_no_op() {
        let mut config = parse_me2_config(STOCK_ME2).unwrap();
        assert!(me2_add_coop_dll(&mut config, SEAMLESS_COOP_DLL));
        assert!(!me2_add_coop_dll(&mut config, SEAMLESS_COOP_DLL));
        // Separator style must not create a duplicate either.
        assert!(!me2_add_coop_dll(&mut config, "SeamlessCoop\\ersc.dll"));
        assert_eq!(config.external_dlls.len(), 1);
    }

    #[test]
    fn me3_profile_matches_the_documented_shape() {
        let mut profile = Me3Profile::new(Game::EldenRing);
        profile.packages.push(Me3Package {
            path: "mods/Convergence".into(),
            id: Some("convergence".into()),
            enabled: true,
        });
        me3_add_coop_native(&mut profile, SEAMLESS_COOP_DLL);

        let toml = profile.to_toml();
        assert!(toml.contains(r#"profileVersion = "v1""#));
        assert!(toml.contains("[[supports]]"));
        assert!(toml.contains(r#"game = "eldenring""#));
        assert!(toml.contains("[[packages]]"));
        assert!(toml.contains("[[natives]]"));
        assert!(toml.contains("SeamlessCoop/ersc.dll"));

        let parsed = Me3Profile::from_toml(&toml).unwrap();
        assert_eq!(parsed.game, Game::EldenRing);
        assert_eq!(parsed.packages.len(), 1);
        assert_eq!(parsed.packages[0].id.as_deref(), Some("convergence"));
        assert_eq!(parsed.natives.len(), 1);
    }

    #[test]
    fn me3_profile_keeps_savefile_and_flags() {
        let mut profile = Me3Profile::new(Game::EldenRing);
        profile.savefile = Some("Convergence.co2".into());
        profile.disable_arxan = Some(true);
        profile.mem_patch = Some(true);

        let parsed = Me3Profile::from_toml(&profile.to_toml()).unwrap();
        assert_eq!(parsed.savefile.as_deref(), Some("Convergence.co2"));
        assert_eq!(parsed.disable_arxan, Some(true));
        assert_eq!(parsed.mem_patch, Some(true));
        // Untouched options must stay absent rather than defaulting to false.
        assert_eq!(parsed.start_online, None);
    }

    #[test]
    fn disabled_entries_are_left_out_of_the_generated_profile() {
        let mut profile = Me3Profile::new(Game::EldenRing);
        profile.natives.push(Me3Native {
            path: "off.dll".into(),
            enabled: false,
            optional: false,
        });
        profile.packages.push(Me3Package {
            path: "off".into(),
            id: None,
            enabled: false,
        });
        let toml = profile.to_toml();
        assert!(!toml.contains("off.dll"));
        assert!(!toml.contains("[[natives]]"));
        assert!(!toml.contains("[[packages]]"));
    }

    #[test]
    fn every_documented_game_alias_resolves() {
        for (alias, expected) in [
            ("er", Game::EldenRing),
            ("elden-ring", Game::EldenRing),
            ("nightreign", Game::Nightreign),
            ("nr", Game::Nightreign),
            ("ds3", Game::DarkSouls3),
            ("sdt", Game::Sekiro),
            ("ac6", Game::ArmoredCore6),
        ] {
            assert_eq!(me3_id_to_game(alias), Some(expected), "alias {alias}");
        }
        assert_eq!(me3_id_to_game("bloodborne"), None);
    }

    #[test]
    fn only_me3_advertises_skip_steam_init() {
        assert!(LoaderKind::Me3.supports_skip_steam_init());
        assert!(!LoaderKind::ModEngine2.supports_skip_steam_init());
    }
}
