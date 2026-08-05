import { useEffect, useMemo, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { CommandPalette, type Command } from "./components/CommandPalette";
import { Icon } from "./components/Icons";
import { useToast } from "./components/ui";
import { api } from "./lib/ipc";
import { useApp } from "./lib/store";
import type { GameId } from "./lib/types";

import Library from "./pages/Library";
import GamePage from "./pages/GamePage";
import Downloads from "./pages/Downloads";
import SettingsOverlay from "./pages/SettingsOverlay";

/**
 * Two destinations, not seven.
 *
 * Everything a game needs — profiles, mods, co-op, saves — lives on that game's
 * own page, because that is where you are when you want it. Settings is an overlay
 * rather than a place, so it can never compete with the library for attention.
 */
export type View = { kind: "library" } | { kind: "game"; id: GameId } | { kind: "downloads" };

export default function App() {
  const [view, setView] = useState<View>({ kind: "library" });
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { games, settings, patchSettings, profiles, activeProfile, gameRunning, installed } =
    useApp();
  const toast = useToast();

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      } else if ((event.ctrlKey || event.metaKey) && event.key === ",") {
        event.preventDefault();
        setSettingsOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const open = (id: GameId) => setView({ kind: "game", id });

  const commands = useMemo<Command[]>(() => {
    const list: Command[] = [];

    for (const game of games) {
      list.push({
        id: `open-${game.id}`,
        group: "Games",
        label: game.name,
        glyph: Icon.Play,
        keywords: installed.has(game.id) ? "installed" : "",
        run: () => open(game.id),
      });
    }

    list.push(
      {
        id: "view-library",
        group: "Go to",
        label: "Library",
        glyph: Icon.Library,
        run: () => setView({ kind: "library" }),
      },
      {
        id: "view-downloads",
        group: "Go to",
        label: "Downloads",
        glyph: Icon.Download,
        run: () => setView({ kind: "downloads" }),
      },
      {
        id: "view-settings",
        group: "Go to",
        label: "Settings",
        glyph: Icon.Settings,
        hint: "Ctrl ,",
        run: () => setSettingsOpen(true),
      },
    );

    for (const profile of profiles) {
      list.push({
        id: `profile-${profile.id}`,
        group: "Profiles",
        label: profile.name,
        glyph: Icon.Layers,
        keywords: profile.seamlessCoop ? "co-op coop" : "",
        run: () => {
          void patchSettings({ activeProfile: profile.id });
          open(profile.game);
        },
      });
    }

    if (activeProfile) {
      list.push({
        id: "act-play",
        group: "Actions",
        label: `Play ${activeProfile.name}`,
        glyph: Icon.Play,
        keywords: "launch start run",
        run: async () => {
          await toast.run(`${activeProfile.name} started`, () =>
            api.launchRun(activeProfile.game, activeProfile.id),
          );
        },
      });
    }

    list.push({
      id: "act-clear-cache",
      group: "Actions",
      label: "Clear shader caches",
      glyph: Icon.Broom,
      keywords: "stutter fps performance",
      run: async () => {
        const caches = await api.sysShaderCaches();
        const targets = caches.filter((c) => c.exists && c.sizeBytes > 0).map((c) => c.path);
        if (targets.length === 0) {
          toast.info("Nothing to clear", "The shader caches are already empty.");
          return;
        }
        await toast.run(
          "Shader caches cleared",
          () => api.sysClearCaches(targets),
          (report) => `${(report.bytesFreed / 1024 / 1024).toFixed(0)} MB reclaimed`,
        );
      },
    });

    return list;
  }, [games, profiles, activeProfile, installed, patchSettings, toast]);

  return (
    <div className="app">
      <Titlebar />

      <div className="body">
        <aside className="side">
          <button type="button" className="side__search" onClick={() => setPaletteOpen(true)}>
            <Icon.Search size={15} />
            Search
            <span className="kbd">Ctrl K</span>
          </button>

          <div className="side__scroll">
            <div className="side__label">Games</div>
            {games.map((game) => {
              const here = view.kind === "game" && view.id === game.id;
              const isInstalled = installed.has(game.id);
              const running = gameRunning === game.id;
              return (
                <button
                  key={game.id}
                  type="button"
                  className="grow-row"
                  aria-current={here}
                  onClick={() => open(game.id)}
                >
                  <img className="mini-cover" src={game.coverUrl} alt="" loading="lazy" />
                  <span className="grow">
                    <span className="truncate" style={{ display: "block" }}>
                      {game.short}
                    </span>
                    <span className="grow-row__meta">
                      {running ? "Running" : isInstalled ? "Installed" : "Not found"}
                    </span>
                  </span>
                  {running && (
                    <span className="dot pulse" style={{ color: "var(--success)" }} />
                  )}
                </button>
              );
            })}

            <div className="side__label">Browse</div>
            <button
              type="button"
              className="nav-link"
              aria-current={view.kind === "library" ? "page" : undefined}
              onClick={() => setView({ kind: "library" })}
            >
              <Icon.Library size={17} />
              Library
            </button>
            <button
              type="button"
              className="nav-link"
              aria-current={view.kind === "downloads" ? "page" : undefined}
              onClick={() => setView({ kind: "downloads" })}
            >
              <Icon.Download size={17} />
              Downloads
            </button>
          </div>

          <button type="button" className="nav-link" onClick={() => setSettingsOpen(true)}>
            <Icon.Settings size={17} />
            Settings
            <span className="kbd">Ctrl ,</span>
          </button>
        </aside>

        <main className="main">
          {view.kind === "library" && <Library onOpen={open} />}
          {view.kind === "game" && (
            <GamePage
              key={view.id}
              gameId={view.id}
              onBack={() => setView({ kind: "library" })}
            />
          )}
          {view.kind === "downloads" && <Downloads />}
        </main>
      </div>

      {paletteOpen && (
        <CommandPalette commands={commands} onClose={() => setPaletteOpen(false)} />
      )}
      {settingsOpen && (
        <SettingsOverlay
          settings={settings}
          onPatch={patchSettings}
          onClose={() => setSettingsOpen(false)}
        />
      )}
    </div>
  );
}

function Titlebar() {
  const win = getCurrentWindow();
  const { gameRunning, games } = useApp();
  const running = games.find((g) => g.id === gameRunning);

  return (
    <header className="titlebar" data-tauri-drag-region>
      <span className="brand">
        <span className="brand__dot" />
        Roundtable
      </span>
      {running && (
        <span className="chip chip--success">
          <span className="dot pulse" />
          {running.short}
        </span>
      )}
      <div className="win-btns">
        <button
          type="button"
          className="win-btn"
          aria-label="Minimise"
          onClick={() => void win.minimize()}
        >
          <Icon.Minus size={15} />
        </button>
        <button
          type="button"
          className="win-btn"
          aria-label="Maximise"
          onClick={() => void win.toggleMaximize()}
        >
          <Icon.Square size={13} />
        </button>
        <button
          type="button"
          className="win-btn win-btn--close"
          aria-label="Close"
          onClick={() => void win.close()}
        >
          <Icon.Close size={15} />
        </button>
      </div>
    </header>
  );
}
