import { motion, useScroll, useTransform, type Variants } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";

import { EASE, Rise, SOFT, Words, shelf, tile } from "../components/Motion";
import { usePointer } from "../lib/motion";
import { glideTo } from "../lib/smooth";
import { useApp } from "../lib/store";
import type { GameId, GameInfo } from "../lib/types";

/**
 * The way in.
 *
 * A line of type over fog, the catalogue as a shelf, three statements, and
 * nothing else. Everything below the fold waits and surfaces when it is reached;
 * the only things that move unprompted are the fog and the title ribbon, and
 * both are slow enough that you notice them after looking away rather than while
 * looking at them.
 */
export default function Landing({ onOpen }: { onOpen: (id: GameId) => void }) {
  const { games, installed } = useApp();
  const grid = useRef<HTMLElement>(null);

  const { feature, rest, exclusives } = useMemo(() => {
    const playable = games.filter((g) => g.playable);
    // Newest first, with the flagship pulled out to lead the shelf.
    const ordered = [...playable].sort((a, b) => b.year - a.year);
    const lead = ordered.find((g) => g.id === "elden-ring") ?? ordered[0] ?? null;
    return {
      feature: lead,
      rest: ordered.filter((g) => g.id !== lead?.id),
      exclusives: games.filter((g) => !g.playable),
    };
  }, [games]);

  const playableCount = games.filter((g) => g.playable).length;
  const installedCount = games.filter((g) => g.playable && installed.has(g.id)).length;

  const toGrid = () => {
    if (grid.current) glideTo(grid.current, -40);
  };

  return (
    <>
      <Hero
        titles={games.map((g) => g.short)}
        installedCount={installedCount}
        total={playableCount}
        onChoose={toGrid}
      />

      <section className="section" ref={grid} id="games">
        <Rise className="section__head">
          <div className="section__label">The catalogue</div>
          <h2 className="section__title">
            <Words text={`${spell(games.length)} titles`} />
          </h2>
          <p className="section__note">
            {playableCount} of them run on this machine. The rest never left
            PlayStation, and the shelf would be wrong without them.
          </p>
        </Rise>

        <motion.div
          className="shelf"
          variants={shelf}
          initial="hidden"
          whileInView="show"
          viewport={{ once: true, amount: 0.08 }}
        >
          {feature && (
            <Poster
              feature
              game={feature}
              index={1}
              installed={installed.has(feature.id)}
              onOpen={() => onOpen(feature.id)}
            />
          )}
          {rest.map((game, index) => (
            <Poster
              key={game.id}
              game={game}
              index={index + 2}
              installed={installed.has(game.id)}
              onOpen={() => onOpen(game.id)}
            />
          ))}
        </motion.div>
      </section>

      {exclusives.length > 0 && (
        <section className="section section--tight">
          <Rise className="section__head">
            <div className="section__label">Not on this platform</div>
            <p className="section__note">
              Nothing to launch and nothing to patch. They are here because the
              catalogue is the catalogue.
            </p>
          </Rise>

          <motion.div
            className="shelf shelf--rest"
            variants={shelf}
            initial="hidden"
            whileInView="show"
            viewport={{ once: true, amount: 0.25 }}
          >
            {exclusives.map((game, index) => (
              <Poster
                key={game.id}
                game={game}
                index={playableCount + index + 1}
                installed={false}
              />
            ))}
          </motion.div>
        </section>
      )}

      <Creed />

      <Rise className="foot" y={24} blur={4}>
        <span>Roundtable</span>
        <span>No account. No telemetry. Nothing leaves this machine.</span>
      </Rise>
    </>
  );
}

/* ── Hero ────────────────────────────────────────────────────────── */

function Hero({
  titles,
  installedCount,
  total,
  onChoose,
}: {
  titles: string[];
  installedCount: number;
  total: number;
  onChoose: () => void;
}) {
  const section = useRef<HTMLElement>(null);

  // The hero recedes as the page is pulled up: it sinks, shrinks a little and
  // goes out of focus, so the shelf reads as arriving over the top of it rather
  // than pushing it off screen.
  const { scrollYProgress } = useScroll({
    target: section,
    offset: ["start start", "end start"],
  });

  const y = useTransform(scrollYProgress, [0, 1], [0, 140]);
  const scale = useTransform(scrollYProgress, [0, 1], [1, 0.93]);
  const opacity = useTransform(scrollYProgress, [0, 0.75], [1, 0]);
  const blur = useTransform(scrollYProgress, [0, 1], ["blur(0px)", "blur(9px)"]);

  return (
    <section className="hero" ref={section}>
      <motion.div className="hero__inner" style={{ y, scale, opacity, filter: blur }}>
        <Words text="FromSoftware" className="hero__eyebrow" delay={0.25} />

        <h1 className="hero__title">
          <Words text="Roundtable" delay={0.5} />
          <em>
            <Words text="Launcher" delay={0.78} />
          </em>
        </h1>

        <motion.p
          className="hero__sub"
          initial={{ opacity: 0, y: 22, filter: "blur(8px)" }}
          animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
          transition={{ duration: 1.4, ease: EASE, delay: 1.35 }}
        >
          Mods, co-op and saves for every FromSoftware title, on one machine,
          without a single file leaving it.
        </motion.p>

        <motion.div
          className="hero__cta"
          initial={{ opacity: 0, y: 18 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 1.2, ease: EASE, delay: 1.75 }}
        >
          <motion.button
            type="button"
            className="btn btn--solid btn--lg"
            onClick={onChoose}
            whileHover={{ scale: 1.035 }}
            whileTap={{ scale: 0.975 }}
            transition={{ duration: 0.5, ease: SOFT }}
          >
            Choose a game
          </motion.button>
        </motion.div>
      </motion.div>

      {titles.length > 0 && <Ribbon titles={titles} />}

      {/* The wrapper carries the scroll-linked fade; the button carries its own
          entrance, so the two do not both try to own `opacity`. */}
      <motion.div className="hero__foot" style={{ opacity }}>
        <motion.button
          type="button"
          className="hero__scroll"
          onClick={onChoose}
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 1.6, ease: EASE, delay: 2.5 }}
        >
          <span>{installedCount > 0 ? `${installedCount} of ${total} installed` : "Scroll"}</span>
          <span className="hero__tick" />
        </motion.button>
      </motion.div>
    </section>
  );
}

/**
 * A ribbon of every title, drifting right forever.
 *
 * Two identical runs sit end to end and the pair travels from minus half its
 * width back to zero, which is what makes the loop seamless while the motion
 * reads left to right.
 */
function Ribbon({ titles }: { titles: string[] }) {
  const run = [...titles, ...titles];
  return (
    <motion.div
      className="ribbon"
      aria-hidden="true"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 2, ease: EASE, delay: 2.2 }}
    >
      <div className="ribbon__track">
        {run.map((title, index) => (
          <span className="ribbon__item" key={`${title}-${index}`}>
            {title}
            <i className="ribbon__sep" />
          </span>
        ))}
      </div>
    </motion.div>
  );
}

/* ── Poster ──────────────────────────────────────────────────────── */

/**
 * The cover, at rest and on hover.
 *
 * The hover is deliberately not a switch. Light washes over the art in one
 * pass — bright for a moment, then settling half a step down — so the card
 * catches the light rather than turning on.
 */
const art: Variants = {
  rest: {
    filter: "grayscale(1) brightness(0.6) contrast(1.06)",
    scale: 1.02,
    transition: { duration: 1.4, ease: SOFT },
  },
  hover: {
    // Light, not colour. The peak lifts the exposure and lets a trace of the
    // original palette through; it never becomes a colour image, because the
    // rest of the page never does either.
    filter: [
      "grayscale(1) brightness(0.6) contrast(1.06)",
      "grayscale(0.55) brightness(1.22) contrast(0.98)",
      "grayscale(0.86) brightness(0.94) contrast(1.02)",
    ],
    scale: 1.08,
    transition: {
      filter: { duration: 1.35, times: [0, 0.26, 1], ease: SOFT },
      scale: { duration: 2.2, ease: EASE },
    },
  },
};

const sweep: Variants = {
  rest: { x: "-130%", opacity: 0, transition: { duration: 0.01 } },
  hover: {
    x: ["-130%", "130%"],
    opacity: [0, 0.9, 0],
    transition: { duration: 1.5, ease: SOFT },
  },
};

const caption: Variants = {
  rest: { y: 0, transition: { duration: 0.9, ease: SOFT } },
  hover: { y: -6, transition: { duration: 0.9, ease: SOFT } },
};

function Poster({
  game,
  index,
  installed,
  feature = false,
  onOpen,
}: {
  game: GameInfo;
  index: number;
  installed: boolean;
  feature?: boolean;
  onOpen?: () => void;
}) {
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const pointer = usePointer<HTMLButtonElement>();

  useEffect(() => {
    setLoaded(false);
    setFailed(false);
  }, [game.id]);

  // The feature card is wide, so it wants the landscape key art; the shelf
  // cards want the portrait cover.
  const source = feature ? (game.heroUrl ?? game.coverUrl) : game.coverUrl;
  const plate = !source || failed;

  return (
    <motion.div
      variants={tile}
      className={`cell${feature ? " cell--feature" : ""}`}
    >
      <motion.button
        ref={pointer}
        type="button"
        className={`poster${plate ? " poster--plate" : ""}${game.playable ? "" : " poster--dim"}`}
        onClick={onOpen}
        disabled={!game.playable}
        aria-label={game.name}
        initial="rest"
        animate="rest"
        whileHover={game.playable ? "hover" : "rest"}
        whileFocus={game.playable ? "hover" : "rest"}
        whileTap={game.playable ? { scale: 0.982 } : undefined}
        transition={{ duration: 0.6, ease: SOFT }}
      >
        <div className="poster__frame">
          {plate ? (
            <span className="poster__glyph">{initials(game.short)}</span>
          ) : (
            // No `animate` prop here on purpose: setting one would cut this
            // element off from the hover variants the button propagates. The
            // load fade is a plain CSS transition, which nothing else touches.
            <motion.img
              layoutId={`art-${game.id}`}
              src={source ?? ""}
              alt=""
              loading="lazy"
              variants={art}
              onLoad={() => setLoaded(true)}
              onError={() => setFailed(true)}
              style={{
                opacity: loaded ? 1 : 0,
                transition: "opacity 1400ms cubic-bezier(0.22, 0.61, 0.36, 1)",
              }}
            />
          )}

          <span className="poster__wash" />
          <span className="sheen" />
          <motion.span className="poster__sweep" variants={sweep} />
          <span className="poster__edge" />

          <span className="poster__no">{String(index).padStart(2, "0")}</span>

          {game.playable && (
            <span className="poster__mark">{installed ? "Installed" : "Locate"}</span>
          )}
        </div>

        {/*
          The caption sits under the frame, not on it. Every one of these covers
          already has the game's name set into the artwork, and a second title
          laid over the first is the one thing a shelf like this cannot survive.
        */}
        <motion.span className="poster__cap" variants={caption}>
          <span className="poster__name">{game.short}</span>
          <span className="poster__meta">
            <span className="poster__year">{game.year}</span>
            <i className="poster__dot" />
            <span className="poster__note">{game.note}</span>
          </span>
        </motion.span>
      </motion.button>
    </motion.div>
  );
}

/* ── Creed ───────────────────────────────────────────────────────── */

const CREED = [
  {
    no: "01",
    title: "Mods",
    body:
      "Install an archive, order it, see what collides. A full overhaul and Seamless Co-op in the same session, wired correctly.",
  },
  {
    no: "02",
    title: "Co-op",
    body:
      "Passwords, session settings and the loader entry that makes the two find each other. Written for you, before the game starts.",
  },
  {
    no: "03",
    title: "Saves",
    body:
      "Move a character between a cracked copy and a licensed one. Identifiers rebound, checksums recomputed, the old file kept.",
  },
] as const;

function Creed() {
  return (
    <section className="section creed">
      <motion.div
        className="creed__grid"
        variants={shelf}
        initial="hidden"
        whileInView="show"
        viewport={{ once: true, amount: 0.3 }}
      >
        {CREED.map((entry) => (
          <motion.article className="creed__item" key={entry.no} variants={tile}>
            <div className="creed__no">{entry.no}</div>
            <h3 className="creed__t">
              <Words text={entry.title} />
            </h3>
            <p className="creed__b">{entry.body}</p>
          </motion.article>
        ))}
      </motion.div>
    </section>
  );
}

/* ── Helpers ─────────────────────────────────────────────────────── */

/** Two letters for the plate, so it reads as a mark rather than a blank. */
function initials(name: string): string {
  const words = name.replace(/[^\w\s]/g, "").split(/\s+/).filter(Boolean);
  if (words.length === 0) return "??";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[words.length - 1][0]).toUpperCase();
}

/** A heading that reads "Nine titles" rather than "9 titles". */
function spell(count: number): string {
  const words = [
    "No", "One", "Two", "Three", "Four", "Five",
    "Six", "Seven", "Eight", "Nine", "Ten",
  ];
  return words[count] ?? String(count);
}
