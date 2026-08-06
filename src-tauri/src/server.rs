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
