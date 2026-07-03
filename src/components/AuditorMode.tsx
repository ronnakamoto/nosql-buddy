import { useState, useCallback, useEffect, useRef } from "react";
import type {
  VerificationReport,
  AuditorHandoffMaterial,
  ReadonlyVerifyResult,
  DisclosureClaim,
  RecordedVerifyResult,
  OnChainDisclosureRecord,
} from "../ipc/commands";
import commands, { formatError } from "../ipc/commands";
import { useToast } from "../context/ToastContext";
import { Alert, Badge, Button, Card, CardHeader, TxHashLink } from "./AuditUi";
import { InfoPopover } from "./InfoPopover";
import {
  Eye,
  EyeOff,
  Key,
  RefreshCw,
  Shield,
  ShieldCheck,
} from "lucide-react";

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

/**
 * The auditor form only collects a raw RPC URL (not a network enum), so we
 * infer which Stellar Explorer network to link to from its hostname. Falls
 * back to testnet, the default the form is pre-filled with.
 */
function networkFromRpcUrl(rpcUrl: string): "testnet" | "mainnet" {
  return /mainnet|soroban-rpc\.stellar\.org|horizon\.stellar\.org/i.test(rpcUrl)
    ? "mainnet"
    : "testnet";
}

export default function AuditorMode({
  roleNotice,
}: {
  /** Shown when the role was auto-detected (e.g. read-only connection). */
  roleNotice?: string | null;
}) {
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

  // ─── Proof bundle verification ───────────────────────────────────────
  const [bundleText, setBundleText] = useState("");
  const [verifyLoading, setVerifyLoading] = useState(false);
  const [verifyResult, setVerifyResult] = useState<ReadonlyVerifyResult | null>(
    null
  );
  const [verifyError, setVerifyError] = useState<string | null>(null);
  const [verifiedClaimText, setVerifiedClaimText] = useState<string | null>(
    null
  );

  // ─── On-chain recorded verification ──────────────────────────────────
  const [verifiedBundle, setVerifiedBundle] = useState<{
    rootHex: string;
    leafHex: string;
    claim: DisclosureClaim;
    proofA: string;
    proofB: string;
    proofC: string;
    contractId: string;
  } | null>(null);
  const [recordSecretKey, setRecordSecretKey] = useState("");
  const [revealRecordKey, setRevealRecordKey] = useState(false);
  const [recordLoading, setRecordLoading] = useState(false);
  const [recordResult, setRecordResult] = useState<RecordedVerifyResult | null>(
    null
  );
  const [records, setRecords] = useState<OnChainDisclosureRecord[] | null>(
    null
  );
  const [recordsLoading, setRecordsLoading] = useState(false);

  // Auto-load handoff material on mount, silently. This only succeeds when
  // the operator has already run Dev Mode setup on the same machine — the
  // common demo case — so a co-located auditor starts pre-filled instead of
  // staring at empty credential fields. On a separate machine it quietly
  // finds nothing and the manual fields take over.
  useEffect(() => {
    let cancelled = false;
    commands
      .auditDevStackAuditorMaterial()
      .then((material) => {
        if (cancelled || !material) return;
        setHandoff(material);
        setForm((f) => ({
          ...f,
          contractId: material.contractId || f.contractId,
          rpcUrl: material.rpcUrl || f.rpcUrl,
          ageIdentity: material.ageAttesterSecret || f.ageIdentity,
        }));
        // Dev Mode convenience: the setup wizard generated and funded the
        // attester's Stellar account on this machine, so prefill the
        // "Verify & Record" signing key too. Never overwrite a typed key.
        if (material.attesterStellarSecret) {
          setRecordSecretKey((k) => k || material.attesterStellarSecret);
        }
      })
      .catch(() => {
        // best-effort — manual entry still works
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Manual retry (with feedback) for when the operator finishes setup after
  // this view mounted.
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
        if (material.attesterStellarSecret) {
          setRecordSecretKey((k) => k || material.attesterStellarSecret);
        }
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

  // ─── Proof bundle verification action ────────────────────────────────
  //
  // Verifies a Groth16 proof bundle via a read-only Soroban simulation from
  // THIS machine: no transaction, no fee, no keys. The pairing check runs in
  // the Soroban runtime against the contract's pinned verifying key and
  // committed-root index — the operator is not in the loop.
  const handleVerifyBundle = useCallback(async () => {
    setVerifyError(null);
    setVerifyResult(null);

    let bundle: {
      rootHex?: string;
      leafHex?: string;
      proof?: { a?: string; b?: string; c?: string };
      proofA?: string;
      proofB?: string;
      proofC?: string;
      contractId?: string;
      claim?: DisclosureClaim;
      claimText?: string;
    };
    try {
      bundle = JSON.parse(bundleText);
    } catch {
      setVerifyError("Invalid JSON — paste the proof bundle exactly as exported.");
      return;
    }

    const rootHex = bundle.rootHex?.trim();
    const leafHex = bundle.leafHex?.trim();
    const proofA = (bundle.proof?.a ?? bundle.proofA)?.trim();
    const proofB = (bundle.proof?.b ?? bundle.proofB)?.trim();
    const proofC = (bundle.proof?.c ?? bundle.proofC)?.trim();
    if (!rootHex || !leafHex || !proofA || !proofB || !proofC) {
      setVerifyError(
        "Bundle is missing required fields (rootHex, leafHex, proof.a/b/c)."
      );
      return;
    }

    const contractId = (bundle.contractId || form.contractId).trim();
    if (!contractId) {
      setVerifyError("No contract ID — set it in Configuration or in the bundle.");
      return;
    }

    setVerifyLoading(true);
    try {
      // A bundle carrying a `claim` is an Audited-Action Disclosure proof
      // (predicates over the still-private event); otherwise it's a plain
      // inclusion proof. Both verify via read-only simulation.
      const result = bundle.claim
        ? await commands.auditVerifyDisclosureReadonly({
            rootHex,
            leafHex,
            claim: bundle.claim,
            proofA,
            proofB,
            proofC,
            rpcUrl: form.rpcUrl.trim() || undefined,
            contractId,
          })
        : await commands.auditVerifyProofReadonly({
            rootHex,
            leafHex,
            proofA,
            proofB,
            proofC,
            rpcUrl: form.rpcUrl.trim() || undefined,
            contractId,
          });
      setVerifyResult(result);
      setVerifiedClaimText(
        result.verified && bundle.claim ? bundle.claimText || null : null
      );
      setRecordResult(null);
      setVerifiedBundle(
        result.verified && bundle.claim
          ? { rootHex, leafHex, claim: bundle.claim, proofA, proofB, proofC, contractId }
          : null
      );
      if (result.verified) {
        push("Proof verified on-chain (read-only simulation)", "success");
      } else {
        push(result.reason || "Proof verification failed", "error");
      }
    } catch (e) {
      const msg = formatError(e);
      setVerifyError(msg);
      push(`Verification failed: ${msg}`, "error");
    } finally {
      setVerifyLoading(false);
    }
  }, [bundleText, form.contractId, form.rpcUrl, push]);

  // ─── Record verification on-chain ────────────────────────────────────
  //
  // The attestation form of verification: the same pairing check runs
  // on-chain, but as a transaction signed by the AUDITOR's own account.
  // The contract appends a verifier-attributed record — permanent, citable
  // evidence (by tx hash) that this party checked this claim.
  const handleRecordVerification = useCallback(async () => {
    if (!verifiedBundle || !recordSecretKey.trim()) return;
    setRecordLoading(true);
    setRecordResult(null);
    try {
      const result = await commands.auditVerifyDisclosureRecord({
        rootHex: verifiedBundle.rootHex,
        leafHex: verifiedBundle.leafHex,
        claim: verifiedBundle.claim,
        proofA: verifiedBundle.proofA,
        proofB: verifiedBundle.proofB,
        proofC: verifiedBundle.proofC,
        secretKey: recordSecretKey.trim(),
        rpcUrl: form.rpcUrl.trim() || undefined,
        contractId: verifiedBundle.contractId,
      });
      setRecordResult(result);
      push(
        `Verification recorded on-chain (record #${result.recordId})`,
        "success"
      );
    } catch (e) {
      push(`Recording failed: ${formatError(e)}`, "error");
    } finally {
      setRecordLoading(false);
    }
  }, [verifiedBundle, recordSecretKey, form.rpcUrl, push]);

  const handleLoadRecords = useCallback(async () => {
    if (!form.contractId.trim()) {
      push("Enter a contract ID first", "info");
      return;
    }
    setRecordsLoading(true);
    try {
      const result = await commands.auditListDisclosureRecords({
        limit: 20,
        rpcUrl: form.rpcUrl.trim() || undefined,
        contractId: form.contractId.trim(),
      });
      setRecords(result);
    } catch (e) {
      push(`Failed to load records: ${formatError(e)}`, "error");
    } finally {
      setRecordsLoading(false);
    }
  }, [form.contractId, form.rpcUrl, push]);

  // Auto-load the recorded-verifications list as soon as a contract ID is
  // known (from handoff material or manual entry) — an auditor opening this
  // view shouldn't have to click "Load" just to see the append-only log.
  // Tracks the last contract ID it fired for so it doesn't refetch on every
  // keystroke while someone is still typing a contract ID.
  const recordsAutoLoadedFor = useRef<string | null>(null);
  useEffect(() => {
    const contractId = form.contractId.trim();
    if (!contractId || recordsAutoLoadedFor.current === contractId) return;
    recordsAutoLoadedFor.current = contractId;
    handleLoadRecords();
  }, [form.contractId, handleLoadRecords]);

  // Refresh the list right after this auditor records a new verification,
  // so their own entry (and its tx hash) shows up without a manual reload.
  useEffect(() => {
    if (recordResult) {
      handleLoadRecords();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recordResult]);

  // ─── Derived display values ──────────────────────────────────────────
  const canRebuild =
    form.contractId.trim().length > 0 && form.ageIdentity.trim().length > 0;
  const rebuildDone =
    report !== null && report.onchainRootFound && !report.tamperDetected;
  const claimVerified = verifyResult?.verified === true;

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "auto" }}>
      {/* ─── Workflow step guide (mirrors the operator surface) ──────── */}
      <div className="audit-step-guide">
        <div className={`audit-step ${canRebuild ? "audit-step--done" : "audit-step--active"}`}>
          <span className="audit-step__num">{canRebuild ? "✓" : "1"}</span>
          <span className="audit-step__label">Credentials</span>
        </div>
        <div className={`audit-step ${rebuildDone ? "audit-step--done" : canRebuild ? "audit-step--active" : ""}`}>
          <span className="audit-step__num">{rebuildDone ? "✓" : canRebuild ? "2" : ""}</span>
          <span className="audit-step__label">Rebuild &amp; Compare</span>
        </div>
        <div className={`audit-step ${claimVerified ? "audit-step--done" : rebuildDone || bundleText.trim().length > 0 ? "audit-step--active" : ""}`}>
          <span className="audit-step__num">{claimVerified ? "✓" : rebuildDone || bundleText.trim().length > 0 ? "3" : ""}</span>
          <span className="audit-step__label">Verify Claims</span>
        </div>
      </div>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-3)",
          padding: "var(--space-3)",
          flex: 1,
        }}
      >
      {roleNotice && <Alert tone="info">{roleNotice}</Alert>}

      {/* ─── Header ─────────────────────────────────────────────────── */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-3)",
        }}
      >
        <Shield size={20} style={{ color: "var(--accent-500)" }} />
        <div>
          <h3
            style={{
              fontSize: "var(--font-size-lg)",
              fontWeight: 600,
              margin: 0,
            }}
          >
            Independent verification
          </h3>
          <p
            style={{
              fontSize: "var(--font-size-sm)",
              color: "var(--ink-muted)",
              margin: "var(--space-1) 0 0",
            }}
          >
            Check the operator's audit trail against the Stellar blockchain —
            no MongoDB access, no trust in the operator's software.
          </p>
        </div>
      </div>

      {/* ─── Step 1: credentials ──────────────────────────────────────── */}
      <Card>
        <CardHeader
          title={
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <Key size={14} />
              Credentials
            </span>
          }
          actions={
            <span style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
              {handoff && <Badge tone="success">Autofilled</Badge>}
              <Button
                onClick={loadHandoff}
                loading={handoffLoading}
                disabled={handoffLoading}
                variant="ghost"
              >
                Load operator handoff
              </Button>
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
          You need three things from the operator: the audit contract ID, a
          Stellar RPC endpoint, and your age secret key for decrypting batches.
          When this app shares a machine with the operator's Dev Mode setup,
          they are filled in automatically.
        </p>

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

        <div
          style={{
            display: "grid",
            gap: "var(--space-3)",
            marginTop: handoff ? "var(--space-3)" : 0,
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
                  background: "var(--surface)",
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
              color: "var(--accent-600)",
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

      {/* ─── Step 2: rebuild & compare ────────────────────────────────── */}
      <Card>
        <CardHeader
          title={
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <RefreshCw size={14} />
              Rebuild &amp; compare
            </span>
          }
          actions={
            report && (
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
                  ? "Tamper detected"
                  : report.onchainRootFound
                    ? "Roots match"
                    : "No data"}
              </Badge>
            )
          }
        />
        <p
          style={{
            fontSize: "var(--font-size-sm)",
            color: "var(--ink-muted)",
            marginBottom: "var(--space-3)",
          }}
        >
          Downloads the encrypted audit batches from IPFS, decrypts them with
          your key, rebuilds the Merkle root locally, and compares it against
          the root anchored on Stellar. A mismatch means the log was altered
          after it was committed.
        </p>
        <div
          style={{
            display: "flex",
            gap: "var(--space-3)",
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
              Complete the credentials above to enable rebuild
            </span>
          )}
        </div>

        {error && (
          <Alert tone="danger" style={{ marginTop: "var(--space-3)" }}>
            {error}
          </Alert>
        )}

        {report && (
          <div
            style={{
              marginTop: "var(--space-3)",
              paddingTop: "var(--space-3)",
              borderTop: "1px solid var(--border)",
            }}
          >
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
          </div>
        )}
      </Card>

      {/* ─── Step 3: independent proof verification ──────────────────── */}
      <Card>
        <CardHeader
          title={
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <ShieldCheck size={14} />
              Verify a proof bundle
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
          Paste a proof bundle exported by the operator — either a plain
          inclusion proof or an Audited-Action Disclosure proof (a ZK claim
          about a still-private event). The pairing check runs inside the
          Soroban runtime via a read-only simulation from this machine — no
          transaction, no fees, no keys, and no trust in the operator's
          software.
          <InfoPopover label="Help" title="Independent Verification">
            <p>
              The contract's <code>verify_inclusion</code> and{" "}
              <code>verify_disclosure</code> functions are permissionless:
              they check the proof against verifying keys pinned at
              deployment and the on-chain committed-root index.
            </p>
            <p>
              Because this app calls the Stellar RPC directly, a valid result
              proves the claim against the anchored audit log even if the
              operator is fully malicious. A disclosure proof reveals only
              its claim (operation, collection, time range) — never the
              document, database, or exact timestamp.
            </p>
          </InfoPopover>
        </p>
        <textarea
          value={bundleText}
          onChange={(e) => setBundleText(e.target.value)}
          placeholder={'{"rootHex":"…","leafHex":"…","proof":{"a":"…","b":"…","c":"…"}}'}
          rows={5}
          style={{
            width: "100%",
            padding: "8px 12px",
            borderRadius: "var(--radius-md)",
            border: "1px solid var(--border)",
            background: "var(--surface)",
            color: "var(--ink)",
            fontFamily: "var(--font-mono)",
            fontSize: "var(--font-size-sm)",
            resize: "vertical",
            marginBottom: "var(--space-3)",
          }}
        />
        <div
          style={{
            display: "flex",
            gap: "var(--space-3)",
            alignItems: "center",
          }}
        >
          <Button
            onClick={handleVerifyBundle}
            loading={verifyLoading}
            disabled={verifyLoading || bundleText.trim().length === 0}
            variant="primary"
          >
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <ShieldCheck size={16} />
              Verify On-Chain
            </span>
          </Button>
          {verifyResult && (
            <Badge tone={verifyResult.verified ? "success" : "danger"}>
              {verifyResult.verified ? "Proof Valid" : "Proof Invalid"}
            </Badge>
          )}
        </div>
        {verifyError && (
          <Alert tone="danger" style={{ marginTop: "var(--space-3)" }}>
            {verifyError}
          </Alert>
        )}
        {verifyResult && !verifyResult.verified && verifyResult.reason && (
          <Alert tone="danger" style={{ marginTop: "var(--space-3)" }}>
            {verifyResult.reason}
          </Alert>
        )}
        {verifyResult?.verified && (
          <Alert tone="success" style={{ marginTop: "var(--space-3)" }}>
            {verifiedClaimText ? (
              <>
                The Soroban runtime confirmed: <b>{verifiedClaimText}</b>.
                Nothing else about the event was revealed. Verified
                independently — the operator was not involved.
              </>
            ) : (
              <>
                The Soroban runtime confirmed this leaf is included in a
                Merkle root that was anchored on-chain. Verified
                independently — the operator was not involved.
              </>
            )}
          </Alert>
        )}

        {/* ─── Record the verification on-chain (optional, signed) ──── */}
        {verifiedBundle && verifyResult?.verified && (
          <div
            style={{
              marginTop: "var(--space-4)",
              paddingTop: "var(--space-3)",
              borderTop: "1px solid var(--border)",
            }}
          >
            <div style={{ fontWeight: 600, fontSize: "var(--font-size-sm)", marginBottom: "var(--space-1)" }}>
              Record this verification on-chain
              <InfoPopover label="Help" title="Recorded Verification">
                <p>
                  Submits the same verification as a transaction signed with{" "}
                  <b>your</b> Stellar account. The contract stores a permanent
                  record — who verified, the exact claim, and when — and the
                  tx hash becomes citable evidence in audit reports or
                  disputes.
                </p>
                <p>
                  This costs a small network fee and requires a funded
                  account (testnet: fund via friendbot). The free
                  verification above is otherwise identical.
                </p>
              </InfoPopover>
            </div>
            <p
              style={{
                fontSize: "var(--font-size-xs)",
                color: "var(--ink-muted)",
                margin: "0 0 var(--space-2)",
              }}
            >
              Optional: sign with your own Stellar key to publish a permanent,
              third-party-attributable record of this verification.
              {handoff?.attesterStellarSecret &&
                recordSecretKey === handoff.attesterStellarSecret &&
                " Pre-filled with the dev attester key from the operator handoff."}
            </p>
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <input
                type={revealRecordKey ? "text" : "password"}
                value={recordSecretKey}
                onChange={(e) => setRecordSecretKey(e.target.value)}
                placeholder="Your Stellar secret key (S...)"
                style={{
                  flex: 1,
                  padding: "8px 12px",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--border)",
                  background: "var(--surface)",
                  color: "var(--ink)",
                  fontFamily: "var(--font-mono)",
                  fontSize: "var(--font-size-sm)",
                }}
              />
              <button
                type="button"
                onClick={() => setRevealRecordKey((s) => !s)}
                style={{
                  padding: "8px 10px",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--border)",
                  background: "var(--surface-2)",
                  cursor: "pointer",
                }}
                title={revealRecordKey ? "Hide secret" : "Reveal secret"}
              >
                {revealRecordKey ? <EyeOff size={16} /> : <Eye size={16} />}
              </button>
              <Button
                onClick={handleRecordVerification}
                loading={recordLoading}
                disabled={recordLoading || !recordSecretKey.trim()}
                variant="secondary"
              >
                Verify &amp; Record
              </Button>
            </div>
            {recordResult && (
              <div style={{ marginTop: "var(--space-3)" }}>
                <Alert tone="success" compact>
                  Recorded as <b>#{recordResult.recordId}</b> by{" "}
                  <span style={{ fontFamily: "var(--font-mono)" }}>
                    {shortHash(recordResult.verifier)}
                  </span>
                  . The tx hash below is permanent, citable evidence.
                </Alert>
                <div
                  style={{
                    marginTop: "var(--space-2)",
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-2)",
                    fontFamily: "var(--font-mono)",
                  }}
                >
                  <span
                    style={{ color: "var(--ink-muted)", minWidth: 160, flexShrink: 0 }}
                  >
                    Tx hash:
                  </span>
                  <TxHashLink
                    txHash={recordResult.txHash}
                    network={networkFromRpcUrl(form.rpcUrl)}
                  />
                  <CopyButton value={recordResult.txHash} />
                </div>
              </div>
            )}
          </div>
        )}
      </Card>

      {/* ─── Recorded verifications ─────────────────────────────────── */}
      <Card>
        <CardHeader
          title={
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <Shield size={14} />
              Recorded Verifications
            </span>
          }
          actions={
            <Button
              variant="ghost"
              onClick={handleLoadRecords}
              loading={recordsLoading}
              disabled={recordsLoading}
            >
              <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <RefreshCw size={14} />
                Load
              </span>
            </Button>
          }
        />
        <p
          style={{
            fontSize: "var(--font-size-sm)",
            color: "var(--ink-muted)",
            marginBottom: records && records.length > 0 ? "var(--space-3)" : 0,
          }}
        >
          The contract's append-only log of recorded disclosure verifications
          — who verified what, and when. Readable by anyone.
        </p>
        {records && records.length === 0 && (
          <p style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-faint)", margin: 0 }}>
            No recorded verifications yet.
          </p>
        )}
        {records && records.length > 0 && (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              borderRadius: "var(--radius-md)",
              border: "1px solid var(--border)",
              overflow: "hidden",
            }}
          >
            {records.map((rec) => (
              <div
                key={rec.recordId}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-3)",
                  padding: "var(--space-2) var(--space-3)",
                  borderBottom: "1px solid var(--border)",
                  fontSize: "var(--font-size-xs)",
                }}
              >
                <Badge tone="success">#{rec.recordId}</Badge>
                <span style={{ fontFamily: "var(--font-mono)", color: "var(--ink-muted)" }}>
                  {shortHash(rec.verifier)}
                </span>
                <span style={{ color: "var(--ink-faint)" }}>
                  {[
                    rec.claim.checkOp && "op",
                    rec.claim.checkColl && "collection",
                    rec.claim.checkTs && "time range",
                  ]
                    .filter(Boolean)
                    .join(" + ") || "inclusion only"}
                </span>
                <span style={{ flex: 1, fontFamily: "var(--font-mono)", color: "var(--ink-faint)" }}>
                  root {shortHash(rec.rootHex)}
                </span>
                <span style={{ color: "var(--ink-faint)" }}>
                  {rec.timestamp
                    ? new Date(rec.timestamp * 1000).toLocaleString()
                    : "—"}
                </span>
              </div>
            ))}
          </div>
        )}
      </Card>
      </div>
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
          background: "var(--surface)",
          color: "var(--ink)",
          fontFamily: type === "password" ? undefined : "var(--font-mono)",
          fontSize: "var(--font-size-sm)",
        }}
      />
    </div>
  );
}

function CopyButton({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(value).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [value]);

  return (
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
  const display = value.length > 40 ? shortHash(value) : value;

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
      {copyable && <CopyButton value={value} />}
    </div>
  );
}
