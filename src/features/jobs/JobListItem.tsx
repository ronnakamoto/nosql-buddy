import { memo, useState } from "react";
import {
  Download,
  Upload,
  FileDown,
  FileUp,
  X,
  RotateCcw,
  Trash2,
  ChevronDown,
  ChevronRight,
  CalendarClock,
  Pause,
  Play,
  Pencil,
} from "lucide-react";
import type { JobMeta } from "../../ipc/commands";
import { JobLogViewer } from "./JobLogViewer";

interface JobListItemProps {
  job: JobMeta;
  onCancel: (jobId: string) => void;
  onDelete: (jobId: string) => void;
  onRerun: (jobId: string) => void;
  onToggleSchedule?: (jobId: string, enabled: boolean) => void;
  onEditSchedule?: (jobId: string) => void;
  /** Generated runs spawned by this schedule template (Scheduled view only). */
  scheduledRuns?: JobMeta[];
}

function kindIcon(kind: JobMeta["kind"]) {
  switch (kind) {
    case "dump":
      return <Download size={14} />;
    case "restore":
      return <Upload size={14} />;
    case "export":
      return <FileDown size={14} />;
    case "import":
      return <FileUp size={14} />;
  }
}

function kindLabel(kind: JobMeta["kind"]): string {
  switch (kind) {
    case "dump":
      return "Dump";
    case "restore":
      return "Restore";
    case "export":
      return "Export";
    case "import":
      return "Import";
  }
}

function formatRelative(date: Date): string {
  const now = new Date();
  const diff = date.getTime() - now.getTime();
  const seconds = Math.round(diff / 1000);
  const minutes = Math.round(seconds / 60);
  const hours = Math.round(minutes / 60);
  const days = Math.round(hours / 24);

  if (Math.abs(seconds) < 60) return seconds > 0 ? "in moments" : "just now";
  if (Math.abs(minutes) < 60) return minutes > 0 ? `in ${minutes}m` : `${Math.abs(minutes)}m ago`;
  if (Math.abs(hours) < 24) return hours > 0 ? `in ${hours}h` : `${Math.abs(hours)}h ago`;
  if (Math.abs(days) < 7) return days > 0 ? `in ${days}d` : `${Math.abs(days)}d ago`;
  return date.toLocaleDateString();
}

function statusClass(status: JobMeta["status"]): string {
  switch (status) {
    case "queued":
      return "job-status--queued";
    case "running":
      return "job-status--running";
    case "done":
      return "job-status--done";
    case "failed":
      return "job-status--failed";
    case "cancelled":
      return "job-status--cancelled";
  }
}

export const JobListItem = memo(function JobListItem({
  job,
  onCancel,
  onDelete,
  onRerun,
  onToggleSchedule,
  onEditSchedule,
  scheduledRuns,
}: JobListItemProps) {
  const [expanded, setExpanded] = useState(false);
  const progress = job.total && job.total > 0 ? (job.processed / job.total) * 100 : 0;
  const isActive = job.status === "running" || job.status === "queued";
  const hasSchedule = job.schedule != null;

  return (
    <div className="job-list-item">
      <div className="job-list-item__row" onClick={() => setExpanded((e) => !e)}>
        <span className="job-list-item__icon" aria-hidden="true">
          {kindIcon(job.kind)}
        </span>
        <span className="job-list-item__kind">{kindLabel(job.kind)}</span>
        <span className="job-list-item__target">
          {job.connectionId}/{job.database}
          {job.collections.length > 0 ? ` (${job.collections.length})` : ""}
        </span>
        <span className={`job-list-item__status ${statusClass(job.status)}`}>
          {job.status}
        </span>
        <span className="job-list-item__next-run">
          {job.schedule?.enabled && job.schedule.nextRunAt ? (
            <span title={`Next run: ${new Date(job.schedule.nextRunAt).toLocaleString()}`}>
              <CalendarClock size={12} aria-hidden="true" />
              {formatRelative(new Date(job.schedule.nextRunAt))}
            </span>
          ) : (
            "—"
          )}
        </span>
        <span className="job-list-item__progress-cell">
          <span className="job-progress-track">
            <span
              className="job-progress-fill"
              style={{ width: `${Math.min(progress, 100)}%` }}
            />
          </span>
          <span className="job-progress-text">
            {job.processed.toLocaleString()}
            {job.total ? ` / ${job.total.toLocaleString()}` : ""}
          </span>
        </span>
        <span className="job-list-item__actions">
          {isActive && (
            <button
              className="btn btn--sm btn--ghost"
              onClick={(e) => {
                e.stopPropagation();
                onCancel(job.jobId);
              }}
              title="Cancel job"
            >
              <X size={14} />
            </button>
          )}
          {!isActive && (
            <>
              <button
                className="btn btn--sm btn--ghost"
                onClick={(e) => {
                  e.stopPropagation();
                  onRerun(job.jobId);
                }}
                title="Rerun job"
              >
                <RotateCcw size={14} />
              </button>
              <button
                className="btn btn--sm btn--ghost"
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(job.jobId);
                }}
                title="Delete job"
              >
                <Trash2 size={14} />
              </button>
            </>
          )}
          <button
            className="btn btn--sm btn--ghost"
            onClick={(e) => {
              e.stopPropagation();
              setExpanded((v) => !v);
            }}
            title={expanded ? "Collapse" : "Expand"}
          >
            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </button>
        </span>
      </div>
      {expanded && (
        <div className="job-list-item__detail">
          <div className="job-list-item__meta">
            <span>
              Job ID: <code className="job-meta-code">{job.jobId}</code>
            </span>
            <span>Created: {new Date(job.createdAt).toLocaleString()}</span>
            {job.startedAt && <span>Started: {new Date(job.startedAt).toLocaleString()}</span>}
            {job.finishedAt && <span>Finished: {new Date(job.finishedAt).toLocaleString()}</span>}
            {job.outputPath && (
              <span>
                Output: <code className="job-meta-code">{job.outputPath}</code>
              </span>
            )}
            {job.sourcePath && (
              <span>
                Source: <code className="job-meta-code">{job.sourcePath}</code>
              </span>
            )}
            {job.errors > 0 && (
              <span className="job-meta--errors">{job.errors.toLocaleString()} error(s)</span>
            )}
            {job.schedule && (
              <span className="job-meta--schedule">
                Schedule: {job.schedule.enabled ? "enabled" : "disabled"}
                {job.schedule.enabled && job.schedule.nextRunAt && (
                  <> — next run {formatRelative(new Date(job.schedule.nextRunAt))}</>
                )}
                {job.schedule.retentionCount != null && (
                  <> — keep last {job.schedule.retentionCount} backups</>
                )}
              </span>
            )}
          </div>

          {hasSchedule && onToggleSchedule && onEditSchedule && (
            <div className="job-list-item__schedule-actions">
              <button
                className="btn btn--sm btn--ghost"
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleSchedule(job.jobId, !job.schedule!.enabled);
                }}
                title={job.schedule!.enabled ? "Pause schedule" : "Resume schedule"}
              >
                {job.schedule!.enabled ? <Pause size={14} /> : <Play size={14} />}
                {job.schedule!.enabled ? "Pause" : "Resume"}
              </button>
              <button
                className="btn btn--sm btn--ghost"
                onClick={(e) => {
                  e.stopPropagation();
                  onEditSchedule(job.jobId);
                }}
                title="Edit schedule"
              >
                <Pencil size={14} />
                Edit
              </button>
            </div>
          )}

          {scheduledRuns && (
            <div className="job-runs">
              <div className="job-runs__title">
                Recent runs ({scheduledRuns.length})
              </div>
              {scheduledRuns.length === 0 ? (
                <div className="job-runs__empty">
                  No runs yet. The next run is{" "}
                  {job.schedule?.enabled && job.schedule.nextRunAt
                    ? formatRelative(new Date(job.schedule.nextRunAt))
                    : "not scheduled"}
                  .
                </div>
              ) : (
                <ul className="job-runs__list">
                  {scheduledRuns
                    .slice()
                    .sort((a, b) => b.createdAt.localeCompare(a.createdAt))
                    .map((run) => (
                      <li key={run.jobId} className="job-runs__item">
                        <span className={`job-runs__status ${statusClass(run.status)}`}>
                          {run.status}
                        </span>
                        <span className="job-runs__time">
                          {new Date(run.finishedAt ?? run.createdAt).toLocaleString()}
                        </span>
                        <span className="job-runs__count">
                          {run.processed.toLocaleString()} docs
                          {run.errors > 0 ? `, ${run.errors} error(s)` : ""}
                        </span>
                        {!(run.status === "running" || run.status === "queued") && (
                          <button
                            className="btn btn--sm btn--ghost"
                            onClick={(e) => {
                              e.stopPropagation();
                              onDelete(run.jobId);
                            }}
                            title="Delete this run"
                            aria-label="Delete this run"
                          >
                            <Trash2 size={12} />
                          </button>
                        )}
                      </li>
                    ))}
                </ul>
              )}
            </div>
          )}

          {job.message && <div className="job-list-item__message">{job.message}</div>}
          <JobLogViewer jobId={job.jobId} />
        </div>
      )}
    </div>
  );
});
