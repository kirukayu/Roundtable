import { Icon } from "../../components/Icons";
import { Blank, Card, Chip, NoticeBlock, Skeleton } from "../../components/ui";
import { api } from "../../lib/ipc";
import { when } from "../../lib/format";
import type { GameInfo, Installation, PreparedLaunch, Profile } from "../../lib/types";

/**
 * What will happen when Play is pressed, spelled out before it happens.
 *
 * Anything that would stop the launch is at the top; the rest is a short summary
 * rather than a settings screen, because settings belong on the tab that owns them.
 */
export default function PlayPane({
  game,
  install,
  profile,
  prepared,
  onCreateProfile,
  onManageMods,
  onPatch,
}: {
  game: GameInfo;
  install: Installation;
  profile: Profile | null;
  prepared: PreparedLaunch | null;
  onCreateProfile: () => void;
  onManageMods: () => void;
  onPatch: () => Promise<void>;
}) {
  if (!profile) {
    return (
      <Card>
        <Blank
          icon={Icon.Layers}
          title="No profile yet"
          action={
            <button type="button" className="btn btn--a" onClick={onCreateProfile}>
              <Icon.Plus size={15} />
              Create one
            </button>
          }
        >
          A profile is a load order plus the options that belong with it. One for vanilla,
          one for your overhaul, and switching between them moves no files.
        </Blank>
      </Card>
    );
  }

  const enabled = profile.mods.filter((m) => m.enabled).length;

  return (
    <div className="col rev">
      {prepared?.plan.notices.map((notice, index) => (
        <NoticeBlock key={index} notice={notice} />
      ))}

      <div className="g2">
        <Card
          title="This launch"
          action={
            <button type="button" className="btn btn--g btn--s" onClick={onPatch}>
              Write config
            </button>
          }
        >
          {prepared ? (
            <>
              <div className="row wrap" style={{ gap: "var(--s2)", marginBottom: "var(--s4)" }}>
                <Chip tone="a">{routeName(prepared.plan.route)}</Chip>
                {prepared.plan.coopEnabled && <Chip tone="ok">Co-op</Chip>}
                {prepared.plan.skipSteamInit && <Chip>skip-steam-init</Chip>}
              </div>
              <ol className="col2" style={{ margin: 0, paddingLeft: 18, fontSize: "var(--t-sm)" }}>
                {prepared.plan.steps.map((step, index) => (
                  <li key={index} className="dim">
                    {step}
                  </li>
                ))}
              </ol>
            </>
          ) : (
            <Skeleton variant="line" count={3} />
          )}
        </Card>

        <Card
          title="Profile"
          action={
            <button type="button" className="btn btn--g btn--s" onClick={onManageMods}>
              Manage
            </button>
          }
        >
          <div className="col2">
            <Line k="Name" v={profile.name} />
            <Line k="Mods enabled" v={String(enabled)} />
            <Line k="Save file" v={profile.savefile ?? "shared with vanilla"} mono />
            <Line k="Last played" v={when(profile.lastPlayed)} />
          </div>

          <hr className="hr" />

          <div className="row wrap" style={{ gap: "var(--s2)" }}>
            {profile.seamlessCoop && <Chip tone="ok">Seamless Co-op</Chip>}
            {profile.disableArxan && <Chip>Arxan off</Chip>}
            {profile.memPatch && <Chip>Memory patched</Chip>}
            {profile.skipLogos && <Chip>Logos skipped</Chip>}
            {profile.startOnline && <Chip tone="bad">Online</Chip>}
          </div>
        </Card>
      </div>

      <Card title="Where things live">
        <div className="col2">
          <div className="between">
            <span className="dim" style={{ fontSize: "var(--t-sm)" }}>
              Game
            </span>
            <button
              type="button"
              className="btn btn--g btn--s"
              onClick={() => void api.openPath(install.root)}
              title={install.root}
            >
              <Icon.Folder size={13} />
              <span className="mono truncate" style={{ maxWidth: 260 }}>
                {install.root}
              </span>
            </button>
          </div>
          <Line
            k="Type"
            v={
              install.kind === "steam"
                ? "Steam"
                : install.kind === "standalone"
                  ? "Standalone"
                  : "Unrecognised"
            }
          />
          <Line k="Executable" v={game.executable} mono />
          {install.markers.length > 0 && (
            <Line k="Detected" v={install.markers.join(", ")} mono />
          )}
        </div>
      </Card>
    </div>
  );
}

function Line({ k, v, mono }: { k: string; v: string; mono?: boolean }) {
  return (
    <div className="between">
      <span className="dim" style={{ fontSize: "var(--t-sm)" }}>
        {k}
      </span>
      <span
        className={mono ? "mono truncate" : "truncate"}
        style={{ fontSize: "var(--t-sm)", fontWeight: 600, maxWidth: "62%", textAlign: "right" }}
      >
        {v}
      </span>
    </div>
  );
}

function routeName(route: string): string {
  switch (route) {
    case "me3":
      return "me3";
    case "mod-engine2":
      return "ModEngine 2";
    case "seamless-coop-launcher":
      return "Co-op launcher";
    default:
      return "Direct";
  }
}
