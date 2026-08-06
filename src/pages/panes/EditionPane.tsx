import { useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icons";
import { Blank, Card, Chip, NoticeBlock, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import { bytes } from "../../lib/format";
import type { EditionJob, EditionStatus, GameId, Installation } from "../../lib/types";

/**
 * One total conversion: install it, wire it up, start it.
 *
 * The mod ships a batch file that calls `me3 launch --auto-detect`, and
 * `--auto-detect` resolves the game through Steam. On a repack the lookup fails,
 * the game never starts, and thirty seconds later the batch file tells you to
 * buy the game on Steam. Roundtable passes `--exe` and `--skip-steam-init`
 * instead, which is the entire difference.
 */
export default function EditionPane({
  game,
  install,
  status,
  coop,
  onCoop,
  onChanged,
}: {
  game: GameId;
  install: Installation;
  status: EditionStatus;
  coop: boolean;
  onCoop: (next: boolean) => void;
  onChanged: () => Promise<void>;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [job, setJob] = useState<EditionJob | null>(null);
  const polling = useRef<number | null>(null);

  const { spec, install: edition, plan } = status;

  // While an archive is unpacking the only honest thing to show is how far it
  // has got, so the pane polls until the thread reports it finished.
  useEffect(() => {
    const stop = () => {
      if (polling.current !== null) {
        window.clearInterval(polling.current);
        polling.current = null;
      }
    };

    const tick = async () => {
      try {
        const next = await api.editionJob();
        setJob(next.running || next.done ? next : null);
        if (!next.running && next.done) {
          stop();
          if (next.error) toast.error("Unpacking failed", next.error);
          else toast.success(`${spec.name} installed`, next.message);
          await onChanged();
        }
      } catch {
        stop();
      }
    };

    void tick();
    polling.current = window.setInterval(tick, 700);
    return stop;
  }, [spec.name, onChanged, toast]);

  /**
   * Point at the archive; everything else is worked out.
   *
   * The destination used to be a second prompt, which asked the user to know
   * something Roundtable already knows: the mod goes beside the game, never
   * inside it, on the same drive. It is shown below rather than asked for.
   */
  const chooseArchive = async () => {
    const archive = await api.pickFile(`Where is the ${spec.short} archive?`, "zip;7z");
    if (!archive) return;

    setBusy(true);
    try {
      await api.editionInstall(game, spec.id, archive);
      toast.info("Unpacking", "Ten gigabytes, so a few minutes.");
    } catch (error) {
      toast.error("Could not start", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  /** For when it is already unpacked somewhere Roundtable did not look. */
  const chooseFolder = async () => {
    const path = await api.pickFolder(`Where is ${spec.short} installed?`);
    if (!path) return;
    const found = await toast.run(`${spec.short} found`, () => api.editionLocate(spec.id, path));
    if (found) await onChanged();
  };

  /**
   * Searches every drive for an existing copy.
   *
   * The rings around the game cover where the mod usually goes; this covers
   * where it went instead.
   */
  const [scanning, setScanning] = useState(false);
  const [scanAt, setScanAt] = useState("");

  const scan = async () => {
    try {
      await api.editionScan(game, spec.id);
      setScanning(true);
    } catch (error) {
      toast.error("Could not search", error instanceof Error ? error.message : String(error));
    }
  };

  useEffect(() => {
    if (!scanning) return;
    const timer = window.setInterval(async () => {
      try {
        const state = await api.installsScanState();
        setScanAt(state.at);
        if (!state.running) {
          window.clearInterval(timer);
          setScanning(false);
          setScanAt("");
          await onChanged();
        }
      } catch {
        window.clearInterval(timer);
        setScanning(false);
      }
    }, 500);
    return () => window.clearInterval(timer);
  }, [scanning, onChanged]);

  const locate = async () => {
    const path = await api.pickFolder(`Where is ${spec.short} installed?`);
    if (!path) return;
    const found = await toast.run(`${spec.short} found`, () =>
      api.editionLocate(spec.id, path),
    );
    if (found) await onChanged();
  };

  const patch = async () => {
    setBusy(true);
    try {
      const report = await api.editionPatch(game, spec.id, coop);
      toast.success("Patched", report.changes.join(" · "));
      await onChanged();
    } catch (error) {
      toast.error("Patch failed", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  const play = async () => {
    setBusy(true);
    try {
      await api.editionRun(game, spec.id, coop);
      toast.success(`${spec.short} started`, coop ? "With Seamless Co-op" : "Solo");
    } catch (error) {
      toast.error("Could not start", error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  };

  if (job?.running) {
    const pct = job.bytesTotal > 0 ? (job.bytesDone / job.bytesTotal) * 100 : 0;
    return (
      <Card title="Unpacking">
        <div className="col2">
          <div className="between">
            <span className="w3">{job.message}</span>
            <span className="mono">{pct.toFixed(1)}%</span>
          </div>
          <div className="bar2">
            <div className="bar2__f" style={{ width: `${pct}%` }} />
          </div>
          <div className="between">
            <span className="mono w4">
              {job.filesDone} / {job.filesTotal} files
            </span>
            <span className="mono w4">
              {bytes(job.bytesDone)} / {bytes(job.bytesTotal)}
            </span>
          </div>
        </div>
      </Card>
    );
  }

  if (!edition) {
    return (
      <div className="col">
        <Card>
          <Blank
            icon={Icon.Box}
            title={`${spec.name} is not installed`}
            action={
              <div className="row wrap" style={{ gap: "var(--s3)" }}>
                <button
                  type="button"
                  className="btn btn--solid"
                  onClick={chooseArchive}
                  disabled={busy || scanning}
                >
                  {busy ? <span className="spin" /> : null}
                  Choose the archive
                </button>
                <button type="button" className="btn" onClick={scan} disabled={scanning}>
                  {scanning ? <span className="spin" /> : null}
                  {scanning ? "Searching" : "Already have it"}
                </button>
                <button type="button" className="btn btn--ghost" onClick={chooseFolder} disabled={scanning}>
                  Point at the folder
                </button>
              </div>
            }
          >
            {scanning ? (
              <>
                Searching every drive for {spec.short}.
                <br />
                <span className="mono truncate w4" style={{ display: "block", marginTop: "var(--s3)", fontSize: "var(--t-2xs)" }}>
                  {scanAt || "…"}
                </span>
              </>
            ) : (
              <>
                Pick the zip you downloaded. Roundtable unpacks it, wires up
                Seamless Co-op and starts the game without Steam. Around ten
                gigabytes.
              </>
            )}
          </Blank>

          <hr className="hr" />

          <div className="col2">
            <div className="between">
              <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
                It will go here
              </span>
              <span className="mono truncate w4" style={{ fontSize: "var(--t-xs)", maxWidth: "60%" }}>
                {status.suggestedDestination || "beside the game"}
              </span>
            </div>
            <div className="between">
              <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
                Get it from
              </span>
              <a
                className="mono"
                style={{ fontSize: "var(--t-xs)", color: "var(--w2)" }}
                href={spec.site}
                target="_blank"
                rel="noreferrer noopener"
              >
                {spec.site.replace("https://", "").replace(/\/$/, "")}
              </a>
            </div>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className="col">
      {plan?.notices.map((notice, index) => (
        <NoticeBlock key={index} notice={notice} />
      ))}

      <div className="g2">
        <Card
          title="This launch"
          action={
            <button type="button" className="btn btn--ghost btn--sm" onClick={patch} disabled={busy}>
              Patch
            </button>
          }
        >
          <div className="row wrap" style={{ gap: "var(--s2)", marginBottom: "var(--s4)" }}>
            <Chip tone="solid">me3</Chip>
            {coop && <Chip tone="ok">Seamless Co-op</Chip>}
            {plan?.skipSteamInit && <Chip>skip-steam-init</Chip>}
          </div>

          <ol className="col2" style={{ margin: 0, paddingLeft: 18, fontSize: "var(--t-sm)" }}>
            {plan?.steps.map((step, index) => (
              <li key={index} className="w3">
                {step}
              </li>
            ))}
          </ol>

          <hr className="hr" />

          <label className="opt" style={{ borderBottom: 0, padding: 0 }}>
            <span className="opt__t">
              <span className="opt__l">Seamless Co-op</span>
            </span>
            <button
              type="button"
              className="sw"
              role="switch"
              aria-checked={coop}
              onClick={() => onCoop(!coop)}
            />
          </label>
        </Card>

        <Card title={spec.short}>
          <div className="col2">
            <Line k="Version" v={edition.version ?? "unknown"} />
            <Line k="Save file" v={coop ? spec.savefileCoop : spec.savefile} mono />
            <Line k="Loader" v={edition.me3 ? "bundled me3" : "missing"} />
            <Line k="Co-op DLL" v={edition.coopDll ? "in place" : "not copied yet"} />
          </div>

          <hr className="hr" />

          <div className="col2">
            <div className="between">
              <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
                Folder
              </span>
              <button
                type="button"
                className="btn btn--ghost btn--sm"
                onClick={() => void api.openPath(edition.root)}
                title={edition.root}
              >
                <Icon.Folder size={13} />
                <span className="mono truncate" style={{ maxWidth: 240 }}>
                  {edition.root}
                </span>
              </button>
            </div>
            <div className="between">
              <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
                Game
              </span>
              <span className="mono truncate w4" style={{ maxWidth: 240, fontSize: "var(--t-xs)" }}>
                {install.executable}
              </span>
            </div>
          </div>
        </Card>
      </div>

      <div className="row" style={{ gap: "var(--s3)" }}>
        <button
          type="button"
          className="btn btn--solid btn--lg"
          onClick={play}
          disabled={busy || !plan || plan.notices.some((n) => n.severity === "blocker")}
        >
          {busy ? <span className="spin" /> : null}
          Play {spec.short}
        </button>
        <button type="button" className="btn btn--lg" onClick={locate}>
          Change folder
        </button>
      </div>
    </div>
  );
}

function Line({ k, v, mono }: { k: string; v: string; mono?: boolean }) {
  return (
    <div className="between">
      <span className="w3" style={{ fontSize: "var(--t-sm)" }}>
        {k}
      </span>
      <span
        className={mono ? "mono truncate" : "truncate"}
        style={{ fontSize: "var(--t-sm)", maxWidth: "62%", textAlign: "right" }}
      >
        {v}
      </span>
    </div>
  );
}
