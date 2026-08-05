import { Icon } from "../components/Icons";
import { Blank, Card } from "../components/ui";

/**
 * Downloads. Nothing is queued yet, so the page explains what will appear here and
 * offers the two ways to start one rather than showing an empty table.
 */
export default function Downloads() {
  return (
    <div className="view pad">
      <div className="section-head">
        <h2>Downloads</h2>
      </div>

      <Card>
        <Blank icon={Icon.Download} title="Nothing downloading">
          Mods you fetch from Nexus, and any direct link you paste, land here with resumable
          multi-connection transfers. Start one from a game's Mods tab.
        </Blank>
      </Card>
    </div>
  );
}
