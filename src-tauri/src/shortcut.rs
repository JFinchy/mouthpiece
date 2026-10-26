use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

use crate::config::{self, ShortcutConfig};

/// Named `PushToTalk...` to avoid clashing with
/// `tauri_plugin_global_shortcut::ShortcutState` (the press/release enum).
pub struct PushToTalkState {
    pub current: Mutex<ShortcutConfig>,
    /// Why the current shortcut isn't registered (another app owns the
    /// combination, or a hand-edited config holds an invalid one). `None`
    /// means the hotkey works. Surfaced by the readiness checks.
    pub error: Mutex<Option<String>>,
}

/// The "paste as-is" shortcut: the push-to-talk hotkey plus Shift (or
/// plus Option when the hotkey already uses Shift), so it's one extra key
/// on a combination the user already knows. ⌘D → ⇧⌘D.
pub fn skip_variant(cfg: &ShortcutConfig) -> ShortcutConfig {
    let mut skip = cfg.clone();
    if skip.shift {
        skip.alt = true;
    } else {
        skip.shift = true;
    }
    skip
}

/// Registers the skip shortcut for `cfg`, logging rather than failing:
/// it's a convenience on top of the widget's click target.
fn register_skip(app: &AppHandle, cfg: &ShortcutConfig) {
    if let Ok(skip) = to_shortcut(&skip_variant(cfg)) {
        if let Err(err) = app.global_shortcut().register(skip) {
            crate::applog!("hotkey: paste-as-is shortcut unavailable: {err}");
        }
    }
}

fn unregister_skip(app: &AppHandle, cfg: &ShortcutConfig) {
    if let Ok(skip) = to_shortcut(&skip_variant(cfg)) {
        if app.global_shortcut().is_registered(skip) {
            let _ = app.global_shortcut().unregister(skip);
        }
    }
}

/// Registers the saved shortcut at launch. Failure is recorded rather than
/// returned: a hotkey taken by another app must not stop Mouthpiece from
/// starting, since the widget button, tray and deep links still work and
/// Settings is where the user picks a different combination.
pub fn register_initial(app: &AppHandle, cfg: ShortcutConfig) -> PushToTalkState {
    let error = to_shortcut(&cfg)
        .and_then(|shortcut| {
            app.global_shortcut()
                .register(shortcut)
                .map_err(|e| e.to_string())
        })
        .err();
    if let Some(err) = &error {
        crate::applog!("hotkey: could not register {cfg:?}: {err}");
    }
    register_skip(app, &cfg);
    PushToTalkState {
        current: Mutex::new(cfg),
        error: Mutex::new(error),
    }
}

pub fn to_shortcut(cfg: &ShortcutConfig) -> Result<Shortcut, String> {
    let code = parse_code(&cfg.code).ok_or_else(|| format!("unsupported key: {}", cfg.code))?;

    let mut modifiers = Modifiers::empty();
    if cfg.meta {
        modifiers |= Modifiers::SUPER;
    }
    if cfg.ctrl {
        modifiers |= Modifiers::CONTROL;
    }
    if cfg.alt {
        modifiers |= Modifiers::ALT;
    }
    if cfg.shift {
        modifiers |= Modifiers::SHIFT;
    }
    if modifiers.is_empty() {
        return Err("shortcut needs at least one modifier key".to_string());
    }

    Ok(Shortcut::new(Some(modifiers), code))
}

#[tauri::command]
pub fn get_shortcut(state: State<PushToTalkState>) -> ShortcutConfig {
    state.current.lock().unwrap().clone()
}

#[tauri::command]
pub fn set_shortcut(
    app: AppHandle,
    new_cfg: ShortcutConfig,
    state: State<PushToTalkState>,
) -> Result<ShortcutConfig, String> {
    let new_shortcut = to_shortcut(&new_cfg)?;
    // Same check `set_agent_reply_shortcut` makes from the other side: the
    // shortcut and its paste-as-is variant both have to be free.
    if let Some(agents) = app.try_state::<crate::agents::AgentState>() {
        let clash = [new_cfg.clone(), skip_variant(&new_cfg)]
            .iter()
            .filter_map(|c| to_shortcut(c).ok())
            .any(|s| agents.is_reply_shortcut(&s));
        if clash {
            return Err(
                "That clashes with the reply-to-agent shortcut. Pick a different one.".to_string(),
            );
        }
    }

    let mut current = state.current.lock().unwrap();
    // The old shortcut may never have registered (see `register_initial`),
    // in which case there's nothing to unregister or restore.
    let old_shortcut = to_shortcut(&current)
        .ok()
        .filter(|old| app.global_shortcut().is_registered(*old));

    if old_shortcut != Some(new_shortcut) {
        if let Some(old) = old_shortcut {
            app.global_shortcut()
                .unregister(old)
                .map_err(|e| e.to_string())?;
        }
        if let Err(err) = app.global_shortcut().register(new_shortcut) {
            // best-effort restore so the app doesn't end up with no hotkey
            if let Some(old) = old_shortcut {
                let _ = app.global_shortcut().register(old);
            }
            return Err(err.to_string());
        }
    }

    unregister_skip(&app, &current);
    *current = new_cfg.clone();
    drop(current);
    register_skip(&app, &new_cfg);
    *state.error.lock().unwrap() = None;
    let _ = app.emit("shortcut-changed", &new_cfg);

    let mut cfg = config::load(&app);
    cfg.shortcut = Some(new_cfg.clone());
    let _ = config::save(&app, &cfg);

    Ok(new_cfg)
}

/// Maps a JS `KeyboardEvent.code` string to the matching `Code` variant.
/// Covers the keys realistic for a push-to-talk shortcut; unrecognized
/// codes surface as an error in the UI rather than silently no-op-ing.
fn parse_code(code: &str) -> Option<Code> {
    use Code::*;
    Some(match code {
        "Space" => Space,
        "Tab" => Tab,
        "Escape" => Escape,
        "Enter" => Enter,
        "Backspace" => Backspace,
        "CapsLock" => CapsLock,
        "ArrowUp" => ArrowUp,
        "ArrowDown" => ArrowDown,
        "ArrowLeft" => ArrowLeft,
        "ArrowRight" => ArrowRight,
        "KeyA" => KeyA,
        "KeyB" => KeyB,
        "KeyC" => KeyC,
        "KeyD" => KeyD,
        "KeyE" => KeyE,
        "KeyF" => KeyF,
        "KeyG" => KeyG,
        "KeyH" => KeyH,
        "KeyI" => KeyI,
        "KeyJ" => KeyJ,
        "KeyK" => KeyK,
        "KeyL" => KeyL,
        "KeyM" => KeyM,
        "KeyN" => KeyN,
        "KeyO" => KeyO,
        "KeyP" => KeyP,
        "KeyQ" => KeyQ,
        "KeyR" => KeyR,
        "KeyS" => KeyS,
        "KeyT" => KeyT,
        "KeyU" => KeyU,
        "KeyV" => KeyV,
        "KeyW" => KeyW,
        "KeyX" => KeyX,
        "KeyY" => KeyY,
        "KeyZ" => KeyZ,
        "Digit0" => Digit0,
        "Digit1" => Digit1,
        "Digit2" => Digit2,
        "Digit3" => Digit3,
        "Digit4" => Digit4,
        "Digit5" => Digit5,
        "Digit6" => Digit6,
        "Digit7" => Digit7,
        "Digit8" => Digit8,
        "Digit9" => Digit9,
        "F1" => F1,
        "F2" => F2,
        "F3" => F3,
        "F4" => F4,
        "F5" => F5,
        "F6" => F6,
        "F7" => F7,
        "F8" => F8,
        "F9" => F9,
        "F10" => F10,
        "F11" => F11,
        "F12" => F12,
        "Backquote" => Backquote,
        "Minus" => Minus,
        "Equal" => Equal,
        "BracketLeft" => BracketLeft,
        "BracketRight" => BracketRight,
        "Backslash" => Backslash,
        "Semicolon" => Semicolon,
        "Quote" => Quote,
        "Comma" => Comma,
        "Period" => Period,
        "Slash" => Slash,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn skip_shortcut_adds_shift_or_option() {
        let base = cfg(true, false, false, false, "KeyD");
        let skip = skip_variant(&base);
        assert!(skip.meta && skip.shift && !skip.alt);
        let with_shift = cfg(true, false, false, true, "Space");
        let skip = skip_variant(&with_shift);
        assert!(skip.meta && skip.shift && skip.alt);
        assert_ne!(
            to_shortcut(&base).unwrap(),
            to_shortcut(&skip_variant(&base)).unwrap()
        );
    }
    use super::*;

    fn cfg(meta: bool, ctrl: bool, alt: bool, shift: bool, code: &str) -> ShortcutConfig {
        ShortcutConfig {
            meta,
            ctrl,
            alt,
            shift,
            code: code.to_string(),
        }
    }

    #[test]
    fn default_shortcut_is_valid() {
        assert!(to_shortcut(&ShortcutConfig::default()).is_ok());
    }

    #[test]
    fn rejects_no_modifiers() {
        let err = to_shortcut(&cfg(false, false, false, false, "Space")).unwrap_err();
        assert!(err.contains("modifier"));
    }

    #[test]
    fn rejects_unknown_code() {
        let err = to_shortcut(&cfg(true, false, false, false, "NotAKey")).unwrap_err();
        assert!(err.contains("NotAKey"));
    }

    #[test]
    fn different_configs_produce_different_shortcuts() {
        let a = to_shortcut(&cfg(true, false, false, true, "Space")).unwrap();
        let b = to_shortcut(&cfg(true, false, false, false, "Space")).unwrap();
        let c = to_shortcut(&cfg(true, false, false, true, "KeyV")).unwrap();
        assert_ne!(
            a, b,
            "differing shift flag should produce a different shortcut"
        );
        assert_ne!(
            a, c,
            "differing key code should produce a different shortcut"
        );
    }

    #[test]
    fn same_config_produces_equal_shortcuts() {
        let a = to_shortcut(&cfg(true, true, false, true, "KeyA")).unwrap();
        let b = to_shortcut(&cfg(true, true, false, true, "KeyA")).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn covers_letters_digits_and_function_keys() {
        for code in [
            "KeyA", "KeyZ", "Digit0", "Digit9", "F1", "F12", "ArrowUp", "Tab", "Escape",
        ] {
            assert!(
                parse_code(code).is_some(),
                "expected {code} to be a recognized key"
            );
        }
    }
}
