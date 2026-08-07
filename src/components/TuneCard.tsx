import { useCallback, useEffect, useState } from "react";

import { Icon } from "./Icons";
import { Card, Chip, useToast } from "./ui";
import { api } from "../lib/ipc";
import type { GameId, TuneStatus } from "../lib/types";

/**
 * Everything outside the game that decides how it feels.
 *
 * A borderless window is composited by the desktop by default, which costs a
 * frame of latency and blocks variable refresh; Windows can put it on the same
 * presentation path exclusive fullscreen uses, and that switch is in no menu the
 * game has. Alongside it: which card the game runs on, background recording that
 * encodes every frame nobody watches, and pointer acceleration that turns a fast
 * flick into a jump.
 *
 * One button applies the lot, and every change comes back as a line. Nothing is
 * silent and nothing is one-way.
 */
export function TuneCard({ game }: { game: GameId }) {
  const toast = useToast();
  const [status, setStatus] = useState<TuneStatus | null>(null);
  const [busy, setBusy] = useState<"apply" | "revert" | null>(null);
  const [done, setDone] = useState<string[]>([]);

  const load = useCallback(async () => {
    try {
      setStatus(await api.tune(game));
    } catch {
      setStatus(null);
    }
  }, [game]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!status) return null;

  const left = status.levers.filter((lever) => !lever.done);

  const apply = async () => {
    setBusy("apply");
    try {
      const result = await api.tuneApply(game);
      setDone(result.changes);
      if (result.changes.length === 0) toast.info("Nothing left", "Already as good as it gets.");
      else toast.success(`${result.changes.length} changes`, result.changes[0]);
      await load();
    } catch (error) {
      toast.error("Could not apply", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  const revert = async () => {
    setBusy("revert");
    try {
      const back = await api.tuneRevert();
      setDone([]);
      toast.success("Put back", `${back.length} settings restored`);
      await load();
    } catch (error) {
      toast.error("Could not revert", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Card
      title="Optimise"
      action={
        <div className="row" style={{ gap: "var(--s2)" }}>
          {left.length === 0 && <Chip tone="ok">Done</Chip>}
          <button
            type="button"
            className="btn btn--solid btn--sm"
            onClick={apply}
            disabled={busy !== null}
          >
            {busy === "apply" ? <span className="spin" /> : null}
            Optimise everything
          </button>
          <button type="button" className="btn btn--sm" onClick={revert} disabled={busy !== null}>
            Put back
          </button>
        </div>
      }
    >
      <p className="w4" style={{ fontSize: "var(--t-xs)", marginBottom: "var(--s3)", lineHeight: 1.7 }}>
        Graphics settings for this machine, a frame cap it holds every frame, and the
        Windows switches the game cannot reach. Read before written, and every one of
        them reversible.
      </p>

      <div className="col2">
        {status.levers
          .filter((lever) => !lever.byHand)
          .map((lever) => (
            <div className="between" key={lever.id} title={lever.detail}>
              <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
                {lever.title}
              </span>
              <span className="row" style={{ gap: "var(--s3)" }}>
                <span
                  className="mono"
                  style={{
                    fontSize: "var(--t-xs)",
                    color: lever.done ? "var(--w2)" : "var(--warn)",
                  }}
                >
                  {lever.done ? "on" : lever.current}
                </span>
                {lever.needsAdmin && !lever.done && (
                  <span className="mono w4" style={{ fontSize: "var(--t-xs)" }}>
                    needs admin
                  </span>
                )}
              </span>
            </div>
          ))}
      </div>

      {status.levers
        .filter((lever) => lever.byHand)
        .map((lever) => (
          <div className="note" style={{ marginTop: "var(--s4)" }} key={lever.id}>
            <Icon.Info size={15} />
            <div>
              <div className="note__t">{lever.title} — worth more than everything above</div>
              <div className="note__b">
                {lever.detail} No program can tick that box, so this one is by hand:{" "}
                {lever.byHand}
              </div>
            </div>
          </div>
        ))}

      {status.competitors.length > 0 && (
        <div className="note note--warn" style={{ marginTop: "var(--s4)" }}>
          <Icon.Warning size={15} />
          <div>
            <div className="note__t">Something else is on the graphics card</div>
            <div className="note__b">
              {status.competitors.join(". ")}. Roundtable will not close these — a
              screen share is usually deliberate — but they cost frames while you play.
            </div>
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
