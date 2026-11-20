import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { type WidgetMode } from "../shared";
import { shortcutLabel, skipVariant, type ShortcutConfig } from "../../shortcut";
import {
  DEFAULT_WAVEFORM_CONFIG,
  getWaveformColorway,
  SOLID_COLORWAY_ORDER,
  SPECTRUM_COLORWAY_ORDER,
  WAVEFORM_COLORWAYS,
  type WaveformColorwayId,
  type WaveformConfig,
  type WaveformEngineId,
} from "../../waveformConfig";




export const IGNORED_KEYS = new Set(["Meta", "Control", "Alt", "Shift"]);

export function ShortcutSection() {
  const [shortcut, setShortcut] = useState<ShortcutConfig | null>(null);
  const [listening, setListening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [doubleTapEnabled, setDoubleTapEnabled] = useState(false);
  const [doubleTapChecked, setDoubleTapChecked] = useState(false);

  useEffect(() => {
    invoke<ShortcutConfig>("get_shortcut")
      .then(setShortcut)
      .catch((err) => notifyError("get_shortcut", err));

    invoke<boolean>("get_double_tap_enabled")
      .then(setDoubleTapEnabled)
      .catch((err) => notifyError("get_double_tap_enabled", err))
      .finally(() => setDoubleTapChecked(true));
  }, []);

  function toggleDoubleTap(enabled: boolean) {
    setDoubleTapEnabled(enabled);
    invoke("set_double_tap_enabled", { enabled }).catch((err) => {
      notifyError("set_double_tap_enabled", err);
      setDoubleTapEnabled(!enabled);
    });
  }

  useEffect(() => {
    if (!listening) return;

    function onKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setListening(false);
        return;
      }
      if (IGNORED_KEYS.has(e.key)) return;

      const next: ShortcutConfig = {
        meta: e.metaKey,
        ctrl: e.ctrlKey,
        alt: e.altKey,
        shift: e.shiftKey,
        code: e.code,
      };
      setListening(false);
      setError(null);
      invoke<ShortcutConfig>("set_shortcut", { newCfg: next })
        .then(setShortcut)
        .catch((err) => setError(String(err)));
    }

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [listening]);

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Push-to-talk key</h3>
      <div className="flex items-center gap-2">
        <button
          className={`btn btn-sm flex-1 ${listening ? "btn-primary" : ""}`}
          onClick={() => setListening(true)}
        >
          {listening ? "Press a key combo… (Esc to cancel)" : shortcut ? shortcutLabel(shortcut) : "…"}
        </button>
      </div>
      {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
      {shortcut && (
        <p className="mt-1.5 text-xs opacity-60" data-testid="skip-shortcut-hint">
          {shortcutLabel(skipVariant(shortcut))} stops and pastes as-is, without LLM cleanup. Also works while it's
          refining, as does clicking the violet bar.
        </p>
      )}
      {doubleTapChecked && (
        <label className="mt-3 flex items-center gap-2 text-xs opacity-80">
          <input
            type="checkbox"
            className="checkbox checkbox-xs"
            checked={doubleTapEnabled}
            onChange={(e) => toggleDoubleTap(e.target.checked)}
          />
          Double-tap Fn to start/stop recording (in addition to the key above)
        </label>
      )}
      {doubleTapEnabled && (
        <p className="mt-1.5 text-xs opacity-60">
          Needs macOS's Input Monitoring permission — System Settings → Privacy &amp; Security → Input Monitoring.
        </p>
      )}
    </div>
  );
}

type AudioDeviceStatus =
  | { kind: "preferred"; using: string }
  | { kind: "fallback"; wanted: string; using: string }
  | { kind: "system_default"; using: string }
  | { kind: "unavailable"; wanted: string | null };

const MIC_SEGMENTS = 16;
const MICRO_SEGMENTS = 10;
const NOISE_FLOOR = 0.006;
const PEAK_HOLD_MS = 900;

function formatDbfs(level: number): string {
  if (level < NOISE_FLOOR) return "Silent";
  const db = Math.max(-60, Math.min(0, Math.round(20 * Math.log10(level))));
  return `${db} dB`;
}

function litSegmentCount(level: number, total: number): number {
  if (level < NOISE_FLOOR) return 0;
  return Math.min(total, Math.max(1, Math.round(Math.pow(level, 0.58) * total)));
}

function clamp01(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;
}

function parseAudioBands(payload: unknown): [number, number, number] | null {
  if (Array.isArray(payload) && payload.length >= 3) {
    if (payload.length === 3) {
      return [clamp01(payload[0]), clamp01(payload[1]), clamp01(payload[2])];
    }
    const third = Math.max(1, Math.floor(payload.length / 3));
    const avg = (slice: unknown[]) =>
      slice.length === 0 ? 0 : slice.reduce<number>((sum, v) => sum + clamp01(v), 0) / slice.length;
    return [
      avg(payload.slice(0, third)),
      avg(payload.slice(third, third * 2)),
      avg(payload.slice(third * 2)),
    ];
  }
  if (payload && typeof payload === "object") {
    const obj = payload as Record<string, unknown>;
    if ("bands" in obj) return parseAudioBands(obj.bands);
    if ("low" in obj || "mid" in obj || "high" in obj) {
      return [clamp01(obj.low), clamp01(obj.mid), clamp01(obj.high)];
    }
  }
  return null;
}

function micStatusBadge(status: AudioDeviceStatus | null): {
  label: string;
  shortLabel: string;
  detail: string;
  dotColor: string;
  textColor: string;
} | null {
  if (!status) return null;
  switch (status.kind) {
    case "preferred":
      return {
        label: "Active",
        shortLabel: "Active",
        detail: `Using ${status.using}`,
        dotColor: "#10b981",
        textColor: "color-mix(in srgb, #10b981 74%, var(--dw-fg))",
      };
    case "system_default":
      return {
        label: "Active · Default",
        shortLabel: "Active",
        detail: `Using system default (${status.using})`,
        dotColor: "#10b981",
        textColor: "color-mix(in srgb, #10b981 74%, var(--dw-fg))",
      };
    case "fallback":
      return {
        label: "Fallback",
        shortLabel: "Fallback",
        detail: `${status.wanted} unavailable — using ${status.using}`,
        dotColor: "#f59e0b",
        textColor: "color-mix(in srgb, #f59e0b 74%, var(--dw-fg))",
      };
    case "unavailable":
      return {
        label: "Unavailable",
        shortLabel: "Unavailable",
        detail: status.wanted ? `${status.wanted} is unavailable` : "No active microphone available",
        dotColor: "#ef4444",
        textColor: "color-mix(in srgb, #ef4444 78%, var(--dw-fg))",
      };
  }
}

function deviceOptionNote(
  name: string,
  status: AudioDeviceStatus | null,
  isSelected: boolean,
): { text: string; color: string } | null {
  if (status) {
    if (status.kind === "fallback") {
      if (name === status.using) {
        return { text: "Fallback · active input", color: "color-mix(in srgb, #f59e0b 74%, var(--dw-fg))" };
      }
      if (name === status.wanted) {
        return { text: "Preferred · unavailable", color: "color-mix(in srgb, #ef4444 78%, var(--dw-fg))" };
      }
    } else if (status.kind === "preferred" && name === status.using) {
      return { text: "Preferred · active", color: "color-mix(in srgb, #10b981 74%, var(--dw-fg))" };
    } else if (status.kind === "system_default" && name === status.using) {
      return { text: "System default · active", color: "color-mix(in srgb, #10b981 74%, var(--dw-fg))" };
    } else if (status.kind === "unavailable" && name === status.wanted) {
      return { text: "Unavailable", color: "color-mix(in srgb, #ef4444 78%, var(--dw-fg))" };
    }
  }
  if (isSelected) {
    return { text: "Preferred input", color: "var(--dw-muted)" };
  }
  return null;
}

function InlineMicroMeter({
  level,
  showReadout = false,
  testId,
}: {
  level: number;
  showReadout?: boolean;
  testId?: string;
}) {
  const lit = litSegmentCount(level, MICRO_SEGMENTS);
  const dbfs = formatDbfs(level);
  const hot = level >= NOISE_FLOOR;
  return (
    <span
      className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-1.5 py-1"
      style={{
        background: "var(--dw-bg-well)",
        border: "1px solid color-mix(in srgb, var(--dw-border-strong) 75%, transparent)",
      }}
      data-testid={testId}
      aria-hidden="true"
    >
      <span className="flex items-center gap-[2px]">
        {Array.from({ length: MICRO_SEGMENTS }, (_, i) => {
          const active = i < lit;
          const color = i >= 8 ? "#ef4444" : i >= 6 ? "#f59e0b" : "#10b981";
          return (
            <span
              key={i}
              className="h-3 w-[3px] rounded-[1.5px] transition-opacity duration-75"
              style={{
                backgroundColor: active ? color : "rgba(255, 255, 255, 0.16)",
                boxShadow: active ? `0 0 5px ${color}88` : undefined,
                opacity: active ? 1 : 0.45,
              }}
            />
          );
        })}
      </span>
      {showReadout && (
        <span
          className="w-10 text-right text-[9.5px] tabular-nums"
          style={{
            fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
            color: hot ? "#10b981" : "rgba(255, 255, 255, 0.45)",
          }}
        >
          {dbfs}
        </span>
      )}
    </span>
  );
}

const BAND_META = [
  { key: "LOW", color: "#10b981" },
  { key: "MID", color: "#f59e0b" },
  { key: "HIGH", color: "#06b6d4" },
] as const;

export function DeviceSection() {
  const [devices, setDevices] = useState<string[]>([]);
  const [activeDevice, setActiveDevice] = useState<string | null>(null);
  const [deviceStatus, setDeviceStatus] = useState<AudioDeviceStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState(0);
  const [micCheckActive, setMicCheckActive] = useState(false);
  const [micCheckError, setMicCheckError] = useState<string | null>(null);
  const [micLevel, setMicLevel] = useState(0);
  const [peakLevel, setPeakLevel] = useState(0);
  const [bands, setBands] = useState<[number, number, number]>([0, 0, 0]);

  const dropdownRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const levelRef = useRef(0);
  const peakRef = useRef(0);
  const peakHoldUntilRef = useRef(0);
  const bandsRef = useRef<[number, number, number]>([0, 0, 0]);
  const decayTimerRef = useRef<number | null>(null);
  const micCheckActiveRef = useRef(false);
  const micCheckGenRef = useRef(0);

  useEffect(() => {
    let disposed = false;

    const syncActiveFromStatus = (status: AudioDeviceStatus | null) => {
      if (!status) return;
      if (status.kind === "preferred" || status.kind === "system_default") {
        setActiveDevice(status.using);
      } else if (status.kind === "fallback") {
        setActiveDevice(status.wanted);
      } else if (status.kind === "unavailable" && status.wanted) {
        setActiveDevice(status.wanted);
      }
    };

    Promise.all([
      invoke<string[]>("list_input_devices"),
      invoke<string | null>("get_active_input_device"),
    ])
      .then(([deviceList, active]) => {
        if (disposed) return;
        setDevices(deviceList);
        setActiveDevice(active);
      })
      .catch((err) => {
        if (!disposed) notifyError("Couldn't load audio devices", err);
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });

    invoke<AudioDeviceStatus | null>("get_audio_device_status")
      .then((status) => {
        if (disposed) return;
        setDeviceStatus(status);
      })
      .catch(() => {});

    const unlistenDevice = listen<AudioDeviceStatus>("audio-device-status", (e) => {
      if (disposed) return;
      setDeviceStatus(e.payload);
      syncActiveFromStatus(e.payload);
    });

    const stopDecay = () => {
      if (decayTimerRef.current !== null) {
        window.clearInterval(decayTimerRef.current);
        decayTimerRef.current = null;
      }
    };

    const ensureDecayLoop = () => {
      if (disposed || decayTimerRef.current !== null) return;
      decayTimerRef.current = window.setInterval(() => {
        if (disposed) {
          stopDecay();
          return;
        }
        const nextLevel = levelRef.current * 0.82;
        levelRef.current = nextLevel < 0.003 ? 0 : nextLevel;
        setMicLevel(levelRef.current);

        const now = performance.now();
        if (now >= peakHoldUntilRef.current) {
          const decayedPeak = Math.max(levelRef.current, peakRef.current * 0.9);
          peakRef.current = decayedPeak < 0.003 ? 0 : decayedPeak;
          setPeakLevel(peakRef.current);
        }

        const nextBands: [number, number, number] = [
          bandsRef.current[0] * 0.8 < 0.003 ? 0 : bandsRef.current[0] * 0.8,
          bandsRef.current[1] * 0.8 < 0.003 ? 0 : bandsRef.current[1] * 0.8,
          bandsRef.current[2] * 0.8 < 0.003 ? 0 : bandsRef.current[2] * 0.8,
        ];
        bandsRef.current = nextBands;
        setBands(nextBands);

        if (
          levelRef.current === 0 &&
          peakRef.current === 0 &&
          nextBands[0] === 0 &&
          nextBands[1] === 0 &&
          nextBands[2] === 0
        ) {
          stopDecay();
        }
      }, 45);
    };

    const unlistenLevel = listen<number>("audio-level", (e) => {
      if (disposed) return;
      const clamped = clamp01(e.payload);
      const smoothed = Math.max(clamped, levelRef.current * 0.65);
      levelRef.current = smoothed;
      setMicLevel(smoothed);
      if (smoothed >= peakRef.current) {
        peakRef.current = smoothed;
        peakHoldUntilRef.current = performance.now() + PEAK_HOLD_MS;
        setPeakLevel(smoothed);
      }
      if (smoothed >= 0.003 || peakRef.current >= 0.003) {
        ensureDecayLoop();
      }
    });

    const unlistenBands = listen<unknown>("audio-bands", (e) => {
      if (disposed) return;
      const parsed = parseAudioBands(e.payload);
      if (!parsed) return;
      const smoothed: [number, number, number] = [
        Math.max(parsed[0], bandsRef.current[0] * 0.55),
        Math.max(parsed[1], bandsRef.current[1] * 0.55),
        Math.max(parsed[2], bandsRef.current[2] * 0.55),
      ];
      bandsRef.current = smoothed;
      setBands(smoothed);
      if (smoothed[0] >= 0.003 || smoothed[1] >= 0.003 || smoothed[2] >= 0.003) {
        ensureDecayLoop();
      }
    });

    const onVisibilityChange = () => {
      if (document.hidden && micCheckActiveRef.current) {
        micCheckGenRef.current += 1;
        micCheckActiveRef.current = false;
        setMicCheckActive(false);
        invoke("stop_mic_check").catch(() => {});
      }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);

    return () => {
      disposed = true;
      document.removeEventListener("visibilitychange", onVisibilityChange);
      stopDecay();
      if (micCheckActiveRef.current) {
        micCheckGenRef.current += 1;
        micCheckActiveRef.current = false;
        invoke("stop_mic_check").catch(() => {});
      }
      unlistenDevice.then((f) => f()).catch(() => {});
      unlistenLevel.then((f) => f()).catch(() => {});
      unlistenBands.then((f) => f()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!dropdownOpen) return;

    function onMouseDown(e: MouseEvent) {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setDropdownOpen(false);
      }
    }

    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        setDropdownOpen(false);
        triggerRef.current?.focus();
      }
    }

    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [dropdownOpen]);

  function selectDevice(name: string) {
    const previous = activeDevice;
    setActiveDevice(name);
    setDropdownOpen(false);
    levelRef.current = 0;
    peakRef.current = 0;
    peakHoldUntilRef.current = 0;
    bandsRef.current = [0, 0, 0];
    setMicLevel(0);
    setPeakLevel(0);
    setBands([0, 0, 0]);
    invoke("set_input_device", { name }).catch((err) => {
      notifyError("set_input_device", err);
      setActiveDevice(previous);
    });
  }

  function toggleDropdown() {
    setDropdownOpen((open) => {
      if (!open) {
        const idx = Math.max(0, devices.indexOf(selectedName));
        setHighlightedIndex(idx);
      }
      return !open;
    });
  }

  function onTriggerKeyDown(e: ReactKeyboardEvent<HTMLButtonElement>) {
    if (devices.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!dropdownOpen) {
        setHighlightedIndex(Math.max(0, devices.indexOf(selectedName)));
        setDropdownOpen(true);
      } else {
        setHighlightedIndex((i) => (i + 1) % devices.length);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (!dropdownOpen) {
        setHighlightedIndex(Math.max(0, devices.indexOf(selectedName)));
        setDropdownOpen(true);
      } else {
        setHighlightedIndex((i) => (i - 1 + devices.length) % devices.length);
      }
    } else if (e.key === "Enter" || e.key === " ") {
      if (dropdownOpen) {
        e.preventDefault();
        const choice = clampedHighlighted >= 0 ? devices[clampedHighlighted] : undefined;
        if (choice) selectDevice(choice);
        triggerRef.current?.focus();
      }
    }
  }

  function toggleMicCheck() {
    if (micCheckActiveRef.current) {
      micCheckGenRef.current += 1;
      micCheckActiveRef.current = false;
      setMicCheckActive(false);
      setMicCheckError(null);
      invoke("stop_mic_check").catch(() => {});
      return;
    }

    const gen = ++micCheckGenRef.current;
    setMicCheckError(null);
    micCheckActiveRef.current = true;
    setMicCheckActive(true);
    invoke("start_mic_check").catch(() => {
      if (micCheckGenRef.current !== gen) return;
      micCheckActiveRef.current = false;
      setMicCheckActive(false);
      setMicCheckError("Live mic check isn't available right now.");
    });
  }

  const badge = micStatusBadge(deviceStatus);
  const selectedName = activeDevice ?? devices[0] ?? "";
  const activeUsingName =
    deviceStatus && "using" in deviceStatus ? deviceStatus.using : selectedName;
  const missingPreferredName =
    deviceStatus &&
    (deviceStatus.kind === "fallback" || deviceStatus.kind === "unavailable") &&
    deviceStatus.wanted &&
    !devices.includes(deviceStatus.wanted)
      ? deviceStatus.wanted
      : null;
  const clampedHighlighted =
    devices.length > 0 ? Math.min(highlightedIndex, devices.length - 1) : -1;
  const audioFlowing = micLevel >= NOISE_FLOOR;
  const litSegments = litSegmentCount(micLevel, MIC_SEGMENTS);
  const peakSegments = litSegmentCount(peakLevel, MIC_SEGMENTS);
  const dbfsText = formatDbfs(micLevel);

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <div className="mb-3 flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide opacity-60">Microphone</h3>
        {micCheckActive && (
          <span
            className="inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider"
            style={{
              background: "color-mix(in srgb, #10b981 14%, transparent)",
              border: "1px solid color-mix(in srgb, #10b981 40%, transparent)",
              color: "color-mix(in srgb, #10b981 74%, var(--dw-fg))",
            }}
          >
            <span
              className="h-1.5 w-1.5 animate-pulse rounded-full"
              style={{ backgroundColor: "#10b981", boxShadow: "0 0 6px #10b981" }}
            />
            Live input
          </span>
        )}
      </div>

      {loading ? (
        <span className="loading loading-spinner loading-xs" />
      ) : devices.length === 0 ? (
        <p className="text-xs opacity-60">No input devices found.</p>
      ) : (
        <>
          <div
            className="relative"
            ref={dropdownRef}
            onBlur={(e) => {
              if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
                setDropdownOpen(false);
              }
            }}
          >
            <button
              ref={triggerRef}
              type="button"
              role="combobox"
              aria-label="Microphone"
              aria-expanded={dropdownOpen}
              aria-haspopup="listbox"
              aria-controls="studio-mic-listbox"
              aria-activedescendant={
                dropdownOpen && clampedHighlighted >= 0
                  ? `studio-mic-opt-${clampedHighlighted}`
                  : undefined
              }
              onClick={toggleDropdown}
              onKeyDown={onTriggerKeyDown}
              className="flex w-full items-center justify-between gap-3 rounded-xl px-3 py-2.5 text-left transition-all duration-150 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--dw-accent)]"
              style={{
                background: "var(--dw-bg-sunk)",
                border: `1px solid ${dropdownOpen ? "var(--dw-accent)" : "var(--dw-border-strong)"}`,
                boxShadow: dropdownOpen
                  ? "0 0 0 3px color-mix(in srgb, var(--dw-accent) 18%, transparent)"
                  : undefined,
                color: "var(--dw-fg)",
              }}
              data-testid="studio-mic-trigger"
            >
              <span className="flex min-w-0 flex-1 items-center gap-2.5">
                <span
                  className="h-2 w-2 shrink-0 rounded-full"
                  style={{
                    backgroundColor: badge ? badge.dotColor : "var(--dw-muted)",
                    boxShadow: badge ? `0 0 7px ${badge.dotColor}88` : undefined,
                  }}
                />
                <span className="flex min-w-0 flex-col">
                  <span className="truncate text-[13px] font-semibold leading-tight">
                    {selectedName || "Select microphone"}
                  </span>
                  {deviceStatus?.kind === "fallback" && (
                    <span
                      className="truncate text-[10.5px]"
                      style={{ color: "color-mix(in srgb, #f59e0b 74%, var(--dw-fg))" }}
                    >
                      Using {deviceStatus.using} (fallback)
                    </span>
                  )}
                  {deviceStatus?.kind === "unavailable" && (
                    <span
                      className="truncate text-[10.5px]"
                      style={{ color: "color-mix(in srgb, #ef4444 78%, var(--dw-fg))" }}
                    >
                      {badge?.detail}
                    </span>
                  )}
                </span>
              </span>

              <span className="flex shrink-0 items-center gap-2">
                {audioFlowing && <InlineMicroMeter level={micLevel} testId="mic-trigger-meter" />}
                {badge && (
                  <span
                    className="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-semibold"
                    style={{
                      background: `color-mix(in srgb, ${badge.dotColor} 14%, transparent)`,
                      border: `1px solid color-mix(in srgb, ${badge.dotColor} 35%, transparent)`,
                      color: badge.textColor,
                    }}
                  >
                    {badge.shortLabel}
                  </span>
                )}
                <span
                  className="text-xs transition-transform duration-150"
                  style={{
                    color: "var(--dw-muted)",
                    transform: dropdownOpen ? "rotate(180deg)" : "rotate(0deg)",
                  }}
                  aria-hidden="true"
                >
                  ▾
                </span>
              </span>
            </button>

            {dropdownOpen && (
              <div
                id="studio-mic-listbox"
                role="listbox"
                aria-label="Input devices"
                onMouseDown={(e) => {
                  // Prevent the trigger button from blurring on mousedown in macOS WKWebView
                  // (where <button> is not mouse-focusable and relatedTarget is null), while
                  // still allowing native scrollbar dragging on the container edge.
                  if (e.target !== e.currentTarget || e.nativeEvent.offsetX < e.currentTarget.clientWidth) {
                    e.preventDefault();
                  }
                }}
                className="absolute left-0 right-0 z-30 mt-1.5 flex max-h-64 flex-col gap-1 overflow-y-auto rounded-xl p-1.5 shadow-2xl"
                style={{
                  background: "var(--dw-card-raised)",
                  border: "1px solid var(--dw-border-strong)",
                }}
              >
                {missingPreferredName && (
                  <button
                    key={`missing-${missingPreferredName}`}
                    type="button"
                    role="option"
                    aria-selected={selectedName === missingPreferredName}
                    aria-disabled="true"
                    disabled
                    tabIndex={-1}
                    className="flex w-full cursor-not-allowed items-center justify-between gap-2.5 rounded-lg px-2.5 py-2 text-left opacity-70"
                    style={{
                      background: "color-mix(in srgb, #ef4444 8%, var(--dw-bg-sunk))",
                      border: "1px solid color-mix(in srgb, #ef4444 30%, var(--dw-border))",
                      color: "var(--dw-fg)",
                    }}
                  >
                    <span className="flex min-w-0 flex-1 items-start gap-2">
                      <span
                        className="mt-0.5 w-3.5 shrink-0 text-center text-xs font-bold"
                        style={{ color: "color-mix(in srgb, #ef4444 78%, var(--dw-fg))" }}
                        aria-hidden="true"
                      >
                        !
                      </span>
                      <span className="flex min-w-0 flex-col gap-0.5">
                        <span className="truncate text-[12.5px] font-semibold leading-tight">
                          {missingPreferredName}
                        </span>
                        <span
                          className="truncate text-[10.5px]"
                          style={{ color: "color-mix(in srgb, #ef4444 78%, var(--dw-fg))" }}
                        >
                          Preferred · unavailable
                        </span>
                      </span>
                    </span>
                  </button>
                )}
                {devices.map((name, idx) => {
                  const isSelected = name === selectedName;
                  const isActiveInput = name === activeUsingName;
                  const isHighlighted = idx === clampedHighlighted;
                  const note = deviceOptionNote(name, deviceStatus, isSelected);
                  return (
                    <button
                      key={name}
                      id={`studio-mic-opt-${idx}`}
                      type="button"
                      role="option"
                      tabIndex={-1}
                      aria-selected={isSelected}
                      onMouseDown={(e) => e.preventDefault()}
                      onMouseEnter={() => setHighlightedIndex(idx)}
                      onClick={() => {
                        selectDevice(name);
                        triggerRef.current?.focus();
                      }}
                      className="flex w-full items-center justify-between gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors duration-100"
                      style={{
                        background: isSelected
                          ? "color-mix(in srgb, var(--dw-accent) 13%, var(--dw-bg-sunk))"
                          : isHighlighted
                            ? "var(--dw-bg-sunk)"
                            : "transparent",
                        border: `1px solid ${
                          isSelected
                            ? "color-mix(in srgb, var(--dw-accent) 45%, var(--dw-border))"
                            : isHighlighted
                              ? "var(--dw-border)"
                              : "transparent"
                        }`,
                        color: "var(--dw-fg)",
                      }}
                    >
                      <span className="flex min-w-0 flex-1 items-start gap-2">
                        <span
                          className="mt-0.5 w-3.5 shrink-0 text-center text-xs font-bold"
                          style={{ color: isSelected ? "var(--dw-accent)" : "transparent" }}
                          aria-hidden="true"
                        >
                          ✓
                        </span>
                        <span className="flex min-w-0 flex-col gap-0.5">
                          <span className="truncate text-[12.5px] font-semibold leading-tight">{name}</span>
                          {note && (
                            <span className="truncate text-[10.5px]" style={{ color: note.color }}>
                              {note.text}
                            </span>
                          )}
                        </span>
                      </span>

                      {isActiveInput && (
                        <InlineMicroMeter
                          level={micLevel}
                          showReadout
                          testId="mic-option-active-meter"
                        />
                      )}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          <div
            className="mt-2.5 flex flex-col gap-2 rounded-xl px-3 py-2.5"
            style={{
              background: "var(--dw-bg-sunk)",
              border: `1px solid ${
                micCheckActive
                  ? "color-mix(in srgb, #10b981 40%, var(--dw-border))"
                  : "var(--dw-border)"
              }`,
            }}
            data-testid="live-mic-check"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wider opacity-60">
                  Mic check
                </span>
                <button
                  type="button"
                  onClick={toggleMicCheck}
                  data-testid="mic-check-toggle"
                  className="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-semibold transition-all duration-150 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[var(--dw-accent)]"
                  style={
                    micCheckActive
                      ? {
                          background: "color-mix(in srgb, #ef4444 16%, transparent)",
                          border: "1px solid color-mix(in srgb, #ef4444 45%, transparent)",
                          color: "color-mix(in srgb, #ef4444 78%, var(--dw-fg))",
                        }
                      : {
                          background: "var(--dw-card-raised)",
                          border: "1px solid var(--dw-border-strong)",
                          color: "var(--dw-fg)",
                        }
                  }
                >
                  <span
                    className="h-1.5 w-1.5 rounded-full"
                    style={{
                      backgroundColor: micCheckActive ? "#ef4444" : "var(--dw-accent)",
                      boxShadow: micCheckActive ? "0 0 6px #ef4444" : undefined,
                    }}
                  />
                  {micCheckActive ? "Stop check" : "Test mic"}
                </button>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                {micCheckActive && (
                  <span
                    className="inline-flex items-center gap-2 rounded-full px-2 py-0.5"
                    style={{
                      background: "var(--dw-bg-well)",
                      border: "1px solid var(--dw-border)",
                    }}
                    data-testid="mic-tonal-bands"
                    title="Tonal balance (Low / Mid / High frequency energy)"
                  >
                    {BAND_META.map((band, idx) => {
                      const energy = bands[idx];
                      const active = energy >= NOISE_FLOOR;
                      const scale = 0.85 + Math.min(1, Math.pow(energy, 0.55)) * 0.45;
                      return (
                        <span
                          key={band.key}
                          className="inline-flex items-center gap-1 text-[9px] font-semibold tracking-wider"
                          style={{
                            fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
                            color: active ? band.color : "rgba(255, 255, 255, 0.42)",
                          }}
                        >
                          <span
                            className="h-1.5 w-1.5 rounded-full transition-transform duration-75"
                            style={{
                              backgroundColor: active ? band.color : "rgba(255, 255, 255, 0.25)",
                              boxShadow: active ? `0 0 6px ${band.color}` : undefined,
                              transform: `scale(${scale.toFixed(2)})`,
                            }}
                          />
                          {band.key}
                        </span>
                      );
                    })}
                  </span>
                )}

                {badge && (
                  <span
                    className="inline-flex shrink-0 items-center gap-1.5 rounded-full border border-base-content/10 bg-base-100/80 px-2 py-0.5 text-[10.5px] font-medium"
                    title={badge.detail}
                    data-testid="live-mic-status"
                  >
                    <span
                      className="h-1.5 w-1.5 rounded-full"
                      style={{
                        backgroundColor: badge.dotColor,
                        boxShadow: `0 0 5px ${badge.dotColor}88`,
                      }}
                    />
                    <span style={{ color: badge.textColor }}>{badge.label}</span>
                  </span>
                )}
              </div>
            </div>

            <div className="flex min-w-0 items-center gap-2.5">
              <div
                className="flex flex-1 items-center gap-[3px] rounded-md px-2 py-1.5"
                style={{
                  background: "var(--dw-bg-well)",
                  border: "1px solid color-mix(in srgb, var(--dw-border) 80%, transparent)",
                }}
                role="meter"
                aria-label="Live microphone input level"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(micLevel * 100)}
                aria-valuetext={dbfsText}
              >
                {Array.from({ length: MIC_SEGMENTS }, (_, i) => {
                  const active = i < litSegments;
                  const isPeakHold = peakSegments > 0 && i === peakSegments - 1 && i >= litSegments;
                  const segColor = i >= 13 ? "#ef4444" : i >= 10 ? "#f59e0b" : "#10b981";
                  return (
                    <span
                      key={i}
                      data-peak={isPeakHold ? "true" : undefined}
                      className="h-3 flex-1 rounded-[2px] transition-opacity duration-75"
                      style={{
                        backgroundColor:
                          active || isPeakHold ? segColor : "rgba(255, 255, 255, 0.14)",
                        boxShadow: isPeakHold
                          ? `0 0 8px ${segColor}, inset 0 0 0 1px rgba(255, 255, 255, 0.55)`
                          : active
                            ? `0 0 6px ${segColor}77`
                            : undefined,
                        opacity: active ? 1 : isPeakHold ? 0.92 : 0.4,
                      }}
                    />
                  );
                })}
              </div>
              <span
                className="w-12 shrink-0 text-right text-[11px] tabular-nums"
                style={{
                  fontFamily: "var(--dw-font-mono, ui-monospace, monospace)",
                  opacity: micLevel < NOISE_FLOOR ? 0.55 : 0.95,
                }}
                data-testid="live-mic-dbfs"
              >
                {dbfsText}
              </span>
            </div>

            <div className="flex flex-wrap items-center justify-between gap-2 text-[10.5px]">
              <span style={{ color: "var(--dw-muted)" }} data-testid="live-mic-hint">
                {micCheckActive
                  ? "Speak to check input gain"
                  : "Listening automatically during dictation"}
              </span>
              {peakLevel >= NOISE_FLOOR && (
                <span
                  className="tabular-nums opacity-70"
                  style={{ fontFamily: "var(--dw-font-mono, ui-monospace, monospace)" }}
                  data-testid="live-mic-peak"
                >
                  Peak {formatDbfs(peakLevel)}
                </span>
              )}
            </div>

            {micCheckError && (
              <p className="text-[11px] text-warning" role="status">
                {micCheckError}
              </p>
            )}
          </div>
        </>
      )}
    </div>
  );
}

export const WIDGET_MODE_LABEL: Record<WidgetMode, string> = {
  minimal: "Minimal",
  pill: "Pill",
  compact: "Compact",
  stacked: "Compact, two-line",
  notch: "Notch",
  caption: "Live caption",
  detailed: "Detailed",
};

const WIDGET_MODE_HINT: Record<WidgetMode, string> = {
  minimal: "Just the button",
  pill: "Dot and waveform; text on hover",
  compact: "Button, status and settings",
  stacked: "Compact plus mic and next mode",
  notch: "Hangs under the menu bar, grows while you talk",
  caption: "Your words as you speak",
  detailed: "Full status and messages",
};

const WIDGET_MODE_ORDER: WidgetMode[] = ["minimal", "pill", "compact", "stacked", "notch", "caption", "detailed"];

// A tiny drawing of each style, so the choice is visual.
function StylePreview({ mode }: { mode: WidgetMode }) {
  const dot = <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-error" />;
  const wave = (
    <span className="flex items-center gap-[1.5px]">
      {[3, 6, 4, 8, 5, 3].map((h, i) => (
        <span key={i} className="w-[1.5px] rounded-full bg-current opacity-60" style={{ height: h }} />
      ))}
    </span>
  );
  const line = (w: string) => <span className={`h-1 rounded-full bg-current opacity-40 ${w}`} />;
  const box = "flex items-center gap-1 border border-base-content/20 bg-base-300 px-1.5";
  switch (mode) {
    case "minimal":
      return <span className={`${box} h-5 w-5 justify-center rounded-full px-0`}>{dot}</span>;
    case "pill":
      return (
        <span className={`${box} h-4 w-14 rounded-full`}>
          {dot}
          {wave}
        </span>
      );
    case "compact":
      return (
        <span className={`${box} h-6 w-20 rounded-md`}>
          {dot}
          {line("w-8")}
        </span>
      );
    case "stacked":
      return (
        <span className={`${box} h-8 w-20 flex-col items-start justify-center gap-1 rounded-md`}>
          <span className="flex items-center gap-1">
            {dot}
            {line("w-8")}
          </span>
          {line("ml-2.5 w-10")}
        </span>
      );
    case "notch":
      return (
        <span className="flex w-24 flex-col items-center">
          <span className="h-1 w-full rounded-sm bg-base-content/20" />
          <span className="flex h-3.5 w-14 items-center justify-center gap-1 rounded-b-lg bg-black px-1.5 text-white">
            {dot}
            {wave}
          </span>
        </span>
      );
    case "caption":
      return (
        <span className={`${box} h-10 w-24 flex-col items-start justify-center gap-1 rounded-md`}>
          <span className="flex items-center gap-1">
            {dot}
            {wave}
          </span>
          {line("w-16")}
          {line("w-12")}
        </span>
      );
    case "detailed":
      return (
        <span className={`${box} h-10 w-20 flex-col items-start justify-center gap-1 rounded-md`}>
          <span className="flex items-center gap-1">
            {dot}
            {line("w-8")}
          </span>
          {line("w-14")}
          {line("w-10")}
        </span>
      );
  }
}

const WIDGET_MODE_HAS_HOVER_POPOVER: Record<WidgetMode, boolean> = {
  minimal: false,
  pill: false,
  compact: true,
  stacked: true,
  notch: false,
  caption: true,
  detailed: true,
};

const QUICK_WAVEFORM_ENGINES: { id: WaveformEngineId; label: string; title: string }[] = [
  { id: "precision", label: "Studio", title: "Studio Bars" },
  { id: "silk", label: "Silk", title: "Harmonic Silk" },
  { id: "matrix", label: "Matrix", title: "LED Matrix" },
  { id: "hyperdom", label: "Needles", title: "Kinetic Needles" },
];

export function WidgetStylePicker({ value, onChange }: { value: WidgetMode; onChange: (mode: WidgetMode) => void }) {
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3" role="radiogroup" aria-label="Widget style">
      {WIDGET_MODE_ORDER.map((m) => {
        const hasHoverPopover = WIDGET_MODE_HAS_HOVER_POPOVER[m];
        return (
          <button
            key={m}
            type="button"
            role="radio"
            aria-checked={value === m}
            onClick={() => onChange(m)}
            className={`flex flex-col items-center gap-1.5 rounded-lg border p-3 text-center transition-colors ${
              value === m ? "border-primary bg-primary/10" : "border-base-content/10 hover:border-base-content/30"
            }`}
            data-testid={`style-${m}`}
          >
            <span className="flex h-11 items-center justify-center">
              <StylePreview mode={m} />
            </span>
            <span className="text-xs font-medium">{WIDGET_MODE_LABEL[m]}</span>
            <span className="text-[11px] leading-tight opacity-60">{WIDGET_MODE_HINT[m]}</span>
            <span
              className="mt-auto inline-flex items-center rounded-full px-2 py-0.5 text-[9.5px] font-medium tracking-wide"
              style={
                hasHoverPopover
                  ? {
                      background: "color-mix(in srgb, var(--dw-accent) 12%, transparent)",
                      border: "1px solid color-mix(in srgb, var(--dw-accent) 32%, transparent)",
                      color: "var(--dw-accent)",
                    }
                  : {
                      background: "color-mix(in srgb, var(--dw-muted) 10%, transparent)",
                      border: "1px solid color-mix(in srgb, var(--dw-border-strong) 70%, transparent)",
                      color: "var(--dw-muted)",
                    }
              }
              data-testid={`style-${m}-popover-badge`}
            >
              {hasHoverPopover ? "Hover popover" : "Right-click / menu"}
            </span>
          </button>
        );
      })}
    </div>
  );
}

export function GeneralSection({
  onOpenAppearance,
  waveform: controlledWaveform,
  onChangeWaveform,
}: {
  onOpenAppearance?: () => void;
  waveform?: WaveformConfig;
  onChangeWaveform?: (next: WaveformConfig) => void;
} = {}) {
  const [autostart, setAutostart] = useState(false);
  const [widgetMode, setWidgetModeState] = useState<WidgetMode>("compact");
  const [onlyWhileDictating, setOnlyWhileDictating] = useState(false);
  const [sounds, setSounds] = useState(false);
  const [localWaveform, setLocalWaveform] = useState<WaveformConfig>(DEFAULT_WAVEFORM_CONFIG);
  const [checked, setChecked] = useState(false);

  const isWaveformControlled = controlledWaveform !== undefined;
  const waveform = controlledWaveform ?? localWaveform;

  useEffect(() => {
    let disposed = false;

    const waveformPromise = isWaveformControlled
      ? Promise.resolve<WaveformConfig | null>(null)
      : invoke<WaveformConfig>("get_waveform_config").catch(() => null);

    Promise.all([
      invoke<boolean>("get_autostart_enabled"),
      invoke<WidgetMode>("get_widget_mode"),
      invoke<boolean>("get_widget_only_while_dictating"),
      invoke<boolean>("get_dictation_sounds"),
      waveformPromise,
    ])
      .then(([autostartValue, widgetModeValue, onlyWhileDictatingValue, soundsValue, waveformValue]) => {
        if (disposed) return;
        setAutostart(autostartValue);
        setWidgetModeState(widgetModeValue);
        setOnlyWhileDictating(onlyWhileDictatingValue);
        setSounds(soundsValue);
        if (waveformValue) {
          setLocalWaveform({ ...DEFAULT_WAVEFORM_CONFIG, ...waveformValue });
        }
      })
      .catch((err) => {
        if (!disposed) notifyError("Couldn't load general settings", err);
      })
      .finally(() => {
        if (!disposed) setChecked(true);
      });

    // Follow changes made from the widget, Appearance, or the menu bar.
    const unlisten = listen<WidgetMode>("widget-mode-changed", (e) => {
      if (!disposed) setWidgetModeState(e.payload);
    });
    const unlistenWaveform = isWaveformControlled
      ? null
      : listen<WaveformConfig>("waveform-config-changed", (e) => {
          if (!disposed) setLocalWaveform({ ...DEFAULT_WAVEFORM_CONFIG, ...e.payload });
        });
    return () => {
      disposed = true;
      unlisten.then((f) => f()).catch(() => {});
      unlistenWaveform?.then((f) => f()).catch(() => {});
    };
  }, [isWaveformControlled]);

  function toggleAutostart(enabled: boolean) {
    setAutostart(enabled);
    invoke("set_autostart_enabled", { enabled }).catch((err) => {
      notifyError("set_autostart_enabled", err);
      setAutostart(!enabled);
    });
  }

  function toggleOnlyWhileDictating(enabled: boolean) {
    setOnlyWhileDictating(enabled);
    invoke("set_widget_only_while_dictating", { enabled }).catch((err) => {
      notifyError("set_widget_only_while_dictating", err);
      setOnlyWhileDictating(!enabled);
    });
  }

  function toggleSounds(enabled: boolean) {
    setSounds(enabled);
    invoke("set_dictation_sounds", { enabled }).catch((err) => {
      notifyError("set_dictation_sounds", err);
      setSounds(!enabled);
    });
  }

  function changeWidgetMode(newMode: WidgetMode) {
    const previous = widgetMode;
    setWidgetModeState(newMode);
    invoke("set_widget_mode", { mode: newMode }).catch((err) => {
      notifyError("set_widget_mode", err);
      setWidgetModeState(previous);
    });
  }

  const localWaveformSeqRef = useRef(0);

  function updateWaveform(patch: Partial<WaveformConfig>) {
    const next = { ...waveform, ...patch };
    if (onChangeWaveform) {
      onChangeWaveform(next);
      return;
    }
    const seq = ++localWaveformSeqRef.current;
    const previous = waveform;
    setLocalWaveform(next);
    invoke("set_waveform_config", { config: next }).catch((err) => {
      notifyError("set_waveform_config", err);
      if (seq === localWaveformSeqRef.current) {
        setLocalWaveform(previous);
      }
    });
  }

  const activeColorway = getWaveformColorway(waveform.colorway);
  const activeEngine =
    QUICK_WAVEFORM_ENGINES.find((eng) => eng.id === waveform.engine) ?? QUICK_WAVEFORM_ENGINES[0];

  function renderQuickColorwayOrb(id: WaveformColorwayId) {
    const cw = WAVEFORM_COLORWAYS[id];
    const active = waveform.colorway === id;
    const ringColor = id === "solid-white" ? "var(--dw-accent)" : cw.mid;
    return (
      <button
        key={id}
        type="button"
        onClick={() => updateWaveform({ colorway: id })}
        aria-label={cw.name}
        aria-pressed={active}
        title={`${cw.name} (${cw.kind === "spectrum" ? "Spectrum" : "Solid"})`}
        data-testid={`quick-waveform-colorway-${id}`}
        className="flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded-full transition-transform duration-150 hover:scale-110 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--dw-accent)]"
        style={{
          background: cw.swatchBg,
          border: active
            ? "1.5px solid var(--dw-fg)"
            : "1px solid color-mix(in srgb, var(--dw-fg) 26%, transparent)",
          boxShadow: active
            ? `0 0 0 2px var(--dw-bg-sunk), 0 0 0 3.5px ${ringColor}, 0 0 10px ${cw.peakGlow}`
            : undefined,
        }}
      />
    );
  }

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">General</h3>
      {!checked ? (
        <span className="loading loading-spinner loading-xs" />
      ) : (
        <div className="flex flex-col gap-3">
          <label className="flex items-start gap-2 text-xs">
            <input
              type="checkbox"
              className="checkbox checkbox-xs mt-0.5"
              checked={autostart}
              onChange={(e) => toggleAutostart(e.target.checked)}
              data-testid="open-at-login"
            />
            <span className="flex flex-col gap-0.5">
              Open at login
              <span className="text-[11px] opacity-60">
                Start Mouthpiece when you log in to your Mac, so dictation is ready right away.
              </span>
            </span>
          </label>
          <label className="flex items-start gap-2 text-xs">
            <input
              type="checkbox"
              className="checkbox checkbox-xs mt-0.5"
              checked={onlyWhileDictating}
              onChange={(e) => toggleOnlyWhileDictating(e.target.checked)}
            />
            <span className="flex flex-col gap-0.5">
              Show the widget only while dictating
              <span className="text-[11px] opacity-60">
                It appears when you press the shortcut and hides once your words are pasted. Show it any
                time from the menu bar icon.
              </span>
            </span>
          </label>
          <label className="flex items-start gap-2 text-xs">
            <input
              type="checkbox"
              className="checkbox checkbox-xs mt-0.5"
              checked={sounds}
              onChange={(e) => toggleSounds(e.target.checked)}
            />
            <span className="flex flex-col gap-0.5">
              Play sounds while dictating
              <span className="text-[11px] opacity-60">
                A short sound when recording starts, when it stops, and when your words are pasted (or
                something went wrong).
              </span>
            </span>
          </label>
          <div className="flex flex-col gap-1.5">
            <span className="text-xs">Widget style</span>
            <WidgetStylePicker value={widgetMode} onChange={changeWidgetMode} />
            <span className="text-[11px] opacity-60">
              Hover-popover styles open quick controls on cursor hover; Minimal, Pill, and Notch switch via
              right-click or the menu bar icon.
            </span>
          </div>

          <div
            className="mt-0.5 flex flex-col gap-2.5 rounded-xl px-3 py-2.5"
            style={{
              background: "var(--dw-bg-sunk)",
              border: "1px solid var(--dw-border)",
            }}
            data-testid="quick-waveform-strip"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex min-w-0 items-center gap-2">
                <span
                  className="h-3 w-3 shrink-0 rounded-full"
                  style={{
                    background: activeColorway.swatchBg,
                    border: "1px solid color-mix(in srgb, var(--dw-fg) 26%, transparent)",
                    boxShadow: waveform.phosphor_glow ? `0 0 8px ${activeColorway.peakGlow}` : undefined,
                  }}
                  aria-hidden="true"
                />
                <span className="text-[11.5px] font-semibold">Waveform &amp; Colors</span>
                <span className="truncate text-[10.5px] opacity-60">
                  {activeEngine.title} · {activeColorway.name}
                </span>
              </div>
              {onOpenAppearance && (
                <button
                  type="button"
                  onClick={onOpenAppearance}
                  data-testid="open-waveform-studio"
                  className="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded-full px-2.5 py-0.5 text-[10.5px] font-semibold transition-all hover:brightness-115 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--dw-accent)]"
                  style={{
                    background: "color-mix(in srgb, var(--dw-accent) 14%, transparent)",
                    border: "1px solid color-mix(in srgb, var(--dw-accent) 38%, transparent)",
                    color: "var(--dw-accent)",
                  }}
                >
                  Open Waveform Studio →
                </button>
              )}
            </div>

            <div className="flex flex-wrap items-center justify-between gap-2.5">
              <div
                className="inline-flex items-center gap-0.5 rounded-lg p-0.5"
                style={{
                  background: "var(--dw-card)",
                  border: "1px solid var(--dw-border)",
                }}
                role="group"
                aria-label="Waveform engine"
              >
                {QUICK_WAVEFORM_ENGINES.map((eng) => {
                  const active = waveform.engine === eng.id;
                  return (
                    <button
                      key={eng.id}
                      type="button"
                      onClick={() => updateWaveform({ engine: eng.id })}
                      aria-pressed={active}
                      title={eng.title}
                      data-testid={`quick-waveform-engine-${eng.id}`}
                      className="cursor-pointer rounded-md px-2 py-1 text-[11px] font-semibold transition-all focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[var(--dw-accent)]"
                      style={
                        active
                          ? {
                              background: "color-mix(in srgb, var(--dw-accent) 18%, var(--dw-card-raised))",
                              color: "var(--dw-fg)",
                              boxShadow:
                                "inset 0 0 0 1px color-mix(in srgb, var(--dw-accent) 55%, transparent)",
                            }
                          : {
                              color: "var(--dw-muted)",
                            }
                      }
                    >
                      {eng.label}
                    </button>
                  );
                })}
              </div>

              <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Waveform colorway">
                {SPECTRUM_COLORWAY_ORDER.map(renderQuickColorwayOrb)}
                <span
                  className="mx-0.5 h-3.5 w-px"
                  style={{ background: "var(--dw-border-strong)" }}
                  aria-hidden="true"
                />
                {SOLID_COLORWAY_ORDER.map(renderQuickColorwayOrb)}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/// Punctuation words that insert a literal character — kept to one
/// canonical phrase per symbol (`punctuation.rs`'s `COMMANDS` table has
/// more aliases than are useful to show here, e.g. "full stop" alongside
/// "period"). Order matches roughly how often each comes up dictating
/// code/prose, not alphabetical.
export const PUNCTUATION_COMMANDS: [string, string][] = [
  ["period", "."],
  ["comma", ","],
  ["question mark", "?"],
  ["exclamation point", "!"],
  ["colon", ":"],
  ["semicolon", ";"],
  ["new line", "↵"],
  ["new paragraph", "↵↵"],
  ["open paren", "("],
  ["close paren", ")"],
  ["dash", "–"],
  ["em dash", "—"],
  ["underscore", "_"],
  ["slash", "/"],
  ["backslash", "\\"],
  ["apostrophe", "'"],
  ["quotation mark", "“ ” (paired)"],
  ["asterisk", "*"],
  ["ampersand", "&"],
  ["at sign", "@"],
  ["hashtag", "#"],
  ["plus", "+"],
  ["minus", "-"],
  ["equals", "="],
  ["tilde", "~"],
  ["percent sign", "%"],
  ["degree sign", "°"],
  ["copyright", "©"],
  ["trademark", "™"],
];

export type VoiceCommandGroup = {
  title: string;
  description: string;
  examples: { say: string; get: string }[];
};

/// Cross-cutting trigger phrases: work in any app, regardless of Mode, and
/// resolve instantly without waiting on the LLM (except Boilerplate, which
/// is the one group that needs Ollama). Kept as one reference list so a
/// user hearing "why did my dictation turn into a checklist" or "why did
/// that get pasted in ALL_CAPS" has one place to check rather than having
/// to ask.
export const VOICE_COMMAND_GROUPS: VoiceCommandGroup[] = [
  {
    title: "Casing & syntax",
    description: "Say a case name, then what to convert. Instant, no LLM involved.",
    examples: [
      { say: '"snake case error response handler"', get: "error_response_handler" },
      { say: '"camel case error response handler"', get: "errorResponseHandler" },
      { say: '"pascal case error response handler"', get: "ErrorResponseHandler" },
      { say: '"kebab case error response handler"', get: "error-response-handler" },
      { say: '"title case error response handler"', get: "Error Response Handler" },
      { say: '"screaming snake case max retry count"', get: "MAX_RETRY_COUNT" },
    ],
  },
  {
    title: "Numbered lists",
    description: 'Count out loud ("one... two... three...", or "first... second...") and it becomes a real list.',
    examples: [{ say: '"one set up the repo two install deps three run tests"', get: "1. set up the repo\n2. install deps\n3. run tests" }],
  },
  {
    title: "Self-correction (Backtrack)",
    description: 'Correct yourself mid-dictation instead of stopping and restarting — everything before the trigger is discarded.',
    examples: [
      { say: '"call the client at three scratch that four"', get: "four" },
      { say: '"let\'s do coffee at two, actually three"', get: "three" },
    ],
  },
  {
    title: "Boilerplate generation",
    description: 'Say one of these, then describe what to generate. Sent to your local LLM instead of pasted as text — needs Ollama running.',
    examples: [
      {
        say: '"generate boilerplate for a React component called UserCard with name and avatar props"',
        get: "the generated component, pasted directly",
      },
    ],
  },
  {
    title: "Snippets",
    description: 'Speak a saved trigger phrase on its own and it expands to its full saved body. Manage the list from Vocabulary → Snippets.',
    examples: [{ say: '"pr checklist"', get: "the saved PR checklist template, pasted directly" }],
  },
  {
    title: "Press enter",
    description:
      'Say this at the end to send Enter after pasting. "Send it" (or "submit it") does the same, but only after a pause or period, so "…and send it" in a sentence stays text. Off by default — enable it in General above.',
    examples: [
      { say: '"looks good press enter"', get: 'pastes "looks good", then Enter' },
      { say: '"fix the failing test. send it."', get: 'pastes "Fix the failing test.", then Enter' },
    ],
  },
  {
    title: "Attach the error",
    description:
      'Say this at the end to append the latest error output as a code block: the visible text of the terminal you\'re in (Terminal, iTerm), otherwise the clipboard if it holds an error. "With the error" works after a pause. If nothing is found, a note appears and just your words are pasted.',
    examples: [{ say: '"why does this test fail, attach the error"', get: "your sentence, then the stack trace in a ``` block" }],
  },
  {
    title: "Scratch that (undo)",
    description:
      'Say it on its own to take back the last paste: Cmd+Z in most apps; in a terminal, exactly as many backspaces as were pasted (single line, within 15 seconds, not sent). Anywhere else it does nothing and says so. Mid-sentence it is still a correction.',
    examples: [{ say: '"scratch that"', get: "the last pasted text is removed" }],
  },
  {
    title: "Append clipboard",
    description: 'Say this at the end to append whatever you last copied — a stack trace, a URL, a snippet — without reading it aloud.',
    examples: [{ say: '"here\'s the error append clipboard"', get: "here's the error <clipboard contents>" }],
  },
];

export function VoiceCommandsSection() {
  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-1 text-xs font-semibold uppercase tracking-wide opacity-60">Voice Commands</h3>
      <p className="mb-3 text-xs opacity-60">
        Trigger phrases recognized in any app, in any Mode — if dictation ever came out looking unexpectedly
        transformed, check here first.
      </p>
      <div className="flex flex-col gap-4">
        {VOICE_COMMAND_GROUPS.map((group) => (
          <div key={group.title}>
            <p className="text-xs font-medium">{group.title}</p>
            <p className="mb-1.5 text-xs opacity-60">{group.description}</p>
            <div className="flex flex-col gap-1">
              {group.examples.map((ex) => (
                <p key={ex.say} className="text-xs">
                  <code className="opacity-80">{ex.say}</code>
                  <span className="opacity-50"> {"→"} </span>
                  <span className="opacity-80">{ex.get}</span>
                </p>
              ))}
            </div>
          </div>
        ))}
        <div>
          <p className="text-xs font-medium">Punctuation & symbols</p>
          <p className="mb-1.5 text-xs opacity-60">Say the word, get the character — works mid-sentence, not just at the end.</p>
          <div className="grid grid-cols-2 gap-x-3 gap-y-1">
            {PUNCTUATION_COMMANDS.map(([word, symbol]) => (
              <div key={word} className="flex items-center justify-between text-xs">
                <code className="opacity-80">{word}</code>
                <span className="opacity-60">{symbol}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
