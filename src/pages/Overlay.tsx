import { AnimatePresence, motion, useAnimationControls } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import type { AskTurn } from "../lib/types";

/**
 * The overlay.
 *
 * A column that stands over the game on a keypress, takes a question at the
 * bottom and grows the answer upward. Vertical because that is the shape of the
 * space beside a game: a wide panel across the middle covers the fight, a narrow
 * one down the side covers a wall. The line you type in is at the bottom, where
 * a chat window keeps it and where your hands already are.
 *
 * It has to earn its place in a second, so nothing waits for anything else. The
 * articles being read appear while the model is still working, and the answer
 * arrives a word at a time rather than all at once at the end of a spinner —
 * the same second and a half, spent very differently.
 *
 * It can be moved. The window is handed to the window manager on the first
 * pointer press, which then follows the mouse itself; dragging it from here,
 * one position per frame over HTTP, would trail behind the cursor.
 */

type Stage = "idle" | "working" | "answering" | "answered" | "failed";

interface Said {
  /** Whose line it is. */
  from: "you" | "it";
  text: string;
  sources?: string[];
  lane?: string | null;
  ms?: number | null;
}

/** Kept out of the reply that follows, so the model is not quoting itself. */
const REMEMBERED = 3;

/** Rising into place: slow out of the gate, settling rather than stopping. */
const ENTER = { duration: 0.42, ease: [0.16, 1, 0.3, 1] } as const;

/**
 * Going: quicker, and into the motion rather than out of it.
 *
 * Shorter than the entrance because leaving should feel like a decision that
 * has already been made. It has to finish inside the pause the window gives it.
 */
const LEAVE = { duration: 0.15, ease: [0.4, 0, 1, 1] } as const;

const OFF_SCREEN = { opacity: 0, y: 16, scale: 0.985 } as const;
const IN_PLACE = { opacity: 1, y: 0, scale: 1 } as const;

export function Overlay() {
  const [question, setQuestion] = useState("");
  const [stage, setStage] = useState<Stage>("idle");
  const [steps, setSteps] = useState<string[]>([]);
  const [said, setSaid] = useState<Said[]>([]);
  const sources = useRef<string[]>([]);
  const field = useRef<HTMLInputElement>(null);
  const thread = useRef<HTMLDivElement>(null);
  const asking = useRef<AbortController | null>(null);
  const column = useAnimationControls();

  // The window is shown and hidden rather than built each time, so focus has to
  // be taken every time it comes back rather than once on mount.
  useEffect(() => {
    const focus = () => field.current?.focus();
    focus();
    window.addEventListener("focus", focus);
    return () => window.removeEventListener("focus", focus);
  }, []);

  /**
   * Arriving and leaving, played on demand rather than on mount.
   *
   * Nothing here ever unmounts — the window is hidden and shown around a page
   * that stays loaded — so an `initial` prop animates exactly once, the first
   * time the overlay is opened after the app starts. Every opening after that
   * used to blink into place. The window says when it is shown and when it is
   * about to go, and the animation is driven from those.
   */
  useEffect(() => {
    const entering = () => {
      column.set(OFF_SCREEN);
      void column.start({ ...IN_PLACE, transition: ENTER });
      field.current?.focus();
    };
    const leaving = () => void column.start({ ...OFF_SCREEN, transition: LEAVE });

    entering();
    window.addEventListener("roundtable:shown", entering);
    window.addEventListener("roundtable:leaving", leaving);
    return () => {
      window.removeEventListener("roundtable:shown", entering);
      window.removeEventListener("roundtable:leaving", leaving);
    };
  }, [column]);

  // Out of the way first, gone second. The window waits for this.
  const dismiss = useCallback(async () => {
    asking.current?.abort();
    await column.start({ ...OFF_SCREEN, transition: LEAVE });
    void api.overlayHide();
  }, [column]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") void dismiss();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dismiss]);

  // The newest line stays in view as the answer grows into it.
  useEffect(() => {
    const box = thread.current;
    if (box) box.scrollTop = box.scrollHeight;
  }, [said, stage, steps]);

  const busy = stage === "working" || stage === "answering";

  const ask = async () => {
    const asked = question.trim();
    if (!asked || busy) return;

    asking.current?.abort();
    const stop = new AbortController();
    asking.current = stop;

    // What has been said already, so "and how do I beat her" has a her. Pairs
    // only — a question with no answer under it teaches the model nothing.
    const history: AskTurn[] = [];
    for (let i = 0; i < said.length - 1; i++) {
      if (said[i].from === "you" && said[i + 1].from === "it") {
        history.push({ question: said[i].text, answer: said[i + 1].text });
      }
    }

    setQuestion("");
    setSteps([]);
    sources.current = [];
    setStage("working");
    setSaid((was) => [...was, { from: "you", text: asked }]);

    let answered = false;
    try {
      for await (const event of api.askStream(asked, {
        history: history.slice(-REMEMBERED),
        signal: stop.signal,
      })) {
        if (stop.signal.aborted) return;

        switch (event.kind) {
          // What the model chose to do, not a spinner: the search it wrote,
          // the article it opened. It writes its own searches, so this is the
          // only honest account of what is happening.
          case "doing":
            setSteps((was) => (was.includes(event.note) ? was : [...was, event.note]));
            break;

          case "sources":
            sources.current = event.sources;
            break;

          case "delta":
            setStage("answering");
            setSaid((was) => {
              // The first piece opens a new line; every piece after it grows
              // the same one.
              if (answered) {
                const rest = was.slice(0, -1);
                const last = was[was.length - 1];
                return [...rest, { ...last, text: last.text + event.text }];
              }
              answered = true;
              return [...was, { from: "it", text: event.text, sources: sources.current }];
            });
            break;

          case "done":
            setStage("answered");
            setSaid((was) => {
              const rest = was.slice(0, -1);
              const last = was[was.length - 1];
              if (!last || last.from !== "it") return was;
              return [...rest, { ...last, lane: event.lane, ms: event.ms }];
            });
            break;

          case "failed":
            setStage("failed");
            setSaid((was) => [...was, { from: "it", text: event.error }]);
            break;
        }
      }
      if (!answered && stage !== "failed") setStage("answered");
    } catch (error) {
      if (stop.signal.aborted) return;
      setStage("failed");
      setSaid((was) => [
        ...was,
        { from: "it", text: error instanceof Error ? error.message : String(error) },
      ]);
    }
  };

  const empty = said.length === 0;

  return (
    <div className="ov">
      <motion.div className="ov__column" initial={OFF_SCREEN} animate={column}>
        {/* A slow sheen down the edge, the same one the site uses. */}
        <div className="ov__sheen" aria-hidden />

        {/*
          The whole top of the column is the handle. There is nothing to click
          up here anyway, and a window you can pick up anywhere is a window
          nobody has to look for the grip on.
        */}
        <div className="ov__grip" onPointerDown={() => void api.overlayDrag()}>
          <Mark busy={busy} />
          <span className="ov__title">Roundtable</span>
          <button
            type="button"
            className="ov__close"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={() => void dismiss()}
            aria-label="Close"
          >
            <svg viewBox="0 0 24 24" width={12} height={12} aria-hidden>
              <path
                d="M6 6l12 12M18 6L6 18"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>

        <div className="ov__thread" ref={thread}>
          {empty && (
            <motion.div
              className="ov__blank"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 0.7, delay: 0.25 }}
            >
              <p className="ov__blankLead">Ask about the game.</p>
              <p className="ov__blankSub">
                Both wikis are on this machine, so it reads them here and answers in
                whatever language you asked in.
              </p>
            </motion.div>
          )}

          {said.map((line, index) => (
            <motion.div
              className="ov__said"
              data-from={line.from}
              key={index}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.36, ease: [0.16, 1, 0.3, 1] }}
            >
              <p className="ov__text">{line.text}</p>

              {line.from === "it" && line.sources && line.sources.length > 0 && (
                <div className="ov__sources">
                  {line.sources.slice(0, 4).map((title) => (
                    <span className="ov__src" key={title}>
                      {title.split(" · ")[0]}
                    </span>
                  ))}
                </div>
              )}

              {line.lane && (
                <div className="ov__lane">
                  {line.lane}
                  {line.ms ? ` · ${(line.ms / 1000).toFixed(1)}s` : ""}
                </div>
              )}
            </motion.div>
          ))}

          <AnimatePresence>
            {stage === "working" && (
              <motion.div
                className="ov__working"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.3, ease: [0.16, 1, 0.3, 1] }}
              >
                <div className="ov__stage">
                  <span className="ov__dots" aria-hidden>
                    <i />
                    <i />
                    <i />
                  </span>
                  {steps.length === 0 ? "Thinking" : "Working"}
                </div>

                {/*
                  Every search the model wrote and every article it opened, in
                  the order it did them. These are its decisions, not a
                  progress bar — it chose the words, and seeing them is how you
                  can tell whether it understood the question.
                */}
                {steps.length > 0 && (
                  <div className="ov__steps">
                    {steps.map((note, index) => (
                      <motion.div
                        className="ov__step"
                        key={note}
                        initial={{ opacity: 0, x: -4 }}
                        animate={{ opacity: 1, x: 0 }}
                        transition={{ duration: 0.28, delay: index * 0.03 }}
                      >
                        {note}
                      </motion.div>
                    ))}
                  </div>
                )}
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        {/* The line you type in, at the bottom of the column. */}
        <div className="ov__ask" data-busy={busy}>
          <input
            ref={field}
            className="ov__field"
            value={question}
            spellCheck={false}
            placeholder={busy ? "Working…" : "Ask about the game"}
            onChange={(event) => setQuestion(event.target.value)}
            onKeyDown={(event) => event.key === "Enter" && void ask()}
          />
          <button
            type="button"
            className="ov__send"
            onClick={() => void ask()}
            disabled={busy || question.trim().length === 0}
            aria-label="Ask"
          >
            <svg viewBox="0 0 24 24" width={14} height={14} aria-hidden>
              <path
                d="M5 12h13M12 6l6 6-6 6"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.7"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </div>

        <div className="ov__hint">
          <span>Shift F1 closes</span>
          {said.length > 0 && (
            <button type="button" className="ov__wipe" onClick={() => setSaid([])}>
              Clear
            </button>
          )}
        </div>
      </motion.div>
    </div>
  );
}

/** The mark from the launcher, turning while it works. */
function Mark({ busy }: { busy: boolean }) {
  return (
    <motion.svg
      className="ov__mark"
      viewBox="0 0 24 24"
      width={14}
      height={14}
      animate={busy ? { rotate: 360 } : { rotate: 0 }}
      transition={
        busy
          ? { duration: 2.2, repeat: Infinity, ease: "linear" }
          : { duration: 0.6, ease: [0.16, 1, 0.3, 1] }
      }
      aria-hidden
    >
      <circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" strokeWidth="1" opacity={0.3} />
      <circle
        cx="12"
        cy="12"
        r="9"
        fill="none"
        stroke="currentColor"
        strokeWidth="1"
        strokeLinecap="round"
        strokeDasharray="14 43"
      />
    </motion.svg>
  );
}
