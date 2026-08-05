//! Save discovery, backup, conversion and cross-account transfer.
//!
//! ELDEN RING keeps saves in `%APPDATA%\EldenRing\<SteamID64>\`. A Steam copy uses
//! the real account id; a repack uses whatever id its Steamworks emulator invented,
//! which is why moving a character between the two is not a plain file copy — the
//! account id is baked into the save and has to be rewritten.
//!
//! Seamless Co-op writes the same container under a different extension (`co2` by
//! default) so the vanilla game never opens it. Converting between the two is a
//! rename plus, when the accounts differ, a rebind.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, IoContext, Result};
use crate::formats::save::{SaveFile, SaveSummary};
use crate::games::Game;
use crate::steam;

/// Extensions that always belong to a save container, plus whatever custom
/// extension the user configured for co-op.
const KNOWN_EXTENSIONS: &[&str] = &["sl2", "co2"];

/// Steam's placeholder id, handed out by most Steamworks emulators. Seeing it is a
/// strong hint that a save folder belongs to a cracked copy.
const ANONYMOUS_STEAM_ID: u64 = 76_561_197_960_287_930;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SaveFlavour {
    Vanilla,
    SeamlessCoop,
    /// The game's own rolling backup, written next to the live save.
    GameBackup,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveEntry {
    pub path: PathBuf,
    pub file_name: String,
    pub extension: String,
    pub flavour: SaveFlavour,
    pub size_bytes: u64,
    pub modified: Option<String>,
    /// SteamID64 taken from the folder name.
    pub folder_id: Option<u64>,
    /// Friendly account name when this id belongs to a local Steam login.
    pub account_name: Option<String>,
    /// True when the folder id is not a Steam account known to this machine.
    pub likely_cracked: bool,
    /// Populated on demand; listing thousands of saves does not parse them all.
    pub summary: Option<SaveSummary>,
    /// Hash of the file contents, used to spot duplicates across folders.
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveFolder {
    pub path: PathBuf,
    pub folder_id: Option<u64>,
    pub account_name: Option<String>,
    pub likely_cracked: bool,
    pub entries: Vec<SaveEntry>,
}

/// Root of the game's save tree, e.g. `%APPDATA%\EldenRing`.
pub fn save_root(game: Game) -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(game.appdata_folder()))
}

/// Additional roots used by Steamworks emulators that redirect the save path.
fn extra_roots(game: Game) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(data) = dirs::data_dir() {
        roots.push(
            data.join("Goldberg SteamEmu Saves")
                .join(game.steam_app_id().to_string())
                .join(game.appdata_folder()),
        );
    }
    if let Some(docs) = dirs::document_dir() {
        roots.push(docs.join(game.appdata_folder()));
    }
    roots.into_iter().filter(|p| p.is_dir()).collect()
}

/// Lists every save folder Roundtable can see, labelling Steam accounts by name.
pub fn discover(game: Game, extra_extension: Option<&str>) -> Vec<SaveFolder> {
    let accounts: BTreeMap<u64, String> = steam::local_accounts()
        .into_iter()
        .map(|a| {
            let label = if a.persona_name.is_empty() {
                a.account_name
            } else {
                a.persona_name
            };
            (a.steam_id64, label)
        })
        .collect();

    let mut roots = Vec::new();
    if let Some(root) = save_root(game) {
        roots.push(root);
    }
    roots.extend(extra_roots(game));

    let mut folders = Vec::new();
    for root in roots {
        let Ok(children) = std::fs::read_dir(&root) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            if !path.is_dir() {
                continue;
            }
            let folder_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse::<u64>().ok());

            let entries = list_entries(&path, extra_extension);
            if entries.is_empty() {
                continue;
            }

            let account_name = folder_id.and_then(|id| accounts.get(&id).cloned());
            let likely_cracked = match folder_id {
                Some(id) => id == ANONYMOUS_STEAM_ID || account_name.is_none(),
                None => true,
            };

            folders.push(SaveFolder {
                folder_id,
                account_name: account_name.clone(),
                likely_cracked,
                entries: entries
                    .into_iter()
                    .map(|mut e| {
                        e.folder_id = folder_id;
                        e.account_name = account_name.clone();
                        e.likely_cracked = likely_cracked;
                        e
                    })
                    .collect(),
                path,
            });
        }
    }

    folders.sort_by(|a, b| a.likely_cracked.cmp(&b.likely_cracked).then(a.path.cmp(&b.path)));
    folders
}

fn list_entries(folder: &Path, extra_extension: Option<&str>) -> Vec<SaveEntry> {
    let Ok(children) = std::fs::read_dir(folder) else {
        return Vec::new();
    };

    let mut entries: Vec<SaveEntry> = children
        .flatten()
        .filter_map(|child| {
            let path = child.path();
            if !path.is_file() {
                return None;
            }
            let file_name = path.file_name()?.to_string_lossy().to_string();
            let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();

            let is_backup = extension == "bak";
            // `ER0000.sl2.bak` keeps its real kind in the stem.
            let effective = if is_backup {
                Path::new(path.file_stem()?)
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default()
            } else {
                extension.clone()
            };

            let recognised = KNOWN_EXTENSIONS.contains(&effective.as_str())
                || extra_extension.is_some_and(|x| x.eq_ignore_ascii_case(&effective));
            if !recognised {
                return None;
            }

            let metadata = child.metadata().ok();
            let flavour = if is_backup {
                SaveFlavour::GameBackup
            } else if effective == "sl2" {
                SaveFlavour::Vanilla
            } else {
                SaveFlavour::SeamlessCoop
            };

            Some(SaveEntry {
                file_name,
                extension: effective,
                flavour,
                size_bytes: metadata.as_ref().map(std::fs::Metadata::len).unwrap_or(0),
                modified: metadata
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .map(|t| DateTime::<Local>::from(t).to_rfc3339()),
                folder_id: None,
                account_name: None,
                likely_cracked: false,
                summary: None,
                content_hash: None,
                path,
            })
        })
        .collect();

    entries.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    entries
}

pub fn load(path: &Path) -> Result<SaveFile> {
    let bytes = std::fs::read(path).at(path)?;
    SaveFile::from_bytes(bytes)
}

/// Reads a save and returns its slot listing.
pub fn inspect(path: &Path) -> Result<SaveSummary> {
    load(path)?.summary()
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).at(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Groups saves that are byte-identical, which is how the same character ends up
/// looking like several different ones after a few manual copies.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub hash: String,
    pub size_bytes: u64,
    pub paths: Vec<PathBuf>,
}

pub fn find_duplicates(paths: &[PathBuf]) -> Result<Vec<DuplicateGroup>> {
    let mut by_size: BTreeMap<u64, Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        by_size.entry(metadata.len()).or_default().push(path.clone());
    }

    let mut groups: Vec<DuplicateGroup> = Vec::new();
    for (size, candidates) in by_size {
        // Hashing is only worth it once at least two files share a size.
        if candidates.len() < 2 {
            continue;
        }
        let mut by_hash: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for path in candidates {
            if let Ok(hash) = hash_file(&path) {
                by_hash.entry(hash).or_default().push(path);
            }
        }
        for (hash, paths) in by_hash {
            if paths.len() > 1 {
                groups.push(DuplicateGroup {
                    hash,
                    size_bytes: size,
                    paths,
                });
            }
        }
    }

    Ok(groups)
}

// ---------------------------------------------------------------------------
// Backups
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRecord {
    pub id: String,
    pub game: Game,
    pub created: String,
    pub label: String,
    /// Where the save was copied from, so a restore can put it back.
    pub origin: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub steam_id: Option<u64>,
    /// Short description of each occupied slot, for the restore list.
    pub characters: Vec<String>,
    pub automatic: bool,
}

pub fn backup_dir(app_data: &Path, game: Game) -> PathBuf {
    app_data.join("backups").join(game.appdata_folder())
}

/// Copies a save into the backup store and records what was in it.
pub fn create_backup(
    app_data: &Path,
    game: Game,
    source: &Path,
    label: &str,
    automatic: bool,
) -> Result<BackupRecord> {
    if !source.is_file() {
        return Err(Error::msg(format!("{} does not exist", source.display())));
    }

    let dir = backup_dir(app_data, game);
    std::fs::create_dir_all(&dir).at(&dir)?;

    let now = Local::now();
    let id = format!("{}-{}", now.format("%Y%m%d-%H%M%S"), short_id());
    let file_name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| game.save_file().to_string());

    let payload = dir.join(format!("{id}.save"));
    std::fs::copy(source, &payload).at(source)?;

    // Reading the character list is best effort: a corrupt save should still be
    // backed up, that is exactly when a backup matters most.
    let (steam_id, characters) = match load(source).and_then(|s| s.summary()) {
        Ok(summary) => (
            Some(summary.steam_id),
            summary
                .slots
                .iter()
                .filter(|s| s.active)
                .map(|s| {
                    let name = if s.name.trim().is_empty() {
                        "Unnamed".to_string()
                    } else {
                        s.name.clone()
                    };
                    format!("{name} - level {}", s.level)
                })
                .collect(),
        ),
        Err(_) => (None, Vec::new()),
    };

    let record = BackupRecord {
        id: id.clone(),
        game,
        created: now.to_rfc3339(),
        label: label.to_string(),
        origin: source.to_path_buf(),
        file_name,
        size_bytes: std::fs::metadata(&payload).map(|m| m.len()).unwrap_or(0),
        steam_id,
        characters,
        automatic,
    };

    let meta_path = dir.join(format!("{id}.json"));
    std::fs::write(&meta_path, serde_json::to_vec_pretty(&record)?).at(&meta_path)?;

    Ok(record)
}

pub fn list_backups(app_data: &Path, game: Game) -> Vec<BackupRecord> {
    let dir = backup_dir(app_data, game);
    let Ok(children) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut records: Vec<BackupRecord> = children
        .flatten()
        .filter(|c| c.path().extension().is_some_and(|e| e == "json"))
        .filter_map(|c| std::fs::read(c.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<BackupRecord>(&bytes).ok())
        .collect();

    records.sort_by(|a, b| b.created.cmp(&a.created));
    records
}

/// Restores a backup. The current file is itself backed up first, so a restore is
/// never a one-way door.
pub fn restore_backup(
    app_data: &Path,
    game: Game,
    backup_id: &str,
    destination: Option<&Path>,
) -> Result<PathBuf> {
    let dir = backup_dir(app_data, game);
    let meta_path = dir.join(format!("{backup_id}.json"));
    let payload = dir.join(format!("{backup_id}.save"));

    let record: BackupRecord = serde_json::from_slice(&std::fs::read(&meta_path).at(&meta_path)?)?;
    if !payload.is_file() {
        return Err(Error::msg(format!(
            "backup {backup_id} is missing its data file"
        )));
    }

    let target = destination.map(Path::to_path_buf).unwrap_or(record.origin);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }

    if target.is_file() {
        create_backup(app_data, game, &target, "before restore", true)?;
    }

    std::fs::copy(&payload, &target).at(&payload)?;
    Ok(target)
}

pub fn delete_backup(app_data: &Path, game: Game, backup_id: &str) -> Result<()> {
    let dir = backup_dir(app_data, game);
    for suffix in ["json", "save"] {
        let path = dir.join(format!("{backup_id}.{suffix}"));
        if path.exists() {
            std::fs::remove_file(&path).at(&path)?;
        }
    }
    Ok(())
}

/// Keeps the newest `keep` automatic backups and deletes the rest. Manual backups
/// are never pruned.
pub fn prune_backups(app_data: &Path, game: Game, keep: usize) -> Result<usize> {
    let automatic: Vec<BackupRecord> = list_backups(app_data, game)
        .into_iter()
        .filter(|r| r.automatic)
        .collect();

    let mut removed = 0;
    for record in automatic.into_iter().skip(keep) {
        delete_backup(app_data, game, &record.id)?;
        removed += 1;
    }
    Ok(removed)
}

fn short_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..6)
        .map(|_| {
            let n: u8 = rng.random_range(0..36);
            char::from_digit(u32::from(n), 36).unwrap_or('0')
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Transfer and conversion
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferReport {
    pub destination: PathBuf,
    pub slots_copied: Vec<usize>,
    pub rebound_from: Option<u64>,
    pub rebound_to: Option<u64>,
    pub backup_id: Option<String>,
}

/// Copies chosen characters from one save into another, rewriting the account id so
/// the destination account owns them.
///
/// This is the pirate-to-Steam path and vice versa: the two installs use different
/// folders *and* different ids, and the id is what the game checks.
pub fn transfer_slots(
    app_data: &Path,
    game: Game,
    source_path: &Path,
    destination_path: &Path,
    slot_pairs: &[(usize, usize)],
) -> Result<TransferReport> {
    if slot_pairs.is_empty() {
        return Err(Error::msg("no characters were selected to copy"));
    }
    if source_path == destination_path {
        return Err(Error::msg(
            "the source and destination are the same file; pick a different destination",
        ));
    }

    let source = load(source_path)?;

    // The destination may not exist yet, in which case the source becomes the
    // starting point and is rebound wholesale.
    let mut destination = if destination_path.is_file() {
        load(destination_path)?
    } else {
        let mut fresh = source.clone();
        for slot in 0..crate::formats::save::SLOT_COUNT {
            fresh.clear_slot(slot)?;
        }
        fresh
    };

    let backup_id = if destination_path.is_file() {
        Some(
            create_backup(app_data, game, destination_path, "before transfer", true)?
                .id,
        )
    } else {
        None
    };

    let source_id = source.steam_id();
    let target_id = destination.steam_id();

    let mut copied = Vec::new();
    for (from, to) in slot_pairs {
        destination.import_slot(&source, *from, *to)?;
        copied.push(*to);
    }

    if let Some(parent) = destination_path.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    write_atomic(destination_path, destination.as_bytes())?;

    Ok(TransferReport {
        destination: destination_path.to_path_buf(),
        slots_copied: copied,
        rebound_from: (source_id != target_id).then_some(source_id),
        rebound_to: (source_id != target_id).then_some(target_id),
        backup_id,
    })
}

/// Rewrites a whole save so a different account owns every character in it.
pub fn rebind(app_data: &Path, game: Game, path: &Path, new_steam_id: u64) -> Result<String> {
    let mut save = load(path)?;
    let backup = create_backup(app_data, game, path, "before rebind", true)?;
    save.rebind_to_account(new_steam_id);
    write_atomic(path, save.as_bytes())?;
    Ok(backup.id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversionReport {
    pub destination: PathBuf,
    pub rebound: bool,
    pub overwrote_existing: bool,
}

/// Converts between the vanilla `sl2` container and Seamless Co-op's `co2`
/// (or any custom extension), optionally rebinding to another account.
pub fn convert(
    app_data: &Path,
    game: Game,
    source_path: &Path,
    target_extension: &str,
    destination_dir: Option<&Path>,
    rebind_to: Option<u64>,
) -> Result<ConversionReport> {
    let extension = target_extension.trim().trim_start_matches('.');
    if extension.is_empty() || !extension.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::msg(
            "the target extension must be letters and digits only",
        ));
    }

    let mut save = load(source_path)?;

    let stem = source_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| game.save_file().to_string());
    // `ER0000.sl2.bak` has a stem of `ER0000.sl2`; strip that too.
    let stem = Path::new(&stem)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(stem);

    let dir = destination_dir
        .map(Path::to_path_buf)
        .or_else(|| source_path.parent().map(Path::to_path_buf))
        .ok_or_else(|| Error::msg("could not work out where to write the converted save"))?;
    std::fs::create_dir_all(&dir).at(&dir)?;

    let destination = dir.join(format!("{stem}.{extension}"));
    if destination == source_path {
        return Err(Error::msg(
            "the converted save would overwrite the original; choose a different extension or folder",
        ));
    }

    let overwrote_existing = destination.is_file();
    if overwrote_existing {
        create_backup(app_data, game, &destination, "before conversion", true)?;
    }

    let rebound = match rebind_to {
        Some(id) if id != save.steam_id() => {
            save.rebind_to_account(id);
            true
        }
        _ => false,
    };

    write_atomic(&destination, save.as_bytes())?;

    Ok(ConversionReport {
        destination,
        rebound,
        overwrote_existing,
    })
}

/// Writes through a temporary file so a crash mid-write cannot leave a half-save
/// where a character used to be.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!(
        "{}.roundtable-tmp",
        path.extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default()
    ));
    std::fs::write(&temp, bytes).at(&temp)?;
    // Windows will not rename onto an existing file.
    if path.exists() {
        std::fs::remove_file(path).at(path)?;
    }
    std::fs::rename(&temp, path).at(&temp)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::save::SLOT_COUNT;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-saves-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Minimal but geometrically correct container.
    fn write_save(path: &Path, steam_id: u64, active_slot: Option<usize>) {
        let mut bytes = vec![0u8; 0x19003B0 + 0x60000];
        bytes[0..4].copy_from_slice(b"BND4");
        let mut save = SaveFile::from_bytes(bytes).unwrap();
        save.set_steam_id(steam_id);
        if let Some(slot) = active_slot {
            save.set_slot_active(slot, true).unwrap();
        }
        save.recompute_checksums();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, save.as_bytes()).unwrap();
    }

    #[test]
    fn transfer_rebinds_between_a_cracked_and_a_steam_account() {
        let dir = scratch("transfer");
        let app_data = dir.join("appdata");
        let cracked = dir.join("cracked").join("ER0000.sl2");
        let steam = dir.join("steam").join("ER0000.sl2");

        write_save(&cracked, ANONYMOUS_STEAM_ID, Some(0));
        write_save(&steam, 76561198111111111, None);

        let report =
            transfer_slots(&app_data, Game::EldenRing, &cracked, &steam, &[(0, 1)]).unwrap();

        assert_eq!(report.slots_copied, vec![1]);
        assert_eq!(report.rebound_from, Some(ANONYMOUS_STEAM_ID));
        assert_eq!(report.rebound_to, Some(76561198111111111));
        assert!(report.backup_id.is_some(), "the destination must be backed up first");

        let result = load(&steam).unwrap();
        assert!(result.verify_checksums(), "the game rejects bad checksums");
        assert!(result.is_slot_active(1).unwrap());
        assert_eq!(result.steam_id(), 76561198111111111);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn transfer_into_a_missing_destination_creates_an_empty_container_first() {
        let dir = scratch("transfer-new");
        let app_data = dir.join("appdata");
        let source = dir.join("src").join("ER0000.sl2");
        let destination = dir.join("dst").join("ER0000.sl2");

        write_save(&source, 42, Some(3));
        let report =
            transfer_slots(&app_data, Game::EldenRing, &source, &destination, &[(3, 0)]).unwrap();

        assert!(report.backup_id.is_none());
        let result = load(&destination).unwrap();
        assert!(result.is_slot_active(0).unwrap());
        // Every other slot must be empty, not a copy of the source's slots.
        for slot in 1..SLOT_COUNT {
            assert!(!result.is_slot_active(slot).unwrap(), "slot {slot} leaked");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn transfer_refuses_a_no_op_and_a_self_copy() {
        let dir = scratch("transfer-guard");
        let app_data = dir.join("appdata");
        let save = dir.join("ER0000.sl2");
        write_save(&save, 1, Some(0));

        assert!(transfer_slots(&app_data, Game::EldenRing, &save, &save, &[(0, 1)]).is_err());
        let other = dir.join("other.sl2");
        write_save(&other, 2, None);
        assert!(transfer_slots(&app_data, Game::EldenRing, &save, &other, &[]).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn conversion_produces_a_co2_next_to_the_original() {
        let dir = scratch("convert");
        let app_data = dir.join("appdata");
        let source = dir.join("ER0000.sl2");
        write_save(&source, 76561198000000001, Some(0));

        let report = convert(&app_data, Game::EldenRing, &source, "co2", None, None).unwrap();
        assert_eq!(report.destination, dir.join("ER0000.co2"));
        assert!(!report.rebound);
        assert!(report.destination.is_file());
        assert!(source.is_file(), "the original must be left alone");
        assert!(load(&report.destination).unwrap().verify_checksums());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn conversion_can_rebind_at_the_same_time() {
        let dir = scratch("convert-rebind");
        let app_data = dir.join("appdata");
        let source = dir.join("ER0000.sl2");
        write_save(&source, ANONYMOUS_STEAM_ID, Some(0));

        let report = convert(
            &app_data,
            Game::EldenRing,
            &source,
            "co2",
            None,
            Some(76561198222222222),
        )
        .unwrap();

        assert!(report.rebound);
        let converted = load(&report.destination).unwrap();
        assert_eq!(converted.steam_id(), 76561198222222222);
        assert!(converted.verify_checksums());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn conversion_rejects_an_extension_that_would_overwrite_the_source() {
        let dir = scratch("convert-guard");
        let app_data = dir.join("appdata");
        let source = dir.join("ER0000.sl2");
        write_save(&source, 1, None);

        assert!(convert(&app_data, Game::EldenRing, &source, "sl2", None, None).is_err());
        assert!(convert(&app_data, Game::EldenRing, &source, "", None, None).is_err());
        assert!(convert(&app_data, Game::EldenRing, &source, "co 2", None, None).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backups_round_trip_and_prune_only_automatic_ones() {
        let dir = scratch("backup");
        let app_data = dir.join("appdata");
        let save = dir.join("ER0000.sl2");
        write_save(&save, 7, Some(0));

        let manual = create_backup(&app_data, Game::EldenRing, &save, "manual", false).unwrap();
        for i in 0..5 {
            create_backup(&app_data, Game::EldenRing, &save, &format!("auto {i}"), true).unwrap();
        }
        assert_eq!(list_backups(&app_data, Game::EldenRing).len(), 6);

        let removed = prune_backups(&app_data, Game::EldenRing, 2).unwrap();
        assert_eq!(removed, 3);

        let remaining = list_backups(&app_data, Game::EldenRing);
        assert_eq!(remaining.len(), 3);
        assert!(
            remaining.iter().any(|r| r.id == manual.id),
            "a manual backup must never be pruned"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restoring_backs_up_what_it_replaces() {
        let dir = scratch("restore");
        let app_data = dir.join("appdata");
        let save = dir.join("ER0000.sl2");

        write_save(&save, 7, Some(0));
        let backup = create_backup(&app_data, Game::EldenRing, &save, "snapshot", false).unwrap();

        // Overwrite the live save with a different character layout.
        write_save(&save, 7, Some(5));
        assert!(load(&save).unwrap().is_slot_active(5).unwrap());

        restore_backup(&app_data, Game::EldenRing, &backup.id, None).unwrap();
        let restored = load(&save).unwrap();
        assert!(restored.is_slot_active(0).unwrap());
        assert!(!restored.is_slot_active(5).unwrap());

        // The pre-restore state must itself be recoverable.
        assert!(list_backups(&app_data, Game::EldenRing)
            .iter()
            .any(|r| r.label == "before restore"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_detection_groups_identical_files_only() {
        let dir = scratch("dupes");
        let a = dir.join("a.sl2");
        let b = dir.join("b.sl2");
        let c = dir.join("c.sl2");
        write_save(&a, 1, Some(0));
        std::fs::copy(&a, &b).unwrap();
        write_save(&c, 2, Some(0));

        let groups = find_duplicates(&[a.clone(), b.clone(), c.clone()]).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 2);
        assert!(groups[0].paths.contains(&a) && groups[0].paths.contains(&b));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn listing_recognises_vanilla_coop_and_backup_files() {
        let dir = scratch("listing");
        for name in ["ER0000.sl2", "ER0000.co2", "ER0000.sl2.bak", "notes.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }

        let entries = list_entries(&dir, None);
        let names: Vec<&str> = entries.iter().map(|e| e.file_name.as_str()).collect();
        assert!(names.contains(&"ER0000.sl2"));
        assert!(names.contains(&"ER0000.co2"));
        assert!(names.contains(&"ER0000.sl2.bak"));
        assert!(!names.contains(&"notes.txt"));

        let by_name = |n: &str| entries.iter().find(|e| e.file_name == n).unwrap();
        assert_eq!(by_name("ER0000.sl2").flavour, SaveFlavour::Vanilla);
        assert_eq!(by_name("ER0000.co2").flavour, SaveFlavour::SeamlessCoop);
        assert_eq!(by_name("ER0000.sl2.bak").flavour, SaveFlavour::GameBackup);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_custom_coop_extension_is_picked_up() {
        let dir = scratch("custom-ext");
        std::fs::write(dir.join("ER0000.mycoop"), b"x").unwrap();

        assert!(list_entries(&dir, None).is_empty());
        let entries = list_entries(&dir, Some("mycoop"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].flavour, SaveFlavour::SeamlessCoop);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_leaves_no_temporary_behind() {
        let dir = scratch("atomic");
        let path = dir.join("ER0000.sl2");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("roundtable-tmp"))
            .collect();
        assert!(leftovers.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
