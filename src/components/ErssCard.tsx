import { useCallback, useEffect, useState } from "react";

import { Icon } from "./Icons";
import { Card, Chip, useToast } from "./ui";
import { api } from "../lib/ipc";
import type { ErssStatus, GameId } from "../lib/types";

/**
 * DLSS, frame generation and Reflex, in a game that shipped with none of them.
 *
 * ELDEN RING has no upscaler at all, which is why the usual wrappers — the ones
 * that swap an existing DLSS for a newer one — cannot help it. huutaiii's ERSS
 * stands in front of D3D12 itself and brings the whole stack: DLSS 4 and DLAA,
 * DLSS frame generation, FSR 3.1, XeSS and Reflex, and it lifts the sixty cap on
 * its own.
 *
 * Everything it needs around it, Roundtable already deals with: the anti-cheat
 * off, hardware GPU scheduling on, and ray tracing down because it flickers the
 * lighting with this mod loaded. So this is a button rather than a page of
 * instructions — the only thing it cannot do for itself is know the password the
 * release archives are locked with.
 */
export function ErssCard({ game }: { game: GameId }) {
  const toast = useToast();
  const [status, setStatus] = useState<ErssStatus | null>(null);
  const [password, setPassword] = useState("");
  const [overlay, setOverlay] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string[]>([]);

  const load = useCallback(async () => {
    try {
      setStatus(await api.erss(game));
    } catch {
      setStatus(null);
    }
  }, [game]);

  useEffect(() => {
    void load();
  }, [load]);

  // Only offered where the mod exists, which for now is one game.
  if (!status || game !== "elden-ring") return null;

  const releases = status.archives.filter((path) => /ERSS-FG-v/i.test(path));

  const install = async () => {
    setBusy(true);
    try {
      const result = await api.erssInstall(game, overlay, password || undefined);
      setDone(result.changes);
      setPassword("");
      toast.success("Installed", result.changes[0] ?? "");
      await load();
    } catch (error) {
      toast.error("Could not install", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    try {
      const gone = await api.erssUninstall(game);
      setDone([]);
      toast.success("Removed", `${gone.length} files and folders`);
      await load();
    } catch (error) {
      toast.error("Could not remove", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card
      title="DLSS and frame generation"
      action={
        <div className="row" style={{ gap: "var(--s2)" }}>
          {status.installed && <Chip tone="ok">{status.version ?? "installed"}</Chip>}
          {status.installed ? (
            <button type="button" className="btn btn--sm" onClick={remove} disabled={busy}>
              {busy ? <span className="spin" /> : null}
              Remove
            </button>
          ) : (
            <button
              type="button"
              className="btn btn--solid btn--sm"
              onClick={install}
              disabled={busy || releases.length === 0}
            >
              {busy ? <span className="spin" /> : null}
              Install
            </button>
          )}
        </div>
      }
    >
      <p className="w4" style={{ fontSize: "var(--t-xs)", marginBottom: "var(--s3)", lineHeight: 1.7 }}>
        The game has no upscaler of its own, so this adds one: DLSS 4 and DLAA, DLSS
        frame generation, FSR 3.1, XeSS and Reflex, and the sixty cap goes with it.
        Press Home in game for its settings. By huutaiii.
      </p>

      {releases.length === 0 && (
        <div className="note note--warn">
          <Icon.Warning size={15} />
          <div>
            <div className="note__t">No release archive here yet</div>
            <div className="note__b">
              Download one and it will be picked up from your Downloads folder — nothing
              else to do.
            </div>
          </div>
        </div>
      )}

      {releases.length > 0 && !status.installed && (
        <div className="between" style={{ marginBottom: "var(--s3)" }}>
          <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
            Newest release found
          </span>
          <span className="mono w4" style={{ fontSize: "var(--t-xs)" }}>
            {releases[0].split("\\").pop()}
          </span>
        </div>
      )}

      {status.needsPassword && !status.installed && (
        <div style={{ marginBottom: "var(--s3)" }}>
          <label className="w3" style={{ fontSize: "var(--t-sm)", display: "block" }}>
            Archive password
          </label>
          <input
            type="password"
            className="input"
            style={{ marginTop: "var(--s2)", width: "100%" }}
            value={password}
            placeholder="from the post you downloaded it from"
            onChange={(event) => setPassword(event.target.value)}
          />
          <p className="w4" style={{ fontSize: "var(--t-xs)", marginTop: "var(--s2)" }}>
            The files inside are encrypted. Used to unpack them and not kept anywhere.
          </p>
        </div>
      )}

      {!status.installed && (
        <label className="row" style={{ gap: "var(--s2)", marginBottom: "var(--s3)" }}>
          <input
            type="checkbox"
            checked={overlay}
            onChange={(event) => setOverlay(event.target.checked)}
          />
          <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
            Keep the Steam overlay
          </span>
          <span className="w4" style={{ fontSize: "var(--t-xs)" }}>
            renames the loader, which the overlay needs the original name of
          </span>
        </label>
      )}

      {status.blockers.map((line) => (
        <div className="note note--warn" style={{ marginBottom: "var(--s2)" }} key={line}>
          <Icon.Warning size={15} />
          <div className="note__b">{line}</div>
        </div>
      ))}

      {status.installed && (
        <div className="col2">
          <div className="between">
            <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
              Loader
            </span>
            <span className="mono w4" style={{ fontSize: "var(--t-xs)" }}>
              {status.loader ?? "missing"}
            </span>
          </div>
          <div className="between">
            <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
              Frame time addon
            </span>
            <span className="mono w4" style={{ fontSize: "var(--t-xs)" }}>
              {status.frameTimeAddon ? "in" : "not in"}
            </span>
          </div>
        </div>
      )}

      {done.length > 0 && (
        <>
          <hr className="hr" />
          <div className="col" style={{ gap: "var(--s1)" }}>
            {done.map((line) => (
              <div className="mono w3" style={{ fontSize: "var(--t-xs)" }} key={line}>
                {line}
              </div>
            ))}
          </div>
        </>
      )}
    </Card>
  );
}
