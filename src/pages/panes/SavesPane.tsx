import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Icon } from "../../components/Icons";
import { EASE, SOFT } from "../../components/Motion";
import { Blank, Card, Chip, Confirm, Modal, Skeleton, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import { bytes, exact, playtime, when } from "../../lib/format";
import type {
  BackupRecord,
  GameId,
  SaveEntry,
  SaveFolder,
  SaveSummary,
  SlotSummary,
  SteamAccount,
} from "../../lib/types";

/**
 * Saves, as characters.
 *
 * Nobody thinks in `.sl2` files. They think about the level 90 mage they want on
 * the co-op save, so that is the unit here: every character in every file, in
 * one grid, each with the things you can do to it. The files themselves become a
 * filter along the top rather than a list you have to navigate first.
 */

/**
 * What a save file is, in words.
 *
 * "Co-op" twice tells you nothing when both a vanilla and a Convergence co-op
 * save exist side by side, and they do — the mod writes `ER0000.cnv.co2`.
 */
function saveLabel(entry: SaveEntry): string {
  const name = entry.fileName.toLowerCase();
  if (name.includes(".cnv")) return "Convergence";
  if (entry.flavour === "seamless-coop") return "Co-op";
  return "Solo";
}

/** A character, with enough about its file to act on it. */
interface Character {
  entry: SaveFolder["entries"][number];
  folder: SaveFolder;
  slot: SlotSummary;
  summary: SaveSummary;
}

export default function SavesPane({ gameId }: { gameId: GameId }) {
  const toast = useToast();
  const [folders, setFolders] = useState<SaveFolder[] | null>(null);
  const [backups, setBackups] = useState<BackupRecord[]>([]);
  const [accounts, setAccounts] = useState<SteamAccount[]>([]);
  const [summaries, setSummaries] = useState<Record<string, SaveSummary | null>>({});
  const [filter, setFilter] = useState<string>("");
  const [move, setMove] = useState<Character | null>(null);
  const [convert, setConvert] = useState<SaveEntry | null>(null);
  const [showBackups, setShowBackups] = useState(false);
  const [restoring, setRestoring] = useState<BackupRecord | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    const [found, stored, steam] = await Promise.all([
      api.savesDiscover(gameId),
      api.savesBackups(gameId),
      api.steamAccounts(),
    ]);
    setFolders(found);
    setBackups(stored);
    setAccounts(steam);

    // Every container is read up front. There are rarely more than a handful,
    // and reading them lazily is what made this feel like a file browser.
    const files = found.flatMap((f) => f.entries).filter((e) => e.flavour !== "game-backup");
    const read = await Promise.all(
      files.map(async (entry) => {
        try {
          return [entry.path, await api.savesInspect(entry.path)] as const;
        } catch {
          return [entry.path, null] as const;
        }
      }),
    );
    setSummaries(Object.fromEntries(read));
  }, [gameId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const files = useMemo(
    () => folders?.flatMap((f) => f.entries).filter((e) => e.flavour !== "game-backup") ?? [],
    [folders],
  );

  const characters = useMemo<Character[]>(() => {
    const out: Character[] = [];
    for (const folder of folders ?? []) {
      for (const entry of folder.entries) {
        if (entry.flavour === "game-backup") continue;
        const summary = summaries[entry.path];
        if (!summary) continue;
        for (const slot of summary.slots) {
          if (slot.active) out.push({ entry, folder, slot, summary });
        }
      }
    }
    return out.sort((a, b) => b.slot.level - a.slot.level);
  }, [folders, summaries]);

  const shown = filter
    ? characters.filter((character) => character.entry.path === filter)
    : characters;

  const snapshot = async (entry: SaveEntry) => {
    setBusy(true);
    try {
      await toast.run("Snapshot taken", () => api.savesBackup(gameId, entry.path, "manual"));
      setBackups(await api.savesBackups(gameId));
    } finally {
      setBusy(false);
    }
  };

  if (folders === null) {
    return (
      <div className="g3">
        <Card><Skeleton variant="line" count={3} /></Card>
        <Card><Skeleton variant="line" count={3} /></Card>
        <Card><Skeleton variant="line" count={3} /></Card>
      </div>
    );
  }

  if (files.length === 0) {
    return (
      <Card>
        <Blank icon={Icon.Save} title="No saves yet">
          Start the game once so it creates a character.
        </Blank>
      </Card>
    );
  }

  return (
    <div className="col">
      <div className="sv__bar">
        <div className="sv__files">
          <button
            type="button"
            className="codex__filter"
            data-on={filter === ""}
            onClick={() => setFilter("")}
          >
            All {characters.length}
          </button>
          {files.map((entry) => {
            const count = summaries[entry.path]?.slots.filter((s) => s.active).length ?? 0;
            return (
              <button
                key={entry.path}
                type="button"
                className="codex__filter"
                data-on={filter === entry.path}
                onClick={() => setFilter(filter === entry.path ? "" : entry.path)}
                title={entry.path}
              >
                {saveLabel(entry)} · {count}
              </button>
            );
          })}
        </div>

        <div className="row" style={{ gap: "var(--s2)" }}>
          <button type="button" className="btn btn--sm" onClick={() => setShowBackups(true)}>
            <Icon.Clock size={13} />
            Snapshots {backups.length > 0 ? backups.length : ""}
          </button>
        </div>
      </div>

      {shown.length === 0 ? (
        <Card>
          <Blank
            icon={Icon.Users}
            title={filter ? "That save is empty" : "No characters yet"}
          >
            {filter
              ? "All ten slots in this file are free."
              : "The save files exist but nobody lives in them. Start the game and create a character."}
          </Blank>
        </Card>
      ) : (
        <motion.div
          className="sv__grid"
          initial="hidden"
          animate="show"
          variants={{ show: { transition: { staggerChildren: 0.05 } } }}
        >
          <AnimatePresence mode="popLayout">
            {shown.map((character) => (
              <CharacterCard
                key={`${character.entry.path}-${character.slot.index}`}
                character={character}
                accounts={accounts}
                busy={busy}
                onMove={() => setMove(character)}
                onConvert={() => setConvert(character.entry)}
                onSnapshot={() => void snapshot(character.entry)}
              />
            ))}
          </AnimatePresence>
        </motion.div>
      )}

      {move && (
        <MoveModal
          gameId={gameId}
          character={move}
          files={files}
          summaries={summaries}
          onClose={() => setMove(null)}
          onDone={refresh}
        />
      )}

      {convert && (
        <ConvertModal
          gameId={gameId}
          entry={convert}
          accounts={accounts}
          onClose={() => setConvert(null)}
          onDone={refresh}
        />
      )}

      {showBackups && (
        <Modal title="Snapshots" wide onClose={() => setShowBackups(false)}>
          {backups.length === 0 ? (
            <Blank icon={Icon.Clock} title="No snapshots yet">
              One is taken before every launch and before anything that writes.
            </Blank>
          ) : (
            <div className="rows">
              {backups.map((backup) => (
                <div key={backup.id} className="rw">
                  <div className="grow" style={{ minWidth: 0 }}>
                    <div className="row" style={{ gap: "var(--s2)" }}>
                      <span className="rw__t">{backup.label}</span>
                      {backup.automatic ? <Chip>auto</Chip> : <Chip tone="solid">manual</Chip>}
                    </div>
                    <div className="rw__s">
                      {exact(backup.created)} · {bytes(backup.sizeBytes)}
                    </div>
                    {backup.characters.length > 0 && (
                      <div className="rw__s truncate">{backup.characters.join(" · ")}</div>
                    )}
                  </div>
                  <button type="button" className="btn btn--sm" onClick={() => setRestoring(backup)}>
                    Restore
                  </button>
                  <button
                    type="button"
                    className="btn btn--ghost btn--sm btn--icon btn--bad"
                    aria-label="Delete"
                    onClick={async () => {
                      await api.savesDeleteBackup(gameId, backup.id);
                      setBackups(await api.savesBackups(gameId));
                    }}
                  >
                    <Icon.Trash size={13} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </Modal>
      )}

      {restoring && (
        <Confirm
          title="Restore this snapshot?"
          confirmLabel="Restore"
          body={
            <p className="fld__h">
              Whatever is there now is snapshotted first, so this is reversible.
            </p>
          }
          onCancel={() => setRestoring(null)}
          onConfirm={async () => {
            await toast.run("Save restored", () => api.savesRestore(gameId, restoring.id));
            setRestoring(null);
            await refresh();
          }}
        />
      )}
    </div>
  );
}

/* ── One character ────────────────────────────────────────────────── */

function CharacterCard({
  character,
  accounts,
  busy,
  onMove,
  onConvert,
  onSnapshot,
}: {
  character: Character;
  accounts: SteamAccount[];
  busy: boolean;
  onMove: () => void;
  onConvert: () => void;
  onSnapshot: () => void;
}) {
  const { slot, entry, folder, summary } = character;
  const account =
    accounts.find((a) => a.steamId64 === summary.steamId)?.personaName ??
    folder.accountName ??
    (folder.likelyCracked ? "Non-Steam" : "Unknown");

  return (
    <motion.article
      className="sv__card"
      layout
      variants={{
        hidden: { opacity: 0, y: 18 },
        show: { opacity: 1, y: 0, transition: { duration: 0.6, ease: EASE } },
      }}
      exit={{ opacity: 0, scale: 0.97, transition: { duration: 0.25 } }}
      whileHover={{ y: -3 }}
      transition={{ duration: 0.4, ease: SOFT }}
    >
      <header className="sv__top">
        <div className="sv__level">{slot.level}</div>
        <div className="sv__who">
          <span className="sv__name">{slot.name.trim() || "Unnamed"}</span>
          <span className="sv__meta">
            {playtime(slot.secondsPlayed)} · slot {slot.index + 1}
          </span>
        </div>
        {entry.flavour === "seamless-coop" ? <Chip tone="ok">Co-op</Chip> : <Chip>Solo</Chip>}
      </header>

      <div className="sv__facts">
        <span className="sv__fact">
          <span className="w4">Account</span>
          <span className="truncate">{account}</span>
        </span>
        <span className="sv__fact">
          <span className="w4">Saved</span>
          <span>{when(entry.modified)}</span>
        </span>
        {!summary.checksumsValid && <Chip tone="bad">Checksum mismatch</Chip>}
      </div>

      <footer className="sv__acts">
        <button type="button" className="btn btn--ghost btn--sm" onClick={onMove}>
          <Icon.Swap size={13} />
          Move
        </button>
        <button type="button" className="btn btn--ghost btn--sm" onClick={onConvert}>
          Convert
        </button>
        <button
          type="button"
          className="btn btn--ghost btn--sm"
          onClick={onSnapshot}
          disabled={busy}
        >
          <Icon.Save size={13} />
          Snapshot
        </button>
      </footer>
    </motion.article>
  );
}

/* ── Moving one character ─────────────────────────────────────────── */

/**
 * Moving starts from a character rather than from two file pickers.
 *
 * You already chose who; the only question left is which save they go into, and
 * the destination slot is picked automatically because nobody cares which of
 * the ten empty slots it lands in.
 */
function MoveModal({
  gameId,
  character,
  files,
  summaries,
  onClose,
  onDone,
}: {
  gameId: GameId;
  character: Character;
  files: SaveEntry[];
  summaries: Record<string, SaveSummary | null>;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const toast = useToast();
  const targets = files.filter((file) => file.path !== character.entry.path);
  const [to, setTo] = useState(targets[0]?.path ?? "");
  const [busy, setBusy] = useState(false);

  const destination = summaries[to] ?? null;
  const free = destination?.slots.filter((s) => !s.active).map((s) => s.index) ?? [];
  const ok = Boolean(to) && free.length > 0;

  return (
    <Modal
      title={`Move ${character.slot.name.trim() || "this character"}`}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn--ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--solid"
            disabled={!ok || busy}
            onClick={async () => {
              setBusy(true);
              try {
                const done = await toast.run("Character moved", () =>
                  api.savesTransfer(gameId, character.entry.path, to, [
                    [character.slot.index, free[0]],
                  ]),
                );
                if (done) {
                  onClose();
                  await onDone();
                }
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? <span className="spin" /> : null}
            Move
          </button>
        </>
      }
    >
      <div className="col">
        {targets.length === 0 ? (
          <div className="note note--warn">
            <Icon.Warning size={15} />
            <div className="note__b">
              There is only one save file, so there is nowhere to move this to.
            </div>
          </div>
        ) : (
          <>
            <div className="fld">
              <label className="fld__l" htmlFor="mv-to">Into</label>
              <select id="mv-to" className="sel2" value={to} onChange={(e) => setTo(e.target.value)}>
                {targets.map((file) => (
                  <option key={file.path} value={file.path}>
                    {saveLabel(file)} · {file.accountName ?? file.folderId ?? "unknown account"}
                  </option>
                ))}
              </select>
            </div>

            {free.length === 0 ? (
              <div className="note note--bad">
                <Icon.Warning size={15} />
                <div className="note__b">That save has no free slot. All ten are taken.</div>
              </div>
            ) : (
              <div className="note">
                <Icon.Info size={15} />
                <div className="note__b">
                  Lands in slot {free[0] + 1}. The account id inside the file is rewritten
                  and the checksums recomputed, and the destination is snapshotted first.
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </Modal>
  );
}

/* ── Converting a file ────────────────────────────────────────────── */

function ConvertModal({
  gameId,
  entry,
  accounts,
  onClose,
  onDone,
}: {
  gameId: GameId;
  entry: SaveEntry;
  accounts: SteamAccount[];
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const toast = useToast();
  const target = entry.extension === "sl2" ? "co2" : "sl2";
  const [rebind, setRebind] = useState("");
  const [busy, setBusy] = useState(false);

  return (
    <Modal
      title="Convert this save"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn--ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--solid"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                const done = await toast.run("Save converted", () =>
                  api.savesConvert(gameId, entry.path, target, rebind ? Number(rebind) : undefined),
                );
                if (done) {
                  onClose();
                  await onDone();
                }
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? <span className="spin" /> : null}
            Make it {target === "co2" ? "co-op" : "solo"}
          </button>
        </>
      }
    >
      <div className="col">
        <div className="note">
          <Icon.Info size={15} />
          <div className="note__b">
            A copy is written beside the original as{" "}
            <span className="mono">.{target}</span>. Nothing is overwritten.
          </div>
        </div>

        <div className="fld">
          <label className="fld__l" htmlFor="cv-acct">Give it to another account</label>
          <select
            id="cv-acct"
            className="sel2"
            value={rebind}
            onChange={(e) => setRebind(e.target.value)}
          >
            <option value="">Keep the current account</option>
            {accounts.map((account) => (
              <option key={account.steamId64} value={String(account.steamId64)}>
                {account.personaName || account.accountName}
              </option>
            ))}
          </select>
          <span className="fld__h">Needed when the save came from a different copy.</span>
        </div>
      </div>
    </Modal>
  );
}
