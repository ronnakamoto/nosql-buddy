import { useCallback, useEffect, useMemo, useState } from "react";
import commands, {
  type AppInfo,
  type AppSettings,
  type CollectionSummary,
  type ConnectionHandle,
  type DatabaseSummary,
  type DocumentPage,
  type ProfileSummary,
} from "./ipc/commands";
import { onMenuAction } from "./ipc/events";
import { ConnectionForm } from "./components/ConnectionForm";
import { CommandPalette, type CommandPaletteItem } from "./components/CommandPalette";
import { QueryTab } from "./features/QueryTab";
import { IndexTab } from "./features/IndexTab";
import { SchemaTab } from "./features/SchemaTab";
import { ShellTab } from "./features/ShellTab";
import { ToastStack, useToasts } from "./components/Toast";
import AuditPanel from "./components/AuditPanel";
import ErrorBoundary from "./components/ErrorBoundary";

type Tab =
  | { id: string; kind: "query"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "indexes"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "schema"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "shell"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "audit" };

interface ActiveConnection {
  handle: ConnectionHandle;
  /** Profile metadata used by the Driver Code panel to embed the
   *  user's real URI and a profile/auth comment. Stored alongside
   *  the handle so it survives tab close + reopen. */
  profile: ProfileSummary;
  databases: DatabaseSummary[];
  collections: Record<string, CollectionSummary[]>;
}

export default function App() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [profiles, setProfiles] = useState<ProfileSummary[]>([]);
  const [active, setActive] = useState<ActiveConnection | null>(null);
  const [treeFilter, setTreeFilter] = useState("");
  const [connectionFormOpen, setConnectionFormOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [queryTime, setQueryTime] = useState<number | null>(null);
  const [docCount, setDocCount] = useState<number | null>(null);
  const toasts = useToasts();

  // Initial load
  useEffect(() => {
    (async () => {
      try {
        const [s, i, ps] = await Promise.all([
          commands.getSettings(),
          commands.appInfo(),
          commands.listProfiles(),
        ]);
        setSettings(s);
        setInfo(i);
        setProfiles(ps);
        applyTheme(s.theme);
      } catch (e) {
        setError(describeError(e));
      }
    })();
  }, []);

  // Menu events
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onMenuAction((action) => {
      if (action === "new_connection") setConnectionFormOpen(true);
      else if (action === "command_palette") setPaletteOpen((o) => !o);
      else if (action === "new_tab") {
        if (active) {
          const db = active.databases[0]?.name ?? "admin";
          openQueryTab(active.handle.connectionId, db, db + "_new");
        }
      }
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  }, [active]);

  // Command palette shortcut
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      } else if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setConnectionFormOpen(true);
      } else if (mod && e.key.toLowerCase() === "t") {
        e.preventDefault();
        if (active) {
          const db = active.databases[0]?.name ?? "admin";
          openQueryTab(active.handle.connectionId, db, db + "_new");
        }
      } else if (mod && e.key.toLowerCase() === "b") {
        e.preventDefault();
        const el = document.querySelector(".app__sidebar");
        if (el) (el as HTMLElement).style.display = "";
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  async function refreshProfiles() {
    try {
      setProfiles(await commands.listProfiles());
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }

  async function openProfile(profile: ProfileSummary) {
    setError(null);
    try {
      const handle = await commands.openConnection(profile.id);
      // Fetch collection list for each database so the tree is populated.
      const collections: Record<string, CollectionSummary[]> = {};
      for (const db of handle.databases) {
        try {
          collections[db.name] = await commands.listCollections(
            handle.connectionId,
            db.name,
          );
        } catch {
          collections[db.name] = [];
        }
      }
      setActive({ handle, profile, databases: handle.databases, collections });
      toasts.push(`Connected to ${profile.name}`, "success");
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }

  async function closeConnection() {
    if (!active) return;
    try {
      await commands.closeConnection(active.handle.connectionId);
    } catch {
      // ignore
    }
    setActive(null);
    setTabs([]);
    setActiveTabId(null);
  }

  async function setTheme(theme: AppSettings["theme"]) {
    try {
      const next = { ...(settings ?? { lastConnectionId: null }), theme };
      await commands.updateSettings(next);
      setSettings(next);
      applyTheme(theme);
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }

  async function deleteProfile(profile: ProfileSummary) {
    if (!window.confirm(`Delete connection "${profile.name}"?`)) return;
    try {
      await commands.deleteProfile(profile.id);
      await refreshProfiles();
      toasts.push(`Deleted ${profile.name}`, "success");
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }

  function openQueryTab(connectionId: string, database: string, collection: string) {
    const id = `q:${connectionId}:${database}:${collection}:${Date.now()}`;
    const tab: Tab = { id, kind: "query", connectionId, database, collection };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }
  function openIndexTab(connectionId: string, database: string, collection: string) {
    const id = `i:${connectionId}:${database}:${collection}:${Date.now()}`;
    const tab: Tab = { id, kind: "indexes", connectionId, database, collection };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }
  function openSchemaTab(connectionId: string, database: string, collection: string) {
    const id = `s:${connectionId}:${database}:${collection}:${Date.now()}`;
    const tab: Tab = { id, kind: "schema", connectionId, database, collection };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }
  function openShellTab(connectionId: string, database: string) {
    const id = `sh:${connectionId}:${database}:${Date.now()}`;
    const tab: Tab = {
      id,
      kind: "shell",
      connectionId,
      database,
      collection: "_shell",
    };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }
  function closeTab(id: string) {
    setTabs((current) => {
      const next = current.filter((t) => t.id !== id);
      if (id === activeTabId) {
        setActiveTabId(next.length > 0 ? next[next.length - 1].id : null);
      }
      return next;
    });
  }

  const paletteItems = useMemo<CommandPaletteItem[]>(() => {
    const items: CommandPaletteItem[] = [
      {
        id: "new-connection",
        label: "New connection…",
        hint: "Save a MongoDB connection profile",
        shortcut: "CmdOrCtrl+N",
        run: () => setConnectionFormOpen(true),
      },
      {
        id: "toggle-palette",
        label: "Open command palette",
        hint: "Quick navigation and commands",
        shortcut: "CmdOrCtrl+K",
        run: () => setPaletteOpen(true),
      },
    ];
    if (active) {
      items.push({
        id: "close-conn",
        label: `Disconnect from ${active.handle.name}`,
        hint: "Close the active connection",
        run: closeConnection,
      });
      for (const db of active.databases) {
        const collections = active.collections[db.name] ?? [];
        for (const coll of collections) {
          items.push({
            id: `q:${db.name}:${coll.name}`,
            label: `Open ${db.name}.${coll.name}`,
            hint: "Query tab",
            run: () => openQueryTab(active.handle.connectionId, db.name, coll.name),
          });
        }
      }
    }
    return items;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, tabs]);

  const tree = useMemo(() => {
    if (!active) return null;
    const filter = treeFilter.toLowerCase();
    return (
      <div className="app__sidebar-list">
        {active.databases.map((db) => {
          const collections = (active.collections[db.name] ?? []).filter((c) =>
            c.name.toLowerCase().includes(filter),
          );
          return (
            <div key={db.name} className="tree-group">
              <div className="tree-group__label">{db.name}</div>
              {collections.length === 0 && (
                <div className="tree-item" style={{ color: "var(--ink-faint)" }}>
                  <span className="tree-item__name">No collections</span>
                </div>
              )}
              {collections.map((c) => (
                <div key={`${db.name}.${c.name}`}>
                  <div className="tree-item" onDoubleClick={() => openQueryTab(active.handle.connectionId, db.name, c.name)}>
                    <span className="tree-item__icon" aria-hidden="true">
                      ▢
                    </span>
                    <span className="tree-item__name">{c.name}</span>
                    <span className="tree-item__meta">
                      {c.documentCount != null ? formatNumber(c.documentCount) : ""}
                    </span>
                    <button
                      className="btn btn--ghost btn--sm"
                      onClick={(e) => {
                        e.stopPropagation();
                        openQueryTab(active.handle.connectionId, db.name, c.name);
                      }}
                      title="Open query tab"
                    >
                      Find
                    </button>
                    <button
                      className="btn btn--ghost btn--sm"
                      onClick={(e) => {
                        e.stopPropagation();
                        openIndexTab(active.handle.connectionId, db.name, c.name);
                      }}
                      title="Open indexes"
                    >
                      Indexes
                    </button>
                    <button
                      className="btn btn--ghost btn--sm"
                      onClick={(e) => {
                        e.stopPropagation();
                        openSchemaTab(active.handle.connectionId, db.name, c.name);
                      }}
                      title="Open schema"
                    >
                      Schema
                    </button>
                  </div>
                </div>
              ))}
            </div>
          );
        })}
      </div>
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, treeFilter]);

  const activeTab = useMemo(
    () => tabs.find((t) => t.id === activeTabId) ?? null,
    [tabs, activeTabId],
  );

  return (
    <div className="app">
      <header className="app__titlebar">
        <span className="app__titlebar-brand">NoSQLBuddy</span>
        <span className="kbd">v{info?.appVersion ?? "0.1.0"}</span>
        <div
          className={`app__titlebar-conn ${active ? "is-connected" : error ? "is-error" : ""}`}
        >
          <span className="dot" aria-hidden="true" />
          {active ? (
            <>
              <span>{active.handle.name}</span>
              <span style={{ color: "var(--ink-faint)" }}>·</span>
              <span style={{ color: "var(--ink-faint)" }}>
                {active.handle.serverInfo?.topology ?? "unknown"}
              </span>
              <button className="btn btn--ghost btn--sm" onClick={closeConnection}>
                Disconnect
              </button>
            </>
          ) : (
            <span>No active connection</span>
          )}
        </div>
      </header>
      <aside className="app__sidebar" aria-label="Connection tree">
        <div className="app__sidebar-header">
          <h2 className="app__sidebar-title">Connections</h2>
          <button
            className="btn btn--sm"
            onClick={() => setConnectionFormOpen(true)}
            style={{ marginLeft: "auto" }}
          >
            New
          </button>
        </div>
        {profiles.length === 0 ? (
          <div className="app__sidebar-empty">
            <p>No saved connections yet.</p>
            <button className="btn btn--primary" onClick={() => setConnectionFormOpen(true)}>
              Add your first connection
            </button>
          </div>
        ) : (
          <div className="app__sidebar-list">
            {profiles
              .filter((p) => p.name.toLowerCase().includes(treeFilter.toLowerCase()))
              .map((p) => (
                <div
                  key={p.id}
                  className={`tree-item ${active?.handle.profileId === p.id ? "is-active" : ""}`}
                >
                  <span className="tree-item__icon" aria-hidden="true">
                    ◆
                  </span>
                  <span className="tree-item__name" title={p.maskedUri}>
                    {p.name}
                    {p.group && (
                      <span style={{ color: "var(--ink-faint)", marginLeft: 6 }}>· {p.group}</span>
                    )}
                  </span>
                  <button
                    className="btn btn--ghost btn--sm"
                    onClick={() => openProfile(p)}
                    title="Connect"
                  >
                    Open
                  </button>
                  <button
                    className="btn btn--ghost btn--sm"
                    onClick={() => deleteProfile(p)}
                    title="Delete connection"
                    aria-label={`Delete ${p.name}`}
                  >
                    ×
                  </button>
                </div>
              ))}
          </div>
        )}
        <div className="app__sidebar-search">
          <input
            type="text"
            placeholder="Filter…"
            value={treeFilter}
            onChange={(e) => setTreeFilter(e.target.value)}
          />
        </div>
        {tree}
        <div className="app__sidebar-footer">
          <span className="kbd-bar">
            <span className="kbd">Cmd</span>+<span className="kbd">K</span> palette
          </span>
        </div>
      </aside>
      <main className="app__workspace" aria-label="Workspace">
        <div className="tabs" role="tablist" aria-label="Open tabs">
          {tabs.map((t) => (
            <div
              key={t.id}
              role="tab"
              tabIndex={0}
              aria-selected={t.id === activeTabId}
              className={`tab ${t.id === activeTabId ? "is-active" : ""}`}
              onClick={() => setActiveTabId(t.id)}
              onKeyDown={(e) => e.key === "Enter" && setActiveTabId(t.id)}
            >
              <span>
                {t.kind === "query"
                  ? `Find · ${t.database}.${t.collection}`
                  : t.kind === "indexes"
                    ? `Indexes · ${t.database}.${t.collection}`
                    : t.kind === "shell"
                      ? `Shell · ${t.database}`
                      : t.kind === "audit"
                        ? "ZK Audit Log"
                        : `Schema · ${t.database}.${t.collection}`}
              </span>
              <span
                className="tab__close"
                role="button"
                tabIndex={0}
                aria-label="Close tab"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(t.id);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") closeTab(t.id);
                }}
              >
                ×
              </span>
            </div>
          ))}
          {active && (
            <>
              <button
                className="tab-add"
                onClick={() => openShellTab(active.handle.connectionId, active.databases[0]?.name ?? "admin")}
                title="Open mongo shell (Cmd/Ctrl+;)"
                aria-label="Open shell tab"
                style={{ marginLeft: "var(--space-2)" }}
              >
                &gt;_
              </button>
              <button
                className="tab-add"
                onClick={() => openQueryTab(active.handle.connectionId, active.databases[0]?.name ?? "admin", "new")}
                title="New query tab (Cmd/Ctrl+T)"
                aria-label="New query tab"
              >
                +
              </button>
              <button
                className="tab-add"
                onClick={() => {
                  const id = `audit-${Date.now()}`;
                  setTabs((prev) => [...prev, { id, kind: "audit" }]);
                  setActiveTabId(id);
                }}
                title="Open ZK Audit Log panel"
                aria-label="ZK Audit Log"
                style={{ fontSize: "11px", padding: "0 8px" }}
              >
                Audit
              </button>
            </>
          )}
        </div>
        {activeTab ? (
          <ErrorBoundary label={activeTab.kind}>
            <TabPane
              key={activeTab.id}
              tab={activeTab}
              profile={active?.profile ?? null}
              onQueryTime={setQueryTime}
              onDocCount={setDocCount}
            />
          </ErrorBoundary>
        ) : (
          <div className="empty-state">
            <h2>Connect to a database to get started</h2>
            <p>
              Add a connection in the sidebar, or open one of the saved profiles.
              NoSQLBuddy supports clusters, replica sets, and sharded topologies.
            </p>
            <div className="row row--end">
              <button className="btn btn--primary" onClick={() => setConnectionFormOpen(true)}>
                New connection
              </button>
            </div>
          </div>
        )}
      </main>
      <footer className="statusbar" role="status" aria-live="polite">
        <span className="statusbar__item">
          {info?.platform ?? "—"} · {info?.arch ?? "—"}
        </span>
        <span className="statusbar__item">
          Tauri {info?.tauriVersion ?? "—"}
        </span>
        <span className="statusbar__item">
          {active ? `${active.databases.length} databases` : "No connection"}
        </span>
        <span className="statusbar__spacer" />
        {queryTime != null && (
          <span className="statusbar__item">Query: {queryTime} ms</span>
        )}
        {docCount != null && (
          <span className="statusbar__item">Rows: {docCount}</span>
        )}
        <span className="statusbar__item">
          <label htmlFor="theme-toggle" className="sr-only">Theme</label>
          <select
            id="theme-toggle"
            className="statusbar__select"
            value={settings?.theme ?? "system"}
            onChange={(e) => void setTheme(e.target.value as AppSettings["theme"])}
            title="Switch theme"
          >
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </span>
      </footer>
      <ConnectionForm
        open={connectionFormOpen}
        onClose={() => setConnectionFormOpen(false)}
        onSaved={refreshProfiles}
      />
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        items={paletteItems}
      />
      <ToastStack toasts={toasts.toasts} />
    </div>
  );
}

function TabPane({
  tab,
  profile,
  onQueryTime,
  onDocCount,
}: {
  tab: Tab;
  profile: ProfileSummary | null;
  onQueryTime: (ms: number) => void;
  onDocCount: (count: number) => void;
}) {
  const handleResult = useCallback(
    (page: DocumentPage | null) => {
      if (!page) {
        onQueryTime(0);
        onDocCount(0);
        return;
      }
      onQueryTime(page.executionMs ?? 0);
      onDocCount(page.documents.length);
    },
    [onQueryTime, onDocCount],
  );

  if (tab.kind === "query") {
    return (
      <QueryTab
        connectionId={tab.connectionId}
        database={tab.database}
        collection={tab.collection}
        profile={profile}
        onClose={() => {
          /* tab strip handles close */
        }}
        onResult={handleResult}
      />
    );
  }
  if (tab.kind === "indexes") {
    return (
      <IndexTab
        connectionId={tab.connectionId}
        database={tab.database}
        collection={tab.collection}
      />
    );
  }
  if (tab.kind === "shell") {
    return (
      <ShellTab
        connectionId={tab.connectionId}
        database={tab.database}
        profile={profile}
      />
    );
  }
  if (tab.kind === "audit") {
    return <AuditPanel />;
  }
  return (
    <SchemaTab
      connectionId={tab.connectionId}
      database={tab.database}
      collection={tab.collection}
    />
  );
}

function applyTheme(theme: "system" | "light" | "dark") {
  const root = document.documentElement;
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

function describeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return "Unexpected error";
}

function formatNumber(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}
