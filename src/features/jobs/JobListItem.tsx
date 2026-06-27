import { useState } from "react";
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
} from "lucide-react";
import type { JobMeta } from "../../ipc/commands";
import { JobLogViewer } from "./JobLogViewer";

interface JobListItemProps {
  job: JobMeta;
  onCancel: (jobId: string) => void;
  onDelete: (jobId: string) => void;
  onRerun: (jobId: string) => void;
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

export function JobListItem({ job, onCancel, onDelete, onRerun }: JobListItemProps) {
  const [expanded, setExpanded] = useState(false);
  const progress = job.total && job.total > 0 ? (job.processed / job.total) * 100 : 0;
  const isActive = job.status === "running" || job.status === "queued";

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
          </div>
          {job.message && <div className="job-list-item__message">{job.message}</div>}
          <JobLogViewer jobId={job.jobId} />
        </div>
      )}
    </div>
  );
}
