export type WaveformEngineId = "precision" | "silk" | "matrix" | "hyperdom";

export type HyperLayoutMode = "center" | "scroll";

export type WaveformColorwayId =
  | "ember"
  | "aurora"
  | "mint"
  | "arctic"
  | "titanium"
  | "solid-red"
  | "solid-white"
  | "solid-amber"
  | "solid-emerald"
  | "solid-cyan";

export type WaveformConfig = {
  engine: WaveformEngineId;
  colorway: WaveformColorwayId;
  needle_bars: number;
  sensitivity: number;
  snap_ms: number;
  needle_width: number;
  hyper_mode: HyperLayoutMode;
  phosphor_glow: boolean;
  show_in_flyout: boolean;
};

export const DEFAULT_WAVEFORM_CONFIG: WaveformConfig = {
  engine: "precision",
  colorway: "ember",
  needle_bars: 56,
  sensitivity: 1.35,
  snap_ms: 36,
  needle_width: 2.0,
  hyper_mode: "center",
  phosphor_glow: true,
  show_in_flyout: true,
};

export type NeedlePresetId = "clean" | "balanced" | "dense" | "ultra";

export type NeedlePreset = {
  label: string;
  needle_bars: number;
  sensitivity: number;
  snap_ms: number;
  needle_width: number;
};

export const NEEDLE_PRESETS: Record<NeedlePresetId, NeedlePreset> = {
  clean: {
    label: "Clean",
    needle_bars: 32,
    sensitivity: 1.25,
    snap_ms: 44,
    needle_width: 2.5,
  },
  balanced: {
    label: "Balanced",
    needle_bars: 56,
    sensitivity: 1.35,
    snap_ms: 36,
    needle_width: 2.0,
  },
  dense: {
    label: "Dense",
    needle_bars: 88,
    sensitivity: 1.5,
    snap_ms: 28,
    needle_width: 1.5,
  },
  ultra: {
    label: "Ultra",
    needle_bars: 136,
    sensitivity: 1.65,
    snap_ms: 22,
    needle_width: 1.5,
  },
};

export type WaveformColorwayDef = {
  id: WaveformColorwayId;
  name: string;
  kind: "spectrum" | "solid";
  outer: string;
  mid: string;
  bright: string;
  core: string;
  secondary: string;
  silkFill1: string;
  silkFill2: string;
  peakGlow: string;
  swatchBg: string;
  needleGrad: string;
  needleMidGrad: string;
};

export const WAVEFORM_COLORWAYS: Record<WaveformColorwayId, WaveformColorwayDef> = {
  // Spectrum Gradients
  ember: {
    id: "ember",
    name: "Solar Ember",
    kind: "spectrum",
    outer: "#d9483b",
    mid: "#f97316",
    bright: "#fbbf24",
    core: "#fef3c7",
    secondary: "#38bdf8",
    silkFill1: "rgba(249, 115, 22, 0.36)",
    silkFill2: "rgba(251, 191, 36, 0.24)",
    peakGlow: "rgba(251, 146, 60, 0.72)",
    swatchBg: "linear-gradient(135deg, #d9483b 0%, #f97316 52%, #fbbf24 100%)",
    needleGrad: "linear-gradient(180deg, #fbbf24 0%, #f97316 50%, #d9483b 100%)",
    needleMidGrad: "linear-gradient(180deg, #fef3c7 0%, #fbbf24 45%, #f97316 100%)",
  },
  aurora: {
    id: "aurora",
    name: "Plasma Aurora",
    kind: "spectrum",
    outer: "#7c3aed",
    mid: "#d946ef",
    bright: "#22d3ee",
    core: "#f5f3ff",
    secondary: "#fbbf24",
    silkFill1: "rgba(217, 70, 239, 0.34)",
    silkFill2: "rgba(34, 211, 238, 0.26)",
    peakGlow: "rgba(217, 70, 239, 0.72)",
    swatchBg: "linear-gradient(135deg, #7c3aed 0%, #d946ef 50%, #22d3ee 100%)",
    needleGrad: "linear-gradient(180deg, #22d3ee 0%, #d946ef 52%, #7c3aed 100%)",
    needleMidGrad: "linear-gradient(180deg, #f5f3ff 0%, #22d3ee 48%, #d946ef 100%)",
  },
  mint: {
    id: "mint",
    name: "Cyber Mint",
    kind: "spectrum",
    outer: "#0d9488",
    mid: "#10b981",
    bright: "#34d399",
    core: "#d1fae5",
    secondary: "#a78bfa",
    silkFill1: "rgba(16, 185, 129, 0.34)",
    silkFill2: "rgba(52, 211, 153, 0.24)",
    peakGlow: "rgba(52, 211, 153, 0.72)",
    swatchBg: "linear-gradient(135deg, #0d9488 0%, #10b981 52%, #5eead4 100%)",
    needleGrad: "linear-gradient(180deg, #5eead4 0%, #10b981 50%, #0d9488 100%)",
    needleMidGrad: "linear-gradient(180deg, #d1fae5 0%, #34d399 48%, #10b981 100%)",
  },
  arctic: {
    id: "arctic",
    name: "Arctic Cobalt",
    kind: "spectrum",
    outer: "#2563eb",
    mid: "#38bdf8",
    bright: "#7dd3fc",
    core: "#f0f9ff",
    secondary: "#fb7185",
    silkFill1: "rgba(56, 189, 248, 0.34)",
    silkFill2: "rgba(125, 211, 252, 0.24)",
    peakGlow: "rgba(56, 189, 248, 0.72)",
    swatchBg: "linear-gradient(135deg, #2563eb 0%, #38bdf8 54%, #bae6fd 100%)",
    needleGrad: "linear-gradient(180deg, #bae6fd 0%, #38bdf8 50%, #2563eb 100%)",
    needleMidGrad: "linear-gradient(180deg, #f0f9ff 0%, #7dd3fc 48%, #38bdf8 100%)",
  },
  titanium: {
    id: "titanium",
    name: "Titanium Rams",
    kind: "spectrum",
    outer: "#71717a",
    mid: "#d4d4d8",
    bright: "#f4f4f5",
    core: "#f97316",
    secondary: "#94a3b8",
    silkFill1: "rgba(212, 212, 216, 0.30)",
    silkFill2: "rgba(249, 115, 22, 0.22)",
    peakGlow: "rgba(249, 115, 22, 0.68)",
    swatchBg: "linear-gradient(135deg, #52525b 0%, #d4d4d8 62%, #f97316 100%)",
    needleGrad: "linear-gradient(180deg, #f4f4f5 0%, #a1a1aa 55%, #52525b 100%)",
    needleMidGrad: "linear-gradient(180deg, #f97316 0%, #f4f4f5 48%, #a1a1aa 100%)",
  },

  // Solid Colors
  "solid-red": {
    id: "solid-red",
    name: "Classic Red",
    kind: "solid",
    outer: "#ef5b5f",
    mid: "#ef5b5f",
    bright: "#ef5b5f",
    core: "#ff9ea1",
    secondary: "#38bdf8",
    silkFill1: "rgba(239, 91, 95, 0.34)",
    silkFill2: "rgba(239, 91, 95, 0.18)",
    peakGlow: "rgba(239, 91, 95, 0.65)",
    swatchBg: "#ef5b5f",
    needleGrad: "#ef5b5f",
    needleMidGrad: "#ef5b5f",
  },
  "solid-white": {
    id: "solid-white",
    name: "Pure White",
    kind: "solid",
    outer: "#f8fafc",
    mid: "#f8fafc",
    bright: "#f8fafc",
    core: "#ffffff",
    secondary: "#94a3b8",
    silkFill1: "rgba(248, 250, 252, 0.30)",
    silkFill2: "rgba(248, 250, 252, 0.16)",
    peakGlow: "rgba(248, 250, 252, 0.60)",
    swatchBg: "#f8fafc",
    needleGrad: "#f8fafc",
    needleMidGrad: "#f8fafc",
  },
  "solid-amber": {
    id: "solid-amber",
    name: "Studio Amber",
    kind: "solid",
    outer: "#f59e0b",
    mid: "#f59e0b",
    bright: "#f59e0b",
    core: "#fcd34d",
    secondary: "#38bdf8",
    silkFill1: "rgba(245, 158, 11, 0.32)",
    silkFill2: "rgba(245, 158, 11, 0.18)",
    peakGlow: "rgba(245, 158, 11, 0.65)",
    swatchBg: "#f59e0b",
    needleGrad: "#f59e0b",
    needleMidGrad: "#f59e0b",
  },
  "solid-emerald": {
    id: "solid-emerald",
    name: "Signal Emerald",
    kind: "solid",
    outer: "#10b981",
    mid: "#10b981",
    bright: "#10b981",
    core: "#6ee7b7",
    secondary: "#a78bfa",
    silkFill1: "rgba(16, 185, 129, 0.32)",
    silkFill2: "rgba(16, 185, 129, 0.18)",
    peakGlow: "rgba(16, 185, 129, 0.65)",
    swatchBg: "#10b981",
    needleGrad: "#10b981",
    needleMidGrad: "#10b981",
  },
  "solid-cyan": {
    id: "solid-cyan",
    name: "Electric Cyan",
    kind: "solid",
    outer: "#22d3ee",
    mid: "#22d3ee",
    bright: "#22d3ee",
    core: "#a5f3fc",
    secondary: "#fb923c",
    silkFill1: "rgba(34, 211, 238, 0.32)",
    silkFill2: "rgba(34, 211, 238, 0.18)",
    peakGlow: "rgba(34, 211, 238, 0.65)",
    swatchBg: "#22d3ee",
    needleGrad: "#22d3ee",
    needleMidGrad: "#22d3ee",
  },
};

export const SPECTRUM_COLORWAY_ORDER: WaveformColorwayId[] = [
  "ember",
  "aurora",
  "mint",
  "arctic",
  "titanium",
];

export const SOLID_COLORWAY_ORDER: WaveformColorwayId[] = [
  "solid-red",
  "solid-white",
  "solid-amber",
  "solid-emerald",
  "solid-cyan",
];

export function getWaveformColorway(id?: string): WaveformColorwayDef {
  if (id && Object.prototype.hasOwnProperty.call(WAVEFORM_COLORWAYS, id)) {
    return WAVEFORM_COLORWAYS[id as WaveformColorwayId];
  }
  return WAVEFORM_COLORWAYS[DEFAULT_WAVEFORM_CONFIG.colorway];
}

export type WaveformEngineMeta = {
  id: WaveformEngineId;
  num: string;
  title: string;
  shortLabel: string;
  desc: string;
  badge: string;
};

export const WAVEFORM_ENGINE_META: WaveformEngineMeta[] = [
  {
    id: "precision",
    num: "01",
    title: "Studio Bars",
    shortLabel: "Precision",
    desc: "Asymmetric attack/decay bars with floating peak-hold caps and VAD color gating.",
    badge: "Studio",
  },
  {
    id: "silk",
    num: "02",
    title: "Silk Ribbon",
    shortLabel: "Silk",
    desc: "Dual overlapping fluid Bezier ribbons with harmonic motion and phosphor sheen.",
    badge: "Organic",
  },
  {
    id: "matrix",
    num: "03",
    title: "LED Matrix",
    shortLabel: "Matrix",
    desc: "OP-1 inspired quantized LED dot matrix with illuminated peak cells.",
    badge: "Hardware",
  },
  {
    id: "hyperdom",
    num: "04",
    title: "Kinetic Needles",
    shortLabel: "Hyper",
    desc: "Ultra-snappy tunable needle array with center symmetry or scrolling tape.",
    badge: "Tunable",
  },
];
