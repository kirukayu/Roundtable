//! The open web, for the questions the mirrors cannot answer.
//!
//! Almost everything the assistant needs is already on this machine — both
//! wikis, the game's own text, the tables the installation runs on. What is not
//! there is anything newer than the last mirror and anything nobody wrote a
//! wiki page about: a patch note from last week, a build somebody posted, an
//! argument about whether a boss is worth fighting early.
//!
//! A wiki page is edited by people who play the game; a search result is
//! whatever ranked. The two are not the same kind of source and the assistant
//! is told so.
//!
//! No key, which is the point: this repository is public and a key in it would
//! be a key given away. The cost is that this reads search engines' pages
//! rather than an API, so there are two of them and each is written to fail
//! into an empty list rather than into nonsense.

use std::sync::OnceLock;

use regex::Regex;

/// One result, as much as a search page gives.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub title: String,
    pub url: String,
    /// The engine's own summary, which is often enough to decide whether the
    /// page is worth opening.
    pub summary: String,
}

/// Long enough for a slow answer, short enough that a dead network does not
/// hold up a question the model could have answered without this.
///
/// It bounds one engine, and the engines are asked together, so it bounds the
/// whole search rather than each leg of it in turn.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(12);

/// Sent on every request. Without the full set DuckDuckGo answers 202 with a
/// challenge page instead of results — a user agent alone is not enough, and
/// the failure looks exactly like "the web knows nothing", which is the worst
/// way for this to break.
const BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                       (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// The engines, in the order they are asked.
///
/// Two rather than one because either can start refusing a plain HTTP client
/// on any given day, and a searchless assistant answers from memory, which is
/// the behaviour this whole module exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    DuckDuckGo,
    Mojeek,
}

impl Engine {
    const ALL: [Engine; 2] = [Engine::DuckDuckGo, Engine::Mojeek];

    fn url(self, query: &str) -> String {
        let query = urlencoding(query);
        match self {
            Engine::DuckDuckGo => format!("https://html.duckduckgo.com/html/?q={query}"),
            Engine::Mojeek => format!("https://www.mojeek.com/search?q={query}"),
        }
    }

    fn home(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => "https://html.duckduckgo.com/",
            Engine::Mojeek => "https://www.mojeek.com/",
        }
    }

    fn parse(self, body: &str, limit: usize) -> Vec<Found> {
        match self {
            Engine::DuckDuckGo => parse_duckduckgo(body, limit),
            Engine::Mojeek => parse_mojeek(body, limit),
        }
    }
}

/// Searches the web and returns what came back.
///
/// Every engine at once, first useful answer wins. They used to be asked in
/// turn, which meant a slow or blocked first engine was paid for in full —
/// twelve seconds of nothing — before the second was tried at all, and the
/// player sat through both. Asking together costs one more HTTP request and
/// bounds the wait at the fastest engine rather than the sum of the slow ones.
pub async fn search(http: &reqwest::Client, query: &str, limit: usize) -> Vec<Found> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut racing: futures_util::stream::FuturesUnordered<_> = Engine::ALL
        .into_iter()
        .map(|engine| async move { (engine, ask(http, engine, query, limit).await) })
        .collect();

    // The first that actually found something. An engine that answers instantly
    // with nothing must not beat one that answers in a second with results.
    use futures_util::StreamExt;
    while let Some((_, found)) = racing.next().await {
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// One engine, one question.
async fn ask(http: &reqwest::Client, engine: Engine, query: &str, limit: usize) -> Vec<Found> {
    let Ok(reply) = http
        .get(engine.url(query))
        .header("user-agent", BROWSER)
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("accept-language", "en-US,en;q=0.9")
        .header("referer", engine.home())
        .timeout(PATIENCE)
        .send()
        .await
    else {
        return Vec::new();
    };
    if !reply.status().is_success() {
        return Vec::new();
    }
    let Ok(body) = reply.text().await else {
        return Vec::new();
    };
    engine.parse(&body, limit)
}

/// Pulls the results out of a DuckDuckGo page.
///
/// Kept apart from the fetching so it can be tested against a saved page: this
/// is the part that will break when the engine changes its markup, and it
/// should break into an empty list rather than into nonsense.
pub fn parse_duckduckgo(body: &str, limit: usize) -> Vec<Found> {
    static RESULT: OnceLock<Regex> = OnceLock::new();
    static SUMMARY: OnceLock<Regex> = OnceLock::new();

    let result = RESULT.get_or_init(|| {
        Regex::new(r#"(?s)class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#)
            .expect("the result pattern compiles")
    });
    let summary = SUMMARY.get_or_init(|| {
        Regex::new(r#"(?s)class="result__snippet"[^>]*>(.*?)</a>"#)
            .expect("the summary pattern compiles")
    });

    let summaries: Vec<String> = summary
        .captures_iter(body)
        .map(|found| plain(&found[1]))
        .collect();

    gather(
        result.captures_iter(body).enumerate().map(|(at, found)| {
            (
                plain(&found[2]),
                unwrap_redirect(&found[1]),
                summaries.get(at).cloned().unwrap_or_default(),
            )
        }),
        limit,
    )
}

/// Pulls the results out of a Mojeek page.
pub fn parse_mojeek(body: &str, limit: usize) -> Vec<Found> {
    static RESULT: OnceLock<Regex> = OnceLock::new();

    // Title and summary sit in one list item, so they are taken together
    // rather than by position — a result with no summary cannot then shift
    // every summary after it onto the wrong title.
    let result = RESULT.get_or_init(|| {
        Regex::new(
            r#"(?s)<h2><a class="title"[^>]*href="([^"]+)"[^>]*>(.*?)</a></h2>(?:.*?<p class="s">(.*?)</p>)?"#,
        )
        .expect("the result pattern compiles")
    });

    gather(
        result.captures_iter(body).map(|found| {
            (
                plain(&found[2]),
                found[1].trim().to_string(),
                found.get(3).map(|s| plain(s.as_str())).unwrap_or_default(),
            )
        }),
        limit,
    )
}

/// The shared tail of both parsers: drop what is not a result, stop at the
/// limit.
fn gather(raw: impl Iterator<Item = (String, String, String)>, limit: usize) -> Vec<Found> {
    raw.filter_map(|(title, url, summary)| {
        if title.is_empty() || !url.starts_with("http") {
            return None;
        }
        Some(Found {
            title,
            url,
            summary,
        })
    })
    .take(limit)
    .collect()
}

/// How much of a page is read before it is treated as something other than an
/// article. Generous — a changelog is long — but finite.
const MOST: usize = 4 * 1024 * 1024;

/// Opens one page and gives back its markup with the unreadable parts cut out.
///
/// A search result is a title and a sentence; the answer is usually further in.
/// Without this the model can find the mod's own changelog and still be unable
/// to say what is in it, which is exactly the failure the search was added to
/// fix.
///
/// Returns the reason on failure rather than an empty string, because "the page
/// refused" and "the page said nothing" lead to different answers.
pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<String, String> {
    let url = url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Not a web address.".into());
    }

    match direct(http, url).await {
        // A page that came back but has almost nothing in it is a page whose
        // words are drawn by JavaScript. The fetch "worked" and the answer is
        // still empty, which reads to a model as a page that says nothing.
        Ok(html) if readable(&html).chars().count() > THIN => Ok(html),
        Ok(_) => through_a_reader(http, url).await.map_err(|_| {
            "The page came back nearly empty — its words are drawn by a script this cannot run."
                .to_string()
        }),
        Err(direct_said) => through_a_reader(http, url).await.map_err(|_| direct_said),
    }
}

/// Below this a page has not really been read.
///
/// Chosen against what real pages look like rather than picked: a wiki article
/// runs to tens of thousands of characters, and the shortest thing worth
/// quoting is a paragraph. Anything under this is a shell, a consent wall or a
/// challenge page.
const THIN: usize = 600;

/// The page as somebody else already rendered it.
///
/// A free, keyless reader that runs the scripts and hands back markdown. It is
/// the difference between "this page says nothing" and an answer, on every site
/// that draws its words client-side or refuses a plain HTTP client — and those
/// are most of the ones a player is sent to. Verified against a real article:
/// 119,000 characters where the direct fetch returns a shell.
///
/// Nothing is sent but the address, and no key is needed, which is what makes
/// it usable from a launcher whose source anybody can read.
async fn through_a_reader(http: &reqwest::Client, url: &str) -> Result<String, String> {
    // Deliberately NOT the browser user-agent every other request here sends.
    // The reader answers 403 to one — it is for programs, and a browser string
    // is how it tells the two apart. Measured: 403 with Chrome's, 200 and a
    // hundred and nineteen thousand characters with the launcher's own. The
    // client's default is used by saying nothing.
    let reply = http
        .get(format!("https://r.jina.ai/{url}"))
        .header("x-return-format", "markdown")
        .timeout(PATIENCE)
        .send()
        .await
        .map_err(|problem| format!("Could not be reached: {problem}"))?;
    if !reply.status().is_success() {
        return Err(format!("The page answered {}.", reply.status()));
    }
    let text = reply.text().await.map_err(|problem| problem.to_string())?;
    if text.chars().count() <= THIN {
        return Err("Nothing on the page.".into());
    }
    Ok(text)
}

async fn direct(http: &reqwest::Client, url: &str) -> Result<String, String> {
    let reply = http
        .get(url)
        .header("user-agent", BROWSER)
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("accept-language", "en-US,en;q=0.9")
        .timeout(PATIENCE)
        .send()
        .await
        .map_err(|problem| format!("Could not be reached: {problem}"))?;

    let status = reply.status();
    if !status.is_success() {
        return Err(format!("The page answered {status}."));
    }

    // A page, not a download. Without this a mislinked archive is read as
    // millions of characters of binary.
    let kind = reply
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !kind.is_empty() && !kind.contains("html") && !kind.contains("text") && !kind.contains("xml")
    {
        return Err(format!("Not a page to read ({kind})."));
    }

    let body = reply
        .text()
        .await
        .map_err(|problem| format!("Could not be read: {problem}"))?;
    let body: String = body.chars().take(MOST).collect();

    let readable = readable(&body);
    if readable.trim().is_empty() {
        return Err("The page had nothing readable in it.".into());
    }
    Ok(readable)
}

/// Throws away the parts of a page that are not prose.
///
/// Stripping tags alone is not enough on the open web the way it is on a wiki:
/// a script survives tag-stripping as a wall of code, and a page is mostly
/// script.
fn readable(html: &str) -> String {
    /// One pattern each, because this crate's regex has no backreference to
    /// say "the tag we just opened".
    const JUNK: [&str; 10] = [
        "script", "style", "noscript", "svg", "head", "nav", "footer", "form", "iframe", "template",
    ];
    static DROP: OnceLock<Vec<Regex>> = OnceLock::new();

    let drop = DROP.get_or_init(|| {
        JUNK.iter()
            .map(|tag| {
                Regex::new(&format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}\s*>"))
                    .expect("the pattern compiles")
            })
            .collect()
    });

    let mut left = html.to_string();
    // Twice: a nav inside a footer only disappears once its container is gone,
    // and one pass leaves the outer one behind.
    for _ in 0..2 {
        for pattern in drop {
            left = pattern.replace_all(&left, " ").into_owned();
        }
    }
    left
}

/// The real address behind an engine's own redirect.
fn unwrap_redirect(href: &str) -> String {
    let href = href.trim();
    let target = href
        .split(&['?', '&'][..])
        .find_map(|part| part.strip_prefix("uddg="))
        .map(undo_encoding);
    match target {
        Some(real) if real.starts_with("http") => real,
        _ if href.starts_with("//") => format!("https:{href}"),
        _ => href.to_string(),
    }
}

/// Markup and entities out, text left.
fn plain(html: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new("<[^>]*>").expect("the tag pattern compiles"));

    let bare = tag.replace_all(html, "");
    let bare = bare
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&rsaquo;", "›")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        // Last, so an escaped ampersand cannot become the start of another
        // entity that then gets replaced a second time.
        .replace("&amp;", "&");
    bare.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn urlencoding(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn undo_encoding(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' if at + 2 < bytes.len() => {
                let pair = std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or("");
                match u8::from_str_radix(pair, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        at += 3;
                    }
                    Err(_) => {
                        out.push(bytes[at]);
                        at += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                at += 1;
            }
            other => {
                out.push(other);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the reader gets a page the direct fetch cannot.
    ///
    /// `cargo test --lib show_reader -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, and it goes to the open web"]
    fn show_reader() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let http = reqwest::Client::builder()
            .user_agent("Roundtable/0.2.3")
            .build()
            .expect("a client");

        runtime.block_on(async {
            for url in [
                "https://en.wikipedia.org/wiki/Elden_Ring",
                "https://www.nexusmods.com/eldenring",
                "https://store.steampowered.com/app/1245620/ELDEN_RING/",
            ] {
                let began = std::time::Instant::now();
                let straight = direct(&http, url).await;
                let plain = straight.as_ref().map(|html| readable(html).chars().count());
                println!(
                    "\n  {url}\n    direct  {:>6} ms  {:?}",
                    began.elapsed().as_millis(),
                    plain.as_ref().map_err(|why| &why[..why.len().min(48)])
                );

                let began = std::time::Instant::now();
                let read = through_a_reader(&http, url).await;
                println!(
                    "    reader  {:>6} ms  {:?}",
                    began.elapsed().as_millis(),
                    read.as_ref()
                        .map(|text| text.chars().count())
                        .map_err(|why| &why[..why.len().min(48)])
                );
            }
        });
    }

    /// What each engine costs, and what racing them saves.
    ///
    /// `cargo test --lib show_search_speed -- --ignored --nocapture`
    #[test]
    #[ignore = "a probe, and it goes to the open web"]
    fn show_search_speed() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let http = reqwest::Client::builder()
            .user_agent("Roundtable/0.2.3")
            .build()
            .expect("a client");

        runtime.block_on(async {
            for query in [
                "seamless co-op elden ring password setup",
                "smithing stone farming route",
                "dark souls 3 mod engine 2 install",
            ] {
                println!("\n  {query:?}");
                // Raced FIRST, on cold connections, or it wins on a pool the
                // individual runs warmed up and the figure flatters itself.
                let began = std::time::Instant::now();
                let found = search(&http, query, 6).await;
                println!(
                    "    {:<12} {:>6} ms, {} results  <- all at once, cold",
                    "raced",
                    began.elapsed().as_millis(),
                    found.len()
                );
                for engine in Engine::ALL {
                    let began = std::time::Instant::now();
                    let found = ask(&http, engine, query, 6).await;
                    println!(
                        "    {:<12} {:>6} ms, {} results",
                        format!("{engine:?}"),
                        began.elapsed().as_millis(),
                        found.len()
                    );
                }
            }
        });
    }

    /// A page shaped the way DuckDuckGo writes one.
    const DUCKDUCKGO: &str = r##"
      <div class="result">
        <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Feldenring.fandom.com%2Fwiki%2FRellana&amp;rut=x">
          Rellana, Twin Moon &amp; Knight
        </a>
        <a class="result__snippet">She is the boss of <b>Castle Ensis</b>.</a>
      </div>
      <div class="result">
        <a class="result__a" href="https://example.com/plain">Plain link</a>
        <a class="result__snippet">Nothing much.</a>
      </div>
    "##;

    /// A page shaped the way Mojeek writes one, second result deliberately
    /// without a summary.
    const MOJEEK: &str = r##"
      <ul class="results-standard">
      <li class="r1"><a title="x" href="https://forum.example.com/t/patch-notes/1" class="ob"><p class="i"><span class="url">forum.example.com<span> &rsaquo; t</span></span></p></a><h2><a class="title" title="x" href="https://forum.example.com/t/patch-notes/1">The Convergence Update &amp; Patch Notes</a></h2><p class="s">Due to the size of the <strong>patch</strong> notes.</p></li>
      <li class="r2"><h2><a class="title" title="y" href="https://example.org/second">Second, no summary</a></h2></li>
      </ul>
    "##;

    #[test]
    fn results_come_back_as_addresses_rather_than_redirects() {
        let found = parse_duckduckgo(DUCKDUCKGO, 10);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].url, "https://eldenring.fandom.com/wiki/Rellana");
        assert_eq!(found[1].url, "https://example.com/plain");
    }

    #[test]
    fn titles_and_summaries_arrive_as_text() {
        let found = parse_duckduckgo(DUCKDUCKGO, 10);
        assert_eq!(found[0].title, "Rellana, Twin Moon & Knight");
        assert_eq!(found[0].summary, "She is the boss of Castle Ensis.");
        assert!(!found[0].summary.contains('<'));
    }

    #[test]
    fn the_second_engine_is_read_too() {
        let found = parse_mojeek(MOJEEK, 10);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].title, "The Convergence Update & Patch Notes");
        assert_eq!(found[0].url, "https://forum.example.com/t/patch-notes/1");
        assert_eq!(found[0].summary, "Due to the size of the patch notes.");
    }

    #[test]
    fn a_missing_summary_does_not_shift_the_others() {
        // The bug this shape of parser invites: pairing titles and summaries
        // by position, so one result without a summary borrows the next one's.
        let found = parse_mojeek(MOJEEK, 10);
        assert_eq!(found[1].title, "Second, no summary");
        assert_eq!(found[1].summary, "");
    }

    #[test]
    fn a_page_that_changed_shape_gives_nothing_rather_than_nonsense() {
        // The failure this will actually have one day. An empty list sends the
        // model back to the wikis; a list of junk sends it to a made-up answer.
        for engine in Engine::ALL {
            assert!(engine
                .parse("<html><body>consent wall</body></html>", 10)
                .is_empty());
            assert!(engine.parse("", 10).is_empty());
        }
        // A result with no address is not a result.
        assert!(parse_duckduckgo(r#"<a class="result__a" href="">Nothing</a>"#, 10).is_empty());
        assert!(parse_mojeek(r#"<h2><a class="title" href="">Nothing</a></h2>"#, 10).is_empty());
    }

    #[test]
    fn the_limit_is_respected() {
        assert_eq!(parse_duckduckgo(DUCKDUCKGO, 1).len(), 1);
        assert_eq!(parse_mojeek(MOJEEK, 1).len(), 1);
    }

    #[test]
    fn a_query_is_escaped_rather_than_pasted() {
        assert_eq!(urlencoding("blood initiate build"), "blood+initiate+build");
        assert_eq!(urlencoding("a&b=c"), "a%26b%3Dc");
        // Russian survives, since that is what half the questions are in.
        assert_eq!(
            urlencoding("Редувия"),
            "%D0%A0%D0%B5%D0%B4%D1%83%D0%B2%D0%B8%D1%8F"
        );
    }

    #[test]
    fn a_page_comes_back_without_its_machinery() {
        let page = "<html><head><title>x</title></head><body>\
                    <script>var a = 1; if (a < 2) { alert('not prose'); }</script>\
                    <style>.a { color: red }</style>\
                    <nav><a href=\"/\">Home</a></nav>\
                    <p>Patch 3.0.1 fixes the Church.</p>\
                    <footer>Copyright</footer></body></html>";
        let left = readable(page);
        assert!(left.contains("Patch 3.0.1 fixes the Church."), "{left}");
        for gone in ["alert", "color: red", "Home", "Copyright", "<title>"] {
            assert!(!left.contains(gone), "{gone} survived: {left}");
        }
    }

    #[test]
    fn a_nested_script_does_not_survive_its_container() {
        // One pass removes the inner block and leaves the outer tags; the
        // second removes the container. Without it a page's whole menu is read
        // as prose.
        let page = "<body><nav><script>junk()</script><a>Menu</a></nav><p>Real.</p></body>";
        let left = readable(page);
        assert!(left.contains("Real."), "{left}");
        assert!(!left.contains("Menu"), "{left}");
        assert!(!left.contains("junk"), "{left}");
    }

    #[test]
    fn both_engines_are_asked_at_a_real_address() {
        // A typo in one of these is a search that silently never works.
        assert!(Engine::DuckDuckGo.url("a b").ends_with("?q=a+b"));
        assert!(Engine::Mojeek.url("a b").ends_with("?q=a+b"));
        for engine in Engine::ALL {
            assert!(engine.url("x").starts_with("https://"));
            assert!(engine.home().starts_with("https://"));
        }
    }
}
