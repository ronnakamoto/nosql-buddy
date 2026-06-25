import { useCallback } from "react";
import type { AuditMode } from "../ipc/commands";
import { AuditModeChooser } from "./AuditModeChooser";
import { AuditDevFlow } from "./AuditDevFlow";
import { AuditProductionFlow } from "./AuditProductionFlow";
import { AuditSettings } from "./AuditSettings";
import { injectAuditKeyframes } from "./AuditUi";

/**
 * ZK Audit Log panel — state is owned by the App tab so it survives tab
 * switches. The parent passes the current mode/view and notifies us when
 * either changes.
 */
type View = "chooser" | "dev" | "production" | "settings";

export interface AuditPanelProps {
  mode: AuditMode;
  view: View;
  connectionId?: string | null;
  onModeChange: (mode: AuditMode) => void;
  onViewChange: (view: View) => void;
}

export default function AuditPanel({
  mode,
  view,
  connectionId,
  onModeChange,
  onViewChange,
}: AuditPanelProps) {
  injectAuditKeyframes();

  const handleChoose = useCallback(
    (m: AuditMode) => {
      onModeChange(m);
      onViewChange(m === "dev" ? "dev" : "production");
    },
    [onModeChange, onViewChange]
  );

  const switchMode = useCallback(
    (m: AuditMode) => {
      onModeChange(m);
      onViewChange(m === "dev" ? "dev" : "production");
    },
    [onModeChange, onViewChange]
  );

  const showSettings = useCallback(() => onViewChange("settings"), [onViewChange]);
  const backFromSettings = useCallback(() => {
    onViewChange(mode === "dev" ? "dev" : "production");
  }, [mode, onViewChange]);

  let body: React.ReactNode;
  if (view === "chooser") {
    body = <AuditModeChooser onChoose={handleChoose} />;
  } else if (view === "settings") {
    body = <AuditSettings onBack={backFromSettings} onModeChanged={switchMode} />;
  } else if (view === "dev") {
    body = <AuditDevFlow onShowSettings={showSettings} onSwitchMode={() => switchMode("production")} />;
  } else {
    body = <AuditProductionFlow onShowSettings={showSettings} onSwitchMode={() => switchMode("dev")} connectionId={connectionId} />;
  }

  return (
    <div className="pane">
      <div className="pane__header audit-pane-header">
        {/* Left: title + subtitle */}
        <div className="audit-pane-header__title-group">
          <h2 className="pane__title">Audit Log</h2>
          <span className="pane__sub">
            {view === "settings"
              ? "Settings"
              : view === "dev"
                ? "Local Docker stack"
                : view === "production"
                  ? "Production pipeline"
                  : "Choose a mode"}
          </span>
        </div>

        {/* Center: inline mode tabs (hidden in chooser/settings views) */}
        {view !== "chooser" && view !== "settings" && (
          <div className="audit-mode-tabs">
            <button
              className={`audit-mode-tab ${view === "dev" ? "is-active" : ""}`}
              onClick={() => switchMode("dev")}
            >
              Dev
            </button>
            <button
              className={`audit-mode-tab ${view === "production" ? "is-active" : ""}`}
              onClick={() => switchMode("production")}
            >
              Production
            </button>
          </div>
        )}

        {/* Right: settings link */}
        <div className="pane__actions">
          {view !== "chooser" && (
            <button
              className={`audit-mode-tab ${view === "settings" ? "is-active" : ""}`}
              onClick={view === "settings" ? backFromSettings : showSettings}
            >
              Settings
            </button>
          )}
        </div>
      </div>
      <div className="pane__body" style={{ padding: 0, display: "flex", flexDirection: "column" }}>
        {body}
      </div>
    </div>
  );
}
