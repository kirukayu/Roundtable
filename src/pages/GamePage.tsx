import { useCallback, useEffect, useState } from "react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

import { Icon } from "../components/Icons";
import { Blank, Card, Chip, NoticeBlock, Skeleton, useToast } from "../components/ui";
import { api } from "../lib/ipc";
import { when } from "../lib/format";
import { useApp } from "../lib/store";
import type {
  EacStatus,
  GameId,
  Installation,
  LoaderInstall,
  PreparedLaunch,
  Profile,
} from "../lib/types";

import ModsTab from "./tabs/ModsTab";
import SavesTab from "./tabs/SavesTab";
import CoopTab from "./tabs/CoopTab";
import SystemTab from "./tabs/SystemTab";

type Tab = "play" | "mods" | "saves" | "coop" | "system";

/**
 * Everything about one game, in one place.
 *
 * The top third is the game: art, title, state, and the Play button. Below it,
 * tabs for the things you only look at occasionally. You should never have to
 * leave this page mid-thought.
 */
export default function GamePage({
  gameId,
  onBack,
}: {
  gameId: GameId;
  onBack: () => void;
}) {
  const { games, profiles, settings, patchSettings, gameRunning, refreshProfiles, refreshInstalled } =
    useApp();
  const toast = useToast();

  const game = games.find((g) => g.id === gameId);
  const [tab, setTab] = useState<Tab>("play");
  const [install, setInstall] = useState<Installation | null>(null);
  const [loading, setLoading] = useState(true);
  const [loaders, setLoaders] = useState<LoaderInstall[]>([]);
  const [eac, setEac] = useState<EacStatus | null>(null);
  const [prepared, setPrepared] = useState<PreparedLaunch | null>(null);
  const [busy, setBusy] = useState(false);

  const gameProfiles = profiles.filter((p) => p.game === gameId);
  const active =
    gameProfiles.find((p) => p.id === settings.activeProfile) ?? gameProfiles[0] ?? null;
  const running = gameRunning === gameId;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const found = await api.installsActive(gameId);
      setInstall(found);
      const [foundLoaders, foundEac] = await Promise.all([
        api.loadersDiscover(gameId).catch(() => []),
        api.eacStatus(gameId).catch(() => null),
      ]);
      setLoaders(foundLoaders);
      setEac(foundEac);
    } catch {
      setInstall(null);
    } finally {
      setLoading(false);
    }
  }, [gameId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!install || !active) {
      setPrepared(null);
      return;
    }
    api
      .launchPlan(gameId, active.id)
      .then(setPrepared)
      .catch(() => setPrepared(null));
  }, [gameId, install, active]);

  if (!game) return null;

  const locate = async () => {
    const picked = await pickFolder({ directory: true, title: `Where is ${game.name}?` });
    if (typeof picked !== "string") return;
    const added = await toast.run(`${game.name} added`, () =>
      api.installsRemember(gameId, picked, true),
    );
    if (added) {
      await refreshInstalled();
      await load();
    }
  };

  const play = async () => {
    if (!active) return;
    setBusy(true);
    try {
      const result = await api.launchRun(gameId, active.id);
      toast.success(
        `${active.name} started`,
        `${result.route}${result.backupId ? " · save backed up" : ""}`,
      );
      await refreshProfiles();
    } catch (error) {
      toast.error("Could not start", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const createProfile = async () => {
    const made = await toast.run("Profile created", () =>
      api.profileCreate(gameId, gameProfiles.length === 0 ? "Default" : "New profile"),
    );
    if (made) {
      await refreshProfiles();
      void patchSettings({ activeProfile: made.id });
      setTab("mods");
    }
  };

  const blockers = prepared?.plan.notices.filter((n) => n.severity === "blocker") ?? [];
  const warnings = prepared?.plan.notices.filter((n) => n.severity !== "blocker") ?? [];

  const tabs: { id: Tab; label: string; glyph: (p: { size?: number }) => React.ReactNode }[] = [
    { id: "play", label: "Play", glyph: Icon.Play },
    { id: "mods", label: "Mods", glyph: Icon.Layers },
    { id: "saves", label: "Saves", glyph: Icon.Save },
    ...(game.supportsSeamlessCoop
      ? [{ id: "coop" as Tab, label: "Co-op", glyph: Icon.Users }]
      : []),
    { id: "system", label: "System", glyph: Icon.Tools },
  ];

  return (
    <div className="view">
      <section className="hero">
        <div className="hero__art" style={{ backgroundImage: `url(${game.heroUrl})` }} />
        <div className="hero__wash" />

        <button
          type="button"
          className="btn btn--ghost btn--icon"
          onClick={onBack}
          aria-label="Back to library"
          style={{
            position: "absolute",
            top: "var(--s5)",
            left: "var(--s5)",
            zIndex: 2,
            background: "rgba(8,10,13,0.6)",
            backdropFilter: "blur(8px)",
          }}
        >
          <Icon.Back size={17} />
        </button>

        <div className="hero__body">
          <div className="hero__cover">
            <img src={game.coverUrl} alt="" />
          </div>

          <div className="grow">
            <div className="row wrap" style={{ gap: "var(--s2)", marginBottom: "var(--s3)" }}>
              {running ? (
                <Chip tone="success">
                  <span className="dot pulse" />
                  Running
                </Chip>
              ) : install ? (
                <Chip tone="accent">
                  {install.kind === "steam" ? "Steam" : install.kind === "standalone" ? "Standalone" : "Installed"}
                </Chip>
              ) : (
                <Chip tone="warning">Not located</Chip>
              )}
              {install?.version && <Chip>{install.version}</Chip>}
              {install?.hasSeamlessCoop && <Chip tone="info">Seamless Co-op</Chip>}
              {eac?.state === "bypassed" && <Chip tone="error">Anti-cheat off</Chip>}
              {loaders.map((l) => (
                <Chip key={l.executable}>{l.kind === "me3" ? "me3" : "ModEngine 2"}</Chip>
              ))}
            </div>

            <h1 className="hero__title">{game.name}</h1>

            <div className="row wrap" style={{ gap: "var(--s3)", marginTop: "var(--s5)" }}>
              {!install ? (
                <button type="button" className="btn btn--play" onClick={locate}>
                  <Icon.Folder size={18} />
                  Locate game
                </button>
              ) : !active ? (
                <button type="button" className="btn btn--play" onClick={createProfile}>
                  <Icon.Plus size={18} />
                  Create a profile
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    className="btn btn--play"
                    onClick={play}
                    disabled={busy || running || blockers.length > 0}
                  >
                    <Icon.Play size={18} />
                    {busy ? "Starting" : running ? "Running" : "Play"}
                  </button>

                  {gameProfiles.length > 1 && (
                    <select
                      className="select"
                      style={{ width: 200, height: 52, borderRadius: "var(--r)" }}
                      value={active.id}
                      onChange={(event) =>
                        void patchSettings({ activeProfile: event.target.value })
                      }
                      aria-label="Profile"
                    >
                      {gameProfiles.map((profile) => (
                        <option key={profile.id} value={profile.id}>
                          {profile.name}
                        </option>
                      ))}
                    </select>
                  )}
                </>
              )}
            </div>
          </div>
        </div>
      </section>

      <div className="pad" style={{ paddingTop: "var(--s5)" }}>
        <div className="tabs" style={{ marginBottom: "var(--s5)" }}>
          {tabs.map((entry) => {
            const Glyph = entry.glyph;
            return (
              <button
                key={entry.id}
                type="button"
                role="tab"
                className="tab"
                aria-selected={tab === entry.id}
                onClick={() => setTab(entry.id)}
              >
                <Glyph size={14} />
                {entry.label}
              </button>
            );
          })}
        </div>

        {loading ? (
          <div className="grid-2">
            <Card><Skeleton variant="line" count={4} /></Card>
            <Card><Skeleton variant="line" count={4} /></Card>
          </div>
        ) : !install ? (
          <Card>
            <Blank
              icon={Icon.Folder}
              title={`${game.name} is not set up yet`}
              action={
                <button type="button" className="btn btn--primary" onClick={locate}>
                  <Icon.Folder size={15} />
                  Choose the folder
                </button>
              }
            >
              Point Roundtable at the folder containing{" "}
              <span className="mono">Game\{game.executable}</span>. Steam copies and
              standalone ones both work.
            </Blank>
          </Card>
        ) : (
          <>
            {tab === "play" && (
              <PlayTab
                install={install}
                profile={active}
                profiles={gameProfiles}
                prepared={prepared}
                blockers={blockers}
                warnings={warnings}
                onCreateProfile={createProfile}
                onEditProfiles={() => setTab("mods")}
                onPatch={async () => {
                  if (!active) return;
                  await toast.run("Configuration written", () =>
                    api.launchPatch(gameId, active.id),
                  );
                }}
              />
            )}
            {tab === "mods" && (
              <ModsTab
                gameId={gameId}
                profile={active}
                profiles={gameProfiles}
                onChanged={refreshProfiles}
              />
            )}
            {tab === "saves" && <SavesTab gameId={gameId} />}
            {tab === "coop" && <CoopTab gameId={gameId} install={install} />}
            {tab === "system" && (
              <SystemTab
                gameId={gameId}
                install={install}
                eac={eac}
                onEacChanged={setEac}
                onForget={async () => {
                  await api.installsForget(gameId, install.root);
                  await refreshInstalled();
                  onBack();
                }}
              />
            )}
          </>
        )}
      </div>
    </div>
  );
}

function PlayTab({
  install,
  profile,
  profiles,
  prepared,
  blockers,
  warnings,
  onCreateProfile,
  onEditProfiles,
  onPatch,
}: {
  install: Installation;
  profile: Profile | null;
  profiles: Profile[];
  prepared: PreparedLaunch | null;
  blockers: import("../lib/types").Notice[];
  warnings: import("../lib/types").Notice[];
  onCreateProfile: () => void;
  onEditProfiles: () => void;
  onPatch: () => Promise<void>;
}) {
  if (!profile) {
    return (
      <Card>
        <Blank
          icon={Icon.Layers}
          title="No profile yet"
          action={
            <button type="button" className="btn btn--primary" onClick={onCreateProfile}>
              <Icon.Plus size={15} />
              Create one
            </button>
          }
        >
          A profile holds a load order and the settings that go with it. Make one for
          vanilla, one for your overhaul, and switch in a click.
        </Blank>
      </Card>
    );
  }

  return (
    <div className="col reveal">
      {blockers.map((notice, index) => (
        <NoticeBlock key={index} notice={notice} />
      ))}
      {warnings.map((notice, index) => (
        <NoticeBlock key={index} notice={notice} />
      ))}

      <div className="grid-2">
        <Card
          title="This launch"
          action={
            <button type="button" className="btn btn--ghost btn--sm" onClick={onPatch}>
              Write config
            </button>
          }
        >
          {prepared ? (
            <>
              <div className="row wrap" style={{ gap: "var(--s2)", marginBottom: "var(--s4)" }}>
                <Chip tone="accent">{routeName(prepared.plan.route)}</Chip>
                {prepared.plan.coopEnabled && <Chip tone="success">Co-op</Chip>}
                {prepared.plan.skipSteamInit && <Chip tone="info">skip-steam-init</Chip>}
              </div>
              <ol
                className="col-sm"
                style={{ margin: 0, paddingLeft: 18, fontSize: "var(--text-sm)" }}
              >
                {prepared.plan.steps.map((step, index) => (
                  <li key={index} className="dim">
                    {step}
                  </li>
                ))}
              </ol>
            </>
          ) : (
            <Skeleton variant="line" count={3} />
          )}
        </Card>

        <Card
          title="Profile"
          action={
            <button type="button" className="btn btn--ghost btn--sm" onClick={onEditProfiles}>
              Edit
            </button>
          }
        >
          <div className="col-sm">
            <div className="row-between">
              <span className="dim" style={{ fontSize: "var(--text-sm)" }}>Name</span>
              <strong style={{ fontSize: "var(--text-sm)" }}>{profile.name}</strong>
            </div>
            <div className="row-between">
              <span className="dim" style={{ fontSize: "var(--text-sm)" }}>Mods enabled</span>
              <strong style={{ fontSize: "var(--text-sm)" }}>
                {profile.mods.filter((m) => m.enabled).length}
              </strong>
            </div>
            <div className="row-between">
              <span className="dim" style={{ fontSize: "var(--text-sm)" }}>Save file</span>
              <span className="mono">{profile.savefile ?? "shared"}</span>
            </div>
            <div className="row-between">
              <span className="dim" style={{ fontSize: "var(--text-sm)" }}>Last played</span>
              <span style={{ fontSize: "var(--text-sm)" }}>{when(profile.lastPlayed)}</span>
            </div>
          </div>

          {profiles.length > 1 && (
            <>
              <hr className="divider" />
              <div className="row wrap" style={{ gap: "var(--s2)" }}>
                {profiles.map((entry) => (
                  <Chip key={entry.id} tone={entry.id === profile.id ? "accent" : undefined}>
                    {entry.name}
                  </Chip>
                ))}
              </div>
            </>
          )}
        </Card>
      </div>

      <Card title="Installation">
        <div className="col-sm">
          <div className="row-between">
            <span className="dim" style={{ fontSize: "var(--text-sm)" }}>Folder</span>
            <button
              type="button"
              className="btn btn--ghost btn--sm mono"
              onClick={() => void api.openPath(install.root)}
              title={install.root}
            >
              <Icon.Folder size={13} />
              Open
            </button>
          </div>
          <div className="row-between">
            <span className="dim" style={{ fontSize: "var(--text-sm)" }}>Type</span>
            <span style={{ fontSize: "var(--text-sm)" }}>
              {install.kind === "steam"
                ? "Steam"
                : install.kind === "standalone"
                  ? "Standalone"
                  : "Unrecognised"}
            </span>
          </div>
          {install.markers.length > 0 && (
            <div className="row-between">
              <span className="dim" style={{ fontSize: "var(--text-sm)" }}>Detected</span>
              <span className="mono faint truncate" style={{ maxWidth: 260 }}>
                {install.markers.join(", ")}
              </span>
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}

function routeName(route: string): string {
  switch (route) {
    case "me3":
      return "me3";
    case "mod-engine2":
      return "ModEngine 2";
    case "seamless-coop-launcher":
      return "Co-op launcher";
    default:
      return "Direct";
  }
}
