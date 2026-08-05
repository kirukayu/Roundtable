//! Steam discovery: install root, library folders, installed apps and local accounts.
//!
//! Everything here is read-only. Roundtable never writes into Steam's own files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::Result;

/// A node in Valve's KeyValues text format (`.vdf` / `.acf`).
#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Map(BTreeMap<String, Vdf>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(map) => map
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Map(_) => None,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Vdf)> {
        match self {
            Vdf::Map(map) => itertools_entries(map),
            Vdf::Str(_) => itertools_entries(EMPTY_MAP.get_or_init(BTreeMap::new)),
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Vdf> {
        keys.iter().try_fold(self, |node, key| node.get(key))
    }

    pub fn str_at(&self, keys: &[&str]) -> Option<&str> {
        self.path(keys).and_then(Vdf::as_str)
    }
}

static EMPTY_MAP: std::sync::OnceLock<BTreeMap<String, Vdf>> = std::sync::OnceLock::new();

fn itertools_entries(map: &BTreeMap<String, Vdf>) -> std::collections::btree_map::Iter<'_, String, Vdf> {
    map.iter()
}

/// Parses Valve KeyValues text.
///
/// The format is quoted tokens plus `{ }` blocks; comments start with `//`. Duplicate
/// keys are rare in the files we read, and the last one wins, matching Steam.
pub fn parse_vdf(input: &str) -> Vdf {
    let mut chars = input.char_indices().peekable();
    let mut bytes = input.as_bytes();
    let _ = &mut bytes;
    let mut tokens: Vec<Token> = Vec::new();

    while let Some((_, ch)) = chars.next() {
        match ch {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => {
                let mut value = String::new();
                while let Some((_, c)) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some((_, escaped)) = chars.next() {
                                value.push(match escaped {
                                    'n' => '\n',
                                    't' => '\t',
                                    other => other,
                                });
                            }
                        }
                        '"' => break,
                        other => value.push(other),
                    }
                }
                tokens.push(Token::Value(value));
            }
            '/' => {
                if matches!(chars.peek(), Some((_, '/'))) {
                    for (_, c) in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
            }
            c if c.is_whitespace() => {}
            // Unquoted tokens appear in hand-edited files; accept them too.
            other => {
                let mut value = String::from(other);
                while let Some((_, c)) = chars.peek() {
                    if c.is_whitespace() || *c == '{' || *c == '}' || *c == '"' {
                        break;
                    }
                    value.push(*c);
                    chars.next();
                }
                tokens.push(Token::Value(value));
            }
        }
    }

    let mut cursor = 0usize;
    parse_block(&tokens, &mut cursor)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Open,
    Close,
    Value(String),
}

fn parse_block(tokens: &[Token], cursor: &mut usize) -> Vdf {
    let mut map = BTreeMap::new();

    while *cursor < tokens.len() {
        match &tokens[*cursor] {
            Token::Close => {
                *cursor += 1;
                break;
            }
            Token::Open => {
                // A block with no key; skip past it rather than losing sync.
                *cursor += 1;
                parse_block(tokens, cursor);
            }
            Token::Value(key) => {
                *cursor += 1;
                match tokens.get(*cursor) {
                    Some(Token::Open) => {
                        *cursor += 1;
                        let child = parse_block(tokens, cursor);
                        map.insert(key.clone(), child);
                    }
                    Some(Token::Value(value)) => {
                        map.insert(key.clone(), Vdf::Str(value.clone()));
                        *cursor += 1;
                    }
                    _ => {
                        map.insert(key.clone(), Vdf::Str(String::new()));
                    }
                }
            }
        }
    }

    Vdf::Map(map)
}

/// Where Steam is installed, from the registry.
#[cfg(windows)]
pub fn steam_root() -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey("Software\\Valve\\Steam") {
        if let Ok(path) = key.get_value::<String, _>("SteamPath") {
            let path = PathBuf::from(path.replace('/', "\\"));
            if path.is_dir() {
                return Some(path);
            }
        }
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) =
        hklm.open_subkey_with_flags("SOFTWARE\\Valve\\Steam", KEY_READ | KEY_WOW64_32KEY)
    {
        if let Ok(path) = key.get_value::<String, _>("InstallPath") {
            let path = PathBuf::from(path);
            if path.is_dir() {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(not(windows))]
pub fn steam_root() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    [".steam/steam", ".local/share/Steam"]
        .into_iter()
        .map(|rel| home.join(rel))
        .find(|p| p.is_dir())
}

/// Every Steam library folder, including the one inside the Steam install itself.
pub fn library_folders() -> Vec<PathBuf> {
    let Some(root) = steam_root() else {
        return Vec::new();
    };

    let mut libraries = vec![root.clone()];

    let manifest = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&manifest) {
        let parsed = parse_vdf(&text);
        // Modern layout: libraryfolders -> "0" -> { path = "..." }.
        if let Some(node) = parsed.get("libraryfolders") {
            for (_, entry) in node.entries() {
                let candidate = match entry {
                    Vdf::Map(_) => entry.get("path").and_then(Vdf::as_str).map(PathBuf::from),
                    Vdf::Str(raw) => Some(PathBuf::from(raw)),
                };
                if let Some(path) = candidate {
                    if path.is_dir() {
                        libraries.push(path);
                    }
                }
            }
        }
    }

    libraries.sort();
    libraries.dedup();
    libraries.retain(|p| p.join("steamapps").is_dir());
    libraries
}

/// Resolves the install directory of a Steam app by reading its manifest.
pub fn app_install_dir(app_id: u32) -> Option<PathBuf> {
    for library in library_folders() {
        let manifest = library
            .join("steamapps")
            .join(format!("appmanifest_{app_id}.acf"));
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let parsed = parse_vdf(&text);
        let Some(install_dir) = parsed.str_at(&["AppState", "installdir"]) else {
            continue;
        };
        let path = library
            .join("steamapps")
            .join("common")
            .join(install_dir);
        if path.is_dir() {
            return Some(path);
        }
    }
    None
}

/// A Steam account that has signed in on this machine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamAccount {
    pub steam_id64: u64,
    pub account_name: String,
    pub persona_name: String,
    pub most_recent: bool,
}

/// Reads `config/loginusers.vdf` so save folders can be labelled with real names
/// instead of bare 17-digit ids.
pub fn local_accounts() -> Vec<SteamAccount> {
    let Some(root) = steam_root() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(root.join("config").join("loginusers.vdf")) else {
        return Vec::new();
    };

    let parsed = parse_vdf(&text);
    let Some(users) = parsed.get("users") else {
        return Vec::new();
    };

    let mut accounts: Vec<SteamAccount> = users
        .entries()
        .filter_map(|(id, node)| {
            let steam_id64 = id.parse::<u64>().ok()?;
            Some(SteamAccount {
                steam_id64,
                account_name: node
                    .get("AccountName")
                    .and_then(Vdf::as_str)
                    .unwrap_or_default()
                    .to_string(),
                persona_name: node
                    .get("PersonaName")
                    .and_then(Vdf::as_str)
                    .unwrap_or_default()
                    .to_string(),
                most_recent: node
                    .get("MostRecent")
                    .and_then(Vdf::as_str)
                    .is_some_and(|v| v == "1"),
            })
        })
        .collect();

    accounts.sort_by(|a, b| b.most_recent.cmp(&a.most_recent).then(a.persona_name.cmp(&b.persona_name)));
    accounts
}

/// True when a Steam client process is currently running.
pub fn is_running(system: &sysinfo::System) -> bool {
    system
        .processes()
        .values()
        .any(|p| p.name().to_string_lossy().eq_ignore_ascii_case("steam.exe"))
}

/// Writes the `steam_appid.txt` that Steamworks reads when a game is started
/// outside the client. Harmless for Steam copies, and it is what stops many mods
/// from failing with "trying to find steam" on non-Steam installs.
pub fn write_appid_file(game_dir: &Path, app_id: u32) -> Result<PathBuf> {
    let target = game_dir.join("steam_appid.txt");
    std::fs::write(&target, app_id.to_string())?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_blocks() {
        let text = r#"
            "AppState"
            {
                "appid"      "1245620"
                "installdir" "ELDEN RING"
                "UserConfig"
                {
                    "language" "english"
                }
            }
        "#;
        let vdf = parse_vdf(text);
        assert_eq!(vdf.str_at(&["AppState", "installdir"]), Some("ELDEN RING"));
        assert_eq!(
            vdf.str_at(&["AppState", "UserConfig", "language"]),
            Some("english")
        );
    }

    #[test]
    fn key_lookup_ignores_case() {
        let vdf = parse_vdf(r#""Users" { "76561198000000000" { "AccountName" "tarnished" } }"#);
        assert_eq!(
            vdf.str_at(&["users", "76561198000000000", "accountname"]),
            Some("tarnished")
        );
    }

    #[test]
    fn skips_comments_and_handles_escapes() {
        let text = r#"
            // a comment line
            "libraryfolders"
            {
                "0" { "path" "D:\\SteamLibrary" } // trailing comment
            }
        "#;
        let vdf = parse_vdf(text);
        assert_eq!(
            vdf.str_at(&["libraryfolders", "0", "path"]),
            Some("D:\\SteamLibrary")
        );
    }

    #[test]
    fn tolerates_unterminated_input() {
        // Truncated files must not panic; they simply yield what was readable.
        let vdf = parse_vdf(r#""AppState" { "appid" "1245620" "#);
        assert_eq!(vdf.str_at(&["AppState", "appid"]), Some("1245620"));
    }

    #[test]
    fn missing_keys_return_none_rather_than_panicking() {
        let vdf = parse_vdf(r#""a" { "b" "c" }"#);
        assert_eq!(vdf.str_at(&["a", "nope"]), None);
        assert_eq!(vdf.str_at(&["nope", "b"]), None);
        assert_eq!(vdf.str_at(&["a", "b", "too", "deep"]), None);
    }

    #[test]
    fn string_node_has_no_children() {
        let vdf = parse_vdf(r#""a" "b""#);
        assert_eq!(vdf.get("a").unwrap().as_str(), Some("b"));
        assert_eq!(vdf.get("a").unwrap().entries().count(), 0);
    }
}
