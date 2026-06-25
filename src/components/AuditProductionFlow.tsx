import { useState, useEffect, useCallback } from "react";
import commands, {
  type AuditModeConfig,
  type AuditNetwork,
  formatError,
} from "../ipc/commands";
import {
  Card,
  CardHeader,
  Badge,
  Button,
  Alert,
  Spinner,
  EmptyState,
} from "./AuditUi";
import { AuditLiveViewV2 } from "./AuditLiveViewV2";

/**
 * Production Mode flow — the in-app audit pipeline with the user's own keys.
 *
 *  1. Pick a network: testnet or mainnet (the "double check").
 *  2. Import a Stellar secret key (saved to the OS keychain).
 *  3. If mainnet: provide the contract ID + RPC URL.
 *  4. Once configured → the live view commits via `auditCommitRootProduction`,
 *     which signs with the imported keypair on the chosen network.
 */
export function AuditProductionFlow({ onShowSettings }: { onShowSettings: () => void }) {
  const [config, setConfig] = useState<AuditModeConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Setup form state
  const [network, setNetwork] = useState<AuditNetwork>("testnet");
  const [secretKey, setSecretKey] = useState("");
  const [contractId, setContractId] = useState("");
  const [rpcUrl, setRpcUrl] = useState("https://rpc.mainnet.stellar.org");
  const [saving, setSaving] = useState(false);
  const [accountId, setAccountId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const c = await commands.auditGetModeConfig();
      setConfig(c);
      setNetwork(c.network);
      setContractId(c.mainnetContractId);
      setRpcUrl(c.mainnetRpcUrl || "https://rpc.mainnet.stellar.org");
      const acct = await commands.auditGetActiveAccount();
      setAccountId(acct);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const importKey = async () => {
    setSaving(true);
    setError(null);
    try {
      if (!secretKey.trim()) {
        setError("Enter your Stellar secret key (S...)");
        return;
      }
      const acct = await commands.auditImportProductionKeypair(secretKey.trim());
      setAccountId(acct);
      setSecretKey("");
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  };

  const saveNetwork = async () => {
    setSaving(true);
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
      await refresh();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "var(--space-8)" }}>
        <Spinner size={22} />
      </div>
    );
  }

  const hasKeypair = config?.hasProductionKeypair ?? false;
  const needsSetup = !hasKeypair || (network === "mainnet" && !contractId.trim());

  // ─── Setup screen ──────────────────────────────────────────────────
  if (needsSetup) {
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-3)",
          padding: "var(--space-4)",
          maxWidth: "640px",
          margin: "0 auto",
          animation: "audit-fade-in 0.2s ease",
        }}
      >
        <Card padded={false}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-3)",
              padding: "var(--space-3) var(--space-4)",
              flexWrap: "wrap",
            }}
          >
            <Badge tone="success" dot>Production Mode</Badge>
            <div style={{ flex: 1 }} />
            <Button variant="ghost" onClick={onShowSettings}>Settings</Button>
          </div>
        </Card>

        {error && <Alert tone="danger">{error}</Alert>}

        <Card>
          <CardHeader
            title="Configure Production"
            subtitle="Import your Stellar keypair and pick a network"
          />

          {/* Network choice */}
          <FieldLabel>Network</FieldLabel>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-2)", marginBottom: "var(--space-3)" }}>
            <NetworkOption
              active={network === "testnet"}
              label="Testnet"
              hint="Auto-funded contract · verify a testnet deployment"
              onClick={() => setNetwork("testnet")}
            />
            <NetworkOption
              active={network === "mainnet"}
              label="Mainnet"
              hint="Your contract + RPC · real commitments"
              onClick={() => setNetwork("mainnet")}
            />
          </div>

          {/* Mainnet contract / rpc */}
          {network === "mainnet" && (
            <div style={{ marginBottom: "var(--space-3)" }}>
              <FieldLabel>Contract ID</FieldLabel>
              <input
                value={contractId}
                onChange={(e) => setContractId(e.target.value)}
                placeholder="C... (Soroban contract ID)"
                style={inputStyle}
              />
              <FieldLabel style={{ marginTop: "var(--space-2)" }}>RPC URL</FieldLabel>
              <input
                value={rpcUrl}
                onChange={(e) => setRpcUrl(e.target.value)}
                placeholder="https://rpc.mainnet.stellar.org"
                style={inputStyle}
              />
            </div>
          )}

          {/* Keypair import */}
          <FieldLabel>Stellar Secret Key</FieldLabel>
          <input
            value={secretKey}
            onChange={(e) => setSecretKey(e.target.value)}
            placeholder="S... (56 characters)"
            type="password"
            style={inputStyle}
            disabled={hasKeypair}
          />
          {hasKeypair ? (
            <Alert tone="success">
              ✓ Keypair saved {accountId ? `(${shortAddr(accountId)})` : ""}. To replace it, clear it in Settings first.
            </Alert>
          ) : (
            <div style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)", marginTop: "var(--space-1)" }}>
              The secret key is stored in your OS keychain. It never leaves your machine.
            </div>
          )}

          <div style={{ display: "flex", gap: "var(--space-2)", marginTop: "var(--space-3)" }}>
            {!hasKeypair && (
              <Button variant="primary" loading={saving} onClick={importKey}>
                Import Keypair
              </Button>
            )}
            <Button variant="secondary" loading={saving} onClick={saveNetwork}>
              Save Network
            </Button>
          </div>
        </Card>

        <Card>
          <EmptyState
            icon="🛰️"
            title="Almost there"
            body="Import your keypair and save your network choice to start the production audit pipeline. You'll be able to commit roots to your chosen Stellar network and verify integrity."
          />
        </Card>
      </div>
    );
  }

  // ─── Live view ─────────────────────────────────────────────────────
  const commitFn = (metadata?: string) => commands.auditCommitRootProduction(metadata);
  const badge = (
    <Badge tone="success" dot>
      Production · {network === "mainnet" ? "Mainnet" : "Testnet"}
    </Badge>
  );

  return (
    <AuditLiveViewV2
      commitFn={commitFn}
      badge={badge}
      onShowSettings={onShowSettings}
    />
  );
}

function NetworkOption({
  active,
  label,
  hint,
  onClick,
}: {
  active: boolean;
  label: string;
  hint: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        textAlign: "left",
        padding: "var(--space-3)",
        borderRadius: "var(--radius-md)",
        background: active ? "var(--accent-100)" : "var(--surface-2)",
        border: `1px solid ${active ? "var(--accent-500)" : "var(--border)"}`,
        cursor: "pointer",
        transition: "border-color 0.12s ease, background 0.12s ease",
      }}
    >
      <div style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, color: "var(--ink)" }}>{label}</div>
      <div style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)", marginTop: "2px" }}>{hint}</div>
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
