export type GameId =
  | "elden-ring"
  | "nightreign"
  | "dark-souls-remastered"
  | "dark-souls2"
  | "dark-souls3"
  | "sekiro"
  | "armored-core6"
  | "bloodborne"
  | "demons-souls";

export interface GameInfo {
  id: GameId;
  name: string;
  short: string;
  year: number;
  /** One line of context shown under the title. */
  note: string;
  /** False for the console exclusives, which have no PC build to manage. */
  playable: boolean;
  steamAppId: number;
  executable: string;
  saveFile: string;
  supportsSeamlessCoop: boolean;
  supportsModengine2: boolean;
  supportsMe3: boolean;
  coverUrl: string | null;
  heroUrl: string | null;
  logoUrl: string | null;
}

export type InstallKind = "steam" | "standalone" | "unknown";

export interface Installation {
  game: GameId;
  root: string;
  gameDir: string;
  executable: string;
  kind: InstallKind;
  version: string | null;
  hasEac: boolean;
  eacBypassed: boolean;
  hasSeamlessCoop: boolean;
  seamlessCoopVersion: string | null;
  sizeBytes: number | null;
  markers: string[];
}

export type LoaderKind = "mod-engine2" | "me3";

export interface LoaderInstall {
  kind: LoaderKind;
  executable: string;
  directory: string;
  version: string | null;
  config: string | null;
}

export type EacState = "active" | "bypassed" | "not-present";

export interface EacStatus {
  state: EacState;
  shim: string | null;
  backup: string | null;
  detail: string;
}

export type FieldKind = "toggle" | "range" | "choice" | "text";

export interface FieldSpec {
  section: string;
  key: string;
  label: string;
  help: string;
  kind: FieldKind;
  default: string;
  min: number | null;
  max: number | null;
  options: [number, string][];
}

export interface CoopSettings {
  path: string;
  values: Record<string, string>;
  installed: boolean;
  dllVersion: string | null;
}

export type ModKind = "assets" | "native" | "mixed";

export interface ModRecord {
  id: string;
  name: string;
  version: string | null;
  author: string | null;
  summary: string | null;
  nexusModId: number | null;
  game: GameId;
  kind: ModKind;
  path: string;
  natives: string[];
  fileCount: number;
  sizeBytes: number;
  installedAt: string;
  bundledLoader: string | null;
}

export interface LayoutAnalysis {
  assetRoot: string;
  kind: ModKind;
  natives: string[];
  bundledLoader: string | null;
  recognised: boolean;
}

export interface ProfileMod {
  modId: string;
  enabled: boolean;
}

export interface Profile {
  id: string;
  name: string;
  game: GameId;
  mods: ProfileMod[];
  seamlessCoop: boolean;
  savefile: string | null;
  skipLogos: boolean;
  disableArxan: boolean;
  memPatch: boolean;
  startOnline: boolean;
  created: string;
  lastPlayed: string | null;
  notes: string | null;
}

export interface FileConflict {
  relativePath: string;
  providers: string[];
  winner: string;
  mergeable: boolean;
}

export interface ConflictReport {
  conflicts: FileConflict[];
  totalFiles: number;
  regulationProviders: string[];
}

export type LaunchRoute =
  | "me3"
  | "mod-engine2"
  | "seamless-coop-launcher"
  | "direct";

export type Severity = "info" | "warning" | "blocker";

export interface Notice {
  severity: Severity;
  title: string;
  detail: string;
}

export interface LaunchPlan {
  route: LaunchRoute;
  program: string;
  args: string[];
  workingDir: string;
  env: Record<string, string>;
  steps: string[];
  notices: Notice[];
  writes: string[];
  coopEnabled: boolean;
  skipSteamInit: boolean;
}

export interface PreparedLaunch {
  plan: LaunchPlan;
  commandLine: string;
}

/* ── Editions ─────────────────────────────────────────────────────── */

/** A total conversion Roundtable knows how to drive as its own game. */
export interface EditionSpec {
  id: string;
  game: GameId;
  name: string;
  short: string;
  note: string;
  site: string;
  savefile: string;
  savefileCoop: string;
}

export interface EditionInstall {
  id: string;
  name: string;
  root: string;
  version: string | null;
  me3: string | null;
  profile: string | null;
  profileCoop: string | null;
  coopDll: string | null;
  sizeBytes: number | null;
  /** The mod refuses to run from inside the game's own folder. */
  insideGameDir: boolean;
}

export interface EditionStatus {
  spec: EditionSpec;
  install: EditionInstall | null;
  plan: LaunchPlan | null;
  commandLine: string | null;
  suggestedDestination: string;
}

export interface EditionJob {
  edition: string;
  running: boolean;
  done: boolean;
  message: string;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  destination: string | null;
  error: string | null;
}

/* ── Codex ────────────────────────────────────────────────────────── */

export interface CodexFact {
  label: string;
  value: string;
}

export interface CodexHit {
  id: string;
  kind: string;
  kindLabel: string;
  name: string;
  image: string | null;
  description: string | null;
  facts: CodexFact[];
  /** Routed per edition: Fextralife normally, the mod's wiki when one is active. */
  wiki: string;
}

export interface CodexState {
  entries: number;
  kinds: number;
  syncing: boolean;
  message: string;
  doneKinds: number;
  totalKinds: number;
  error: string | null;
}

export interface CodexResult {
  hits: CodexHit[];
  total: number;
  /** [id, label, count] per collection. */
  kinds: [string, string, number][];
  state: CodexState;
}

/* ── Wiki ─────────────────────────────────────────────────────────── */

export interface WikiSource {
  id: string;
  name: string;
}

export interface WikiPage {
  source: string;
  title: string;
  /** Already stripped of scripts, handlers and javascript: links. */
  html: string;
  origin: string;
}

export interface WikiIndexState {
  source: string;
  titles: number;
  cachedPages: number;
  syncing: boolean;
  message: string;
  error: string | null;
}

export interface WikiSearchResult {
  source: WikiSource;
  sources: WikiSource[];
  titles: string[];
  state: WikiIndexState;
}

/** Progress of the whole-machine search for a game. */
export interface ScanState {
  running: boolean;
  done: boolean;
  /** The folder being looked at, so the wait shows something. */
  at: string;
  found: Installation[];
  cancelled: boolean;
}

/* ── Language ─────────────────────────────────────────────────────── */

export interface LanguageFile {
  file: string;
  path: string;
  value: string | null;
  /** The line exists but is commented out, which is the usual cause. */
  disabled: boolean;
}

/** A translation Roundtable already carries, so installing it is one click. */
export interface BundledText {
  version: string;
  author: string;
  source: string;
}

/** Whether a total conversion's own text exists in the game's language. */
export interface EditionText {
  edition: string;
  /** The folder the game reads, e.g. `rusru`. */
  locale: string;
  /** False when the mod ships one English archive copied into every locale. */
  translated: boolean;
  folder: string;
  bundled: BundledText | null;
  /** The mod's own text is kept aside, so this can be undone. */
  revertible: boolean;
}

export interface LanguageStatus {
  files: LanguageFile[];
  current: string | null;
  /** Two configs disagree, so one of them is being ignored. */
  conflict: boolean;
  options: [string, string][];
  selector: string | null;
  /** One per installed conversion that ships its own text. */
  editions: EditionText[];
}

/* ── Frame rate ───────────────────────────────────────────────────── */

/** What the preset was worked out from. */
export interface Machine {
  gpu: string | null;
  vramMb: number;
  ramMb: number;
  cores: number;
  width: number;
  height: number;
  refreshHz: number;
  tier: "weak" | "modest" | "strong" | "ample";
  /** The highest clean division of the panel this machine holds every frame. */
  suggestedCap: number;
}

/** An answer drawn out of the wiki, with the articles it came from. */
export interface AskAnswer {
  answer: string;
  sources: string[];
  /** Which model answered, and how long it took. */
  lane: string | null;
  ms: number | null;
}

/** One exchange, kept so the next question can refer back to it. */
export interface AskTurn {
  question: string;
  answer: string;
}

/**
 * Something that happened on the way to an answer.
 *
 * Reported as it happens rather than at the end: which articles are being read,
 * then the answer itself a few words at a time.
 */
export type AskEvent =
  /** What the model chose to do, in its own words: the search it wrote, the
      article it opened. Not a spinner — the actual step. */
  | { kind: "doing"; note: string }
  | { kind: "sources"; sources: string[] }
  | { kind: "delta"; text: string }
  | { kind: "done"; lane: string | null; ms: number | null }
  | { kind: "failed"; error: string };

/** A Windows setting the game cannot reach on its own. */
export interface Lever {
  id: string;
  title: string;
  detail: string;
  current: string;
  wanted: string;
  done: boolean;
  needsReboot: boolean;
  needsAdmin: boolean;
  /** Where to click, for the ones no program can set. */
  byHand: string | null;
}

export interface TuneStatus {
  levers: Lever[];
  /** Things holding the graphics card while the game runs. */
  competitors: string[];
}

/**
 * One value a setting can take.
 *
 * Separate from the value itself because the mod stores its enums as bare
 * numbers — DLSS Quality is written `2` — and showing the number helps nobody.
 */
export interface ErssChoice {
  value: string;
  label: string;
}

/** One of the mod's own settings, read out of its TOML. */
export interface ErssSetting {
  key: string;
  title: string;
  detail: string;
  value: string;
  kind: "bool" | "number" | "text";
  choices: ErssChoice[];
  /** False when Roundtable knows nothing about this key beyond its name. */
  described: boolean;
}

/** DLSS, frame generation and Reflex, added to a game that has none. */
export interface ErssStatus {
  settings: ErssSetting[];
  installed: boolean;
  loader: string | null;
  version: string | null;
  frameTimeAddon: boolean;
  archives: string[];
  /** The one that will be installed: the newest release found. */
  release: string | null;
  reshade: boolean;
  /** That release is encrypted. The published password is tried on its own. */
  locked: boolean;
  /** What stops it working until it is dealt with. */
  blockers: string[];
}

export interface TuneResult {
  changes: string[];
  competitors: string[];
}

/** What the frame cap patch did. */
export interface UnlockReport {
  fps: number;
  framelock: boolean;
  /** The hardcoded 60 Hz display request, cleared. This is the 30 fps one. */
  hertz: boolean;
}

export interface PerfSetting {
  key: string;
  value: string;
  /** What this would become, when it is costing frames. */
  suggested: string | null;
  reason: string | null;
}

export interface PerfStatus {
  path: string | null;
  settings: PerfSetting[];
  display: string | null;
  /** Forces 60 Hz with unbreakable vsync, which halves to 30 on a late frame. */
  exclusiveFullscreen: boolean;
  improvable: number;
  /** A third-party unlocker DLL somebody dropped in before. */
  unlocker: string | null;
  machine: Machine;
  /** The game is up, so its frame cap can be rewritten now. */
  gameRunning: boolean;
}

/* ── Diagnostics ──────────────────────────────────────────────────── */

export interface Finding {
  id: string;
  level: "blocker" | "warning" | "note" | "pass";
  title: string;
  detail: string;
  /** The error text this would produce, for recognising a problem you have. */
  symptom: string | null;
  fix: string | null;
}

export interface DiagnoseReport {
  findings: Finding[];
  blockers: number;
  warnings: number;
}

/* ── Co-op match check ────────────────────────────────────────────── */

export interface MatchTrait {
  key: string;
  label: string;
  value: string;
  matters: string;
}

export interface Fingerprint {
  traits: MatchTrait[];
  block: string;
}

export interface MatchDifference {
  label: string;
  mine: string;
  theirs: string;
  matters: string;
}

export interface Comparison {
  verdict: "match" | "differs" | "unreadable";
  differences: MatchDifference[];
  unknown: string[];
}

export interface PatchReport {
  route: LaunchRoute;
  written: string[];
  changes: string[];
  notices: Notice[];
}

export interface LaunchResult {
  pid: number;
  route: string;
  patched: PatchReport;
  backupId: string | null;
}

export type SaveFlavour = "vanilla" | "seamless-coop" | "game-backup";

export interface SlotSummary {
  index: number;
  active: boolean;
  name: string;
  level: number;
  secondsPlayed: number;
  steamId: number | null;
}

export interface SaveSummary {
  steamId: number;
  slots: SlotSummary[];
  byteLen: number;
  checksumsValid: boolean;
}

export interface SaveEntry {
  path: string;
  fileName: string;
  extension: string;
  flavour: SaveFlavour;
  sizeBytes: number;
  modified: string | null;
  folderId: number | null;
  accountName: string | null;
  likelyCracked: boolean;
  summary: SaveSummary | null;
  contentHash: string | null;
}

export interface SaveFolder {
  path: string;
  folderId: number | null;
  accountName: string | null;
  likelyCracked: boolean;
  entries: SaveEntry[];
}

export interface BackupRecord {
  id: string;
  game: GameId;
  created: string;
  label: string;
  origin: string;
  fileName: string;
  sizeBytes: number;
  steamId: number | null;
  characters: string[];
  automatic: boolean;
}

export interface TransferReport {
  destination: string;
  slotsCopied: number[];
  reboundFrom: number | null;
  reboundTo: number | null;
  backupId: string | null;
}

export interface ConversionReport {
  destination: string;
  rebound: boolean;
  overwroteExisting: boolean;
}

export interface DuplicateGroup {
  hash: string;
  sizeBytes: number;
  paths: string[];
}

export interface CacheLocation {
  label: string;
  path: string;
  exists: boolean;
  sizeBytes: number;
  fileCount: number;
  owner: string;
}

export interface CleanReport {
  cleared: string[];
  skipped: string[];
  bytesFreed: number;
  filesRemoved: number;
}

export interface DiskInfo {
  mount: string;
  totalBytes: number;
  availableBytes: number;
}

export interface SystemReport {
  os: string;
  cpu: string;
  cpuCores: number;
  totalMemoryBytes: number;
  availableMemoryBytes: number;
  disks: DiskInfo[];
  steamRunning: boolean;
  gameRunning: boolean;
}

export interface SteamAccount {
  steamId64: number;
  accountName: string;
  personaName: string;
  mostRecent: boolean;
}

export interface SavedInstall {
  game: GameId;
  root: string;
  isDefault: boolean;
  label: string | null;
}

export interface Settings {
  selectedGame: GameId;
  installations: SavedInstall[];
  favourites: GameId[];
  activeProfile: string | null;
  nexusApiKey: string | null;
  discordPresence: boolean;
  autoBackupOnLaunch: boolean;
  autoBackupKeep: number;
  theme: string;
  accent: string;
  uiScale: number;
  reduceMotion: boolean;
  language: string;
  useJunctionDeploy: boolean;
  confirmDestructive: boolean;
  downloadConnections: number;
  downloadDir: string | null;
  torrentPort: number;
  useDoh: boolean;
  firstRunComplete: boolean;
  /** Frame cap written into the game after it starts. Null leaves 60 alone. */
  unlockFps: number | null;
}
