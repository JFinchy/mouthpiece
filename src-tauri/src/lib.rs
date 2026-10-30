mod agents;
mod ambient;
mod app_detect;
mod audio;
mod backtrack;
mod boilerplate;
mod capture;
mod clipboard;
mod config;
mod corrections;
mod discard;
mod doubletap;
#[cfg(test)]
mod e2e_audio;
mod error_context;
mod file_tagging;
mod focus;
mod history;
mod input_devices;
mod insights;
mod isolate;
mod lifecycle;
mod live;
mod llm;
mod logging;
mod meeting;
mod mic_permission;
mod mic_usage;
mod models;
mod modes;
mod names;
mod paste;
mod pipeline;
mod privacy;
mod project;
mod punctuation;
mod readiness;
mod recording;
mod rename;
mod shortcut;
mod snippets;
mod stt;
mod syntax;
mod theme;
mod undo;
mod voice_isolation;
mod webhook;
mod widget;
mod widget_menu;

use std::sync::Mutex;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, Submenu},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::ShortcutState as PressState;
use tauri_plugin_updater::UpdaterExt;

use audio::AudioHandle;
use corrections::{
    accept_correction_suggestion, dismiss_correction_suggestion, get_correction_suggestions,
    get_learn_corrections, get_replacements, set_learn_corrections, set_replacements,
};
use doubletap::{get_double_tap_enabled, set_double_tap_enabled, DoubleTapState};
use history::{
    clear_history, delete_history_entry, get_history_retention_days, get_journal_enabled,
    list_history_entries, reprocess_history_text, search_history_entries,
    set_history_retention_days, set_journal_enabled, update_history_entry_text,
    use_original_history_text,
};
use insights::get_insights;
use llm::{get_llm_model, list_llm_catalog, list_ollama_models, pull_llm_model, set_llm_model};
use logging::{clear_logs, get_logs};
use models::{download_model, list_models, set_active_model};
use modes::{get_modes, list_running_apps, set_modes};
use recording::{
    get_active_input_device, get_audio_device_status, get_copy_only, get_isolated_voice_enabled,
    get_last_frontmost_app, get_next_mode_override, get_press_enter_enabled, get_vocabulary,
    list_input_devices, set_copy_only, set_input_device, set_isolated_voice_enabled,
    set_next_mode_override, set_press_enter_enabled, set_vocabulary, toggle_recording,
    toggle_recording_command, RecordingState,
};
use shortcut::{get_shortcut, set_shortcut, PushToTalkState};
use snippets::{get_snippets, set_snippets};
use stt::WhisperEngine;
use theme::{get_layout, get_theme, set_layout, set_theme};
use voice_isolation::{
    get_voice_enrollment_status, start_voice_enrollment, stop_voice_enrollment, VoiceIsolationState,
};
use webhook::{get_webhook_url, send_test_webhook, set_webhook_url};
use widget::{
    get_widget_mode, get_widget_only_while_dictating, hide_idle_widget, set_widget_mode,
    set_widget_only_while_dictating, set_widget_size,
};

/// Shared between the tray builder in `run()` and `recording::toggle_recording`
/// (which looks the tray back up via `AppHandle::tray_by_id` to swap its icon)
/// so both sides agree on which tray they mean without threading a handle
/// through `RecordingState`.
pub const TRAY_ICON_ID: &str = "main";

/// The menu bar's "Widget style" items, kept so the check mark can follow
/// a style change made anywhere else.
struct WidgetStyleMenu(Vec<CheckMenuItem<tauri::Wry>>);

/// The tray's meeting item, relabelled while a meeting is recorded.
struct MeetingMenuItem(MenuItem<tauri::Wry>);

pub fn refresh_meeting_menu(app: &tauri::AppHandle) {
    if let Some(item) = app.try_state::<MeetingMenuItem>() {
        let label = if meeting::is_recording(app) {
            "Stop Meeting"
        } else {
            "Start Meeting"
        };
        let _ = item.0.set_text(label);
    }
}

pub fn sync_widget_style_menu(app: &tauri::AppHandle, mode: widget::WidgetMode) {
    if let Some(menu) = app.try_state::<WidgetStyleMenu>() {
        for item in &menu.0 {
            let _ = item.set_checked(item.id().as_ref() == format!("style:{}", mode.id()));
        }
    }
}

/// The tray glyph is a separate, simpler asset from the Dock icon
/// (icons/icon.png): the same prompt, wave and cursor as single strokes, no
/// background or glow, which would turn to mush at menu bar size. It's
/// wider than tall (macOS scales it to 18pt high and keeps the shape).
/// Rendered as a macOS "template" image by default so the system tints it
/// to match light/dark menu bars; while recording or listening it gets a
/// colored dot instead, and template mode is off since template images
/// can't show color. Drawn by `icons/source/tray-icon.py`.
pub fn tray_base_icon() -> tauri::image::Image<'static> {
    tauri::image::Image::from_bytes(include_bytes!("../icons/tray-icon-template.png"))
        .expect("bundled tray-icon-template.png must decode")
}

/// Draws a small red dot over the bottom-right corner of `base` — gives the
/// tray icon a distinct "recording" state without needing a separate icon
/// asset checked into the repo. Recomputed on each recording start rather
/// than cached; a 32x32 pixel loop is sub-millisecond.
pub fn recording_tray_icon(base: &tauri::image::Image) -> tauri::image::Image<'static> {
    tray_icon_with_dot(base, [220, 38, 38])
}

/// macOS's secondary label grey: legible on light and dark menu bars.
const TRAY_GLYPH_GREY: [u8; 3] = [142, 142, 147];

fn tray_icon_with_dot(
    base: &tauri::image::Image,
    [r, g, b]: [u8; 3],
) -> tauri::image::Image<'static> {
    let width = base.width();
    let height = base.height();
    let mut rgba = base.rgba().to_vec();

    // Without template mode macOS no longer recolors the glyph for the menu
    // bar, so a black glyph would vanish on a dark one. Mid-grey reads on
    // both.
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[..3].copy_from_slice(&TRAY_GLYPH_GREY);
    }

    // Top-right, over the end of the cursor stroke, clear of the wave.
    let radius = (width.min(height) as f32 * 0.24).max(3.0);
    let cx = width as f32 - radius;
    let cy = radius;

    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= radius * radius {
                let idx = ((y * width + x) * 4) as usize;
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = 255;
            }
        }
    }

    tauri::image::Image::new_owned(rgba, width, height)
}

/// Sets the tray icon from current state: red dot while recording, green
/// dot while ambient listening is awake, plain template glyph otherwise.
/// Template mode is off for the colored variants since template images
/// can't show color. Best-effort: a missing tray is skipped.
pub fn refresh_tray_indicator(app: &tauri::AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ICON_ID) else {
        return;
    };
    let recording = app
        .try_state::<RecordingState>()
        .is_some_and(|s| !matches!(s.capture.state(), capture::CaptureState::Idle));
    let listening = app
        .try_state::<ambient::AmbientState>()
        .is_some_and(|s| s.phase() == ambient::AmbientPhase::Listening);
    let base = tray_base_icon();
    let (icon, template) = if recording {
        (recording_tray_icon(&base), false)
    } else if listening {
        (tray_icon_with_dot(&base, [34, 170, 120]), false)
    } else {
        (base, true)
    };
    let _ = tray.set_icon_with_as_template(Some(icon), template);
}

/// Called by each window once its UI has rendered. A webview that fails to
/// load (a blocked script, a CSP mistake) never calls it, so `app.log`
/// shows the gap.
#[tauri::command]
fn ui_ready(window: tauri::Window) {
    crate::applog!("ui: {} window ready", window.label());
}

#[tauri::command]
fn open_settings(app: tauri::AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("settings") {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }

    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Mouthpiece Settings")
    // Wide enough for the signal-chain layout (5 nodes + connectors) to
    // lay out on one row without wrapping — see src/settings/layouts.tsx.
    .inner_size(880.0, 720.0)
    .min_inner_size(760.0, 560.0)
    .resizable(true);

    // Anchor settings just below the widget instead of both windows
    // centering on top of each other.
    if let Some(widget) = app.get_webview_window("widget") {
        if let (Ok(pos), Ok(size), Ok(scale)) = (
            widget.outer_position(),
            widget.outer_size(),
            widget.scale_factor(),
        ) {
            let logical_pos = pos.to_logical::<f64>(scale);
            let logical_size = size.to_logical::<f64>(scale);
            let gap = 12.0;
            builder = builder.position(logical_pos.x, logical_pos.y + logical_size.height + gap);
        } else {
            builder = builder.center();
        }
    } else {
        builder = builder.center();
    }

    builder.build()?;
    Ok(())
}

/// Whether Mouthpiece is currently registered as a macOS login item.
/// Off by default (see `run()` — the autostart plugin is registered but
/// never auto-enabled) since silently adding a login item without an
/// explicit user action would be a surprising thing for a privacy-first
/// app to do; this only reflects whatever the user has toggled in Settings.
#[tauri::command]
fn get_autostart_enabled(app: tauri::AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable()
    } else {
        autostart.disable()
    }
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn prompt_for_accessibility_permission() {
    // Shows the system permission dialog if not already granted. Spawned so
    // the (potentially blocking) prompt doesn't delay startup.
    std::thread::spawn(|| {
        let trusted =
            macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
        crate::applog!("accessibility: trusted={trusted}");
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    if rename::move_bundle() {
        return;
    }
    tauri::Builder::default()
        // Must be the first plugin. A second launch (Spotlight, a login
        // item racing a manual open, a deep link while running) hands off
        // to this instance and exits, instead of starting a second copy
        // that fights over the hotkey, the mic and config.json.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            crate::applog!("lifecycle: second launch handed off to the running instance");
            if let Some(widget) = app.get_webview_window("widget") {
                let _ = widget.show();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, triggered, event| {
                    // Esc / ⇧Esc are only registered while they mean something
                    // (see discard.rs).
                    if event.state == PressState::Pressed && *triggered == discard::escape() {
                        discard::on_escape(app);
                        return;
                    }
                    if event.state == PressState::Pressed && *triggered == discard::undo_shortcut()
                    {
                        discard::undo(app);
                        return;
                    }
                    if event.state == PressState::Pressed
                        && app
                            .try_state::<agents::AgentState>()
                            .is_some_and(|agents| agents.is_reply_shortcut(triggered))
                    {
                        agents::reply_hotkey(app);
                        return;
                    }
                    let state = app.state::<PushToTalkState>();
                    let cfg = state.current.lock().unwrap().clone();
                    if let Ok(expected) = shortcut::to_shortcut(&cfg) {
                        if *triggered == expected {
                            crate::applog!("hotkey: {:?}", event.state);
                        }
                        if *triggered == expected && event.state == PressState::Pressed {
                            toggle_recording(app, capture::Trigger::Hotkey);
                        }
                    }
                    let skip = shortcut::to_shortcut(&shortcut::skip_variant(&cfg));
                    if skip.is_ok_and(|skip| *triggered == skip)
                        && event.state == PressState::Pressed
                    {
                        recording::paste_as_is(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            toggle_recording_command,
            ui_ready,
            recording::paste_as_is_command,
            ambient::get_ambient_enabled,
            ambient::set_ambient_enabled,
            ambient::get_ambient_phase,
            open_settings,
            get_autostart_enabled,
            set_autostart_enabled,
            list_input_devices,
            get_active_input_device,
            get_audio_device_status,
            set_input_device,
            widget_menu::show_mic_menu,
            widget_menu::show_widget_menu,
            get_shortcut,
            set_shortcut,
            get_double_tap_enabled,
            set_double_tap_enabled,
            list_models,
            download_model,
            set_active_model,
            get_modes,
            set_modes,
            list_running_apps,
            get_last_frontmost_app,
            get_vocabulary,
            set_vocabulary,
            project::get_project_names_enabled,
            project::set_project_names_enabled,
            project::get_project_status,
            get_copy_only,
            set_copy_only,
            get_isolated_voice_enabled,
            set_isolated_voice_enabled,
            live::get_live_transcribe_enabled,
            live::set_live_transcribe_enabled,
            set_next_mode_override,
            get_next_mode_override,
            get_theme,
            set_theme,
            get_layout,
            set_layout,
            get_press_enter_enabled,
            set_press_enter_enabled,
            start_voice_enrollment,
            stop_voice_enrollment,
            get_voice_enrollment_status,
            list_history_entries,
            search_history_entries,
            reprocess_history_text,
            update_history_entry_text,
            use_original_history_text,
            get_learn_corrections,
            set_learn_corrections,
            get_correction_suggestions,
            accept_correction_suggestion,
            dismiss_correction_suggestion,
            get_replacements,
            set_replacements,
            clear_history,
            delete_history_entry,
            get_history_retention_days,
            set_history_retention_days,
            get_journal_enabled,
            set_journal_enabled,
            list_ollama_models,
            list_llm_catalog,
            pull_llm_model,
            get_llm_model,
            set_llm_model,
            get_logs,
            clear_logs,
            get_widget_mode,
            set_widget_mode,
            get_widget_only_while_dictating,
            set_widget_only_while_dictating,
            hide_idle_widget,
            set_widget_size,
            get_webhook_url,
            set_webhook_url,
            send_test_webhook,
            get_snippets,
            set_snippets,
            get_insights,
            readiness::get_readiness,
            llm::get_llm_only_when_needed,
            llm::set_llm_only_when_needed,
            readiness::open_accessibility_settings,
            readiness::request_accessibility,
            agents::get_waiting_agents,
            agents::dismiss_waiting_agent,
            agents::focus_waiting_agent,
            agents::get_agent_reply_shortcut,
            agents::get_agent_reply_shortcut_error,
            agents::set_agent_reply_shortcut,
            discard::discard_recording,
            discard::undo_discard,
            mic_permission::request_microphone_access,
            mic_permission::open_microphone_settings,
            privacy::get_history_lock,
            privacy::set_history_lock_enabled,
            privacy::unlock_history,
            privacy::lock_history,
            privacy::get_log_transcripts,
            privacy::set_log_transcripts,
            meeting::start_meeting,
            meeting::stop_meeting,
            meeting::get_meeting_status,
            meeting::list_meetings,
            meeting::read_meeting,
            meeting::open_meeting,
            meeting::delete_meeting,
            meeting::transcribe_meeting,
            meeting::get_meeting_speakers,
            meeting::set_meeting_speakers,
            agents::get_agent_sound_enabled,
            agents::set_agent_sound_enabled,
            agents::get_agent_hook_config,
        ])
        .setup(|app| {
            use tauri::Manager;
            match app.path().app_data_dir() {
                Ok(dir) => {
                    rename::move_data_dir(&dir);
                    if let Err(err) = logging::init_file(dir.join("app.log")) {
                        crate::applog!("logging: could not open app.log: {err}");
                    }
                }
                Err(err) => crate::applog!("logging: no app data dir: {err}"),
            }
            logging::install_panic_hook();
            crate::applog!(
                "lifecycle: started version={} pid={}",
                app.package_info().version,
                std::process::id()
            );
            lifecycle::spawn_watchers();
            rename::move_login_item(app.handle());

            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                prompt_for_accessibility_permission();
            }

            let saved_config = config::load(app.handle());
            logging::set_log_transcripts(saved_config.log_transcripts);
            app.manage(privacy::PrivacyState::default());
            app.manage(discard::DiscardState::default());
            app.manage(meeting::MeetingState::default());
            history::make_private(app.handle());

            let shortcut_cfg = saved_config.shortcut.clone().unwrap_or_default();
            app.manage(shortcut::register_initial(app.handle(), shortcut_cfg));

            app.manage(agents::init(app.handle(), &saved_config));

            app.manage(DoubleTapState::new(saved_config.double_tap_fn_enabled));
            if saved_config.double_tap_fn_enabled {
                doubletap::ensure_listener_started(app.handle());
            }

            let status_app = app.handle().clone();
            let audio = AudioHandle::spawn(saved_config.input_device.clone(), move |status| {
                let _ = status_app.emit("audio-device-status", status);
            });

            let model_id = saved_config
                .active_model
                .clone()
                .unwrap_or_else(|| models::default_model_id().to_string());
            let model_path = models::resolve_model_path(app.handle(), &model_id)
                .or_else(|| models::model_path(app.handle(), &model_id).ok())
                .unwrap_or_default();
            let whisper = WhisperEngine::new(model_id, model_path);
            whisper.set_vocabulary(saved_config.vocabulary.clone());

            app.manage(RecordingState {
                audio,
                whisper,
                capture: capture::CaptureController::new(),
                capture_op: Mutex::new(()),
                active_app: Mutex::new(None),
                mode_override: Mutex::new(None),
                skip_refine: Default::default(),
                last_paste: Default::default(),
                live: Mutex::new(None),
            });

            let voice_isolation = VoiceIsolationState::new();
            voice_isolation.load_persisted(app.handle());
            app.manage(voice_isolation);

            app.manage(ambient::AmbientState::new());
            if saved_config.ambient_enabled {
                if let Err(err) = ambient::start(app.handle()) {
                    crate::applog!("ambient: enabled in config but couldn't start: {err}");
                }
            }

            // Deep-link hooks for external automation (Raycast, Hammerspoon,
            // Alfred, shell scripts): `open mouthpiece://toggle-recording`
            // and friends. start/stop are idempotent — start while already
            // recording is rejected (and logged) rather than toggling off,
            // so a script doesn't need to track state itself.
            let deep_link_app = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                use capture::{Request, Trigger::DeepLink};
                for url in event.urls() {
                    let action = url.host_str().unwrap_or("");
                    // Only the action: agent links carry the hook token
                    // (and the agent's folder and message) in the query.
                    crate::applog!("deep-link: received action={action:?}");
                    let request = match action {
                        "toggle-recording" => Request::ToggleDictation(DeepLink),
                        "start-recording" => Request::StartDictation(DeepLink),
                        "stop-recording" => Request::StopDictation(DeepLink),
                        // From the Claude Code hook script (see agents.rs).
                        "agent-waiting" | "agent-resumed" => {
                            agents::handle_link(&deep_link_app, &url);
                            continue;
                        }
                        other => {
                            crate::applog!("deep-link: unrecognized action {other:?}");
                            continue;
                        }
                    };
                    recording::request_dictation(&deep_link_app, request);
                }
            });

            // Warm up the model in the background so the first real
            // recording doesn't pay for Metal shader compilation, which can
            // take several seconds and otherwise happens on the user's
            // first "transcribing…".
            // Modes can pin their own model (e.g. Terminal on base.en while
            // the default is tiny.en); those are warmed too, or the first
            // dictation in that mode after launch paid the cold load
            // (observed: 2.7s to transcribe a 3.5s clip).
            let warm_app = app.handle().clone();
            let mode_models: Vec<String> = {
                let mut ids: Vec<String> = saved_config
                    .modes
                    .iter()
                    .filter_map(|m| m.stt_model.clone())
                    .collect();
                ids.sort();
                ids.dedup();
                ids
            };
            std::thread::spawn(move || {
                let state = warm_app.state::<RecordingState>();
                let started = std::time::Instant::now();
                if let Err(err) = state.whisper.ensure_loaded() {
                    crate::applog!("model warm-up skipped: {err}");
                }
                for id in mode_models {
                    if id == state.whisper.active_model_id() {
                        continue;
                    }
                    match models::resolve_model_path(&warm_app, &id) {
                        Some(path) => {
                            if let Err(err) = state.whisper.ensure_loaded_model(id.clone(), path) {
                                crate::applog!("model warm-up of {id} skipped: {err}");
                            }
                        }
                        None => crate::applog!("model warm-up of {id} skipped: not downloaded"),
                    }
                }
                crate::applog!(
                    "models: warm-up done in {}ms",
                    started.elapsed().as_millis()
                );
            });

            // Purge transcript history older than the configured retention
            // window on every launch.
            let purge_app = app.handle().clone();
            let retention_days = saved_config.history_retention_days;
            std::thread::spawn(move || {
                history::purge_old_entries(&purge_app, retention_days);
            });

            // Check GitHub for a newer release once per launch — a plain,
            // unauthenticated request to the updater endpoint (see
            // tauri.conf.json's plugins.updater.endpoints), nothing about
            // this device or its usage is sent. Best-effort: no network,
            // no release yet, or a malformed response should never surface
            // as an error, same reasoning as the model warm-up and history
            // purge above — this is a nice-to-know, not something the
            // user's session should ever block or fail on.
            let update_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                match update_app.updater() {
                    Ok(updater) => match updater.check().await {
                        Ok(Some(update)) => {
                            crate::applog!(
                                "update: version {} available (current {})",
                                update.version,
                                update.current_version
                            );
                            let _ = update_app.emit("update-available", update.version);
                        }
                        Ok(None) => {}
                        Err(err) => crate::applog!("update: check failed, ignoring: {err}"),
                    },
                    Err(err) => crate::applog!("update: updater unavailable: {err}"),
                }
            });

            // Reopen the widget wherever the user last dragged it, instead
            // of always re-centering (tauri.conf.json's "center": true is
            // just the first-ever-launch fallback, before any position has
            // been saved).
            if let Some(widget) = app.get_webview_window("widget") {
                if let Some((x, y)) = saved_config.widget_position {
                    let _ = widget.set_position(tauri::LogicalPosition::new(x, y));
                }
                // tauri.conf.json's fixed 220x60 is only the Compact-mode
                // size; apply whatever mode was last saved before the
                // window is shown.
                let (width, height) = saved_config.widget_mode.base_size();
                let _ = widget.set_size(tauri::LogicalSize::new(width, height));

                // A drag fires dozens of Moved events; saving each one meant
                // a config read+write per frame. Only the latest position is
                // kept, and one save runs once the widget stops moving.
                let position_app = app.handle().clone();
                let pending: std::sync::Arc<Mutex<Option<(f64, f64)>>> = Default::default();
                widget.on_window_event(move |event| {
                    let tauri::WindowEvent::Moved(physical_pos) = event else {
                        return;
                    };
                    let Some(window) = position_app.get_webview_window("widget") else {
                        return;
                    };
                    let Ok(scale) = window.scale_factor() else {
                        return;
                    };
                    let logical = physical_pos.to_logical::<f64>(scale);
                    let was_idle = pending
                        .lock()
                        .unwrap()
                        .replace((logical.x, logical.y))
                        .is_none();
                    if was_idle {
                        let (save_app, pending) = (position_app.clone(), pending.clone());
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(400));
                            if let Some(position) = pending.lock().unwrap().take() {
                                let mut cfg = config::load(&save_app);
                                cfg.widget_position = Some(position);
                                let _ = config::save(&save_app, &cfg);
                            }
                        });
                    }
                });
            }

            let toggle_recording_item = MenuItem::with_id(
                app,
                "toggle_recording",
                "Start/Stop Recording",
                true,
                None::<&str>,
            )?;
            let meeting_item =
                MenuItem::with_id(app, "toggle_meeting", "Start Meeting", true, None::<&str>)?;
            app.manage(MeetingMenuItem(meeting_item.clone()));
            let toggle_widget =
                MenuItem::with_id(app, "toggle_widget", "Show/Hide Widget", true, None::<&str>)?;
            let open_settings_item =
                MenuItem::with_id(app, "open_settings", "Open Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            // "Widget style" submenu: one checked item per style.
            let current_style = saved_config.widget_mode;
            let style_items = widget::ALL_MODES
                .iter()
                .map(|mode| {
                    CheckMenuItem::with_id(
                        app,
                        format!("style:{}", mode.id()),
                        mode.label(),
                        true,
                        *mode == current_style,
                        None::<&str>,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let style_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = style_items
                .iter()
                .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>)
                .collect();
            let style_menu = Submenu::with_items(app, "Widget style", true, &style_refs)?;
            app.manage(WidgetStyleMenu(style_items));
            let menu = Menu::with_items(
                app,
                &[
                    &toggle_recording_item,
                    &meeting_item,
                    &toggle_widget,
                    &style_menu,
                    &open_settings_item,
                    &quit,
                ],
            )?;

            // The widget's own menus (`widget_menu`).
            app.on_menu_event(|app, event| widget_menu::handle(app, event.id.as_ref()));

            TrayIconBuilder::with_id(TRAY_ICON_ID)
                .icon(tray_base_icon())
                .icon_as_template(true)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle_recording" => toggle_recording(app, capture::Trigger::Tray),
                    "toggle_meeting" => {
                        let result = if meeting::is_recording(app) {
                            meeting::stop(app)
                        } else {
                            meeting::start(app)
                        };
                        if let Err(err) = result {
                            crate::applog!("meeting: from the menu bar: {err}");
                            let _ = app.emit("meeting-failed", err);
                        }
                    }
                    "toggle_widget" => {
                        if let Some(window) = app.get_webview_window("widget") {
                            let visible = window.is_visible().unwrap_or(false);
                            if visible {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                    "open_settings" => {
                        let _ = open_settings(app.clone());
                    }
                    "quit" => app.exit(0),
                    id => {
                        if let Some(mode) = id
                            .strip_prefix("style:")
                            .and_then(widget::WidgetMode::from_id)
                        {
                            widget::set_mode(app, mode);
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| lifecycle::log_run_event(&event));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_tray_glyph_decodes() {
        let base = tray_base_icon();
        assert!(base.width() > base.height(), "the glyph is wider than tall");
    }

    #[test]
    fn recording_tray_icon_paints_a_red_dot_without_changing_dimensions() {
        let width = 44;
        let height = 32;
        let base = tauri::image::Image::new_owned(
            vec![10u8; (width * height * 4) as usize],
            width,
            height,
        );

        let marked = recording_tray_icon(&base);
        assert_eq!(marked.width(), width);
        assert_eq!(marked.height(), height);

        // Center of the dot (top-right corner) should be opaque red.
        let radius = height as f32 * 0.24;
        let cx = (width as f32 - radius) as u32;
        let cy = radius as u32;
        let idx = ((cy * width + cx) * 4) as usize;
        let rgba = marked.rgba();
        assert_eq!(&rgba[idx..idx + 4], &[220, 38, 38, 255]);

        // Bottom-left, far from the dot: recolored grey, alpha kept.
        let last_row = ((height - 1) * width * 4) as usize;
        assert_eq!(&rgba[last_row..last_row + 4], &[142, 142, 147, 10]);
    }
}
