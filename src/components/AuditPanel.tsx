import { useCallback, useEffect, useState, useMemo } from "react";
import type { AuditMode, AuditModeConfig, DevPrerequisites } from "../ipc/commands";
import commands from "../ipc/commands";
import { AuditModeChooser } from "./AuditModeChooser";
import { AuditDevFlow } from "./AuditDevFlow";
import { AuditProductionFlow } from "./AuditProductionFlow";
import { AuditSettings } from "./AuditSettings";
import { AuditSurface } from "./AuditSurface";
import AuditorMode from "./AuditorMode";
import { injectAuditKeyframes, Spinner } from "./AuditUi";

/**
 * ZK Audit Log panel — state is owned by the App tab so it survives tab
 * switches. The parent passes the current mode/view/role and notifies us
 * when any of them change.
 *
 * Routing logic:
 *   - "chooser"    → AuditModeChooser landing (first run)
 *   - "settings"   → AuditSettings
 *   - "dev" + operator role → AuditDevFlow (stack control, seal → commit)
 *   - "dev" + auditor role  → AuditorMode (independent verification)
 *   - "production" with keypair → AuditSurface (unified surface)
 *   - "production" without keypair → ProductionFlow setup
 *
 * The Dev role defaults from the active connection's privileges (write →
 * operator, read-only → auditor) and is always switchable — the server
 * enforces the real permissions, this only picks the right surface.
 */
type View = "chooser" | "dev" | "production" | "settings";
export type AuditRole = "operator" | "auditor";

export interface AuditPanelProps {
  mode: AuditMode;
  view: View;
  /** Explicit role override chosen by the user; null = auto-detect. */
  role: AuditRole | null;
  connectionId?: string | null;
  onModeChange: (mode: AuditMode) => void;
  onViewChange: (view: View) => void;
  onRoleChange: (role: AuditRole) => void;
}

export default function AuditPanel({
  view,
  role,
  connectionId,
  onModeChange,
  onViewChange,
  onRoleChange,
}: AuditPanelProps) {
  injectAuditKeyframes();

  // ─── Config loading ──────────────────────────────────────────────────
  const [config, setConfig] = useState<AuditModeConfig | null>(null);
  const [configLoading, setConfigLoading] = useState(true);
  const [devPrereqs, setDevPrereqs] = useState<DevPrerequisites | null>(null);
  const [devPrereqsLoading, setDevPrereqsLoading] = useState(false);

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

  // ─── Role detection ──────────────────────────────────────────────────
  // Classify the active connection's privileges once per connection. A
  // read-only credential can't operate the audit stack, so it defaults to
  // the auditor surface; write access defaults to operator. The user can
  // override either way — this is a UX default, not a permission gate.
  const [detectedRole, setDetectedRole] = useState<AuditRole | null>(null);
  const [detecting, setDetecting] = useState(false);

  useEffect(() => {
    if (!connectionId) {
      setDetectedRole(null);
      return;
    }
    let cancelled = false;
    setDetecting(true);
    commands
      .connectionAccessLevel(connectionId)
      .then((access) => {
        if (cancelled) return;
        setDetectedRole(
          access.level === "read"
            ? "auditor"
            : access.level === "write"
              ? "operator"
              : null,
        );
      })
      .catch(() => {
        if (!cancelled) setDetectedRole(null);
      })
      .finally(() => {
        if (!cancelled) setDetecting(false);
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId]);

  // Effective role: explicit choice wins, then detection, then operator
  // (the dev-mode demo works without any MongoDB connection open).
  const effectiveRole: AuditRole = role ?? detectedRole ?? "operator";
  const roleAutoDetected = role === null && detectedRole !== null;

  const loadDevPrereqs = useCallback(async () => {
    setDevPrereqsLoading(true);
    try {
      const p = await commands.auditCheckDevPrerequisites();
      setDevPrereqs(p);
    } catch {
      setDevPrereqs(null);
    } finally {
      setDevPrereqsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (view !== "dev" || effectiveRole !== "operator") return;
    loadDevPrereqs();
    const interval = window.setInterval(loadDevPrereqs, 2000);
    return () => window.clearInterval(interval);
  }, [view, effectiveRole, loadDevPrereqs]);

  const showSettings = useCallback(() => onViewChange("settings"), [onViewChange]);

  // Back from settings returns to the active mode's view.
  const handleBackFromSettings = useCallback(() => {
    const m = config?.mode ?? "dev";
    onViewChange(m === "dev" ? "dev" : "production");
    loadConfig();
  }, [config, onViewChange, loadConfig]);

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

  // ─── Is configured? ──────────────────────────────────────────────────
  // Production mode needs a keypair before it shows the in-app surface. Dev
  // mode ALWAYS uses AuditDevFlow, which is the dashboard for the Dockerized
  // audit service (publisher/attester/reader) — it must never fall through to
  // the in-app AuditSurface, or the daemons would be bypassed entirely.
  const isConfigured = useMemo((): boolean => {
    if (!config) return false;
    if (view === "production") return config.hasProductionKeypair;
    return false;
  }, [config, view]);

  const isUnifiedSurface =
    view === "production" && isConfigured && !configLoading;

  // ─── Body ─────────────────────────────────────────────────────────────
  let body: React.ReactNode;

  const waitingForRole = view === "dev" && role === null && detecting;

  if (
    configLoading ||
    waitingForRole ||
    (view === "dev" &&
      effectiveRole === "operator" &&
      devPrereqsLoading &&
      !devPrereqs)
  ) {
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
  } else if (isUnifiedSurface && config) {
    // Configured audit mode → unified surface
    body = (
      <AuditSurface
        config={config}
        connectionId={connectionId}
        onShowSettings={showSettings}
      />
    );
  } else if (view === "dev" && effectiveRole === "auditor") {
    // Auditor role → independent verification. No MongoDB required.
    body = (
      <AuditorMode
        roleNotice={
          roleAutoDetected
            ? "Your MongoDB connection is read-only, so the auditor tools are shown. Use the role switch above if you operate the audit stack."
            : null
        }
      />
    );
  } else if (view === "dev") {
    // Operator role → Docker stack flow (Set up / Start Stack / live view).
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
  const showModeTabs = view !== "chooser" && view !== "settings" && !isUnifiedSurface;
  const showRoleSwitch = view === "dev" && !waitingForRole;

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
                  ? effectiveRole === "auditor"
                    ? "Independent verification"
                    : "Local Docker stack"
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
                className="audit-mode-tab"
                disabled
                aria-disabled="true"
                title="Production mode is coming soon"
              >
                Production
                <span className="audit-mode-tab__soon">Soon</span>
              </button>
            </div>
          )}

          <div className="pane__actions">
            {showRoleSwitch && (
              <div
                className="audit-role-switch"
                role="group"
                aria-label="Audit role"
              >
                <span className="audit-role-switch__label">Role</span>
                <button
                  className={`audit-mode-tab ${effectiveRole === "operator" ? "is-active" : ""}`}
                  onClick={() => onRoleChange("operator")}
                  title="Run the audit stack, seal epochs, and commit roots to Stellar"
                >
                  Operator
                </button>
                <button
                  className={`audit-mode-tab ${effectiveRole === "auditor" ? "is-active" : ""}`}
                  onClick={() => onRoleChange("auditor")}
                  title="Independently verify the audit trail — no MongoDB access needed"
                >
                  Auditor
                </button>
              </div>
            )}
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
