import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { BEHAVIORS, BEHAVIOR_LABEL, type Behavior, type LlmModelStatus, type ModelStatus } from "../shared";

export const GLOBAL_MODEL = "__global__";

export type AppRef = { bundle_id: string; app_name: string };

// Mirrors the Rust `LlmRefinement` enum's serde shape: unit variants
// serialize as bare strings, the tuple variant as `{ model: "<id>" }`.
export type LlmRefinement = "off" | "global" | { model: string };

export type ModeDefinition = {
  name: string;
  behavior: Behavior;
  apps: AppRef[];
  stt_model: string | null;
  llm_refinement: LlmRefinement;
  custom_instructions: string | null;
  /// Reorganise 80+ word dictations into Goal / Context / Constraints / Done when.
  structure_long_prompts?: boolean;
  is_default: boolean;
};

export type FrontmostApp = { bundle_id: string; name: string; icon_data_uri: string | null };

export type RunningApp = { bundle_id: string; name: string; icon_data_uri: string | null; is_running: boolean };

type ModePreviewResult = {
  output: string;
  elapsed_ms: number;
  used_llm: boolean;
  model: string | null;
  structured: boolean;
};

type SamplePresetKey = "short" | "messy" | "long";

const SAMPLE_PRESET_CHIPS: { key: SamplePresetKey; label: string }[] = [
  { key: "short", label: "Short" },
  { key: "messy", label: "Messy" },
  { key: "long", label: "Long 80+w" },
];

const MODE_SAMPLES: Record<Behavior, Record<SamplePresetKey, string>> = {
  cli: {
    short: "uh list all files in source sorted by size and check git status",
    messy: "um run cargo test no scratch that run cargo check dash dash all targets pipe head minus twenty",
    long: "so basically I want to audit the build pipeline in our Rust Tauri workspace because incremental compiles have gotten slow on macOS. First check cargo metadata and list the largest workspace dependencies sorted by compile time, then inspect our Cargo toml feature flags for whisper and core audio so we do not compile unused backends during local development. Keep all release binary optimizations untouched and do not change any public command signatures. We are done when a clean debug rebuild finishes under forty seconds and cargo test passes with zero warnings.",
  },
  casual: {
    short: "hey um can we push our sync to thursday afternoon around three thanks",
    messy: "hey um can we move standup to tuesday no scratch that wednesday afternoon around three thirty thanks",
    long: "hey team um quick heads up before tomorrow's release sync. We just finished testing the new audio device switcher and live mic check meter across built-in MacBook mics, USB mics, and AirPods fallback, and everything is looking super solid so far. One thing to keep in mind is we should hold off on flipping the default waveform engine until QA finishes checking light mode themes and low-power laptops. Could everyone drop any remaining edge cases in the release thread before three pm so we can lock the build tonight and ship cleanly tomorrow morning?",
  },
  plain: {
    short: "so basically we should refactor the audio buffer to use a lock free ring buffer and test it on mac os",
    messy: "so um basically we need to rename snake case audio buffer to ring buffer in recording dot rs comma and verify zero allocations period",
    long: "so basically we should refactor the audio capture ring buffer so that the microphone check meter and live transcription share a single lock free reader instead of spawning a duplicate audio stream on macOS. Right now switching input devices while monitoring can briefly drop frames on Bluetooth headsets. Make sure we keep all existing Tauri event payloads compatible, avoid allocations on the real-time audio callback thread, and preserve automatic fallback to the system default microphone. We are done when the live meter stays smooth at sixty frames per second and all unit and end-to-end tests pass.",
  },
};

function countWords(text: string): number {
  const trimmed = text.trim();
  return trimmed ? trimmed.split(/\s+/).length : 0;
}

function pipelineSummary(mode: ModeDefinition): string {
  const behavior = BEHAVIOR_LABEL[mode.behavior];
  const llm =
    mode.llm_refinement === "off"
      ? "Rules only"
      : mode.llm_refinement === "global"
        ? "Global LLM"
        : mode.llm_refinement.model;
  return `${behavior} · ${llm}${mode.structure_long_prompts ? " · 80+w structuring" : ""}`;
}

function ModePlayground({ mode, index }: { mode: ModeDefinition; index: number }) {
  const [activePreset, setActivePreset] = useState<SamplePresetKey | null>("short");
  const [dryText, setDryText] = useState<string>(() => MODE_SAMPLES[mode.behavior].short);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<ModePreviewResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const prevBehaviorRef = useRef<Behavior>(mode.behavior);
  const copyTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (prevBehaviorRef.current !== mode.behavior) {
      prevBehaviorRef.current = mode.behavior;
      if (activePreset !== null) {
        setDryText(MODE_SAMPLES[mode.behavior][activePreset]);
      }
    }
  }, [mode.behavior, activePreset]);

  const reqIdRef = useRef(0);

  useEffect(() => {
    return () => {
      reqIdRef.current += 1;
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  function applyPreset(key: SamplePresetKey) {
    setActivePreset(key);
    setDryText(MODE_SAMPLES[mode.behavior][key]);
    setError(null);
  }

  function runPreview() {
    if (!dryText.trim()) return;
    const reqId = ++reqIdRef.current;
    setRunning(true);
    setError(null);
    invoke<ModePreviewResult>("preview_mode_transform", {
      text: dryText,
      behavior: mode.behavior,
      llmRefinement: mode.llm_refinement,
      customInstructions: mode.custom_instructions,
      structureLongPrompts: mode.structure_long_prompts ?? false,
    })
      .then((res) => {
        if (reqId !== reqIdRef.current) return;
        setResult(res);
      })
      .catch((err) => {
        if (reqId !== reqIdRef.current) return;
        setResult(null);
        setError(String(err));
      })
      .finally(() => {
        if (reqId !== reqIdRef.current) return;
        setRunning(false);
      });
  }

  function copyOutput() {
    if (!result?.output) return;
    navigator.clipboard?.writeText(result.output).then(
      () => {
        setCopied(true);
        if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
        copyTimerRef.current = window.setTimeout(() => setCopied(false), 1500);
      },
      () => {},
    );
  }

  const words = countWords(dryText);
  const willStructure = Boolean(mode.structure_long_prompts) && words >= 80;
  const telemetryLabel = result
    ? `${result.elapsed_ms}ms · ${result.used_llm ? (result.model ?? "LLM") : "Rules only"}`
    : null;

  return (
    <div
      id={`mode-playground-${index}`}
      data-testid={`mode-playground-${index}`}
      className="mt-3 rounded-xl p-3"
      style={{
        background: "var(--dw-bg-sunk)",
        border: "1px solid var(--dw-border)",
      }}
    >
      <div className="mb-2.5 flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-1.5">
          <span
            className="mr-1 text-[10px] font-semibold uppercase tracking-wider"
            style={{ color: "var(--dw-muted)" }}
          >
            Samples
          </span>
          {SAMPLE_PRESET_CHIPS.map((chip) => {
            const selected = activePreset === chip.key;
            return (
              <button
                key={chip.key}
                type="button"
                onClick={() => applyPreset(chip.key)}
                className="rounded-full px-2.5 py-0.5 text-[11px] font-medium transition-all duration-150 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[var(--dw-accent)]"
                style={{
                  background: selected
                    ? "color-mix(in srgb, var(--dw-accent) 16%, var(--dw-card))"
                    : "var(--dw-card)",
                  border: `1px solid ${
                    selected ? "color-mix(in srgb, var(--dw-accent) 60%, var(--dw-border))" : "var(--dw-border)"
                  }`,
                  color: selected ? "var(--dw-accent)" : "var(--dw-muted)",
                }}
              >
                {chip.label}
              </button>
            );
          })}
        </div>

        <button
          type="button"
          onClick={runPreview}
          disabled={running || dryText.trim().length === 0}
          className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-[11px] font-semibold transition-all duration-150 disabled:cursor-not-allowed disabled:opacity-45"
          style={{
            background: "var(--dw-accent)",
            color: "var(--dw-accent-fg)",
          }}
          data-testid={`mode-playground-run-${index}`}
        >
          {running && <span className="loading loading-spinner loading-xs" />}
          <span>{running ? "Running…" : "Run preview"}</span>
        </button>
      </div>

      <div className="grid grid-cols-1 gap-2.5 md:grid-cols-2">
        <div
          className="flex flex-col rounded-lg p-2.5"
          style={{
            background: "var(--dw-card)",
            border: "1px solid var(--dw-border)",
          }}
        >
          <div className="mb-1.5 flex items-center justify-between gap-2">
            <span
              className="text-[10px] font-bold uppercase tracking-wider"
              style={{ color: "var(--dw-muted)" }}
            >
              Dry (Spoken)
            </span>
            <span
              className="inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px] tabular-nums"
              style={{
                fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
                background: willStructure
                  ? "color-mix(in srgb, var(--dw-accent) 15%, transparent)"
                  : "var(--dw-bg-sunk)",
                color: willStructure ? "var(--dw-accent)" : "var(--dw-muted)",
              }}
            >
              {words}w{willStructure ? " · 80+w" : ""}
            </span>
          </div>
          <textarea
            className="textarea textarea-xs min-h-[88px] w-full flex-1 resize-y font-sans text-xs leading-relaxed"
            style={{
              background: "var(--dw-bg-sunk)",
              borderColor: "var(--dw-border)",
            }}
            rows={4}
            aria-label={`Dry spoken input for ${mode.name}`}
            value={dryText}
            onChange={(e) => {
              setDryText(e.target.value);
              setActivePreset(null);
            }}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                runPreview();
              }
            }}
            placeholder="Type or paste spoken words to test this mode…"
          />
        </div>

        <div
          className="flex flex-col rounded-lg p-2.5"
          style={{
            background: "var(--dw-card)",
            border: `1px solid ${
              error
                ? "color-mix(in srgb, #ef4444 50%, var(--dw-border))"
                : result
                  ? result.used_llm
                    ? "color-mix(in srgb, var(--dw-refine) 50%, var(--dw-border))"
                    : "color-mix(in srgb, var(--dw-accent) 45%, var(--dw-border))"
                  : "var(--dw-border)"
            }`,
          }}
        >
          <div className="mb-1.5 flex flex-wrap items-center justify-between gap-1.5">
            <span
              className="text-[10px] font-bold uppercase tracking-wider"
              style={{ color: result?.used_llm ? "var(--dw-refine)" : "var(--dw-accent)" }}
            >
              Wet (Mode Output)
            </span>
            <div className="flex flex-wrap items-center gap-1.5">
              {telemetryLabel && (
                <span
                  className="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium tabular-nums"
                  style={{
                    fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
                    background: "var(--dw-bg-sunk)",
                    border: "1px solid var(--dw-border)",
                    color: "var(--dw-fg)",
                  }}
                  data-testid={`mode-playground-telemetry-${index}`}
                >
                  {telemetryLabel}
                </span>
              )}
              {result?.structured && (
                <span
                  className="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-semibold"
                  style={{
                    background: "color-mix(in srgb, var(--dw-accent) 16%, transparent)",
                    border: "1px solid color-mix(in srgb, var(--dw-accent) 45%, transparent)",
                    color: "var(--dw-accent)",
                  }}
                  data-testid={`mode-playground-structured-${index}`}
                >
                  Structured
                </span>
              )}
              {result?.output && (
                <button
                  type="button"
                  onClick={copyOutput}
                  className="rounded-md px-2 py-0.5 text-[10.5px] font-medium transition-colors"
                  style={{
                    background: "var(--dw-bg-sunk)",
                    border: "1px solid var(--dw-border-strong)",
                    color: copied ? "var(--color-success)" : "var(--dw-fg)",
                  }}
                >
                  {copied ? "Copied ✓" : "Copy output"}
                </button>
              )}
            </div>
          </div>

          <div
            className="flex min-h-[88px] flex-1 flex-col justify-between rounded-md p-2 text-xs leading-relaxed"
            style={{
              background: "var(--dw-bg-sunk)",
              border: "1px solid color-mix(in srgb, var(--dw-border) 75%, transparent)",
            }}
            data-testid={`mode-playground-output-${index}`}
          >
            {error ? (
              <div
                role="alert"
                className="flex flex-col gap-1.5 rounded-md p-2 text-[11.5px]"
                style={{
                  background: "color-mix(in srgb, var(--color-error) 10%, transparent)",
                  border: "1px solid color-mix(in srgb, var(--color-error) 35%, transparent)",
                  color: "var(--color-error)",
                }}
              >
                <span className="font-semibold">Preview failed</span>
                <span className="break-words opacity-90">{error}</span>
                {mode.llm_refinement !== "off" && (
                  <span className="text-[11px]" style={{ color: "var(--dw-muted)" }}>
                    Check that your local LLM (Ollama / LM Studio) is running, or switch to No LLM refinement to preview rule-based cleanup.
                  </span>
                )}
              </div>
            ) : running ? (
              <div className="flex flex-1 items-center gap-2 text-xs" style={{ color: "var(--dw-muted)" }}>
                <span className="loading loading-spinner loading-xs" />
                <span>Running {pipelineSummary(mode)}…</span>
              </div>
            ) : result ? (
              <div
                className={`whitespace-pre-wrap break-words ${mode.behavior === "cli" ? "font-mono text-[11.5px]" : ""}`}
                style={{ color: "var(--dw-fg)" }}
              >
                {result.output || <span className="opacity-50">No output produced.</span>}
              </div>
            ) : (
              <div className="flex flex-1 flex-col justify-center gap-1 text-[11.5px]" style={{ color: "var(--dw-muted)" }}>
                <span>
                  Click <strong>Run preview</strong> (or press <kbd className="dw-kbd">⌘↵</kbd>) to see how this mode transforms spoken text.
                </span>
                <span className="text-[10.5px] opacity-75">{pipelineSummary(mode)}</span>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export function llmSelectValue(r: LlmRefinement): string {
  return typeof r === "string" ? r : r.model;
}

export function llmFromSelectValue(v: string): LlmRefinement {
  return v === "off" || v === "global" ? v : { model: v };
}

export function AppIcon({ src, name }: { src: string | null; name: string }) {
  if (src) {
    return <img src={src} alt="" className="h-4 w-4 shrink-0 rounded-sm" />;
  }
  // Fallback for apps whose icon couldn't be resolved: first letter avatar.
  return (
    <span className="flex h-4 w-4 shrink-0 items-center justify-center rounded-sm bg-base-300 text-[9px] font-semibold opacity-60">
      {name.charAt(0).toUpperCase()}
    </span>
  );
}

export function InfoPopover({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="relative inline-block">
      <button
        type="button"
        className="btn btn-ghost btn-circle btn-xs opacity-60 hover:opacity-100"
        aria-label="More info"
        onClick={() => setOpen((v) => !v)}
        onBlur={() => setOpen(false)}
      >
        ⓘ
      </button>
      {open && (
        <div className="absolute right-0 z-20 mt-1 w-64 rounded-md border border-base-content/10 bg-base-100 p-2.5 text-[11px] leading-snug opacity-100 shadow-lg">
          {children}
        </div>
      )}
    </span>
  );
}

export function AppModesSection() {
  const [modes, setModes] = useState<ModeDefinition[]>([]);
  const [modeIds, setModeIds] = useState<number[]>([]);
  const nextModeIdRef = useRef(1);
  const [loading, setLoading] = useState(true);
  const [lastApp, setLastApp] = useState<FrontmostApp | null>(null);
  const [runningApps, setRunningApps] = useState<RunningApp[]>([]);
  const [availableModels, setAvailableModels] = useState<ModelStatus[]>([]);
  const [llmCatalog, setLlmCatalog] = useState<LlmModelStatus[]>([]);
  const [pickerOpenFor, setPickerOpenFor] = useState<number | null>(null);
  const [openPlaygrounds, setOpenPlaygrounds] = useState<Record<number, boolean>>({});
  const [llmProgress, setLlmProgress] = useState<Record<string, { status: string; percent: number | null }>>({});
  const [llmErrors, setLlmErrors] = useState<Record<string, string>>({});

  function refresh() {
    invoke<ModeDefinition[]>("get_modes")
      .then((list) => {
        setModes(list);
        setModeIds((prev) => {
          if (prev.length === list.length) return prev;
          return list.map((_, i) => prev[i] ?? nextModeIdRef.current++);
        });
      })
      .catch((err) => notifyError("get_modes", err))
      .finally(() => setLoading(false));
  }

  function loadRunningApps() {
    invoke<RunningApp[]>("list_running_apps")
      .then(setRunningApps)
      .catch((err) => notifyError("list_running_apps", err));
  }

  function loadLlmCatalog() {
    invoke<LlmModelStatus[]>("list_llm_catalog")
      .then(setLlmCatalog)
      .catch((err) => notifyError("list_llm_catalog", err));
  }

  function pullLlmModel(id: string) {
    setLlmErrors((er) => {
      const next = { ...er };
      delete next[id];
      return next;
    });
    setLlmProgress((p) => ({ ...p, [id]: { status: "starting…", percent: null } }));
    invoke("pull_llm_model", { id }).catch((err) => {
      setLlmProgress((p) => {
        const next = { ...p };
        delete next[id];
        return next;
      });
      setLlmErrors((er) => ({ ...er, [id]: String(err) }));
    });
  }

  useEffect(() => {
    refresh();
    loadRunningApps();
    loadLlmCatalog();
    invoke<FrontmostApp | null>("get_last_frontmost_app")
      .then(setLastApp)
      .catch((err) => notifyError("get_last_frontmost_app", err));
    invoke<ModelStatus[]>("list_models")
      .then(setAvailableModels)
      .catch((err) => notifyError("list_models", err));

    const unlistenProgress = listen<{ id: string; status: string; percent: number | null }>(
      "llm-pull-progress",
      (e) => {
        setLlmProgress((p) => ({ ...p, [e.payload.id]: { status: e.payload.status, percent: e.payload.percent } }));
      },
    );
    const unlistenDone = listen<string>("llm-pull-complete", (e) => {
      setLlmProgress((p) => {
        const next = { ...p };
        delete next[e.payload];
        return next;
      });
      loadLlmCatalog();
    });
    const unlistenError = listen<{ id: string; error: string }>("llm-pull-error", (e) => {
      setLlmProgress((p) => {
        const next = { ...p };
        delete next[e.payload.id];
        return next;
      });
      setLlmErrors((er) => ({ ...er, [e.payload.id]: e.payload.error }));
    });

    return () => {
      unlistenProgress.then((f) => f());
      unlistenDone.then((f) => f());
      unlistenError.then((f) => f());
    };
  }, []);

  // Full-list replace, mirroring `SnippetsSection` — the whole list lives
  // client-side (renaming a mode, adding/removing an app, adding/deleting
  // a mode are all just array edits) and gets saved back in one shot
  // rather than the backend exposing per-field mutation commands.
  function save(next: ModeDefinition[]) {
    setModes(next);
    invoke("set_modes", { modes: next }).catch((err) => notifyError("set_modes", err));
  }

  function updateMode(index: number, patch: Partial<ModeDefinition>) {
    const next = modes.slice();
    next[index] = { ...next[index], ...patch };
    save(next);
  }

  function addMode() {
    const existingNames = new Set(modes.map((m) => m.name.trim().toLowerCase()));
    let name = "New mode";
    if (existingNames.has(name.toLowerCase())) {
      let n = 2;
      while (existingNames.has(`new mode ${n}`)) n += 1;
      name = `New mode ${n}`;
    }
    const newId = nextModeIdRef.current++;
    setModeIds((prev) => [...prev, newId]);
    save([
      ...modes,
      {
        name,
        behavior: "plain",
        apps: [],
        stt_model: null,
        llm_refinement: "off",
        custom_instructions: null,
        structure_long_prompts: false,
        is_default: false,
      },
    ]);
  }

  function removeMode(index: number) {
    const removedId = modeIds[index];
    setModeIds((prev) => prev.filter((_, i) => i !== index));
    if (removedId !== undefined) {
      setOpenPlaygrounds((prev) => {
        if (!(removedId in prev)) return prev;
        const next = { ...prev };
        delete next[removedId];
        return next;
      });
      setPickerOpenFor((cur) => (cur === removedId ? null : cur));
    }
    save(modes.filter((_, i) => i !== index));
  }

  function togglePlayground(modeId: number) {
    setOpenPlaygrounds((prev) => ({ ...prev, [modeId]: !prev[modeId] }));
  }

  function addAppToMode(index: number, bundleId: string, appName: string) {
    const next = modes.slice();
    next[index] = { ...next[index], apps: [...next[index].apps, { bundle_id: bundleId, app_name: appName }] };
    save(next);
    setPickerOpenFor(null);
  }

  function removeAppFromMode(index: number, bundleId: string) {
    const next = modes.slice();
    next[index] = { ...next[index], apps: next[index].apps.filter((a) => a.bundle_id !== bundleId) };
    save(next);
  }

  const assignedBundleIds = new Set(modes.flatMap((m) => m.apps.map((a) => a.bundle_id)));
  const lastAppAlreadyAssigned = lastApp && assignedBundleIds.has(lastApp.bundle_id);
  const appIcon = (bundleId: string) => runningApps.find((a) => a.bundle_id === bundleId)?.icon_data_uri ?? null;

  if (loading) {
    return <span className="loading loading-spinner loading-xs" />;
  }

  return (
    <>
      <div className="flex flex-col gap-3">
        {modes.map((m, index) => {
          const modeId = modeIds[index] ?? index;
          const pickableApps = runningApps.filter((a) => !assignedBundleIds.has(a.bundle_id));
          const playgroundOpen = Boolean(openPlaygrounds[modeId]);
          return (
            <div key={modeId} className="rounded-lg border border-base-content/10 bg-base-100 p-4">
              <div className="mb-2 flex items-center justify-between gap-2">
                <input
                  className="input input-xs flex-1 font-medium"
                  value={m.name}
                  onChange={(e) => updateMode(index, { name: e.target.value })}
                  aria-label="Mode name"
                />
                {!m.is_default && (
                  <button
                    className="btn btn-ghost btn-xs"
                    onClick={() => removeMode(index)}
                    aria-label={`Delete ${m.name}`}
                  >
                    ✕
                  </button>
                )}
              </div>

              <div className="mb-2 flex flex-wrap items-center gap-1.5">
                {m.apps.map((a) => (
                  <span
                    key={a.bundle_id}
                    className="flex items-center gap-1 rounded-full bg-base-200 py-0.5 pl-1 pr-1.5 text-[11px]"
                  >
                    <AppIcon src={appIcon(a.bundle_id)} name={a.app_name} />
                    {a.app_name}
                    <button
                      className="opacity-50 hover:opacity-100"
                      onClick={() => removeAppFromMode(index, a.bundle_id)}
                      aria-label={`Remove ${a.app_name} from ${m.name}`}
                    >
                      ✕
                    </button>
                  </span>
                ))}
                <div className="relative">
                  <button
                    className="btn btn-ghost btn-xs"
                    onClick={() => {
                      setPickerOpenFor(pickerOpenFor === modeId ? null : modeId);
                      loadRunningApps();
                    }}
                  >
                    + Add app
                  </button>
                  {pickerOpenFor === modeId && (
                    <ul className="absolute z-10 mt-1 max-h-52 w-48 overflow-y-auto rounded-md bg-base-100 p-1 shadow-lg">
                      {pickableApps.length === 0 && <li className="px-2 py-1 text-xs opacity-50">Loading…</li>}
                      {pickableApps.map((a, i) => {
                        const prevRunning = i > 0 ? pickableApps[i - 1].is_running : true;
                        return (
                          <li key={a.bundle_id}>
                            {prevRunning && !a.is_running && (
                              <div className="mt-1 border-t border-base-content/10 pt-1 text-[10px] uppercase opacity-40">
                                Other apps
                              </div>
                            )}
                            <button
                              className="flex w-full items-center gap-1.5 rounded px-2 py-1 text-left text-xs hover:bg-base-200"
                              onClick={() => addAppToMode(index, a.bundle_id, a.name)}
                            >
                              <AppIcon src={a.icon_data_uri} name={a.name} />
                              <span className="truncate">{a.name}</span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-1.5">
                <select
                  className="select select-xs"
                  value={m.behavior}
                  onChange={(e) => updateMode(index, { behavior: e.target.value as Behavior })}
                >
                  {BEHAVIORS.map((b) => (
                    <option key={b} value={b}>
                      {BEHAVIOR_LABEL[b]}
                    </option>
                  ))}
                </select>
                <select
                  className="select select-xs"
                  value={m.stt_model ?? GLOBAL_MODEL}
                  onChange={(e) =>
                    updateMode(index, { stt_model: e.target.value === GLOBAL_MODEL ? null : e.target.value })
                  }
                >
                  <option value={GLOBAL_MODEL}>Global model</option>
                  {availableModels.map((mo) => (
                    <option key={mo.id} value={mo.id}>
                      {mo.label}
                    </option>
                  ))}
                </select>
                <select
                  className="select select-xs"
                  value={llmSelectValue(m.llm_refinement)}
                  onChange={(e) => updateMode(index, { llm_refinement: llmFromSelectValue(e.target.value) })}
                >
                  <option value="off">No LLM refinement</option>
                  <option value="global">Refine with global LLM</option>
                  {llmCatalog.map((l) => (
                    <option key={l.id} value={l.id}>
                      {l.downloaded ? "✓ " : "⬇ "}
                      {l.label}
                      {l.source !== "Ollama" ? ` · ${l.source}` : ""}
                      {!l.downloaded && l.size_gb > 0 ? ` — ${l.size_gb.toFixed(1)}GB, not installed` : ""}
                    </option>
                  ))}
                </select>
                <InfoPopover>
                  <p className="mb-1 font-semibold">Installing a model</p>
                  <p>
                    Models run fully offline via Ollama, or a local LM Studio, llama.cpp or Jan server. "✓" means it's already downloaded and ready to use for free;
                    "⬇" means selecting it triggers a one-time download (the size shown) that's kept permanently on
                    disk. No ongoing cost either way — no data ever leaves this machine, and there's no per-use fee.
                  </p>
                </InfoPopover>

                <button
                  type="button"
                  data-testid={`test-mode-${index}`}
                  aria-expanded={playgroundOpen}
                  aria-controls={`mode-playground-${index}`}
                  onClick={() => togglePlayground(modeId)}
                  className="ml-auto inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-semibold transition-all duration-150 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[var(--dw-accent)]"
                  style={
                    playgroundOpen
                      ? {
                          background: "color-mix(in srgb, var(--dw-accent) 16%, transparent)",
                          border: "1px solid color-mix(in srgb, var(--dw-accent) 55%, var(--dw-border))",
                          color: "var(--dw-accent)",
                        }
                      : {
                          background: "var(--dw-bg-sunk)",
                          border: "1px solid var(--dw-border-strong)",
                          color: "var(--dw-fg)",
                        }
                  }
                >
                  <span aria-hidden="true" className="text-[9px]">
                    {playgroundOpen ? "▾" : "▸"}
                  </span>
                  Test mode
                </button>
              </div>

              {(() => {
                const selectedModelId = typeof m.llm_refinement === "object" ? m.llm_refinement.model : null;
                const selected = selectedModelId ? llmCatalog.find((l) => l.id === selectedModelId) : undefined;
                if (!selectedModelId || !selected || selected.downloaded) return null;
                const progress = llmProgress[selectedModelId];
                const error = llmErrors[selectedModelId];
                return (
                  <div className="mt-1.5 flex items-center gap-1.5 text-[11px] text-warning">
                    <span>
                      {selected.label} isn't installed yet{selected.size_gb > 0 ? ` (${selected.size_gb.toFixed(1)}GB)` : ""}.
                    </span>
                    {progress ? (
                      <span className="opacity-70" title={progress.status}>
                        {progress.percent !== null ? `${progress.percent}%` : progress.status}
                      </span>
                    ) : (
                      <button className="btn btn-xs" onClick={() => pullLlmModel(selectedModelId)}>
                        Download
                      </button>
                    )}
                    {error && <span className="text-error">{error}</span>}
                  </div>
                );
              })()}

              {llmSelectValue(m.llm_refinement) !== "off" && (
                <textarea
                  className="textarea textarea-xs mt-2 w-full font-sans"
                  rows={2}
                  aria-label={`Extra LLM instructions for ${m.name}`}
                  placeholder={
                    'Extra instructions for the LLM, e.g. "Always sign off with \'thanks, Jake\'."\n\n' +
                    'Example — said: "hey can we push the meeting to thursday" -> output: "Hi, could we push ' +
                    'our meeting to Thursday?\\n\\nThanks,\\nJake"'
                  }
                  value={m.custom_instructions ?? ""}
                  onChange={(e) =>
                    updateMode(index, { custom_instructions: e.target.value === "" ? null : e.target.value })
                  }
                />
              )}

              <label
                className="mt-2 flex items-start gap-1.5 text-xs"
                title="Uses this mode's model, or the global one when cleanup is off. The structured version is only used if every name, file, number and requirement you said is still in it; otherwise your words are pasted as spoken."
              >
                <input
                  type="checkbox"
                  className="checkbox checkbox-xs mt-0.5"
                  checked={m.structure_long_prompts ?? false}
                  onChange={(e) => updateMode(index, { structure_long_prompts: e.target.checked })}
                  aria-label={`Structure long prompts in ${m.name}`}
                />
                <span>
                  Structure long prompts
                  <span className="block opacity-60">
                    Dictations of 80+ words become Goal, Context, Constraints and Done when, for a coding agent.
                  </span>
                </span>
              </label>

              {playgroundOpen && <ModePlayground mode={m} index={index} />}
            </div>
          );
        })}
        <button className="btn btn-xs w-full justify-start" onClick={addMode}>
          + Add mode
        </button>
      </div>

      {lastApp && !lastAppAlreadyAssigned && (
        <div className="mt-1.5 flex items-center gap-1.5 text-xs">
          <AppIcon src={lastApp.icon_data_uri} name={lastApp.name} />
          <span className="truncate opacity-70">Add {lastApp.name} to:</span>
          <select
            className="select select-xs flex-1"
            value=""
            onChange={(e) => {
              const index = Number(e.target.value);
              if (Number.isInteger(index) && index >= 0 && index < modes.length) {
                addAppToMode(index, lastApp.bundle_id, lastApp.name);
              }
            }}
          >
            <option value="" disabled>
              Choose a mode…
            </option>
            {modes.map((m, idx) => (
              <option key={modeIds[idx] ?? idx} value={String(idx)}>
                {m.name}
              </option>
            ))}
          </select>
        </div>
      )}
    </>
  );
}
