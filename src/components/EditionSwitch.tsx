import { motion } from "motion/react";

import { Icon } from "./Icons";
import { SOFT } from "./Motion";
import type { EditionStatus, GameInfo } from "../lib/types";

/**
 * Switching between a game and a total conversion of it.
 *
 * A line of text, not a control. There are only ever two states and the sentence
 * says which one you are leaving and which you are going to, so a switch with
 * artwork on it was three times the size for less meaning.
 *
 * It sits above the title because it changes what the title says.
 */
export function EditionSwitch({
  game,
  editions,
  active,
  onSelect,
}: {
  game: GameInfo;
  editions: EditionStatus[];
  /** Null means the game itself. */
  active: string | null;
  onSelect: (id: string | null) => void;
}) {
  if (editions.length === 0) return null;

  // With one conversion this is a toggle. With more it becomes a short list,
  // minus wherever you already are.
  const options: { id: string | null; label: string; note?: string }[] = [
    ...(active === null
      ? []
      : [{ id: null, label: `Switch to ${game.short}` }]),
    ...editions
      .filter((entry) => entry.spec.id !== active)
      .map((entry) => ({
        id: entry.spec.id as string | null,
        label: `Switch to ${entry.spec.short}`,
        note: entry.install ? undefined : "not installed",
      })),
  ];

  return (
    <div className="swap">
      {options.map((option) => (
        <motion.button
          key={option.id ?? "base"}
          type="button"
          className="swap__go"
          onClick={() => onSelect(option.id)}
          whileHover={{ x: 3 }}
          transition={{ duration: 0.4, ease: SOFT }}
        >
          <Icon.Swap size={13} />
          <span>{option.label}</span>
          {option.note && <span className="swap__note">{option.note}</span>}
        </motion.button>
      ))}
    </div>
  );
}
