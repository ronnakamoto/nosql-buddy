import { useCallback, useEffect, useRef, useState } from "react";
import { Database, Search, Plus, Download, Upload, CalendarClock } from "lucide-react";
import { useJobStore } from "./useJobStore";
import { JobListItem } from "./JobListItem";
import { EditScheduleModal } from "./EditScheduleModal";
import { Alert } from "../../components/Alert";
import { DumpWizard } from "../backupRestore/DumpWizard";
import { RestoreWizard } from "../backupRestore/RestoreWizard";

interface JobsHubProps {
  connectionId?: string | null;
  profileId?: string | null;
}

export function JobsHub({ connectionId, profileId }: JobsHubProps) {
  const {
    jobs,
    loading,
    error,
    startPolling,
    stopPolling,
    cancelJob,
    deleteJob,
    rerunJob,
    updateSchedule,
    toggleScheduleEnabled,
  } = useJobStore();
  const [filterKind, setFilterKind] = useState<string>("");
  const [filterStatus, setFilterStatus] = useState<string>("");
  const [search, setSearch] = useState("");
  const [view, setView] = useState<"all" | "scheduled">("all");
  const [wizard, setWizard] = useState<"dump" | "restore" | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [editingScheduleId, setEditingScheduleId] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    // Filter by the stable profile id so job history and schedules persist
    // across app restarts (connectionId is a fresh UUID on every launch).
    startPolling(
      view === "scheduled"
        ? { profileId: profileId ?? null }
        : {
            profileId: profileId ?? null,
            kind: filterKind || null,
            status: filterStatus || null,
          },
    );
    return () => stopPolling();
  }, [profileId, filterKind, filterStatus, view, startPolling, stopPolling]);

  // Close new-job dropdown on outside click.
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      if (btnRef.current?.contains(e.target as Node)) return;
      setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  const matchesSearch = (j: typeof jobs[number]) => {
    if (!search) return true;
    const q = search.toLowerCase();
    return (
      j.database.toLowerCase().includes(q) ||
      j.connectionId.toLowerCase().includes(q) ||
      j.kind.toLowerCase().includes(q) ||
      j.message.toLowerCase().includes(q)
    );
  };

  // Schedule templates are jobs that carry a schedule config.
  const templates = jobs.filter((j) => j.schedule != null && matchesSearch(j));
  const runsFor = (templateId: string) =>
    jobs.filter((j) => j.parentJobId === templateId);
  const filtered = jobs.filter(matchesSearch);

  const openDump = useCallback(() => {
    setMenuOpen(false);
    setWizard("dump");
  }, []);

  const openRestore = useCallback(() => {
    setMenuOpen(false);
    setWizard("restore");
  }, []);

  const saveSchedule = useCallback(
    (jobId: string, config: Parameters<typeof updateSchedule>[1]) =>
      updateSchedule(jobId, { ...config, profileId: profileId ?? null }),
    [profileId, updateSchedule],
  );

  const toggleSchedule = useCallback(
    (jobId: string, enabled: boolean) =>
      toggleScheduleEnabled(jobId, enabled, profileId ?? null),
    [profileId, toggleScheduleEnabled],
  );

  return (
    <div className="jobs-hub pane">
      <div className="jobs-hub__header">
        <h2 className="jobs-hub__title">
          <Database size={16} aria-hidden="true" />
          Jobs Hub
        </h2>
        <div className="jobs-hub__filters">
          <div className="jobs-hub__view-toggle" role="tablist" aria-label="Job view">
            <button
              role="tab"
              aria-selected={view === "all"}
              className={`btn btn--sm ${view === "all" ? "is-active" : ""}`}
              onClick={() => setView("all")}
            >
              All jobs
            </button>
            <button
              role="tab"
              aria-selected={view === "scheduled"}
              className={`btn btn--sm ${view === "scheduled" ? "is-active" : ""}`}
              onClick={() => setView("scheduled")}
            >
              Scheduled
            </button>
          </div>
          {view === "all" && (
          <select
            className="field__select"
            value={filterKind}
            onChange={(e) => setFilterKind(e.target.value)}
            aria-label="Filter by kind"
          >
            <option value="">All kinds</option>
            <option value="dump">Dump</option>
            <option value="restore">Restore</option>
            <option value="export">Export</option>
            <option value="import">Import</option>
          </select>
          )}
          {view === "all" && (
          <select
            className="field__select"
            value={filterStatus}
            onChange={(e) => setFilterStatus(e.target.value)}
            aria-label="Filter by status"
          >
            <option value="">All statuses</option>
            <option value="queued">Queued</option>
            <option value="running">Running</option>
            <option value="done">Done</option>
            <option value="failed">Failed</option>
            <option value="cancelled">Cancelled</option>
          </select>
          )}
          <div className="jobs-hub__search">
            <Search size={14} className="jobs-hub__search-icon" aria-hidden="true" />
            <input
              type="text"
              className="field__input"
              placeholder="Search jobs..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              aria-label="Search jobs"
            />
          </div>
          <div style={{ position: "relative" }}>
            <button
              ref={btnRef}
              className="btn btn--primary btn--sm"
              onClick={() => setMenuOpen((o) => !o)}
              aria-expanded={menuOpen}
              aria-haspopup="menu"
              disabled={!connectionId}
              title={connectionId ? "Start a new dump or restore job" : "Connect to a database first"}
            >
              <Plus size={14} />
              New job
            </button>
            {menuOpen && (
              <div
                ref={menuRef}
                className="conn-pop"
                role="menu"
                style={{
                  position: "absolute",
                  top: "calc(100% + 4px)",
                  right: 0,
                  width: 200,
                  zIndex: "var(--z-dropdown)",
                }}
              >
                <button className="conn-pop__item" role="menuitem" onClick={openDump}>
                  <span className="conn-pop__icon" aria-hidden="true">
                    <Download size={14} />
                  </span>
                  <span className="conn-pop__label">Dump database</span>
                </button>
                <button className="conn-pop__item" role="menuitem" onClick={openRestore}>
                  <span className="conn-pop__icon" aria-hidden="true">
                    <Upload size={14} />
                  </span>
                  <span className="conn-pop__label">Restore database</span>
                </button>
              </div>
            )}
          </div>
        </div>
      </div>

      {error && <Alert tone="danger">{error}</Alert>}

      <div className="jobs-hub__list">
        {view === "scheduled" ? (
          <>
            {templates.length === 0 && !loading && (
              <div className="jobs-hub__empty">
                <CalendarClock size={32} aria-hidden="true" />
                <p>No scheduled jobs.</p>
                <p className="jobs-hub__empty-hint">
                  Enable a schedule in the Dump or Export wizard to run a job
                  automatically on a recurring basis.
                </p>
              </div>
            )}
            {templates.map((job) => (
              <JobListItem
                key={job.jobId}
                job={job}
                scheduledRuns={runsFor(job.jobId)}
                onCancel={cancelJob}
                onDelete={deleteJob}
                onRerun={rerunJob}
                onToggleSchedule={toggleSchedule}
                onEditSchedule={setEditingScheduleId}
              />
            ))}
            {loading && templates.length === 0 && (
              <div className="jobs-hub__loading">Loading jobs...</div>
            )}
          </>
        ) : (
          <>
        {filtered.length === 0 && !loading && (
          <div className="jobs-hub__empty">
            <Database size={32} aria-hidden="true" />
            <p>No jobs yet.</p>
            <p className="jobs-hub__empty-hint">
              Dump a database, export a collection, or import data to see jobs here.
            </p>
            {connectionId && (
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button className="btn btn--primary btn--sm" onClick={openDump}>
                  <Download size={14} />
                  Dump database
                </button>
                <button className="btn btn--ghost btn--sm" onClick={openRestore}>
                  <Upload size={14} />
                  Restore database
                </button>
              </div>
            )}
          </div>
        )}
        {filtered.map((job) => (
          <JobListItem
            key={job.jobId}
            job={job}
            onCancel={cancelJob}
            onDelete={deleteJob}
            onRerun={rerunJob}
            onToggleSchedule={toggleSchedule}
            onEditSchedule={setEditingScheduleId}
          />
        ))}
        {loading && filtered.length === 0 && (
          <div className="jobs-hub__loading">Loading jobs...</div>
        )}
          </>
        )}
      </div>

      {wizard === "dump" && connectionId && (
        <DumpWizard connectionId={connectionId} onClose={() => setWizard(null)} />
      )}
      {wizard === "restore" && connectionId && (
        <RestoreWizard connectionId={connectionId} onClose={() => setWizard(null)} />
      )}

      {editingScheduleId && (
        <EditScheduleModal
          open={true}
          jobId={editingScheduleId}
          schedule={jobs.find((j) => j.jobId === editingScheduleId)?.schedule!}
          onClose={() => setEditingScheduleId(null)}
          onSave={saveSchedule}
        />
      )}
    </div>
  );
}
