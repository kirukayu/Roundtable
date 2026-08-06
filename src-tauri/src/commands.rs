//! Every operation the interface can invoke.

use std::collections::BTreeMap;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::State;

use crate::error::{Error, Result};
use crate::game::Installation;
use crate::games::{Game, GameInfo};
use crate::launch::{LaunchPlan, PatchReport, PlanInput};
use crate::loader::LoaderInstall;
use crate::mods::{ConflictReport, ModRecord, Profile};
use crate::saves::{BackupRecord, ConversionReport, DuplicateGroup, SaveFolder, TransferReport};
use crate::settings::Settings;
use crate::{coop, eac, game, loader, mods, saves, steam, sys};

pub struct AppState {
    pub app_data: PathBuf,
    pub settings: Mutex<Settings>,
    pub presence: crate::presence::Presence,
    pub http: reqwest::Client,
    /// Progress of an edition being unpacked. Extraction runs on its own thread
    /// and writes here; the interface polls it. An eight gigabyte archive takes
    /// minutes, and a request that just blocks tells the user nothing.
    pub edition_job: Mutex<crate::edition::EditionJob>,
    /// The codex, loaded from disk on first use and kept for the session. Two
    /// and a half thousand rows is nothing to hold and everything to re-read on
    /// each keystroke.
    pub codex: Mutex<Option<Vec<crate::codex::CodexEntry>>>,
    pub codex_job: Mutex<crate::codex::CodexState>,
    pub wiki_job: Mutex<crate::wiki::WikiIndexState>,
    /// A full-disk search for a game, and where it has got to.
    pub scan_job: Mutex<ScanState>,
}

/// Progress of the whole-machine search.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanState {
    pub running: bool,
    pub done: bool,
    /// The folder being looked at, so the wait has something to show.
    pub at: String,
    pub found: Vec<Installation>,
    pub cancelled: bool,
}

impl AppState {
    pub fn new(app_data: PathBuf) -> AppState {
        let settings = Settings::load(&app_data);
        let presence = crate::presence::Presence::default();
        if settings.discord_presence && presence.connect() {
            presence.set_browsing();
        }
        AppState {
            app_data,
            settings: Mutex::new(settings),
            presence,
            http: reqwest::Client::builder()
                .user_agent(concat!("Roundtable/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            edition_job: Mutex::new(crate::edition::EditionJob::default()),
            codex: Mutex::new(None),
            codex_job: Mutex::new(crate::codex::CodexState::default()),
            wiki_job: Mutex::new(crate::wiki::WikiIndexState::default()),
            scan_job: Mutex::new(ScanState::default()),
        }
    }

    /// The codex, reading it from disk the first time it is asked for.
    pub fn codex(&self) -> Vec<crate::codex::CodexEntry> {
        let mut slot = self.codex.lock();
        if slot.is_none() {
            *slot = Some(crate::codex::load(&self.app_data));
        }
        slot.clone().unwrap_or_default()
    }

    /// Drops the in-memory copy so the next read picks up a finished sync.
    pub fn forget_codex(&self) {
        *self.codex.lock() = None;
    }

    fn persist(&self) -> Result<()> {
        self.settings.lock().save(&self.app_data)
    }

    /// Resolves the installation the user is currently working with.
    pub fn active_install(&self, game: Game) -> Result<Installation> {
        let root = {
            let settings = self.settings.lock();
            settings.install_for(game).map(|i| i.root.clone())
        };
        let root = root.ok_or(Error::NoGameSelected)?;
        Installation::probe(game, &root)
    }

    fn work_dir(&self, game: Game, profile_id: &str) -> PathBuf {
        self.app_data
            .join("launch")
            .join(game.appdata_folder())
            .join(profile_id)
    }
}

// ---------------------------------------------------------------------------
// Catalogue and settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn games_list() -> Vec<GameInfo> {
    Game::ALL.into_iter().map(GameInfo::from).collect()
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, settings: Settings) -> Result<Settings> {
    let presence_wanted = settings.discord_presence;
    let presence_was = state.settings.lock().discord_presence;

    *state.settings.lock() = settings;
    state.persist()?;

    // Honour the toggle immediately rather than at the next launch.
    if presence_wanted != presence_was {
        if presence_wanted {
            if state.presence.connect() {
                state.presence.set_browsing();
            }
        } else {
            state.presence.disconnect();
        }
    }

    Ok(state.settings.lock().clone())
}

#[tauri::command]
pub fn steam_accounts() -> Vec<steam::SteamAccount> {
    steam::local_accounts()
}

// ---------------------------------------------------------------------------
// Installations
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn installs_discover(game: Game) -> Vec<Installation> {
    game::discover(game)
}

#[tauri::command]
pub fn installs_probe(game: Game, path: PathBuf) -> Result<Installation> {
    Installation::probe(game, &path)
}

#[tauri::command]
pub fn installs_deep_scan(game: Game, root: PathBuf, max_depth: Option<usize>) -> Vec<PathBuf> {
    game::deep_scan(game, &root, max_depth.unwrap_or(5))
}

#[tauri::command]
pub fn installs_saved(state: State<'_, AppState>, game: Game) -> Vec<Installation> {
    let roots: Vec<PathBuf> = state
        .settings
        .lock()
        .installations
        .iter()
        .filter(|i| i.game == game)
        .map(|i| i.root.clone())
        .collect();

    roots
        .into_iter()
        .filter_map(|root| Installation::probe(game, &root).ok())
        .collect()
}

#[tauri::command]
pub fn installs_remember(
    state: State<'_, AppState>,
    game: Game,
    path: PathBuf,
    make_default: bool,
) -> Result<Installation> {
    let install = Installation::probe(game, &path)?;
    state
        .settings
        .lock()
        .remember_install(game, install.root.clone(), make_default);
    state.persist()?;
    Ok(install)
}

#[tauri::command]
pub fn installs_forget(state: State<'_, AppState>, game: Game, path: PathBuf) -> Result<()> {
    state.settings.lock().forget_install(game, &path);
    state.persist()
}

/// The remembered installation for a game, or nothing.
///
/// Not having located a game is the state every game starts in, so it is
/// reported as an absent value rather than a failure.
#[tauri::command]
pub fn installs_active(state: State<'_, AppState>, game: Game) -> Result<Option<Installation>> {
    match state.active_install(game) {
        Ok(install) => Ok(Some(install)),
        Err(crate::error::Error::NoGameSelected) => Ok(None),
        Err(other) => Err(other),
    }
}

#[tauri::command]
pub fn installs_size(path: PathBuf) -> u64 {
    game::folder_size(&path)
}

// ---------------------------------------------------------------------------
// Loaders and anti-cheat
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn loaders_discover(state: State<'_, AppState>, game: Game) -> Vec<LoaderInstall> {
    let root = state.active_install(game).ok().map(|i| i.root);
    loader::discover(game, root.as_deref())
}

/// The anti-cheat state, or nothing while the game has not been located.
#[tauri::command]
pub fn eac_status(state: State<'_, AppState>, game: Game) -> Result<Option<eac::EacStatus>> {
    match state.active_install(game) {
        Ok(install) => Ok(Some(eac::status(game, &install.game_dir))),
        Err(crate::error::Error::NoGameSelected) => Ok(None),
        Err(other) => Err(other),
    }
}

#[tauri::command]
pub fn eac_set(state: State<'_, AppState>, game: Game, enabled: bool) -> Result<eac::EacStatus> {
    let install = state.active_install(game)?;
    if enabled {
        eac::enable(game, &install.game_dir)
    } else {
        eac::disable(game, &install.game_dir)
    }
}

// ---------------------------------------------------------------------------
// Seamless Co-op
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn coop_fields() -> Vec<&'static coop::FieldSpec> {
    coop::FIELDS.iter().collect()
}

#[tauri::command]
pub fn coop_read(state: State<'_, AppState>, game: Game) -> Result<coop::CoopSettings> {
    let install = state.active_install(game)?;
    coop::read(&install.game_dir)
}

#[tauri::command]
pub fn coop_write(
    state: State<'_, AppState>,
    game: Game,
    changes: BTreeMap<String, String>,
) -> Result<coop::CoopSettings> {
    let install = state.active_install(game)?;
    coop::write(&install.game_dir, &changes)
}

#[tauri::command]
pub fn coop_generate_password() -> String {
    coop::generate_password()
}

// ---------------------------------------------------------------------------
// Mods and profiles
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn mods_list(state: State<'_, AppState>, game: Game) -> Vec<ModRecord> {
    mods::list_mods(&state.app_data, game)
}

#[tauri::command]
pub fn mods_analyse(path: PathBuf) -> mods::LayoutAnalysis {
    mods::analyse_layout(&path)
}

#[tauri::command]
pub fn mods_install_folder(
    state: State<'_, AppState>,
    game: Game,
    source: PathBuf,
    name: Option<String>,
) -> Result<ModRecord> {
    crate::install::from_folder(&state.app_data, game, &source, name.as_deref())
}

#[tauri::command]
pub fn mods_install_archive(
    state: State<'_, AppState>,
    game: Game,
    archive: PathBuf,
    name: Option<String>,
) -> Result<ModRecord> {
    crate::install::from_archive(&state.app_data, game, &archive, name.as_deref())
}

#[tauri::command]
pub fn mods_delete(state: State<'_, AppState>, game: Game, id: String) -> Result<()> {
    mods::delete_mod(&state.app_data, game, &id)
}

#[tauri::command]
pub fn mods_update(state: State<'_, AppState>, record: ModRecord) -> Result<()> {
    mods::save_record(&state.app_data, &record)
}

#[tauri::command]
pub fn profiles_list(state: State<'_, AppState>, game: Game) -> Vec<Profile> {
    mods::list_profiles(&state.app_data, game)
}

#[tauri::command]
pub fn profile_create(state: State<'_, AppState>, game: Game, name: String) -> Result<Profile> {
    let mut profile = Profile::new(game, &name);
    profile.id = mods::unique_profile_id(&state.app_data, game, &profile.id);
    mods::save_profile(&state.app_data, &profile)?;
    Ok(profile)
}

#[tauri::command]
pub fn profile_save(state: State<'_, AppState>, profile: Profile) -> Result<Profile> {
    mods::save_profile(&state.app_data, &profile)?;
    Ok(profile)
}

#[tauri::command]
pub fn profile_delete(state: State<'_, AppState>, game: Game, id: String) -> Result<()> {
    mods::delete_profile(&state.app_data, game, &id)
}

#[tauri::command]
pub fn profile_clone(
    state: State<'_, AppState>,
    game: Game,
    id: String,
    name: String,
) -> Result<Profile> {
    mods::clone_profile(&state.app_data, game, &id, &name)
}

#[tauri::command]
pub fn profile_conflicts(
    state: State<'_, AppState>,
    game: Game,
    id: String,
) -> Result<ConflictReport> {
    let profile = mods::list_profiles(&state.app_data, game)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::msg(format!("profile '{id}' was not found")))?;

    let library = mods::list_mods(&state.app_data, game);
    let ordered: Vec<ModRecord> = profile
        .enabled_mod_ids()
        .into_iter()
        .filter_map(|mid| library.iter().find(|m| m.id == mid).cloned())
        .collect();

    Ok(mods::detect_conflicts(&ordered))
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedLaunch {
    pub plan: LaunchPlan,
    pub command_line: String,
}

fn build_plan(
    state: &AppState,
    game: Game,
    profile_id: &str,
) -> Result<(Installation, Profile, Vec<ModRecord>, Vec<LoaderInstall>, PathBuf)> {
    let install = state.active_install(game)?;
    let profile = mods::list_profiles(&state.app_data, game)
        .into_iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| Error::msg(format!("profile '{profile_id}' was not found")))?;

    let library = mods::list_mods(&state.app_data, game);
    let loaders = loader::discover(game, Some(&install.root));
    let work_dir = state.work_dir(game, profile_id);

    Ok((install, profile, library, loaders, work_dir))
}

#[tauri::command]
pub fn launch_plan(
    state: State<'_, AppState>,
    game: Game,
    profile_id: String,
) -> Result<PreparedLaunch> {
    let (install, profile, library, loaders, work_dir) = build_plan(&state, game, &profile_id)?;

    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let input = PlanInput {
        install: &install,
        profile: &profile,
        mods: &library,
        loaders: &loaders,
        work_dir,
        steam_running: steam::is_running(&system),
    };

    let plan = crate::launch::plan(&input)?;
    Ok(PreparedLaunch {
        command_line: plan.command_line(),
        plan,
    })
}

#[tauri::command]
pub fn launch_patch(
    state: State<'_, AppState>,
    game: Game,
    profile_id: String,
) -> Result<PatchReport> {
    let (install, profile, library, loaders, work_dir) = build_plan(&state, game, &profile_id)?;

    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let input = PlanInput {
        install: &install,
        profile: &profile,
        mods: &library,
        loaders: &loaders,
        work_dir,
        steam_running: steam::is_running(&system),
    };

    let plan = crate::launch::plan(&input)?;
    crate::launch::apply(&input, &plan)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResult {
    pub pid: u32,
    pub route: String,
    pub patched: PatchReport,
    pub backup_id: Option<String>,
}

#[tauri::command]
pub fn launch_run(
    state: State<'_, AppState>,
    game: Game,
    profile_id: String,
) -> Result<LaunchResult> {
    let (install, mut profile, library, loaders, work_dir) =
        build_plan(&state, game, &profile_id)?;

    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    if crate::launch::is_game_running(game, &system) {
        return Err(Error::msg(
            "the game is already running; close it before starting another profile",
        ));
    }

    let input = PlanInput {
        install: &install,
        profile: &profile,
        mods: &library,
        loaders: &loaders,
        work_dir,
        steam_running: steam::is_running(&system),
    };

    let plan = crate::launch::plan(&input)?;
    let patched = crate::launch::apply(&input, &plan)?;

    // Take a snapshot before anything can touch the save.
    let backup_id = {
        let (auto, keep) = {
            let settings = state.settings.lock();
            (settings.auto_backup_on_launch, settings.auto_backup_keep)
        };
        if auto {
            let live = install
                .appdata_dir()
                .map(|dir| dir.join(game.save_file()))
                .filter(|p| p.is_file());
            match live {
                Some(path) => {
                    let record = saves::create_backup(
                        &state.app_data,
                        game,
                        &path,
                        &format!("before {}", profile.name),
                        true,
                    )?;
                    saves::prune_backups(&state.app_data, game, keep).ok();
                    Some(record.id)
                }
                None => None,
            }
        } else {
            None
        }
    };

    let pid = crate::launch::spawn(&plan)?;

    if state.settings.lock().discord_presence {
        state.presence.set_playing(game, Some(&profile.name));
    }

    profile.last_played = Some(chrono::Local::now().to_rfc3339());
    mods::save_profile(&state.app_data, &profile).ok();
    {
        let mut settings = state.settings.lock();
        settings.active_profile = Some(profile.id.clone());
    }
    state.persist().ok();

    Ok(LaunchResult {
        pid,
        route: plan.route.label().to_string(),
        patched,
        backup_id,
    })
}

#[tauri::command]
pub fn game_is_running(state: State<'_, AppState>, game: Game) -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let running = crate::launch::is_game_running(game, &system);

    // The UI polls this, which makes it the natural place to notice the game has
    // exited and put the presence card back to browsing.
    if state.settings.lock().discord_presence && !running {
        state.presence.set_browsing();
    }

    running
}

// ---------------------------------------------------------------------------
// Nexus Mods
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn nexus_validate(state: State<'_, AppState>) -> Result<crate::net::nexus::Account> {
    let key = state
        .settings
        .lock()
        .nexus_api_key
        .clone()
        .unwrap_or_default();
    crate::net::nexus::validate(&state.http, &key).await
}

#[tauri::command]
pub async fn nexus_mod_info(
    state: State<'_, AppState>,
    game: Game,
    mod_id: u32,
) -> Result<crate::net::nexus::ModInfo> {
    let key = state
        .settings
        .lock()
        .nexus_api_key
        .clone()
        .unwrap_or_default();
    crate::net::nexus::mod_info(&state.http, &key, game, mod_id).await
}

#[tauri::command]
pub async fn nexus_mod_files(
    state: State<'_, AppState>,
    game: Game,
    mod_id: u32,
) -> Result<Vec<crate::net::nexus::ModFile>> {
    let key = state
        .settings
        .lock()
        .nexus_api_key
        .clone()
        .unwrap_or_default();
    crate::net::nexus::mod_files(&state.http, &key, game, mod_id).await
}

#[tauri::command]
pub fn nexus_parse_link(link: String) -> Result<crate::net::nexus::NxmLink> {
    crate::net::nexus::parse_nxm(&link)
}

// ---------------------------------------------------------------------------
// Saves
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn saves_discover(
    state: State<'_, AppState>,
    game: Game,
) -> Vec<SaveFolder> {
    // The co-op extension is configurable, so read it before scanning.
    let extra = state
        .active_install(game)
        .ok()
        .and_then(|install| coop::read(&install.game_dir).ok())
        .and_then(|settings| settings.values.get("SAVE.save_file_extension").cloned());

    saves::discover(game, extra.as_deref())
}

#[tauri::command]
pub fn saves_inspect(path: PathBuf) -> Result<crate::formats::save::SaveSummary> {
    saves::inspect(&path)
}

#[tauri::command]
pub fn saves_backup(
    state: State<'_, AppState>,
    game: Game,
    path: PathBuf,
    label: String,
) -> Result<BackupRecord> {
    saves::create_backup(&state.app_data, game, &path, &label, false)
}

#[tauri::command]
pub fn saves_backups(state: State<'_, AppState>, game: Game) -> Vec<BackupRecord> {
    saves::list_backups(&state.app_data, game)
}

#[tauri::command]
pub fn saves_restore(
    state: State<'_, AppState>,
    game: Game,
    backup_id: String,
    destination: Option<PathBuf>,
) -> Result<PathBuf> {
    saves::restore_backup(&state.app_data, game, &backup_id, destination.as_deref())
}

#[tauri::command]
pub fn saves_delete_backup(
    state: State<'_, AppState>,
    game: Game,
    backup_id: String,
) -> Result<()> {
    saves::delete_backup(&state.app_data, game, &backup_id)
}

#[tauri::command]
pub fn saves_transfer(
    state: State<'_, AppState>,
    game: Game,
    source: PathBuf,
    destination: PathBuf,
    slot_pairs: Vec<(usize, usize)>,
) -> Result<TransferReport> {
    saves::transfer_slots(&state.app_data, game, &source, &destination, &slot_pairs)
}

#[tauri::command]
pub fn saves_convert(
    state: State<'_, AppState>,
    game: Game,
    source: PathBuf,
    extension: String,
    destination_dir: Option<PathBuf>,
    rebind_to: Option<u64>,
) -> Result<ConversionReport> {
    saves::convert(
        &state.app_data,
        game,
        &source,
        &extension,
        destination_dir.as_deref(),
        rebind_to,
    )
}

#[tauri::command]
pub fn saves_rebind(
    state: State<'_, AppState>,
    game: Game,
    path: PathBuf,
    steam_id: u64,
) -> Result<String> {
    saves::rebind(&state.app_data, game, &path, steam_id)
}

#[tauri::command]
pub fn saves_duplicates(paths: Vec<PathBuf>) -> Result<Vec<DuplicateGroup>> {
    saves::find_duplicates(&paths)
}

// ---------------------------------------------------------------------------
// System tools
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn sys_shader_caches() -> Vec<sys::CacheLocation> {
    sys::shader_caches()
}

#[tauri::command]
pub fn sys_clear_caches(paths: Vec<PathBuf>) -> Result<sys::CleanReport> {
    sys::clear_caches(&paths)
}

#[tauri::command]
pub fn sys_report(game: Game) -> sys::SystemReport {
    sys::system_report(game)
}

#[tauri::command]
pub fn open_path(path: PathBuf) -> Result<()> {
    let target = if path.is_file() {
        path.parent().map(std::path::Path::to_path_buf).unwrap_or(path)
    } else {
        path
    };
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(&target)
            .spawn()
            .map_err(|e| Error::Io {
                path: target.clone(),
                source: e,
            })?;
    }
    #[cfg(not(windows))]
    {
        let _ = &target;
    }
    Ok(())
}
