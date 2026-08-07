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
  LanguageStatus,
  LaunchResult,
  LoaderInstall,
  ModRecord,
  PatchReport,
  PerfStatus,
  PreparedLaunch,
  Profile,
  ScanState,
  SaveFolder,
  SaveSummary,
  Settings,
  SteamAccount,
  AskAnswer,
  ErssStatus,
  AskTurn,
  AskEvent,
  SystemReport,
  TransferReport,
  TuneResult,
  TuneStatus,
  UnlockReport,
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
    // The fragment has to survive. It carries which screen was asked for — the
    // overlay is `#/overlay` and a wiki deep link is `#wiki:source:title` — and
    // rebuilding the address from the path alone silently dropped it, so the
    // overlay window opened on the launcher instead.
    window.history.replaceState(
      {},
      "",
      window.location.pathname + (rest ? `?${rest}` : "") + window.location.hash,
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

/**
 * A response that arrives a line at a time.
 *
 * One JSON object per line, handed over as it lands rather than at the end. A
 * chunk from the network can stop in the middle of a line, so the remainder is
 * carried forward — reading each chunk as if it were whole loses an event every
 * few hundred characters and it is never the same one twice.
 */
async function* lines(path: string, payload: unknown, signal?: AbortSignal) {
  const response = await fetch(new URL(BASE + path, window.location.origin), {
    method: "POST",
    headers: { "x-roundtable-key": KEY, "content-type": "application/json" },
    body: JSON.stringify(payload),
    signal,
  });

  if (!response.ok || !response.body) {
    throw new Error(`Request failed: ${response.status}`);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let held = "";

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      held += decoder.decode(value, { stream: true });

      let cut;
      while ((cut = held.indexOf("\n")) !== -1) {
        const line = held.slice(0, cut).trim();
        held = held.slice(cut + 1);
        if (!line) continue;
        try {
          yield JSON.parse(line) as unknown;
        } catch {
          // One malformed line is one lost event, not a failed answer.
        }
      }
    }
  } finally {
    reader.cancel().catch(() => {});
  }
}

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
  /** Searches every drive. Returns at once; watch installsScanState. */
  installsScan: (game: GameId) => post<{ started: boolean }>(`/installs/scan?game=${game}`, {}),
  installsScanState: () => get<ScanState>("/installs/scan/state"),
  installsScanStop: () => post<{ ok: boolean }>("/installs/scan/stop", {}),

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
  /** Searches every drive. Watch installsScanState for progress. */
  editionScan: (game: GameId, edition: string) =>
    post<{ started: boolean }>("/editions/scan", { game, edition }),
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

  /* The graphics settings, and the one that turns 60 into 30. */
  perf: (game: GameId) => get<PerfStatus>("/perf", { game }),
  perfSmooth: (game: GameId) => post<string[]>("/perf/smooth", { game }),
  perfSet: (game: GameId, key: string, value: string) =>
    post<string>("/perf/set", { game, key, value }),
  /* Rewrites the frame cap in the running game. 0 puts the shipped 60 back. */
  perfUnlock: (game: GameId, fps: number) =>
    post<UnlockReport>("/perf/unlock", { game, fps }),
  /* Rebuilds the display mode, which unsticks a juddering pointer. */
  perfBounce: () => post<string>("/perf/bounce", {}),

  /* The Windows levers the game cannot reach, and the one button for all of it. */
  tune: (game: GameId) => get<TuneStatus>("/tune", { game }),
  tuneApply: (game: GameId) => post<TuneResult>("/tune", { game }),
  tuneRevert: () => post<string[]>("/tune/revert", {}),

  /* A question about the game, answered out of the mirrored wiki. */
  ask: (question: string, edition?: string | null) =>
    post<AskAnswer>("/ask", { question, edition: edition ?? null }),
  /* The same, reported as it happens: what it is reading, then the answer as
     the model writes it. `history` is what was said before, so a follow-up can
     say "her". */
  askStream: (
    question: string,
    options: { edition?: string | null; history?: AskTurn[]; signal?: AbortSignal } = {},
  ) =>
    lines(
      "/ask/stream",
      {
        question,
        edition: options.edition ?? null,
        history: options.history ?? [],
      },
      options.signal,
    ) as AsyncGenerator<AskEvent>,
  /* The overlay closing itself. It reaches its own window through the server,
     because the page has no Tauri bridge — most of the time it is a browser. */
  overlayHide: () => post<{ ok: boolean }>("/overlay/hide", {}),
  /* Hands the window to the window manager, which follows the mouse itself. */
  overlayDrag: () => post<{ ok: boolean }>("/overlay/drag", {}),
  overlayCentre: () => post<{ ok: boolean }>("/overlay/centre", {}),

  /* DLSS, frame generation and Reflex, in a game that ships with none. */
  erss: (game: GameId) => get<ErssStatus>("/erss", { game }),
  erssInstall: (game: GameId, steamOverlay: boolean, password?: string) =>
    post<{ changes: string[] }>("/erss", { game, steamOverlay, password: password ?? null }),
  erssUninstall: (game: GameId) => post<string[]>("/erss/uninstall", { game }),
  /* One of the mod's own settings, changed before the game starts. */
  erssSet: (game: GameId, key: string, value: string) =>
    post<string>("/erss/set", { game, key, value }),
  /* Everything across all three configs that shows up as a generated-frame artefact. */
  erssTune: (game: GameId) => post<string[]>("/erss/tune", { game }),

  /* What language the emulated Steam tells the game to use. */
  language: (game: GameId) => get<LanguageStatus>("/language", { game }),
  languageSet: (game: GameId, language: string) =>
    post<string[]>("/language", { game, language }),

  /* Puts a total conversion's own text into that language. */
  editionTextInstall: (game: GameId, edition: string, language: string, archive?: string) =>
    post<string[]>("/language/edition", { game, edition, language, archive: archive ?? null }),
  editionTextRevert: (game: GameId, edition: string, language: string) =>
    post<string[]>("/language/edition/revert", { game, edition, language, archive: null }),

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
