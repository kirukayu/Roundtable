import { useCallback, useEffect, useMemo, useState } from "react";

import { Icon } from "../../components/Icons";
import { Blank, Card, Chip, Confirm, Modal, Skeleton, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import { bytes, exact, playtime, when } from "../../lib/format";
import type {
  BackupRecord,
  GameId,
  SaveEntry,
  SaveFolder,
  SaveSummary,
  SteamAccount,
} from "../../lib/types";

/**
 * Saves, presented as characters rather than as files.
 *
 * The list on the left is every container Roundtable found; the panel on the right
 * is who lives inside the selected one. Transfer and conversion are actions on a
 * character, not separate screens.
 */
export default function SavesTab({ gameId }: { gameId: GameId }) {
  const toast = useToast();
  const [folders, setFolders] = useState<SaveFolder[] | null>(null);
  const [backups, setBackups] = useState<BackupRecord[]>([]);
  const [accounts, setAccounts] = useState<SteamAccount[]>([]);
  const [selected, setSelected] = useState<SaveEntry | null>(null);
  const [summary, setSummary] = useState<SaveSummary | null>(null);
  const [transferOpen, setTransferOpen] = useState(false);
  const [convertOpen, setConvertOpen] = useState(false);
  const [showBackups, setShowBackups] = useState(false);
  const [restoring, setRestoring] = useState<BackupRecord | null>(null);

  const refresh = useCallback(async () => {
    const [found, stored, steam] = await Promise.all([
      api.savesDiscover(gameId),
      api.savesBackups(gameId),
      api.steamAccounts(),
    ]);
    setFolders(found);
    setBackups(stored);
    setAccounts(steam);
    setSelected((current) => {
      if (current && found.some((f) => f.entries.some((e) => e.path === current.path))) {
        return current;
      }
      return found.flatMap((f) => f.entries).find((e) => e.flavour !== "game-backup") ?? null;
    });
  }, [gameId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!selected) {
      setSummary(null);
      return;
    }
    let cancelled = false;
    setSummary(null);
    api
      .savesInspect(selected.path)
      .then((result) => !cancelled && setSummary(result))
      .catch(() => !cancelled && setSummary(null));
    return () => {
      cancelled = true;
    };
  }, [selected]);

  const entries = useMemo(
    () => folders?.flatMap((f) => f.entries).filter((e) => e.flavour !== "game-backup") ?? [],
    [folders],
  );

  if (folders === null) {
    return (
      <div className="grid-2">
        <Card><Skeleton variant="line" count={4} /></Card>
        <Card><Skeleton variant="line" count={4} /></Card>
      </div>
    );
  }

  if (folders.length === 0) {
    return (
      <Card>
        <Blank icon={Icon.Save} title="No saves yet">
          Nothing under <span className="mono">%APPDATA%</span>. Start the game once so it
          creates a character, then come back.
        </Blank>
      </Card>
    );
  }

  const active = summary?.slots.filter((s) => s.active) ?? [];

  return (
    <div className="col reveal">
      <div className="row-between">
        <div className="row" style={{ gap: "var(--s2)" }}>
          <Chip>{entries.length} save files</Chip>
          <Chip>{backups.length} snapshots</Chip>
        </div>
        <div className="row" style={{ gap: "var(--s2)" }}>
          <button type="button" className="btn btn--sm" onClick={() => setShowBackups(true)}>
            <Icon.Clock size={14} />
            Snapshots
          </button>
          <button
            type="button"
            className="btn btn--sm"
            disabled={entries.length < 2}
            onClick={() => setTransferOpen(true)}
          >
            <Icon.Swap size={14} />
            Transfer
          </button>
          <button
            type="button"
            className="btn btn--primary btn--sm"
            disabled={!selected}
            onClick={async () => {
              if (!selected) return;
              const made = await toast.run("Snapshot taken", () =>
                api.savesBackup(gameId, selected.path, "manual"),
              );
              if (made) setBackups(await api.savesBackups(gameId));
            }}
          >
            <Icon.Save size={14} />
            Back up
          </button>
        </div>
      </div>

      <div className="grid-2">
        <Card title="Save files" flush>
          <div className="rows" style={{ padding: "var(--s3)" }}>
            {folders.map((folder) => (
              <div key={folder.path}>
                <div className="row" style={{ padding: "var(--s2) var(--s2) var(--s1)" }}>
                  <span className="rw__sub">
                    {folder.accountName ?? folder.folderId ?? "Unknown account"}
                  </span>
                  {folder.likelyCracked ? (
                    <Chip tone="warning">Non-Steam</Chip>
                  ) : (
                    <Chip tone="info">
                      <Icon.Steam size={11} />
                      Steam
                    </Chip>
                  )}
                </div>
                {folder.entries.map((entry) => (
                  <button
                    key={entry.path}
                    type="button"
                    className={`rw rw--action${selected?.path === entry.path ? " rw--on" : ""}`}
                    onClick={() => setSelected(entry)}
                  >
                    <div className="grow" style={{ minWidth: 0 }}>
                      <div className="rw__title mono truncate">{entry.fileName}</div>
                      <div className="rw__sub">
                        {bytes(entry.sizeBytes)} · {when(entry.modified)}
                      </div>
                    </div>
                    {entry.flavour === "seamless-coop" && <Chip tone="success">co-op</Chip>}
                    {entry.flavour === "game-backup" && <Chip>backup</Chip>}
                  </button>
                ))}
              </div>
            ))}
          </div>
        </Card>

        <Card
          title={selected ? selected.fileName : "Characters"}
          action={
            selected && (
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                onClick={() => setConvertOpen(true)}
              >
                <Icon.Swap size={13} />
                Convert
              </button>
            )
          }
        >
          {!selected ? (
            <Blank icon={Icon.Save} title="Pick a save">
              Select a file to see its characters.
            </Blank>
          ) : !summary ? (
            <Skeleton variant="line" count={4} />
          ) : (
            <>
              <div className="row" style={{ gap: "var(--s6)", marginBottom: "var(--s4)" }}>
                <div>
                  <div className="stat__v">{active.length}</div>
                  <div className="stat__k">characters</div>
                </div>
                <div>
                  <div className="stat__v" style={{ fontSize: "var(--text-lg)" }}>
                    {summary.checksumsValid ? (
                      <span style={{ color: "var(--success)" }}>Valid</span>
                    ) : (
                      <span style={{ color: "var(--error)" }}>Mismatch</span>
                    )}
                  </div>
                  <div className="stat__k">checksums</div>
                </div>
                <div>
                  <div className="stat__v mono" style={{ fontSize: "var(--text-sm)" }}>
                    {summary.steamId}
                  </div>
                  <div className="stat__k">
                    {accounts.find((a) => a.steamId64 === summary.steamId)?.personaName ??
                      "account"}
                  </div>
                </div>
              </div>

              {active.length === 0 ? (
                <Blank icon={Icon.Users} title="No characters">
                  All ten slots are empty.
                </Blank>
              ) : (
                <div className="rows">
                  {active.map((slot) => (
                    <div key={slot.index} className="rw">
                      <span className="mono faint" style={{ width: 16 }}>
                        {slot.index + 1}
                      </span>
                      <div className="grow">
                        <div className="rw__title">{slot.name.trim() || "Unnamed"}</div>
                        <div className="rw__sub">
                          Level {slot.level} · {playtime(slot.secondsPlayed)}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </Card>
      </div>

      {transferOpen && (
        <TransferModal
          gameId={gameId}
          entries={entries}
          onClose={() => setTransferOpen(false)}
          onDone={refresh}
        />
      )}

      {convertOpen && selected && (
        <ConvertModal
          gameId={gameId}
          entry={selected}
          accounts={accounts}
          onClose={() => setConvertOpen(false)}
          onDone={refresh}
        />
      )}

      {showBackups && (
        <Modal title="Snapshots" wide onClose={() => setShowBackups(false)}>
          {backups.length === 0 ? (
            <Blank icon={Icon.Clock} title="No snapshots yet">
              Roundtable takes one before every launch and before anything that writes to a
              save.
            </Blank>
          ) : (
            <div className="rows">
              {backups.map((backup) => (
                <div key={backup.id} className="rw">
                  <div className="grow" style={{ minWidth: 0 }}>
                    <div className="row" style={{ gap: "var(--s2)" }}>
                      <span className="rw__title">{backup.label}</span>
                      {backup.automatic ? <Chip>auto</Chip> : <Chip tone="accent">manual</Chip>}
                    </div>
                    <div className="rw__sub">
                      {exact(backup.created)} · {bytes(backup.sizeBytes)}
                    </div>
                    {backup.characters.length > 0 && (
                      <div className="rw__sub">{backup.characters.join(" · ")}</div>
                    )}
                  </div>
                  <button
                    type="button"
                    className="btn btn--sm"
                    onClick={() => setRestoring(backup)}
                  >
                    Restore
                  </button>
                  <button
                    type="button"
                    className="btn btn--ghost btn--sm btn--icon btn--danger"
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
            <>
              <p>
                <span className="mono">{restoring.fileName}</span> goes back to{" "}
                <span className="mono">{restoring.origin}</span>.
              </p>
              <p className="field__help">
                Whatever is there now is snapshotted first, so this is reversible.
              </p>
            </>
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

function TransferModal({
  gameId,
  entries,
  onClose,
  onDone,
}: {
  gameId: GameId;
  entries: SaveEntry[];
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const toast = useToast();
  const [from, setFrom] = useState(entries[0]?.path ?? "");
  const [to, setTo] = useState(entries[1]?.path ?? "");
  const [fromSummary, setFromSummary] = useState<SaveSummary | null>(null);
  const [toSummary, setToSummary] = useState<SaveSummary | null>(null);
  const [picked, setPicked] = useState<number[]>([]);

  useEffect(() => {
    if (!from) return;
    api.savesInspect(from).then(setFromSummary).catch(() => setFromSummary(null));
    setPicked([]);
  }, [from]);

  useEffect(() => {
    if (!to) return;
    api.savesInspect(to).then(setToSummary).catch(() => setToSummary(null));
  }, [to]);

  const free = toSummary?.slots.filter((s) => !s.active).map((s) => s.index) ?? [];
  const ok = from !== to && picked.length > 0 && picked.length <= free.length;

  return (
    <Modal
      title="Move characters"
      wide
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn--ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={!ok}
            onClick={async () => {
              const pairs = picked.map((slot, i) => [slot, free[i]] as [number, number]);
              const report = await toast.run("Characters copied", () =>
                api.savesTransfer(gameId, from, to, pairs),
              );
              if (report) {
                onClose();
                await onDone();
              }
            }}
          >
            Copy {picked.length || ""}
          </button>
        </>
      }
    >
      <div className="col">
        <div className="notice">
          <Icon.Info size={15} />
          <div className="notice__body">
            The account id is written inside every save, which is why copying the file
            between installations does not work. This rewrites it and recomputes the
            checksums. The destination is snapshotted first.
          </div>
        </div>

        <div className="grid-2">
          <div className="col-sm">
            <label className="field__label" htmlFor="tfrom">From</label>
            <select
              id="tfrom"
              className="select"
              value={from}
              onChange={(e) => setFrom(e.target.value)}
            >
              {entries.map((entry) => (
                <option key={entry.path} value={entry.path}>
                  {entry.fileName} · {entry.accountName ?? entry.folderId ?? "unknown"}
                </option>
              ))}
            </select>
            <div className="rows" style={{ marginTop: "var(--s2)" }}>
              {(fromSummary?.slots ?? [])
                .filter((s) => s.active)
                .map((slot) => {
                  const on = picked.includes(slot.index);
                  return (
                    <button
                      key={slot.index}
                      type="button"
                      className={`rw rw--action${on ? " rw--on" : ""}`}
                      onClick={() =>
                        setPicked((current) =>
                          on
                            ? current.filter((i) => i !== slot.index)
                            : [...current, slot.index],
                        )
                      }
                    >
                      <span style={{ width: 16, color: on ? "var(--accent)" : "transparent" }}>
                        <Icon.Check size={14} />
                      </span>
                      <div className="grow">
                        <div className="rw__title">{slot.name.trim() || "Unnamed"}</div>
                        <div className="rw__sub">
                          Level {slot.level} · {playtime(slot.secondsPlayed)}
                        </div>
                      </div>
                    </button>
                  );
                })}
            </div>
          </div>

          <div className="col-sm">
            <label className="field__label" htmlFor="tto">To</label>
            <select
              id="tto"
              className="select"
              value={to}
              onChange={(e) => setTo(e.target.value)}
            >
              {entries.map((entry) => (
                <option key={entry.path} value={entry.path}>
                  {entry.fileName} · {entry.accountName ?? entry.folderId ?? "unknown"}
                </option>
              ))}
            </select>
            <p className="field__help">
              {free.length} free slot{free.length === 1 ? "" : "s"}
            </p>

            {from === to && (
              <div className="notice notice--blocker">
                <Icon.Warning size={15} />
                <div className="notice__body">Pick two different files.</div>
              </div>
            )}
            {from !== to && picked.length > free.length && (
              <div className="notice notice--blocker">
                <Icon.Warning size={15} />
                <div className="notice__body">
                  {picked.length} chosen but only {free.length} free slots.
                </div>
              </div>
            )}
            {ok && (
              <div className="rows">
                {picked.map((slot, i) => (
                  <div key={slot} className="rw">
                    <span className="grow rw__title">
                      {fromSummary?.slots[slot]?.name.trim() || "Unnamed"}
                    </span>
                    <Icon.Chevron size={13} />
                    <span className="mono">slot {free[i] + 1}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </Modal>
  );
}

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
  const [extension, setExtension] = useState(entry.extension === "sl2" ? "co2" : "sl2");
  const [rebind, setRebind] = useState("");

  return (
    <Modal
      title="Convert save"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn btn--ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--primary"
            onClick={async () => {
              const done = await toast.run("Save converted", () =>
                api.savesConvert(
                  gameId,
                  entry.path,
                  extension,
                  undefined,
                  rebind ? Number(rebind) : undefined,
                ),
              );
              if (done) {
                onClose();
                await onDone();
              }
            }}
          >
            Convert
          </button>
        </>
      }
    >
      <div className="col">
        <p className="field__help">
          A copy is written beside the original with the new extension. Nothing is
          overwritten.
        </p>

        <div className="field">
          <label className="field__label" htmlFor="cext">Extension</label>
          <div className="row" style={{ gap: "var(--s2)" }}>
            <input
              id="cext"
              className="input mono"
              style={{ maxWidth: 140 }}
              value={extension}
              onChange={(e) => setExtension(e.target.value)}
            />
            <button type="button" className="btn btn--sm" onClick={() => setExtension("sl2")}>
              sl2
            </button>
            <button type="button" className="btn btn--sm" onClick={() => setExtension("co2")}>
              co2
            </button>
          </div>
          <span className="field__help">
            <span className="mono">sl2</span> is the vanilla game;{" "}
            <span className="mono">co2</span> is Seamless Co-op.
          </span>
        </div>

        <div className="field">
          <label className="field__label" htmlFor="creb">Give it to another account</label>
          <select
            id="creb"
            className="select"
            value={rebind}
            onChange={(e) => setRebind(e.target.value)}
          >
            <option value="">Keep the current account</option>
            {accounts.map((account) => (
              <option key={account.steamId64} value={String(account.steamId64)}>
                {account.personaName || account.accountName} · {account.steamId64}
              </option>
            ))}
          </select>
          <span className="field__help">
            Needed when the save came from a different copy of the game.
          </span>
        </div>
      </div>
    </Modal>
  );
}
