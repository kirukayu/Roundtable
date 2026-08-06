import { motion, type Transition, type Variants } from "motion/react";
import type { ReactNode } from "react";

/**
 * The motion vocabulary.
 *
 * Two curves and one rule: nothing appears, everything arrives. Text rises out
 * of a mask a word at a time, blocks surface out of blur, and every duration is
 * long enough that you register the movement rather than the jump.
 */
export const EASE = [0.16, 1, 0.3, 1] as const;
export const SOFT = [0.22, 0.61, 0.36, 1] as const;

export const GLIDE: Transition = { duration: 1.1, ease: EASE };

/* ── Words ───────────────────────────────────────────────────────── */

const line: Variants = {
  hidden: {},
  show: (delay: number = 0) => ({
    transition: { staggerChildren: 0.055, delayChildren: delay },
  }),
};

const word: Variants = {
  hidden: { y: "110%", opacity: 0 },
  show: { y: "0%", opacity: 1, transition: { duration: 1.15, ease: EASE } },
};

/**
 * A line of text that climbs into view one word at a time.
 *
 * Each word sits inside its own clipping span, so what you see is the word
 * rising past an edge rather than sliding across the page. The non-breaking
 * space matters: inline-block words lose their gaps without it.
 */
export function Words({
  text,
  className,
  delay = 0,
  once = true,
  amount = 0.6,
}: {
  text: string;
  className?: string;
  delay?: number;
  once?: boolean;
  amount?: number;
}) {
  return (
    <motion.span
      className={className}
      variants={line}
      custom={delay}
      initial="hidden"
      whileInView="show"
      viewport={{ once, amount }}
    >
      {text.split(" ").map((piece, index) => (
        <span className="wordmask" key={`${piece}-${index}`}>
          <motion.span className="wordmask__w" variants={word}>
            {piece}
            {" "}
          </motion.span>
        </span>
      ))}
    </motion.span>
  );
}

/* ── Blocks ──────────────────────────────────────────────────────── */

/** A block that surfaces out of the fog when it is scrolled to. */
export function Rise({
  children,
  className,
  delay = 0,
  y = 42,
  blur = 10,
  once = true,
  amount = 0.15,
  ...rest
}: {
  children: ReactNode;
  className?: string;
  delay?: number;
  y?: number;
  blur?: number;
  once?: boolean;
  amount?: number;
  id?: string;
}) {
  return (
    <motion.div
      className={className}
      initial={{ opacity: 0, y, filter: `blur(${blur}px)` }}
      whileInView={{ opacity: 1, y: 0, filter: "blur(0px)" }}
      viewport={{ once, amount }}
      transition={{ duration: 1.2, ease: EASE, delay }}
      {...rest}
    >
      {children}
    </motion.div>
  );
}

/** A container whose children arrive in sequence. */
export const shelf: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.09, delayChildren: 0.1 } },
};

export const tile: Variants = {
  hidden: { opacity: 0, y: 46, scale: 0.965, filter: "blur(8px)" },
  show: {
    opacity: 1,
    y: 0,
    scale: 1,
    filter: "blur(0px)",
    transition: { duration: 1.25, ease: EASE },
  },
};
