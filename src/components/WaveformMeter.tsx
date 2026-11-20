import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  DEFAULT_WAVEFORM_CONFIG,
  getWaveformColorway,
  type HyperLayoutMode,
  type WaveformColorwayDef,
  type WaveformConfig,
} from "../waveformConfig";

export type AudioBands = { low: number; mid: number; high: number };

export type WaveformMeterProps = {
  /** Chronological audio levels 0..1 (oldest first, newest last), e.g. from audio-level events */
  levels: number[];
  /** Optional remote/system audio levels 0..1 for split-axis Meeting Mode */
  remoteLevels?: number[];
  /** Optional 3-band spectral energy (low/mid/high 0..1) from audio-bands events */
  bands?: AudioBands;
  /** Split-axis dual-channel mode (Top: local mic, Bottom: remote system audio) */
  dualChannel?: boolean;
  /** Waveform visual configuration */
  config?: WaveformConfig;
  /** Compact mode (for small floating HUD widgets vs large Settings preview) */
  compact?: boolean;
  /** Optional simulated demo animation when no live audio is flowing (used in Settings live preview) */
  demoMode?: boolean;
  className?: string;
  testId?: string;
};

const VAD_GATE = 0.055;
const MAX_BUFFER_LEN = 256;

function clamp01(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return 0;
  if (v >= 1) return 1;
  return v;
}

function applySensitivity(raw: number, sensitivity: number): number {
  const clamped = clamp01(raw);
  if (clamped <= 0.002) return 0;
  const scaled = clamped * sensitivity;
  if (scaled <= 0.85) return scaled;
  return Math.min(1, 0.85 + (1 - Math.exp(-(scaled - 0.85) * 3.2)) * 0.15);
}

function hasLiveAudio(levels?: number[]): boolean {
  if (!levels || levels.length === 0) return false;
  for (let i = levels.length - 1; i >= 0; i--) {
    const v = levels[i];
    if (Number.isFinite(v) && v > 0.015) return true;
  }
  return false;
}

/**
 * Synthesizes realistic syllabic speech cadence for Settings live preview
 * when no live microphone audio is flowing.
 */
function demoSpeechEnvelope(t: number, seed: number): number {
  const phrase = 0.5 + 0.5 * Math.sin(t * 1.15 + seed);
  const breathGate = phrase > 0.16 ? 1 : Math.max(0, (phrase - 0.05) / 0.11);
  const word = 0.52 + 0.48 * Math.sin(t * 3.4 + seed * 1.7 + Math.sin(t * 1.3 + seed) * 1.1);
  const syllable = 0.5 + 0.5 * Math.sin(t * 8.9 + seed * 2.3 + Math.cos(t * 2.7) * 0.8);
  const crisp = 0.5 + 0.5 * Math.sin(t * 15.2 + seed * 0.9);
  const raw = breathGate * (0.18 + 0.54 * word * (0.45 + 0.55 * syllable) + 0.18 * crisp * syllable);
  return clamp01(raw);
}

function fillTargets(
  levels: number[] | undefined,
  count: number,
  mode: HyperLayoutMode,
  sensitivity: number,
  timeSec: number,
  useDemo: boolean,
  demoSeed: number,
  out: Float32Array,
): void {
  const safeCount = Math.max(1, Math.min(count, out.length));
  const mid = (safeCount - 1) * 0.5;

  if (useDemo) {
    for (let i = 0; i < safeCount; i++) {
      if (mode === "center") {
        const dist = mid > 0 ? Math.abs(i - mid) / mid : 0;
        const tSample = timeSec - dist * 0.42;
        const base = demoSpeechEnvelope(tSample, demoSeed);
        const ripple = 0.78 + 0.22 * Math.sin(i * 0.9 + timeSec * 6.4 + demoSeed);
        const windowEnv = 0.26 + 0.74 * Math.cos(dist * Math.PI * 0.48);
        out[i] = applySensitivity(base * ripple * windowEnv, sensitivity);
      } else {
        const ageNorm = safeCount > 1 ? (safeCount - 1 - i) / (safeCount - 1) : 0;
        const tSample = timeSec - ageNorm * 2.2;
        const base = demoSpeechEnvelope(tSample, demoSeed);
        const micro = 0.88 + 0.12 * Math.sin(i * 1.35 - timeSec * 4.5 + demoSeed);
        out[i] = applySensitivity(base * micro, sensitivity);
      }
    }
    return;
  }

  if (!levels || levels.length === 0) {
    out.fill(0, 0, safeCount);
    return;
  }

  const len = levels.length;

  if (mode === "center") {
    // Map newest audio energy at the center, radiating outward symmetrically
    const span = Math.min(len, Math.max(10, Math.ceil(safeCount * 0.5)));
    for (let i = 0; i < safeCount; i++) {
      const dist = mid > 0 ? Math.abs(i - mid) / mid : 0;
      const offsetFromNewest = dist * (span - 1);
      const exactIdx = len - 1 - offsetFromNewest;
      const idx0 = Math.max(0, Math.min(len - 1, Math.floor(exactIdx)));
      const idx1 = Math.max(0, Math.min(len - 1, Math.ceil(exactIdx)));
      const frac = exactIdx - Math.floor(exactIdx);
      const raw = levels[idx0] * (1 - frac) + levels[idx1] * frac;
      const windowEnv = 0.34 + 0.66 * Math.cos(dist * Math.PI * 0.46);
      const ripple =
        raw > 0.01 ? 0.84 + 0.16 * Math.sin(i * 1.15 + timeSec * 7.2 + demoSeed) : 1;
      out[i] = applySensitivity(raw * windowEnv * ripple, sensitivity);
    }
    return;
  }

  // Chronological "scroll" tape: oldest on left (0), newest on right (safeCount - 1)
  if (len >= safeCount) {
    const start = len - safeCount;
    for (let i = 0; i < safeCount; i++) {
      out[i] = applySensitivity(levels[start + i], sensitivity);
    }
  } else {
    const denom = Math.max(1, safeCount - 1);
    for (let i = 0; i < safeCount; i++) {
      const pos = (i / denom) * (len - 1);
      const idx0 = Math.floor(pos);
      const idx1 = Math.min(len - 1, idx0 + 1);
      const frac = pos - idx0;
      const raw = levels[idx0] * (1 - frac) + levels[idx1] * frac;
      out[i] = applySensitivity(raw, sensitivity);
    }
  }
}

type BallisticsState = {
  smooth: Float32Array;
  peak: Float32Array;
  peakHold: Float32Array;
};

function createBallistics(): BallisticsState {
  return {
    smooth: new Float32Array(MAX_BUFFER_LEN),
    peak: new Float32Array(MAX_BUFFER_LEN),
    peakHold: new Float32Array(MAX_BUFFER_LEN),
  };
}

function updateBallistics(
  state: BallisticsState,
  targets: Float32Array,
  count: number,
  dt: number,
): void {
  const alphaAttack = 1 - Math.exp(-dt / 0.022);
  const alphaDecay = 1 - Math.exp(-dt / 0.095);
  const safeCount = Math.min(count, MAX_BUFFER_LEN);

  for (let i = 0; i < safeCount; i++) {
    const target = targets[i];
    const curr = state.smooth[i];
    const next =
      target > curr
        ? curr + (target - curr) * alphaAttack
        : curr + (target - curr) * alphaDecay;
    state.smooth[i] = next < 0.001 ? 0 : next;

    if (state.smooth[i] >= state.peak[i]) {
      state.peak[i] = state.smooth[i];
      state.peakHold[i] = 0.24;
    } else if (state.peakHold[i] > 0) {
      state.peakHold[i] -= dt;
    } else {
      state.peak[i] = Math.max(state.smooth[i], state.peak[i] - dt * 0.92);
    }
  }
}

function drawRoundedRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
): void {
  if (w <= 0 || h <= 0) return;
  const radius = Math.min(r, w * 0.5, h * 0.5);
  if (typeof ctx.roundRect === "function") {
    ctx.beginPath();
    ctx.roundRect(x, y, w, h, radius);
    ctx.fill();
    return;
  }
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.lineTo(x + w - radius, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + radius);
  ctx.lineTo(x + w, y + h - radius);
  ctx.quadraticCurveTo(x + w, y + h, x + w - radius, y + h);
  ctx.lineTo(x + radius, y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - radius);
  ctx.lineTo(x, y + radius);
  ctx.quadraticCurveTo(x, y, x + radius, y);
  ctx.closePath();
  ctx.fill();
}

function drawPrecisionCanvas(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  local: BallisticsState,
  remote: BallisticsState,
  cols: number,
  dualChannel: boolean,
  compact: boolean,
  colorway: WaveformColorwayDef,
  glow: boolean,
  bands: AudioBands | null,
): void {
  const barW = compact ? 2.5 : 3.5;
  const gap = compact ? 2 : 3;
  const totalW = cols * barW + (cols - 1) * gap;
  const startX = (w - totalW) * 0.5;
  const midY = h * 0.5;
  const padY = compact ? 2 : 5;
  const maxHalfH = Math.max(4, midY - padY - (dualChannel ? 2 : 0));
  const capH = compact ? 1.5 : 2;
  const radius = barW * 0.5;
  const bandMean = bands ? Math.max(0.04, (bands.low + bands.mid + bands.high) / 3) : 1;

  if (dualChannel) {
    ctx.save();
    ctx.strokeStyle = "rgba(148, 163, 184, 0.22)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(startX, midY);
    ctx.lineTo(startX + totalW, midY);
    ctx.stroke();
    ctx.restore();
  }

  let primaryGradUp: CanvasGradient | string = colorway.mid;
  let primaryGradSym: CanvasGradient | string = colorway.mid;
  if (colorway.kind === "spectrum") {
    const up = ctx.createLinearGradient(0, midY, 0, padY);
    up.addColorStop(0, colorway.outer);
    up.addColorStop(0.45, colorway.mid);
    up.addColorStop(0.82, colorway.bright);
    up.addColorStop(1, colorway.core);
    primaryGradUp = up;

    const sym = ctx.createLinearGradient(0, padY, 0, h - padY);
    sym.addColorStop(0, colorway.core);
    sym.addColorStop(0.18, colorway.bright);
    sym.addColorStop(0.42, colorway.mid);
    sym.addColorStop(0.5, colorway.outer);
    sym.addColorStop(0.58, colorway.mid);
    sym.addColorStop(0.82, colorway.bright);
    sym.addColorStop(1, colorway.core);
    primaryGradSym = sym;
  }

  for (let i = 0; i < cols; i++) {
    const x = startX + i * (barW + gap);
    const ageAlpha = 0.36 + 0.64 * ((i + 1) / cols);
    let lv = local.smooth[i];
    let pk = local.peak[i];
    let sibilantSheen = 0;

    if (bands && lv > 0.004) {
      const u = cols > 1 ? i / (cols - 1) : 0.5;
      const edgeOuter = Math.abs(u - 0.5) * 2;
      // Low weights left/outer bass energy, mid anchors the vocal core, high adds crisp sibilant sheen
      const wLow = Math.pow(Math.max(0, 1 - u * 1.45), 1.35) * 0.75 + Math.pow(edgeOuter, 1.8) * 0.25;
      const wMid = Math.exp(-Math.pow((u - 0.48) / 0.3, 2));
      const wHigh = Math.pow(Math.max(0, (u - 0.38) / 0.62), 1.35);
      const wSum = wLow + wMid + wHigh || 1;
      const bandEnergy = (wLow * bands.low + wMid * bands.mid + wHigh * bands.high) / wSum;
      const rel = Math.max(0.62, Math.min(1.48, bandEnergy / bandMean));
      const weight = 0.72 + 0.28 * rel;
      lv = clamp01(lv * weight + bandEnergy * 0.09);
      pk = Math.max(lv, clamp01(pk * weight + bandEnergy * 0.07));
      sibilantSheen = wHigh * bands.high;
    }

    const isVoice = lv >= VAD_GATE;

    if (dualChannel) {
      // Top channel: local microphone
      const barH = isVoice ? Math.max(3, lv * maxHalfH) : compact ? 2 : 2.5;
      ctx.save();
      ctx.globalAlpha = isVoice ? (colorway.kind === "solid" ? ageAlpha : 0.45 + 0.55 * ageAlpha) : 0.22 * ageAlpha;
      ctx.fillStyle = isVoice ? primaryGradUp : colorway.outer;
      if (glow && isVoice) {
        ctx.shadowColor = colorway.peakGlow;
        ctx.shadowBlur = (compact ? 5 : 10) + sibilantSheen * (compact ? 2.5 : 5);
      }
      drawRoundedRect(ctx, x, midY - 2 - barH, barW, barH, radius);
      ctx.restore();

      if (pk >= VAD_GATE && pk > lv + 0.03) {
        const capY = midY - 2 - Math.min(maxHalfH, pk * maxHalfH) - capH;
        ctx.save();
        ctx.globalAlpha = Math.min(1, 0.55 + 0.45 * ageAlpha + sibilantSheen * 0.22);
        ctx.fillStyle = colorway.core;
        if (glow) {
          ctx.shadowColor = colorway.peakGlow;
          ctx.shadowBlur = (compact ? 4 : 8) + sibilantSheen * (compact ? 2 : 4);
        }
        drawRoundedRect(ctx, x, capY, barW, capH, capH * 0.5);
        ctx.restore();
      }

      // Bottom channel: remote / system audio
      const rv = remote.smooth[i];
      const rpk = remote.peak[i];
      const isRemoteVoice = rv >= VAD_GATE;
      const rBarH = isRemoteVoice ? Math.max(3, rv * maxHalfH) : compact ? 2 : 2.5;

      ctx.save();
      ctx.globalAlpha = isRemoteVoice ? 0.38 + 0.62 * ageAlpha : 0.2 * ageAlpha;
      ctx.fillStyle = colorway.secondary;
      if (glow && isRemoteVoice) {
        ctx.shadowColor = colorway.secondary;
        ctx.shadowBlur = compact ? 5 : 9;
      }
      drawRoundedRect(ctx, x, midY + 2, barW, rBarH, radius);
      ctx.restore();

      if (rpk >= VAD_GATE && rpk > rv + 0.03) {
        const rCapY = midY + 2 + Math.min(maxHalfH, rpk * maxHalfH);
        ctx.save();
        ctx.globalAlpha = 0.5 + 0.5 * ageAlpha;
        ctx.fillStyle = colorway.core;
        drawRoundedRect(ctx, x, rCapY, barW, capH, capH * 0.5);
        ctx.restore();
      }
    } else {
      // Single-channel symmetrical studio bars
      const halfH = isVoice ? Math.max(2.5, lv * maxHalfH) : compact ? 1.5 : 2;
      ctx.save();
      ctx.globalAlpha = isVoice ? (colorway.kind === "solid" ? ageAlpha : 0.45 + 0.55 * ageAlpha) : 0.22 * ageAlpha;
      ctx.fillStyle = isVoice ? primaryGradSym : colorway.outer;
      if (glow && isVoice) {
        ctx.shadowColor = colorway.peakGlow;
        ctx.shadowBlur = (compact ? 5 : 11) + sibilantSheen * (compact ? 2.5 : 5);
      }
      drawRoundedRect(ctx, x, midY - halfH, barW, halfH * 2, radius);
      ctx.restore();

      // Symmetrical peak-hold caps
      if (pk >= VAD_GATE && pk > lv + 0.035) {
        const peakHalf = Math.min(maxHalfH, pk * maxHalfH);
        ctx.save();
        ctx.globalAlpha = Math.min(1, 0.55 + 0.45 * ageAlpha + sibilantSheen * 0.22);
        ctx.fillStyle = colorway.core;
        if (glow) {
          ctx.shadowColor = colorway.peakGlow;
          ctx.shadowBlur = (compact ? 4 : 8) + sibilantSheen * (compact ? 2 : 4);
        }
        drawRoundedRect(ctx, x, midY - peakHalf - capH, barW, capH, capH * 0.5);
        drawRoundedRect(ctx, x, midY + peakHalf, barW, capH, capH * 0.5);
        ctx.restore();
      }
    }
  }
}

function traceSmoothCurve(
  ctx: CanvasRenderingContext2D,
  xs: number[],
  ys: number[],
  moveToFirst: boolean,
): void {
  const n = xs.length;
  if (n === 0) return;
  if (moveToFirst) {
    ctx.moveTo(xs[0], ys[0]);
  } else {
    ctx.lineTo(xs[0], ys[0]);
  }
  if (n === 1) return;
  for (let i = 0; i < n - 1; i++) {
    const mx = (xs[i] + xs[i + 1]) * 0.5;
    const my = (ys[i] + ys[i + 1]) * 0.5;
    ctx.quadraticCurveTo(xs[i], ys[i], mx, my);
  }
  ctx.lineTo(xs[n - 1], ys[n - 1]);
}

function drawSilkCanvas(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  local: BallisticsState,
  remote: BallisticsState,
  points: number,
  timeSec: number,
  harmonicPhase: number,
  dualChannel: boolean,
  compact: boolean,
  colorway: WaveformColorwayDef,
  glow: boolean,
  bands: AudioBands | null,
  sensitivity: number,
): void {
  const midY = h * 0.5;
  const padX = compact ? 2 : 6;
  const padY = compact ? 2 : 6;
  const usableW = Math.max(4, w - padX * 2);
  const maxAmp = Math.max(4, midY - padY);

  // Delicate baseline horizon
  ctx.save();
  ctx.strokeStyle = "rgba(148, 163, 184, 0.18)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(padX, midY);
  ctx.lineTo(w - padX, midY);
  ctx.stroke();
  ctx.restore();

  const xs: number[] = new Array(points);
  const up1: number[] = new Array(points);
  const down1: number[] = new Array(points);
  const up2: number[] = new Array(points);
  const down2: number[] = new Array(points);

  const primaryBandAmp = bands
    ? applySensitivity(bands.low * 0.45 + bands.mid * 0.55, sensitivity)
    : 0;
  const harmonicBandAmp = bands
    ? applySensitivity(bands.high * 0.65 + bands.mid * 0.35, sensitivity)
    : 0;

  for (let i = 0; i < points; i++) {
    const u = points > 1 ? i / (points - 1) : 0.5;
    const edgeWindow = Math.sin(u * Math.PI);
    const smoothEdge = Math.pow(Math.max(0, edgeWindow), 0.55);
    const lv = local.smooth[i];
    const carrier1 = 0.78 + 0.22 * Math.sin(u * Math.PI * 3.5 - timeSec * 4.2);
    const carrier2 = 0.72 + 0.28 * Math.cos(u * Math.PI * 5.0 + harmonicPhase);
    let amp1: number;
    let amp2: number;
    if (bands) {
      const primaryLevel = lv > 0.002 || primaryBandAmp > 0.005 ? clamp01(lv * 0.48 + primaryBandAmp * 0.58) : 0;
      const harmonicLevel = lv > 0.002 || harmonicBandAmp > 0.005 ? clamp01(lv * 0.32 + harmonicBandAmp * 0.72) : 0;
      amp1 = primaryLevel * maxAmp * smoothEdge * carrier1;
      amp2 = harmonicLevel * maxAmp * smoothEdge * 0.82 * carrier2;
    } else {
      amp1 = lv * maxAmp * smoothEdge * carrier1;
      amp2 = lv * maxAmp * smoothEdge * 0.76 * carrier2;
    }

    xs[i] = padX + u * usableW;
    up1[i] = midY - amp1;
    up2[i] = midY - amp2;

    if (dualChannel) {
      const rv = remote.smooth[i];
      const rAmp1 = rv * maxAmp * smoothEdge * carrier1;
      const rAmp2 = rv * maxAmp * smoothEdge * 0.76 * carrier2;
      down1[i] = midY + rAmp1;
      down2[i] = midY + rAmp2;
    } else {
      down1[i] = midY + amp1;
      down2[i] = midY + amp2;
    }
  }

  const xsRev = xs.slice().reverse();

  // Horizontal stroke gradient for spectrum or solid
  let strokePrimary: CanvasGradient | string = colorway.mid;
  if (colorway.kind === "spectrum") {
    const g = ctx.createLinearGradient(padX, 0, w - padX, 0);
    g.addColorStop(0, colorway.outer);
    g.addColorStop(0.45, colorway.mid);
    g.addColorStop(0.8, colorway.bright);
    g.addColorStop(1, colorway.core);
    strokePrimary = g;
  }

  if (dualChannel) {
    // Top local ribbon (harmonic + primary)
    ctx.save();
    ctx.beginPath();
    traceSmoothCurve(ctx, xs, up2, true);
    ctx.lineTo(xs[points - 1], midY);
    ctx.lineTo(xs[0], midY);
    ctx.closePath();
    ctx.fillStyle = colorway.silkFill2;
    ctx.fill();

    ctx.beginPath();
    traceSmoothCurve(ctx, xs, up1, true);
    ctx.lineTo(xs[points - 1], midY);
    ctx.lineTo(xs[0], midY);
    ctx.closePath();
    ctx.fillStyle = colorway.silkFill1;
    ctx.fill();

    ctx.beginPath();
    traceSmoothCurve(ctx, xs, up1, true);
    ctx.strokeStyle = strokePrimary;
    ctx.lineWidth = compact ? 1.5 : 2;
    if (glow) {
      ctx.shadowColor = colorway.peakGlow;
      ctx.shadowBlur = compact ? 6 : 12;
    }
    ctx.stroke();
    ctx.restore();

    // Bottom remote ribbon
    ctx.save();
    ctx.beginPath();
    traceSmoothCurve(ctx, xs, down2, true);
    ctx.lineTo(xs[points - 1], midY);
    ctx.lineTo(xs[0], midY);
    ctx.closePath();
    ctx.fillStyle = colorway.secondary;
    ctx.globalAlpha = 0.16;
    ctx.fill();

    ctx.beginPath();
    traceSmoothCurve(ctx, xs, down1, true);
    ctx.lineTo(xs[points - 1], midY);
    ctx.lineTo(xs[0], midY);
    ctx.closePath();
    ctx.fillStyle = colorway.secondary;
    ctx.globalAlpha = 0.28;
    ctx.fill();

    ctx.beginPath();
    traceSmoothCurve(ctx, xs, down1, true);
    ctx.globalAlpha = 0.92;
    ctx.strokeStyle = colorway.secondary;
    ctx.lineWidth = compact ? 1.5 : 2;
    if (glow) {
      ctx.shadowColor = colorway.secondary;
      ctx.shadowBlur = compact ? 6 : 10;
    }
    ctx.stroke();
    ctx.restore();
    return;
  }

  // Single-channel dual overlapping symmetrical silk ribbons
  // Secondary harmonic ribbon
  ctx.save();
  ctx.beginPath();
  traceSmoothCurve(ctx, xs, up2, true);
  traceSmoothCurve(ctx, xsRev, down2.slice().reverse(), false);
  ctx.closePath();
  ctx.fillStyle = colorway.silkFill2;
  ctx.fill();

  ctx.beginPath();
  traceSmoothCurve(ctx, xs, up2, true);
  ctx.strokeStyle = colorway.outer;
  ctx.globalAlpha = 0.55;
  ctx.lineWidth = compact ? 1 : 1.25;
  ctx.stroke();

  ctx.beginPath();
  traceSmoothCurve(ctx, xs, down2, true);
  ctx.stroke();
  ctx.restore();

  // Primary luminous ribbon
  ctx.save();
  ctx.beginPath();
  traceSmoothCurve(ctx, xs, up1, true);
  traceSmoothCurve(ctx, xsRev, down1.slice().reverse(), false);
  ctx.closePath();
  ctx.fillStyle = colorway.silkFill1;
  ctx.fill();

  ctx.strokeStyle = strokePrimary;
  ctx.lineWidth = compact ? 1.5 : 2;
  if (glow) {
    ctx.shadowColor = colorway.peakGlow;
    ctx.shadowBlur = compact ? 6 : 13;
  }
  ctx.beginPath();
  traceSmoothCurve(ctx, xs, up1, true);
  ctx.stroke();

  ctx.beginPath();
  traceSmoothCurve(ctx, xs, down1, true);
  ctx.stroke();
  ctx.restore();
}

function drawMatrixCanvas(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  local: BallisticsState,
  remote: BallisticsState,
  cols: number,
  dualChannel: boolean,
  compact: boolean,
  colorway: WaveformColorwayDef,
  glow: boolean,
  bands: AudioBands | null,
  timeSec: number,
): void {
  const dotSize = compact ? 3 : 4.5;
  const gap = compact ? 2 : 2.5;
  const pitch = dotSize + gap;
  const totalW = cols * dotSize + (cols - 1) * gap;
  const startX = (w - totalW) * 0.5;
  const midY = h * 0.5;
  const halfRows = Math.max(2, Math.floor((midY - gap) / pitch));
  const radius = compact ? 1 : 1.5;
  const hiEnergy = bands ? clamp01(bands.high) : 0;
  const loEnergy = bands ? clamp01(bands.low) : 0;

  const pickCellColor = (rowIdx: number, total: number): string => {
    if (colorway.kind === "solid") {
      return rowIdx >= total - 1 ? colorway.core : colorway.mid;
    }
    const ratio = total > 1 ? rowIdx / (total - 1) : 0;
    if (ratio < 0.34) return colorway.outer;
    if (ratio < 0.68) return colorway.mid;
    if (ratio < 0.92) return colorway.bright;
    return colorway.core;
  };

  for (let c = 0; c < cols; c++) {
    const x = startX + c * pitch;
    const u = cols > 1 ? c / (cols - 1) : 0.5;
    const ageAlpha = 0.42 + 0.58 * ((c + 1) / cols);
    const rawLv = local.smooth[c];
    const lv =
      bands && rawLv >= VAD_GATE ? clamp01(rawLv + (u - 0.5) * (hiEnergy - loEnergy) * 0.16) : rawLv;
    const lpk = local.peak[c];
    const litLocal = lv >= VAD_GATE ? Math.max(1, Math.round(lv * halfRows)) : 0;
    const peakLocalRow = lpk >= VAD_GATE ? Math.min(halfRows - 1, Math.floor(lpk * halfRows)) : -1;

    const rv = dualChannel ? remote.smooth[c] : lv;
    const rpk = dualChannel ? remote.peak[c] : lpk;
    const litBottom = rv >= VAD_GATE ? Math.max(1, Math.round(rv * halfRows)) : 0;
    const peakBottomRow = rpk >= VAD_GATE ? Math.min(halfRows - 1, Math.floor(rpk * halfRows)) : -1;

    for (let r = 0; r < halfRows; r++) {
      const yUp = midY - gap * 0.5 - (r + 1) * dotSize - r * gap;
      const yDown = midY + gap * 0.5 + r * pitch;
      const rowFrac = halfRows > 1 ? r / (halfRows - 1) : 1;
      const hiShimmer =
        hiEnergy > 0.14 && rowFrac >= 0.45
          ? hiEnergy * rowFrac * (0.5 + 0.5 * Math.sin(timeSec * 20 + c * 1.85 + r * 2.6))
          : 0;

      // Top cell (local mic)
      const isLitUp = r < litLocal;
      const isPeakUp = !isLitUp && r === peakLocalRow;
      const isShimmerSpark =
        !isLitUp && !isPeakUp && litLocal > 0 && r === litLocal && hiShimmer > 0.44;
      if (isLitUp || isPeakUp || isShimmerSpark) {
        ctx.save();
        if (isShimmerSpark) {
          ctx.fillStyle = colorway.bright;
          ctx.globalAlpha = Math.min(0.85, 0.25 + hiShimmer * 0.55);
        } else {
          ctx.fillStyle = isPeakUp
            ? colorway.core
            : hiShimmer > 0.42
              ? colorway.core
              : pickCellColor(r, halfRows);
          ctx.globalAlpha = isPeakUp ? 0.95 : Math.min(1, ageAlpha + hiShimmer * 0.35);
        }
        if (glow && (isPeakUp || r >= halfRows - 2 || hiShimmer > 0.35)) {
          ctx.shadowColor = colorway.peakGlow;
          ctx.shadowBlur = (compact ? 4 : 8) + hiShimmer * (compact ? 2.5 : 4.5);
        }
        drawRoundedRect(ctx, x, yUp, dotSize, dotSize, radius);
        ctx.restore();
      } else {
        ctx.fillStyle = "rgba(148, 163, 184, 0.11)";
        drawRoundedRect(ctx, x, yUp, dotSize, dotSize, radius);
      }

      // Bottom cell (mirrored local or remote system audio)
      const isLitDown = r < litBottom;
      const isPeakDown = !isLitDown && r === peakBottomRow;
      if (isLitDown || isPeakDown) {
        ctx.save();
        if (dualChannel) {
          ctx.fillStyle = isPeakDown ? colorway.core : colorway.secondary;
        } else {
          ctx.fillStyle = isPeakDown
            ? colorway.core
            : hiShimmer > 0.46
              ? colorway.bright
              : pickCellColor(r, halfRows);
        }
        ctx.globalAlpha = isPeakDown ? 0.95 : Math.min(1, ageAlpha + (dualChannel ? 0 : hiShimmer * 0.25));
        if (glow && (isPeakDown || r >= halfRows - 2)) {
          ctx.shadowColor = dualChannel ? colorway.secondary : colorway.peakGlow;
          ctx.shadowBlur = compact ? 4 : 8;
        }
        drawRoundedRect(ctx, x, yDown, dotSize, dotSize, radius);
        ctx.restore();
      } else {
        ctx.fillStyle = "rgba(148, 163, 184, 0.11)";
        drawRoundedRect(ctx, x, yDown, dotSize, dotSize, radius);
      }
    }
  }
}

function computeHyperBarLayout(
  width: number,
  config: WaveformConfig,
  compact: boolean,
): { count: number; needleWidth: number } {
  const safeW = Math.max(32, width);
  const needleWidth = compact
    ? Math.max(1, Math.min(config.needle_width, safeW < 150 ? 1.5 : config.needle_width))
    : Math.max(1, config.needle_width);
  const minGap = compact ? 1 : 1.25;
  const maxFit = Math.max(8, Math.floor((safeW + minGap) / (needleWidth + minGap)));

  const desired = compact
    ? Math.round(config.needle_bars * Math.min(1, safeW / 260))
    : config.needle_bars;

  const count = Math.max(8, Math.min(MAX_BUFFER_LEN, Math.min(Math.max(8, desired), maxFit)));
  return { count, needleWidth };
}

export function WaveformMeter({
  levels,
  remoteLevels,
  bands,
  dualChannel = false,
  config = DEFAULT_WAVEFORM_CONFIG,
  compact = false,
  demoMode = false,
  className,
  testId,
}: WaveformMeterProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sizeRef = useRef<{ width: number; height: number }>({ width: 160, height: 32 });

  // Latest props stored in refs so the 60fps rAF loop stays unbroken across prop updates
  const propsRef = useRef({
    levels,
    remoteLevels,
    dualChannel,
    config,
    compact,
    demoMode,
  });
  propsRef.current = { levels, remoteLevels, dualChannel, config, compact, demoMode };

  const bandsRef = useRef<AudioBands | undefined>(bands);
  bandsRef.current = bands;
  const smoothBandsRef = useRef<AudioBands>({ low: 0, mid: 0, high: 0 });

  const localTargetsRef = useRef<Float32Array>(new Float32Array(MAX_BUFFER_LEN));
  const remoteTargetsRef = useRef<Float32Array>(new Float32Array(MAX_BUFFER_LEN));
  const localBallisticsRef = useRef<BallisticsState>(createBallistics());
  const remoteBallisticsRef = useRef<BallisticsState>(createBallistics());

  // HyperDOM needle refs (direct DOM mutation at ~30-60Hz, zero React re-renders)
  const localNeedleRefs = useRef<(HTMLSpanElement | null)[]>([]);
  const remoteNeedleRefs = useRef<(HTMLSpanElement | null)[]>([]);
  const lastLocalPctRef = useRef<Float32Array>(new Float32Array(MAX_BUFFER_LEN));
  const lastRemotePctRef = useRef<Float32Array>(new Float32Array(MAX_BUFFER_LEN));
  const lastLocalOpacityRef = useRef<Float32Array>(new Float32Array(MAX_BUFFER_LEN));
  const lastRemoteOpacityRef = useRef<Float32Array>(new Float32Array(MAX_BUFFER_LEN));

  const [measuredWidth, setMeasuredWidth] = useState(160);

  // Wake handle for the on-demand 60fps rAF loop (pauses completely when idle/hidden)
  const wakeRef = useRef<() => void>(() => {});

  // Measure container via ResizeObserver without causing layout feedback loops
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const updateSize = () => {
      const rect = el.getBoundingClientRect();
      // When hidden (e.g. Pill mode's `group-hover:hidden`), keep the last
      // measuredWidth so hyperdom doesn't tear down and recreate its needles.
      if (rect.width < 2 || rect.height < 2) {
        sizeRef.current = { width: 0, height: 0 };
        return;
      }
      const w = Math.round(rect.width);
      const h = Math.round(rect.height);
      sizeRef.current = { width: w, height: h };
      setMeasuredWidth((prev) => (prev === w ? prev : w));
      wakeRef.current();
    };

    updateSize();
    const observer = new ResizeObserver(updateSize);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const colorway = useMemo(() => getWaveformColorway(config.colorway), [config.colorway]);

  const hyperLayout = useMemo(
    () => computeHyperBarLayout(measuredWidth, config, compact),
    [measuredWidth, config.needle_bars, config.needle_width, compact],
  );

  // Trim ref arrays cleanly when bar count, layout mode, or channel split changes
  useEffect(() => {
    if (config.engine !== "hyperdom") return;
    localNeedleRefs.current.length = hyperLayout.count;
    remoteNeedleRefs.current.length = hyperLayout.count;
    lastLocalPctRef.current.fill(-1);
    lastRemotePctRef.current.fill(-1);
    lastLocalOpacityRef.current.fill(-1);
    lastRemoteOpacityRef.current.fill(-1);
    wakeRef.current();
  }, [config.engine, config.hyper_mode, dualChannel, hyperLayout.count]);

  // Wake the animation loop whenever incoming audio or display config changes
  useEffect(() => {
    wakeRef.current();
  }, [levels, remoteLevels, bands, dualChannel, config, compact, demoMode]);

  // Unified 60fps animation loop (handles Canvas engines + HyperDOM ref-driven updates).
  // Suspends completely when hidden or when live audio & ballistics have settled to zero
  // so idle floating HUD modes (Pill/Notch) consume ~0% CPU/GPU.
  useEffect(() => {
    let rafId = 0;
    let lastTs = performance.now();
    let lastDomPushTs = 0;
    let silkHarmonicPhase = 0;
    const startTs = lastTs;

    const isSettled = (state: BallisticsState, count: number): boolean => {
      for (let i = 0; i < count; i++) {
        if (state.smooth[i] > 0.002 || state.peak[i] > 0.002) return false;
      }
      return true;
    };

    const scheduleNext = () => {
      if (rafId === 0 && !document.hidden) {
        lastTs = performance.now();
        rafId = window.requestAnimationFrame(tick);
      }
    };

    const tick = (now: number) => {
      rafId = 0;
      if (document.hidden) return;

      const dt = Math.min(0.05, Math.max(0.001, (now - lastTs) / 1000));
      lastTs = now;
      const timeSec = (now - startTs) / 1000;

      const {
        levels: curLevels,
        remoteLevels: curRemote,
        dualChannel: isDual,
        config: curCfg,
        compact: isCompact,
        demoMode: isDemo,
      } = propsRef.current;

      const cw = getWaveformColorway(curCfg.colorway);
      const { width: w, height: h } = sizeRef.current;
      if (w < 2 || h < 2) return;

      const liveLocal = hasLiveAudio(curLevels);
      const liveRemote = isDual && hasLiveAudio(curRemote);
      const useDemoLocal = isDemo && !liveLocal;
      const useDemoRemote = isDemo && !liveRemote;
      const localTargets = localTargetsRef.current;
      const remoteTargets = remoteTargetsRef.current;
      const localState = localBallisticsRef.current;
      const remoteState = remoteBallisticsRef.current;

      // Smooth 3-band spectral energy at 60fps; stays strictly null when absent/zero or in demoMode
      const rawBands = bandsRef.current;
      const smBands = smoothBandsRef.current;
      if (rawBands && !useDemoLocal) {
        const tLow = clamp01(rawBands.low);
        const tMid = clamp01(rawBands.mid);
        const tHigh = clamp01(rawBands.high);
        if (tLow + tMid + tHigh <= 0.0005) {
          const dec = 1 - Math.exp(-dt / 0.065);
          smBands.low = smBands.low * (1 - dec) < 0.001 ? 0 : smBands.low * (1 - dec);
          smBands.mid = smBands.mid * (1 - dec) < 0.001 ? 0 : smBands.mid * (1 - dec);
          smBands.high = smBands.high * (1 - dec) < 0.001 ? 0 : smBands.high * (1 - dec);
        } else {
          const aAtk = 1 - Math.exp(-dt / 0.022);
          const aDec = 1 - Math.exp(-dt / 0.085);
          smBands.low += (tLow - smBands.low) * (tLow > smBands.low ? aAtk : aDec);
          smBands.mid += (tMid - smBands.mid) * (tMid > smBands.mid ? aAtk : aDec);
          smBands.high += (tHigh - smBands.high) * (tHigh > smBands.high ? aAtk : aDec);
        }
      } else {
        smBands.low = 0;
        smBands.mid = 0;
        smBands.high = 0;
      }
      const activeBands: AudioBands | null =
        smBands.low + smBands.mid + smBands.high > 0.001 ? smBands : null;

      if (curCfg.engine === "hyperdom") {
        // Pace DOM height mutations slightly (~30ms) so CSS cubic-bezier(0.16, 1, 0.3, 1)
        // transitions complete their physical snap cleanly without restarting every frame
        if (now - lastDomPushTs < 28) {
          rafId = window.requestAnimationFrame(tick);
          return;
        }
        lastDomPushTs = now;

        const { count } = computeHyperBarLayout(w, curCfg, isCompact);
        fillTargets(
          curLevels,
          count,
          curCfg.hyper_mode,
          curCfg.sensitivity,
          timeSec,
          useDemoLocal,
          0.4,
          localTargets,
        );

        if (isDual) {
          fillTargets(
            curRemote,
            count,
            curCfg.hyper_mode,
            curCfg.sensitivity,
            timeSec,
            useDemoRemote,
            2.85,
            remoteTargets,
          );
        }

        const localNodes = localNeedleRefs.current;
        const remoteNodes = remoteNeedleRefs.current;
        const prevLocal = lastLocalPctRef.current;
        const prevRemote = lastRemotePctRef.current;
        const prevLocalOpacity = lastLocalOpacityRef.current;
        const prevRemoteOpacity = lastRemoteOpacityRef.current;
        const midIdx = (count - 1) * 0.5;
        const bandMean = activeBands
          ? Math.max(0.04, (activeBands.low + activeBands.mid + activeBands.high) / 3)
          : 1;

        for (let i = 0; i < count; i++) {
          let lv = localTargets[i];
          const isVoice = lv >= VAD_GATE;
          let shimmerBoost = 0;

          if (activeBands && isVoice) {
            const dist = midIdx > 0 ? Math.abs(i - midIdx) / midIdx : 0;
            const u = count > 1 ? i / (count - 1) : 0.5;
            let bandSample: number;
            if (curCfg.hyper_mode === "center") {
              const centerEnergy = activeBands.mid * 0.65 + activeBands.high * 0.35;
              const outerEnergy = activeBands.low * 0.62 + activeBands.high * 0.38;
              bandSample = centerEnergy * (1 - dist) + outerEnergy * dist;
            } else {
              const wLow = Math.max(0, 1 - u * 1.6);
              const wMid = Math.exp(-Math.pow((u - 0.5) / 0.32, 2));
              const wHigh = Math.max(0, (u - 0.35) / 0.65);
              const wSum = wLow + wMid + wHigh || 1;
              bandSample =
                (wLow * activeBands.low + wMid * activeBands.mid + wHigh * activeBands.high) / wSum;
            }
            const rel = Math.max(0.75, Math.min(1.28, bandSample / bandMean));
            const hiRipple = 1 + activeBands.high * 0.12 * Math.sin(i * 1.65 + timeSec * 14);
            lv = clamp01(lv * (0.82 + 0.18 * rel) * hiRipple);
            shimmerBoost = Math.max(
              0,
              activeBands.high * (0.1 + 0.1 * Math.sin(i * 2.1 + timeSec * 16)),
            );
          }

          const pct = isVoice ? Math.max(10, Math.min(100, lv * 100)) : isCompact ? 8 : 5;
          const posFade =
            curCfg.hyper_mode === "center"
              ? 0.45 + 0.55 * (1 - (midIdx > 0 ? Math.abs(i - midIdx) / midIdx : 0) * 0.7)
              : 0.34 + 0.66 * ((i + 1) / count);
          const nextOpacity = isVoice ? Math.min(1, posFade + shimmerBoost) : 0.24 * posFade;
          const node = localNodes[i];
          if (node) {
            if (Math.abs(pct - prevLocal[i]) >= 0.45) {
              prevLocal[i] = pct;
              node.style.height = `${pct.toFixed(1)}%`;
            }
            if (Math.abs(nextOpacity - prevLocalOpacity[i]) >= 0.015) {
              prevLocalOpacity[i] = nextOpacity;
              node.style.opacity = nextOpacity.toFixed(2);
            }
          }

          if (isDual) {
            const rv = remoteTargets[i];
            const isRemoteVoice = rv >= VAD_GATE;
            const rPct = isRemoteVoice ? Math.max(10, Math.min(100, rv * 100)) : isCompact ? 8 : 5;
            const rPosFade =
              curCfg.hyper_mode === "center"
                ? 0.42 + 0.58 * (1 - (midIdx > 0 ? Math.abs(i - midIdx) / midIdx : 0) * 0.7)
                : 0.34 + 0.66 * ((i + 1) / count);
            const nextRemoteOpacity = isRemoteVoice ? rPosFade : 0.22 * rPosFade;
            const rNode = remoteNodes[i];
            if (rNode) {
              if (Math.abs(rPct - prevRemote[i]) >= 0.45) {
                prevRemote[i] = rPct;
                rNode.style.height = `${rPct.toFixed(1)}%`;
              }
              if (Math.abs(nextRemoteOpacity - prevRemoteOpacity[i]) >= 0.015) {
                prevRemoteOpacity[i] = nextRemoteOpacity;
                rNode.style.opacity = nextRemoteOpacity.toFixed(2);
              }
            }
          }
        }

        if (isDemo || liveLocal || liveRemote || activeBands !== null) {
          rafId = window.requestAnimationFrame(tick);
        }
        return;
      }

      // Canvas engines: precision | silk | matrix
      const canvas = canvasRef.current;
      if (!canvas) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
      const pixelW = Math.max(1, Math.round(w * dpr));
      const pixelH = Math.max(1, Math.round(h * dpr));
      if (canvas.width !== pixelW || canvas.height !== pixelH) {
        canvas.width = pixelW;
        canvas.height = pixelH;
      }

      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      let sampleCount = 32;
      if (curCfg.engine === "precision") {
        const barW = isCompact ? 2.5 : 3.5;
        const gap = isCompact ? 2 : 3;
        sampleCount = Math.max(6, Math.min(MAX_BUFFER_LEN, Math.floor((w + gap) / (barW + gap))));
      } else if (curCfg.engine === "silk") {
        sampleCount = isCompact ? 24 : 40;
      } else {
        const dotSize = isCompact ? 3 : 4.5;
        const gap = isCompact ? 2 : 2.5;
        sampleCount = Math.max(6, Math.min(MAX_BUFFER_LEN, Math.floor((w + gap) / (dotSize + gap))));
      }

      fillTargets(
        curLevels,
        sampleCount,
        "scroll",
        curCfg.sensitivity,
        timeSec,
        useDemoLocal,
        0.4,
        localTargets,
      );
      updateBallistics(localState, localTargets, sampleCount, dt);

      if (isDual) {
        fillTargets(
          curRemote,
          sampleCount,
          "scroll",
          curCfg.sensitivity,
          timeSec,
          useDemoRemote,
          2.85,
          remoteTargets,
        );
        updateBallistics(remoteState, remoteTargets, sampleCount, dt);
      }

      if (curCfg.engine === "precision") {
        drawPrecisionCanvas(
          ctx,
          w,
          h,
          localState,
          remoteState,
          sampleCount,
          isDual,
          isCompact,
          cw,
          curCfg.phosphor_glow,
          activeBands,
        );
      } else if (curCfg.engine === "silk") {
        const harmonicSpeed = 3.3 + (activeBands ? activeBands.high * 2.2 : 0);
        silkHarmonicPhase = (silkHarmonicPhase + dt * harmonicSpeed) % (Math.PI * 2000);
        drawSilkCanvas(
          ctx,
          w,
          h,
          localState,
          remoteState,
          sampleCount,
          timeSec,
          silkHarmonicPhase,
          isDual,
          isCompact,
          cw,
          curCfg.phosphor_glow,
          activeBands,
          curCfg.sensitivity,
        );
      } else {
        drawMatrixCanvas(
          ctx,
          w,
          h,
          localState,
          remoteState,
          sampleCount,
          isDual,
          isCompact,
          cw,
          curCfg.phosphor_glow,
          activeBands,
          timeSec,
        );
      }

      const stillActive =
        isDemo ||
        liveLocal ||
        liveRemote ||
        activeBands !== null ||
        !isSettled(localState, sampleCount) ||
        (isDual && !isSettled(remoteState, sampleCount));
      if (stillActive) {
        rafId = window.requestAnimationFrame(tick);
      }
    };

    wakeRef.current = scheduleNext;
    const onVisibility = () => {
      if (!document.hidden) scheduleNext();
    };
    document.addEventListener("visibilitychange", onVisibility);
    scheduleNext();

    return () => {
      wakeRef.current = () => {};
      document.removeEventListener("visibilitychange", onVisibility);
      if (rafId !== 0) window.cancelAnimationFrame(rafId);
    };
  }, []);

  const isHyper = config.engine === "hyperdom";
  const snapMs = Math.max(10, Math.round(config.snap_ms));
  const needleTransition = `height ${snapMs}ms cubic-bezier(0.16, 1, 0.3, 1), opacity ${Math.max(40, Math.round(snapMs * 1.4))}ms ease-out`;

  const hyperIndices = useMemo(() => {
    const arr: number[] = new Array(hyperLayout.count);
    for (let i = 0; i < hyperLayout.count; i++) arr[i] = i;
    return arr;
  }, [hyperLayout.count]);

  const getNeedleBg = (index: number, total: number): string => {
    if (colorway.kind === "solid") return colorway.mid;
    if (config.hyper_mode === "center") {
      const mid = (total - 1) * 0.5;
      const dist = mid > 0 ? Math.abs(index - mid) / mid : 0;
      return dist <= 0.36 ? colorway.needleMidGrad : colorway.needleGrad;
    }
    return index >= total * 0.7 ? colorway.needleMidGrad : colorway.needleGrad;
  };

  const glowFilter =
    config.phosphor_glow && isHyper
      ? `drop-shadow(0 0 ${compact ? 3 : 6}px ${colorway.peakGlow})`
      : undefined;

  const remoteGlowFilter =
    config.phosphor_glow && isHyper
      ? `drop-shadow(0 0 ${compact ? 3 : 5}px ${colorway.secondary})`
      : undefined;

  return (
    <div
      ref={containerRef}
      className={`relative h-full w-full min-w-0 overflow-hidden select-none ${className ?? ""}`}
      data-testid={testId ?? "level-meter"}
      data-waveform-engine={config.engine}
      data-waveform-colorway={colorway.id}
    >
      {isHyper ? (
        dualChannel ? (
          <div className="absolute inset-0 flex flex-col">
            <div
              className="flex h-1/2 w-full items-end justify-between overflow-hidden pb-[2px]"
              style={{ filter: glowFilter }}
            >
              {hyperIndices.map((i) => (
                <span
                  key={i}
                  ref={(el) => {
                    localNeedleRefs.current[i] = el;
                  }}
                  className="shrink-0 rounded-full"
                  style={{
                    width: `${hyperLayout.needleWidth}px`,
                    height: compact ? "8%" : "5%",
                    minHeight: "2px",
                    opacity: 0.25,
                    background: getNeedleBg(i, hyperLayout.count),
                    transition: needleTransition,
                    willChange: "height, opacity",
                  }}
                />
              ))}
            </div>
            <div className="h-px w-full shrink-0 bg-white/15" />
            <div
              className="flex h-1/2 w-full items-start justify-between overflow-hidden pt-[2px]"
              style={{ filter: remoteGlowFilter }}
            >
              {hyperIndices.map((i) => (
                <span
                  key={i}
                  ref={(el) => {
                    remoteNeedleRefs.current[i] = el;
                  }}
                  className="shrink-0 rounded-full"
                  style={{
                    width: `${hyperLayout.needleWidth}px`,
                    height: compact ? "8%" : "5%",
                    minHeight: "2px",
                    opacity: 0.22,
                    background: colorway.secondary,
                    transition: needleTransition,
                    willChange: "height, opacity",
                  }}
                />
              ))}
            </div>
          </div>
        ) : (
          <div
            className="absolute inset-0 flex items-center justify-between overflow-hidden"
            style={{ filter: glowFilter }}
          >
            {hyperIndices.map((i) => (
              <span
                key={i}
                ref={(el) => {
                  localNeedleRefs.current[i] = el;
                }}
                className="shrink-0 rounded-full"
                style={{
                  width: `${hyperLayout.needleWidth}px`,
                  height: compact ? "8%" : "5%",
                  minHeight: "2px",
                  opacity: 0.25,
                  background: getNeedleBg(i, hyperLayout.count),
                  transition: needleTransition,
                  willChange: "height, opacity",
                }}
              />
            ))}
          </div>
        )
      ) : (
        <canvas
          ref={canvasRef}
          aria-hidden="true"
          className="absolute inset-0 block h-full w-full"
        />
      )}
    </div>
  );
}

export default WaveformMeter;
