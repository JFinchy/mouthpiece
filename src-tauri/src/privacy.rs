//! Privacy settings: the History lock (Touch ID or the Mac password before
//! History is shown) and whether dictated text is written to `app.log`.
//!
//! The lock is a screen against someone else at your unlocked Mac, not
//! encryption: history stays in a file only your user account can read.
//! It's enforced here, in the commands that return or change history, not
//! only in the UI.

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// How long History stays open after unlocking.
const UNLOCK_FOR: Duration = Duration::from_secs(5 * 60);

pub const LOCKED: &str = "History is locked.";

#[derive(Default)]
pub struct PrivacyState {
    unlocked_until: Mutex<Option<Instant>>,
}

impl PrivacyState {
    fn is_unlocked(&self) -> bool {
        self.unlocked_until
            .lock()
            .unwrap()
            .is_some_and(|until| Instant::now() < until)
    }

    fn set_unlocked(&self, unlocked: bool) {
        *self.unlocked_until.lock().unwrap() = unlocked.then(|| Instant::now() + UNLOCK_FOR);
    }
}

/// Whether History can be shown right now.
pub fn history_locked(app: &AppHandle) -> bool {
    crate::config::load(app).history_lock_enabled
        && !app
            .try_state::<PrivacyState>()
            .is_some_and(|state| state.is_unlocked())
}

/// For history commands: an error while the lock is closed.
pub fn require_history_unlocked(app: &AppHandle) -> Result<(), String> {
    if history_locked(app) {
        Err(LOCKED.to_string())
    } else {
        Ok(())
    }
}

/// Asks macOS to confirm it's the Mac's owner: Touch ID where available,
/// otherwise the login password. Blocks until the user answers, so it must
/// not run on the main thread.
#[cfg(target_os = "macos")]
pub fn authenticate(reason: &str) -> Result<(), String> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    let context = unsafe { LAContext::new() };
    let policy = LAPolicy::DeviceOwnerAuthentication;
    if let Err(err) = unsafe { context.canEvaluatePolicy_error(policy) } {
        return Err(format!(
            "This Mac can't confirm it's you: {}",
            err.localizedDescription()
        ));
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let reply = RcBlock::new(move |ok: Bool, err: *mut NSError| {
        let result = if ok.as_bool() {
            Ok(())
        } else if err.is_null() {
            Err("Not confirmed.".to_string())
        } else {
            // Cancelling is an answer, not a failure worth explaining.
            let err = unsafe { &*err };
            if matches!(err.code(), -2 | -4 | -9) {
                Err("Cancelled.".to_string())
            } else {
                Err(err.localizedDescription().to_string())
            }
        };
        let _ = tx.send(result);
    });
    unsafe {
        context.evaluatePolicy_localizedReason_reply(policy, &NSString::from_str(reason), &reply);
    }
    rx.recv_timeout(Duration::from_secs(120))
        .unwrap_or_else(|_| Err("Timed out waiting for Touch ID or your password.".to_string()))
}

#[cfg(not(target_os = "macos"))]
pub fn authenticate(_reason: &str) -> Result<(), String> {
    Err("Not supported on this platform.".to_string())
}

#[derive(serde::Serialize)]
pub struct HistoryLock {
    pub enabled: bool,
    pub locked: bool,
}

#[tauri::command]
pub fn get_history_lock(app: AppHandle) -> HistoryLock {
    HistoryLock {
        enabled: crate::config::load(&app).history_lock_enabled,
        locked: history_locked(&app),
    }
}

/// Turning the lock on needs nothing; turning it off needs the same
/// confirmation as opening History, so someone at your Mac can't simply
/// switch it off.
#[tauri::command(async)]
pub fn set_history_lock_enabled(app: AppHandle, enabled: bool) -> Result<HistoryLock, String> {
    let mut cfg = crate::config::load(&app);
    if cfg.history_lock_enabled && !enabled && history_locked(&app) {
        authenticate("turn off the History lock")?;
    }
    cfg.history_lock_enabled = enabled;
    crate::config::save(&app, &cfg)?;
    // Turning it on locks right away.
    app.state::<PrivacyState>().set_unlocked(!enabled);
    crate::applog!(
        "privacy: history lock {}",
        if enabled { "on" } else { "off" }
    );
    Ok(get_history_lock(app))
}

#[tauri::command(async)]
pub fn unlock_history(app: AppHandle) -> Result<HistoryLock, String> {
    if history_locked(&app) {
        authenticate("show your Mouthpiece history")?;
        app.state::<PrivacyState>().set_unlocked(true);
        crate::applog!("privacy: history unlocked");
    }
    Ok(get_history_lock(app))
}

#[tauri::command]
pub fn lock_history(app: AppHandle) -> HistoryLock {
    app.state::<PrivacyState>().set_unlocked(false);
    get_history_lock(app)
}

#[tauri::command]
pub fn get_log_transcripts(app: AppHandle) -> bool {
    crate::config::load(&app).log_transcripts
}

#[tauri::command]
pub fn set_log_transcripts(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut cfg = crate::config::load(&app);
    cfg.log_transcripts = enabled;
    crate::config::save(&app, &cfg)?;
    crate::logging::set_log_transcripts(enabled);
    crate::applog!(
        "privacy: transcript text in logs {}",
        if enabled { "on" } else { "off" }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to the real LocalAuthentication framework without showing a
    /// prompt: only asks whether this Mac can authenticate the owner.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a logged-in macOS session"]
    fn local_authentication_is_available() {
        use objc2_local_authentication::{LAContext, LAPolicy};
        let context = unsafe { LAContext::new() };
        let result =
            unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) };
        assert!(
            result.is_ok(),
            "{:?}",
            result.err().map(|e| e.localizedDescription())
        );
    }

    #[test]
    fn unlock_expires() {
        let state = PrivacyState::default();
        assert!(!state.is_unlocked());
        state.set_unlocked(true);
        assert!(state.is_unlocked());
        *state.unlocked_until.lock().unwrap() = Some(Instant::now() - Duration::from_secs(1));
        assert!(!state.is_unlocked());
        state.set_unlocked(true);
        state.set_unlocked(false);
        assert!(!state.is_unlocked());
    }
}
