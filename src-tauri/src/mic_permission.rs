//! Microphone permission (macOS privacy, TCC). macOS only shows the
//! "allow microphone" prompt to an app whose Info.plist has an
//! `NSMicrophoneUsageDescription`; without one, or once access is denied,
//! the input stream still opens but delivers nothing but zeros. So the
//! status is checked explicitly, shown in the setup checklist, and used to
//! explain a silent recording.

use serde::Serialize;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MicAccess {
    /// Never asked; macOS prompts on the first request.
    NotDetermined,
    /// Blocked by a device-management profile (MDM), not by the user.
    Restricted,
    Denied,
    Authorized,
}

#[cfg(target_os = "macos")]
pub fn status() -> MicAccess {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return MicAccess::Authorized;
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::NotDetermined => MicAccess::NotDetermined,
        AVAuthorizationStatus::Restricted => MicAccess::Restricted,
        AVAuthorizationStatus::Denied => MicAccess::Denied,
        _ => MicAccess::Authorized,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> MicAccess {
    MicAccess::Authorized
}

/// Shows macOS's microphone prompt if it hasn't been answered yet, and
/// waits for the answer. Returns the resulting status.
#[cfg(target_os = "macos")]
pub fn request() -> MicAccess {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
    if status() != MicAccess::NotDetermined {
        return status();
    }
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return status();
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let handler = RcBlock::new(move |granted: Bool| {
        let _ = tx.send(granted.as_bool());
    });
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler) };
    let _ = rx.recv_timeout(std::time::Duration::from_secs(120));
    status()
}

#[cfg(not(target_os = "macos"))]
pub fn request() -> MicAccess {
    MicAccess::Authorized
}

/// What to tell the user when a recording came back silent, if the cause
/// is permission rather than a muted or broken mic.
pub fn silence_explanation(access: MicAccess) -> Option<&'static str> {
    match access {
        MicAccess::Authorized => None,
        MicAccess::NotDetermined => {
            Some("macOS hasn't given Mouthpiece the microphone yet. Allow it in Settings > Setup.")
        }
        MicAccess::Denied => Some(
            "macOS is blocking the microphone. Turn on Mouthpiece in System Settings > \
             Privacy & Security > Microphone.",
        ),
        MicAccess::Restricted => {
            Some("Your organization's device management blocks microphone access for Mouthpiece.")
        }
    }
}

/// The installed app's own Info.plist has the usage description macOS
/// requires before it will prompt. Always true for a correct build; a
/// missing key is the bug this module exists for, so the checklist says so.
#[cfg(target_os = "macos")]
pub fn usage_description_present() -> bool {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::mainBundle();
    let key = NSString::from_str("NSMicrophoneUsageDescription");
    bundle.objectForInfoDictionaryKey(&key).is_some()
}

#[cfg(not(target_os = "macos"))]
pub fn usage_description_present() -> bool {
    true
}

#[tauri::command(async)]
pub fn request_microphone_access() -> MicAccess {
    let result = request();
    crate::applog!("mic: access request answered: {result:?}");
    result
}

#[tauri::command]
pub fn open_microphone_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_only_blamed_on_permission_when_access_is_missing() {
        assert_eq!(silence_explanation(MicAccess::Authorized), None);
        for access in [
            MicAccess::NotDetermined,
            MicAccess::Denied,
            MicAccess::Restricted,
        ] {
            assert!(silence_explanation(access).is_some());
        }
    }

    /// Asks the real framework (no prompt): the call must not crash.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs a logged-in macOS session"]
    fn status_can_be_read() {
        let _ = status();
    }
}
