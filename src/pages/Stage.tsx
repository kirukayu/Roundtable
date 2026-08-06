import { motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";

import { EditionSwitch } from "../components/EditionSwitch";
import { Icon } from "../components/Icons";
import { HealthCard } from "../components/HealthCard";
import { LanguageCard } from "../components/LanguageCard";
import { MatchCard } from "../components/MatchCard";
import { EASE } from "../components/Motion";
import { Blank, Card, Chip, Skeleton, useToast } from "../components/ui";
import { api } from "../lib/ipc";
import { useApp } from "../lib/store";
import type {
  EacStatus,
  EditionStatus,
  GameInfo,
  Installation,
  LoaderInstall,
  PreparedLaunch,
} from "../lib/types";

import CodexPane from "./panes/CodexPane";
import WikiPane from "./panes/WikiPane";
import EditionPane from "./panes/EditionPane";
import PlayPane from "./panes/PlayPane";
import ModsPane from "./panes/ModsPane";
import SavesPane from "./panes/SavesPane";
import CoopPane from "./panes/CoopPane";
import SystemPane from "./panes/SystemPane";

type Pane = "play" | "mods" | "saves" | "coop" | "codex" | "wiki" | "system";

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
  const [scanning, setScanning] = useState(false);
  const [scanAt, setScanAt] = useState("");

  // Which edition of this game is on screen. Null is the game itself.
  const [edition, setEdition] = useState<string | null>(null);
  const [editions, setEditions] = useState<EditionStatus[]>([]);
  const [editionCoop, setEditionCoop] = useState(true);

  const gameProfiles = profiles.filter((p) => p.game === game.id);
  const active =
    gameProfiles.find((p) => p.id === settings.activeProfile) ?? gameProfiles[0] ?? null;
  const running = gameRunning === game.id;

  const openEdition = editions.find((e) => e.spec.id === edition) ?? null;

  const loadEditions = useCallback(async () => {
    const list = await api.editions(game.id, editionCoop).catch(() => []);
    setEditions(list);
  }, [game.id, editionCoop]);

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
    void loadEditions();
  }, [loadEditions]);

  // Leaving for another game drops back to the base edition, so the page never
  // opens on a conversion that belongs to a title you are no longer looking at.
  useEffect(() => {
    setEdition(null);
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

  const accept = useCallback(
    async (root: string) => {
      await api.installsRemember(game.id, root, true);
      await refreshInstalled();
      await load();
      toast.success(`${game.name} found`, root);
    },
    [game.id, game.name, refreshInstalled, load, toast],
  );

  /**
   * Find the game.
   *
   * The quick pass covers the Steam registry and the folders installers use,
   * and takes a second or two. When it comes up empty the search widens to
   * every drive rather than telling the user to go and find it themselves —
   * repacks live in folders named anything at all.
   */
  const detect = async () => {
    setBusy(true);
    try {
      const results = await api.installsDiscover(game.id);
      if (results.length > 0) {
        await accept(results[0].root);
        return;
      }
      await api.installsScan(game.id);
      setScanning(true);
    } catch (error) {
      toast.error("Search failed", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  // While the whole machine is being searched, show where it has got to.
  useEffect(() => {
    if (!scanning) return;
    const timer = window.setInterval(async () => {
      try {
        const state = await api.installsScanState();
        setScanAt(state.at);
        if (!state.running) {
          window.clearInterval(timer);
          setScanning(false);
          setScanAt("");
          if (state.found.length > 0) await accept(state.found[0].root);
          else if (!state.cancelled) {
            toast.info("Nothing found", "Point the launcher at the folder instead.");
          }
        }
      } catch {
        window.clearInterval(timer);
        setScanning(false);
      }
    }, 500);
    return () => window.clearInterval(timer);
  }, [scanning, accept, toast]);

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

  // A conversion brings its own mods, so that tab has nothing to say about it.
  const panes: { id: Pane; label: string }[] = [
    { id: "play", label: "Play" },
    ...(openEdition ? [] : [{ id: "mods" as Pane, label: "Mods" }]),
    { id: "saves", label: "Saves" },
    ...(game.supportsSeamlessCoop ? [{ id: "coop" as Pane, label: "Co-op" }] : []),
    ...(game.id === "elden-ring"
      ? [{ id: "codex" as Pane, label: "Codex" }, { id: "wiki" as Pane, label: "Wiki" }]
      : []),
    { id: "system", label: "System" },
  ];

  // The banner follows the edition: picking The Convergence should look like
  // opening it, not like reading about it on the ELDEN RING page.
  const bannerArt = openEdition
    ? `/editions/${openEdition.spec.id}-hero.jpg`
    : game.heroUrl;

  return (
    <div className="detail">
      <section className="detail__banner">
        {bannerArt && (
          // Shares its identity with the cover on the shelf, so arriving here
          // is the poster expanding rather than a new page appearing.
          <motion.div
            layoutId={openEdition ? undefined : `art-${game.id}`}
            key={bannerArt}
            className="detail__art"
            style={{ backgroundImage: `url(${bannerArt})` }}
            initial={openEdition ? { opacity: 0 } : false}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.9, ease: EASE }}
          />
        )}
        <div className="detail__veil" />

        <button type="button" className="btn btn--ghost btn--sm detail__back" onClick={onBack}>
          <Icon.Back size={14} />
          Catalogue
        </button>

        {/* In the left margin, clear of the title. */}
        {install && editions.length > 0 && (
          <EditionSwitch
            game={game}
            editions={editions}
            active={edition}
            onSelect={(id) => {
              setEdition(id);
              setPane("play");
            }}
          />
        )}

        <motion.div
          className="detail__body"
          initial={{ opacity: 0, y: 26, filter: "blur(8px)" }}
          animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
          transition={{ duration: 1.1, ease: EASE, delay: 0.34 }}
        >
          <h1 className="detail__title">{openEdition?.spec.name ?? game.name}</h1>

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
                  disabled={busy || scanning}
                >
                  {busy || scanning ? <span className="spin" /> : null}
                  {scanning ? "Searching every drive" : "Find it"}
                </button>
                {scanning ? (
                  <button
                    type="button"
                    className="btn btn--lg"
                    onClick={() => void api.installsScanStop()}
                  >
                    Stop
                  </button>
                ) : (
                  <button type="button" className="btn btn--lg" onClick={locate}>
                    Choose the folder
                  </button>
                )}
              </>
            ) : openEdition ? (
              <button
                type="button"
                className="btn btn--solid btn--lg"
                onClick={() => setPane("play")}
              >
                {openEdition.install ? `Open ${openEdition.spec.short}` : "Install it"}
              </button>
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

                {/*
                  Always visible, even with a single profile. Hiding the picker
                  until a second one exists means creating the first appears to
                  do nothing at all.
                */}
                <select
                  className="sel2"
                  style={{ width: 210, height: 56, borderRadius: "var(--r-full)" }}
                  value={active.id}
                  aria-label="Profile"
                  onChange={(event) => {
                    if (event.target.value === "+") void createProfile();
                    else void patchSettings({ activeProfile: event.target.value });
                  }}
                >
                  {gameProfiles.map((profile) => (
                    <option key={profile.id} value={profile.id}>
                      {profile.name}
                    </option>
                  ))}
                  <option value="+">New profile…</option>
                </select>
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
          <div className="tabs__list">
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
                    disabled={busy || scanning}
                  >
                    {busy || scanning ? <span className="spin" /> : null}
                    {scanning ? "Searching" : "Find it"}
                  </button>
                  <button type="button" className="btn" onClick={locate} disabled={scanning}>
                    Choose the folder
                  </button>
                </div>
              }
            >
              {scanning ? (
                <>
                  Searching every drive for {game.name}. Any folder near it will
                  do, whatever it is called.
                  <br />
                  <span className="mono w4 truncate" style={{ display: "block", marginTop: "var(--s3)", fontSize: "var(--t-2xs)" }}>
                    {scanAt || "…"}
                  </span>
                </>
              ) : (
                <>
                  Press Find it and Roundtable checks Steam, the usual folders,
                  then every drive. Or point it at the game yourself — any folder
                  at or near it works.
                </>
              )}
            </Blank>
          </Card>
        ) : (
          <>
            {pane === "play" && openEdition && (
              <EditionPane
                game={game.id}
                install={install}
                status={openEdition}
                coop={editionCoop}
                onCoop={setEditionCoop}
                onChanged={loadEditions}
              />
            )}

            {pane === "play" && !openEdition && (
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
            {pane === "coop" && (
              <div className="col">
                <HealthCard game={game.id} edition={edition} />
                <MatchCard game={game.id} edition={edition} />
                <CoopPane gameId={game.id} install={install} />
              </div>
            )}
            {pane === "codex" && <CodexPane edition={edition} />}
            {pane === "wiki" && <WikiPane edition={edition} />}
            {pane === "system" && (
              <div className="col">
                <LanguageCard game={game.id} />
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
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
