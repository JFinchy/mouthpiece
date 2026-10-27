import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { LockedHistory, useHistoryLock } from "./privacy";

export type MeetingProgress = { id: string; step: string; percent: number | null };
export type MeetingStatus = {
  recording: { id: string; started_ms: number } | null;
  processing: MeetingProgress | null;
  queued: string[];
};
export type MeetingSummary = {
  id: string;
  title: string;
  detail: string | null;
  state: "ready" | "not_transcribed" | "recording" | "processing";
};

const SPEAKER_OPTIONS = [0, 2, 3, 4, 5, 6, 7, 8];

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(h ? 2 : 1, "0");
  return `${h ? `${h}:` : ""}${mm}:${String(s).padStart(2, "0")}`;
}

export function useMeetingStatus() {
  const [status, setStatus] = useState<MeetingStatus | null>(null);
  useEffect(() => {
    invoke<MeetingStatus>("get_meeting_status")
      .then(setStatus)
      .catch((err) => console.error("get_meeting_status failed:", err));
    const unlisten = listen<MeetingStatus>("meeting-changed", (e) => setStatus(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);
  return status;
}

function RecordControl({ status }: { status: MeetingStatus | null }) {
  const [now, setNow] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  const recording = status?.recording ?? null;

  useEffect(() => {
    if (!recording) return;
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [recording]);

  function toggle() {
    setBusy(true);
    invoke(recording ? "stop_meeting" : "start_meeting")
      .catch((err) => notifyError(recording ? "Couldn't stop the meeting" : "Couldn't start a meeting", err))
      .finally(() => setBusy(false));
  }

  const processing = status?.processing ?? null;
  return (
    <div className="mb-4 rounded-md bg-base-100 px-3 py-3" data-testid="meeting-control">
      <div className="flex items-center gap-3">
        <button
          className={`btn btn-sm ${recording ? "btn-error" : "btn-primary"}`}
          onClick={toggle}
          disabled={busy || status === null}
          data-testid="meeting-toggle"
        >
          {recording ? "Stop meeting" : "Start meeting"}
        </button>
        {recording ? (
          <span className="flex items-center gap-1.5 text-sm tabular-nums" data-testid="meeting-elapsed">
            <span className="h-2 w-2 animate-pulse rounded-full bg-error motion-reduce:animate-none" />
            Recording · {formatDuration(now - recording.started_ms)}
          </span>
        ) : processing ? (
          <span className="flex min-w-0 flex-1 flex-col gap-1 text-xs" data-testid="meeting-processing">
            <span className="opacity-70">
              {processing.step}
              {processing.percent !== null ? ` · ${processing.percent}%` : "…"}
            </span>
            <progress
              className="progress progress-primary h-1 w-full"
              value={processing.percent ?? undefined}
              max={100}
            />
          </span>
        ) : (
          <span className="text-xs opacity-60">Also in the menu bar icon.</span>
        )}
      </div>
      <p className="mt-2.5 text-xs opacity-70">
        Records everything your mic hears until you stop, then writes a transcript split by speaker. Audio and
        transcript stay on this Mac. Dictation is paused while a meeting records.
      </p>
      <p className="mt-1.5 text-xs opacity-60">
        Works for meetings in the room and calls on your speakers. On headphones, only your side is heard. Let people
        know you're recording.
      </p>
    </div>
  );
}

function SpeakerCount() {
  const [speakers, setSpeakers] = useState<number | null>(null);
  useEffect(() => {
    invoke<number>("get_meeting_speakers")
      .then(setSpeakers)
      .catch((err) => notifyError("Couldn't read the meeting settings", err));
  }, []);

  function change(value: number) {
    const before = speakers;
    setSpeakers(value);
    invoke("set_meeting_speakers", { speakers: value }).catch((err) => {
      setSpeakers(before);
      notifyError("Couldn't save the number of people", err);
    });
  }

  return (
    <div className="mb-4 flex items-start justify-between gap-3">
      <span className="text-xs">
        People in the meeting
        <span className="block opacity-60">
          Setting the number gives the best split, especially when voices sound alike. Enroll your voice (Voice page)
          and you're labelled "You".
        </span>
      </span>
      <select
        className="select select-xs"
        value={speakers ?? 0}
        disabled={speakers === null}
        onChange={(e) => change(Number(e.target.value))}
        aria-label="People in the meeting"
        data-testid="meeting-speakers"
      >
        {SPEAKER_OPTIONS.map((n) => (
          <option key={n} value={n}>
            {n === 0 ? "Work it out" : n}
          </option>
        ))}
      </select>
    </div>
  );
}

function MeetingList({ status }: { status: MeetingStatus | null }) {
  const [meetings, setMeetings] = useState<MeetingSummary[] | null>(null);
  const [open, setOpen] = useState<Record<string, string>>({});
  const [copied, setCopied] = useState<string | null>(null);
  // Delete asks once more (the transcript is gone for good).
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  useEffect(() => {
    if (!confirmDelete) return;
    const timer = setTimeout(() => setConfirmDelete(null), 4000);
    return () => clearTimeout(timer);
  }, [confirmDelete]);

  function refresh() {
    invoke<MeetingSummary[]>("list_meetings")
      .then(setMeetings)
      .catch((err) => notifyError("Couldn't load meetings", err));
  }
  // The list changes when a meeting starts, stops, joins or leaves the
  // queue; not on every progress tick.
  const listKey = `${status?.recording?.id ?? ""}|${status?.queued.join(",") ?? ""}`;
  useEffect(refresh, [listKey]);

  function toggleView(id: string) {
    if (open[id] !== undefined) {
      setOpen(({ [id]: _, ...rest }) => rest);
      return;
    }
    invoke<string>("read_meeting", { id })
      .then((text) => setOpen((current) => ({ ...current, [id]: text })))
      .catch((err) => notifyError("Couldn't read the transcript", err));
  }

  function copy(id: string) {
    invoke<string>("read_meeting", { id })
      .then((text) => navigator.clipboard.writeText(text))
      .then(() => {
        setCopied(id);
        setTimeout(() => setCopied((c) => (c === id ? null : c)), 1500);
      })
      .catch((err) => notifyError("Couldn't copy the transcript", err));
  }

  function run(command: string, id: string, what: string) {
    invoke(command, { id })
      .then(refresh)
      .catch((err) => notifyError(`Couldn't ${what}`, err));
  }

  if (meetings === null) return <span className="loading loading-spinner loading-xs" />;
  if (meetings.length === 0) return <p className="text-xs opacity-60">No meetings yet.</p>;
  return (
    <ul className="flex flex-col gap-1 rounded-md bg-base-100 p-1.5" data-testid="meeting-list">
      {meetings.map((m) => (
        <li key={m.id} className="group rounded px-1.5 py-1.5 text-xs hover:bg-base-200" data-testid="meeting-row">
          <div className="flex items-center justify-between gap-2">
            <span className="min-w-0">
              <span className="block truncate font-medium">{m.title}</span>
              <span className="block opacity-60">
                {m.state === "recording"
                  ? "Recording…"
                  : m.state === "processing"
                    ? status?.processing?.id === m.id
                      ? "Transcribing…"
                      : "Waiting to be transcribed"
                    : m.state === "not_transcribed"
                      ? "Not transcribed yet"
                      : m.detail}
              </span>
            </span>
            <span className="flex shrink-0 items-center gap-1">
              {m.state === "ready" && (
                <>
                  <button className="btn btn-ghost btn-xs" onClick={() => toggleView(m.id)} aria-expanded={open[m.id] !== undefined}>
                    {open[m.id] !== undefined ? "Hide" : "View"}
                  </button>
                  <button className="btn btn-ghost btn-xs" onClick={() => copy(m.id)}>
                    {copied === m.id ? "Copied" : "Copy"}
                  </button>
                  <button className="btn btn-ghost btn-xs" onClick={() => run("open_meeting", m.id, "open the transcript")}>
                    Open
                  </button>
                </>
              )}
              {m.state === "not_transcribed" && (
                <button
                  className="btn btn-ghost btn-xs"
                  onClick={() => run("transcribe_meeting", m.id, "transcribe the meeting")}
                >
                  Transcribe
                </button>
              )}
              {(m.state === "ready" || m.state === "not_transcribed") && (
                <button
                  className={`btn btn-ghost btn-xs hover:text-error focus:opacity-100 ${
                    confirmDelete === m.id ? "text-error opacity-100" : "opacity-0 group-hover:opacity-100"
                  }`}
                  onClick={() => {
                    if (confirmDelete === m.id) {
                      setConfirmDelete(null);
                      run("delete_meeting", m.id, "delete the meeting");
                    } else {
                      setConfirmDelete(m.id);
                    }
                  }}
                  aria-label={confirmDelete === m.id ? `Confirm deleting ${m.title}` : `Delete ${m.title}`}
                >
                  {confirmDelete === m.id ? "Delete for good?" : "Delete"}
                </button>
              )}
            </span>
          </div>
          {open[m.id] !== undefined && (
            <pre className="dw-scroll mt-1.5 max-h-64 overflow-y-auto whitespace-pre-wrap rounded bg-base-200 p-2 font-sans text-xs leading-relaxed">
              {open[m.id]}
            </pre>
          )}
        </li>
      ))}
    </ul>
  );
}

export function MeetingsSection() {
  const status = useMeetingStatus();
  const lock = useHistoryLock();
  return (
    <>
      <RecordControl status={status} />
      <SpeakerCount />
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide opacity-60">Transcripts</h3>
      {lock.status === null ? (
        <span className="loading loading-spinner loading-xs" />
      ) : lock.status.locked ? (
        <LockedHistory lock={lock} />
      ) : (
        <MeetingList status={status} />
      )}
    </>
  );
}
