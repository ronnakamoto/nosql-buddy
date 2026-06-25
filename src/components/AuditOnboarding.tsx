import { useState, useCallback, type ReactNode, type CSSProperties } from "react";
import commands, { type OnboardingStatus } from "../ipc/commands";
import { formatError } from "../ipc/commands";

/**
 * Audit onboarding flow — the "Start Audit Trial" experience.
 *
 * Shows an empty state with a single call-to-action button. When clicked,
 * walks the user through:
 * 1. Pinata API key entry (if not already saved)
 * 2. Stellar testnet account generation + funding
 * 3. Progress animation
 * 4. Transitions to the live audit view on success
 */

type SetupStep = "idle" | "pinata" | "funding" | "done" | "error";

export function AuditOnboarding({
  onComplete,
}: {
  onComplete: () => void;
}) {
  const [step, setStep] = useState<SetupStep>("idle");
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<string[]>([]);

  // Pinata key form state
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [testingPinata, setTestingPinata] = useState(false);

  const addProgress = useCallback((msg: string) => {
    setProgress((prev) => [...prev, msg]);
  }, []);

  const startOnboarding = useCallback(async () => {
    setError(null);
    setProgress([]);
    setStep("pinata");
    addProgress("Checking existing setup...");

    try {
      // Check what's already provisioned.
      const status: OnboardingStatus = await commands.auditCheckOnboarding();
      let pinataReady = status.hasPinata;
      let keypairReady = status.hasKeypair;

      if (pinataReady) {
        addProgress("✓ Pinata IPFS storage already configured");
      }
      // If Pinata is not configured, we need the user to enter their key.
      // The Pinata form is shown below — the user fills it in and clicks
      // "Save & Continue", which calls savePinataAndContinue().

      if (!pinataReady) {
        // Wait for the user to enter Pinata credentials.
        // The form is rendered below when step === "pinata" && !pinataReady.
        return;
      }

      // Pinata is ready — proceed to fund the Stellar account.
      await fundAndComplete(keypairReady);
    } catch (e) {
      setStep("error");
      setError(formatError(e));
    }
  }, [addProgress]);

  const savePinataAndContinue = useCallback(async () => {
    if (!apiKey.trim() || !apiSecret.trim()) {
      setError("Please enter both API key and API secret");
      return;
    }

    setTestingPinata(true);
    setError(null);

    try {
      // Test + save in one call (the backend tests before saving).
      await commands.auditSavePinataConfig(apiKey.trim(), apiSecret.trim());
      addProgress("✓ Pinata IPFS storage connected");

      // Now proceed to fund the Stellar account.
      const status = await commands.auditCheckOnboarding();
      await fundAndComplete(status.hasKeypair);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setTestingPinata(false);
    }
  }, [apiKey, apiSecret, addProgress]);

  const fundAndComplete = useCallback(
    async (keypairReady: boolean) => {
      setStep("funding");

      if (keypairReady) {
        addProgress("✓ Stellar testnet account already exists");
      } else {
        addProgress("Generating Stellar testnet account...");
        const accountId = await commands.auditGenerateStellarAccount();
        addProgress(`✓ Account funded: ${accountId.slice(0, 8)}...${accountId.slice(-4)}`);
      }

      addProgress("✓ Setup complete — starting audit view");
      setStep("done");

      // Brief delay so the user sees the success state.
      setTimeout(() => onComplete(), 800);
    },
    [addProgress, onComplete],
  );

  // ─── Render ──────────────────────────────────────────────────────

  if (step === "idle") {
    return <EmptyState onStart={startOnboarding} />;
  }

  if (step === "pinata" && !error) {
    // Check if Pinata is already configured — if so, we're waiting for funding.
    // If not, show the Pinata key form.
    const pinataAlreadyConfigured = progress.some((p) =>
      p.includes("Pinata IPFS storage already configured"),
    );

    if (!pinataAlreadyConfigured) {
      return (
        <PinataForm
          apiKey={apiKey}
          apiSecret={apiSecret}
          onApiKeyChange={setApiKey}
          onApiSecretChange={setApiSecret}
          onSave={savePinataAndContinue}
          testing={testingPinata}
          error={error}
          progress={progress}
        />
      );
    }
    // If Pinata is already configured, fall through to the progress view.
  }

  return (
    <ProgressView step={step} progress={progress} error={error} onRetry={startOnboarding} />
  );
}

// ─── Sub-components ───────────────────────────────────────────────────

function EmptyState({ onStart }: { onStart: () => void }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        padding: "48px 24px",
        textAlign: "center",
        gap: "16px",
      }}
    >
      <div style={{ fontSize: "32px" }}>🔒</div>
      <div
        style={{
          fontSize: "15px",
          fontWeight: 600,
          fontFamily: "var(--font-sans)",
          color: "var(--ink)",
          maxWidth: "360px",
        }}
      >
        Track every database write cryptographically
      </div>
      <div
        style={{
          fontSize: "12px",
          color: "var(--ink-faint)",
          maxWidth: "340px",
          lineHeight: 1.5,
        }}
      >
        Every insert, update, and delete is recorded in a tamper-evident log,
        pinned to IPFS, and anchored on the Stellar blockchain.
      </div>
      <button
        onClick={onStart}
        style={{
          padding: "8px 24px",
          fontSize: "13px",
          fontFamily: "var(--font-sans)",
          fontWeight: 600,
          cursor: "pointer",
          background: "var(--accent-500)",
          color: "#fff",
          border: "none",
          borderRadius: "var(--radius-sm)",
        }}
      >
        Start Audit Trial
      </button>
      <div
        style={{
          fontSize: "11px",
          color: "var(--ink-faint)",
        }}
      >
        Dev Mode · Stellar Testnet · Real IPFS via Pinata
      </div>
    </div>
  );
}

function PinataForm({
  apiKey,
  apiSecret,
  onApiKeyChange,
  onApiSecretChange,
  onSave,
  testing,
  error,
  progress,
}: {
  apiKey: string;
  apiSecret: string;
  onApiKeyChange: (v: string) => void;
  onApiSecretChange: (v: string) => void;
  onSave: () => void;
  testing: boolean;
  error: string | null;
  progress: string[];
}) {
  return (
    <div
      style={{
        padding: "24px",
        display: "flex",
        flexDirection: "column",
        gap: "16px",
        maxWidth: "420px",
        margin: "0 auto",
      }}
    >
      <ProgressList progress={progress} />

      <div
        style={{
          fontSize: "13px",
          fontWeight: 600,
          fontFamily: "var(--font-sans)",
          color: "var(--ink)",
        }}
      >
        Configure Pinata IPFS Storage
      </div>
      <div style={{ fontSize: "11px", color: "var(--ink-faint)", lineHeight: 1.5 }}>
        Batches are published to IPFS via Pinata. Create a free account at{" "}
        <span style={{ color: "var(--accent-500)" }}>pinata.cloud</span>, then paste
        your API key and secret below. This is used for both dev mode and production.
      </div>

      <FormField label="API Key">
        <input
          type="text"
          value={apiKey}
          onChange={(e) => onApiKeyChange(e.target.value)}
          placeholder="Your Pinata API key"
          style={inputStyle}
          disabled={testing}
        />
      </FormField>

      <FormField label="API Secret">
        <input
          type="password"
          value={apiSecret}
          onChange={(e) => onApiSecretChange(e.target.value)}
          placeholder="Your Pinata API secret"
          style={inputStyle}
          disabled={testing}
        />
      </FormField>

      {error && <ErrorBanner message={error} />}

      <button
        onClick={onSave}
        disabled={testing || !apiKey.trim() || !apiSecret.trim()}
        style={{
          padding: "6px 16px",
          fontSize: "12px",
          fontFamily: "var(--font-sans)",
          fontWeight: 600,
          cursor: testing ? "wait" : "pointer",
          background: "var(--accent-500)",
          color: "#fff",
          border: "none",
          borderRadius: "var(--radius-sm)",
          opacity: testing || !apiKey.trim() || !apiSecret.trim() ? 0.55 : 1,
        }}
      >
        {testing ? "Testing..." : "Save & Continue"}
      </button>
    </div>
  );
}

function ProgressView({
  step,
  progress,
  error,
  onRetry,
}: {
  step: SetupStep;
  progress: string[];
  error: string | null;
  onRetry: () => void;
}) {
  return (
    <div
      style={{
        padding: "32px 24px",
        display: "flex",
        flexDirection: "column",
        gap: "16px",
        maxWidth: "420px",
        margin: "0 auto",
      }}
    >
      <div
        style={{
          fontSize: "13px",
          fontWeight: 600,
          fontFamily: "var(--font-sans)",
          color: "var(--ink)",
        }}
      >
        {step === "done"
          ? "Audit environment ready!"
          : step === "error"
            ? "Setup failed"
            : "Setting up your audit environment..."}
      </div>

      <ProgressList progress={progress} />

      {error && <ErrorBanner message={error} />}

      {step === "error" && (
        <button
          onClick={onRetry}
          style={{
            padding: "6px 16px",
            fontSize: "12px",
            fontFamily: "var(--font-sans)",
            fontWeight: 600,
            cursor: "pointer",
            background: "var(--accent-500)",
            color: "#fff",
            border: "none",
            borderRadius: "var(--radius-sm)",
          }}
        >
          Try Again
        </button>
      )}
    </div>
  );
}

function ProgressList({ progress }: { progress: string[] }) {
  if (progress.length === 0) return null;
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "4px",
        padding: "12px",
        background: "var(--surface-2)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-md)",
      }}
    >
      {progress.map((msg, i) => (
        <div
          key={i}
          style={{
            fontSize: "11px",
            fontFamily: "var(--font-mono)",
            color: msg.startsWith("✓")
              ? "var(--ink)"
              : "var(--ink-faint)",
          }}
        >
          {msg}
        </div>
      ))}
    </div>
  );
}

function FormField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
      <label
        style={{
          fontSize: "11px",
          color: "var(--ink-faint)",
          fontFamily: "var(--font-sans)",
        }}
      >
        {label}
      </label>
      {children}
    </div>
  );
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <div
      style={{
        padding: "8px 12px",
        fontSize: "11px",
        color: "var(--danger-500, #c00)",
        background: "var(--surface-2)",
        border: "1px solid var(--danger-500, #c00)",
        borderRadius: "var(--radius-sm)",
        fontFamily: "var(--font-mono)",
      }}
    >
      {message}
    </div>
  );
}

const inputStyle: CSSProperties = {
  padding: "6px 10px",
  fontSize: "12px",
  fontFamily: "var(--font-mono)",
  background: "var(--surface-1)",
  color: "var(--ink)",
  border: "1px solid var(--border-strong)",
  borderRadius: "var(--radius-sm)",
  outline: "none",
};
