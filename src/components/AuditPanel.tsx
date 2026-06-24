import { useEffect, useState, useCallback } from "react";
import commands, {
  AuditStatus,
  AuditEvent,
  CommitResult,
  OnChainRoot,
  VerificationReport,
  IpfsPublishResult,
  Publisher,
  AttestationStatus,
} from "../ipc/commands";

/**
 * ZK Audit Log panel.
 *
 * Displays the current Merkle root, leaf count, and event list.
 * Allows generating inclusion proofs for individual events.
 * Supports committing roots to Stellar testnet and querying on-chain state.
 */
export default function AuditPanel() {
  const [status, setStatus] = useState<AuditStatus | null>(null);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [proofIndex, setProofIndex] = useState<number | null>(null);
  const [proofResult, setProofResult] = useState<string | null>(null);
  const [proofLoading, setProofLoading] = useState(false);
  const [commitLoading, setCommitLoading] = useState(false);
  const [commitResult, setCommitResult] = useState<CommitResult | null>(null);
  const [onchainRoot, setOnchainRoot] = useState<OnChainRoot | null>(null);
  const [onchainLoading, setOnchainLoading] = useState(false);
  const [activeTab, setActiveTab] = useState<
    "events" | "reader" | "ipfs" | "attestation"
  >("events");

  // Reader mode state
  const [verificationReport, setVerificationReport] =
    useState<VerificationReport | null>(null);
  const [verifyLoading, setVerifyLoading] = useState(false);

  // IPFS state
  const [ipfsDaemonOnline, setIpfsDaemonOnline] = useState<boolean | null>(
    null,
  );
  const [ipfsPublishResult, setIpfsPublishResult] =
    useState<IpfsPublishResult | null>(null);
  const [ipfsLoading, setIpfsLoading] = useState(false);
  const [ipfsEpochNumber, setIpfsEpochNumber] = useState<number>(0);

  // Attestation state
  const [publishers, setPublishers] = useState<Publisher[]>([]);
  const [attestationStatus, setAttestationStatus] =
    useState<AttestationStatus | null>(null);
  const [attestationThreshold, setAttestationThreshold] = useState<number>(2);
  const [newPublisherKey, setNewPublisherKey] = useState("");
  const [newPublisherName, setNewPublisherName] = useState("");
  const [attestationLoading, setAttestationLoading] = useState(false);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    setError(null);
    try {
      const [s, e] = await Promise.all([
        commands.auditGetStatus(),
        commands.auditListEvents(),
      ]);
      setStatus(s);
      setEvents(e);
    } catch (err) {
      setError(String(err));
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleGenerateProof = async (index: number) => {
    setProofIndex(index);
    setProofLoading(true);
    setProofResult(null);
    setError(null);
    try {
      // Use bundled circuit resources (no explicit paths needed).
      const result = await commands.auditGenerateProof(index);
      setProofResult(
        `Proof generated for leaf ${index}.\nRoot: ${result.rootHex}\n` +
          `Public signal: ${result.pubSignals[0]}\n` +
          `Proof A: ${result.proof.a.slice(0, 32)}...`,
      );
    } catch (err) {
      setError(String(err));
    } finally {
      setProofLoading(false);
    }
  };

  const handleCommitRoot = async () => {
    setCommitLoading(true);
    setError(null);
    setCommitResult(null);
    try {
      const result = await commands.auditCommitRoot();
      setCommitResult(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setCommitLoading(false);
    }
  };

  const handleCheckOnchain = async () => {
    setOnchainLoading(true);
    setError(null);
    try {
      const result = await commands.auditGetOnchainRoot();
      setOnchainRoot(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setOnchainLoading(false);
    }
  };

  // ─── Reader mode ───────────────────────────────────────────────────
  const handleVerifyReaderMode = async () => {
    setVerifyLoading(true);
    setError(null);
    try {
      const report = await commands.auditVerifyReaderMode();
      setVerificationReport(report);
    } catch (err) {
      setError(String(err));
    } finally {
      setVerifyLoading(false);
    }
  };

  // ─── IPFS ──────────────────────────────────────────────────────────
  const handleCheckIpfsDaemon = async () => {
    setError(null);
    try {
      const online = await commands.auditCheckIpfsDaemon();
      setIpfsDaemonOnline(online);
    } catch (err) {
      setError(String(err));
    }
  };

  const handlePublishToIpfs = async () => {
    setIpfsLoading(true);
    setError(null);
    setIpfsPublishResult(null);
    try {
      const result = await commands.auditPublishEpochToIpfs(ipfsEpochNumber);
      setIpfsPublishResult(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setIpfsLoading(false);
    }
  };

  // ─── Attestation ───────────────────────────────────────────────────
  const refreshPublishers = useCallback(async () => {
    try {
      const list = await commands.auditListPublishers();
      setPublishers(list);
      const threshold = await commands.auditGetAttestationThreshold();
      setAttestationThreshold(threshold);
    } catch (err) {
      // Ignore — attestation may not be initialized
    }
  }, []);

  const handleAddPublisher = async () => {
    if (!newPublisherKey.trim() || !newPublisherName.trim()) return;
    setAttestationLoading(true);
    setError(null);
    try {
      await commands.auditAddPublisher(
        newPublisherKey.trim(),
        newPublisherName.trim(),
      );
      setNewPublisherKey("");
      setNewPublisherName("");
      await refreshPublishers();
    } catch (err) {
      setError(String(err));
    } finally {
      setAttestationLoading(false);
    }
  };

  const handleRemovePublisher = async (publicKey: string) => {
    setError(null);
    try {
      await commands.auditRemovePublisher(publicKey);
      await refreshPublishers();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleSetThreshold = async () => {
    setError(null);
    try {
      await commands.auditSetAttestationThreshold(attestationThreshold);
    } catch (err) {
      setError(String(err));
    }
  };

  const handleCheckAttestationStatus = async () => {
    if (!status) return;
    setAttestationLoading(true);
    setError(null);
    try {
      const result = await commands.auditGetAttestationStatus(
        0,
        status.rootHex,
      );
      setAttestationStatus(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setAttestationLoading(false);
    }
  };

  useEffect(() => {
    if (activeTab === "attestation") {
      refreshPublishers();
    }
  }, [activeTab, refreshPublishers]);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        padding: "16px",
        gap: "12px",
        overflow: "auto",
        fontFamily: "monospace",
        fontSize: "13px",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "16px" }}>ZK Audit Log</h2>
        <button
          onClick={refresh}
          disabled={refreshing}
          style={{
            padding: "4px 12px",
            fontSize: "12px",
            cursor: refreshing ? "wait" : "pointer",
          }}
        >
          {refreshing ? "Refreshing..." : "Refresh"}
        </button>
      </div>

      {error && (
        <div
          style={{
            padding: "8px 12px",
            background: "#fee",
            border: "1px solid #c33",
            borderRadius: "4px",
            color: "#c33",
          }}
        >
          {error}
        </div>
      )}

      {/* Tab navigation */}
      <div style={{ display: "flex", gap: "4px", borderBottom: "1px solid var(--border, #333)" }}>
        {(["events", "reader", "ipfs", "attestation"] as const).map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              padding: "6px 14px",
              fontSize: "12px",
              cursor: "pointer",
              background: activeTab === tab ? "var(--bg-secondary, #1a1a2e)" : "transparent",
              border: "none",
              borderBottom: activeTab === tab ? "2px solid var(--accent, #0ff)" : "2px solid transparent",
              color: activeTab === tab ? "var(--accent, #0ff)" : "inherit",
              opacity: activeTab === tab ? 1 : 0.6,
            }}
          >
            {tab === "events" ? "Events" : tab === "reader" ? "Reader Mode" : tab === "ipfs" ? "IPFS" : "Attestation"}
          </button>
        ))}
      </div>

      {status && (
        <div
          style={{
            padding: "12px",
            background: "var(--bg-secondary, #1a1a2e)",
            border: "1px solid var(--border, #333)",
            borderRadius: "6px",
          }}
        >
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "auto 1fr",
              gap: "4px 12px",
            }}
          >
            <span style={{ opacity: 0.6 }}>Merkle Root:</span>
            <span
              style={{
                fontWeight: "bold",
                color: "var(--accent, #0ff)",
                wordBreak: "break-all",
              }}
            >
              0x{status.rootHex}
            </span>
            <span style={{ opacity: 0.6 }}>Leaves:</span>
            <span>{status.leafCount}</span>
            <span style={{ opacity: 0.6 }}>Events:</span>
            <span>{status.eventCount}</span>
            <span style={{ opacity: 0.6 }}>Tree Height:</span>
            <span>{status.treeHeight} levels</span>
          </div>
        </div>
      )}

      {/* On-chain commitment section — shown on Events and Reader tabs */}
      {(activeTab === "events" || activeTab === "reader") && (
      <div
        style={{
          padding: "12px",
          background: "var(--bg-secondary, #1a1a2e)",
          border: "1px solid var(--border, #333)",
          borderRadius: "6px",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            marginBottom: "8px",
          }}
        >
          <strong style={{ fontSize: "13px" }}>Stellar On-Chain Commitment</strong>
          <div style={{ display: "flex", gap: "8px" }}>
            <button
              onClick={handleCommitRoot}
              disabled={commitLoading || !status}
              style={{
                padding: "4px 12px",
                fontSize: "11px",
                cursor: commitLoading ? "wait" : "pointer",
                background: "var(--accent, #0ff)",
                color: "#000",
                border: "none",
                borderRadius: "3px",
              }}
            >
              {commitLoading ? "Committing..." : "Commit Root"}
            </button>
            <button
              onClick={handleCheckOnchain}
              disabled={onchainLoading}
              style={{
                padding: "4px 12px",
                fontSize: "11px",
                cursor: onchainLoading ? "wait" : "pointer",
              }}
            >
              {onchainLoading ? "Checking..." : "Check On-Chain"}
            </button>
          </div>
        </div>
        {commitResult && (
          <div style={{ fontSize: "11px", opacity: 0.8 }}>
            <span style={{ opacity: 0.6 }}>Committed:</span> seq #
            {commitResult.sequence} · root 0x{commitResult.rootHex.slice(0, 16)}
            ...
            {commitResult.txHash && (
              <>
                {" · "}
                <a
                  href={`https://stellar.expert/explorer/testnet/tx/${commitResult.txHash}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{ color: "var(--accent, #0ff)" }}
                >
                  tx ↗
                </a>
              </>
            )}
          </div>
        )}
        {onchainRoot && (
          <div style={{ fontSize: "11px", opacity: 0.8 }}>
            <span style={{ opacity: 0.6 }}>On-chain root:</span> seq #
            {onchainRoot.sequence} · 0x{onchainRoot.rootHex.slice(0, 16)}... ·
            ts {onchainRoot.timestamp}
            {onchainRoot.metadata && ` · "${onchainRoot.metadata}"`}
          </div>
        )}
        {!onchainRoot && !onchainLoading && (
          <div style={{ fontSize: "11px", opacity: 0.4 }}>
            No on-chain root queried yet. Click "Check On-Chain" to fetch the
            latest committed root from Stellar testnet.
          </div>
        )}
      </div>
      )}

      {/* ─── Reader Mode tab ─────────────────────────────────────────── */}
      {activeTab === "reader" && (
        <div
          style={{
            padding: "12px",
            background: "var(--bg-secondary, #1a1a2e)",
            border: "1px solid var(--border, #333)",
            borderRadius: "6px",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "8px" }}>
            <strong style={{ fontSize: "13px" }}>Reader Mode — Verify Against On-Chain Root</strong>
            <button
              onClick={handleVerifyReaderMode}
              disabled={verifyLoading}
              style={{
                padding: "4px 12px",
                fontSize: "11px",
                cursor: verifyLoading ? "wait" : "pointer",
                background: "var(--accent, #0ff)",
                color: "#000",
                border: "none",
                borderRadius: "3px",
              }}
            >
              {verifyLoading ? "Verifying..." : "Verify"}
            </button>
          </div>
          {verificationReport && (
            <div style={{ fontSize: "12px" }}>
              <div
                style={{
                  padding: "8px 12px",
                  marginBottom: "8px",
                  background: verificationReport.tamperDetected
                    ? "#400"
                    : verificationReport.onchainRootFound
                      ? "#040"
                      : "#440",
                  border: `1px solid ${verificationReport.tamperDetected ? "#c33" : verificationReport.onchainRootFound ? "#0a3" : "#aa0"}`,
                  borderRadius: "4px",
                }}
              >
                {verificationReport.summary}
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 12px" }}>
                <span style={{ opacity: 0.6 }}>On-chain root found:</span>
                <span>{verificationReport.onchainRootFound ? "yes" : "no"}</span>
                <span style={{ opacity: 0.6 }}>Commitment event:</span>
                <span>{verificationReport.commitmentEventIndex ?? "—"}</span>
                <span style={{ opacity: 0.6 }}>Total events:</span>
                <span>{verificationReport.totalEvents}</span>
                <span style={{ opacity: 0.6 }}>Verified events:</span>
                <span>{verificationReport.verifiedEvents}</span>
                <span style={{ opacity: 0.6 }}>Events after commitment:</span>
                <span>{verificationReport.eventsAfterCommitment}</span>
                <span style={{ opacity: 0.6 }}>Chain intact:</span>
                <span style={{ color: verificationReport.chainIntact ? "#0a3" : "#c33" }}>
                  {verificationReport.chainIntact ? "yes" : "no"}
                </span>
                <span style={{ opacity: 0.6 }}>Tamper detected:</span>
                <span style={{ color: verificationReport.tamperDetected ? "#c33" : "#0a3" }}>
                  {verificationReport.tamperDetected ? "YES" : "no"}
                </span>
              </div>
            </div>
          )}
          {!verificationReport && !verifyLoading && (
            <div style={{ fontSize: "11px", opacity: 0.4 }}>
              Click "Verify" to check the local audit log against the latest
              on-chain root. This reads the on-chain root from Stellar and
              verifies the root chain up to the commitment point.
            </div>
          )}
        </div>
      )}

      {/* ─── IPFS tab ────────────────────────────────────────────────── */}
      {activeTab === "ipfs" && (
        <div
          style={{
            padding: "12px",
            background: "var(--bg-secondary, #1a1a2e)",
            border: "1px solid var(--border, #333)",
            borderRadius: "6px",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "8px" }}>
            <strong style={{ fontSize: "13px" }}>IPFS Batch Publishing</strong>
            <button
              onClick={handleCheckIpfsDaemon}
              style={{ padding: "4px 12px", fontSize: "11px", cursor: "pointer" }}
            >
              Check Daemon
            </button>
          </div>
          {ipfsDaemonOnline !== null && (
            <div style={{ fontSize: "11px", marginBottom: "8px" }}>
              <span style={{ opacity: 0.6 }}>Daemon:</span>{" "}
              <span style={{ color: ipfsDaemonOnline ? "#0a3" : "#c33" }}>
                {ipfsDaemonOnline ? "online" : "offline"}
              </span>
            </div>
          )}
          <div style={{ display: "flex", gap: "8px", alignItems: "center", marginBottom: "8px" }}>
            <label style={{ fontSize: "11px", opacity: 0.6 }}>Epoch:</label>
            <input
              type="number"
              min={0}
              value={ipfsEpochNumber}
              onChange={(e) => setIpfsEpochNumber(Number(e.target.value))}
              style={{
                width: "60px",
                padding: "2px 6px",
                fontSize: "12px",
                background: "var(--bg-primary, #111)",
                border: "1px solid var(--border, #333)",
                borderRadius: "3px",
                color: "inherit",
              }}
            />
            <button
              onClick={handlePublishToIpfs}
              disabled={ipfsLoading}
              style={{
                padding: "4px 12px",
                fontSize: "11px",
                cursor: ipfsLoading ? "wait" : "pointer",
                background: "var(--accent, #0ff)",
                color: "#000",
                border: "none",
                borderRadius: "3px",
              }}
            >
              {ipfsLoading ? "Publishing..." : "Publish to IPFS"}
            </button>
          </div>
          {ipfsPublishResult && (
            <div style={{ fontSize: "12px" }}>
              <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 12px" }}>
                <span style={{ opacity: 0.6 }}>CID:</span>
                <span style={{ wordBreak: "break-all", color: "var(--accent, #0ff)" }}>
                  {ipfsPublishResult.cid}
                </span>
                <span style={{ opacity: 0.6 }}>Epoch:</span>
                <span>{ipfsPublishResult.epochNumber}</span>
                <span style={{ opacity: 0.6 }}>Events:</span>
                <span>{ipfsPublishResult.eventCount}</span>
                <span style={{ opacity: 0.6 }}>Size:</span>
                <span>{ipfsPublishResult.batchSizeBytes} bytes</span>
                <span style={{ opacity: 0.6 }}>Gateway:</span>
                <span>
                  <a
                    href={ipfsPublishResult.gatewayUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    style={{ color: "var(--accent, #0ff)" }}
                  >
                    {ipfsPublishResult.gatewayUrl} ↗
                  </a>
                </span>
              </div>
            </div>
          )}
          {!ipfsPublishResult && !ipfsLoading && (
            <div style={{ fontSize: "11px", opacity: 0.4 }}>
              Publish an epoch's event batch to IPFS for decentralized
              verification. Requires a running IPFS daemon (Kubo) at
              localhost:5001.
            </div>
          )}
        </div>
      )}

      {/* ─── Attestation tab ─────────────────────────────────────────── */}
      {activeTab === "attestation" && (
        <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
          <div
            style={{
              padding: "12px",
              background: "var(--bg-secondary, #1a1a2e)",
              border: "1px solid var(--border, #333)",
              borderRadius: "6px",
            }}
          >
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "8px" }}>
              <strong style={{ fontSize: "13px" }}>Threshold Attestation (K-of-N)</strong>
              <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
                <label style={{ fontSize: "11px", opacity: 0.6 }}>K:</label>
                <input
                  type="number"
                  min={1}
                  value={attestationThreshold}
                  onChange={(e) => setAttestationThreshold(Number(e.target.value))}
                  style={{
                    width: "40px",
                    padding: "2px 6px",
                    fontSize: "12px",
                    background: "var(--bg-primary, #111)",
                    border: "1px solid var(--border, #333)",
                    borderRadius: "3px",
                    color: "inherit",
                  }}
                />
                <button
                  onClick={handleSetThreshold}
                  style={{ padding: "3px 10px", fontSize: "11px", cursor: "pointer" }}
                >
                  Set
                </button>
                <button
                  onClick={handleCheckAttestationStatus}
                  disabled={attestationLoading || !status}
                  style={{ padding: "3px 10px", fontSize: "11px", cursor: "pointer" }}
                >
                  Check Status
                </button>
              </div>
            </div>
            {attestationStatus && (
              <div style={{ fontSize: "12px" }}>
                <div
                  style={{
                    padding: "6px 10px",
                    marginBottom: "6px",
                    background: attestationStatus.thresholdMet ? "#040" : "#440",
                    border: `1px solid ${attestationStatus.thresholdMet ? "#0a3" : "#aa0"}`,
                    borderRadius: "3px",
                  }}
                >
                  {attestationStatus.thresholdMet ? "✅" : "⏳"} Threshold{" "}
                  {attestationStatus.validAttestations}/{attestationStatus.threshold}{" "}
                  (of {attestationStatus.totalPublishers} publishers)
                </div>
                {attestationStatus.attestedBy.length > 0 && (
                  <div style={{ fontSize: "11px", opacity: 0.7 }}>
                    <strong>Attested by:</strong>{" "}
                    {attestationStatus.attestedBy.map((pk) => pk.slice(0, 12) + "...").join(", ")}
                  </div>
                )}
                {attestationStatus.pending.length > 0 && (
                  <div style={{ fontSize: "11px", opacity: 0.5 }}>
                    <strong>Pending:</strong>{" "}
                    {attestationStatus.pending.map((pk) => pk.slice(0, 12) + "...").join(", ")}
                  </div>
                )}
              </div>
            )}
          </div>

          <div
            style={{
              padding: "12px",
              background: "var(--bg-secondary, #1a1a2e)",
              border: "1px solid var(--border, #333)",
              borderRadius: "6px",
            }}
          >
            <strong style={{ fontSize: "13px" }}>Publishers ({publishers.length})</strong>
            <div style={{ display: "flex", gap: "8px", margin: "8px 0" }}>
              <input
                type="text"
                placeholder="Public key (hex, 32 bytes)"
                value={newPublisherKey}
                onChange={(e) => setNewPublisherKey(e.target.value)}
                style={{
                  flex: 1,
                  padding: "4px 8px",
                  fontSize: "11px",
                  background: "var(--bg-primary, #111)",
                  border: "1px solid var(--border, #333)",
                  borderRadius: "3px",
                  color: "inherit",
                  fontFamily: "monospace",
                }}
              />
              <input
                type="text"
                placeholder="Name"
                value={newPublisherName}
                onChange={(e) => setNewPublisherName(e.target.value)}
                style={{
                  width: "100px",
                  padding: "4px 8px",
                  fontSize: "11px",
                  background: "var(--bg-primary, #111)",
                  border: "1px solid var(--border, #333)",
                  borderRadius: "3px",
                  color: "inherit",
                }}
              />
              <button
                onClick={handleAddPublisher}
                disabled={attestationLoading || !newPublisherKey.trim() || !newPublisherName.trim()}
                style={{
                  padding: "4px 12px",
                  fontSize: "11px",
                  cursor: "pointer",
                  background: "var(--accent, #0ff)",
                  color: "#000",
                  border: "none",
                  borderRadius: "3px",
                }}
              >
                Add
              </button>
            </div>
            {publishers.length === 0 ? (
              <div style={{ fontSize: "11px", opacity: 0.4 }}>
                No publishers registered. Add a publisher with their ed25519
                public key to enable threshold attestation.
              </div>
            ) : (
              <table style={{ width: "100%", borderCollapse: "collapse", fontSize: "11px" }}>
                <thead>
                  <tr style={{ textAlign: "left", borderBottom: "1px solid var(--border, #333)" }}>
                    <th style={{ padding: "4px 8px" }}>Name</th>
                    <th style={{ padding: "4px 8px" }}>Public Key</th>
                    <th style={{ padding: "4px 8px" }}>Registered</th>
                    <th style={{ padding: "4px 8px" }}></th>
                  </tr>
                </thead>
                <tbody>
                  {publishers.map((p) => (
                    <tr key={p.publicKey} style={{ borderBottom: "1px solid var(--border, #222)" }}>
                      <td style={{ padding: "4px 8px" }}>{p.name}</td>
                      <td style={{ padding: "4px 8px", fontFamily: "monospace", opacity: 0.7 }}>
                        {p.publicKey.slice(0, 24)}...
                      </td>
                      <td style={{ padding: "4px 8px", opacity: 0.5 }}>
                        {p.registeredAt.slice(0, 10)}
                      </td>
                      <td style={{ padding: "4px 8px" }}>
                        <button
                          onClick={() => handleRemovePublisher(p.publicKey)}
                          style={{ padding: "2px 6px", fontSize: "10px", cursor: "pointer" }}
                        >
                          Remove
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </div>
      )}

      {proofResult && activeTab === "events" && (
        <div
          style={{
            padding: "12px",
            background: "var(--bg-secondary, #1a1a2e)",
            border: "1px solid var(--accent, #0ff)",
            borderRadius: "6px",
            whiteSpace: "pre-wrap",
          }}
        >
          <strong>Proof Result:</strong>
          {"\n"}
          {proofResult}
          <button
            onClick={() => setProofResult(null)}
            style={{ float: "right", fontSize: "11px", cursor: "pointer" }}
          >
            ✕
          </button>
        </div>
      )}

      {activeTab === "events" && (
      <div>
        <h3 style={{ fontSize: "14px", margin: "0 0 8px 0" }}>
          Audit Events ({events.length})
        </h3>
        {events.length === 0 ? (
          <div style={{ opacity: 0.5, padding: "12px" }}>
            No audit events recorded yet. Perform an insert, update, or delete
            operation to generate audit entries.
          </div>
        ) : (
          <table
            style={{
              width: "100%",
              borderCollapse: "collapse",
              fontSize: "12px",
            }}
          >
            <thead>
              <tr
                style={{
                  borderBottom: "1px solid var(--border, #333)",
                  textAlign: "left",
                }}
              >
                <th style={{ padding: "4px 8px" }}>#</th>
                <th style={{ padding: "4px 8px" }}>Operation</th>
                <th style={{ padding: "4px 8px" }}>Database</th>
                <th style={{ padding: "4px 8px" }}>Collection</th>
                <th style={{ padding: "4px 8px" }}>Leaf Hash</th>
                <th style={{ padding: "4px 8px" }}>Timestamp</th>
                <th style={{ padding: "4px 8px" }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {events.map((event) => (
                <tr
                  key={event.index}
                  style={{ borderBottom: "1px solid var(--border, #222)" }}
                >
                  <td style={{ padding: "4px 8px" }}>{event.index}</td>
                  <td style={{ padding: "4px 8px" }}>
                    <span
                      style={{
                        padding: "1px 6px",
                        borderRadius: "3px",
                        fontSize: "11px",
                        background:
                          event.operation === "insert"
                            ? "#0a3"
                            : event.operation === "update"
                              ? "#aa0"
                              : event.operation === "delete"
                                ? "#c33"
                                : event.operation === "drop_collection" ||
                                    event.operation === "drop_database"
                                  ? "#a30"
                                  : event.operation === "create_index"
                                    ? "#06a"
                                    : event.operation === "drop_index"
                                      ? "#a06"
                                      : event.operation === "rename"
                                        ? "#66a"
                                        : "#888",
                        color: "#fff",
                      }}
                    >
                      {event.operation}
                    </span>
                  </td>
                  <td style={{ padding: "4px 8px" }}>{event.database}</td>
                  <td style={{ padding: "4px 8px" }}>{event.collection}</td>
                  <td
                    style={{
                      padding: "4px 8px",
                      opacity: 0.7,
                      fontSize: "11px",
                      maxWidth: "120px",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                    title={event.leafHex}
                  >
                    {event.leafHex.slice(0, 16)}...
                  </td>
                  <td
                    style={{
                      padding: "4px 8px",
                      opacity: 0.6,
                      fontSize: "11px",
                    }}
                  >
                    {event.timestamp.slice(11, 19)}
                  </td>
                  <td style={{ padding: "4px 8px" }}>
                    <button
                      onClick={() => handleGenerateProof(event.index)}
                      disabled={
                        proofLoading && proofIndex === event.index
                      }
                      style={{
                        padding: "2px 8px",
                        fontSize: "11px",
                        cursor: "pointer",
                      }}
                    >
                      {proofLoading && proofIndex === event.index
                        ? "Proving..."
                        : "Prove"}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      )}
    </div>
  );
}
