import { useEffect, useMemo, useState } from "react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

import { Icon } from "../components/Icons";
import { Blank, Card, Chip, Skeleton, useToast } from "../components/ui";
import { api } from "../lib/ipc";
import { when } from "../lib/format";
import { useApp } from "../lib/store";
import type { GameId, GameInfo, Profile } from "../lib/types";

/**
 * The home screen answers one question: what do I want to play?
 *
 * A hero for whatever you touched last, then every title as a cover tile. No
 * documentation, no configuration, no empty panels.
 */
export default function Library({ onOpen }: { onOpen: (id: GameId) => void }) {
  const { games, installed, profiles, gameRunning, settings, patchSettings, refreshInstalled } =
    useApp();
  const toast = useToast();
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);

  const favourites = new Set(settings.favourites ?? []);

  // Most recently played first; that is almost always what you came back for.
  const recent = useMemo(
    () =>
      [...profiles]
        .filter((p) => p.lastPlayed)
        .sort((a, b) => (b.lastPlayed ?? "").localeCompare(a.lastPlayed ?? ""))
        .slice(0, 4),
    [profiles],
  );

  const featured = useMemo(() => {
    const lastGame = recent[0]?.game;
    return (
      games.find((g) => g.id === gameRunning) ??
      games.find((g) => g.id === lastGame) ??
      games.find((g) => installed.has(g.id)) ??
      games[0]
    );
  }, [games, gameRunning, recent, installed]);

  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const list = needle
      ? games.filter((g) => g.name.toLowerCase().includes(needle))
      : games;
    // Favourites float, then installed, then the rest.
    return [...list].sort((a, b) => {
      const favDelta = Number(favourites.has(b.id)) - Number(favourites.has(a.id));
      if (favDelta !== 0) return favDelta;
      return Number(installed.has(b.id)) - Number(installed.has(a.id));
    });
  }, [games, query, installed, settings.favourites]);

  const toggleFavourite = (id: GameId) => {
    const next = new Set(settings.favourites ?? []);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    void patchSettings({ favourites: [...next] });
  };

  const addInstall = async (game: GameInfo) => {
    const picked = await pickFolder({ directory: true, title: `Where is ${game.name}?` });
    if (typeof picked !== "string") return;
    setBusy(true);
    const added = await toast.run(`${game.name} added`, () =>
      api.installsRemember(game.id, picked, true),
    );
    if (added) {
      await refreshInstalled();
      onOpen(game.id);
    }
    setBusy(false);
  };

  const detectAll = async () => {
    setBusy(true);
    let found = 0;
    for (const game of games) {
      if (installed.has(game.id)) continue;
      try {
        const results = await api.installsDiscover(game.id);
        if (results.length > 0) {
          await api.installsRemember(game.id, results[0].root, true);
          found += 1;
        }
      } catch {
        /* a title that is not installed simply yields nothing */
      }
    }
    await refreshInstalled();
    setBusy(false);
    toast[found > 0 ? "success" : "info"](
      found > 0 ? `Found ${found} game${found === 1 ? "" : "s"}` : "Nothing new found",
      found > 0 ? undefined : "Use Locate on a tile to point Roundtable at a folder.",
    );
  };

  const installedCount = games.filter((g) => installed.has(g.id)).length;

  return (
    <div className="view">
      {featured && (
        <Hero
          game={featured}
          installed={installed.has(featured.id)}
          running={gameRunning === featured.id}
          profile={recent.find((p) => p.game === featured.id) ?? null}
          onOpen={() => onOpen(featured.id)}
          onLocate={() => void addInstall(featured)}
        />
      )}

      <div className="pad" style={{ paddingTop: "var(--s6)" }}>
        {recent.length > 0 && (
          <section style={{ marginBottom: "var(--s7)" }}>
            <div className="section-head">
              <h2>
                Continue
                <span className="count">{recent.length}</span>
              </h2>
            </div>
            <div className="grid-2 reveal">
              {recent.map((profile) => {
                const game = games.find((g) => g.id === profile.game);
                if (!game) return null;
                return (
                  <button
                    key={profile.id}
                    type="button"
                    className="card card--action"
                    onClick={() => {
                      void patchSettings({ activeProfile: profile.id });
                      onOpen(profile.game);
                    }}
                    style={{ padding: "var(--s3)" }}
                  >
                    <div className="row">
                      <img
                        src={game.coverUrl}
                        alt=""
                        loading="lazy"
                        style={{
                          width: 52,
                          aspectRatio: "3 / 4",
                          objectFit: "cover",
                          borderRadius: "var(--r-sm)",
                          flexShrink: 0,
                        }}
                      />
                      <div className="grow">
                        <div className="rw__title truncate">{profile.name}</div>
                        <div className="rw__sub">
                          {game.short} · {when(profile.lastPlayed)}
                        </div>
                        <div className="row" style={{ gap: 4, marginTop: 6 }}>
                          {profile.seamlessCoop && <Chip tone="success">Co-op</Chip>}
                          <Chip>
                            {profile.mods.filter((m) => m.enabled).length || "No"} mods
                          </Chip>
                        </div>
                      </div>
                      <span style={{ color: "var(--accent)" }}>
                        <Icon.Play size={20} />
                      </span>
                    </div>
                  </button>
                );
              })}
            </div>
          </section>
        )}

        <div className="section-head">
          <h2>
            Library
            <span className="count">
              {installedCount} of {games.length} installed
            </span>
          </h2>
          <div className="row" style={{ gap: "var(--s2)" }}>
            <input
              className="input"
              style={{ width: 200, height: 32 }}
              placeholder="Filter"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <button
              type="button"
              className="btn btn--sm"
              onClick={detectAll}
              disabled={busy}
            >
              {busy ? <span className="spin" /> : <Icon.Refresh size={14} />}
              Detect
            </button>
          </div>
        </div>

        {shown.length === 0 ? (
          <Card>
            <Blank icon={Icon.Search} title="No match">
              Nothing in the library is called “{query}”.
            </Blank>
          </Card>
        ) : (
          <div className="grid-covers reveal">
            {shown.map((game) => (
              <Tile
                key={game.id}
                game={game}
                installed={installed.has(game.id)}
                running={gameRunning === game.id}
                favourite={favourites.has(game.id)}
                profileCount={profiles.filter((p) => p.game === game.id).length}
                onOpen={() => onOpen(game.id)}
                onLocate={() => void addInstall(game)}
                onFavourite={() => toggleFavourite(game.id)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function Hero({
  game,
  installed,
  running,
  profile,
  onOpen,
  onLocate,
}: {
  game: GameInfo;
  installed: boolean;
  running: boolean;
  profile: Profile | null;
  onOpen: () => void;
  onLocate: () => void;
}) {
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    const image = new Image();
    image.src = game.heroUrl;
    image.onload = () => setLoaded(true);
    return () => {
      image.onload = null;
    };
  }, [game.heroUrl]);

  return (
    <section className="hero">
      <div
        className="hero__art"
        style={{
          backgroundImage: `url(${game.heroUrl})`,
          opacity: loaded ? 1 : 0,
          transition: "opacity 600ms var(--ease-out)",
        }}
      />
      <div className="hero__wash" />
      <div className="hero__body">
        <div className="hero__cover">
          <img src={game.coverUrl} alt="" />
        </div>
        <div className="grow">
          <div className="row" style={{ gap: "var(--s2)", marginBottom: "var(--s3)" }}>
            {running ? (
              <Chip tone="success">
                <span className="dot pulse" />
                Running
              </Chip>
            ) : installed ? (
              <Chip tone="accent">Installed</Chip>
            ) : (
              <Chip>Not installed</Chip>
            )}
            <Chip>{game.year}</Chip>
            {profile && <Chip tone="info">{profile.name}</Chip>}
          </div>

          <h1 className="hero__title">{game.name}</h1>

          <div className="row" style={{ gap: "var(--s3)", marginTop: "var(--s5)" }}>
            {installed ? (
              <button type="button" className="btn btn--play" onClick={onOpen}>
                <Icon.Play size={18} />
                {running ? "Open" : "Play"}
              </button>
            ) : (
              <button type="button" className="btn btn--play" onClick={onLocate}>
                <Icon.Folder size={18} />
                Locate
              </button>
            )}
            <button type="button" className="btn btn--lg" onClick={onOpen}>
              Manage
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}

function Tile({
  game,
  installed,
  running,
  favourite,
  profileCount,
  onOpen,
  onLocate,
  onFavourite,
}: {
  game: GameInfo;
  installed: boolean;
  running: boolean;
  favourite: boolean;
  profileCount: number;
  onOpen: () => void;
  onLocate: () => void;
  onFavourite: () => void;
}) {
  const [ready, setReady] = useState(false);

  return (
    <div className={`tile${installed ? "" : " tile--ghost"}`}>
      <button
        type="button"
        className="tile"
        onClick={installed ? onOpen : onLocate}
        aria-label={game.name}
      >
        <div className="tile__art">
          {!ready && <div className="sk" style={{ position: "absolute", inset: 0 }} />}
          <img
            src={game.coverUrl}
            alt=""
            loading="lazy"
            onLoad={() => setReady(true)}
            style={{ opacity: ready ? 1 : 0, transition: "opacity 400ms var(--ease-out)" }}
          />
          <span className="tile__ring" />

          <span className="tile__flag">
            {running && (
              <span className="chip chip--success">
                <span className="dot pulse" />
                Running
              </span>
            )}
            {!installed && <span className="chip chip--solid">Not found</span>}
          </span>

          <span className="tile__veil">
            <span className="row" style={{ gap: "var(--s2)" }}>
              <span className="btn btn--primary btn--sm grow" style={{ justifyContent: "center" }}>
                {installed ? <Icon.Play size={13} /> : <Icon.Folder size={13} />}
                {installed ? "Open" : "Locate"}
              </span>
            </span>
          </span>
        </div>
      </button>

      <div className="row-between" style={{ marginTop: "var(--s3)", gap: "var(--s2)" }}>
        <div className="grow" style={{ minWidth: 0 }}>
          <div className="tile__name truncate">{game.short}</div>
          <div className="tile__sub">
            {profileCount > 0
              ? `${profileCount} profile${profileCount === 1 ? "" : "s"}`
              : game.year}
          </div>
        </div>
        <button
          type="button"
          className="btn btn--ghost btn--sm btn--icon"
          aria-label={favourite ? "Remove from favourites" : "Add to favourites"}
          onClick={onFavourite}
          style={{ color: favourite ? "var(--accent)" : undefined }}
        >
          {favourite ? <Icon.StarFilled size={14} /> : <Icon.Star size={14} />}
        </button>
      </div>
    </div>
  );
}

/** Placeholder tiles while the catalogue loads. */
export function LibrarySkeleton() {
  return (
    <div className="grid-covers">
      {Array.from({ length: 5 }, (_, i) => (
        <div key={i}>
          <Skeleton variant="tile" />
          <Skeleton variant="line" width="70%" />
        </div>
      ))}
    </div>
  );
}
