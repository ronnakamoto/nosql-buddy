import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import commands, {
  formatError,
  type AppInfo,
  type AppSettings,
  type AuditMode,
  type CollectionSummary,
  type ConnectionHandle,
  type DatabaseSummary,
  type DocumentPage,
  type ProfileSummary,
  type SaveProfileRequest,
} from "./ipc/commands";
import { onMenuAction } from "./ipc/events";
import { ConnectionForm } from "./components/ConnectionForm";
import { CommandPalette, type CommandPaletteItem } from "./components/CommandPalette";
import { QueryTab } from "./features/QueryTab";
import { IndexTab } from "./features/IndexTab";
import { SchemaTab } from "./features/SchemaTab";
import { ShellTab } from "./features/ShellTab";
import { ToastStack, useToasts } from "./components/Toast";
import { ToastProvider } from "./context/ToastContext";
import AuditPanel from "./components/AuditPanel";
import ErrorBoundary from "./components/ErrorBoundary";
import { JobsHub } from "./features/jobs/JobsHub";
import { DataModelTab } from "./features/dataModel/DataModelTab";
import { TimelinePanel } from "./features/timeline/TimelinePanel";
import { DumpWizard } from "./features/backupRestore/DumpWizard";
import { RestoreWizard } from "./features/backupRestore/RestoreWizard";
import type { CollectionItem } from "./features/backupRestore/CollectionCheckList";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ShortcutsMap } from "./components/ShortcutsMap";
import { ShortcutButton } from "./components/ShortcutButton";
import { AboutScreen } from "./components/AboutScreen";
import { ConnectionOverview } from "./components/ConnectionOverview";
import logoUrl from "./assets/logo.png";
import {
  Search,
  Terminal,
  ShieldCheck,
  Database,
  Layers,
  LayoutGrid,
  Server,
  Plus,
  ChevronsUpDown,
  ChevronRight,
  MoreHorizontal,
  Pencil,
  Copy,
  Trash2,
  Plug,
  Unplug,
  HardDrive,
  Download,
  Upload,
  RefreshCw,
  Network,
  History,
  Info,
} from "lucide-react";

type AuditView = "chooser" | "dev" | "production" | "settings";

type Tab =
  | { id: string; kind: "query"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "indexes"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "schema"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "shell"; connectionId: string; database: string; collection: string }
  | { id: string; kind: "audit"; auditMode: AuditMode; auditView: AuditView }
  | { id: string; kind: "jobs"; connectionId?: string | null; profileId?: string | null }
  | { id: string; kind: "timeline"; connectionId?: string | null; profileId?: string | null; database?: string | null; collection?: string | null }
  | { id: string; kind: "diagram"; connectionId: string; database: string };

interface ActiveConnection {
  handle: ConnectionHandle;
  /** Profile metadata used by the Driver Code panel to embed the
   *  user's real URI and a profile/auth comment. Stored alongside
   *  the handle so it survives tab close + reopen. */
  profile: ProfileSummary;
  databases: DatabaseSummary[];
  collections: Record<string, CollectionSummary[]>;
}

/** A unified "new tab" button with a fixed-positioned dropdown menu.
 *  The button lives inside the scrollable `.tabs` list so it sits right
 *  after the last tab. The dropdown is rendered with `position: fixed`
 *  so it escapes the `overflow: auto` clipping context and repositions
 *  itself when it would overflow the right edge of the viewport. */
function NewTabMenu({
  onNewQuery,
  onNewShell,
  onOpenAudit,
  onOpenJobs,
  onOpenDiagram,
  onOpenTimeline,
}: {
  onNewQuery: () => void;
  onNewShell: () => void;
  onOpenAudit: () => void;
  onOpenJobs: () => void;
  onOpenDiagram: () => void;
  onOpenTimeline: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number | null; right: number | null }>({ top: 0, left: 0, right: null });
  const menuRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const updatePosition = useCallback(() => {
    const btn = triggerRef.current;
    if (!btn) return;
    const rect = btn.getBoundingClientRect();
    const dropdownWidth = 200;
    const gap = 4;
    const top = rect.bottom + gap;
    if (rect.left + dropdownWidth > window.innerWidth - 8) {
      setPos({ top, left: null, right: window.innerWidth - rect.right });
    } else {
      setPos({ top, left: rect.left, right: null });
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    updatePosition();
    const onClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onResize = () => updatePosition();
    document.addEventListener("mousedown", onClick);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onClick);
      window.removeEventListener("resize", onResize);
    };
  }, [open, updatePosition]);

  return (
    <div className="new-tab-menu" ref={menuRef}>
      <button
        className="new-tab-menu__trigger"
        ref={triggerRef}
        onClick={() => {
          if (!open) updatePosition();
          setOpen((o) => !o);
        }}
        aria-label="New tab"
        aria-expanded={open}
        title="New tab (CmdOrCtrl+T)"
      >
        <Plus size={16} />
      </button>
      {open && (
        <div
          className="new-tab-menu__dropdown"
          role="menu"
          style={{
            position: "fixed",
            top: pos.top,
            left: pos.left ?? undefined,
            right: pos.right ?? undefined,
          }}
        >
          <button
            className="new-tab-menu__item"
            role="menuitem"
            onClick={() => { setOpen(false); onNewQuery(); }}
          >
            <span className="new-tab-menu__icon" aria-hidden="true"><Search size={14} /></span>
            <span className="new-tab-menu__label">Query</span>
            <span className="new-tab-menu__hint">Find documents</span>
            <span className="kbd">CmdOrCtrl+T</span>
          </button>
          <button
            className="new-tab-menu__item"
            role="menuitem"
            onClick={() => { setOpen(false); onNewShell(); }}
          >
            <span className="new-tab-menu__icon" aria-hidden="true"><Terminal size={14} /></span>
            <span className="new-tab-menu__label">Shell</span>
            <span className="new-tab-menu__hint">Run mongosh</span>
          </button>
          <button
            className="new-tab-menu__item"
            role="menuitem"
            onClick={() => { setOpen(false); onOpenAudit(); }}
          >
            <span className="new-tab-menu__icon" aria-hidden="true"><ShieldCheck size={14} /></span>
            <span className="new-tab-menu__label">Audit Log</span>
            <span className="new-tab-menu__hint">ZK tamper-evident log</span>
          </button>
          <button
            className="new-tab-menu__item"
            role="menuitem"
            onClick={() => { setOpen(false); onOpenJobs(); }}
          >
            <span className="new-tab-menu__icon" aria-hidden="true"><HardDrive size={14} /></span>
            <span className="new-tab-menu__label">Jobs</span>
            <span className="new-tab-menu__hint">Dump, restore, export, import</span>
          </button>
          <button
            className="new-tab-menu__item"
            role="menuitem"
            onClick={() => { setOpen(false); onOpenTimeline(); }}
          >
            <span className="new-tab-menu__icon" aria-hidden="true"><History size={14} /></span>
            <span className="new-tab-menu__label">Timeline</span>
            <span className="new-tab-menu__hint">Operation history & audit trail</span>
          </button>
          <button
            className="new-tab-menu__item"
            role="menuitem"
            onClick={() => { setOpen(false); onOpenDiagram(); }}
          >
            <span className="new-tab-menu__icon" aria-hidden="true"><Network size={14} /></span>
            <span className="new-tab-menu__label">Data Model</span>
            <span className="new-tab-menu__hint">Schema map & relationships</span>
          </button>
        </div>
      )}
    </div>
  );
}

// ─── Per-connection overflow menu ────────────────────────────────
// Fixed-positioned so it escapes the popover's own scroll clipping,
// mirroring the NewTabMenu technique.
function ConnectionRowMenu({
  isActive,
  onConnect,
  onDisconnect,
  onEdit,
  onDuplicate,
  onDelete,
}: {
  isActive: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number | null; right: number | null }>({ top: 0, left: null, right: null });
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const place = useCallback(() => {
    const btn = triggerRef.current;
    if (!btn) return;
    const r = btn.getBoundingClientRect();
    const width = 184;
    const top = r.bottom + 4;
    if (r.right - width < 8) setPos({ top, left: r.left, right: null });
    else setPos({ top, left: null, right: window.innerWidth - r.right });
  }, []);

  useEffect(() => {
    if (!open) return;
    place();
    const onDown = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      if (triggerRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", place);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", place);
    };
  }, [open, place]);

  const run = (fn: () => void) => () => { setOpen(false); fn(); };

  return (
    <>
      <button
        ref={triggerRef}
        className="conn-row__more"
        aria-label="Connection actions"
        aria-expanded={open}
        title="Actions"
        onClick={(e) => { e.stopPropagation(); if (!open) place(); setOpen((o) => !o); }}
      >
        <MoreHorizontal size={15} />
      </button>
      {open && (
        <div
          ref={menuRef}
          className="row-menu"
          role="menu"
          style={{ position: "fixed", top: pos.top, left: pos.left ?? undefined, right: pos.right ?? undefined }}
          onClick={(e) => e.stopPropagation()}
        >
          {isActive ? (
            <button className="row-menu__item" role="menuitem" onClick={run(onDisconnect)}>
              <Unplug size={14} aria-hidden="true" /> Disconnect
            </button>
          ) : (
            <button className="row-menu__item" role="menuitem" onClick={run(onConnect)}>
              <Plug size={14} aria-hidden="true" /> Connect
            </button>
          )}
          <button className="row-menu__item" role="menuitem" onClick={run(onEdit)}>
            <Pencil size={14} aria-hidden="true" /> Edit
          </button>
          <button className="row-menu__item" role="menuitem" onClick={run(onDuplicate)}>
            <Copy size={14} aria-hidden="true" /> Duplicate
          </button>
          <div className="row-menu__sep" role="separator" />
          <button className="row-menu__item row-menu__item--danger" role="menuitem" onClick={run(onDelete)}>
            <Trash2 size={14} aria-hidden="true" /> Delete
          </button>
        </div>
      )}
    </>
  );
}

// ─── Per-database overflow menu ───────────────────────────────────
function DatabaseRowMenu({
  onDump,
  onRestore,
  onVisualize,
}: {
  onDump: () => void;
  onRestore: () => void;
  onVisualize: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number | null; right: number | null }>({ top: 0, left: null, right: null });
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const place = useCallback(() => {
    const btn = triggerRef.current;
    if (!btn) return;
    const r = btn.getBoundingClientRect();
    const width = 184;
    const top = r.bottom + 4;
    if (r.right - width < 8) setPos({ top, left: r.left, right: null });
    else setPos({ top, left: null, right: window.innerWidth - r.right });
  }, []);

  useEffect(() => {
    if (!open) return;
    place();
    const onDown = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      if (triggerRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", place);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", place);
    };
  }, [open, place]);

  const run = (fn: () => void) => () => { setOpen(false); fn(); };

  return (
    <>
      <button
        ref={triggerRef}
        className="conn-row__more"
        style={{ marginLeft: 4, opacity: open ? 1 : undefined }}
        aria-label="Database actions"
        aria-expanded={open}
        title="Actions"
        onClick={(e) => { e.stopPropagation(); if (!open) place(); setOpen((o) => !o); }}
      >
        <MoreHorizontal size={13} />
      </button>
      {open && (
        <div
          ref={menuRef}
          className="row-menu"
          role="menu"
          style={{ position: "fixed", top: pos.top, left: pos.left ?? undefined, right: pos.right ?? undefined }}
          onClick={(e) => e.stopPropagation()}
        >
          <button className="row-menu__item" role="menuitem" onClick={run(onDump)}>
            <Download size={14} aria-hidden="true" /> Dump database
          </button>
          <button className="row-menu__item" role="menuitem" onClick={run(onRestore)}>
            <Upload size={14} aria-hidden="true" /> Restore to database
          </button>
          <button className="row-menu__item" role="menuitem" onClick={run(onVisualize)}>
            <Network size={14} aria-hidden="true" /> Visualize data model
          </button>
        </div>
      )}
    </>
  );
}

// ─── Connection switcher (active status + searchable popover) ──────
function ConnectionSwitcher({
  active,
  profiles,
  error,
  open,
  onOpenChange,
  onConnect,
  onDisconnect,
  onAdd,
  onEdit,
  onDuplicate,
  onDelete,
}: {
  active: ActiveConnection | null;
  profiles: ProfileSummary[];
  error: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConnect: (p: ProfileSummary) => void;
  onDisconnect: () => void;
  onAdd: () => void;
  onEdit: (p: ProfileSummary) => void;
  onDuplicate: (p: ProfileSummary) => void;
  onDelete: (p: ProfileSummary) => void;
}) {
  const [search, setSearch] = useState("");
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const rootRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) onOpenChange(false);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onOpenChange(false); };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    const raf = requestAnimationFrame(() => searchRef.current?.focus());
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      cancelAnimationFrame(raf);
    };
  }, [open, onOpenChange]);

  const status: "connected" | "error" | "idle" = active ? "connected" : error ? "error" : "idle";

  const q = search.trim().toLowerCase();
  const filtered = profiles.filter(
    (p) => p.name.toLowerCase().includes(q) || (p.group ?? "").toLowerCase().includes(q),
  );
  const groups = new Map<string, ProfileSummary[]>();
  for (const p of filtered) {
    const key = p.group?.trim() || "Ungrouped";
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push(p);
  }
  const groupNames = [...groups.keys()].sort((a, b) =>
    a === "Ungrouped" ? 1 : b === "Ungrouped" ? -1 : a.localeCompare(b),
  );

  return (
    <div className="conn-switcher" ref={rootRef}>
      <button
        className="conn-switcher__trigger"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        title="Switch or manage connections"
      >
        <span className={`conn-status conn-status--${status}`} aria-hidden="true" />
        <span className="conn-switcher__text">
          <span className="conn-switcher__name">{active ? active.handle.name : "No connection"}</span>
          <span className="conn-switcher__meta">
            {active
              ? (active.handle.serverInfo?.topology ?? "connected")
              : error
                ? "Connection error"
                : `${profiles.length} saved connection${profiles.length === 1 ? "" : "s"}`}
          </span>
        </span>
        <ChevronsUpDown size={15} className="conn-switcher__chevron" aria-hidden="true" />
      </button>

      {open && (
        <div className="conn-pop" role="dialog" aria-label="Connections">
          <div className="conn-pop__search">
            <Search size={13} aria-hidden="true" />
            <input
              ref={searchRef}
              type="text"
              placeholder="Search connections…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              aria-label="Search connections"
            />
          </div>

          <div className="conn-pop__list">
            {filtered.length === 0 ? (
              <div className="conn-pop__empty">
                {profiles.length === 0 ? "No saved connections yet." : "No matches."}
              </div>
            ) : (
              groupNames.map((g) => {
                const items = groups.get(g)!;
                const isCollapsed = !!collapsed[g];
                return (
                  <div className="conn-group" key={g}>
                    <button
                      className="conn-group__header"
                      onClick={() => setCollapsed((c) => ({ ...c, [g]: !c[g] }))}
                      aria-expanded={!isCollapsed}
                    >
                      <ChevronRight
                        size={12}
                        className={`conn-group__chevron ${isCollapsed ? "" : "is-open"}`}
                        aria-hidden="true"
                      />
                      <span className="conn-group__label">{g}</span>
                      <span className="conn-group__count">{items.length}</span>
                    </button>
                    {!isCollapsed &&
                      items.map((p) => {
                        const isActive = active?.profile.id === p.id;
                        return (
                          <div
                            key={p.id}
                            className={`conn-row ${isActive ? "is-active" : ""}`}
                            role="button"
                            tabIndex={0}
                            onClick={() => { if (!isActive) onConnect(p); onOpenChange(false); }}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") { if (!isActive) onConnect(p); onOpenChange(false); }
                            }}
                            title={p.maskedUri}
                          >
                            <Server
                              size={14}
                              className="conn-row__icon"
                              style={!isActive && p.color ? { color: p.color } : undefined}
                              aria-hidden="true"
                            />
                            <span className="conn-row__text">
                              <span className="conn-row__name">{p.name}</span>
                              <span className="conn-row__uri">{p.maskedUri}</span>
                            </span>
                            {isActive && <span className="conn-row__badge">Connected</span>}
                            <ConnectionRowMenu
                              isActive={isActive}
                              onConnect={() => { onConnect(p); onOpenChange(false); }}
                              onDisconnect={() => { onDisconnect(); onOpenChange(false); }}
                              onEdit={() => { onEdit(p); onOpenChange(false); }}
                              onDuplicate={() => { onDuplicate(p); onOpenChange(false); }}
                              onDelete={() => onDelete(p)}
                            />
                          </div>
                        );
                      })}
                  </div>
                );
              })
            )}
          </div>

          <div className="conn-pop__footer">
            <button className="conn-pop__add" onClick={() => { onOpenChange(false); onAdd(); }}>
              <Plus size={14} aria-hidden="true" /> Add connection
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export default function App() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [profiles, setProfiles] = useState<ProfileSummary[]>([]);
  const [active, setActive] = useState<ActiveConnection | null>(null);
  const [treeFilter, setTreeFilter] = useState("");
  const [connectionFormOpen, setConnectionFormOpen] = useState(false);
  const [connFormInitial, setConnFormInitial] = useState<Partial<SaveProfileRequest> | undefined>(undefined);
  const [connFormKey, setConnFormKey] = useState(0);
  const [connSwitcherOpen, setConnSwitcherOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsMapOpen, setShortcutsMapOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [queryTime, setQueryTime] = useState<number | null>(null);
  const [docCount, setDocCount] = useState<number | null>(null);
  const [dumpTarget, setDumpTarget] = useState<{ connectionId: string; database: string; collections: CollectionItem[] } | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<{ connectionId: string; database: string } | null>(null);
  const [refreshingConn, setRefreshingConn] = useState(false);
  const [refreshingDb, setRefreshingDb] = useState<string | null>(null);
  const [pendingDeleteProfile, setPendingDeleteProfile] = useState<ProfileSummary | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(300);
  const sidebarResizeRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const toasts = useToasts();

  // ─── Sidebar resize ─────────────────────────────────────────────────
  const startSidebarResize = useCallback((e: React.MouseEvent) => {
    sidebarResizeRef.current = { startX: e.clientX, startWidth: sidebarWidth };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const onMove = (ev: MouseEvent) => {
      const ref = sidebarResizeRef.current;
      if (!ref) return;
      const dx = ev.clientX - ref.startX;
      const next = Math.min(600, Math.max(200, ref.startWidth + dx));
      setSidebarWidth(next);
    };
    const onUp = () => {
      sidebarResizeRef.current = null;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [sidebarWidth]);

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
      else if (action === "export_results") {
        window.dispatchEvent(new CustomEvent("nosqlbuddy:export-results"));
      }
      else if (action === "import_data") {
        window.dispatchEvent(new CustomEvent("nosqlbuddy:import-data"));
      }
      else if (action === "dump_database") {
        if (active) {
          const db = active.databases[0]?.name ?? "";
          setDumpTarget({ connectionId: active.handle.connectionId, database: db, collections: [] });
        }
      }
      else if (action === "restore_database") {
        if (active) {
          setRestoreTarget({ connectionId: active.handle.connectionId, database: "" });
        }
      }
      else if (action === "new_tab") {
        if (active) {
          const db = active.databases[0]?.name ?? "admin";
          openQueryTab(active.handle.connectionId, db, db + "_new");
        }
      }
      else if (action === "about") {
        setAboutOpen(true);
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
      } else if (mod && e.key === "?") {
        e.preventDefault();
        setShortcutsMapOpen(true);
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

  const openProfile = useCallback(async (profile: ProfileSummary) => {
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
  }, [toasts]);

  const refreshConnection = useCallback(async () => {
    if (!active) return;
    try {
      const handle = await commands.openConnection(active.profile.id);
      const collections: Record<string, CollectionSummary[]> = {};
      for (const db of handle.databases) {
        try {
          collections[db.name] = await commands.listCollections(handle.connectionId, db.name);
        } catch {
          collections[db.name] = [];
        }
      }
      setActive({ ...active, handle, databases: handle.databases, collections });
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }, [active, toasts]);

  const refreshDatabase = useCallback(async (database: string) => {
    if (!active) return;
    try {
      const cols = await commands.listCollections(active.handle.connectionId, database);
      setActive((prev) => {
        if (!prev) return prev;
        return { ...prev, collections: { ...prev.collections, [database]: cols } };
      });
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }, [active, toasts]);

  const closeConnection = useCallback(async () => {
    if (!active) return;
    try {
      await commands.closeConnection(active.handle.connectionId);
    } catch {
      // ignore
    }
    setActive(null);
    setTabs([]);
    setActiveTabId(null);
  }, [active]);

  // Switch the active connection: close the current handle (and its tabs),
  // then open the chosen profile. Re-selecting the active one is a no-op.
  const switchConnection = useCallback(async (profile: ProfileSummary) => {
    if (active?.profile.id === profile.id) return;
    if (active) {
      try {
        await commands.closeConnection(active.handle.connectionId);
      } catch {
        // ignore
      }
      setTabs([]);
      setActiveTabId(null);
    }
    await openProfile(profile);
  }, [active, openProfile]);

  const openAddConnection = useCallback(() => {
    setConnFormInitial(undefined);
    setConnFormKey((k) => k + 1);
    setConnectionFormOpen(true);
  }, []);

  const editConnection = useCallback(async (profile: ProfileSummary) => {
    let uri = profile.maskedUri;
    try {
      uri = await commands.resolveProfileUri(profile.id);
    } catch {
      // fall back to masked URI; saving still updates name/group/notes
    }
    setConnFormInitial({
      id: profile.id,
      name: profile.name,
      uri,
      authMechanism: profile.authMechanism,
      group: profile.group,
      notes: profile.notes,
      tls: profile.tls,
    });
    setConnFormKey((k) => k + 1);
    setConnectionFormOpen(true);
  }, []);

  const duplicateConnection = useCallback(async (profile: ProfileSummary) => {
    let uri = profile.maskedUri;
    try {
      uri = await commands.resolveProfileUri(profile.id);
    } catch {
      // fall back to masked URI
    }
    setConnFormInitial({
      name: `${profile.name} copy`,
      uri,
      authMechanism: profile.authMechanism,
      group: profile.group,
      notes: profile.notes,
      tls: profile.tls,
    });
    setConnFormKey((k) => k + 1);
    setConnectionFormOpen(true);
  }, []);

  const setTheme = useCallback(async (theme: AppSettings["theme"]) => {
    try {
      const next = { ...(settings ?? { lastConnectionId: null }), theme };
      await commands.updateSettings(next);
      setSettings(next);
      applyTheme(theme);
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }, [settings, toasts]);

  const deleteProfile = useCallback((profile: ProfileSummary) => {
    setPendingDeleteProfile(profile);
  }, []);

  const confirmDeleteProfile = useCallback(async () => {
    if (!pendingDeleteProfile) return;
    const profile = pendingDeleteProfile;
    setPendingDeleteProfile(null);
    try {
      await commands.deleteProfile(profile.id);
      await refreshProfiles();
      toasts.push(`Deleted ${profile.name}`, "success");
    } catch (e) {
      toasts.push(describeError(e), "error");
    }
  }, [pendingDeleteProfile, toasts]);

  const openQueryTab = useCallback((connectionId: string, database: string, collection: string) => {
    const id = `q:${connectionId}:${database}:${collection}:${Date.now()}`;
    const tab: Tab = { id, kind: "query", connectionId, database, collection };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }, []);

  const openIndexTab = useCallback((connectionId: string, database: string, collection: string) => {
    const id = `i:${connectionId}:${database}:${collection}:${Date.now()}`;
    const tab: Tab = { id, kind: "indexes", connectionId, database, collection };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }, []);

  const openSchemaTab = useCallback((connectionId: string, database: string, collection: string) => {
    const id = `s:${connectionId}:${database}:${collection}:${Date.now()}`;
    const tab: Tab = { id, kind: "schema", connectionId, database, collection };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }, []);

  const openDiagramTab = useCallback((connectionId: string, database: string) => {
    const id = `dm:${connectionId}:${database}:${Date.now()}`;
    const tab: Tab = { id, kind: "diagram", connectionId, database };
    setTabs((current) => [...current, tab]);
    setActiveTabId(id);
  }, []);

  const openShellTab = useCallback((connectionId: string, database: string) => {
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
  }, []);

  const openAuditTab = useCallback(() => {
    const id = `audit-${Date.now()}`;
    setTabs((current) => [...current, { id, kind: "audit", auditMode: "dev", auditView: "chooser" }]);
    setActiveTabId(id);
  }, []);

  const openJobsTab = useCallback(() => {
    const id = `jobs-${Date.now()}`;
    setTabs((current) => [
      ...current,
      {
        id,
        kind: "jobs",
        connectionId: active?.handle.connectionId ?? null,
        profileId: active?.handle.profileId ?? null,
      },
    ]);
    setActiveTabId(id);
  }, [active]);

  const openTimelineTab = useCallback(() => {
    const id = `timeline-${Date.now()}`;
    setTabs((current) => [
      ...current,
      {
        id,
        kind: "timeline",
        connectionId: active?.handle.connectionId ?? null,
        profileId: active?.handle.profileId ?? null,
        database: null,
        collection: null,
      },
    ]);
    setActiveTabId(id);
  }, [active]);

  // Connection-dependent launchers shared by the feature toolbar and the
  // tab-bar "+" menu. They no-op without an active connection.
  const handleNewQuery = useCallback(() => {
    if (!active) return;
    openQueryTab(active.handle.connectionId, active.databases[0]?.name ?? "admin", "new");
  }, [active, openQueryTab]);

  const handleNewShell = useCallback(() => {
    if (!active) return;
    openShellTab(active.handle.connectionId, active.databases[0]?.name ?? "admin");
  }, [active, openShellTab]);

  const handleNewDiagram = useCallback(() => {
    if (!active) return;
    openDiagramTab(active.handle.connectionId, active.databases[0]?.name ?? "admin");
  }, [active, openDiagramTab]);

  const closeTab = useCallback((id: string) => {
    setTabs((current) => {
      const next = current.filter((t) => t.id !== id);
      if (id === activeTabId) {
        setActiveTabId(next.length > 0 ? next[next.length - 1].id : null);
      }
      return next;
    });
  }, [activeTabId]);

  const updateTab = useCallback((id: string, patch: Partial<Tab>) => {
    setTabs((current) =>
      current.map((t) => (t.id === id ? ({ ...t, ...patch } as Tab) : t))
    );
  }, []);

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
      {
        id: "shortcuts-map",
        label: "Keyboard shortcuts",
        hint: "View all available keyboard shortcuts",
        shortcut: "CmdOrCtrl+?",
        run: () => setShortcutsMapOpen(true),
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
        items.push({
          id: `dm:${db.name}`,
          label: `Visualize data model for ${db.name}`,
          hint: "Data Model tab",
          run: () => openDiagramTab(active.handle.connectionId, db.name),
        });
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
    items.push({
      id: "about",
      label: "About NoSQLBuddy",
      hint: "App version, system info, and credits",
      run: () => setAboutOpen(true),
    });
    return items;
  }, [active, tabs, closeConnection, openDiagramTab, openQueryTab]);

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
              <div className="tree-group__header">
                <span className="tree-group__icon" aria-hidden="true"><Database size={13} /></span>
                <span className="tree-group__label">{db.name}</span>
                <span className="tree-group__count">{collections.length} collections</span>
                <button
                  className="tree-item__action"
                  style={{ opacity: refreshingDb === db.name ? 1 : undefined }}
                  onClick={async (e) => {
                    e.stopPropagation();
                    setRefreshingDb(db.name);
                    await refreshDatabase(db.name);
                    setRefreshingDb(null);
                  }}
                  disabled={refreshingDb === db.name}
                  title="Refresh database"
                  aria-label="Refresh database"
                >
                  <RefreshCw size={13} className={refreshingDb === db.name ? "spin" : ""} />
                </button>
                {active && (
                  <DatabaseRowMenu
                    onDump={() => {
                      const items: CollectionItem[] = collections.map((c) => ({
                        name: c.name,
                        documentCount: c.documentCount,
                        sizeBytes: c.sizeBytes,
                      }));
                      setDumpTarget({ connectionId: active.handle.connectionId, database: db.name, collections: items });
                    }}
                    onRestore={() => {
                      setRestoreTarget({ connectionId: active.handle.connectionId, database: db.name });
                    }}
                    onVisualize={() => {
                      openDiagramTab(active.handle.connectionId, db.name);
                    }}
                  />
                )}
              </div>
              {collections.length === 0 && (
                <div className="tree-item" style={{ color: "var(--ink-faint)", cursor: "default" }}>
                  <span className="tree-item__name">No collections</span>
                </div>
              )}
              {collections.map((c) => (
                <div key={`${db.name}.${c.name}`}>
                  <div
                    className="tree-item tree-item--collection"
                    onClick={() => openQueryTab(active.handle.connectionId, db.name, c.name)}
                    title={`${db.name}.${c.name}`}
                    tabIndex={0}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") openQueryTab(active.handle.connectionId, db.name, c.name);
                    }}
                  >
                    <span className="tree-item__icon" aria-hidden="true"><Layers size={13} /></span>
                    <span className="tree-item__name">{c.name}</span>
                    <span className="tree-item__meta">
                      {c.documentCount != null ? formatNumber(c.documentCount) : ""}
                    </span>
                    <span className="tree-item__actions" role="group" aria-label="Collection actions">
                      <button
                        className="tree-item__action"
                        onClick={(e) => {
                          e.stopPropagation();
                          openQueryTab(active.handle.connectionId, db.name, c.name);
                        }}
                        title="Find documents"
                        aria-label="Find documents"
                      >
                        <Search size={13} />
                      </button>
                      <button
                        className="tree-item__action"
                        onClick={(e) => {
                          e.stopPropagation();
                          openIndexTab(active.handle.connectionId, db.name, c.name);
                        }}
                        title="Indexes"
                        aria-label="Indexes"
                      >
                        <Database size={13} />
                      </button>
                      <button
                        className="tree-item__action"
                        onClick={(e) => {
                          e.stopPropagation();
                          openSchemaTab(active.handle.connectionId, db.name, c.name);
                        }}
                        title="Schema"
                        aria-label="Schema"
                      >
                        <LayoutGrid size={13} />
                      </button>
                    </span>
                  </div>
                </div>
              ))}
            </div>
          );
        })}
      </div>
    );
  }, [
    active,
    treeFilter,
    refreshingDb,
    refreshDatabase,
    openQueryTab,
    openIndexTab,
    openSchemaTab,
  ]);

  const activeTab = useMemo(
    () => tabs.find((t) => t.id === activeTabId) ?? null,
    [tabs, activeTabId],
  );

  return (
    <ToastProvider value={toasts}>
    <div className="app">
      <header className="app__titlebar">
        <img className="app__titlebar-logo" src={logoUrl} alt="NoSQLBuddy" draggable={false} />
        <span className="app__titlebar-brand">NoSQLBuddy</span>
        <span className="kbd">v{info?.appVersion ?? "0.1.0"}</span>
      </header>
      <nav className="app__toolbar" aria-label="Open feature">
        <div className="app__toolbar-group">
          <button
            className="toolbar-btn"
            onClick={handleNewQuery}
            disabled={!active}
            title={active ? "Open a Query tab (CmdOrCtrl+T)" : "Connect to open a Query tab"}
          >
            <Search size={15} aria-hidden="true" />
            <span>Query</span>
          </button>
          <button
            className="toolbar-btn"
            onClick={handleNewShell}
            disabled={!active}
            title={active ? "Open a Shell tab" : "Connect to open a Shell tab"}
          >
            <Terminal size={15} aria-hidden="true" />
            <span>Shell</span>
          </button>
          <button
            className="toolbar-btn"
            onClick={handleNewDiagram}
            disabled={!active}
            title={active ? "Open a Data Model tab" : "Connect to open a Data Model tab"}
          >
            <Network size={15} aria-hidden="true" />
            <span>Data Model</span>
          </button>
        </div>
        <div className="app__toolbar-sep" role="separator" aria-hidden="true" />
        <div className="app__toolbar-group">
          <button
            className="toolbar-btn"
            onClick={openTimelineTab}
            disabled={!active}
            title={active ? "Open the Timeline" : "Connect to open the Timeline"}
          >
            <History size={15} aria-hidden="true" />
            <span>Timeline</span>
          </button>
          <button
            className="toolbar-btn"
            onClick={openJobsTab}
            disabled={!active}
            title={active ? "Open Jobs (dump, restore, export, import)" : "Connect to open Jobs"}
          >
            <HardDrive size={15} aria-hidden="true" />
            <span>Jobs</span>
          </button>
          <button
            className="toolbar-btn"
            onClick={openAuditTab}
            disabled={!active}
            title={active ? "Open the Audit Log" : "Connect to open the Audit Log"}
          >
            <ShieldCheck size={15} aria-hidden="true" />
            <span>Audit</span>
          </button>
        </div>
      </nav>
      <aside className="app__sidebar" aria-label="Connections" style={{ width: sidebarWidth }}>
        <ConnectionSwitcher
          active={active}
          profiles={profiles}
          error={error}
          open={connSwitcherOpen}
          onOpenChange={setConnSwitcherOpen}
          onConnect={switchConnection}
          onDisconnect={closeConnection}
          onAdd={openAddConnection}
          onEdit={editConnection}
          onDuplicate={duplicateConnection}
          onDelete={deleteProfile}
        />
        <div className="app__sidebar-explorer">
          {active ? (
            <>
              <div className="app__sidebar-toolbar">
                <button
                  className="btn btn--ghost btn--sm"
                  onClick={async () => {
                    setRefreshingConn(true);
                    await refreshConnection();
                    setRefreshingConn(false);
                  }}
                  disabled={refreshingConn}
                  title="Refresh connection"
                  aria-label="Refresh connection"
                >
                  <RefreshCw size={13} className={refreshingConn ? "spin" : ""} />
                  <span>Refresh</span>
                </button>
                <div className="app__sidebar-toolbar-spacer" />
                <button
                  className="btn btn--ghost btn--sm"
                  onClick={closeConnection}
                  title="Disconnect"
                  aria-label="Disconnect"
                >
                  <Unplug size={13} />
                  <span>Disconnect</span>
                </button>
              </div>
              <div className="app__sidebar-search">
                <span className="app__sidebar-search-icon" aria-hidden="true"><Search size={13} /></span>
                <input
                  type="text"
                  placeholder="Filter collections…"
                  value={treeFilter}
                  onChange={(e) => setTreeFilter(e.target.value)}
                  aria-label="Filter collections"
                />
                {treeFilter && (
                  <button
                    className="app__sidebar-search-clear"
                    onClick={() => setTreeFilter("")}
                    title="Clear filter"
                    aria-label="Clear filter"
                  >
                    ×
                  </button>
                )}
              </div>
              {tree}
            </>
          ) : (
            <div className="app__sidebar-empty">
              <p>{profiles.length === 0 ? "No saved connections yet." : "Not connected."}</p>
              <button
                className="btn btn--primary btn--sm"
                onClick={() => (profiles.length === 0 ? openAddConnection() : setConnSwitcherOpen(true))}
              >
                {profiles.length === 0 ? "Add your first connection" : "Choose a connection"}
              </button>
            </div>
          )}
        </div>
        <div className="app__sidebar-footer">
          <span className="kbd-bar">
            <span className="kbd">Cmd</span>+<span className="kbd">K</span> palette
          </span>
          <span className="app__sidebar-footer-spacer" />
          <button
            className="btn btn--sm btn--ghost"
            onClick={() => setAboutOpen(true)}
            title="About NoSQLBuddy"
            aria-label="About NoSQLBuddy"
          >
            <Info size={14} />
          </button>
        </div>
      </aside>
      <div
        className="app__sidebar-resizer"
        onMouseDown={startSidebarResize}
        aria-hidden="true"
        title="Drag to resize sidebar"
      />
      <main className="app__workspace" aria-label="Workspace">
        <div className="tabs" role="tablist" aria-label="Open tabs">
          {tabs.map((t) => (
            <div
              key={t.id}
              role="tab"
              tabIndex={0}
              aria-selected={t.id === activeTabId}
              className={`tab tab--${t.kind} ${t.id === activeTabId ? "is-active" : ""}`}
              onClick={() => setActiveTabId(t.id)}
              onKeyDown={(e) => e.key === "Enter" && setActiveTabId(t.id)}
            >
              <span className="tab__icon" aria-hidden="true">
                {t.kind === "query"
                  ? <Search size={14} />
                  : t.kind === "indexes"
                    ? <Database size={14} />
                    : t.kind === "schema"
                      ? <LayoutGrid size={14} />
                      : t.kind === "shell"
                        ? <Terminal size={14} />
                        : t.kind === "jobs"
                          ? <HardDrive size={14} />
                          : t.kind === "timeline"
                            ? <History size={14} />
                            : t.kind === "diagram"
                              ? <Network size={14} />
                              : <ShieldCheck size={14} />}
              </span>
              <span className="tab__label">
                {t.kind === "query"
                  ? `${t.database}.${t.collection}`
                  : t.kind === "indexes"
                    ? `${t.database}.${t.collection}`
                    : t.kind === "schema"
                      ? `${t.database}.${t.collection}`
                      : t.kind === "shell"
                        ? t.database
                        : t.kind === "diagram"
                          ? t.database
                          : t.kind === "jobs"
                            ? "Jobs"
                            : t.kind === "timeline"
                              ? "Timeline"
                              : "Audit Log"}
              </span>
              <span className="tab__kind" aria-hidden="true">
                {t.kind === "query"
                  ? "Find"
                  : t.kind === "indexes"
                    ? "Indexes"
                    : t.kind === "schema"
                      ? "Schema"
                      : t.kind === "shell"
                        ? "Shell"
                        : t.kind === "diagram"
                          ? "Model"
                          : t.kind === "jobs"
                            ? "Jobs"
                            : t.kind === "timeline"
                              ? "Timeline"
                              : "ZK"}
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
            <NewTabMenu
              onNewQuery={handleNewQuery}
              onNewShell={handleNewShell}
              onOpenAudit={openAuditTab}
              onOpenJobs={openJobsTab}
              onOpenDiagram={handleNewDiagram}
              onOpenTimeline={openTimelineTab}
            />
          )}
        </div>
        {activeTab ? (
          <ErrorBoundary label={activeTab.kind}>
            <TabPane
              key={activeTab.id}
              tab={activeTab}
              profile={active?.profile ?? null}
              connectionId={active?.handle.connectionId ?? null}
              onQueryTime={setQueryTime}
              onDocCount={setDocCount}
              updateTab={updateTab}
              onImported={refreshConnection}
            />
          </ErrorBoundary>
        ) : active ? (
          <ConnectionOverview
            active={active}
            onNewQuery={handleNewQuery}
          />
        ) : (
          <div className="empty-state">
            <h2>Connect to a database to get started</h2>
            <p>
              Add a connection in the sidebar, or open one of the saved profiles.
              NoSQLBuddy supports clusters, replica sets, and sharded topologies.
            </p>
            <div className="row row--end">
              <ShortcutButton
                variant="primary"
                shortcut="CmdOrCtrl+N"
                onClick={() => setConnectionFormOpen(true)}
                className="btn btn--primary"
              >
                New connection
              </ShortcutButton>
            </div>
          </div>
        )}
      </main>
      <footer className="statusbar" role="status" aria-live="polite">
        <span className="statusbar__item" title={`${active?.databases.length ?? 0} databases on this connection`}>
          {active ? `${active.databases.length} databases` : "No connection"}
        </span>
        {active && (
          <span className="statusbar__item">
            {Object.values(active.collections).reduce((n, cols) => n + cols.length, 0)} collections
          </span>
        )}
        <span className="statusbar__spacer" />
        {queryTime != null && (
          <span className="statusbar__item" title="Last query execution time">
            <span className="statusbar__label">Query</span>
            <strong>{queryTime} ms</strong>
          </span>
        )}
        {docCount != null && (
          <span className="statusbar__item" title="Documents returned by last query">
            <span className="statusbar__label">Rows</span>
            <strong>{docCount}</strong>
          </span>
        )}
        <span className="statusbar__item statusbar__item--theme">
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
        key={connFormKey}
        open={connectionFormOpen}
        onClose={() => { setConnectionFormOpen(false); setConnFormInitial(undefined); }}
        onSaved={refreshProfiles}
        initial={connFormInitial}
      />
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        items={paletteItems}
      />
      <ShortcutsMap
        open={shortcutsMapOpen}
        onClose={() => setShortcutsMapOpen(false)}
      />
      <ToastStack toasts={toasts.toasts} onDismiss={toasts.dismiss} />
      {dumpTarget && (
        <DumpWizard
          connectionId={dumpTarget.connectionId}
          database={dumpTarget.database}
          collections={dumpTarget.collections}
          onClose={() => setDumpTarget(null)}
          onDumped={refreshConnection}
        />
      )}
      {restoreTarget && (
        <RestoreWizard
          connectionId={restoreTarget.connectionId}
          onClose={() => setRestoreTarget(null)}
          onRestored={refreshConnection}
        />
      )}
      <ConfirmDialog
        open={pendingDeleteProfile !== null}
        title="Delete connection?"
        description={`"${pendingDeleteProfile?.name}" will be permanently removed. Any saved credentials for this connection will be deleted.`}
        confirmLabel="Delete connection"
        onConfirm={() => void confirmDeleteProfile()}
        onCancel={() => setPendingDeleteProfile(null)}
      />
      <AboutScreen
        open={aboutOpen}
        onClose={() => setAboutOpen(false)}
        info={info}
        onOpenShortcuts={() => setShortcutsMapOpen(true)}
      />
    </div>
    </ToastProvider>
  );
}

const TabPane = memo(function TabPane({
  tab,
  profile,
  connectionId,
  onQueryTime,
  onDocCount,
  updateTab,
  onImported,
}: {
  tab: Tab;
  profile: ProfileSummary | null;
  connectionId: string | null;
  onQueryTime: (ms: number) => void;
  onDocCount: (count: number) => void;
  updateTab: (id: string, patch: Partial<Tab>) => void;
  onImported?: () => void;
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

  const noop = useCallback(() => {}, []);

  if (tab.kind === "query") {
    return (
      <QueryTab
        connectionId={tab.connectionId}
        database={tab.database}
        collection={tab.collection}
        profile={profile}
        onClose={noop}
        onResult={handleResult}
        onImported={onImported}
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
    return (
      <AuditPanel
        mode={tab.auditMode}
        view={tab.auditView}
        connectionId={connectionId}
        onModeChange={(auditMode) => updateTab(tab.id, { auditMode })}
        onViewChange={(auditView) => updateTab(tab.id, { auditView })}
      />
    );
  }
  if (tab.kind === "jobs") {
    return <JobsHub connectionId={tab.connectionId} profileId={tab.profileId} />;
  }
  if (tab.kind === "timeline") {
    return (
      <TimelinePanel
        profileId={tab.profileId}
        database={tab.database}
        collection={tab.collection}
      />
    );
  }
  if (tab.kind === "diagram") {
    return <DataModelTab connectionId={tab.connectionId} database={tab.database} />;
  }
  return (
    <SchemaTab
      connectionId={tab.connectionId}
      database={tab.database}
      collection={tab.collection}
    />
  );
});

function applyTheme(theme: "system" | "light" | "dark") {
  const root = document.documentElement;
  if (theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

function describeError(e: unknown): string {
  return formatError(e);
}

function formatNumber(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}
