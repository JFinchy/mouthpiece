// An in-memory stand-in for the Rust backend, so the UI runs in a plain
// browser: Playwright e2e tests drive it (see e2e/), and `bun run dev:mock`
// uses it for UI work without building the app. Only loaded when
// VITE_MOCK_BACKEND=1, so it never ships (see main.tsx).
//
// Tests shape the starting state through `window.__DW_SCENARIO__` (set
// with page.addInitScript before load), then drive and inspect it through
// `window.__dw`: `emit` plays a backend event, `calls` records every
// command the UI sent.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

type Json = unknown;
type Call = { cmd: string; args: Record<string, Json> | undefined };

export type Scenario = {
  readiness?: { id: string; level: "ok" | "warn" | "error"; title: string; detail: string; fix: string | null }[];
  devices?: string[];
  deviceStatus?: Json;
  /// Commands that reject, with this message.
  failing?: Record<string, string>;
  widgetMode?: "minimal" | "compact" | "detailed" | "pill" | "notch" | "caption" | "stacked";
  layout?: "sidebar" | "chain";
  insights?: Json;
  /// The project Settings reports as last detected.
  projectStatus?: { name: string; names: number } | null;
  /// Correction suggestions Settings > Vocabulary shows (already past the threshold).
  corrections?: { from: string; to: string; source: "edit" | "original"; count: number }[];
  /// Coding agents already waiting when the page loads.
  agents?: Json[];
  /// History lock already turned on (and closed) when the page loads.
  historyLock?: boolean;
  /// Saved meetings; defaults to one transcribed and one not.
  meetings?: { id: string; title: string; detail: string | null; state: string; text?: string }[];
};

declare global {
  interface Window {
    __DW_SCENARIO__?: Scenario;
    __dw?: {
      emit: (event: string, payload?: Json) => Promise<void>;
      calls: Call[];
      state: ReturnType<typeof initialState>;
    };
  }
}

const READY: NonNullable<Scenario["readiness"]> = [
  { id: "model", level: "ok", title: "Speech model ready", detail: "Base (English)", fix: null },
  { id: "paste", level: "ok", title: "Pasting allowed", detail: "Accessibility permission is granted.", fix: null },
  { id: "hotkey", level: "ok", title: "Hotkey ready", detail: "Press it anywhere to start dictating.", fix: null },
  { id: "microphone", level: "ok", title: "Microphone ready", detail: "MacBook Pro Microphone", fix: null },
];

function initialState(scenario: Scenario) {
  const devices = scenario.devices ?? ["MacBook Pro Microphone", "AIRHUG 28", "Microsoft Teams Audio"];
  return {
    readiness: scenario.readiness ?? READY,
    devices,
    selectedDevice: null as string | null,
    deviceStatus: scenario.deviceStatus ?? { kind: "system_default", using: devices[0] },
    widgetMode: scenario.widgetMode ?? "compact",
    layout: scenario.layout ?? "sidebar",
    theme: "terminal",
    shortcut: { meta: true, ctrl: false, alt: false, shift: true, code: "Space" },
    doubleTap: false,
    agents: (scenario.agents ?? []) as { session_id: string }[],
    agentShortcut: { meta: false, ctrl: true, alt: true, shift: false, code: "KeyR" },
    agentSound: true,
    historyLockEnabled: scenario.historyLock ?? false,
    meetingSpeakers: 0,
    meetingStatus: {
      recording: null as { id: string; started_ms: number } | null,
      processing: null as { id: string; step: string; percent: number | null } | null,
      queued: [] as string[],
    },
    meetings: scenario.meetings ?? [
      {
        id: "2026-10-26-093000",
        title: "Meeting, Mon 26 Oct 2026, 09:30",
        detail: "24 min · 3 speakers",
        state: "ready",
        text: "# Meeting, Mon 26 Oct 2026, 09:30\n\n24 min · 3 speakers\n\n**You** · 00:00\nLet's get started.\n",
      },
      {
        id: "2026-10-25-161500",
        title: "Meeting, Sun 25 Oct 2026, 16:15",
        detail: null,
        state: "not_transcribed",
      },
    ],
    historyUnlocked: false,
    logTranscripts: false,
    copyOnly: false,
    pressEnter: false,
    isolatedVoice: false,
    liveTranscribe: true,
    ambient: false,
    autostart: false,
    journal: false,
    retentionDays: 90,
    webhookUrl: null as string | null,
    llmModel: "qwen3.5:4b",
    llmOnlyWhenNeeded: true,
    nextModeOverride: null as string | null,
    vocabulary: ["kubectl", "JSON", "macOS", "Tauri"],
    projectNames: true,
    projectStatus: scenario.projectStatus ?? null,
    snippets: [{ trigger: "pr checklist", body: "- [ ] Tests pass locally\n- [ ] Self-reviewed the diff" }],
    modes: [
      { name: "Default", behavior: "plain", apps: [], stt_model: null, llm_refinement: "off", custom_instructions: null, is_default: true },
      { name: "Terminal", behavior: "cli", apps: [{ bundle_id: "com.apple.Terminal", app_name: "Terminal" }], stt_model: null, llm_refinement: "global", custom_instructions: null, is_default: false },
      { name: "Messaging", behavior: "casual", apps: [{ bundle_id: "com.tinyspeck.slackmacgap", app_name: "Slack" }], stt_model: null, llm_refinement: "global", custom_instructions: null, is_default: false },
    ],
    models: [
      { id: "tiny.en", label: "Tiny (English) — fastest, least accurate", size_mb: 32, downloaded: false, active: false },
      { id: "base.en", label: "Base (English) — balanced", size_mb: 57, downloaded: true, active: true },
      { id: "small.en", label: "Small (English) — more accurate, slower", size_mb: 190, downloaded: false, active: false },
    ],
    history: [
      { timestamp_ms: Date.now() - 60_000, text: "Look at @recording.rs and fix the crash", app_name: "Terminal", mode: "Cli", summary: null },
      { timestamp_ms: Date.now() - 3_600_000, text: "Let's meet at three.", app_name: "Slack", mode: "Casual", summary: null, original_text: "um let's meet at three" },
    ] as { timestamp_ms: number; text: string; app_name: string; mode: string; summary: string | null; original_text?: string }[],
    learnCorrections: true,
    corrections: scenario.corrections ?? [],
    replacements: [] as { from: string; to: string }[],
    logs: [{ timestamp_ms: Date.now(), message: "lifecycle: started version=0.1.2 (mock backend)" }],
    insights: scenario.insights ?? {
      total_dictations: 42,
      total_words: 1337,
      smart_formatting_count: 12,
      avg_words_per_minute: 148,
      personal_best_wpm: 171,
      latency: { median_ms: 640, p90_ms: 1900, samples: 42 },
      app_usage: [
        { app_name: "Terminal", dictations: 30, percent: 71 },
        { app_name: "Slack", dictations: 12, percent: 29 },
      ],
      streak: { current_days: 3, longest_days: 9, daily_counts: {} },
      adoption: { score_percent: 50, items: [] },
    },
  };
}

export function installMockBackend() {
  const scenario = window.__DW_SCENARIO__ ?? {};
  const label = new URLSearchParams(window.location.search).get("window") ?? "widget";
  mockWindows(label, ...["widget", "settings"].filter((l) => l !== label));

  const state = initialState(scenario);
  const calls: Call[] = [];
  const historyLocked = () => state.historyLockEnabled && !state.historyUnlocked;
  const lockStatus = () => ({ enabled: state.historyLockEnabled, locked: historyLocked() });
  const setter = <K extends keyof typeof state>(key: K, argName: string) => (args: Record<string, Json>) => {
    state[key] = args[argName] as (typeof state)[K];
    return null;
  };

  const handlers: Record<string, (args: Record<string, Json>) => Json> = {
    // readiness + recording
    get_readiness: () => state.readiness,
    open_accessibility_settings: () => null,
    request_accessibility: () => null,
    show_mic_menu: () => null,
    show_widget_menu: () => null,
    toggle_recording_command: () => null,
    paste_as_is_command: () => null,
    ui_ready: () => null,
    get_audio_device_status: () => state.deviceStatus,
    list_input_devices: () => state.devices,
    get_active_input_device: () => state.selectedDevice ?? state.devices[0],
    set_input_device: (args) => {
      state.selectedDevice = (args.name as string | null) ?? null;
      state.deviceStatus = state.selectedDevice
        ? { kind: "preferred", using: state.selectedDevice }
        : { kind: "system_default", using: state.devices[0] };
      void emit("audio-device-status", state.deviceStatus);
      return null;
    },
    set_next_mode_override: setter("nextModeOverride", "modeName"),
    get_last_frontmost_app: () => ({ bundle_id: "com.apple.Terminal", name: "Terminal", icon_data_uri: null }),
    // widget + window
    get_widget_mode: () => state.widgetMode,
    set_widget_mode: (args) => {
      state.widgetMode = args.mode as typeof state.widgetMode;
      void emit("widget-mode-changed", state.widgetMode);
      return null;
    },
    set_widget_size: () => null,
    open_settings: () => null,
    get_theme: () => state.theme,
    set_theme: setter("theme", "theme"),
    get_layout: () => state.layout,
    set_layout: setter("layout", "layout"),
    // simple settings
    get_shortcut: () => state.shortcut,
    set_shortcut: (args) => {
      state.shortcut = args.newCfg as typeof state.shortcut;
      return state.shortcut;
    },
    get_double_tap_enabled: () => state.doubleTap,
    set_double_tap_enabled: setter("doubleTap", "enabled"),
    get_copy_only: () => state.copyOnly,
    set_copy_only: setter("copyOnly", "enabled"),
    get_press_enter_enabled: () => state.pressEnter,
    set_press_enter_enabled: setter("pressEnter", "enabled"),
    get_isolated_voice_enabled: () => state.isolatedVoice,
    set_isolated_voice_enabled: setter("isolatedVoice", "enabled"),
    get_live_transcribe_enabled: () => state.liveTranscribe,
    set_live_transcribe_enabled: setter("liveTranscribe", "enabled"),
    get_ambient_enabled: () => state.ambient,
    set_ambient_enabled: setter("ambient", "enabled"),
    get_ambient_phase: () => (state.ambient ? "listening" : "off"),
    get_autostart_enabled: () => state.autostart,
    set_autostart_enabled: setter("autostart", "enabled"),
    get_journal_enabled: () => state.journal,
    set_journal_enabled: setter("journal", "enabled"),
    get_history_retention_days: () => state.retentionDays,
    set_history_retention_days: setter("retentionDays", "days"),
    get_webhook_url: () => state.webhookUrl,
    set_webhook_url: setter("webhookUrl", "url"),
    send_test_webhook: () => null,
    get_vocabulary: () => state.vocabulary,
    set_vocabulary: setter("vocabulary", "terms"),
    get_project_names_enabled: () => state.projectNames,
    set_project_names_enabled: setter("projectNames", "enabled"),
    get_project_status: () => state.projectStatus,
    get_snippets: () => state.snippets,
    set_snippets: setter("snippets", "snippets"),
    get_modes: () => state.modes,
    set_modes: setter("modes", "modes"),
    list_running_apps: () => [
      { bundle_id: "com.apple.Terminal", name: "Terminal", icon_data_uri: null, is_running: true },
      { bundle_id: "com.tinyspeck.slackmacgap", name: "Slack", icon_data_uri: null, is_running: true },
    ],
    // coding agents
    get_waiting_agents: () => state.agents,
    dismiss_waiting_agent: (args) => {
      state.agents = state.agents.filter((a) => a.session_id !== args.sessionId);
      void emit("agents-changed", state.agents);
      return null;
    },
    focus_waiting_agent: () => "Terminal",
    get_agent_reply_shortcut: () => state.agentShortcut,
    get_agent_reply_shortcut_error: () => null,
    set_agent_reply_shortcut: (args) => {
      state.agentShortcut = args.newCfg as typeof state.agentShortcut;
      return state.agentShortcut;
    },
    get_agent_sound_enabled: () => state.agentSound,
    set_agent_sound_enabled: setter("agentSound", "enabled"),
    get_agent_hook_config: () => ({
      script_path: "/Users/me/Library/Application Support/dev.mouthpiece/bin/mouthpiece-hook",
      json: JSON.stringify(
        {
          hooks: Object.fromEntries(
            ["Notification", "Stop", "UserPromptSubmit"].map((event) => [
              event,
              [
                {
                  hooks: [
                    {
                      type: "command",
                      command: "'/Users/me/Library/Application Support/dev.mouthpiece/bin/mouthpiece-hook'",
                    },
                  ],
                },
              ],
            ]),
          ),
        },
        null,
        2,
      ),
    }),
    // models
    list_models: () => state.models,
    download_model: (args) => {
      const id = args.id as string;
      setTimeout(() => {
        state.models = state.models.map((m) => (m.id === id ? { ...m, downloaded: true } : m));
        void emit("model-download-complete", id);
      }, 50);
      return null;
    },
    set_active_model: (args) => {
      state.models = state.models.map((m) => ({ ...m, active: m.id === args.id }));
      return null;
    },
    // LLM
    list_llm_catalog: () => [
      { id: "gemma3:1b", label: "Gemma 3 (1B) — fastest", size_gb: 0.8, downloaded: false, last_latency_ms: null },
      { id: "qwen3.5:4b", label: "qwen3.5:4b", size_gb: 0, downloaded: true, last_latency_ms: 820 },
    ],
    get_llm_model: () => state.llmModel,
    get_llm_only_when_needed: () => state.llmOnlyWhenNeeded,
    set_llm_only_when_needed: setter("llmOnlyWhenNeeded", "enabled"),
    set_llm_model: setter("llmModel", "model"),
    pull_llm_model: () => null,
    // voice isolation
    get_voice_enrollment_status: () => ({ enrolled: false, enrolled_at_ms: null }),
    start_voice_enrollment: () => null,
    stop_voice_enrollment: () => null,
    // history, insights, logs
    list_history_entries: () => (historyLocked() ? Promise.reject("History is locked.") : state.history),
    search_history_entries: (args) =>
      historyLocked()
        ? Promise.reject("History is locked.")
        : state.history.filter((h) => h.text.toLowerCase().includes(String(args.query).toLowerCase())),
    // privacy (Touch ID always succeeds here)
    discard_recording: () => null,
    undo_discard: () => true,
    request_microphone_access: () => "authorized",
    open_microphone_settings: () => null,
    get_meeting_status: () => state.meetingStatus,
    start_meeting: () => {
      state.meetingStatus = { ...state.meetingStatus, recording: { id: "2026-10-26-174005", started_ms: Date.now() } };
      void emit("recording-started");
      void emit("meeting-changed", state.meetingStatus);
      return null;
    },
    stop_meeting: () => {
      const id = state.meetingStatus.recording?.id;
      if (!id) return Promise.reject("Not recording a meeting");
      state.meetingStatus = {
        recording: null,
        processing: { id, step: "Finding who spoke", percent: null },
        queued: [...state.meetingStatus.queued, id],
      };
      void emit("meeting-stopped");
      void emit("meeting-changed", state.meetingStatus);
      return null;
    },
    list_meetings: () =>
      historyLocked()
        ? Promise.reject("History is locked.")
        : state.meetings.map(({ text: _, ...m }) => m),
    read_meeting: (args) => {
      const m = state.meetings.find((x) => x.id === args.id);
      return m?.text ?? Promise.reject("That meeting has no transcript yet.");
    },
    open_meeting: () => null,
    delete_meeting: (args) => {
      state.meetings = state.meetings.filter((x) => x.id !== args.id);
      return null;
    },
    transcribe_meeting: (args) => {
      state.meetingStatus = { ...state.meetingStatus, queued: [...state.meetingStatus.queued, String(args.id)] };
      void emit("meeting-changed", state.meetingStatus);
      return null;
    },
    get_meeting_speakers: () => state.meetingSpeakers,
    set_meeting_speakers: (args) => {
      state.meetingSpeakers = Number(args.speakers);
      return null;
    },
    get_history_lock: () => lockStatus(),
    set_history_lock_enabled: (args) => {
      state.historyLockEnabled = Boolean(args.enabled);
      state.historyUnlocked = !state.historyLockEnabled;
      return lockStatus();
    },
    unlock_history: () => {
      state.historyUnlocked = true;
      return lockStatus();
    },
    lock_history: () => {
      state.historyUnlocked = false;
      return lockStatus();
    },
    get_log_transcripts: () => state.logTranscripts,
    set_log_transcripts: setter("logTranscripts", "enabled"),
    reprocess_history_text: (args) => String(args.text),
    update_history_entry_text: (args) => {
      state.history = state.history.map((h) => (h.timestamp_ms === args.timestampMs ? { ...h, text: String(args.text) } : h));
      return null;
    },
    use_original_history_text: (args) => {
      state.history = state.history.map((h) => {
        if (h.timestamp_ms !== args.timestampMs || !h.original_text) return h;
        const { original_text, ...rest } = h;
        return { ...rest, text: original_text };
      });
      return null;
    },
    // learning from corrections
    get_learn_corrections: () => state.learnCorrections,
    set_learn_corrections: setter("learnCorrections", "enabled"),
    get_correction_suggestions: () => state.corrections,
    accept_correction_suggestion: (args) => {
      const hit = state.corrections.find((c) => c.from === args.from && c.to === args.to);
      state.corrections = state.corrections.filter((c) => c !== hit);
      if (!state.vocabulary.includes(String(args.to))) state.vocabulary = [...state.vocabulary, String(args.to)];
      if (hit?.source === "edit") state.replacements = [...state.replacements, { from: String(args.from), to: String(args.to) }];
      return null;
    },
    dismiss_correction_suggestion: (args) => {
      state.corrections = state.corrections.filter((c) => !(c.from === args.from && c.to === args.to));
      return null;
    },
    get_replacements: () => state.replacements,
    set_replacements: setter("replacements", "replacements"),
    delete_history_entry: (args) => {
      state.history = state.history.filter((h) => h.timestamp_ms !== args.timestampMs);
      return null;
    },
    clear_history: () => {
      state.history = [];
      return null;
    },
    get_insights: () => state.insights,
    get_logs: () => state.logs,
    clear_logs: () => {
      state.logs = [];
      return null;
    },
    // plugins
    "plugin:app|version": () => "0.1.2",
    "plugin:updater|check": () => null,
    "plugin:window|start_dragging": () => null,
    "plugin:opener|open_url": () => null,
  };

  mockIPC(
    (cmd, args) => {
      const payload = args as Record<string, Json> | undefined;
      calls.push({ cmd, args: payload });
      const failure = scenario.failing?.[cmd];
      if (failure) return Promise.reject(failure);
      const handler = handlers[cmd];
      if (!handler) {
        console.warn(`[mock backend] unhandled command ${cmd}`, args);
        return null;
      }
      return handler(payload ?? {});
    },
    { shouldMockEvents: true },
  );

  window.__dw = { emit: (event, payload) => emit(event, payload), calls, state };
}
