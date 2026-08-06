import type {
  BackupRecord,
  CacheLocation,
  CleanReport,
  CodexResult,
  CodexState,
  Comparison,
  ConflictReport,
  ConversionReport,
  CoopSettings,
  EacStatus,
  EditionInstall,
  EditionJob,
  EditionStatus,
  FieldSpec,
  DiagnoseReport,
  Fingerprint,
  WikiPage,
  WikiSearchResult,
  GameId,
  GameInfo,
  Installation,
  LaunchResult,
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
 * The interface runs in a real browser and talks to the launcher over HTTP on
 * loopback. The session key arrives in the URL when the launch screen opens the
 * tab; it is moved into memory and stripped from the address bar so it is not
 * left sitting in history or copied along with a shared link.
 */
const KEY = (() => {
  const params = new URLSearchParams(window.location.search);
  const supplied = params.get("k");
  if (supplied) {
    sessionStorage.setItem("rt-key", supplied);
    params.delete("k");
    const rest = params.toString();
    window.history.replaceState(
      {},
      "",
      window.location.pathname + (rest ? `?${rest}` : ""),
    );
    return supplied;
  }
  return sessionStorage.getItem("rt-key") ?? "";
})();

const BASE = "/api";

async function request<T>(
  method: "GET" | "POST",
  path: string,
  payload?: unknown,
  query?: Record<string, string | number | undefined>,
): Promise<T> {
  const url = new URL(BASE + path, window.location.origin);
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value !== undefined) url.searchParams.set(key, String(value));
  }

  const response = await fetch(url, {
    method,
    headers: {
      "x-roundtable-key": KEY,
      ...(payload === undefined ? {} : { "content-type": "application/json" }),
    },
    body: payload === undefined ? undefined : JSON.stringify(payload),
  });

  if (response.status === 401) {
    throw new Error("This tab has lost its session. Reopen it from the launcher.");
  }

  const text = await response.text();
  const body = text ? JSON.parse(text) : null;

  if (!response.ok) {
    throw new Error(body?.error ?? `Request failed: ${response.status}`);
  }
  return body as T;
}

const get = <T>(path: string, query?: Record<string, string | number | undefined>) =>
  request<T>("GET", path, undefined, query);
const post = <T>(path: string, payload?: unknown) => request<T>("POST", path, payload);

export const api = {
  games: () => get<GameInfo[]>("/games"),

  settingsGet: () => get<Settings>("/settings"),
  settingsSet: (settings: Settings) => post<Settings>("/settings", settings),

  steamAccounts: () => get<SteamAccount[]>("/steam/accounts"),

  installsDiscover: (game: GameId) => get<Installation[]>("/installs/discover", { game }),
  /** Null when the game has not been located yet, which is not an error. */
  installsActive: (game: GameId) => get<Installation | null>("/installs/active", { game }),
  installsRemember: (game: GameId, path: string, makeDefault: boolean) =>
    post<Installation>("/installs/remember", { game, path, makeDefault }),
  installsForget: (game: GameId, path: string) =>
    post<void>("/installs/forget", { game, path }),

  loadersDiscover: (game: GameId) => get<LoaderInstall[]>("/loaders", { game }),

  /** Null while the game has not been located; there is nothing to inspect. */
  eacStatus: (game: GameId) => get<EacStatus | null>("/eac", { game }),
  eacSet: (game: GameId, enabled: boolean) => post<EacStatus>("/eac", { game, enabled }),

  coopFields: () => get<FieldSpec[]>("/coop/fields"),
  coopRead: (game: GameId) => get<CoopSettings>("/coop", { game }),
  coopWrite: (game: GameId, changes: Record<string, string>) =>
    post<CoopSettings>("/coop", { game, changes }),
  coopGeneratePassword: async () =>
    (await get<{ password: string }>("/coop/password")).password,

  modsList: (game: GameId) => get<ModRecord[]>("/mods", { game }),
  /** Takes a folder or an archive; the path comes from a native picker. */
  modsInstall: (game: GameId, path: string, name?: string) =>
    post<ModRecord>("/mods/install", { game, path, name }),
  modsDelete: (game: GameId, id: string) => post<void>("/mods/delete", { game, id }),

  profilesList: (game: GameId) => get<Profile[]>("/profiles", { game }),
  profileCreate: (game: GameId, name: string) =>
    post<Profile>("/profiles/create", { game, name }),
  profileSave: (profile: Profile) => post<Profile>("/profiles/save", profile),
  profileDelete: (game: GameId, id: string) => post<void>("/profiles/delete", { game, id }),
  profileConflicts: (game: GameId, id: string) =>
    get<ConflictReport>("/profiles/conflicts", { game, profile: id }),

  launchPlan: (game: GameId, profileId: string) =>
    get<PreparedLaunch>("/launch/plan", { game, profile: profileId }),
  launchPatch: (game: GameId, profileId: string) =>
    post<PatchReport>("/launch/patch", { game, profile: profileId }),
  launchRun: (game: GameId, profileId: string) =>
    post<LaunchResult>("/launch/run", { game, profile: profileId }),

  /** Which title is running, if any. One call covers every game. */
  runningGame: async () => (await get<{ game: GameId | null }>("/running")).game,

  /* Editions: total conversions with their own loader, saves and cover. */
  editions: (game: GameId, coop: boolean) =>
    get<EditionStatus[]>("/editions", { game, coop: String(coop) }),
  editionLocate: (edition: string, path: string) =>
    post<EditionInstall>("/editions/locate", { edition, path }),
  editionPatch: (game: GameId, edition: string, coop: boolean) =>
    post<PatchReport>("/editions/patch", { game, edition, coop }),
  editionRun: (game: GameId, edition: string, coop: boolean) =>
    post<{ pid: number; route: string }>("/editions/run", { game, edition, coop }),
  /** Returns as soon as unpacking starts; watch `editionJob` for progress. */
  editionInstall: (game: GameId, edition: string, archive: string, destination?: string) =>
    post<{ started: boolean }>("/editions/install", { game, edition, archive, destination }),
  editionJob: () => get<EditionJob>("/editions/job"),

  /* The codex. Cached on disk, so search never touches the network. */
  codex: (query: string, kind?: string, edition?: string | null) =>
    get<CodexResult>("/codex", { q: query, kind, edition: edition ?? undefined }),
  codexSync: () => post<{ started: boolean }>("/codex/sync", {}),
  codexState: () => get<CodexState>("/codex/state"),

  /* The wikis, mirrored. Titles are indexed in full; bodies arrive on open. */
  wiki: (query: string, edition?: string | null, source?: string, limit?: number) =>
    get<WikiSearchResult>("/wiki", { q: query, edition: edition ?? undefined, source, limit }),
  wikiPage: (title: string, edition?: string | null, source?: string, refresh = false) =>
    get<WikiPage>("/wiki/page", {
      title,
      edition: edition ?? undefined,
      source,
      refresh: refresh ? "true" : undefined,
    }),
  wikiSync: (source?: string, edition?: string | null) =>
    post<{ started: boolean }>(`/wiki/sync?${new URLSearchParams({
      ...(source ? { source } : {}),
      ...(edition ? { edition } : {}),
    })}`, {}),

  /* Every check Roundtable can run against this machine. */
  diagnose: (game: GameId, edition?: string | null) =>
    get<DiagnoseReport>("/diagnose", { game, edition: edition ?? undefined }),

  /* Whether you and a friend can actually see each other. */
  matchFingerprint: (game: GameId, edition?: string | null) =>
    get<Fingerprint>("/match", { game, edition: edition ?? undefined }),
  matchCompare: (game: GameId, theirs: string, edition?: string | null) =>
    post<Comparison>("/match/compare", { game, theirs, edition: edition ?? null }),

  savesDiscover: (game: GameId) => get<SaveFolder[]>("/saves", { game }),
  savesInspect: (path: string) => get<SaveSummary>("/saves/inspect", { path }),
  savesBackups: (game: GameId) => get<BackupRecord[]>("/saves/backups", { game }),
  savesBackup: (game: GameId, path: string, label: string) =>
    post<BackupRecord>("/saves/backup", { game, path, label }),
  savesRestore: (game: GameId, backupId: string) =>
    post<string>("/saves/restore", { game, id: backupId }),
  savesDeleteBackup: (game: GameId, backupId: string) =>
    post<void>("/saves/backup/delete", { game, id: backupId }),
  savesTransfer: (
    game: GameId,
    source: string,
    destination: string,
    slotPairs: [number, number][],
  ) => post<TransferReport>("/saves/transfer", { game, source, destination, slotPairs }),
  savesConvert: (game: GameId, source: string, extension: string, rebindTo?: number) =>
    post<ConversionReport>("/saves/convert", { game, source, extension, rebindTo }),

  sysShaderCaches: () => get<CacheLocation[]>("/sys/caches"),
  sysClearCaches: (paths: string[]) => post<CleanReport>("/sys/caches/clear", { paths }),
  sysReport: (game: GameId) => get<SystemReport>("/sys/report", { game }),

  openPath: (path: string) => post<void>("/open", { path }),

  /**
   * Native pickers. A browser will not tell a page where a folder is, so the
   * launcher's own window opens the real dialog and returns the path.
   */
  pickFolder: async (title?: string) =>
    (await get<{ path: string | null }>("/pick/folder", { title })).path,
  pickFile: async (title?: string, filter?: string) =>
    (await get<{ path: string | null }>("/pick/file", { title, filter })).path,
};
