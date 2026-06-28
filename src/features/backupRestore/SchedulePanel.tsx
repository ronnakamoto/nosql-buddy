import { useCallback, useEffect, useMemo, useState } from "react";
import type { ScheduleConfig } from "../../ipc/commands";

interface SchedulePanelProps {
  value: ScheduleConfig | null;
  onChange: (value: ScheduleConfig | null) => void;
}

type Frequency = "hourly" | "daily" | "weekdays" | "weekly" | "monthly";

interface Preset {
  frequency: Frequency;
  label: string;
  makeCron: (hour: number, minute: number) => string;
}

// The Rust `cron` crate expects 6 fields: sec min hour day month dow.
// All generated expressions include an explicit `0` for seconds.
const PRESETS: Preset[] = [
  {
    frequency: "hourly",
    label: "Every hour",
    makeCron: () => "0 0 * * * *",
  },
  {
    frequency: "daily",
    label: "Every day",
    makeCron: (h, m) => `0 ${m} ${h} * * *`,
  },
  {
    frequency: "weekdays",
    label: "Every weekday",
    makeCron: (h, m) => `0 ${m} ${h} * * 1-5`,
  },
  {
    frequency: "weekly",
    label: "Every Monday",
    makeCron: (h, m) => `0 ${m} ${h} * * 1`,
  },
  {
    frequency: "monthly",
    label: "Every 1st of month",
    makeCron: (h, m) => `0 ${m} ${h} 1 * *`,
  },
];

function parseCron(cron: string): { frequency: Frequency; hour: number; minute: number } | null {
  const parts = cron.split(" ");
  if (parts.length !== 6) return null;
  const [secStr, minStr, hourStr, day, month, dow] = parts;
  const minute = parseInt(minStr, 10);
  const hour = parseInt(hourStr, 10);
  if (Number.isNaN(minute) || Number.isNaN(hour)) return null;

  if (secStr === "0" && minStr === "0" && hourStr === "*" && day === "*" && month === "*" && dow === "*") {
    return { frequency: "hourly", hour: 9, minute: 0 };
  }
  if (secStr === "0" && day === "*" && month === "*" && dow === "*") {
    return { frequency: "daily", hour, minute };
  }
  if (secStr === "0" && day === "*" && month === "*" && dow === "1-5") {
    return { frequency: "weekdays", hour, minute };
  }
  if (secStr === "0" && day === "*" && month === "*" && dow === "1") {
    return { frequency: "weekly", hour, minute };
  }
  if (secStr === "0" && day === "1" && month === "*" && dow === "*") {
    return { frequency: "monthly", hour, minute };
  }
  return null;
}

function pad2(n: number): string {
  return n.toString().padStart(2, "0");
}

export function SchedulePanel({ value, onChange }: SchedulePanelProps) {
  const enabled = value?.enabled ?? false;

  // Parse incoming cron, or default to daily at 9:00
  const parsed = useMemo(() => {
    if (value?.cron) {
      const p = parseCron(value.cron);
      if (p) return p;
    }
    return { frequency: "daily" as Frequency, hour: 9, minute: 0 };
  }, [value?.cron]);

  const [frequency, setFrequency] = useState<Frequency>(parsed.frequency);
  const [hour, setHour] = useState<number>(parsed.hour);
  const [minute, setMinute] = useState<number>(parsed.minute);
  const [retention, setRetention] = useState<number>(value?.retentionCount ?? 5);
  const [customCron, setCustomCron] = useState<string | null>(null);

  // If the value prop changes from outside, sync local state
  useEffect(() => {
    setFrequency(parsed.frequency);
    setHour(parsed.hour);
    setMinute(parsed.minute);
    setRetention(value?.retentionCount ?? 5);
  }, [value?.cron, value?.retentionCount, parsed.frequency, parsed.hour, parsed.minute]);

  const buildConfig = useCallback(
    (opts: {
      enabled: boolean;
      frequency: Frequency;
      hour: number;
      minute: number;
      retention: number;
      customCron: string | null;
    }): ScheduleConfig | null => {
      if (!opts.enabled) return null;
      const preset = PRESETS.find((p) => p.frequency === opts.frequency);
      const cron = preset ? preset.makeCron(opts.hour, opts.minute) : (opts.customCron ?? "0 0 9 * * *");
      return {
        cron,
        enabled: true,
        retentionCount: opts.retention,
        nextRunAt: null,
      };
    },
    [],
  );

  const emit = useCallback(
    (patch: Partial<{
      enabled: boolean;
      frequency: Frequency;
      hour: number;
      minute: number;
      retention: number;
      customCron: string | null;
    }>) => {
      const nextEnabled = patch.enabled ?? enabled;
      const nextFrequency = patch.frequency ?? frequency;
      const nextHour = patch.hour ?? hour;
      const nextMinute = patch.minute ?? minute;
      const nextRetention = patch.retention ?? retention;
      const nextCustomCron = patch.customCron !== undefined ? patch.customCron : customCron;
      onChange(buildConfig({
        enabled: nextEnabled,
        frequency: nextFrequency,
        hour: nextHour,
        minute: nextMinute,
        retention: nextRetention,
        customCron: nextCustomCron,
      }));
    },
    [enabled, frequency, hour, minute, retention, customCron, buildConfig, onChange],
  );

  const showTimePicker = frequency !== "hourly";

  return (
    <div className="schedule-panel" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <label className="row" style={{ gap: 8, alignItems: "center" }}>
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => emit({ enabled: e.target.checked })}
        />
        <span className="field__label" style={{ margin: 0 }}>
          Run this automatically
        </span>
      </label>

      {enabled && (
        <>
          <div className="field">
            <label className="field__label">Frequency</label>
            <select
              className="field__select"
              value={customCron ? "__custom__" : frequency}
              onChange={(e) => {
                const v = e.target.value;
                if (v === "__custom__") {
                  setCustomCron(value?.cron ?? "0 0 9 * * *");
                  emit({ customCron: value?.cron ?? "0 0 9 * * *" });
                } else {
                  setCustomCron(null);
                  emit({ frequency: v as Frequency });
                }
              }}
            >
              {PRESETS.map((p) => (
                <option key={p.frequency} value={p.frequency}>
                  {p.label}
                </option>
              ))}
              <option value="__custom__">Custom cron…</option>
            </select>
          </div>

          {customCron && (
            <div className="field">
              <label className="field__label">Cron expression</label>
              <input
                className="field__input"
                value={customCron}
                onChange={(e) => {
                  setCustomCron(e.target.value);
                  emit({ customCron: e.target.value });
                }}
                placeholder="0 0 9 * * *"
              />
            </div>
          )}

          {showTimePicker && !customCron && (
            <div className="field">
              <label className="field__label">Time</label>
              <div className="row" style={{ gap: 8, alignItems: "center" }}>
                <select
                  className="field__select"
                  value={hour}
                  onChange={(e) => emit({ hour: parseInt(e.target.value, 10) })}
                  style={{ width: 80 }}
                >
                  {Array.from({ length: 24 }, (_, i) => (
                    <option key={i} value={i}>
                      {pad2(i)}
                    </option>
                  ))}
                </select>
                <span style={{ color: "var(--ink-muted)" }}>:</span>
                <select
                  className="field__select"
                  value={minute}
                  onChange={(e) => emit({ minute: parseInt(e.target.value, 10) })}
                  style={{ width: 80 }}
                >
                  {Array.from({ length: 60 }, (_, i) => (
                    <option key={i} value={i}>
                      {pad2(i)}
                    </option>
                  ))}
                </select>
              </div>
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
                min={1}
                max={100}
                onChange={(e) => {
                  const n = Math.max(1, Math.min(100, parseInt(e.target.value, 10) || 1));
                  emit({ retention: n });
                }}
                style={{ width: 64 }}
              />
              <span>backups</span>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
