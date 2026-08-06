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

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
];

pub fn source(id: &str) -> Option<&'static WikiSource> {
    SOURCES.iter().find(|s| s.id == id)
}

/// The wiki that belongs to whichever edition is loaded.
pub fn for_edition(edition: Option<&str>) -> &'static WikiSource {
    match edition {
        Some("convergence") => &SOURCES[1],
        _ => &SOURCES[0],
    }
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

/// Ranked title search, same ordering rules as the codex.
pub fn search<'a>(titles: &'a [String], query: &str, limit: usize) -> Vec<&'a String> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return titles.iter().take(limit).collect();
    }

    let mut hits: Vec<(u8, &String)> = titles
        .iter()
        .filter_map(|title| {
            let lower = title.to_ascii_lowercase();
            if lower == needle {
                Some((0, title))
            } else if lower.starts_with(&needle) {
                Some((1, title))
            } else if lower.contains(&needle) {
                Some((2, title))
            } else {
                None
            }
        })
        .collect();

    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.len().cmp(&b.1.len())));
    hits.into_iter().take(limit).map(|(_, t)| t).collect()
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

/// Mirrors every article title. This is what makes search cover the whole wiki.
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
            "{}?action=query&list=allpages&aplimit=500&apfilterredir=nonredirects&format=json&formatversion=2",
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

    #[test]
    fn a_title_with_a_slash_still_gets_one_file() {
        let base = std::env::temp_dir();
        let a = page_path(&base, "convergence", "Weapons/Katana");
        let b = page_path(&base, "convergence", "weapons/katana");
        assert_eq!(a, b, "titles differing only in case share a page");
        assert!(!a.to_string_lossy().contains("Katana"), "the slash must not become a folder");
    }
}
