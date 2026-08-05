import { invoke } from "@tauri-apps/api/core";
import type {
  BackupRecord,
  CacheLocation,
  CleanReport,
  ConflictReport,
  ConversionReport,
  CoopSettings,
  DuplicateGroup,
  EacStatus,
  FieldSpec,
  GameId,
  GameInfo,
  Installation,
  LaunchResult,
  LayoutAnalysis,
  LoaderInstall,
  ModRecord,
  PatchReport,
  PreparedLaunch,
  Profile,
  SaveFolder,
  SaveSummary,
  Settings,
  SteamAccount,
  SystemReport,
  TransferReport,
} from "./types";

/**
 * Every backend call goes through here so failures surface as real Error objects
 * rather than the bare strings Tauri hands back.
 */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const message =
      typeof error === "string"
        ? error
        : error instanceof Error
          ? error.message
          : JSON.stringify(error);
    throw new Error(message);
  }
}

export const api = {
  games: () => call<GameInfo[]>("games_list"),

  settingsGet: () => call<Settings>("settings_get"),
  settingsSet: (settings: Settings) => call<Settings>("settings_set", { settings }),

  steamAccounts: () => call<SteamAccount[]>("steam_accounts"),

  installsDiscover: (game: GameId) => call<Installation[]>("installs_discover", { game }),
  installsProbe: (game: GameId, path: string) =>
    call<Installation>("installs_probe", { game, path }),
  installsDeepScan: (game: GameId, root: string, maxDepth?: number) =>
    call<string[]>("installs_deep_scan", { game, root, maxDepth }),
  installsSaved: (game: GameId) => call<Installation[]>("installs_saved", { game }),
  installsRemember: (game: GameId, path: string, makeDefault: boolean) =>
    call<Installation>("installs_remember", { game, path, makeDefault }),
  installsForget: (game: GameId, path: string) =>
    call<void>("installs_forget", { game, path }),
  installsActive: (game: GameId) => call<Installation>("installs_active", { game }),
  installsSize: (path: string) => call<number>("installs_size", { path }),

  loadersDiscover: (game: GameId) => call<LoaderInstall[]>("loaders_discover", { game }),

  eacStatus: (game: GameId) => call<EacStatus>("eac_status", { game }),
  eacSet: (game: GameId, enabled: boolean) => call<EacStatus>("eac_set", { game, enabled }),

  coopFields: () => call<FieldSpec[]>("coop_fields"),
  coopRead: (game: GameId) => call<CoopSettings>("coop_read", { game }),
  coopWrite: (game: GameId, changes: Record<string, string>) =>
    call<CoopSettings>("coop_write", { game, changes }),
  coopGeneratePassword: () => call<string>("coop_generate_password"),

  modsList: (game: GameId) => call<ModRecord[]>("mods_list", { game }),
  modsAnalyse: (path: string) => call<LayoutAnalysis>("mods_analyse", { path }),
  modsInstallFolder: (game: GameId, source: string, name?: string) =>
    call<ModRecord>("mods_install_folder", { game, source, name }),
  modsInstallArchive: (game: GameId, archive: string, name?: string) =>
    call<ModRecord>("mods_install_archive", { game, archive, name }),
  modsDelete: (game: GameId, id: string) => call<void>("mods_delete", { game, id }),
  modsUpdate: (record: ModRecord) => call<void>("mods_update", { record }),

  profilesList: (game: GameId) => call<Profile[]>("profiles_list", { game }),
  profileCreate: (game: GameId, name: string) =>
    call<Profile>("profile_create", { game, name }),
  profileSave: (profile: Profile) => call<Profile>("profile_save", { profile }),
  profileDelete: (game: GameId, id: string) => call<void>("profile_delete", { game, id }),
  profileClone: (game: GameId, id: string, name: string) =>
    call<Profile>("profile_clone", { game, id, name }),
  profileConflicts: (game: GameId, id: string) =>
    call<ConflictReport>("profile_conflicts", { game, id }),

  launchPlan: (game: GameId, profileId: string) =>
    call<PreparedLaunch>("launch_plan", { game, profileId }),
  launchPatch: (game: GameId, profileId: string) =>
    call<PatchReport>("launch_patch", { game, profileId }),
  launchRun: (game: GameId, profileId: string) =>
    call<LaunchResult>("launch_run", { game, profileId }),
  gameIsRunning: (game: GameId) => call<boolean>("game_is_running", { game }),

  savesDiscover: (game: GameId) => call<SaveFolder[]>("saves_discover", { game }),
  savesInspect: (path: string) => call<SaveSummary>("saves_inspect", { path }),
  savesBackup: (game: GameId, path: string, label: string) =>
    call<BackupRecord>("saves_backup", { game, path, label }),
  savesBackups: (game: GameId) => call<BackupRecord[]>("saves_backups", { game }),
  savesRestore: (game: GameId, backupId: string, destination?: string) =>
    call<string>("saves_restore", { game, backupId, destination }),
  savesDeleteBackup: (game: GameId, backupId: string) =>
    call<void>("saves_delete_backup", { game, backupId }),
  savesTransfer: (
    game: GameId,
    source: string,
    destination: string,
    slotPairs: [number, number][],
  ) => call<TransferReport>("saves_transfer", { game, source, destination, slotPairs }),
  savesConvert: (
    game: GameId,
    source: string,
    extension: string,
    destinationDir?: string,
    rebindTo?: number,
  ) =>
    call<ConversionReport>("saves_convert", {
      game,
      source,
      extension,
      destinationDir,
      rebindTo,
    }),
  savesRebind: (game: GameId, path: string, steamId: number) =>
    call<string>("saves_rebind", { game, path, steamId }),
  savesDuplicates: (paths: string[]) => call<DuplicateGroup[]>("saves_duplicates", { paths }),

  sysShaderCaches: () => call<CacheLocation[]>("sys_shader_caches"),
  sysClearCaches: (paths: string[]) => call<CleanReport>("sys_clear_caches", { paths }),
  sysReport: (game: GameId) => call<SystemReport>("sys_report", { game }),

  openPath: (path: string) => call<void>("open_path", { path }),
};
