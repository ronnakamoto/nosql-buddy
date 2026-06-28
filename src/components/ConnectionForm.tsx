import { useState } from "react";
import commands, { type SaveProfileRequest, type TestResult } from "../ipc/commands";
import { Modal } from "./Modal";
import { InfoPopover } from "./InfoPopover";
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
    initial?.authMechanism ?? "scram-sha-256",
  );
  const [secret, setSecret] = useState("");
  const [group, setGroup] = useState(initial?.group ?? "");
  const [notes, setNotes] = useState(initial?.notes ?? "");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);
  const [saving, setSaving] = useState(false);

  async function handleTest() {
    setTestResult(null);
    setTesting(true);
    try {
      const result = await commands.testProfile({
        id: initial?.id,
        name: name || "test",
        uri,
        authMechanism,
        secret: secret || undefined,
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
        secret: secret || undefined,
        group: group || null,
        notes: notes || null,
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
          <button className="btn" onClick={handleTest} disabled={testing || saving}>
            {testing ? "Testing…" : "Test connection"}
          </button>
          <button className="btn" onClick={onClose} disabled={saving}>
            Cancel
          </button>
          <button className="btn btn--primary" onClick={handleSave} disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </button>
        </>
      }
    >
      {testResult && (
        <div
          role="status"
          className="toast"
          style={{
            position: "static",
            margin: 0,
            marginBottom: 12,
            background: testResult.ok ? "var(--success-500)" : "var(--danger-500)",
            color: "oklch(0.99 0.002 240)",
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
          Credentials in the URI are accepted, but the password is also stored
          in the OS keychain and never sent back to the UI after save.
        </div>
      </div>
      <div className="field">
        <label className="field__label" htmlFor="conn-auth">
          Authentication <InfoPopover label="What is authentication?" title="Authentication mechanism">
            <p>Choose how MongoDB validates your identity.</p>
            <ul>
              <li><strong>SCRAM-SHA-256</strong>: modern default for username and password.</li>
              <li><strong>x.509</strong>: certificate-based authentication.</li>
              <li><strong>LDAP</strong>: enterprise directory integration.</li>
              <li><strong>Kerberos</strong>: Active Directory integration.</li>
              <li><strong>AWS IAM</strong>: MongoDB Atlas IAM roles.</li>
            </ul>
          </InfoPopover>
        </label>
        <select
          id="conn-auth"
          className="field__select"
          value={authMechanism}
          onChange={(e) => setAuthMechanism(e.target.value as SaveProfileRequest["authMechanism"])}
        >
          <option value="none">No authentication</option>
          <option value="scram-sha-1">SCRAM-SHA-1</option>
          <option value="scram-sha-256">SCRAM-SHA-256</option>
          <option value="x509">x.509 certificate</option>
          <option value="ldap">LDAP</option>
          <option value="kerberos">Kerberos</option>
          <option value="aws-iam">AWS IAM</option>
        </select>
      </div>
      {authMechanism !== "none" && (
        <div className="field">
          <label className="field__label" htmlFor="conn-secret">
            Password / secret (stored in OS keychain)
          </label>
          <input
            id="conn-secret"
            className="field__input"
            type="password"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
            placeholder="Stored once. Cleared after save."
            autoComplete="off"
          />
        </div>
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
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return "Unexpected error";
}
