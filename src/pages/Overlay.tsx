import { AnimatePresence, motion, useAnimationControls } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";

import { EASE } from "../components/Motion";
import { api } from "../lib/ipc";
import type {
  AskTurn,
  BackupRecord,
  Comparison,
  ErssSetting,
  ErssStatus,
  Fingerprint,
  SaveEntry,
} from "../lib/types";

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
  /** The answer stops early. Said out loud, because a sentence that ends
      mid-word otherwise reads as the whole answer. */
  cut?: boolean;
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
              return [...rest, { ...last, lane: event.lane, ms: event.ms, cut: event.cut }];
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

              {line.cut && <div className="ov__cut">This answer stops early — ask again for the rest.</div>}

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

        <Snapshot />
        <Password />
        <Match />
        <Picture />

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

/**
 * The upscaler and frame generation, changed without leaving the game.
 *
 * The mod ships its own panel in the ReShade style; this is the same settings
 * in the launcher's, and reachable from the same key as everything else here.
 * Nothing is injected to do it — the mod reads its own configuration file, and
 * most of it takes effect the moment the file changes. The ones that do not say
 * so rather than pretending.
 *
 * Absent entirely when the mod is not installed, which is most people.
 */
function Picture() {
  const [status, setStatus] = useState<ErssStatus | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [needsRestart, setNeedsRestart] = useState<string[]>([]);

  const load = useCallback(async () => {
    try {
      const game = (await api.settingsGet()).selectedGame;
      setStatus(await api.erss(game));
    } catch {
      setStatus(null);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const settings = status?.settings.filter((one) => one.described && one.choices.length > 0) ?? [];
  if (!status?.installed || settings.length === 0) return null;

  const choose = async (setting: ErssSetting, value: string) => {
    setBusy(setting.key);
    try {
      const game = (await api.settingsGet()).selectedGame;
      await api.erssSet(game, setting.key, value);
      if (setting.restart && !needsRestart.includes(setting.title)) {
        setNeedsRestart((had) => [...had, setting.title]);
      }
      await load();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="ov__picture">
      <button
        type="button"
        className="ov__pictureTop"
        onClick={() => setOpen((was) => !was)}
        aria-expanded={open}
      >
        <span>Picture</span>
        <span className="ov__pictureNow">
          {settings[0].choices.find((c) => c.value === settings[0].value)?.label ?? "—"}
        </span>
      </button>

      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            className="ov__pictureBody"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.22, ease: EASE }}
          >
            {settings.map((setting) => (
              <div className="ov__set" key={setting.key}>
                <span className="ov__setName">{setting.title}</span>
                <div className="ov__setRow">
                  {setting.choices.map((choice) => (
                    <button
                      key={choice.value}
                      type="button"
                      className="ov__pick"
                      aria-pressed={choice.value === setting.value}
                      disabled={busy !== null}
                      onClick={() => void choose(setting, choice.value)}
                    >
                      {choice.label}
                    </button>
                  ))}
                </div>
              </div>
            ))}

            {needsRestart.length > 0 && (
              <p className="ov__setNote">
                {needsRestart.join(", ")} {needsRestart.length === 1 ? "takes" : "take"} effect
                the next time the game starts. Everything else here is already live.
              </p>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/**
 * The co-op password, for when somebody asks for it mid-session.
 *
 * A small thing that only makes sense in an overlay: a friend messages "what's
 * the password?" while you are standing in a boss arena, and the alternative is
 * alt-tabbing to the Co-op tab to read eight characters back to them.
 *
 * Read-only on purpose. Changing it is a launch-time decision — the mod reads
 * the file when the game starts — so an editable field here would look like it
 * did something and would not.
 */
function Password() {
  const [password, setPassword] = useState<string | null>(null);
  const [shown, setShown] = useState(false);

  useEffect(() => {
    void (async () => {
      try {
        const game = (await api.settingsGet()).selectedGame;
        const coop = await api.coopRead(game);
        if (!coop.installed) return;
        setPassword(coop.values["PASSWORD.cooppassword"] ?? "");
      } catch {
        setPassword(null);
      }
    })();
  }, []);

  // Nothing to show for a solo player, and an empty password IS solo.
  if (!password) return null;

  return (
    <div className="ov__picture">
      <button
        type="button"
        className="ov__pictureTop"
        onClick={() => setShown((was) => !was)}
        aria-expanded={shown}
      >
        <span>Co-op password</span>
        <span className="ov__pictureNow">{shown ? password : "••••••••"}</span>
      </button>

      <AnimatePresence initial={false}>
        {shown && (
          <motion.div
            className="ov__pictureBody"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.22, ease: EASE }}
          >
            <div className="ov__set">
              <span className="ov__setName">Everyone in the session needs this one</span>
              <div className="ov__setRow">
                <button
                  type="button"
                  className="ov__pick"
                  onClick={() => void navigator.clipboard.writeText(password)}
                >
                  Copy
                </button>
              </div>
            </div>
            <p className="ov__setNote">
              Changing it is a launch-time decision, so it is read-only here — the mod reads the
              file when the game starts.
            </p>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/**
 * Why the two of you cannot connect, answered where it happens.
 *
 * A Seamless session refuses to join when the two installs differ, and the
 * message the game gives says nothing about which mod is the culprit. The
 * launcher can already tell — it fingerprints what is installed and names the
 * differences that matter — but that lived on a screen you had to alt-tab to,
 * which is the wrong place: the moment you need it is while your friend is
 * sitting in a lobby waiting.
 *
 * `block` is the line to paste them. Their line pasted back gives the verdict
 * and, when it differs, the field-by-field reason with `matters` explaining why
 * that particular difference stops a session.
 *
 * Reads files only. Nothing here touches the running game.
 */
function Match() {
  const [mine, setMine] = useState<Fingerprint | null>(null);
  const [open, setOpen] = useState(false);
  const [theirs, setTheirs] = useState("");
  const [verdict, setVerdict] = useState<Comparison | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void (async () => {
      try {
        const game = (await api.settingsGet()).selectedGame;
        const coop = await api.coopRead(game);
        // Solo players are not comparing anything, so this stays out of their way.
        if (!coop.installed) return;
        setMine(await api.matchFingerprint(game));
      } catch {
        setMine(null);
      }
    })();
  }, []);

  if (!mine) return null;

  const compare = async () => {
    if (!theirs.trim()) return;
    setBusy(true);
    try {
      const game = (await api.settingsGet()).selectedGame;
      setVerdict(await api.matchCompare(game, theirs.trim()));
    } catch {
      setVerdict(null);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ov__picture">
      <button
        type="button"
        className="ov__pictureTop"
        onClick={() => setOpen((was) => !was)}
        aria-expanded={open}
      >
        <span>Setup match</span>
        <span className="ov__pictureNow">
          {verdict === null ? "compare" : verdict.verdict === "match" ? "same" : verdict.verdict}
        </span>
      </button>

      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            className="ov__pictureBody"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.22, ease: EASE }}
          >
            <div className="ov__set">
              <span className="ov__setName">Send them yours</span>
              <div className="ov__setRow">
                <button
                  type="button"
                  className="ov__pick"
                  onClick={() => void navigator.clipboard.writeText(mine.block)}
                >
                  Copy mine
                </button>
              </div>
            </div>

            <div className="ov__set">
              <span className="ov__setName">Paste theirs</span>
              <div className="ov__setRow">
                <input
                  className="ov__field"
                  value={theirs}
                  onChange={(event) => setTheirs(event.target.value)}
                  placeholder="their line"
                  spellCheck={false}
                />
                <button
                  type="button"
                  className="ov__pick"
                  onClick={() => void compare()}
                  disabled={busy || !theirs.trim()}
                >
                  {busy ? "Reading…" : "Compare"}
                </button>
              </div>
            </div>

            {verdict?.verdict === "match" && (
              <p className="ov__setNote">Same setup. If it still will not connect, it is not the mods.</p>
            )}

            {verdict?.verdict === "differs" &&
              verdict.differences.map((difference) => (
                <div className="ov__set" key={difference.label}>
                  <span className="ov__setName">{difference.label}</span>
                  <p className="ov__setNote">
                    You: {difference.mine} · Them: {difference.theirs}
                    <br />
                    {difference.matters}
                  </p>
                </div>
              ))}

            {verdict?.verdict === "unreadable" && (
              <p className="ov__setNote">That line could not be read. Ask them to copy it again.</p>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/**
 * One button: snapshot the save, without leaving the game.
 *
 * The overlay could ask questions and change the picture settings, and nothing
 * else — so the honest answer to "how do I back up before this boss" was to
 * alt-tab out, find the Saves tab and click there, which is the moment you are
 * least willing to leave. This is that click, in front of the fog gate.
 *
 * Deliberately not the whole Saves screen. Restoring, moving and converting a
 * character are decisions, and a decision belongs on a screen you can read;
 * taking a snapshot is the one action with no downside and no options, which
 * is what makes it worth a single button over the top of a game.
 */
function Snapshot() {
  const [entry, setEntry] = useState<SaveEntry | null>(null);
  const [last, setLast] = useState<BackupRecord | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [failed, setFailed] = useState(false);
  /** Putting one back asks first. See `Rollback`. */
  const [rolling, setRolling] = useState(false);
  const [putBack, setPutBack] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const game = (await api.settingsGet()).selectedGame;
      const folders = await api.savesDiscover(game);
      // The one the game is actually writing: the newest real save, skipping
      // the launcher's own backup copies.
      const all = folders
        .flatMap((folder) => folder.entries)
        .filter((one) => one.flavour !== "game-backup" && one.modified !== null);
      all.sort((a, b) => (a.modified! < b.modified! ? 1 : -1));
      setEntry(all[0] ?? null);

      const backups = await api.savesBackups(game);
      setLast(backups[0] ?? null);
    } catch {
      setEntry(null);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (!entry) return null;

  const take = async () => {
    setBusy(true);
    setFailed(false);
    try {
      const game = (await api.settingsGet()).selectedGame;
      await api.savesBackup(game, entry.path, "overlay");
      await load();
      setDone(true);
      // Long enough to read over a game, short enough not to sit there.
      setTimeout(() => setDone(false), 2600);
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  };

  const when = last?.created ? new Date(last.created) : null;
  const said = failed
    ? "could not — the file may be open"
    : done
      ? "taken just now"
      : when
        ? `last ${when.toLocaleString(undefined, {
            month: "short",
            day: "numeric",
            hour: "2-digit",
            minute: "2-digit",
          })}`
        : "none yet";

  // Putting a snapshot back, which is the half that was missing.
  //
  // The overlay could TAKE one and not restore one, and the moment a restore
  // is wanted is exactly the moment the launcher window is unreachable: the
  // boss has just killed you and the game is full screen. `savesRestore` was
  // already there; nothing but the button was.
  //
  // It asks first, and the asking is not politeness. This overwrites the file
  // the running game has open, and the game keeps its own copy in memory and
  // writes it back — so a restore while playing is undone the next time it
  // saves, and can leave the two disagreeing. The confirmation says that in
  // as many words rather than hiding it behind "are you sure?".
  const restore = async () => {
    if (!last) return;
    setBusy(true);
    setFailed(false);
    try {
      const game = (await api.settingsGet()).selectedGame;
      await api.savesRestore(game, last.id);
      setPutBack(last.id);
      setRolling(false);
      setTimeout(() => setPutBack(null), 4000);
      await load();
    } catch {
      setFailed(true);
      setRolling(false);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ov__picture">
      <button type="button" className="ov__pictureTop" onClick={() => void take()} disabled={busy}>
        <span>{busy ? "Taking a snapshot…" : "Snapshot the save"}</span>
        <span className="ov__pictureNow">{said}</span>
      </button>

      {last && !rolling && (
        <button
          type="button"
          className="ov__pictureRow"
          onClick={() => setRolling(true)}
          disabled={busy}
        >
          <span>{putBack === last.id ? "Put back" : "Put the last one back"}</span>
        </button>
      )}

      {last && rolling && (
        <div className="ov__pictureAsk">
          <span className="ov__pictureWarn">
            This writes over the save the game has open. Quit to the title screen first, or
            the game will write its own copy back over it.
          </span>
          <div className="ov__pictureAskRow">
            <button
              type="button"
              className="ov__pictureGo"
              onClick={() => void restore()}
              disabled={busy}
            >
              {busy ? "Putting it back…" : "Put it back"}
            </button>
            <button type="button" className="ov__pictureNo" onClick={() => setRolling(false)}>
              Leave it
            </button>
          </div>
        </div>
      )}
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
