import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { shortcutLabel, type ShortcutConfig } from "../shortcut";
import { type WaveformConfig } from "../waveformConfig";
import { ReadinessPanel } from "./ReadinessPanel";
import { type ReadinessFix } from "./readiness";
import { IconAgent, IconBook, IconChart, IconClock, IconCpu, IconGear, IconInfo, IconMic, IconPalette, IconPeople, IconPlug, IconWave, IconWindow } from "./icons";
import { type ModelStatus } from "./shared";
import { AgentsSection } from "./sections/agents";
import { AboutSection, LogsSection, UpdatesSection } from "./sections/app";
import { DeviceSection, GeneralSection, ShortcutSection, VoiceCommandsSection } from "./sections/dictation";
import { HistorySection } from "./sections/history";
import { InsightsSection } from "./sections/insights";
import { DeliverySection, WebhookSection } from "./sections/integrations";
import { LlmSection } from "./sections/llm";
import { MeetingsSection } from "./sections/meetings";
import { AppModesSection } from "./sections/modes";
import { SnippetsSection, VocabularySection } from "./sections/vocabulary";
import { ModelsSection, VoiceIsolationSection } from "./sections/voice";

export type PageId =
  | "dictation"
  | "voice"
  | "vocabulary"
  | "modes"
  | "llm"
  | "insights"
  | "history"
  | "meetings"
  | "integrations"
  | "agents"
  | "appearance"
  | "about"
  | "advanced";

export const NAV: { id: PageId; label: string; icon: () => ReactElement }[] = [
  { id: "dictation", label: "Dictation", icon: IconMic },
  { id: "appearance", label: "Appearance", icon: IconPalette },
  { id: "voice", label: "Voice", icon: IconWave },
  { id: "vocabulary", label: "Vocabulary", icon: IconBook },
  { id: "modes", label: "Modes", icon: IconWindow },
  { id: "llm", label: "LLM", icon: IconCpu },
  { id: "insights", label: "Insights", icon: IconChart },
  { id: "history", label: "History", icon: IconClock },
  { id: "meetings", label: "Meetings", icon: IconPeople },
  { id: "integrations", label: "Integrations", icon: IconPlug },
  { id: "agents", label: "Coding agents", icon: IconAgent },
];

export const FOOTER_NAV: { id: PageId; label: string; icon: () => ReactElement }[] = [
  { id: "about", label: "About", icon: IconInfo },
  { id: "advanced", label: "Advanced", icon: IconGear },
];

export const PAGE_TITLE: Record<PageId, string> = {
  dictation: "Dictation",
  voice: "Voice — Whisper model & isolation",
  vocabulary: "Vocabulary",
  modes: "Modes — app-aware formatting rules",
  llm: "LLM — local refinement",
  insights: "Insights — usage & feature adoption",
  history: "History",
  meetings: "Meetings — transcripts split by speaker",
  integrations: "Integrations — delivery & webhook",
  agents: "Coding agents",
  appearance: "Appearance",
  about: "About — features & how it works",
  advanced: "Advanced",
};

export function NavItem({
  label,
  icon,
  active,
  onClick,
}: {
  label: string;
  icon: ReactElement;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className={`dw-nav-item ${active ? "active" : ""}`} onClick={onClick}>
      <span className="dw-icon">{icon}</span>
      {label}
    </button>
  );
}

// Where each in-app readiness fix lives, per layout.
export const FIX_PAGE: Partial<Record<ReadinessFix, PageId>> = {
  download_model: "voice",
  change_shortcut: "dictation",
  choose_microphone: "dictation",
  pull_llm_model: "llm",
};

export const FIX_NODE: Partial<Record<ReadinessFix, NodeId>> = {
  download_model: "recognition",
  change_shortcut: "input",
  choose_microphone: "input",
  pull_llm_model: "refinement",
};

export type SettingsNavigation = {
  page: string;
  meeting_id: string | null;
};

export function useSettingsNavigation(onNavigate: (nav: SettingsNavigation) => void) {
  const callbackRef = useRef(onNavigate);
  callbackRef.current = onNavigate;

  useEffect(() => {
    invoke<SettingsNavigation | null>("take_settings_navigation")
      .then((nav) => {
        if (nav) callbackRef.current(nav);
      })
      .catch(() => {});

    const unlisten = listen<SettingsNavigation>("navigate-settings", (e) => {
      invoke<SettingsNavigation | null>("take_settings_navigation")
        .then((pending) => callbackRef.current(pending ?? e.payload))
        .catch(() => callbackRef.current(e.payload));
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);
}

export function SidebarLayout({
  appearance,
  waveform,
  onChangeWaveform,
  focusMeetingId = null,
  focusNonce = 0,
  onFocusHandled,
}: {
  appearance: ReactElement;
  waveform?: WaveformConfig;
  onChangeWaveform?: (next: WaveformConfig) => void;
  focusMeetingId?: string | null;
  focusNonce?: number;
  onFocusHandled?: () => void;
}) {
  const [page, setPage] = useState<PageId>(focusNonce > 0 ? "meetings" : "dictation");
  const pageRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (focusNonce > 0) {
      setPage("meetings");
    }
  }, [focusNonce]);

  useEffect(() => {
    pageRef.current?.scrollTo({ top: 0 });
  }, [page]);

  return (
    <main className="dw-shell h-screen">
      <div className="dw-app-shell">
        <nav className="dw-sidebar">
          <div className="dw-brand">
            <IconMic />
            Mouthpiece
          </div>
          {NAV.map((item) => (
            <NavItem
              key={item.id}
              label={item.label}
              icon={item.icon()}
              active={page === item.id}
              onClick={() => setPage(item.id)}
            />
          ))}
          <div className="dw-sidebar-foot">
            {FOOTER_NAV.map((item) => (
              <NavItem
                key={item.id}
                label={item.label}
                icon={item.icon()}
                active={page === item.id}
                onClick={() => setPage(item.id)}
              />
            ))}
          </div>
        </nav>

        <div className="dw-page" ref={pageRef}>
          <h2 className="dw-page-title">{PAGE_TITLE[page]}</h2>
          <div className="dw-page-body">
            <ReadinessPanel onNavigate={(fix) => setPage(FIX_PAGE[fix] ?? page)} />
            {page === "dictation" && (
              <>
                <GeneralSection
                  onOpenAppearance={() => setPage("appearance")}
                  waveform={waveform}
                  onChangeWaveform={onChangeWaveform}
                />
                <DeviceSection />
                <ShortcutSection />
                <VoiceCommandsSection />
              </>
            )}
            {page === "voice" && (
              <>
                <ModelsSection />
                <VoiceIsolationSection />
              </>
            )}
            {page === "vocabulary" && (
              <>
                <VocabularySection />
                <SnippetsSection />
              </>
            )}
            {page === "modes" && <AppModesSection />}
            {page === "llm" && <LlmSection />}
            {page === "insights" && <InsightsSection />}
            {page === "history" && <HistorySection />}
            {page === "meetings" && <MeetingsSection focusMeetingId={focusMeetingId} onFocusHandled={onFocusHandled} />}
            {page === "integrations" && (
              <>
                <DeliverySection />
                <WebhookSection />
              </>
            )}
            {page === "agents" && <AgentsSection />}
            {page === "appearance" && appearance}
            {page === "about" && (
              <>
                <UpdatesSection />
                <AboutSection />
              </>
            )}
            {page === "advanced" && <LogsSection />}
          </div>
        </div>
      </div>
    </main>
  );
}

export type NodeId = "input" | "recognition" | "mode" | "refinement" | "output";

export type ActiveNode = NodeId | "app" | null;

export const CHAIN: { id: NodeId; stage: string; label: string; icon: () => ReactElement }[] = [
  { id: "input", stage: "01", label: "Input", icon: IconMic },
  { id: "recognition", stage: "02", label: "Recognition", icon: IconWave },
  { id: "mode", stage: "03", label: "Mode", icon: IconWindow },
  { id: "refinement", stage: "04", label: "Refinement", icon: IconCpu },
  { id: "output", stage: "05", label: "Output", icon: IconPlug },
];

export const NODE_DRAWER_TITLE: Record<NodeId, string> = {
  input: "Input — microphone & shortcut",
  recognition: "Recognition — Whisper model, voice isolation, vocabulary",
  mode: "Mode — app-aware formatting rules",
  refinement: "Refinement — local LLM (Ollama)",
  output: "Output — delivery, webhook, history, meetings & insights",
};

type ChainAudioDeviceStatus =
  | { kind: "preferred"; using: string }
  | { kind: "fallback"; wanted: string; using: string }
  | { kind: "system_default"; using: string }
  | { kind: "unavailable"; wanted: string | null };

type ChainFrontmostApp = {
  bundle_id: string;
  name: string;
  icon_data_uri?: string | null;
};

type ChainModeSummary = {
  name: string;
  apps: { bundle_id: string; app_name: string }[];
  is_default: boolean;
};

type ChainInsightsSummary = {
  latency?: { median_ms: number; p90_ms: number; samples: number } | null;
};

type ChainTelemetry = {
  micName: string | null;
  micUnavailable: boolean;
  shortcut: string | null;
  whisperModel: string | null;
  voiceIso: boolean | null;
  vocabCount: number | null;
  frontmostApp: ChainFrontmostApp | null;
  modes: ChainModeSummary[];
  llmModel: string | null;
  medianLatencyMs: number | null;
  waitingAgents: number | null;
};

const EMPTY_TELEMETRY: ChainTelemetry = {
  micName: null,
  micUnavailable: false,
  shortcut: null,
  whisperModel: null,
  voiceIso: null,
  vocabCount: null,
  frontmostApp: null,
  modes: [],
  llmModel: null,
  medianLatencyMs: null,
  waitingAgents: null,
};

function parseDeviceStatus(status: ChainAudioDeviceStatus | null | undefined): {
  micName: string | null;
  micUnavailable: boolean;
} {
  if (!status || typeof status !== "object") return { micName: null, micUnavailable: false };
  if (status.kind === "unavailable") return { micName: "No mic", micUnavailable: true };
  return { micName: status.using ?? null, micUnavailable: false };
}

function compactLlmModel(model: string | null): string {
  if (!model || model === "off") return "Off";
  return model.startsWith("lmstudio::") ? model.slice("lmstudio::".length) : model;
}

function formatCompactLatency(ms: number | null): string | null {
  if (ms == null || !Number.isFinite(ms)) return null;
  return ms < 1000 ? `~${Math.round(ms)}ms` : `~${(ms / 1000).toFixed(1)}s`;
}

function resolveModeTelemetry(
  frontmostApp: ChainFrontmostApp | null,
  modes: ChainModeSummary[],
): { appName: string | null; modeName: string | null } {
  const matched = frontmostApp
    ? modes.find((m) => m.apps.some((a) => a.bundle_id === frontmostApp.bundle_id))
    : undefined;
  const resolved = matched ?? modes.find((m) => m.is_default) ?? modes[0] ?? null;
  return {
    appName: frontmostApp?.name ?? null,
    modeName: resolved?.name ?? null,
  };
}

export function ChainNode({
  label,
  icon,
  active,
  onClick,
  stage,
  telemetry,
  badge,
  badgeTone = "muted",
  pulsing = false,
  pulseColor,
  audioLevel = 0,
  warning = false,
  tooltip,
  nodeId,
}: {
  label: string;
  icon: ReactElement;
  active: boolean;
  onClick: () => void;
  stage?: string;
  telemetry?: string | null;
  badge?: string | null;
  badgeTone?: "muted" | "accent" | "cyan" | "refine" | "warning";
  pulsing?: boolean;
  pulseColor?: string;
  audioLevel?: number;
  warning?: boolean;
  tooltip?: string;
  nodeId?: string;
}) {
  const accent = pulseColor ?? "var(--dw-accent)";
  const lit = active || pulsing;
  const audioGlow = Math.min(1, Math.pow(Math.max(0, audioLevel) * 3.2, 0.7));
  const audioLive = audioGlow > 0.08;

  const badgeStyle =
    badgeTone === "accent"
      ? {
          color: "var(--dw-accent)",
          backgroundColor: "color-mix(in srgb, var(--dw-accent) 16%, transparent)",
          borderColor: "color-mix(in srgb, var(--dw-accent) 42%, transparent)",
        }
      : badgeTone === "cyan"
        ? {
            color: "var(--dw-cyan, #06b6d4)",
            backgroundColor: "color-mix(in srgb, var(--dw-cyan, #06b6d4) 16%, transparent)",
            borderColor: "color-mix(in srgb, var(--dw-cyan, #06b6d4) 42%, transparent)",
          }
        : badgeTone === "refine"
          ? {
              color: "color-mix(in srgb, var(--dw-refine, #a78bfa) 74%, var(--dw-fg))",
              backgroundColor: "color-mix(in srgb, var(--dw-refine, #a78bfa) 18%, transparent)",
              borderColor: "color-mix(in srgb, var(--dw-refine, #a78bfa) 42%, transparent)",
            }
          : badgeTone === "warning"
            ? {
                color: "var(--color-warning)",
                backgroundColor: "color-mix(in srgb, var(--color-warning) 16%, transparent)",
                borderColor: "color-mix(in srgb, var(--color-warning) 40%, transparent)",
              }
            : {
                color: "var(--dw-muted)",
                backgroundColor: "color-mix(in srgb, var(--dw-muted) 10%, transparent)",
                borderColor: "color-mix(in srgb, var(--dw-border-strong, var(--dw-border)) 75%, transparent)",
              };

  const ledColor = audioLive
    ? "#34d399"
    : pulsing
      ? accent
      : warning
        ? "var(--color-error)"
        : active
          ? "var(--dw-accent)"
          : "color-mix(in srgb, var(--dw-muted) 35%, transparent)";

  const ledShadow = audioLive
    ? `0 0 ${4 + Math.round(audioGlow * 8)}px #34d399`
    : pulsing
      ? `0 0 9px ${accent}`
      : active
        ? "0 0 6px var(--dw-accent)"
        : undefined;

  return (
    <button
      type="button"
      className={`dw-chain-node min-w-[134px] ${active ? "active" : ""} ${pulsing ? "is-pulsing" : ""}`}
      onClick={onClick}
      aria-pressed={active}
      title={tooltip}
      data-node-id={nodeId}
      data-pulsing={pulsing ? "true" : undefined}
      style={
        pulsing
          ? {
              borderColor: accent,
              color: "var(--dw-fg)",
              background: `linear-gradient(180deg, color-mix(in srgb, ${accent} 18%, var(--dw-card)) 0%, var(--dw-card) 100%)`,
              boxShadow: `0 0 0 1.5px ${accent}, 0 0 22px -2px color-mix(in srgb, ${accent} 62%, transparent), inset 0 1px 0 color-mix(in srgb, ${accent} 42%, transparent)`,
            }
          : active
            ? {
                background:
                  "linear-gradient(180deg, color-mix(in srgb, var(--dw-accent) 10%, var(--dw-card)) 0%, var(--dw-card) 100%)",
                boxShadow:
                  "0 0 0 1px var(--dw-accent), 0 0 18px -3px color-mix(in srgb, var(--dw-accent) 42%, transparent), inset 0 1px 0 color-mix(in srgb, var(--dw-accent) 30%, transparent)",
              }
            : undefined
      }
    >
      {stage && (
        <span
          className="flex w-full items-center justify-between gap-1 -mt-1 px-0.5"
          aria-hidden="true"
        >
          <span
            className="text-[9px] font-semibold tracking-wider tabular-nums"
            style={{
              fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
              color: lit ? accent : "var(--dw-muted)",
              opacity: lit ? 1 : 0.72,
            }}
          >
            {stage}
          </span>
          {badge && (
            <span
              className="max-w-[74px] truncate rounded border px-1 py-[1px] text-[8.5px] font-semibold leading-none tracking-tight tabular-nums"
              style={{
                fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
                ...badgeStyle,
              }}
            >
              {badge}
            </span>
          )}
          <span
            className="h-1.5 w-1.5 shrink-0 rounded-full transition-all duration-100"
            style={{
              backgroundColor: ledColor,
              boxShadow: ledShadow,
              transform: audioLive ? `scale(${(1 + audioGlow * 0.45).toFixed(2)})` : pulsing ? "scale(1.25)" : undefined,
            }}
          />
        </span>
      )}
      <span
        className="dw-icon transition-all duration-150"
        style={
          lit
            ? {
                borderColor: `color-mix(in srgb, ${accent} 58%, var(--dw-border))`,
                backgroundColor: `color-mix(in srgb, ${accent} ${pulsing ? 22 : 14}%, var(--dw-bg))`,
                color: accent,
                boxShadow: `0 0 ${pulsing ? 16 : 12}px -2px color-mix(in srgb, ${accent} ${pulsing ? 60 : 40}%, transparent)`,
              }
            : undefined
        }
      >
        {icon}
      </span>
      <span className="dw-label">{label}</span>
      <span
        className="dw-chain-telemetry"
        aria-hidden="true"
        style={
          warning
            ? {
                color: "var(--color-error)",
                borderColor: "color-mix(in srgb, var(--color-error) 40%, var(--dw-border))",
              }
            : pulsing
              ? {
                  color: accent,
                  borderColor: `color-mix(in srgb, ${accent} 48%, var(--dw-border))`,
                  backgroundColor: `color-mix(in srgb, ${accent} 14%, var(--dw-bg-sunk))`,
                }
              : undefined
        }
      >
        {telemetry ?? "—"}
      </span>
    </button>
  );
}

function ChainConnector({
  lit,
  pulsing = false,
  loop = false,
  pulseKey = 0,
  pulseColor,
}: {
  lit: boolean;
  pulsing?: boolean;
  loop?: boolean;
  pulseKey?: number;
  pulseColor?: string;
}) {
  const accent = pulseColor ?? "var(--dw-accent)";
  const illuminated = lit || pulsing;
  const strokeColor = illuminated
    ? `color-mix(in srgb, ${accent} 76%, var(--dw-border))`
    : "var(--dw-border)";
  return (
    <div
      className="dw-chain-link flex items-center justify-between"
      aria-hidden="true"
      style={{
        background: illuminated
          ? `linear-gradient(90deg, color-mix(in srgb, ${accent} 45%, var(--dw-border)), ${accent})`
          : "var(--dw-border)",
        boxShadow: illuminated ? `0 0 8px color-mix(in srgb, ${accent} 48%, transparent)` : undefined,
        transition: "background 0.2s ease, box-shadow 0.2s ease",
      }}
    >
      <span
        className="h-1.5 w-1.5 -ml-[2px] rounded-full"
        style={{ backgroundColor: strokeColor }}
      />
      <span
        className="px-0.5 text-[10px] leading-none"
        style={{
          fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
          color: illuminated ? accent : "var(--dw-muted)",
          backgroundColor: "var(--dw-bg)",
          opacity: illuminated ? 1 : 0.6,
        }}
      >
        ›
      </span>
      <span
        className="h-1.5 w-1.5 -mr-[2px] rounded-full"
        style={{ backgroundColor: strokeColor }}
      />
      {pulsing && (
        <span
          key={pulseKey}
          className="dw-chain-pulse"
          style={{
            animationDuration: loop ? "950ms" : "260ms",
            animationIterationCount: loop ? "infinite" : 1,
            animationFillMode: "both",
            ...(pulseColor
              ? {
                  background: `linear-gradient(90deg, transparent 0%, ${pulseColor} 60%, #ffffff 100%)`,
                }
              : undefined),
          }}
        />
      )}
    </div>
  );
}

const TEST_STEP_MS = 280; // 5 stages × 280ms = 1.4s total sequential signal sweep

export function ChainLayout({
  appearance,
  waveform,
  onChangeWaveform,
  focusMeetingId = null,
  focusNonce = 0,
  onFocusHandled,
}: {
  appearance: ReactElement;
  waveform?: WaveformConfig;
  onChangeWaveform?: (next: WaveformConfig) => void;
  focusMeetingId?: string | null;
  focusNonce?: number;
  onFocusHandled?: () => void;
}) {
  const [active, setActive] = useState<ActiveNode>(focusNonce > 0 ? "output" : "input");
  const [telemetry, setTelemetry] = useState<ChainTelemetry>(EMPTY_TELEMETRY);
  const [audioLevel, setAudioLevel] = useState(0);
  const [liveStage, setLiveStage] = useState<NodeId | null>(null);
  const [testStep, setTestStep] = useState<number | null>(null);
  const [testRunId, setTestRunId] = useState(0);

  const mountedRef = useRef(true);
  const telemetrySeqRef = useRef(0);
  const audioDecayRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const liveClearRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const testTimersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (audioDecayRef.current) clearTimeout(audioDecayRef.current);
      if (liveClearRef.current) clearTimeout(liveClearRef.current);
      for (const timer of testTimersRef.current) clearTimeout(timer);
      testTimersRef.current = [];
    };
  }, []);

  const refreshTelemetry = useCallback(() => {
    const seq = ++telemetrySeqRef.current;
    Promise.allSettled([
      invoke<ChainAudioDeviceStatus>("get_audio_device_status"),
      invoke<ShortcutConfig>("get_shortcut"),
      invoke<ModelStatus[]>("list_models"),
      invoke<boolean>("get_isolated_voice_enabled"),
      invoke<string[]>("get_vocabulary"),
      invoke<ChainFrontmostApp | null>("get_last_frontmost_app"),
      invoke<ChainModeSummary[]>("get_modes"),
      invoke<string | null>("get_llm_model"),
      invoke<ChainInsightsSummary>("get_insights"),
      invoke<unknown[]>("get_waiting_agents"),
    ]).then((results) => {
      if (!mountedRef.current || seq !== telemetrySeqRef.current) return;
      const [
        statusRes,
        shortcutRes,
        modelsRes,
        isoRes,
        vocabRes,
        appRes,
        modesRes,
        llmRes,
        insightsRes,
        agentsRes,
      ] = results;

      setTelemetry((cur) => {
        const next: ChainTelemetry = { ...cur };
        if (statusRes.status === "fulfilled") {
          const parsed = parseDeviceStatus(statusRes.value);
          next.micName = parsed.micName;
          next.micUnavailable = parsed.micUnavailable;
        }
        if (shortcutRes.status === "fulfilled" && shortcutRes.value) {
          next.shortcut = shortcutLabel(shortcutRes.value);
        }
        if (modelsRes.status === "fulfilled" && Array.isArray(modelsRes.value)) {
          const activeModel = modelsRes.value.find((m) => m.active) ?? null;
          next.whisperModel = activeModel?.id ?? null;
        }
        if (isoRes.status === "fulfilled" && typeof isoRes.value === "boolean") {
          next.voiceIso = isoRes.value;
        }
        if (vocabRes.status === "fulfilled" && Array.isArray(vocabRes.value)) {
          next.vocabCount = vocabRes.value.length;
        }
        if (appRes.status === "fulfilled") {
          next.frontmostApp = appRes.value ?? null;
        }
        if (modesRes.status === "fulfilled" && Array.isArray(modesRes.value)) {
          next.modes = modesRes.value;
        }
        if (llmRes.status === "fulfilled") {
          next.llmModel = llmRes.value ?? null;
        }
        if (insightsRes.status === "fulfilled") {
          next.medianLatencyMs = insightsRes.value?.latency?.median_ms ?? null;
        }
        if (agentsRes.status === "fulfilled" && Array.isArray(agentsRes.value)) {
          next.waitingAgents = agentsRes.value.length;
        }
        return next;
      });
    });
  }, []);

  useEffect(() => {
    refreshTelemetry();
  }, [refreshTelemetry, active]);

  useEffect(() => {
    const clearLiveTimer = () => {
      if (liveClearRef.current) {
        clearTimeout(liveClearRef.current);
        liveClearRef.current = null;
      }
    };

    const unlisteners = [
      listen<ChainAudioDeviceStatus>("audio-device-status", (e) => {
        if (!mountedRef.current) return;
        const parsed = parseDeviceStatus(e.payload);
        setTelemetry((cur) => ({ ...cur, ...parsed }));
      }),
      listen<number>("audio-level", (e) => {
        if (!mountedRef.current) return;
        const nextLevel = typeof e.payload === "number" && Number.isFinite(e.payload) ? Math.max(0, Math.min(1, e.payload)) : 0;
        setAudioLevel(nextLevel);
        if (audioDecayRef.current) clearTimeout(audioDecayRef.current);
        audioDecayRef.current = setTimeout(() => {
          if (mountedRef.current) setAudioLevel(0);
        }, 300);
      }),
      listen("recording-started", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage("input");
      }),
      listen("recording-stopped", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage("recognition");
      }),
      listen("refining-started", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage("refinement");
      }),
      listen("transcript-ready", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage("output");
        refreshTelemetry();
        liveClearRef.current = setTimeout(() => {
          if (mountedRef.current) setLiveStage(null);
        }, 1200);
      }),
      listen("transcript-copied", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage("output");
        refreshTelemetry();
        liveClearRef.current = setTimeout(() => {
          if (mountedRef.current) setLiveStage(null);
        }, 1200);
      }),
      listen("dictation-discarded", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage(null);
      }),
      listen("recording-error", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage(null);
      }),
      listen("transcript-error", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage(null);
      }),
      listen("paste-needs-access", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage(null);
      }),
      listen("meeting-stopped", () => {
        if (!mountedRef.current) return;
        clearLiveTimer();
        setLiveStage(null);
      }),
      listen<unknown[]>("agents-changed", (e) => {
        if (!mountedRef.current) return;
        if (Array.isArray(e.payload)) {
          setTelemetry((cur) => ({ ...cur, waitingAgents: e.payload.length }));
        } else {
          invoke<unknown[]>("get_waiting_agents")
            .then((agents) => {
              if (mountedRef.current && Array.isArray(agents)) {
                setTelemetry((cur) => ({ ...cur, waitingAgents: agents.length }));
              }
            })
            .catch(() => {});
        }
      }),
      listen<string[]>("vocabulary-changed", (e) => {
        if (!mountedRef.current || !Array.isArray(e.payload)) return;
        setTelemetry((cur) => ({ ...cur, vocabCount: e.payload.length }));
      }),
      listen<ChainModeSummary[]>("modes-changed", (e) => {
        if (!mountedRef.current) return;
        if (Array.isArray(e.payload)) {
          setTelemetry((cur) => ({ ...cur, modes: e.payload }));
        } else {
          refreshTelemetry();
        }
      }),
      listen("model-download-complete", () => {
        if (!mountedRef.current) return;
        refreshTelemetry();
      }),
      listen("shortcut-changed", () => {
        if (!mountedRef.current) return;
        refreshTelemetry();
      }),
      listen("history-updated", () => {
        if (!mountedRef.current) return;
        invoke<ChainInsightsSummary>("get_insights")
          .then((insights) => {
            if (!mountedRef.current) return;
            setTelemetry((cur) => ({ ...cur, medianLatencyMs: insights?.latency?.median_ms ?? null }));
          })
          .catch(() => {});
      }),
    ];

    return () => {
      for (const p of unlisteners) {
        p.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, [refreshTelemetry]);

  useEffect(() => {
    if (focusNonce > 0) {
      setActive("output");
    }
  }, [focusNonce]);

  function toggle(id: ActiveNode) {
    setActive((current) => (current === id ? null : id));
  }

  function runTestSignalChain() {
    for (const timer of testTimersRef.current) clearTimeout(timer);
    testTimersRef.current = [];
    setTestRunId((id) => id + 1);
    setTestStep(0);

    for (let step = 1; step < CHAIN.length; step++) {
      testTimersRef.current.push(
        setTimeout(() => {
          if (mountedRef.current) setTestStep(step);
        }, step * TEST_STEP_MS),
      );
    }

    testTimersRef.current.push(
      setTimeout(() => {
        if (mountedRef.current) setTestStep(null);
      }, CHAIN.length * TEST_STEP_MS),
    );
  }

  const activeChainIndex = active && active !== "app" ? CHAIN.findIndex((n) => n.id === active) : -1;
  const liveStageIndex = liveStage ? CHAIN.findIndex((n) => n.id === liveStage) : -1;
  const testing = testStep !== null;

  const activeStage =
    active === "app"
      ? { stage: "06", label: "APP" }
      : activeChainIndex >= 0
        ? { stage: CHAIN[activeChainIndex].stage, label: CHAIN[activeChainIndex].label.toUpperCase() }
        : null;

  const { appName: modeAppName, modeName } = resolveModeTelemetry(telemetry.frontmostApp, telemetry.modes);
  const latencyLabel = formatCompactLatency(telemetry.medianLatencyMs);
  const llmLabel = compactLlmModel(telemetry.llmModel);
  const waitingCount = telemetry.waitingAgents ?? 0;

  const nodeTelemetry: Record<
    NodeId,
    {
      telemetry: string | null;
      badge: string | null;
      badgeTone: "muted" | "accent" | "cyan" | "refine" | "warning";
      warning?: boolean;
      tooltip: string;
    }
  > = {
    input: {
      telemetry: telemetry.micName ?? "System mic",
      badge: telemetry.shortcut,
      badgeTone: telemetry.micUnavailable ? "warning" : "muted",
      warning: telemetry.micUnavailable,
      tooltip: [
        `Mic: ${telemetry.micName ?? "System default"}`,
        telemetry.shortcut ? `Hotkey: ${telemetry.shortcut}` : null,
      ]
        .filter(Boolean)
        .join(" · "),
    },
    recognition: {
      telemetry:
        telemetry.whisperModel || telemetry.vocabCount !== null
          ? [telemetry.whisperModel ?? "Whisper", telemetry.vocabCount !== null ? `${telemetry.vocabCount}w` : null]
              .filter(Boolean)
              .join(" · ")
          : null,
      badge: telemetry.voiceIso === null ? null : telemetry.voiceIso ? "ISO ON" : "STD",
      badgeTone: telemetry.voiceIso ? "cyan" : "muted",
      tooltip: [
        `Model: ${telemetry.whisperModel ?? "None"}`,
        `Voice Isolation: ${telemetry.voiceIso ? "On" : "Standard"}`,
        telemetry.vocabCount !== null ? `Vocabulary: ${telemetry.vocabCount} terms` : null,
      ]
        .filter(Boolean)
        .join(" · "),
    },
    mode: {
      telemetry: modeAppName && modeName ? `${modeAppName} → ${modeName}` : modeName ?? modeAppName ?? "Default",
      badge: modeName,
      badgeTone: modeName && modeName !== "Default" ? "accent" : "muted",
      tooltip: modeAppName ? `${modeAppName} → ${modeName ?? "Default"} mode` : `Mode: ${modeName ?? "Default"}`,
    },
    refinement: {
      telemetry: llmLabel,
      badge: latencyLabel,
      badgeTone: llmLabel === "Off" ? "muted" : "refine",
      tooltip: [`LLM: ${llmLabel}`, latencyLabel ? `Median latency: ${latencyLabel}` : null]
        .filter(Boolean)
        .join(" · "),
    },
    output: {
      telemetry: waitingCount > 0 ? `${waitingCount} waiting` : "Paste ready",
      badge: waitingCount > 0 ? `${waitingCount} LIVE` : "RDY",
      badgeTone: waitingCount > 0 ? "accent" : "muted",
      tooltip:
        waitingCount > 0
          ? `${waitingCount} coding ${waitingCount === 1 ? "agent" : "agents"} waiting`
          : "Ready to paste",
    },
  };

  const statusText = testing
    ? `TEST PULSE · ${CHAIN[testStep].stage} ${CHAIN[testStep].label.toUpperCase()}`
    : liveStageIndex >= 0
      ? `LIVE · ${CHAIN[liveStageIndex].stage} ${CHAIN[liveStageIndex].label.toUpperCase()}`
      : activeStage
        ? `STAGE ${activeStage.stage} · ${activeStage.label}`
        : "SIGNAL PATH · 01→06";

  const statusAccent =
    liveStage === "refinement"
      ? "var(--dw-refine)"
      : testing || liveStageIndex >= 0 || activeStage
        ? "var(--dw-accent)"
        : "var(--dw-muted)";

  return (
    <main className="dw-shell h-screen overflow-y-auto">
      <div className="dw-titlebar flex items-center justify-between gap-3">
        <h1>Mouthpiece — signal chain</h1>
        <div className="flex items-center gap-2">
          <button
            type="button"
            data-testid="test-signal-chain"
            onClick={runTestSignalChain}
            className="inline-flex cursor-pointer items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[10.5px] font-semibold tracking-wide transition-all hover:brightness-115 focus-visible:outline-2 focus-visible:outline-offset-2"
            style={{
              fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
              borderColor: testing
                ? "var(--dw-accent)"
                : "color-mix(in srgb, var(--dw-accent) 42%, var(--dw-border))",
              backgroundColor: testing
                ? "color-mix(in srgb, var(--dw-accent) 22%, var(--dw-card))"
                : "color-mix(in srgb, var(--dw-accent) 10%, var(--dw-card))",
              color: "var(--dw-accent)",
              boxShadow: testing
                ? "0 0 14px -2px color-mix(in srgb, var(--dw-accent) 55%, transparent)"
                : undefined,
            }}
          >
            <span
              className="h-1.5 w-1.5 rounded-full"
              aria-hidden="true"
              style={{
                backgroundColor: "var(--dw-accent)",
                boxShadow: "0 0 6px var(--dw-accent)",
              }}
            />
            {testing ? "Testing…" : "Test chain"}
          </button>
          <span
            className="inline-flex items-center gap-1.5 rounded-full border border-base-content/10 bg-base-100/70 px-2.5 py-0.5 text-[10px] font-medium tracking-wider tabular-nums"
            style={{
              fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
              color: statusAccent,
            }}
            aria-hidden="true"
          >
            <span
              className="h-1.5 w-1.5 rounded-full"
              style={{
                backgroundColor: statusAccent,
                boxShadow: statusAccent !== "var(--dw-muted)" ? `0 0 6px ${statusAccent}` : undefined,
              }}
            />
            {statusText}
          </span>
        </div>
      </div>

      <div className="dw-chain-readiness">
        <ReadinessPanel onNavigate={(fix) => setActive(FIX_NODE[fix] ?? active)} />
      </div>

      <div className="dw-chain-row">
        {CHAIN.map((node, idx) => {
          const info = nodeTelemetry[node.id];
          const isNodePulsing = testStep === idx || liveStage === node.id;
          const nodePulseColor =
            liveStage === "refinement" && node.id === "refinement" ? "var(--dw-refine)" : undefined;
          const connectorLit =
            activeChainIndex > idx ||
            (testStep !== null && testStep >= idx) ||
            (liveStageIndex >= 0 && liveStageIndex >= idx);
          const connectorPulsing = testStep === idx || (testStep === null && liveStage === node.id);
          const connectorPulseColor =
            liveStage === "refinement" && node.id === "refinement" ? "var(--dw-refine)" : undefined;

          return (
            <div key={node.id} className="contents">
              <ChainNode
                nodeId={node.id}
                stage={node.stage}
                label={node.label}
                icon={node.icon()}
                active={active === node.id}
                pulsing={isNodePulsing}
                pulseColor={nodePulseColor}
                audioLevel={node.id === "input" ? audioLevel : 0}
                telemetry={info.telemetry}
                badge={info.badge}
                badgeTone={info.badgeTone}
                warning={info.warning}
                tooltip={info.tooltip}
                onClick={() => toggle(node.id)}
              />
              {node.id !== "output" && (
                <ChainConnector
                  lit={connectorLit}
                  pulsing={connectorPulsing}
                  loop={testStep === null && liveStage === node.id}
                  pulseKey={testStep === idx ? testRunId * 10 + idx : idx}
                  pulseColor={connectorPulseColor}
                />
              )}
            </div>
          );
        })}
        <div className="dw-chain-app-gap" />
        <ChainNode
          nodeId="app"
          stage="06"
          label="App"
          icon={<IconGear />}
          active={active === "app"}
          badge="SYS"
          telemetry="Theme & logs"
          tooltip="General, appearance, updates, about & logs"
          onClick={() => toggle("app")}
        />
      </div>

      {active && (
        <div className="dw-drawer">
          <p className="dw-drawer-title flex items-center gap-2">
            {activeStage && (
              <span
                className="rounded border px-1.5 py-0.5 text-[10px] font-semibold tracking-wider tabular-nums"
                style={{
                  fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
                  borderColor: "color-mix(in srgb, var(--dw-accent) 40%, transparent)",
                  color: "var(--dw-accent)",
                  backgroundColor: "color-mix(in srgb, var(--dw-accent) 10%, transparent)",
                }}
                aria-hidden="true"
              >
                {activeStage.stage}
              </span>
            )}
            <span>{active === "app" ? "App — general, appearance & advanced" : NODE_DRAWER_TITLE[active]}</span>
          </p>
          {active === "input" && (
            <>
              <DeviceSection />
              <ShortcutSection />
              <VoiceCommandsSection />
            </>
          )}
          {active === "recognition" && (
            <>
              <ModelsSection />
              <VoiceIsolationSection />
              <VocabularySection />
              <SnippetsSection />
            </>
          )}
          {active === "mode" && <AppModesSection />}
          {active === "refinement" && <LlmSection />}
          {active === "output" && (
            <>
              <DeliverySection />
              <WebhookSection />
              <div className="mb-4 border-t border-base-content/10 pt-3">
                <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide opacity-60">Coding agents</h3>
                <AgentsSection />
              </div>
              <HistorySection />
              <div className="mb-4 border-t border-base-content/10 pt-3">
                <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide opacity-60">Meetings</h3>
                <MeetingsSection focusMeetingId={focusMeetingId} onFocusHandled={onFocusHandled} />
              </div>
              <div className="mb-4 border-t border-base-content/10 pt-3">
                <InsightsSection />
              </div>
            </>
          )}
          {active === "app" && (
            <>
              <GeneralSection
                onOpenAppearance={() => {
                  document
                    .querySelector<HTMLElement>('[data-testid="waveform-studio"]')
                    ?.scrollIntoView({ behavior: "smooth", block: "start" });
                }}
                waveform={waveform}
                onChangeWaveform={onChangeWaveform}
              />
              {appearance}
              <UpdatesSection />
              <div className="mb-4 border-t border-base-content/10 pt-3">
                <AboutSection />
              </div>
              <LogsSection />
            </>
          )}
        </div>
      )}
    </main>
  );
}
