import { type CSSProperties, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { check as checkForUpdate, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { THEME_ORDER, THEME_LABEL, THEMES, type ThemeId } from "../../theme";
import {
  DEFAULT_WAVEFORM_CONFIG,
  getWaveformColorway,
  NEEDLE_PRESETS,
  SOLID_COLORWAY_ORDER,
  SPECTRUM_COLORWAY_ORDER,
  WAVEFORM_COLORWAYS,
  WAVEFORM_ENGINE_META,
  type HyperLayoutMode,
  type NeedlePresetId,
  type WaveformColorwayId,
  type WaveformConfig,
  type WaveformEngineId,
} from "../../waveformConfig";
import { WaveformMeter } from "../../components/WaveformMeter";
import { notifyError } from "../../ui/toast";
import { formatTimestamp } from "./history";

export type LogEntry = { timestamp_ms: number; message: string };

/// Collapsed by default — this is a diagnostic tool ("why didn't it
/// paste?", "which mode did it pick?"), not something most users need
/// open, and polling only while expanded avoids invoke() calls for a
/// panel nobody's looking at.
export function LogsSection() {
  const [open, setOpen] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);

  function refresh() {
    invoke<LogEntry[]>("get_logs")
      .then(setLogs)
      .catch((err) => notifyError("get_logs", err));
  }

  useEffect(() => {
    if (!open) return;
    refresh();
    const interval = setInterval(refresh, 2000);
    return () => clearInterval(interval);
  }, [open]);

  function clear() {
    setLogs([]);
    invoke("clear_logs").catch((err) => notifyError("clear_logs", err));
  }

  return (
    <div className="mb-4 border-t border-base-content/10 pt-3">
      <div className="mb-1 flex items-center justify-between">
        <button
          className="text-xs font-medium opacity-70"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
        >
          {open ? "▾" : "▸"} Logs
        </button>
        {open && (
          <div className="flex items-center gap-1.5">
            <button className="btn btn-ghost btn-xs" onClick={refresh}>
              Refresh
            </button>
            <button className="btn btn-ghost btn-xs" onClick={clear} disabled={logs.length === 0}>
              Clear
            </button>
          </div>
        )}
      </div>
      {open &&
        (logs.length === 0 ? (
          <p className="text-xs opacity-60">No log entries yet.</p>
        ) : (
          <pre className="max-h-48 overflow-y-auto whitespace-pre-wrap rounded-md bg-base-100 p-1.5 font-mono text-[10px] leading-tight">
            {logs.map((l) => `[${formatTimestamp(l.timestamp_ms)}] ${l.message}`).join("\n")}
          </pre>
        ))}
    </div>
  );
}

export function ThemeSection({ theme, onChange }: { theme: ThemeId; onChange: (id: ThemeId) => void }) {
  return (
    <div className="mb-4 border-b border-base-content/10 pb-3">
      <label className="mb-1.5 block text-xs font-medium opacity-70">Appearance</label>
      <div className="flex flex-col gap-1.5">
        {THEME_ORDER.map((id, idx) => {
          const tokens = THEMES[id];
          return (
            <button
              key={id}
              type="button"
              className={`dw-theme-row ${theme === id ? "active" : ""}`}
              onClick={() => onChange(id)}
            >
              <span className="flex items-center gap-2.5">
                <span className="flex items-center -space-x-1" aria-hidden="true">
                  <span
                    className="h-3 w-3 rounded-full border"
                    style={{
                      background: tokens.bg,
                      borderColor: "color-mix(in srgb, var(--dw-fg) 22%, transparent)",
                    }}
                  />
                  <span
                    className="h-3 w-3 rounded-full border"
                    style={{
                      background: tokens.card,
                      borderColor: "color-mix(in srgb, var(--dw-fg) 22%, transparent)",
                    }}
                  />
                  <span
                    className="dw-theme-dot border"
                    style={{
                      background: tokens.accent,
                      borderColor: "color-mix(in srgb, var(--dw-fg) 26%, transparent)",
                    }}
                  />
                </span>
                <span>{THEME_LABEL[id]}</span>
              </span>
              <span className="flex items-center gap-2">
                {theme === id && <span aria-hidden="true">✓</span>}
                <kbd className="dw-kbd">⌘{idx + 1}</kbd>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export type LayoutId = "sidebar" | "chain";

export const LAYOUT_LABEL: Record<LayoutId, string> = { sidebar: "Signal", chain: "Signal Chain" };

export const LAYOUT_DESCRIPTION: Record<LayoutId, string> = {
  sidebar: "Left sidebar, one page of content at a time",
  chain: "Horizontal pipeline — click a stage to expand it",
};

export function LayoutSection({ layout, onChange }: { layout: LayoutId; onChange: (id: LayoutId) => void }) {
  return (
    <div className="mb-4 border-b border-base-content/10 pb-3">
      <label className="mb-1.5 block text-xs font-medium opacity-70">Layout</label>
      <div className="flex flex-col gap-1.5">
        {(["sidebar", "chain"] as LayoutId[]).map((id) => (
          <button
            key={id}
            type="button"
            className={`dw-theme-row ${layout === id ? "active" : ""}`}
            onClick={() => onChange(id)}
          >
            <span className="flex flex-col items-start gap-0.5">
              <span>{LAYOUT_LABEL[id]}</span>
              <span className="text-[10px] font-normal opacity-60">{LAYOUT_DESCRIPTION[id]}</span>
            </span>
            {layout === id && <span>✓</span>}
          </button>
        ))}
      </div>
    </div>
  );
}

const ENGINE_DISPLAY_TITLES: Record<WaveformEngineId, string> = {
  precision: "Studio Bars",
  silk: "Harmonic Silk",
  matrix: "LED Matrix",
  hyperdom: "Kinetic Needles",
};

const PRESET_ORDER: { id: NeedlePresetId; caption: string }[] = [
  { id: "clean", caption: "Clean 32" },
  { id: "balanced", caption: "Balanced 56 · Default" },
  { id: "dense", caption: "Dense 88" },
  { id: "ultra", caption: "Ultra 136" },
];

function EngineGlyph({ engine, accent }: { engine: WaveformEngineId; accent: string }) {
  if (engine === "precision") {
    return (
      <svg viewBox="0 0 36 18" className="h-4 w-8 shrink-0" aria-hidden="true">
        {[
          { x: 2, h: 6 },
          { x: 8, h: 12 },
          { x: 14, h: 16 },
          { x: 20, h: 10 },
          { x: 26, h: 14 },
          { x: 32, h: 7 },
        ].map((b) => (
          <rect
            key={b.x}
            x={b.x}
            y={(18 - b.h) / 2}
            width="3"
            height={b.h}
            rx="1.5"
            fill={accent}
            opacity={b.h > 10 ? 0.95 : 0.6}
          />
        ))}
      </svg>
    );
  }
  if (engine === "silk") {
    return (
      <svg viewBox="0 0 36 18" className="h-4 w-8 shrink-0" fill="none" aria-hidden="true">
        <path
          d="M1 9 C7 2, 13 16, 18 9 C23 2, 29 16, 35 9"
          stroke={accent}
          strokeWidth="1.8"
          strokeLinecap="round"
        />
        <path
          d="M1 9 C8 14, 14 3, 18 9 C22 15, 28 4, 35 9"
          stroke={accent}
          strokeOpacity="0.45"
          strokeWidth="1.3"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  if (engine === "matrix") {
    const heights = [1, 2, 3, 2, 3, 1];
    return (
      <svg viewBox="0 0 36 18" className="h-4 w-8 shrink-0" aria-hidden="true">
        {heights.map((lit, col) =>
          [0, 1, 2].map((row) => (
            <rect
              key={`${col}-${row}`}
              x={2 + col * 5.5}
              y={13 - row * 5}
              width="3.5"
              height="3.5"
              rx="1"
              fill={row < lit ? accent : "rgba(255, 255, 255, 0.85)"}
              opacity={row < lit ? 0.92 : 0.16}
            />
          )),
        )}
      </svg>
    );
  }
  return (
    <svg viewBox="0 0 36 18" className="h-4 w-8 shrink-0" aria-hidden="true">
      {[3, 6, 11, 15, 17, 13, 16, 11, 6, 3].map((h, i) => (
        <rect
          key={i}
          x={2 + i * 3.4}
          y={(18 - h) / 2}
          width="1.6"
          height={h}
          rx="0.8"
          fill={accent}
          opacity={i >= 3 && i <= 6 ? 1 : 0.65}
        />
      ))}
    </svg>
  );
}

export function WaveformSection({
  config,
  onChange,
}: {
  config: WaveformConfig;
  onChange: (next: WaveformConfig) => void;
}) {
  const colorway = getWaveformColorway(config.colorway);
  const activeEngineMeta =
    WAVEFORM_ENGINE_META.find((m) => m.id === config.engine) ?? WAVEFORM_ENGINE_META[0];
  const activeEngineLabel = `${activeEngineMeta.num} · ${ENGINE_DISPLAY_TITLES[activeEngineMeta.id]}`;

  function selectEngine(engine: WaveformEngineId) {
    onChange({ ...config, engine });
  }

  function selectColorway(id: WaveformColorwayId) {
    onChange({ ...config, colorway: id });
  }

  function applyPreset(presetId: NeedlePresetId) {
    const p = NEEDLE_PRESETS[presetId];
    onChange({
      ...config,
      engine: "hyperdom",
      needle_bars: p.needle_bars,
      sensitivity: p.sensitivity,
      snap_ms: p.snap_ms,
      needle_width: p.needle_width,
    });
  }

  function setHyperMode(hyper_mode: HyperLayoutMode) {
    onChange({ ...config, engine: "hyperdom", hyper_mode });
  }

  function resetTuning() {
    onChange({
      ...config,
      needle_bars: DEFAULT_WAVEFORM_CONFIG.needle_bars,
      sensitivity: DEFAULT_WAVEFORM_CONFIG.sensitivity,
      snap_ms: DEFAULT_WAVEFORM_CONFIG.snap_ms,
      needle_width: DEFAULT_WAVEFORM_CONFIG.needle_width,
      hyper_mode: DEFAULT_WAVEFORM_CONFIG.hyper_mode,
    });
  }

  function renderColorwayPill(id: WaveformColorwayId) {
    const cw = WAVEFORM_COLORWAYS[id];
    const active = config.colorway === id;
    const ringColor = id === "solid-white" ? "var(--dw-accent)" : cw.mid;
    return (
      <button
        key={id}
        type="button"
        className={`dw-colorway-pill ${active ? "active" : ""}`}
        style={{ "--cw-ring": ringColor } as CSSProperties}
        onClick={() => selectColorway(id)}
        aria-pressed={active}
        data-testid={`waveform-colorway-${id}`}
      >
        <span className="dw-colorway-orb" style={{ background: cw.swatchBg }} aria-hidden="true" />
        <span>{cw.name}</span>
      </button>
    );
  }

  return (
    <div className="mb-4 flex flex-col gap-3.5" data-testid="waveform-studio">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <label className="dw-display block text-xs font-semibold opacity-85">
            Waveform Studio
          </label>
          <p className="text-[11px] opacity-60">
            60fps acoustic visualizer used across the floating dictation HUD and meeting bar.
          </p>
        </div>
        <label
          className="flex cursor-pointer items-center gap-1.5 rounded-lg border border-base-content/10 bg-base-100 px-2.5 py-1.5 text-[11px] opacity-85 hover:opacity-100"
          title="Show or hide the quick waveform switcher inside the floating widget's cursor hover popover"
        >
          <input
            type="checkbox"
            className="checkbox checkbox-xs"
            checked={config.show_in_flyout}
            onChange={(e) => onChange({ ...config, show_in_flyout: e.target.checked })}
            data-testid="waveform-show-in-flyout"
          />
          <span>Show in cursor popover</span>
        </label>
      </div>

      {/* Live Studio Preview Stage */}
      <div
        className="rounded-xl border border-white/10 bg-[#05070b] p-3 text-white shadow-inner"
        style={{
          boxShadow: "inset 0 2px 16px rgba(0, 0, 0, 0.65), 0 1px 0 rgba(255, 255, 255, 0.04)",
        }}
      >
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-[10.5px]">
          <span className="flex items-center gap-2 font-medium text-white/85">
            <span
              className="inline-block h-2 w-2 rounded-full"
              style={{
                background: colorway.bright,
                boxShadow: config.phosphor_glow ? `0 0 8px ${colorway.peakGlow}` : undefined,
              }}
              aria-hidden="true"
            />
            <span>{activeEngineLabel}</span>
            <span className="text-white/35">·</span>
            <span className="text-white/65">{colorway.name}</span>
          </span>
          <span className="dw-mono rounded-full border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] text-white/70">
            {config.engine === "hyperdom"
              ? `${config.needle_bars} bars · ${config.sensitivity.toFixed(2)}× · ${config.snap_ms}ms`
              : `${config.sensitivity.toFixed(2)}× gain · ${config.phosphor_glow ? "Glow" : "Crisp"}`}
          </span>
        </div>
        <WaveformMeter
          levels={[]}
          config={config}
          demoMode
          className="h-[88px] w-full"
          testId="settings-waveform-preview"
        />
      </div>

      {/* 4 Waveform Engine Cards */}
      <div>
        <span className="mb-1.5 block text-[11px] font-medium opacity-70">Visualizer Engine</span>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {WAVEFORM_ENGINE_META.map((eng) => {
            const active = config.engine === eng.id;
            const title = `${eng.num} · ${ENGINE_DISPLAY_TITLES[eng.id]}`;
            return (
              <button
                key={eng.id}
                type="button"
                className={`dw-engine-card ${active ? "active" : ""}`}
                onClick={() => selectEngine(eng.id)}
                aria-pressed={active}
                data-testid={`waveform-engine-${eng.id}`}
              >
                <div className="flex w-full items-center justify-between gap-2">
                  <div>
                    <div className="dw-engine-card-title">{title}</div>
                    <div className="dw-engine-card-meta mt-0.5 uppercase tracking-wider">
                      {eng.badge}
                    </div>
                  </div>
                  <span
                    className="inline-flex shrink-0 items-center justify-center rounded-md px-1.5 py-1"
                    style={{
                      background: "var(--dw-bg-well)",
                      border: "1px solid var(--dw-border)",
                    }}
                  >
                    <EngineGlyph engine={eng.id} accent={active ? colorway.bright : colorway.mid} />
                  </span>
                </div>
                <p className="text-[11px] leading-snug opacity-70">{eng.desc}</p>
              </button>
            );
          })}
        </div>
      </div>

      {/* Colorway Picker: Spectrum + Solid + Luminous Glow */}
      <div className="rounded-xl border border-base-content/10 bg-base-100 p-3">
        <div className="mb-2.5 flex flex-wrap items-center justify-between gap-2">
          <span className="text-xs font-medium opacity-75">Colorway</span>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs opacity-80">
            <input
              type="checkbox"
              className="checkbox checkbox-xs"
              checked={config.phosphor_glow}
              onChange={(e) => onChange({ ...config, phosphor_glow: e.target.checked })}
            />
            <span>Luminous Glow</span>
          </label>
        </div>

        <div className="flex flex-col gap-2.5">
          <div>
            <span className="dw-mono mb-1.5 block text-[10px] uppercase tracking-wider opacity-50">
              Spectrum
            </span>
            <div className="flex flex-wrap gap-1.5">
              {SPECTRUM_COLORWAY_ORDER.map(renderColorwayPill)}
            </div>
          </div>

          <div>
            <span className="dw-mono mb-1.5 block text-[10px] uppercase tracking-wider opacity-50">
              Solid
            </span>
            <div className="flex flex-wrap gap-1.5">
              {SOLID_COLORWAY_ORDER.map(renderColorwayPill)}
            </div>
          </div>
        </div>
      </div>

      {/* Kinetic Needles & Sensitivity Tuning Panel */}
      <div className="rounded-xl border border-base-content/10 bg-base-100 p-3.5">
        <div className="mb-2.5 flex flex-wrap items-center justify-between gap-2">
          <div>
            <span className="block text-xs font-semibold opacity-85">
              Kinetic Needles &amp; Sensitivity
            </span>
            <span className="text-[10.5px] opacity-55">
              Sensitivity applies to all engines; needle sliders activate Kinetic Needles.
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <div className="inline-flex rounded-lg border border-base-content/15 p-0.5 text-[11px]">
              {(
                [
                  ["center", "Center Symmetry"],
                  ["scroll", "Scrolling Tape"],
                ] as [HyperLayoutMode, string][]
              ).map(([mode, label]) => {
                const active = config.hyper_mode === mode;
                return (
                  <button
                    key={mode}
                    type="button"
                    className={`rounded-md px-2 py-0.5 font-medium transition-colors ${
                      active
                        ? "bg-base-content/15 text-base-content"
                        : "opacity-60 hover:opacity-90"
                    }`}
                    onClick={() => setHyperMode(mode)}
                    aria-pressed={active}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
            <button
              type="button"
              className="btn btn-ghost btn-xs text-[11px]"
              onClick={resetTuning}
            >
              Reset
            </button>
          </div>
        </div>

        {/* Density Presets */}
        <div className="mb-3.5 flex flex-wrap gap-1.5">
          {PRESET_ORDER.map(({ id, caption }) => {
            const p = NEEDLE_PRESETS[id];
            const isMatch =
              config.engine === "hyperdom" &&
              config.needle_bars === p.needle_bars &&
              config.snap_ms === p.snap_ms &&
              Math.abs(config.needle_width - p.needle_width) < 0.01;
            return (
              <button
                key={id}
                type="button"
                className={`dw-mono rounded-lg border px-2.5 py-1 text-[11px] font-medium transition-colors ${
                  isMatch
                    ? "border-[var(--dw-accent)] bg-[var(--dw-nav-active-bg)] text-[var(--dw-fg)]"
                    : "border-base-content/15 opacity-70 hover:opacity-100"
                }`}
                onClick={() => applyPreset(id)}
                aria-pressed={isMatch}
                data-testid={`waveform-preset-${id}`}
              >
                {caption}
              </button>
            );
          })}
        </div>

        {/* 4 Studio Range Sliders */}
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <label className="flex flex-col gap-1.5">
            <span className="flex items-center justify-between text-[11.5px]">
              <span className="font-medium opacity-80">Bar Count</span>
              <span className="dw-mono text-[11px] opacity-75">{config.needle_bars} bars</span>
            </span>
            <input
              type="range"
              className="dw-studio-range"
              min={16}
              max={140}
              step={2}
              value={config.needle_bars}
              onChange={(e) =>
                onChange({
                  ...config,
                  engine: "hyperdom",
                  needle_bars: Number(e.target.value),
                })
              }
            />
          </label>

          <label className="flex flex-col gap-1.5">
            <span className="flex items-center justify-between text-[11.5px]">
              <span className="font-medium opacity-80">Sensitivity</span>
              <span className="dw-mono text-[11px] opacity-75">
                {config.sensitivity.toFixed(2)}×
              </span>
            </span>
            <input
              type="range"
              className="dw-studio-range"
              min={0.5}
              max={2.5}
              step={0.05}
              value={config.sensitivity}
              onChange={(e) =>
                onChange({
                  ...config,
                  sensitivity: Number(e.target.value),
                })
              }
            />
          </label>

          <label className="flex flex-col gap-1.5">
            <span className="flex items-center justify-between text-[11.5px]">
              <span className="font-medium opacity-80">Snap Speed</span>
              <span className="dw-mono text-[11px] opacity-75">{config.snap_ms}ms</span>
            </span>
            <input
              type="range"
              className="dw-studio-range"
              min={12}
              max={110}
              step={2}
              value={config.snap_ms}
              onChange={(e) =>
                onChange({
                  ...config,
                  engine: "hyperdom",
                  snap_ms: Number(e.target.value),
                })
              }
            />
          </label>

          <label className="flex flex-col gap-1.5">
            <span className="flex items-center justify-between text-[11.5px]">
              <span className="font-medium opacity-80">Needle Width</span>
              <span className="dw-mono text-[11px] opacity-75">
                {config.needle_width.toFixed(2)}px
              </span>
            </span>
            <input
              type="range"
              className="dw-studio-range"
              min={1.0}
              max={4.0}
              step={0.25}
              value={config.needle_width}
              onChange={(e) =>
                onChange({
                  ...config,
                  engine: "hyperdom",
                  needle_width: Number(e.target.value),
                })
              }
            />
          </label>
        </div>
      </div>

      {/* Collapsible Architecture Disclosure */}
      <details className="rounded-lg border border-base-content/10 bg-base-100/60 px-3 py-2 text-xs">
        <summary className="cursor-pointer select-none font-medium opacity-75 hover:opacity-100">
          Under the Hood: 60fps Ring Buffer
        </summary>
        <p className="mt-2 leading-relaxed opacity-65">
          Audio levels arrive from Rust at 20Hz over Tauri IPC. Instead of triggering React DOM
          re-renders on every packet, <code className="dw-mono">WaveformMeter</code> writes samples
          into pre-allocated typed-array ring buffers and runs a decoupled{" "}
          <code className="dw-mono">requestAnimationFrame</code> loop with dt-normalized asymmetric
          attack (22ms) and decay (95ms) ballistics — keeping Studio Bars, Harmonic Silk, LED
          Matrix, and ref-driven Kinetic Needles butter-smooth with zero layout jitter.
        </p>
      </details>
    </div>
  );
}

export const ABOUT_ENTRIES: { title: string; body: string }[] = [
  {
    title: "How dictation works",
    body: "Hold the push-to-talk shortcut (or click-and-hold the widget) and speak. When you release, Mouthpiece transcribes locally with Whisper, applies your active Mode's formatting, and pastes the result into whatever's focused — no cloud round-trip unless you've turned on LLM refinement or a webhook.",
  },
  {
    title: "Voice Isolation",
    body: "Filters recordings down to your voice before transcription, so background noise or other people talking don't end up in the transcript. Works out of the box with an energy-based fallback; enroll your voice in Voice for stronger speaker-based isolation.",
  },
  {
    title: "Vocabulary",
    body: "Terms you add bias Whisper's recognition toward your jargon — tool names, library names, anything Whisper would otherwise mishear. Vocabulary books let you adopt a curated bundle (Frontend, Backend, Full-Stack, Product) in one click, then prune anything you don't need.",
  },
  {
    title: "Snippets",
    body: "Spoken trigger phrases that expand into saved blocks of text — say a snippet's trigger and it's pasted in full, instead of dictating the same boilerplate every time.",
  },
  {
    title: "Modes",
    body: 'Plain, Casual, or CLI formatting, applied automatically based on which app is frontmost, or overridden for a single dictation from the widget\'s hover flyout. CLI mode turns "git commit update readme" into git commit -m "update readme", for example.',
  },
  {
    title: "LLM refinement",
    body: "An optional local pass through Ollama that cleans up a transcript further — only runs for modes with it explicitly enabled, since it's an extra background call.",
  },
  {
    title: "History",
    body: "Every delivered transcript is logged, searchable, and auto-purged after your configured retention window. Turn on the work journal to get a one-line LLM-generated summary attached to each entry.",
  },
  {
    title: "Integrations",
    body: "Copy-only mode skips the simulated paste keystroke entirely (and the Accessibility permission it needs). The output webhook fires a POST with the delivered transcript to any URL that accepts incoming webhooks — Notion, Slack, n8n, Zapier, Make.com, or your own endpoint.",
  },
];

export function AboutSection() {
  return (
    <div>
      {ABOUT_ENTRIES.map((entry) => (
        <div key={entry.title} className="dw-about-entry">
          <h3>{entry.title}</h3>
          <p>{entry.body}</p>
        </div>
      ))}
    </div>
  );
}

export type UpdateCheckState = "checking" | "idle" | "available" | "downloading" | "error";

export function UpdatesSection() {
  const [version, setVersion] = useState("");
  const [state, setState] = useState<UpdateCheckState>("checking");
  const [update, setUpdate] = useState<Update | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  function runCheck() {
    setState("checking");
    setError(null);
    checkForUpdate()
      .then((found) => {
        setUpdate(found);
        setState(found ? "available" : "idle");
      })
      .catch((err) => {
        notifyError("checkForUpdate", err);
        setError(String(err));
        setState("error");
      });
  }

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch((err) => notifyError("getVersion", err));
    runCheck();
  }, []);

  function installUpdate() {
    if (!update) return;
    setState("downloading");
    setPercent(0);
    // contentLength isn't guaranteed by every server response, so a null
    // percent (rather than a wrong one) is the honest fallback — the
    // "Downloading…" label alone still shows real progress is happening.
    let total = 0;
    let downloaded = 0;
    update
      .downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setPercent(total > 0 ? Math.round((downloaded / total) * 100) : null);
        }
      })
      .then(() => relaunch())
      .catch((err) => {
        notifyError("downloadAndInstall", err);
        setError(String(err));
        setState("error");
      });
  }

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Updates</h3>
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="opacity-70">Version {version || "…"}</span>
        {state === "checking" && <span className="loading loading-spinner loading-xs" />}
        {state === "idle" && (
          <button className="btn btn-ghost btn-xs" onClick={runCheck}>
            Check for updates
          </button>
        )}
        {state === "available" && update && (
          <button className="btn btn-xs" onClick={installUpdate}>
            Update to v{update.version}
          </button>
        )}
        {state === "downloading" && (
          <span className="opacity-70">{percent !== null ? `Downloading… ${percent}%` : "Downloading…"}</span>
        )}
        {state === "error" && (
          <button className="btn btn-ghost btn-xs" onClick={runCheck}>
            Retry
          </button>
        )}
      </div>
      {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
      <p className="mt-1.5 text-xs opacity-50">
        Checks GitHub on launch and whenever this page opens — no data sent, just "is there a newer release."
      </p>
    </div>
  );
}
