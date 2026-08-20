//! The local HTTP server.

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
        // The pins on the world map, which live in the save.
        .route("/markers", get(markers_read))
        .route("/markers/places", get(markers_places))
        .route("/markers/add", post(markers_add))
        .route("/markers/remove", post(markers_remove))
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

/// Where the co-op settings the game will actually read live.
fn coop_dir(ctx: &Ctx, game: crate::games::Game) -> crate::error::Result<PathBuf> {
    let root = ctx
        .app
        .settings
        .lock()
        .install_for(game)
        .map(|i| i.root.clone())
        .ok_or(crate::error::Error::NoGameSelected)?;
    let install = crate::game::Installation::probe(game, &root)?;

    for root in edition_roots(ctx, game) {
        if crate::coop::settings_path(&root).is_file() {
            return Ok(root);
        }
    }
    Ok(install.game_dir)
}

/// Editions the launcher already knows the path of.
fn edition_roots(ctx: &Ctx, game: crate::games::Game) -> Vec<PathBuf> {
    crate::edition::for_game(game)
        .into_iter()
        .filter_map(|spec| remembered(ctx, spec.id))
        .collect()
}

async fn coop_read(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    match coop_dir(&ctx, q.game) {
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
    let Ok(dir) = coop_dir(&ctx, body.game) else {
        return out::<()>(Err(crate::error::Error::NoGameSelected));
    };
    let written = match crate::coop::write(&dir, &body.changes) {
        Ok(written) => written,
        Err(error) => return out::<()>(Err(error)),
    };

    // Every other copy gets the same thing.
    //
    // A password that is right in one file and stale in another is worse than
    // one that is stale in both, because it looks correct wherever you check.
    // The mod's copy is the one the game reads and is written first; the game
    // folder's copy is kept in step so switching back to vanilla does not
    // silently drop you onto an old password.
    if let Some(root) = ctx.app.settings.lock().install_for(body.game).map(|i| i.root.clone()) {
        if let Ok(install) = crate::game::Installation::probe(body.game, &root) {
            let mut also: Vec<PathBuf> = vec![install.game_dir.clone()];
            also.extend(edition_roots(&ctx, body.game));
            for other in also {
                if other != dir && crate::coop::settings_path(&other).is_file() {
                    let _ = crate::coop::write(&other, &body.changes);
                }
            }
        }
    }

    out(Ok(written))
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

/// The longest run of words every one of these names shares.
fn shared_words(names: &[String]) -> Option<String> {
    let first: Vec<&str> = names.first()?.split_whitespace().collect();
    if names.len() < 2 {
        return None;
    }
    let mut best: Option<String> = None;
    for start in 0..first.len() {
        for end in (start + 1..=first.len()).rev() {
            let run = first[start..end].join(" ");
            if run.chars().count() < 3 {
                continue;
            }
            let low = run.to_lowercase();
            if names.iter().all(|name| name.to_lowercase().contains(&low))
                && best.as_ref().is_none_or(|had| run.chars().count() > had.chars().count())
            {
                best = Some(run);
            }
        }
    }
    best
}

/// A question about the game, answered out of the wiki.
async fn ask_question(State(ctx): State<Ctx>, Json(body): Json<AskBody>) -> Response {
    let player = player_state(&ctx, body.edition.as_deref(), &body.question);
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
fn player_state(ctx: &Ctx, edition: Option<&str>, asked: &str) -> crate::ask::Player {
    let game = ctx.app.settings.lock().selected_game;
    let mut player = crate::ask::Player { asked: asked.to_string(), ..Default::default() };

    if let Ok(install) = ctx.app.active_install(game) {
        player.version = install.version.clone();
        player.framegen = crate::erss::owns_the_frame_cap(&install.game_dir);
        player.frames = frame_rate_facts(ctx, game, &install, player.framegen);
        // Where the mod lives depends on how the game is launched, so both the
        // base folder and whichever edition is remembered are worth a look.
        player.seamless = crate::coop::dll_path(&install.game_dir).is_file()
            || edition_roots(ctx, game)
                .iter()
                .any(|root| crate::coop::dll_path(root).is_file());
        // From the game's own configuration rather than from the process, so
        // it costs a file read and is known before the game is even open.
        player.language = crate::language::status(&install.game_dir).current;
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

    // What the running game says, read only if the model asks for it. The
    // richer version, which needs the mod folder, is wired further down.
    player.live = Some(Box::new(move || crate::live::read(game)));

    // The tables the installation balances itself with. Two lookups, in the
    // order that gets the right answer soonest: what they are holding, which
    // the game names for us, then the whole catalogue.
    // Which tables the game will load. Remembered paths only, never a scan:
    // working out where an edition lives by walking the disk is what once made
    // a routine question hang for a minute with the port open.
    //
    // Falling back to any remembered edition matters. A caller that does not
    // name one used to get the base game's regulation, so the assistant
    // answered a modded player with the vanilla figures — through the very tool
    // built to stop that.
    let has_tables = |root: &PathBuf| {
        let dir = root.join("mod");
        dir.join("regulation.bin").is_file().then_some(dir)
    };
    let mut from_edition: Option<String> = None;
    let mod_dir = edition
        .and_then(|id| remembered(ctx, id))
        .and_then(|root| has_tables(&root))
        .or_else(|| {
            let settings = ctx.app.settings.lock();
            settings.editions.iter().find_map(|(id, root)| {
                let dir = has_tables(root)?;
                from_edition = Some(id.clone());
                Some(dir)
            })
        });

    // Whose tables these are is also the answer to "what am I playing". Left
    // apart, the two disagreed: the figures came out of a total conversion
    // while the assistant told the player they were on the base game and their
    // class did not exist.
    if player.edition.is_none() {
        player.edition = from_edition.and_then(|id| {
            crate::edition::for_game(game)
                .into_iter()
                .find(|spec| spec.id == id)
                .map(|spec| spec.name.to_string())
        });
    }
    // Whether the game is in a language the launcher can translate a name into.
    //
    // The table is the English wiki's own langlinks, so it is that wiki's, not
    // the edition's — a total conversion's wiki has no translations of its own.
    // And it is the *player's* language that matters here, not the wiki's.
    let asking = Asking {
        app_data: ctx.app.app_data.clone(),
        source: crate::wiki::for_edition(None).id,
        language: player.language.as_deref().and_then(wiki_language),
    };

    if let Ok(install) = ctx.app.active_install(game) {
        player.holdings = what_it_holds(game, &install.game_dir, mod_dir.as_deref());
        player.safety = where_the_anticheat_stands(game, &install.game_dir, player.seamless);
    }
    player.backups = what_is_kept(&ctx.app.app_data, game);
    player.mirrors = what_is_mirrored(&ctx.app.app_data);
    player.set_up = {
        let settings = ctx.app.settings.lock();
        let here: Vec<&str> = crate::games::Game::ALL
            .iter()
            .filter(|one| settings.installations.iter().any(|saved| saved.game == **one))
            .map(|one| one.display_name())
            .collect();
        Some(match here.as_slice() {
            [] => "  This player has not pointed it at any installation yet, so there is \
                   nothing of theirs to read for any game.\n"
                .to_string(),
            _ => format!(
                "  What THIS player has set up is {}, and nothing else. Asked about a character, \
                 a save or a mod in one of the other titles, the answer is that the launcher \
                 handles that game but they have not added it here — never that it cannot, and \
                 never a count of characters in a game it has never been shown.\n",
                here.join(", ")
            ),
        })
    };

    let pin_dir = mod_dir.clone();

    // What the running game says, read only if the model asks for it.
    //
    // The region label comes out of the launcher's own survey table, which is
    // in English whoever is playing — so a Russian player asking in Russian was
    // told they were standing in "Weeping Peninsula - Castle Morne Rampart"
    // while their armour and items came back in Russian in the same breath. The
    // game names its own places, in the language they are reading, and those
    // names carry map coordinates; the nearest one is what their map prints.
    let placed_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    player.live = Some(Box::new(move || {
        let mut live = crate::live::read(game)?;
        if let (Some(place), Some((game_dir, mod_dir))) = (live.place.as_mut(), placed_in.as_ref())
        {
            let language = crate::language::status(game_dir)
                .current
                .as_deref()
                .and_then(crate::language::locale_folder)
                .unwrap_or("engus")
                .to_string();
            if let Some(theirs) = crate::places::nearest_named(
                game_dir,
                mod_dir.as_deref(),
                &language,
                &place.map,
                place.x,
                place.z,
            ) {
                // Both, because they are different things: theirs is what the
                // map prints and ours is the region it sits in. Saying only the
                // nearest named point would put somebody "at the Church of
                // Elleh" when they are a field away from it.
                place.name = Some(match &place.name {
                    Some(ours) => format!("{theirs} ({ours})"),
                    None => theirs,
                });
            }
        }
        Some(live)
    }));

    let dirs = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), mod_dir));
    let armed_asking = asking.clone();
    player.weapon = Box::new(move |wanted| {
        let (game_dir, mod_dir) = dirs.clone()?;
        let regulation =
            crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())?;
        let looking = wanted.trim().to_lowercase();

        // What is in their hands: the game has already given us the names and
        // the ids, so nothing has to be guessed. An empty query means exactly
        // this, and it answers for every hand rather than the first — one of
        // them is often a seal or a torch with no row of its own.
        if let Some(live) = crate::live::read(game) {
            // Their own attributes, so the figures can be the ones on their
            // stat screen rather than the table's bare base.
            let theirs = attributes_of(&live);
            if let Some(gear) = live.gear {
                let held: Vec<crate::ask::Armed> = gear
                    .weapon_ids
                    .iter()
                    .filter(|(name, _)| {
                        if looking.is_empty() {
                            return true;
                        }
                        let name = name.to_lowercase();
                        name.contains(&looking) || looking.contains(&name)
                    })
                    .filter_map(|(name, id)| {
                        Some(crate::ask::Armed {
                            name: name.clone(),
                            weapon: regulation.weapon(*id)?,
                            hits: regulation.attack_with(*id, theirs),
                            // These already ARE their attributes, so there is
                            // no before-and-after to draw.
                            now: Vec::new(),
                            skill: skill_on(&regulation, &game_dir, mod_dir.as_deref(), *id),
                            modded: mod_dir.is_some(),
                        })
                    })
                    .collect();
                if !held.is_empty() || looking.is_empty() {
                    return Some(held);
                }
            }
        }

        // Otherwise by name, through the game's own text — under the name it
        // was asked by, and then under whatever else the same thing is called.
        if looking.is_empty() {
            return Some(Vec::new());
        }
        let named = spellings(&armed_asking, wanted)
            .into_iter()
            .find_map(|spelling| {
                let found = crate::text::look_up(game, &spelling, 8)?;
                (!found.is_empty()).then_some(found)
            })
            .map(|found| {
                found
                    .into_iter()
                    .filter(|hit| hit.kind == crate::text::Kind::Weapon)
                    .map(|hit| (hit.name, i64::from(hit.id - hit.id % 100)))
                    .collect::<Vec<_>>()
            })
            // Game shut means `look_up` is empty, so fall back to the disk — or
            // "my weapon" aside, no weapon was findable by name without the game.
            .filter(|hits| !hits.is_empty())
            .unwrap_or_else(|| {
                named_offline(&game_dir, mod_dir.as_deref(), "weapon", wanted)
            });
        let theirs = crate::live::read(game).map(|live| attributes_of(&live));
        let found: Vec<crate::ask::Armed> = named
            .into_iter()
            .filter_map(|(name, id)| {
                Some(crate::ask::Armed {
                    weapon: regulation.weapon(id)?,
                    // Only with the game open: without their attributes there
                    // is no "in their hands" figure, and a base pretending to
                    // be one is the thing this fixes.
                    hits: theirs.map_or_else(Vec::new, |theirs| regulation.attack_with(id, theirs)),
                    now: Vec::new(),
                    skill: skill_on(&regulation, &game_dir, mod_dir.as_deref(), id),
                    name,
                    modded: mod_dir.is_some(),
                })
            })
            .take(3)
            .collect();
        Some(found)
    });
    // Armour, by name, out of the same tables. Named only: the game hands us
    // ids for what is in their hands but not for what is on their back, and
    // the name is what player_status has already told the model anyway.
    let dressed_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let dressed_asking = asking.clone();
    player.armour = Box::new(move |wanted| {
        let (game_dir, mod_dir) = dressed_in.clone()?;
        let regulation = crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())?;

        let named = spellings(&dressed_asking, wanted).into_iter().find_map(|spelling| {
            let found = crate::text::look_up(game, &spelling, 8)?;
            let armour: Vec<crate::text::Found> = found
                .into_iter()
                .filter(|hit| hit.kind == crate::text::Kind::Armour)
                .collect();
            (!armour.is_empty()).then_some(armour)
        })?;

        Some(
            named
                .into_iter()
                .filter_map(|hit| {
                    Some(crate::ask::Dressed {
                        armour: regulation.armour(i64::from(hit.id))?,
                        name: hit.name,
                        modded: mod_dir.is_some(),
                    })
                })
                .take(4)
                .collect(),
        )
    });

    // Every piece in the game against one kind of damage, ranked here rather
    // than a call at a time. Asked which armour best holds lightning, a model
    // fetched the four pieces they had on and answered from those; the table
    // has 913 and the ranking is a sort.
    let ranking = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let ranking_data = ctx.app.app_data.clone();
    player.armoury = Box::new(move |kind| {
        let Some((game_dir, mod_dir)) = ranking.clone() else {
            return Vec::new();
        };
        let Some(regulation) =
            crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())
        else {
            return Vec::new();
        };
        let Some(table) = regulation.table("EquipParamProtector") else {
            return Vec::new();
        };
        // Poise is not a kind of damage and is exactly what somebody means by
        // "which armour holds up best". Asked for it, the ranking refused —
        // there is no such damage kind — and an answer then declared that poise
        // "is not in the armour tables as a parameter of its own, it is worked
        // out from physical defence and weight" and ranked by physical instead.
        // Every part of that is false and the figure was sitting unread.
        // Стойкость is officially ENDURANCE — the game's own menu reads
        // "Стойкость(END)" — and poise is Баланс. It stays on this list anyway,
        // because armour has no endurance to rank by, so somebody who types it
        // at an armour ranking means the thing heavy armour gives them. Do not
        // copy this list anywhere that could answer with an attribute.
        let by_poise = {
            let said = kind.trim().to_lowercase();
            ["poise", "пойз", "стойкост", "баланс", "poise/", "haltung", "aguante", "equilibrio"]
                .iter()
                .any(|word| said.starts_with(word) || word.starts_with(&said) && said.len() > 3)
        };
        // Weight, which is a family of its own and had none. Asked which
        // armour is the heaviest, the ranking had nothing to rank by: it tried
        // "weight" as a damage kind, got nothing, and the answer then guessed
        // "Bull-Goat" — the base game's ENGLISH name, at a Russian table in a
        // total conversion — and gave up. The figure was there the whole time.
        // Every piece already carries it, because the sort at the bottom of
        // this closure uses it as the tie-breaker.
        //
        // "light" is deliberately NOT on this list: it is a prefix of
        // "lightning", which is a real damage kind, and swallowing that would
        // break a question that works today. "heavy" is off it for the same
        // reason — "best armour against heavy attacks" is not a weight
        // question. The superlatives are unambiguous and are what gets typed.
        let by_weight = !by_poise && crate::formats::regulation::bulk::asked_for(kind);

        // The four the equipment screen shows. Not damage, not negated, and
        // already read — but the ranking used to refuse them, and the refusal
        // told a German player that Robustheit was not a thing armour has.
        let by_resistance = if by_poise || by_weight {
            None
        } else {
            crate::formats::regulation::resistance::named(kind)
        };

        // Whatever the player called it. A German question sent "blitz" and
        // burned a round being told there is no such kind.
        // Last of the four, and last on purpose: poise and the resistances keep
        // whatever they already meant. Armour here GRANTS attributes and there
        // was no way to ask which — "quelle armure pour un build foi" came back
        // as the lightest armour in the game with no mention of faith.
        let by_attribute = if by_poise || by_weight || by_resistance.is_some() {
            None
        } else {
            crate::formats::regulation::attribute::named(kind)
        };

        let wanted = if by_poise {
            "poise"
        } else if by_weight {
            "weight"
        } else if let Some(named) = by_resistance {
            named
        } else if let Some(named) = by_attribute {
            named
        } else {
            let Some(named) = crate::formats::regulation::kind::named(kind) else {
                return Vec::new();
            };
            named
        };

        // Names in their own language, whichever source has them.
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let mut named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|one| one.what == "armour")
                .map(|one| (one.id, one.name.clone()))
                .collect();
        if named.is_empty() {
            named = crate::text::names(
                &ranking_data,
                game,
                Some(&game_dir),
                mod_dir.as_deref(),
                crate::text::Kind::Armour,
            )
            .into_iter()
            .collect();
        }

        let mut out: Vec<crate::ask::Shielding> = table
            .ids()
            .filter_map(|id| {
                let piece = regulation.armour(id)?;
                // A row with no weight is one of the game's placeholders.
                if piece.weight <= 0.0 {
                    return None;
                }
                let name = named.get(&u32::try_from(id).ok()?)?;
                let stopped = if by_poise {
                    piece.poise?
                } else if by_weight {
                    piece.weight
                } else if by_attribute.is_some() {
                    // Only pieces that actually grant it. A ranking of 851
                    // pieces where 840 give zero faith is not a ranking.
                    let given = piece
                        .gives
                        .iter()
                        .find(|(what, _)| what == wanted)
                        .map(|(_, value)| *value)?;
                    if given == 0 {
                        return None;
                    }
                    given as f32
                } else if by_resistance.is_some() {
                    // Stored under "robustness — bleed and frost" and its
                    // fellows, so the canonical word is the prefix.
                    piece
                        .resistance
                        .iter()
                        .find(|(what, _)| what.starts_with(wanted))
                        .map(|(_, value)| f32::from(*value))?
                } else {
                    piece
                        .negation
                        .iter()
                        .find(|(what, _)| what.eq_ignore_ascii_case(wanted))
                        .map(|(_, value)| *value)?
                };
                Some((piece.worn?, name.clone(), stopped, piece.weight))
            })
            .collect();
        // By slot, then by what it stops. A flat ranking put four chest pieces
        // at the top and a player wears one of those; grouped, the list is a
        // set they can actually put on.
        out.sort_by(|a, b| {
            let order = |what: &str| {
                crate::formats::regulation::slot::NAMES.iter().position(|s| *s == what)
            };
            order(a.0)
                .cmp(&order(b.0))
                .then_with(|| b.2.total_cmp(&a.2))
                .then_with(|| a.3.total_cmp(&b.3))
        });
        out.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        out
    });

    // Spells, out of the same tables. A total conversion invents hundreds of
    // its own and rewrites the costs of the rest, so the base game's figures
    // are about a different game.
    let cast_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let cast_asking = asking.clone();
    player.spell = Box::new(move |wanted| {
        let (game_dir, mod_dir) = cast_in.clone()?;
        let regulation = crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())?;

        let named = spellings(&cast_asking, wanted).into_iter().find_map(|spelling| {
            let found = crate::text::look_up(game, &spelling, 8)?;
            let spells: Vec<crate::text::Found> = found
                .into_iter()
                .filter(|hit| hit.kind == crate::text::Kind::Goods)
                .collect();
            (!spells.is_empty()).then_some(spells)
        })?;

        Some(
            named
                .into_iter()
                .filter_map(|hit| {
                    Some(crate::ask::Cast {
                        spell: regulation.spell(i64::from(hit.id))?,
                        name: hit.name,
                        modded: mod_dir.is_some(),
                    })
                })
                .take(4)
                .collect(),
        )
    });

    // Which spells a set of attributes opens. Names come from the running game
    // when it is up and off the disk when it is not, so the answer is the same
    // either way.
    let reach_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let reach_kept = ctx.app.app_data.clone();
    // Who is standing where. The first call walks 634 map files and takes
    // about eight seconds; every one after it is free, which is why this is a
    // tool rather than anything the block pays for.
    let living = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let living_data = ctx.app.app_data.clone();
    player.dwellers = Box::new(move |wanted| {
        let Some((game_dir, mod_dir)) = living.clone() else {
            return Vec::new();
        };
        let all = crate::bestiary::everyone(&living_data, game, &game_dir, mod_dir.as_deref());
        // No map named means the one they are standing on. Anything that is not
        // a map id is taken as the name of something and looked for across the
        // whole world — a player names a boss, not a map, and a model asked to
        // find one it had no id for went to a wiki and came back with a
        // different creature.
        let map = match wanted.trim() {
            "" => match crate::live::read(game).and_then(|live| live.place).map(|at| at.map) {
                Some(map) => map,
                None => return Vec::new(),
            },
            named if !crate::bestiary::is_a_map(named) => {
                return crate::bestiary::called(&all, named).into_iter().cloned().collect();
            }
            named => named.to_string(),
        };
        crate::bestiary::on_map(&all, &map)
    });

    // What can be got on a map, which is a different question from who is on
    // it: the named things almost never drop anything, and the soldiers around
    // them have no names at all.
    let farming = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let farming_data = ctx.app.app_data.clone();
    player.haul = Box::new(move |wanted| {
        let Some((game_dir, mod_dir)) = farming.clone() else {
            return Vec::new();
        };
        let all = crate::bestiary::everyone(&farming_data, game, &game_dir, mod_dir.as_deref());
        let map = match wanted.trim() {
            "" => match crate::live::read(game).and_then(|live| live.place).map(|at| at.map) {
                Some(map) => map,
                None => return Vec::new(),
            },
            named => named.to_string(),
        };
        crate::bestiary::haul_on(&all, &map).to_vec()
    });

    // One weapon's weight, by id, out of the tables and nothing else. No live
    // read — see `weigh` for why that matters.
    let scales = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    player.weigh = Box::new(move |id| {
        let (game_dir, mod_dir) = scales.clone()?;
        crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())?.weapon(id)
    });

    // The skill on one weapon, for the block. Everything it reads is cached, so
    // it costs the same whether it is asked for one weapon or both hands.
    let arts = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    player.skill_on = Box::new(move |id| {
        let (game_dir, mod_dir) = arts.clone()?;
        let regulation = crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())?;
        skill_on(&regulation, &game_dir, mod_dir.as_deref(), id)
    });

    // Everything the installation names, searched by name and by effect. The
    // same loose-then-cached reading as the talismans, and the same stemming,
    // because the languages this game is played in inflect.
    let shelf_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let shelf_kept = ctx.app.app_data.clone();
    player.catalogue_of = Box::new(move |kind, wanted| {
        let Some((game_dir, mod_dir)) = shelf_in.clone() else {
            return Vec::new();
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");

        let shelf = crate::library::everything(&game_dir, mod_dir.as_deref(), language);
        let mut all: Vec<crate::ask::Named> = shelf
            .iter()
            .filter(|one| kind.is_empty() || one.what == kind)
            .map(|one| crate::ask::Named {
                what: one.what.clone(),
                name: one.name.clone(),
                effect: one.effect.clone(),
            })
            .collect();
        // A plain installation has no loose text; what the game had loaded is
        // written down instead. Names only there, which is still the
        // difference between an answer and a guess.
        if all.is_empty() {
            let kinds: &[(&str, crate::text::Kind)] = &[
                ("weapon", crate::text::Kind::Weapon),
                ("armour", crate::text::Kind::Armour),
                ("talisman", crate::text::Kind::Talisman),
                ("item", crate::text::Kind::Goods),
            ];
            for (what, which) in kinds {
                if !kind.is_empty() && *what != kind {
                    continue;
                }
                all.extend(
                    crate::text::names(
                        &shelf_kept,
                        game,
                        Some(&game_dir),
                        mod_dir.as_deref(),
                        *which,
                    )
                    .into_iter()
                    .map(|(_, name)| crate::ask::Named {
                        what: (*what).to_string(),
                        name,
                        effect: None,
                    }),
                );
            }
        }

        let looking = wanted.trim().to_lowercase();
        if looking.is_empty() {
            return all;
        }
        let length = looking.chars().count();
        let trim = match length {
            0..=3 => 0,
            4..=5 => 1,
            _ => 2,
        };
        let stem: String = looking.chars().take(length - trim).collect();
        // And on the root, for the words that only share one.
        //
        // Trimming an ending is enough for "вера" against "веру". It is not
        // enough across a derivation: "кровотечение" and "кровавый" have
        // nothing in common past "кров", so a search for the ailment found
        // none of the three ashes of war named after it. Every ash carries the
        // same generic line for its effect — "наделяет оружие свойствами и
        // навыками" — so the name is the only thing that can be matched, and a
        // Russian name shares a root rather than a whole word.
        let root: Option<String> =
            (length >= 6).then(|| looking.chars().take(4).collect::<String>());
        let starts_with_root = |text: &str| match &root {
            Some(root) => text
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .any(|word| word.starts_with(root.as_str())),
            None => false,
        };
        all.retain(|one| {
            let name = one.name.to_lowercase();
            let says = one.effect.as_deref().unwrap_or_default().to_lowercase();
            name.contains(&stem)
                || says.contains(&stem)
                || starts_with_root(&name)
                || starts_with_root(&says)
        });
        all
    });

    // Talismans: the table for the list, the game's own text for the names.
    // Neither half was missing; the join was.
    let charm_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let kept_in = ctx.app.app_data.clone();
    player.talismans = Box::new(move || {
        let Some((game_dir, mod_dir)) = charm_in.clone() else {
            return Vec::new();
        };
        let Some(regulation) = crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())
        else {
            return Vec::new();
        };
        // Read off the disk rather than out of the running game, because the
        // disk carries the line that says what each one does and the game's
        // name table does not. Both give the same names in the same language;
        // only one of them makes the list answerable.
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let mut described: std::collections::HashMap<u32, (String, Option<String>)> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .filter(|item| item.what == "talisman")
                .map(|item| (item.id, (item.name.clone(), item.effect.clone())))
                .collect();
        // A plain installation keeps its text in the packed archives, so the
        // files above hold nothing for it. What the game itself had loaded is
        // written down on every launch; fall back to that, names only, rather
        // than telling a player without a mod that their game has no talismans.
        if described.is_empty() {
            described = crate::text::names(
                &kept_in,
                game,
                Some(&game_dir),
                mod_dir.as_deref(),
                crate::text::Kind::Talisman,
            )
            .into_iter()
            .map(|(id, name)| (id, (name, None)))
            .collect();
        }
        if described.is_empty() {
            return Vec::new();
        }

        let mut out: Vec<crate::ask::Charm> = regulation
            .talismans()
            .into_iter()
            .filter_map(|charm| {
                let id = u32::try_from(charm.id).ok()?;
                let (name, effect) = described.get(&id)?;
                Some(crate::ask::Charm {
                    name: name.clone(),
                    weight: charm.weight,
                    effect: effect.clone(),
                    figures: regulation.charm(charm.id),
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out.dedup_by(|a, b| a.name == b.name);
        out
    });

    // The same weapons at attributes they do not have yet. Held first, then by
    // name, exactly as `weapon` matches them — a different match here would
    // answer about a different weapon than the line above it.
    let ifs_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let ifs_asking = asking.clone();
    player.what_if = Box::new(move |wanted, asked| {
        let Some((game_dir, mod_dir)) = ifs_in.clone() else {
            return Vec::new();
        };
        let Some(regulation) = crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())
        else {
            return Vec::new();
        };
        let looking = wanted.trim().to_lowercase();

        let reading = crate::live::read(game);
        // Whatever was not asked about stays as it is. Treating a missing
        // attribute as zero made a question about raising one read as a
        // question about dropping the other four.
        let now = reading.as_ref().map(attributes_of).unwrap_or([0; 5]);
        let mut stats = now;
        for at in 0..5 {
            if let Some(value) = asked[at] {
                stats[at] = value;
            }
        }

        let mut candidates: Vec<(String, i64)> = Vec::new();
        if let Some(live) = reading {
            if let Some(gear) = live.gear {
                for (name, id) in &gear.weapon_ids {
                    let lowered = name.to_lowercase();
                    if looking.is_empty()
                        || lowered.contains(&looking)
                        || looking.contains(&lowered)
                    {
                        candidates.push((name.clone(), *id));
                    }
                }
            }
        }
        if candidates.is_empty() && !looking.is_empty() {
            for spelling in spellings(&ifs_asking, wanted) {
                let Some(found) = crate::text::look_up(game, &spelling, 8) else {
                    continue;
                };
                candidates.extend(
                    found
                        .into_iter()
                        .filter(|hit| hit.kind == crate::text::Kind::Weapon)
                        .map(|hit| (hit.name, i64::from(hit.id))),
                );
                if !candidates.is_empty() {
                    break;
                }
            }
        }

        candidates
            .into_iter()
            .take(3)
            .filter_map(|(name, id)| {
                Some(crate::ask::Armed {
                    weapon: regulation.weapon(id)?,
                    hits: regulation.attack_with(id, stats),
                    // The same weapon at what they have now, so the difference
                    // is subtracted here rather than in prose. Told to ask
                    // twice and compare, a model asked once and read the new
                    // total as the gain: "ten points give +57" where the gain
                    // was eight.
                    now: regulation.attack_with(id, now),
                    skill: skill_on(&regulation, &game_dir, mod_dir.as_deref(), id),
                    name,
                    modded: mod_dir.is_some(),
                })
            })
            .collect()
    });

    // What upgrading costs. The weapon is found the same way `weapon` finds it
    // — what is in their hands first, then by name — and the materials are
    // named out of the game's own text so the answer is in their language and
    // carries whatever a total conversion renamed.
    let climb_in = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let climb_asking = asking.clone();
    let climb_kept = ctx.app.app_data.clone();
    player.upgrading = Box::new(move |wanted| {
        let Some((game_dir, mod_dir)) = climb_in.clone() else {
            return Vec::new();
        };
        let Some(regulation) = crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())
        else {
            return Vec::new();
        };

        // Every item this installation names, for turning material ids into
        // words. Read off the disk so it works with the game closed too.
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let mut named: std::collections::HashMap<u32, String> =
            crate::library::everything(&game_dir, mod_dir.as_deref(), language)
                .iter()
                .map(|item| (item.id, item.name.clone()))
                .collect();
        // Same fallback as the talismans: without a mod there are no loose text
        // files, and a material shown as "item 10160" is not an answer. The
        // upgrade materials are goods.
        if named.is_empty() {
            named = crate::text::names(
                &climb_kept,
                game,
                Some(&game_dir),
                mod_dir.as_deref(),
                crate::text::Kind::Goods,
            )
            .into_iter()
            .collect();
        }

        let looking = wanted.trim().to_lowercase();
        // What they are carrying answers "my weapon" without a name.
        let mut candidates: Vec<(String, i64)> = Vec::new();
        if let Some(live) = crate::live::read(game) {
            if let Some(gear) = live.gear {
                for (name, id) in &gear.weapon_ids {
                    let lowered = name.to_lowercase();
                    if looking.is_empty()
                        || lowered.contains(&looking)
                        || looking.contains(&lowered)
                    {
                        candidates.push((name.clone(), *id));
                    }
                }
            }
        }
        if candidates.is_empty() && !looking.is_empty() {
            for spelling in spellings(&climb_asking, wanted) {
                let Some(found) = crate::text::look_up(game, &spelling, 6) else {
                    continue;
                };
                candidates.extend(
                    found
                        .into_iter()
                        .filter(|hit| hit.kind == crate::text::Kind::Weapon)
                        .map(|hit| (hit.name, i64::from(hit.id))),
                );
                if !candidates.is_empty() {
                    break;
                }
            }
        }
        // With the game shut, `look_up` reads nothing, so Reduvia — asked about
        // most — came back "not a weapon in this installation". Match on disk.
        if candidates.is_empty() {
            candidates =
                named_offline(&game_dir, mod_dir.as_deref(), "weapon", wanted);
        }

        candidates
            .into_iter()
            .take(3)
            .map(|(name, id)| crate::ask::Climb {
                weapon: name,
                steps: regulation
                    .upgrade_steps(id)
                    .into_iter()
                    .map(|step| {
                        let costs = step
                            .costs
                            .into_iter()
                            .map(|(item, count)| {
                                let called = u32::try_from(item)
                                    .ok()
                                    .and_then(|item| named.get(&item).cloned())
                                    // A material the text tables do not name is
                                    // said as its id rather than dropped: a step
                                    // with an ingredient missing reads complete.
                                    .unwrap_or_else(|| format!("item {item}"));
                                (called, count)
                            })
                            .collect();
                        (step.level, costs)
                    })
                    .collect(),
                modded: mod_dir.is_some(),
            })
            .collect()
    });

    player.spells_at = Box::new(move |int, fth, arc| {
        let Some((game_dir, mod_dir)) = reach_in.clone() else {
            return Vec::new();
        };
        let Some(regulation) = crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref())
        else {
            return Vec::new();
        };
        let Some(table) = regulation.table("Magic") else {
            return Vec::new();
        };

        // Every spell's name by its id. This used to read the running game and,
        // failing that, build the map with `(0u32, name)` — every entry under
        // the same key, so the whole thing collapsed to one and no spell could
        // be named at all. It was invisible while the game was up and total the
        // moment it was not, which is when somebody is most likely to be
        // planning a build.
        let named: std::collections::HashMap<u32, String> = crate::text::names(
            &reach_kept,
            game,
            Some(&game_dir),
            mod_dir.as_deref(),
            crate::text::Kind::Goods,
        )
        .into_iter()
        .collect();
        if named.is_empty() {
            return Vec::new();
        }

        let meets = |spell: &crate::formats::regulation::Spell| {
            spell.needs.iter().all(|(what, wanted)| {
                let have = if what.starts_with("intel") {
                    int
                } else if what.starts_with("faith") {
                    fth
                } else {
                    arc
                };
                have >= *wanted
            })
        };

        let mut out: Vec<crate::ask::Cast> = table
            .ids()
            .filter_map(|id| {
                let spell = regulation.spell(id)?;
                // A spell that asks for nothing is castable by anyone, and
                // dropping those made "what can I cast" quietly incomplete.
                if !meets(&spell) {
                    return None;
                }
                let name = named.get(&u32::try_from(id).ok()?)?.clone();
                Some(crate::ask::Cast { name, spell, modded: mod_dir.is_some() })
            })
            .collect();
        // Hardest first: what a player wants to see is what the points bought.
        out.sort_by_key(|cast| {
            std::cmp::Reverse(cast.spell.needs.iter().map(|(_, value)| *value).max().unwrap_or(0))
        });
        out
    });

    // What is around them, out of the same table the markers use.
    let near_dirs = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| (install.game_dir.clone(), pin_dir.clone()));
    let near_data = ctx.app.app_data.clone();
    player.nearby = Box::new(move |map_x, map_y| {
        let Some((game_dir, mod_dir)) = near_dirs.clone() else {
            return Vec::new();
        };
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let places = crate::places::everywhere(crate::places::Where {
            game,
            game_dir: &game_dir,
            mod_dir: mod_dir.as_deref(),
            language,
            keep_in: Some(&near_data),
        });

        let mut found: Vec<(String, f64)> = places
            .iter()
            .map(|place| {
                let across = f64::from(place.map_x - map_x);
                let down = f64::from(place.map_y - map_y);
                (place.name.clone(), (across * across + down * down).sqrt())
            })
            .collect();
        found.sort_by(|a, b| a.1.total_cmp(&b.1));
        found.truncate(8);
        found
    });

    // The running game first, because that is the copy the player is looking
    // at. Off the disk when it is not, so a question asked before the game
    // starts gets the same answer as one asked during it.
    let shelf_game = ctx
        .app
        .active_install(game)
        .ok()
        .map(|install| install.game_dir.clone());
    let shelf_mod = pin_dir.clone();
    let shelf_asking = asking.clone();

    // Everything the game has written, not only what it has named. Straight off
    // the disk rather than through the running game: the text archives hold all
    // forty-four tables, where the live read only exposes the name lookups, and
    // the tutorials and menu entries this exists for are in the other forty.
    let written_in = shelf_game.clone();
    let written_mod = shelf_mod.clone();
    player.written = Box::new(move |words| {
        let Some(game_dir) = written_in.as_ref() else {
            return (Vec::new(), None);
        };
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus")
            .to_string();
        crate::text::search_saying_how(game_dir, written_mod.as_deref(), &language, words)
    });

    // What their endurance lets them carry, off the game's own curve.
    let load_in = shelf_game.clone();
    let load_mod = shelf_mod.clone();
    player.load = Box::new(move |endurance| {
        let game_dir = load_in.as_ref()?;
        let regulation =
            crate::formats::regulation::installed(game, game_dir, load_mod.as_deref())?;
        regulation.can_carry(endurance)
    });

    // Every weapon that builds one ailment, ranked. The other half of what the
    // armour ranking does, and the answer to a question that used to cost
    // seventeen lookups.
    let arming = shelf_game.clone();
    let arming_mod = shelf_mod.clone();
    let arming_asking = asking.clone();
    player.arsenal = Box::new(move |wanted| {
        let Some(game_dir) = arming.as_ref() else {
            return Vec::new();
        };
        let Some(regulation) =
            crate::formats::regulation::installed(game, game_dir, arming_mod.as_deref())
        else {
            return Vec::new();
        };
        // In whatever they called it, then in the tables' own word.
        let said = wanted.trim().to_lowercase();
        let Some(ailment) = crate::formats::regulation::buildup::AILMENTS
            .iter()
            .map(|(name, _)| *name)
            .find(|name| said.contains(name) || name.contains(&said))
            .or_else(|| {
                // The words a player types for them, in the languages the
                // launcher already commits to.
                [
                    ("bleed", ["кровот", "кров", "blut", "sangr"]),
                    ("poison", ["яд", "отрав", "gift", "veneno"]),
                    ("rot", ["гнил", "гние", "скверн", "fäul"]),
                    ("curse", ["прокл", "порч", "fluch", "maldic"]),
                    // The three that were unreadable until the paramdef walk
                    // was fixed. Their words go in with them; a figure that can
                    // be read and cannot be asked for is no better than unread.
                    ("frost", ["обморож", "мороз", "freeze", "kälte"]),
                    ("sleep", ["сон", "сонл", "schlaf", "sueñ"]),
                    ("madness", ["безум", "wahn", "locur", "madn"]),
                ]
                .iter()
                .find(|(_, words)| words.iter().any(|word| said.starts_with(word)))
                .map(|(name, _)| *name)
            })
        else {
            return Vec::new();
        };

        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(game_dir, arming_mod.as_deref(), language)
                .iter()
                .filter(|one| one.what == "weapon")
                .map(|one| (one.id, one.name.clone()))
                .collect();
        let _ = &arming_asking;

        let Some(table) = regulation.table("EquipParamWeapon") else {
            return Vec::new();
        };
        let sorted = crate::library::tables_for(game_dir, arming_mod.as_deref(), language);
        let sorted = sorted.get("GR_MenuText");
        let mut out: Vec<(String, i32, f32, String)> = table
            .ids()
            // Base rows only: an upgraded copy is the same weapon and would
            // fill the list with itself.
            .filter(|id| id % 100 == 0)
            .filter_map(|id| {
                let name = named.get(&u32::try_from(id).ok()?)?;
                let builds = regulation
                    .ailments(id)
                    .into_iter()
                    .find(|(what, _)| *what == ailment)
                    .map(|(_, value)| value)?;
                let held = regulation.weapon(id);
                let weight = held.as_ref().map_or(0.0, |held| held.weight);
                // What sort of thing it is, in the game's own word where there
                // is one, so an arrow is visibly an arrow and not just the
                // lightest thing in the list.
                let sort = held
                    .as_ref()
                    .and_then(|held| held.sort)
                    .map(|(kind, english)| {
                        crate::formats::regulation::sort::menu_id(kind)
                            .and_then(|at| Some(sorted?.get(&at)?.trim().to_string()))
                            .unwrap_or_else(|| english.to_string())
                    })
                    .unwrap_or_default();
                Some((name.clone(), builds, weight, sort))
            })
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.2.total_cmp(&b.2)));
        out.dedup_by(|a, b| a.0 == b.0);
        out
    });

    // Every weapon of one class, ranked by one figure. See `ask::Armoury` for
    // the two questions in a single battery that this exists because of.
    let classed = shelf_game.clone();
    let classed_mod = shelf_mod.clone();
    player.armed = Box::new(move |wanted, by| {
        use crate::formats::regulation::sort;
        let game_dir = classed.as_ref()?;
        let regulation =
            crate::formats::regulation::installed(game, game_dir, classed_mod.as_deref())?;
        let table = regulation.table("EquipParamWeapon")?;
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");

        // In English first, because that is the language the tool is called in
        // whatever the player typed. Then in the game's OWN words, which costs
        // nothing to support and means "катана" and "Katana" both work without
        // a single translation being written down here.
        let said = wanted.trim().to_lowercase();
        let mut wanted_sorts = sort::named(&said);
        let menu = crate::library::tables_for(game_dir, classed_mod.as_deref(), language);
        let menu = menu.get("GR_MenuText");
        if wanted_sorts.is_empty() {
            if let Some(menu) = menu {
                // Longest name first: "Большой щит" must not lose to "щит".
                let mut theirs: Vec<(usize, u16)> = sort::ALL
                    .iter()
                    .filter_map(|(kind, id, _)| {
                        let called = menu.get(id)?.trim().to_lowercase();
                        (!called.is_empty()
                            && (said.contains(&called) || called.contains(&said)))
                        .then_some((called.chars().count(), *kind))
                    })
                    .collect();
                theirs.sort_by_key(|(length, _)| std::cmp::Reverse(*length));
                // Only the longest match and its equals, so asking for a
                // greatshield does not also fetch every buckler in the game.
                if let Some((best, _)) = theirs.first().copied() {
                    wanted_sorts =
                        theirs.iter().filter(|(l, _)| *l == best).map(|(_, k)| *k).collect();
                }
            }
        }
        if wanted_sorts.is_empty() {
            return None;
        }

        let named: std::collections::HashMap<u32, String> =
            crate::library::everything(game_dir, classed_mod.as_deref(), language)
                .iter()
                .filter(|one| one.what == "weapon")
                .map(|one| (one.id, one.name.clone()))
                .collect();

        // What to rank on. A shield asked about without a figure is asked about
        // for its block, and everything else for what it hits for.
        let guarding = wanted_sorts.iter().copied().all(sort::blocks);
        let by = by.trim().to_lowercase();
        let measure = if by.is_empty() {
            if guarding { "physical".to_string() } else { "damage".to_string() }
        } else if ["weight", "вес", "gewicht", "peso", "light", "lightest"]
            .iter()
            .any(|word| by.contains(word))
        {
            "weight".to_string()
        } else if ["boost", "stability", "стабильн", "guard"]
            .iter()
            .any(|word| by.contains(word))
        {
            "boost".to_string()
        } else if let Some(ailment) = crate::formats::regulation::buildup::AILMENTS
            .iter()
            .map(|(name, _)| *name)
            .find(|name| by.contains(name))
        {
            ailment.to_string()
        } else if let Some(kind) = crate::formats::regulation::kind::named(&by) {
            kind.to_string()
        } else {
            // An unreadable measure is not a reason to refuse the class; the
            // list is still what they asked for.
            if guarding { "physical".to_string() } else { "damage".to_string() }
        };

        let mut best: Vec<crate::ask::OfSort> = table
            .ids()
            // Base rows only: +7 of a thing is the same thing.
            .filter(|id| id % 100 == 0)
            .filter_map(|id| {
                let kind = table.u16(id, sort::AT)?;
                if !wanted_sorts.contains(&kind) {
                    return None;
                }
                let name = named.get(&u32::try_from(id).ok()?)?;
                let held = regulation.weapon(id)?;
                let blocks = held.blocks.clone().unwrap_or_default();
                let damage: u16 = held.damage.iter().map(|(_, value)| value).sum();
                let figure = match measure.as_str() {
                    "weight" => -held.weight,
                    "boost" => f32::from(held.boost.unwrap_or(0)),
                    "damage" => f32::from(damage),
                    other => {
                        if let Some(built) =
                            held.ailments.iter().find(|(what, _)| what == other)
                        {
                            built.1 as f32
                        } else if guarding {
                            blocks
                                .iter()
                                .find(|(what, _)| what == other)
                                .map_or(0.0, |(_, value)| *value)
                        } else {
                            held.damage
                                .iter()
                                .find(|(what, _)| what == other)
                                .map_or(0.0, |(_, value)| f32::from(*value))
                        }
                    }
                };
                Some(crate::ask::OfSort {
                    name: name.clone(),
                    sort: menu
                        .and_then(|menu| menu.get(&sort::menu_id(kind)?))
                        .map(|called| called.trim().to_string())
                        .or_else(|| sort::english(kind).map(str::to_string))
                        .unwrap_or_default(),
                    weight: held.weight,
                    needs: held.needs.clone(),
                    figure,
                    // Block figures belong to SHIELDS, and giving them to
                    // everything hid what a weapon hits for.
                    //
                    // Every weapon in this game has guard values — a great
                    // hammer blocks 68% of physical — so this was never empty,
                    // and `a_class_of` prints block INSTEAD of damage whenever
                    // it is filled. Asked which great hammer hits hardest, the
                    // listing came back with 239 hammers showing block
                    // percentages and no damage at all, and the answer read it
                    // correctly and said: "в этой сборке большие молоты
                    // переделаны в щиты — урон не прописан, только блок". Every
                    // one of those 239 has damage — 200 physical, 217 lightning
                    // — and none of it was printed. The model was not
                    // inventing; the tool told it that.
                    //
                    // Kept when the class really is a shield, and when guard
                    // boost is what was asked for, which is the one time a
                    // weapon's block is the question.
                    blocks: if guarding || measure == "boost" { blocks } else { Vec::new() },
                    boost: held.boost,
                    damage,
                    ailments: held.ailments.clone(),
                })
            })
            .collect();
        best.sort_by(|a, b| b.figure.total_cmp(&a.figure).then_with(|| a.weight.total_cmp(&b.weight)));
        best.dedup_by(|a, b| a.name == b.name);
        let all = best.len();
        best.truncate(12);

        let english = wanted_sorts
            .iter()
            .filter_map(|kind| sort::english(*kind))
            .collect::<Vec<_>>()
            .join(", ");
        let called = wanted_sorts
            .iter()
            .filter_map(|kind| Some(menu?.get(&sort::menu_id(*kind)?)?.trim().to_string()))
            .collect::<Vec<_>>()
            .join(", ");
        Some(crate::ask::Sorted {
            called: if called.is_empty() { english.clone() } else { called },
            english,
            by: measure,
            all,
            best,
        })
    });

    // How far weapons upgrade here — a fact, computed, because it is one that
    // gets invented. See `ask::Player::upgrades_to`.
    if let (Some(game_dir), regulation_mod) = (shelf_game.clone(), shelf_mod.clone()) {
        if let Some(regulation) =
            crate::formats::regulation::installed(game, &game_dir, regulation_mod.as_deref())
        {
            player.upgrades_to = regulation.upgrade_ceilings();
        }

        // The game's own word for each attribute, so the player's line carries
        // it and nothing has to be translated on the way out. An answer turned
        // "Faith (FTH) 22" into "Фея 22" — a fairy — while quoting the right
        // number, and the abbreviation was already there to stop exactly that.
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let menu = crate::library::tables_for(&game_dir, regulation_mod.as_deref(), language);
        if let Some(menu) = menu.get("GR_MenuText") {
            player.attribute_words = crate::formats::regulation::attribute::MENU
                .iter()
                .filter_map(|(english, at)| {
                    let word = menu.get(at)?.trim();
                    // "Стойкость(END)" — the parenthetical is the proof of
                    // which attribute it is and is not part of the name.
                    let word = word.split('(').next().unwrap_or(word).trim();
                    (!word.is_empty() && !word.eq_ignore_ascii_case(english))
                        .then(|| ((*english).to_string(), word.to_string()))
                })
                .collect();
        }
    }

    // Ashes of war. They live in EquipParamGem — the files call an ash a gem —
    // and point at the SwordArtsParam rows already read for a weapon's skill.
    let honing = shelf_game.clone();
    let honing_mod = shelf_mod.clone();
    player.ashes = Box::new(move || {
        let Some(game_dir) = honing.as_ref() else {
            return Vec::new();
        };
        let Some(regulation) =
            crate::formats::regulation::installed(game, game_dir, honing_mod.as_deref())
        else {
            return Vec::new();
        };
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let tables = crate::library::tables_for(game_dir, honing_mod.as_deref(), language);
        let Some(named) = tables.get("ArtsName") else {
            return Vec::new();
        };

        let mut out: Vec<crate::ask::WarAsh> = regulation
            .ashes_of_war()
            .into_iter()
            .filter_map(|(_, skill)| {
                let skill = skill?;
                let name = named.get(&skill.text)?.trim().to_string();
                (!name.is_empty())
                    .then(|| crate::ask::WarAsh { name, costs: skill.costs })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out.dedup_by(|a, b| a.name == b.name);
        out
    });

    // The starting classes. Two param tables joined in `regulation::classes`,
    // then joined again here to the menu text that names them and to the
    // catalogue that names what they are holding.
    let starting = shelf_game.clone();
    let starting_mod = shelf_mod.clone();
    player.classes = Box::new(move || {
        let Some(game_dir) = starting.as_ref() else {
            return Vec::new();
        };
        let Some(regulation) =
            crate::formats::regulation::installed(game, game_dir, starting_mod.as_deref())
        else {
            return Vec::new();
        };
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let tables = crate::library::tables_for(game_dir, starting_mod.as_deref(), language);
        let Some(menu) = tables.get("GR_MenuText") else {
            return Vec::new();
        };
        // What it is holding, by name where the catalogue has it. Worth the
        // join: the difference between two classes on the same points is
        // mostly the weapon, so a list of bare ids answers nothing.
        let gear_names: std::collections::HashMap<u32, String> =
            crate::library::everything(game_dir, starting_mod.as_deref(), language)
                .iter()
                .map(|one| (one.id, one.name.clone()))
                .collect();

        regulation
            .classes()
            .into_iter()
            .filter_map(|class| {
                let name = menu.get(&class.name)?.trim().to_string();
                if name.is_empty() {
                    return None;
                }
                Some(crate::ask::StartingClass {
                    name,
                    level: class.level,
                    attributes: class.attributes,
                    gear: class
                        .gear
                        .iter()
                        .filter_map(|(where_, id)| {
                            let named = gear_names.get(&u32::try_from(*id).ok()?)?;
                            Some(format!("{where_} {named}"))
                        })
                        .collect(),
                })
            })
            .collect()
    });

    // The crystal tears for the physick. Same route as a talisman: the goods
    // row points at an effect, and `what_an_effect_does` reads it.
    let mixing = shelf_game.clone();
    let mixing_mod = shelf_mod.clone();
    player.tears = Box::new(move || {
        let Some(game_dir) = mixing.as_ref() else {
            return Vec::new();
        };
        let Some(regulation) =
            crate::formats::regulation::installed(game, game_dir, mixing_mod.as_deref())
        else {
            return Vec::new();
        };
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let described: std::collections::HashMap<u32, (String, Option<String>)> =
            crate::library::everything(game_dir, mixing_mod.as_deref(), language)
                .iter()
                .map(|item| (item.id, (item.name.clone(), item.effect.clone())))
                .collect();

        let mut out: Vec<crate::ask::Tear> = regulation
            .tears()
            .into_iter()
            .filter_map(|(id, gives, changes, adds)| {
                let (name, effect) = described.get(&u32::try_from(id).ok()?)?;
                Some(crate::ask::Tear {
                    name: name.clone(),
                    effect: effect.clone(),
                    gives,
                    changes,
                    adds,
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out.dedup_by(|a, b| a.name == b.name);
        out
    });

    // The spirit ashes. Named out of this installation, because the answer
    // they replace was five names remembered from the base game.
    let summoning = shelf_game.clone();
    let summoning_mod = shelf_mod.clone();
    player.spirits = Box::new(move || {
        let Some(game_dir) = summoning.as_ref() else {
            return Vec::new();
        };
        let Some(regulation) =
            crate::formats::regulation::installed(game, game_dir, summoning_mod.as_deref())
        else {
            return Vec::new();
        };
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let shelf = crate::library::everything(game_dir, summoning_mod.as_deref(), language);
        let described: std::collections::HashMap<u32, (String, Option<String>)> = shelf
            .iter()
            .map(|item| (item.id, (item.name.clone(), item.effect.clone())))
            .collect();
        // The upgrade material has to be looked up among GOODS ALONE.
        //
        // Ids are only unique WITHIN a table and the catalogue merges six of
        // them — weapons, armour, talismans, goods, gems and skills — keyed on
        // the id by itself. So they collide, and the map keeps whichever was
        // read last. Measured: material id 10000 came back as "Пепел Войны:
        // Коготь льва", an ash of WAR out of the gem table, and the listing
        // would have printed that as the thing you go and find to upgrade a
        // spirit ash. A confidently wrong name is worse than the silence it
        // replaced, and it is the same confusion the answer itself fell into.
        let goods: std::collections::HashMap<u32, String> = shelf
            .iter()
            .filter(|item| item.what == "item")
            .map(|item| (item.id, item.name.clone()))
            .collect();

        let mut out: Vec<crate::ask::Ash> = regulation
            .spirits()
            .into_iter()
            .filter_map(|summon| {
                let (name, effect) = described.get(&u32::try_from(summon.id).ok()?)?;
                // The upgrade material by NAME. The id was read and thrown
                // away; `described` already holds every item's name, so this
                // is one lookup rather than a new reader.
                // Through the material SET — see `Regulation::ingredients`.
                // The field names a row of EquipMtrlSetParam, not an item, and
                // reading it as an item gave "Пепел Войны: Коготь льва" one way
                // and "Осколок стекла" the other. Resolved properly it is
                // "Могильный ландыш [1]", which is what a player actually goes
                // and finds.
                let material = summon.material.map(i64::from).map(|set| {
                    regulation
                        .ingredients(set)
                        .into_iter()
                        .filter_map(|(item, count)| {
                            let called = goods.get(&u32::try_from(item).ok()?)?;
                            Some(if count > 1 {
                                format!("{called} ×{count}")
                            } else {
                                called.clone()
                            })
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                let material = material.filter(|named| !named.is_empty());
                Some(crate::ask::Ash {
                    name: name.clone(),
                    effect: effect.clone(),
                    material,
                    summon,
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out.dedup_by(|a, b| a.name == b.name);
        out
    });

    // A whole armour set, found from any one of its pieces and totalled here.
    // See `ask::Suited` for the wiki answer that made this necessary.
    let suited = shelf_game.clone();
    let suited_mod = shelf_mod.clone();
    let suited_asking = asking.clone();
    player.suit = Box::new(move |wanted| {
        let game_dir = suited.as_ref()?;
        let regulation =
            crate::formats::regulation::installed(game, game_dir, suited_mod.as_deref())?;
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let everything = crate::library::everything(game_dir, suited_mod.as_deref(), language);
        let armour: Vec<(u32, String)> = everything
            .iter()
            .filter(|one| one.what == "armour")
            .map(|one| (one.id, one.name.clone()))
            .collect();
        if armour.is_empty() {
            return None;
        }

        // Whichever spelling of their word finds a piece. The set is then
        // whatever shares that piece's id/1000, which is the whole point: they
        // name one thing and get the set it belongs to.
        let said = spellings(&suited_asking, wanted)
            .into_iter()
            .find(|spelling| {
                let low = spelling.to_lowercase();
                low.len() > 2 && armour.iter().any(|(_, name)| name.to_lowercase().contains(&low))
            })
            .unwrap_or_else(|| wanted.to_string())
            .to_lowercase();

        // The set with the most matching pieces wins, so a word appearing once
        // somewhere else does not beat the set actually named.
        let mut tally: std::collections::HashMap<u32, usize> = Default::default();
        for (id, name) in &armour {
            if name.to_lowercase().contains(&said) {
                *tally.entry(crate::formats::regulation::slot::set_of(*id)).or_default() += 1;
            }
        }
        let (group, _) = tally.into_iter().max_by_key(|(group, hits)| (*hits, *group))?;

        let mut all: Vec<(u32, String)> = armour
            .iter()
            .filter(|(id, _)| crate::formats::regulation::slot::set_of(*id) == group)
            .cloned()
            .collect();
        // Head, body, arms, legs — the order the game wears them in, which is
        // the hundreds digit.
        all.sort_by_key(|(id, _)| *id);
        all.dedup_by(|a, b| a.0 == b.0);

        let mut pieces = Vec::new();
        let mut names = Vec::new();
        let (mut weight, mut poise) = (0.0, 0.0);
        for (id, name) in all {
            let Some(piece) = regulation.armour(i64::from(id)) else { continue };
            weight += piece.weight;
            poise += piece.poise.unwrap_or(0.0);
            pieces.push(piece);
            names.push(name);
        }
        if pieces.is_empty() {
            return None;
        }

        // The set's own name: the longest run of words every piece shares. Read
        // out of the names rather than written down, so it comes back in the
        // player's language and needs no table of set names to go stale.
        let called = shared_words(&names).unwrap_or_else(|| wanted.to_string());
        Some(crate::ask::Suited { called, pieces, names, weight, poise, carrying: None })
    });

    // One talisman's figures, by id, for the item lookup — see `ask::Player`
    // for why the same numbers live in two places on purpose.
    let charmed = shelf_game.clone();
    let charmed_mod = shelf_mod.clone();
    player.charm = Box::new(move |id| {
        let game_dir = charmed.as_ref()?;
        let regulation =
            crate::formats::regulation::installed(game, game_dir, charmed_mod.as_deref())?;
        regulation.charm(i64::from(id))
    });

    // What they carry now and the most they could, as one pair, so a tool that
    // wants to weigh a set against their limit does not have to do the sum in
    // prose. Read live: both halves change the moment they take something off.
    let hauling_in = shelf_game.clone();
    let hauling_mod = shelf_mod.clone();
    player.carrying = Box::new(move || {
        let game_dir = hauling_in.as_ref()?;
        let regulation =
            crate::formats::regulation::installed(game, game_dir, hauling_mod.as_deref())?;
        let live = crate::live::read(game)?;
        let gear = live.gear.as_ref()?;
        let language = crate::language::status(game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let named: std::collections::HashMap<String, u32> =
            crate::library::everything(game_dir, hauling_mod.as_deref(), language)
                .iter()
                .map(|one| (one.name.clone(), one.id))
                .collect();
        let weigh = |name: &String| -> f32 {
            named
                .get(name)
                .map(|id| i64::from(*id))
                .and_then(|id| {
                    regulation
                        .armour(id)
                        .map(|piece| piece.weight)
                        .or_else(|| regulation.weapon(id).map(|held| held.weight))
                })
                .unwrap_or(0.0)
        };
        let now: f32 = gear
            .armour
            .iter()
            .map(|(_, name)| weigh(name))
            .chain(gear.weapons.iter().map(weigh))
            .sum();
        let endurance = live
            .stats
            .iter()
            .find(|(what, _)| what.starts_with("Endurance"))
            .map(|(_, value)| *value)?;
        Some((now, regulation.can_carry(endurance)?))
    });

    player.catalogue = Box::new(move |query| {
        // Under the name asked for, then under whatever else the same thing is
        // called. Without this the catalogue was the one lookup that stayed
        // monolingual: asked what Radagon's Scarseal does on a Russian
        // installation, it came back "not in the game" and the question died.
        let live = spellings(&shelf_asking, query).into_iter().find_map(|spelling| {
            let found = crate::text::look_up(game, &spelling, 6)?;
            (!found.is_empty()).then_some(found)
        });
        if let Some(found) = live {
            return Some(
                found
                    .into_iter()
                    .map(|item| crate::ask::Catalogued {
                        what: item.kind.what().to_string(),
                        id: item.id,
                        name: item.name,
                        effect: item.effect,
                        caption: item.caption,
                    })
                    .collect(),
            );
        }

        let game_dir = shelf_game.clone()?;
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let shelf = crate::library::everything(&game_dir, shelf_mod.as_deref(), language);
        if shelf.is_empty() {
            return None;
        }
        // The same courtesy off the disk as from the running game.
        let asked = spellings(&shelf_asking, query)
            .into_iter()
            .find(|spelling| !crate::library::look_up(&shelf, spelling, 1).is_empty())
            .unwrap_or_else(|| query.to_string());
        Some(
            crate::library::look_up(&shelf, &asked, 6)
                .into_iter()
                .map(|item| crate::ask::Catalogued {
                    what: item.what,
                    id: item.id,
                    name: item.name,
                    effect: item.effect,
                    caption: item.caption,
                })
                .collect(),
        )
    });

    // Pinning a place on the map. Everything the launcher needs is gathered
    // here rather than inside the closure, because by the time a model asks for
    // this the settings lock is somebody else's problem.
    let install = ctx.app.active_install(game).ok();
    let game_dir = install.as_ref().map(|i| i.game_dir.clone());
    let saves_dir = install.as_ref().and_then(|i| i.appdata_dir());
    let app_data = ctx.app.app_data.clone();
    let pin_tables = pin_dir.clone();
    player.mark = Box::new(move |place, character| {
        let (Some(game_dir), Some(saves_dir)) = (game_dir.clone(), saves_dir.clone()) else {
            return Err("The game has not been located, so there is no save to write to.".into());
        };
        if crate::unlock::running_pid(game.executable()).is_some() {
            return Err("The game is running. It keeps the map in memory and writes the whole \
                        save on its own schedule, so anything added now would be thrown away — \
                        this has to wait until they quit."
                .into());
        }

        let Some(save) = newest_save(&saves_dir) else {
            return Err("No save file could be found for this game, which is a fault at this \
                        end rather than an empty map — do not report it as one."
                .into());
        };
        let slots = crate::saves::read_markers(&save).map_err(|e| e.to_string())?;
        let chosen = pick_character(&slots, character)?;

        // Where the place is, in the game they actually have installed.
        let language = crate::language::status(&game_dir)
            .current
            .as_deref()
            .and_then(crate::language::locale_folder)
            .unwrap_or("engus");
        let places = crate::places::everywhere(crate::places::Where {
            game,
            game_dir: &game_dir,
            mod_dir: pin_tables.as_deref(),
            language,
            keep_in: Some(&app_data),
        });
        if places.is_empty() {
            return Err("The installed game's own place names could not be read, so there is \
                        nothing to look the place up in."
                .into());
        }
        let Some(found) = crate::places::find(&places, place) else {
            return Err(format!(
                "The map has no place called \"{place}\". Legacy dungeons are drawn on their own \
                 maps and are not on this one."
            ));
        };

        let marker = crate::saves::add_marker(
            &app_data,
            game,
            &save,
            chosen.slot,
            found.map_x,
            found.map_y,
            0,
        )
        .map_err(|e| e.to_string())?;

        Ok(format!(
            "Pinned {} on {}'s map, marker {}. The save was backed up first.",
            found.name, chosen.name, marker.id
        ))
    });

    // Reading the map, and clearing it. Same save and same character as
    // placing, so the two agree about whose map is whose.
    let read_game = install.as_ref().map(|i| i.game_dir.clone());
    let read_saves = install.as_ref().and_then(|i| i.appdata_dir());
    let read_data = ctx.app.app_data.clone();
    let read_tables = pin_dir;
    player.pins = Box::new(move |character, remove| {
        let (Some(game_dir), Some(saves_dir)) = (read_game.clone(), read_saves.clone()) else {
            return Err("The game has not been located, so there is no save to read.".into());
        };
        let Some(save) = newest_save(&saves_dir) else {
            return Err("No save file could be found for this game, which is a fault at this \
                        end rather than an empty map — do not report it as one."
                .into());
        };
        let slots = crate::saves::read_markers(&save).map_err(|e| e.to_string())?;

        // Reading is harmless, so a save with several characters shows all of
        // them rather than asking which. Asked how many markers they had, the
        // assistant listed some and then said it could not count them — it had
        // been refused the other character and reported that as not knowing.
        // Removing still has to name one: that writes.
        if remove.is_none() && character.is_none() {
            let named: Vec<&crate::saves::SlotMarkers> = slots
                .iter()
                .filter(|slot| !slot.name.trim().is_empty())
                .collect();
            if named.len() > 1 {
                let mut said = String::new();
                for slot in named {
                    said.push_str(&describe_map(slot, &game_dir, read_tables.as_deref()));
                    said.push('\n');
                }
                return Ok(said);
            }
        }

        let chosen = pick_character(&slots, character)?;

        if let Some(what) = remove {
            if crate::unlock::running_pid(game.executable()).is_some() {
                return Err("The game is running, and it would write over this. It has to be \
                            closed first."
                    .into());
            }
            let id = if what.eq_ignore_ascii_case("all") {
                None
            } else {
                Some(what.parse::<i32>().map_err(|_| {
                    format!("\"{what}\" is not a marker id, and not the word `all` either.")
                })?)
            };
            let gone = crate::saves::remove_markers(&read_data, game, &save, chosen.slot, id)
                .map_err(|e| e.to_string())?;
            return Ok(match (gone, id) {
                (0, Some(id)) => format!("{}'s map has no marker {id}.", chosen.name),
                (0, None) => format!("{}'s map was already empty.", chosen.name),
                (1, _) => format!("Taken off {}'s map. The save was backed up first.", chosen.name),
                (many, _) => format!(
                    "Cleared {many} markers off {}'s map. The save was backed up first.",
                    chosen.name
                ),
            });
        }

        Ok(describe_map(chosen, &game_dir, read_tables.as_deref()))
    });

    player
}

/// The same question, reported as it is answered.
async fn ask_stream(State(ctx): State<Ctx>, Json(body): Json<AskBody>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
    let began = std::time::Instant::now();
    let player = player_state(&ctx, body.edition.as_deref(), &body.question);
    crate::ask::note_timing("player_state", began);

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

/// Searches every drive for an edition, reusing the install scan's progress.
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

// ---------------------------------------------------------------------------
// Map markers
// ---------------------------------------------------------------------------

async fn markers_read(Query(q): Query<PathQ>) -> Response {
    out(crate::saves::read_markers(&q.path))
}

/// Everywhere the launcher knows how to pin, named in the player's language.
fn describe_map(
    slot: &crate::saves::SlotMarkers,
    game_dir: &std::path::Path,
    mod_dir: Option<&std::path::Path>,
) -> String {
    if slot.markers.is_empty() {
        return format!("{} has nothing pinned.\n", slot.name);
    }

    let language = crate::language::status(game_dir)
        .current
        .as_deref()
        .and_then(crate::language::locale_folder)
        .unwrap_or("engus");
    let places = crate::places::everywhere(crate::places::Where {
        game: crate::games::Game::EldenRing,
        game_dir,
        mod_dir,
        language,
        keep_in: None,
    });

    let mut said = format!(
        "{} has {} on the map:\n",
        slot.name,
        if slot.markers.len() == 1 {
            "one marker".to_string()
        } else {
            format!("{} markers", slot.markers.len())
        }
    );
    for marker in &slot.markers {
        let region = crate::markers::map_id(marker.x, marker.y).and_then(crate::live::place);
        let near = places
            .iter()
            .map(|place| {
                let across = f64::from(place.map_x - marker.x);
                let down = f64::from(place.map_y - marker.y);
                ((across * across + down * down).sqrt(), place)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));

        let where_it_is = match near {
            // Close enough to call it that spot rather than near it.
            Some((away, place)) if away < 100.0 => format!("at {}", place.name),
            Some((away, place)) => format!("{away:.0} from {}", place.name),
            None => format!("at {:.0}, {:.0} on the map", marker.x, marker.y),
        };
        match region {
            Some(region) => said.push_str(&format!("  {} — {region}, {where_it_is}\n", marker.id)),
            None => said.push_str(&format!("  {} — {where_it_is}\n", marker.id)),
        }
    }
    said.push_str(
        "The distances are in the map's own units, which are the world's — say roughly how far \
         rather than quoting the number as if it were metres.\n",
    );
    said
}

/// How much of each thing this installation holds, counted rather than recalled.
fn what_it_holds(
    game: crate::games::Game,
    game_dir: &std::path::Path,
    mod_dir: Option<&std::path::Path>,
) -> Option<String> {
    let regulation = crate::formats::regulation::installed(game, game_dir, mod_dir)?;

    let armour = regulation.table("EquipParamProtector").map(|table| {
        table
            .ids()
            .filter(|id| regulation.armour(*id).is_some_and(|piece| piece.weight > 0.0))
            .count()
    });
    let spells = regulation
        .table("Magic")
        .map(|table| table.ids().filter(|id| regulation.spell(*id).is_some()).count());

    let language = crate::language::status(game_dir)
        .current
        .as_deref()
        .and_then(crate::language::locale_folder)
        .unwrap_or("engus");
    let places = crate::places::everywhere(crate::places::Where {
        game,
        game_dir,
        mod_dir,
        language,
        keep_in: None,
    })
    .len();

    // The weapons caveat leads rather than trails, and is stated rather than
    // ordered. It used to end this block as "Say it cannot be counted rather
    // than giving one", and a lane translated that into Russian and handed it
    // to the player as the last line of its answer. A copied fact is at worst
    // redundant; a copied instruction is addressed to nobody.
    let mut said = String::from(
        "Counted out of this installation's own tables, for any question of the form \"how \
         many\" — these, never a figure off a wiki. How many WEAPONS there are is not among \
         them and cannot be counted honestly: the table carries every affinity, weapons only \
         enemies hold, and entries nobody can pick up, so no number in it means \"weapons in \
         this game\".\n",
    );
    if let Some(armour) = armour {
        said.push_str(&format!("  {armour} pieces of armour\n"));
    }
    if let Some(spells) = spells {
        said.push_str(&format!("  {spells} sorceries and incantations together\n"));
    }
    if places > 0 {
        said.push_str(&format!("  {places} named places on the world map\n"));
    }
    (said.lines().count() > 2).then_some(said)
}

/// The skill a weapon carries, named in the player's own language, with what
fn skill_on(
    regulation: &crate::formats::regulation::Regulation,
    game_dir: &std::path::Path,
    mod_dir: Option<&std::path::Path>,
    id: i64,
) -> Option<crate::ask::Skill> {
    let skill = regulation.skill_of(id)?;
    let language = crate::language::status(game_dir)
        .current
        .as_deref()
        .and_then(crate::language::locale_folder)
        .unwrap_or("engus");
    // Cached on the folder and its timestamp, so this costs one read per
    // installation however many weapons are asked about.
    let name = crate::library::everything(game_dir, mod_dir, language)
        .iter()
        .find(|one| one.what == "skill" && one.id == skill.text)
        .map(|one| one.name.clone())?;
    Some((name, skill.costs))
}

/// The five attributes the weapon tables care about, in their own order.
fn attributes_of(live: &crate::live::Live) -> [u32; 5] {
    let mut out = [0u32; 5];
    for (what, value) in &live.stats {
        let lowered = what.to_lowercase();
        let at = if lowered.contains("stre") {
            0
        } else if lowered.contains("dex") {
            1
        } else if lowered.contains("intel") {
            2
        } else if lowered.contains("fai") {
            3
        } else if lowered.contains("arc") {
            4
        } else {
            continue;
        };
        out[at] = *value;
    }
    out
}

/// Which wikis are on this machine, and how much of the game's own catalogue
fn what_is_mirrored(app_data: &std::path::Path) -> Option<String> {
    // Mirrored means its title index is on disk — the same file the search
    // reads, so this cannot claim a wiki the search will not find.
    let held: Vec<String> = crate::wiki::SOURCES
        .iter()
        .filter_map(|source| {
            let titles = crate::wiki::titles(app_data, source.id).len();
            (titles > 0).then(|| {
                let pages = crate::wiki::cached_page_count(app_data, source.id);
                // When it was taken, which the file's own timestamp says.
                let when = crate::wiki::taken_at(app_data, source.id)
                    .map(chrono::DateTime::<chrono::Local>::from)
                    .map(|at| {
                        let days = (chrono::Local::now() - at).num_days();
                        match days {
                            0 => format!(", taken today ({})", at.format("%Y-%m-%d")),
                            1 => format!(", taken yesterday ({})", at.format("%Y-%m-%d")),
                            _ => format!(", taken {days} days ago ({})", at.format("%Y-%m-%d")),
                        }
                    })
                    .unwrap_or_default();
                format!(
                    "{} ({titles} articles, {pages} of them read and kept{when})",
                    source.name
                )
            })
        })
        .collect();
    if held.is_empty() {
        return Some(
            "No wiki is mirrored onto this machine yet, so the wiki tools will find nothing \
             until one is. The launcher downloads them; say that rather than that the search \
             failed.\n"
                .into(),
        );
    }
    // The date beside each is there because "how fresh is my wiki" used to be
    // answered "that cannot be read" by an assistant standing on the file that
    // says so. That is why the rule below exists; the model does not need the
    // story, only the rule, and the story costs its length on every round of
    // every question. Same for the rest of this block — where a sentence
    // explains how a bug happened, the explanation belongs up here.
    Some(format!(
        "Mirrored here and searched locally: {}. The date beside each is when it was taken, and \
         that is the answer to \"how fresh is my wiki\". A mirror does not update itself; the \
         launcher re-downloads it on request.\n",
        held.join(", ")
    ))
}

/// Where the anti-cheat stands, and what that means for playing at all.
fn where_the_anticheat_stands(
    game: crate::games::Game,
    game_dir: &std::path::Path,
    seamless: bool,
) -> Option<String> {
    use crate::eac::EacState;
    let status = crate::eac::status(game, game_dir);
    let mods_need_it_off = seamless;

    let said = match status.state {
        EacState::NotPresent => return None,
        EacState::Bypassed => {
            let mut said = String::from(
                "Their Easy Anti-Cheat is BYPASSED — the launcher has replaced the shim, so \
                 every way of starting the game, Steam's Play button included, skips it. That is \
                 what mods need and it is already done: asked whether they have to turn the \
                 anti-cheat off, the answer is that it is off. Say so rather than explaining how \
                 to do it.\n",
            );
            said.push_str(
                "  The other half of that, which they will not think to ask: with it bypassed \
                 they should stay off the game's own online play. Mods loaded against the live \
                 servers is what gets an account banned. Seamless Co-op is a separate thing and \
                 is fine — it does not use those servers.\n",
            );
            said
        }
        EacState::Active => {
            let mut said = String::from(
                "Their Easy Anti-Cheat is ON. The launcher can turn it off for them, on the \
                 anti-cheat switch.\n",
            );
            if mods_need_it_off {
                said.push_str(
                    "  What they have installed will not load until it is: it sits in front of \
                     the game and the mod loader never gets to run. If they say a mod is doing \
                     nothing, this is the first thing to check and usually the whole answer.\n",
                );
            }
            said
        }
    };
    Some(said)
}

/// What the launcher is holding, for a question about undoing something.
fn in_round_numbers(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let size = bytes as f64;
    if size >= GB {
        format!("{:.1} GB", size / GB)
    } else if size >= 10.0 * MB {
        format!("{:.0} MB", size / MB)
    } else if size >= MB {
        format!("{:.1} MB", size / MB)
    } else {
        format!("{:.0} KB", (size / KB).max(1.0))
    }
}

fn what_is_kept(app_data: &std::path::Path, game: crate::games::Game) -> Option<String> {
    let kept = crate::saves::list_backups(app_data, game);
    if kept.is_empty() {
        return Some(
            "The launcher is holding NO save backups for this game yet — the first one is taken \
             the next time they launch. Do not say a restore is available.\n"
                .into(),
        );
    }
    // Newest first is how `list_backups` returns them, and the newest is the
    // one every question about this is really about.
    let newest = &kept[0];
    let how_many = kept.len();
    // The room they take, because it is in every record and was not being
    // passed on. Asked how much space the snapshots used, an answer said the
    // size "is not given, but in Roundtable snapshots take little space,
    // usually a few megabytes" — a guess, about their own disk, from inside
    // the program that knows the byte count of every one of them.
    let room: u64 = kept.iter().map(|one| one.size_bytes).sum();
    let mut said = format!(
        "The launcher is holding {how_many} save backup{} for this game, {} altogether. The \
         newest was taken {} — labelled \"{}\". Any of them can be put back from the saves \
         screen. Use these figures; do not produce a count, a date or a size of your own.\n",
        if how_many == 1 { "" } else { "s" },
        in_round_numbers(room),
        newest.created,
        newest.label,
    );
    if !newest.characters.is_empty() {
        said.push_str(&format!(
            "  What is in the newest one: {}.\n",
            newest.characters.join("; ")
        ));
    }
    Some(said)
}

/// Why the frame rate is what it is, in the words the player needs.
fn frame_rate_facts(
    ctx: &Ctx,
    game: crate::games::Game,
    install: &crate::game::Installation,
    framegen: bool,
) -> Option<String> {
    let mut roots = vec![install.game_dir.clone()];
    roots.extend(edition_roots(ctx, game));
    let perf = crate::perf::status(game, &roots, framegen);

    let mut said = String::new();
    if perf.exclusive_fullscreen {
        said.push_str(
            "Their game is in EXCLUSIVE FULLSCREEN. If they say they are getting 30, this is \
             why and nothing else is: the game asks Windows for a 60 Hz mode and holds vsync to \
             it, so one late frame halves the rate to exactly 30 until the next second. \
             Borderless has neither problem, and the launcher's own \"Smooth it out\" switches \
             to it. Say this before anything about drivers or background programs.\n",
        );
    }
    // The cap against the screen. A player on a fast monitor asking about the
    // frame rate is usually asking about this and does not know it — the game
    // ships locked at 60 whatever the display can do.
    let cap = ctx.app.settings.lock().unlock_fps;
    if let Some(display) = &perf.display {
        match cap {
            Some(fps) if fps > 60 => said.push_str(&format!(
                "Their display runs at {display}, and the launcher's Frame cap control is \
                 already set to raise the game's built-in 60 to {fps} once it is running.\n"
            )),
            _ => said.push_str(&format!(
                "Their display runs at {display} but the game ships locked to 60. The launcher \
                 raises that with its Frame cap control, on the Frame rate card — which is a \
                 different thing from \"Smooth it out\" beside it, and that one only changes \
                 graphics settings. Do not name one when you mean the other.\n"
            )),
        }
    }
    // What the picture settings are actually set to. Asked whether frame
    // generation was on and how much it added, a model answered entirely about
    // the frame cap and never said — while the mod's own config carries both,
    // with the mod's own words for them.
    {
        let settings = crate::erss::settings(&install.game_dir);
        let told = |key: &str| {
            settings.iter().find(|one| one.key == key).map(|one| {
                one.choices
                    .iter()
                    .find(|choice| choice.value == one.value)
                    .map_or_else(|| one.value.clone(), |choice| choice.label.clone())
            })
        };
        if let Some(mode) = told("FrameGeneration.FrameGenMode") {
            if mode.eq_ignore_ascii_case("off") {
                said.push_str(
                    "Frame generation is installed but switched OFF. Asked whether it is on, say \
                     so; asked what it would give, say it doubles the frames it is set to make \
                     and leave the rest to them trying it.\n",
                );
            } else {
                let many = told("DLSS-G.NumGenFrames").unwrap_or_else(|| "unknown".into());
                said.push_str(&format!(
                    "Frame generation is ON, using {mode}, set to {many} — that multiplier is \
                     how many frames come out for each one the game actually draws, so {many} \
                     means one generated frame between every pair of real ones. Those are the \
                     figures; do not invent a percentage or a frames-per-second gain, because \
                     what it works out to on their machine is not readable from here.\n"
                ));
            }
        }
    }

    // The upscaling mod keeps a frame limiter of its own, and it is the one
    // that wins. Two caps in two places is exactly the situation where an
    // answer sounds right and is wrong: the launcher's control was set to 180,
    // the mod's limiter was sitting at 90, and the player would have been told
    // their cap was 180 while the game handed them 90 all evening.
    let mod_cap = crate::erss::settings(&install.game_dir)
        .into_iter()
        .find(|setting| setting.key == "Renderer.MaxFPS")
        .and_then(|setting| setting.value.trim().parse::<f32>().ok())
        .filter(|limit| *limit > 0.0);
    if let Some(limit) = mod_cap {
        let limit = limit.round() as u32;
        match cap {
            // Where to change it matters: this one is not on the in-game
            // overlay. The overlay's Picture panel only carries settings with a
            // list of choices, and a frame limit is a number they type.
            Some(fps) if fps > limit => said.push_str(&format!(
                "But the upscaling mod has a frame limit of its own, set to {limit}, and that is \
                 the lower of the two — so they get {limit}, not the {fps} the launcher's control \
                 says. It is \"Frame limit\", on the \"DLSS and frame generation\" card in the \
                 launcher's main window, NOT on the in-game overlay. Tell them which of the two \
                 is actually holding them back rather than quoting the higher number.\n"
            )),
            _ => said.push_str(&format!(
                "The upscaling mod has its own frame limit, set to {limit}. Counted in finished \
                 frames, so with frame generation on it means {limit} on screen. It is \"Frame \
                 limit\", on the \"DLSS and frame generation\" card in the launcher's main \
                 window, NOT on the in-game overlay.\n"
            )),
        }
        // That card name is the only one anywhere in here, and it started
        // turning up as the answer to every question about where something
        // lives: asked where their co-op password was, a model sent them to the
        // "DLSS and frame generation" card to look for it.
        // Said as what the card is for, not as a list of what it is not.
        // Written the other way round — "the co-op password is not on it" — the
        // model repeated the negation to a player who had asked about the
        // password and had never heard of the card: "look at the DLSS and frame
        // generation card — no, it is not there". Naming a screen in order to
        // rule it out is still naming a screen.
        said.push_str(
            "  That card is for upscaling and frame settings, and it is the only screen you have \
             been named. For anything else, say what to do without naming where — and never \
             bring this card up in an answer that is not about upscaling or frame rate, not even \
             to rule it out.\n",
        );
    }
    if perf.improvable > 0 {
        said.push_str(&format!(
            "{} of their graphics settings are worse than this machine wants; \"Smooth it out\" \
             in the launcher changes them.\n",
            perf.improvable
        ));
    }
    // Something else already unlocking the frame rate. Two of them fighting is
    // a real cause of a game that hitches, and the launcher is the only thing
    // that can see the file sitting there.
    if let Some(unlocker) = &perf.unlocker {
        let name = unlocker
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "an unlocker".to_string());
        said.push_str(&format!(
            "There is a separate frame-rate unlocker in their game folder — {name} — from before \
             the launcher could do this itself. Two things rewriting the same cap is a known \
             cause of an uneven frame time. Worth removing.\n"
        ));
    }

    // Everything above is about the frame RATE. Stutter is a different
    // complaint with different causes, and the two were being conflated:
    // asked why the game was hitching, the assistant explained the 90 FPS
    // limit, which cannot cause a hitch — a cap makes frames evenly spaced,
    // which is the opposite.
    said.push_str(
        "All of the above is about how HIGH the frame rate is. If they say the game stutters, \
         hitches or freezes for a moment, that is a different thing and a cap is not the cause — \
         a limiter spaces frames out evenly, which is the opposite of a hitch. Do not answer a \
         stutter question with any of the frame-cap figures above. What can be seen from here \
         that does bear on it is named above where it applies: exclusive fullscreen, settings \
         above what this machine is rated for, and a second frame-rate unlocker in the folder. \
         Beyond those, the usual cause in this game is shaders compiling the first time through \
         an area, which settles on a second pass — say that is from experience rather than from \
         anything read off their machine.\n",
    );
    (!said.is_empty()).then_some(said)
}

/// What the launcher needs to know to try a name in more than one language.
#[derive(Clone)]
struct Asking {
    app_data: PathBuf,
    /// The wiki that belongs to whatever is installed.
    source: &'static str,
    /// The language its titles are translated into, when there is one.
    language: Option<&'static str>,
}

/// The wiki's code for the language the game is set to.
fn wiki_language(game_language: &str) -> Option<&'static str> {
    Some(match game_language {
        "russian" => "ru",
        "japanese" => "ja",
        "german" => "de",
        "french" => "fr",
        "spanish" | "latam" => "es",
        "italian" => "it",
        "polish" => "pl",
        "brazilian" => "pt",
        "koreana" => "ko",
        "schinese" | "tchinese" => "zh",
        _ => return None,
    })
}

/// A name, and the other names for the same thing.
fn spellings(asking: &Asking, wanted: &str) -> Vec<String> {
    let mut out = vec![wanted.to_string()];
    if let Some(language) = asking.language {
        out.extend(crate::wiki::also_called(
            &asking.app_data,
            asking.source,
            language,
            wanted,
        ));
    }
    out
}

/// An item by name, read off the disk when the game is shut.
///
/// `text::look_up` needs the running process, so with the game closed every
/// name search came back empty and even Reduvia read as "not in this game". The
/// catalogue is on disk, so match there instead — `what` is "weapon", "armour"
/// and so on. The language is the game's own, so a Russian name matches Russian.
fn named_offline(
    game_dir: &std::path::Path,
    mod_dir: Option<&std::path::Path>,
    what: &str,
    wanted: &str,
) -> Vec<(String, i64)> {
    let looking = wanted.trim().to_lowercase();
    if looking.is_empty() {
        return Vec::new();
    }
    let language = crate::language::status(game_dir)
        .current
        .as_deref()
        .and_then(crate::language::locale_folder)
        .unwrap_or("engus");
    crate::library::everything(game_dir, mod_dir, language)
        .iter()
        .filter(|item| item.what == what)
        .filter(|item| {
            let name = item.name.to_lowercase();
            name.contains(&looking) || looking.contains(&name)
        })
        .map(|item| (item.name.clone(), i64::from(item.id)))
        .take(3)
        .collect()
}

/// The save the game wrote last.
fn newest_save(dir: &std::path::Path) -> Option<PathBuf> {
    // The game keeps its saves in a folder named after the account, inside the
    // one this points at, and that name is different on every machine — so
    // look here and one level down, and nowhere else.
    let here = std::fs::read_dir(dir).into_iter().flatten().flatten();
    let mut everywhere: Vec<PathBuf> = Vec::new();
    for entry in here {
        let path = entry.path();
        if path.is_dir() {
            everywhere.extend(
                std::fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|inner| inner.path()),
            );
        } else {
            everywhere.push(path);
        }
    }

    let mut saves: Vec<PathBuf> = everywhere
        .into_iter()
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("sl2") | Some("co2")
            )
        })
        .collect();
    saves.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    saves.pop()
}

/// Whose map. One character is no question; several and the player has to say,
fn pick_character<'a>(
    slots: &'a [crate::saves::SlotMarkers],
    wanted: Option<&str>,
) -> std::result::Result<&'a crate::saves::SlotMarkers, String> {
    let named: Vec<&crate::saves::SlotMarkers> = slots
        .iter()
        .filter(|slot| !slot.name.trim().is_empty())
        .collect();
    let all = || {
        named
            .iter()
            .map(|slot| slot.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    match (named.len(), wanted) {
        (0, _) => Err("That save has no characters in it.".into()),
        (1, _) => Ok(named[0]),
        (_, Some(wanted)) => named
            .iter()
            .find(|slot| slot.name.to_lowercase().contains(&wanted.to_lowercase()))
            .copied()
            .ok_or_else(|| {
                format!("No character called \"{wanted}\". The save holds: {}.", all())
            }),
        (_, None) => Err(format!(
            "That save holds more than one character — {} — so nothing was read. Ask which of \
             them this is for and call this again with that name. Do not guess, and do not say \
             anything about what is or is not on a map you have not been shown.",
            all()
        )),
    }
}

/// What the launcher can pin, and where it read it from.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Places {
    places: Vec<crate::places::Place>,
    language: String,
    source: Option<PathBuf>,
}

async fn markers_places(State(ctx): State<Ctx>, Query(q): Query<GameQ>) -> Response {
    let root = ctx
        .app
        .settings
        .lock()
        .install_for(q.game)
        .map(|i| i.root.clone());
    let Some(root) = root else {
        return out::<Places>(Err(crate::error::Error::NoGameSelected));
    };
    // The install root is not the folder the game runs out of, and the library
    // that unpacks its text sits next to the executable.
    let root = match crate::game::Installation::probe(q.game, &root) {
        Ok(install) => install.game_dir,
        Err(problem) => return out::<Places>(Err(problem)),
    };

    // An edition is remembered by its own folder, and its files sit either
    // there or one level down in `mod` depending on how it was packaged.
    let mod_dir = edition_roots(&ctx, q.game)
        .into_iter()
        .flat_map(|root| [root.join("mod"), root])
        .find(|dir| dir.join("regulation.bin").is_file());
    let language = crate::language::status(&root)
        .current
        .as_deref()
        .and_then(crate::language::locale_folder)
        .unwrap_or("engus")
        .to_string();

    let places = crate::places::everywhere(crate::places::Where {
        game: q.game,
        game_dir: &root,
        mod_dir: mod_dir.as_deref(),
        language: &language,
        keep_in: Some(&ctx.app.app_data),
    });
    out(Ok(Places {
        places,
        language,
        source: mod_dir,
    }))
}

/// Refuses while the game is up.
fn game_is_up(game: crate::games::Game) -> Option<crate::error::Error> {
    crate::unlock::running_pid(game.executable()).map(|_| {
        crate::error::Error::msg(
            "close the game first — it holds the map in memory and would write over this"
                .to_string(),
        )
    })
}

#[derive(Deserialize)]
struct MarkerBody {
    game: crate::games::Game,
    path: PathBuf,
    slot: usize,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    /// Left out to clear every marker the character has.
    #[serde(default)]
    id: Option<i32>,
}

async fn markers_add(State(ctx): State<Ctx>, Json(body): Json<MarkerBody>) -> Response {
    if let Some(problem) = game_is_up(body.game) {
        return out::<()>(Err(problem));
    }
    out(crate::saves::add_marker(
        &ctx.app.app_data,
        body.game,
        &body.path,
        body.slot,
        body.x,
        body.y,
        0,
    ))
}

async fn markers_remove(State(ctx): State<Ctx>, Json(body): Json<MarkerBody>) -> Response {
    if let Some(problem) = game_is_up(body.game) {
        return out::<()>(Err(problem));
    }
    out(crate::saves::remove_markers(
        &ctx.app.app_data,
        body.game,
        &body.path,
        body.slot,
        body.id,
    ))
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

/// A browser cannot hand a page a folder path, so the desktop side picks.
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

    /// Reduvia is found on disk with the game shut, at the id `weapon()` takes.
    #[test]
    #[ignore = "needs the game installed"]
    fn offline_name_search_finds_reduvia() {
        let game = crate::games::Game::EldenRing;
        let Some(game_dir) = crate::testing::game_dir(game) else {
            return;
        };
        let mod_dir = crate::testing::mod_dir(game);
        let regulation =
            crate::formats::regulation::installed(game, &game_dir, mod_dir.as_deref()).unwrap();
        // The Russian name, since a Convergence install runs in Russian here.
        let hits = named_offline(&game_dir, mod_dir.as_deref(), "weapon", "Редувия");
        assert_eq!(hits.len(), 1, "one weapon matches Редувия");
        let (_, id) = hits[0];
        assert_eq!(id, 1_040_000, "the row this project pins");
        assert!(regulation.weapon(id).is_some(), "and weapon() reads it");
    }

    /// A size the way somebody says it, not the way a disk stores it.
    #[test]
    fn a_byte_count_is_said_out_loud() {
        assert_eq!(in_round_numbers(0), "1 KB");
        assert_eq!(in_round_numbers(700), "1 KB");
        assert_eq!(in_round_numbers(512 * 1024), "512 KB");
        assert_eq!(in_round_numbers(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(in_round_numbers(47_382_016), "45 MB");
        assert_eq!(in_round_numbers(3 * 1024 * 1024 * 1024), "3.0 GB");
        // Never zero and never a bare number: both read as "it does not know".
        for bytes in [1u64, 2, 999, 1_000_000, u64::from(u32::MAX)] {
            let said = in_round_numbers(bytes);
            assert!(!said.starts_with('0'), "{bytes} bytes came out as {said}");
            assert!(said.ends_with("KB") || said.ends_with("MB") || said.ends_with("GB"));
        }
    }

    /// Naming an armour set out of its own pieces, in whatever language.
    #[test]
    fn a_set_is_named_by_what_its_pieces_share() {
        let russian = [
            "Шлем овцебыка".to_string(),
            "Доспех овцебыка".to_string(),
            "Перчатки овцебыка".to_string(),
            "Поножи овцебыка".to_string(),
        ];
        assert_eq!(shared_words(&russian).as_deref(), Some("овцебыка"));

        // Shared word first, and more than one of them.
        let english = [
            "Bull-Goat Helm".to_string(),
            "Bull-Goat Armor".to_string(),
            "Bull-Goat Gauntlets".to_string(),
            "Bull-Goat Greaves".to_string(),
        ];
        assert_eq!(shared_words(&english).as_deref(), Some("Bull-Goat"));

        // In the middle, and the LONGEST run wins rather than the first.
        let middle = [
            "Knight of the Great Jar Helm".to_string(),
            "Knight of the Great Jar Armor".to_string(),
        ];
        assert_eq!(shared_words(&middle).as_deref(), Some("Knight of the Great Jar"));

        // A three-piece set is a real set here: 13 of this installation's have
        // exactly three and 33 have two, so nothing may assume four.
        let three = [
            "Одеяние бандита".to_string(),
            "Нарукавники бандита".to_string(),
            "Обувь бандита".to_string(),
        ];
        assert_eq!(shared_words(&three).as_deref(), Some("бандита"));

        // Nothing shared, and nothing invented from it.
        let unrelated = ["Reduvia".to_string(), "Bull-Goat Helm".to_string()];
        assert_eq!(shared_words(&unrelated), None);

        // One piece is not a set and has nothing to share with.
        assert_eq!(shared_words(&["Шлем овцебыка".to_string()]), None);
        assert_eq!(shared_words(&[]), None);

        // A run of one or two characters is not a name. Without this, two
        // pieces sharing only "de" would be "the de set".
        let tiny = ["Casque de fer".to_string(), "Gants de cuir".to_string()];
        assert_eq!(shared_words(&tiny), None);
    }

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
