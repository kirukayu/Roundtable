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
 * A rounded pane of glass over a slow aurora.
 *
 * Choosing a game rewrites the aurora, the accent and everything tinted by them,
 * over a full second, so the launcher dissolves from one world into the next
 * rather than swapping a highlight colour.
 */
export default function App() {
  const { games, settings, patchSettings, profiles, activeProfile, gameRunning, installed } =
    useApp();
  const toast = useToast();

  const [current, setCurrent] = useState<GameId>(settings.selectedGame);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const game = games.find((g) => g.id === current) ?? games[0];

  useEffect(() => {
    if (!game) return;
    const root = document.documentElement.style;
    root.setProperty("--a1", game.aurora[0]);
    root.setProperty("--a2", game.aurora[1]);
    root.setProperty("--accent", game.accent);
    root.setProperty("--accent-soft", alpha(game.accent, 0.16));
    root.setProperty("--accent-glow", alpha(game.accent, 0.42));
    root.setProperty("--accent-ink", ink(game.accent));
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
      keywords: installed.has(entry.id) ? "installed" : "not set up",
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
    <div className="win">
      <div className="aur" aria-hidden="true">
        <span className="aur__b aur__b--1" />
        <span className="aur__b aur__b--2" />
        <span className="aur__b aur__b--3" />
        <span className="aur__g" />
      </div>

      <Bar running={gameRunning ? games.find((g) => g.id === gameRunning)?.short : undefined} />

      <div className="body">
        <nav className="rail">
          <div className="rail__cap">Games</div>
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
                >
                  <img className="gm__cv" src={entry.coverUrl} alt="" loading="lazy" />
                  <span className="grow">
                    <span className="truncate" style={{ display: "block" }}>
                      {entry.short}
                    </span>
                    <span className="gm__s">
                      {running ? "Running" : installed.has(entry.id) ? "Ready" : "Not set up"}
                    </span>
                  </span>
                  {running && <span className="dot beat" style={{ color: "var(--ok)" }} />}
                </button>
              );
            })}
          </div>

          <button type="button" className="rbtn" onClick={() => setPaletteOpen(true)}>
            <Icon.Search size={16} />
            Search
            <span className="kbd">Ctrl K</span>
          </button>
          <button type="button" className="rbtn" onClick={() => setSettingsOpen(true)}>
            <Icon.Settings size={16} />
            Settings
          </button>
        </nav>

        <main className="stage">{game && <Stage key={game.id} game={game} />}</main>
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

function alpha(hex: string, a: number): string {
  const v = hex.replace("#", "");
  return `rgba(${parseInt(v.slice(0, 2), 16)}, ${parseInt(v.slice(2, 4), 16)}, ${parseInt(v.slice(4, 6), 16)}, ${a})`;
}

/** Black or white text, whichever keeps contrast on the accent. */
function ink(hex: string): string {
  const v = hex.replace("#", "");
  const lin = (c: number) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  const l =
    0.2126 * lin(parseInt(v.slice(0, 2), 16) / 255) +
    0.7152 * lin(parseInt(v.slice(2, 4), 16) / 255) +
    0.0722 * lin(parseInt(v.slice(4, 6), 16) / 255);
  return l > 0.4 ? "#0d0a04" : "#ffffff";
}
