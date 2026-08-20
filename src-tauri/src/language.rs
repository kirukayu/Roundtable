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

/// How a config wants the language written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Casing {
    /// `russian` — the Steam API's own code, which emulators pass straight to
    /// the game.
    Lower,
    /// `Russian` — how the repack launcher's own language table spells it.
    Title,
}

struct Config {
    file: &'static str,
    key: &'static str,
    /// Section to create the key under when it is missing.
    section: &'static str,
    casing: Casing,
}

/// Every file that can decide the language, and the key each one uses.
///
/// Writing all of them is the point. A repack ships two or three of these and
/// sets the language in one; the launcher has no way to know which is being
/// read, and an unrecognised key in an ini is ignored rather than harmful.
const CONFIGS: &[Config] = &[
    Config { file: "SteamFix.ini", key: "Language", section: "Main", casing: Casing::Lower },
    Config { file: "steam_emu.ini", key: "Language", section: "Settings", casing: Casing::Lower },
    Config { file: "ColdClientLoader.ini", key: "Language", section: "SteamClient", casing: Casing::Lower },
    Config { file: "OnlineFix.ini", key: "Language", section: "Main", casing: Casing::Lower },
    Config { file: "SmartSteamEmu.ini", key: "Language", section: "Steam", casing: Casing::Lower },
    Config { file: "hlm.ini", key: "Language", section: "Settings", casing: Casing::Lower },
    Config { file: "ALI213.ini", key: "Language", section: "Settings", casing: Casing::Lower },
    Config { file: "valve.ini", key: "Language", section: "Settings", casing: Casing::Lower },
    // The FreeTP launcher's own file. Its picker writes GameLanguage here, under
    // a [Settings] section it creates, spelled the way [Languages] lists it.
    Config { file: "Origins.ini", key: "GameLanguage", section: "Settings", casing: Casing::Title },
    Config { file: "steam_settings/force_language.txt", key: "", section: "", casing: Casing::Lower },
];

/// `russian` becomes `Russian`.
fn title_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The game's own name for each language's text folder.
///
/// A total conversion replaces text by dropping a `.msgbnd.dcx` into one of
/// these, so knowing the folder is the only way to tell whether the mod has
/// been translated into the language the game is set to.
pub fn locale_folder(language: &str) -> Option<&'static str> {
    Some(match language {
        "english" => "engus",
        "russian" => "rusru",
        "japanese" => "jpnjp",
        "koreana" => "korkr",
        "schinese" => "zhocn",
        "tchinese" => "zhotw",
        "french" => "frafr",
        "italian" => "itait",
        "german" => "deude",
        "spanish" => "spaes",
        "latam" => "spaar",
        "polish" => "polpl",
        "brazilian" => "porbr",
        "thai" => "thath",
        _ => return None,
    })
}

/// Locales in unrelated scripts, used as the yardstick.
///
/// A real translation cannot be byte-identical to Japanese and Korean at once.
const YARDSTICKS: &[&str] = &["jpnjp", "korkr", "zhocn", "thath"];

/// A translation carried inside Roundtable, so installing it is one button and
/// no download.
struct Bundled {
    edition: &'static str,
    language: &'static str,
    version: &'static str,
    /// Credited because the licence asks for it, and because somebody did the work.
    author: &'static str,
    source: &'static str,
    bytes: &'static [u8],
}

const BUNDLED: &[Bundled] = &[Bundled {
    edition: "convergence",
    language: "russian",
    version: "3.0.1",
    author: "S1RBI",
    source: "https://www.nexusmods.com/eldenring/mods/4697",
    bytes: include_bytes!("../assets/translations/convergence-russian-3.0.1.zip"),
}];

fn bundled(edition: &str, language: &str) -> Option<&'static Bundled> {
    BUNDLED
        .iter()
        .find(|b| b.edition == edition && b.language == language)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundledText {
    pub version: String,
    pub author: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionText {
    pub edition: String,
    /// The folder the game will read, e.g. `rusru`.
    pub locale: String,
    /// False when the mod ships one English archive copied into every locale.
    pub translated: bool,
    /// Where a translation has to be unpacked.
    pub folder: PathBuf,
    /// A translation Roundtable already carries, when there is one.
    pub bundled: Option<BundledText>,
    /// True when the mod's own text has been kept aside, so this is revertible.
    pub revertible: bool,
}

/// Whether a total conversion's own text exists in the language the game runs in.
///
/// The Convergence ships a single English archive copied into all fourteen
/// locale folders, so its items and menus read English on a Russian game. The
/// copies are byte-identical, which is what separates them from a translation:
/// no real Russian file is also the Japanese one.
pub fn edition_text(
    edition: &str,
    mod_dir: &Path,
    language: &str,
) -> Option<EditionText> {
    let locale = locale_folder(language)?;
    let msg = mod_dir.join("msg");
    let folder = msg.join(locale);
    if !folder.is_dir() {
        return None;
    }

    let offer = bundled(edition, language).map(|b| BundledText {
        version: b.version.to_string(),
        author: b.author.to_string(),
        source: b.source.to_string(),
    });
    let revertible = folder.join("_original").is_dir();

    // English is what these mods are written in, so it is never the problem.
    if locale == "engus" {
        return Some(EditionText {
            edition: edition.to_string(),
            locale: locale.to_string(),
            translated: true,
            folder,
            bundled: offer,
            revertible,
        });
    }

    let archives: Vec<PathBuf> = std::fs::read_dir(&folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(".msgbnd.dcx")
        })
        .collect();

    if archives.is_empty() {
        return None;
    }

    let shared = archives.iter().any(|archive| {
        let name = archive.file_name();
        YARDSTICKS
            .iter()
            .filter(|other| **other != locale)
            .filter_map(|other| name.map(|name| msg.join(other).join(name)))
            .any(|twin| same_bytes(archive, &twin))
    });

    Some(EditionText {
        edition: edition.to_string(),
        locale: locale.to_string(),
        translated: !shared,
        folder,
        bundled: offer,
        revertible,
    })
}

/// What the game calls a text archive.
const TEXT_SUFFIX: &str = ".msgbnd.dcx";

/// Whether an archive is a text translation and nothing else.
///
/// A translation is a handful of `.msgbnd.dcx` files. Anything carrying scripts,
/// executables or regulation data is a different kind of mod and must not be
/// dropped into a locale folder.
pub fn archive_holds_text(archive: &Path) -> bool {
    let Ok(file) = std::fs::File::open(archive) else {
        return false;
    };
    holds_text(file)
}

fn holds_text<R: std::io::Read + std::io::Seek>(source: R) -> bool {
    let Ok(mut zip) = zip::ZipArchive::new(source) else {
        return false;
    };

    let mut text = 0usize;
    for index in 0..zip.len() {
        let Ok(entry) = zip.by_index_raw(index) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_ascii_lowercase();
        if name.ends_with(TEXT_SUFFIX) {
            text += 1;
        } else {
            // One stray readme is normal; a payload is not.
            let readme = name.ends_with(".txt") || name.ends_with(".md");
            if !readme {
                return false;
            }
        }
    }
    text > 0
}

/// Unpacks a text translation into an edition's locale folder.
///
/// The mod's own copies are kept in `_original` so the English text is one
/// folder away if the translation turns out to be worse than none.
pub fn install_edition_text(
    mod_dir: &Path,
    language: &str,
    archive: &Path,
) -> Result<Vec<String>> {
    let file = std::fs::File::open(archive).at(archive)?;
    install_text_from(mod_dir, language, file)
}

/// The install itself, over anything that reads like a zip.
///
/// Taking a reader rather than a path is what lets the bundled translation go
/// straight from the binary into the game folder. A scratch file on the way
/// through would be one shared path that two installs could race over.
fn install_text_from<R: std::io::Read + std::io::Seek>(
    mod_dir: &Path,
    language: &str,
    source: R,
) -> Result<Vec<String>> {
    let locale = locale_folder(language).ok_or_else(|| {
        crate::error::Error::msg(format!("{language} has no text folder in this game"))
    })?;

    let folder = mod_dir.join("msg").join(locale);
    if !folder.is_dir() {
        return Err(crate::error::Error::msg(format!(
            "this conversion has no {locale} folder to translate"
        )));
    }

    let mut zip = zip::ZipArchive::new(source)
        .map_err(|e| crate::error::Error::Archive(e.to_string()))?;

    // Every name is read before a single byte is written, so an archive holding
    // anything other than text is rejected with the folder still untouched. The
    // name alone is kept: a nested folder in the archive must not escape, and
    // must not bury the files somewhere the game will not look.
    let mut wanted: Vec<(usize, String)> = Vec::new();
    for index in 0..zip.len() {
        let entry = zip
            .by_index_raw(index)
            .map_err(|e| crate::error::Error::Archive(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let lower = entry.name().to_ascii_lowercase();
        if lower.ends_with(TEXT_SUFFIX) {
            let Some(name) = entry
                .enclosed_name()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            else {
                continue;
            };
            wanted.push((index, name));
        } else if !(lower.ends_with(".txt") || lower.ends_with(".md")) {
            // One stray readme is normal; a payload is not.
            return Err(crate::error::Error::msg(
                "that archive is not a text translation".to_string(),
            ));
        }
    }

    if wanted.is_empty() {
        return Err(crate::error::Error::msg(
            "the archive held no text files".to_string(),
        ));
    }

    let backup = folder.join("_original");
    let mut written = Vec::new();

    for (index, name) in wanted {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| crate::error::Error::Archive(e.to_string()))?;

        let target = folder.join(&name);
        if target.is_file() {
            std::fs::create_dir_all(&backup).at(&backup)?;
            let kept = backup.join(&name);
            if !kept.exists() {
                std::fs::copy(&target, &kept).at(&kept)?;
            }
        }

        let mut out = std::io::BufWriter::new(std::fs::File::create(&target).at(&target)?);
        std::io::copy(&mut entry, &mut out).at(&target)?;
        written.push(name);
    }

    Ok(written)
}

/// Installs the translation Roundtable carries for this edition.
///
/// One button, no download, no account: the bytes are already in the binary and
/// go straight from there into the game folder.
pub fn install_bundled_text(
    edition: &str,
    mod_dir: &Path,
    language: &str,
) -> Result<Vec<String>> {
    let offer = bundled(edition, language).ok_or_else(|| {
        crate::error::Error::msg(format!("no {language} translation ships with Roundtable"))
    })?;

    install_text_from(mod_dir, language, std::io::Cursor::new(offer.bytes))
}

/// Puts the conversion's own text back.
pub fn revert_edition_text(mod_dir: &Path, language: &str) -> Result<Vec<String>> {
    let locale = locale_folder(language).ok_or_else(|| {
        crate::error::Error::msg(format!("{language} has no text folder in this game"))
    })?;

    let folder = mod_dir.join("msg").join(locale);
    let backup = folder.join("_original");
    if !backup.is_dir() {
        return Err(crate::error::Error::msg(
            "there is nothing to go back to".to_string(),
        ));
    }

    let mut restored = Vec::new();
    for entry in std::fs::read_dir(&backup).at(&backup)?.flatten() {
        let from = entry.path();
        if !from.is_file() {
            continue;
        }
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = folder.join(name);
        std::fs::copy(&from, &to).at(&to)?;
        restored.push(name.to_string_lossy().to_string());
    }

    // Gone, so the interface stops offering a revert that would do nothing.
    std::fs::remove_dir_all(&backup).at(&backup)?;
    Ok(restored)
}

/// Looks where a browser leaves downloads for a text translation.
///
/// Saves the user finding the file they just downloaded. Newest first, because
/// the one they want is the one they just fetched.
pub fn find_text_archive() -> Option<PathBuf> {
    let mut folders: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        folders.push(home.join("Downloads"));
        folders.push(home.join("Desktop"));
    }
    if let Some(downloads) = dirs::download_dir() {
        folders.push(downloads);
    }

    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("zip")) {
                continue;
            }
            let Ok(when) = entry.metadata().and_then(|m| m.modified()) else {
                continue;
            };
            found.push((when, path));
        }
    }

    found.sort_by(|a, b| b.0.cmp(&a.0));
    // Opening each archive is the only honest test, so stop at the first hit
    // rather than reading a folder full of unrelated downloads.
    found
        .into_iter()
        .map(|(_, path)| path)
        .find(|path| archive_holds_text(path))
}

/// Compares two files without holding both in memory twice over.
fn same_bytes(left: &Path, right: &Path) -> bool {
    let (Ok(a), Ok(b)) = (left.metadata(), right.metadata()) else {
        return false;
    };
    if a.len() != b.len() || a.len() == 0 {
        return false;
    }
    match (std::fs::read(left), std::fs::read(right)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

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
///
/// When the key is absent it goes under `section`, which is created if it is not
/// there. A key written into the wrong section is read by nobody.
fn write_ini(text: &str, key: &str, value: &str, section: &str) -> String {
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
        let header = format!("[{section}]");
        let wanted = out
            .iter()
            .position(|line| line.trim().eq_ignore_ascii_case(&header));

        match wanted {
            Some(at) => out.insert(at + 1, format!("{key}={value}")),
            None if section.is_empty() => out.push(format!("{key}={value}")),
            None => {
                out.push(String::new());
                out.push(header);
                out.push(format!("{key}={value}"));
            }
        }
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

    for config in CONFIGS {
        let path = game_dir.join(config.file);
        if !path.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };

        let (value, disabled) = if config.key.is_empty() {
            // A bare text file holding nothing but the language.
            let value = text.trim().to_ascii_lowercase();
            ((!value.is_empty()).then_some(value), false)
        } else {
            read_ini(&text, config.key)
        };

        files.push(LanguageFile {
            file: config.file.to_string(),
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

    for config in CONFIGS {
        let path = game_dir.join(config.file);
        if !path.is_file() {
            continue;
        }

        if config.key.is_empty() {
            std::fs::write(&path, &language).at(&path)?;
            written.push(path);
            continue;
        }

        let value = match config.casing {
            Casing::Lower => language.clone(),
            Casing::Title => title_case(&language),
        };

        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let updated = write_ini(&text, config.key, &value, config.section);
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
        let out = write_ini(STEAMFIX, "Language", "russian", "Main");
        assert!(out.contains("Language=russian"));
        assert!(!out.contains("#Language"), "got {out}");
        // The rest of the file survives.
        assert!(out.contains("RealAppId=1245620"));
        assert!(out.contains("[Misc]"));
        assert!(out.contains("Overlay=true"));
    }

    #[test]
    fn an_existing_value_is_replaced_rather_than_duplicated() {
        let out = write_ini("[Main]\r\nLanguage=english\r\n", "Language", "russian", "Main");
        assert_eq!(out.matches("Language=").count(), 1, "got {out}");
        assert!(out.contains("Language=russian"));
    }

    #[test]
    fn a_missing_line_is_added_under_the_section() {
        let out = write_ini("[Main]\r\nRealAppId=1\r\n", "Language", "russian", "Main");
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
            "Main",
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
    fn a_missing_section_is_created_rather_than_writing_into_the_wrong_one() {
        // Origins.ini ships with no [Settings]; the repack's own picker adds it.
        // A GameLanguage under [Target] would be read by nobody.
        let out = write_ini(
            "[Target]\r\nExe=start_protected_game.exe\r\n\r\n[Languages]\r\n1=English\r\n",
            "GameLanguage",
            "Russian",
            "Settings",
        );
        let lines: Vec<&str> = out.lines().map(str::trim).collect();
        let header = lines.iter().position(|line| *line == "[Settings]").expect("got {out}");
        assert_eq!(lines[header + 1], "GameLanguage=Russian");
        assert!(out.contains("1=English"), "the language table survives: {out}");
    }

    #[test]
    fn the_repack_launcher_gets_its_own_spelling() {
        let dir = temp("casing");
        std::fs::write(dir.join("SteamFix.ini"), STEAMFIX).unwrap();
        std::fs::write(dir.join("Origins.ini"), "[Target]\r\nExe=x\r\n").unwrap();

        set(&dir, "russian").unwrap();

        // The emulator wants the Steam API code, the launcher wants its own label.
        let emu = std::fs::read_to_string(dir.join("SteamFix.ini")).unwrap();
        let launcher = std::fs::read_to_string(dir.join("Origins.ini")).unwrap();
        assert!(emu.contains("Language=russian"), "got {emu}");
        assert!(launcher.contains("GameLanguage=Russian"), "got {launcher}");

        // And they still read as one language rather than a conflict.
        let after = status(&dir);
        assert_eq!(after.current.as_deref(), Some("russian"));
        assert!(!after.conflict, "same language, different spelling");

        std::fs::remove_dir_all(&dir).ok();
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

    /// Lays out a mod's msg folder the way a total conversion ships one.
    fn msg_folder(dir: &Path, translated: &[&str]) {
        let msg = dir.join("msg");
        // The one English archive, copied into every locale that has no
        // translation of its own. This is what The Convergence does.
        let shared = b"THE ENGLISH TEXT ARCHIVE".as_slice();
        for locale in [
            "engus", "rusru", "jpnjp", "korkr", "zhocn", "thath", "deude", "frafr",
        ] {
            let at = msg.join(locale);
            std::fs::create_dir_all(&at).unwrap();
            let body: &[u8] = if translated.contains(&locale) {
                b"TRANSLATED TEXT FOR THIS LOCALE"
            } else {
                shared
            };
            std::fs::write(at.join("item_dlc02.msgbnd.dcx"), body).unwrap();
        }
    }

    #[test]
    fn an_untranslated_conversion_is_caught_by_its_identical_copies() {
        let dir = temp("edition-plain");
        msg_folder(&dir, &[]);

        let found = edition_text("convergence", &dir, "russian").unwrap();
        assert_eq!(found.locale, "rusru");
        assert!(
            !found.translated,
            "rusru cannot be a translation and the Japanese file at the same time"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_real_translation_is_left_alone() {
        let dir = temp("edition-translated");
        msg_folder(&dir, &["rusru"]);

        let found = edition_text("convergence", &dir, "russian").unwrap();
        assert!(found.translated, "this one differs from every other locale");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn english_is_never_reported_as_missing() {
        // These mods are written in English, so engus is the original.
        let dir = temp("edition-english");
        msg_folder(&dir, &[]);
        assert!(edition_text("convergence", &dir, "english").unwrap().translated);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_conversion_that_ships_no_text_says_nothing() {
        let dir = temp("edition-notext");
        std::fs::create_dir_all(dir.join("mod")).unwrap();
        assert!(edition_text("convergence", &dir, "russian").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Builds a zip in memory the way a translation is published.
    fn zip_of(at: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(at).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for (name, body) in entries {
            zip.start_file(*name, options).unwrap();
            std::io::Write::write_all(&mut zip, body).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn a_translation_replaces_the_text_and_keeps_the_original() {
        let dir = temp("install-text");
        msg_folder(&dir, &[]);
        let archive = dir.join("ru.zip");
        zip_of(
            &archive,
            &[("item_dlc02.msgbnd.dcx", b"RUSSIAN TEXT".as_slice())],
        );

        let written = install_edition_text(&dir, "russian", &archive).unwrap();
        assert_eq!(written, vec!["item_dlc02.msgbnd.dcx".to_string()]);

        let installed = dir.join("msg").join("rusru").join("item_dlc02.msgbnd.dcx");
        assert_eq!(std::fs::read(&installed).unwrap(), b"RUSSIAN TEXT");

        let kept = dir
            .join("msg")
            .join("rusru")
            .join("_original")
            .join("item_dlc02.msgbnd.dcx");
        assert!(kept.is_file(), "the mod's own text must stay recoverable");

        // And the check now agrees the locale is translated.
        assert!(edition_text("convergence", &dir, "russian").unwrap().translated);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_nested_archive_still_lands_in_the_locale_folder() {
        // Some uploads wrap the files in a folder named after the mod.
        let dir = temp("install-nested");
        msg_folder(&dir, &[]);
        let archive = dir.join("ru.zip");
        zip_of(
            &archive,
            &[("RU Translation/rusru/item_dlc02.msgbnd.dcx", b"RU".as_slice())],
        );

        install_edition_text(&dir, "russian", &archive).unwrap();
        let installed = dir.join("msg").join("rusru").join("item_dlc02.msgbnd.dcx");
        assert_eq!(std::fs::read(&installed).unwrap(), b"RU");
        assert!(
            !dir.join("msg").join("rusru").join("RU Translation").exists(),
            "the wrapper folder is not copied along"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_archive_that_is_not_a_translation_is_refused() {
        // A whole mod dropped into a locale folder would do real damage.
        let dir = temp("install-wrong");
        msg_folder(&dir, &[]);
        let archive = dir.join("mod.zip");
        zip_of(
            &archive,
            &[
                ("item_dlc02.msgbnd.dcx", b"text".as_slice()),
                ("regulation.bin", b"not text".as_slice()),
            ],
        );

        assert!(!archive_holds_text(&archive));
        assert!(install_edition_text(&dir, "russian", &archive).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_translation_roundtable_carries_installs_on_its_own() {
        // No download, no account: the bytes are in the binary.
        let dir = temp("install-bundled");
        msg_folder(&dir, &[]);

        let before = edition_text("convergence", &dir, "russian").unwrap();
        assert!(!before.translated);
        assert!(before.bundled.is_some(), "russian ships with Roundtable");
        assert!(!before.revertible);

        let written = install_bundled_text("convergence", &dir, "russian").unwrap();
        assert!(
            written.iter().any(|n| n == "item_dlc02.msgbnd.dcx"),
            "got {written:?}"
        );

        let after = edition_text("convergence", &dir, "russian").unwrap();
        assert!(after.translated);
        assert!(after.revertible, "the mod's own text is one button away");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reverting_puts_the_conversions_own_text_back() {
        let dir = temp("revert-text");
        msg_folder(&dir, &[]);
        let file = dir.join("msg").join("rusru").join("item_dlc02.msgbnd.dcx");
        let original = std::fs::read(&file).unwrap();

        install_bundled_text("convergence", &dir, "russian").unwrap();
        assert_ne!(std::fs::read(&file).unwrap(), original);

        revert_edition_text(&dir, "russian").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert!(
            !edition_text("convergence", &dir, "russian").unwrap().revertible,
            "nothing left to revert"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_language_with_no_bundled_translation_says_so() {
        let dir = temp("install-none");
        msg_folder(&dir, &[]);
        assert!(install_bundled_text("convergence", &dir, "thai").is_err());
        assert!(edition_text("convergence", &dir, "thai").unwrap().bundled.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_readme_beside_the_text_is_tolerated() {
        let dir = temp("install-readme");
        let archive = dir.join("ru.zip");
        std::fs::create_dir_all(&dir).unwrap();
        zip_of(
            &archive,
            &[
                ("item_dlc02.msgbnd.dcx", b"text".as_slice()),
                ("readme.txt", b"put these in rusru".as_slice()),
            ],
        );
        assert!(archive_holds_text(&archive));
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
