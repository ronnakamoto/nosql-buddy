import { useCallback, useEffect, useRef, useState } from "react";
import commands, { formatError, type JobFilterRequest, type JobMeta } from "../../ipc/commands";
import { onJobStatusChanged, onJobLogEntry } from "../../ipc/events";

const POLL_INTERVAL_MS = 2000;

export interface UseJobStoreReturn {
  jobs: JobMeta[];
  loading: boolean;
  error: string | null;
  fetchJobs: (filter?: JobFilterRequest) => Promise<void>;
  startPolling: (filter?: JobFilterRequest) => void;
  stopPolling: () => void;
  cancelJob: (jobId: string) => Promise<void>;
  deleteJob: (jobId: string) => Promise<void>;
  rerunJob: (jobId: string) => Promise<void>;
}

export function useJobStore(): UseJobStoreReturn {
  const [jobs, setJobs] = useState<JobMeta[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const unlistenRef = useRef<(() => void) | null>(null);
  const unlistenLogRef = useRef<(() => void) | null>(null);

  const fetchJobs = useCallback(async (filter?: JobFilterRequest) => {
    setLoading(true);
    setError(null);
    try {
      const res = await commands.listJobs(filter ?? {});
      setJobs(res.jobs);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const startPolling = useCallback(
    (filter?: JobFilterRequest) => {
      fetchJobs(filter);
      if (timerRef.current) clearInterval(timerRef.current);
      timerRef.current = setInterval(() => fetchJobs(filter), POLL_INTERVAL_MS);
    },
    [fetchJobs],
  );

  const stopPolling = useCallback(() => {
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => stopPolling();
  }, [stopPolling]);

  // Real-time status updates from backend events.
  useEffect(() => {
    let cancelled = false;
    onJobStatusChanged((payload) => {
      if (cancelled) return;
      setJobs((prev) =>
        prev.map((j) =>
          j.jobId === payload.jobId
            ? {
                ...j,
                status: payload.status as JobMeta["status"],
                message: payload.message,
                finishedAt: payload.finishedAt ?? j.finishedAt,
              }
            : j,
        ),
      );
    }).then((unlisten) => {
      if (!cancelled) unlistenRef.current = unlisten;
      else unlisten();
    });

    onJobLogEntry((_payload) => {
      if (cancelled) return;
      // Logs are fetched on-demand by JobLogViewer; no-op here.
      // In future we could maintain a per-job log buffer.
    }).then((unlisten) => {
      if (!cancelled) unlistenLogRef.current = unlisten;
      else unlisten();
    });

    return () => {
      cancelled = true;
      unlistenRef.current?.();
      unlistenLogRef.current?.();
    };
  }, []);

  const cancelJob = useCallback(async (jobId: string) => {
    try {
      await commands.cancelJob(jobId);
      setJobs((prev) =>
        prev.map((j) =>
          j.jobId === jobId
            ? { ...j, status: "cancelled" as const, message: "Cancelling..." }
            : j,
        ),
      );
    } catch (e) {
      setError(formatError(e));
    }
  }, []);

  const deleteJob = useCallback(async (jobId: string) => {
    try {
      await commands.deleteJob(jobId);
      setJobs((prev) => prev.filter((j) => j.jobId !== jobId));
    } catch (e) {
      setError(formatError(e));
    }
  }, []);

  const rerunJob = useCallback(async (jobId: string) => {
    try {
      const meta = await commands.rerunJob(jobId);
      setJobs((prev) => [meta, ...prev]);
    } catch (e) {
      setError(formatError(e));
    }
  }, []);

  return {
    jobs,
    loading,
    error,
    fetchJobs,
    startPolling,
    stopPolling,
    cancelJob,
    deleteJob,
    rerunJob,
  };
}
