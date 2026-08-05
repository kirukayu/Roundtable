import { useCallback, useEffect, useState } from "react";

import { Icon } from "../../components/Icons";
import { Card, Chip, Confirm, Skeleton, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import { bytes } from "../../lib/format";
import type { CacheLocation, EacStatus, GameId, Installation, SystemReport } from "../../lib/types";

/**
 * The tertiary tab: anti-cheat, shader caches, machine state, and removing the
 * installation. Deliberately last, because none of it is why you opened the app.
 */
export default function SystemPane({
  gameId,
  install,
  eac,
  onEacChanged,
  onForget,
}: {
  gameId: GameId;
  install: Installation;
  eac: EacStatus | null;
  onEacChanged: (next: EacStatus) => void;
  onForget: () => Promise<void>;
}) {
  const toast = useToast();
  const [caches, setCaches] = useState<CacheLocation[] | null>(null);
  const [report, setReport] = useState<SystemReport | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [confirmEac, setConfirmEac] = useState(false);
  const [confirmForget, setConfirmForget] = useState(false);
  const [clearing, setClearing] = useState(false);

  const load = useCallback(async () => {
    const [found, system] = await Promise.all([
      api.sysShaderCaches(),
      api.sysReport(gameId),
    ]);
    setCaches(found);
    setReport(system);
    setChosen(new Set(found.filter((c) => c.exists && c.sizeBytes > 0).map((c) => c.path)));
  }, [gameId]);

  useEffect(() => {
    void load();
  }, [load]);

  const selectedSize = (caches ?? [])
    .filter((c) => chosen.has(c.path))
    .reduce((sum, c) => sum + c.sizeBytes, 0);

  return (
    <div className="col rev">
      <div className="g2">
        <Card
          title="Shader caches"
          action={
            <button
              type="button"
              className="btn btn--a btn--s"
              disabled={chosen.size === 0 || clearing}
              onClick={async () => {
                setClearing(true);
                const result = await toast.run(
                  "Caches cleared",
                  () => api.sysClearCaches([...chosen]),
                  (value) => `${bytes(value.bytesFreed)} reclaimed`,
                );
                if (result?.skipped.length) {
                  toast.info("Some items were in use", result.skipped.join(" · "));
                }
                setClearing(false);
                await load();
              }}
            >
              {clearing ? <span className="spin" /> : <Icon.Broom size={14} />}
              Clear {bytes(selectedSize)}
            </button>
          }
        >
          <p className="fld__h" style={{ marginBottom: "var(--s3)" }}>
            Stale after every driver or game update, and the usual cause of stutter blamed
            on mods.
          </p>
          {caches === null ? (
            <Skeleton variant="line" count={4} />
          ) : (
            <div className="rows">
              {caches
                .filter((cache) => cache.exists && cache.sizeBytes > 0)
                .map((cache) => {
                  const on = chosen.has(cache.path);
                  return (
                    <button
                      key={cache.path}
                      type="button"
                      className={`rw rw--a${on ? " rw--on" : ""}`}
                      onClick={() =>
                        setChosen((current) => {
                          const next = new Set(current);
                          if (next.has(cache.path)) next.delete(cache.path);
                          else next.add(cache.path);
                          return next;
                        })
                      }
                    >
                      <span style={{ width: 16, color: on ? "var(--accent)" : "transparent" }}>
                        <Icon.Check size={14} />
                      </span>
                      <div className="grow">
                        <div className="rw__t">
                          {cache.owner} · {cache.label}
                        </div>
                        <div className="rw__s">{cache.fileCount} files</div>
                      </div>
                      <span className="mono" style={{ color: "var(--accent)" }}>
                        {bytes(cache.sizeBytes)}
                      </span>
                    </button>
                  );
                })}
              {caches.every((c) => !c.exists || c.sizeBytes === 0) && (
                <p className="fld__h">Everything is already empty.</p>
              )}
            </div>
          )}
        </Card>

        <Card title="Anti-cheat">
          {!eac ? (
            <Skeleton variant="line" count={3} />
          ) : (
            <>
              <div className="row" style={{ gap: "var(--s3)", marginBottom: "var(--s3)" }}>
                <span style={{ color: eac.state === "bypassed" ? "var(--error)" : "var(--success)" }}>
                  {eac.state === "bypassed" ? (
                    <Icon.ShieldOff size={22} />
                  ) : (
                    <Icon.Shield size={22} />
                  )}
                </span>
                <div className="grow">
                  <div className="opt__l">
                    {eac.state === "bypassed"
                      ? "Bypassed"
                      : eac.state === "active"
                        ? "Active"
                        : "Not present"}
                  </div>
                  <div className="opt__h">{eac.detail}</div>
                </div>
              </div>
              {eac.state !== "not-present" && (
                <button
                  type="button"
                  className={`btn btn--w${eac.state === "bypassed" ? " btn--a" : ""}`}
                  onClick={() => setConfirmEac(true)}
                >
                  {eac.state === "bypassed" ? "Restore anti-cheat" : "Bypass anti-cheat"}
                </button>
              )}
              <p className="fld__h" style={{ marginTop: "var(--s3)" }}>
                Mod loaders already skip anti-cheat for their own launches. This extends that
                to Steam's Play button so a modded session cannot start one by accident.
              </p>
            </>
          )}
        </Card>
      </div>

      <Card title="Machine">
        {report === null ? (
          <Skeleton variant="line" count={3} />
        ) : (
          <>
            <div className="row wrap" style={{ gap: "var(--s7)", marginBottom: "var(--s4)" }}>
              <div>
                <div className="stat__v">{report.cpuCores}</div>
                <div className="stat__k">cores</div>
              </div>
              <div>
                <div className="stat__v">{bytes(report.totalMemoryBytes)}</div>
                <div className="stat__k">memory</div>
              </div>
              <div>
                <div className="stat__v" style={{ fontSize: "var(--text-lg)" }}>
                  {report.steamRunning ? (
                    <span style={{ color: "var(--success)" }}>Running</span>
                  ) : (
                    <span className="faint">Closed</span>
                  )}
                </div>
                <div className="stat__k">steam</div>
              </div>
            </div>
            <p className="fld__h">
              {report.os} · {report.cpu}
            </p>
            <hr className="hr" />
            <div className="col2">
              {report.disks.map((disk) => {
                const used = disk.totalBytes - disk.availableBytes;
                const pct = disk.totalBytes > 0 ? (used / disk.totalBytes) * 100 : 0;
                return (
                  <div key={disk.mount}>
                    <div className="between" style={{ fontSize: "var(--text-xs)" }}>
                      <span className="mono">{disk.mount}</span>
                      <span className="faint">
                        {bytes(disk.availableBytes)} free of {bytes(disk.totalBytes)}
                      </span>
                    </div>
                    <div className="bar2" style={{ marginTop: 4 }}>
                      <div className="bar2__f" style={{ width: `${pct}%` }} />
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        )}
      </Card>

      <Card title="Installation">
        <div className="col2">
          <div className="between">
            <span className="mono faint truncate" style={{ maxWidth: "60%" }}>
              {install.root}
            </span>
            <div className="row" style={{ gap: "var(--s2)" }}>
              <button
                type="button"
                className="btn btn--s"
                onClick={() => void api.openPath(install.root)}
              >
                <Icon.Folder size={13} />
                Open
              </button>
              <button
                type="button"
                className="btn btn--s btn--bad"
                onClick={() => setConfirmForget(true)}
              >
                Forget
              </button>
            </div>
          </div>
          {install.markers.length > 0 && (
            <div className="row wrap" style={{ gap: 4 }}>
              {install.markers.map((marker) => (
                <Chip key={marker}>{marker}</Chip>
              ))}
            </div>
          )}
        </div>
      </Card>

      {confirmEac && eac && (
        <Confirm
          title={eac.state === "bypassed" ? "Restore anti-cheat?" : "Bypass anti-cheat?"}
          destructive={eac.state !== "bypassed"}
          confirmLabel={eac.state === "bypassed" ? "Restore" : "Bypass"}
          body={
            eac.state === "bypassed" ? (
              <p>
                The original launcher goes back. Steam's Play button will boot anti-cheat
                again, so remove your mods before playing online.
              </p>
            ) : (
              <>
                <p>
                  The anti-cheat launcher is renamed and the real executable takes its place.
                  Every way of starting the game then skips it.
                </p>
                <p className="fld__h">
                  Playing online with modified files risks a ban either way. This makes modded
                  offline play predictable; it is not a way to cheat online. The original is
                  kept and restorable from here.
                </p>
              </>
            )
          }
          onCancel={() => setConfirmEac(false)}
          onConfirm={async () => {
            const next = await toast.run(
              eac.state === "bypassed" ? "Anti-cheat restored" : "Anti-cheat bypassed",
              () => api.eacSet(gameId, eac.state === "bypassed"),
            );
            if (next) onEacChanged(next);
            setConfirmEac(false);
          }}
        />
      )}

      {confirmForget && (
        <Confirm
          title="Forget this installation?"
          confirmLabel="Forget"
          body={
            <>
              <p>
                Roundtable stops tracking <span className="mono">{install.root}</span>.
              </p>
              <p className="fld__h">
                Nothing is deleted. The game, its mods and its saves stay where they are.
              </p>
            </>
          }
          onCancel={() => setConfirmForget(false)}
          onConfirm={async () => {
            setConfirmForget(false);
            await onForget();
          }}
        />
      )}
    </div>
  );
}
