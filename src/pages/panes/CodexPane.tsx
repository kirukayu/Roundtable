import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icons";
import { SOFT } from "../../components/Motion";
import { Blank, Card, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import type { CodexHit, CodexResult } from "../../lib/types";

/**
 * Everything in the game, without leaving the launcher.
 *
 * The whole dataset is a few megabytes, so it is downloaded once and searched
 * on disk from then on. That makes it instant, and it keeps working offline —
 * which is the state a lot of modded copies are played in.
 */
export default function CodexPane({ edition }: { edition: string | null }) {
  const toast = useToast();
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<string>("");
  const [result, setResult] = useState<CodexResult | null>(null);
  const [open, setOpen] = useState<CodexHit | null>(null);
  const [busy, setBusy] = useState(false);
  const poll = useRef<number | null>(null);

  const search = useCallback(async () => {
    try {
      setResult(await api.codex(query, kind || undefined, edition));
    } catch {
      /* the codex is a nicety; a failed search should not shout */
    }
  }, [query, kind, edition]);

  // Debounced: the search itself is local and instant, but re-rendering a
  // hundred rows on every keystroke is not free.
  useEffect(() => {
    const timer = window.setTimeout(() => void search(), 140);
    return () => window.clearTimeout(timer);
  }, [search]);

  const download = async () => {
    setBusy(true);
    try {
      await api.codexSync();
      poll.current = window.setInterval(async () => {
        const state = await api.codexState();
        setResult((prev) => (prev ? { ...prev, state } : prev));
        if (!state.syncing) {
          if (poll.current) window.clearInterval(poll.current);
          poll.current = null;
          setBusy(false);
          if (state.error) toast.error("Download failed", state.error);
          else toast.success("Codex ready", state.message);
          await search();
        }
      }, 900);
    } catch (error) {
      setBusy(false);
      toast.error("Could not start", error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => () => {
    if (poll.current) window.clearInterval(poll.current);
  }, []);

  const state = result?.state;

  if (result && result.total === 0) {
    return (
      <Card>
        <Blank
          icon={Icon.Library}
          title="The codex is empty"
          action={
            <button type="button" className="btn btn--solid" onClick={download} disabled={busy}>
              {busy ? <span className="spin" /> : null}
              {busy ? (state?.message ?? "Downloading") : "Download it"}
            </button>
          }
        >
          Weapons, armour, spells, bosses and locations. A few megabytes, fetched
          once, then searched offline.
        </Blank>
      </Card>
    );
  }

  return (
    <div className="col">
      <div className="codex__bar">
        <input
          className="in codex__search"
          placeholder="Search weapons, spells, bosses…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          autoFocus
        />
        <span className="mono w4 codex__count">
          {result ? `${result.hits.length} of ${result.total}` : "…"}
        </span>
      </div>

      <div className="codex__kinds">
        <Filter label="All" active={kind === ""} onClick={() => setKind("")} />
        {result?.kinds.map(([id, label, count]) => (
          <Filter
            key={id}
            label={`${label} ${count}`}
            active={kind === id}
            onClick={() => setKind(kind === id ? "" : id)}
          />
        ))}
      </div>

      {result && result.hits.length === 0 ? (
        <Card>
          <Blank icon={Icon.Search} title="Nothing matches that">
            Try a shorter word, or clear the collection filter.
          </Blank>
        </Card>
      ) : (
        <div className="codex__grid">
          {result?.hits.map((hit) => (
            <button
              key={`${hit.kind}-${hit.id}`}
              type="button"
              className="codex__row"
              onClick={() => setOpen(hit)}
            >
              <span className="codex__thumb">
                {hit.image ? <img src={hit.image} alt="" loading="lazy" /> : null}
              </span>
              <span className="codex__body">
                <span className="codex__name">{hit.name}</span>
                <span className="codex__kind">{hit.kindLabel}</span>
              </span>
            </button>
          ))}
        </div>
      )}

      <AnimatePresence>
        {open && <Detail hit={open} onClose={() => setOpen(null)} />}
      </AnimatePresence>
    </div>
  );
}

function Filter({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className="codex__filter" data-on={active} onClick={onClick}>
      {label}
    </button>
  );
}

function Detail({ hit, onClose }: { hit: CodexHit; onClose: () => void }) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <motion.div
      className="scrim"
      onClick={onClose}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.3 }}
    >
      <motion.div
        className="modal"
        onClick={(event) => event.stopPropagation()}
        initial={{ opacity: 0, y: 18, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 10, scale: 0.99 }}
        transition={{ duration: 0.45, ease: SOFT }}
      >
        <div className="modal__h">
          <h2>{hit.name}</h2>
          <span className="chip">{hit.kindLabel}</span>
        </div>

        <div className="modal__b">
          {hit.image && (
            <img
              src={hit.image}
              alt=""
              style={{ maxHeight: 180, display: "block", margin: "0 auto var(--s5)" }}
            />
          )}

          {hit.description && (
            <p className="w2" style={{ fontSize: "var(--t-sm)", lineHeight: 1.8 }}>
              {hit.description}
            </p>
          )}

          {hit.facts.length > 0 && (
            <>
              <hr className="hr" />
              <div className="col2">
                {hit.facts.map((fact) => (
                  <div className="between" key={fact.label}>
                    <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
                      {fact.label}
                    </span>
                    <span className="mono" style={{ fontSize: "var(--t-xs)", textAlign: "right" }}>
                      {fact.value}
                    </span>
                  </div>
                ))}
              </div>
            </>
          )}
        </div>

        <div className="modal__f">
          <button type="button" className="btn btn--ghost" onClick={onClose}>
            Close
          </button>
          {/*
            A plain anchor, not a call into the launcher. This page is already
            in a browser, and handing a URL to the desktop side would route it
            through `explorer`, which opens Documents rather than the link.
          */}
          <a
            className="btn btn--solid"
            href={hit.wiki}
            target="_blank"
            rel="noreferrer noopener"
          >
            Open wiki
          </a>
        </div>
      </motion.div>
    </motion.div>
  );
}
