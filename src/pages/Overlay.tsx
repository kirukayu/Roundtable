import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";

/**
 * The overlay.
 *
 * A window that sits over the game on a keypress, takes a question, and answers
 * it out of the wiki. It is deliberately one thing: a line to type in, and the
 * answer underneath. Anything else belongs in the launcher, where there is room
 * for it and nobody is waiting to get back to a boss.
 *
 * It has to earn its place in a second. So the searching and the thinking are
 * shown as they happen — the articles being read appear while the model is
 * still working, because a wait you can see the shape of is a shorter wait.
 */

type Stage = "idle" | "searching" | "thinking" | "answered" | "failed";

interface Result {
  answer: string;
  sources: string[];
  lane?: string | null;
  ms?: number | null;
}

export function Overlay() {
  const [question, setQuestion] = useState("");
  const [stage, setStage] = useState<Stage>("idle");
  const [found, setFound] = useState<string[]>([]);
  const [result, setResult] = useState<Result | null>(null);
  const [failure, setFailure] = useState("");
  const field = useRef<HTMLInputElement>(null);
  const asking = useRef(0);

  // The window is shown and hidden rather than built each time, so focus has to
  // be taken every time it comes back rather than once on mount.
  useEffect(() => {
    const focus = () => field.current?.focus();
    focus();
    window.addEventListener("focus", focus);
    return () => window.removeEventListener("focus", focus);
  }, []);

  const dismiss = useCallback(() => {
    void api.overlayHide();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") dismiss();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dismiss]);

  const ask = async () => {
    const asked = question.trim();
    if (!asked || stage === "searching" || stage === "thinking") return;

    // A late reply from a question that has been replaced must not overwrite
    // the answer to the current one.
    const turn = ++asking.current;
    setStage("searching");
    setFound([]);
    setResult(null);
    setFailure("");

    // Named first, answered second: the titles come back in milliseconds and
    // give something true to look at while the model works.
    void api
      .askSources(asked)
      .then((titles) => {
        if (turn === asking.current) {
          setFound(titles);
          setStage((was) => (was === "searching" ? "thinking" : was));
        }
      })
      .catch(() => {});

    try {
      const answer = await api.ask(asked);
      if (turn !== asking.current) return;
      setResult(answer);
      setFound(answer.sources);
      setStage("answered");
    } catch (error) {
      if (turn !== asking.current) return;
      setFailure(error instanceof Error ? error.message : String(error));
      setStage("failed");
    }
  };

  const busy = stage === "searching" || stage === "thinking";

  return (
    <div className="ov" onMouseDown={(e) => e.target === e.currentTarget && dismiss()}>
      <motion.div
        className="ov__card"
        initial={{ opacity: 0, y: 14, scale: 0.985 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ duration: 0.42, ease: [0.16, 1, 0.3, 1] }}
      >
        {/* A slow sheen across the top edge, the same one the site uses. */}
        <div className="ov__sheen" aria-hidden />

        <div className="ov__ask">
          <Mark busy={busy} />
          <input
            ref={field}
            className="ov__field"
            value={question}
            spellCheck={false}
            placeholder="Ask about the game"
            onChange={(event) => setQuestion(event.target.value)}
            onKeyDown={(event) => event.key === "Enter" && void ask()}
          />
          <kbd className="ov__kbd">{busy ? "…" : "↵"}</kbd>
        </div>

        <AnimatePresence mode="wait">
          {stage !== "idle" && (
            <motion.div
              key={stage === "answered" || stage === "failed" ? "done" : "working"}
              className="ov__body"
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.34, ease: [0.16, 1, 0.3, 1] }}
            >
              <div className="ov__line" />

              {busy && <Working stage={stage} found={found} />}

              {stage === "answered" && result && (
                <motion.div
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.4, ease: [0.16, 1, 0.3, 1] }}
                >
                  <p className="ov__answer">{result.answer}</p>
                  <div className="ov__foot">
                    <div className="ov__sources">
                      {result.sources.slice(0, 4).map((title) => (
                        <span className="ov__src" key={title}>
                          {title}
                        </span>
                      ))}
                    </div>
                    {result.lane && <span className="ov__lane">{result.lane}</span>}
                  </div>
                </motion.div>
              )}

              {stage === "failed" && <p className="ov__failed">{failure}</p>}
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>

      <div className="ov__hint">
        <span>Shift F1 to close</span>
      </div>
    </div>
  );
}

/** The mark from the launcher, turning while it works. */
function Mark({ busy }: { busy: boolean }) {
  return (
    <motion.svg
      className="ov__mark"
      viewBox="0 0 24 24"
      width={15}
      height={15}
      animate={busy ? { rotate: 360 } : { rotate: 0 }}
      transition={
        busy
          ? { duration: 2.4, repeat: Infinity, ease: "linear" }
          : { duration: 0.6, ease: [0.16, 1, 0.3, 1] }
      }
      aria-hidden
    >
      <circle cx="12" cy="12" r="9" fill="none" stroke="currentColor" strokeWidth="1" opacity={0.35} />
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

/**
 * What is happening, while it happens.
 *
 * The article titles are real — they are what the answer will be built from —
 * so this is progress rather than a decorative spinner.
 */
function Working({ stage, found }: { stage: Stage; found: string[] }) {
  return (
    <div className="ov__working">
      <div className="ov__stage">
        <span className="ov__dots" aria-hidden>
          <i />
          <i />
          <i />
        </span>
        {stage === "searching" ? "Searching the wiki" : "Reading"}
      </div>

      <div className="ov__sources">
        <AnimatePresence>
          {found.map((title, index) => (
            <motion.span
              className="ov__src"
              key={title}
              initial={{ opacity: 0, y: 5 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.32, delay: index * 0.06, ease: [0.16, 1, 0.3, 1] }}
            >
              {title}
            </motion.span>
          ))}
        </AnimatePresence>
      </div>
    </div>
  );
}
