/**
 * SafeChangeModal — Safe Change Mode preview dialog.
 *
 * Shows the user a preview of what a write operation will do before it runs:
 *  - Matched document count & risk score
 *  - Before/after document samples with field-level diff
 *  - Rollback plan (copyable JSON)
 *  - Typed confirmation for high-risk / production operations
 */

import { useState, useEffect, useRef, useCallback } from "react";
import { AlertTriangle, ShieldAlert, ShieldCheck, Copy, Check } from "lucide-react";
import type {
  SafeChangePreview,
  SafeChangeDocumentDiff,
  SafeChangeFieldChange,
} from "../ipc/commands";

export interface SafeChangeModalProps {
  open: boolean;
  preview: SafeChangePreview | null;
  loading: boolean;
  error: string | null;
  onConfirm: () => void;
  onCancel: () => void;
}

// ─── Risk badge ───────────────────────────────────────────────────────────────

function RiskBadge({ score }: { score: number }) {
  let label: string;
  let color: string;
  if (score >= 70) {
    label = "High risk";
    color = "var(--danger-500)";
  } else if (score >= 40) {
    label = "Moderate risk";
    color = "var(--warning-500)";
  } else {
    label = "Low risk";
    color = "var(--success-500)";
  }
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "4px",
        padding: "2px 8px",
        borderRadius: "var(--radius)",
        background: `color-mix(in oklch, ${color}, transparent 85%)`,
        color,
        fontSize: "var(--font-size-xs)",
        fontWeight: 600,
        border: `1px solid color-mix(in oklch, ${color}, transparent 65%)`,
      }}
    >
      {score >= 70 ? <ShieldAlert size={12} /> : <ShieldCheck size={12} />}
      {label} ({score}/100)
    </span>
  );
}

// ─── Diff viewer ─────────────────────────────────────────────────────────────

function FieldChangeRow({ change }: { change: SafeChangeFieldChange }) {
  const isAdded = change.changeType === "added";
  const isRemoved = change.changeType === "removed";
  const dotColor = isAdded
    ? "var(--success-500)"
    : isRemoved
    ? "var(--danger-500)"
    : "var(--warning-500)";

  return (
    <tr>
      <td
        style={{
          padding: "2px 6px",
          fontSize: "var(--font-size-xs)",
          fontFamily: "var(--font-mono)",
          color: "var(--ink)",
          whiteSpace: "nowrap",
        }}
      >
        <span
          style={{
            display: "inline-block",
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: dotColor,
            marginRight: 6,
            flexShrink: 0,
          }}
        />
        {change.field}
      </td>
      <td
        style={{
          padding: "2px 6px",
          fontSize: "var(--font-size-xs)",
          fontFamily: "var(--font-mono)",
          color: isRemoved ? "var(--danger-500)" : "var(--ink-muted)",
          textDecoration: isRemoved ? "line-through" : "none",
        }}
      >
        {change.oldValue !== undefined && change.oldValue !== null
          ? JSON.stringify(change.oldValue)
          : "–"}
      </td>
      <td
        style={{
          padding: "2px 6px",
          fontSize: "var(--font-size-xs)",
          fontFamily: "var(--font-mono)",
          color: isAdded ? "var(--success-500)" : isRemoved ? "var(--ink-muted)" : "var(--ink)",
        }}
      >
        {change.newValue !== undefined && change.newValue !== null
          ? JSON.stringify(change.newValue)
          : "–"}
      </td>
    </tr>
  );
}

function DiffTable({ diffs }: { diffs: SafeChangeDocumentDiff[] }) {
  if (diffs.length === 0) return null;
  const totalChanges = diffs.reduce((n, d) => n + d.fieldChanges.length, 0);
  if (totalChanges === 0) return null;
  return (
    <div style={{ overflowY: "auto", maxHeight: 220 }}>
      {diffs.map((diff) => (
        <div key={diff.documentIndex}>
          {diffs.length > 1 && (
            <div
              style={{
                padding: "2px 6px",
                fontSize: "var(--font-size-xs)",
                color: "var(--ink-muted)",
                background: "var(--surface-2)",
                borderBottom: "1px solid var(--border)",
              }}
            >
              Document {diff.documentIndex + 1}
            </div>
          )}
          {diff.fieldChanges.length === 0 ? (
            <div
              style={{
                padding: "4px 8px",
                fontSize: "var(--font-size-xs)",
                color: "var(--ink-muted)",
              }}
            >
              (no field changes)
            </div>
          ) : (
            <table style={{ width: "100%", borderCollapse: "collapse" }}>
              <thead>
                <tr>
                  <th
                    style={{
                      padding: "2px 6px",
                      fontSize: "var(--font-size-xs)",
                      color: "var(--ink-muted)",
                      textAlign: "left",
                      fontWeight: 500,
                    }}
                  >
                    Field
                  </th>
                  <th
                    style={{
                      padding: "2px 6px",
                      fontSize: "var(--font-size-xs)",
                      color: "var(--ink-muted)",
                      textAlign: "left",
                      fontWeight: 500,
                    }}
                  >
                    Before
                  </th>
                  <th
                    style={{
                      padding: "2px 6px",
                      fontSize: "var(--font-size-xs)",
                      color: "var(--ink-muted)",
                      textAlign: "left",
                      fontWeight: 500,
                    }}
                  >
                    After
                  </th>
                </tr>
              </thead>
              <tbody>
                {diff.fieldChanges.map((fc, i) => (
                  <FieldChangeRow key={i} change={fc} />
                ))}
              </tbody>
            </table>
          )}
        </div>
      ))}
    </div>
  );
}

// ─── Copy button ─────────────────────────────────────────────────────────────

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(() => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [text]);
  return (
    <button
      className="btn btn--sm"
      type="button"
      onClick={handleCopy}
      title="Copy rollback script"
      style={{ padding: "2px 8px" }}
    >
      {copied ? <Check size={12} /> : <Copy size={12} />}
      {copied ? "Copied" : "Copy"}
    </button>
  );
}

// ─── Section ─────────────────────────────────────────────────────────────────

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div
        style={{
          fontSize: "var(--font-size-xs)",
          fontWeight: 600,
          color: "var(--ink-muted)",
          textTransform: "uppercase",
          letterSpacing: "0.05em",
        }}
      >
        {title}
      </div>
      {children}
    </div>
  );
}

// ─── Main modal ──────────────────────────────────────────────────────────────

export function SafeChangeModal({
  open,
  preview,
  loading,
  error,
  onConfirm,
  onCancel,
}: SafeChangeModalProps) {
  const [typedText, setTypedText] = useState("");
  const [activeTab, setActiveTab] = useState<"diff" | "rollback">("diff");
  const cancelRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset typed text when the modal opens / preview changes.
  useEffect(() => {
    if (open) {
      setTypedText("");
      setActiveTab("diff");
    }
  }, [open, preview]);

  // Focus cancel button on open; Escape closes.
  useEffect(() => {
    if (!open) return;
    cancelRef.current?.focus();
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onCancel]);

  if (!open) return null;

  const confirmAllowed =
    preview &&
    (!preview.requiresTypedConfirmation ||
      typedText.trim() === preview.confirmationText.trim());

  const isDeleteOp =
    preview?.kind === "deleteOne" || preview?.kind === "deleteMany";

  return (
    <div
      className="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="scm-title"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          e.preventDefault();
          onCancel();
        }
      }}
    >
      <div
        className="modal"
        style={{ width: "min(680px, 96vw)", maxHeight: "90vh", display: "flex", flexDirection: "column" }}
      >
        {/* Header */}
        <div className="modal__header">
          <div className="modal__heading">
            <h2 className="modal__title" id="scm-title" style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <AlertTriangle size={16} style={{ color: "var(--warning-500)", flexShrink: 0 }} />
              Safe Change Preview
              {preview?.isProduction && (
                <span
                  style={{
                    fontSize: "var(--font-size-xs)",
                    fontWeight: 600,
                    padding: "2px 6px",
                    borderRadius: "var(--radius)",
                    background: "color-mix(in oklch, var(--danger-500), transparent 80%)",
                    color: "var(--danger-500)",
                    border: "1px solid color-mix(in oklch, var(--danger-500), transparent 60%)",
                  }}
                >
                  PRODUCTION
                </span>
              )}
            </h2>
          </div>
          <button
            className="modal__close"
            type="button"
            onClick={onCancel}
            aria-label="Cancel and close"
          >
            ×
          </button>
        </div>

        {/* Scrollable body */}
        <div
          className="modal__body"
          style={{ overflowY: "auto", flex: 1, display: "flex", flexDirection: "column", gap: 16 }}
        >
          {/* Loading */}
          {loading && (
            <div style={{ textAlign: "center", padding: "24px 0", color: "var(--ink-muted)", fontSize: "var(--font-size-sm)" }}>
              Analyzing operation…
            </div>
          )}

          {/* Error */}
          {error && !loading && (
            <div
              style={{
                padding: "12px 14px",
                borderRadius: "var(--radius)",
                background: "color-mix(in oklch, var(--danger-500), transparent 88%)",
                color: "var(--danger-500)",
                border: "1px solid color-mix(in oklch, var(--danger-500), transparent 65%)",
                fontSize: "var(--font-size-sm)",
              }}
            >
              <strong>Preview failed:</strong> {error}
            </div>
          )}

          {/* Preview content */}
          {preview && !loading && (
            <>
              {/* Summary row */}
              <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
                <div style={{ fontSize: "var(--font-size-sm)" }}>
                  <strong>{preview.matchedCount}</strong>{" "}
                  <span style={{ color: "var(--ink-muted)" }}>
                    document{preview.matchedCount !== 1 ? "s" : ""} matched
                  </span>
                </div>
                <RiskBadge score={preview.riskScore} />
                {!preview.indexInfo.indexUsed && (
                  <span style={{ fontSize: "var(--font-size-xs)", color: "var(--warning-500)" }}>
                    ⚠ No index used ({preview.indexInfo.stage})
                  </span>
                )}
              </div>

              {/* Warnings */}
              {preview.warnings.length > 0 && (
                <Section title="Warnings">
                  <ul
                    style={{
                      margin: 0,
                      padding: "0 0 0 16px",
                      fontSize: "var(--font-size-xs)",
                      color: "var(--warning-500)",
                      display: "flex",
                      flexDirection: "column",
                      gap: 2,
                    }}
                  >
                    {preview.warnings.map((w, i) => <li key={i}>{w}</li>)}
                  </ul>
                </Section>
              )}

              {/* Risk reasons */}
              {preview.riskReasons.length > 0 && (
                <Section title="Risk factors">
                  <ul
                    style={{
                      margin: 0,
                      padding: "0 0 0 16px",
                      fontSize: "var(--font-size-xs)",
                      color: "var(--ink-muted)",
                      display: "flex",
                      flexDirection: "column",
                      gap: 2,
                    }}
                  >
                    {preview.riskReasons.map((r, i) => <li key={i}>{r}</li>)}
                  </ul>
                </Section>
              )}

              {/* Tab switcher: Diff | Rollback */}
              <div>
                <div style={{ display: "flex", gap: 2, marginBottom: 8 }}>
                  {(["diff", "rollback"] as const).map((tab) => (
                    <button
                      key={tab}
                      type="button"
                      className={`btn btn--sm ${activeTab === tab ? "btn--active" : ""}`}
                      style={{
                        opacity: activeTab === tab ? 1 : 0.65,
                        fontWeight: activeTab === tab ? 600 : 400,
                      }}
                      onClick={() => setActiveTab(tab)}
                    >
                      {tab === "diff" ? "Field changes" : "Rollback plan"}
                    </button>
                  ))}
                </div>

                {activeTab === "diff" && (
                  <>
                    {preview.matchedCount === 0 ? (
                      <div
                        style={{
                          padding: "10px 12px",
                          borderRadius: "var(--radius)",
                          background: "var(--surface-2)",
                          fontSize: "var(--font-size-sm)",
                          color: "var(--ink-muted)",
                        }}
                      >
                        No documents matched — the operation would be a no-op.
                      </div>
                    ) : isDeleteOp ? (
                      <Section title={`Sample documents that will be deleted (${preview.sampleBefore.length} shown)`}>
                        <div
                          style={{
                            borderRadius: "var(--radius)",
                            border: "1px solid var(--border)",
                            overflow: "hidden",
                          }}
                        >
                          {preview.sampleBefore.map((doc, i) => (
                            <pre
                              key={i}
                              style={{
                                margin: 0,
                                padding: "8px 10px",
                                fontSize: "11px",
                                fontFamily: "var(--font-mono)",
                                overflowX: "auto",
                                borderBottom:
                                  i < preview.sampleBefore.length - 1
                                    ? "1px solid var(--border)"
                                    : "none",
                                background: "color-mix(in oklch, var(--danger-500), transparent 93%)",
                                color: "var(--ink)",
                              }}
                            >
                              {doc}
                            </pre>
                          ))}
                        </div>
                      </Section>
                    ) : (
                      <Section title="Field-level changes (sample)">
                        <div
                          style={{
                            borderRadius: "var(--radius)",
                            border: "1px solid var(--border)",
                            overflow: "hidden",
                          }}
                        >
                          <DiffTable diffs={preview.diffs} />
                        </div>
                      </Section>
                    )}
                  </>
                )}

                {activeTab === "rollback" && (
                  <Section title="Rollback plan">
                    <div
                      style={{
                        position: "relative",
                        borderRadius: "var(--radius)",
                        border: "1px solid var(--border)",
                        overflow: "hidden",
                      }}
                    >
                      <div
                        style={{
                          position: "absolute",
                          top: 6,
                          right: 8,
                          zIndex: 1,
                        }}
                      >
                        <CopyButton text={preview.rollbackScript} />
                      </div>
                      <pre
                        style={{
                          margin: 0,
                          padding: "10px 10px 10px 10px",
                          paddingRight: 80,
                          fontSize: "11px",
                          fontFamily: "var(--font-mono)",
                          overflowX: "auto",
                          maxHeight: 200,
                          overflowY: "auto",
                          background: "var(--surface-2)",
                          color: "var(--ink)",
                        }}
                      >
                        {preview.rollbackScript || "(no rollback plan generated)"}
                      </pre>
                    </div>
                    <div style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>
                      Rollback level:{" "}
                      <strong>
                        {preview.rollbackLevel === "full"
                          ? "Full pre-images captured"
                          : preview.rollbackLevel === "sampleBased"
                          ? `Sample-based (${preview.matchedCount} docs exceed capture limit)`
                          : "Metadata only — no pre-images captured"}
                      </strong>
                    </div>
                  </Section>
                )}
              </div>

              {/* Typed confirmation */}
              {preview.requiresTypedConfirmation && (
                <Section title="Typed confirmation required">
                  <div style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>
                    Type the following to confirm:
                  </div>
                  <code
                    style={{
                      display: "block",
                      padding: "6px 10px",
                      borderRadius: "var(--radius)",
                      background: "var(--surface-2)",
                      fontSize: "var(--font-size-xs)",
                      fontFamily: "var(--font-mono)",
                      color: "var(--ink)",
                      userSelect: "all",
                    }}
                  >
                    {preview.confirmationText}
                  </code>
                  <input
                    ref={inputRef}
                    type="text"
                    className="input input--sm"
                    value={typedText}
                    onChange={(e) => setTypedText(e.target.value)}
                    placeholder={preview.confirmationText}
                    style={{ fontFamily: "var(--font-mono)" }}
                    aria-label="Type the confirmation phrase"
                    autoComplete="off"
                    spellCheck={false}
                  />
                </Section>
              )}
            </>
          )}
        </div>

        {/* Footer */}
        <div className="modal__footer">
          <button
            ref={cancelRef}
            className="btn btn--sm"
            type="button"
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            className={`btn btn--sm ${isDeleteOp ? "btn--danger-filled" : "btn--primary"}`}
            type="button"
            disabled={!confirmAllowed || loading}
            onClick={onConfirm}
            title={
              preview?.requiresTypedConfirmation && !confirmAllowed
                ? "Type the confirmation phrase first"
                : undefined
            }
          >
            {isDeleteOp ? "Delete" : "Apply changes"}
          </button>
        </div>
      </div>
    </div>
  );
}
