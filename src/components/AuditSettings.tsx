import { useState, useEffect, useCallback } from "react";
import commands, {
  type AuditMode,
  type AuditModeConfig,
  type AuditNetwork,
  type DevPrerequisites,
  type DevStackStatus,
  formatError,
} from "../ipc/commands";
import {
  Card,
  CardHeader,
  Badge,
  Button,
  Alert,
  Spinner,
  KeyValue,
} from "./AuditUi";

/**
 * Redesigned audit settings.
 *
 * - Switch between Dev and Production mode (re-routes the whole panel).
 * - Production: network choice (testnet/mainnet) + contract/RPC + keypair
 *   import/clear.
 * - Pinata IPFS credentials (shared across modes).
 * - Dev: quick stack start/stop/status.
 */
export function AuditSettings({
  onBack,
  onModeChanged,
}: {
  onBack: () => void;
  onModeChanged: (mode: AuditMode) => void;
}) {
  const [config, setConfig] = useState<AuditModeConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  // Mode switch
  const [switching, setSwitching] = useState(false);

  // Production network form
  const [network, setNetwork] = useState<AuditNetwork>("testnet");
  const [contractId, setContractId] = useState("");
  const [rpcUrl, setRpcUrl] = useState("https://rpc.mainnet.stellar.org");
  const [secretKey, setSecretKey] = useState("");
  const [savingProd, setSavingProd] = useState(false);
  const [accountId, setAccountId] = useState<string | null>(null);

  // Pinata
  const [showPinata, setShowPinata] = useState(false);
  const [pinataKey, setPinataKey] = useState("");
  const [pinataSecret, setPinataSecret] = useState("");
  const [pinataBusy, setPinataBusy] = useState(false);
  const [hasPinata, setHasPinata] = useState(false);

  // Dev stack
  const [prereqs, setPrereqs] = useState<DevPrerequisites | null>(null);
  const [stack, setStack] = useState<DevStackStatus | null>(null);
  const [stackBusy, setStackBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [c, onboarding, acct] = await Promise.all([
        commands.auditGetModeConfig(),
        commands.auditCheckOnboarding(),
        commands.auditGetActiveAccount(),
      ]);
      setConfig(c);
      setNetwork(c.network);
      setContractId(c.mainnetContractId);
      setRpcUrl(c.mainnetRpcUrl || "https://rpc.mainnet.stellar.org");
      setAccountId(acct);
      setHasPinata(onboarding.hasPinata);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const refreshDev = useCallback(async () => {
    try {
      const [p, s] = await Promise.all([
        commands.auditCheckDevPrerequisites(),
        commands.auditDevStackStatus(),
      ]);
      setPrereqs(p);
      setStack(s);
    } catch {
      /* best-effort */
    }
  }, []);

  useEffect(() => {
    refresh();
    refreshDev();
  }, [refresh, refreshDev]);

  const switchMode = async (mode: AuditMode) => {
    setSwitching(true);
    setError(null);
    try {
      await commands.auditSetAuditMode(mode);
      setMessage(`Switched to ${mode === "dev" ? "Dev" : "Production"} mode`);
      onModeChanged(mode);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSwitching(false);
    }
  };

  const saveProduction = async () => {
    setSavingProd(true);
    setError(null);
    try {
      if (network === "mainnet" && !contractId.trim()) {
        setError("Mainnet requires a contract ID");
        return;
      }
      await commands.auditSetProductionNetwork(
        network,
        network === "mainnet" ? contractId.trim() : "",
        network === "mainnet" ? rpcUrl.trim() : "",
      );
      setMessage("Production network saved");
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSavingProd(false);
    }
  };

  const importKey = async () => {
    setSavingProd(true);
    setError(null);
    try {
      if (!secretKey.trim()) {
        setError("Enter a secret key");
        return;
      }
      const acct = await commands.auditImportProductionKeypair(secretKey.trim());
      setAccountId(acct);
      setSecretKey("");
      setMessage("Production keypair saved to keychain");
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSavingProd(false);
    }
  };

  const clearKey = async () => {
    setSavingProd(true);
    setError(null);
    try {
      await commands.auditClearProductionKeypair();
      setAccountId(null);
      setMessage("Production keypair cleared");
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSavingProd(false);
    }
  };

  const savePinata = async () => {
    setPinataBusy(true);
    setError(null);
    try {
      await commands.auditSavePinataConfig(pinataKey.trim(), pinataSecret.trim());
      setHasPinata(true);
      setShowPinata(false);
      setPinataKey("");
      setPinataSecret("");
      setMessage("Pinata credentials updated");
    } catch (e) {
      setError(formatError(e));
    } finally {
      setPinataBusy(false);
    }
  };

  const stackUp = async () => {
    setStackBusy(true);
    setError(null);
    try {
      await commands.auditDevStackUp();
      await refreshDev();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setStackBusy(false);
    }
  };

  const stackDown = async () => {
    setStackBusy(true);
    setError(null);
    try {
      await commands.auditDevStackDown();
      await refreshDev();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setStackBusy(false);
    }
  };

  if (loading) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "var(--space-8)" }}>
        <Spinner size={22} />
      </div>
    );
  }

  const mode = config?.mode ?? "dev";

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-3)",
        padding: "var(--space-3)",
        flex: 1,
        overflow: "auto",
      }}
    >
      {/* Header */}
      <div style={{ display: "flex", alignItems: "center", gap: "var(--space-3)" }}>
        <h2 style={{ margin: 0, fontSize: "var(--font-size-xl)", fontWeight: 700, color: "var(--ink)" }}>Audit Settings</h2>
        <div style={{ flex: 1 }} />
        <Button variant="secondary" onClick={onBack}>Back</Button>
      </div>

      {error && <Alert tone="danger">{error}</Alert>}
      {message && <Alert tone="success">{message}</Alert>}

      {/* ─── Mode ─────────────────────────────────────────────────── */}
      <Card>
        <CardHeader title="Mode" subtitle="Switch between Dev and Production" />
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-2)" }}>
          <ModeToggle
            active={mode === "dev"}
            label="Dev Mode"
            hint="Full stack via Docker"
            onClick={() => switchMode("dev")}
            loading={switching && mode !== "dev"}
          />
          <ModeToggle
            active={mode === "production"}
            label="Production Mode"
            hint="In-app, your keys"
            onClick={() => switchMode("production")}
            loading={switching && mode !== "production"}
          />
        </div>
      </Card>

      {/* ─── Production config ────────────────────────────────────── */}
      <Card>
        <CardHeader title="Production Network" subtitle="Testnet or Mainnet — the double check" />
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-2)", marginBottom: "var(--space-3)" }}>
          <ModeToggle
            active={network === "testnet"}
            label="Testnet"
            hint="Auto-funded contract"
            onClick={() => setNetwork("testnet")}
          />
          <ModeToggle
            active={network === "mainnet"}
            label="Mainnet"
            hint="Your contract + RPC"
            onClick={() => setNetwork("mainnet")}
          />
        </div>

        {network === "mainnet" && (
          <div style={{ marginBottom: "var(--space-3)" }}>
            <FieldLabel>Contract ID</FieldLabel>
            <input value={contractId} onChange={(e) => setContractId(e.target.value)} placeholder="C..." style={inputStyle} />
            <FieldLabel style={{ marginTop: "var(--space-2)" }}>RPC URL</FieldLabel>
            <input value={rpcUrl} onChange={(e) => setRpcUrl(e.target.value)} style={inputStyle} />
            <div style={{ marginTop: "var(--space-2)" }}>
              <Alert tone="warning">Mainnet commits spend real XLM. Ensure your account is funded.</Alert>
            </div>
          </div>
        )}

        <Button variant="secondary" loading={savingProd} onClick={saveProduction}>Save Network</Button>

        {/* Keypair */}
        <div style={{ marginTop: "var(--space-4)", paddingTop: "var(--space-3)", borderTop: "1px solid var(--border)" }}>
          <FieldLabel>Production Keypair</FieldLabel>
          {accountId ? (
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", flexWrap: "wrap" }}>
              <Badge tone="success" dot>Saved</Badge>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>
                {shortAddr(accountId)}
              </span>
              <div style={{ flex: 1 }} />
              <Button variant="danger" loading={savingProd} onClick={clearKey}>Clear</Button>
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
              <input
                value={secretKey}
                onChange={(e) => setSecretKey(e.target.value)}
                placeholder="S... (56 characters)"
                type="password"
                style={inputStyle}
              />
              <Button variant="primary" loading={savingProd} onClick={importKey}>Import Keypair</Button>
            </div>
          )}
        </div>
      </Card>

      {/* ─── Pinata ───────────────────────────────────────────────── */}
      <Card>
        <CardHeader title="IPFS Storage (Pinata)" subtitle="Shared across dev and production" />
        {hasPinata && !showPinata ? (
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
            <Badge tone="success" dot>Configured</Badge>
            <span style={{ fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>
              API Key: ••••••••••••
            </span>
            <div style={{ flex: 1 }} />
            <Button variant="secondary" onClick={() => setShowPinata(true)}>Update Key</Button>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
            <FieldLabel>API Key</FieldLabel>
            <input value={pinataKey} onChange={(e) => setPinataKey(e.target.value)} placeholder="Pinata API key" style={inputStyle} />
            <FieldLabel>API Secret</FieldLabel>
            <input value={pinataSecret} onChange={(e) => setPinataSecret(e.target.value)} placeholder="Pinata API secret" type="password" style={inputStyle} />
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <Button variant="primary" loading={pinataBusy} onClick={savePinata}>Save</Button>
              {showPinata && <Button variant="ghost" onClick={() => setShowPinata(false)}>Cancel</Button>}
            </div>
          </div>
        )}
      </Card>

      {/* ─── Dev stack quick controls ─────────────────────────────── */}
      <Card>
        <CardHeader
          title="Dev Stack"
          subtitle="Quick controls for the local audit containers"
          actions={
            stack?.running ? (
              <Button variant="danger" loading={stackBusy} onClick={stackDown}>Stop</Button>
            ) : (
              <Button
                variant="primary"
                loading={stackBusy}
                disabled={!prereqs?.dockerInstalled || !prereqs?.composeFilePresent}
                onClick={stackUp}
              >
                Start
              </Button>
            )
          }
        />
        {prereqs && (
          <KeyValue label="Prerequisites" value={prereqs.summary} mono={false} />
        )}
        {stack && stack.services.length > 0 && (
          <div style={{ marginTop: "var(--space-2)" }}>
            {stack.services.map((s) => (
              <div key={s.name} style={{ display: "flex", gap: "var(--space-2)", alignItems: "center", padding: "var(--space-1) 0" }}>
                <Badge tone={s.state.toLowerCase().includes("up") ? "success" : "neutral"} dot>{s.name}</Badge>
                <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>{s.state}</span>
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}

function ModeToggle({
  active,
  label,
  hint,
  onClick,
  loading,
}: {
  active: boolean;
  label: string;
  hint: string;
  onClick: () => void;
  loading?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={loading}
      style={{
        textAlign: "left",
        padding: "var(--space-3)",
        borderRadius: "var(--radius-md)",
        background: active ? "var(--accent-100)" : "var(--surface-2)",
        border: `1px solid ${active ? "var(--accent-500)" : "var(--border)"}`,
        cursor: loading ? "wait" : "pointer",
        transition: "border-color 0.12s ease, background 0.12s ease",
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
      }}
    >
      {loading && <Spinner size={13} />}
      <div>
        <div style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, color: "var(--ink)" }}>{label}</div>
        <div style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)", marginTop: "2px" }}>{hint}</div>
      </div>
    </button>
  );
}

function FieldLabel({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) {
  return (
    <div
      style={{
        fontSize: "var(--font-size-xs)",
        color: "var(--ink-faint)",
        textTransform: "uppercase",
        letterSpacing: "0.04em",
        marginBottom: "var(--space-1)",
        ...style,
      }}
    >
      {children}
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  width: "100%",
  padding: "8px 10px",
  borderRadius: "var(--radius-md)",
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "var(--ink)",
  fontSize: "var(--font-size-sm)",
  fontFamily: "var(--font-mono)",
};

function shortAddr(a: string): string {
  return a.length > 12 ? `${a.slice(0, 6)}…${a.slice(-4)}` : a;
}
