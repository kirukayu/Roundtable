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
}
