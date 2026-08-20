//! The wikis, inside the launcher.
//!
//! Both of the wikis that matter here run MediaWiki, which means both have an
//! API that hands over a rendered article as HTML. So rather than sending people
//! out to a browser tab full of adverts, the article is fetched, stripped down
//! to its content, and rendered in the launcher's own type.
//!
//! Titles are mirrored in full — six thousand strings is nothing, and it makes
//! search cover everything. Article bodies are fetched the first time they are
//! opened and kept from then on, so a page you have read once still opens with
//! the network off.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiSource {
    pub id: &'static str,
    pub name: &'static str,
    #[serde(skip)]
    pub api: &'static str,
    #[serde(skip)]
    pub origin: &'static str,
    /// Where an article lives on the original site.
    #[serde(skip)]
    pub article: &'static str,
}

pub const SOURCES: &[WikiSource] = &[
    WikiSource {
        id: "eldenring",
        name: "ELDEN RING",
        api: "https://eldenring.fandom.com/api.php",
        origin: "https://eldenring.fandom.com",
        article: "https://eldenring.fandom.com/wiki/",
    },
    WikiSource {
        id: "convergence",
        name: "The Convergence",
        api: "https://wiki.convergencemod.com/api.php",
        origin: "https://wiki.convergencemod.com",
        article: "https://wiki.convergencemod.com/",
    },
    // The same wiki in Russian, and the only place the game's Russian names
    // are written down as prose.
    //
    // A model asked in Russian where a boss is will otherwise translate the
    // English name itself and produce something the player has never seen —
    // Redmane Castle came back as "Крепость Красного Лва" where the game says
    // "Замок Красногривых", and the Wailing Dunes as "Стонущие дюны" where it
    // says "Воющие дюны". Reading a wiki written in their language is what
    // stops the guessing.
    WikiSource {
        id: "eldenring-ru",
        name: "ELDEN RING (Russian)",
        api: "https://eldenring.fandom.com/ru/api.php",
        origin: "https://eldenring.fandom.com/ru",
        article: "https://eldenring.fandom.com/ru/wiki/",
    },
    // German and Spanish, for the same reason and on the same terms: a player
    // reading their game in one of these has the names on their screen written
    // out here and nowhere else.
    //
    // Only these two of the eight that exist. The article counts were read off
    // each wiki's own statistics before any of them went in — English 4,874,
    // Russian 3,835, German 1,390, Spanish 404, and then a cliff: Chinese 147,
    // Italian 92, Ukrainian 53, French 42, Polish 30. A wiki of thirty
    // articles is not coverage; it is a download the player pays for and a
    // handful of titles that dilute every search they run. Portuguese,
    // Japanese and Korean have no wiki at all.
    WikiSource {
        id: "eldenring-de",
        name: "ELDEN RING (German)",
        api: "https://eldenring.fandom.com/de/api.php",
        origin: "https://eldenring.fandom.com/de",
        article: "https://eldenring.fandom.com/de/wiki/",
    },
    WikiSource {
        id: "eldenring-es",
        name: "ELDEN RING (Spanish)",
        api: "https://eldenring.fandom.com/es/api.php",
        origin: "https://eldenring.fandom.com/es",
        article: "https://eldenring.fandom.com/es/wiki/",
    },
];

pub fn source(id: &str) -> Option<&'static WikiSource> {
    SOURCES.iter().find(|s| s.id == id)
}

/// The language a wiki is written in, when it is written in one in particular.
///
/// `None` means English and everybody. A wiki that names a language is only
/// worth searching for somebody playing in it.
pub fn spoken_in(source: &WikiSource) -> Option<&'static str> {
    match source.id {
        "eldenring-ru" => Some("rus"),
        "eldenring-de" => Some("german"),
        "eldenring-es" => Some("spanish"),
        _ => None,
    }
}

/// The wiki that belongs to whichever edition is loaded.
///
/// By id rather than by position: the list is ordered for reading and adding a
/// wiki to the middle of it should not silently hand every Convergence player
/// somebody else's.
pub fn for_edition(edition: Option<&str>) -> &'static WikiSource {
    let wanted = match edition {
        Some("convergence") => "convergence",
        _ => "eldenring",
    };
    source(wanted).unwrap_or(&SOURCES[0])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiPage {
    pub source: String,
    pub title: String,
    /// Content HTML, already stripped of everything that is not the article.
    pub html: String,
    /// The article on the original site, for when someone wants the real thing.
    pub origin: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiIndexState {
    pub source: String,
    pub titles: usize,
    pub cached_pages: usize,
    pub syncing: bool,
    pub message: String,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

fn dir(app_data: &Path, source: &str) -> PathBuf {
    app_data.join("wiki").join(source)
}

fn index_path(app_data: &Path, source: &str) -> PathBuf {
    dir(app_data, source).join("_titles.json")
}

/// A title turned into something safe to use as a file name.
///
/// Article titles contain slashes, colons and quotes. Hashing sidesteps every
/// one of those and the case-insensitivity of the filesystem at the same time.
fn page_path(app_data: &Path, source: &str, title: &str) -> PathBuf {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(title.to_lowercase().as_bytes());
    let name = hex::encode(hasher.finalize())[..24].to_string();
    dir(app_data, source).join(format!("{name}.json"))
}

pub fn titles(app_data: &Path, source: &str) -> Vec<String> {
    std::fs::read_to_string(index_path(app_data, source))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// When this mirror was taken.
///
/// The title index is written once, when the wiki is downloaded, and not
/// touched again until it is downloaded afresh — so the file's own timestamp is
/// the answer to "how old is my copy". The launcher used to say that was not
/// readable, and the assistant repeated it: asked how fresh their wiki was, it
/// answered that it had no way to tell, standing on the file.
pub fn taken_at(app_data: &Path, source: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(index_path(app_data, source)).ok()?.modified().ok()
}

pub fn cached_page_count(app_data: &Path, source: &str) -> usize {
    std::fs::read_dir(dir(app_data, source))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .ends_with(".json")
                })
                .count()
                .saturating_sub(1) // the title index lives here too
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod when_tests {
    use super::*;

    /// A mirror that is there has a date, and one that is not has none.
    ///
    /// The launcher used to say the date was unreadable and the assistant
    /// repeated it, asked how fresh the wiki was, while standing on the file
    /// that carries it.
    #[test]
    fn a_mirror_says_when_it_was_taken() {
        let dir = std::env::temp_dir().join("roundtable-wiki-when");
        let _ = std::fs::remove_dir_all(&dir);

        // Nothing downloaded: no date, and no pretending there is one.
        assert!(taken_at(&dir, "eldenring").is_none());

        let holding = super::dir(&dir, "eldenring");
        std::fs::create_dir_all(&holding).expect("make the folder");
        std::fs::write(index_path(&dir, "eldenring"), "[\"Reduvia\"]").expect("write an index");

        let when = taken_at(&dir, "eldenring").expect("a file just written has a timestamp");
        let age = when.elapsed().expect("written in the past");
        assert!(age.as_secs() < 120, "a file written now reads as {age:?} old");
        // And it is the index's date, not the folder's: a page cached later
        // must not make the mirror look freshly taken.
        assert_eq!(titles(&dir, "eldenring"), vec!["Reduvia".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Ranked title search, same ordering rules as the codex.
/// Titles matching a query, best first.
///
/// Two things were wrong with this and both were silent.
///
/// It lowercased with `to_ascii_lowercase`, which leaves every non-ASCII letter
/// exactly as it was. On the Russian mirror that means a query beginning with a
/// capital — which is how anybody writes a name — matched nothing, because the
/// titles were lowercased just as ineffectively and the two never met in the
/// middle. Half the search was dead for half the mirrors.
///
/// And it matched the query as ONE string, so anything a person would actually
/// type failed: "Radahn boss fight" is not a substring of any title, and asking
/// about two things at once found neither. It now scores on WORDS — a title
/// carrying all of them ranks above one carrying some — which is what makes a
/// phrase usable as a query at all.
pub fn search<'a>(titles: &'a [String], query: &str, limit: usize) -> Vec<&'a String> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return titles.iter().take(limit).collect();
    }
    let words: Vec<&str> = needle.split_whitespace().filter(|w| w.len() > 1).collect();

    let mut hits: Vec<(u8, usize, &String)> = titles
        .iter()
        .filter_map(|title| {
            let lower = title.to_lowercase();
            // The whole query, which is still the best thing that can happen.
            let rank = if lower == needle {
                0
            } else if lower.starts_with(&needle) {
                1
            } else if lower.contains(&needle) {
                2
            } else if !words.is_empty() && words.iter().all(|word| lower.contains(word)) {
                // Every word, in any order and anywhere in the title.
                3
            } else if words.len() > 1 && words.iter().any(|word| lower.contains(word)) {
                // Some of them. Last, and only for a phrase — for a single
                // word this is the same as the contains above.
                4
            } else {
                return None;
            };
            // Within a rank, the title that spends least of itself on other
            // things: "Reduvia" before "List of daggers including Reduvia".
            Some((rank, title.chars().count(), title))
        })
        .collect();

    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    hits.into_iter().take(limit).map(|(_, _, title)| title).collect()
}

// ---------------------------------------------------------------------------
// Cleaning the article
// ---------------------------------------------------------------------------

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern")
}

struct Patterns {
    script: Regex,
    style: Regex,
    noscript: Regex,
    editsection: Regex,
    comment: Regex,
    handler: Regex,
    href: Regex,
    src: Regex,
    srcset: Regex,
    empty_class: Regex,
    data_src: Regex,
    lazy: Regex,
    hidden_attr: Regex,
    inline_display_none: Regex,
    aria_hidden: Regex,
}

fn patterns() -> &'static Patterns {
    static CELL: OnceLock<Patterns> = OnceLock::new();
    CELL.get_or_init(|| Patterns {
        script: re(r"(?is)<script\b[^>]*>.*?</script>"),
        style: re(r"(?is)<style\b[^>]*>.*?</style>"),
        // Fandom puts a copy of every image in a `<noscript>` beside the lazy
        // one. A browser with scripting on ignores it, so it is thirty-odd
        // invisible duplicates per page and nothing else.
        noscript: re(r"(?is)<noscript\b[^>]*>.*?</noscript>"),
        // Finds where an [edit] control starts. Removing it needs depth
        // counting, not a pattern — see `strip_spans`.
        editsection: re(r#"(?is)<span[^>]*class="[^"]*mw-editsection[^"]*"[^>]*>"#),
        comment: re(r"(?s)<!--.*?-->"),
        // Any inline event handler. MediaWiki does not emit these, but this
        // renders third-party HTML and the cost of being sure is one regex.
        handler: re(r#"(?i)\son[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)"#),
        href: re(r#"(?i)href\s*=\s*"([^"]*)""#),
        src: re(r#"(?i)\bsrc\s*=\s*"([^"]*)""#),
        srcset: re(r#"(?i)\ssrcset\s*=\s*"[^"]*""#),
        empty_class: re(r#"(?i)\sclass="\s*""#),
        // Fandom lazy-loads a fifth of its images: the real address is in
        // `data-src` and `src` holds a placeholder until their script runs.
        data_src: re(r#"(?is)<img([^>]*?)\sdata-src\s*=\s*"([^"]+)"([^>]*)>"#),
        // Without their script, a lazy image inside a container that never
        // scrolls simply never loads.
        lazy: re(r#"(?i)\sloading\s*=\s*"lazy""#),
        // Everything below is a thing MediaWiki hides for JavaScript to reveal:
        // tab panels, collapsed sections, spoiler boxes. There is no JavaScript
        // here, so hidden means gone, and gone is half the article.
        // No look-ahead in this regex engine, so the delimiter is captured and
        // put back. Requiring whitespace before `hidden` is what stops it
        // matching inside `data-hidden-by` or a class of `not-hidden-really`.
        hidden_attr: re(r#"(?i)\shidden(?:\s*=\s*(?:"[^"]*"|'[^']*'))?([\s>])"#),
        inline_display_none: re(r"(?i)display\s*:\s*none\s*;?"),
        aria_hidden: re(r#"(?i)\saria-hidden\s*=\s*"true""#),
    })
}

/// Drops any `src` already on the tag, so promoting `data-src` cannot leave two.
fn strip_src(attrs: &str) -> String {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| re(r#"(?i)\ssrc\s*=\s*"[^"]*""#))
        .replace_all(attrs, "")
        .to_string()
}

/// Percent-decodes just enough to turn a wiki href back into a title.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(if bytes[index] == b'_' { b' ' } else { bytes[index] });
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Pulls an article title out of a MediaWiki link, if it is one.
fn link_title(href: &str) -> Option<String> {
    let path = href.split('#').next().unwrap_or(href);

    if let Some(rest) = path.strip_prefix("/wiki/") {
        return (!rest.is_empty()).then(|| decode(rest));
    }
    if path.starts_with("/index.php") {
        let query = path.split_once('?')?.1;
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("title=") {
                return Some(decode(value));
            }
        }
        return None;
    }
    None
}

/// Removes every `<span>` whose opening tag `opener` matches, along with
/// everything nested inside it.
///
/// A regex cannot do this. MediaWiki's [edit] control wraps two anchors that
/// each contain their own spans, so a lazy `.*?</span>` stops at the first inner
/// close and leaves the rest of the control on the page.
fn strip_spans(html: &str, opener: &Regex) -> String {
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;

    while let Some(found) = opener.find_at(html, cursor) {
        out.push_str(&html[cursor..found.start()]);

        let mut depth = 1usize;
        let mut index = found.end();
        while depth > 0 && index < html.len() {
            let rest = &html[index..];
            if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("<span") {
                depth += 1;
                index += 5;
            } else if rest.len() >= 7 && rest[..7].eq_ignore_ascii_case("</span>") {
                depth -= 1;
                index += 7;
            } else {
                // Advance by a whole character; the byte after a multi-byte one
                // is not a valid slice boundary.
                index += html[index..].chars().next().map_or(1, char::len_utf8);
            }
        }
        cursor = index;
    }

    out.push_str(&html[cursor.min(html.len())..]);
    out
}

/// Reduces a MediaWiki article to the part worth reading, and makes every link
/// in it either safe or internal.
pub fn clean(html: &str, source: &WikiSource) -> String {
    let p = patterns();

    let mut out = p.script.replace_all(html, "").to_string();
    out = p.style.replace_all(&out, "").to_string();
    out = p.noscript.replace_all(&out, "").to_string();
    out = strip_spans(&out, &p.editsection);
    out = p.comment.replace_all(&out, "").to_string();
    out = p.handler.replace_all(&out, "").to_string();
    // Responsive image sets point at sizes that are not cached; one src is enough.
    out = p.srcset.replace_all(&out, "").to_string();

    // Promote the lazy address over the placeholder, then stop the browser
    // deferring what it is given.
    out = p
        .data_src
        .replace_all(&out, |caps: &regex::Captures| {
            let before = strip_src(&caps[1]);
            let after = strip_src(&caps[3]);
            format!(r#"<img{before} src="{}"{after}>"#, &caps[2])
        })
        .to_string();
    out = p.lazy.replace_all(&out, "").to_string();

    // Reveal what the wiki's own scripts would have revealed. Tab panels and
    // collapsed sections carry real article text, and this reader has no
    // scripts to open them with.
    out = p.hidden_attr.replace_all(&out, " $1").to_string();
    out = p.inline_display_none.replace_all(&out, "").to_string();
    out = p.aria_hidden.replace_all(&out, "").to_string();

    let origin = source.origin;
    let id = source.id;

    out = p
        .href
        .replace_all(&out, |caps: &regex::Captures| {
            let raw = &caps[1];
            match () {
                // An article on the same wiki stays inside the launcher.
                _ if link_title(raw).is_some() => {
                    let title = link_title(raw).unwrap_or_default();
                    // Doubled hashes: the value itself contains `"#`, which
                    // would close a single-hash raw string early.
                    format!(r##"href="#wiki:{id}:{}""##, title.replace('"', ""))
                }
                // Anything that could execute is not a link at all.
                _ if raw.trim_start().to_ascii_lowercase().starts_with("javascript:") => {
                    r##"href="#""##.to_string()
                }
                _ if raw.starts_with("//") => format!(r#"href="https:{raw}" target="_blank" rel="noreferrer""#),
                _ if raw.starts_with('/') => format!(r#"href="{origin}{raw}" target="_blank" rel="noreferrer""#),
                _ if raw.starts_with('#') => format!(r#"href="{raw}""#),
                _ => format!(r#"href="{raw}" target="_blank" rel="noreferrer""#),
            }
        })
        .to_string();

    // `referrerpolicy="no-referrer"` is not decoration. Fandom's CDN refuses
    // images when the Referer is an unknown host, and every request from here
    // carries `http://127.0.0.1:<port>` — which is how a page full of working
    // URLs still renders as broken icons.
    out = p
        .src
        .replace_all(&out, |caps: &regex::Captures| {
            let raw = &caps[1];
            let absolute = if raw.starts_with("//") {
                format!("https:{raw}")
            } else if raw.starts_with('/') {
                format!("{origin}{raw}")
            } else {
                raw.to_string()
            };
            format!(r#"referrerpolicy="no-referrer" src="{absolute}""#)
        })
        .to_string();

    out = p.empty_class.replace_all(&out, "").to_string();
    out.trim().to_string()
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

async fn get_json(http: &reqwest::Client, url: &str) -> Result<serde_json::Value> {
    let mut wait = 700u64;
    let mut last = String::new();

    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            wait *= 2;
        }
        match http.get(url).send().await {
            Ok(response) if response.status().is_success() => {
                let text = response
                    .text()
                    .await
                    .map_err(|e| Error::Network(e.to_string()))?;
                return serde_json::from_str(&text).map_err(|e| Error::Parse {
                    what: url.to_string(),
                    detail: e.to_string(),
                });
            }
            Ok(response) => last = format!("HTTP {}", response.status()),
            Err(error) => last = error.to_string(),
        }
    }
    Err(Error::Network(format!("{url}: {last}")))
}

/// What each article is called in the other languages the wiki is written in.
///
/// Telling a model not to translate a name itself is worth nothing unless it
/// can be given the real one. The wiki keeps that mapping — every article links
/// to its own translations — so "Redmane Castle" resolves to "Замок Рыжей
/// Гривы", which is the name printed in a Russian copy of the game. Left to
/// translate, the model produced "Крепость Красного Лва", which exists nowhere.
///
/// Built once alongside the titles, fifty at a time, and read from disk after.
pub async fn sync_langlinks(
    http: &reqwest::Client,
    app_data: &Path,
    source: &WikiSource,
    language: &str,
    mut progress: impl FnMut(usize),
) -> Result<usize> {
    let titles = titles(app_data, source.id);
    let mut found: BTreeMap<String, String> = BTreeMap::new();

    for batch in titles.chunks(50) {
        let joined = batch
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join("|");
        let url = format!(
            "{}?action=query&prop=langlinks&lllang={language}&lllimit=500&titles={}\
             &format=json&formatversion=2",
            source.api,
            encode(&joined)
        );

        let Ok(body) = get_json(http, &url).await else {
            continue;
        };
        if let Some(pages) = body
            .pointer("/query/pages")
            .and_then(serde_json::Value::as_array)
        {
            for page in pages {
                let (Some(english), Some(links)) = (
                    page.get("title").and_then(serde_json::Value::as_str),
                    page.get("langlinks").and_then(serde_json::Value::as_array),
                ) else {
                    continue;
                };
                if let Some(local) = links
                    .first()
                    .and_then(|l| l.get("title"))
                    .and_then(serde_json::Value::as_str)
                {
                    found.insert(english.to_string(), local.to_string());
                }
            }
        }
        progress(found.len());
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }

    let path = names_path(app_data, source.id, language);
    let text = serde_json::to_string(&found).map_err(|e| Error::Parse {
        what: "language names".into(),
        detail: e.to_string(),
    })?;
    std::fs::write(&path, text).at(&path)?;
    Ok(found.len())
}

fn names_path(app_data: &Path, source: &str, language: &str) -> PathBuf {
    dir(app_data, source).join(format!("_names-{language}.json"))
}

/// Article titles in English, and what the wiki calls them elsewhere.
type Names = Arc<BTreeMap<String, String>>;

/// The English-to-other-language table, loaded once.
fn names(app_data: &Path, source: &str, language: &str) -> Names {
    static CACHE: OnceLock<Mutex<HashMap<String, Names>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{source}/{language}");

    let mut held = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    held.entry(key)
        .or_insert_with(|| {
            let loaded = std::fs::read_to_string(names_path(app_data, source, language))
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default();
            Arc::new(loaded)
        })
        .clone()
}

/// What an article is called in the player's language, when the wiki says.
pub fn called_in(app_data: &Path, source: &str, language: &str, title: &str) -> Option<String> {
    names(app_data, source, language).get(title).cloned()
}

/// The other spellings of the same thing, for matching a question against a
/// game running in a different language.
///
/// The model writes in whichever language the player did, and the game answers
/// in whichever language it was installed in. Asked "what does Reduvia do" on a
/// Russian installation, the game's own text has never heard of Reduvia — it
/// knows Редувия — and the lookup came back empty on the first try every time.
///
/// The wiki already holds this table: it is the same langlink map that stops
/// the assistant inventing Russian names for places, read in both directions
/// because the question can arrive in either language.
pub fn also_called(app_data: &Path, source: &str, language: &str, name: &str) -> Vec<String> {
    let map = names(app_data, source, language);
    let wanted = name.trim();
    if wanted.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    if let Some(other) = map.get(wanted) {
        out.push(other.clone());
    }
    // Case matters less than the answer does, so try again without it, and
    // the other way round for a question asked in the game's language.
    let folded = wanted.to_lowercase();
    for (english, translated) in map.iter() {
        if out.len() >= 4 {
            break;
        }
        if english.to_lowercase() == folded {
            out.push(translated.clone());
        } else if translated.to_lowercase() == folded {
            out.push(english.clone());
        }
    }

    // Nothing under the whole name. A model asks by part of one as often as by
    // all of it — "Radagon" and then "Scarseal", never "Radagon's Scarseal" —
    // and the table is keyed by the full title, so a part matched nothing and
    // the question died. Take the titles that contain it as a word.
    if out.is_empty() && wanted.chars().count() >= 4 {
        for (english, translated) in map.iter() {
            if out.len() >= 4 {
                break;
            }
            let holds = |title: &str| {
                title
                    .to_lowercase()
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word == folded)
            };
            if holds(english) {
                out.push(translated.clone());
            } else if holds(translated) {
                out.push(english.clone());
            }
        }
    }

    out.retain(|other| !other.eq_ignore_ascii_case(wanted));
    out.dedup();
    out
}

/// Mirrors every article title, redirects included.
///
/// The redirects are the point of taking them. A wiki keeps its own list of the
/// other names for a thing — "Bleed", "Hemorrhage" and "Bleeding" all standing
/// for "Blood Loss" — and that list is exactly the table of synonyms this
/// launcher refuses to write by hand, kept up to date by people who play the
/// game. Without them a search for a bleed build found nothing at all, because
/// the game's own word for it is blood loss and nothing else was indexed.
///
/// Reading one costs nothing extra: the fetch already asks the wiki to follow
/// redirects, so opening "Bleed" returns the Blood Loss article.
pub async fn sync_titles(
    http: &reqwest::Client,
    app_data: &Path,
    source: &WikiSource,
    mut progress: impl FnMut(usize),
) -> Result<usize> {
    let folder = dir(app_data, source.id);
    std::fs::create_dir_all(&folder).at(&folder)?;

    let mut all: Vec<String> = Vec::new();
    let mut cont: Option<String> = None;

    loop {
        let mut url = format!(
            "{}?action=query&list=allpages&aplimit=500&apfilterredir=all&format=json&formatversion=2",
            source.api
        );
        if let Some(from) = &cont {
            url.push_str(&format!("&apcontinue={}", encode(from)));
        }

        let body = get_json(http, &url).await?;

        if let Some(pages) = body
            .pointer("/query/allpages")
            .and_then(serde_json::Value::as_array)
        {
            for page in pages {
                if let Some(title) = page.get("title").and_then(serde_json::Value::as_str) {
                    all.push(title.to_string());
                }
            }
        }
        progress(all.len());

        match body
            .pointer("/continue/apcontinue")
            .and_then(serde_json::Value::as_str)
        {
            Some(next) => cont = Some(next.to_string()),
            None => break,
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    all.sort();
    all.dedup();

    let path = index_path(app_data, source.id);
    let text = serde_json::to_string(&all).map_err(|e| Error::Parse {
        what: "titles".into(),
        detail: e.to_string(),
    })?;
    std::fs::write(&path, text).at(&path)?;

    Ok(all.len())
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// One article, from the cache when it is there and from the wiki when it is not.
pub async fn page(
    http: &reqwest::Client,
    app_data: &Path,
    source: &WikiSource,
    title: &str,
    refresh: bool,
) -> Result<WikiPage> {
    let path = page_path(app_data, source.id, title);

    if !refresh {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(page) = serde_json::from_str::<WikiPage>(&text) {
                return Ok(page);
            }
        }
    }

    let url = format!(
        "{}?action=parse&page={}&prop=text&redirects=1&format=json&formatversion=2",
        source.api,
        encode(title)
    );
    let body = get_json(http, &url).await?;

    if let Some(message) = body.pointer("/error/info").and_then(serde_json::Value::as_str) {
        return Err(Error::msg(message.to_string()));
    }

    let raw = body
        .pointer("/parse/text")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::msg(format!("{title} has no content")))?;
    let resolved = body
        .pointer("/parse/title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(title);

    let page = WikiPage {
        source: source.id.to_string(),
        title: resolved.to_string(),
        html: clean(raw, source),
        origin: format!("{}{}", source.article, encode(&resolved.replace(' ', "_"))),
    };

    let folder = dir(app_data, source.id);
    std::fs::create_dir_all(&folder).at(&folder)?;
    if let Ok(text) = serde_json::to_string(&page) {
        let _ = std::fs::write(&path, text);
    }

    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convergence() -> &'static WikiSource {
        source("convergence").unwrap()
    }

    #[test]
    fn the_duplicate_image_behind_noscript_is_dropped() {
        // Fandom ships every gallery image twice: once lazily, once inside a
        // `<noscript>` a scripted browser never renders. Keeping both doubles
        // the page for nothing.
        let html = concat!(
            r#"<div class="wikia-gallery-item">"#,
            r#"<noscript><img src="https://x/a.png" alt="dup"></noscript>"#,
            r#"<img class="lazyload" src="https://x/a.png" alt="real">"#,
            "</div>"
        );
        let out = clean(html, convergence());
        assert!(!out.contains("<noscript"), "got {out}");
        assert_eq!(out.matches("<img").count(), 1, "one image survives: {out}");
        assert!(out.contains(r#"alt="real""#), "and it is the visible one: {out}");
    }

    #[test]
    fn scripts_and_styles_never_reach_the_page() {
        let html = r#"<div>keep<script>alert(1)</script><style>body{}</style>me</div>"#;
        let out = clean(html, convergence());
        assert!(!out.contains("alert"));
        assert!(!out.contains("<style"));
        assert!(out.contains("keep"));
        assert!(out.contains("me"));
    }

    #[test]
    fn inline_handlers_are_stripped() {
        let html = r#"<a href="/wiki/X" onclick="steal()" onmouseover='x'>link</a>"#;
        let out = clean(html, convergence());
        assert!(!out.contains("onclick"), "got {out}");
        assert!(!out.contains("onmouseover"), "got {out}");
    }

    #[test]
    fn a_javascript_href_is_defused() {
        let out = clean(r#"<a href="javascript:alert(1)">x</a>"#, convergence());
        assert!(!out.contains("javascript:"), "got {out}");
    }

    #[test]
    fn links_to_other_articles_stay_inside_the_launcher() {
        let out = clean(r#"<a href="/wiki/Abyssal_Woods">there</a>"#, convergence());
        assert!(out.contains(r##"href="#wiki:convergence:Abyssal Woods""##), "got {out}");
        assert!(!out.contains("target="), "an internal link must not open a tab");
    }

    #[test]
    fn index_php_links_are_articles_too() {
        let out = clean(
            r#"<a href="/index.php?title=Spell_Runes&amp;action=view">x</a>"#,
            convergence(),
        );
        assert!(out.contains("#wiki:convergence:Spell Runes"), "got {out}");
    }

    #[test]
    fn percent_escapes_in_a_title_are_decoded() {
        assert_eq!(link_title("/wiki/Marika%27s_Scarseal").as_deref(), Some("Marika's Scarseal"));
        assert_eq!(link_title("/wiki/Abyssal_Woods").as_deref(), Some("Abyssal Woods"));
        assert_eq!(link_title("https://example.com/x"), None);
    }

    #[test]
    fn other_sites_open_in_a_tab_rather_than_the_launcher() {
        let out = clean(r#"<a href="https://example.com">out</a>"#, convergence());
        assert!(out.contains("target=\"_blank\""), "got {out}");
        assert!(out.contains("rel=\"noreferrer\""));
    }

    #[test]
    fn a_lazy_image_gets_its_real_address() {
        let html = r#"<img src="data:image/gif;base64,R0lGOD" data-src="https://static.wikia.nocookie.net/a.png" width="14">"#;
        let out = clean(html, convergence());
        assert!(out.contains("https://static.wikia.nocookie.net/a.png"), "got {out}");
        assert!(!out.contains("base64"), "the placeholder is still there: {out}");
        assert_eq!(
            out.matches("src=").count(),
            1,
            "one src, not two: {out}"
        );
    }

    #[test]
    fn deferred_loading_is_turned_off() {
        // The article sits in a container that may never scroll, and a lazy
        // image in one of those never loads at all.
        let out = clean(r#"<img src="/a.png" loading="lazy">"#, convergence());
        assert!(!out.contains("loading="), "got {out}");
    }

    #[test]
    fn content_the_wiki_hides_for_javascript_is_revealed() {
        let tabs = r#"<div class="wds-tab__content" hidden><p>second tab</p></div>"#;
        assert!(clean(tabs, convergence()).contains("second tab"));
        assert!(!clean(tabs, convergence()).contains("hidden"), "still hidden");

        let collapsed = r#"<div style="display:none;"><p>collapsed text</p></div>"#;
        let out = clean(collapsed, convergence());
        assert!(out.contains("collapsed text"));
        assert!(!out.contains("display"), "got {out}");

        let aria = r#"<section aria-hidden="true"><p>tab two</p></section>"#;
        assert!(!clean(aria, convergence()).contains("aria-hidden"));
    }

    #[test]
    fn a_hidden_attribute_is_not_confused_with_a_word_inside_one() {
        // `data-hidden-by` and a class mentioning the word must survive.
        let html = r#"<div class="not-hidden-really" data-hidden-by="x">text</div>"#;
        let out = clean(html, convergence());
        assert!(out.contains("not-hidden-really"), "got {out}");
        assert!(out.contains("data-hidden-by"), "got {out}");
    }

    #[test]
    fn relative_images_are_made_absolute() {
        let out = clean(r#"<img src="/images/a.png">"#, convergence());
        assert!(out.contains("https://wiki.convergencemod.com/images/a.png"), "got {out}");
        let protocol_relative = clean(r#"<img src="//static.example/a.png">"#, convergence());
        assert!(protocol_relative.contains("https://static.example/a.png"));
    }

    #[test]
    fn images_are_requested_without_a_referer() {
        // Fandom's CDN 404s when the Referer is an unknown host, and every
        // request from the launcher carries http://127.0.0.1:<port>.
        let out = clean(r#"<img src="https://static.example/a.png">"#, convergence());
        assert!(out.contains(r#"referrerpolicy="no-referrer""#), "got {out}");

        // Including the ones promoted out of data-src, which is most of them.
        let lazy = clean(
            r#"<img src="data:image/gif;base64,R0lGOD" data-src="https://static.example/b.png">"#,
            convergence(),
        );
        assert!(lazy.contains(r#"referrerpolicy="no-referrer""#), "got {lazy}");
        assert_eq!(lazy.matches("referrerpolicy").count(), 1, "once, not twice: {lazy}");
    }

    #[test]
    fn the_edit_control_beside_every_heading_is_removed() {
        // The real shape: two anchors, each holding its own spans. A lazy
        // pattern stops at the first inner close and leaves the rest behind.
        let html = concat!(
            r#"<h2>History</h2><span class="mw-editsection">"#,
            r#"<a role="button" href="/index.php?title=X&amp;veaction=edit">"#,
            r#"<span class="icon"></span><span>edit</span></a>"#,
            r#"<a href="/index.php?title=X&amp;action=edit"><span>src</span></a>"#,
            r#"</span><p>text</p>"#
        );
        let out = clean(html, convergence());
        assert!(!out.contains("mw-editsection"), "got {out}");
        assert!(!out.contains(">edit<"), "the control's label survived: {out}");
        assert!(out.contains("History"));
        assert!(out.contains("<p>text</p>"), "the article after it was eaten: {out}");
    }

    #[test]
    fn stripping_a_control_never_eats_the_article_after_it() {
        let opener = re(r#"(?is)<span[^>]*class="[^"]*kill[^"]*"[^>]*>"#);
        assert_eq!(strip_spans(r#"a<span class="kill">x</span>b"#, &opener), "ab");
        assert_eq!(
            strip_spans(r#"a<span class="kill"><span>x</span>y</span>b"#, &opener),
            "ab"
        );
        // Two of them, and text between.
        assert_eq!(
            strip_spans(r#"<span class="kill">1</span>mid<span class="kill">2</span>end"#, &opener),
            "midend"
        );
        // Multi-byte characters must not split a slice.
        assert_eq!(
            strip_spans(r#"«<span class="kill">…</span>»"#, &opener),
            "«»"
        );
    }

    /// The same thing under whichever name it was asked by.
    ///
    /// A model writes in the player's language and the game answers in the one
    /// it was installed in, so on a Russian installation "what does Reduvia do"
    /// used to come back empty on the first try — the game has never heard of
    /// Reduvia, only of Редувия.
    #[test]
    fn a_name_is_found_under_the_other_languages_spelling() {
        // Its own source name, because the loaded table is cached by source and
        // language and another test must not be handed this one.
        let source = "test-also-called";
        let app_data = std::env::temp_dir().join("roundtable-wiki-also-called");
        let dir = app_data.join("wiki").join(source);
        std::fs::create_dir_all(&dir).expect("a place to write the table");
        std::fs::write(
            dir.join("_names-ru.json"),
            r#"{"Reduvia":"Редувия","Church of Elleh":"Храм Элле"}"#,
        )
        .expect("the table is written");

        // English in, Russian out.
        assert_eq!(
            also_called(&app_data, source, "ru", "Reduvia"),
            vec!["Редувия".to_string()]
        );
        // And the other way, for a question asked in the game's own language.
        assert_eq!(
            also_called(&app_data, source, "ru", "Редувия"),
            vec!["Reduvia".to_string()]
        );
        // Case is not the point.
        assert_eq!(
            also_called(&app_data, source, "ru", "church of elleh"),
            vec!["Храм Элле".to_string()]
        );
        // Part of a name finds it too, because that is how it gets asked.
        assert_eq!(
            also_called(&app_data, source, "ru", "Reduvia"),
            vec!["Редувия".to_string()]
        );
        assert_eq!(
            also_called(&app_data, source, "ru", "Elleh"),
            vec!["Храм Элле".to_string()],
            "one word out of a title has to find it"
        );

        // Something with no other name gives none rather than a guess.
        assert!(also_called(&app_data, source, "ru", "Moonveil").is_empty());
        assert!(also_called(&app_data, source, "ru", "  ").is_empty());
        // And a language with no table at all is quiet.
        assert!(also_called(&app_data, source, "de", "Reduvia").is_empty());

        std::fs::remove_dir_all(&app_data).ok();
    }

    #[test]
    fn the_edition_picks_its_own_wiki() {
        assert_eq!(for_edition(Some("convergence")).id, "convergence");
        assert_eq!(for_edition(None).id, "eldenring");
        assert_eq!(for_edition(Some("nonsense")).id, "eldenring");
    }

    #[test]
    fn title_search_ranks_exact_then_prefix_then_contains() {
        let titles: Vec<String> = ["Moonveil Talisman", "The Moonveil", "Moonveil"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let hits = search(&titles, "moonveil", 10);
        assert_eq!(hits[0], "Moonveil");
        assert_eq!(hits[1], "Moonveil Talisman");
        assert_eq!(hits[2], "The Moonveil");
    }

    /// A name in a language that has capitals of its own.
    ///
    /// This lowercased with `to_ascii_lowercase`, which does nothing to
    /// Cyrillic — so on the Russian mirror a query written the way anybody
    /// writes a name, with a capital, matched nothing at all. Half of two of
    /// the five mirrors were unsearchable and it failed silently.
    #[test]
    fn a_query_is_found_whatever_case_its_alphabet_uses() {
        let titles: Vec<String> = ["Редувия", "Кинжал Редувия", "Мизерикордия"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();

        for written in ["Редувия", "редувия", "РЕДУВИЯ"] {
            let hits = search(&titles, written, 10);
            assert_eq!(hits.first().map(|t| t.as_str()), Some("Редувия"), "{written} found none");
            assert_eq!(hits.len(), 2, "{written} should also find the longer one");
        }
    }

    /// A question is words, not one string.
    ///
    /// Matching the query whole meant anything anybody would actually type
    /// found nothing: no title contains the phrase "radahn boss fight", so a
    /// perfectly clear question came back empty.
    #[test]
    fn a_phrase_finds_the_title_carrying_its_words() {
        let titles: Vec<String> = [
            "Starscourge Radahn",
            "Radahn Festival",
            "Boss Guide: Starscourge Radahn Fight",
            "Malenia",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();

        let hits = search(&titles, "radahn boss fight", 10);
        assert_eq!(
            hits.first().map(|t| t.as_str()),
            Some("Boss Guide: Starscourge Radahn Fight"),
            "the title carrying every word should lead: {hits:?}"
        );
        // The ones carrying only some still come back, behind it.
        assert!(hits.len() >= 3, "titles with some of the words were dropped: {hits:?}");
        assert!(!hits.iter().any(|t| *t == "Malenia"), "a title with none of them got in");

        // And a single word still behaves as it always did.
        let one = search(&titles, "radahn", 10);
        assert_eq!(one.first().map(|t| t.as_str()), Some("Radahn Festival"));
    }

    #[test]
    fn a_title_with_a_slash_still_gets_one_file() {
        let base = std::env::temp_dir();
        let a = page_path(&base, "convergence", "Weapons/Katana");
        let b = page_path(&base, "convergence", "weapons/katana");
        assert_eq!(a, b, "titles differing only in case share a page");
        assert!(!a.to_string_lossy().contains("Katana"), "the slash must not become a folder");
    }
}
