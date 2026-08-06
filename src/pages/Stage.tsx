import { motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";

import { Icon } from "../components/Icons";
import { EASE } from "../components/Motion";
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

/**
 * One game, in full.
 *
 * The banner is the game's own key art, drained of colour so the type stays the
 * loudest thing on screen. Everything the launcher can do to that game lives in
 * the tabs beneath it.
 */
export default function Stage({ game, onBack }: { game: GameInfo; onBack: () => void }) {
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
    const picked = await api.pickFolder(`Where is ${game.name}?`);
    if (!picked) return;
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
        toast.info("Not found automatically", "Point the launcher at the folder instead.");
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

  const panes: { id: Pane; label: string }[] = [
    { id: "play", label: "Play" },
    { id: "mods", label: "Mods" },
    { id: "saves", label: "Saves" },
    ...(game.supportsSeamlessCoop ? [{ id: "coop" as Pane, label: "Co-op" }] : []),
    { id: "system", label: "System" },
  ];

  return (
    <div className="detail">
      <section className="detail__banner">
        {game.heroUrl && (
          // Shares its identity with the cover on the shelf, so arriving here
          // is the poster expanding rather than a new page appearing.
          <motion.div
            layoutId={`art-${game.id}`}
            className="detail__art"
            style={{ backgroundImage: `url(${game.heroUrl})` }}
          />
        )}
        <div className="detail__veil" />

        <button type="button" className="btn btn--ghost btn--sm detail__back" onClick={onBack}>
          <Icon.Back size={14} />
          Catalogue
        </button>

        <motion.div
          className="detail__body"
          initial={{ opacity: 0, y: 26, filter: "blur(8px)" }}
          animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
          transition={{ duration: 1.1, ease: EASE, delay: 0.34 }}
        >
          <h1 className="detail__title">{game.name}</h1>

          <div className="detail__facts">
            <span className="fact">
              Released
              <strong>{game.year}</strong>
            </span>
            <span className="fact">
              Status
              <strong>{running ? "Running" : install ? "Installed" : "Not located"}</strong>
            </span>
            {install?.version && (
              <span className="fact">
                Build
                <strong>{install.version}</strong>
              </span>
            )}
            {install && (
              <span className="fact">
                Source
                <strong>
                  {install.kind === "steam"
                    ? "Steam"
                    : install.kind === "standalone"
                      ? "Standalone"
                      : "Unrecognised"}
                </strong>
              </span>
            )}
            {active && (
              <span className="fact">
                Profile
                <strong>{active.name}</strong>
              </span>
            )}
          </div>

          <div className="detail__act row wrap" style={{ gap: "var(--s3)" }}>
            {!install ? (
              <>
                <button
                  type="button"
                  className="btn btn--solid btn--lg"
                  onClick={detect}
                  disabled={busy}
                >
                  {busy ? <span className="spin" /> : null}
                  Detect
                </button>
                <button type="button" className="btn btn--lg" onClick={locate}>
                  Locate manually
                </button>
              </>
            ) : !active ? (
              <button type="button" className="btn btn--solid btn--lg" onClick={createProfile}>
                Create a profile
              </button>
            ) : (
              <>
                <button
                  type="button"
                  className="btn btn--solid btn--lg"
                  onClick={play}
                  disabled={busy || running || blocked}
                >
                  {busy ? <span className="spin" /> : null}
                  {running ? "Running" : busy ? "Starting" : "Play"}
                </button>
                {gameProfiles.length > 1 && (
                  <select
                    className="sel2"
                    style={{ width: 220, height: 56, borderRadius: "var(--r-full)" }}
                    value={active.id}
                    aria-label="Profile"
                    onChange={(event) =>
                      void patchSettings({ activeProfile: event.target.value })
                    }
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

            {eac?.state === "bypassed" && <Chip tone="bad">Anti-cheat off</Chip>}
            {install?.hasSeamlessCoop && <Chip>Seamless Co-op</Chip>}
            {loaders.map((l) => (
              <Chip key={l.executable}>{l.kind === "me3" ? "me3" : "ModEngine 2"}</Chip>
            ))}
          </div>
        </motion.div>
      </section>

      {install && (
        <div className="tabs">
          {panes.map((entry) => (
            <button
              key={entry.id}
              type="button"
              role="tab"
              className="tab"
              aria-selected={pane === entry.id}
              onClick={() => setPane(entry.id)}
            >
              {entry.label}
              {entry.id === "mods" && modCount > 0 ? ` (${modCount})` : ""}
            </button>
          ))}
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
              title={`${game.name} has not been located`}
              action={
                <div className="row" style={{ gap: "var(--s3)" }}>
                  <button
                    type="button"
                    className="btn btn--solid"
                    onClick={detect}
                    disabled={busy}
                  >
                    Detect automatically
                  </button>
                  <button type="button" className="btn" onClick={locate}>
                    Choose the folder
                  </button>
                </div>
              }
            >
              Roundtable reads the Steam registry and the folders standalone copies usually
              land in. If that misses, point it at the folder holding{" "}
              <span className="mono">{game.executable}</span>.
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
