export type ThemeId = "terminal" | "signal" | "quiet" | "palette";

export type ThemeTokens = {
  bg: string;
  bgSunk: string;
  bgWell: string;
  fg: string;
  muted: string;
  border: string;
  borderStrong: string;
  card: string;
  cardRaised: string;
  accent: string;
  accentBright: string;
  accentFg: string;
  navActiveBg: string;
  refine: string;
  refineBright: string;
  cyan: string;
  specular: string;
};

// Elevated v4 studio palettes for the settings & widget design system.
// Terminal is the default: deep obsidian + molten amber, establishing a
// precision local developer instrument on sight. Terminal's `card`
// (#1C1F23) and `refine` (#A78BFA) also anchor existing e2e assertions.
export const THEMES: Record<ThemeId, ThemeTokens> = {
  terminal: {
    bg: "#0C0E13",
    bgSunk: "#07090D",
    bgWell: "#040508",
    fg: "#F4F2ED",
    muted: "#8C93A4",
    border: "#222733",
    borderStrong: "#313849",
    card: "#1C1F23",
    cardRaised: "#232832",
    accent: "#F59E0B",
    accentBright: "#FBBF24",
    accentFg: "#140B04",
    navActiveBg: "#241D11",
    refine: "#A78BFA",
    refineBright: "#DDD6FE",
    cyan: "#06B6D4",
    specular: "rgba(255, 255, 255, 0.10)",
  },
  signal: {
    bg: "#F4EFE6",
    bgSunk: "#E8E0D2",
    bgWell: "#0C1116",
    fg: "#1A1713",
    muted: "#766B5D",
    border: "#DDD3C2",
    borderStrong: "#C4B7A1",
    card: "#FBF8F2",
    cardRaised: "#FFFFFF",
    accent: "#0F766E",
    accentBright: "#14B8A6",
    accentFg: "#FFFFFF",
    navActiveBg: "#DDEEE9",
    refine: "#7C3AED",
    refineBright: "#6D28D9",
    cyan: "#0284C7",
    specular: "rgba(255, 255, 255, 0.85)",
  },
  quiet: {
    bg: "#F6F7FA",
    bgSunk: "#EAEFF6",
    bgWell: "#0B0E15",
    fg: "#0F131A",
    muted: "#6B7486",
    border: "#DDE2EC",
    borderStrong: "#C4CCD9",
    card: "#FFFFFF",
    cardRaised: "#FFFFFF",
    accent: "#2563EB",
    accentBright: "#60A5FA",
    accentFg: "#FFFFFF",
    navActiveBg: "#E6EEFE",
    refine: "#7C3AED",
    refineBright: "#6D28D9",
    cyan: "#0891B2",
    specular: "rgba(255, 255, 255, 0.95)",
  },
  palette: {
    bg: "#07080B",
    bgSunk: "#030406",
    bgWell: "#020203",
    fg: "#F7F7F5",
    muted: "#7E8391",
    border: "#1C1F28",
    borderStrong: "#2B303D",
    card: "#111319",
    cardRaised: "#181C24",
    accent: "#FF4D2E",
    accentBright: "#FF856D",
    accentFg: "#0D0504",
    navActiveBg: "#261512",
    refine: "#C084FC",
    refineBright: "#E9D5FF",
    cyan: "#38BDF8",
    specular: "rgba(255, 255, 255, 0.09)",
  },
};

export const THEME_ORDER: ThemeId[] = ["terminal", "signal", "quiet", "palette"];

export const THEME_LABEL: Record<ThemeId, string> = {
  terminal: "Terminal — deep obsidian, molten amber",
  signal: "Signal — warm studio linen, jade-teal",
  quiet: "Quiet — clean ceramic, cobalt",
  palette: "Palette — carbon black, vermilion",
};

const VAR_MAP: Record<keyof ThemeTokens, string> = {
  bg: "--dw-bg",
  bgSunk: "--dw-bg-sunk",
  bgWell: "--dw-bg-well",
  fg: "--dw-fg",
  muted: "--dw-muted",
  border: "--dw-border",
  borderStrong: "--dw-border-strong",
  card: "--dw-card",
  cardRaised: "--dw-card-raised",
  accent: "--dw-accent",
  accentBright: "--dw-accent-bright",
  accentFg: "--dw-accent-fg",
  navActiveBg: "--dw-nav-active-bg",
  refine: "--dw-refine",
  refineBright: "--dw-refine-bright",
  cyan: "--dw-cyan",
  specular: "--dw-specular",
};

// Applies a theme's tokens as CSS custom properties on the document root
// so both plain CSS (`var(--dw-*)`) and inline styles can read them —
// used by both SettingsView (full theme) and WidgetView (accent only).
const LIGHT_THEMES: ThemeId[] = ["signal", "quiet"];

export function applyTheme(id: ThemeId) {
  const tokens = THEMES[id];
  const root = document.documentElement.style;
  (Object.keys(VAR_MAP) as (keyof ThemeTokens)[]).forEach((key) => {
    root.setProperty(VAR_MAP[key], tokens[key]);
  });
  // Native controls (scrollbars, inputs, selects) follow the theme, not
  // the OS appearance.
  root.colorScheme = LIGHT_THEMES.includes(id) ? "light" : "dark";
}
