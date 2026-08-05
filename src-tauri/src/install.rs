//! Getting a downloaded mod into the library.
//!
//! Mod archives are packed however the author felt like that day, so the files are
//! extracted to a scratch folder, the real asset root is located, and only that is
//! promoted into the library. The game folder is never written to.

use std::path::Path;

use chrono::Local;

use crate::error::{Error, IoContext, Result};
use crate::games::Game;
use crate::mods::{self, ModRecord};

/// Extracts an archive and installs whatever is inside.
pub fn from_archive(
    app_data: &Path,
    game: Game,
    archive: &Path,
    name: Option<&str>,
) -> Result<ModRecord> {
    let staging = app_data
        .join("staging")
        .join(format!("extract-{}", Local::now().timestamp_millis()));
    std::fs::create_dir_all(&staging).at(&staging)?;

    let result = (|| {
        extract(archive, &staging)?;
        let fallback = archive
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Mod".to_string());
        from_folder(app_data, game, &staging, Some(name.unwrap_or(&fallback)))
    })();

    std::fs::remove_dir_all(&staging).ok();
    result
}

/// Copies an already-extracted mod folder into the library.
pub fn from_folder(
    app_data: &Path,
    game: Game,
    source: &Path,
    name: Option<&str>,
) -> Result<ModRecord> {
    if !source.is_dir() {
        return Err(Error::msg(format!("{} is not a folder", source.display())));
    }

    let analysis = mods::analyse_layout(source);
    let display_name = name
        .map(str::to_string)
        .or_else(|| {
            source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "Mod".to_string());

    let id = unique_mod_id(app_data, game, &mods::slugify(&display_name));
    let destination = mods::library_dir(app_data, game).join(&id);
    std::fs::create_dir_all(&destination).at(&destination)?;

    copy_tree(&analysis.asset_root, &destination)?;

    // Native DLLs that sit outside the asset root would otherwise be left behind.
    for native in &analysis.natives {
        let from = analysis.asset_root.join(native);
        if from.is_file() {
            continue;
        }
        let alternative = source.join(native);
        if alternative.is_file() {
            let to = destination.join(native);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            std::fs::copy(&alternative, &to).at(&alternative)?;
        }
    }

    let (file_count, size_bytes) = count_tree(&destination);

    let record = ModRecord {
        id,
        name: display_name,
        version: None,
        author: None,
        summary: None,
        nexus_mod_id: None,
        game,
        kind: analysis.kind,
        path: destination,
        natives: analysis.natives,
        file_count,
        size_bytes,
        installed_at: Local::now().to_rfc3339(),
        bundled_loader: analysis.bundled_loader,
    };

    mods::save_record(app_data, &record)?;
    Ok(record)
}

fn unique_mod_id(app_data: &Path, game: Game, base: &str) -> String {
    let existing: Vec<String> = mods::list_mods(app_data, game)
        .into_iter()
        .map(|m| m.id)
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

/// Dispatches on the archive's extension.
pub fn extract(archive: &Path, destination: &Path) -> Result<()> {
    let extension = archive
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "zip" => extract_zip(archive, destination),
        "7z" => extract_7z(archive, destination),
        "rar" => Err(Error::Archive(
            "RAR archives are not supported directly. Extract it with 7-Zip or WinRAR first, then use \"Install from folder\".".into(),
        )),
        other => Err(Error::Archive(format!(
            "'{other}' archives are not supported. Use a zip or 7z file."
        ))),
    }
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).at(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Archive(e.to_string()))?;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| Error::Archive(e.to_string()))?;

        // `enclosed_name` rejects `..` traversal, which is what stops a hostile
        // archive from writing outside the destination.
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

fn extract_7z(archive: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination).at(destination)?;
    sevenz_rust2::decompress_file(archive, destination)
        .map_err(|e| Error::Archive(e.to_string()))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
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
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            std::fs::copy(entry.path(), &target).at(entry.path())?;
        }
    }
    Ok(())
}

fn count_tree(path: &Path) -> (usize, u64) {
    let mut count = 0usize;
    let mut size = 0u64;
    for entry in walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() {
                count += 1;
                size += metadata.len();
            }
        }
    }
    (count, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-install-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn installing_a_folder_promotes_the_asset_root() {
        let dir = scratch("folder");
        let app_data = dir.join("appdata");
        let source = dir.join("download").join("MyMod v1.0").join("mod");
        std::fs::create_dir_all(source.join("parts")).unwrap();
        std::fs::write(source.join("regulation.bin"), b"reg").unwrap();
        std::fs::write(source.join("parts").join("a.dcx"), b"part").unwrap();

        let record = from_folder(
            &app_data,
            Game::EldenRing,
            &dir.join("download"),
            Some("My Mod"),
        )
        .unwrap();

        assert_eq!(record.name, "My Mod");
        assert_eq!(record.id, "my-mod");
        // The wrapper folders must not appear inside the library entry.
        assert!(record.path.join("regulation.bin").is_file());
        assert!(record.path.join("parts").join("a.dcx").is_file());
        assert!(!record.path.join("mod").exists());
        assert_eq!(record.file_count, 2);

        // The record must be discoverable afterwards.
        let listed = mods::list_mods(&app_data, Game::EldenRing);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, record.id);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn installing_the_same_mod_twice_gets_a_distinct_id() {
        let dir = scratch("dupe-id");
        let app_data = dir.join("appdata");
        let source = dir.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("regulation.bin"), b"reg").unwrap();

        let first = from_folder(&app_data, Game::EldenRing, &source, Some("Overhaul")).unwrap();
        let second = from_folder(&app_data, Game::EldenRing, &source, Some("Overhaul")).unwrap();

        assert_eq!(first.id, "overhaul");
        assert_eq!(second.id, "overhaul-2");
        assert_ne!(first.path, second.path);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_zip_is_extracted_and_installed() {
        let dir = scratch("zip");
        let app_data = dir.join("appdata");
        let archive = dir.join("CoolMod.zip");
        make_zip(
            &archive,
            &[
                ("CoolMod/mod/regulation.bin", b"reg"),
                ("CoolMod/mod/parts/x.dcx", b"part"),
                ("CoolMod/readme.txt", b"hello"),
            ],
        );

        let record = from_archive(&app_data, Game::EldenRing, &archive, None).unwrap();
        assert_eq!(record.name, "CoolMod");
        assert!(record.path.join("regulation.bin").is_file());
        assert!(record.path.join("parts").join("x.dcx").is_file());

        // Staging must be cleaned up.
        assert!(!app_data.join("staging").join("extract").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_dll_only_zip_records_its_natives() {
        let dir = scratch("zip-dll");
        let app_data = dir.join("appdata");
        let archive = dir.join("Randomiser.zip");
        make_zip(&archive, &[("Randomiser.dll", b"MZ")]);

        let record = from_archive(&app_data, Game::EldenRing, &archive, None).unwrap();
        assert_eq!(record.kind, mods::ModKind::Native);
        assert_eq!(record.natives, vec!["Randomiser.dll"]);
        assert!(record.path.join("Randomiser.dll").is_file());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zip_slip_paths_cannot_escape_the_destination() {
        let dir = scratch("zip-slip");
        let archive = dir.join("evil.zip");
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        make_zip(&archive, &[("../../escaped.txt", b"pwned"), ("safe.txt", b"ok")]);

        extract(&archive, &out).unwrap();

        assert!(out.join("safe.txt").is_file());
        assert!(
            !dir.join("escaped.txt").exists(),
            "traversal entries must be dropped"
        );
        assert!(!dir.parent().unwrap().join("escaped.txt").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsupported_archives_explain_what_to_do() {
        let dir = scratch("unsupported");
        let rar = dir.join("mod.rar");
        std::fs::write(&rar, b"Rar!").unwrap();

        let err = extract(&rar, &dir).unwrap_err().to_string();
        assert!(err.contains("7-Zip"), "got: {err}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn deleting_a_mod_removes_its_files_and_record() {
        let dir = scratch("delete");
        let app_data = dir.join("appdata");
        let source = dir.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("regulation.bin"), b"reg").unwrap();

        let record = from_folder(&app_data, Game::EldenRing, &source, Some("Temp")).unwrap();
        assert!(record.path.is_dir());

        mods::delete_mod(&app_data, Game::EldenRing, &record.id).unwrap();
        assert!(!record.path.exists());
        assert!(mods::list_mods(&app_data, Game::EldenRing).is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
