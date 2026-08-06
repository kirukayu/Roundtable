import { useCallback, useEffect, useMemo, useState } from "react";

import { Icon } from "../../components/Icons";
import { Card, Chip, CopyButton, Skeleton, Switch, useToast } from "../../components/ui";
import { api } from "../../lib/ipc";
import type { CoopSettings, FieldSpec, GameId, Installation } from "../../lib/types";

const PRESETS = [
  {
    name: "Default",
    values: {
      "SCALING.enemy_health_scaling": "35",
      "SCALING.enemy_damage_scaling": "0",
      "SCALING.enemy_posture_scaling": "15",
      "SCALING.boss_health_scaling": "100",
      "SCALING.boss_damage_scaling": "0",
      "SCALING.boss_posture_scaling": "20",
    },
  },
  {
    name: "Relaxed",
    values: {
      "SCALING.enemy_health_scaling": "10",
      "SCALING.enemy_damage_scaling": "0",
      "SCALING.enemy_posture_scaling": "5",
      "SCALING.boss_health_scaling": "40",
      "SCALING.boss_damage_scaling": "0",
      "SCALING.boss_posture_scaling": "10",
    },
  },
  {
    name: "Punishing",
    values: {
      "SCALING.enemy_health_scaling": "55",
      "SCALING.enemy_damage_scaling": "15",
      "SCALING.enemy_posture_scaling": "25",
      "SCALING.boss_health_scaling": "150",
      "SCALING.boss_damage_scaling": "20",
      "SCALING.boss_posture_scaling": "35",
    },
  },
];

export default function CoopPane({
  gameId,
  install,
}: {
  gameId: GameId;
  install: Installation;
}) {
  const toast = useToast();
  const [fields, setFields] = useState<FieldSpec[]>([]);
  const [settings, setSettings] = useState<CoopSettings | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    const [specs, current] = await Promise.all([api.coopFields(), api.coopRead(gameId)]);
    setFields(specs);
    setSettings(current);
    setDraft(current.values);
  }, [gameId]);

  useEffect(() => {
    void load();
  }, [load]);

  const dirty = useMemo(
    () =>
      settings
        ? Object.entries(draft).some(([key, value]) => settings.values[key] !== value)
        : false,
    [draft, settings],
  );

  const set = (key: string, value: string) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const save = async () => {
    setSaving(true);
    const changed = Object.fromEntries(
      Object.entries(draft).filter(([key, value]) => settings?.values[key] !== value),
    );
    const result = await toast.run("Co-op settings saved", () =>
      api.coopWrite(gameId, changed),
    );
    if (result) {
      setSettings(result);
      setDraft(result.values);
    }
    setSaving(false);
  };

  if (!settings) {
    return (
      <div className="g2">
        <Card><Skeleton variant="line" count={5} /></Card>
        <Card><Skeleton variant="line" count={5} /></Card>
      </div>
    );
  }

  const section = (name: string) => fields.filter((f) => f.section === name);
  const password = draft["PASSWORD.cooppassword"] ?? "";
  const extension = draft["SAVE.save_file_extension"] ?? "co2";
  const stem = "ER0000";

  return (
    <div className="col rev">
      <div className="between">
        <div className="row" style={{ gap: "var(--s2)" }}>
          {settings.installed ? (
            <Chip tone="ok">
              <Icon.Check size={11} />
              {settings.dllVersion ?? "Installed"}
            </Chip>
          ) : (
            <Chip tone="warn">Mod not installed</Chip>
          )}
          <Chip>
            {stem}.{extension}
          </Chip>
        </div>
        <button
          type="button"
          className="btn btn--solid btn--sm"
          onClick={save}
          disabled={!dirty || saving}
        >
          {saving ? <span className="spin" /> : <Icon.Check size={14} />}
          {dirty ? "Save" : "Saved"}
        </button>
      </div>

      {!settings.installed && (
        <div className="note note--warn">
          <Icon.Warning size={15} />
          <div>
            <div className="note__t">Seamless Co-op is not in this installation</div>
            <div className="note__b">
              Extract it into <span className="mono">{install.gameDir}</span>. Settings you
              change here are still written and take effect once the mod is present.
            </div>
          </div>
        </div>
      )}

      <div className="g2">
        <Card title="Session">
          <div className="fld">
            <label className="fld__l" htmlFor="cpw">Password</label>
            <div className="row" style={{ gap: "var(--s2)" }}>
              <input
                id="cpw"
                className="in mono"
                value={password}
                placeholder="empty means solo"
                onChange={(e) => set("PASSWORD.cooppassword", e.target.value)}
              />
              <button
                type="button"
                className="btn btn--icon"
                aria-label="Generate"
                onClick={async () => set("PASSWORD.cooppassword", await api.coopGeneratePassword())}
              >
                <Icon.Refresh size={14} />
              </button>
              {password && <CopyButton text={password} />}
            </div>
            <span className="fld__h">
              Everyone in the session needs the same password.
            </span>
          </div>

          <hr className="hr" />

          <div className="fld">
            <label className="fld__l" htmlFor="cext">Save extension</label>
            <input
              id="cext"
              className="in mono"
              style={{ maxWidth: 160 }}
              value={extension}
              onChange={(e) => set("SAVE.save_file_extension", e.target.value)}
            />
            <span className="fld__h">
              Co-op writes <span className="mono">{stem}.{extension}</span> instead of the
              vanilla save. Changing it starts a fresh set of characters.
            </span>
          </div>
        </Card>

        <Card
          title="Scaling"
          action={
            <div className="row" style={{ gap: 4 }}>
              {PRESETS.map((preset) => (
                <button
                  key={preset.name}
                  type="button"
                  className="btn btn--ghost btn--sm"
                  onClick={() => setDraft((current) => ({ ...current, ...preset.values }))}
                >
                  {preset.name}
                </button>
              ))}
            </div>
          }
        >
          <p className="fld__h" style={{ marginBottom: "var(--s3)" }}>
            Added per extra player.
          </p>
          {section("SCALING").map((spec) => (
            <Field
              key={spec.key}
              spec={spec}
              value={draft[`${spec.section}.${spec.key}`] ?? spec.default}
              onChange={(value) => set(`${spec.section}.${spec.key}`, value)}
            />
          ))}
        </Card>
      </div>

      <Card title="Gameplay">
        {section("GAMEPLAY").map((spec) => (
          <Field
            key={spec.key}
            spec={spec}
            value={draft[`${spec.section}.${spec.key}`] ?? spec.default}
            onChange={(value) => set(`${spec.section}.${spec.key}`, value)}
          />
        ))}
      </Card>

      {section("LANGUAGE").length > 0 && (
        <Card title="Language">
          {section("LANGUAGE").map((spec) => (
            <Field
              key={spec.key}
              spec={spec}
              value={draft[`${spec.section}.${spec.key}`] ?? spec.default}
              onChange={(value) => set(`${spec.section}.${spec.key}`, value)}
            />
          ))}
        </Card>
      )}
    </div>
  );
}

function Field({
  spec,
  value,
  onChange,
}: {
  spec: FieldSpec;
  value: string;
  onChange: (next: string) => void;
}) {
  if (spec.kind === "toggle") {
    return (
      <div className="opt">
        <div className="opt__t">
          <div className="opt__l">{spec.label}</div>
          <div className="opt__h">{spec.help}</div>
        </div>
        <Switch
          checked={value === "1"}
          label={spec.label}
          onChange={(next) => onChange(next ? "1" : "0")}
        />
      </div>
    );
  }

  if (spec.kind === "choice") {
    return (
      <div className="opt">
        <div className="opt__t">
          <div className="opt__l">{spec.label}</div>
          <div className="opt__h">{spec.help}</div>
        </div>
        <select
          className="sel2"
          style={{ width: 180 }}
          value={value}
          aria-label={spec.label}
          onChange={(e) => onChange(e.target.value)}
        >
          {spec.options.map(([option, caption]) => (
            <option key={option} value={String(option)}>
              {caption}
            </option>
          ))}
        </select>
      </div>
    );
  }

  if (spec.kind === "range") {
    return (
      <div className="opt">
        <div className="opt__t">
          <div className="opt__l">{spec.label}</div>
          <div className="opt__h">{spec.help}</div>
        </div>
        <div className="row" style={{ gap: "var(--s3)", width: 200 }}>
          <input
            type="range"
            className="rng"
            min={spec.min ?? 0}
            max={spec.max ?? 100}
            value={value}
            aria-label={spec.label}
            onChange={(e) => onChange(e.target.value)}
          />
          <span
            className="mono"
            style={{ width: 34, textAlign: "right", color: "var(--accent)" }}
          >
            {value}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className="fld" style={{ padding: "var(--s3) 0" }}>
      <label className="fld__l" htmlFor={`c-${spec.key}`}>
        {spec.label}
      </label>
      <input
        id={`c-${spec.key}`}
        className="in"
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
      <span className="fld__h">{spec.help}</span>
    </div>
  );
}
