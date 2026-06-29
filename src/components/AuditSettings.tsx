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
import { useToast } from "../context/ToastContext";
import { InfoPopover } from "./InfoPopover";

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
  const toast = useToast();

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
      toast.push(formatError(e), "error");
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

    try {
      await commands.auditSetAuditMode(mode);
      toast.push(`Switched to ${mode === "dev" ? "Dev" : "Production"} mode`, "success");
      onModeChanged(mode);
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setSwitching(false);
    }
  };

  const saveProduction = async () => {
    setSavingProd(true);

    try {
      if (network === "mainnet" && !contractId.trim()) {
        toast.push("Mainnet requires a contract ID", "error");
        return;
      }
      await commands.auditSetProductionNetwork(
        network,
        network === "mainnet" ? contractId.trim() : "",
        network === "mainnet" ? rpcUrl.trim() : "",
      );
      toast.push("Production network saved", "success");
      await refresh();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setSavingProd(false);
    }
  };

  const importKey = async () => {
    setSavingProd(true);

    try {
      if (!secretKey.trim()) {
        toast.push("Enter a secret key", "error");
        return;
      }
      const acct = await commands.auditImportProductionKeypair(secretKey.trim());
      setAccountId(acct);
      setSecretKey("");
      toast.push("Production keypair saved to keychain", "success");
      await refresh();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setSavingProd(false);
    }
  };

  const clearKey = async () => {
    setSavingProd(true);

    try {
      await commands.auditClearProductionKeypair();
      setAccountId(null);
      toast.push("Production keypair cleared", "success");
      await refresh();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setSavingProd(false);
    }
  };

  const savePinata = async () => {
    setPinataBusy(true);

    try {
      await commands.auditSavePinataConfig(pinataKey.trim(), pinataSecret.trim());
      setHasPinata(true);
      setShowPinata(false);
      setPinataKey("");
      setPinataSecret("");
      toast.push("Pinata credentials updated", "success");
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setPinataBusy(false);
    }
  };

  const stackUp = async () => {
    setStackBusy(true);

    try {
      await commands.auditDevStackUp();
      await refreshDev();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setStackBusy(false);
    }
  };

  const stackDown = async () => {
    setStackBusy(true);

    try {
      await commands.auditDevStackDown();
      await refreshDev();
    } catch (e) {
      toast.push(formatError(e), "error");
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

      {/* ─── Mode ─────────────────────────────────────────────────── */}
      <Card>
        <CardHeader title={<>Mode<InfoPopover label="Help: Audit mode" title="Audit mode"><p><strong>Dev Mode</strong>: runs a local Stellar stack via Docker for testing. No real funds required.</p><p><strong>Production Mode</strong>: connects to live Stellar testnet or mainnet using your own keypair.</p></InfoPopover></>} subtitle="Switch between Dev and Production" />
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
            onClick={() => {}}
            disabled
            badge={<Badge tone="neutral">Coming soon</Badge>}
          />
        </div>
      </Card>

      {/* ─── Production config ────────────────────────────────────── */}
      <div style={{ opacity: 0.6, pointerEvents: "none" }} aria-disabled="true">
      <Card>
        <CardHeader title="Production Network" subtitle="Coming soon — production mode is not yet available" />
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr auto", gap: "var(--space-2)", marginBottom: "var(--space-3)", alignItems: "center" }}>
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
          <InfoPopover label="Help: Stellar network" title="Stellar network">
          <p><strong>Testnet</strong>: free test network for development.</p>
          <p><strong>Mainnet</strong>: production network with real transactions and fees.</p>
        </InfoPopover>
        </div>

        {network === "mainnet" && (
          <div style={{ marginBottom: "var(--space-3)" }}>
            <FieldLabel>Contract ID<InfoPopover label="Help: Stellar contract ID" title="Stellar contract ID"><p>The Soroban smart contract address on Stellar that stores audit roots. Required for mainnet.</p></InfoPopover></FieldLabel>
            <input value={contractId} onChange={(e) => setContractId(e.target.value)} placeholder="C..." style={inputStyle} />
            <FieldLabel style={{ marginTop: "var(--space-2)" }}>RPC URL<InfoPopover label="Help: Stellar RPC URL" title="Stellar RPC URL"><p>Endpoint for communicating with the Stellar network. Default is the public Stellar RPC.</p></InfoPopover></FieldLabel>
            <input value={rpcUrl} onChange={(e) => setRpcUrl(e.target.value)} style={inputStyle} />
            <div style={{ marginTop: "var(--space-2)" }}>
              <Alert tone="warning">Mainnet commits spend real XLM. Ensure your account is funded.</Alert>
            </div>
          </div>
        )}

        <Button variant="secondary" loading={savingProd} onClick={saveProduction}>Save Network</Button>

        {/* Keypair */}
        <div style={{ marginTop: "var(--space-4)", paddingTop: "var(--space-3)", borderTop: "1px solid var(--border)" }}>
          <FieldLabel>Production Keypair<InfoPopover label="Help: Stellar secret key" title="Stellar secret key"><p>Your Stellar account secret key used to sign on-chain commitment transactions. Stored securely in your OS keychain. Never share this key.</p></InfoPopover></FieldLabel>
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
      </div>

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
            <FieldLabel>API Key<InfoPopover label="Help: Pinata credentials" title="Pinata credentials"><p>Pinata is an IPFS pinning service. Credentials are required to store audit batches permanently on IPFS.</p></InfoPopover></FieldLabel>
            <input value={pinataKey} onChange={(e) => setPinataKey(e.target.value)} placeholder="Pinata API key" style={inputStyle} />
            <FieldLabel>API Secret<InfoPopover label="Help: Pinata API Secret" title="Pinata API Secret"><p>Your Pinata API secret is used to authenticate IPFS pinning requests. Stored securely in your OS keychain.</p></InfoPopover></FieldLabel>
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
          title={<>Dev Stack<InfoPopover label="Help: Dev Stack" title="Dev Stack"><p>The local Docker stack runs Stellar Core, Horizon, and the Soroban RPC for local development and testing.</p></InfoPopover></>}
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
  disabled = false,
  badge,
}: {
  active: boolean;
  label: string;
  hint: string;
  onClick: () => void;
  loading?: boolean;
  disabled?: boolean;
  badge?: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={loading || disabled}
      style={{
        textAlign: "left",
        padding: "var(--space-3)",
        borderRadius: "var(--radius-md)",
        background: active && !disabled ? "var(--accent-100)" : "var(--surface-2)",
        border: `1px solid ${active && !disabled ? "var(--accent-500)" : "var(--border)"}`,
        cursor: disabled ? "not-allowed" : loading ? "wait" : "pointer",
        opacity: disabled ? 0.6 : 1,
        transition: "border-color 0.12s ease, background 0.12s ease",
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
      }}
    >
      {loading && <Spinner size={13} />}
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
          <span style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, color: "var(--ink)" }}>{label}</span>
          {badge}
        </div>
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
