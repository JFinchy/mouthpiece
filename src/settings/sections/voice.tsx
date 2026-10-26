import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { type ModelStatus } from "../shared";

export type EnrollmentStatus = {
  enrolled: boolean;
  enrolled_at_ms: number | null;
};

/// Qualitative speed/accuracy positioning for the local Whisper catalog —
/// same tradeoff each model's `label` already states in words (see
/// `models.rs::CATALOG`), just rendered as bars too so it scans at a
/// glance. Not measured benchmark numbers, same as the existing text.
export const MODEL_STATS: Record<string, { speed: number; accuracy: number }> = {
  "tiny.en": { speed: 1, accuracy: 0.55 },
  "base.en": { speed: 0.7, accuracy: 0.75 },
  "small.en": { speed: 0.4, accuracy: 0.95 },
};

export function StatBar({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="w-14 shrink-0 opacity-50">{label}</span>
      <div className="h-1 w-16 overflow-hidden rounded-full bg-base-300">
        <div className="h-full rounded-full bg-primary" style={{ width: `${value * 100}%` }} />
      </div>
    </div>
  );
}

function LiveTranscribeToggle() {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<boolean>("get_live_transcribe_enabled")
      .then(setEnabled)
      .catch((err) => notifyError("get_live_transcribe_enabled", err));
  }, []);

  function toggle(next: boolean) {
    setEnabled(next);
    invoke("set_live_transcribe_enabled", { enabled: next }).catch((err) => {
      notifyError("set_live_transcribe_enabled", err);
      setEnabled(!next);
    });
  }

  if (enabled === null) return null;
  return (
    <div className="mt-3 border-t border-base-content/10 pt-3">
      <label className="flex items-center gap-1.5 text-xs">
        <input
          type="checkbox"
          className="checkbox checkbox-xs"
          checked={enabled}
          onChange={(e) => toggle(e.target.checked)}
        />
        Transcribe while you talk
      </label>
      <p className="mt-1 text-xs opacity-60">
        Transcribes in the background at each pause, so only the last few seconds are left when you stop, and
        long silences are skipped. Turn off to transcribe the whole recording after you stop.
      </p>
    </div>
  );
}

export function ModelsSection() {
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});

  function refresh() {
    invoke<ModelStatus[]>("list_models")
      .then(setModels)
      .catch((err) => notifyError("list_models", err));
  }

  useEffect(() => {
    refresh();

    const unlistenProgress = listen<{ id: string; percent: number }>("model-download-progress", (e) => {
      setProgress((p) => ({ ...p, [e.payload.id]: e.payload.percent }));
    });
    const unlistenDone = listen<string>("model-download-complete", (e) => {
      setProgress((p) => {
        const next = { ...p };
        delete next[e.payload];
        return next;
      });
      refresh();
    });
    const unlistenError = listen<{ id: string; error: string }>("model-download-error", (e) => {
      setProgress((p) => {
        const next = { ...p };
        delete next[e.payload.id];
        return next;
      });
      setErrors((er) => ({ ...er, [e.payload.id]: e.payload.error }));
    });

    return () => {
      unlistenProgress.then((f) => f());
      unlistenDone.then((f) => f());
      unlistenError.then((f) => f());
    };
  }, []);

  function download(id: string) {
    setErrors((er) => {
      const next = { ...er };
      delete next[id];
      return next;
    });
    invoke("download_model", { id }).catch((err) => setErrors((er) => ({ ...er, [id]: String(err) })));
  }

  function activate(id: string) {
    invoke("set_active_model", { id })
      .then(refresh)
      .catch((err) => setErrors((er) => ({ ...er, [id]: String(err) })));
  }

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Whisper model</h3>
      <ul className="flex flex-col gap-2">
        {models.map((m) => {
          const stats = MODEL_STATS[m.id];
          return (
            <li key={m.id} className="rounded-md bg-base-200 px-3 py-2 text-xs">
              <div className="flex items-center justify-between">
                <div className="font-medium">{m.label}</div>
                {m.active ? (
                  <span className="badge badge-success badge-sm">Active</span>
                ) : m.id in progress ? (
                  <span className="w-10 text-right opacity-70">{progress[m.id]}%</span>
                ) : m.downloaded ? (
                  <button className="btn btn-xs" onClick={() => activate(m.id)}>
                    Use
                  </button>
                ) : (
                  <button className="btn btn-xs" onClick={() => download(m.id)}>
                    Download
                  </button>
                )}
              </div>
              <div className="mt-1.5 flex items-center gap-4">
                <span className="opacity-50">{m.size_mb}MB</span>
                {stats && (
                  <>
                    <StatBar label="Speed" value={stats.speed} />
                    <StatBar label="Accuracy" value={stats.accuracy} />
                  </>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      {Object.entries(errors).map(([id, err]) => (
        <p key={id} className="mt-1.5 text-xs text-error">
          {err}
        </p>
      ))}
      <LiveTranscribeToggle />
    </div>
  );
}

export type AmbientPhase = "off" | "listening" | "asleep" | "paused";

export const AMBIENT_PHASE_LABEL: Record<AmbientPhase, string> = {
  off: "Off",
  listening: "Listening for “Mouthpiece”",
  asleep: "Asleep — say “Mouthpiece, wake up”",
  paused: "Paused — another app is using the microphone",
};

export function AmbientListeningControl({ enrolled }: { enrolled: boolean }) {
  const [enabled, setEnabled] = useState(false);
  const [phase, setPhase] = useState<AmbientPhase>("off");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<boolean>("get_ambient_enabled")
      .then(setEnabled)
      .catch((err) => notifyError("get_ambient_enabled", err));
    invoke<AmbientPhase>("get_ambient_phase")
      .then(setPhase)
      .catch((err) => notifyError("get_ambient_phase", err));
    const unlisten = listen<AmbientPhase>("ambient-phase", (e) => setPhase(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  function toggle(next: boolean) {
    setError(null);
    setEnabled(next);
    invoke("set_ambient_enabled", { enabled: next }).catch((err) => {
      setEnabled(!next);
      setError(String(err));
    });
  }

  return (
    <div className="flex flex-col gap-1 rounded-md bg-base-100 px-2.5 py-2 text-xs">
      <label className={`flex items-center gap-1.5 ${enrolled ? "" : "opacity-50"}`}>
        <input
          type="checkbox"
          className="checkbox checkbox-xs"
          checked={enabled}
          disabled={!enrolled && !enabled}
          onChange={(e) => toggle(e.target.checked)}
        />
        Always-on listening — start with “Mouthpiece, …” to dictate hands-free
      </label>
      {enabled && <div className="font-medium">{AMBIENT_PHASE_LABEL[phase]}</div>}
      <p className="opacity-60">
        {enrolled
          ? "Only your enrolled voice is acted on, and only when it starts with the wake phrase. Say “Mouthpiece, go to sleep” to pause; it also sleeps after 10 idle minutes, and pauses itself while another app (like a call) is using the microphone. Text is copied instead of pasted if you switch apps before it's ready."
          : "Enroll your voice above first — always-on listening only acts on your voice."}
      </p>
      {error && <p className="text-error">{error}</p>}
    </div>
  );
}

export function VoiceIsolationSection() {
  const [enabled, setEnabled] = useState(false);
  const [checked, setChecked] = useState(false);
  const [status, setStatus] = useState<EnrollmentStatus | null>(null);
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function refreshStatus() {
    invoke<EnrollmentStatus>("get_voice_enrollment_status")
      .then(setStatus)
      .catch((err) => notifyError("get_voice_enrollment_status", err));
  }

  useEffect(() => {
    invoke<boolean>("get_isolated_voice_enabled")
      .then(setEnabled)
      .catch((err) => notifyError("get_isolated_voice_enabled", err))
      .finally(() => setChecked(true));
    refreshStatus();

    const unlistenStarted = listen("enrollment-started", () => setRecording(true));
    const unlistenDone = listen("enrollment-complete", () => {
      setRecording(false);
      setError(null);
      refreshStatus();
    });
    const unlistenError = listen<string>("enrollment-error", (e) => {
      setRecording(false);
      setError(e.payload);
    });

    return () => {
      unlistenStarted.then((f) => f());
      unlistenDone.then((f) => f());
      unlistenError.then((f) => f());
    };
  }, []);

  function toggleEnabled(next: boolean) {
    setEnabled(next);
    invoke("set_isolated_voice_enabled", { enabled: next }).catch((err) => {
      notifyError("set_isolated_voice_enabled", err);
      setEnabled(!next);
    });
  }

  function startEnrollment() {
    setError(null);
    invoke("start_voice_enrollment")
      .then(() => setRecording(true))
      .catch((err) => setError(String(err)));
  }

  function stopEnrollment() {
    invoke("stop_voice_enrollment").catch((err) => {
      notifyError("stop_voice_enrollment", err);
      setRecording(false);
    });
  }

  return (
    <div className="mb-4 border-t border-base-content/10 pt-3">
      <label className="mb-1 block text-xs font-medium opacity-70">Voice Isolation</label>
      {!checked ? (
        <span className="loading loading-spinner loading-xs" />
      ) : (
        <div className="flex flex-col gap-2">
          <label className="flex items-center gap-1.5 text-xs">
            <input
              type="checkbox"
              className="checkbox checkbox-xs"
              checked={enabled}
              onChange={(e) => toggleEnabled(e.target.checked)}
            />
            Isolated Voice — filter out other speakers/background noise before transcribing
          </label>

          <div className="flex items-center justify-between rounded-md bg-base-100 px-2.5 py-1.5 text-xs">
            <div>
              <div className="font-medium">{status?.enrolled ? "Voice enrolled" : "Not enrolled"}</div>
              {status?.enrolled && status.enrolled_at_ms && (
                <div className="opacity-50">
                  {new Date(status.enrolled_at_ms).toLocaleDateString(undefined, {
                    month: "short",
                    day: "numeric",
                    year: "numeric",
                  })}
                </div>
              )}
            </div>
            {recording ? (
              <button className="btn btn-xs btn-error" onClick={stopEnrollment}>
                Stop
              </button>
            ) : (
              <button className="btn btn-xs" onClick={startEnrollment}>
                {status?.enrolled ? "Re-enroll" : "Enroll voice"}
              </button>
            )}
          </div>

          {recording && (
            <p className="text-xs opacity-60">
              Recording — speak naturally for a few seconds, then press Stop.
            </p>
          )}

          <p className="text-xs opacity-60">
            {status?.enrolled
              ? "Enrolled: recordings are matched against your enrolled voice, rejecting other speakers."
              : "Not enrolled: this only suppresses quiet background noise, not a second person talking at similar volume. Enroll your voice above for stronger speaker-based isolation."}
          </p>

          {error && <p className="text-xs text-error">{error}</p>}

          <AmbientListeningControl enrolled={Boolean(status?.enrolled)} />
        </div>
      )}
    </div>
  );
}
