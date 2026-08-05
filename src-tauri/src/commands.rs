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
}

impl AppState {
    pub fn new(app_data: PathBuf) -> AppState {
        let settings = Settings::load(&app_data);
        AppState {
            app_data,
            settings: Mutex::new(settings),
        }
    }

    fn persist(&self) -> Result<()> {
        self.settings.lock().save(&self.app_data)
    }

    /// Resolves the installation the user is currently working with.
    fn active_install(&self, game: Game) -> Result<Installation> {
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
    *state.settings.lock() = settings;
    state.persist()?;
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

#[tauri::command]
pub fn installs_active(state: State<'_, AppState>, game: Game) -> Result<Installation> {
    state.active_install(game)
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

#[tauri::command]
pub fn eac_status(state: State<'_, AppState>, game: Game) -> Result<eac::EacStatus> {
    let install = state.active_install(game)?;
    Ok(eac::status(game, &install.game_dir))
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
pub fn game_is_running(game: Game) -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    crate::launch::is_game_running(game, &system)
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
