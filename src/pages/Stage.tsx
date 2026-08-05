import { useCallback, useEffect, useRef, useState } from "react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

import { Icon } from "../components/Icons";
import { Blank, Card, Chip, Skeleton, useToast } from "../components/ui";
import { api } from "../lib/ipc";
import { useApp } from "../lib/store";
import type { EacStatus, GameInfo, Installation, LoaderInstall, PreparedLaunch } from "../lib/types";

import PlayPane from "./panes/PlayPane";
import ModsPane from "./panes/ModsPane";
import SavesPane from "./panes/SavesPane";
import CoopPane from "./panes/CoopPane";
import SystemPane from "./panes/SystemPane";

type Pane = "play" | "mods" | "saves" | "coop" | "system";

export default function Stage({ game }: { game: GameInfo }) {
  const { profiles, settings, patchSettings, gameRunning, refreshProfiles, refreshInstalled } =
    useApp();
  const toast = useToast();

  const [pane, setPane] = useState<Pane>("play");
  const [install, setInstall] = useState<Installation | null>(null);
  const [loading, setLoading] = useState(true);
  const [loaders, setLoaders] = useState<LoaderInstall[]>([]);
  const [eac, setEac] = useState<EacStatus | null>(null);
  const [prepared, setPrepared] = useState<PreparedLaunch | null>(null);
  const [modCount, setModCount] = useState(0);
  const [busy, setBusy] = useState(false);

  const gameProfiles = profiles.filter((p) => p.game === game.id);
  const active =
    gameProfiles.find((p) => p.id === settings.activeProfile) ?? gameProfiles[0] ?? null;
  const running = gameRunning === game.id;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const found = await api.installsActive(game.id);
      setInstall(found);
      const [l, e, m] = await Promise.all([
        api.loadersDiscover(game.id).catch(() => []),
        api.eacStatus(game.id).catch(() => null),
        api.modsList(game.id).catch(() => []),
      ]);
      setLoaders(l);
      setEac(e);
      setModCount(m.length);
    } catch {
      setInstall(null);
    } finally {
      setLoading(false);
    }
  }, [game.id]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!install || !active) {
      setPrepared(null);
      return;
    }
    api.launchPlan(game.id, active.id).then(setPrepared).catch(() => setPrepared(null));
  }, [game.id, install, active]);

  const locate = async () => {
    const picked = await pickFolder({ directory: true, title: `Where is ${game.name}?` });
    if (typeof picked !== "string") return;
    const added = await toast.run(`${game.name} is set up`, () =>
      api.installsRemember(game.id, picked, true),
    );
    if (added) {
      await refreshInstalled();
      await load();
    }
  };

  const detect = async () => {
    setBusy(true);
    try {
      const results = await api.installsDiscover(game.id);
      if (results.length === 0) {
        toast.info("Not found automatically", "Use Locate to point at the folder.");
        return;
      }
      await api.installsRemember(game.id, results[0].root, true);
      await refreshInstalled();
      await load();
      toast.success(`${game.name} found`, results[0].root);
    } finally {
      setBusy(false);
    }
  };

  const play = async () => {
    if (!active) return;
    setBusy(true);
    try {
      const result = await api.launchRun(game.id, active.id);
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
      api.profileCreate(game.id, gameProfiles.length === 0 ? "Default" : "New profile"),
    );
    if (made) {
      await refreshProfiles();
      void patchSettings({ activeProfile: made.id });
      setPane("mods");
    }
  };

  const blocked = prepared?.plan.notices.some((n) => n.severity === "blocker") ?? false;

  const panes: { id: Pane; label: string; glyph: (p: { size?: number }) => React.ReactNode; n?: number }[] =
    [
      { id: "play", label: "Play", glyph: Icon.Play },
      { id: "mods", label: "Mods", glyph: Icon.Layers, n: modCount || undefined },
      { id: "saves", label: "Saves", glyph: Icon.Save },
      ...(game.supportsSeamlessCoop
        ? [{ id: "coop" as Pane, label: "Co-op", glyph: Icon.Users }]
        : []),
      { id: "system", label: "System", glyph: Icon.Tools },
    ];

  return (
    <div className="scene">
      <Presentation
        game={game}
        running={running}
        install={install}
        eac={eac}
        loaders={loaders}
        profileName={active?.name}
        profiles={gameProfiles}
        activeId={active?.id}
        onPickProfile={(id) => void patchSettings({ activeProfile: id })}
        canPlay={Boolean(install && active && !blocked && !running && !busy)}
        busy={busy}
        onPlay={play}
        onLocate={locate}
        onDetect={detect}
        onCreateProfile={createProfile}
      />

      {install && (
        <div className="tabbar">
          {panes.map((entry) => {
            const Glyph = entry.glyph;
            return (
              <button
                key={entry.id}
                type="button"
                role="tab"
                className="tb"
                aria-selected={pane === entry.id}
                onClick={() => setPane(entry.id)}
              >
                <Glyph size={14} />
                {entry.label}
                {entry.n !== undefined && <span className="tb__n">{entry.n}</span>}
              </button>
            );
          })}
        </div>
      )}

      <div className="pane">
        {loading ? (
          <div className="g2">
            <Card><Skeleton variant="line" count={4} /></Card>
            <Card><Skeleton variant="line" count={4} /></Card>
          </div>
        ) : !install ? (
          <Card>
            <Blank
              icon={Icon.Folder}
              title={`${game.name} is not set up`}
              action={
                <div className="row" style={{ gap: "var(--s2)" }}>
                  <button type="button" className="btn btn--a" onClick={detect} disabled={busy}>
                    {busy ? <span className="spin" /> : <Icon.Search size={15} />}
                    Detect automatically
                  </button>
                  <button type="button" className="btn" onClick={locate}>
                    <Icon.Folder size={15} />
                    Choose folder
                  </button>
                </div>
              }
            >
              Roundtable looks through Steam libraries and the folders standalone copies
              usually land in. If that misses, point it at the folder containing{" "}
              <span className="mono">Game\{game.executable}</span>.
            </Blank>
          </Card>
        ) : (
          <>
            {pane === "play" && (
              <PlayPane
                game={game}
                install={install}
                profile={active}
                prepared={prepared}
                onCreateProfile={createProfile}
                onManageMods={() => setPane("mods")}
                onPatch={async () => {
                  if (!active) return;
                  await toast.run("Configuration written", () =>
                    api.launchPatch(game.id, active.id),
                  );
                }}
              />
            )}
            {pane === "mods" && (
              <ModsPane
                gameId={game.id}
                profile={active}
                profiles={gameProfiles}
                onChanged={async () => {
                  await refreshProfiles();
                  const list = await api.modsList(game.id);
                  setModCount(list.length);
                }}
              />
            )}
            {pane === "saves" && <SavesPane gameId={game.id} />}
            {pane === "coop" && <CoopPane gameId={game.id} install={install} />}
            {pane === "system" && (
              <SystemPane
                gameId={game.id}
                install={install}
                eac={eac}
                onEacChanged={setEac}
                onForget={async () => {
                  await api.installsForget(game.id, install.root);
                  await refreshInstalled();
                  await load();
                }}
              />
            )}
          </>
        )}
      </div>
    </div>
  );
}

/**
 * The trailer plays behind the title, muted and looping, with the still image
 * underneath so nothing is ever blank. Sound is off by default and remembered
 * only for the session; a launcher that makes noise unprompted is a bad neighbour.
 */
function Presentation({
  game,
  running,
  install,
  eac,
  loaders,
  profileName,
  profiles,
  activeId,
  onPickProfile,
  canPlay,
  busy,
  onPlay,
  onLocate,
  onDetect,
  onCreateProfile,
}: {
  game: GameInfo;
  running: boolean;
  install: Installation | null;
  eac: EacStatus | null;
  loaders: LoaderInstall[];
  profileName?: string;
  profiles: import("../lib/types").Profile[];
  activeId?: string;
  onPickProfile: (id: string) => void;
  canPlay: boolean;
  busy: boolean;
  onPlay: () => void;
  onLocate: () => void;
  onDetect: () => void;
  onCreateProfile: () => void;
}) {
  const video = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(true);
  const [logoOk, setLogoOk] = useState(true);

  useEffect(() => {
    setPlaying(false);
    setLogoOk(true);
  }, [game.id]);

  useEffect(() => {
    const element = video.current;
    if (!element) return;
    element.muted = muted;
  }, [muted]);

  return (
    <section className="pres">
      <div className="pres__media">
        <img
          className="pres__still"
          src={game.heroUrl}
          alt=""
          style={{ opacity: playing ? 0 : 1 }}
        />
        <video
          ref={video}
          className="pres__vid"
          data-on={playing}
          src={game.trailerUrl}
          poster={game.heroUrl}
          muted
          loop
          playsInline
          preload="auto"
          onCanPlay={(event) => {
            const element = event.currentTarget;
            element.play().then(() => setPlaying(true)).catch(() => setPlaying(false));
          }}
          onError={() => setPlaying(false)}
        />
      </div>
      <div className="pres__fade" />
      <div className="pres__tint" />

      {playing && (
        <button
          type="button"
          className="btn btn--g btn--i pres__mute"
          aria-label={muted ? "Unmute trailer" : "Mute trailer"}
          onClick={() => setMuted((m) => !m)}
          style={{ background: "rgba(0,0,0,0.55)", backdropFilter: "blur(8px)" }}
        >
          {muted ? <Icon.Muted size={16} /> : <Icon.Sound size={16} />}
        </button>
      )}

      <div className="pres__body">
        <div className="grow">
          {logoOk ? (
            <img
              className="pres__logo"
              src={game.logoUrl}
              alt={game.name}
              onError={() => setLogoOk(false)}
            />
          ) : (
            <h1 className="pres__name">{game.name}</h1>
          )}

          <div className="row wrap pres__meta" style={{ gap: "var(--s2)", marginTop: "var(--s4)" }}>
            {running ? (
              <Chip tone="ok">
                <span className="dot beat" />
                Running
              </Chip>
            ) : install ? (
              <Chip tone="a">
                {install.kind === "steam"
                  ? "Steam"
                  : install.kind === "standalone"
                    ? "Standalone"
                    : "Installed"}
              </Chip>
            ) : (
              <Chip tone="warn">Not set up</Chip>
            )}
            {install?.version && <Chip>{install.version}</Chip>}
            {install?.hasSeamlessCoop && <Chip>Seamless Co-op</Chip>}
            {loaders.map((l) => (
              <Chip key={l.executable}>{l.kind === "me3" ? "me3" : "ModEngine 2"}</Chip>
            ))}
            {eac?.state === "bypassed" && <Chip tone="bad">Anti-cheat off</Chip>}
          </div>

          <div className="row wrap pres__act" style={{ gap: "var(--s3)", marginTop: "var(--s5)" }}>
            {!install ? (
              <>
                <button type="button" className="play" onClick={onDetect} disabled={busy}>
                  {busy ? <span className="spin" /> : <Icon.Search size={18} />}
                  Detect
                </button>
                <button type="button" className="btn" style={{ height: 54, padding: "0 var(--s5)" }} onClick={onLocate}>
                  <Icon.Folder size={16} />
                  Locate
                </button>
              </>
            ) : !profileName ? (
              <button type="button" className="play" onClick={onCreateProfile}>
                <Icon.Plus size={18} />
                Create a profile
              </button>
            ) : (
              <>
                <button type="button" className="play" onClick={onPlay} disabled={!canPlay}>
                  {busy ? <span className="spin" /> : <Icon.Play size={18} />}
                  {running ? "Running" : busy ? "Starting" : "Play"}
                </button>
                {profiles.length > 1 && (
                  <select
                    className="sel2"
                    style={{ width: 210, height: 54, borderRadius: "var(--r)" }}
                    value={activeId}
                    aria-label="Profile"
                    onChange={(event) => onPickProfile(event.target.value)}
                  >
                    {profiles.map((profile) => (
                      <option key={profile.id} value={profile.id}>
                        {profile.name}
                      </option>
                    ))}
                  </select>
                )}
                {profiles.length === 1 && (
                  <span className="chip chip--dark" style={{ height: 30 }}>
                    <Icon.Layers size={12} />
                    {profileName}
                  </span>
                )}
              </>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
