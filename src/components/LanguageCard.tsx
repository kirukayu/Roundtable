import { useCallback, useEffect, useState } from "react";

import { Icon } from "./Icons";
import { Card, Chip, useToast } from "./ui";
import { api } from "../lib/ipc";
import type { EditionText, GameId, LanguageStatus } from "../lib/types";

/**
 * The game's language, on a copy that has no Steam to ask.
 *
 * Steam normally decides this. A repack emulates Steam, so an ini decides it
 * instead — and repacks ship two or three emulators while setting the language
 * in only one, which is how a Russian repack starts in English. Writing all of
 * them is the whole fix.
 */
export function LanguageCard({ game }: { game: GameId }) {
  const toast = useToast();
  const [status, setStatus] = useState<LanguageStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setStatus(await api.language(game));
    } catch {
      setStatus(null);
    }
  }, [game]);

  useEffect(() => {
    void load();
  }, [load]);

  // Nothing to configure on a Steam copy: Steam owns the setting there.
  if (!status || status.files.length === 0) return null;

  const apply = async (language: string) => {
    setBusy(true);
    try {
      const written = await api.languageSet(game, language);
      toast.success(
        "Language set",
        written.length === 1 ? "1 config written" : `${written.length} configs written`,
      );
      await load();
    } catch (error) {
      toast.error("Could not set", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const label = status.options.find(([id]) => id === status.current)?.[1];
  const commented = status.files.filter((f) => f.disabled && !f.value);

  return (
    <Card
      title="Language"
      action={
        status.current ? <Chip>{label ?? status.current}</Chip> : <Chip tone="warn">Not set</Chip>
      }
    >
      {status.conflict && (
        <div className="note note--warn" style={{ marginBottom: "var(--s4)" }}>
          <Icon.Warning size={15} />
          <div className="note__b">
            Two emulator configs disagree, so one of them is being ignored. Pick a
            language below to write both.
          </div>
        </div>
      )}

      {!status.current && commented.length > 0 && (
        <div className="note" style={{ marginBottom: "var(--s4)" }}>
          <Icon.Info size={15} />
          <div className="note__b">
            The line is there but commented out in{" "}
            <span className="mono">{commented.map((f) => f.file).join(", ")}</span>, so
            the game falls back to English.
          </div>
        </div>
      )}

      <div className="codex__kinds">
        {status.options.map(([id, name]) => (
          <button
            key={id}
            type="button"
            className="codex__filter"
            data-on={status.current === id}
            disabled={busy}
            onClick={() => void apply(id)}
          >
            {name}
          </button>
        ))}
      </div>

      {status.editions.map((edition) => (
        <EditionTextRow
          key={edition.edition}
          game={game}
          language={status.current ?? "english"}
          languageName={label ?? status.current ?? "this language"}
          text={edition}
          onDone={load}
        />
      ))}

      <hr className="hr" />

      <div className="col2">
        {status.files.map((file) => (
          <div className="between" key={file.path}>
            <span className="mono w3" style={{ fontSize: "var(--t-xs)" }}>
              {file.file}
            </span>
            <span className="mono w4" style={{ fontSize: "var(--t-xs)" }}>
              {file.value ?? (file.disabled ? "commented out" : "not set")}
            </span>
          </div>
        ))}
      </div>

      <p className="w4" style={{ fontSize: "var(--t-xs)", marginTop: "var(--s3)" }}>
        Restart the game for this to take effect.
        {status.selector && " This copy also ships its own language picker."}
      </p>
    </Card>
  );
}

/**
 * A total conversion's own text.
 *
 * Setting the game to Russian does nothing for these: The Convergence ships one
 * English archive copied into all fourteen locale folders, so every item and
 * menu reads English on a Russian game. Roundtable carries the translation, so
 * fixing it is a button rather than an account, a download and a folder.
 */
function EditionTextRow({
  game,
  language,
  languageName,
  text,
  onDone,
}: {
  game: GameId;
  language: string;
  languageName: string;
  text: EditionText;
  onDone: () => Promise<void>;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);

  // Nothing to say when the text already reads in the right language and there
  // is no way back to the original.
  if (text.translated && !text.revertible) return null;

  const name = text.edition === "convergence" ? "The Convergence" : text.edition;

  const run = async (revert: boolean) => {
    setBusy(true);
    try {
      const files = revert
        ? await api.editionTextRevert(game, text.edition, language)
        : await api.editionTextInstall(game, text.edition, language);
      toast.success(
        revert ? "Original text restored" : "Translation installed",
        `${files.length} file${files.length === 1 ? "" : "s"} in ${text.locale}`,
      );
      await onDone();
    } catch (error) {
      toast.error("Could not apply", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  if (text.translated) {
    return (
      <div className="between" style={{ marginTop: "var(--s3)" }}>
        <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
          {name} is translated
        </span>
        <button
          type="button"
          className="btn btn--sm"
          disabled={busy}
          onClick={() => void run(true)}
        >
          {busy ? <span className="spin" /> : null}
          Restore original text
        </button>
      </div>
    );
  }

  return (
    <div className="note note--warn" style={{ marginTop: "var(--s4)" }}>
      <Icon.Warning size={15} />
      <div style={{ flex: 1 }}>
        <div className="note__t">
          {name} has no {languageName} text
        </div>
        <div className="note__b">
          The mod replaces every item and menu with its own English text, so the
          game reads English even though it is set to {languageName}.
          {text.bundled && (
            <>
              {" "}
              Roundtable carries the translation ({text.bundled.version}) by{" "}
              {text.bundled.author}.
            </>
          )}
        </div>
        {text.bundled && (
          <button
            type="button"
            className="btn btn--solid btn--sm"
            style={{ marginTop: "var(--s3)" }}
            disabled={busy}
            onClick={() => void run(false)}
          >
            {busy ? <span className="spin" /> : null}
            Install {languageName}
          </button>
        )}
      </div>
    </div>
  );
}
