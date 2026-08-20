//! The codex: an offline reference for what is actually in the game.
//!
//! Looking a weapon up means leaving the launcher, finding a wiki, and waiting
//! for a page of adverts to render. The data is small enough that none of that
//! is necessary — the whole of ELDEN RING is about two and a half thousand rows
//! across fifteen collections, which fits in a few megabytes of JSON.
//!
//! It is fetched once and kept. The upstream API rate-limits hard: fifteen
//! requests in a row start answering 404, so the sync paces itself and retries,
//! and after that every search is a local string match. It also means the codex
//! keeps working with the network off, which is the state a lot of people play
//! modded copies in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};

const BASE: &str = "https://eldenring.fanapis.com/api";
const PAGE: usize = 100;

/// The collections the API exposes, in the order they are worth browsing.
pub const KINDS: &[(&str, &str)] = &[
    ("weapons", "Weapons"),
    ("shields", "Shields"),
    ("armors", "Armour"),
    ("talismans", "Talismans"),
    ("items", "Items"),
    ("ammos", "Ammunition"),
    ("sorceries", "Sorceries"),
    ("incantations", "Incantations"),
    ("ashes", "Ashes of War"),
    ("spirits", "Spirit Ashes"),
    ("bosses", "Bosses"),
    ("creatures", "Creatures"),
    ("npcs", "NPCs"),
    ("locations", "Locations"),
    ("classes", "Classes"),
];

pub fn label_for(kind: &str) -> &str {
    KINDS
        .iter()
        .find(|(id, _)| *id == kind)
        .map(|(_, label)| *label)
        .unwrap_or(kind)
}

/// One readable line of an entry's stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexEntry {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub image: Option<String>,
    pub description: Option<String>,
    pub facts: Vec<Fact>,
}

impl CodexEntry {
    /// Where to read more.
    ///
    /// Routed per edition on purpose: with The Convergence loaded, a vanilla
    /// wiki page describes a weapon the player does not have.
    pub fn wiki_url(&self, edition: Option<&str>) -> String {
        match edition {
            Some("convergence") => format!(
                "https://wiki.convergencemod.com/index.php?search={}",
                urlencode(&self.name)
            ),
            _ => format!(
                "https://eldenring.wiki.fextralife.com/{}",
                self.name.replace(' ', "+")
            ),
        }
    }
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Turning the API's shapes into something uniform
// ---------------------------------------------------------------------------

/// Flattens one API record.
///
/// Every collection has a different shape — a weapon carries `attack`,
/// `scalesWith` and `requiredAttributes`, a boss carries `drops` and `region` —
/// and none of them is worth its own struct. Anything that is not the name,
/// image or description becomes a labelled line, and the arrays of
/// `{name, amount}` objects the API likes collapse into one line each.
pub fn flatten(kind: &str, value: &serde_json::Value) -> Option<CodexEntry> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.to_string();

    let text = |key: &str| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let mut facts = Vec::new();
    for (key, raw) in object {
        if matches!(key.as_str(), "id" | "name" | "image" | "description") {
            continue;
        }
        if let Some(value) = render(raw) {
            facts.push(Fact {
                label: humanise(key),
                value,
            });
        }
    }

    Some(CodexEntry {
        id: object
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&name)
            .to_string(),
        kind: kind.to_string(),
        name,
        image: text("image"),
        description: text("description"),
        facts,
    })
}

/// One field, as a line of text, or nothing when it says nothing.
fn render(raw: &serde_json::Value) -> Option<String> {
    match raw {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(if *b { "Yes" } else { "No" }.into()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::String(s) => (!s.is_empty()).then(|| s.clone()),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(render_item).collect();
            (!parts.is_empty()).then(|| parts.join("  ·  "))
        }
        serde_json::Value::Object(_) => render_item(raw),
    }
}

/// One element of an array. `{name: "Phy", amount: 113}` becomes `Phy 113`.
fn render_item(raw: &serde_json::Value) -> Option<String> {
    match raw {
        serde_json::Value::String(s) => (!s.is_empty()).then(|| s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Object(map) => {
            let name = map.get("name").and_then(serde_json::Value::as_str)?;
            let amount = map
                .get("amount")
                .or_else(|| map.get("scaling"))
                .or_else(|| map.get("value"));
            match amount {
                // A zero here means the weapon does no damage of that type, and
                // printing five zeroes buries the one number that matters.
                Some(serde_json::Value::Number(n)) if n.as_f64() == Some(0.0) => None,
                Some(serde_json::Value::String(s)) if s.is_empty() || s == "-" => None,
                Some(other) => Some(format!("{name} {}", render(other)?)),
                None => Some(name.to_string()),
            }
        }
        _ => None,
    }
}

/// `requiredAttributes` becomes `Required attributes`.
fn humanise(key: &str) -> String {
    let mut out = String::new();
    for (index, ch) in key.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            out.push(' ');
            out.push(ch.to_ascii_lowercase());
        } else if index == 0 {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

pub fn cache_dir(app_data: &Path) -> PathBuf {
    app_data.join("codex")
}

fn cache_file(app_data: &Path, kind: &str) -> PathBuf {
    cache_dir(app_data).join(format!("{kind}.json"))
}

/// Everything already downloaded, in browse order.
pub fn load(app_data: &Path) -> Vec<CodexEntry> {
    let mut all = Vec::new();
    for (kind, _) in KINDS {
        let path = cache_file(app_data, kind);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(mut entries) = serde_json::from_str::<Vec<CodexEntry>>(&text) {
            all.append(&mut entries);
        }
    }
    all
}

/// How much of the codex is on disk.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexState {
    pub entries: usize,
    pub kinds: usize,
    pub syncing: bool,
    pub message: String,
    pub done_kinds: usize,
    pub total_kinds: usize,
    pub error: Option<String>,
}

/// Ranked substring search.
///
/// Two and a half thousand rows is small enough that scanning them all beats
/// maintaining an index, and it keeps the ordering rules obvious: an exact name
/// first, then names that start with the query, then anything that contains it,
/// and only then a match in the description.
pub fn search<'a>(
    entries: &'a [CodexEntry],
    query: &str,
    kind: Option<&str>,
    limit: usize,
) -> Vec<&'a CodexEntry> {
    let needle = query.trim().to_ascii_lowercase();
    let mut hits: Vec<(u8, &CodexEntry)> = entries
        .iter()
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .filter_map(|entry| {
            let name = entry.name.to_ascii_lowercase();
            if needle.is_empty() {
                return Some((3, entry));
            }
            if name == needle {
                Some((0, entry))
            } else if name.starts_with(&needle) {
                Some((1, entry))
            } else if name.contains(&needle) {
                Some((2, entry))
            } else if entry
                .description
                .as_deref()
                .is_some_and(|d| d.to_ascii_lowercase().contains(&needle))
            {
                Some((4, entry))
            } else {
                None
            }
        })
        .collect();

    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    hits.into_iter().take(limit).map(|(_, e)| e).collect()
}

/// Downloads one collection, page by page.
pub async fn fetch_kind(http: &reqwest::Client, kind: &str) -> Result<Vec<CodexEntry>> {
    let mut entries = Vec::new();
    let mut page = 0usize;

    loop {
        let url = format!("{BASE}/{kind}?limit={PAGE}&page={page}");
        let body = get_with_retry(http, &url).await?;

        let parsed: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| Error::Parse {
                what: format!("{kind} page {page}"),
                detail: e.to_string(),
            })?;

        let rows = parsed
            .get("data")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        if rows.is_empty() {
            break;
        }
        for row in &rows {
            if let Some(entry) = flatten(kind, row) {
                entries.push(entry);
            }
        }
        if rows.len() < PAGE {
            break;
        }
        page += 1;

        // The API answers 404 to a burst. Pacing is what keeps a sync from
        // half-finishing.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    }

    Ok(entries)
}

/// A GET that treats the API's throttling as something to wait out.
async fn get_with_retry(http: &reqwest::Client, url: &str) -> Result<String> {
    let mut wait = 800u64;
    let mut last = String::new();

    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            wait *= 2;
        }
        match http.get(url).send().await {
            Ok(response) if response.status().is_success() => {
                return response.text().await.map_err(|e| Error::Network(e.to_string()));
            }
            Ok(response) => last = format!("HTTP {}", response.status()),
            Err(error) => last = error.to_string(),
        }
    }

    Err(Error::Network(format!("{url}: {last}")))
}

/// Downloads everything and writes it to the cache.
pub async fn sync(
    http: &reqwest::Client,
    app_data: &Path,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<usize> {
    let dir = cache_dir(app_data);
    std::fs::create_dir_all(&dir).at(&dir)?;

    let mut total = 0usize;
    for (index, (kind, label)) in KINDS.iter().enumerate() {
        progress(index, KINDS.len(), label);
        let entries = fetch_kind(http, kind).await?;
        total += entries.len();

        let path = cache_file(app_data, kind);
        let text = serde_json::to_string(&entries).map_err(|e| Error::Parse {
            what: (*kind).to_string(),
            detail: e.to_string(),
        })?;
        std::fs::write(&path, text).at(&path)?;
    }
    progress(KINDS.len(), KINDS.len(), "done");

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_weapon_flattens_into_readable_lines() {
        let raw = json!({
            "id": "abc",
            "name": "Hand Axe",
            "image": "https://x/y.png",
            "description": "Commonly known as a hatchet.",
            "attack": [
                { "name": "Phy", "amount": 113 },
                { "name": "Mag", "amount": 0 },
                { "name": "Crit", "amount": 100 }
            ],
            "scalesWith": [{ "name": "Str", "scaling": "D" }],
            "requiredAttributes": [{ "name": "Str", "amount": 9 }],
            "category": "Axe",
            "weight": 3.5
        });

        let entry = flatten("weapons", &raw).expect("flattened");
        assert_eq!(entry.name, "Hand Axe");
        assert_eq!(entry.kind, "weapons");

        let get = |label: &str| {
            entry
                .facts
                .iter()
                .find(|f| f.label == label)
                .map(|f| f.value.clone())
        };

        // Zero-damage types are dropped so the real numbers stay visible.
        assert_eq!(get("Attack").as_deref(), Some("Phy 113  ·  Crit 100"));
        assert_eq!(get("Scales with").as_deref(), Some("Str D"));
        assert_eq!(get("Required attributes").as_deref(), Some("Str 9"));
        assert_eq!(get("Category").as_deref(), Some("Axe"));
        assert_eq!(get("Weight").as_deref(), Some("3.5"));
    }

    #[test]
    fn a_boss_keeps_its_drops_and_drops_its_nulls() {
        let raw = json!({
            "id": "b1",
            "name": "Abductor Virgins",
            "image": null,
            "region": "Mount Gelmir",
            "description": "Deadly mechanical constructs.",
            "drops": ["10.000 Runes", "Inquisitor's Girandole"],
            "healthPoints": "???"
        });

        let entry = flatten("bosses", &raw).unwrap();
        assert!(entry.image.is_none(), "a null image must not become a broken one");
        assert!(entry
            .facts
            .iter()
            .any(|f| f.label == "Drops" && f.value.contains("Girandole")));
    }

    #[test]
    fn camel_case_keys_become_sentences() {
        assert_eq!(humanise("requiredAttributes"), "Required attributes");
        assert_eq!(humanise("healthPoints"), "Health points");
        assert_eq!(humanise("region"), "Region");
    }

    fn entry(name: &str, kind: &str, description: &str) -> CodexEntry {
        CodexEntry {
            id: name.into(),
            kind: kind.into(),
            name: name.into(),
            image: None,
            description: Some(description.into()),
            facts: Vec::new(),
        }
    }

    #[test]
    fn search_puts_an_exact_name_above_a_prefix_above_a_description() {
        let all = vec![
            entry("Moonveil Talisman", "talismans", "nothing"),
            entry("Rusted Anchor", "weapons", "A moonveil is mentioned here"),
            entry("Moonveil", "weapons", "nothing"),
        ];

        let hits = search(&all, "moonveil", None, 10);
        assert_eq!(hits[0].name, "Moonveil");
        assert_eq!(hits[1].name, "Moonveil Talisman");
        assert_eq!(hits[2].name, "Rusted Anchor");
    }

    #[test]
    fn search_can_be_held_to_one_collection() {
        let all = vec![
            entry("Moonveil", "weapons", ""),
            entry("Moonveil", "talismans", ""),
        ];
        let hits = search(&all, "moon", Some("talismans"), 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "talismans");
    }

    #[test]
    fn an_empty_query_lists_the_collection() {
        let all = vec![entry("B", "weapons", ""), entry("A", "weapons", "")];
        let hits = search(&all, "  ", None, 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "A", "and sorts by name");
    }

    #[test]
    fn the_wiki_link_follows_the_active_edition() {
        let sword = entry("Sword of Night and Flame", "weapons", "");
        assert_eq!(
            sword.wiki_url(None),
            "https://eldenring.wiki.fextralife.com/Sword+of+Night+and+Flame"
        );
        // With a total conversion loaded, the vanilla page describes a weapon
        // the player does not have.
        assert!(sword
            .wiki_url(Some("convergence"))
            .starts_with("https://wiki.convergencemod.com/index.php?search=Sword+of+Night"));
    }

    #[test]
    fn names_with_punctuation_survive_the_url() {
        let ring = entry("Marika's Scarseal", "talismans", "");
        assert!(ring.wiki_url(Some("convergence")).contains("Marika%27s"));
        assert_eq!(
            ring.wiki_url(None),
            "https://eldenring.wiki.fextralife.com/Marika's+Scarseal"
        );
    }

    #[test]
    fn every_kind_has_a_label() {
        assert_eq!(label_for("weapons"), "Weapons");
        assert_eq!(label_for("ashes"), "Ashes of War");
        assert_eq!(label_for("nonsense"), "nonsense");
    }
}
