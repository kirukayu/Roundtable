import { useCallback, useEffect, useState } from "react";

import { Icon } from "../../components/Icons";
import { Blank, Card, Chip, Confirm, Modal, Option, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import { bytes } from "../../lib/format";
import type { ConflictReport, GameId, ModRecord, Profile } from "../../lib/types";

/**
 * Mods and load order together, because they are the same decision.
 *
 * The left column is the profile's order, drag-free but reorderable; the right is
 * everything installed. Adding is one button, and conflicts are a click away.
 */
export default function ModsPane({
  gameId,
  profile,
  profiles,
  onChanged,
}: {
  gameId: GameId;
  profile: Profile | null;
  profiles: Profile[];
  onChanged: () => Promise<void>;
}) {
  const toast = useToast();
  const [library, setLibrary] = useState<ModRecord[]>([]);
  const [draft, setDraft] = useState<Profile | null>(profile);
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<ModRecord | null>(null);
  const [conflicts, setConflicts] = useState<ConflictReport | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const refresh = useCallback(async () => {
    setLibrary(await api.modsList(gameId));
  }, [gameId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    setDraft(profile);
  }, [profile]);

  const save = async (next: Profile) => {
    setDraft(next);
    await api.profileSave(next);
    await onChanged();
  };

  const addArchive = async () => {
    const picked = await api.pickFile("Select a mod archive", "zip,7z");
    if (!picked) return;
    setBusy(true);
    const added = await toast.run("Mod added", () => api.modsInstall(gameId, picked));
    if (added) {
      await refresh();
      if (draft) {
        await save({
          ...draft,
          mods: [{ modId: added.id, enabled: true }, ...draft.mods],
        });
      }
    }
    setBusy(false);
  };

  const addFolder = async () => {
    const picked = await api.pickFolder("Select a mod folder");
    if (!picked) return;
    setBusy(true);
    const added = await toast.run("Mod added", () => api.modsInstall(gameId, picked));
    if (added) {
      await refresh();
      if (draft) {
        await save({
          ...draft,
          mods: [{ modId: added.id, enabled: true }, ...draft.mods],
        });
      }
    }
    setBusy(false);
  };

  const inOrder = draft
    ? draft.mods
        .map((entry) => ({ entry, mod: library.find((m) => m.id === entry.modId) }))
        .filter((pair): pair is { entry: typeof pair.entry; mod: ModRecord } => Boolean(pair.mod))
    : [];

  const unused = library.filter((mod) => !draft?.mods.some((m) => m.modId === mod.id));

  const move = (index: number, delta: -1 | 1) => {
    if (!draft) return;
    const target = index + delta;
    if (target < 0 || target >= draft.mods.length) return;
    const mods = [...draft.mods];
    [mods[index], mods[target]] = [mods[target], mods[index]];
    void save({ ...draft, mods });
  };

  const toggle = (index: number) => {
    if (!draft) return;
    const mods = [...draft.mods];
    mods[index] = { ...mods[index], enabled: !mods[index].enabled };
    void save({ ...draft, mods });
  };

  const addToProfile = (mod: ModRecord) => {
    if (!draft) return;
    void save({ ...draft, mods: [...draft.mods, { modId: mod.id, enabled: true }] });
  };

  const removeFromProfile = (modId: string) => {
    if (!draft) return;
    void save({ ...draft, mods: draft.mods.filter((m) => m.modId !== modId) });
  };

  return (
    <div className="col rev">
      <div className="between">
        <div className="row" style={{ gap: "var(--s2)" }}>
          {profiles.map((entry) => (
            <Chip key={entry.id} tone={entry.id === draft?.id ? "a" : undefined}>
              {entry.name}
            </Chip>
          ))}
        </div>
        <div className="row" style={{ gap: "var(--s2)" }}>
          {draft && (
            <>
              <button
                type="button"
                className="btn btn--s"
                onClick={() => setSettingsOpen(true)}
              >
                <Icon.Settings size={14} />
                Options
              </button>
              <button
                type="button"
                className="btn btn--s"
                disabled={inOrder.filter((p) => p.entry.enabled).length < 2}
                onClick={async () => {
                  const report = await toast.run("Conflicts checked", () =>
                    api.profileConflicts(gameId, draft.id),
                  );
                  if (report) setConflicts(report);
                }}
              >
                <Icon.Merge size={14} />
                Conflicts
              </button>
            </>
          )}
          <button type="button" className="btn btn--s" onClick={addFolder} disabled={busy}>
            <Icon.Folder size={14} />
            Folder
          </button>
          <button
            type="button"
            className="btn btn--a btn--s"
            onClick={addArchive}
            disabled={busy}
          >
            {busy ? <span className="spin" /> : <Icon.Plus size={14} />}
            Add mod
          </button>
        </div>
      </div>

      {library.length === 0 ? (
        <Card>
          <Blank
            icon={Icon.Layers}
            title="No mods yet"
            action={
              <div className="row" style={{ gap: "var(--s2)" }}>
                <button type="button" className="btn btn--a" onClick={addArchive}>
                  <Icon.Plus size={15} />
                  Add an archive
                </button>
                <button type="button" className="btn" onClick={addFolder}>
                  <Icon.Folder size={15} />
                  Add a folder
                </button>
              </div>
            }
          >
            Drop in the zip or 7z exactly as it downloaded. Wrapper folders, bundled
            loaders and stray readmes are all handled for you.
          </Blank>
        </Card>
      ) : (
        <div className="g2">
          <Card title={`Load order · ${inOrder.filter((p) => p.entry.enabled).length} active`}>
            {inOrder.length === 0 ? (
              <Blank icon={Icon.Layers} title="Nothing in this profile">
                Add mods from the list on the right.
              </Blank>
            ) : (
              <div className="rows">
                {inOrder.map(({ entry, mod }, index) => (
                  <div key={mod.id} className={`rw${entry.enabled ? " rw--on" : ""}`}>
                    <span className="mono faint" style={{ width: 16 }}>
                      {index + 1}
                    </span>
                    <div className="grow" style={{ minWidth: 0 }}>
                      <div className="rw__t truncate">{mod.name}</div>
                      <div className="rw__s">
                        {mod.kind === "assets"
                          ? "Assets"
                          : mod.kind === "native"
                            ? "DLL"
                            : "Assets + DLL"}{" "}
                        · {bytes(mod.sizeBytes)}
                      </div>
                    </div>
                    <div className="row" style={{ gap: 2 }}>
                      <button
                        type="button"
                        className="btn btn--g btn--s btn--i"
                        aria-label="Move up"
                        disabled={index === 0}
                        onClick={() => move(index, -1)}
                      >
                        <Icon.Chevron size={13} style={{ transform: "rotate(-90deg)" }} />
                      </button>
                      <button
                        type="button"
                        className="btn btn--g btn--s btn--i"
                        aria-label="Move down"
                        disabled={index === inOrder.length - 1}
                        onClick={() => move(index, 1)}
                      >
                        <Icon.Chevron size={13} style={{ transform: "rotate(90deg)" }} />
                      </button>
                      <button
                        type="button"
                        role="switch"
                        aria-checked={entry.enabled}
                        aria-label={`Enable ${mod.name}`}
                        className="sw"
                        onClick={() => toggle(index)}
                      />
                      <button
                        type="button"
                        className="btn btn--g btn--s btn--i"
                        aria-label="Remove from profile"
                        onClick={() => removeFromProfile(mod.id)}
                      >
                        <Icon.Close size={13} />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
            <p className="fld__h" style={{ marginTop: "var(--s3)" }}>
              The mod at the top wins when two provide the same file.
            </p>
          </Card>

          <Card title={`Installed · ${library.length}`}>
            {unused.length === 0 ? (
              <Blank icon={Icon.Check} title="All in use">
                Every mod in the library is part of this profile.
              </Blank>
            ) : (
              <div className="rows">
                {unused.map((mod) => (
                  <div key={mod.id} className="rw">
                    <div className="grow" style={{ minWidth: 0 }}>
                      <div className="rw__t truncate">{mod.name}</div>
                      <div className="rw__s">
                        {mod.fileCount} files · {bytes(mod.sizeBytes)}
                        {mod.bundledLoader ? ` · ships ${mod.bundledLoader}` : ""}
                      </div>
                    </div>
                    <div className="row" style={{ gap: 2 }}>
                      <button
                        type="button"
                        className="btn btn--g btn--s btn--i"
                        aria-label="Open folder"
                        onClick={() => void api.openPath(mod.path)}
                      >
                        <Icon.Folder size={13} />
                      </button>
                      <button
                        type="button"
                        className="btn btn--g btn--s btn--i btn--bad"
                        aria-label="Delete"
                        onClick={() => setRemoving(mod)}
                      >
                        <Icon.Trash size={13} />
                      </button>
                      {draft && (
                        <button
                          type="button"
                          className="btn btn--s"
                          onClick={() => addToProfile(mod)}
                        >
                          Add
                        </button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </Card>
        </div>
      )}

      {removing && (
        <Confirm
          title="Delete this mod?"
          destructive
          confirmLabel="Delete"
          body={
            <>
              <p>
                <strong>{removing.name}</strong> and its {removing.fileCount} files leave the
                library.
              </p>
              <p className="fld__h">Your game folder is untouched.</p>
            </>
          }
          onCancel={() => setRemoving(null)}
          onConfirm={async () => {
            await api.modsDelete(gameId, removing.id);
            if (draft) removeFromProfile(removing.id);
            setRemoving(null);
            await refresh();
            toast.success("Mod deleted");
          }}
        />
      )}

      {conflicts && (
        <Modal title="File conflicts" wide onClose={() => setConflicts(null)}>
          <div className="col">
            <div className="row" style={{ gap: "var(--s7)" }}>
              <div>
                <div className="stat__v">{conflicts.conflicts.length}</div>
                <div className="stat__k">conflicting files</div>
              </div>
              <div>
                <div className="stat__v">{conflicts.totalFiles}</div>
                <div className="stat__k">files in total</div>
              </div>
            </div>

            {conflicts.regulationProviders.length > 1 && (
              <NoticeRegulation providers={conflicts.regulationProviders} />
            )}

            {conflicts.conflicts.length === 0 ? (
              <Blank icon={Icon.Check} title="No overlap">
                These mods touch entirely separate files, so the order does not matter.
              </Blank>
            ) : (
              <div className="rows scroll-cap">
                {conflicts.conflicts.slice(0, 200).map((clash) => (
                  <div key={clash.relativePath} className="rw">
                    <div className="grow" style={{ minWidth: 0 }}>
                      <div className="mono truncate">{clash.relativePath}</div>
                      <div className="rw__s">{clash.providers.join(" · ")}</div>
                    </div>
                    <Chip tone={clash.mergeable ? "bad" : "warn"}>
                      {clash.winner} wins
                    </Chip>
                  </div>
                ))}
              </div>
            )}
          </div>
        </Modal>
      )}

      {settingsOpen && draft && (
        <Modal title={`${draft.name} options`} onClose={() => setSettingsOpen(false)}>
          <div className="col">
            <div className="fld">
              <label className="fld__l" htmlFor="pname">Name</label>
              <input
                id="pname"
                className="in"
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                onBlur={() => void save(draft)}
              />
            </div>

            <Option
              label="Seamless Co-op"
              help="Loads ersc.dll alongside these mods."
              checked={draft.seamlessCoop}
              onChange={(next) => void save({ ...draft, seamlessCoop: next })}
            />
            <Option
              label="Skip intro logos"
              checked={draft.skipLogos}
              onChange={(next) => void save({ ...draft, skipLogos: next })}
            />
            <Option
              label="Neutralise Arxan"
              help="Makes heavy mods considerably more stable."
              checked={draft.disableArxan}
              onChange={(next) => void save({ ...draft, disableArxan: next })}
            />
            <Option
              label="Patch memory limits"
              help="Large overhauls need the extra allocation ceiling."
              checked={draft.memPatch}
              onChange={(next) => void save({ ...draft, memPatch: next })}
            />
            <Option
              label="Connect to official servers"
              help="Leave off. Going online with mods loaded risks a ban."
              checked={draft.startOnline}
              onChange={(next) => void save({ ...draft, startOnline: next })}
            />

            <div className="fld">
              <label className="fld__l" htmlFor="psave">Separate save file</label>
              <input
                id="psave"
                className="in mono"
                placeholder="leave blank to share the normal save"
                value={draft.savefile ?? ""}
                disabled={draft.seamlessCoop}
                onChange={(event) =>
                  setDraft({ ...draft, savefile: event.target.value || null })
                }
                onBlur={() => void save(draft)}
              />
              <span className="fld__h">
                {draft.seamlessCoop
                  ? "Co-op already isolates saves through its own extension."
                  : "A modded run writing into your vanilla save is how characters get lost."}
              </span>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

function NoticeRegulation({ providers }: { providers: string[] }) {
  return (
    <div className="note note--warn">
      <Icon.Warning size={15} />
      <div>
        <div className="note__t">Two mods change the same balance data</div>
        <div className="note__b">
          {providers.join(" and ")} both ship regulation.bin. Only the first takes effect, so
          the other's weapon, spell and enemy changes are discarded. Mods this large usually
          need a purpose-built merged build.
        </div>
      </div>
    </div>
  );
}
