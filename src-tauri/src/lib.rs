pub mod commands;
pub mod coop;
pub mod eac;
pub mod error;
pub mod formats;
pub mod game;
pub mod games;
pub mod install;
pub mod launch;
pub mod loader;
pub mod mods;
pub mod saves;
pub mod settings;
pub mod steam;
pub mod sys;

use commands::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::Manager;
            let app_data = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            std::fs::create_dir_all(&app_data).ok();
            app.manage(AppState::new(app_data));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::games_list,
            commands::settings_get,
            commands::settings_set,
            commands::steam_accounts,
            commands::installs_discover,
            commands::installs_probe,
            commands::installs_deep_scan,
            commands::installs_saved,
            commands::installs_remember,
            commands::installs_forget,
            commands::installs_active,
            commands::installs_size,
            commands::loaders_discover,
            commands::eac_status,
            commands::eac_set,
            commands::coop_fields,
            commands::coop_read,
            commands::coop_write,
            commands::coop_generate_password,
            commands::mods_list,
            commands::mods_analyse,
            commands::mods_install_folder,
            commands::mods_install_archive,
            commands::mods_delete,
            commands::mods_update,
            commands::profiles_list,
            commands::profile_create,
            commands::profile_save,
            commands::profile_delete,
            commands::profile_clone,
            commands::profile_conflicts,
            commands::launch_plan,
            commands::launch_patch,
            commands::launch_run,
            commands::game_is_running,
            commands::saves_discover,
            commands::saves_inspect,
            commands::saves_backup,
            commands::saves_backups,
            commands::saves_restore,
            commands::saves_delete_backup,
            commands::saves_transfer,
            commands::saves_convert,
            commands::saves_rebind,
            commands::saves_duplicates,
            commands::sys_shader_caches,
            commands::sys_clear_caches,
            commands::sys_report,
            commands::open_path,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Roundtable");
}
