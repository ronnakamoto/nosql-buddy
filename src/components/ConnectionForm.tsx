import { useEffect, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import commands, { formatError, type SaveProfileRequest, type TestResult, type TlsConfig } from "../ipc/commands";
import { Modal } from "./Modal";
import { InfoPopover } from "./InfoPopover";
import { ShortcutButton } from "./ShortcutButton";
import { useToast } from "../context/ToastContext";

export interface ConnectionFormProps {
  open: boolean;
  onClose: () => void;
  onSaved: () => void;
  initial?: Partial<SaveProfileRequest>;
}

export function ConnectionForm({ open, onClose, onSaved, initial }: ConnectionFormProps) {
  const toast = useToast();
  const [name, setName] = useState(initial?.name ?? "");
  const [uri, setUri] = useState(
    initial?.uri ?? "mongodb://127.0.0.1:27017/?retryWrites=true",
  );
  const [authMechanism, setAuthMechanism] = useState<SaveProfileRequest["authMechanism"]>(
    initial?.authMechanism ?? "none",
  );
  const [secret, setSecret] = useState("");
  const [group, setGroup] = useState(initial?.group ?? "");
  const [notes, setNotes] = useState(initial?.notes ?? "");
  const [tlsEnabled, setTlsEnabled] = useState(initial?.tls?.enabled ?? false);
  const [tlsCertFile, setTlsCertFile] = useState(initial?.tls?.certKeyFile ?? "");
  const [tlsCaFile, setTlsCaFile] = useState(initial?.tls?.caFile ?? "");
  const [tlsAllowInvalid, setTlsAllowInvalid] = useState(
    initial?.tls?.allowInvalidCertificates ?? false,
  );
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);
  const [saving, setSaving] = useState(false);
  const storesPasswordSecret = (
    ["scram-sha-1", "scram-sha-256", "ldap", "aws-iam"] as const
  ).includes(authMechanism as "scram-sha-1" | "scram-sha-256" | "ldap" | "aws-iam");
  const isX509 = authMechanism === "x509";
  const isLdap = authMechanism === "ldap";
  const isAwsIam = authMechanism === "aws-iam";
  const tlsEffective = tlsEnabled || isX509;
  const tlsSectionVisible = tlsEffective;

  const secretLabel = isAwsIam ? "AWS secret access key" : isLdap ? "LDAP password" : "Password / secret (stored in OS keychain)";
  const secretPlaceholder = isAwsIam ? "AWS secret access key (leave blank to use env/instance credentials)" : "Stored once. Cleared after save.";

  function buildTlsConfig(): TlsConfig | null {
    if (!tlsEffective && !tlsCertFile && !tlsCaFile) return null;
    return {
      enabled: tlsEffective,
      certKeyFile: tlsCertFile || null,
      caFile: tlsCaFile || null,
      allowInvalidCertificates: tlsAllowInvalid || null,
    };
  }

  async function pickFile(setter: (v: string) => void) {
    const chosen = await openFileDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "PEM certificates", extensions: ["pem", "crt", "cert", "key"] }],
    });
    if (typeof chosen === "string") setter(chosen);
  }

  // Keyboard shortcuts for the connection form
  useEffect(() => {
    if (!open) return;
    
    const handleKeyDown = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === "s") {
        e.preventDefault();
        if (!saving) {
          handleSave();
        }
      } else if (mod && e.key === "t") {
        e.preventDefault();
        if (!testing && !saving) {
          handleTest();
        }
      }
    };
    
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, saving, testing]);

  async function handleTest() {
    setTestResult(null);
    setTesting(true);
    try {
      const result = await commands.testProfile({
        id: initial?.id,
        name: name || "test",
        uri,
        authMechanism,
        secret: storesPasswordSecret ? secret || undefined : undefined,
        tls: buildTlsConfig(),
      });
      setTestResult(result);
    } catch (e) {
      toast.push(describeError(e), "error");
    } finally {
      setTesting(false);
    }
  }

  async function handleSave() {
    if (!name.trim()) {
      toast.push("Give the connection a name.", "error");
      return;
    }
    if (!uri.trim()) {
      toast.push("A connection URI is required.", "error");
      return;
    }
    setSaving(true);
    try {
      await commands.saveProfile({
        id: initial?.id,
        name: name.trim(),
        uri: uri.trim(),
        authMechanism,
        secret: storesPasswordSecret ? secret || undefined : "",
        group: group || null,
        notes: notes || null,
        tls: buildTlsConfig(),
      });
      onSaved();
      onClose();
    } catch (e) {
      toast.push(describeError(e), "error");
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={initial?.id ? "Edit connection" : "New connection"}
      width={560}
      footer={
        <>
          <ShortcutButton
            shortcut="CmdOrCtrl+T"
            onClick={handleTest}
            disabled={testing || saving}
            className="btn"
          >
            {testing ? "Testing…" : "Test connection"}
          </ShortcutButton>
          <button className="btn" onClick={onClose} disabled={saving}>
            Cancel
          </button>
          <ShortcutButton
            variant="primary"
            shortcut="CmdOrCtrl+S"
            onClick={handleSave}
            disabled={saving}
            className="btn btn--primary"
          >
            {saving ? "Saving…" : "Save"}
          </ShortcutButton>
        </>
      }
    >
      {testResult && (
        <div
          role="status"
          style={{
            margin: 0,
            marginBottom: 12,
            padding: "8px 12px",
            borderRadius: "var(--radius-sm)",
            background: testResult.ok ? "var(--success-500)" : "var(--danger-500)",
            color: "oklch(0.99 0.002 240)",
            fontSize: "var(--font-size-sm)",
          }}
        >
          {testResult.ok ? "Connection works." : `Failed: ${testResult.message}`}
        </div>
      )}
      <div className="field">
        <label className="field__label" htmlFor="conn-name">
          Name
        </label>
        <input
          id="conn-name"
          className="field__input"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Local dev"
        />
      </div>
      <div className="field">
        <label className="field__label" htmlFor="conn-uri">
          Connection URI <InfoPopover label="What is a connection URI?" title="MongoDB connection URI"><p>A standard MongoDB connection string. Include host, port, credentials, and optional parameters like <code>retryWrites=true</code> or <code>directConnection=true</code>.</p></InfoPopover>
        </label>
        <input
          id="conn-uri"
          className="field__input"
          value={uri}
          onChange={(e) => setUri(e.target.value)}
          placeholder="mongodb://user:password@host:27017/?retryWrites=true"
          spellCheck={false}
          autoComplete="off"
        />
        <div className="field__hint">
          Credentials in the URI are accepted. For password-based authentication,
          put the username in the URI and the password below to keep it in the OS keychain.
        </div>
      </div>
      <div className="field">
        <label className="field__label" htmlFor="conn-auth">
          Authentication <InfoPopover label="What is authentication?" title="Authentication mechanism">
            <p>Choose how MongoDB validates your identity.</p>
            <ul>
              <li><strong>No authentication</strong>: best when the URI already contains credentials (e.g. MongoDB Atlas connection strings). The driver negotiates automatically.</li>
              <li><strong>SCRAM-SHA-256</strong>: modern username/password auth. Use when the password lives in the keychain, not the URI.</li>
              <li><strong>x.509</strong>: certificate-based authentication (requires TLS section below).</li>
              <li><strong>LDAP</strong>: enterprise directory integration.</li>
              <li><strong>AWS IAM</strong>: Atlas clusters with IAM database authentication.</li>
            </ul>
          </InfoPopover>
        </label>
        <select
          id="conn-auth"
          className="field__select"
          value={authMechanism}
          onChange={(e) => setAuthMechanism(e.target.value as SaveProfileRequest["authMechanism"])}
        >
          <option value="none">No authentication (use credentials in URI)</option>
          <option value="scram-sha-1">SCRAM-SHA-1</option>
          <option value="scram-sha-256">SCRAM-SHA-256</option>
          <option value="x509">x.509 certificate</option>
          <option value="ldap">LDAP</option>
          <option value="aws-iam">AWS IAM</option>
        </select>
      </div>
      {authMechanism === "none" && (
        <div className="field__hint">
          For MongoDB Atlas and most hosted providers, paste the full connection
          string (including username and password) and leave this on
          "No authentication". The driver handles credential negotiation automatically.
        </div>
      )}
      {storesPasswordSecret && (
        <div className="field">
          <label className="field__label" htmlFor="conn-secret">
            {secretLabel}
          </label>
          <input
            id="conn-secret"
            className="field__input"
            type="password"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            placeholder={secretPlaceholder}
            autoComplete="off"
          />
          {isAwsIam && (
            <div className="field__hint">
              Put the AWS access key ID in the URI username. Leave this blank to use
              AWS environment variables, shared credentials, or instance metadata.
            </div>
          )}
          {isLdap && (
            <div className="field__hint">
              Put the LDAP username in the URI. Authentication uses the SASL PLAIN
              mechanism against the $external database.
            </div>
          )}
        </div>
      )}
      {isX509 && (
        <div style={{
          marginBottom: 12,
          padding: "8px 12px",
          borderRadius: "var(--radius-sm)",
          background: "var(--accent-100)",
          color: "var(--ink)",
          fontSize: "var(--font-size-sm)",
          lineHeight: 1.5,
        }}>
          x.509 authentication requires a client certificate. Provide one in the TLS section below.
        </div>
      )}
      <div className="field">
        <label className="field__label" htmlFor="conn-tls-enabled">
          TLS / SSL <InfoPopover label="What is TLS?" title="TLS / SSL configuration"><p>Enable TLS to encrypt the connection. Required for x.509 authentication. Provide a client certificate (PEM with cert and private key) and optionally a root CA file to validate the server certificate.</p></InfoPopover>
        </label>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <input
            id="conn-tls-enabled"
            type="checkbox"
            checked={tlsEffective}
            onChange={(e) => setTlsEnabled(e.target.checked)}
            disabled={isX509}
          />
          <span style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>
            Use TLS protocol to connect{isX509 ? " (required for x.509)" : ""}
          </span>
        </div>
      </div>
      {tlsSectionVisible && (
        <>
          <div className="field">
            <label className="field__label" htmlFor="conn-tls-cert">
              Client certificate (PEM)
            </label>
            <div style={{ display: "flex", gap: 8 }}>
              <input
                id="conn-tls-cert"
                className="field__input"
                value={tlsCertFile}
                onChange={(e) => setTlsCertFile(e.target.value)}
                placeholder="/path/to/client.pem"
                spellCheck={false}
                autoComplete="off"
                style={{ flex: 1 }}
              />
              <button
                type="button"
                className="btn"
                onClick={() => pickFile(setTlsCertFile)}
                disabled={saving || testing}
              >
                Browse
              </button>
            </div>
            {isX509 && !tlsCertFile && (
              <div className="field__hint" style={{ color: "var(--warning-500)" }}>
                A client certificate is required for x.509 authentication.
              </div>
            )}
          </div>
          <div className="field">
            <label className="field__label" htmlFor="conn-tls-ca">
              Root CA file (optional)
            </label>
            <div style={{ display: "flex", gap: 8 }}>
              <input
                id="conn-tls-ca"
                className="field__input"
                value={tlsCaFile}
                onChange={(e) => setTlsCaFile(e.target.value)}
                placeholder="/path/to/ca.pem"
                spellCheck={false}
                autoComplete="off"
                style={{ flex: 1 }}
              />
              <button
                type="button"
                className="btn"
                onClick={() => pickFile(setTlsCaFile)}
                disabled={saving || testing}
              >
                Browse
              </button>
            </div>
          </div>
          <div className="field">
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <input
                id="conn-tls-invalid"
                type="checkbox"
                checked={tlsAllowInvalid}
                onChange={(e) => setTlsAllowInvalid(e.target.checked)}
              />
              <label htmlFor="conn-tls-invalid" style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>
                Accept invalid server certificates (insecure, testing only)
              </label>
            </div>
          </div>
        </>
      )}
      <div className="field">
        <label className="field__label" htmlFor="conn-group">
          Group <InfoPopover label="What is a connection group?" title="Connection group"><p>Organize related connections (e.g. Production, Staging, Local). Groups appear as collapsible sections in the connection switcher.</p></InfoPopover>
        </label>
        <input
          id="conn-group"
          className="field__input"
          value={group}
          onChange={(e) => setGroup(e.target.value)}
          placeholder="Production / Staging / Local"
        />
      </div>
      <div className="field">
        <label className="field__label" htmlFor="conn-notes">
          Notes
        </label>
        <textarea
          id="conn-notes"
          className="field__textarea"
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          placeholder="Anything worth remembering about this connection."
        />
      </div>
    </Modal>
  );
}

function describeError(e: unknown): string {
  return formatError(e);
}
