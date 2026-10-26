import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTheme, type ThemeId } from "./theme";
import { getReadiness, type ReadinessCheck } from "./settings/readiness";
import { shortcutLabel, skipVariant, type ShortcutConfig } from "./shortcut";
import { type WaitingAgent } from "./settings/sections/agents";

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

function compactFlyoutSize(modeCount: number, micRowCount: number) {
  const extraRows = Math.ceil(Math.max(modeCount, 1) / 2) - 1;
  const micRows = micRowCount > 0 ? FLYOUT_MIC_SECTION_BASE + micRowCount * (FLYOUT_MODE_ROW_HEIGHT + FLYOUT_MODE_ROW_GAP) : 0;
  const natural =
    COMPACT_FLYOUT_BASE_HEIGHT + extraRows * (FLYOUT_MODE_ROW_HEIGHT + FLYOUT_MODE_ROW_GAP) + micRows + FLYOUT_STYLE_SECTION;
  return { width: COMPACT_WIDTH, height: Math.min(natural, COMPACT_FLYOUT_MAX_HEIGHT) };
}

function usingMic(status: DeviceStatus | null): string | null {
  return status && status.kind !== "unavailable" ? status.using : null;
}

// How many recent level samples the meter shows at once — enough to read
// as a little waveform rather than a single flickering bar, short enough
// that the whole strip reacts quickly to a pause or a loud word.
// Recent input levels kept for the meter (20 per second, so about 3 s);
// each meter shows as many of the newest as fit its width.
const LEVEL_BARS = 64;
const SILENT_LEVELS = Array<number>(LEVEL_BARS).fill(0);
const METER_BAR = 3;
const METER_GAP = 2;

// Live "is audio actually coming through" waveform, in the spirit of the
// one most dictation apps show while listening. `levels` is the most
// recent samples from the backend's `audio-level` event (oldest first).
// It fills whatever width it's given: as many bars as fit, newest on the
// right, mirrored around the middle, older bars fading out. Levels are
// square-rooted so quiet speech still visibly moves.
function LevelMeter({ levels, className }: { levels: number[]; className?: string }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [count, setCount] = useState(12);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const fit = Math.floor((el.clientWidth + METER_GAP) / (METER_BAR + METER_GAP));
      setCount(Math.max(4, Math.min(fit, levels.length)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [levels.length]);
  const shown = levels.slice(-count);
  // The bars sit in an absolutely positioned layer, so they never give the
  // meter a width of its own: its width comes only from the layout, and
  // measuring it can't feed back into it (a resize loop that froze the
  // widget otherwise).
  return (
    <div ref={ref} className={`relative h-full min-w-0 ${className ?? ""}`} data-testid="level-meter">
      <div className="absolute inset-0 flex items-center justify-end gap-[2px] overflow-hidden">
        {shown.map((level, i) => (
          <span
            key={i}
            className="shrink-0 rounded-full bg-error transition-[height] duration-100 ease-out"
            style={{
              width: METER_BAR,
              height: `${Math.max(Math.sqrt(level) * 100, 14)}%`,
              opacity: 0.35 + 0.65 * ((i + 1) / shown.length),
            }}
          />
        ))}
      </div>
    </div>
  );
}

// The end of the running transcript, which is what's just been said; the
// widget shows a few lines at most.
const LIVE_TAIL_CHARS = 160;
function liveTail(text: string) {
  return text.length <= LIVE_TAIL_CHARS ? text : `…${text.slice(-LIVE_TAIL_CHARS).replace(/^\S*\s/, "")}`;
}

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

// One processing stage: a label, its percentage, and a slim bar whose width
// animates between backend updates (a short dictation only gets a
// handful), so it reads as continuous. Each stage has its own color so
// "Whisper is listening back" and "the LLM is rewriting" read as different
// steps: amber for transcribing, violet with a sparkle and a shimmer for
// LLM refinement.
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

function StageProgress({
  stage,
  percent,
  title,
  onMouseDown,
  onSkip,
}: {
  stage: Stage;
  percent: number | null;
  title?: string;
  onMouseDown: (e: React.MouseEvent) => void;
  /// Refining only: clicking pastes the unrefined text now.
  onSkip?: () => void;
}) {
  const label = stage === "transcribe" ? "Transcribing" : "Refining";
  return (
    <div
      className={`flex min-w-0 flex-1 flex-col gap-1 ${
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
        className="flex items-baseline justify-between gap-2 text-sm"
        style={stage === "refine" ? { color: STAGE_COLOR.refine } : { opacity: 0.75 }}
      >
        <span className="flex min-w-0 items-center gap-1">
          {stage === "refine" && <SparkleIcon />}
          <span className="truncate">{label}</span>
        </span>
        {percent !== null && <span className="shrink-0 tabular-nums">{percent}%</span>}
      </span>
      <div className="h-[3px] overflow-hidden rounded-full bg-white/10">
        <div
          className={`h-full rounded-full transition-[width] duration-300 ease-out motion-reduce:transition-none ${
            stage === "refine" ? "dw-shimmer" : ""
          }`}
          style={{
            width: `${percent ?? 100}%`,
            background: STAGE_COLOR[stage],
            opacity: percent === null ? 0.5 : 1,
          }}
        />
      </div>
    </div>
  );
}

// Minimal mode's version: a ring that fills around the record button, in
// the stage's color.
// A hairline progress bar for the small styles (Pill, Notch), in the
// stage's color; indeterminate (a shimmer) without a percentage.
function ThinProgress({ stage, percent }: { stage: Stage; percent: number | null }) {
  return (
    <div
      className="dw-thin-progress"
      role="progressbar"
      aria-label={stage === "transcribe" ? "Transcribing" : "Refining"}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      data-testid={`${stage}-progress`}
    >
      <span
        className={percent === null ? "dw-thin-indeterminate" : undefined}
        style={{ background: STAGE_COLOR[stage], width: percent === null ? undefined : `${percent}%` }}
      />
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
  const [phase, setPhase] = useState<Phase>("idle");
  // `message` is sticky — it holds whatever the last status/error text
  // was until something new replaces it. `flashVisible` controls whether
  // compact/minimal show it as a temporary overlay vs. falling back to
  // the plain phase label; detailed mode ignores `flashVisible` and
  // always shows `message`, since persistence is the point of that mode.
  const [message, setMessage] = useState<string | null>(null);
  const [isError, setIsError] = useState(false);
  const [flashVisible, setFlashVisible] = useState(false);
  const flashTimeout = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Quick-actions hover flyout state (compact mode only, for now) — see
  // the `.dw-fly-*` styles in App.css. `micMode`/`overrideMode` mirror
  // real backend state (isolated_voice_enabled / the one-shot mode
  // override), not local-only UI state.
  const [flyoutHovered, setFlyoutHovered] = useState(false);
  // A discarded recording (or cancelled paste) can be brought back for a
  // few seconds; see discard.rs.
  const [undoOffer, setUndoOffer] = useState(false);
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

  // Rolling window of recent input levels for the live level meter — see
  // LevelMeter above. Reset to silence whenever recording isn't active, so
  // a stale "last thing you said was loud" reading doesn't linger once
  // idle/transcribing.
  const [levels, setLevels] = useState<number[]>(SILENT_LEVELS);
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
  useEffect(() => {
    if (phase !== "recording") return;
    const started = Date.now();
    setElapsedMs(0);
    const tick = setInterval(() => setElapsedMs(Date.now() - started), 250);
    return () => clearInterval(tick);
  }, [phase]);

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
  const [replyKey, setReplyKey] = useState<string | null>(null);
  useEffect(() => {
    const refresh = () =>
      invoke<WaitingAgent[]>("get_waiting_agents")
        .then(setWaiting)
        .catch((err) => console.error("get_waiting_agents failed:", err));
    refresh();
    const unlisten = listen<WaitingAgent[]>("agents-changed", (e) => setWaiting(e.payload));
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
  useEffect(() => {
    const refresh = () =>
      getReadiness()
        .then((checks) => setSetupProblem(checks.find((c) => c.level === "error") ?? null))
        .catch((err) => console.error("get_readiness failed:", err));
    refresh();
    const events = ["model-download-complete", "audio-device-status", "shortcut-changed", "transcript-error"];
    const unlisteners = events.map((event) => listen(event, refresh));
    // Catches what has no event, like Accessibility being granted.
    const interval = setInterval(refresh, 30_000);
    return () => {
      clearInterval(interval);
      unlisteners.forEach((u) => u.then((f) => f()));
    };
  }, []);

  useEffect(() => {
    if (!flyoutHovered) return;
    invoke<string[]>("list_input_devices")
      .then(setDevices)
      .catch((err) => console.error("list_input_devices failed:", err));
  }, [flyoutHovered]);

  useEffect(() => {
    if (phase !== "recording") setLevels(SILENT_LEVELS);
    if (phase !== "transcribing") setProgress(null);
    if (phase !== "refining") setRefineProgress(null);
  }, [phase]);

  useEffect(() => {
    invoke<WidgetMode>("get_widget_mode")
      .then(setMode)
      .catch((err) => console.error("get_widget_mode failed:", err));

    invoke<boolean>("get_isolated_voice_enabled")
      .then(setMicMode)
      .catch((err) => console.error("get_isolated_voice_enabled failed:", err));

    invoke<ModeSummary[]>("get_modes")
      .then(setModes)
      .catch((err) => console.error("get_modes failed:", err));

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

    const unlistenModeChanged = listen<WidgetMode>("widget-mode-changed", (e) => setMode(e.payload));
    const unlistenLive = listen<string>("live-transcript", (e) => setLiveText(e.payload));
    const unlistenStart = listen("recording-started", () => {
      setUndoOffer(false);
      setLiveText("");
      setMicSilent(null);
      setPhase("recording");
    });
    const unlistenStop = listen("recording-stopped", () => {
      setLiveText("");
      setMicSilent(null);
      setPhase("transcribing");
    });
    const unlistenMicSilent = listen<string>("mic-silent", (e) => setMicSilent(e.payload));
    const unlistenMicCleared = listen("mic-silent-cleared", () => setMicSilent(null));
    const unlistenRefining = listen<string | null>("refining-started", (e) => {
      setRefineModel(e.payload ?? null);
      // Stays null (an indeterminate bar) for boilerplate generation, which
      // reports no progress.
      setRefineProgress(null);
      setPhase("refining");
    });
    const unlistenRefineProgress = listen<number>("refine-progress", (e) => setRefineProgress(e.payload));
    const unlistenProgress = listen<number>("transcribe-progress", (e) => setProgress(e.payload));
    const unlistenLevel = listen<number>("audio-level", (e) => {
      setLevels((prev) => [...prev.slice(1), e.payload]);
    });
    const unlistenRecordingError = listen<string>("recording-error", (event) => {
      setPhase("idle");
      announce(event.payload, true);
    });
    const unlistenTranscriptReady = listen<string>("transcript-ready", (event) => {
      setLastPasted(event.payload || null);
      setPhase("idle");
      announce("Pasted", false);
      // The backend consumes the override the moment it's used (see
      // recording.rs's `mode_override.take()`) — clear the pill so it
      // doesn't look like it's still armed for the next dictation.
      setOverrideMode(null);
    });
    const unlistenTranscriptError = listen<string>("transcript-error", (event) => {
      setPhase("idle");
      setOverrideMode(null);
      announce(event.payload, true);
    });
    const unlistenCopied = listen<string>("transcript-copied", (event) => {
      setPhase("idle");
      setOverrideMode(null);
      announce(event.payload, false);
    });
    const unlistenDiscarded = listen<string>("dictation-discarded", (event) => {
      setPhase("idle");
      setLiveText("");
      setMicSilent(null);
      setUndoOffer(true);
      announce(event.payload === "cancelled" ? "Paste cancelled" : "Recording discarded", false);
    });
    const unlistenDiscardOver = listen("discard-expired", () => setUndoOffer(false));
    const unlistenDiscardUndone = listen("discard-undone", () => setUndoOffer(false));
    const unlistenUpdateAvailable = listen<string>("update-available", (event) => {
      setUpdateAvailable(event.payload);
    });

    return () => {
      unlistenTheme.then((f) => f());
      unlistenModeChanged.then((f) => f());
      unlistenStart.then((f) => f());
      unlistenLive.then((f) => f());
      unlistenMicSilent.then((f) => f());
      unlistenMicCleared.then((f) => f());
      unlistenStop.then((f) => f());
      unlistenRefining.then((f) => f());
      unlistenProgress.then((f) => f());
      unlistenRefineProgress.then((f) => f());
      unlistenLevel.then((f) => f());
      unlistenRecordingError.then((f) => f());
      unlistenTranscriptReady.then((f) => f());
      unlistenTranscriptError.then((f) => f());
      unlistenUpdateAvailable.then((f) => f());
      unlistenDevice.then((f) => f());
      unlistenAmbient.then((f) => f());
      unlistenCopied.then((f) => f());
      unlistenDiscarded.then((f) => f());
      unlistenDiscardOver.then((f) => f());
      unlistenDiscardUndone.then((f) => f());
    };
  }, []);

  // Notch grows from the middle while it has something to show (the
  // backend re-centers it on every resize).
  const notchActive = phase !== "idle" || (flashVisible && message !== null) || undoOffer;
  useEffect(() => {
    if (mode !== "notch") return;
    invoke("set_widget_size", notchActive ? NOTCH_ACTIVE : NOTCH_IDLE).catch((err) =>
      console.error("set_widget_size failed:", err),
    );
  }, [mode, notchActive]);

  useEffect(() => {
    if (mode !== "compact" && mode !== "stacked") return;
    const extra = mode === "stacked" ? STACKED_EXTRA : 0;
    // The flyout takes priority over the error-expansion size — hovering
    // to check mic mode mid-error is a rare enough overlap that showing
    // the flyout (rather than fighting over which one "wins" the window
    // height) is the simpler, still-correct choice.
    const size = flyoutHovered
      ? compactFlyoutSize(modes.length, micRows.length)
      : flashVisible && isError
        ? errorHeight
          ? { width: COMPACT_WIDTH, height: errorHeight + extra }
          : COMPACT_EXPANDED_SIZE
        : { width: COMPACT_WIDTH, height: COMPACT_BASE_SIZE.height + extra };
    invoke("set_widget_size", size).catch((err) => console.error("set_widget_size failed:", err));
  }, [mode, flashVisible, isError, flyoutHovered, modes.length, micRows.length, errorHeight]);

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
    setFlyoutHovered(false);
    invoke("set_widget_mode", { mode: next }).catch(actionFailed("switch the widget style"));
  }

  function toggleRecording() {
    invoke("toggle_recording_command").catch(actionFailed("start recording"));
  }

  function focusAgent(agent: WaitingAgent) {
    invoke("focus_waiting_agent", { sessionId: agent.session_id }).catch(actionFailed("go to the agent"));
  }

  function pasteAsIs() {
    invoke("paste_as_is_command").catch(actionFailed("skip refining"));
  }

  function openSettings() {
    invoke("open_settings").catch(actionFailed("open Settings"));
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
      getCurrentWindow()
        .startDragging()
        .catch((err) => console.error("startDragging failed:", err));
    }
  }

  const busy = phase === "transcribing" || phase === "refining";
  // Which processing stage to draw, if any.
  const activeStage: { stage: Stage; percent: number | null } | null =
    phase === "transcribing" && progress !== null
      ? { stage: "transcribe", percent: progress }
      : phase === "refining"
        ? { stage: "refine", percent: refineProgress }
        : null;
  const showProgress = activeStage !== null;
  const stageTitle =
    phase === "refining"
      ? `${refineModel ? `Cleaning up with ${refineModel}. ` : ""}Click to paste as-is${skipKey ? ` (${skipKey})` : ""}.`
      : undefined;
  const idleLabel =
    phase !== "idle"
      ? STATUS_LABEL[phase]
      : ambientPhase === "listening"
        ? "Listening for “Mouthpiece”"
        : ambientPhase === "asleep"
          ? "Asleep"
          : ambientPhase === "paused"
            ? "Paused (mic in use)"
            : STATUS_LABEL.idle;
  const flashing = flashVisible && message !== null;
  const showSetup = phase === "idle" && !flashing && setupProblem !== null;

  const setupButton = (
    <button
      type="button"
      className="flex-1 truncate text-left text-sm font-medium text-error hover:underline"
      onClick={openSettings}
      title={setupProblem ? `${setupProblem.title}. ${setupProblem.detail}` : undefined}
      data-testid="widget-setup"
    >
      Finish setup
    </button>
  );

  // A coding agent is waiting (newest first). Replaces the idle label; the
  // click brings that agent's window forward.
  const newestAgent = waiting[0];
  const agentTitle = newestAgent
    ? `${newestAgent.kind === "done" ? "Finished" : newestAgent.message || "Needs your input"}${
        newestAgent.app_name ? ` (${newestAgent.app_name})` : ""
      }. Click to go to it${replyKey ? `, or press ${replyKey} to reply by voice` : ""}.`
    : undefined;
  const agentPill =
    newestAgent && phase === "idle" && !flashing ? (
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-1.5 truncate rounded-full bg-info/20 px-2 py-0.5 text-left text-xs font-medium text-info hover:bg-info/30"
        onClick={() => focusAgent(newestAgent)}
        title={agentTitle}
        data-testid="agent-pill"
      >
        <span className="h-1.5 w-1.5 shrink-0 animate-pulse rounded-full bg-info motion-reduce:animate-none" />
        <span className="truncate">
          Claude is waiting · {newestAgent.folder}
          {waiting.length > 1 ? ` +${waiting.length - 1}` : ""}
        </span>
      </button>
    ) : null;

  // ✕ while recording or preparing the paste: same as pressing Esc.
  const discardButton =
    phase !== "idle" ? (
      <button
        type="button"
        className="dw-icon-btn"
        onClick={() => invoke("discard_recording").catch(actionFailed("discard the recording"))}
        aria-label={phase === "recording" ? "Discard recording (Esc)" : "Cancel paste (Esc)"}
        title={phase === "recording" ? "Discard recording (Esc)" : "Cancel paste (Esc)"}
        data-testid="discard"
      >
        <svg aria-hidden="true" viewBox="0 0 12 12" width="10" height="10">
          <path d="M2.5 2.5l7 7m0-7l-7 7" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      </button>
    ) : null;

  const undoButton =
    phase === "idle" && undoOffer ? (
      <button
        type="button"
        className="dw-undo-btn"
        onClick={() => {
          setUndoOffer(false);
          invoke("undo_discard").catch(actionFailed("undo"));
        }}
        title="Bring it back (⇧Esc)"
        data-testid="undo-discard"
      >
        Undo <kbd>⇧⎋</kbd>
      </button>
    ) : null;

  const recordButton = (
    <button
      className={`btn btn-circle border-none ${mode === "minimal" ? "btn-md" : "btn-sm"} ${
        phase === "recording" ? "bg-white/15" : "bg-white/8 hover:bg-white/15"
      } ${
        (flashing && isError) || (showSetup && mode === "minimal")
          ? "ring-2 ring-error"
          : (warning || micSilent) && mode === "minimal"
            ? "ring-2 ring-warning"
            : newestAgent && mode === "minimal"
              ? "ring-2 ring-info"
              : ""
      }`}
      onClick={toggleRecording}
      disabled={busy}
      aria-label={phase === "idle" ? "Start recording" : "Stop recording"}
      title={
        micSilent ??
        message ??
        (showSetup && setupProblem ? `${setupProblem.title}. ${setupProblem.detail}` : null) ??
        warning?.full ??
        (newestAgent && mode === "minimal" ? `Claude is waiting · ${newestAgent.folder}` : null) ??
        (micName ? `${STATUS_LABEL[phase]} · Mic: ${micName}` : STATUS_LABEL[phase])
      }
    >
      {busy ? (
        <span className="loading loading-spinner loading-xs" />
      ) : (
        <span
          className={`rounded-full transition-transform duration-100 ${
            mode === "minimal" ? "h-3.5 w-3.5" : "h-2.5 w-2.5"
          } ${phase === "recording" ? "bg-error shadow-[0_0_8px] shadow-error" : "bg-neutral-content/40"}`}
          // Minimal mode has no room for a multi-bar meter, so instead the
          // dot itself pulses with the latest input level — same "is audio
          // actually coming through" signal, scaled to fit a 46x46 icon.
          style={
            mode === "minimal" && phase === "recording"
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
      ? micSilent
        ? "No sound from mic"
        : formatElapsed(elapsedMs)
      : phase === "transcribing"
        ? `Transcribing${progress !== null ? ` ${progress}%` : "…"}`
        : phase === "refining"
          ? "Refining…"
          : flashing
            ? (message ?? "")
            : showSetup
              ? "Finish setup"
              : idleLabel;
  const statusTone =
    flashing && isError ? "text-error" : micSilent || (showSetup && !flashing) ? "text-warning" : "";
  const fullStatus =
    micSilent ??
    (flashing ? message : null) ??
    (showSetup && setupProblem ? `${setupProblem.title}. ${setupProblem.detail}` : null) ??
    (micName ? `${STATUS_LABEL[phase]} · Mic: ${micName}` : STATUS_LABEL[phase]);

  if (mode === "pill") {
    return (
      <main
        className="dw-widget group flex h-screen w-screen cursor-grab items-center gap-1.5 overflow-hidden rounded-full border border-white/10 bg-neutral/90 pl-0.5 pr-2 text-neutral-content backdrop-blur-md active:cursor-grabbing"
        onMouseDown={startDrag}
        title={fullStatus ?? undefined}
      >
        <span className="dw-small-record">{recordButton}</span>
        <div className="flex h-4 min-w-0 flex-1 items-center">
          {showProgress ? (
            <ThinProgress stage={activeStage!.stage} percent={activeStage!.percent} />
          ) : flashing || showSetup ? (
            <span className={`truncate text-xs ${statusTone}`}>{shortStatus}</span>
          ) : (
            <>
              <LevelMeter
                levels={levels}
                className={`flex-1 group-hover:hidden ${phase === "idle" ? "opacity-40 grayscale" : ""}`}
              />
              <span className="hidden truncate text-xs opacity-75 group-hover:inline">{shortStatus}</span>
            </>
          )}
        </div>
        {discardButton}
        {undoButton}
      </main>
    );
  }

  if (mode === "notch") {
    return (
      <main
        className={`dw-widget dw-notch flex h-screen w-screen items-center overflow-hidden text-neutral-content ${
          notchActive ? "gap-2 px-2" : "justify-center gap-1.5 px-3"
        }`}
        title={fullStatus ?? undefined}
      >
        {notchActive ? (
          <>
            <span className="dw-small-record">{recordButton}</span>
            <span className={`min-w-0 shrink truncate text-xs ${statusTone || "opacity-80"}`}>{shortStatus}</span>
            <div className="flex h-4 min-w-0 flex-1 items-center">
              {showProgress ? (
                <ThinProgress stage={activeStage!.stage} percent={activeStage!.percent} />
              ) : phase === "recording" ? (
                <LevelMeter levels={levels} className="flex-1" />
              ) : null}
            </div>
            {discardButton}
            {undoButton}
          </>
        ) : (
          <button type="button" className="dw-notch-idle" onClick={toggleRecording} aria-label="Start recording">
            <span className={`h-1.5 w-1.5 rounded-full ${showSetup ? "bg-warning" : "bg-neutral-content/50"}`} />
            <LevelMeter levels={SILENT_LEVELS} className="w-10 opacity-40 grayscale" />
          </button>
        )}
      </main>
    );
  }

  if (mode === "caption") {
    const body =
      phase === "recording" ? (
        micSilent ? (
          <span className="text-warning">{micSilent}</span>
        ) : liveText ? (
          <span data-testid="live-transcript">{liveTail(liveText)}</span>
        ) : (
          <span className="opacity-40">Listening…</span>
        )
      ) : busy ? (
        <span className="opacity-60">{liveText ? liveTail(liveText) : shortStatus}</span>
      ) : flashing && isError ? (
        <span className="text-error">{message}</span>
      ) : lastPasted ? (
        <span className="opacity-60" data-testid="last-pasted">
          {lastPasted}
        </span>
      ) : (
        <span className="opacity-40">
          Press {hotkey ?? "the hotkey"} and start talking. Your words show up here as you speak.
        </span>
      );
    return (
      <main className="dw-widget flex h-screen flex-col gap-1.5 overflow-hidden rounded-2xl border border-white/10 bg-neutral/90 p-2.5 text-neutral-content backdrop-blur-md">
        <div className="flex items-center gap-2">
          {recordButton}
          <div className="flex h-5 min-w-0 flex-1 cursor-grab items-center gap-2 active:cursor-grabbing" onMouseDown={startDrag}>
            {phase === "recording" && !micSilent ? (
              <>
                <LevelMeter levels={levels} className="flex-1" />
                <span className="shrink-0 text-xs tabular-nums opacity-60">{shortStatus}</span>
              </>
            ) : (
              <>
                <span className={`shrink-0 truncate text-xs ${statusTone || "opacity-70"}`}>
                  {phase === "idle" && !flashing && lastPasted ? "Pasted" : shortStatus}
                </span>
                {showProgress && <ThinProgress stage={activeStage!.stage} percent={activeStage!.percent} />}
              </>
            )}
          </div>
          {discardButton}
          {undoButton}
          <button
            className="btn btn-ghost btn-circle relative h-7 w-7 min-h-0"
            onClick={openSettings}
            aria-label={updateAvailable ? `Open settings — update to v${updateAvailable} available` : "Open settings"}
          >
            <GearIcon />
            {updateAvailable && <span className="absolute right-0.5 top-0.5 h-2 w-2 rounded-full bg-accent" />}
          </button>
        </div>
        <p className="dw-caption-text line-clamp-3 flex-1 text-sm leading-snug" onMouseDown={startDrag}>
          {body}
        </p>
      </main>
    );
  }

  if (mode === "minimal") {
    return (
      <main
        className="dw-widget relative flex h-screen w-screen cursor-grab items-center justify-center overflow-hidden rounded-full border border-white/10 bg-neutral/90 backdrop-blur-md active:cursor-grabbing"
        onMouseDown={startDrag}
      >
        {recordButton}
        {activeStage?.percent != null && <ProgressRing stage={activeStage.stage} percent={activeStage.percent} />}
      </main>
    );
  }

  if (mode === "detailed") {
    return (
      <main className="dw-widget flex h-screen flex-col gap-1.5 overflow-hidden rounded-2xl border border-white/10 bg-neutral/90 p-2.5 text-neutral-content backdrop-blur-md">
        <div className="flex items-center gap-2">
          {recordButton}
          {phase === "recording" && micSilent ? (
            <span
              className="flex-1 cursor-grab truncate text-sm text-warning active:cursor-grabbing"
              onMouseDown={startDrag}
              title={micSilent}
            >
              No sound from mic
            </span>
          ) : phase === "recording" ? (
            <div className="flex h-4 flex-1 cursor-grab items-center gap-2 active:cursor-grabbing" onMouseDown={startDrag}>
              <LevelMeter levels={levels} className="flex-1" />
              <span className="shrink-0 text-xs tabular-nums opacity-60" data-testid="recording-elapsed">
                {formatElapsed(elapsedMs)}
              </span>
            </div>
          ) : showProgress ? (
            <StageProgress
            stage={activeStage!.stage}
            percent={activeStage!.percent}
            title={stageTitle}
            onMouseDown={startDrag}
            onSkip={activeStage!.stage === "refine" ? pasteAsIs : undefined}
          />
          ) : showSetup ? (
            setupButton
          ) : agentPill ? (
            agentPill
          ) : (
            <span
              className="flex-1 cursor-grab truncate text-sm font-medium active:cursor-grabbing"
              onMouseDown={startDrag}
            >
              {idleLabel}
            </span>
          )}
          {discardButton}
          {undoButton}
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
        {phase === "recording" && liveText && !micSilent ? (
          <p className="dw-caption-text flex-1 overflow-hidden text-sm leading-snug" data-testid="live-transcript">
            <span className="line-clamp-5">{liveTail(liveText)}</span>
          </p>
        ) : (
        <p
          className={`flex-1 overflow-y-auto whitespace-pre-wrap text-xs ${
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
      </main>
    );
  }

  // Compact
  return (
    <main
      className={`dw-widget flex h-screen flex-col gap-1 overflow-hidden rounded-2xl border border-white/10 bg-neutral/90 px-2.5 py-2 text-neutral-content backdrop-blur-md ${
        flyoutHovered ? "justify-start" : "justify-center"
      }`}
      onMouseEnter={() => setFlyoutHovered(true)}
      onMouseLeave={() => setFlyoutHovered(false)}
    >
      <div className="flex items-center gap-2">
        {recordButton}
        {phase === "recording" && micSilent ? (
          <span
            className="flex-1 cursor-grab truncate text-sm text-warning active:cursor-grabbing"
            onMouseDown={startDrag}
            title={micSilent}
          >
            No sound from mic
          </span>
        ) : phase === "recording" ? (
          <div className="flex h-4 flex-1 cursor-grab items-center gap-2 active:cursor-grabbing" onMouseDown={startDrag}>
            <LevelMeter levels={levels} className="flex-1" />
            <span className="shrink-0 text-xs tabular-nums opacity-60" data-testid="recording-elapsed">
              {formatElapsed(elapsedMs)}
            </span>
          </div>
        ) : showProgress ? (
          <StageProgress
            stage={activeStage!.stage}
            percent={activeStage!.percent}
            title={stageTitle}
            onMouseDown={startDrag}
            onSkip={activeStage!.stage === "refine" ? pasteAsIs : undefined}
          />
        ) : showSetup ? (
          setupButton
        ) : agentPill ? (
          agentPill
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
        {discardButton}
        {undoButton}
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
      {mode === "stacked" && !flyoutHovered && (
        <div className="dw-second-line" data-testid="second-line">
          <span className="truncate">{micName ?? "No microphone"}</span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0 truncate">{overrideMode ? `Next: ${overrideMode}` : "Mode by app"}</span>
        </div>
      )}
      {flyoutHovered && (
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
      )}
    </main>
  );
}

export default WidgetView;
