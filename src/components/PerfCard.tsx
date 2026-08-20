import { useCallback, useEffect, useState } from "react";

import { Icon } from "./Icons";
import { Card, Chip, useToast } from "./ui";
import { api } from "../lib/ipc";
import type { GameId, Machine, PerfStatus, Settings } from "../lib/types";

/**
 * The frame rate.
 *
 * Two things hold the game at 60 and neither is a setting. The frame limiter is
 * a float compiled into the code, and every time the game changes display mode
 * it asks Windows for 60 Hz whatever the monitor is set to — which is where the
 * halving to 30 comes from. Roundtable rewrites both in the running process, so
 * there is no DLL to find and nothing on disk to undo.
 *
 * The settings themselves are decided per machine. The same preset is wrong in
 * both directions: MAX on a 1650 is a slideshow, and dropping a 4080 to HIGH
 * gives away picture for frames it already had.
 */
export function PerfCard({ game }: { game: GameId }) {
  const toast = useToast();
  const [status, setStatus] = useState<PerfStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [busy, setBusy] = useState<"smooth" | "unlock" | "bounce" | null>(null);

  const load = useCallback(async () => {
    try {
      const [perf, saved] = await Promise.all([api.perf(game), api.settingsGet()]);
      setStatus(perf);
      setSettings(saved);
    } catch {
      setStatus(null);
    }
  }, [game]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!status) return null;

  const machine = status.machine;
  // The cap worth setting is the highest clean division of the panel this
  // machine holds every frame — not the panel itself. Half the frames at 180 and
  // half at 90 tears and judders; all of them at 90 does not.
  const best = machine.suggestedCap;
  const unlocked = settings?.unlockFps ?? null;
  const choices = [60, best, machine.refreshHz].filter(
    (fps, index, all) => fps > 0 && all.indexOf(fps) === index,
  );

  const smooth = async () => {
    setBusy("smooth");
    try {
      const changed = await api.perfSmooth(game);
      if (changed.length === 0) toast.info("Already tuned", "Nothing left to change.");
      else toast.success(`${changed.length} settings changed`, changed.slice(0, 3).join(" · "));
      await load();
    } catch (error) {
      toast.error("Could not apply", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const setCap = async (fps: number | null) => {
    if (!settings) return;
    setBusy("unlock");
    try {
      await api.settingsSet({ ...settings, unlockFps: fps });
      // Patch the running game too, rather than waiting for the next start.
      if (status.gameRunning) {
        const report = await api.perfUnlock(game, fps ?? 0);
        toast.success(
          fps ? `Capped at ${report.fps}` : "Back to 60",
          report.hertz ? "Frame limiter and the 60 Hz lock" : "Frame limiter",
        );
      } else {
        toast.success(fps ? `Set to ${fps}` : "Back to 60", "Applied when the game starts.");
      }
      await load();
    } catch (error) {
      toast.error("Could not apply", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Card
      title="Frame rate"
      action={
        <div className="row" style={{ gap: "var(--s2)" }}>
          {status.exclusiveFullscreen && <Chip tone="bad">Capped at 60, halves to 30</Chip>}
          {status.path && status.improvable > 0 && (
            <button
              type="button"
              className="btn btn--solid btn--sm"
              onClick={smooth}
              disabled={busy !== null}
            >
              {busy === "smooth" ? <span className="spin" /> : null}
              Smooth it out
            </button>
          )}
          {status.path && status.improvable === 0 && <Chip tone="ok">Tuned</Chip>}
        </div>
      }
    >
      {status.exclusiveFullscreen && (
        <div className="note note--bad" style={{ marginBottom: "var(--s4)" }}>
          <Icon.Warning size={15} />
          <div>
            <div className="note__t">Exclusive fullscreen is why you get 30</div>
            <div className="note__b">
              The game asks Windows for a 60 Hz mode and holds vsync to it. One late
              frame and it halves to exactly 30 until the next second.
              {" "}
              It also swallows the overlay — Shift F1 opens it and draws it, and
              nothing appears over an exclusive-fullscreen game, which reads as a
              broken key rather than as a mode you chose.
              {" "}
              Borderless has neither problem
              {status.improvable > 0 ? ", and Smooth it out switches to it." : "."}
            </div>
          </div>
        </div>
      )}

      <MachineRow machine={machine} />

      <hr className="hr" />

      <div className="between">
        <div>
          <div className="w3" style={{ fontSize: "var(--t-sm)" }}>
            Frame cap
          </div>
          <div className="w4" style={{ fontSize: "var(--t-xs)", marginTop: 2 }}>
            {unlocked
              ? `Rewritten to ${unlocked} in the running game.`
              : "The game ships locked to 60."}
          </div>
        </div>
        <div className="row" style={{ gap: "var(--s2)" }}>
          {choices.map((fps) => (
            <button
              key={fps}
              type="button"
              className="codex__filter"
              data-on={fps === 60 ? unlocked === null : unlocked === fps}
              disabled={busy !== null}
              onClick={() => void setCap(fps === 60 ? null : fps)}
            >
              {fps}
              {fps === best && fps !== 60 ? " · steady" : ""}
              {fps === machine.refreshHz && fps !== best ? " · your screen" : ""}
            </button>
          ))}
        </div>
      </div>

      <p className="w4" style={{ fontSize: "var(--t-xs)", marginTop: "var(--s2)", lineHeight: 1.7 }}>
        Written into the running game and nothing on disk, so closing it undoes
        everything. Never with the anti-cheat on — that is a ban, and Roundtable
        refuses while it is armed.
        {status.unlocker && " An unlocker DLL is also in the game folder; it is not needed."}
      </p>

      <hr className="hr" />

      {/*
        Windows sometimes leaves the desktop at its full refresh on paper while
        the pointer is still drawn at sixty — usually after a game exits or after
        sleep. Nothing looks wrong in Settings, because nothing is. Picking
        another refresh rate and picking this one back rebuilds the mode, which
        is the cure people find by hand.
      */}
      <div className="between">
        <div>
          <div className="w3" style={{ fontSize: "var(--t-sm)" }}>
            Pointer moving in steps
          </div>
          <div className="w4" style={{ fontSize: "var(--t-xs)", marginTop: 2, lineHeight: 1.7 }}>
            Windows can leave the cursor at 60 while the desktop says otherwise.
            Rebuilding the display mode fixes it. Shift F2 does this mid-game.
          </div>
        </div>
        <button
          type="button"
          className="btn btn--sm"
          disabled={busy !== null}
          onClick={async () => {
            setBusy("bounce");
            try {
              toast.success("Fixed", await api.perfBounce());
            } catch (error) {
              toast.error("Could not", error instanceof Error ? error.message : String(error));
            } finally {
              setBusy(null);
            }
          }}
        >
          {busy === "bounce" ? <span className="spin" /> : null}
          Fix it
        </button>
      </div>

      {status.path && (
        <>
          <hr className="hr" />
          <div className="col2">
            {status.settings.map((setting) => (
              <div className="between" key={setting.key}>
                <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
                  {spaced(setting.key)}
                </span>
                <span className="row" style={{ gap: "var(--s3)" }}>
                  <span
                    className="mono"
                    style={{
                      fontSize: "var(--t-xs)",
                      color: setting.suggested ? "var(--warn)" : "var(--w2)",
                    }}
                  >
                    {setting.value}
                  </span>
                  {setting.suggested && (
                    <span className="mono w4" style={{ fontSize: "var(--t-xs)" }}>
                      → {setting.suggested}
                    </span>
                  )}
                </span>
              </div>
            ))}
          </div>
        </>
      )}

      {!status.path && (
        <p className="w3" style={{ fontSize: "var(--t-sm)", marginTop: "var(--s3)" }}>
          Start the game once so it writes its graphics settings, then the preset can
          be worked out.
        </p>
      )}
    </Card>
  );
}

/** What the preset was decided from, so the numbers are not a black box. */
function MachineRow({ machine }: { machine: Machine }) {
  const tier: Record<Machine["tier"], string> = {
    weak: "aiming for playable",
    modest: "comfortable at 1080p",
    strong: "comfortable at 1440p",
    ample: "everything on",
  };

  const parts = [
    machine.gpu,
    machine.vramMb > 0 ? `${Math.round(machine.vramMb / 1024)} GB VRAM` : null,
    machine.width > 0 ? `${machine.width}x${machine.height} at ${machine.refreshHz} Hz` : null,
  ].filter(Boolean);

  return (
    <div>
      <div className="between">
        <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
          Your machine
        </span>
        <Chip>{tier[machine.tier]}</Chip>
      </div>
      <div className="mono w4" style={{ fontSize: "var(--t-xs)", marginTop: "var(--s2)" }}>
        {parts.join("  ·  ")}
      </div>
    </div>
  );
}

/** `ShadowQuality` reads better as `Shadow quality`. */
function spaced(key: string): string {
  const words = key.replace(/([a-z])([A-Z])/g, "$1 $2");
  return words.charAt(0) + words.slice(1).toLowerCase();
}
