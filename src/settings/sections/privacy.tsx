import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";

export type HistoryLockStatus = { enabled: boolean; locked: boolean };

// History stays open this long after unlocking; matches privacy.rs.
const UNLOCK_MS = 5 * 60 * 1000;

export function useHistoryLock() {
  const [status, setStatus] = useState<HistoryLockStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<HistoryLockStatus>("get_history_lock")
      .then(setStatus)
      .catch((err) => notifyError("Couldn't read the History lock", err));
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = listen<HistoryLockStatus>("history-lock-changed", (e) => setStatus(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, [refresh]);

  // Lock again when History is left or the window is hidden, and when the
  // unlock runs out.
  useEffect(() => {
    if (!status?.enabled || status.locked) return;
    const relock = () => invoke<HistoryLockStatus>("lock_history").then(setStatus).catch(() => {});
    const onHide = () => document.hidden && relock();
    const timer = setTimeout(refresh, UNLOCK_MS + 500);
    document.addEventListener("visibilitychange", onHide);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onHide);
    };
  }, [status, refresh]);
  useEffect(
    () => () => {
      invoke("lock_history").catch(() => {});
    },
    [],
  );

  // Touch ID or password; the system dialog explains why.
  function run(command: string, args?: Record<string, unknown>) {
    setBusy(true);
    setError(null);
    invoke<HistoryLockStatus>(command, args)
      .then(setStatus)
      .catch((err) => {
        const msg = String(err);
        if (msg !== "Cancelled.") setError(msg);
      })
      .finally(() => setBusy(false));
  }

  return {
    status,
    busy,
    error,
    unlock: () => run("unlock_history"),
    setEnabled: (enabled: boolean) => run("set_history_lock_enabled", { enabled }),
  };
}

type Lock = ReturnType<typeof useHistoryLock>;

export function LockedHistory({ lock }: { lock: Lock }) {
  return (
    <div className="flex flex-col items-center gap-2 rounded-md bg-base-100 px-4 py-8 text-center" data-testid="history-locked">
      <svg aria-hidden="true" viewBox="0 0 24 24" width="22" height="22" className="opacity-60">
        <path
          d="M7 11V8a5 5 0 0 1 10 0v3M6 11h12v9H6z"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
      </svg>
      <p className="text-sm font-medium">History is locked</p>
      <p className="max-w-xs text-xs opacity-60">Confirm it's you with Touch ID or your Mac password to see your transcripts.</p>
      <button className="btn btn-primary btn-sm mt-1" onClick={lock.unlock} disabled={lock.busy}>
        {lock.busy ? <span className="loading loading-spinner loading-xs" /> : "Unlock History"}
      </button>
      {lock.error && <p className="text-xs text-error">{lock.error}</p>}
    </div>
  );
}

export function HistoryPrivacy({ lock }: { lock: Lock }) {
  const [logTranscripts, setLogTranscripts] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<boolean>("get_log_transcripts")
      .then(setLogTranscripts)
      .catch((err) => notifyError("get_log_transcripts", err));
  }, []);

  function toggleLogs(enabled: boolean) {
    setLogTranscripts(enabled);
    invoke("set_log_transcripts", { enabled }).catch((err) => {
      notifyError("set_log_transcripts", err);
      setLogTranscripts(!enabled);
    });
  }

  return (
    <div className="mb-4 rounded-md bg-base-100 px-3 py-2.5" data-testid="history-privacy">
      <p className="mb-1.5 text-xs font-medium opacity-70">Privacy</p>
      <label className="flex items-start gap-1.5 text-xs">
        <input
          type="checkbox"
          className="checkbox checkbox-xs mt-0.5"
          checked={lock.status?.enabled ?? false}
          disabled={lock.status === null || lock.busy}
          onChange={(e) => lock.setEnabled(e.target.checked)}
          data-testid="history-lock-toggle"
        />
        <span>
          Lock History with Touch ID or your password
          <span className="block opacity-60">
            Locks again when you leave History, hide Settings, or after 5 minutes.
          </span>
        </span>
      </label>
      <label className="mt-2 flex items-start gap-1.5 text-xs">
        <input
          type="checkbox"
          className="checkbox checkbox-xs mt-0.5"
          checked={logTranscripts ?? false}
          disabled={logTranscripts === null}
          onChange={(e) => toggleLogs(e.target.checked)}
          data-testid="log-transcripts-toggle"
        />
        <span>
          Include transcript text in logs
          <span className="block opacity-60">
            Off: logs keep timings and events only. Turn on while reporting a dictation bug.
          </span>
        </span>
      </label>
      {lock.error && !lock.status?.locked && <p className="mt-1.5 text-xs text-error">{lock.error}</p>}
    </div>
  );
}
