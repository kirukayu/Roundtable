import { useEffect, useState } from "react";

import { Card, Chip, useToast } from "./ui";
import { api } from "../lib/ipc";
import type { Comparison, Fingerprint, GameId } from "../lib/types";

/**
 * Whether you and a friend can actually see each other.
 *
 * "Failed, no session found" almost always means one of four things does not
 * match: the game build, the co-op release, regulation.bin, or the password.
 * None of them is visible in game. This reads all four, prints a block to paste
 * into chat, and names the line that differs when you paste theirs back.
 */
export function MatchCard({
  game,
  edition,
}: {
  game: GameId;
  edition: string | null;
}) {
  const toast = useToast();
  const [mine, setMine] = useState<Fingerprint | null>(null);
  const [theirs, setTheirs] = useState("");
  const [result, setResult] = useState<Comparison | null>(null);

  useEffect(() => {
    api.matchFingerprint(game, edition).then(setMine).catch(() => setMine(null));
    setResult(null);
  }, [game, edition]);

  const check = async () => {
    if (!theirs.trim()) return;
    try {
      setResult(await api.matchCompare(game, theirs, edition));
    } catch (error) {
      toast.error("Could not compare", error instanceof Error ? error.message : String(error));
    }
  };

  const copy = async () => {
    if (!mine) return;
    await navigator.clipboard.writeText(mine.block);
    toast.success("Copied", "Paste it to whoever you are playing with.");
  };

  if (!mine) return null;

  return (
    <Card
      title="Match check"
      action={
        <button type="button" className="btn btn--ghost btn--sm" onClick={copy}>
          Copy
        </button>
      }
    >
      <div className="col2">
        {mine.traits.map((entry) => (
          <div className="between" key={entry.key}>
            <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
              {entry.label}
            </span>
            <span className="mono" style={{ fontSize: "var(--t-xs)" }}>
              {entry.value}
            </span>
          </div>
        ))}
      </div>

      <hr className="hr" />

      <textarea
        className="in"
        style={{ minHeight: 92 }}
        placeholder="Paste their block here"
        value={theirs}
        onChange={(event) => setTheirs(event.target.value)}
      />

      <div className="row" style={{ gap: "var(--s3)", marginTop: "var(--s3)" }}>
        <button
          type="button"
          className="btn btn--sm"
          onClick={check}
          disabled={!theirs.trim()}
        >
          Compare
        </button>

        {result?.verdict === "match" && <Chip tone="ok">Everything matches</Chip>}
        {result?.verdict === "unreadable" && <Chip tone="warn">That is not a block</Chip>}
        {result?.verdict === "differs" && (
          <Chip tone="bad">
            {result.differences.length === 1
              ? "1 difference"
              : `${result.differences.length} differences`}
          </Chip>
        )}
      </div>

      {result?.differences.map((difference) => (
        <div className="note note--bad" key={difference.label} style={{ marginTop: "var(--s3)" }}>
          <div>
            <div className="note__t">
              {difference.label}: {difference.mine} vs {difference.theirs}
            </div>
            <div className="note__b">{difference.matters}</div>
          </div>
        </div>
      ))}
    </Card>
  );
}
