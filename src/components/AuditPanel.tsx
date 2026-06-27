import { useCallback, useEffect, useState, useMemo } from "react";
import type { AuditMode, AuditModeConfig } from "../ipc/commands";
import commands from "../ipc/commands";
import { AuditModeChooser } from "./AuditModeChooser";
import { AuditDevFlow } from "./AuditDevFlow";
import { AuditProductionFlow } from "./AuditProductionFlow";
import { AuditSettings } from "./AuditSettings";
import { AuditSurface } from "./AuditSurface";
import { injectAuditKeyframes, Spinner } from "./AuditUi";

/**
 * ZK Audit Log panel — state is owned by the App tab so it survives tab
 * switches. The parent passes the current mode/view and notifies us when
 * either changes.
 *
 * Routing logic:
 *   - "chooser"    → AuditModeChooser landing (first run)
 *   - "settings"   → AuditSettings
 *   - "dev"/"production" with stack not configured → DevFlow/ProductionFlow setup
 *   - "dev"/"production" configured → AuditSurface (unified surface)
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

  // ─── Config loading ──────────────────────────────────────────────────
  const [config, setConfig] = useState<AuditModeConfig | null>(null);
  const [configLoading, setConfigLoading] = useState(true);

  const loadConfig = useCallback(async () => {
    try {
      const c = await commands.auditGetModeConfig();
      setConfig(c);
    } catch {
      // ignore — will show chooser
    } finally {
      setConfigLoading(false);
    }
  }, []);

  useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  // Re-load config whenever we leave settings (mode may have changed).
  const handleBackFromSettings = useCallback(
    (updatedMode?: AuditMode) => {
      const m = updatedMode ?? mode;
      onViewChange(m === "dev" ? "dev" : "production");
      loadConfig();
    },
    [mode, onViewChange, loadConfig],
  );

  const handleChoose = useCallback(
    (m: AuditMode) => {
      onModeChange(m);
      onViewChange(m === "dev" ? "dev" : "production");
      loadConfig();
    },
    [onModeChange, onViewChange, loadConfig],
  );

  const switchMode = useCallback(
    (m: AuditMode) => {
      onModeChange(m);
      onViewChange(m === "dev" ? "dev" : "production");
      loadConfig();
    },
    [onModeChange, onViewChange, loadConfig],
  );

  const showSettings = useCallback(() => onViewChange("settings"), [onViewChange]);

  // ─── Is configured? ──────────────────────────────────────────────────
  // Production mode needs a keypair. Dev mode is always "configured" (Docker setup
  // is shown inside DevFlow, not as a blocker here — DevFlow shows the docker step
  // inline). We route to AuditSurface once the user has been through the chooser.
  const isConfigured = useMemo((): boolean => {
    if (!config) return false;
    if (config.mode === "production") return config.hasProductionKeypair;
    return true; // dev mode — docker onboarding is optional/inline
  }, [config]);

  // ─── Body ─────────────────────────────────────────────────────────────
  let body: React.ReactNode;

  if (configLoading) {
    body = (
      <div style={{ display: "flex", justifyContent: "center", padding: "var(--space-8)" }}>
        <Spinner size={22} />
      </div>
    );
  } else if (view === "chooser") {
    body = <AuditModeChooser onChoose={handleChoose} />;
  } else if (view === "settings") {
    body = (
      <AuditSettings
        onBack={handleBackFromSettings}
        onModeChanged={switchMode}
      />
    );
  } else if (view === "production" && config && isConfigured) {
    // Configured production mode → unified surface
    body = (
      <AuditSurface
        config={config}
        connectionId={connectionId}
        onShowSettings={showSettings}
      />
    );
  } else if (view === "dev") {
    // Dev mode → Docker stack flow (Set up / Start Stack / live view).
    body = (
      <AuditDevFlow
        onShowSettings={showSettings}
        onSwitchMode={() => switchMode("production")}
      />
    );
  } else {
    // Production setup flow (keypair import)
    body = (
      <AuditProductionFlow
        onShowSettings={showSettings}
        onSwitchMode={() => switchMode("dev")}
        connectionId={connectionId}
      />
    );
  }

  // ─── Header ───────────────────────────────────────────────────────────
  // When showing the unified surface, the AuditSurface itself renders its own
  // sticky header. We still show the pane chrome title so it's consistent with
  // other tabs — but we hide the mode tabs (they live in Settings now).
  const isUnifiedSurface = view === "production" && isConfigured && !configLoading;
  const showModeTabs = view !== "chooser" && view !== "settings" && !isUnifiedSurface;

  return (
    <div
      className="pane"
      style={isUnifiedSurface ? { gridTemplateRows: "1fr" } : undefined}
    >
      {/* Only show the legacy pane header when NOT in the unified surface */}
      {!isUnifiedSurface && (
        <div className="pane__header audit-pane-header">
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

          {showModeTabs && (
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

          <div className="pane__actions">
            {view !== "chooser" && (
              <button
                className={`audit-mode-tab ${view === "settings" ? "is-active" : ""}`}
                onClick={view === "settings" ? () => handleBackFromSettings() : showSettings}
              >
                Settings
              </button>
            )}
          </div>
        </div>
      )}

      <div
        className="pane__body"
        style={{
          padding: 0,
          // pane__body already has overflow:auto + min-height:0 from styles.css.
          // For the unified surface we also need display:flex so audit-surface
          // can flex:1 inside it. overflow stays auto (pane__body scrolls).
          display: isUnifiedSurface ? "flex" : undefined,
          flexDirection: isUnifiedSurface ? "column" : undefined,
        }}
      >
        {body}
      </div>
    </div>
  );
}


