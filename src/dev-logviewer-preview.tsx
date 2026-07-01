import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/ibm-plex-sans";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "./styles.css";
import "./components/audit.css";
import { LogViewer } from "./components/LogViewer";
import { Card, CardHeader, Alert, Modal } from "./components/AuditUi";

const SETUP_LINES = [
  "Funding publisher account",
  "GD4Y2U77ZL2AZJTQMAVOPPCKUDM2ZQMBVSEZDTJ2ILFTSY4MFD6VXGVJ... OK",
  "Waiting for publisher account to be visible on Horizon... OK",
  "Funding attester account",
  "GBF2UZCEPU7JEGAN7CTXCYZFDN3XNQMRK32V7ZWKHT5EB6ZUBBIOYPRX... OK",
  "Waiting for attester account to be visible on Horizon... OK",
  "Funding confirmed; waiting 3s for RPC propagation...",
  "Building WASM...",
  "Deploying contract via stellar CLI...",
  "Using prebuilt contract WASM: /opt/contract/zk_audit_commitment.wasm",
  "Deploying to testnet...",
  "(This requires the stellar CLI installed:",
  "https://docs.stellar.org/tools/developer-tools/cli/install)",
  "WARNING: falling back to default network passphrase",
  "Contract deployed: CA...XYZ",
  "Authorizing attester on-chain... OK",
];

const DOCKER_LOGS = [
  "publisher_1  | [2026-07-02T10:00:01Z] INFO  starting publisher on :9173",
  "publisher_1  | [2026-07-02T10:00:01Z] INFO  connected to mongodb://127.0.0.1:27020",
  "attester_1   | [2026-07-02T10:00:02Z] INFO  attester listening on :9174",
  "reader_1     | [2026-07-02T10:00:02Z] INFO  reader ready on :9175",
  "publisher_1  | [2026-07-02T10:00:05Z] WARN  slow oplog tail (120ms)",
  "publisher_1  | [2026-07-02T10:00:07Z] ERROR failed to reach attester: connection refused",
  "publisher_1  | [2026-07-02T10:00:08Z] INFO  retrying attester connection",
  "attester_1   | [2026-07-02T10:00:09Z] INFO  epoch 4 committed, root=9f21ac...",
  "reader_1     | [2026-07-02T10:00:10Z] INFO  serving proof for leaf 128",
].join("\n");

function LiveDemo() {
  const [lines, setLines] = useState<string[]>([]);
  const [busy, setBusy] = useState(true);

  useEffect(() => {
    let i = 0;
    const id = setInterval(() => {
      if (i >= SETUP_LINES.length) {
        clearInterval(id);
        setBusy(false);
        return;
      }
      setLines((prev) => [...prev, SETUP_LINES[i]]);
      i++;
    }, 350);
    return () => clearInterval(id);
  }, []);

  return (
    <Card>
      <CardHeader title="Live setup progress (streaming)" subtitle="Auto-follows the tail; scroll up to pause." />
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)", marginTop: "var(--space-3)" }}>
        {busy ? (
          <LogViewer
            lines={lines}
            loading={lines.length === 0}
            loadingLabel="Waiting for the setup wizard to start…"
            live
            showLineNumbers={false}
            minHeight={120}
            maxHeight={240}
          />
        ) : (
          <>
            <Alert tone="success">Setup complete. Credentials were written locally. You can now start the stack.</Alert>
            <LogViewer lines={lines.join("\n")} copyable showLineNumbers={false} maxHeight={320} />
          </>
        )}
      </div>
    </Card>
  );
}

function StaticDemo() {
  const [open, setOpen] = useState(true);
  return (
    <Modal open={open} onClose={() => setOpen(false)} title="Dev Stack Logs" subtitle="most recent 120 lines" maxWidth={780}>
      <LogViewer lines={DOCKER_LOGS} searchable copyable loadingLabel="Fetching logs…" maxHeight="48vh" minHeight={200} />
    </Modal>
  );
}

function App() {
  return (
    <div style={{ padding: 32, display: "flex", flexDirection: "column", gap: 24, maxWidth: 640 }}>
      <LiveDemo />
      <StaticDemo />
    </div>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing #root");
createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
