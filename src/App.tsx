import { useEffect, useMemo, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { CommandPalette, type Command } from "./components/CommandPalette";
import { Icon } from "./components/Icons";
import { useToast } from "./components/ui";
import { api } from "./lib/ipc";
import { useApp } from "./lib/store";
import type { GameId } from "./lib/types";

import Stage from "./pages/Stage";
import SettingsOverlay from "./pages/SettingsOverlay";

/**
 * There is no library page and no store front. The rail picks a game; the stage
 * *is* that game. Selecting a different title recolours the entire shell from its
 * key art, so switching feels like changing where you are rather than filtering
 * a list.
 */
export default function App() {
  const { games, settings, patchSettings, profiles, activeProfile, gameRunning, installed } =
    useApp();
  const toast = useToast();

  const [current, setCurrent] = useState<GameId>(settings.selectedGame);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const game = games.find((g) => g.id === current) ?? games[0];

  // The accent is the only colour in the app, and it belongs to the game.
  useEffect(() => {
    if (!game) return;
    const root = document.documentElement;
    root.style.setProperty("--accent", game.accent);
    root.style.setProperty("--accent-soft", hexA(game.accent, 0.16));
    root.style.setProperty("--accent-line", hexA(game.accent, 0.4));
    root.style.setProperty("--accent-glow", hexA(game.accent, 0.45));
    root.style.setProperty("--accent-ink", readableInk(game.accent));
  }, [game]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      } else if ((event.ctrlKey || event.metaKey) && event.key === ",") {
        event.preventDefault();
        setSettingsOpen(true);
      } else if (event.altKey) {
        const index = Number.parseInt(event.key, 10) - 1;
        if (Number.isInteger(index) && index >= 0 && index < games.length) {
          event.preventDefault();
          select(games[index].id);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [games]);

  const select = (id: GameId) => {
    setCurrent(id);
    void patchSettings({ selectedGame: id });
  };

  const commands = useMemo<Command[]>(() => {
    const list: Command[] = games.map((entry, index) => ({
      id: `go-${entry.id}`,
      group: "Games",
      label: entry.name,
      hint: `Alt ${index + 1}`,
      glyph: Icon.Play,
      keywords: installed.has(entry.id) ? "installed" : "not installed",
      run: () => select(entry.id),
    }));

    for (const profile of profiles) {
      list.push({
        id: `pf-${profile.id}`,
        group: "Profiles",
        label: profile.name,
        glyph: Icon.Layers,
        keywords: profile.seamlessCoop ? "co-op coop" : "",
        run: () => {
          void patchSettings({ activeProfile: profile.id });
          select(profile.game);
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

    list.push(
      {
        id: "act-cache",
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
            (r) => `${(r.bytesFreed / 1048576).toFixed(0)} MB reclaimed`,
          );
        },
      },
      {
        id: "act-settings",
        group: "Actions",
        label: "Settings",
        glyph: Icon.Settings,
        hint: "Ctrl ,",
        run: () => setSettingsOpen(true),
      },
    );

    return list;
  }, [games, profiles, activeProfile, installed, patchSettings, toast]);

  return (
    <div className="app">
      <Bar running={gameRunning ? games.find((g) => g.id === gameRunning)?.short : undefined} />

      <div className="body">
        <nav className="rail">
          <div className="rail__label">Games</div>
          <div className="rail__list">
            {games.map((entry) => {
              const here = entry.id === current;
              const running = gameRunning === entry.id;
              return (
                <button
                  key={entry.id}
                  type="button"
                  className="gm"
                  aria-current={here}
                  onClick={() => select(entry.id)}
                  style={
                    here
                      ? ({ "--accent": entry.accent } as React.CSSProperties)
                      : undefined
                  }
                >
                  <img className="gm__cv" src={entry.coverUrl} alt="" loading="lazy" />
                  <span className="grow">
                    <span className="truncate" style={{ display: "block" }}>
                      {entry.short}
                    </span>
                    <span className="gm__sub">
                      {running ? "Running" : installed.has(entry.id) ? "Ready" : "Not set up"}
                    </span>
                  </span>
                  {running && <span className="dot beat" style={{ color: "var(--ok)" }} />}
                </button>
              );
            })}
          </div>

          <button type="button" className="rail__btn" onClick={() => setPaletteOpen(true)}>
            <Icon.Search size={16} />
            Search
            <span className="kbd">Ctrl K</span>
          </button>
          <button type="button" className="rail__btn" onClick={() => setSettingsOpen(true)}>
            <Icon.Settings size={16} />
            Settings
          </button>
        </nav>

        <main className="stage">
          {game && <Stage key={game.id} game={game} />}
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

function Bar({ running }: { running?: string }) {
  const win = getCurrentWindow();
  return (
    <header className="bar" data-tauri-drag-region>
      <span className="mark">
        <span className="mark__d" />
        Roundtable
      </span>
      {running && (
        <span className="chip chip--ok">
          <span className="dot beat" />
          {running}
        </span>
      )}
      <div className="wbtns">
        <button type="button" className="wbtn" aria-label="Minimise" onClick={() => void win.minimize()}>
          <Icon.Minus size={15} />
        </button>
        <button type="button" className="wbtn" aria-label="Maximise" onClick={() => void win.toggleMaximize()}>
          <Icon.Square size={13} />
        </button>
        <button type="button" className="wbtn wbtn--x" aria-label="Close" onClick={() => void win.close()}>
          <Icon.Close size={15} />
        </button>
      </div>
    </header>
  );
}

/** `#rrggbb` plus an alpha, as an rgba string. */
function hexA(hex: string, alpha: number): string {
  const value = hex.replace("#", "");
  const r = Number.parseInt(value.slice(0, 2), 16);
  const g = Number.parseInt(value.slice(2, 4), 16);
  const b = Number.parseInt(value.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Black or white text, whichever stays legible on the accent. */
function readableInk(hex: string): string {
  const value = hex.replace("#", "");
  const r = Number.parseInt(value.slice(0, 2), 16) / 255;
  const g = Number.parseInt(value.slice(2, 4), 16) / 255;
  const b = Number.parseInt(value.slice(4, 6), 16) / 255;
  const channel = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  const luminance = 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  return luminance > 0.42 ? "#0a0803" : "#ffffff";
}
