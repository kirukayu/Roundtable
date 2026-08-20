import { Modal, Option } from "../components/ui";
import type { GameId, Settings } from "../lib/types";

/**
 * Settings as an overlay rather than a destination.
 *
 * Nothing in here is part of playing a game, so it never gets a slot in the
 * navigation beside the catalogue. There is no theme picker: the interface is
 * monochrome by design and offering to break that would be offering to make it
 * worse.
 */
export default function SettingsOverlay({
  settings,
  onPatch,
  onClose,
}: {
  settings: Settings;
  onPatch: (patch: Partial<Settings>) => Promise<void>;
  onClose: () => void;
}) {
  return (
    <Modal title="Settings" onClose={onClose}>
      <div className="col">
        <section>
          <div className="card__t" style={{ marginBottom: "var(--s3)" }}>
            Saves
          </div>
          <Option
            label="Snapshot before every launch"
            help="The single setting most likely to save a character."
            checked={settings.autoBackupOnLaunch}
            onChange={(next) => void onPatch({ autoBackupOnLaunch: next })}
          />
          <div className="opt">
            <div className="opt__t">
              <div className="opt__l">Automatic snapshots kept</div>
              <div className="opt__h">Manual ones are never pruned.</div>
            </div>
            <div className="row" style={{ gap: "var(--s4)", width: 190 }}>
              <input
                type="range"
                className="rng"
                min={5}
                max={100}
                step={5}
                value={settings.autoBackupKeep}
                aria-label="Automatic snapshots kept"
                onChange={(e) => void onPatch({ autoBackupKeep: Number(e.target.value) })}
              />
              <span className="mono" style={{ width: 26, color: "var(--w1)" }}>
                {settings.autoBackupKeep}
              </span>
            </div>
          </div>
          <Option
            label="Confirm destructive actions"
            checked={settings.confirmDestructive}
            onChange={(next) => void onPatch({ confirmDestructive: next })}
          />
        </section>

        <hr className="hr" />

        <section>
          <div className="card__t" style={{ marginBottom: "var(--s3)" }}>
            Interface
          </div>
          <Option
            label="Reduce motion"
            help="Stills the fog and removes every transition."
            checked={settings.reduceMotion}
            onChange={(next) => void onPatch({ reduceMotion: next })}
          />
        </section>

        <hr className="hr" />

        <section>
          <div className="card__t" style={{ marginBottom: "var(--s3)" }}>
            Integrations
          </div>
          <Option
            label="Discord Rich Presence"
            help="Shows what you are playing. Never reports which mods you use."
            checked={settings.discordPresence}
            onChange={(next) => void onPatch({ discordPresence: next })}
          />
          <div className="fld" style={{ paddingTop: "var(--s4)" }}>
            <label className="fld__l" htmlFor="nexus">
              Nexus Mods API key
            </label>
            <input
              id="nexus"
              className="in mono"
              type="password"
              placeholder="paste from your Nexus account page"
              value={settings.nexusApiKey ?? ""}
              onChange={(e) => void onPatch({ nexusApiKey: e.target.value || null })}
            />
            <span className="fld__h">
              Stored on this machine and sent only to api.nexusmods.com.
            </span>
          </div>
        </section>

        <hr className="hr" />

        <section>
          <div className="card__t" style={{ marginBottom: "var(--s3)" }}>
            Advanced
          </div>
          <Option
            label="Deploy profiles as a junction"
            help="Off by default. Only needed for older tools that require a literal mod folder beside the game."
            checked={settings.useJunctionDeploy}
            onChange={(next) => void onPatch({ useJunctionDeploy: next })}
          />
          <div className="opt">
            <div className="opt__t">
              <div className="opt__l">Download connections</div>
              <div className="opt__h">Parallel ranges per transfer.</div>
            </div>
            <div className="row" style={{ gap: "var(--s4)", width: 190 }}>
              <input
                type="range"
                className="rng"
                min={1}
                max={16}
                value={settings.downloadConnections}
                aria-label="Download connections"
                onChange={(e) => void onPatch({ downloadConnections: Number(e.target.value) })}
              />
              <span className="mono" style={{ width: 26, color: "var(--w1)" }}>
                {settings.downloadConnections}
              </span>
            </div>
          </div>
          <Option
            label="Resolve through DNS over HTTPS"
            help="Helps when a network answers mod-host lookups with a block page."
            checked={settings.useDoh}
            onChange={(next) => void onPatch({ useDoh: next })}
          />
        </section>

        {settings.installations.length > 0 && (
          <>
            <hr className="hr" />
            <section>
              <div className="card__t" style={{ marginBottom: "var(--s3)" }}>
                Known installations
              </div>
              <div className="rows">
                {settings.installations.map((entry) => (
                  <div key={`${entry.game}-${entry.root}`} className="rw">
                    <div className="grow" style={{ minWidth: 0 }}>
                      <div className="rw__t">{label(entry.game)}</div>
                      <div className="rw__s mono truncate">{entry.root}</div>
                    </div>
                    {entry.isDefault && <span className="chip">Default</span>}
                  </div>
                ))}
              </div>
            </section>
          </>
        )}

        <hr className="hr" />

        <div className="between">
          <div>
            <div className="opt__l">Roundtable 0.1.0</div>
            <div className="opt__h">MIT licensed. No account, no telemetry.</div>
          </div>
        </div>
      </div>
    </Modal>
  );
}

function label(id: GameId): string {
  switch (id) {
    case "elden-ring":
      return "Elden Ring";
    case "nightreign":
      return "Nightreign";
    case "dark-souls-remastered":
      return "Dark Souls Remastered";
    case "dark-souls2":
      return "Dark Souls II";
    case "dark-souls3":
      return "Dark Souls III";
    case "sekiro":
      return "Sekiro";
    case "armored-core6":
      return "Armored Core VI";
    case "bloodborne":
      return "Bloodborne";
    case "demons-souls":
      return "Demon's Souls";
  }
}
