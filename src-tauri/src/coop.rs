//! Seamless Co-op configuration.
//!
//! Field names, defaults and value ranges come from the `ersc_settings.ini` shipped
//! in the Seamless Co-op release archive, so the editor matches the mod exactly
//! rather than guessing.
//!
//! The mod's layout inside the game folder:
//!
//! ```text
//! ersc_launcher.exe
//! SeamlessCoop/ersc.dll
//! SeamlessCoop/ersc_settings.ini
//! SeamlessCoop/crashpad/crashpad_handler.exe
//! SeamlessCoop/locale/english.json
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::Rng;
use serde::Serialize;

use crate::error::{Error, IoContext, Result};

pub const COOP_DIR: &str = "SeamlessCoop";
pub const SETTINGS_FILE: &str = "ersc_settings.ini";
pub const DLL_FILE: &str = "ersc.dll";
pub const LAUNCHER_FILE: &str = "ersc_launcher.exe";

/// How a setting should be presented in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldKind {
    Toggle,
    /// A bounded integer, rendered as a slider.
    Range,
    /// A fixed set of numeric options, rendered as a select.
    Choice,
    Text,
}

/// Static description of one INI key, used to build the co-op page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSpec {
    pub section: &'static str,
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: FieldKind,
    pub default: &'static str,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub options: &'static [(i64, &'static str)],
}

/// Every key Seamless Co-op understands, in the order the stock file lists them.
pub const FIELDS: &[FieldSpec] = &[
    FieldSpec {
        section: "GAMEPLAY",
        key: "allow_invaders",
        label: "Allow invaders",
        help: "Other players may join your world uninvited and try to kill your party.",
        kind: FieldKind::Toggle,
        default: "1",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "GAMEPLAY",
        key: "death_debuffs",
        label: "Death debuffs",
        help: "Dying applies Rot Essence, cured only by resting at a site of grace.",
        kind: FieldKind::Toggle,
        default: "1",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "GAMEPLAY",
        key: "allow_summons",
        label: "Allow spirit summons",
        help: "Spirit ashes can be summoned during multiplayer.",
        kind: FieldKind::Toggle,
        default: "1",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "GAMEPLAY",
        key: "overhead_player_display",
        label: "Overhead display",
        help: "What is shown above other players' heads.",
        kind: FieldKind::Choice,
        default: "0",
        min: None,
        max: None,
        options: &[
            (0, "Normal"),
            (1, "None"),
            (2, "Ping"),
            (3, "Soul level"),
            (4, "Death count"),
            (5, "Soul level and ping"),
        ],
    },
    FieldSpec {
        section: "GAMEPLAY",
        key: "skip_splash_screens",
        label: "Skip intro logos",
        help: "Boots straight past the publisher logos.",
        kind: FieldKind::Toggle,
        default: "0",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "GAMEPLAY",
        key: "append_steam_id_to_players",
        label: "Show Steam IDs",
        help: "Appends each player's Steam ID to their overhead display, which helps identify and block cheaters.",
        kind: FieldKind::Toggle,
        default: "0",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "GAMEPLAY",
        key: "always_spectate_on_death",
        label: "Always spectate on death",
        help: "Off spectates only during bosses and invasions; on spectates until a party wipe or a rest.",
        kind: FieldKind::Toggle,
        default: "0",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "GAMEPLAY",
        key: "default_boot_master_volume",
        label: "Boot volume",
        help: "Master volume before the first save loads. 0 mutes, 10 is maximum.",
        kind: FieldKind::Range,
        default: "5",
        min: Some(0),
        max: Some(10),
        options: &[],
    },
    FieldSpec {
        section: "SCALING",
        key: "enemy_health_scaling",
        label: "Enemy health",
        help: "Extra enemy health percentage per additional player.",
        kind: FieldKind::Range,
        default: "35",
        min: Some(0),
        max: Some(400),
        options: &[],
    },
    FieldSpec {
        section: "SCALING",
        key: "enemy_damage_scaling",
        label: "Enemy damage",
        help: "Extra enemy damage percentage per additional player.",
        kind: FieldKind::Range,
        default: "0",
        min: Some(0),
        max: Some(400),
        options: &[],
    },
    FieldSpec {
        section: "SCALING",
        key: "enemy_posture_scaling",
        label: "Enemy posture",
        help: "Extra enemy posture absorption per additional player.",
        kind: FieldKind::Range,
        default: "15",
        min: Some(0),
        max: Some(400),
        options: &[],
    },
    FieldSpec {
        section: "SCALING",
        key: "boss_health_scaling",
        label: "Boss health",
        help: "Extra boss health percentage per additional player.",
        kind: FieldKind::Range,
        default: "100",
        min: Some(0),
        max: Some(400),
        options: &[],
    },
    FieldSpec {
        section: "SCALING",
        key: "boss_damage_scaling",
        label: "Boss damage",
        help: "Extra boss damage percentage per additional player.",
        kind: FieldKind::Range,
        default: "0",
        min: Some(0),
        max: Some(400),
        options: &[],
    },
    FieldSpec {
        section: "SCALING",
        key: "boss_posture_scaling",
        label: "Boss posture",
        help: "Extra boss posture absorption per additional player.",
        kind: FieldKind::Range,
        default: "20",
        min: Some(0),
        max: Some(400),
        options: &[],
    },
    FieldSpec {
        section: "PASSWORD",
        key: "cooppassword",
        label: "Session password",
        help: "Everyone in the session must use the same password. Leave blank to play alone.",
        kind: FieldKind::Text,
        default: "",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "SAVE",
        key: "save_file_extension",
        label: "Save file extension",
        help: "Co-op saves use this extension instead of sl2 so the vanilla game never touches them. Alphanumeric, up to 120 characters.",
        kind: FieldKind::Text,
        default: "co2",
        min: None,
        max: None,
        options: &[],
    },
    FieldSpec {
        section: "LANGUAGE",
        key: "mod_language_override",
        label: "Language override",
        help: "Leave blank to follow the game's language.",
        kind: FieldKind::Text,
        default: "",
        min: None,
        max: None,
        options: &[],
    },
];

/// A comment-preserving INI document.
///
/// Seamless Co-op's file documents every key inline; rewriting it with a naive
/// serialiser would throw that away and leave users staring at bare numbers.
#[derive(Debug, Clone, Default)]
pub struct IniDocument {
    lines: Vec<IniLine>,
}

#[derive(Debug, Clone)]
enum IniLine {
    Raw(String),
    Section(String),
    Pair {
        section: String,
        key: String,
        /// Everything before the key on the line, preserved verbatim.
        indent: String,
        /// Text between the key and `=`, plus between `=` and the value.
        spacing: (String, String),
        value: String,
        /// Trailing comment, including its leading whitespace and `;`.
        trailer: String,
    },
}

impl IniDocument {
    pub fn parse(text: &str) -> IniDocument {
        let mut lines = Vec::new();
        let mut section = String::new();

        for raw in text.lines() {
            let trimmed = raw.trim();

            if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                lines.push(IniLine::Raw(raw.to_string()));
                continue;
            }

            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                section = trimmed[1..trimmed.len() - 1].trim().to_string();
                lines.push(IniLine::Section(raw.to_string()));
                continue;
            }

            let Some(eq) = raw.find('=') else {
                lines.push(IniLine::Raw(raw.to_string()));
                continue;
            };

            let (left, right) = raw.split_at(eq);
            let right = &right[1..];

            let indent: String = left.chars().take_while(|c| c.is_whitespace()).collect();
            let key_and_space = &left[indent.len()..];
            let key = key_and_space.trim_end().to_string();
            let space_before_eq = key_and_space[key.len()..].to_string();

            // A value may carry a trailing `;` comment.
            let (value_part, trailer) = match right.find(';') {
                Some(at) => (&right[..at], right[at..].to_string()),
                None => (right, String::new()),
            };
            let space_after_eq: String =
                value_part.chars().take_while(|c| c.is_whitespace()).collect();
            let value = value_part.trim().to_string();
            // Whitespace between the value and the comment belongs to the trailer.
            let gap = &value_part[space_after_eq.len() + value.len()..];

            lines.push(IniLine::Pair {
                section: section.clone(),
                key,
                indent,
                spacing: (space_before_eq, space_after_eq),
                value,
                trailer: format!("{gap}{trailer}"),
            });
        }

        IniDocument { lines }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.lines.iter().find_map(|line| match line {
            IniLine::Pair {
                section: s,
                key: k,
                value,
                ..
            } if s.eq_ignore_ascii_case(section) && k.eq_ignore_ascii_case(key) => {
                Some(value.as_str())
            }
            _ => None,
        })
    }

    /// Updates a key in place. Returns false when the key is not in the file.
    pub fn set(&mut self, section: &str, key: &str, new_value: &str) -> bool {
        for line in &mut self.lines {
            if let IniLine::Pair {
                section: s,
                key: k,
                value,
                ..
            } = line
            {
                if s.eq_ignore_ascii_case(section) && k.eq_ignore_ascii_case(key) {
                    *value = new_value.to_string();
                    return true;
                }
            }
        }
        false
    }

    /// Updates a key, appending it under its section when it is missing so that
    /// older config files gain keys added by newer mod versions.
    pub fn upsert(&mut self, section: &str, key: &str, new_value: &str) {
        if self.set(section, key, new_value) {
            return;
        }

        let insert_at = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| match line {
                IniLine::Pair { section: s, .. } => s.eq_ignore_ascii_case(section),
                _ => false,
            })
            .map(|(i, _)| i + 1)
            .next_back();

        let entry = IniLine::Pair {
            section: section.to_string(),
            key: key.to_string(),
            indent: String::new(),
            spacing: (" ".into(), " ".into()),
            value: new_value.to_string(),
            trailer: String::new(),
        };

        match insert_at {
            Some(index) => self.lines.insert(index, entry),
            None => {
                self.lines.push(IniLine::Raw(String::new()));
                self.lines.push(IniLine::Section(format!("[{section}]")));
                self.lines.push(entry);
            }
        }
    }

    pub fn to_map(&self) -> BTreeMap<String, String> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                IniLine::Pair {
                    section, key, value, ..
                } => Some((format!("{section}.{key}"), value.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                IniLine::Raw(raw) | IniLine::Section(raw) => out.push_str(raw),
                IniLine::Pair {
                    indent,
                    key,
                    spacing,
                    value,
                    trailer,
                    ..
                } => {
                    out.push_str(indent);
                    out.push_str(key);
                    out.push_str(&spacing.0);
                    out.push('=');
                    out.push_str(&spacing.1);
                    out.push_str(value);
                    out.push_str(trailer);
                }
            }
            out.push_str("\r\n");
        }
        out
    }
}

/// Current co-op configuration plus where it came from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoopSettings {
    pub path: PathBuf,
    pub values: BTreeMap<String, String>,
    pub installed: bool,
    pub dll_version: Option<String>,
}

pub fn settings_path(game_dir: &Path) -> PathBuf {
    game_dir.join(COOP_DIR).join(SETTINGS_FILE)
}

pub fn dll_path(game_dir: &Path) -> PathBuf {
    game_dir.join(COOP_DIR).join(DLL_FILE)
}

pub fn read(game_dir: &Path) -> Result<CoopSettings> {
    let path = settings_path(game_dir);
    let dll = dll_path(game_dir);

    let values = if path.is_file() {
        IniDocument::parse(&std::fs::read_to_string(&path).at(&path)?).to_map()
    } else {
        FIELDS
            .iter()
            .map(|f| (format!("{}.{}", f.section, f.key), f.default.to_string()))
            .collect()
    };

    Ok(CoopSettings {
        installed: dll.is_file(),
        dll_version: crate::game::file_version(&dll),
        path,
        values,
    })
}

/// Applies a batch of `SECTION.key -> value` changes, keeping comments intact.
pub fn write(game_dir: &Path, changes: &BTreeMap<String, String>) -> Result<CoopSettings> {
    let path = settings_path(game_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| stock_settings());
    let mut doc = IniDocument::parse(&existing);

    for (qualified, raw_value) in changes {
        let Some((section, key)) = qualified.split_once('.') else {
            return Err(Error::msg(format!(
                "setting '{qualified}' is not in SECTION.key form"
            )));
        };
        let value = validate(section, key, raw_value)?;
        doc.upsert(section, key, &value);
    }

    std::fs::write(&path, doc.render()).at(&path)?;
    read(game_dir)
}

/// Rejects values the mod would silently misread.
fn validate(section: &str, key: &str, value: &str) -> Result<String> {
    let Some(spec) = FIELDS
        .iter()
        .find(|f| f.section.eq_ignore_ascii_case(section) && f.key.eq_ignore_ascii_case(key))
    else {
        // Unknown keys are passed through so a newer mod version still works.
        return Ok(value.to_string());
    };

    match spec.kind {
        FieldKind::Toggle => match value.trim() {
            "0" | "1" => Ok(value.trim().to_string()),
            other => Err(Error::msg(format!(
                "{} must be 0 or 1, got '{other}'",
                spec.label
            ))),
        },
        FieldKind::Range => {
            let parsed: i64 = value.trim().parse().map_err(|_| {
                Error::msg(format!("{} must be a whole number, got '{value}'", spec.label))
            })?;
            let min = spec.min.unwrap_or(i64::MIN);
            let max = spec.max.unwrap_or(i64::MAX);
            if parsed < min || parsed > max {
                return Err(Error::msg(format!(
                    "{} must be between {min} and {max}, got {parsed}",
                    spec.label
                )));
            }
            Ok(parsed.to_string())
        }
        FieldKind::Choice => {
            let parsed: i64 = value.trim().parse().map_err(|_| {
                Error::msg(format!("{} must be a whole number, got '{value}'", spec.label))
            })?;
            if !spec.options.iter().any(|(v, _)| *v == parsed) {
                return Err(Error::msg(format!(
                    "{} does not accept the value {parsed}",
                    spec.label
                )));
            }
            Ok(parsed.to_string())
        }
        FieldKind::Text => {
            if spec.key == "save_file_extension" {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(Error::msg("The save file extension cannot be empty."));
                }
                if trimmed.len() > 120 {
                    return Err(Error::msg(
                        "The save file extension is limited to 120 characters.",
                    ));
                }
                if !trimmed.chars().all(|c| c.is_ascii_alphanumeric()) {
                    return Err(Error::msg(
                        "The save file extension must be letters and digits only.",
                    ));
                }
                if trimmed.eq_ignore_ascii_case("sl2") {
                    return Err(Error::msg(
                        "Using sl2 makes co-op share the vanilla save. Pick a different extension, such as co2.",
                    ));
                }
                return Ok(trimmed.to_string());
            }
            if spec.key == "cooppassword" {
                let trimmed = value.trim();
                if trimmed.contains(['\r', '\n', ';']) {
                    return Err(Error::msg(
                        "A password cannot contain line breaks or semicolons.",
                    ));
                }
                return Ok(trimmed.to_string());
            }
            Ok(value.trim().to_string())
        }
    }
}

/// Generates a readable session password: two words plus digits, easy to say aloud
/// over voice chat and still hard to guess.
pub fn generate_password() -> String {
    const WORDS: &[&str] = &[
        "erdtree", "grace", "lantern", "tarnished", "rune", "godrick", "sellia", "limgrave",
        "caelid", "ranni", "maiden", "golden", "ashen", "crimson", "nomad", "roundtable",
        "altus", "raya", "mohg", "malenia", "torrent", "spirit", "somber", "ember",
    ];
    let mut rng = rand::rng();
    let first = WORDS[rng.random_range(0..WORDS.len())];
    let second = WORDS[rng.random_range(0..WORDS.len())];
    let number: u16 = rng.random_range(10..1000);
    format!("{first}-{second}-{number}")
}

/// The stock file, used when a config has to be created from nothing.
fn stock_settings() -> String {
    let mut out = String::new();
    let mut current = "";
    for field in FIELDS {
        if field.section != current {
            if !out.is_empty() {
                out.push_str("\r\n");
            }
            out.push_str(&format!("[{}]\r\n\r\n", field.section));
            current = field.section;
        }
        out.push_str(&format!("; {}\r\n", field.help));
        out.push_str(&format!("{} = {}\r\n\r\n", field.key, field.default));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A verbatim excerpt of the shipped file, comments and all.
    const REAL_INI: &str = "[GAMEPLAY]\r\n\r\n; Invaders are other players.  0=FALSE  1=TRUE\r\nallow_invaders = 1\r\n\r\n[PASSWORD]\r\n\r\n; Session password\r\ncooppassword = \r\n\r\n[SAVE]\r\n\r\n;Your save file extension (in the vanilla game this is .sl2).\r\nsave_file_extension = co2\r\n";

    #[test]
    fn parsing_then_rendering_preserves_comments() {
        let doc = IniDocument::parse(REAL_INI);
        let rendered = doc.render();
        assert!(rendered.contains("; Invaders are other players."));
        assert!(rendered.contains(";Your save file extension"));
        assert!(rendered.contains("allow_invaders = 1"));
    }

    #[test]
    fn reads_values_by_section_and_key() {
        let doc = IniDocument::parse(REAL_INI);
        assert_eq!(doc.get("GAMEPLAY", "allow_invaders"), Some("1"));
        assert_eq!(doc.get("SAVE", "save_file_extension"), Some("co2"));
        // An empty value is a value, not a missing key.
        assert_eq!(doc.get("PASSWORD", "cooppassword"), Some(""));
        assert_eq!(doc.get("SAVE", "nope"), None);
    }

    #[test]
    fn setting_a_value_keeps_the_surrounding_comment() {
        let mut doc = IniDocument::parse(REAL_INI);
        assert!(doc.set("GAMEPLAY", "allow_invaders", "0"));
        let rendered = doc.render();
        assert!(rendered.contains("allow_invaders = 0"));
        assert!(rendered.contains("; Invaders are other players."));
    }

    #[test]
    fn upsert_adds_keys_missing_from_older_files() {
        let mut doc = IniDocument::parse(REAL_INI);
        doc.upsert("GAMEPLAY", "always_spectate_on_death", "1");
        let rendered = doc.render();
        assert!(rendered.contains("always_spectate_on_death = 1"));
        // It must land inside GAMEPLAY, before the PASSWORD header.
        let added = rendered.find("always_spectate_on_death").unwrap();
        let password = rendered.find("[PASSWORD]").unwrap();
        assert!(added < password);
    }

    #[test]
    fn upsert_creates_a_missing_section() {
        let mut doc = IniDocument::parse("[GAMEPLAY]\r\nallow_invaders = 1\r\n");
        doc.upsert("LANGUAGE", "mod_language_override", "russian");
        let rendered = doc.render();
        assert!(rendered.contains("[LANGUAGE]"));
        assert!(rendered.contains("mod_language_override = russian"));
        assert_eq!(
            IniDocument::parse(&rendered).get("LANGUAGE", "mod_language_override"),
            Some("russian")
        );
    }

    #[test]
    fn trailing_comments_survive_an_edit() {
        let mut doc = IniDocument::parse("[SCALING]\r\nboss_health_scaling = 100 ; default\r\n");
        doc.set("SCALING", "boss_health_scaling", "150");
        assert!(doc.render().contains("boss_health_scaling = 150 ; default"));
    }

    #[test]
    fn toggles_reject_anything_but_zero_and_one() {
        assert!(validate("GAMEPLAY", "allow_invaders", "1").is_ok());
        assert!(validate("GAMEPLAY", "allow_invaders", "0").is_ok());
        assert!(validate("GAMEPLAY", "allow_invaders", "2").is_err());
        assert!(validate("GAMEPLAY", "allow_invaders", "true").is_err());
    }

    #[test]
    fn ranges_are_clamped_by_the_spec() {
        assert!(validate("GAMEPLAY", "default_boot_master_volume", "10").is_ok());
        assert!(validate("GAMEPLAY", "default_boot_master_volume", "11").is_err());
        assert!(validate("SCALING", "boss_health_scaling", "0").is_ok());
    }

    #[test]
    fn choices_only_accept_listed_values() {
        assert!(validate("GAMEPLAY", "overhead_player_display", "5").is_ok());
        assert!(validate("GAMEPLAY", "overhead_player_display", "6").is_err());
    }

    #[test]
    fn save_extension_rules_protect_the_vanilla_save() {
        assert!(validate("SAVE", "save_file_extension", "co2").is_ok());
        assert!(validate("SAVE", "save_file_extension", "coop2").is_ok());
        // Sharing the vanilla extension is the one mistake that loses characters.
        assert!(validate("SAVE", "save_file_extension", "sl2").is_err());
        assert!(validate("SAVE", "save_file_extension", "SL2").is_err());
        assert!(validate("SAVE", "save_file_extension", "").is_err());
        assert!(validate("SAVE", "save_file_extension", "co 2").is_err());
        assert!(validate("SAVE", "save_file_extension", &"a".repeat(121)).is_err());
    }

    #[test]
    fn passwords_reject_characters_that_would_corrupt_the_file() {
        assert!(validate("PASSWORD", "cooppassword", "grace-rune-42").is_ok());
        assert!(validate("PASSWORD", "cooppassword", "a;b").is_err());
        assert!(validate("PASSWORD", "cooppassword", "a\nb").is_err());
    }

    #[test]
    fn unknown_keys_pass_through_for_forward_compatibility() {
        assert_eq!(
            validate("GAMEPLAY", "some_future_key", "whatever").unwrap(),
            "whatever"
        );
    }

    #[test]
    fn generated_passwords_are_usable_as_ini_values() {
        for _ in 0..64 {
            let password = generate_password();
            assert!(validate("PASSWORD", "cooppassword", &password).is_ok());
            assert!(!password.contains(' '));
            assert!(password.len() >= 7);
        }
    }

    #[test]
    fn a_file_built_from_scratch_reparses_to_the_documented_defaults() {
        let doc = IniDocument::parse(&stock_settings());
        assert_eq!(doc.get("GAMEPLAY", "allow_invaders"), Some("1"));
        assert_eq!(doc.get("SCALING", "boss_health_scaling"), Some("100"));
        assert_eq!(doc.get("SCALING", "enemy_health_scaling"), Some("35"));
        assert_eq!(doc.get("SCALING", "enemy_posture_scaling"), Some("15"));
        assert_eq!(doc.get("SCALING", "boss_posture_scaling"), Some("20"));
        assert_eq!(doc.get("SAVE", "save_file_extension"), Some("co2"));
        assert_eq!(doc.get("PASSWORD", "cooppassword"), Some(""));
    }

    #[test]
    fn every_field_default_passes_its_own_validation() {
        for field in FIELDS {
            assert!(
                validate(field.section, field.key, field.default).is_ok(),
                "default for {}.{} is invalid",
                field.section,
                field.key
            );
        }
    }
}
