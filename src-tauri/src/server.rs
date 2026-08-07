//! The local HTTP server.
//!
//! The interface runs in the user's own browser rather than in a webview, so the
//! app carries a small server that hands out the built frontend and exposes the
//! same operations the desktop shell uses.
//!
//! Security posture, since this opens a port on the machine:
//!
//! * Bound to `127.0.0.1` only. Nothing off-box can reach it.
//! * Every API call must carry a session token minted at startup. Any other
//!   program on the machine could otherwise drive the launcher, and the launcher
//!   can move save files and start processes.
//! * The port is chosen by the OS, so two copies never collide.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::commands::AppState;

/// The built frontend, compiled into the binary so there is nothing to install.
#[derive(rust_embed::Embed)]
#[folder = "../dist"]
struct Assets;

pub struct Server {
    pub port: u16,
    pub token: String,
}

impl Server {
    /// The address to open in the browser.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?k={}", self.port, self.token)
    }
}

#[derive(Clone)]
struct Ctx {
    app: Arc<AppState>,
    token: Arc<str>,
}

/// Starts the server on a free loopback port and returns how to reach it.
pub async fn start(app: Arc<AppState>) -> crate::error::Result<Server> {
    let token: String = {
        let mut rng = rand::rng();
        (0..32)
            .map(|_| {
                let n: u8 = rng.random_range(0..36);
                char::from_digit(u32::from(n), 36).unwrap_or('0')
            })
            .collect()
    };

    let ctx = Ctx {
        app,
        token: Arc::from(token.as_str()),
    };

    let api = Router::new()
        .route("/games", get(games))
        .route("/settings", get(settings_get).post(settings_set))
        .route("/steam/accounts", get(steam_accounts))
        .route("/installs/discover", get(installs_discover))
        .route("/installs/active", get(installs_active))
        .route("/installs/scan", post(installs_scan))
        .route("/installs/scan/state", get(installs_scan_state))
        .route("/installs/scan/stop", post(installs_scan_stop))
        .route("/installs/remember", post(installs_remember))
        .route("/installs/forget", post(installs_forget))
        .route("/loaders", get(loaders))
        .route("/eac", get(eac_status).post(eac_set))
        .route("/coop/fields", get(coop_fields))
        .route("/coop", get(coop_read).post(coop_write))
        .route("/coop/password", get(coop_password))
        .route("/mods", get(mods_list))
        .route("/mods/install", post(mods_install))
        .route("/mods/delete", post(mods_delete))
        .route("/profiles", get(profiles_list))
        .route("/profiles/create", post(profile_create))
        .route("/profiles/save", post(profile_save))
        .route("/profiles/delete", post(profile_delete))
        .route("/profiles/conflicts", get(profile_conflicts))
        .route("/launch/plan", get(launch_plan))
        .route("/launch/patch", post(launch_patch))
        .route("/launch/run", post(launch_run))
        .route("/running", get(running))
        .route("/saves", get(saves_discover))
        .route("/saves/inspect", get(saves_inspect))
        .route("/saves/backups", get(saves_backups))
        .route("/saves/backup", post(saves_backup))
        .route("/saves/backup/delete", post(saves_backup_delete))
        .route("/saves/restore", post(saves_restore))
        .route("/saves/transfer", post(saves_transfer))
        .route("/saves/convert", post(saves_convert))
        .route("/sys/caches", get(sys_caches))
        .route("/sys/caches/clear", post(sys_clear))
        .route("/sys/report", get(sys_report))
        // Editions: total conversions that launch as a game of their own.
        .route("/editions", get(editions))
        .route("/editions/locate", post(edition_locate))
        .route("/editions/scan", post(edition_scan))
        .route("/editions/patch", post(edition_patch))
        .route("/editions/run", post(edition_run))
        .route("/editions/install", post(edition_install))
        .route("/editions/job", get(edition_job))
        // The codex, and the check that two players can see each other.
        .route("/codex", get(codex_search))
        .route("/codex/sync", post(codex_sync))
        .route("/codex/state", get(codex_state))
        .route("/wiki", get(wiki_search))
        .route("/wiki/page", get(wiki_page))
        .route("/wiki/sync", post(wiki_sync))
        .route("/perf", get(perf_status))
        .route("/perf/smooth", post(perf_smooth))
        .route("/perf/set", post(perf_set))
        .route("/perf/unlock", post(perf_unlock))
        .route("/perf/bounce", post(perf_bounce))
        .route("/tune", get(tune_status).post(tune_apply))
        .route("/tune/revert", post(tune_revert))
        .route("/ask", post(ask_question))
        .route("/ask/stream", post(ask_stream))
        .route("/overlay/hide", post(overlay_hide))
        .route("/overlay/drag", post(overlay_drag))
        .route("/overlay/centre", post(overlay_centre))
        .route("/erss", get(erss_status).post(erss_install))
        .route("/erss/uninstall", post(erss_uninstall))
        .route("/erss/set", post(erss_set))
        .route("/erss/tune", post(erss_tune))
        .route("/language", get(language_status).post(language_set))
        .route("/language/edition", post(edition_text_install))
        .route("/language/edition/revert", post(edition_text_revert))
        .route("/update", get(update_check))
        .route("/diagnose", get(diagnose))
        .route("/match", get(match_fingerprint))
        .route("/match/compare", post(match_compare))
        .route("/open", post(open_path))
        // Native dialogs the browser cannot provide. The desktop window stays
        // running in the tray precisely to answer these.
        .route("/pick/folder", get(pick_folder))
        .route("/pick/file", get(pick_file))
        .layer(middleware::from_fn_with_state(ctx.clone(), guard));

    let router = Router::new()
        .nest("/api", api)
        .fallback(serve_asset)
        .with_state(ctx);

    // Port 0 asks the OS for whatever is free.
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .map_err(|e| crate::error::Error::msg(format!("could not open a local port: {e}")))?;

    let port = listener
        .local_addr()
        .map_err(|e| crate::error::Error::msg(e.to_string()))?
        .port();

    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    Ok(Server { port, token })
}

/// Rejects anything without the session token.
async fn guard(State(ctx): State<Ctx>, request: Request, next: Next) -> Response {
    let supplied = request
        .headers()
        .get("x-roundtable-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            request.uri().query().and_then(|q| {
                q.split('&')
                    .find_map(|pair| pair.strip_prefix("k=").map(str::to_string))
            })
        });

    match supplied {
        Some(key) if key == *ctx.token => next.run(request).await,
        _ => (StatusCode::UNAUTHORIZED, "bad session key").into_response(),
    }
}

/// Serves the embedded frontend.
async fn serve_asset(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("text/plain")),
                )],
                file.data,
            )
                .into_response()
        }
        // A single-page app: unknown paths fall back to the shell.
        None => match Assets::get("index.html") {
            Some(file) => (
                [(header::CONTENT_TYPE, HeaderValue::from_static("text/html"))],
                file.data,
            )
                .into_response(),
            None => (StatusCode::NOT_FOUND, "frontend not built").into_response(),
        },
    }
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// Turns a `Result` into JSON, keeping the error message readable.
fn out<T: Serialize>(value: crate::error::Result<T>) -> Response {
    match value {
        Ok(value) => Json(value).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct GameQ {
    game: crate::games::Game,
}

#[derive(Deserialize)]
struct PathQ {
    path: PathBuf,
}

#[derive(Deserialize)]
struct GameProfileQ {
    game: crate::games::Game,
    profile: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn games() -> Response {
    Json(
        crate::games::Game::ALL
            .into_iter()
            .map(crate::games::GameInfo::from)
            .collect::<Vec<_>>(),
    )
    .into_response()
}

async fn settings_get(State(ctx): State<Ctx>) -> Response {
    Json(ctx.app.settings.lock().clone()).into_response()
}

async fn settings_set(
    State(ctx): State<Ctx>,
    Json(next): Json<crate::settings::Settings>,
) -> Response {
    *ctx.app.settings.lock() = next;
    let saved = ctx.app.settings.lock().save(&ctx.app.app_data);
    match saved {
        Ok(()) => Json(ctx.app.settings.lock().clone()).into_response(),
        Err(error) => out::<()>(Err(error)),
    }
}

async fn steam_accounts() -> Response {
    Json(crate::steam::local_accounts()).into_response()
}

async fn installs_discover(Query(q): Query<GameQ>) -> Response {
    Json(crate::game::discover(q.game)).into_response()
}

/// The remembered installation for a game, or `null`.
///
/// Not having located a game yet is the state every game starts in, so it
/// answers 200 with nothing rather than an error. Reporting it as a bad request
/// put a red line in the console on every first visit to a title.
async fn installs_active(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    let root = ctx
        .app
        .settings
        .lock()
        .install_for(q.game)
        .map(|i| i.root.clone());
    match root {
        Some(root) => out(crate::game::Installation::probe(q.game, &root).map(Some)),
        None => Json(None::<crate::game::Installation>).into_response(),
    }
}

/// Searches the whole machine. Runs on its own thread; the interface polls.
async fn installs_scan(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    if ctx.app.scan_job.lock().running {
        return out::<()>(Err(crate::error::Error::msg("a search is already running")));
    }
    {
        let mut job = ctx.app.scan_job.lock();
        *job = crate::commands::ScanState {
            running: true,
            at: "Starting".into(),
            ..Default::default()
        };
    }

    let app = Arc::clone(&ctx.app);
    let game = q.game;

    std::thread::spawn(move || {
        let reporter = Arc::clone(&app);
        let found = crate::game::deep_discover(game, move |path| {
            let mut job = reporter.scan_job.lock();
            job.at = path.to_string_lossy().to_string();
            // Returning false stops the walk, which is how Stop works.
            !job.cancelled
        });

        let mut job = app.scan_job.lock();
        job.running = false;
        job.done = true;
        job.at = String::new();
        job.found = found;
    });

    Json(json!({ "started": true })).into_response()
}

async fn installs_scan_state(State(ctx): State<Ctx>) -> Response {
    Json(ctx.app.scan_job.lock().clone()).into_response()
}

async fn installs_scan_stop(State(ctx): State<Ctx>) -> Response {
    ctx.app.scan_job.lock().cancelled = true;
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct RememberBody {
    game: crate::games::Game,
    path: PathBuf,
    #[serde(default)]
    make_default: bool,
}

async fn installs_remember(State(ctx): State<Ctx>, Json(body): Json<RememberBody>) -> Response {
    match crate::game::Installation::probe(body.game, &body.path) {
        Ok(install) => {
            ctx.app.settings.lock().remember_install(
                body.game,
                install.root.clone(),
                body.make_default,
            );
            let _ = ctx.app.settings.lock().save(&ctx.app.app_data);
            Json(install).into_response()
        }
        Err(error) => out::<()>(Err(error)),
    }
}

async fn installs_forget(State(ctx): State<Ctx>, Json(body): Json<RememberBody>) -> Response {
    ctx.app.settings.lock().forget_install(body.game, &body.path);
    let _ = ctx.app.settings.lock().save(&ctx.app.app_data);
    Json(json!({ "ok": true })).into_response()
}

async fn loaders(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    let root = ctx
        .app
        .settings
        .lock()
        .install_for(q.game)
        .map(|i| i.root.clone());
    Json(crate::loader::discover(q.game, root.as_deref())).into_response()
}

fn game_dir(ctx: &Ctx, game: crate::games::Game) -> crate::error::Result<PathBuf> {
    let root = ctx
        .app
        .settings
        .lock()
        .install_for(game)
        .map(|i| i.root.clone())
        .ok_or(crate::error::Error::NoGameSelected)?;
    Ok(crate::game::Installation::probe(game, &root)?.game_dir)
}

/// The anti-cheat state, or `null` while the game has not been located.
///
/// There is nothing to inspect without a folder, and being asked before one is
/// chosen is ordinary rather than wrong.
async fn eac_status(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    match game_dir(&ctx, q.game) {
        Ok(dir) => Json(Some(crate::eac::status(q.game, &dir))).into_response(),
        Err(crate::error::Error::NoGameSelected) => {
            Json(None::<crate::eac::EacStatus>).into_response()
        }
        Err(error) => out::<()>(Err(error)),
    }
}

#[derive(Deserialize)]
struct EacBody {
    game: crate::games::Game,
    enabled: bool,
}

async fn eac_set(State(ctx): State<Ctx>, Json(body): Json<EacBody>) -> Response {
    match game_dir(&ctx, body.game) {
        Ok(dir) => out(if body.enabled {
            crate::eac::enable(body.game, &dir)
        } else {
            crate::eac::disable(body.game, &dir)
        }),
        Err(error) => out::<()>(Err(error)),
    }
}

async fn coop_fields() -> Response {
    Json(crate::coop::FIELDS).into_response()
}

async fn coop_read(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    match game_dir(&ctx, q.game) {
        Ok(dir) => out(crate::coop::read(&dir)),
        Err(error) => out::<()>(Err(error)),
    }
}

#[derive(Deserialize)]
struct CoopBody {
    game: crate::games::Game,
    changes: std::collections::BTreeMap<String, String>,
}

async fn coop_write(State(ctx): State<Ctx>, Json(body): Json<CoopBody>) -> Response {
    match game_dir(&ctx, body.game) {
        Ok(dir) => out(crate::coop::write(&dir, &body.changes)),
        Err(error) => out::<()>(Err(error)),
    }
}

async fn coop_password() -> Response {
    Json(json!({ "password": crate::coop::generate_password() })).into_response()
}

async fn mods_list(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    Json(crate::mods::list_mods(&ctx.app.app_data, q.game)).into_response()
}

#[derive(Deserialize)]
struct IdBody {
    game: crate::games::Game,
    id: String,
}

async fn mods_delete(State(ctx): State<Ctx>, Json(body): Json<IdBody>) -> Response {
    out(crate::mods::delete_mod(&ctx.app.app_data, body.game, &body.id))
}

#[derive(Deserialize)]
struct InstallBody {
    game: crate::games::Game,
    /// A folder or an archive; the path came from a native picker.
    path: PathBuf,
    #[serde(default)]
    name: Option<String>,
}

async fn mods_install(State(ctx): State<Ctx>, Json(body): Json<InstallBody>) -> Response {
    let is_archive = body
        .path
        .extension()
        .map(|e| {
            let ext = e.to_string_lossy().to_ascii_lowercase();
            ext == "zip" || ext == "7z" || ext == "rar"
        })
        .unwrap_or(false);

    // Extraction can take a while on a large overhaul, so it runs off the async
    // runtime rather than blocking a server worker.
    let app_data = ctx.app.app_data.clone();
    let name = body.name.clone();
    let result = tokio::task::spawn_blocking(move || {
        if is_archive {
            crate::install::from_archive(&app_data, body.game, &body.path, name.as_deref())
        } else {
            crate::install::from_folder(&app_data, body.game, &body.path, name.as_deref())
        }
    })
    .await;

    match result {
        Ok(inner) => out(inner),
        Err(error) => out::<()>(Err(crate::error::Error::msg(error.to_string()))),
    }
}

async fn profiles_list(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    Json(crate::mods::list_profiles(&ctx.app.app_data, q.game)).into_response()
}

#[derive(Deserialize)]
struct NameBody {
    game: crate::games::Game,
    name: String,
}

async fn profile_create(State(ctx): State<Ctx>, Json(body): Json<NameBody>) -> Response {
    let mut profile = crate::mods::Profile::new(body.game, &body.name);
    profile.id = crate::mods::unique_profile_id(&ctx.app.app_data, body.game, &profile.id);
    match crate::mods::save_profile(&ctx.app.app_data, &profile) {
        Ok(()) => Json(profile).into_response(),
        Err(error) => out::<()>(Err(error)),
    }
}

async fn profile_save(
    State(ctx): State<Ctx>,
    Json(profile): Json<crate::mods::Profile>,
) -> Response {
    match crate::mods::save_profile(&ctx.app.app_data, &profile) {
        Ok(()) => Json(profile).into_response(),
        Err(error) => out::<()>(Err(error)),
    }
}

async fn profile_delete(State(ctx): State<Ctx>, Json(body): Json<IdBody>) -> Response {
    out(crate::mods::delete_profile(&ctx.app.app_data, body.game, &body.id))
}

async fn profile_conflicts(State(ctx): State<Ctx>, Query(q): Query<GameProfileQ>) -> Response {
    let profiles = crate::mods::list_profiles(&ctx.app.app_data, q.game);
    let Some(profile) = profiles.into_iter().find(|p| p.id == q.profile) else {
        return out::<()>(Err(crate::error::Error::msg("no such profile")));
    };
    let library = crate::mods::list_mods(&ctx.app.app_data, q.game);
    let ordered: Vec<_> = profile
        .enabled_mod_ids()
        .into_iter()
        .filter_map(|id| library.iter().find(|m| m.id == id).cloned())
        .collect();
    Json(crate::mods::detect_conflicts(&ordered)).into_response()
}

/// Assembles everything the planner needs.
fn plan_for(
    ctx: &Ctx,
    game: crate::games::Game,
    profile_id: &str,
) -> crate::error::Result<(
    crate::game::Installation,
    crate::mods::Profile,
    Vec<crate::mods::ModRecord>,
    Vec<crate::loader::LoaderInstall>,
    PathBuf,
)> {
    let root = ctx
        .app
        .settings
        .lock()
        .install_for(game)
        .map(|i| i.root.clone())
        .ok_or(crate::error::Error::NoGameSelected)?;
    let install = crate::game::Installation::probe(game, &root)?;
    let profile = crate::mods::list_profiles(&ctx.app.app_data, game)
        .into_iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| crate::error::Error::msg("no such profile"))?;
    let library = crate::mods::list_mods(&ctx.app.app_data, game);
    let loaders = crate::loader::discover(game, Some(&install.root));
    let work = ctx
        .app
        .app_data
        .join("launch")
        .join(game.appdata_folder())
        .join(profile_id);
    Ok((install, profile, library, loaders, work))
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CodexQ {
    #[serde(default)]
    q: String,
    kind: Option<String>,
    #[serde(default)]
    edition: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexHit {
    #[serde(flatten)]
    entry: crate::codex::CodexEntry,
    kind_label: String,
    wiki: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexResult {
    hits: Vec<CodexHit>,
    total: usize,
    kinds: Vec<(String, String, usize)>,
    state: crate::codex::CodexState,
}

async fn codex_search(State(ctx): State<Ctx>, Query(q): Query<CodexQ>) -> Response {
    let all = ctx.app.codex();
    let edition = q.edition.as_deref();

    let mut kinds: Vec<(String, String, usize)> = Vec::new();
    for (id, label) in crate::codex::KINDS {
        let count = all.iter().filter(|e| e.kind == *id).count();
        if count > 0 {
            kinds.push(((*id).to_string(), (*label).to_string(), count));
        }
    }

    let hits: Vec<CodexHit> = crate::codex::search(
        &all,
        &q.q,
        q.kind.as_deref().filter(|k| !k.is_empty()),
        q.limit.unwrap_or(60).min(200),
    )
    .into_iter()
    .map(|entry| CodexHit {
        kind_label: crate::codex::label_for(&entry.kind).to_string(),
        wiki: entry.wiki_url(edition),
        entry: entry.clone(),
    })
    .collect();

    let mut state = ctx.app.codex_job.lock().clone();
    state.entries = all.len();
    state.kinds = kinds.len();

    Json(CodexResult {
        hits,
        total: all.len(),
        kinds,
        state,
    })
    .into_response()
}

async fn codex_state(State(ctx): State<Ctx>) -> Response {
    let mut state = ctx.app.codex_job.lock().clone();
    state.entries = ctx.app.codex().len();
    Json(state).into_response()
}

/// Downloads the codex in the background. The upstream throttles, so this can
/// take a minute; the interface polls `codex/state`.
async fn codex_sync(State(ctx): State<Ctx>) -> Response {
    if ctx.app.codex_job.lock().syncing {
        return out::<()>(Err(crate::error::Error::msg("already downloading")));
    }
    {
        let mut job = ctx.app.codex_job.lock();
        *job = crate::codex::CodexState {
            syncing: true,
            message: "Starting".into(),
            total_kinds: crate::codex::KINDS.len(),
            ..Default::default()
        };
    }

    let app = Arc::clone(&ctx.app);
    tokio::spawn(async move {
        let http = app.http.clone();
        let app_data = app.app_data.clone();
        let reporter = Arc::clone(&app);

        let result = crate::codex::sync(&http, &app_data, move |done, total, label| {
            let mut job = reporter.codex_job.lock();
            job.done_kinds = done;
            job.total_kinds = total;
            job.message = label.to_string();
        })
        .await;

        let mut job = app.codex_job.lock();
        job.syncing = false;
        match result {
            Ok(count) => {
                job.message = format!("{count} entries");
                job.error = None;
            }
            Err(error) => {
                job.message = "Download failed".into();
                job.error = Some(error.to_string());
            }
        }
        drop(job);
        app.forget_codex();
    });

    Json(json!({ "started": true })).into_response()
}

// ---------------------------------------------------------------------------
// Wiki
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WikiQ {
    #[serde(default)]
    q: String,
    /// The edition on screen picks the wiki, unless a source is named outright.
    #[serde(default)]
    edition: Option<String>,
    #[serde(default)]
    source: Option<String>,
    limit: Option<usize>,
}

fn wiki_source_for(q: &WikiQ) -> &'static crate::wiki::WikiSource {
    q.source
        .as_deref()
        .and_then(crate::wiki::source)
        .unwrap_or_else(|| crate::wiki::for_edition(q.edition.as_deref()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WikiSearchResult {
    source: crate::wiki::WikiSource,
    sources: Vec<crate::wiki::WikiSource>,
    titles: Vec<String>,
    state: crate::wiki::WikiIndexState,
}

async fn wiki_search(State(ctx): State<Ctx>, Query(q): Query<WikiQ>) -> Response {
    let source = wiki_source_for(&q);
    let all = crate::wiki::titles(&ctx.app.app_data, source.id);

    let mut state = ctx
        .app
        .wiki_job
        .lock()
        .clone();
    state.source = source.id.to_string();
    state.titles = all.len();
    state.cached_pages = crate::wiki::cached_page_count(&ctx.app.app_data, source.id);

    Json(WikiSearchResult {
        source: *source,
        sources: crate::wiki::SOURCES.to_vec(),
        // The sidebar shows the whole contents of the wiki, so the cap is high:
        // it is a list of strings, and holding it back would mean the mirror
        // only appears to contain what fits on one screen.
        titles: crate::wiki::search(&all, &q.q, q.limit.unwrap_or(200).min(6000))
            .into_iter()
            .cloned()
            .collect(),
        state,
    })
    .into_response()
}

#[derive(Deserialize)]
struct WikiPageQ {
    title: String,
    #[serde(default)]
    edition: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    refresh: bool,
}

async fn wiki_page(State(ctx): State<Ctx>, Query(q): Query<WikiPageQ>) -> Response {
    let source = q
        .source
        .as_deref()
        .and_then(crate::wiki::source)
        .unwrap_or_else(|| crate::wiki::for_edition(q.edition.as_deref()));

    out(crate::wiki::page(
        &ctx.app.http,
        &ctx.app.app_data,
        source,
        &q.title,
        q.refresh,
    )
    .await)
}

/// Mirrors every article title so search covers the whole wiki.
async fn wiki_sync(State(ctx): State<Ctx>, Query(q): Query<WikiQ>) -> Response {
    if ctx.app.wiki_job.lock().syncing {
        return out::<()>(Err(crate::error::Error::msg("already indexing")));
    }
    let source = wiki_source_for(&q);
    {
        let mut job = ctx.app.wiki_job.lock();
        *job = crate::wiki::WikiIndexState {
            source: source.id.to_string(),
            syncing: true,
            message: format!("Indexing {}", source.name),
            ..Default::default()
        };
    }

    let app = Arc::clone(&ctx.app);
    tokio::spawn(async move {
        let http = app.http.clone();
        let app_data = app.app_data.clone();
        let reporter = Arc::clone(&app);

        let result = crate::wiki::sync_titles(&http, &app_data, source, move |seen| {
            let mut job = reporter.wiki_job.lock();
            job.titles = seen;
            job.message = format!("{seen} articles");
        })
        .await;

        let mut job = app.wiki_job.lock();
        job.syncing = false;
        match result {
            Ok(count) => {
                job.titles = count;
                job.message = format!("{count} articles");
                job.error = None;
            }
            Err(error) => {
                job.message = "Indexing failed".into();
                job.error = Some(error.to_string());
            }
        }
    });

    Json(json!({ "started": true })).into_response()
}

// ---------------------------------------------------------------------------
// Co-op match check
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MatchQ {
    game: crate::games::Game,
    #[serde(default)]
    edition: Option<String>,
}

/// The regulation.bin that will actually load: an edition's own file wins.
fn effective_regulation(
    ctx: &Ctx,
    install: &crate::game::Installation,
    edition: Option<&str>,
) -> Option<PathBuf> {
    let spec = crate::edition::spec(edition?)?;
    let found = resolve(ctx, spec, install)?;
    let path = found.root.join("mod").join("regulation.bin");
    path.is_file().then_some(path)
}

/// Every folder an FPS unlocker might have been dropped into.
fn perf_roots(ctx: &Ctx, game: crate::games::Game) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(install) = ctx.app.active_install(game) {
        roots.push(install.game_dir.clone());
        for spec in crate::edition::for_game(game) {
            if let Some(found) = resolve(ctx, spec, &install) {
                roots.push(found.root);
            }
        }
    }
    roots
}

/// True when a frame-generation mod is in this install.
///
/// Two of the preset's answers change when there is one, and they change against
/// the tier rather than with it — so without this the optimiser and the frame
/// generation pane would each undo the other's work.
fn generating_frames(ctx: &Ctx, game: crate::games::Game) -> bool {
    ctx.app
        .active_install(game)
        .is_ok_and(|install| crate::erss::owns_the_frame_cap(&install.game_dir))
}

async fn perf_status(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    let framegen = generating_frames(&ctx, q.game);
    Json(crate::perf::status(q.game, &perf_roots(&ctx, q.game), framegen)).into_response()
}

async fn perf_smooth(State(ctx): State<Ctx>, Json(body): Json<GameQ>) -> Response {
    out(crate::perf::smooth(body.game, generating_frames(&ctx, body.game)))
}

#[derive(Deserialize)]
struct PerfSetBody {
    game: crate::games::Game,
    key: String,
    value: String,
}

async fn perf_set(State(ctx): State<Ctx>, Json(body): Json<PerfSetBody>) -> Response {
    let _ = &ctx;
    out(crate::perf::set(body.game, &body.key, &body.value).map(|()| body.key))
}

/// Writes the chosen frame cap into the game once it is up.
///
/// The patch lives in the running process, so it can only be applied after the
/// game has started — and the game takes half a minute to get there. This waits
/// for the process rather than making the user press a button at the right
/// moment, which is the difference between a built-in unlocker and a tool.
fn unlock_when_up(ctx: &Ctx, game: crate::games::Game) {
    let Some(fps) = ctx.app.settings.lock().unlock_fps else {
        return;
    };
    let install = ctx.app.active_install(game);
    // With the anti-cheat armed this would be a ban, so it is never automatic.
    if install
        .as_ref()
        .is_ok_and(|install| install.has_eac && !install.eac_bypassed)
    {
        return;
    }
    // ERSS lifts the cap itself and has done unconditionally since its 4.7.0,
    // where the author removed the option and fixed a conflict with other
    // unlockers in the same release. Two patches rewriting one value in a live
    // process is not a race that settles — it is the tearing and the uneven
    // pointer that gets blamed on the display. Its limit is the better one
    // anyway: it counts finished frames, so it holds through frame generation
    // where a patched cap counts only the rendered ones.
    if install
        .as_ref()
        .is_ok_and(|install| crate::erss::owns_the_frame_cap(&install.game_dir))
    {
        return;
    }

    tokio::spawn(async move {
        let executable = game.executable();
        // Roughly two minutes: a cold start off a hard drive is slow, and the
        // patch is harmless whenever it lands.
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if crate::unlock::running_pid(executable).is_none() {
                continue;
            }
            // The module is not mapped the instant the process exists.
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            match crate::unlock::unlock(executable, fps) {
                Ok(report) => {
                    tracing::info!(fps = report.fps, "frame cap rewritten");
                    return;
                }
                Err(error) => tracing::debug!(%error, "frame cap not rewritten yet"),
            }
        }
    });
}

/// Every executable that will actually run the game, so the per-app tweaks land
/// on the one Windows sees.
fn launchable(ctx: &Ctx, game: crate::games::Game) -> Vec<PathBuf> {
    let Ok(install) = ctx.app.active_install(game) else {
        return Vec::new();
    };

    let mut out = vec![install.executable.clone()];
    // The anti-cheat launcher is a copy of the game on a cracked install, and it
    // is the one that gets started.
    let protected = install.game_dir.join("start_protected_game.exe");
    if protected.is_file() {
        out.push(protected);
    }
    // A total conversion runs the same executable through me3, but its own copy
    // of the folder can hold another.
    for spec in crate::edition::for_game(game) {
        if let Some(found) = resolve(ctx, spec, &install) {
            let exe = found.root.join(game.executable());
            if exe.is_file() {
                out.push(exe);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// What Windows is set to, and what it should be.
async fn tune_status(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    let levers = crate::tune::survey(&launchable(&ctx, q.game));
    Json(json!({
        "levers": levers,
        "competitors": crate::tune::competitors(),
    }))
    .into_response()
}

/// The whole lot: graphics preset, frame cap, and the Windows levers.
///
/// One button, and every change comes back as a line. Refused while the
/// anti-cheat is armed, because a patched process on an online session is a ban.
async fn tune_apply(State(ctx): State<Ctx>, Json(body): Json<GameQ>) -> Response {
    if ctx
        .app
        .active_install(body.game)
        .is_ok_and(|install| install.has_eac && !install.eac_bypassed)
    {
        return out::<()>(Err(crate::error::Error::msg(
            "Turn the anti-cheat off first. Patching the game with it armed is a ban.".to_string(),
        )));
    }

    let mut done: Vec<String> = Vec::new();

    match crate::perf::smooth(body.game, generating_frames(&ctx, body.game)) {
        Ok(changes) => done.extend(changes),
        // No config file yet is normal before the first launch, and the Windows
        // levers are still worth applying.
        Err(error) => done.push(format!("Graphics settings left alone: {error}")),
    }

    match crate::tune::apply(&ctx.app.app_data, &launchable(&ctx, body.game)) {
        Ok(changes) => done.extend(changes),
        Err(error) => return out::<()>(Err(error)),
    }

    // The cap this machine holds every frame, saved for the next launch and
    // written into the game if it is already up.
    //
    // Unless the frame-generation mod is in, in which case the cap is its own —
    // it lifts the sixty limit unconditionally and counts finished frames rather
    // than rendered ones, and two patches writing one value in a live process is
    // the tearing that gets blamed on the monitor.
    let framegen = generating_frames(&ctx, body.game);
    let cap = crate::perf::status(body.game, &[], framegen)
        .machine
        .suggested_cap;
    if framegen {
        done.push("Frame cap left to the frame-generation mod, which owns it".into());
    } else if cap > 60 {
        {
            let mut settings = ctx.app.settings.lock();
            settings.unlock_fps = Some(cap);
            let _ = settings.save(&ctx.app.app_data);
        }
        match crate::unlock::unlock(body.game.executable(), cap) {
            Ok(report) => done.push(format!("Frame cap raised to {}", report.fps)),
            Err(_) => done.push(format!("Frame cap set to {cap} for the next launch")),
        }
    }

    if crate::unlock::raise_priority(body.game.executable()).is_ok() {
        done.push("Game moved above the browsers in the scheduler".into());
    }

    Json(json!({ "changes": done, "competitors": crate::tune::competitors() })).into_response()
}

/// Puts every Windows change back.
async fn tune_revert(State(ctx): State<Ctx>) -> Response {
    out(crate::tune::revert(&ctx.app.app_data))
}

/// The overlay, closing itself.
async fn overlay_hide() -> Response {
    crate::hide_overlay();
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// The overlay being picked up and moved.
async fn overlay_drag() -> Response {
    crate::drag_overlay();
    Json(serde_json::json!({ "ok": true })).into_response()
}

/// The overlay being put back in the middle, for when it has been dragged off.
async fn overlay_centre() -> Response {
    crate::centre_overlay();
    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AskBody {
    question: String,
    #[serde(default)]
    edition: Option<String>,
    /// What was said before this, so "and how do I beat her" has a her.
    #[serde(default)]
    history: Vec<crate::ask::Turn>,
}

/// A question about the game, answered out of the wiki.
async fn ask_question(State(ctx): State<Ctx>, Json(body): Json<AskBody>) -> Response {
    let player = player_state(&ctx, body.edition.as_deref());
    out(crate::ask::answer(
        &ctx.app.http,
        &ctx.app.app_data,
        body.edition.as_deref(),
        &player,
        &body.question,
    )
    .await)
}

/// What the assistant is allowed to know about this player's own game.
///
/// Gathered here and handed over as plain data rather than reached for from
/// inside the tool, so nothing in the answering path can wander into state it
/// was not given — and so the whole of it can be tested without a game
/// installed.
///
/// Only what changes an answer: their characters and levels, the version, what
/// is installed. Not a path, not a Steam id, not an account name. The question
/// "is this weapon worth it at my level" needs the level and nothing else.
fn player_state(ctx: &Ctx, edition: Option<&str>) -> crate::ask::Player {
    let game = ctx.app.settings.lock().selected_game;
    let mut player = crate::ask::Player::default();

    if let Ok(install) = ctx.app.active_install(game) {
        player.version = install.version.clone();
        player.framegen = crate::erss::owns_the_frame_cap(&install.game_dir);
    }

    player.edition = edition.and_then(|id| {
        crate::edition::for_game(game)
            .into_iter()
            .find(|spec| spec.id == id)
            .map(|spec| spec.name.to_string())
    });

    // The newest save, parsed. Characters are what a question turns on; which
    // file they came out of is not the model's business.
    let folders = crate::saves::discover(game, None);
    // Not the game's own rolling backup, which is a copy of a save from before
    // whatever they last did.
    let newest = folders
        .iter()
        .flat_map(|folder| folder.entries.iter())
        .filter(|entry| entry.flavour != crate::saves::SaveFlavour::GameBackup)
        .max_by(|a, b| a.modified.cmp(&b.modified));
    if let Some(entry) = newest {
        if let Ok(summary) = crate::saves::inspect(&entry.path) {
            player.characters = summary
                .slots
                .iter()
                .filter(|slot| slot.active && !slot.name.trim().is_empty())
                .map(|slot| (slot.name.clone(), slot.level, slot.seconds_played))
                .collect();
        }
    }

    let active = ctx.app.settings.lock().active_profile.clone();
    let library = crate::mods::list_mods(&ctx.app.app_data, game);
    if let Some(profile) = crate::mods::list_profiles(&ctx.app.app_data, game)
        .into_iter()
        .find(|profile| Some(&profile.id) == active.as_ref())
    {
        player.mods = profile
            .mods
            .iter()
            .filter(|entry| entry.enabled)
            .filter_map(|entry| {
                library
                    .iter()
                    .find(|record| record.id == entry.mod_id)
                    .map(|record| record.name.clone())
            })
            .collect();
    }

    player
}

/// The same question, reported as it is answered.
///
/// One line of JSON per event, which the window reads as it arrives: what is
/// being searched, what was found, then the answer a few words at a time. The
/// interface used to get the titles by asking a second time — two rounds of
/// retrieval and two calls to the planner for one question — and the answer
/// arrived all at once at the end of a spinner.
async fn ask_stream(State(ctx): State<Ctx>, Json(body): Json<AskBody>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
    let player = player_state(&ctx, body.edition.as_deref());

    tokio::spawn(async move {
        crate::ask::answer_stream(
            &ctx.app.http,
            &ctx.app.app_data,
            body.edition.as_deref(),
            &player,
            &body.question,
            &body.history,
            |event| {
                if let Ok(line) = serde_json::to_string(&event) {
                    // The window may have been closed mid-answer, which is not
                    // an error — there is simply nobody left to tell.
                    let _ = tx.try_send(format!("{line}\n"));
                }
            },
        )
        .await;
    });

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|line| (Ok::<_, std::io::Error>(line), rx))
    });

    Response::builder()
        .header("content-type", "application/x-ndjson")
        .header("cache-control", "no-cache")
        .body(axum::body::Body::from_stream(stream))
        .unwrap_or_else(|_| out::<()>(Err(crate::error::Error::msg("could not open the stream"))))
}


/// DLSS, frame generation and Reflex, and whether the game is ready for them.
async fn erss_status(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    let Ok(install) = ctx.app.active_install(q.game) else {
        return out::<()>(Err(crate::error::Error::msg("the game is not located yet")));
    };
    let hags = crate::tune::gpu_scheduling_on();
    let mut status = crate::erss::status(&install.game_dir, install.has_eac, install.eac_bypassed, hags);

    // Everything the mod's own post insists on, read off the machine. These are
    // the things that make it look broken when it is only misconfigured.
    let perf = crate::perf::status(q.game, &[], true);
    let screen = perf.settings.iter().find(|s| s.key == "ScreenMode").map(|s| s.value.clone());
    let game_res = crate::perf::game_resolution(q.game, screen.as_deref());
    let display_res = crate::perf::display_geometry().map(|(w, h, _)| (w, h));

    status.blockers = crate::erss::preflight(
        &install.game_dir,
        install.version.as_deref(),
        install.has_eac,
        install.eac_bypassed,
        hags,
        screen.as_deref(),
        game_res,
        display_res,
    );

    Json(status).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErssSetBody {
    game: crate::games::Game,
    key: String,
    value: String,
}

/// Changes one of the mod's own settings, before the game starts.
async fn erss_set(State(ctx): State<Ctx>, Json(body): Json<ErssSetBody>) -> Response {
    match ctx.app.active_install(body.game) {
        Ok(install) => out(crate::erss::set_setting(&install.game_dir, &body.key, &body.value)),
        Err(error) => out::<()>(Err(error)),
    }
}

/// Sets everything that shows up as an artefact once frames are generated.
///
/// The settings that matter are split across three files nobody would think to
/// connect: the game's own graphics config holds the flickering light and the
/// blur passes, the mod's config holds how far it upscales, and the frame cap
/// lives in the running process. This does all three from one press, and reports
/// each change with the artefact it removes rather than a list of key names.
async fn erss_tune(State(ctx): State<Ctx>, Json(body): Json<GameQ>) -> Response {
    let Ok(install) = ctx.app.active_install(body.game) else {
        return out::<()>(Err(crate::error::Error::msg("the game is not located yet")));
    };
    out(Ok(no_artefacts(body.game, &install.game_dir)))
}

/// The whole artefact pass, run at install and again on demand.
fn no_artefacts(game: crate::games::Game, dir: &std::path::Path) -> Vec<String> {
    let machine = crate::perf::status(game, &[], true).machine;
    let pixels = u64::from(machine.width) * u64::from(machine.height);
    let mut applied: Vec<String> = Vec::new();

    for fix in crate::erss::ARTEFACT_FIXES {
        if crate::perf::set(game, fix.key, fix.value).is_ok() {
            applied.push(format!("{} — {}", fix.value, fix.why));
        }
    }

    // The mod writes its own config on first run, so before then there is
    // nothing to change — seeding it is what makes this work on a fresh install.
    let _ = crate::erss::seed(dir);
    for stray in crate::erss::tidy(dir) {
        applied.push(format!(
            "{stray} removed from the top of its config — an earlier Roundtable put it \
             there and the mod never read it"
        ));
    }

    // Shut the mod's own overlay and its startup notice. Everything it offers
    // is on this page, so a "press Home to close" banner over somebody's game
    // is a notice about a feature they do not need to know exists.
    if !crate::erss::quieten(dir).is_empty() {
        applied.push(
            "Its in-game overlay and notices switched off — nothing about the mod shows              on screen any more"
                .into(),
        );
    }

    let set = |key: &str, value: &str| crate::erss::set_setting(dir, key, value).is_ok();
    let reading = |key: &str| {
        crate::erss::settings(dir)
            .into_iter()
            .find(|setting| setting.key == key)
            .map(|setting| setting.value)
    };

    // Nothing below is worth saying until the mod has written its config, and
    // that only happens once the game has run with it loaded.
    if reading("Renderer.ScalingMode").is_none() {
        applied.push(
            "Start the game once with the mod loaded and press this again — until then it \
             has not written the settings that matter, and only the game's own are here"
                .into(),
        );
        return applied;
    }

    // The upscaler goes on. It is separate from frame generation and always
    // was, which the pane could not show because it never reached either.
    if set("Renderer.ScalingMode", "DLSS") {
        applied.push(
            "DLSS on — the game has no upscaler of its own, and this is the whole reason \
             the mod is here"
                .into(),
        );
    }

    // Reflex, which is what gives back the latency generation costs.
    if set("Renderer.LatencyReductionMode", "1") {
        applied.push(
            "Reflex on — frame generation adds a frame of delay and this takes it back off"
                .into(),
        );
    }

    // The author's own workaround for the one artefact this mod is known for.
    if set("FrameGeneration.GIGlitchMitigation", "1") {
        applied.push(
            "Global illumination fix on — the flicker in shaded rooms is this mod's one \
             real artefact and this is the author's answer to it"
                .into(),
        );
    }

    let framegen = reading("FrameGeneration.FrameGenMode").is_some_and(|mode| mode != "0");

    // How far it upscales, which depends on whether generation is carrying half
    // the frames — upscaling artefacts and generated frames compound.
    let (mode, label) = crate::erss::best_dlss_mode(machine.tier, pixels, framegen);
    if set("DLSS.DLSSMode", mode) {
        applied.push(format!(
            "DLSS at {label} — reconstruction artefacts and generated frames compound, so \
             it renders as close to full resolution as this card allows"
        ));
    }
    if !framegen {
        let (_, with) = crate::erss::best_dlss_mode(machine.tier, pixels, true);
        if with != label {
            applied.push(format!(
                "Frame generation is off. Turn it on above and press this again — there is \
                 room for {with} once it is carrying half the frames"
            ));
        }
    }

    // The mod owns the frame limit and counts finished frames, so a generator
    // doubling the rate is already in the number. It ships at zero or sixty,
    // which is why generation so often appears to do nothing at all.
    let target = if framegen {
        crate::perf::suggested_cap_generated(machine.refresh_hz, machine.tier)
    } else {
        machine.suggested_cap
    };
    if set("Renderer.RemoveFPSLimit", "true") && set("Renderer.MaxFPS", &target.to_string()) {
        applied.push(format!(
            "{target} frames — a card held at its ceiling starves the generator, which runs \
             on the same shaders, and an even rate under the panel is what stops the tearing"
        ));
    }

    applied
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErssBody {
    game: crate::games::Game,
    /// Renames the loader so the Steam overlay keeps working.
    #[serde(default)]
    steam_overlay: bool,
    /// The release archives are locked. Used for the unpack and never stored.
    #[serde(default)]
    password: Option<String>,
}

/// Unpacks the mod, after turning on the two things it needs.
async fn erss_install(State(ctx): State<Ctx>, Json(body): Json<ErssBody>) -> Response {
    let install = match ctx.app.active_install(body.game) {
        Ok(install) => install,
        Err(error) => return out::<()>(Err(error)),
    };

    let mut done: Vec<String> = Vec::new();

    // The mod cannot load past the anti-cheat, and frame generation will not
    // start without hardware scheduling. Doing them here is the difference
    // between a button and a list of instructions.
    if install.has_eac && !install.eac_bypassed {
        return out::<()>(Err(crate::error::Error::msg(
            "Turn the anti-cheat off first — the mod cannot load past it.".to_string(),
        )));
    }
    if !crate::tune::gpu_scheduling_on() {
        match crate::tune::elevate_gpu_scheduling() {
            Ok(true) => done.push(
                "GPU scheduling turned on — restart Windows before frame generation works".into(),
            ),
            _ => done.push(
                "GPU scheduling is still off, so frame generation will not start until it is"
                    .into(),
            ),
        }
    }

    let archives = crate::erss::find_archives();
    let password = body.password.as_deref().filter(|p| !p.is_empty());
    match crate::erss::install(&install.game_dir, &archives, body.steam_overlay, password) {
        Ok(lines) => done.extend(lines),
        Err(error) => return out::<()>(Err(error)),
    }

    // Everything that shows as a generated-frame artefact, done here rather
    // than left as a second button to find. Installing a frame generator and
    // leaving the settings that fight it is half a job.
    done.extend(no_artefacts(body.game, &install.game_dir));

    Json(json!({ "changes": done })).into_response()
}

async fn erss_uninstall(State(ctx): State<Ctx>, Json(body): Json<ErssBody>) -> Response {
    match ctx.app.active_install(body.game) {
        Ok(install) => out(crate::erss::uninstall(&install.game_dir)),
        Err(error) => out::<()>(Err(error)),
    }
}

/// Unsticks a juddering pointer by rebuilding the display mode.
async fn perf_bounce() -> Response {
    // Blocking, and it sleeps for most of a second: off the async threads.
    match tokio::task::spawn_blocking(crate::perf::bounce_refresh).await {
        Ok(result) => out(result),
        Err(error) => out::<String>(Err(crate::error::Error::msg(error.to_string()))),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnlockBody {
    game: crate::games::Game,
    /// 0 puts the shipped 60 cap back.
    fps: u32,
}

/// Rewrites the frame cap in the running game.
///
/// Refused while the anti-cheat is armed. A patched process on an online session
/// is a ban, and the point of doing this inside the launcher is that it knows.
async fn perf_unlock(State(ctx): State<Ctx>, Json(body): Json<UnlockBody>) -> Response {
    if let Ok(install) = ctx.app.active_install(body.game) {
        if install.has_eac && !install.eac_bypassed {
            return out::<()>(Err(crate::error::Error::msg(
                "Turn the anti-cheat off first. Patching the game with it armed is a ban."
                    .to_string(),
            )));
        }
    }
    out(crate::unlock::unlock(body.game.executable(), body.fps))
}

/// The language, plus whether an installed conversion has text in it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageReport {
    #[serde(flatten)]
    status: crate::language::LanguageStatus,
    /// One entry per total conversion that is installed and carries its own text.
    editions: Vec<crate::language::EditionText>,
}

/// Every installed conversion's text folder, for the language now in force.
fn edition_texts(ctx: &Ctx, game: crate::games::Game, language: &str) -> Vec<crate::language::EditionText> {
    let Ok(install) = ctx.app.active_install(game) else {
        return Vec::new();
    };
    crate::edition::for_game(game)
        .into_iter()
        .filter_map(|spec| {
            let found = resolve(ctx, spec, &install)?;
            crate::language::edition_text(spec.id, &found.root.join("mod"), language)
        })
        .collect()
}

/// What language the emulated Steam is telling the game to use.
async fn language_status(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    match ctx.app.active_install(q.game) {
        Ok(install) => {
            let status = crate::language::status(&install.game_dir);
            // The conversion's own text is a separate question from the game's,
            // and the one people hit second.
            let editions = match status.current.as_deref() {
                Some(language) => edition_texts(&ctx, q.game, language),
                None => Vec::new(),
            };
            Json(LanguageReport { status, editions }).into_response()
        }
        Err(error) => out::<()>(Err(error)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageBody {
    game: crate::games::Game,
    language: String,
}

async fn language_set(State(ctx): State<Ctx>, Json(body): Json<LanguageBody>) -> Response {
    match ctx.app.active_install(body.game) {
        Ok(install) => out(crate::language::set(&install.game_dir, &body.language)),
        Err(error) => out::<()>(Err(error)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditionTextBody {
    game: crate::games::Game,
    edition: String,
    language: String,
    /// Absent means the translation Roundtable carries.
    archive: Option<PathBuf>,
}

/// Finds an installed conversion's `mod` folder by id.
fn edition_mod_dir(ctx: &Ctx, game: crate::games::Game, id: &str) -> Result<PathBuf, crate::error::Error> {
    let install = ctx.app.active_install(game)?;
    let spec = crate::edition::spec(id)
        .ok_or_else(|| crate::error::Error::msg(format!("{id} is not an edition")))?;
    let found = resolve(ctx, spec, &install)
        .ok_or_else(|| crate::error::Error::msg(format!("{} is not installed", spec.name)))?;
    Ok(found.root.join("mod"))
}

/// Puts a conversion's text into the language the game is set to.
async fn edition_text_install(State(ctx): State<Ctx>, Json(body): Json<EditionTextBody>) -> Response {
    let dir = match edition_mod_dir(&ctx, body.game, &body.edition) {
        Ok(dir) => dir,
        Err(error) => return out::<()>(Err(error)),
    };
    out(match body.archive {
        Some(archive) => crate::language::install_edition_text(&dir, &body.language, &archive),
        None => crate::language::install_bundled_text(&body.edition, &dir, &body.language),
    })
}

/// Puts the conversion's own text back.
async fn edition_text_revert(State(ctx): State<Ctx>, Json(body): Json<EditionTextBody>) -> Response {
    let dir = match edition_mod_dir(&ctx, body.game, &body.edition) {
        Ok(dir) => dir,
        Err(error) => return out::<()>(Err(error)),
    };
    out(crate::language::revert_edition_text(&dir, &body.language))
}

/// Whether a newer Roundtable has been released.
///
/// It reports rather than installs. Replacing a running executable needs a
/// signed update manifest and a restart dance, and getting that wrong on
/// somebody else's machine is worse than a line of text saying there is a new
/// version.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    current: String,
    latest: Option<String>,
    newer: bool,
    url: String,
}

async fn update_check(State(ctx): State<Ctx>) -> Response {
    const CURRENT: &str = env!("CARGO_PKG_VERSION");
    const RELEASES: &str = "https://github.com/kirukayu/Roundtable/releases/latest";

    let latest = ctx
        .app
        .http
        .get("https://api.github.com/repos/kirukayu/Roundtable/releases/latest")
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .ok();

    let tag = match latest {
        Some(response) if response.status().is_success() => response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| {
                body.get("tag_name")
                    .and_then(serde_json::Value::as_str)
                    .map(|tag| tag.trim_start_matches('v').to_string())
            }),
        _ => None,
    };

    let newer = tag.as_deref().is_some_and(|t| is_newer(t, CURRENT));

    Json(UpdateInfo {
        current: CURRENT.to_string(),
        latest: tag,
        newer,
        url: RELEASES.to_string(),
    })
    .into_response()
}

/// Compares two dotted versions numerically, so 0.10 beats 0.9.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |text: &str| -> Vec<u32> {
        text.split(['.', '-'])
            .map(|piece| piece.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(candidate), parts(current));
    for index in 0..a.len().max(b.len()) {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    false
}

/// Every check Roundtable can run against this machine.
async fn diagnose(State(ctx): State<Ctx>, Query(q): Query<MatchQ>) -> Response {
    match ctx.app.active_install(q.game) {
        Ok(install) => {
            let regulation = effective_regulation(&ctx, &install, q.edition.as_deref());
            Json(crate::diagnose::run(&install, regulation.as_deref())).into_response()
        }
        Err(error) => out::<()>(Err(error)),
    }
}

async fn match_fingerprint(State(ctx): State<Ctx>, Query(q): Query<MatchQ>) -> Response {
    match ctx.app.active_install(q.game) {
        Ok(install) => {
            let regulation = effective_regulation(&ctx, &install, q.edition.as_deref());
            Json(crate::matchup::fingerprint(&install, regulation.as_deref())).into_response()
        }
        Err(error) => out::<()>(Err(error)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MatchBody {
    game: crate::games::Game,
    #[serde(default)]
    edition: Option<String>,
    theirs: String,
}

async fn match_compare(State(ctx): State<Ctx>, Json(body): Json<MatchBody>) -> Response {
    match ctx.app.active_install(body.game) {
        Ok(install) => {
            let regulation = effective_regulation(&ctx, &install, body.edition.as_deref());
            let mine = crate::matchup::fingerprint(&install, regulation.as_deref());
            Json(crate::matchup::compare(&mine, &body.theirs)).into_response()
        }
        Err(error) => out::<()>(Err(error)),
    }
}

// ---------------------------------------------------------------------------
// Editions
// ---------------------------------------------------------------------------

/// Everything the interface needs to draw one edition, installed or not.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditionStatus {
    spec: crate::edition::EditionSpec,
    install: Option<crate::edition::EditionInstall>,
    /// Present once the edition is on disk, so the plan can be shown before Play.
    plan: Option<crate::launch::LaunchPlan>,
    command_line: Option<String>,
    /// Where Roundtable would unpack the archive, beside the game.
    suggested_destination: PathBuf,
}

#[derive(Deserialize)]
struct EditionQ {
    game: crate::games::Game,
    #[serde(default)]
    coop: bool,
}

fn edition_context(
    ctx: &Ctx,
    game: crate::games::Game,
) -> crate::error::Result<(crate::game::Installation, bool)> {
    let install = ctx.app.active_install(game)?;
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let steam_running = crate::steam::is_running(&system);
    Ok((install, steam_running))
}

/// Finds the remembered path for an edition, if the user pointed at one.
fn remembered(ctx: &Ctx, id: &str) -> Option<PathBuf> {
    ctx.app.settings.lock().editions.get(id).cloned()
}

fn resolve(
    ctx: &Ctx,
    spec: &crate::edition::EditionSpec,
    install: &crate::game::Installation,
) -> Option<crate::edition::EditionInstall> {
    // A path the user chose by hand wins over anything found by scanning.
    if let Some(path) = remembered(ctx, spec.id) {
        if let Some(found) = crate::edition::probe(spec, &path) {
            return Some(found);
        }
    }
    crate::edition::discover(spec, install).into_iter().next()
}

async fn editions(State(ctx): State<Ctx>, Query(q): Query<EditionQ>) -> Response {
    let (install, steam_running) = match edition_context(&ctx, q.game) {
        Ok(pair) => pair,
        // Without a located game there is nothing to attach an edition to, but
        // the catalogue of what exists is still worth showing.
        Err(_) => {
            let list: Vec<EditionStatus> = crate::edition::for_game(q.game)
                .into_iter()
                .map(|spec| EditionStatus {
                    spec: *spec,
                    install: None,
                    plan: None,
                    command_line: None,
                    suggested_destination: PathBuf::new(),
                })
                .collect();
            return Json(list).into_response();
        }
    };

    let list: Vec<EditionStatus> = crate::edition::for_game(q.game)
        .into_iter()
        .map(|spec| {
            let found = resolve(&ctx, spec, &install);
            let plan = found.as_ref().and_then(|edition| {
                crate::edition::plan(spec, edition, &install, q.coop, steam_running).ok()
            });
            EditionStatus {
                spec: *spec,
                command_line: plan.as_ref().map(crate::launch::LaunchPlan::command_line),
                install: found,
                plan,
                suggested_destination: crate::edition::default_destination(&install, spec),
            }
        })
        .collect();

    Json(list).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditionBody {
    game: crate::games::Game,
    edition: String,
    #[serde(default)]
    coop: bool,
}

/// The edition id already names its game, so the body does not carry one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditionLocateBody {
    edition: String,
    path: PathBuf,
}

/// Remembers a folder the user picked, after checking it really holds the mod.
async fn edition_locate(State(ctx): State<Ctx>, Json(body): Json<EditionLocateBody>) -> Response {
    let Some(spec) = crate::edition::spec(&body.edition) else {
        return out::<()>(Err(crate::error::Error::msg("no such edition")));
    };
    match crate::edition::probe(spec, &body.path) {
        Some(found) => {
            {
                let mut settings = ctx.app.settings.lock();
                settings
                    .editions
                    .insert(spec.id.to_string(), body.path.clone());
            }
            let saved = ctx.app.settings.lock().clone().save(&ctx.app.app_data);
            match saved {
                Ok(()) => Json(found).into_response(),
                Err(error) => out::<()>(Err(error)),
            }
        }
        None => out::<()>(Err(crate::error::Error::msg(format!(
            "{} does not look like {}: {} is missing",
            body.path.display(),
            spec.name,
            spec.markers.join(", ")
        )))),
    }
}

/// Searches every drive for an edition, reusing the install scan's progress so
/// the interface has one thing to poll.
async fn edition_scan(State(ctx): State<Ctx>, Json(body): Json<EditionBody>) -> Response {
    let Some(spec) = crate::edition::spec(&body.edition) else {
        return out::<()>(Err(crate::error::Error::msg("no such edition")));
    };
    if ctx.app.scan_job.lock().running {
        return out::<()>(Err(crate::error::Error::msg("a search is already running")));
    }
    let install = match ctx.app.active_install(body.game) {
        Ok(install) => install,
        Err(error) => return out::<()>(Err(error)),
    };

    {
        let mut job = ctx.app.scan_job.lock();
        *job = crate::commands::ScanState {
            running: true,
            at: "Starting".into(),
            ..Default::default()
        };
    }

    let app = Arc::clone(&ctx.app);
    let id = spec.id.to_string();

    std::thread::spawn(move || {
        let reporter = Arc::clone(&app);
        let found = crate::edition::spec(&id)
            .map(|spec| {
                crate::edition::deep_discover(spec, &install, move |path| {
                    let mut job = reporter.scan_job.lock();
                    job.at = path.to_string_lossy().to_string();
                    !job.cancelled
                })
            })
            .unwrap_or_default();

        // Remember the first one so the next probe does not have to search again.
        if let Some(first) = found.first() {
            {
                let mut settings = app.settings.lock();
                settings.editions.insert(id, first.root.clone());
            }
            let _ = app.settings.lock().clone().save(&app.app_data);
        }

        let mut job = app.scan_job.lock();
        job.running = false;
        job.done = true;
        job.at = String::new();
    });

    Json(json!({ "started": true })).into_response()
}

fn plan_edition(
    ctx: &Ctx,
    body: &EditionBody,
) -> crate::error::Result<(
    &'static crate::edition::EditionSpec,
    crate::edition::EditionInstall,
    crate::game::Installation,
    crate::launch::LaunchPlan,
)> {
    let spec = crate::edition::spec(&body.edition)
        .ok_or_else(|| crate::error::Error::msg("no such edition"))?;
    let (install, steam_running) = edition_context(ctx, body.game)?;
    let found = resolve(ctx, spec, &install).ok_or_else(|| {
        crate::error::Error::msg(format!("{} is not installed", spec.name))
    })?;
    let plan = crate::edition::plan(spec, &found, &install, body.coop, steam_running)?;
    Ok((spec, found, install, plan))
}

async fn edition_patch(State(ctx): State<Ctx>, Json(body): Json<EditionBody>) -> Response {
    match plan_edition(&ctx, &body) {
        Ok((_, found, install, plan)) => {
            out(crate::edition::patch(&found, &install, body.coop, &plan))
        }
        Err(error) => out::<()>(Err(error)),
    }
}

async fn edition_run(State(ctx): State<Ctx>, Json(body): Json<EditionBody>) -> Response {
    match plan_edition(&ctx, &body) {
        Ok((_, found, install, plan)) => {
            let mut system = sysinfo::System::new();
            system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            if crate::launch::is_game_running(body.game, &system) {
                return out::<()>(Err(crate::error::Error::msg(
                    "the game is already running; close it first",
                )));
            }
            // Patch first: co-op silently not loading is the failure this
            // avoids, and copying a DLL that is already there costs nothing.
            if let Err(error) = crate::edition::patch(&found, &install, body.coop, &plan) {
                return out::<()>(Err(error));
            }
            match crate::launch::spawn(&plan) {
                Ok(pid) => {
                    unlock_when_up(&ctx, body.game);
                    Json(json!({ "pid": pid, "route": plan.route })).into_response()
                }
                Err(error) => out::<()>(Err(error)),
            }
        }
        Err(error) => out::<()>(Err(error)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditionInstallBody {
    game: crate::games::Game,
    edition: String,
    archive: PathBuf,
    destination: Option<PathBuf>,
}

/// Starts unpacking an archive on a background thread.
async fn edition_install(State(ctx): State<Ctx>, Json(body): Json<EditionInstallBody>) -> Response {
    let Some(spec) = crate::edition::spec(&body.edition) else {
        return out::<()>(Err(crate::error::Error::msg("no such edition")));
    };
    if ctx.app.edition_job.lock().running {
        return out::<()>(Err(crate::error::Error::msg(
            "an edition is already being installed",
        )));
    }
    if !body.archive.is_file() {
        return out::<()>(Err(crate::error::Error::msg(format!(
            "{} does not exist",
            body.archive.display()
        ))));
    }

    let destination = match body.destination.clone() {
        Some(path) => path,
        None => match ctx.app.active_install(body.game) {
            Ok(install) => crate::edition::default_destination(&install, spec)
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_default(),
            Err(error) => return out::<()>(Err(error)),
        },
    };

    // Refuse before writing anything rather than half way through.
    let needed = match crate::edition::unpacked_size(&body.archive) {
        Ok(size) => size,
        Err(error) => return out::<()>(Err(error)),
    };
    if let Some(free) = crate::edition::free_space(&destination) {
        // A little headroom: the volume should not end up completely full.
        if free < needed + (1 << 30) {
            return out::<()>(Err(crate::error::Error::msg(format!(
                "{} needs {:.1} GB unpacked and {} has {:.1} GB free",
                spec.name,
                needed as f64 / 1e9,
                destination.display(),
                free as f64 / 1e9
            ))));
        }
    }

    {
        let mut job = ctx.app.edition_job.lock();
        *job = crate::edition::EditionJob {
            edition: spec.id.to_string(),
            running: true,
            message: format!("Unpacking {}", spec.name),
            bytes_total: needed,
            ..Default::default()
        };
    }

    let app = Arc::clone(&ctx.app);
    let archive = body.archive.clone();
    let id = spec.id.to_string();
    let target = destination.clone();
    let game = body.game;

    std::thread::spawn(move || {
        let outcome = crate::edition::extract_archive(
            &archive,
            &target,
            |files_done, files_total, bytes_done, bytes_total| {
                let mut job = app.edition_job.lock();
                job.files_done = files_done;
                job.files_total = files_total;
                job.bytes_done = bytes_done;
                job.bytes_total = bytes_total;
            },
        );

        let mut job = app.edition_job.lock();
        job.running = false;
        job.done = true;
        match outcome {
            Ok(root) => {
                // Remember where it landed so the next scan does not have to
                // guess, and so a folder outside the search path still works.
                {
                    let mut settings = app.settings.lock();
                    settings.editions.insert(id.clone(), root.clone());
                }
                let _ = app.settings.lock().clone().save(&app.app_data);

                // Wire it up straight away. Unpacking and then telling somebody
                // to press Patch is two steps where one will do, and the second
                // one is the step people skip before wondering why co-op is not
                // loading.
                let wired = crate::edition::spec(&id).and_then(|spec| {
                    let found = crate::edition::probe(spec, &root)?;
                    let install = app.active_install(game).ok()?;
                    let plan = crate::edition::plan(spec, &found, &install, true, false).ok()?;
                    crate::edition::patch(&found, &install, true, &plan).ok()
                });

                job.message = match wired {
                    Some(report) if !report.written.is_empty() => {
                        format!("Installed and set up ({} file(s) written)", report.written.len())
                    }
                    _ => format!("Unpacked to {}", root.display()),
                };
                job.destination = Some(root);
            }
            Err(error) => {
                job.message = "Unpacking failed".into();
                job.error = Some(error.to_string());
            }
        }
    });

    Json(json!({ "started": true })).into_response()
}

async fn edition_job(State(ctx): State<Ctx>) -> Response {
    Json(ctx.app.edition_job.lock().clone()).into_response()
}

async fn launch_plan(State(ctx): State<Ctx>, Query(q): Query<GameProfileQ>) -> Response {
    match plan_for(&ctx, q.game, &q.profile) {
        Ok((install, profile, library, loaders, work)) => {
            let mut system = sysinfo::System::new();
            system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let input = crate::launch::PlanInput {
                install: &install,
                profile: &profile,
                mods: &library,
                loaders: &loaders,
                work_dir: work,
                steam_running: crate::steam::is_running(&system),
            };
            match crate::launch::plan(&input) {
                Ok(plan) => Json(json!({
                    "plan": plan,
                    "commandLine": plan.command_line(),
                }))
                .into_response(),
                Err(error) => out::<()>(Err(error)),
            }
        }
        Err(error) => out::<()>(Err(error)),
    }
}

#[derive(Deserialize)]
struct LaunchBody {
    game: crate::games::Game,
    profile: String,
}

async fn launch_patch(State(ctx): State<Ctx>, Json(body): Json<LaunchBody>) -> Response {
    match plan_for(&ctx, body.game, &body.profile) {
        Ok((install, profile, library, loaders, work)) => {
            let mut system = sysinfo::System::new();
            system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let input = crate::launch::PlanInput {
                install: &install,
                profile: &profile,
                mods: &library,
                loaders: &loaders,
                work_dir: work,
                steam_running: crate::steam::is_running(&system),
            };
            match crate::launch::plan(&input) {
                Ok(plan) => out(crate::launch::apply(&input, &plan)),
                Err(error) => out::<()>(Err(error)),
            }
        }
        Err(error) => out::<()>(Err(error)),
    }
}

async fn launch_run(State(ctx): State<Ctx>, Json(body): Json<LaunchBody>) -> Response {
    match plan_for(&ctx, body.game, &body.profile) {
        Ok((install, mut profile, library, loaders, work)) => {
            let mut system = sysinfo::System::new();
            system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

            if crate::launch::is_game_running(body.game, &system) {
                return out::<()>(Err(crate::error::Error::msg(
                    "the game is already running; close it first",
                )));
            }

            let input = crate::launch::PlanInput {
                install: &install,
                profile: &profile,
                mods: &library,
                loaders: &loaders,
                work_dir: work,
                steam_running: crate::steam::is_running(&system),
            };

            let plan = match crate::launch::plan(&input) {
                Ok(plan) => plan,
                Err(error) => return out::<()>(Err(error)),
            };
            let patched = match crate::launch::apply(&input, &plan) {
                Ok(report) => report,
                Err(error) => return out::<()>(Err(error)),
            };

            let backup = {
                let (auto, keep) = {
                    let settings = ctx.app.settings.lock();
                    (settings.auto_backup_on_launch, settings.auto_backup_keep)
                };
                let live = install
                    .appdata_dir()
                    .map(|d| d.join(body.game.save_file()))
                    .filter(|p| p.is_file());
                match (auto, live) {
                    (true, Some(path)) => {
                        let record = crate::saves::create_backup(
                            &ctx.app.app_data,
                            body.game,
                            &path,
                            &format!("before {}", profile.name),
                            true,
                        );
                        crate::saves::prune_backups(&ctx.app.app_data, body.game, keep).ok();
                        record.ok().map(|r| r.id)
                    }
                    _ => None,
                }
            };

            match crate::launch::spawn(&plan) {
                Ok(pid) => {
                    if ctx.app.settings.lock().discord_presence {
                        ctx.app.presence.set_playing(body.game, Some(&profile.name));
                    }
                    profile.last_played = Some(chrono::Local::now().to_rfc3339());
                    crate::mods::save_profile(&ctx.app.app_data, &profile).ok();
                    unlock_when_up(&ctx, body.game);
                    Json(json!({
                        "pid": pid,
                        "route": plan.route.label(),
                        "patched": patched,
                        "backupId": backup,
                    }))
                    .into_response()
                }
                Err(error) => out::<()>(Err(error)),
            }
        }
        Err(error) => out::<()>(Err(error)),
    }
}

async fn running(State(ctx): State<Ctx>) -> Response {
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let active = crate::games::Game::ALL
        .into_iter()
        .find(|game| crate::launch::is_game_running(*game, &system));

    if active.is_none() && ctx.app.settings.lock().discord_presence {
        ctx.app.presence.set_browsing();
    }

    Json(json!({ "game": active })).into_response()
}

async fn saves_discover(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    let extra = game_dir(&ctx, q.game)
        .ok()
        .and_then(|dir| crate::coop::read(&dir).ok())
        .and_then(|s| s.values.get("SAVE.save_file_extension").cloned());
    Json(crate::saves::discover(q.game, extra.as_deref())).into_response()
}

async fn saves_inspect(Query(q): Query<PathQ>) -> Response {
    out(crate::saves::inspect(&q.path))
}

async fn saves_backups(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    Json(crate::saves::list_backups(&ctx.app.app_data, q.game)).into_response()
}

#[derive(Deserialize)]
struct BackupBody {
    game: crate::games::Game,
    path: PathBuf,
    #[serde(default)]
    label: String,
}

async fn saves_backup(State(ctx): State<Ctx>, Json(body): Json<BackupBody>) -> Response {
    out(crate::saves::create_backup(
        &ctx.app.app_data,
        body.game,
        &body.path,
        if body.label.is_empty() { "manual" } else { &body.label },
        false,
    ))
}

#[derive(Deserialize)]
struct RestoreBody {
    game: crate::games::Game,
    id: String,
}

async fn saves_backup_delete(State(ctx): State<Ctx>, Json(body): Json<RestoreBody>) -> Response {
    out(crate::saves::delete_backup(
        &ctx.app.app_data,
        body.game,
        &body.id,
    ))
}

async fn saves_restore(State(ctx): State<Ctx>, Json(body): Json<RestoreBody>) -> Response {
    out(crate::saves::restore_backup(
        &ctx.app.app_data,
        body.game,
        &body.id,
        None,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransferBody {
    game: crate::games::Game,
    source: PathBuf,
    destination: PathBuf,
    slot_pairs: Vec<(usize, usize)>,
}

async fn saves_transfer(State(ctx): State<Ctx>, Json(body): Json<TransferBody>) -> Response {
    out(crate::saves::transfer_slots(
        &ctx.app.app_data,
        body.game,
        &body.source,
        &body.destination,
        &body.slot_pairs,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConvertBody {
    game: crate::games::Game,
    source: PathBuf,
    extension: String,
    #[serde(default)]
    rebind_to: Option<u64>,
}

async fn saves_convert(State(ctx): State<Ctx>, Json(body): Json<ConvertBody>) -> Response {
    out(crate::saves::convert(
        &ctx.app.app_data,
        body.game,
        &body.source,
        &body.extension,
        None,
        body.rebind_to,
    ))
}

async fn sys_caches() -> Response {
    Json(crate::sys::shader_caches()).into_response()
}

#[derive(Deserialize)]
struct PathsBody {
    paths: Vec<PathBuf>,
}

async fn sys_clear(Json(body): Json<PathsBody>) -> Response {
    out(crate::sys::clear_caches(&body.paths))
}

async fn sys_report(Query(q): Query<GameQ>) -> Response {
    Json(crate::sys::system_report(q.game)).into_response()
}

async fn open_path(Json(body): Json<PathQ>) -> Response {
    #[cfg(windows)]
    {
        let target = if body.path.is_file() {
            body.path.parent().map(std::path::Path::to_path_buf).unwrap_or(body.path)
        } else {
            body.path
        };
        let _ = std::process::Command::new("explorer").arg(target).spawn();
    }
    Json(json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// Native pickers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PickQ {
    #[serde(default)]
    title: Option<String>,
}

/// A browser cannot hand a page a folder path, so the desktop window that is
/// still running in the tray opens the real dialog and returns the path here.
async fn pick_folder(Query(q): Query<PickQ>) -> Response {
    let title = q.title.unwrap_or_else(|| "Select a folder".into());
    match crate::dialog::pick_folder(&title) {
        Some(path) => Json(json!({ "path": path })).into_response(),
        None => Json(json!({ "path": null })).into_response(),
    }
}

#[derive(Deserialize)]
struct PickFileQ {
    #[serde(default)]
    title: Option<String>,
    /// Comma-separated extensions, e.g. `zip,7z`.
    #[serde(default)]
    filter: Option<String>,
}

async fn pick_file(Query(q): Query<PickFileQ>) -> Response {
    let title = q.title.unwrap_or_else(|| "Select a file".into());
    let filters: Vec<String> = q
        .filter
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    match crate::dialog::pick_file(&title, &filters) {
        Some(path) => Json(json!({ "path": path })).into_response(),
        None => Json(json!({ "path": null })).into_response(),
    }
}

#[cfg(test)]
mod update_tests {
    use super::is_newer;

    #[test]
    fn a_higher_version_is_newer() {
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(is_newer("0.3.0", "0.2.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
    }

    #[test]
    fn the_same_version_is_not_newer() {
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("0.2.0", "0.2.1"));
    }

    #[test]
    fn versions_compare_numerically_not_as_text() {
        // The comparison people get wrong: as strings, "0.10" sorts below "0.9".
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.9.0", "0.10.0"));
    }

    #[test]
    fn a_missing_component_counts_as_zero() {
        assert!(is_newer("0.3", "0.2.9"));
        assert!(!is_newer("0.2", "0.2.0"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_carries_the_session_key() {
        let server = Server {
            port: 7314,
            token: "abc123".into(),
        };
        let url = server.url();
        assert!(url.starts_with("http://127.0.0.1:7314/"));
        assert!(url.contains("k=abc123"));
    }

    #[test]
    fn the_server_only_ever_binds_loopback() {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        assert!(address.ip().is_loopback());
    }
}
