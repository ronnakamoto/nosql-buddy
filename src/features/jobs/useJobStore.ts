import { useCallback, useEffect, useRef, useState } from "react";
import commands, { formatError, type JobFilterRequest, type JobMeta } from "../../ipc/commands";
import { onJobStatusChanged, onJobLogEntry } from "../../ipc/events";

const POLL_INTERVAL_MS = 15000;

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
  updateSchedule: (jobId: string, config: { cron: string; enabled: boolean; retentionCount?: number | null }) => Promise<void>;
  toggleScheduleEnabled: (jobId: string, enabled: boolean) => Promise<void>;
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
      setJobs((prev) => {
        // Only replace state if the data actually changed.
        // This prevents re-renders on every poll tick when nothing
        // has happened since the last fetch.
        if (prev.length === res.jobs.length) {
          let same = true;
          for (let i = 0; i < prev.length; i++) {
            const a = prev[i];
            const b = res.jobs[i];
            if (
              a.jobId !== b.jobId ||
              a.status !== b.status ||
              a.processed !== b.processed ||
              a.total !== b.total ||
              a.errors !== b.errors ||
              a.message !== b.message ||
              a.finishedAt !== b.finishedAt ||
              a.startedAt !== b.startedAt ||
              a.schedule?.nextRunAt !== b.schedule?.nextRunAt
            ) {
              same = false;
              break;
            }
          }
          if (same) return prev;
        }
        return res.jobs;
      });
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
      setJobs((prev) => {
        const idx = prev.findIndex((j) => j.jobId === payload.jobId);
        if (idx === -1) return prev;
        const j = prev[idx];
        const nextStatus = payload.status as JobMeta["status"];
        if (
          j.status === nextStatus &&
          j.message === payload.message &&
          j.finishedAt === (payload.finishedAt ?? j.finishedAt)
        ) {
          return prev; // no change
        }
        const next = prev.slice();
        next[idx] = {
          ...j,
          status: nextStatus,
          message: payload.message,
          finishedAt: payload.finishedAt ?? j.finishedAt,
        };
        return next;
      });
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

  const updateSchedule = useCallback(async (jobId: string, config: { cron: string; enabled: boolean; retentionCount?: number | null }) => {
    try {
      const meta = await commands.updateSchedule({ jobId, ...config });
      setJobs((prev) => prev.map((j) => (j.jobId === jobId ? meta : j)));
    } catch (e) {
      setError(formatError(e));
      throw e;
    }
  }, []);

  const toggleScheduleEnabled = useCallback(async (jobId: string, enabled: boolean) => {
    const job = jobs.find((j) => j.jobId === jobId);
    if (!job?.schedule) return;
    try {
      const meta = await commands.updateSchedule({
        jobId,
        cron: job.schedule.cron,
        enabled,
        retentionCount: job.schedule.retentionCount,
      });
      setJobs((prev) => prev.map((j) => (j.jobId === jobId ? meta : j)));
    } catch (e) {
      setError(formatError(e));
    }
  }, [jobs]);

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
    updateSchedule,
    toggleScheduleEnabled,
  };
}
