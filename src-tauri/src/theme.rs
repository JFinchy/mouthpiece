use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Visual theme for the Settings window, and the accent color the floating
/// widget borrows for its non-recording highlights (mic-mode selection,
/// gear hover, the quick-actions flyout). Purely cosmetic — no field here
/// changes app behavior, only `config::AppConfig`'s other fields do that.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Dark graphite, amber accent, monospace-forward. Default — this is a
    /// local developer tool first, not a consumer dictation app.
    #[default]
    Terminal,
    /// Warm paper, teal accent, editorial hero.
    Signal,
    /// Light, restrained blue accent, no decoration.
    Quiet,
    /// Near-black, coral accent.
    Palette,
}

#[tauri::command]
pub fn get_theme(app: AppHandle) -> Theme {
    crate::config::load(&app).theme
}

#[tauri::command]
pub fn set_theme(app: AppHandle, theme: Theme) {
    let mut cfg = crate::config::load(&app);
    cfg.theme = theme;
    let _ = crate::config::save(&app, &cfg);
    // Settings and the widget are separate windows — this is how the
    // widget picks up an accent change made in Settings without a restart.
    let _ = app.emit("theme-changed", theme);
}

/// Settings window navigation shape — independent of `Theme` (color) and
/// purely about how sections are organized/reached. Both read the same
/// section components; only the chrome around them differs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Left sidebar with one page of content on the right. Default — most
    /// section content is reachable in a single click.
    #[default]
    Sidebar,
    /// Horizontal pipeline of nodes (Input/Recognition/Mode/Refinement/
    /// Output/App); clicking a node expands a drawer beneath it.
    Chain,
}

#[tauri::command]
pub fn get_layout(app: AppHandle) -> Layout {
    crate::config::load(&app).layout
}

#[tauri::command]
pub fn set_layout(app: AppHandle, layout: Layout) {
    let mut cfg = crate::config::load(&app);
    cfg.layout = layout;
    let _ = crate::config::save(&app, &cfg);
    let _ = app.emit("layout-changed", layout);
}

/// Visual rendering engine for the live recording waveform in the widget.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WaveformEngine {
    /// Crisp vertical needle bars. Default.
    #[default]
    Precision,
    /// Smooth continuous ribbon.
    Silk,
    /// Dot-matrix equalizer columns.
    Matrix,
    /// High-density DOM waveform strip.
    Hyperdom,
}

/// Layout behavior for the `Hyperdom` waveform engine.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HyperLayoutMode {
    /// Symmetric mirrored growth from center. Default.
    #[default]
    Center,
    /// Left-to-right scrolling amplitude history.
    Scroll,
}

fn default_colorway() -> String {
    "ember".to_string()
}

fn default_needle_bars() -> u32 {
    56
}

fn default_sensitivity() -> f32 {
    1.35
}

fn default_snap_ms() -> u32 {
    36
}

fn default_needle_width() -> f32 {
    2.0
}

fn default_true() -> bool {
    true
}

/// Visual configuration for the widget's live recording waveform. Purely
/// cosmetic — persisted in `AppConfig` and broadcast between Settings and
/// the floating widget via `"waveform-config-changed"`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WaveformConfig {
    #[serde(default)]
    pub engine: WaveformEngine,
    /// Palette name — supports `"ember"`, `"aurora"`, `"mint"`, `"arctic"`,
    /// `"titanium"`, `"solid-red"`, `"solid-white"`, `"solid-amber"`,
    /// `"solid-emerald"`, `"solid-cyan"`.
    #[serde(default = "default_colorway")]
    pub colorway: String,
    #[serde(default = "default_needle_bars")]
    pub needle_bars: u32,
    #[serde(default = "default_sensitivity")]
    pub sensitivity: f32,
    #[serde(default = "default_snap_ms")]
    pub snap_ms: u32,
    #[serde(default = "default_needle_width")]
    pub needle_width: f32,
    #[serde(default)]
    pub hyper_mode: HyperLayoutMode,
    #[serde(default = "default_true")]
    pub phosphor_glow: bool,
    /// Show the quick waveform engine switcher inside the floating widget's
    /// cursor hover popover. Defaults to true; uncheck in Settings to hide it.
    #[serde(default = "default_true")]
    pub show_in_flyout: bool,
}

impl Default for WaveformConfig {
    fn default() -> Self {
        Self {
            engine: WaveformEngine::default(),
            colorway: default_colorway(),
            needle_bars: default_needle_bars(),
            sensitivity: default_sensitivity(),
            snap_ms: default_snap_ms(),
            needle_width: default_needle_width(),
            hyper_mode: HyperLayoutMode::default(),
            phosphor_glow: default_true(),
            show_in_flyout: default_true(),
        }
    }
}

const VALID_COLORWAYS: &[&str] = &[
    "ember",
    "aurora",
    "mint",
    "arctic",
    "titanium",
    "solid-red",
    "solid-white",
    "solid-amber",
    "solid-emerald",
    "solid-cyan",
];

impl WaveformConfig {
    /// Clamps numeric waveform parameters and normalizes unknown colorways.
    pub fn clamped(mut self) -> Self {
        if !VALID_COLORWAYS.contains(&self.colorway.as_str()) {
            self.colorway = default_colorway();
        }
        self.needle_bars = self.needle_bars.clamp(16, 140);
        self.sensitivity = if !self.sensitivity.is_finite() {
            default_sensitivity()
        } else {
            self.sensitivity.clamp(0.5, 2.5)
        };
        self.snap_ms = self.snap_ms.clamp(12, 110);
        self.needle_width = if !self.needle_width.is_finite() {
            default_needle_width()
        } else {
            self.needle_width.clamp(1.0, 4.0)
        };
        self
    }
}

#[tauri::command]
pub fn get_waveform_config(app: AppHandle) -> WaveformConfig {
    crate::config::load(&app).waveform.clamped()
}

#[tauri::command]
pub fn set_waveform_config(app: AppHandle, config: WaveformConfig) {
    let clamped = config.clamped();
    let mut cfg = crate::config::load(&app);
    cfg.waveform = clamped.clone();
    let _ = crate::config::save(&app, &cfg);
    let _ = app.emit("waveform-config-changed", clamped);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_is_terminal() {
        assert_eq!(Theme::default(), Theme::Terminal);
    }

    #[test]
    fn serializes_as_lowercase_for_the_frontend() {
        assert_eq!(
            serde_json::to_string(&Theme::Terminal).unwrap(),
            "\"terminal\""
        );
        assert_eq!(serde_json::to_string(&Theme::Signal).unwrap(), "\"signal\"");
        assert_eq!(serde_json::to_string(&Theme::Quiet).unwrap(), "\"quiet\"");
        assert_eq!(
            serde_json::to_string(&Theme::Palette).unwrap(),
            "\"palette\""
        );
    }

    #[test]
    fn default_layout_is_sidebar() {
        assert_eq!(Layout::default(), Layout::Sidebar);
    }

    #[test]
    fn layout_serializes_as_lowercase_for_the_frontend() {
        assert_eq!(
            serde_json::to_string(&Layout::Sidebar).unwrap(),
            "\"sidebar\""
        );
        assert_eq!(serde_json::to_string(&Layout::Chain).unwrap(), "\"chain\"");
    }

    #[test]
    fn waveform_config_defaults_match_spec() {
        let cfg = WaveformConfig::default();
        assert_eq!(cfg.engine, WaveformEngine::Precision);
        assert_eq!(cfg.colorway, "ember");
        assert_eq!(cfg.needle_bars, 56);
        assert_eq!(cfg.sensitivity, 1.35);
        assert_eq!(cfg.snap_ms, 36);
        assert_eq!(cfg.needle_width, 2.0);
        assert_eq!(cfg.hyper_mode, HyperLayoutMode::Center);
        assert!(cfg.phosphor_glow);
        assert!(cfg.show_in_flyout);
    }

    #[test]
    fn waveform_enums_serialize_as_lowercase() {
        assert_eq!(
            serde_json::to_string(&WaveformEngine::Precision).unwrap(),
            "\"precision\""
        );
        assert_eq!(
            serde_json::to_string(&WaveformEngine::Silk).unwrap(),
            "\"silk\""
        );
        assert_eq!(
            serde_json::to_string(&WaveformEngine::Matrix).unwrap(),
            "\"matrix\""
        );
        assert_eq!(
            serde_json::to_string(&WaveformEngine::Hyperdom).unwrap(),
            "\"hyperdom\""
        );
        assert_eq!(
            serde_json::to_string(&HyperLayoutMode::Center).unwrap(),
            "\"center\""
        );
        assert_eq!(
            serde_json::to_string(&HyperLayoutMode::Scroll).unwrap(),
            "\"scroll\""
        );
    }

    #[test]
    fn waveform_config_json_round_trip() {
        let original = WaveformConfig {
            engine: WaveformEngine::Hyperdom,
            colorway: "solid-cyan".to_string(),
            needle_bars: 96,
            sensitivity: 1.8,
            snap_ms: 48,
            needle_width: 2.5,
            hyper_mode: HyperLayoutMode::Scroll,
            phosphor_glow: false,
            show_in_flyout: false,
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: WaveformConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn waveform_config_partial_json_uses_defaults() {
        let empty: WaveformConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, WaveformConfig::default());

        let partial: WaveformConfig = serde_json::from_str(
            r#"{"engine":"matrix","colorway":"aurora","phosphor_glow":false}"#,
        )
        .unwrap();
        assert_eq!(partial.engine, WaveformEngine::Matrix);
        assert_eq!(partial.colorway, "aurora");
        assert!(!partial.phosphor_glow);
        assert!(partial.show_in_flyout);
        assert_eq!(partial.needle_bars, 56);
        assert_eq!(partial.sensitivity, 1.35);
        assert_eq!(partial.snap_ms, 36);
        assert_eq!(partial.needle_width, 2.0);
        assert_eq!(partial.hyper_mode, HyperLayoutMode::Center);
    }

    #[test]
    fn waveform_config_clamps_numeric_bounds() {
        let too_low = WaveformConfig {
            needle_bars: 0,
            sensitivity: -1.0,
            snap_ms: 1,
            needle_width: 0.1,
            ..WaveformConfig::default()
        }
        .clamped();
        assert_eq!(too_low.needle_bars, 16);
        assert_eq!(too_low.sensitivity, 0.5);
        assert_eq!(too_low.snap_ms, 12);
        assert_eq!(too_low.needle_width, 1.0);

        let too_high = WaveformConfig {
            needle_bars: 999,
            sensitivity: 10.0,
            snap_ms: 500,
            needle_width: 12.0,
            ..WaveformConfig::default()
        }
        .clamped();
        assert_eq!(too_high.needle_bars, 140);
        assert_eq!(too_high.sensitivity, 2.5);
        assert_eq!(too_high.snap_ms, 110);
        assert_eq!(too_high.needle_width, 4.0);

        let nan_values = WaveformConfig {
            sensitivity: f32::NAN,
            needle_width: f32::NAN,
            ..WaveformConfig::default()
        }
        .clamped();
        assert_eq!(nan_values.sensitivity, 1.35);
        assert_eq!(nan_values.needle_width, 2.0);

        let valid = WaveformConfig {
            needle_bars: 72,
            sensitivity: 1.5,
            snap_ms: 40,
            needle_width: 2.5,
            ..WaveformConfig::default()
        };
        assert_eq!(valid.clone().clamped(), valid);
    }
}
