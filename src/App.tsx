import { AnimatePresence, MotionConfig, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";

import { CommandPalette, type Command } from "./components/CommandPalette";
import { Fog } from "./components/Fog";
import { Icon } from "./components/Icons";
import { EASE } from "./components/Motion";
import { useToast } from "./components/ui";
import { api } from "./lib/ipc";
import { useProgress, useScrolled } from "./lib/motion";
import { jumpToTop, useSmoothScroll } from "./lib/smooth";
import { useApp } from "./lib/store";
import type { GameId } from "./lib/types";

import Landing from "./pages/Landing";
import Stage from "./pages/Stage";
import SettingsOverlay from "./pages/SettingsOverlay";

/**
 * Two screens: the catalogue and one game. Everything else is an overlay.
 *
 * The fog sits behind both and never restarts, so moving between them feels
 * like walking through the same room rather than loading a new page.
 */
export default function App() {
  const { games, settings, patchSettings, profiles, activeProfile, gameRunning, installed } =
    useApp();
  const toast = useToast();

  const [open, setOpen] = useState<GameId | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const game = open ? (games.find((g) => g.id === open) ?? null) : null;

  const rail = useProgress<HTMLDivElement>();
  // The bar is transparent over the hero and only earns its backdrop once the
  // page has actually moved.
  const scrolled = useScrolled(24);

  // Inertial scrolling for the whole document. Everything else on the page is
  // paced to arrive; the scroll itself has to match or none of it lands.
  useSmoothScroll(!settings.reduceMotion);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.reduceMotion = String(settings.reduceMotion);
  }, [settings.reduceMotion]);

  // Opening a game starts at its banner rather than wherever the catalogue was
  // scrolled to. No easing here: a long glide would fight the screen change.
  useEffect(() => {
    jumpToTop();
  }, [open]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((was) => !was);
      } else if ((event.ctrlKey || event.metaKey) && event.key === ",") {
        event.preventDefault();
        setSettingsOpen(true);
      } else if (event.key === "Escape" && open && !paletteOpen && !settingsOpen) {
        setOpen(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, paletteOpen, settingsOpen]);

  const commands = useMemo<Command[]>(() => {
    const list: Command[] = games
      .filter((entry) => entry.playable)
      .map((entry) => ({
        id: `go-${entry.id}`,
        group: "Games",
        label: entry.name,
        glyph: Icon.Play,
        keywords: installed.has(entry.id) ? "installed" : "locate",
        run: () => setOpen(entry.id),
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
          setOpen(profile.game);
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
        id: "act-catalogue",
        group: "Actions",
        label: "Back to the catalogue",
        glyph: Icon.Library,
        run: () => setOpen(null),
      },
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

  const running = gameRunning ? games.find((g) => g.id === gameRunning) : null;

  return (
    // One switch governs everything that moves. "never" is deliberate: the app
    // has its own setting for this, and it should not be overruled by an OS
    // preference the user did not set for this window.
    <MotionConfig reducedMotion={settings.reduceMotion ? "always" : "never"}>
      <Fog />

      <div className="rail" ref={rail} aria-hidden="true" />

      <nav className="nav" data-solid={scrolled}>
        <button type="button" className="nav__mark" onClick={() => setOpen(null)}>
          <span className="nav__glyph" />
          Roundtable
        </button>

        <div className="nav__links">
          {running && (
            <span className="chip chip--ok">
              <span className="dot beat" />
              {running.short}
            </span>
          )}
          <button
            type="button"
            className="nav__link"
            aria-current={open === null ? "page" : undefined}
            onClick={() => setOpen(null)}
          >
            Games
          </button>
          <button type="button" className="nav__link" onClick={() => setPaletteOpen(true)}>
            Search
          </button>
          <button type="button" className="nav__link" onClick={() => setSettingsOpen(true)}>
            Settings
          </button>
        </div>
      </nav>

      {/*
        The two screens overlap for the length of the change. That overlap is
        the point: the cover the catalogue was showing and the banner the game
        page opens with carry the same `layoutId`, so one becomes the other
        instead of the page being swapped underneath you.
      */}
      <div className="app">
        <AnimatePresence initial={false} mode="popLayout">
          {game ? (
            <motion.div
              key={game.id}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1, transition: { duration: 0.9, ease: EASE, delay: 0.12 } }}
              exit={{ opacity: 0, transition: { duration: 0.45, ease: EASE } }}
            >
              <Stage game={game} onBack={() => setOpen(null)} />
            </motion.div>
          ) : (
            <motion.div
              key="catalogue"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1, transition: { duration: 0.9, ease: EASE, delay: 0.12 } }}
              exit={{
                opacity: 0,
                filter: "blur(8px)",
                transition: { duration: 0.5, ease: EASE },
              }}
            >
              <Landing onOpen={setOpen} />
            </motion.div>
          )}
        </AnimatePresence>
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
    </MotionConfig>
  );
}
