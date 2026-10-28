//! The widget's native menus: the full one (⌄ button or right-click, every
//! style) and "Switch mic" (shown when the mic sends nothing). Native menus
//! draw outside the widget, so the small styles don't have to grow to show
//! them the way Compact's hover panel does.

use crate::recording::RecordingState;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};

const MIC: &str = "mic:";
const MIC_DEFAULT: &str = "mic-default";
const MIC_MODE: &str = "micmode:";
const NEXT: &str = "next:";
const NEXT_AUTO: &str = "next-auto";
const STYLE: &str = "widget-style:";
const SETTINGS: &str = "widget-settings";

fn err(e: tauri::Error) -> String {
    e.to_string()
}

fn heading(app: &AppHandle, text: &str) -> Result<MenuItem<Wry>, String> {
    MenuItem::new(app, text, false, None::<&str>).map_err(err)
}

fn check(app: &AppHandle, id: String, label: &str, on: bool) -> Result<CheckMenuItem<Wry>, String> {
    CheckMenuItem::with_id(app, id, label, true, on, None::<&str>).map_err(err)
}

fn separator(app: &AppHandle) -> Result<PredefinedMenuItem<Wry>, String> {
    PredefinedMenuItem::separator(app).map_err(err)
}

/// One item per mic, the Mac's own first, the one recording ticked, then
/// "System default".
fn mic_items(app: &AppHandle) -> Result<Vec<Box<dyn IsMenuItem<Wry>>>, String> {
    use crate::audio::DeviceStatus::*;
    let state = app.state::<RecordingState>();
    let using = match state.audio.device_status() {
        Some(Preferred { using } | Fallback { using, .. } | SystemDefault { using }) => Some(using),
        _ => None,
    };
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    for input in crate::input_devices::suggestion_order(&crate::input_devices::inputs()) {
        let label = match input.kind {
            crate::input_devices::Kind::BuiltIn => format!("{} (this Mac)", input.name),
            _ => input.name.clone(),
        };
        let on = using.as_deref() == Some(input.name.as_str());
        items.push(Box::new(check(
            app,
            format!("{MIC}{}", input.name),
            &label,
            on,
        )?));
    }
    let picked_default = state.audio.selected_device().is_none();
    items.push(Box::new(check(
        app,
        MIC_DEFAULT.to_string(),
        "System default",
        picked_default,
    )?));
    Ok(items)
}

fn append_all(menu: &Menu<Wry>, items: &[Box<dyn IsMenuItem<Wry>>]) -> Result<(), String> {
    for item in items {
        menu.append(item.as_ref()).map_err(err)?;
    }
    Ok(())
}

/// The widget's menu: microphone, mic mode, the next dictation's mode,
/// widget style, Settings. The same choices as Compact's hover panel.
#[tauri::command]
pub fn show_widget_menu(window: tauri::Window) -> Result<(), String> {
    let app = window.app_handle();
    let cfg = crate::config::load(app);
    let state = app.state::<RecordingState>();
    let menu = Menu::new(app).map_err(err)?;

    menu.append(&heading(app, "Microphone")?).map_err(err)?;
    append_all(&menu, &mic_items(app)?)?;
    menu.append(&separator(app)?).map_err(err)?;

    menu.append(&heading(app, "Mic mode")?).map_err(err)?;
    let isolated = cfg.isolated_voice_enabled;
    menu.append(&check(
        app,
        format!("{MIC_MODE}standard"),
        "Standard",
        !isolated,
    )?)
    .map_err(err)?;
    menu.append(&check(
        app,
        format!("{MIC_MODE}isolated"),
        "Voice Isolation",
        isolated,
    )?)
    .map_err(err)?;
    menu.append(&separator(app)?).map_err(err)?;

    let next = state.mode_override.lock().unwrap().clone();
    let next_menu = Submenu::new(app, "Next dictation", true).map_err(err)?;
    next_menu
        .append(&check(
            app,
            NEXT_AUTO.to_string(),
            "Mode by app",
            next.is_none(),
        )?)
        .map_err(err)?;
    next_menu.append(&separator(app)?).map_err(err)?;
    for mode in &cfg.modes {
        let on = next.as_deref() == Some(mode.name.as_str());
        next_menu
            .append(&check(app, format!("{NEXT}{}", mode.name), &mode.name, on)?)
            .map_err(err)?;
    }
    menu.append(&next_menu).map_err(err)?;

    let style_menu = Submenu::new(app, "Widget style", true).map_err(err)?;
    for style in crate::widget::ALL_MODES {
        let on = style == cfg.widget_mode;
        style_menu
            .append(&check(
                app,
                format!("{STYLE}{}", style.id()),
                style.label(),
                on,
            )?)
            .map_err(err)?;
    }
    menu.append(&style_menu).map_err(err)?;
    menu.append(&separator(app)?).map_err(err)?;
    menu.append(
        &MenuItem::with_id(app, SETTINGS, "Open Settings…", true, None::<&str>).map_err(err)?,
    )
    .map_err(err)?;

    window.popup_menu(&menu).map_err(err)
}

/// The "Switch mic" menu: just the mics.
#[tauri::command]
pub fn show_mic_menu(window: tauri::Window) -> Result<(), String> {
    let app = window.app_handle();
    let menu = Menu::new(app).map_err(err)?;
    menu.append(&heading(app, "Record with")?).map_err(err)?;
    append_all(&menu, &mic_items(app)?)?;
    window.popup_menu(&menu).map_err(err)
}

/// Handles a click in either menu. Other menus' ids are ignored.
pub fn handle(app: &AppHandle, id: &str) {
    if id == MIC_DEFAULT {
        let app = app.clone();
        // Off the main thread: opening the new mic can take a moment.
        std::thread::spawn(move || crate::recording::pick_mic_now(&app, None));
    } else if let Some(name) = id.strip_prefix(MIC) {
        let (app, name) = (app.clone(), name.to_string());
        std::thread::spawn(move || crate::recording::pick_mic_now(&app, Some(name)));
    } else if let Some(mode) = id.strip_prefix(MIC_MODE) {
        let enabled = mode == "isolated";
        crate::recording::set_isolated_voice_enabled(app.clone(), enabled);
        let _ = app.emit("mic-mode-changed", enabled);
    } else if id == NEXT_AUTO || id.starts_with(NEXT) {
        let state = app.state::<RecordingState>();
        let picked = id.strip_prefix(NEXT).map(str::to_string);
        let mut current = state.mode_override.lock().unwrap();
        // Picking the mode that's already set goes back to "by app", like
        // the hover panel.
        *current = if picked == *current { None } else { picked };
        let _ = app.emit("next-mode-changed", current.clone());
    } else if let Some(style) = id
        .strip_prefix(STYLE)
        .and_then(crate::widget::WidgetMode::from_id)
    {
        crate::widget::set_mode(app, style);
    } else if id == SETTINGS {
        let _ = crate::open_settings(app.clone());
    }
}
