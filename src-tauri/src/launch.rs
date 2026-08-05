//! The smart bootloader.
//!
//! Starting a modded ELDEN RING correctly depends on three things at once: how the
//! game was installed, which loader is available, and whether Seamless Co-op is in
//! the mix. Getting any of them wrong produces the failures people actually hit —
//! "trying to find steam" on a repack, co-op silently not loading, or a modded
//! character overwriting a vanilla save.
//!
//! Rather than guess at launch time, Roundtable builds an explicit plan, shows it,
//! and only then runs it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::coop;
use crate::error::{Error, IoContext, Result};
use crate::game::{InstallKind, Installation};
use crate::games::Game;
use crate::loader::{self, LoaderInstall, LoaderKind, Me3Profile};
use crate::mods::{ModRecord, Profile};

/// Which chain the plan settled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchRoute {
    /// `me3 launch -p <profile>` — the only route that can skip Steam init.
    Me3,
    /// `modengine2_launcher -t er -c <config>`.
    ModEngine2,
    /// Seamless Co-op's own launcher, when co-op is the only thing enabled.
    SeamlessCoopLauncher,
    /// The game executable, unmodded.
    Direct,
}

impl LaunchRoute {
    pub fn label(self) -> &'static str {
        match self {
            LaunchRoute::Me3 => "me3",
            LaunchRoute::ModEngine2 => "ModEngine 2",
            LaunchRoute::SeamlessCoopLauncher => "Seamless Co-op launcher",
            LaunchRoute::Direct => "Direct",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Info,
    Warning,
    Blocker,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
}

impl Notice {
    fn info(title: &str, detail: impl Into<String>) -> Notice {
        Notice {
            severity: Severity::Info,
            title: title.into(),
            detail: detail.into(),
        }
    }
    fn warn(title: &str, detail: impl Into<String>) -> Notice {
        Notice {
            severity: Severity::Warning,
            title: title.into(),
            detail: detail.into(),
        }
    }
    fn blocker(title: &str, detail: impl Into<String>) -> Notice {
        Notice {
            severity: Severity::Blocker,
            title: title.into(),
            detail: detail.into(),
        }
    }
}

/// A fully resolved launch, ready to run and readable by a human first.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchPlan {
    pub route: LaunchRoute,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Step-by-step description shown in the UI before anything runs.
    pub steps: Vec<String>,
    pub notices: Vec<Notice>,
    /// Files Roundtable will write when the plan is applied.
    pub writes: Vec<PathBuf>,
    pub coop_enabled: bool,
    pub skip_steam_init: bool,
}

impl LaunchPlan {
    pub fn is_runnable(&self) -> bool {
        !self
            .notices
            .iter()
            .any(|n| n.severity == Severity::Blocker)
    }

    pub fn command_line(&self) -> String {
        let mut out = quote(&self.program.to_string_lossy());
        for arg in &self.args {
            out.push(' ');
            out.push_str(&quote(arg));
        }
        out
    }
}

fn quote(text: &str) -> String {
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text.to_string()
    }
}

/// Everything the planner needs to decide.
pub struct PlanInput<'a> {
    pub install: &'a Installation,
    pub profile: &'a Profile,
    pub mods: &'a [ModRecord],
    pub loaders: &'a [LoaderInstall],
    /// Folder holding the generated loader config for this profile.
    pub work_dir: PathBuf,
    pub steam_running: bool,
}

/// Chooses the launch chain and spells out why.
pub fn plan(input: &PlanInput<'_>) -> Result<LaunchPlan> {
    let install = input.install;
    let profile = input.profile;
    let standalone = install.kind == InstallKind::Standalone;
    let coop = profile.seamless_coop && install.game.supports_seamless_coop();

    let mut notices = Vec::new();
    let mut steps = Vec::new();
    let mut writes = Vec::new();

    if coop && !install.has_seamless_coop {
        notices.push(Notice::blocker(
            "Seamless Co-op is not installed",
            format!(
                "This profile wants co-op, but {} is missing. Install Seamless Co-op from the Co-op page first.",
                coop::dll_path(&install.game_dir).display()
            ),
        ));
    }

    let has_mods = !profile.enabled_mod_ids().is_empty();
    let me3 = input
        .loaders
        .iter()
        .find(|l| l.kind == LoaderKind::Me3);
    let me2 = input
        .loaders
        .iter()
        .find(|l| l.kind == LoaderKind::ModEngine2);

    // me3 is preferred whenever mods are involved: it is the maintained loader and
    // the only one that can start a cracked copy without a Steam client.
    let route = if has_mods || (coop && (me3.is_some() || me2.is_some())) {
        match (me3, me2) {
            (Some(_), _) => LaunchRoute::Me3,
            (None, Some(_)) => LaunchRoute::ModEngine2,
            (None, None) => {
                notices.push(Notice::blocker(
                    "No mod loader found",
                    "This profile has mods enabled but neither me3 nor ModEngine 2 is installed. Install me3 from the Tools page.",
                ));
                LaunchRoute::Direct
            }
        }
    } else if coop {
        LaunchRoute::SeamlessCoopLauncher
    } else {
        LaunchRoute::Direct
    };

    if standalone {
        notices.push(Notice::info(
            "Non-Steam installation",
            "Steamworks is emulated in this copy, so Roundtable writes steam_appid.txt and asks the loader not to initialise Steam.",
        ));
    }
    if install.kind == InstallKind::Steam && !input.steam_running && route != LaunchRoute::Direct {
        notices.push(Notice::warn(
            "Steam is not running",
            "This is a Steam copy. Start Steam first, or the game may close immediately.",
        ));
    }

    if standalone && route == LaunchRoute::ModEngine2 {
        notices.push(Notice::warn(
            "ModEngine 2 cannot skip Steam initialisation",
            "ModEngine 2 has no --skip-steam-init flag, which is what causes \"trying to find steam\" on repacks. Roundtable writes steam_appid.txt to help, but installing me3 is the reliable fix.",
        ));
    }

    if profile.start_online && coop {
        notices.push(Notice::warn(
            "Online mode is on",
            "Seamless Co-op runs its own matchmaking and does not need official servers. Leaving this on connects to FromSoftware's servers with mods loaded, which risks a ban.",
        ));
    }

    if install.has_eac && !install.eac_bypassed && route != LaunchRoute::Direct {
        notices.push(Notice::info(
            "Anti-cheat is skipped for this launch",
            "Mod loaders start the game directly, so Easy Anti-Cheat does not run. Steam's own Play button still boots it.",
        ));
    }

    let mut env = BTreeMap::new();
    env.insert(
        "SteamAppId".to_string(),
        install.game.steam_app_id().to_string(),
    );
    env.insert(
        "SteamGameId".to_string(),
        install.game.steam_app_id().to_string(),
    );

    let (program, args) = match route {
        LaunchRoute::Me3 => {
            let loader = me3.expect("route implies a me3 install");
            let profile_path = input.work_dir.join(format!("{}.me3", profile.id));
            writes.push(profile_path.clone());

            steps.push(format!(
                "Write a me3 profile listing {} mod package(s){}",
                input.mods.len(),
                if coop { " plus the co-op DLL" } else { "" }
            ));

            let mut args = vec![
                "launch".to_string(),
                "--game".to_string(),
                install.game.me3_id().to_string(),
                "--profile".to_string(),
                profile_path.to_string_lossy().to_string(),
            ];

            if standalone {
                args.push("--skip-steam-init".to_string());
                args.push("true".to_string());
                steps.push("Pass --skip-steam-init so the loader does not wait for Steam".into());
            }

            // Always pin the executable: it removes any doubt about which copy runs
            // when both a Steam and a cracked install exist.
            args.push("--exe".to_string());
            args.push(install.executable.to_string_lossy().to_string());
            steps.push(format!("Start {}", install.executable.display()));

            (loader.executable.clone(), args)
        }
        LaunchRoute::ModEngine2 => {
            let loader = me2.expect("route implies a ModEngine 2 install");
            let config_name = install
                .game
                .me2_config_name()
                .ok_or_else(|| Error::msg("ModEngine 2 does not support this game"))?;
            let config_path = input.work_dir.join(config_name);
            writes.push(config_path.clone());

            let target = install
                .game
                .me2_id()
                .ok_or_else(|| Error::msg("ModEngine 2 does not support this game"))?;

            steps.push(format!("Write {config_name} with absolute mod paths"));
            steps.push(format!("Start {}", install.executable.display()));

            let args = vec![
                "-t".to_string(),
                target.to_string(),
                "-c".to_string(),
                config_path.to_string_lossy().to_string(),
                "-p".to_string(),
                install.executable.to_string_lossy().to_string(),
            ];

            (loader.executable.clone(), args)
        }
        LaunchRoute::SeamlessCoopLauncher => {
            let launcher = install.game_dir.join(coop::LAUNCHER_FILE);
            if !launcher.is_file() {
                notices.push(Notice::blocker(
                    "Co-op launcher missing",
                    format!("{} was not found.", launcher.display()),
                ));
            }
            steps.push("Start Seamless Co-op directly (no other mods are enabled)".into());
            (launcher, Vec::new())
        }
        LaunchRoute::Direct => {
            steps.push(format!("Start {} unmodded", install.executable.display()));
            (install.executable.clone(), Vec::new())
        }
    };

    if standalone {
        writes.push(install.game_dir.join("steam_appid.txt"));
    }

    Ok(LaunchPlan {
        route,
        program,
        args,
        working_dir: install.game_dir.clone(),
        env,
        steps,
        notices,
        writes,
        coop_enabled: coop,
        skip_steam_init: standalone && route == LaunchRoute::Me3,
    })
}

/// Writes every file the plan depends on: the loader config, the co-op wiring and
/// `steam_appid.txt`. This is the "Patch" button.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchReport {
    pub route: LaunchRoute,
    pub written: Vec<PathBuf>,
    pub changes: Vec<String>,
    pub notices: Vec<Notice>,
}

pub fn apply(input: &PlanInput<'_>, plan: &LaunchPlan) -> Result<PatchReport> {
    let install = input.install;
    let profile = input.profile;
    let mut written = Vec::new();
    let mut changes = Vec::new();

    std::fs::create_dir_all(&input.work_dir).at(&input.work_dir)?;

    // Ordered so the mod the user put first actually wins.
    let ordered: Vec<&ModRecord> = profile
        .enabled_mod_ids()
        .into_iter()
        .filter_map(|id| input.mods.iter().find(|m| m.id == id))
        .collect();

    match plan.route {
        LaunchRoute::Me3 => {
            let mut me3_profile = Me3Profile::new(install.game);
            me3_profile.disable_arxan = Some(profile.disable_arxan);
            me3_profile.mem_patch = Some(profile.mem_patch);
            me3_profile.start_online = Some(profile.start_online);

            // me3 applies later packages on top of earlier ones, so the list is
            // reversed relative to the user's "first wins" load order.
            for record in ordered.iter().rev() {
                me3_profile.packages.push(loader::Me3Package {
                    path: record.path.to_string_lossy().to_string(),
                    id: Some(record.id.clone()),
                    enabled: true,
                });
                for native in &record.natives {
                    me3_profile.natives.push(loader::Me3Native {
                        path: record.path.join(native).to_string_lossy().to_string(),
                        enabled: true,
                        optional: false,
                    });
                }
            }

            if plan.coop_enabled {
                let dll = coop::dll_path(&install.game_dir);
                if loader::me3_add_coop_native(&mut me3_profile, &dll.to_string_lossy()) {
                    changes.push(format!("Added {} to the profile's natives", dll.display()));
                }
                // Seamless Co-op already isolates saves through its own extension;
                // setting me3's savefile on top would fight it.
                changes.push(
                    "Left the save file alone: Seamless Co-op isolates saves by extension".into(),
                );
            } else if let Some(savefile) = &profile.savefile {
                me3_profile.savefile = Some(savefile.clone());
                changes.push(format!("Modded run uses its own save file, {savefile}"));
            }

            let path = input.work_dir.join(format!("{}.me3", profile.id));
            me3_profile.save(&path)?;
            written.push(path);
            changes.push(format!(
                "Wrote a me3 profile with {} package(s) and {} native(s)",
                me3_profile.packages.len(),
                me3_profile.natives.len()
            ));

            // Persist the per-game defaults so double-clicking the profile in
            // Explorer behaves the same as launching from Roundtable.
            let standalone = install.kind == InstallKind::Standalone;
            match loader::write_me3_game_defaults(
                install.game,
                standalone,
                Some(&install.executable),
                Some(profile.skip_logos),
            ) {
                Ok(config) => {
                    changes.push(format!(
                        "Set me3 defaults for this game (skip_steam_init = {standalone})"
                    ));
                    written.push(config);
                }
                Err(err) => changes.push(format!("Could not update me3.toml: {err}")),
            }
        }
        LaunchRoute::ModEngine2 => {
            let config_name = install
                .game
                .me2_config_name()
                .ok_or_else(|| Error::msg("ModEngine 2 does not support this game"))?;
            let path = input.work_dir.join(config_name);

            // Start from the loader's own config so its comments are carried over.
            let mut config = input
                .loaders
                .iter()
                .find(|l| l.kind == LoaderKind::ModEngine2)
                .and_then(|l| l.config.clone())
                .filter(|p| p.is_file())
                .and_then(|source| {
                    std::fs::copy(&source, &path).ok()?;
                    loader::read_me2_config(&path).ok()
                })
                .unwrap_or_default();

            config.mod_loader_enabled = true;
            config.mods = ordered
                .iter()
                .map(|record| loader::Me2Mod {
                    enabled: true,
                    name: record.name.clone(),
                    path: record.path.to_string_lossy().to_string(),
                })
                .collect();

            if plan.coop_enabled {
                let dll = coop::dll_path(&install.game_dir);
                if loader::me2_add_coop_dll(&mut config, &dll.to_string_lossy()) {
                    changes.push(format!("Added {} to external_dlls", dll.display()));
                }
            }

            for record in &ordered {
                for native in &record.natives {
                    let full = record.path.join(native).to_string_lossy().to_string();
                    if loader::me2_add_coop_dll(&mut config, &full) {
                        changes.push(format!("Added {native} to external_dlls"));
                    }
                }
            }

            loader::write_me2_config(&path, &config)?;
            written.push(path);
            changes.push(format!(
                "Wrote {config_name} with {} mod folder(s)",
                config.mods.len()
            ));
        }
        LaunchRoute::SeamlessCoopLauncher | LaunchRoute::Direct => {}
    }

    if install.kind == InstallKind::Standalone {
        match crate::steam::write_appid_file(&install.game_dir, install.game.steam_app_id()) {
            Ok(path) => {
                changes.push(format!(
                    "Wrote steam_appid.txt so the emulator reports app {}",
                    install.game.steam_app_id()
                ));
                written.push(path);
            }
            Err(err) => changes.push(format!("Could not write steam_appid.txt: {err}")),
        }
    }

    Ok(PatchReport {
        route: plan.route,
        written,
        changes,
        notices: plan.notices.clone(),
    })
}

/// Runs a prepared plan.
pub fn spawn(plan: &LaunchPlan) -> Result<u32> {
    if !plan.is_runnable() {
        let blockers: Vec<String> = plan
            .notices
            .iter()
            .filter(|n| n.severity == Severity::Blocker)
            .map(|n| n.title.clone())
            .collect();
        return Err(Error::msg(format!(
            "this profile cannot start yet: {}",
            blockers.join("; ")
        )));
    }
    if !plan.program.is_file() {
        return Err(Error::msg(format!(
            "{} was not found",
            plan.program.display()
        )));
    }

    let mut command = std::process::Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(&plan.working_dir)
        .envs(plan.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));

    let child = command.spawn().at(&plan.program)?;
    Ok(child.id())
}

/// True while a process with this executable name is alive.
pub fn is_game_running(game: Game, system: &sysinfo::System) -> bool {
    let target = game.executable().to_ascii_lowercase();
    system.processes().values().any(|p| {
        p.name()
            .to_string_lossy()
            .to_ascii_lowercase()
            .eq(target.as_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::InstallKind;
    use crate::mods::{ModKind, ProfileMod};

    fn install(kind: InstallKind, coop_installed: bool) -> Installation {
        let root = PathBuf::from("D:\\Games\\ELDEN RING");
        Installation {
            game: Game::EldenRing,
            game_dir: root.join("Game"),
            executable: root.join("Game").join("eldenring.exe"),
            root,
            kind,
            version: Some("1.16.0.0".into()),
            has_eac: true,
            eac_bypassed: false,
            has_seamless_coop: coop_installed,
            seamless_coop_version: None,
            size_bytes: None,
            markers: Vec::new(),
        }
    }

    fn loader(kind: LoaderKind) -> LoaderInstall {
        LoaderInstall {
            kind,
            executable: PathBuf::from(match kind {
                LoaderKind::Me3 => "C:\\me3\\me3.exe",
                LoaderKind::ModEngine2 => "D:\\ME2\\modengine2_launcher.exe",
            }),
            directory: PathBuf::from("C:\\loader"),
            version: None,
            config: None,
        }
    }

    fn mod_record(id: &str) -> ModRecord {
        ModRecord {
            id: id.into(),
            name: id.into(),
            version: None,
            author: None,
            summary: None,
            nexus_mod_id: None,
            game: Game::EldenRing,
            kind: ModKind::Assets,
            path: PathBuf::from(format!("C:\\library\\{id}")),
            natives: Vec::new(),
            file_count: 1,
            size_bytes: 1,
            installed_at: "now".into(),
            bundled_loader: None,
        }
    }

    fn profile_with(mods: &[&str], coop: bool) -> Profile {
        let mut profile = Profile::new(Game::EldenRing, "Test");
        profile.seamless_coop = coop;
        profile.mods = mods
            .iter()
            .map(|id| ProfileMod {
                mod_id: (*id).into(),
                enabled: true,
            })
            .collect();
        profile
    }

    fn input<'a>(
        install: &'a Installation,
        profile: &'a Profile,
        mods: &'a [ModRecord],
        loaders: &'a [LoaderInstall],
    ) -> PlanInput<'a> {
        PlanInput {
            install,
            profile,
            mods,
            loaders,
            work_dir: std::env::temp_dir().join("roundtable-plan"),
            steam_running: true,
        }
    }

    #[test]
    fn a_cracked_copy_with_mods_gets_skip_steam_init() {
        let install = install(InstallKind::Standalone, true);
        let profile = profile_with(&["convergence"], false);
        let mods = vec![mod_record("convergence")];
        let loaders = vec![loader(LoaderKind::Me3)];

        let plan = plan(&input(&install, &profile, &mods, &loaders)).unwrap();

        assert_eq!(plan.route, LaunchRoute::Me3);
        assert!(plan.skip_steam_init);
        let joined = plan.args.join(" ");
        assert!(joined.contains("--skip-steam-init true"), "got: {joined}");
        assert!(joined.contains("--exe"));
        assert!(plan
            .writes
            .iter()
            .any(|p| p.file_name().unwrap() == "steam_appid.txt"));
        assert!(plan.is_runnable());
    }

    #[test]
    fn a_steam_copy_does_not_get_skip_steam_init() {
        let install = install(InstallKind::Steam, true);
        let profile = profile_with(&["convergence"], false);
        let mods = vec![mod_record("convergence")];
        let loaders = vec![loader(LoaderKind::Me3)];

        let plan = plan(&input(&install, &profile, &mods, &loaders)).unwrap();
        assert!(!plan.skip_steam_init);
        assert!(!plan.args.join(" ").contains("--skip-steam-init"));
    }

    #[test]
    fn the_convergence_plus_coop_case_resolves_to_one_me3_launch() {
        let install = install(InstallKind::Standalone, true);
        let profile = profile_with(&["convergence"], true);
        let mods = vec![mod_record("convergence")];
        let loaders = vec![loader(LoaderKind::Me3)];

        let plan = plan(&input(&install, &profile, &mods, &loaders)).unwrap();

        assert_eq!(plan.route, LaunchRoute::Me3);
        assert!(plan.coop_enabled);
        assert!(plan.skip_steam_init);
        assert!(plan.is_runnable(), "this is the case that fails for people today");
    }

    #[test]
    fn coop_without_the_dll_installed_is_blocked_with_an_explanation() {
        let install = install(InstallKind::Standalone, false);
        let profile = profile_with(&["convergence"], true);
        let mods = vec![mod_record("convergence")];
        let loaders = vec![loader(LoaderKind::Me3)];

        let plan = plan(&input(&install, &profile, &mods, &loaders)).unwrap();
        assert!(!plan.is_runnable());
        assert!(plan
            .notices
            .iter()
            .any(|n| n.severity == Severity::Blocker && n.title.contains("Seamless Co-op")));
    }

    #[test]
    fn modengine2_on_a_repack_warns_about_the_steam_error() {
        let install = install(InstallKind::Standalone, false);
        let profile = profile_with(&["convergence"], false);
        let mods = vec![mod_record("convergence")];
        let loaders = vec![loader(LoaderKind::ModEngine2)];

        let plan = plan(&input(&install, &profile, &mods, &loaders)).unwrap();
        assert_eq!(plan.route, LaunchRoute::ModEngine2);
        assert!(!plan.skip_steam_init);
        assert!(plan
            .notices
            .iter()
            .any(|n| n.detail.contains("trying to find steam")));
        // It is a warning, not a blocker: with steam_appid.txt it often still works.
        assert!(plan.is_runnable());
    }

    #[test]
    fn me3_wins_when_both_loaders_are_present() {
        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&["a"], false);
        let mods = vec![mod_record("a")];
        let loaders = vec![loader(LoaderKind::ModEngine2), loader(LoaderKind::Me3)];

        assert_eq!(
            plan(&input(&install, &profile, &mods, &loaders)).unwrap().route,
            LaunchRoute::Me3
        );
    }

    #[test]
    fn coop_alone_uses_the_coop_launcher() {
        let install = install(InstallKind::Steam, true);
        let profile = profile_with(&[], true);
        let plan = plan(&input(&install, &profile, &[], &[])).unwrap();
        assert_eq!(plan.route, LaunchRoute::SeamlessCoopLauncher);
    }

    #[test]
    fn a_vanilla_profile_starts_the_game_directly() {
        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&[], false);
        let plan = plan(&input(&install, &profile, &[], &[])).unwrap();
        assert_eq!(plan.route, LaunchRoute::Direct);
        assert!(plan.args.is_empty());
        assert_eq!(plan.program, install.executable);
    }

    #[test]
    fn mods_without_a_loader_are_blocked() {
        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&["a"], false);
        let mods = vec![mod_record("a")];
        let plan = plan(&input(&install, &profile, &mods, &[])).unwrap();
        assert!(!plan.is_runnable());
        assert!(plan.notices.iter().any(|n| n.title == "No mod loader found"));
    }

    #[test]
    fn online_mode_with_coop_raises_a_ban_warning() {
        let install = install(InstallKind::Steam, true);
        let mut profile = profile_with(&[], true);
        profile.start_online = true;
        let plan = plan(&input(&install, &profile, &[], &[])).unwrap();
        assert!(plan
            .notices
            .iter()
            .any(|n| n.severity == Severity::Warning && n.detail.contains("ban")));
    }

    #[test]
    fn a_steam_copy_warns_when_steam_is_closed() {
        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&["a"], false);
        let mods = vec![mod_record("a")];
        let loaders = vec![loader(LoaderKind::Me3)];
        let mut plan_input = input(&install, &profile, &mods, &loaders);
        plan_input.steam_running = false;

        let plan = plan(&plan_input).unwrap();
        assert!(plan.notices.iter().any(|n| n.title == "Steam is not running"));
    }

    #[test]
    fn a_blocked_plan_refuses_to_spawn() {
        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&["a"], false);
        let mods = vec![mod_record("a")];
        let plan = plan(&input(&install, &profile, &mods, &[])).unwrap();
        assert!(spawn(&plan).is_err());
    }

    #[test]
    fn the_command_line_quotes_paths_with_spaces() {
        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&[], false);
        let plan = plan(&input(&install, &profile, &[], &[])).unwrap();
        assert!(plan.command_line().starts_with('"'));
        assert!(plan.command_line().contains("ELDEN RING"));
    }

    #[test]
    fn applying_a_me3_plan_writes_a_profile_with_absolute_paths() {
        let work = std::env::temp_dir().join("roundtable-apply-me3");
        std::fs::remove_dir_all(&work).ok();

        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&["convergence"], false);
        let mods = vec![mod_record("convergence")];
        let loaders = vec![loader(LoaderKind::Me3)];
        let mut plan_input = input(&install, &profile, &mods, &loaders);
        plan_input.work_dir = work.clone();

        let plan = plan(&plan_input).unwrap();
        let report = apply(&plan_input, &plan).unwrap();

        let written = work.join(format!("{}.me3", profile.id));
        assert!(written.is_file());
        let text = std::fs::read_to_string(&written).unwrap();
        assert!(text.contains("profileVersion = \"v1\""));
        assert!(text.contains("C:\\library\\convergence"));
        assert!(text.contains("game = \"eldenring\""));
        assert!(!report.changes.is_empty());

        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn load_order_is_reversed_for_me3_so_the_first_mod_still_wins() {
        let work = std::env::temp_dir().join("roundtable-apply-order");
        std::fs::remove_dir_all(&work).ok();

        let install = install(InstallKind::Steam, false);
        let profile = profile_with(&["first", "second"], false);
        let mods = vec![mod_record("first"), mod_record("second")];
        let loaders = vec![loader(LoaderKind::Me3)];
        let mut plan_input = input(&install, &profile, &mods, &loaders);
        plan_input.work_dir = work.clone();

        let plan = plan(&plan_input).unwrap();
        apply(&plan_input, &plan).unwrap();

        let text = std::fs::read_to_string(work.join(format!("{}.me3", profile.id))).unwrap();
        let second_at = text.find("library\\second").expect("second package present");
        let first_at = text.find("library\\first").expect("first package present");
        assert!(
            second_at < first_at,
            "me3 layers later packages on top, so the user's first choice must be written last"
        );

        std::fs::remove_dir_all(&work).ok();
    }
}
