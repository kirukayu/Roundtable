import { useEffect, useMemo, useRef, useState } from "react";

import { Icon } from "./Icons";

export interface Command {
  id: string;
  label: string;
  group: string;
  hint?: string;
  glyph?: (p: { size?: number }) => React.ReactNode;
  run: () => void | Promise<void>;
  /** Extra words that should match this command without being displayed. */
  keywords?: string;
}

/**
 * Fuzzy subsequence match: "svbk" finds "Save backup".
 *
 * Returns a score so exact prefixes rank above scattered matches, or null when
 * the needle does not appear at all.
 */
function score(haystack: string, needle: string): number | null {
  if (!needle) return 0;
  const text = haystack.toLowerCase();
  const query = needle.toLowerCase();

  if (text.startsWith(query)) return 1000;
  const direct = text.indexOf(query);
  if (direct >= 0) return 700 - direct;

  let cursor = 0;
  let hits = 0;
  let streak = 0;
  let best = 0;
  for (const character of query) {
    const at = text.indexOf(character, cursor);
    if (at < 0) return null;
    // Consecutive characters are worth more than scattered ones.
    streak = at === cursor ? streak + 1 : 0;
    best = Math.max(best, streak);
    hits += 1;
    cursor = at + 1;
  }
  return hits * 8 + best * 12 - cursor;
}

export function CommandPalette({
  commands,
  onClose,
}: {
  commands: Command[];
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    input.current?.focus();
  }, []);

  const matches = useMemo(() => {
    const ranked = commands
      .map((command) => {
        const target = `${command.group} ${command.label} ${command.keywords ?? ""}`;
        const value = score(target, query);
        return value === null ? null : { command, value };
      })
      .filter((entry): entry is { command: Command; value: number } => entry !== null);

    ranked.sort((a, b) => b.value - a.value);
    return ranked.slice(0, 40).map((entry) => entry.command);
  }, [commands, query]);

  useEffect(() => {
    setActive(0);
  }, [query]);

  // Keep the highlighted row in view while arrowing through a long list.
  useEffect(() => {
    const row = listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    row?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const runActive = () => {
    const command = matches[active];
    if (!command) return;
    onClose();
    void command.run();
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((current) => (current + 1) % Math.max(matches.length, 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((current) => (current - 1 + matches.length) % Math.max(matches.length, 1));
    } else if (event.key === "Enter") {
      event.preventDefault();
      runActive();
    } else if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  };

  let lastGroup = "";

  return (
    <div
      className="scrim"
      style={{ alignItems: "flex-start", paddingTop: "12vh" }}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="pal" role="dialog" aria-modal="true" aria-label="Command palette">
        <input
          ref={input}
          className="pal__input"
          placeholder="Type a command…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={onKeyDown}
          aria-label="Command"
          aria-autocomplete="list"
        />
        <div className="pal__list" ref={listRef} role="listbox">
          {matches.length === 0 ? (
            <div className="blank">
              <div className="blank__title">Nothing matches</div>
            </div>
          ) : (
            matches.map((command, index) => {
              const showGroup = command.group !== lastGroup;
              lastGroup = command.group;
              const Glyph = command.glyph ?? Icon.Chevron;
              return (
                <div key={command.id}>
                  {showGroup && <div className="pal__group eyebrow">{command.group}</div>}
                  <button
                    type="button"
                    role="option"
                    aria-selected={index === active}
                    data-index={index}
                    data-active={index === active}
                    className="pal__item"
                    onMouseEnter={() => setActive(index)}
                    onClick={() => {
                      onClose();
                      void command.run();
                    }}
                  >
                    <Glyph size={15} />
                    {command.label}
                    {command.hint && <span className="pal__hint">{command.hint}</span>}
                  </button>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}
