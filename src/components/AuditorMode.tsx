import { useState, useCallback } from "react";
import type {
  VerificationReport,
  AuditorHandoffMaterial,
} from "../ipc/commands";
import commands, { formatError } from "../ipc/commands";
import { useToast } from "../context/ToastContext";
import { Alert, Badge, Button, Card, CardHeader } from "./AuditUi";
import { InfoPopover } from "./InfoPopover";
import { Eye, EyeOff, Key, Link, RefreshCw, Shield } from "lucide-react";

/**
 * AuditorMode — standalone verification interface for the independent auditor.
 *
 * This component lets an auditor (who may be on a completely different machine
 * from the operator) verify the audit trail using only:
 *   - The Soroban contract ID (to query on-chain roots)
 *   - The Stellar RPC URL (to reach the network)
 *   - Their age secret identity (to decrypt IPFS batches)
 *   - Optional Pinata credentials (for faster IPFS gateway access)
 *
 * The auditor never needs a MongoDB connection, never touches the operator's
 * data, and never needs the operator's private keys.
 *
 * When running in the same app as the operator (e.g. Dev Mode demo), the
 * "Load Handoff Material" button fetches the ready-made values from
 * `.env.audit` so the auditor doesn't have to type them in.
 */

interface FormState {
  contractId: string;
  rpcUrl: string;
  ageIdentity: string;
  pinataApiKey: string;
  pinataApiSecret: string;
  pinataGatewayUrl: string;
}

const DEFAULT_FORM: FormState = {
  contractId: "",
  rpcUrl: "https://soroban-testnet.stellar.org:443",
  ageIdentity: "",
  pinataApiKey: "",
  pinataApiSecret: "",
  pinataGatewayUrl: "https://gateway.pinata.cloud",
};

function shortHash(h: string | null | undefined): string {
  if (!h) return "—";
  return h.length > 20 ? `${h.slice(0, 10)}…${h.slice(-8)}` : h;
}

export default function AuditorMode() {
  const { push } = useToast();

  // ─── Form state ──────────────────────────────────────────────────────
  const [form, setForm] = useState<FormState>(DEFAULT_FORM);
  const [revealSecret, setRevealSecret] = useState(false);
  const [showPinata, setShowPinata] = useState(false);

  // ─── Handoff material ────────────────────────────────────────────────
  const [handoff, setHandoff] = useState<AuditorHandoffMaterial | null>(null);
  const [handoffLoading, setHandoffLoading] = useState(false);

  // ─── Rebuild ─────────────────────────────────────────────────────────
  const [rebuildLoading, setRebuildLoading] = useState(false);
  const [report, setReport] = useState<VerificationReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Try loading handoff material on mount (best-effort: only works when
  // the operator has already run Dev Mode setup on the same machine).
  const loadHandoff = useCallback(async () => {
    setHandoffLoading(true);
    try {
      const material = await commands.auditDevStackAuditorMaterial();
      if (material) {
        setHandoff(material);
        setForm((f) => ({
          ...f,
          contractId: material.contractId || f.contractId,
          rpcUrl: material.rpcUrl || f.rpcUrl,
          ageIdentity: material.ageAttesterSecret || f.ageIdentity,
        }));
        push("Handoff material loaded from Dev Mode setup", "success");
      } else {
        push("No handoff material found. Enter values manually.", "info");
      }
    } catch (e) {
      push(`Handoff load failed: ${formatError(e)}`, "error");
    } finally {
      setHandoffLoading(false);
    }
  }, [push]);

  // ─── Rebuild action ──────────────────────────────────────────────────
  const handleRebuild = useCallback(async () => {
    if (!form.contractId.trim()) {
      setError("Contract ID is required");
      return;
    }
    if (!form.ageIdentity.trim()) {
      setError("Age identity (secret key) is required for decryption");
      return;
    }
    setError(null);
    setReport(null);
    setRebuildLoading(true);
    try {
      const result = await commands.auditRebuildFromChain(
        form.ageIdentity.trim(),
        form.pinataApiKey.trim() || undefined,
        form.pinataApiSecret.trim() || undefined,
        form.pinataGatewayUrl.trim() || undefined,
        handoff?.auditLeafKeyHex?.trim() || undefined
      );
      setReport(result);
      if (result.tamperDetected) {
        push("Tamper detected! Root mismatch.", "error");
      } else if (result.onchainRootFound) {
        push(result.summary, "success");
      } else {
        push(result.summary, "info");
      }
    } catch (e) {
      const msg = formatError(e);
      setError(msg);
      push(`Rebuild failed: ${msg}`, "error");
    } finally {
      setRebuildLoading(false);
    }
  }, [form, push]);

  // ─── Derived display values ──────────────────────────────────────────
  const canRebuild =
    form.contractId.trim().length > 0 && form.ageIdentity.trim().length > 0;

  return (
    <div className="audit-surface" style={{ padding: "var(--space-5)" }}>
      {/* ─── Header ─────────────────────────────────────────────────── */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-3)",
          marginBottom: "var(--space-5)",
        }}
      >
        <Shield size={20} style={{ color: "var(--accent)" }} />
        <div>
          <h3
            style={{
              fontSize: "var(--font-size-lg)",
              fontWeight: 600,
              margin: 0,
            }}
          >
            Auditor Mode
          </h3>
          <p
            style={{
              fontSize: "var(--font-size-sm)",
              color: "var(--ink-muted)",
              margin: "var(--space-1) 0 0",
            }}
          >
            Verify the audit trail independently — no MongoDB access needed.
          </p>
        </div>
      </div>

      {/* ─── Handoff material ─────────────────────────────────────── */}
      <Card style={{ marginBottom: "var(--space-4)" }}>
        <CardHeader
          title={
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <Key size={14} />
              Handoff Material
            </span>
          }
        />
        <p
          style={{
            fontSize: "var(--font-size-sm)",
            color: "var(--ink-muted)",
            marginBottom: "var(--space-3)",
          }}
        >
          If this app shares a workspace with the operator, click Load to pull
          the ready-made auditor credentials from the Dev Mode setup.
        </p>
        <div style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}>
          <Button
            onClick={loadHandoff}
            loading={handoffLoading}
            disabled={handoffLoading}
            variant="primary"
          >
            Load Handoff Material
          </Button>
          {handoff && <Badge tone="success">Loaded</Badge>}
        </div>

        {handoff && (
          <div
            style={{
              marginTop: "var(--space-3)",
              display: "grid",
              gap: "var(--space-2)",
              fontSize: "var(--font-size-sm)",
            }}
          >
            <ReadOnlyRow label="Contract ID" value={handoff.contractId} />
            <ReadOnlyRow label="RPC URL" value={handoff.rpcUrl} />
            <ReadOnlyRow
              label="Age Public Key (Operator)"
              value={handoff.agePublicKeyOperator}
            />
            <ReadOnlyRow
              label="Age Public Key (Auditor)"
              value={handoff.agePublicKeyAttester}
            />
            <ReadOnlyRow
              label="Audit Leaf Key"
              value={shortHash(handoff.auditLeafKeyHex)}
              fullValue={handoff.auditLeafKeyHex}
            />
            <ReadOnlyRow
              label="Auditor MongoDB URI"
              value={handoff.auditorMongoUri}
              copyable
            />
          </div>
        )}
      </Card>

      {/* ─── Configuration form ────────────────────────────────────── */}
      <Card style={{ marginBottom: "var(--space-4)" }}>
        <CardHeader
          title={
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <Link size={14} />
              Configuration
            </span>
          }
        />
        <div
          style={{
            display: "grid",
            gap: "var(--space-3)",
          }}
        >
          <TextField
            label="Soroban Contract ID"
            value={form.contractId}
            onChange={(v) => setForm((f) => ({ ...f, contractId: v }))}
            placeholder="C..."
          />
          <TextField
            label="Stellar RPC URL"
            value={form.rpcUrl}
            onChange={(v) => setForm((f) => ({ ...f, rpcUrl: v }))}
            placeholder="https://soroban-testnet.stellar.org:443"
          />

          <div>
            <label
              style={{
                display: "block",
                fontSize: "var(--font-size-sm)",
                fontWeight: 500,
                marginBottom: "var(--space-1)",
                color: "var(--ink)",
              }}
            >
              Age Identity (Secret Key)
              <InfoPopover
                label="Help"
                title="Age Identity"
              >
                <p>
                  The auditor's age secret key used to decrypt IPFS batches.
                </p>
                <p>
                  In X25519 format, e.g. <code>AGE-SECRET-KEY-1...</code>
                </p>
              </InfoPopover>
            </label>
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <input
                type={revealSecret ? "text" : "password"}
                value={form.ageIdentity}
                onChange={(e) =>
                  setForm((f) => ({ ...f, ageIdentity: e.target.value }))
                }
                placeholder="AGE-SECRET-KEY-1..."
                style={{
                  flex: 1,
                  padding: "8px 12px",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--border)",
                  background: "var(--surface-1)",
                  color: "var(--ink)",
                  fontFamily: "var(--font-mono)",
                  fontSize: "var(--font-size-sm)",
                }}
              />
              <button
                type="button"
                onClick={() => setRevealSecret((s) => !s)}
                style={{
                  padding: "8px 10px",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--border)",
                  background: "var(--surface-2)",
                  cursor: "pointer",
                }}
                title={revealSecret ? "Hide secret" : "Reveal secret"}
              >
                {revealSecret ? <EyeOff size={16} /> : <Eye size={16} />}
              </button>
            </div>
          </div>

          {/* Pinata (optional, collapsed) */}
          <button
            type="button"
            onClick={() => setShowPinata((s) => !s)}
            style={{
              background: "none",
              border: "none",
              color: "var(--accent)",
              fontSize: "var(--font-size-sm)",
              cursor: "pointer",
              textAlign: "left",
              padding: 0,
            }}
          >
            {showPinata ? "− Hide Pinata options" : "+ Show Pinata options (optional)"}
          </button>
          {showPinata && (
            <div
              style={{
                display: "grid",
                gap: "var(--space-2)",
                padding: "var(--space-3)",
                background: "var(--surface-2)",
                borderRadius: "var(--radius-md)",
              }}
            >
              <TextField
                label="Pinata API Key"
                value={form.pinataApiKey}
                onChange={(v) => setForm((f) => ({ ...f, pinataApiKey: v }))}
              />
              <TextField
                label="Pinata API Secret"
                value={form.pinataApiSecret}
                onChange={(v) => setForm((f) => ({ ...f, pinataApiSecret: v }))}
                type="password"
              />
              <TextField
                label="Pinata Gateway URL"
                value={form.pinataGatewayUrl}
                onChange={(v) => setForm((f) => ({ ...f, pinataGatewayUrl: v }))}
                placeholder="https://gateway.pinata.cloud"
              />
            </div>
          )}
        </div>
      </Card>

      {/* ─── Actions ─────────────────────────────────────────────────── */}
      <div
        style={{
          display: "flex",
          gap: "var(--space-3)",
          marginBottom: "var(--space-4)",
          alignItems: "center",
        }}
      >
        <Button
          onClick={handleRebuild}
          loading={rebuildLoading}
          disabled={rebuildLoading || !canRebuild}
          variant="primary"
        >
          <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <RefreshCw size={16} />
            Rebuild from Chain
          </span>
        </Button>
        {!canRebuild && (
          <span style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>
            Enter contract ID and age identity to enable rebuild
          </span>
        )}
      </div>

      {/* ─── Error ───────────────────────────────────────────────────── */}
      {error && (
        <Alert tone="danger" style={{ marginBottom: "var(--space-4)" }}>
          {error}
        </Alert>
      )}

      {/* ─── Results ─────────────────────────────────────────────────── */}
      {report && (
        <Card style={{ marginBottom: "var(--space-4)" }}>
          <CardHeader
            title={
              <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                {report.tamperDetected
                  ? "⚠ Tamper Detected"
                  : report.onchainRootFound
                    ? "✓ Verification Successful"
                    : "ℹ No Data"}
                <Badge
                  tone={
                    report.tamperDetected
                      ? "danger"
                      : report.onchainRootFound
                        ? "success"
                        : "neutral"
                  }
                >
                  {report.tamperDetected
                    ? "Mismatch"
                    : report.onchainRootFound
                      ? "Verified"
                      : "Empty"}
                </Badge>
              </span>
            }
          />
          <p style={{ margin: "0 0 var(--space-2)", fontSize: "var(--font-size-sm)" }}>
            {report.summary}
          </p>
          {report.onchainRootFound && (
            <div
              style={{
                display: "grid",
                gap: "var(--space-1)",
                fontSize: "var(--font-size-sm)",
                fontFamily: "var(--font-mono)",
                marginTop: "var(--space-2)",
              }}
            >
              <div>
                <span style={{ color: "var(--ink-muted)" }}>Rebuilt root: </span>
                {shortHash(report.localRootHex)}
              </div>
              {report.onchainRoot && (
                <div>
                  <span style={{ color: "var(--ink-muted)" }}>On-chain root: </span>
                  {shortHash(report.onchainRoot.rootHex)}
                </div>
              )}
              <div>
                <span style={{ color: "var(--ink-muted)" }}>Events rebuilt: </span>
                {report.totalEvents}
              </div>
              {report.onchainRoot && (
                <div>
                  <span style={{ color: "var(--ink-muted)" }}>Sequence: </span>#
                  {report.onchainRoot.sequence}
                </div>
              )}
            </div>
          )}
        </Card>
      )}
    </div>
  );
}

// ─── Small subcomponents ──────────────────────────────────────────────

function TextField({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
}) {
  return (
    <div>
      <label
        style={{
          display: "block",
          fontSize: "var(--font-size-sm)",
          fontWeight: 500,
          marginBottom: "var(--space-1)",
          color: "var(--ink)",
        }}
      >
        {label}
      </label>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        style={{
          width: "100%",
          padding: "8px 12px",
          borderRadius: "var(--radius-md)",
          border: "1px solid var(--border)",
          background: "var(--surface-1)",
          color: "var(--ink)",
          fontFamily: type === "password" ? undefined : "var(--font-mono)",
          fontSize: "var(--font-size-sm)",
        }}
      />
    </div>
  );
}

function ReadOnlyRow({
  label,
  value,
  fullValue,
  copyable,
}: {
  label: string;
  value: string;
  fullValue?: string;
  copyable?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const display = value.length > 40 ? shortHash(value) : value;

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(value).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [value]);

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
        fontFamily: "var(--font-mono)",
      }}
    >
      <span style={{ color: "var(--ink-muted)", minWidth: 160, flexShrink: 0 }}>
        {label}:
      </span>
      <span title={fullValue || value}>{display}</span>
      {copyable && (
        <button
          type="button"
          onClick={handleCopy}
          style={{
            fontSize: "var(--font-size-xs)",
            padding: "2px 8px",
            borderRadius: "var(--radius-sm)",
            border: "1px solid var(--border)",
            background: "var(--surface-2)",
            cursor: "pointer",
            color: "var(--ink-muted)",
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      )}
    </div>
  );
}
