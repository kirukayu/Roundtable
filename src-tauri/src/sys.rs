//! Shader cache maintenance, system reporting and disk housekeeping.
//!
//! Stale shader caches are the usual cause of the stutter people blame on mods:
//! the driver keeps compiled shaders keyed to a driver version and a game build,
//! and after either changes the cache is dead weight that still gets consulted.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;
use crate::games::Game;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheLocation {
    pub label: String,
    pub path: PathBuf,
    pub exists: bool,
    pub size_bytes: u64,
    pub file_count: usize,
    /// Vendor or subsystem this cache belongs to.
    pub owner: String,
}

fn local_appdata() -> Option<PathBuf> {
    dirs::data_local_dir()
}

fn program_data() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(PathBuf::from)
}

/// Every shader cache Roundtable knows how to clear.
pub fn shader_caches() -> Vec<CacheLocation> {
    let mut candidates: Vec<(String, String, PathBuf)> = Vec::new();

    if let Some(local) = local_appdata() {
        for (owner, label, rel) in [
            ("NVIDIA", "DirectX shader cache", "NVIDIA/DXCache"),
            ("NVIDIA", "OpenGL shader cache", "NVIDIA/GLCache"),
            ("NVIDIA", "Driver cache", "NVIDIA Corporation/NV_Cache"),
            ("AMD", "DirectX shader cache", "AMD/DxCache"),
            ("AMD", "DirectX compiler cache", "AMD/DxcCache"),
            ("AMD", "OpenGL shader cache", "AMD/GLCache"),
            ("AMD", "Vulkan shader cache", "AMD/VkCache"),
            ("Intel", "Shader cache", "Intel/ShaderCache"),
            ("Windows", "D3D shader cache", "D3DSCache"),
        ] {
            candidates.push((owner.to_string(), label.to_string(), local.join(rel)));
        }
    }

    if let Some(program_data) = program_data() {
        candidates.push((
            "NVIDIA".into(),
            "Machine-wide driver cache".into(),
            program_data.join("NVIDIA Corporation").join("NV_Cache"),
        ));
    }

    candidates
        .into_iter()
        .map(|(owner, label, path)| {
            let (size_bytes, file_count) = measure(&path);
            CacheLocation {
                exists: path.is_dir(),
                label,
                owner,
                size_bytes,
                file_count,
                path,
            }
        })
        .collect()
}

fn measure(path: &Path) -> (u64, usize) {
    if !path.is_dir() {
        return (0, 0);
    }
    let mut size = 0u64;
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() {
                size += metadata.len();
                count += 1;
            }
        }
    }
    (size, count)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanReport {
    pub cleared: Vec<String>,
    pub skipped: Vec<String>,
    pub bytes_freed: u64,
    pub files_removed: usize,
}

/// Empties the given caches. The folders themselves are kept so drivers do not
/// have to recreate them, and anything locked by a running process is skipped
/// rather than treated as a failure.
pub fn clear_caches(paths: &[PathBuf]) -> Result<CleanReport> {
    let mut report = CleanReport {
        cleared: Vec::new(),
        skipped: Vec::new(),
        bytes_freed: 0,
        files_removed: 0,
    };

    for path in paths {
        if !is_safe_cache_path(path) {
            report
                .skipped
                .push(format!("{}: not a recognised cache folder", path.display()));
            continue;
        }
        if !path.is_dir() {
            continue;
        }

        let (before, _) = measure(path);
        let mut removed = 0usize;
        let mut locked = 0usize;

        let Ok(children) = std::fs::read_dir(path) else {
            report
                .skipped
                .push(format!("{}: could not be opened", path.display()));
            continue;
        };

        for child in children.flatten() {
            let target = child.path();
            let outcome = if target.is_dir() {
                std::fs::remove_dir_all(&target)
            } else {
                std::fs::remove_file(&target)
            };
            match outcome {
                Ok(()) => removed += 1,
                Err(_) => locked += 1,
            }
        }

        let (after, _) = measure(path);
        report.bytes_freed += before.saturating_sub(after);
        report.files_removed += removed;

        if locked > 0 {
            report.skipped.push(format!(
                "{}: {locked} item(s) are in use and were left alone",
                path.display()
            ));
        }
        if removed > 0 {
            report.cleared.push(path.display().to_string());
        }
    }

    Ok(report)
}

/// Guards against a caller passing something like `C:\Windows`.
fn is_safe_cache_path(path: &Path) -> bool {
    let lossy = path.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
    const NEEDLES: &[&str] = &[
        "dxcache",
        "dxccache",
        "glcache",
        "vkcache",
        "nv_cache",
        "shadercache",
        "d3dscache",
    ];
    // The path must both live under a user or program data folder and name a cache.
    let plausible_root = [local_appdata(), program_data()]
        .into_iter()
        .flatten()
        .any(|root| {
            path.starts_with(&root)
        });
    plausible_root && NEEDLES.iter().any(|needle| lossy.contains(needle))
}

/// The game's own configuration folder, useful for "reset graphics settings".
pub fn graphics_config(game: Game) -> Option<PathBuf> {
    dirs::data_dir()
        .map(|d| d.join(game.appdata_folder()).join("GraphicsConfig.xml"))
        .filter(|p| p.is_file())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub mount: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemReport {
    pub os: String,
    pub cpu: String,
    pub cpu_cores: usize,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub disks: Vec<DiskInfo>,
    pub steam_running: bool,
    pub game_running: bool,
}

pub fn system_report(game: Game) -> SystemReport {
    let mut system = sysinfo::System::new_all();
    system.refresh_all();

    let cpu = system
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_else(|| "Unknown".into());

    let disks = sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .map(|disk| DiskInfo {
            mount: disk.mount_point().to_string_lossy().to_string(),
            total_bytes: disk.total_space(),
            available_bytes: disk.available_space(),
        })
        .collect();

    SystemReport {
        os: format!(
            "{} {}",
            sysinfo::System::name().unwrap_or_else(|| "Windows".into()),
            sysinfo::System::os_version().unwrap_or_default()
        )
        .trim()
        .to_string(),
        cpu,
        cpu_cores: system.cpus().len(),
        total_memory_bytes: system.total_memory(),
        available_memory_bytes: system.available_memory(),
        disks,
        steam_running: crate::steam::is_running(&system),
        game_running: crate::launch::is_game_running(game, &system),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_paths_are_recognised_only_under_user_folders() {
        let Some(local) = local_appdata() else {
            return;
        };
        assert!(is_safe_cache_path(&local.join("NVIDIA").join("DXCache")));
        assert!(is_safe_cache_path(&local.join("AMD").join("VkCache")));
        assert!(is_safe_cache_path(&local.join("D3DSCache")));
    }

    #[test]
    fn system_folders_are_never_treated_as_caches() {
        for dangerous in [
            "C:\\Windows",
            "C:\\Windows\\System32",
            "C:\\",
            "D:\\Games\\ELDEN RING",
            "C:\\Users",
        ] {
            assert!(
                !is_safe_cache_path(Path::new(dangerous)),
                "{dangerous} must be rejected"
            );
        }
    }

    #[test]
    fn a_cache_named_folder_outside_appdata_is_still_rejected() {
        assert!(!is_safe_cache_path(Path::new("D:\\DXCache")));
        assert!(!is_safe_cache_path(Path::new("C:\\Windows\\ShaderCache")));
    }

    #[test]
    fn clearing_refuses_unsafe_paths_without_touching_them() {
        let dir = std::env::temp_dir().join("roundtable-fake-cache");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("important.bin"), b"data").unwrap();

        let report = clear_caches(&[dir.clone()]).unwrap();
        assert_eq!(report.files_removed, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(dir.join("important.bin").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn measuring_a_missing_folder_is_zero_not_an_error() {
        assert_eq!(measure(Path::new("Z:\\definitely-not-here")), (0, 0));
    }

    #[test]
    fn every_listed_cache_has_a_label_and_owner() {
        for cache in shader_caches() {
            assert!(!cache.label.is_empty());
            assert!(!cache.owner.is_empty());
        }
    }
}
