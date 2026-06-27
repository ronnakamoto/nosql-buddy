import { useEffect, useState } from "react";
import commands, { formatError, type JobLogEntry } from "../../ipc/commands";

interface JobLogViewerProps {
  jobId: string;
}

export function JobLogViewer({ jobId }: JobLogViewerProps) {
  const [logs, setLogs] = useState<JobLogEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      setLoading(true);
      try {
        const detail = await commands.getJob(jobId);
        if (!cancelled) setLogs(detail.logs);
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    load();
    return () => {
      cancelled = true;
    };
  }, [jobId]);

  if (loading) {
    return <div className="job-log-viewer job-log-viewer--loading">Loading log...</div>;
  }
  if (error) {
    return <div className="job-log-viewer job-log-viewer--error">{error}</div>;
  }
  if (logs.length === 0) {
    return <div className="job-log-viewer job-log-viewer--empty">No log entries.</div>;
  }

  return (
    <div className="job-log-viewer">
      {logs.map((entry, i) => (
        <div key={i} className={`job-log-entry job-log-entry--${entry.level}`}>
          <span className="job-log-entry__ts">
            {new Date(entry.timestamp).toLocaleTimeString()}
          </span>
          <span className="job-log-entry__level">{entry.level}</span>
          <span className="job-log-entry__msg">{entry.message}</span>
        </div>
      ))}
    </div>
  );
}
