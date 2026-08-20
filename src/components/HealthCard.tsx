import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";

import { Icon } from "./Icons";
import { EASE, SOFT } from "./Motion";
import { Card, Chip } from "./ui";
import { api } from "../lib/ipc";
import type { DiagnoseReport, Finding, GameId } from "../lib/types";

/**
 * What is wrong, before it goes wrong.
 *
 * Seamless Co-op fails in a handful of known ways and its FAQ maps each error
 * to a cause. Those are checks a program can run, so they run here — with the
 * error text people actually see, so somebody who already hit it recognises
 * their own problem.
 *
 * When nothing is wrong this collapses to one line. A panel of green ticks is
 * noise on a screen you visit to launch a game.
 */
export function HealthCard({
  game,
  edition,
}: {
  game: GameId;
  edition: string | null;
}) {
  const [report, setReport] = useState<DiagnoseReport | null>(null);
  const [open, setOpen] = useState(false);

  const load = useCallback(async () => {
    try {
      setReport(await api.diagnose(game, edition));
    } catch {
      setReport(null);
    }
  }, [game, edition]);

  useEffect(() => {
    void load();
  }, [load]);

  if (!report) return null;

  const problems = report.findings.filter(
    (f) => f.level === "blocker" || f.level === "warning",
  );
  const rest = report.findings.filter((f) => f.level !== "blocker" && f.level !== "warning");
  const shown = open ? [...problems, ...rest] : problems;

  return (
    <Card
      title="Checks"
      action={
        <div className="row" style={{ gap: "var(--s2)" }}>
          {report.blockers > 0 && <Chip tone="bad">{report.blockers} blocking</Chip>}
          {report.warnings > 0 && <Chip tone="warn">{report.warnings} to look at</Chip>}
          {problems.length === 0 && <Chip tone="ok">All clear</Chip>}
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            onClick={() => setOpen((was) => !was)}
          >
            {open ? "Less" : `All ${report.findings.length}`}
          </button>
        </div>
      }
    >
      {shown.length === 0 ? (
        <p className="w3" style={{ fontSize: "var(--t-sm)" }}>
          Nothing here will stop the game starting.
        </p>
      ) : (
        <div className="hc">
          <AnimatePresence initial={false}>
            {shown.map((finding, index) => (
              <Row key={finding.id} finding={finding} index={index} />
            ))}
          </AnimatePresence>
        </div>
      )}
    </Card>
  );
}

function Row({ finding, index }: { finding: Finding; index: number }) {
  const Glyph =
    finding.level === "blocker"
      ? Icon.Warning
      : finding.level === "warning"
        ? Icon.Warning
        : finding.level === "pass"
          ? Icon.Check
          : Icon.Info;

  return (
    <motion.div
      className="hc__row"
      data-level={finding.level}
      layout
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0, transition: { duration: 0.4, ease: EASE, delay: index * 0.03 } }}
      exit={{ opacity: 0, transition: { duration: 0.2 } }}
      transition={{ duration: 0.35, ease: SOFT }}
    >
      <Glyph size={14} />
      <div className="hc__text">
        <div className="hc__title">{finding.title}</div>
        <div className="hc__detail">{finding.detail}</div>
        {finding.symptom && <div className="hc__symptom">“{finding.symptom}”</div>}
        {finding.fix && <div className="hc__fix">{finding.fix}</div>}
      </div>
    </motion.div>
  );
}
