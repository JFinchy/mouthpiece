import { Fragment, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTheme, type ThemeId } from "./theme";
import { getReadiness, type ReadinessCheck } from "./settings/readiness";
import { shortcutLabel, skipVariant, type ShortcutConfig } from "./shortcut";
import { spokenFolder, type WaitingAgent } from "./settings/sections/agents";
import { useMeetingStatus, type DetectedCall, type MeetingSummary } from "./settings/sections/meetings";
import {
  DEFAULT_WAVEFORM_CONFIG,
  getWaveformColorway,
  SOLID_COLORWAY_ORDER,
  SPECTRUM_COLORWAY_ORDER,
  WAVEFORM_COLORWAYS,
  WAVEFORM_ENGINE_META,
  type WaveformConfig,
  type WaveformEngineId,
} from "./waveformConfig";
import { WaveformMeter, type AudioBands } from "./components/WaveformMeter";

type Phase = "idle" | "recording" | "transcribing" | "refining";
type WidgetMode = "minimal" | "compact" | "detailed" | "pill" | "notch" | "caption" | "stacked";

// Every style, in picker order, with its short name (the flyout's Style
// row; Settings and the menu bar show the same list).
export const WIDGET_STYLES: { id: WidgetMode; label: string }[] = [
  { id: "minimal", label: "Minimal" },
  { id: "pill", label: "Pill" },
  { id: "compact", label: "Compact" },
  { id: "stacked", label: "Two-line" },
  { id: "notch", label: "Notch" },
  { id: "caption", label: "Caption" },
  { id: "detailed", label: "Detailed" },
];

// Two-line compact is the compact layout plus a second, smaller line.
const STACKED_EXTRA = 6;
// Notch: small while idle, wider while recording or showing something.
const NOTCH_IDLE = { width: 184, height: 32 };
const NOTCH_ACTIVE = { width: 300, height: 34 };
// Only the name is needed here — the flyout just offers a pick list, the
// backend resolves the full mode (behavior/models) by name when the
// override is consumed. See `modes::ModeDefinition` for the full shape.
type ModeSummary = { name: string };
// Mirrors `audio::DeviceStatus` (serde tag = "kind").
type DeviceStatus =
  | { kind: "preferred"; using: string }
  | { kind: "fallback"; wanted: string; using: string }
  | { kind: "system_default"; using: string }
  | { kind: "unavailable"; wanted: string | null };

// Only the cases worth interrupting the idle label for: the picked mic is
// missing (so you're silently on another one), or there's no mic at all.
function deviceWarning(status: DeviceStatus | null): { short: string; full: string } | null {
  if (status?.kind === "fallback") {
    return {
      short: `Using ${status.using}`,
      full: `${status.wanted} isn't connected, so recording is using ${status.using}.`,
    };
  }
  if (status?.kind === "unavailable") {
    return { short: "No microphone", full: "No microphone is available. Connect one or pick another in Settings." };
  }
  return null;
}

const STATUS_LABEL: Record<Phase, string> = {
  idle: "Ready",
  recording: "Listening…",
  transcribing: "Transcribing…",
  refining: "Refining…",
};

// Wide enough for "Transcribing 100%", a mic name and the gear without
// cutting words off (220 was not).
const COMPACT_WIDTH = 280;
const COMPACT_BASE_SIZE = { width: COMPACT_WIDTH, height: 60 };
// How tall compact grows while an error is showing, so the full message
// is readable instead of being cut off by `truncate` — that silently ate
// the Accessibility-permission error before this fix.
const COMPACT_EXPANDED_SIZE = { width: COMPACT_WIDTH, height: 108 };
// An error grows the window to fit the whole message (measured), up to
// this; anything longer is clamped, with the full text in its tooltip.
const COMPACT_ERROR_MAX_HEIGHT = 150;
// Vertical space around the message: the window's padding and border.
const COMPACT_ERROR_CHROME = 22;
// How tall compact grows for the hover flyout (mic mode + a one-off mode
// override for the next dictation), sized for exactly one row of
// "Next dictation" buttons. Grows downward rather than upward — this
// window has no titlebar to anchor from, and downward reuses the exact
// resize mechanism COMPACT_EXPANDED_SIZE already relies on instead of
// introducing new position math.
const COMPACT_FLYOUT_BASE_HEIGHT = 168;
// The flyout never grows the window past this; longer lists (many mics or
// modes) scroll inside it instead of overflowing the window, which made
// macOS draw its own scroll bar outside the widget.
const COMPACT_FLYOUT_MAX_HEIGHT = 340;
// "Next dictation" is a two-column grid of mode buttons (.dw-fly-grid in
// App.css), so the flyout grows by one row per two modes — matches that
// CSS's row height + gap.
const FLYOUT_MODE_ROW_HEIGHT = 26;
const FLYOUT_MODE_ROW_GAP = 4;

// The Microphone section's label plus the gaps around it (.dw-fly-body's
// 6px gap, ~12px label line) — each mic row then adds a full row height.
const FLYOUT_MIC_SECTION_BASE = 18;

// The Style section: its label plus one row per two styles.
const FLYOUT_STYLE_SECTION = 18 + Math.ceil(WIDGET_STYLES.length / 2) * (26 + 4);

// The Waveform section: header row, 4-pill engine strip, and compact colorway orb row.
const FLYOUT_WAVEFORM_SECTION = 56;
const CAPTION_BASE_SIZE = { width: 380, height: 116 };
const DETAILED_BASE_SIZE = { width: 320, height: 170 };
const EXPANDED_FLYOUT_MAX_HEIGHT = 380;

const FLYOUT_SPECTRUM_COLORWAYS = SPECTRUM_COLORWAY_ORDER.map((id) => WAVEFORM_COLORWAYS[id]);
const FLYOUT_SOLID_COLORWAYS = SOLID_COLORWAY_ORDER.map((id) => WAVEFORM_COLORWAYS[id]);

const FLYOUT_ENGINE_LABEL: Record<WaveformEngineId, string> = {
  precision: "Studio",
  silk: "Silk",
  matrix: "Matrix",
  hyperdom: "Needles",
};

function flyoutExtraHeight(modeCount: number, micRowCount: number, showWaveform: boolean) {
  const extraRows = Math.ceil(Math.max(modeCount, 1) / 2) - 1;
  const micRows = micRowCount > 0 ? FLYOUT_MIC_SECTION_BASE + micRowCount * (FLYOUT_MODE_ROW_HEIGHT + FLYOUT_MODE_ROW_GAP) : 0;
  return (
    COMPACT_FLYOUT_BASE_HEIGHT -
    COMPACT_BASE_SIZE.height +
    extraRows * (FLYOUT_MODE_ROW_HEIGHT + FLYOUT_MODE_ROW_GAP) +
    micRows +
    (showWaveform ? FLYOUT_WAVEFORM_SECTION : 0) +
    FLYOUT_STYLE_SECTION
  );
}

function compactFlyoutSize(modeCount: number, micRowCount: number, showWaveform: boolean) {
  const natural = COMPACT_BASE_SIZE.height + flyoutExtraHeight(modeCount, micRowCount, showWaveform);
  return { width: COMPACT_WIDTH, height: Math.min(natural, COMPACT_FLYOUT_MAX_HEIGHT) };
}

function expandedFlyoutSize(
  base: { width: number; height: number },
  modeCount: number,
  micRowCount: number,
  showWaveform: boolean,
) {
  const natural = base.height + flyoutExtraHeight(modeCount, micRowCount, showWaveform);
  return { width: base.width, height: Math.min(natural, EXPANDED_FLYOUT_MAX_HEIGHT) };
}

function usingMic(status: DeviceStatus | null): string | null {
  return status && status.kind !== "unavailable" ? status.using : null;
}

// Recent input levels kept for the live 60fps WaveformMeter (20 per second,
// so about 3 s); WaveformMeter handles ring interpolation, attack/decay
// ballistics, peak caps, and dual-channel Meeting split automatically.
const LEVEL_BARS = 64;
const SILENT_LEVELS = Array<number>(LEVEL_BARS).fill(0);

// Wrapper around the 60fps WaveformMeter that preserves outer flex/width
// sizing classes (e.g. `flex-1`, `w-10`, `group-hover:hidden`) and keeps
// `data-testid="level-meter"` across every widget layout.
function LevelMeter({
  levels,
  remoteLevels,
  bands,
  config,
  dualChannel = false,
  className,
}: {
  levels: number[];
  remoteLevels?: number[];
  bands?: AudioBands;
  config: WaveformConfig;
  dualChannel?: boolean;
  className?: string;
}) {
  return (
    <div className={`relative h-full min-w-0 ${className ?? ""}`}>
      <WaveformMeter
        levels={levels}
        remoteLevels={remoteLevels}
        bands={bands}
        config={config}
        compact
        dualChannel={dualChannel}
        testId="level-meter"
      />
    </div>
  );
}

// The end of the running transcript, which is what's just been said; the
// widget shows a few lines at most.
const LIVE_TAIL_CHARS = 160;
function liveTail(text: string) {
  return text.length <= LIVE_TAIL_CHARS ? text : `…${text.slice(-LIVE_TAIL_CHARS).replace(/^\S*\s/, "")}`;
}

const AT_FILE_RE = /^@[A-Za-z0-9_./-]+$/;
const FILE_EXT_RE =
  /^[A-Za-z0-9_./-]*[A-Za-z0-9_-]\.(?:rs|ts|tsx|js|jsx|mjs|cjs|py|go|rb|java|kt|swift|c|h|cc|cpp|hpp|cs|php|css|scss|html|htm|svg|json|toml|yaml|yml|xml|md|txt|sql|sh|zsh|bash|env|lock|log|csv|wav|mp3|png|jpg|webp|pdf)$/i;
const PATH_RE =
  /^(?:(?:\.{1,2}|~)\/[A-Za-z0-9_./-]+|[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)+)$/;
const BACKTICK_RE = /^`[^`\s]+`$/;
const SNAKE_SYM_RE = /^[A-Za-z0-9]+(?:_[A-Za-z0-9]+)+(?:\(\))?$/;
const CAMEL_OR_PASCAL_RE = /^(?=.*[a-z][A-Z])[A-Za-z][A-Za-z0-9_]*(?:\(\))?$/;

function classifyCaptionToken(core: string): "file" | "sym" | null {
  if (!core) return null;
  if (AT_FILE_RE.test(core) || FILE_EXT_RE.test(core) || PATH_RE.test(core)) {
    return "file";
  }
  if (BACKTICK_RE.test(core) || SNAKE_SYM_RE.test(core) || CAMEL_OR_PASCAL_RE.test(core)) {
    return "sym";
  }
  return null;
}

// Highlights file references (@recording.rs, src/App.tsx) and code symbols
// (paste_text, AudioHandle, useState, `foo`) as monospace pills while
// preserving every character and whitespace run verbatim.
function renderRichCaption(text: string): ReactNode {
  const parts = text.split(/(\s+)/);
  return parts.map((part, idx) => {
    if (!part || /^\s+$/.test(part)) return part;
    const match = part.match(/^([("'\[]*)(.*?)([.,;:!?"'\])]*)$/);
    if (!match) return part;
    let [, lead, core, trail] = match;
    if (trail.startsWith(")") && core.endsWith("(")) {
      core = `${core})`;
      trail = trail.slice(1);
    }
    const kind = classifyCaptionToken(core);
    if (!kind) return part;
    return (
      <Fragment key={idx}>
        {lead}
        <span className={kind === "file" ? "dw-file-pill" : "dw-sym-pill"}>{core}</span>
        {trail}
      </Fragment>
    );
  });
}

// Matches `LIMIT_WARNING` in meeting.rs.
const MEETING_WARNING_MS = 10 * 60_000;

function formatElapsed(ms: number): string {
  const total = Math.floor(ms / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

function GearIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4 opacity-70" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden>
      <path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-3.5 w-3.5 opacity-70" fill="none" stroke="currentColor" strokeWidth="2.2" aria-hidden>
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

// One processing stage: a label, its percentage, and a slim segmented track
// whose width animates between backend updates (a short dictation only gets a
// handful), so it reads as a continuous two-stage pipeline: amber for
// transcribing handing off seamlessly into violet with a sparkle and shimmer
// for LLM refinement.
type Stage = "transcribe" | "refine";

const STAGE_COLOR: Record<Stage, string> = {
  transcribe: "var(--dw-accent)",
  refine: "var(--dw-refine)",
};

function SparkleIcon() {
  return (
    <svg viewBox="0 0 16 16" className="h-3 w-3 shrink-0" fill="currentColor" aria-hidden>
      <path d="M8 0.5l1.6 4.4L14 6.5 9.6 8.1 8 12.5 6.4 8.1 2 6.5l4.4-1.6L8 0.5z" />
      <path d="M13 10.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7.7-1.8z" opacity="0.7" />
    </svg>
  );
}

function PastedBadge({ className = "text-sm" }: { className?: string }) {
  return (
    <span className={`inline-flex items-center gap-1.5 font-medium text-emerald-400 ${className}`}>
      <svg
        viewBox="0 0 12 12"
        className="h-3 w-3 shrink-0"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d="M2.2 6.3 4.8 8.9 9.8 3.3" />
      </svg>
      <span>Pasted</span>
    </span>
  );
}

const STT_FILL_GRADIENT = "linear-gradient(90deg, var(--dw-accent), color-mix(in srgb, var(--dw-accent) 60%, #fff))";

function StageProgress({
  stage,
  percent,
  label: labelOverride,
  title,
  onMouseDown,
  onSkip,
}: {
  stage: Stage;
  percent: number | null;
  label?: string;
  title?: string;
  onMouseDown: (e: React.MouseEvent) => void;
  /// Refining only: clicking pastes the unrefined text now.
  onSkip?: () => void;
}) {
  const isRefine = stage === "refine";
  const label = labelOverride ?? (isRefine ? "Refining" : "Transcribing");
  const heading = percent === null ? `${label}…` : label;
  return (
    <div
      className={`dw-stage-enter flex min-w-0 flex-1 flex-col gap-1.5 ${
        onSkip ? "cursor-pointer" : "cursor-grab active:cursor-grabbing"
      }`}
      onMouseDown={onSkip ? undefined : onMouseDown}
      onClick={onSkip}
      title={title}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      data-testid={`${stage}-progress`}
    >
      <span
        className="flex items-baseline justify-between gap-2 text-xs leading-none"
        style={isRefine ? { color: STAGE_COLOR.refine } : { opacity: 0.9 }}
      >
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="inline-flex shrink-0 items-center gap-0.5" aria-hidden="true">
            <span
              className="h-1.5 w-1.5 rounded-full"
              style={{
                backgroundColor: STAGE_COLOR.transcribe,
                boxShadow: "0 0 6px var(--dw-accent)",
                opacity: isRefine ? 0.9 : 1,
              }}
            />
            <span className={`h-px w-1.5 ${isRefine ? "bg-white/30" : "bg-white/15"}`} />
            {isRefine ? (
              <SparkleIcon />
            ) : (
              <span className="h-1.5 w-1.5 rounded-full bg-white/20" />
            )}
          </span>
          <span className="truncate font-semibold">{heading}</span>
        </span>
        {percent !== null && <span className="shrink-0 font-mono text-[10.5px] tabular-nums">{percent}%</span>}
      </span>
      <div className="dw-pipeline-track">
        <div className="dw-pipeline-seg flex-[3]" aria-hidden="true">
          {isRefine ? (
            <div
              className="h-full w-full rounded-full"
              style={{
                background: STT_FILL_GRADIENT,
                boxShadow: "0 0 8px color-mix(in srgb, var(--dw-accent) 45%, transparent)",
              }}
            />
          ) : percent === null ? (
            <div className="dw-amber-sweep" />
          ) : (
            <div
              className="h-full rounded-full transition-[width] duration-200 ease-out motion-reduce:transition-none"
              style={{
                width: `${percent}%`,
                background: STT_FILL_GRADIENT,
              }}
            />
          )}
        </div>
        <div className="dw-pipeline-seg flex-[2]" aria-hidden="true">
          {isRefine && (
            <div
              className="dw-shimmer h-full rounded-full transition-[width] duration-300 ease-out motion-reduce:transition-none"
              style={{
                width: `${percent ?? 100}%`,
                backgroundColor: STAGE_COLOR.refine,
                opacity: percent === null ? 0.72 : 1,
              }}
            />
          )}
        </div>
      </div>
    </div>
  );
}

// A hairline 60/40 segmented two-stage progress bar for the small styles
// (Pill, Notch), showing amber Transcribing handing off into violet Refining.
function ThinProgress({ stage, percent }: { stage: Stage; percent: number | null }) {
  const isRefine = stage === "refine";
  return (
    <div
      className="dw-stage-enter flex min-w-0 flex-1 items-center gap-1"
      role="progressbar"
      aria-label={isRefine ? "Refining" : "Transcribing"}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      data-testid={`${stage}-progress`}
    >
      <div className="dw-pipeline-track !h-[4px]">
        <div className="dw-pipeline-seg flex-[3]" aria-hidden="true">
          {isRefine ? (
            <div className="h-full w-full rounded-full" style={{ background: STT_FILL_GRADIENT }} />
          ) : percent === null ? (
            <div className="dw-amber-sweep" />
          ) : (
            <div
              className="h-full rounded-full transition-[width] duration-200 ease-out motion-reduce:transition-none"
              style={{ width: `${percent}%`, background: STT_FILL_GRADIENT }}
            />
          )}
        </div>
        <div className="dw-pipeline-seg flex-[2]" aria-hidden="true">
          {isRefine && (
            <div
              className="dw-shimmer h-full rounded-full transition-[width] duration-300 ease-out motion-reduce:transition-none"
              style={{
                width: `${percent ?? 100}%`,
                backgroundColor: STAGE_COLOR.refine,
                opacity: percent === null ? 0.72 : 1,
              }}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function ProgressRing({ stage, percent }: { stage: Stage; percent: number }) {
  const r = 21;
  const circumference = 2 * Math.PI * r;
  return (
    <svg
      className="pointer-events-none absolute inset-0 h-full w-full -rotate-90"
      viewBox="0 0 46 46"
      aria-hidden
      data-testid={`${stage}-progress-ring`}
    >
      <circle cx="23" cy="23" r={r} fill="none" stroke="rgba(255,255,255,0.1)" strokeWidth="2.5" />
      <circle
        cx="23"
        cy="23"
        r={r}
        fill="none"
        stroke={STAGE_COLOR[stage]}
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeDasharray={circumference}
        strokeDashoffset={circumference * (1 - percent / 100)}
        className="transition-[stroke-dashoffset] duration-300 ease-out motion-reduce:transition-none"
      />
    </svg>
  );
}

function WidgetView() {
  const [mode, setMode] = useState<WidgetMode>("compact");
  const [modeLoaded, setModeLoaded] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [waveformConfig, setWaveformConfig] = useState<WaveformConfig>(DEFAULT_WAVEFORM_CONFIG);
  // A meeting recording shows as "recording" with its own label, and its
  // transcription as an idle-time status (dictation still works then).
  const meeting = useMeetingStatus();
  const meetingRecording = meeting?.recording ?? null;
  const meetingProcessing = meeting?.processing ?? null;
  // Follows the status too, so a missed "meeting-stopped" can't leave the
  // widget showing a recording, and finishing a mid-meeting push-to-talk
  // dictation returns the widget to its meeting "recording" state.
  const wasMeeting = useRef(false);
  const meetingRecordingRef = useRef(false);
  meetingRecordingRef.current = meetingRecording !== null;
  // A dictation (not the meeting itself) is recording. Both share the
  // "recording" phase, and push-to-talk works during a meeting, so the main
  // button and the meeting-stopped reset need to tell them apart.
  const [dictating, setDictating] = useState(false);
  const dictatingRef = useRef(false);
  dictatingRef.current = dictating;
  // The phase to show once something (a dictation or the meeting) ends.
  const settledPhase = (p: Phase): Phase =>
    p === "transcribing" || p === "refining" || (p === "recording" && dictatingRef.current) ? p : "idle";
  useEffect(() => {
    if (meetingRecording && phase === "idle") setPhase("recording");
    else if (!meetingRecording && wasMeeting.current) {
      setEndedCall(null);
      setPhase(settledPhase);
    }
    wasMeeting.current = meetingRecording !== null;
  }, [meetingRecording, phase]);
  // A mic that sent nothing was swapped for another mid-recording (or one
  // was picked from the "Switch mic" menu): shown in place of the timer
  // for a moment.
  const [micNote, setMicNote] = useState<string | null>(null);
  useEffect(() => {
    if (!micNote) return;
    const timer = setTimeout(() => setMicNote(null), 4000);
    return () => clearTimeout(timer);
  }, [micNote]);
  // The last recording came back silent: its error offers "Switch mic".
  const [micDead, setMicDead] = useState(false);
  // "Dictation is paused…", shown in place of the meeting's timer for a moment.
  const [meetingNote, setMeetingNote] = useState<string | null>(null);
  const [detectedCall, setDetectedCall] = useState<DetectedCall | null>(null);
  const [endedCall, setEndedCall] = useState<DetectedCall | null>(null);
  const [savedMeeting, setSavedMeeting] = useState<MeetingSummary | null>(null);
  const [savedMeetingCopied, setSavedMeetingCopied] = useState(false);
  useEffect(() => {
    invoke<DetectedCall | null>("get_detected_meeting")
      .then((call) => {
        if (call) setDetectedCall(call);
      })
      .catch(() => {});
    const unlistenBlocked = listen<string>("meeting-blocked", (e) => setMeetingNote(e.payload));
    const unlistenDetected = listen<DetectedCall>("meeting-detected", (e) => {
      setEndedCall(null);
      setDetectedCall(e.payload);
    });
    const unlistenCleared = listen("meeting-detect-cleared", () => setDetectedCall(null));
    const unlistenEnded = listen<DetectedCall>("meeting-ended-detected", (e) => setEndedCall(e.payload));
    // The call came back after a brief drop: withdraw "Call ended · Stop?".
    const unlistenEndedCleared = listen("meeting-ended-cleared", () => setEndedCall(null));
    return () => {
      unlistenEndedCleared.then((f) => f());
      unlistenBlocked.then((f) => f());
      unlistenDetected.then((f) => f());
      unlistenCleared.then((f) => f());
      unlistenEnded.then((f) => f());
    };
  }, []);
  useEffect(() => {
    if (meetingRecording) {
      setDetectedCall(null);
      setSavedMeeting(null);
    } else {
      setEndedCall(null);
    }
  }, [meetingRecording]);
  useEffect(() => {
    if (phase !== "idle") setSavedMeeting(null);
  }, [phase]);
  useEffect(() => {
    if (!meetingNote) return;
    const timer = setTimeout(() => setMeetingNote(null), 2500);
    return () => clearTimeout(timer);
  }, [meetingNote]);
  // `message` is sticky — it holds whatever the last status/error text
  // was until something new replaces it. `flashVisible` controls whether
  // compact/minimal show it as a temporary overlay vs. falling back to
  // the plain phase label; detailed mode ignores `flashVisible` and
  // always shows `message`, since persistence is the point of that mode.
  const [message, setMessage] = useState<string | null>(null);
  const [isError, setIsError] = useState(false);
  const [flashVisible, setFlashVisible] = useState(false);
  const flashTimeout = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Quick-actions hover flyout state (compact / stacked mode) — see
  // the `.dw-fly-*` styles in App.css. `micMode`/`overrideMode` mirror
  // real backend state (isolated_voice_enabled / the one-shot mode
  // override), not local-only UI state. Uses a 160ms hover-intent delay on
  // enter and a 120ms grace delay on leave so grazing the widget doesn't
  // cause jarring 340px window jumps.
  const [flyoutHovered, setFlyoutHovered] = useState(false);
  const flyoutOpenTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const flyoutCloseTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  function clearFlyoutTimers() {
    if (flyoutOpenTimer.current !== null) {
      clearTimeout(flyoutOpenTimer.current);
      flyoutOpenTimer.current = null;
    }
    if (flyoutCloseTimer.current !== null) {
      clearTimeout(flyoutCloseTimer.current);
      flyoutCloseTimer.current = null;
    }
  }

  useEffect(() => clearFlyoutTimers, []);

  function handleFlyoutMouseEnter() {
    if (flyoutCloseTimer.current !== null) {
      clearTimeout(flyoutCloseTimer.current);
      flyoutCloseTimer.current = null;
    }
    if (flyoutOpenTimer.current !== null) {
      clearTimeout(flyoutOpenTimer.current);
    }
    flyoutOpenTimer.current = setTimeout(() => {
      flyoutOpenTimer.current = null;
      setFlyoutHovered(true);
    }, 160);
  }

  function handleFlyoutMouseLeave() {
    if (flyoutOpenTimer.current !== null) {
      clearTimeout(flyoutOpenTimer.current);
      flyoutOpenTimer.current = null;
    }
    if (flyoutCloseTimer.current !== null) {
      clearTimeout(flyoutCloseTimer.current);
    }
    flyoutCloseTimer.current = setTimeout(() => {
      flyoutCloseTimer.current = null;
      setFlyoutHovered(false);
    }, 120);
  }

  // "Show the widget only while dictating" (Settings > General): once a
  // dictation is over, the widget hides itself. `hideWhenDone` is set by a
  // dictation (or by turning the setting on), so a widget opened from the
  // menu bar stays until the next one.
  const [onlyWhileDictating, setOnlyWhileDictating] = useState(false);
  const hideWhenDone = useRef(false);
  const [pointerInside, setPointerInside] = useState(false);
  useEffect(() => {
    if (!savedMeeting || pointerInside) return;
    const timer = setTimeout(() => setSavedMeeting(null), 15_000);
    return () => clearTimeout(timer);
  }, [savedMeeting, pointerInside]);
  // Esc paused the recording: the widget asks whether to resume,
  // transcribe or delete it (see discard.rs).
  const [paused, setPaused] = useState(false);
  // What was pasted last, for the Live caption style.
  const [lastPasted, setLastPasted] = useState<string | null>(null);
  const compactMessageRef = useRef<HTMLSpanElement | null>(null);
  const [errorHeight, setErrorHeight] = useState<number | null>(null);
  const [micMode, setMicMode] = useState(false);
  const [overrideMode, setOverrideMode] = useState<string | null>(null);
  const [modes, setModes] = useState<ModeSummary[]>([]);
  // Set once per launch if the backend's startup check (see lib.rs) finds
  // a newer release; null means either "no update" or "haven't heard back
  // yet" — both render the same (no badge), so they don't need telling
  // apart here.
  const [updateAvailable, setUpdateAvailable] = useState<string | null>(null);

  // Rolling windows of recent input levels for the live level meter — see
  // LevelMeter above. Reset to silence whenever recording isn't active, so
  // a stale "last thing you said was loud" reading doesn't linger once
  // idle/transcribing.
  const [levels, setLevels] = useState<number[]>(SILENT_LEVELS);
  const [remoteLevels, setRemoteLevels] = useState<number[]>(SILENT_LEVELS);
  const [dualMicLevels, setDualMicLevels] = useState<number[] | null>(null);
  const [audioBands, setAudioBands] = useState<AudioBands | undefined>(undefined);
  const [deviceStatus, setDeviceStatus] = useState<DeviceStatus | null>(null);
  const [ambientPhase, setAmbientPhase] = useState<"off" | "listening" | "asleep" | "paused">("off");
  const warning = deviceWarning(deviceStatus);
  const [devices, setDevices] = useState<string[]>([]);
  // Set while recording when the mic has delivered no sound for ~2s
  // (backend `audio::SilenceWatch`); the payload is the full message.
  const [micSilent, setMicSilent] = useState<string | null>(null);
  // Transcription progress, 0-100, while transcribing (backend
  // `transcribe-progress`); null when there's nothing to show.
  const [progress, setProgress] = useState<number | null>(null);
  // Transcript of the recording so far, filled in at each pause while
  // recording (`live-transcript`); shown only in detailed mode.
  const [liveText, setLiveText] = useState("");
  // Same for LLM refinement (`refine-progress`), plus which model is
  // doing it (the `refining-started` payload).
  const [refineProgress, setRefineProgress] = useState<number | null>(null);
  const [refineModel, setRefineModel] = useState<string | null>(null);
  // Elapsed recording time, ticking while recording.
  const [elapsedMs, setElapsedMs] = useState(0);
  const meetingStartedMs = meetingRecording?.started_ms ?? null;
  useEffect(() => {
    if (phase !== "recording") return;
    const started = dictating ? Date.now() : (meetingStartedMs ?? Date.now());
    setElapsedMs(Date.now() - started);
    const tick = setInterval(() => setElapsedMs(Date.now() - started), 250);
    return () => clearInterval(tick);
  }, [phase, meetingStartedMs, dictating]);

  // The real hotkey, for the detailed idle hint, and its "paste as-is"
  // variant for the refining tooltip.
  const [hotkey, setHotkey] = useState<string | null>(null);
  const [skipKey, setSkipKey] = useState<string | null>(null);
  useEffect(() => {
    const load = () =>
      invoke<ShortcutConfig>("get_shortcut")
        .then((cfg) => {
          setHotkey(shortcutLabel(cfg));
          setSkipKey(shortcutLabel(skipVariant(cfg)));
        })
        .catch((err) => console.error("get_shortcut failed:", err));
    load();
    const unlisten = listen("shortcut-changed", load);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);
  const micName = usingMic(deviceStatus);

  // Coding agents waiting for a reply (see agents.rs), newest first. Expiry
  // happens in the backend, so the list is re-read now and then.
  const [waiting, setWaiting] = useState<WaitingAgent[]>([]);
  // An agent asked a question through `mouthpiece listen` / MCP ask_user;
  // the dictation being recorded is the answer. "" = asked without a prompt.
  const [agentQuestion, setAgentQuestion] = useState<string | null>(null);
  useEffect(() => {
    const unlisten = listen<string | null>("agent-question", (e) => setAgentQuestion(e.payload));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);
  const [replyKey, setReplyKey] = useState<string | null>(null);
  // Whether "to <folder>, ..." is on, for the pill's hint. Changing it
  // emits nothing, so it's re-read along with the list.
  const [addressing, setAddressing] = useState(false);
  useEffect(() => {
    const loadAddressing = () =>
      invoke<boolean>("get_agent_addressing_enabled")
        .then(setAddressing)
        .catch((err) => console.error("get_agent_addressing_enabled failed:", err));
    const refresh = () => {
      loadAddressing();
      invoke<WaitingAgent[]>("get_waiting_agents")
        .then(setWaiting)
        .catch((err) => console.error("get_waiting_agents failed:", err));
    };
    refresh();
    const unlisten = listen<WaitingAgent[]>("agents-changed", (e) => {
      setWaiting(e.payload);
      loadAddressing();
    });
    const interval = setInterval(refresh, 30_000);
    const loadKey = () =>
      invoke<ShortcutConfig>("get_agent_reply_shortcut")
        .then((cfg) => setReplyKey(shortcutLabel(cfg)))
        .catch((err) => console.error("get_agent_reply_shortcut failed:", err));
    loadKey();
    const unlistenKey = listen("shortcut-changed", loadKey);
    return () => {
      clearInterval(interval);
      unlisten.then((f) => f());
      unlistenKey.then((f) => f());
    };
  }, []);

  // One row per selectable mic for the hover menu. The highlighted row is
  // the mic actually recording; a picked-but-missing mic gets its own
  // disabled row so a silent fallback is visible.
  const micRows: { key: string; label: string; name: string | null; active: boolean; disabled: boolean }[] = [
    {
      key: "__default",
      // The device name is already its own row; repeating it here
      // truncated and read as a duplicate.
      label: "System default",
      name: null,
      active: deviceStatus?.kind === "system_default",
      disabled: false,
    },
    ...(deviceStatus?.kind === "fallback" && !devices.includes(deviceStatus.wanted)
      ? [{ key: "__missing", label: `${deviceStatus.wanted} (not connected)`, name: deviceStatus.wanted, active: false, disabled: true }]
      : []),
    ...devices.map((d) => ({
      key: d,
      label: d,
      name: d,
      active: (deviceStatus?.kind === "preferred" || deviceStatus?.kind === "fallback") && deviceStatus.using === d,
      disabled: false,
    })),
  ];

  function announce(text: string, error: boolean) {
    setMessage(text);
    setIsError(error);
    setFlashVisible(true);
    if (flashTimeout.current) clearTimeout(flashTimeout.current);
    // Errors stay up longer than a plain "Pasted" confirmation — they're
    // the ones worth actually reading.
    flashTimeout.current = setTimeout(() => setFlashVisible(false), error ? 6000 : 2000);
  }

  // A widget action the backend rejected: say so in the widget itself,
  // which is the only place the user is looking.
  function actionFailed(what: string) {
    return (err: unknown) => {
      console.error(`${what} failed:`, err);
      announce(`Couldn't ${what}: ${String(err)}`, true);
    };
  }

  // Blocking setup problems (no model, can't paste, hotkey taken), shown
  // in place of "Ready" so a fresh install says what's missing up front.
  const [setupProblem, setSetupProblem] = useState<ReadinessCheck | null>(null);
  // The last dictation couldn't paste for lack of Accessibility, so it was
  // copied instead; the widget offers the fix until it's granted.
  const [needsAccess, setNeedsAccess] = useState(false);
  const needsAccessRef = useRef(false);
  needsAccessRef.current = needsAccess;
  useEffect(() => {
    const refresh = () =>
      getReadiness()
        .then((checks) => setSetupProblem(checks.find((c) => c.level === "error") ?? null))
        .catch((err) => console.error("get_readiness failed:", err));
    refresh();
    const events = [
      "model-download-complete",
      "audio-device-status",
      "shortcut-changed",
      "transcript-error",
      "paste-needs-access",
      "accessibility-granted",
    ];
    const unlisteners = events.map((event) => listen(event, refresh));
    // Catches what has no event, like Accessibility being granted.
    const interval = setInterval(refresh, 30_000);
    return () => {
      clearInterval(interval);
      unlisteners.forEach((u) => u.then((f) => f()));
    };
  }, []);

  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  useEffect(() => {
    if (!flyoutHovered) return;
    invoke<string[]>("list_input_devices")
      .then(setDevices)
      .catch((err) => console.error("list_input_devices failed:", err));
    invoke<ModeSummary[]>("get_modes")
      .then(setModes)
      .catch((err) => console.error("get_modes failed:", err));
  }, [flyoutHovered]);

  useEffect(() => {
    if (phase !== "recording") {
      setLevels(SILENT_LEVELS);
      setRemoteLevels(SILENT_LEVELS);
      setDualMicLevels(null);
      setAudioBands(undefined);
    }
    if (phase !== "transcribing") setProgress(null);
    if (phase !== "refining") setRefineProgress(null);
  }, [phase]);

  useEffect(() => {
    if (!meetingRecording) {
      setRemoteLevels(SILENT_LEVELS);
      setDualMicLevels(null);
    }
  }, [meetingRecording]);

  const inFlightTranscriptionsRef = useRef(0);

  useEffect(() => {
    invoke<WidgetMode>("get_widget_mode")
      .then((m) => {
        setMode(m);
        setModeLoaded(true);
      })
      .catch((err) => {
        console.error("get_widget_mode failed:", err);
        setModeLoaded(true);
      });

    invoke<boolean>("get_isolated_voice_enabled")
      .then(setMicMode)
      .catch((err) => console.error("get_isolated_voice_enabled failed:", err));

    const loadModes = () =>
      invoke<ModeSummary[]>("get_modes")
        .then(setModes)
        .catch((err) => console.error("get_modes failed:", err));
    loadModes();
    const unlistenModes = listen("modes-changed", loadModes);

    invoke<DeviceStatus | null>("get_audio_device_status")
      .then(setDeviceStatus)
      .catch((err) => console.error("get_audio_device_status failed:", err));
    const unlistenDevice = listen<DeviceStatus>("audio-device-status", (e) => setDeviceStatus(e.payload));

    invoke<"off" | "listening" | "asleep" | "paused">("get_ambient_phase")
      .then(setAmbientPhase)
      .catch((err) => console.error("get_ambient_phase failed:", err));
    const unlistenAmbient = listen<"off" | "listening" | "asleep" | "paused">("ambient-phase", (e) => setAmbientPhase(e.payload));

    invoke<ThemeId>("get_theme")
      .then(applyTheme)
      .catch((err) => console.error("get_theme failed:", err));
    const unlistenTheme = listen<ThemeId>("theme-changed", (e) => applyTheme(e.payload));

    invoke<WaveformConfig>("get_waveform_config")
      .then(setWaveformConfig)
      .catch(() => {});
    const unlistenWaveformConfig = listen<WaveformConfig>("waveform-config-changed", (e) =>
      setWaveformConfig(e.payload),
    );

    const unlistenModeChanged = listen<WidgetMode>("widget-mode-changed", (e) => {
      clearFlyoutTimers();
      setFlyoutHovered(false);
      setMode(e.payload);
      setModeLoaded(true);
    });
    const unlistenLive = listen<string>("live-transcript", (e) => setLiveText(e.payload));
    // The meeting's own start sends "meeting"; anything else is a dictation.
    const unlistenStart = listen<string | null>("recording-started", (e) => {
      if (flashTimeout.current) {
        clearTimeout(flashTimeout.current);
        flashTimeout.current = null;
      }
      setFlashVisible(false);
      setMessage(null);
      setIsError(false);
      const isDictation = e.payload !== "meeting";
      dictatingRef.current = isDictation;
      setDictating(isDictation);
      setPaused(false);
      setLiveText("");
      setMicSilent(null);
      setMicDead(false);
      setPhase("recording");
    });
    const unlistenStop = listen("recording-stopped", () => {
      dictatingRef.current = false;
      inFlightTranscriptionsRef.current += 1;
      setDictating(false);
      setPaused(false);
      setLiveText("");
      setMicSilent(null);
      setPhase("transcribing");
    });
    const unlistenMicSilent = listen<string>("mic-silent", (e) => setMicSilent(e.payload));
    const unlistenMicCleared = listen("mic-silent-cleared", () => setMicSilent(null));
    const unlistenMicSwitched = listen<{ from: string; to: string }>("mic-switched", (e) => {
      setMicSilent(null);
      setMicNote(`Switched from ${e.payload.from} to ${e.payload.to}`);
    });
    const unlistenMicDead = listen("mic-dead", () => setMicDead(true));
    // Changed from the widget's native menu.
    const unlistenMicMode = listen<boolean>("mic-mode-changed", (e) => setMicMode(e.payload));
    const unlistenNextMode = listen<string | null>("next-mode-changed", (e) => setOverrideMode(e.payload));
    const unlistenRefining = listen<string | null>("refining-started", (e) => {
      if (dictatingRef.current && inFlightTranscriptionsRef.current > 0) return;
      setRefineModel(e.payload ?? null);
      // Stays null (an indeterminate bar) for boilerplate generation, which
      // reports no progress.
      setRefineProgress(null);
      setPhase("refining");
    });
    const unlistenRefineProgress = listen<number>("refine-progress", (e) => setRefineProgress(e.payload));
    const unlistenProgress = listen<number>("transcribe-progress", (e) => setProgress(e.payload));
    const unlistenLevel = listen<number>("audio-level", (e) => {
      // Settings' Mic Check emits global audio-level events; ignore them while
      // the floating widget isn't actively recording so idle Pill/Notch stay still.
      if (phaseRef.current !== "recording") return;
      setLevels((prev) => [...prev.slice(1), e.payload]);
    });
    const unlistenBands = listen<AudioBands>("audio-bands", (e) => {
      if (phaseRef.current !== "recording") return;
      setAudioBands(e.payload);
    });
    const unlistenDualLevel = listen<{ mic: number; system: number }>("audio-dual-level", (e) => {
      if (phaseRef.current !== "recording") return;
      setRemoteLevels((prev) => [...prev.slice(1), e.payload.system]);
      setDualMicLevels((prev) => [...(prev ?? SILENT_LEVELS).slice(1), e.payload.mic]);
    });
    const unlistenRecordingError = listen<string>("recording-error", (event) => {
      dictatingRef.current = false;
      setDictating(false);
      if (meetingRecordingRef.current) {
        setDualMicLevels(null);
        setRemoteLevels(SILENT_LEVELS);
      }
      setPhase(meetingRecordingRef.current ? "recording" : "idle");
      announce(event.payload, true);
    });
    const unlistenNeedsAccess = listen("paste-needs-access", () => {
      const wasInFlight = inFlightTranscriptionsRef.current > 0;
      if (wasInFlight) inFlightTranscriptionsRef.current -= 1;
      if (!(dictatingRef.current && wasInFlight)) {
        dictatingRef.current = false;
        setDictating(false);
        setPhase(meetingRecordingRef.current ? "recording" : "idle");
      }
      setOverrideMode(null);
      setFlashVisible(false);
      setNeedsAccess(true);
    });
    const unlistenAccessGranted = listen("accessibility-granted", () => {
      if (needsAccessRef.current) announce("Pasting is on", false);
      setNeedsAccess(false);
    });
    const unlistenTranscriptReady = listen<string>("transcript-ready", (event) => {
      const wasInFlight = inFlightTranscriptionsRef.current > 0;
      if (wasInFlight) inFlightTranscriptionsRef.current -= 1;
      setNeedsAccess(false);
      setLastPasted(event.payload || null);
      if (!(dictatingRef.current && wasInFlight)) {
        dictatingRef.current = false;
        setDictating(false);
        setPhase(meetingRecordingRef.current ? "recording" : "idle");
        announce("Pasted", false);
      }
      // The backend consumes the override the moment it's used (see
      // recording.rs's `mode_override.take()`) — clear the pill so it
      // doesn't look like it's still armed for the next dictation.
      setOverrideMode(null);
    });
    const unlistenTranscriptError = listen<string>("transcript-error", (event) => {
      const wasInFlight = inFlightTranscriptionsRef.current > 0;
      if (wasInFlight) inFlightTranscriptionsRef.current -= 1;
      if (!(dictatingRef.current && wasInFlight)) {
        dictatingRef.current = false;
        setDictating(false);
        setPhase(meetingRecordingRef.current ? "recording" : "idle");
      }
      setOverrideMode(null);
      announce(event.payload, true);
    });
    const unlistenCopied = listen<string>("transcript-copied", (event) => {
      const wasInFlight = inFlightTranscriptionsRef.current > 0;
      if (wasInFlight) inFlightTranscriptionsRef.current -= 1;
      if (!(dictatingRef.current && wasInFlight)) {
        dictatingRef.current = false;
        setDictating(false);
        setPhase(meetingRecordingRef.current ? "recording" : "idle");
        announce(event.payload, false);
      }
      setOverrideMode(null);
    });
    const unlistenDiscarded = listen<string>("dictation-discarded", (event) => {
      if (event.payload === "cancelled" && inFlightTranscriptionsRef.current > 0) {
        inFlightTranscriptionsRef.current -= 1;
      }
      dictatingRef.current = false;
      setDictating(false);
      if (meetingRecordingRef.current) {
        setDualMicLevels(null);
        setRemoteLevels(SILENT_LEVELS);
      }
      setPhase(meetingRecordingRef.current ? "recording" : "idle");
      setLiveText("");
      setMicSilent(null);
      setPaused(false);
      announce(event.payload === "cancelled" ? "Paste cancelled" : "Recording deleted", false);
    });
    const unlistenMeetingStopped = listen("meeting-stopped", () => {
      setMicSilent(null);
      setEndedCall(null);
      // A push-to-talk dictation still held as the meeting stops keeps going.
      setPhase(settledPhase);
    });
    const unlistenLimitWarning = listen<number>("meeting-limit-warning", (e) =>
      announce(`Meeting stops in ${e.payload} min (2.5-hour limit). Start a new one to keep recording.`, true),
    );
    const unlistenLimitReached = listen("meeting-limit-reached", () =>
      announce("Meeting stopped at the 2.5-hour limit and is being transcribed. Start a new one to keep recording.", true),
    );
    const unlistenMeetingSaved = listen<MeetingSummary | string>("meeting-saved", (e) => {
      const summary: MeetingSummary =
        typeof e.payload === "string"
          ? { id: "latest", title: e.payload, detail: null, state: "ready" }
          : e.payload;
      setSavedMeeting(summary);
      setSavedMeetingCopied(false);
    });
    const unlistenMeetingFailed = listen<string>("meeting-failed", (event) => announce(event.payload, true));
    const unlistenPaused = listen("dictation-paused", () => {
      setPaused(true);
      setMicSilent(null);
    });
    const unlistenResumed = listen("dictation-resumed", () => setPaused(false));
    const unlistenUpdateAvailable = listen<string>("update-available", (event) => {
      setUpdateAvailable(event.payload);
    });

    return () => {
      unlistenModes.then((f) => f());
      unlistenTheme.then((f) => f());
      unlistenWaveformConfig.then((f) => f());
      unlistenModeChanged.then((f) => f());
      unlistenStart.then((f) => f());
      unlistenLive.then((f) => f());
      unlistenMicSilent.then((f) => f());
      unlistenMicCleared.then((f) => f());
      unlistenMicSwitched.then((f) => f());
      unlistenMicDead.then((f) => f());
      unlistenMicMode.then((f) => f());
      unlistenNextMode.then((f) => f());
      unlistenStop.then((f) => f());
      unlistenRefining.then((f) => f());
      unlistenProgress.then((f) => f());
      unlistenRefineProgress.then((f) => f());
      unlistenLevel.then((f) => f());
      unlistenBands.then((f) => f());
      unlistenDualLevel.then((f) => f());
      unlistenRecordingError.then((f) => f());
      unlistenTranscriptReady.then((f) => f());
      unlistenNeedsAccess.then((f) => f());
      unlistenAccessGranted.then((f) => f());
      unlistenTranscriptError.then((f) => f());
      unlistenUpdateAvailable.then((f) => f());
      unlistenDevice.then((f) => f());
      unlistenAmbient.then((f) => f());
      unlistenCopied.then((f) => f());
      unlistenDiscarded.then((f) => f());
      unlistenPaused.then((f) => f());
      unlistenMeetingStopped.then((f) => f());
      unlistenLimitWarning.then((f) => f());
      unlistenLimitReached.then((f) => f());
      unlistenMeetingSaved.then((f) => f());
      unlistenMeetingFailed.then((f) => f());
      unlistenResumed.then((f) => f());
    };
  }, []);

  // Notch grows from the middle while it has something to show (the
  // backend re-centers it on every resize).
  const notchActive =
    phase !== "idle" ||
    meetingProcessing !== null ||
    (flashVisible && message !== null) ||
    needsAccess ||
    detectedCall !== null ||
    endedCall !== null ||
    savedMeeting !== null;
  useEffect(() => {
    if (!modeLoaded || mode !== "notch") return;
    invoke("set_widget_size", notchActive ? NOTCH_ACTIVE : NOTCH_IDLE).catch((err) =>
      console.error("set_widget_size failed:", err),
    );
  }, [modeLoaded, mode, notchActive]);

  // Suppress the hover flyout while a meeting prompt pill or pause action bar
  // is on screen so hovering to click Resume/Transcribe/Delete doesn't jump
  // the window height mid-click.
  const flyoutOpen = flyoutHovered && !paused && !detectedCall && !endedCall && !savedMeeting;

  useEffect(() => {
    if (!modeLoaded) return;
    if (mode === "caption" || mode === "detailed") {
      const base = mode === "caption" ? CAPTION_BASE_SIZE : DETAILED_BASE_SIZE;
      const size = flyoutOpen
        ? expandedFlyoutSize(base, modes.length, micRows.length, waveformConfig.show_in_flyout)
        : base;
      invoke("set_widget_size", size).catch((err) => console.error("set_widget_size failed:", err));
      return;
    }
    if (mode !== "compact" && mode !== "stacked") return;
    const extra = mode === "stacked" ? STACKED_EXTRA : 0;
    // The flyout takes priority over the error-expansion size — hovering
    // to check mic mode mid-error is a rare enough overlap that showing
    // the flyout (rather than fighting over which one "wins" the window
    // height) is the simpler, still-correct choice.
    const size = flyoutOpen
      ? compactFlyoutSize(modes.length, micRows.length, waveformConfig.show_in_flyout)
      : flashVisible && isError
        ? errorHeight
          ? { width: COMPACT_WIDTH, height: errorHeight + extra }
          : COMPACT_EXPANDED_SIZE
        : { width: COMPACT_WIDTH, height: COMPACT_BASE_SIZE.height + extra };
    invoke("set_widget_size", size).catch((err) => console.error("set_widget_size failed:", err));
  }, [modeLoaded, mode, flashVisible, isError, flyoutOpen, modes.length, micRows.length, errorHeight, waveformConfig.show_in_flyout]);

  // How tall the compact window needs to be to show the whole error.
  // Measured from the message itself (its full height, ignoring the line
  // clamp), so a short error stays small and a long one isn't cut off.
  useLayoutEffect(() => {
    const el = compactMessageRef.current;
    if ((mode !== "compact" && mode !== "stacked") || !flashVisible || !isError || !el) {
      setErrorHeight(null);
      return;
    }
    const needed = Math.ceil(el.scrollHeight) + COMPACT_ERROR_CHROME;
    setErrorHeight(Math.min(Math.max(needed, COMPACT_BASE_SIZE.height), COMPACT_ERROR_MAX_HEIGHT));
  }, [mode, flashVisible, isError, message]);

  function pickStyle(next: WidgetMode) {
    if (next === mode) return;
    clearFlyoutTimers();
    setFlyoutHovered(false);
    invoke("set_widget_mode", { mode: next }).catch(actionFailed("switch the widget style"));
  }

  function toggleRecording() {
    // In Minimal mode there is no separate pill slot, so clicking the main dot
    // while a call is detected starts the meeting recording, or opens a newly
    // saved meeting transcript.
    if (mode === "minimal" && detectedCall && phase === "idle" && !meetingRecording) {
      setDetectedCall(null);
      invoke("start_meeting").catch(actionFailed("start the meeting"));
      return;
    }
    if (mode === "minimal" && savedMeeting && phase === "idle" && !meetingRecording) {
      const id = savedMeeting.id === "latest" ? null : savedMeeting.id;
      setSavedMeeting(null);
      invoke("open_meeting_in_settings", { id }).catch(actionFailed("open the meeting transcript"));
      return;
    }
    // During push-to-talk inside a meeting, the button stops the dictation,
    // not the meeting.
    if (meetingRecording && !dictating) {
      setEndedCall(null);
      invoke("stop_meeting").catch(actionFailed("stop the meeting"));
      return;
    }
    invoke("toggle_recording_command").catch(actionFailed("start recording"));
  }

  function focusAgent(agent: WaitingAgent) {
    invoke("focus_waiting_agent", { sessionId: agent.session_id }).catch(actionFailed("go to the agent"));
  }

  // One agent: go to it. Several: a menu to pick which to reply to or open.
  function openAgent(agent: WaitingAgent) {
    if (waiting.length > 1) {
      invoke("show_agents_menu").catch(actionFailed("list the waiting agents"));
    } else {
      focusAgent(agent);
    }
  }

  function pasteAsIs() {
    invoke("paste_as_is_command").catch(actionFailed("skip refining"));
  }

  function openSettings() {
    clearFlyoutTimers();
    setFlyoutHovered(false);
    invoke("open_settings").catch(actionFailed("open Settings"));
  }

  function updateWaveformConfig(patch: Partial<WaveformConfig>) {
    const next: WaveformConfig = { ...waveformConfig, ...patch };
    setWaveformConfig(next);
    invoke("set_waveform_config", { config: next }).catch(() => {});
  }

  function toggleMicMode(next: boolean) {
    setMicMode(next);
    invoke("set_isolated_voice_enabled", { enabled: next }).catch((err) => {
      setMicMode(!next);
      actionFailed("switch mic mode")(err);
    });
  }

  function pickMic(name: string | null) {
    invoke("set_input_device", { name }).catch(actionFailed("switch microphone"));
  }

  function pickOverride(next: string) {
    // Clicking the already-selected pill clears it back to "auto" rather
    // than being stuck — this is a one-off nudge for the very next
    // dictation, not a persistent setting.
    const nextValue = overrideMode === next ? null : next;
    setOverrideMode(nextValue);
    invoke("set_next_mode_override", { modeName: nextValue }).catch(actionFailed("set the next mode"));
  }

  function startDrag(e: React.MouseEvent) {
    if (e.buttons === 1) {
      clearFlyoutTimers();
      setFlyoutHovered(false);
      getCurrentWindow()
        .startDragging()
        .catch((err) => console.error("startDragging failed:", err));
    }
  }

  const busy = phase === "transcribing" || phase === "refining";
  // Which processing stage to draw, if any. Always activate during
  // transcribing so short dictations (<3s, where transcribe-progress never
  // emits a number) still render the animated two-stage pipeline bar.
  const activeStage: { stage: Stage; percent: number | null; label?: string } | null =
    phase === "transcribing"
      ? { stage: "transcribe", percent: progress }
      : phase === "refining"
        ? { stage: "refine", percent: refineProgress }
        : null;
  const showProgress = activeStage !== null;
  const stageTitle =
    phase === "refining"
      ? `${refineModel ? `Cleaning up with ${refineModel}. ` : ""}Click to paste as-is${skipKey ? ` (${skipKey})` : ""}.`
      : undefined;
  const phaseLabel =
    meetingRecording && phase === "recording" && !dictating
      ? "Recording a meeting"
      : agentQuestion !== null && phase === "recording"
        ? agentQuestion
          ? `Agent asks: ${agentQuestion}`
          : "Answering an agent"
        : STATUS_LABEL[phase];
  // The meeting stops itself at its limit (2.5 h): count down to it for the
  // last 10 minutes so the cut-off is never a surprise.
  const meetingRemainingMs =
    meetingRecording?.limit_ms != null && !dictating ? meetingRecording.limit_ms - elapsedMs : null;
  const meetingEndingSoon = meetingRemainingMs !== null && meetingRemainingMs <= MEETING_WARNING_MS;
  const elapsedText =
    meetingRecording && !dictating
      ? (meetingNote ??
        (meetingEndingSoon
          ? `Ends in ${formatElapsed(Math.max(0, meetingRemainingMs!))}`
          : `Meeting ${formatElapsed(elapsedMs)}`))
      : micNote
        ? "Mic switched"
        : formatElapsed(elapsedMs);
  const idleLabel =
    phase !== "idle"
      ? phaseLabel
      : meetingProcessing
        ? `Meeting: ${meetingProcessing.step}${meetingProcessing.percent !== null ? ` ${meetingProcessing.percent}%` : "…"}`
        : ambientPhase === "listening"
        ? "Listening for “Mouthpiece”"
        : ambientPhase === "asleep"
          ? "Asleep"
          : ambientPhase === "paused"
            ? "Paused (mic in use)"
            : STATUS_LABEL.idle;
  const flashing = flashVisible && message !== null;
  const showSetup = phase === "idle" && !flashing && (needsAccess || setupProblem !== null);

  useEffect(() => {
    invoke<boolean>("get_widget_only_while_dictating")
      .then(setOnlyWhileDictating)
      .catch((err) => console.error("get_widget_only_while_dictating failed:", err));
    const unlisten = listen<boolean>("widget-only-while-dictating-changed", (e) => {
      hideWhenDone.current = e.payload;
      setOnlyWhileDictating(e.payload);
    });
    const root = document.documentElement;
    const enter = () => setPointerInside(true);
    const leave = () => setPointerInside(false);
    root.addEventListener("mouseenter", enter);
    root.addEventListener("mouseleave", leave);
    return () => {
      unlisten.then((f) => f());
      root.removeEventListener("mouseenter", enter);
      root.removeEventListener("mouseleave", leave);
    };
  }, []);
  useEffect(() => {
    if (
      phase !== "idle" ||
      meetingProcessing !== null ||
      detectedCall !== null ||
      endedCall !== null ||
      savedMeeting !== null
    ) {
      hideWhenDone.current = true;
    }
  }, [phase, meetingProcessing, detectedCall, endedCall, savedMeeting]);
  // A paste hides the widget almost at once; an error or note stays up
  // until it's had its time, as do "Allow pasting" and
  // anything the pointer is resting on.
  const justPasted = flashing && !isError && message === "Pasted";
  const dictationOver =
    phase === "idle" &&
    !meetingRecording &&
    !meetingProcessing &&
    !endedCall &&
    !savedMeeting &&
    (!flashing || justPasted) &&
    !needsAccess &&
    !detectedCall &&
    !pointerInside;
  useEffect(() => {
    if (!onlyWhileDictating || !dictationOver || !hideWhenDone.current) return;
    const timer = setTimeout(
      () => {
        hideWhenDone.current = false;
        invoke("hide_idle_widget").catch((err) => console.error("hide_idle_widget failed:", err));
      },
      justPasted ? 600 : 300,
    );
    return () => clearTimeout(timer);
  }, [onlyWhileDictating, dictationOver, justPasted]);
  // Pasting is fixed right here (macOS's prompt, then its Accessibility
  // list); anything else opens the setup checklist in Settings.
  const pasteProblem = needsAccess || setupProblem?.id === "paste";
  const narrow = mode === "pill" || mode === "notch";
  const setupLabel =
    needsAccess && !narrow ? "Copied · Allow pasting" : pasteProblem ? "Allow pasting" : "Finish setup";
  const setupTitle = pasteProblem
    ? `${needsAccess ? "Your words are on the clipboard: press ⌘V to paste them. " : ""}Mouthpiece needs Accessibility permission to paste for you. Click to allow it. If Mouthpiece is already on in the list, switch it off and on again: macOS asks again after an update.`
    : setupProblem
      ? `${setupProblem.title}. ${setupProblem.detail}`
      : undefined;
  function fixSetup() {
    if (pasteProblem) {
      invoke("request_accessibility").catch(actionFailed("open the Accessibility settings"));
    } else {
      openSettings();
    }
  }

  const setupButton = (
    <button
      type="button"
      className={`min-w-0 flex-1 truncate text-left font-medium hover:underline ${
        mode === "pill" || mode === "notch" || mode === "caption" ? "text-xs" : "text-sm"
      } ${needsAccess ? "text-warning" : "text-error"}`}
      onClick={fixSetup}
      title={setupTitle}
      data-testid="widget-setup"
    >
      {setupLabel}
    </button>
  );

  // The mic isn't sending anything: offer the others, the Mac's own first.
  const showSwitchMic = (phase === "recording" && micSilent !== null) || (micDead && flashing && isError);
  const switchMicButton = (
    <button
      type="button"
      className="btn btn-warning btn-xs h-5 min-h-0 shrink-0 rounded-full px-2 text-[11px]"
      onClick={() => invoke("show_mic_menu").catch(actionFailed("list the mics"))}
      onMouseDown={(e) => e.stopPropagation()}
      title="Pick another microphone"
      data-testid="switch-mic"
    >
      Switch mic
    </button>
  );

  // The widget's menu (mic, mic mode, next dictation, style, Settings) as a
  // native menu, which can draw outside the widget: every style has it on
  // right-click, and those with room (not Minimal's dot) on a ⌄ button.
  function openMenu(e?: React.MouseEvent) {
    e?.preventDefault();
    clearFlyoutTimers();
    setFlyoutHovered(false);
    invoke("show_widget_menu").catch(actionFailed("open the menu"));
  }
  const menuButton = (className = "") => (
    <button
      type="button"
      className={`btn btn-ghost btn-circle h-6 w-6 min-h-0 shrink-0 ${className}`}
      onClick={() => openMenu()}
      onMouseDown={(e) => e.stopPropagation()}
      aria-label="Widget menu"
      title="Microphone, mode and style"
      data-testid="widget-menu"
    >
      <ChevronIcon />
    </button>
  );

  // A coding agent is waiting (newest first). Replaces the idle label; the
  // click brings that agent's window forward.
  const newestAgent = waiting[0];
  const agentTitle = newestAgent
    ? `${newestAgent.kind === "done" ? "Finished" : newestAgent.message || "Needs your input"}${
        newestAgent.app_name ? ` (${newestAgent.app_name})` : ""
      }. ${waiting.length > 1 ? `${waiting.length} agents are waiting. Click to pick one` : "Click to go to it"}${
        replyKey ? `, or press ${replyKey} to reply to the newest by voice` : ""
      }.${addressing ? ` Start any dictation with "to ${spokenFolder(newestAgent.folder)}," to send it there.` : ""}`
    : undefined;
  const agentPill =
    newestAgent && phase === "idle" && !flashing ? (
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-1.5 truncate rounded-full bg-info/20 px-2 py-0.5 text-left text-xs font-medium text-info hover:bg-info/30"
        onClick={() => openAgent(newestAgent)}
        title={agentTitle}
        data-testid="agent-pill"
      >
        <span className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-info motion-reduce:animate-none" />
        <span className="truncate">
          {newestAgent.agent || "Claude"} is waiting · {newestAgent.folder}
          {waiting.length > 1 ? ` +${waiting.length - 1}` : ""}
        </span>
      </button>
    ) : null;

  // While answering an agent's question, the question replaces the
  // waveform so it's on screen the whole time you answer.
  const agentQuestionLine =
    agentQuestion !== null && phase === "recording" ? (
      <span
        className="min-w-0 flex-1 truncate text-xs font-medium text-info"
        title={agentQuestion || undefined}
        data-testid="agent-question"
      >
        {agentQuestion ? `Agent asks: ${agentQuestion}` : "Answering an agent"}
      </span>
    ) : null;

  const meetingDetectPill =
    detectedCall && phase === "idle" && !meetingRecording && !flashing ? (
      <span
        className="flex min-w-0 flex-1 items-center gap-1 rounded-full bg-primary/20 pl-2 pr-1 py-0.5 text-xs font-medium text-primary"
        onMouseDown={(e) => e.stopPropagation()}
        data-testid="meeting-detect-prompt"
      >
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1.5 truncate text-left hover:underline"
          onClick={() => {
            setDetectedCall(null);
            invoke("start_meeting").catch((err) => {
              actionFailed("start the meeting")(err);
              // The backend still counts the prompt as shown and won't send
              // it again for this call, so put it back.
              invoke<DetectedCall | null>("get_detected_meeting")
                .then((call) => setDetectedCall(call))
                .catch(() => {});
            });
          }}
          onMouseDown={(e) => e.stopPropagation()}
          title={`Record ${detectedCall.app_name} meeting`}
          data-testid="meeting-detect-start"
        >
          <span className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-error motion-reduce:animate-none" />
          <span className="truncate">Record {detectedCall.app_name}?</span>
        </button>
        <button
          type="button"
          className="btn btn-ghost btn-circle h-4 w-4 min-h-0 shrink-0 opacity-70 hover:opacity-100"
          onClick={() => {
            setDetectedCall(null);
            invoke("dismiss_meeting_prompt").catch(() => {});
          }}
          onMouseDown={(e) => e.stopPropagation()}
          aria-label="Dismiss meeting prompt"
          title="Dismiss"
          data-testid="meeting-detect-dismiss"
        >
          <svg aria-hidden="true" viewBox="0 0 12 12" width="8" height="8">
            <path d="M2.5 2.5l7 7m0-7l-7 7" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
          </svg>
        </button>
      </span>
    ) : null;

  const meetingEndedPill =
    endedCall && meetingRecording ? (
      <button
        type="button"
        className="btn btn-error btn-xs h-5 min-h-0 shrink-0 rounded-full px-2 text-[11px]"
        onClick={() => {
          setEndedCall(null);
          invoke("stop_meeting").catch(actionFailed("stop the meeting"));
        }}
        onMouseDown={(e) => e.stopPropagation()}
        title={`${endedCall.app_name} call ended — click to stop and transcribe`}
        data-testid="meeting-ended-prompt"
      >
        Call ended · Stop?
      </button>
    ) : null;

  const meetingSavedPill =
    savedMeeting && phase === "idle" && !meetingRecording && !meetingProcessing && !flashing ? (
      <span
        className="flex min-w-0 flex-1 items-center gap-1 rounded-full bg-success/20 pl-2 pr-1 py-0.5 text-xs font-medium text-success"
        onMouseDown={(e) => e.stopPropagation()}
        data-testid="meeting-saved-prompt"
      >
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1.5 truncate text-left hover:underline"
          onClick={() => {
            const id = savedMeeting.id === "latest" ? null : savedMeeting.id;
            setSavedMeeting(null);
            invoke("open_meeting_in_settings", { id }).catch(actionFailed("open the meeting transcript"));
          }}
          onMouseDown={(e) => e.stopPropagation()}
          title={`${savedMeeting.title}${savedMeeting.detail ? ` (${savedMeeting.detail})` : ""} — click to view in Settings`}
          data-testid="meeting-saved-view"
        >
          <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-success" />
          <span className="truncate">{narrow ? "Saved · View" : "Meeting transcript saved · View"}</span>
        </button>
        {savedMeeting.id !== "latest" && mode !== "pill" && (
          <button
            type="button"
            className="shrink-0 rounded-full bg-success/20 px-1.5 py-0.5 text-[10px] font-semibold hover:bg-success/30"
            onClick={() => {
              const id = savedMeeting.id;
              // Rust writes the clipboard: this window never takes focus, so
              // navigator.clipboard would be refused.
              invoke("copy_meeting", { id })
                .then(() => {
                  setSavedMeetingCopied(true);
                  setTimeout(() => setSavedMeeting((cur) => (cur?.id === id ? null : cur)), 1200);
                })
                .catch((err) => {
                  if (String(err).toLowerCase().includes("locked")) {
                    setSavedMeeting(null);
                    invoke("open_meeting_in_settings", { id }).catch(actionFailed("open the meeting transcript"));
                  } else {
                    actionFailed("copy the meeting transcript")(err);
                  }
                });
            }}
            onMouseDown={(e) => e.stopPropagation()}
            title="Copy transcript to clipboard"
            data-testid="meeting-saved-copy"
          >
            {savedMeetingCopied ? "Copied" : "Copy"}
          </button>
        )}
        <button
          type="button"
          className="btn btn-ghost btn-circle h-4 w-4 min-h-0 shrink-0 opacity-70 hover:opacity-100"
          onClick={() => setSavedMeeting(null)}
          onMouseDown={(e) => e.stopPropagation()}
          aria-label="Dismiss saved meeting prompt"
          title="Dismiss"
          data-testid="meeting-saved-dismiss"
        >
          <svg aria-hidden="true" viewBox="0 0 12 12" width="8" height="8">
            <path d="M2.5 2.5l7 7m0-7l-7 7" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
          </svg>
        </button>
      </span>
    ) : null;

  const isPaused = paused && phase === "recording";
  // ⏸ while dictating (same as Esc); ✕ while the paste is being prepared,
  // which copies the text instead of pasting it. A meeting has its own stop.
  const discardButton =
    phase === "idle" || isPaused || (meetingRecording && !dictating && !busy) ? null : phase === "recording" ? (
      <button
        type="button"
        className="dw-icon-btn"
        onClick={() => invoke("pause_dictation").catch(actionFailed("pause the recording"))}
        aria-label="Pause recording (Esc)"
        title="Pause recording (Esc)"
        data-testid="pause-dictation"
      >
        <svg aria-hidden="true" viewBox="0 0 12 12" width="10" height="10">
          <path d="M4 2.5v7m4-7v7" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      </button>
    ) : (
      <button
        type="button"
        className="dw-icon-btn"
        onClick={() => invoke("cancel_paste_command").catch(actionFailed("cancel the paste"))}
        aria-label="Cancel paste (copies the text instead)"
        title="Cancel paste (copies the text instead)"
        data-testid="cancel-paste"
      >
        <svg aria-hidden="true" viewBox="0 0 12 12" width="10" height="10">
          <path d="M2.5 2.5l7 7m0-7l-7 7" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      </button>
    );

  // Paused with Esc: what happens next. The narrow styles get icons, with
  // the keys in each tooltip; the roomier ones spell the keys out below.
  const transcribeKey = hotkey ?? "the hotkey";
  const pausedControls = isPaused ? (
    <span
      className="flex min-w-0 flex-1 items-center justify-end gap-1"
      onMouseDown={(e) => e.stopPropagation()}
      role="group"
      aria-label="Recording paused"
      data-testid="paused-controls"
    >
      <button
        type="button"
        className="dw-choice-btn"
        onClick={() => invoke("resume_dictation").catch(actionFailed("resume the recording"))}
        title="Keep recording"
        aria-label="Resume"
        data-testid="paused-resume"
      >
        {narrow ? (
          <svg aria-hidden="true" viewBox="0 0 12 12" width="9" height="9">
            <path d="M3.5 2.2v7.6L9.6 6z" fill="currentColor" />
          </svg>
        ) : (
          "Resume"
        )}
      </button>
      <button
        type="button"
        className="dw-choice-btn dw-choice-primary"
        onClick={toggleRecording}
        title={`Transcribe and paste what you said (${transcribeKey})`}
        aria-label="Transcribe"
        data-testid="paused-transcribe"
      >
        {narrow ? (
          <svg aria-hidden="true" viewBox="0 0 12 12" width="10" height="10">
            <path d="M2.5 6.3l2.4 2.4 4.6-5" stroke="currentColor" strokeWidth="1.7" fill="none" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        ) : (
          "Transcribe"
        )}
      </button>
      <button
        type="button"
        className="dw-choice-btn dw-choice-danger"
        onClick={() => invoke("delete_dictation").catch(actionFailed("delete the recording"))}
        title="Delete this recording (Esc)"
        aria-label="Delete"
        data-testid="paused-delete"
      >
        {narrow ? (
          <svg aria-hidden="true" viewBox="0 0 12 12" width="10" height="10">
            <path d="M2.5 3.5h7M5 3.5V2.4h2v1.1M3.5 3.5l.5 6.1h4l.5-6.1" stroke="currentColor" strokeWidth="1.3" fill="none" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        ) : (
          "Delete"
        )}
      </button>
    </span>
  ) : null;
  const pausedHint = `Paused, nothing is being recorded. Press ${transcribeKey} to transcribe what you said, Esc to delete it, or Resume to keep going.`;

  const recordButton = (
    <button
      className={`btn btn-circle border-none disabled:opacity-100 disabled:bg-white/10 ${mode === "minimal" ? "btn-md" : "btn-sm"} ${
        phase === "recording" ? "bg-white/15" : "bg-white/8 hover:bg-white/15"
      } ${
        (flashing && isError) || ((showSetup || endedCall) && mode === "minimal")
          ? "ring-2 ring-error"
          : (warning || micSilent) && mode === "minimal"
            ? "ring-2 ring-warning"
            : detectedCall && mode === "minimal"
              ? "ring-2 ring-primary"
              : savedMeeting && mode === "minimal"
                ? "ring-2 ring-success"
                : newestAgent && mode === "minimal"
                  ? "ring-2 ring-info"
                  : ""
      }`}
      onClick={toggleRecording}
      disabled={busy}
      aria-label={
        mode === "minimal" && detectedCall && phase === "idle" && !meetingRecording
          ? `Record ${detectedCall.app_name} meeting`
          : mode === "minimal" && savedMeeting && phase === "idle" && !meetingRecording
            ? `View ${savedMeeting.title} transcript`
            : meetingRecording && !dictating
              ? "Stop the meeting"
              : phase === "idle"
                ? "Start recording"
                : isPaused
                  ? "Transcribe"
                : "Stop recording"
      }
      title={
        (isPaused ? pausedHint : null) ??
        micSilent ??
        message ??
        (showSetup ? (setupTitle ?? null) : null) ??
        warning?.full ??
        (endedCall && mode === "minimal"
          ? `${endedCall.app_name} call ended — click to stop and transcribe`
          : null) ??
        (detectedCall && mode === "minimal"
          ? `Record ${detectedCall.app_name}? Click to start meeting recording`
          : null) ??
        (savedMeeting && mode === "minimal"
          ? `Meeting transcript saved — click to view ${savedMeeting.title}`
          : null) ??
        (newestAgent && mode === "minimal"
          ? `${newestAgent.agent || "Claude"} is waiting · ${newestAgent.folder}${
              waiting.length > 1 ? ` +${waiting.length - 1}` : ""
            }`
          : null) ??
        (micName ? `${phaseLabel} · Mic: ${micName}` : phaseLabel)
      }
    >
      {justPasted ? (
        <span
          className={`inline-flex items-center justify-center rounded-full ${
            mode === "minimal" ? "h-4 w-4" : "h-3.5 w-3.5"
          }`}
          style={{
            backgroundColor: "#10b981",
            boxShadow: "0 0 12px rgba(16, 185, 129, 0.85)",
            color: "#042016",
          }}
          aria-hidden="true"
        >
          <svg
            viewBox="0 0 12 12"
            className="h-2.5 w-2.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.3"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M2.4 6.2 4.9 8.7 9.6 3.5" />
          </svg>
        </span>
      ) : (
        <span
          className={`rounded-full transition-transform duration-100 ${
            mode === "minimal" ? "h-3.5 w-3.5" : "h-2.5 w-2.5"
          } ${
            phase === "transcribing" || phase === "refining"
              ? "dw-orb-pulse"
              : isPaused
                ? "bg-warning shadow-[0_0_8px] shadow-warning/60"
                : phase === "recording"
                  ? "bg-error shadow-[0_0_12px] shadow-error"
                  : "bg-neutral-content/45"
          }`}
          // Minimal mode has no room for a multi-bar meter, so instead the
          // dot itself pulses with the latest input level — same "is audio
          // actually coming through" signal, scaled to fit a 46x46 icon.
          style={
            phase === "transcribing"
              ? {
                  backgroundColor: "var(--dw-accent)",
                  boxShadow: "0 0 12px var(--dw-accent)",
                }
              : phase === "refining"
                ? {
                    backgroundColor: "var(--dw-refine)",
                    boxShadow: "0 0 14px var(--dw-refine)",
                  }
                : mode === "minimal" && phase === "recording"
                  ? { transform: `scale(${1 + (levels[levels.length - 1] ?? 0) * 0.6})` }
                  : undefined
          }
        />
      )}
    </button>
  );

  // One short status for the small styles.
  const shortStatus =
    phase === "recording"
      ? isPaused
        ? "Paused"
        : micSilent
        ? "No sound from mic"
        : elapsedText
      : phase === "transcribing"
        ? `Transcribing${progress !== null ? ` ${progress}%` : "…"}`
        : phase === "refining"
          ? "Refining…"
          : flashing
            ? (message ?? "")
            : showSetup
              ? setupLabel
              : idleLabel;
  const statusTone =
    flashing && isError ? "text-error" : micSilent || (showSetup && !flashing) ? "text-warning" : "";
  const fullStatus =
    micSilent ??
    (flashing ? message : null) ??
    (phase === "recording" ? micNote : null) ??
    (showSetup ? (setupTitle ?? null) : null) ??
    (micName ? `${phaseLabel} · Mic: ${micName}` : phaseLabel);
  const dualActive = meetingRecording !== null && !dictating;
  const meterLevels = dualActive && dualMicLevels ? dualMicLevels : levels;
  const meterRemoteLevels = dualActive ? remoteLevels : undefined;
  const widgetPhase = justPasted ? "pasted" : phase;
  const activeColorway = getWaveformColorway(waveformConfig.colorway);

  const flyoutPanel = (
    <div className="dw-fly-body dw-scroll" data-testid="widget-flyout">
      <div className="dw-fly-label">Microphone</div>
      <div className="dw-fly-col">
        {micRows.map((row) => (
          <button
            key={row.key}
            type="button"
            className={`dw-fly-btn dw-fly-check ${row.active ? "sel" : ""}`}
            disabled={row.disabled}
            onClick={() => pickMic(row.name)}
            title={row.label}
            aria-pressed={row.active}
          >
            <span className="truncate">{row.label}</span>
          </button>
        ))}
      </div>
      <div className="dw-fly-label">Mic mode</div>
      <div className="dw-fly-row dw-seg" role="group" aria-label="Mic mode">
        <button
          type="button"
          className={`dw-fly-btn ${!micMode ? "sel" : ""}`}
          onClick={() => toggleMicMode(false)}
        >
          Standard
        </button>
        <button
          type="button"
          className={`dw-fly-btn ${micMode ? "sel" : ""}`}
          onClick={() => toggleMicMode(true)}
        >
          Voice Isolation
        </button>
      </div>
      <div className="dw-fly-label">Next dictation</div>
      <div className="dw-fly-grid">
        {modes.map((m) => (
          <button
            key={m.name}
            type="button"
            className={`dw-fly-btn ${overrideMode === m.name ? "sel" : ""}`}
            onClick={() => pickOverride(m.name)}
            title={overrideMode === m.name ? "Click again to use the auto-detected mode" : undefined}
          >
            {m.name}
          </button>
        ))}
      </div>
      {waveformConfig.show_in_flyout && (
        <>
          <div className="flex items-center justify-between gap-2">
            <span className="dw-fly-label">Waveform</span>
            <span
              className="inline-flex min-w-0 items-center gap-1.5 text-[10px] text-white/55"
              title={`Colorway: ${activeColorway.name}`}
              data-testid="waveform-colorway-indicator"
            >
              <span
                className="inline-block h-2 w-2 shrink-0 rounded-full border border-white/25"
                style={{ background: activeColorway.swatchBg }}
                aria-hidden="true"
              />
              <span className="truncate">{activeColorway.name}</span>
            </span>
          </div>
          <div className="dw-fly-row dw-seg" role="group" aria-label="Waveform" data-testid="waveform-picker">
            {WAVEFORM_ENGINE_META.map((engine) => (
              <button
                key={engine.id}
                type="button"
                className={`dw-fly-btn !px-1 !py-1 !text-[10.5px] ${waveformConfig.engine === engine.id ? "sel" : ""}`}
                onClick={() => updateWaveformConfig({ engine: engine.id })}
                title={engine.title}
                aria-pressed={waveformConfig.engine === engine.id}
              >
                {FLYOUT_ENGINE_LABEL[engine.id]}
              </button>
            ))}
          </div>
          <div className="flex items-center justify-between gap-1.5" data-testid="flyout-colorway-picker">
            <div className="flex items-center gap-1" role="group" aria-label="Waveform color">
              {FLYOUT_SPECTRUM_COLORWAYS.map((cw) => {
                const active = waveformConfig.colorway === cw.id;
                return (
                  <button
                    key={cw.id}
                    type="button"
                    className={`dw-fly-orb ${active ? "sel" : ""}`}
                    style={{
                      background: cw.swatchBg,
                      ["--cw-ring" as string]: cw.bright,
                    }}
                    onClick={() => updateWaveformConfig({ colorway: cw.id })}
                    title={cw.name}
                    aria-label={cw.name}
                    aria-pressed={active}
                  />
                );
              })}
              <span className="dw-fly-orb-sep" aria-hidden="true" />
              {FLYOUT_SOLID_COLORWAYS.map((cw) => {
                const active = waveformConfig.colorway === cw.id;
                return (
                  <button
                    key={cw.id}
                    type="button"
                    className={`dw-fly-orb ${active ? "sel" : ""}`}
                    style={{
                      background: cw.swatchBg,
                      ["--cw-ring" as string]: cw.bright,
                    }}
                    onClick={() => updateWaveformConfig({ colorway: cw.id })}
                    title={cw.name}
                    aria-label={cw.name}
                    aria-pressed={active}
                  />
                );
              })}
            </div>
            <button
              type="button"
              className="dw-fly-tune shrink-0"
              onClick={openSettings}
              title="Tune waveform in Settings"
            >
              Tune…
            </button>
          </div>
        </>
      )}
      <div className="dw-fly-label">Style</div>
      <div className="dw-fly-grid" data-testid="style-picker">
        {WIDGET_STYLES.map((style) => (
          <button
            key={style.id}
            type="button"
            className={`dw-fly-btn ${mode === style.id ? "sel" : ""}`}
            onClick={() => pickStyle(style.id)}
            aria-pressed={mode === style.id}
          >
            {style.label}
          </button>
        ))}
      </div>
    </div>
  );

  if (mode === "pill") {
    return (
      <main
        onContextMenu={openMenu}
        data-phase={widgetPhase}
        className="dw-widget group flex h-screen w-screen cursor-grab items-center gap-2 overflow-hidden rounded-full px-3 text-neutral-content active:cursor-grabbing"
        onMouseDown={startDrag}
        title={fullStatus ?? undefined}
      >
        <span className="dw-small-record">{recordButton}</span>
        <div className="flex h-6 min-w-0 flex-1 items-center">
          {isPaused ? (
            <>
              <span className="shrink-0 text-xs text-warning">Paused</span>
              {pausedControls}
            </>
          ) : showProgress ? (
            <ThinProgress stage={activeStage!.stage} percent={activeStage!.percent} />
          ) : showSetup ? (
            setupButton
          ) : meetingDetectPill ? (
            meetingDetectPill
          ) : meetingSavedPill ? (
            meetingSavedPill
          ) : meetingEndedPill ? (
            meetingEndedPill
          ) : showSwitchMic ? (
            switchMicButton
          ) : justPasted ? (
            <PastedBadge className="text-xs" />
          ) : flashing ? (
            <span className={`truncate text-xs ${statusTone}`}>{shortStatus}</span>
          ) : (
            <>
              {(phase === "recording" ? agentQuestionLine : null) ?? (
                <LevelMeter
                  levels={meterLevels}
                  remoteLevels={meterRemoteLevels}
                  bands={audioBands}
                  config={waveformConfig}
                  dualChannel={dualActive}
                  className={`flex-1 group-hover:hidden ${phase === "idle" ? "opacity-40 grayscale" : ""}`}
                />
              )}
              <span className="hidden truncate text-xs opacity-75 group-hover:inline">{shortStatus}</span>
            </>
          )}
        </div>
        {discardButton}
        {!isPaused &&
          !meetingDetectPill &&
          !meetingSavedPill &&
          !meetingEndedPill &&
          menuButton("hidden group-hover:inline-flex")}
      </main>
    );
  }

  if (mode === "notch") {
    return (
      <main
        onContextMenu={openMenu}
        data-phase={widgetPhase}
        className="dw-notch-wrap group h-screen w-screen"
        title={fullStatus ?? undefined}
      >
        <svg
          className="dw-notch-ear -mr-px"
          width="10"
          height="10"
          viewBox="0 0 10 10"
          aria-hidden="true"
        >
          <path d="M0 0 H10 V10 A10 10 0 0 0 0 0 Z" fill="currentColor" />
        </svg>
        <div
          data-phase={widgetPhase}
          className={`dw-widget dw-notch flex h-full min-w-0 flex-1 items-center overflow-hidden text-neutral-content ${
            notchActive ? "gap-2 px-2" : "justify-center gap-1.5 px-3"
          }`}
        >
          {notchActive ? (
            <>
              <span className="dw-small-record">{recordButton}</span>
              {isPaused ? (
                <span className="shrink-0 text-xs text-warning">Paused</span>
              ) : showSetup ? (
                setupButton
              ) : meetingDetectPill ? (
                meetingDetectPill
              ) : meetingSavedPill ? (
                meetingSavedPill
              ) : justPasted ? (
                <PastedBadge className="shrink-0 text-xs" />
              ) : (
                <span className={`min-w-0 shrink truncate text-xs ${statusTone || "opacity-80"}`}>{shortStatus}</span>
              )}
              {!meetingDetectPill && !meetingSavedPill && (
                <div className="flex h-6 min-w-0 flex-1 items-center">
                  {isPaused ? (
                    pausedControls
                  ) : showProgress ? (
                    <ThinProgress stage={activeStage!.stage} percent={activeStage!.percent} />
                  ) : phase === "recording" ? (
                    (agentQuestionLine ?? (
                      <LevelMeter
                        levels={meterLevels}
                        remoteLevels={meterRemoteLevels}
                        bands={audioBands}
                        config={waveformConfig}
                        dualChannel={dualActive}
                        className="flex-1"
                      />
                    ))
                  ) : null}
                </div>
              )}
              {meetingEndedPill}
              {showSwitchMic && switchMicButton}
              {discardButton}
              {menuButton("opacity-0 group-hover:opacity-100 focus-visible:opacity-100")}
            </>
          ) : (
            <button type="button" className="dw-notch-idle" onClick={toggleRecording} aria-label="Start recording">
              <span className={`h-1.5 w-1.5 rounded-full ${showSetup ? "bg-warning" : "bg-neutral-content/50"}`} />
              <LevelMeter levels={SILENT_LEVELS} config={waveformConfig} className="w-10 opacity-40 grayscale" />
            </button>
          )}
        </div>
        <svg
          className="dw-notch-ear -ml-px"
          width="10"
          height="10"
          viewBox="0 0 10 10"
          aria-hidden="true"
        >
          <path d="M10 0 H0 V10 A10 10 0 0 1 10 0 Z" fill="currentColor" />
        </svg>
      </main>
    );
  }

  if (mode === "caption") {
    const body =
      isPaused ? (
        <span className="opacity-70" data-testid="paused-hint">
          {pausedHint}
        </span>
      ) : phase === "recording" ? (
        micSilent ? (
          <span className="text-warning">{micSilent}</span>
        ) : liveText ? (
          <span data-testid="live-transcript">{renderRichCaption(liveTail(liveText))}</span>
        ) : meetingRecording && !dictating ? (
          <span className="opacity-60">Recording a meeting. Stop it here or from the menu bar.</span>
        ) : (
          <span className="opacity-40">Listening…</span>
        )
      ) : busy ? (
        <span className="opacity-60">{shortStatus}</span>
      ) : flashing && isError ? (
        <span className="text-error">{message}</span>
      ) : lastPasted ? (
        <span className="opacity-75" data-testid="last-pasted">
          {renderRichCaption(lastPasted)}
        </span>
      ) : (
        <span className="opacity-40">
          Press {hotkey ?? "the hotkey"} and start talking. Your words show up here as you speak.
        </span>
      );
    return (
      <main
        onContextMenu={openMenu}
        data-phase={widgetPhase}
        className="dw-widget flex h-screen flex-col gap-1.5 overflow-hidden rounded-2xl p-2.5 text-neutral-content"
        onMouseEnter={handleFlyoutMouseEnter}
        onMouseLeave={handleFlyoutMouseLeave}
      >
        <div className="flex items-center gap-2">
          {recordButton}
          <div
            className={`flex min-w-0 flex-1 items-center gap-2 ${showProgress ? "min-h-6" : "h-6 cursor-grab active:cursor-grabbing"}`}
            onMouseDown={showProgress ? undefined : startDrag}
          >
            {isPaused ? (
              <>
                <span className="shrink-0 text-xs text-warning">Paused</span>
                {pausedControls}
              </>
            ) : phase === "recording" && !micSilent ? (
              <>
                {agentQuestionLine ?? (
                  <LevelMeter
                    levels={meterLevels}
                    remoteLevels={meterRemoteLevels}
                    bands={audioBands}
                    config={waveformConfig}
                    dualChannel={dualActive}
                    className="flex-1"
                  />
                )}
                <span className="shrink-0 font-mono text-[11px] tabular-nums opacity-60">{shortStatus}</span>
              </>
            ) : showProgress ? (
              <StageProgress
                stage={activeStage!.stage}
                percent={activeStage!.percent}
                label={activeStage!.label}
                title={stageTitle}
                onMouseDown={startDrag}
                onSkip={activeStage!.stage === "refine" ? pasteAsIs : undefined}
              />
            ) : showSetup ? (
              setupButton
            ) : meetingDetectPill ? (
              meetingDetectPill
            ) : meetingSavedPill ? (
              meetingSavedPill
            ) : justPasted ? (
              <PastedBadge className="text-xs" />
            ) : (
              <span className={`shrink-0 truncate text-xs ${statusTone || "opacity-75"}`}>
                {phase === "idle" && !flashing && !meetingProcessing && lastPasted ? "Pasted" : shortStatus}
              </span>
            )}
          </div>
          {meetingEndedPill}
          {showSwitchMic && switchMicButton}
          {discardButton}
          {menuButton()}
          <button
            className="btn btn-ghost btn-circle relative h-7 w-7 min-h-0"
            onClick={openSettings}
            aria-label={updateAvailable ? `Open settings — update to v${updateAvailable} available` : "Open settings"}
          >
            <GearIcon />
            {updateAvailable && <span className="absolute right-0.5 top-0.5 h-2 w-2 rounded-full bg-accent" />}
          </button>
        </div>
        <p
          className={`dw-caption-text text-sm leading-snug ${flyoutOpen ? "line-clamp-2 shrink-0" : "line-clamp-3 flex-1"}`}
          onMouseDown={startDrag}
        >
          {body}
        </p>
        {flyoutOpen && flyoutPanel}
      </main>
    );
  }

  if (mode === "minimal") {
    return (
      <main
        onContextMenu={openMenu}
        data-phase={widgetPhase}
        className="dw-widget relative flex h-screen w-screen cursor-grab items-center justify-center overflow-hidden rounded-full active:cursor-grabbing"
        onMouseDown={startDrag}
      >
        {recordButton}
        {activeStage?.percent != null && <ProgressRing stage={activeStage.stage} percent={activeStage.percent} />}
      </main>
    );
  }

  if (mode === "detailed") {
    return (
      <main
        onContextMenu={openMenu}
        data-phase={widgetPhase}
        className="dw-widget flex h-screen flex-col gap-1.5 overflow-hidden rounded-2xl p-2.5 text-neutral-content"
        onMouseEnter={handleFlyoutMouseEnter}
        onMouseLeave={handleFlyoutMouseLeave}
      >
        <div className="flex items-center gap-2">
          {recordButton}
          {isPaused ? (
            pausedControls
          ) : phase === "recording" && micSilent ? (
            <span
              className="flex-1 cursor-grab truncate text-sm text-warning active:cursor-grabbing"
              onMouseDown={startDrag}
              title={micSilent}
            >
              No sound
            </span>
          ) : phase === "recording" ? (
            <div className="flex h-6 flex-1 cursor-grab items-center gap-2 active:cursor-grabbing" onMouseDown={startDrag}>
              {agentQuestionLine ?? (
                <LevelMeter
                  levels={meterLevels}
                  remoteLevels={meterRemoteLevels}
                  bands={audioBands}
                  config={waveformConfig}
                  dualChannel={dualActive}
                  className="flex-1"
                />
              )}
              <span
                className={`shrink-0 font-mono text-[11px] tabular-nums ${meetingEndingSoon ? "font-medium text-warning" : "opacity-60"}`}
                data-testid="recording-elapsed"
              >
                {elapsedText}
              </span>
            </div>
          ) : showProgress ? (
            <StageProgress
              stage={activeStage!.stage}
              percent={activeStage!.percent}
              label={activeStage!.label}
              title={stageTitle}
              onMouseDown={startDrag}
              onSkip={activeStage!.stage === "refine" ? pasteAsIs : undefined}
            />
          ) : showSetup ? (
            setupButton
          ) : meetingDetectPill ? (
            meetingDetectPill
          ) : meetingSavedPill ? (
            meetingSavedPill
          ) : agentPill ? (
            agentPill
          ) : justPasted ? (
            <div className="flex-1 cursor-grab active:cursor-grabbing" onMouseDown={startDrag}>
              <PastedBadge />
            </div>
          ) : (
            <span
              className="flex-1 cursor-grab truncate text-sm font-medium active:cursor-grabbing"
              onMouseDown={startDrag}
            >
              {idleLabel}
            </span>
          )}
          {meetingEndedPill}
          {showSwitchMic && switchMicButton}
          {discardButton}
          {menuButton()}
          <button
            className="btn btn-ghost btn-circle relative h-7 w-7 min-h-0"
            onClick={openSettings}
            aria-label={updateAvailable ? `Open settings — update to v${updateAvailable} available` : "Open settings"}
            title={updateAvailable ? `Update to v${updateAvailable} available` : undefined}
          >
            <GearIcon />
            {updateAvailable && (
              <span className="absolute right-0.5 top-0.5 h-2 w-2 rounded-full bg-accent" />
            )}
          </button>
        </div>
        {isPaused ? (
          <p className={`text-xs text-warning ${flyoutOpen ? "shrink-0" : "flex-1"}`} data-testid="paused-hint">
            {pausedHint}
          </p>
        ) : phase === "recording" && liveText && !micSilent ? (
          <p
            className={`dw-caption-text overflow-hidden text-sm leading-snug ${flyoutOpen ? "line-clamp-2 shrink-0" : "flex-1"}`}
            data-testid="live-transcript"
          >
            <span className={flyoutOpen ? "line-clamp-2" : "line-clamp-5"}>{renderRichCaption(liveTail(liveText))}</span>
          </p>
        ) : (
          <p
            className={`whitespace-pre-wrap text-xs ${flyoutOpen ? "line-clamp-2 shrink-0" : "flex-1 overflow-y-auto"} ${
              isError || (!message && setupProblem)
                ? "text-error"
                : micSilent || (!message && warning)
                  ? "text-warning"
                  : "opacity-60"
            }`}
          >
            {micSilent ??
              message ??
              (setupProblem ? `${setupProblem.title}. ${setupProblem.detail}` : null) ??
              warning?.full ??
              `Ready to dictate${micName ? ` with ${micName}` : ""}. Press ${hotkey ?? "the hotkey"} or click the button to start.`}
          </p>
        )}
        {flyoutOpen && flyoutPanel}
      </main>
    );
  }

  // Compact & Stacked
  return (
    <main
      onContextMenu={openMenu}
      data-phase={widgetPhase}
      className={`dw-widget flex h-screen flex-col gap-1 overflow-hidden rounded-2xl px-2.5 py-2 text-neutral-content ${
        flyoutOpen ? "justify-start" : "justify-center"
      }`}
      onMouseEnter={handleFlyoutMouseEnter}
      onMouseLeave={handleFlyoutMouseLeave}
    >
      <div className="flex items-center gap-2">
        {recordButton}
        {isPaused ? (
          pausedControls
        ) : phase === "recording" && micSilent ? (
          <span
            className="flex-1 cursor-grab truncate text-sm text-warning active:cursor-grabbing"
            onMouseDown={startDrag}
            title={micSilent}
          >
            No sound
          </span>
        ) : phase === "recording" ? (
          <div className="flex h-6 flex-1 cursor-grab items-center gap-2 active:cursor-grabbing" onMouseDown={startDrag}>
            {agentQuestionLine ?? (
              <LevelMeter
                levels={meterLevels}
                remoteLevels={meterRemoteLevels}
                bands={audioBands}
                config={waveformConfig}
                dualChannel={dualActive}
                className="flex-1"
              />
            )}
            <span
              className={`shrink-0 font-mono text-[11px] tabular-nums ${meetingEndingSoon ? "font-medium text-warning" : "opacity-60"}`}
              data-testid="recording-elapsed"
            >
              {elapsedText}
            </span>
          </div>
        ) : showProgress ? (
          <StageProgress
            stage={activeStage!.stage}
            percent={activeStage!.percent}
            label={activeStage!.label}
            title={stageTitle}
            onMouseDown={startDrag}
            onSkip={activeStage!.stage === "refine" ? pasteAsIs : undefined}
          />
        ) : showSetup ? (
          setupButton
        ) : meetingDetectPill ? (
          meetingDetectPill
        ) : meetingSavedPill ? (
          meetingSavedPill
        ) : agentPill ? (
          agentPill
        ) : justPasted ? (
          <div className="flex-1 cursor-grab active:cursor-grabbing" onMouseDown={startDrag}>
            <PastedBadge />
          </div>
        ) : (
          <span
            ref={compactMessageRef}
            className={`flex-1 cursor-grab active:cursor-grabbing ${
              flashing && isError ? "line-clamp-6 text-xs leading-snug" : "truncate text-sm"
            } ${!flashing && phase === "idle" && warning ? "text-warning" : "opacity-75"}`}
            onMouseDown={startDrag}
            title={flashing ? (message ?? undefined) : phase === "idle" ? warning?.full : undefined}
          >
            {flashing ? message : phase === "idle" && warning ? warning.short : idleLabel}
          </span>
        )}
        {meetingEndedPill}
        {showSwitchMic && switchMicButton}
        {discardButton}
        <button
          className="btn btn-ghost btn-circle relative h-7 w-7 min-h-0"
          onClick={openSettings}
          aria-label={updateAvailable ? `Open settings — update to v${updateAvailable} available` : "Open settings"}
          title={updateAvailable ? `Update to v${updateAvailable} available` : undefined}
        >
          <GearIcon />
          {updateAvailable && <span className="absolute right-0.5 top-0.5 h-2 w-2 rounded-full bg-accent" />}
        </button>
      </div>
      {mode === "stacked" && !flyoutOpen && (
        <div className="dw-second-line" data-testid="second-line">
          <span className="truncate">{micName ?? "No microphone"}</span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0 truncate">{overrideMode ? `Next: ${overrideMode}` : "Mode by app"}</span>
        </div>
      )}
      {flyoutOpen && flyoutPanel}
    </main>
  );
}

export default WidgetView;
