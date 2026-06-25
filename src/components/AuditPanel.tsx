import { useState, useCallback } from "react";
import type { AuditMode } from "../ipc/commands";
import { AuditModeChooser } from "./AuditModeChooser";
import { AuditDevFlow } from "./AuditDevFlow";
import { AuditProductionFlow } from "./AuditProductionFlow";
import { AuditSettings } from "./AuditSettings";
import { injectAuditKeyframes } from "./AuditUi";

/**
 * ZK Audit Log panel — the redesigned router.
 *
 * The mode chooser is always shown when the user opens the Audit tab. After
 * they pick Dev or Production, the panel routes to the corresponding flow.
 * Settings is reachable from either flow; Back returns to the active flow.
 * Switching mode in Settings re-routes immediately.
 *
 * Hook order is fixed: this component owns exactly two useState hooks plus
 * one useCallback, all called unconditionally before any early return.
 */
type View = "chooser" | "dev" | "production" | "settings";

export default function AuditPanel() {
  injectAuditKeyframes();

  // The selected mode drives which flow we return to from Settings.
  const [mode, setMode] = useState<AuditMode>("dev");
  // The active view. Starts at the chooser every time the tab opens.
  const [view, setView] = useState<View>("chooser");

  const handleChoose = useCallback((m: AuditMode) => {
    setMode(m);
    setView(m === "dev" ? "dev" : "production");
  }, []);

  const handleModeChanged = useCallback((m: AuditMode) => {
    setMode(m);
    setView(m === "dev" ? "dev" : "production");
  }, []);

  const showSettings = useCallback(() => setView("settings"), []);
  const backFromSettings = useCallback(() => {
    setView(mode === "dev" ? "dev" : "production");
  }, [mode]);

  if (view === "chooser") {
    return <AuditModeChooser onChoose={handleChoose} />;
  }

  if (view === "settings") {
    return <AuditSettings onBack={backFromSettings} onModeChanged={handleModeChanged} />;
  }

  if (view === "dev") {
    return <AuditDevFlow onShowSettings={showSettings} />;
  }

  return <AuditProductionFlow onShowSettings={showSettings} />;
}
