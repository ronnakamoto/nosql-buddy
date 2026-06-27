import { useState } from "react";
import type { ScheduleConfig } from "../../ipc/commands";

interface SchedulePanelProps {
  value: ScheduleConfig | null;
  onChange: (value: ScheduleConfig | null) => void;
}

const PRESETS: { label: string; cron: string }[] = [
  { label: "Daily at 2 AM", cron: "0 2 * * *" },
  { label: "Weekly on Sunday at 2 AM", cron: "0 2 * * 0" },
  { label: "Monthly 1st at 2 AM", cron: "0 2 1 * *" },
];

export function SchedulePanel({ value, onChange }: SchedulePanelProps) {
  const [enabled, setEnabled] = useState(value?.enabled ?? false);
  const [cron, setCron] = useState(value?.cron ?? PRESETS[0].cron);
  const [retention, setRetention] = useState(value?.retentionCount ?? 5);
  const [custom, setCustom] = useState(false);

  const apply = () => {
    if (!enabled) {
      onChange(null);
      return;
    }
    onChange({
      cron,
      enabled: true,
      retentionCount: retention,
      nextRunAt: null,
    });
  };

  return (
    <div className="schedule-panel" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <label className="row" style={{ gap: 8, alignItems: "center" }}>
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => {
            setEnabled(e.target.checked);
            if (!e.target.checked) onChange(null);
          }}
        />
        <span className="field__label" style={{ margin: 0 }}>
          Enable recurring schedule
        </span>
      </label>

      {enabled && (
        <>
          <div className="field">
            <label className="field__label">Frequency</label>
            <select
              className="field__select"
              value={custom ? "__custom__" : cron}
              onChange={(e) => {
                const v = e.target.value;
                if (v === "__custom__") {
                  setCustom(true);
                } else {
                  setCustom(false);
                  setCron(v);
                }
              }}
            >
              {PRESETS.map((p) => (
                <option key={p.cron} value={p.cron}>
                  {p.label}
                </option>
              ))}
              <option value="__custom__">Custom cron…</option>
            </select>
          </div>

          {custom && (
            <div className="field">
              <label className="field__label">Cron expression</label>
              <input
                className="field__input"
                value={cron}
                onChange={(e) => setCron(e.target.value)}
                placeholder="0 2 * * *"
              />
            </div>
          )}

          <div className="field">
            <label className="field__label">Retention</label>
            <div className="row" style={{ gap: 8, alignItems: "center" }}>
              <span>Keep last</span>
              <input
                type="number"
                className="field__input"
                value={retention}
                onChange={(e) => setRetention(Math.max(1, parseInt(e.target.value, 10) || 1))}
                style={{ width: 64 }}
              />
              <span>backups</span>
            </div>
          </div>

          <button className="btn btn--sm btn--ghost" onClick={apply}>
            Apply schedule
          </button>
        </>
      )}
    </div>
  );
}
