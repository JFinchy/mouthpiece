import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { LockedHistory, useHistoryLock } from "./privacy";

export type MeetingProgress = { id: string; step: string; percent: number | null };
export type MeetingStatus = {
  // `limit_ms`: the meeting stops itself this long after it started.
  recording: { id: string; started_ms: number; limit_ms?: number } | null;
  processing: MeetingProgress | null;
  queued: string[];
};
export type MeetingSummary = {
  id: string;
  title: string;
  detail: string | null;
  state: "ready" | "not_transcribed" | "recording" | "processing";
};
export type DetectedCall = {
  platform: string;
  app_name: string;
  bundle_id: string;
  pid: number;
};

const SPEAKER_OPTIONS = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12];

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
        Records your mic and system audio until you stop, then writes a transcript split by speaker. Audio and
        transcript stay on this Mac. Push-to-talk dictation still works while a meeting records.
      </p>
      <p className="mt-1.5 text-xs opacity-60">
        Works on headphones, calls on your speakers, and meetings in the room. Let people know you're recording.
      </p>
    </div>
  );
}

function AutoDetectToggle() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  useEffect(() => {
    invoke<boolean>("get_meeting_auto_detect")
      .then(setEnabled)
      .catch((err) => notifyError("Couldn't read the meeting settings", err));
  }, []);

  function toggle(next: boolean) {
    const before = enabled;
    setEnabled(next);
    invoke("set_meeting_auto_detect", { enabled: next }).catch((err) => {
      setEnabled(before);
      notifyError("Couldn't save meeting auto-detection", err);
    });
  }

  return (
    <label className="mb-4 flex cursor-pointer items-start justify-between gap-3">
      <span className="text-xs">
        Offer to record when a call starts
        <span className="block opacity-60">
          Notices active Zoom, Google Meet, Microsoft Teams, Slack Huddle, Webex, FaceTime, and Discord calls locally
          and prompts in the widget when a call starts or ends.
        </span>
      </span>
      <input
        type="checkbox"
        className="toggle toggle-xs mt-0.5"
        checked={enabled ?? true}
        disabled={enabled === null}
        onChange={(e) => toggle(e.target.checked)}
        aria-label="Offer to record when a call starts"
        data-testid="meeting-auto-detect"
      />
    </label>
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

type ParsedTurn = {
  speaker: string;
  timestamp: string;
  text: string;
};

type ParsedMeetingDoc = {
  title: string;
  detail: string;
  notes: string | null;
  turns: ParsedTurn[];
  speakers: string[];
};

function parseTurnHeader(trimmed: string): { speaker: string; timestamp: string } | null {
  if (!trimmed.startsWith("**")) return null;
  const rest = trimmed.slice(2);
  const sep = rest.lastIndexOf("** · ");
  if (sep <= 0) return null;
  const speaker = rest.slice(0, sep).trim();
  const timestamp = rest.slice(sep + 5).trim();
  // Same rule as the Rust parser (parse_turn_header): any number of
  // leading digits, so a 100+ minute meeting's turns still parse.
  if (!speaker || !timestamp || !/^\d+:\d{2}(?::\d{2})?$/.test(timestamp)) return null;
  return { speaker, timestamp };
}

function parseMeetingMarkdown(markdown: string): ParsedMeetingDoc {
  let title = "Meeting";
  let detail = "";
  const notesLines: string[] = [];
  const turns: ParsedTurn[] = [];
  let currentTurn: ParsedTurn | null = null;
  let sawTitle = false;
  let sawDetail = false;

  // CRLF too, like Rust's `lines()`, in case the file was edited elsewhere.
  for (const rawLine of markdown.split(/\r?\n/)) {
    const trimmed = rawLine.trim();
    if (!sawTitle) {
      if (!trimmed) continue;
      if (trimmed.startsWith("# ")) {
        title = trimmed.slice(2).trim();
        sawTitle = true;
        continue;
      }
      sawTitle = true;
    }
    if (!sawDetail && !currentTurn) {
      if (!trimmed) continue;
      if (!trimmed.startsWith("## ") && !parseTurnHeader(trimmed)) {
        detail = trimmed;
        sawDetail = true;
        continue;
      }
      sawDetail = true;
    }

    const header = parseTurnHeader(trimmed);
    if (header) {
      if (currentTurn && currentTurn.text.trim()) {
        turns.push({ ...currentTurn, text: currentTurn.text.trim() });
      }
      currentTurn = { speaker: header.speaker, timestamp: header.timestamp, text: "" };
      continue;
    }

    if (currentTurn) {
      currentTurn.text += (currentTurn.text ? "\n" : "") + rawLine;
    } else if (trimmed === "## Notes") {
      // Notes heading; subsequent lines before the first turn are captured in notesLines.
    } else if (sawDetail) {
      notesLines.push(rawLine);
    }
  }

  if (currentTurn && currentTurn.text.trim()) {
    turns.push({ ...currentTurn, text: currentTurn.text.trim() });
  }

  const speakers: string[] = [];
  for (const t of turns) {
    if (!speakers.includes(t.speaker)) {
      speakers.push(t.speaker);
    }
  }

  const notesJoined = notesLines.join("\n").trim();
  return {
    title,
    detail,
    notes: notesJoined.length > 0 ? notesJoined : null,
    turns,
    speakers,
  };
}

function renderInlineBold(line: string) {
  const parts = line.split(/(\*\*[^*]+\*\*)/g);
  return parts.map((part, idx) =>
    part.startsWith("**") && part.endsWith("**") && part.length > 4 ? (
      <strong key={idx} className="font-semibold text-base-content">
        {part.slice(2, -2)}
      </strong>
    ) : (
      <span key={idx}>{part}</span>
    ),
  );
}

const SPEAKER_PALETTE = [
  "#06b6d4", // cyan
  "#a855f7", // violet
  "#10b981", // emerald
  "#f43f5e", // rose
  "#38bdf8", // sky
];

function buildSpeakerColors(speakers: string[]): Record<string, string> {
  const colors: Record<string, string> = {};
  let paletteIdx = 0;
  for (const speaker of speakers) {
    if (speaker === "You") {
      colors[speaker] = "var(--dw-accent)";
    } else {
      colors[speaker] = SPEAKER_PALETTE[paletteIdx % SPEAKER_PALETTE.length];
      paletteIdx += 1;
    }
  }
  return colors;
}

function countTurnWords(text: string): number {
  const words = text.trim().split(/\s+/).filter(Boolean).length;
  return Math.max(1, words);
}

function MeetingNotesBody({
  id,
  markdown,
  summarizing,
  onSummarize,
  onUpdated,
}: {
  id: string;
  markdown: string;
  // Owned by MeetingList so collapsing and re-opening the row can't start
  // a second summary while one is still running.
  summarizing: boolean;
  onSummarize: () => void;
  onUpdated: (nextMarkdown: string) => void;
}) {
  const doc = parseMeetingMarkdown(markdown);
  const [renamingSpeaker, setRenamingSpeaker] = useState<string | null>(null);
  const [speakerDraft, setSpeakerDraft] = useState("");
  const [savingSpeaker, setSavingSpeaker] = useState(false);
  // Renaming onto a name already in the transcript merges the two
  // speakers for good, so the first Save only asks.
  const [confirmMerge, setConfirmMerge] = useState<string | null>(null);
  const [highlightedTurn, setHighlightedTurn] = useState<number | null>(null);
  const [hoveredSpeaker, setHoveredSpeaker] = useState<string | null>(null);
  const turnRefs = useRef<(HTMLDivElement | null)[]>([]);
  const highlightTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (highlightTimerRef.current !== null) {
        window.clearTimeout(highlightTimerRef.current);
      }
    };
  }, []);

  const speakerColors = buildSpeakerColors(doc.speakers);
  const turnWords = doc.turns.map((t) => countTurnWords(t.text));
  const totalWords = turnWords.reduce((sum, w) => sum + w, 0);
  const speakerShares = doc.speakers.map((speaker) => {
    let words = 0;
    let firstTurnIdx = -1;
    doc.turns.forEach((turn, idx) => {
      if (turn.speaker === speaker) {
        words += turnWords[idx];
        if (firstTurnIdx === -1) firstTurnIdx = idx;
      }
    });
    const pct = totalWords > 0 ? Math.max(1, Math.round((words / totalWords) * 100)) : 0;
    return {
      speaker,
      color: speakerColors[speaker] ?? "var(--dw-accent)",
      pct,
      firstTurnIdx,
    };
  });

  function jumpToTurn(idx: number) {
    if (idx < 0 || idx >= doc.turns.length) return;
    turnRefs.current[idx]?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    setHighlightedTurn(idx);
    if (highlightTimerRef.current !== null) {
      window.clearTimeout(highlightTimerRef.current);
    }
    highlightTimerRef.current = window.setTimeout(() => {
      setHighlightedTurn((current) => (current === idx ? null : current));
      highlightTimerRef.current = null;
    }, 1500);
  }

  function startRenameSpeaker(speaker: string) {
    setRenamingSpeaker(speaker);
    setSpeakerDraft(speaker);
    setConfirmMerge(null);
  }

  function cancelRenameSpeaker() {
    setRenamingSpeaker(null);
    setConfirmMerge(null);
  }

  function submitSpeakerRename(e: React.FormEvent) {
    e.preventDefault();
    if (!renamingSpeaker || savingSpeaker) return;
    const trimmed = speakerDraft.trim();
    if (!trimmed || trimmed === renamingSpeaker) {
      cancelRenameSpeaker();
      return;
    }
    if (doc.speakers.includes(trimmed) && confirmMerge !== trimmed) {
      setConfirmMerge(trimmed);
      return;
    }
    setSavingSpeaker(true);
    invoke<string>("rename_meeting_speaker", {
      id,
      oldSpeaker: renamingSpeaker,
      newSpeaker: trimmed,
    })
      .then((updated) => {
        onUpdated(updated);
        cancelRenameSpeaker();
      })
      .catch((err) => notifyError("Couldn't rename speaker", err))
      .finally(() => setSavingSpeaker(false));
  }

  return (
    <div className="mt-2 flex flex-col gap-2 rounded-md bg-base-200 p-2.5 text-xs" data-testid="meeting-notes-body">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-base-content/10 pb-2">
        <div className="flex flex-wrap items-center gap-1">
          <span className="mr-0.5 text-[11px] opacity-60">Speakers:</span>
          {doc.speakers.map((speaker) => {
            const color = speakerColors[speaker] ?? "var(--dw-accent)";
            return (
              <button
                key={speaker}
                type="button"
                className={`badge badge-sm cursor-pointer gap-1 hover:brightness-95 ${
                  speaker === "You"
                    ? "badge-primary"
                    : speaker === "Group"
                      ? "badge-secondary"
                      : "badge-outline opacity-85"
                }`}
                onClick={() => startRenameSpeaker(speaker)}
                disabled={savingSpeaker || summarizing}
                title={`Click to rename "${speaker}" across this transcript`}
                data-testid="meeting-speaker-chip"
              >
                {speaker !== "You" && (
                  <span
                    className="h-1.5 w-1.5 shrink-0 rounded-full"
                    style={{ backgroundColor: color }}
                    aria-hidden="true"
                  />
                )}
                <span>{speaker}</span>
                <span className="text-[10px] opacity-60" aria-hidden="true">
                  ✎
                </span>
              </button>
            );
          })}
        </div>
        <button
          type="button"
          className="btn btn-ghost btn-xs h-6 min-h-0 gap-1 border border-base-content/15 px-2 text-[11px]"
          onClick={onSummarize}
          disabled={summarizing || savingSpeaker || doc.turns.length === 0}
          title="Generate summary & action items locally using your configured LLM"
          data-testid="meeting-summarize"
        >
          {summarizing ? (
            <>
              <span className="loading loading-spinner loading-xs" />
              Summarizing…
            </>
          ) : doc.notes ? (
            "Regenerate summary"
          ) : (
            "Summarize"
          )}
        </button>
      </div>

      {renamingSpeaker && (
        <form
          onSubmit={submitSpeakerRename}
          className="flex items-center gap-1.5 rounded bg-base-100 px-2 py-1.5"
          data-testid="meeting-speaker-rename-form"
        >
          <span className="text-[11px] opacity-70">Rename &ldquo;{renamingSpeaker}&rdquo; to:</span>
          <input
            type="text"
            className="input input-xs input-bordered h-6 flex-1 text-xs"
            value={speakerDraft}
            onChange={(e) => {
              setSpeakerDraft(e.target.value);
              setConfirmMerge(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape" && !savingSpeaker) {
                e.preventDefault();
                cancelRenameSpeaker();
              }
            }}
            placeholder="e.g. Sarah"
            autoFocus
            disabled={savingSpeaker || summarizing}
            data-testid="meeting-speaker-rename-input"
          />
          <button
            type="submit"
            className={`btn btn-xs h-6 min-h-0 ${confirmMerge ? "btn-warning" : "btn-primary"}`}
            disabled={savingSpeaker || summarizing}
            data-testid="meeting-speaker-rename-save"
          >
            {confirmMerge ? "Merge" : "Save"}
          </button>
          <button
            type="button"
            className="btn btn-ghost btn-xs h-6 min-h-0"
            onClick={cancelRenameSpeaker}
            disabled={savingSpeaker || summarizing}
          >
            Cancel
          </button>
        </form>
      )}
      {renamingSpeaker && confirmMerge && (
        <p className="text-[11px] text-warning" role="alert" data-testid="meeting-speaker-merge-confirm">
          &ldquo;{confirmMerge}&rdquo; is already a speaker here. Merging joins their lines with &ldquo;
          {renamingSpeaker}&rdquo;&apos;s and can&apos;t be undone. Click Merge to go ahead.
        </p>
      )}

      {doc.notes && (
        <div
          className="rounded-md border border-primary/20 bg-base-100 p-2.5 leading-relaxed"
          data-testid="meeting-summary-box"
        >
          <div className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-primary">Meeting Notes</div>
          <div className="flex flex-col gap-1 text-xs opacity-90">
            {doc.notes.split("\n").map((line, i) => (
              <p key={i} className="whitespace-pre-wrap">
                {renderInlineBold(line)}
              </p>
            ))}
          </div>
        </div>
      )}

      {doc.turns.length > 0 && (
        <div
          className="flex flex-col gap-1.5 rounded-md border border-base-content/10 bg-base-100/75 px-2.5 py-2"
          data-testid="meeting-diarization-ribbon"
        >
          <div className="flex flex-wrap items-center justify-between gap-1.5">
            <div className="flex flex-wrap items-center gap-1.5">
              {speakerShares.map((item) => (
                <button
                  key={item.speaker}
                  type="button"
                  className="inline-flex cursor-pointer items-center gap-1.5 rounded-full border border-base-content/10 bg-base-200/70 px-2 py-0.5 text-[10.5px] transition-colors hover:border-base-content/25"
                  onMouseEnter={() => setHoveredSpeaker(item.speaker)}
                  onMouseLeave={() => setHoveredSpeaker((prev) => (prev === item.speaker ? null : prev))}
                  onClick={() => jumpToTurn(item.firstTurnIdx)}
                  title={`Jump to ${item.speaker}'s first turn (~${item.pct}% talk share)`}
                >
                  <span
                    className="h-2 w-2 shrink-0 rounded-full"
                    style={{ backgroundColor: item.color }}
                    aria-hidden="true"
                  />
                  <span className="font-medium opacity-90">{item.speaker}</span>
                  <span
                    className="tabular-nums opacity-60"
                    style={{ fontFamily: "var(--dw-font-mono, ui-monospace, monospace)" }}
                  >
                    {item.pct}%
                  </span>
                </button>
              ))}
            </div>
            <span
              className="text-[10px] uppercase tracking-wider opacity-45"
              style={{ fontFamily: "var(--dw-font-mono, ui-monospace, monospace)" }}
            >
              {doc.turns.length} {doc.turns.length === 1 ? "turn" : "turns"}
            </span>
          </div>

          <div
            className={`flex h-2.5 w-full items-stretch overflow-hidden rounded-full bg-base-300/70 p-[2px] ${
              doc.turns.length > 60 ? "gap-0" : doc.turns.length > 24 ? "gap-[1px]" : "gap-[2px]"
            }`}
            role="group"
            aria-label="Speaker diarization timeline"
          >
            {doc.turns.map((turn, idx) => {
              const color = speakerColors[turn.speaker] ?? "var(--dw-accent)";
              const isHighlighted = highlightedTurn === idx;
              const dimmed = hoveredSpeaker !== null && hoveredSpeaker !== turn.speaker;
              return (
                <button
                  key={idx}
                  type="button"
                  className={`h-full cursor-pointer border-0 p-0 transition-opacity duration-150 focus-visible:outline focus-visible:outline-1 focus-visible:outline-offset-1 focus-visible:outline-primary ${
                    doc.turns.length > 60 ? "rounded-[1px]" : "rounded-full"
                  }`}
                  style={{
                    flexGrow: turnWords[idx],
                    flexShrink: 1,
                    flexBasis: 0,
                    minWidth: doc.turns.length > 40 ? 0 : "3px",
                    backgroundColor: color,
                    opacity: isHighlighted ? 1 : dimmed ? 0.22 : 0.86,
                    boxShadow: isHighlighted ? `0 0 8px ${color}` : undefined,
                  }}
                  onClick={() => jumpToTurn(idx)}
                  onMouseEnter={() => setHoveredSpeaker(turn.speaker)}
                  onMouseLeave={() => setHoveredSpeaker((prev) => (prev === turn.speaker ? null : prev))}
                  title={`${turn.speaker} · ${turn.timestamp}`}
                  aria-label={`Jump to ${turn.speaker} at ${turn.timestamp}`}
                  data-testid="meeting-timeline-segment"
                />
              );
            })}
          </div>
        </div>
      )}

      <div className="dw-scroll flex max-h-72 flex-col gap-2 overflow-y-auto pr-1" data-testid="meeting-turns">
        {doc.turns.length > 0 ? (
          doc.turns.map((turn, idx) => {
            const color = speakerColors[turn.speaker] ?? "var(--dw-accent)";
            const isHighlighted = highlightedTurn === idx;
            return (
              <div
                key={idx}
                ref={(el) => {
                  turnRefs.current[idx] = el;
                }}
                className="rounded bg-base-100/70 px-2.5 py-1.5 leading-relaxed transition-all duration-300"
                style={{
                  borderLeft: `3px solid ${color}`,
                  boxShadow: isHighlighted ? `0 0 0 1px ${color}` : undefined,
                  backgroundColor: isHighlighted
                    ? `color-mix(in srgb, ${color} 12%, var(--dw-card, transparent))`
                    : undefined,
                }}
              >
                <div className="mb-0.5 flex items-baseline gap-1.5">
                  <button
                    type="button"
                    className="font-semibold hover:underline"
                    style={{ color: `color-mix(in srgb, ${color} 72%, var(--dw-fg))` }}
                    onClick={() => startRenameSpeaker(turn.speaker)}
                    disabled={savingSpeaker || summarizing}
                    title={`Click to rename "${turn.speaker}"`}
                  >
                    {turn.speaker}
                  </button>
                  <span className="text-[11px] tabular-nums opacity-50">· {turn.timestamp}</span>
                </div>
                <pre className="whitespace-pre-wrap font-sans text-xs opacity-90">{turn.text}</pre>
              </div>
            );
          })
        ) : (
          <pre className="whitespace-pre-wrap font-sans text-xs leading-relaxed">{markdown}</pre>
        )}
      </div>
    </div>
  );
}

function MeetingList({
  status,
  focusMeetingId,
  onFocusHandled,
}: {
  status: MeetingStatus | null;
  focusMeetingId?: string | null;
  onFocusHandled?: () => void;
}) {
  const [meetings, setMeetings] = useState<MeetingSummary[] | null>(null);
  const [open, setOpen] = useState<Record<string, string>>({});
  const [copied, setCopied] = useState<string | null>(null);
  const [editingTitleId, setEditingTitleId] = useState<string | null>(null);
  const [titleDraft, setTitleDraft] = useState("");
  const [savingTitle, setSavingTitle] = useState(false);
  // Ids with a summary running; kept here, not in the row body, so it
  // survives collapsing the row.
  const [summarizing, setSummarizing] = useState<string[]>([]);
  // Delete asks once more (the transcript is gone for good).
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const rowRefs = useRef<Record<string, HTMLLIElement | null>>({});
  const scrollTimerRef = useRef<number | null>(null);
  const copyTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (scrollTimerRef.current !== null) window.clearTimeout(scrollTimerRef.current);
      if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (!confirmDelete) return;
    const timer = setTimeout(() => setConfirmDelete(null), 4000);
    return () => clearTimeout(timer);
  }, [confirmDelete]);

  function openAndLoad(id: string, scrollIntoView = false) {
    invoke<string>("read_meeting", { id })
      .then((text) => {
        setOpen((current) => ({ ...current, [id]: text }));
        if (scrollIntoView) {
          if (scrollTimerRef.current !== null) window.clearTimeout(scrollTimerRef.current);
          scrollTimerRef.current = window.setTimeout(() => {
            scrollTimerRef.current = null;
            rowRefs.current[id]?.scrollIntoView({ behavior: "smooth", block: "nearest" });
          }, 50);
        }
      })
      .catch((err) => notifyError("Couldn't read the transcript", err));
  }

  function refresh() {
    invoke<MeetingSummary[]>("list_meetings")
      .then(setMeetings)
      .catch((err) => notifyError("Couldn't load meetings", err));
  }
  // The list changes when a meeting starts, stops, starts/finishes processing,
  // or joins/leaves the queue; not on every progress tick.
  const listKey = `${status?.recording?.id ?? ""}|${status?.processing?.id ?? ""}|${status?.queued.join(",") ?? ""}`;
  useEffect(refresh, [listKey]);

  // Also refresh and expand immediately if a meeting finishes transcribing while
  // the user already has the Meetings page open.
  useEffect(() => {
    const unlisten = listen<MeetingSummary | string | null>("meeting-saved", (e) => {
      refresh();
      const savedId =
        e.payload && typeof e.payload === "object" && "id" in e.payload ? e.payload.id : null;
      if (savedId) {
        openAndLoad(savedId, true);
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // Deep-link focus from the widget ("Meeting transcript saved · View") or menu bar ("View Meeting Notes…").
  // SettingsView clears the request once handled (it outlives this list,
  // which remounts each time Meetings is shown). A meeting not in the list
  // yet (still being saved) stays pending until a later refresh has it.
  useEffect(() => {
    if (!focusMeetingId || !meetings || meetings.length === 0) {
      return;
    }
    const target =
      focusMeetingId === "latest"
        ? meetings.find((m) => m.state === "ready")
        : meetings.find((m) => m.id === focusMeetingId && m.state === "ready");
    if (target) {
      onFocusHandled?.();
      openAndLoad(target.id, true);
    }
  }, [focusMeetingId, meetings]);

  function toggleView(id: string) {
    if (open[id] !== undefined) {
      setOpen(({ [id]: _, ...rest }) => rest);
      return;
    }
    openAndLoad(id, false);
  }

  function saveTitle(id: string, e: React.FormEvent) {
    e.preventDefault();
    if (savingTitle) return;
    const trimmed = titleDraft.trim();
    if (!trimmed) {
      setEditingTitleId(null);
      return;
    }
    setSavingTitle(true);
    invoke<MeetingSummary>("rename_meeting_title", { id, title: trimmed })
      .then((updated) => {
        setMeetings((prev) => prev?.map((m) => (m.id === id ? updated : m)) ?? null);
        setEditingTitleId(null);
        if (open[id] !== undefined) {
          openAndLoad(id, false);
        }
      })
      .catch((err) => notifyError("Couldn't rename the meeting", err))
      .finally(() => setSavingTitle(false));
  }

  // Updates an open row's transcript; a row collapsed meanwhile stays shut.
  function updateOpen(id: string, nextMarkdown: string) {
    setOpen((current) => (current[id] === undefined ? current : { ...current, [id]: nextMarkdown }));
  }

  function summarize(id: string) {
    if (summarizing.includes(id)) return;
    setSummarizing((ids) => [...ids, id]);
    invoke<string>("summarize_meeting", { id })
      .then((updated) => {
        updateOpen(id, updated);
        refresh();
      })
      .catch((err) => notifyError("Couldn't summarize the meeting", err))
      .finally(() => setSummarizing((ids) => ids.filter((x) => x !== id)));
  }

  function copy(id: string) {
    invoke<string>("read_meeting", { id })
      .then((text) => navigator.clipboard.writeText(text))
      .then(() => {
        setCopied(id);
        if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
        copyTimerRef.current = window.setTimeout(() => {
          copyTimerRef.current = null;
          setCopied((c) => (c === id ? null : c));
        }, 1500);
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
        <li
          key={m.id}
          ref={(el) => {
            rowRefs.current[m.id] = el;
          }}
          className="group rounded px-1.5 py-1.5 text-xs hover:bg-base-200"
          data-testid="meeting-row"
        >
          <div className="flex items-center justify-between gap-2">
            <span className="min-w-0 flex-1">
              {editingTitleId === m.id ? (
                <form onSubmit={(e) => saveTitle(m.id, e)} className="mb-0.5 flex items-center gap-1">
                  <input
                    type="text"
                    className="input input-xs input-bordered h-6 flex-1 text-xs"
                    value={titleDraft}
                    onChange={(e) => setTitleDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape" && !savingTitle) {
                        e.preventDefault();
                        setEditingTitleId(null);
                      }
                    }}
                    autoFocus
                    disabled={savingTitle}
                    aria-label="Meeting title"
                    data-testid="meeting-title-input"
                  />
                  <button
                    type="submit"
                    className="btn btn-primary btn-xs h-6 min-h-0"
                    disabled={savingTitle}
                    data-testid="meeting-title-save"
                  >
                    Save
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost btn-xs h-6 min-h-0"
                    onClick={() => setEditingTitleId(null)}
                    disabled={savingTitle}
                  >
                    Cancel
                  </button>
                </form>
              ) : (
                <span className="flex items-center gap-1">
                  <span className="truncate font-medium">{m.title}</span>
                  {m.state === "ready" && (
                    <button
                      type="button"
                      className="btn btn-ghost btn-xs h-4 min-h-0 px-1 text-[10px] opacity-0 group-hover:opacity-60 hover:!opacity-100 focus-visible:opacity-100"
                      onClick={() => {
                        setEditingTitleId(m.id);
                        setTitleDraft(m.title);
                      }}
                      title="Rename meeting"
                      aria-label={`Rename ${m.title}`}
                      data-testid="meeting-title-edit"
                    >
                      ✎
                    </button>
                  )}
                </span>
              )}
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
                  <button
                    className="btn btn-ghost btn-xs"
                    onClick={() => toggleView(m.id)}
                    aria-expanded={open[m.id] !== undefined}
                  >
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
            <MeetingNotesBody
              id={m.id}
              markdown={open[m.id]}
              summarizing={summarizing.includes(m.id)}
              onSummarize={() => summarize(m.id)}
              onUpdated={(nextMarkdown) => {
                updateOpen(m.id, nextMarkdown);
                refresh();
              }}
            />
          )}
        </li>
      ))}
    </ul>
  );
}

export function MeetingsSection({
  focusMeetingId,
  onFocusHandled,
}: {
  focusMeetingId?: string | null;
  onFocusHandled?: () => void;
} = {}) {
  const status = useMeetingStatus();
  const lock = useHistoryLock();
  return (
    <>
      <RecordControl status={status} />
      <AutoDetectToggle />
      <SpeakerCount />
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide opacity-60">Transcripts</h3>
      {lock.status === null ? (
        <span className="loading loading-spinner loading-xs" />
      ) : lock.status.locked ? (
        <LockedHistory lock={lock} />
      ) : (
        <MeetingList status={status} focusMeetingId={focusMeetingId} onFocusHandled={onFocusHandled} />
      )}
    </>
  );
}
