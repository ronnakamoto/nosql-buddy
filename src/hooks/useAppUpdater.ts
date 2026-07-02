import { useCallback, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "installing"
  | "error";

export interface UseAppUpdaterResult {
  status: UpdateStatus;
  /** Version string of the pending update, if any. */
  latestVersion: string | null;
  /** Error message, if the last check/install failed. */
  error: string | null;
  /** Check the update endpoint for a newer release. */
  checkForUpdates: () => Promise<void>;
  /** Download and install the previously detected update, then relaunch. */
  installUpdate: () => Promise<void>;
}

/**
 * Thin wrapper around `@tauri-apps/plugin-updater` for the in-app
 * "Check for updates" flow surfaced from the About screen.
 */
export function useAppUpdater(): UseAppUpdaterResult {
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [latestVersion, setLatestVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<Update | null>(null);

  const checkForUpdates = useCallback(async () => {
    setStatus("checking");
    setError(null);
    try {
      const update = await check();
      if (update) {
        setPending(update);
        setLatestVersion(update.version);
        setStatus("available");
      } else {
        setPending(null);
        setLatestVersion(null);
        setStatus("up-to-date");
      }
    } catch (err) {
      setStatus("error");
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const installUpdate = useCallback(async () => {
    if (!pending) return;
    setStatus("downloading");
    setError(null);
    try {
      await pending.downloadAndInstall();
      setStatus("installing");
      await relaunch();
    } catch (err) {
      setStatus("error");
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [pending]);

  return { status, latestVersion, error, checkForUpdates, installUpdate };
}
