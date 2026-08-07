import { useCallback, useEffect, useState } from "react";

import { Icon } from "./Icons";
import { Card, Chip, useToast } from "./ui";
import { api } from "../lib/ipc";
import type { ErssSetting, ErssStatus, GameId } from "../lib/types";

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
  const [askPassword, setAskPassword] = useState(false);
  const [overlay, setOverlay] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string[]>([]);
  const [fixes, setFixes] = useState<string[]>([]);

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

  /**
   * The three files that decide whether generated frames look right, set at once.
   *
   * They are the game's own graphics config, the mod's config and the frame cap,
   * and nobody would think to connect them — the flickering light this mod is
   * known for is a global illumination setting, and the upscaler that feeds the
   * generator its frames is somewhere else entirely.
   */
  const tune = async () => {
    setBusy(true);
    try {
      const changed = await api.erssTune(game);
      setFixes(changed);
      toast.success("Tuned", `${changed.length} settings`);
      await load();
    } catch (error) {
      toast.error("Could not tune", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    try {
      const gone = await api.erssUninstall(game);
      setDone([]);
      setFixes([]);
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

      {status.release && !status.installed && (
        <div className="between" style={{ marginBottom: "var(--s3)" }}>
          <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
            Will install
          </span>
          <span className="mono w4" style={{ fontSize: "var(--t-xs)" }}>
            {status.release.split("\\").pop()}
          </span>
        </div>
      )}

      {/*
        Encrypted releases open on their own — the author prints the password
        beside the download, so asking for it would be asking somebody to fetch
        something Roundtable already has. The field is here for the release that
        changes it, folded away until it is wanted.
      */}
      {status.locked && !status.installed && (
        <div style={{ marginBottom: "var(--s3)" }}>
          {!askPassword ? (
            <div className="between">
              <span className="w4" style={{ fontSize: "var(--t-xs)" }}>
                That release is encrypted and opens with the published password
              </span>
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => setAskPassword(true)}
              >
                Use another
              </button>
            </div>
          ) : (
            <>
              <label className="w3" style={{ fontSize: "var(--t-sm)", display: "block" }}>
                Archive password
              </label>
              <input
                type="password"
                className="in"
                style={{ marginTop: "var(--s2)", width: "100%" }}
                value={password}
                placeholder="from the post you downloaded it from"
                onChange={(event) => setPassword(event.target.value)}
              />
              <p className="w4" style={{ fontSize: "var(--t-xs)", marginTop: "var(--s2)" }}>
                Used to unpack it and not kept anywhere.
              </p>
            </>
          )}
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

      {/*
        The mod's own settings, changed here rather than in its in-game overlay.
        Turning frame generation on needs a restart anyway, so setting it before
        the game starts is the shorter path — and the values are read back out of
        its own file, so this stays right as the mod grows new ones.
      */}
      {status.installed && (
        <>
          <hr className="hr" />
          <div className="between" style={{ marginBottom: "var(--s3)" }}>
            <div>
              <div className="w3" style={{ fontSize: "var(--t-sm)" }}>
                Clean up the artefacts
              </div>
              <div className="w4" style={{ fontSize: "var(--t-2xs)", marginTop: 2 }}>
                Generated frames are guessed from the two either side, so anything that
                changes for reasons the motion cannot explain gets guessed wrong
              </div>
            </div>
            <button type="button" className="btn btn--sm" onClick={tune} disabled={busy}>
              {busy ? <span className="spin" /> : null}
              Tune
            </button>
          </div>

          {fixes.length > 0 && (
            <div className="col" style={{ gap: "var(--s2)", marginBottom: "var(--s3)" }}>
              {fixes.map((line) => (
                <div className="row" style={{ gap: "var(--s2)", alignItems: "flex-start" }} key={line}>
                  <Icon.Check size={13} />
                  <span className="w4" style={{ fontSize: "var(--t-xs)", lineHeight: 1.6 }}>
                    {line}
                  </span>
                </div>
              ))}
            </div>
          )}
        </>
      )}

      {/*
        The mod's own settings, changed here rather than in its in-game overlay.
        Turning frame generation on needs a restart anyway, so setting it before
        the game starts is the shorter path — and the values are read back out of
        its own file, so this stays right as the mod grows new ones.
      */}
      {status.installed && status.settings.length > 0 && (
        <>
          <hr className="hr" />
          <div className="between" style={{ marginBottom: "var(--s3)" }}>
            <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
              Its settings
            </span>
            <span className="w4" style={{ fontSize: "var(--t-2xs)" }}>
              Frame generation needs a restart
            </span>
          </div>
          <div className="col2">
            {status.settings.map((setting) => (
              <ErssSettingRow
                key={setting.key}
                game={game}
                setting={setting}
                onDone={load}
                disabled={busy}
              />
            ))}
          </div>
          <p className="w4" style={{ fontSize: "var(--t-2xs)", marginTop: "var(--s3)", lineHeight: 1.6 }}>
            The rest of them — which upscaler, whether frames are generated, HDR — appear
            here after the game has run once with the mod loaded. It names them itself
            the first time, and Roundtable reads whatever it finds rather than assuming.
          </p>
        </>
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

/**
 * One of the mod's settings.
 *
 * The control follows the value's own type, read out of the TOML: a boolean
 * gets a toggle, a known set of choices gets buttons, anything else gets a
 * field. That way a setting the mod adds in its next release still appears and
 * still works, without this file knowing it exists.
 */
function ErssSettingRow({
  game,
  setting,
  onDone,
  disabled,
}: {
  game: GameId;
  setting: ErssSetting;
  onDone: () => Promise<void>;
  disabled: boolean;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState(setting.value);

  const write = async (value: string) => {
    setBusy(true);
    try {
      await api.erssSet(game, setting.key, value);
      await onDone();
    } catch (error) {
      toast.error("Could not set", error instanceof Error ? error.message : String(error));
      setDraft(setting.value);
    } finally {
      setBusy(false);
    }
  };

  const off = disabled || busy;

  return (
    <div className="between" title={setting.detail}>
      <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
        {setting.title}
      </span>

      {setting.kind === "bool" && (
        <button
          type="button"
          className="codex__filter"
          data-on={setting.value === "true"}
          disabled={off}
          onClick={() => void write(setting.value === "true" ? "false" : "true")}
        >
          {setting.value === "true" ? "on" : "off"}
        </button>
      )}

      {setting.kind !== "bool" && setting.choices.length > 0 && (
        <span className="row" style={{ gap: "var(--s2)", flexWrap: "wrap", justifyContent: "flex-end" }}>
          {setting.choices.map((choice) => (
            <button
              key={choice.value}
              type="button"
              className="codex__filter"
              data-on={setting.value === choice.value}
              disabled={off}
              onClick={() => void write(choice.value)}
            >
              {choice.label}
            </button>
          ))}
        </span>
      )}

      {setting.kind !== "bool" && setting.choices.length === 0 && (
        <input
          className="in"
          style={{ width: 120, textAlign: "right" }}
          value={draft}
          disabled={off}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => draft !== setting.value && void write(draft)}
          onKeyDown={(event) => event.key === "Enter" && void write(draft)}
        />
      )}
    </div>
  );
}
