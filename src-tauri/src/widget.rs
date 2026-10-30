use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// How much the floating widget shows. The window itself isn't
/// user-resizable (no titlebar, no drag handles — see tauri.conf.json), so
/// switching modes resizes it programmatically instead.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WidgetMode {
    /// Icon-only recording button. No status text, no settings button —
    /// errors surface as a color change plus a native tooltip on hover.
    /// Settings is still reachable via the tray menu.
    Minimal,
    /// The original pill: button, one line of status/flash text, settings
    /// gear. Default. Auto-grows temporarily (see WidgetView.tsx) when an
    /// error/status message is too long to fit on one line, instead of
    /// silently truncating it.
    #[default]
    Compact,
    /// Larger fixed panel: full status line, a persistent (not
    /// auto-dismissing) wrapped message area, and the active app-mode —
    /// for when you want the detail visible at a glance rather than
    /// digging through Settings > Logs after the fact.
    Detailed,
    /// Slim capsule: status dot and live waveform; text on hover.
    Pill,
    /// Docked top-center under the menu bar (or notch), Dynamic Island
    /// style: small when idle, wider while recording or working. Always
    /// re-centered there, not draggable.
    Notch,
    /// A card that shows your words as you speak (live transcription),
    /// then what was pasted.
    Caption,
    /// Compact with a second line: mic, and the mode for the next dictation.
    Stacked,
}

/// Every style, in the order the pickers show them.
pub const ALL_MODES: [WidgetMode; 7] = [
    WidgetMode::Minimal,
    WidgetMode::Pill,
    WidgetMode::Compact,
    WidgetMode::Stacked,
    WidgetMode::Notch,
    WidgetMode::Caption,
    WidgetMode::Detailed,
];

impl WidgetMode {
    /// Logical-pixel size for the base state of each mode. Compact grows
    /// further at runtime (frontend-driven) while an error/status message
    /// is showing; this is its resting size.
    pub fn base_size(self) -> (f64, f64) {
        match self {
            WidgetMode::Minimal => (46.0, 46.0),
            WidgetMode::Compact => (280.0, 60.0),
            WidgetMode::Detailed => (320.0, 170.0),
            WidgetMode::Pill => (156.0, 34.0),
            WidgetMode::Notch => (184.0, 32.0),
            WidgetMode::Caption => (380.0, 116.0),
            WidgetMode::Stacked => (280.0, 66.0),
        }
    }

    /// Name in the menu bar's "Widget style" menu.
    pub fn label(self) -> &'static str {
        match self {
            WidgetMode::Minimal => "Minimal",
            WidgetMode::Pill => "Pill",
            WidgetMode::Compact => "Compact",
            WidgetMode::Stacked => "Compact, two-line",
            WidgetMode::Notch => "Notch",
            WidgetMode::Caption => "Live caption",
            WidgetMode::Detailed => "Detailed",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            WidgetMode::Minimal => "minimal",
            WidgetMode::Pill => "pill",
            WidgetMode::Compact => "compact",
            WidgetMode::Stacked => "stacked",
            WidgetMode::Notch => "notch",
            WidgetMode::Caption => "caption",
            WidgetMode::Detailed => "detailed",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        ALL_MODES.into_iter().find(|m| m.id() == id)
    }
}

pub fn apply_size(app: &AppHandle, width: f64, height: f64) {
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
        if crate::config::load(app).widget_mode == WidgetMode::Notch {
            dock_top_center(&window, width);
        }
    }
}

/// Notch style: centered under the menu bar of the screen the widget is
/// on, re-done on every resize so it grows from the middle.
fn dock_top_center(window: &tauri::WebviewWindow, width: f64) {
    let Some(monitor) = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())
    else {
        return;
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let x = area.position.x as f64 + (area.size.width as f64 - width * scale) / 2.0;
    let y = area.position.y as f64;
    let _ = window.set_position(tauri::PhysicalPosition::new(x.round() as i32, y as i32));
}

#[tauri::command]
pub fn get_widget_mode(app: AppHandle) -> WidgetMode {
    crate::config::load(&app).widget_mode
}

#[tauri::command]
pub fn set_widget_mode(app: AppHandle, mode: WidgetMode) {
    set_mode(&app, mode);
}

/// Switches style from anywhere: Settings, the widget's hover menu, or the
/// menu bar's "Widget style" menu.
pub fn set_mode(app: &AppHandle, mode: WidgetMode) {
    let mut cfg = crate::config::load(app);
    cfg.widget_mode = mode;
    let _ = crate::config::save(app, &cfg);

    let (width, height) = mode.base_size();
    apply_size(app, width, height);
    let _ = app.emit("widget-mode-changed", mode);
    crate::sync_widget_style_menu(app, mode);
}

/// Lets the widget itself request a temporary size override (compact
/// mode's auto-grow for long messages) without going through the
/// persisted-mode path — this doesn't change the saved `widget_mode` or
/// notify other windows, it's purely "resize my own window right now".
#[tauri::command]
pub fn set_widget_size(app: AppHandle, width: f64, height: f64) {
    apply_size(&app, width, height);
}

/// Shows the widget for a dictation or meeting starting. `show()` only,
/// never `set_focus()`: stealing keyboard focus made the paste land in the
/// widget instead of the app being dictated into.
pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.show();
    }
}

#[tauri::command]
pub fn get_widget_only_while_dictating(app: AppHandle) -> bool {
    crate::config::load(&app).widget_only_while_dictating
}

/// Turning it off brings the widget back; turning it on lets the widget
/// hide itself once it's idle (it knows whether a dictation is running).
#[tauri::command]
pub fn set_widget_only_while_dictating(app: AppHandle, enabled: bool) {
    let mut cfg = crate::config::load(&app);
    cfg.widget_only_while_dictating = enabled;
    let _ = crate::config::save(&app, &cfg);
    if !enabled {
        show(&app);
    }
    let _ = app.emit("widget-only-while-dictating-changed", enabled);
}

/// Called by the widget once a dictation is over (pasted, or its message
/// has been read). Does nothing unless the widget is set to show only
/// while dictating.
#[tauri::command]
pub fn hide_idle_widget(app: AppHandle) {
    if !crate::config::load(&app).widget_only_while_dictating {
        return;
    }
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_round_trips_through_its_id_and_serde_name() {
        for mode in ALL_MODES {
            assert_eq!(WidgetMode::from_id(mode.id()), Some(mode));
            assert_eq!(
                serde_json::to_string(&mode).unwrap(),
                format!("\"{}\"", mode.id())
            );
        }
    }

    #[test]
    fn default_widget_mode_is_compact() {
        assert_eq!(WidgetMode::default(), WidgetMode::Compact);
    }

    #[test]
    fn serializes_as_lowercase_for_the_frontend() {
        assert_eq!(
            serde_json::to_string(&WidgetMode::Minimal).unwrap(),
            "\"minimal\""
        );
        assert_eq!(
            serde_json::to_string(&WidgetMode::Compact).unwrap(),
            "\"compact\""
        );
        assert_eq!(
            serde_json::to_string(&WidgetMode::Detailed).unwrap(),
            "\"detailed\""
        );
    }

    #[test]
    fn each_mode_has_a_distinct_base_size() {
        let sizes = [
            WidgetMode::Minimal.base_size(),
            WidgetMode::Compact.base_size(),
            WidgetMode::Detailed.base_size(),
        ];
        assert_ne!(sizes[0], sizes[1]);
        assert_ne!(sizes[1], sizes[2]);
        assert_ne!(sizes[0], sizes[2]);
    }
}
