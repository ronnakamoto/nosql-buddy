import { useState, useEffect } from "react";
import commands, {
  type AuditMode,
  type AuditModeConfig,
  formatError,
} from "../ipc/commands";
import { Card, Badge, Button, Spinner, injectAuditKeyframes } from "./AuditUi";
import { useToast } from "../context/ToastContext";
import { FlaskConical, CheckCircle, ChevronRight, ShieldCheck } from "lucide-react";

/**
 * The mode chooser — the landing page for the Audit tab.
 *
 * Explains what the audit log is, shows two clear paths, and gives
 * the user enough context to choose confidently.
 */
export function AuditModeChooser({
  onChoose,
}: {
  onChoose: (mode: AuditMode) => void;
}) {
  const [config, setConfig] = useState<AuditModeConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [selecting, setSelecting] = useState<AuditMode | null>(null);
  const toast = useToast();

  useEffect(() => {
    injectAuditKeyframes();
    commands
      .auditGetModeConfig()
      .then((c) => setConfig(c))
      .catch((e) => toast.push(formatError(e), "error"))
      .finally(() => setLoading(false));
  }, []);

  const choose = async (mode: AuditMode) => {
    setSelecting(mode);
    try {
      await commands.auditSetAuditMode(mode);
      onChoose(mode);
    } catch (e) {
      toast.push(formatError(e), "error");
      setSelecting(null);
    }
  };

  if (loading) {
    return (
      <div style={{ display: "flex", justifyContent: "center", padding: "var(--space-8)" }}>
        <Spinner size={22} />
      </div>
    );
  }

  const lastMode = config?.mode ?? "dev";

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-4)",
        padding: "var(--space-4)",
        flex: 1,
        overflow: "auto",
      }}
    >
      <div>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", marginBottom: "var(--space-2)" }}>
          <span style={{ color: "var(--accent-600)", display: "flex" }}>
            <ShieldCheck size={20} />
          </span>
          <h1
            style={{
              fontSize: "var(--font-size-2xl)",
              fontWeight: 700,
              margin: 0,
              color: "var(--ink)",
              letterSpacing: "-0.02em",
            }}
          >
            Audit Log
          </h1>
        </div>
        <p
          style={{
            fontSize: "var(--font-size-sm)",
            color: "var(--ink-muted)",
            margin: 0,
            lineHeight: "var(--line-height-normal)",
          }}
        >
          Every MongoDB insert, update, and delete is recorded in a tamper-evident log that you can cryptographically prove. Seal batches of changes and anchor their fingerprints on the Stellar blockchain so no one can alter history undetected.
        </p>
      </div>

      {/* How it works */}
      <div style={{
        display: "grid",
        gridTemplateColumns: "repeat(3, 1fr)",
        gap: "var(--space-2)",
        padding: "var(--space-3)",
        background: "var(--surface)",
        borderRadius: "var(--radius-lg)",
        border: "1px solid var(--border)",
      }}>
        <HowItWorks step="1" label="Capture" detail="MongoDB changes are recorded automatically into cryptographic batches." />
        <HowItWorks step="2" label="Seal" detail="Seal a batch to create a tamper-evident fingerprint (Merkle root)." />
        <HowItWorks step="3" label="Anchor" detail="Anchor the fingerprint on Stellar so it can be independently verified." />
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: "var(--space-4)",
        }}
      >
        <ModeCard
          selected={lastMode === "dev"}
          loading={selecting === "dev"}
          onClick={() => choose("dev")}
          tone="accent"
          symbol={<FlaskConical size={22} />}
          title="Dev Mode"
          description="Run the entire audit system locally with Docker. Best for trying it out or developing against the audit pipeline."
          bullets={[
            "Full local stack via Docker",
            "Automatic change capture",
            "Multi-party sign-off (K-of-N)",
            "Stellar testnet (no real funds)",
          ]}
          footerBadge={<Badge tone="accent" dot>Recommended for trying</Badge>}
        />

        <ModeCard
          selected={lastMode === "production"}
          loading={selecting === "production"}
          onClick={() => choose("production")}
          disabled
          tone="success"
          symbol={<CheckCircle size={22} />}
          title="Production Mode"
          description="Use the built-in pipeline with your own Stellar keys. Commit to testnet or mainnet with real transactions."
          bullets={[
            "Built-in audit pipeline",
            "Your Stellar keypair (OS keychain)",
            "Testnet or mainnet",
            "No Docker required",
          ]}
          footerBadge={
            config?.hasProductionKeypair ? (
              <Badge tone="success" dot>Keypair saved</Badge>
            ) : (
              <Badge tone="neutral">No keypair yet</Badge>
            )
          }
        />
      </div>

      <div
        style={{
          textAlign: "center",
          fontSize: "var(--font-size-xs)",
          color: "var(--ink-faint)",
        }}
      >
        You can switch modes any time from Audit Settings.
      </div>
    </div>
  );
}

function ModeCard({
  selected,
  loading,
  onClick,
  disabled = false,
  tone,
  symbol,
  title,
  description,
  bullets,
  footerBadge,
}: {
  selected: boolean;
  loading: boolean;
  onClick: () => void;
  disabled?: boolean;
  tone: "accent" | "success";
  symbol: React.ReactNode;
  title: string;
  description: string;
  bullets: string[];
  footerBadge: React.ReactNode;
}) {
  const accent = tone === "accent" ? "var(--accent-500)" : "var(--success-500)";
  const isSelected = selected && !disabled;
  return (
    <Card
      padded={false}
      style={{
        borderColor: isSelected ? accent : "var(--border)",
        boxShadow: isSelected ? `0 0 0 1px ${accent}` : "none",
        opacity: disabled ? 0.6 : 1,
        transition: "border-color 0.15s ease, box-shadow 0.15s ease, transform 0.05s ease",
        overflow: "hidden",
      }}
    >
      <button
        onClick={onClick}
        disabled={loading || disabled}
        style={{
          width: "100%",
          background: "transparent",
          border: "none",
          textAlign: "left",
          padding: "var(--space-4)",
          cursor: disabled ? "not-allowed" : loading ? "wait" : "pointer",
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-3)",
        }}
      >
        <div style={{ display: "flex", alignItems: "flex-start", gap: "var(--space-3)" }}>
          <div
            style={{
              lineHeight: 1,
              width: "44px",
              height: "44px",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              borderRadius: "var(--radius-md)",
              background: isSelected ? accent : "var(--surface-2)",
              color: isSelected ? "white" : accent,
              flexShrink: 0,
              transition: "background 0.15s ease, color 0.15s ease",
            }}
          >
            {symbol}
          </div>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-2)",
              }}
            >
              <span style={{ fontSize: "var(--font-size-lg)", fontWeight: 700, color: "var(--ink)" }}>
                {title}
              </span>
              {disabled ? (
                <Badge tone="neutral">Coming soon</Badge>
              ) : (
                isSelected && <Badge tone={tone}>Last used</Badge>
              )}
            </div>
          </div>
        </div>

        <p
          style={{
            fontSize: "var(--font-size-sm)",
            color: "var(--ink-muted)",
            lineHeight: "var(--line-height-normal)",
            margin: 0,
          }}
        >
          {description}
        </p>

        <ul
          style={{
            margin: 0,
            padding: 0,
            listStyle: "none",
            display: "flex",
            flexDirection: "column",
            gap: "6px",
          }}
        >
          {bullets.map((b) => (
            <li
              key={b}
              style={{
                display: "flex",
                alignItems: "flex-start",
                gap: "var(--space-2)",
                fontSize: "var(--font-size-xs)",
                color: "var(--ink-muted)",
              }}
            >
              <span style={{ color: accent, lineHeight: "var(--line-height-tight)", display: "flex", alignItems: "center" }}><ChevronRight size={11} /></span>
              <span>{b}</span>
            </li>
          ))}
        </ul>

        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginTop: "var(--space-1)" }}>
          {disabled ? (
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              Available in a future release
            </span>
          ) : (
            <>
              {footerBadge}
              {loading ? <Spinner size={14} /> : <Button variant={isSelected ? "primary" : "secondary"}>Select</Button>}
            </>
          )}
        </div>
      </button>
    </Card>
  );
}

function HowItWorks({ step, label, detail }: { step: string; label: string; detail: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
        <span
          style={{
            width: "18px",
            height: "18px",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            borderRadius: 0,
            fontSize: "10px",
            fontWeight: 700,
            background: "var(--accent-100)",
            color: "var(--accent-700)",
            flexShrink: 0,
          }}
        >
          {step}
        </span>
        <span style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, color: "var(--ink)" }}>{label}</span>
      </div>
      <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-muted)", lineHeight: "var(--line-height-tight)" }}>
        {detail}
      </span>
    </div>
  );
}
