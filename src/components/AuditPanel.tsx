import { useEffect, useState, useCallback } from "react";
import commands, { AuditStatus, AuditEvent } from "../ipc/commands";

/**
 * ZK Audit Log panel.
 *
 * Displays the current Merkle root, leaf count, and event list.
 * Allows generating inclusion proofs for individual events.
 */
export default function AuditPanel() {
  const [status, setStatus] = useState<AuditStatus | null>(null);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [proofIndex, setProofIndex] = useState<number | null>(null);
  const [proofResult, setProofResult] = useState<string | null>(null);
  const [proofLoading, setProofLoading] = useState(false);

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
      // Default circuit paths (relative to the project root).
      const r1csPath = "../zk-spike/circuits/build/merkle_inclusion.r1cs";
      const wasmPath =
        "../zk-spike/circuits/build/merkle_inclusion_js/merkle_inclusion.wasm";
      const result = await commands.auditGenerateProof(
        index,
        r1csPath,
        wasmPath,
      );
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

      {proofResult && (
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
                              : "#c33",
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
    </div>
  );
}
