//! Forcing the game's language on a copy that emulates Steam.
//!
//! A Steam copy takes its language from Steam. A repack has no Steam, so the
//! emulator has to be told, and every emulator keeps its own config with its own
//! spelling of the same setting. Repacks routinely ship two of them and set the
//! language in one, which is how a Russian repack starts in English.
//!
//! Roundtable writes the choice into all of them, including the ones where the
//! line has been commented out.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IoContext, Result};

/// The languages ELDEN RING ships, as the emulators spell them.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("english", "English"),
    ("russian", "Русский"),
    ("japanese", "日本語"),
    ("koreana", "한국어"),
    ("schinese", "简体中文"),
    ("tchinese", "繁體中文"),
    ("french", "Français"),
    ("italian", "Italiano"),
    ("german", "Deutsch"),
    ("spanish", "Español"),
    ("latam", "Español (Latinoamérica)"),
    ("polish", "Polski"),
    ("brazilian", "Português do Brasil"),
    ("thai", "ไทย"),
];

/// Config files emulators read, and the key each uses for the language.
const CONFIGS: &[(&str, &str)] = &[
    ("SteamFix.ini", "Language"),
    ("steam_emu.ini", "Language"),
    ("ColdClientLoader.ini", "Language"),
    ("OnlineFix.ini", "Language"),
    ("SmartSteamEmu.ini", "Language"),
    ("hlm.ini", "Language"),
    ("ALI213.ini", "Language"),
    ("valve.ini", "Language"),
    ("steam_settings/force_language.txt", ""),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageFile {
    pub file: String,
    pub path: PathBuf,
    /// What it is set to now, or nothing when the line is absent or commented.
    pub value: Option<String>,
    /// True when the line exists but is commented out, which is the usual cause
    /// of a repack starting in the wrong language.
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageStatus {
    pub files: Vec<LanguageFile>,
    /// The language actually in force, if the files agree.
    pub current: Option<String>,
    /// True when two configs disagree, which means one of them is being ignored.
    pub conflict: bool,
    pub options: Vec<(String, String)>,
    /// The repack's own picker, when it ships one.
    pub selector: Option<PathBuf>,
}

/// Reads a value out of an ini, noting whether the line is commented out.
fn read_ini(text: &str, key: &str) -> (Option<String>, bool) {
    let mut disabled = false;
    for line in text.lines() {
        let trimmed = line.trim();
        let bare = trimmed.trim_start_matches(['#', ';']).trim();

        let Some((name, value)) = bare.split_once('=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case(key) {
            continue;
        }
        let value = value.trim().to_string();
        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            disabled = true;
            continue;
        }
        if !value.is_empty() {
            return (Some(value.to_ascii_lowercase()), false);
        }
    }
    (None, disabled)
}

/// Sets a key, uncommenting the line when it is only commented out.
///
/// Rewriting the file wholesale would drop the emulator's own comments, and
/// those are the only documentation a repack ships.
fn write_ini(text: &str, key: &str, value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut written = false;

    for line in text.lines() {
        let trimmed = line.trim();
        let bare = trimmed.trim_start_matches(['#', ';']).trim();

        let matches_key = bare
            .split_once('=')
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key));

        if matches_key && !written {
            out.push(format!("{key}={value}"));
            written = true;
        } else if matches_key {
            // A second copy of the same key would be read instead of the first.
            out.push(format!(";{line}"));
        } else {
            out.push(line.to_string());
        }
    }

    if !written {
        // No line to fix, so add one under the first section header.
        let at = out
            .iter()
            .position(|line| line.trim().starts_with('['))
            .map_or(0, |index| index + 1);
        out.insert(at, format!("{key}={value}"));
    }

    let mut joined = out.join("\r\n");
    if text.ends_with('\n') {
        joined.push_str("\r\n");
    }
    joined
}

/// Reads every language setting in a game folder.
pub fn status(game_dir: &Path) -> LanguageStatus {
    let mut files = Vec::new();

    for (name, key) in CONFIGS {
        let path = game_dir.join(name);
        if !path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };

        let (value, disabled) = if key.is_empty() {
            // A bare text file holding nothing but the language.
            let value = text.trim().to_ascii_lowercase();
            ((!value.is_empty()).then_some(value), false)
        } else {
            read_ini(&text, key)
        };

        files.push(LanguageFile {
            file: (*name).to_string(),
            path,
            value,
            disabled,
        });
    }

    let live: Vec<&String> = files.iter().filter_map(|f| f.value.as_ref()).collect();
    let conflict = live.windows(2).any(|pair| pair[0] != pair[1]);
    let current = live.first().map(|v| (*v).clone());

    let selector = ["Language Selector.exe", "LanguageSelector.exe", "language.exe"]
        .iter()
        .map(|name| game_dir.join(name))
        .find(|path| path.is_file());

    LanguageStatus {
        files,
        current,
        conflict,
        options: LANGUAGES
            .iter()
            .map(|(id, label)| ((*id).to_string(), (*label).to_string()))
            .collect(),
        selector,
    }
}

/// Writes a language into every config the game folder has.
pub fn set(game_dir: &Path, language: &str) -> Result<Vec<PathBuf>> {
    let language = language.trim().to_ascii_lowercase();
    if !LANGUAGES.iter().any(|(id, _)| *id == language) {
        return Err(crate::error::Error::msg(format!(
            "{language} is not one of the game's languages"
        )));
    }

    let mut written = Vec::new();

    for (name, key) in CONFIGS {
        let path = game_dir.join(name);
        if !path.is_file() {
            continue;
        }

        if key.is_empty() {
            std::fs::write(&path, &language).at(&path)?;
            written.push(path);
            continue;
        }

        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let updated = write_ini(&text, key, &language);
        if updated != text {
            // Keep the original once, so a bad guess is recoverable.
            let backup = path.with_extension(format!(
                "{}.roundtable-bak",
                path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
            ));
            if !backup.exists() {
                let _ = std::fs::copy(&path, &backup);
            }
            std::fs::write(&path, updated).at(&path)?;
            written.push(path);
        }
    }

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact shape of the file that shipped with the user's repack.
    const STEAMFIX: &str = "[Main]\r\nRealAppId=1245620\r\nFakeAppId=705210\r\nBuildId=0\r\n#Language=russian\r\n\r\n[Misc]\r\nOverlay=true\r\n";

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roundtable-lang-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_commented_line_reads_as_not_set_but_present() {
        let (value, disabled) = read_ini(STEAMFIX, "Language");
        assert_eq!(value, None, "a commented line is not in force");
        assert!(disabled, "and it is worth saying it is only commented out");
    }

    #[test]
    fn setting_a_language_uncomments_the_line_in_place() {
        let out = write_ini(STEAMFIX, "Language", "russian");
        assert!(out.contains("Language=russian"));
        assert!(!out.contains("#Language"), "got {out}");
        // The rest of the file survives.
        assert!(out.contains("RealAppId=1245620"));
        assert!(out.contains("[Misc]"));
        assert!(out.contains("Overlay=true"));
    }

    #[test]
    fn an_existing_value_is_replaced_rather_than_duplicated() {
        let out = write_ini("[Main]\r\nLanguage=english\r\n", "Language", "russian");
        assert_eq!(out.matches("Language=").count(), 1, "got {out}");
        assert!(out.contains("Language=russian"));
    }

    #[test]
    fn a_missing_line_is_added_under_the_section() {
        let out = write_ini("[Main]\r\nRealAppId=1\r\n", "Language", "russian");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "[Main]");
        assert_eq!(lines[1], "Language=russian");
    }

    #[test]
    fn a_second_copy_of_the_key_is_commented_out() {
        // Two live values means one is being read and the other silently is not.
        let out = write_ini(
            "[Main]\r\nLanguage=english\r\n[Other]\r\nLanguage=french\r\n",
            "Language",
            "russian",
        );
        let live = out
            .lines()
            .filter(|line| {
                let t = line.trim();
                !t.starts_with('#') && !t.starts_with(';') && t.to_lowercase().starts_with("language=")
            })
            .count();
        assert_eq!(live, 1, "exactly one value can be in force: {out}");
        assert!(out.contains("Language=russian"), "got {out}");
        assert!(out.contains(";Language=french"), "got {out}");
    }

    #[test]
    fn every_config_in_the_folder_is_written() {
        let dir = temp("all");
        std::fs::write(dir.join("SteamFix.ini"), STEAMFIX).unwrap();
        std::fs::write(dir.join("steam_emu.ini"), "[Settings]\r\nLanguage=english\r\n").unwrap();

        let written = set(&dir, "russian").unwrap();
        assert_eq!(written.len(), 2, "both, or the one left behind wins");

        let after = status(&dir);
        assert_eq!(after.current.as_deref(), Some("russian"));
        assert!(!after.conflict);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_configs_disagreeing_is_reported() {
        let dir = temp("conflict");
        std::fs::write(dir.join("SteamFix.ini"), "[Main]\r\nLanguage=english\r\n").unwrap();
        std::fs::write(dir.join("steam_emu.ini"), "[Settings]\r\nLanguage=russian\r\n").unwrap();

        let found = status(&dir);
        assert!(found.conflict, "one of these is being ignored");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_original_is_kept_before_the_first_write() {
        let dir = temp("backup");
        std::fs::write(dir.join("SteamFix.ini"), STEAMFIX).unwrap();

        set(&dir, "russian").unwrap();
        let backup = dir.join("SteamFix.ini.roundtable-bak");
        assert!(backup.is_file(), "the original must be recoverable");
        assert!(std::fs::read_to_string(&backup).unwrap().contains("#Language"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_language_the_game_does_not_have_is_refused() {
        let dir = temp("bogus");
        std::fs::write(dir.join("SteamFix.ini"), STEAMFIX).unwrap();
        assert!(set(&dir, "klingon").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_repacks_own_picker_is_found_when_it_ships_one() {
        let dir = temp("selector");
        std::fs::write(dir.join("Language Selector.exe"), b"x").unwrap();
        assert!(status(&dir).selector.is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
