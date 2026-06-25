import { useState, useEffect } from "react";
import commands, {
  type AuditMode,
  type AuditModeConfig,
  formatError,
} from "../ipc/commands";
import { Card, Badge, Button, Spinner, Alert, injectAuditKeyframes } from "./AuditUi";

/**
 * The mode chooser — always shown when the user opens the Audit tab.
 *
 * Two big cards: Dev Mode (full stack locally via Docker) and Production
 * Mode (in-app pipeline with the user's own keys on testnet or mainnet).
 * The user picks each time they enter the tab. The last choice is
 * remembered and pre-highlighted, but the chooser is never skipped.
 */
export function AuditModeChooser({
  onChoose,
}: {
  onChoose: (mode: AuditMode) => void;
}) {
  const [config, setConfig] = useState<AuditModeConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selecting, setSelecting] = useState<AuditMode | null>(null);

  useEffect(() => {
    injectAuditKeyframes();
    commands
      .auditGetModeConfig()
      .then((c) => setConfig(c))
      .catch((e) => setError(formatError(e)))
      .finally(() => setLoading(false));
  }, []);

  const choose = async (mode: AuditMode) => {
    setSelecting(mode);
    setError(null);
    try {
      await commands.auditSetAuditMode(mode);
      onChoose(mode);
    } catch (e) {
      setError(formatError(e));
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
        padding: "var(--space-5)",
        maxWidth: "760px",
        margin: "0 auto",
        animation: "audit-fade-in 0.25s ease",
      }}
    >
      <div style={{ textAlign: "center", marginBottom: "var(--space-2)" }}>
        <h1
          style={{
            fontSize: "var(--font-size-2xl)",
            fontWeight: 700,
            margin: 0,
            color: "var(--ink)",
            letterSpacing: "-0.01em",
          }}
        >
          ZK Audit Log
        </h1>
        <p
          style={{
            fontSize: "var(--font-size-sm)",
            color: "var(--ink-muted)",
            margin: "var(--space-2) 0 0",
            lineHeight: "var(--line-height-normal)",
          }}
        >
          Tamper-evident Merkle audit log anchored to Stellar, with IPFS publishing
          and zero-knowledge inclusion proofs.
        </p>
        <p
          style={{
            fontSize: "var(--font-size-xs)",
            color: "var(--ink-faint)",
            margin: "var(--space-2) 0 0",
          }}
        >
          Choose how you want to run the audit system.
        </p>
      </div>

      {error && <Alert tone="danger">{error}</Alert>}

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: "var(--space-4)",
        }}
      >
        {/* ─── Dev Mode card ─────────────────────────────────────────── */}
        <ModeCard
          selected={lastMode === "dev"}
          loading={selecting === "dev"}
          onClick={() => choose("dev")}
          tone="accent"
          icon="🐳"
          title="Dev Mode"
          tagline="Full stack, locally"
          description="Run the complete audit system on your machine — publisher, independent attester, and reader daemons — via Docker Compose. K-of-N attestation, oplog completeness verification, and on-chain commitments to Stellar testnet."
          bullets={[
            "Full production-grade stack in containers",
            "K-of-N attestation + oplog verification",
            "Stellar testnet (auto-configured)",
            "Requires Docker + the replica set running",
          ]}
          footerBadge={<Badge tone="accent" dot>Testnet</Badge>}
        />

        {/* ─── Production Mode card ──────────────────────────────────── */}
        <ModeCard
          selected={lastMode === "production"}
          loading={selecting === "production"}
          onClick={() => choose("production")}
          tone="success"
          icon="🛰️"
          title="Production Mode"
          tagline="Your keys, your network"
          description="Run the in-app audit pipeline with your own Stellar keypair and contract. Choose testnet or mainnet — a double-check that an audit system you deployed elsewhere works end to end."
          bullets={[
            "In-app pipeline, no daemon",
            "Import your own Stellar secret key",
            "Testnet or Mainnet (your choice)",
            "Verify a remote deployment matches",
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
  tone,
  icon,
  title,
  tagline,
  description,
  bullets,
  footerBadge,
}: {
  selected: boolean;
  loading: boolean;
  onClick: () => void;
  tone: "accent" | "success";
  icon: string;
  title: string;
  tagline: string;
  description: string;
  bullets: string[];
  footerBadge: React.ReactNode;
}) {
  const accent = tone === "accent" ? "var(--accent-500)" : "var(--success-500)";
  return (
    <Card
      padded={false}
      style={{
        cursor: loading ? "wait" : "pointer",
        borderColor: selected ? accent : "var(--border)",
        boxShadow: selected ? `0 0 0 1px ${accent}` : "none",
        transition: "border-color 0.15s ease, box-shadow 0.15s ease, transform 0.05s ease",
        overflow: "hidden",
      }}
    >
      <button
        onClick={onClick}
        disabled={loading}
        style={{
          width: "100%",
          background: "transparent",
          border: "none",
          textAlign: "left",
          padding: "var(--space-4)",
          cursor: loading ? "wait" : "pointer",
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-3)",
        }}
      >
        <div style={{ display: "flex", alignItems: "flex-start", gap: "var(--space-3)" }}>
          <div
            style={{
              fontSize: "1.6rem",
              lineHeight: 1,
              width: "44px",
              height: "44px",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              borderRadius: "var(--radius-md)",
              background: "var(--surface-2)",
              flexShrink: 0,
            }}
          >
            {icon}
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
              {selected && <Badge tone={tone}>Last used</Badge>}
            </div>
            <div style={{ fontSize: "var(--font-size-xs)", color: accent, fontWeight: 600, marginTop: "2px" }}>
              {tagline}
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
              <span style={{ color: accent, fontWeight: 700, lineHeight: "var(--line-height-tight)" }}>›</span>
              <span>{b}</span>
            </li>
          ))}
        </ul>

        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginTop: "var(--space-1)" }}>
          {footerBadge}
          {loading ? <Spinner size={14} /> : <Button variant={selected ? "primary" : "secondary"}>Select</Button>}
        </div>
      </button>
    </Card>
  );
}
