import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { api } from "./ipc";
import type { GameId, GameInfo, Profile, Settings } from "./types";

interface Store {
  settings: Settings;
  games: GameInfo[];
  /** Games with a confirmed installation, so the library can show real state. */
  installed: Set<GameId>;
  profiles: Profile[];
  activeProfile: Profile | null;
  /** Which game is running right now, if any. */
  gameRunning: GameId | null;

  patchSettings: (patch: Partial<Settings>) => Promise<void>;
  refreshProfiles: () => Promise<void>;
  refreshInstalled: () => Promise<void>;
}

const StoreContext = createContext<Store | null>(null);

export function useApp(): Store {
  const store = useContext(StoreContext);
  if (!store) throw new Error("useApp must be used inside <AppProvider>");
  return store;
}

export function AppProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [games, setGames] = useState<GameInfo[]>([]);
  const [installed, setInstalled] = useState<Set<GameId>>(new Set());
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [gameRunning, setGameRunning] = useState<GameId | null>(null);

  const refreshInstalled = useCallback(async () => {
    const current = await api.settingsGet();
    setSettings(current);
    setInstalled(new Set(current.installations.map((i) => i.game)));
  }, []);

  const refreshProfiles = useCallback(async () => {
    const all = await Promise.all(
      (await api.games()).map((game) => api.profilesList(game.id).catch(() => [])),
    );
    setProfiles(all.flat());
  }, []);

  useEffect(() => {
    void (async () => {
      const [loadedSettings, loadedGames] = await Promise.all([
        api.settingsGet(),
        api.games(),
      ]);
      setSettings(loadedSettings);
      setGames(loadedGames);
      setInstalled(new Set(loadedSettings.installations.map((i) => i.game)));

      const perGame = await Promise.all(
        loadedGames.map((game) => api.profilesList(game.id).catch(() => [])),
      );
      setProfiles(perGame.flat());
    })();
  }, []);

  // A single call covers every title, so the rail and the Play button agree.
  useEffect(() => {
    if (games.length === 0) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const active = await api.runningGame();
        if (!cancelled) setGameRunning(active);
      } catch {
        /* the process list is best effort */
      }
    };
    void poll();
    const timer = window.setInterval(poll, 5000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [games]);

  useEffect(() => {
    if (!settings) return;
    const root = document.documentElement;
    root.dataset.accent = settings.accent;
    root.dataset.reduceMotion = String(settings.reduceMotion);
  }, [settings]);

  const patchSettings = useCallback(async (patch: Partial<Settings>) => {
    setSettings((current) => {
      if (!current) return current;
      const next = { ...current, ...patch };
      void api.settingsSet(next).catch(() => undefined);
      if (patch.installations) {
        setInstalled(new Set(patch.installations.map((i) => i.game)));
      }
      return next;
    });
  }, []);

  const activeProfile = useMemo(() => {
    if (!settings?.activeProfile) return null;
    return profiles.find((p) => p.id === settings.activeProfile) ?? null;
  }, [profiles, settings?.activeProfile]);

  const value = useMemo<Store | null>(() => {
    if (!settings) return null;
    return {
      settings,
      games,
      installed,
      profiles,
      activeProfile,
      gameRunning,
      patchSettings,
      refreshProfiles,
      refreshInstalled,
    };
  }, [
    settings,
    games,
    installed,
    profiles,
    activeProfile,
    gameRunning,
    patchSettings,
    refreshProfiles,
    refreshInstalled,
  ]);

  if (!value) {
    return (
      <div
        style={{
          height: "100%",
          display: "grid",
          placeItems: "center",
          background: "var(--bg)",
        }}
      >
        <span className="spin" />
      </div>
    );
  }

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>;
}
