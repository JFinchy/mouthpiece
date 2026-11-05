//! Local meeting call detection and auto-stop tracking.
//!
//! Uses `mic_usage::active_audio_holders` (CoreAudio's per-process input and
//! output list) to notice when a conferencing app (Zoom, Microsoft Teams,
//! Slack Huddles, Webex, FaceTime, Discord) or a browser call tab (Google
//! Meet, Teams Web, Zoom Web) is actively in a call, without any network or
//! calendar access.
//!
//! All decision logic lives in `AutoDetectTracker` and pure helper functions
//! so the state machine (confirmation streaks, per-call dismiss latch, and
//! auto-stop detection) is unit-tested without touching CoreAudio or AX.

use crate::mic_usage::AudioProcessHolder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Consecutive positive polls (~2s apart) required before showing a
/// "Start meeting?" prompt, so a momentary mic check doesn't flash the UI.
pub const CONFIRM_START_POLLS: u32 = 3;

/// Consecutive absent polls (~2s apart) required while a bound meeting is
/// recording before prompting that the call has ended.
pub const CONFIRM_STOP_POLLS: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConferencingPlatform {
    Zoom,
    GoogleMeet,
    MicrosoftTeams,
    Slack,
    Webex,
    FaceTime,
    Discord,
}

impl ConferencingPlatform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Zoom => "Zoom",
            Self::GoogleMeet => "Google Meet",
            Self::MicrosoftTeams => "Microsoft Teams",
            Self::Slack => "Slack Huddle",
            Self::Webex => "Webex",
            Self::FaceTime => "FaceTime",
            Self::Discord => "Discord",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedCall {
    pub platform: ConferencingPlatform,
    pub app_name: String,
    pub bundle_id: String,
    pub pid: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutoDetectAction {
    None,
    /// A call has been active for `CONFIRM_START_POLLS` polls while no
    /// meeting is recording.
    PromptStart(DetectedCall),
    /// A previously prompted call ended before the user started recording.
    ClearStartPrompt,
    /// A call that was active during the current meeting recording has ended.
    PromptStop(DetectedCall),
    /// The bound call came back (brief drop, rejoin) after `PromptStop` was
    /// emitted, so the "Call ended" prompt should be withdrawn.
    ClearStopPrompt,
}

/// Strips macOS helper suffixes (`.helper`, `.helper.renderer`, `.modulehost`,
/// etc.) and maps system media/WebKit daemons (`com.apple.WebKit.GPU`,
/// `com.apple.avconferenced`) back to their parent application's bundle
/// identifier.
pub fn canonical_bundle_id(bundle_id: &str) -> &str {
    let trimmed = bundle_id.trim();
    let lower = trimmed.to_ascii_lowercase();
    let stripped = if let Some(idx) = lower.find(".helper") {
        &trimmed[..idx]
    } else if let Some(prefix) = trimmed.strip_suffix(".modulehost") {
        prefix
    } else {
        trimmed
    };

    match stripped {
        "com.apple.WebKit.GPU" | "com.apple.WebKit.WebContent" | "com.apple.WebKit.Networking" => {
            "com.apple.Safari"
        }
        "com.apple.avconferenced" => "com.apple.FaceTime",
        other => other,
    }
}

/// Known native conferencing apps by canonical bundle ID.
pub fn classify_native_bundle(bundle_id: &str) -> Option<(ConferencingPlatform, &'static str)> {
    match canonical_bundle_id(bundle_id) {
        "us.zoom.xos" => Some((ConferencingPlatform::Zoom, "Zoom")),
        "com.microsoft.teams2" | "com.microsoft.teams" => {
            Some((ConferencingPlatform::MicrosoftTeams, "Microsoft Teams"))
        }
        "com.tinyspeck.slackmacgap" => Some((ConferencingPlatform::Slack, "Slack")),
        // `Cisco-Systems.Spark` is the current Webex app; the other is the
        // legacy Webex Meetings app.
        "Cisco-Systems.Spark" | "com.cisco.webexmeetingsapp" => {
            Some((ConferencingPlatform::Webex, "Webex"))
        }
        "com.apple.FaceTime" | "com.apple.avconferenced" => {
            Some((ConferencingPlatform::FaceTime, "FaceTime"))
        }
        "com.hnc.Discord" => Some((ConferencingPlatform::Discord, "Discord")),
        _ => None,
    }
}

/// Whether `bundle_id` belongs to a desktop web browser that can host Google
/// Meet, Microsoft Teams, Zoom Web, or Slack Huddles.
pub fn is_browser_bundle(bundle_id: &str) -> Option<&'static str> {
    match canonical_bundle_id(bundle_id) {
        "com.google.Chrome" | "com.google.Chrome.canary" => Some("Google Chrome"),
        "company.thebrowser.Browser" => Some("Arc"),
        "com.brave.Browser" => Some("Brave"),
        "com.microsoft.edgemac" => Some("Microsoft Edge"),
        "com.apple.Safari" | "com.apple.WebKit.GPU" | "com.apple.WebKit.WebContent" => {
            Some("Safari")
        }
        "org.mozilla.firefox" => Some("Firefox"),
        _ => None,
    }
}

/// Inspects browser window titles and URLs (from Accessibility) for an active
/// web call.
pub fn classify_browser_strings<S: AsRef<str>>(strings: &[S]) -> Option<ConferencingPlatform> {
    for raw in strings {
        let s = raw.as_ref().trim();
        if s.is_empty() {
            continue;
        }
        let lower = s.to_lowercase();
        if s.starts_with("Meet - ") || s.starts_with("Meet – ") || contains_google_meet_code(&lower)
        {
            return Some(ConferencingPlatform::GoogleMeet);
        }
        if lower.contains("teams.microsoft.com")
            || lower.contains("teams.live.com")
            || lower.ends_with("| microsoft teams")
        {
            return Some(ConferencingPlatform::MicrosoftTeams);
        }
        // The Zoom web client's tab is titled exactly "Zoom Meeting"; a bare
        // `contains` would match e.g. a search for "how to join a zoom meeting".
        if lower.contains("zoom.us/wc/") || lower == "zoom meeting" {
            return Some(ConferencingPlatform::Zoom);
        }
        if lower.contains("app.slack.com/huddle") {
            return Some(ConferencingPlatform::Slack);
        }
    }
    None
}

/// Matches a Google Meet code (`abc-defg-hij`) inside a window title that
/// also mentions "meet".
fn contains_google_meet_code(lower: &str) -> bool {
    if !lower.contains("meet") {
        return false;
    }
    lower
        .split(|c: char| !c.is_ascii_lowercase() && c != '-')
        .any(|token| {
            let parts: Vec<&str> = token.split('-').collect();
            parts.len() == 3
                && parts[0].len() == 3
                && parts[1].len() == 4
                && parts[2].len() == 3
                && parts
                    .iter()
                    .all(|p| p.chars().all(|c| c.is_ascii_lowercase()))
        })
}

/// Evaluates a snapshot of active CoreAudio holders (plus optional browser
/// window titles/URLs resolver for browser PIDs) to identify an active call.
///
/// Aggregates `is_input` and `is_output` across subprocesses that share the
/// same canonical bundle ID (e.g. Chrome's main process and audio helper).
pub fn detect_call_from_holders<F>(
    holders: &[AudioProcessHolder],
    browser_inspector: F,
) -> Option<DetectedCall>
where
    F: FnMut(&str, i32) -> Vec<String>,
{
    detect_call_from_holders_inner(holders, true, browser_inspector)
}

/// Like [`detect_call_from_holders`], but also matches conferencing apps or
/// browser call tabs that are only playing output audio (`is_output == true`,
/// e.g. when the local user is muted during an active meeting recording).
pub fn detect_call_from_holders_for_poller<F>(
    holders: &[AudioProcessHolder],
    browser_inspector: F,
) -> Option<DetectedCall>
where
    F: FnMut(&str, i32) -> Vec<String>,
{
    detect_call_from_holders_inner(holders, false, browser_inspector)
}

fn detect_call_from_holders_inner<F>(
    holders: &[AudioProcessHolder],
    require_input: bool,
    mut browser_inspector: F,
) -> Option<DetectedCall>
where
    F: FnMut(&str, i32) -> Vec<String>,
{
    #[derive(Default)]
    struct Group {
        pid: i32,
        is_input: bool,
        is_output: bool,
    }

    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for h in holders {
        let Some(raw_bundle) = h.bundle_id.as_deref() else {
            continue;
        };
        let canon = canonical_bundle_id(raw_bundle).to_string();
        let entry = groups.entry(canon.clone()).or_insert_with(|| {
            order.push(canon);
            Group {
                pid: h.pid,
                is_input: false,
                is_output: false,
            }
        });
        entry.is_input |= h.is_input;
        entry.is_output |= h.is_output;
    }

    let eligible = |g: &Group| -> bool {
        if require_input {
            g.is_input
        } else {
            g.is_input || g.is_output
        }
    };

    // 1. Prefer native conferencing apps holding audio.
    for bundle_id in &order {
        let g = &groups[bundle_id];
        if !eligible(g) {
            continue;
        }
        if let Some((platform, app_name)) = classify_native_bundle(bundle_id) {
            return Some(DetectedCall {
                platform,
                app_name: app_name.to_string(),
                bundle_id: bundle_id.clone(),
                pid: g.pid,
            });
        }
    }

    // 2. Check browsers holding audio. A browser call both captures the mic
    // and plays the call; a tab that only uses the mic (voice search, a
    // dictation site) doesn't, even with a Meet/Teams tab open elsewhere.
    for bundle_id in &order {
        let g = &groups[bundle_id];
        let browser_eligible = if require_input {
            g.is_input && g.is_output
        } else {
            g.is_input || g.is_output
        };
        if !browser_eligible {
            continue;
        }
        if let Some(browser_name) = is_browser_bundle(bundle_id) {
            let strings = browser_inspector(bundle_id, g.pid);
            if let Some(platform) = classify_browser_strings(&strings) {
                return Some(DetectedCall {
                    platform,
                    app_name: browser_name.to_string(),
                    bundle_id: bundle_id.clone(),
                    pid: g.pid,
                });
            }
        }
    }

    None
}

/// Pure state machine tracking call start/stop streaks and user dismissals.
#[derive(Default, Debug)]
pub struct AutoDetectTracker {
    /// Current candidate call and how many consecutive polls it has been seen.
    candidate: Option<(DetectedCall, u32)>,
    /// Whether a start prompt is currently shown in the UI.
    start_prompt_shown: Option<DetectedCall>,
    /// Call platform + bundle_id the user dismissed, suppressed until the call ends.
    dismissed: Option<(ConferencingPlatform, String)>,
    /// Call bound to the currently-recording meeting, for auto-stop detection.
    bound_recording_call: Option<DetectedCall>,
    /// Consecutive polls where `bound_recording_call` was absent after being bound.
    absent_polls: u32,
    /// Whether the stop prompt has already been emitted for the current recording.
    stop_prompt_emitted: bool,
}

impl AutoDetectTracker {
    /// Feeds one poll observation (`detected_call`, whether a meeting is
    /// currently recording, and whether system audio has been silent) and
    /// returns any UI action to take.
    pub fn observe(
        &mut self,
        detected: Option<DetectedCall>,
        meeting_recording: bool,
        system_audio_silent: bool,
    ) -> AutoDetectAction {
        if meeting_recording {
            // Clear any pre-recording start prompt once recording is underway.
            let prior_prompted = self.start_prompt_shown.take();
            let cleared = prior_prompted.is_some();
            self.candidate = None;

            if let Some(call) = detected {
                // Bind to the call if not yet bound, or reset the absence counter
                // if the bound call is still active.
                if self.bound_recording_call.is_none() {
                    self.bound_recording_call = Some(call);
                }
                self.absent_polls = 0;
                if self.stop_prompt_emitted {
                    // The call came back after a brief drop: withdraw the
                    // "Call ended" prompt and re-arm it for the real end.
                    self.stop_prompt_emitted = false;
                    return AutoDetectAction::ClearStopPrompt;
                }
                return if cleared {
                    AutoDetectAction::ClearStartPrompt
                } else {
                    AutoDetectAction::None
                };
            }

            if self.bound_recording_call.is_none() {
                self.bound_recording_call = prior_prompted;
            }

            if let Some(bound) = &self.bound_recording_call {
                if system_audio_silent && !self.stop_prompt_emitted {
                    self.absent_polls += 1;
                    if self.absent_polls >= CONFIRM_STOP_POLLS {
                        self.stop_prompt_emitted = true;
                        return AutoDetectAction::PromptStop(bound.clone());
                    }
                } else if !system_audio_silent {
                    self.absent_polls = 0;
                }
            }

            return if cleared {
                AutoDetectAction::ClearStartPrompt
            } else {
                AutoDetectAction::None
            };
        }

        // Not recording a meeting: reset recording-bound state. If a recording
        // just ended while the bound call is still ongoing, latch `dismissed`
        // so we don't re-prompt "Record <App>?" for the same call 6s later.
        if let Some(bound) = self.bound_recording_call.take() {
            self.dismissed = Some((bound.platform, bound.bundle_id));
        }
        self.absent_polls = 0;
        self.stop_prompt_emitted = false;

        match detected {
            Some(call) => {
                let key = (call.platform, call.bundle_id.clone());
                if self.dismissed.as_ref() == Some(&key) {
                    return AutoDetectAction::None;
                }
                if self.start_prompt_shown.as_ref().is_some_and(|shown| {
                    shown.platform == call.platform && shown.bundle_id == call.bundle_id
                }) {
                    return AutoDetectAction::None;
                }

                let count = match &mut self.candidate {
                    Some((prev, n))
                        if prev.platform == call.platform && prev.bundle_id == call.bundle_id =>
                    {
                        *n += 1;
                        *n
                    }
                    _ => {
                        self.candidate = Some((call.clone(), 1));
                        1
                    }
                };

                if count >= CONFIRM_START_POLLS {
                    self.start_prompt_shown = Some(call.clone());
                    AutoDetectAction::PromptStart(call)
                } else {
                    AutoDetectAction::None
                }
            }
            None => {
                self.candidate = None;
                self.dismissed = None;
                if self.start_prompt_shown.take().is_some() {
                    AutoDetectAction::ClearStartPrompt
                } else {
                    AutoDetectAction::None
                }
            }
        }
    }

    /// Suppresses the start prompt for the currently detected call until that
    /// call ends.
    pub fn dismiss_start_prompt(&mut self) {
        if let Some(call) = self
            .start_prompt_shown
            .take()
            .or_else(|| self.candidate.take().map(|(c, _)| c))
        {
            self.dismissed = Some((call.platform, call.bundle_id));
        }
    }

    /// Forgets all call/recording state (used while auto-detect is switched
    /// off, so a stale binding or latch doesn't leak into the next session).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Returns the call for which a start prompt is currently shown in the UI.
    pub fn prompted_call(&self) -> Option<&DetectedCall> {
        self.start_prompt_shown.as_ref()
    }
}

/// Whether an Accessibility element is a call's leave/hang-up control:
/// Meet "Leave call", Teams/Zoom web "Leave", Slack "Leave huddle",
/// generic "Hang up" / "End call". Only consulted inside windows whose
/// title or URL already looks like a call page.
pub fn is_leave_control(role: &str, label: &str) -> bool {
    if role != "AXButton" && role != "AXMenuButton" {
        return false;
    }
    let l = label.trim().to_lowercase();
    l == "leave"
        || l.starts_with("leave call")
        || l.starts_with("leave meeting")
        || l.starts_with("leave huddle")
        || l.starts_with("leave (")
        || l.starts_with("hang up")
        || l.starts_with("end call")
        || l.starts_with("end meeting")
}

/// What a walk of a call-looking browser window found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallControls {
    /// A leave/hang-up button: the page is in a call.
    Found,
    /// The page's content was fully visible and has no such button: a
    /// landing page, chat, or calendar, not a call.
    Absent,
    /// Page content isn't exposed to Accessibility (Chrome builds it only
    /// on request) or the walk ran out of budget: fall back to the title.
    Unknown,
}

/// Reads window titles and document URLs of a browser process via macOS
/// Accessibility, bounded with a short timeout so a hung browser never stalls
/// the detector thread.
#[cfg(target_os = "macos")]
pub fn inspect_browser_windows_ax(bundle_id: &str, pid: i32) -> Vec<String> {
    inspect_browser_windows(bundle_id, pid, false)
}

/// Like [`inspect_browser_windows_ax`], but a window whose page is fully
/// visible to Accessibility and has no leave/hang-up button is skipped, so
/// a Teams chat or the Meet landing page doesn't count as a call.
#[cfg(target_os = "macos")]
pub fn inspect_browser_call_windows_ax(bundle_id: &str, pid: i32) -> Vec<String> {
    inspect_browser_windows(bundle_id, pid, true)
}

#[cfg(target_os = "macos")]
fn inspect_browser_windows(bundle_id: &str, pid: i32, confirm_controls: bool) -> Vec<String> {
    use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
    use core_foundation::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
    use core_foundation::url::{CFURLGetString, CFURLGetTypeID, CFURLRef};
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;
    use std::ffi::c_void;

    type AXUIElementRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> i32;
    }

    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }

    fn copy_attr(element: AXUIElementRef, name: &str) -> Option<Owned> {
        let attr = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
        };
        (status == 0 && !value.is_null()).then(|| Owned(value))
    }

    fn as_string(value: &Owned) -> Option<String> {
        unsafe {
            let ty = CFGetTypeID(value.0);
            if ty == CFStringGetTypeID() {
                return Some(CFString::wrap_under_get_rule(value.0 as CFStringRef).to_string());
            }
            if ty == CFURLGetTypeID() {
                let url_str = CFURLGetString(value.0 as CFURLRef);
                if !url_str.is_null() {
                    return Some(CFString::wrap_under_get_rule(url_str).to_string());
                }
            }
            None
        }
    }

    fn children(element: AXUIElementRef) -> Vec<Owned> {
        let Some(array) = copy_attr(element, "AXChildren") else {
            return Vec::new();
        };
        unsafe {
            if CFGetTypeID(array.0) != CFArrayGetTypeID() {
                return Vec::new();
            }
            let count = CFArrayGetCount(array.0 as _).min(96);
            (0..count)
                .filter_map(|i| {
                    let child = CFArrayGetValueAtIndex(array.0 as _, i);
                    (!child.is_null()).then(|| {
                        CFRetain(child);
                        AXUIElementSetMessagingTimeout(child, 0.25);
                        Owned(child)
                    })
                })
                .collect()
        }
    }

    fn label(element: AXUIElementRef, attr: &str) -> String {
        copy_attr(element, attr)
            .and_then(|v| as_string(&v))
            .unwrap_or_default()
    }

    /// Breadth-first walk for a leave/hang-up button, bounded in nodes,
    /// depth, and wall-clock time; anything inconclusive is `Unknown`.
    fn find_call_controls(window: AXUIElementRef) -> CallControls {
        use std::collections::VecDeque;
        const MAX_NODES: usize = 4000;
        const MAX_DEPTH: usize = 48;
        const BUDGET: std::time::Duration = std::time::Duration::from_millis(500);
        let deadline = std::time::Instant::now() + BUDGET;
        let mut saw_web_area = false;
        let mut visited = 0usize;
        let mut queue: VecDeque<(Owned, usize)> =
            children(window).into_iter().map(|c| (c, 1)).collect();
        while let Some((node, depth)) = queue.pop_front() {
            visited += 1;
            if visited > MAX_NODES || std::time::Instant::now() >= deadline {
                return CallControls::Unknown;
            }
            let role = label(node.0, "AXRole");
            if role == "AXWebArea" {
                saw_web_area = true;
            }
            if role == "AXButton" || role == "AXMenuButton" {
                for attr in ["AXTitle", "AXDescription", "AXHelp"] {
                    if is_leave_control(&role, &label(node.0, attr)) {
                        return CallControls::Found;
                    }
                }
            }
            if depth < MAX_DEPTH {
                queue.extend(children(node.0).into_iter().map(|c| (c, depth + 1)));
            }
        }
        // A fully walked tree with no web content means Chrome hasn't built
        // its page tree: we can't tell, so don't veto the call.
        if saw_web_area {
            CallControls::Absent
        } else {
            CallControls::Unknown
        }
    }

    // Verdicts per (pid, window title) for a minute: detection polls every
    // 2 s, and the walk is the expensive part.
    type Cache = std::sync::Mutex<Vec<((i32, String), std::time::Instant, CallControls)>>;
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);
    fn cached_controls(key: &(i32, String)) -> Option<CallControls> {
        let cache = CACHE.get_or_init(Default::default);
        let mut entries = cache.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|(_, at, _)| at.elapsed() < TTL);
        entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, v)| *v)
    }
    fn store_controls(key: (i32, String), verdict: CallControls) {
        let cache = CACHE.get_or_init(Default::default);
        let mut entries = cache.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|(k, _, _)| *k != key);
        entries.push((key, std::time::Instant::now(), verdict));
    }

    // If the PID from CoreAudio is a browser helper process, also check the
    // main browser application's PID via NSRunningApplication inside an autoreleasepool.
    let mut pids = vec![pid];
    let canon = canonical_bundle_id(bundle_id);
    objc2::rc::autoreleasepool(|_| {
        let ns_id = NSString::from_str(canon);
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&ns_id);
        for app in apps.iter() {
            let main_pid = app.processIdentifier();
            if main_pid > 0 && !pids.contains(&main_pid) {
                pids.insert(0, main_pid);
            }
        }
    });

    let mut out = Vec::new();
    for target_pid in pids {
        unsafe {
            let app = AXUIElementCreateApplication(target_pid);
            if app.is_null() {
                continue;
            }
            let app = Owned(app);
            AXUIElementSetMessagingTimeout(app.0, 0.4);
            let Some(windows) = copy_attr(app.0, "AXWindows") else {
                continue;
            };
            if CFGetTypeID(windows.0) != CFArrayGetTypeID() {
                continue;
            }
            let count = CFArrayGetCount(windows.0 as _).min(16);
            for i in 0..count {
                let win = CFArrayGetValueAtIndex(windows.0 as _, i);
                if win.is_null() {
                    continue;
                }
                CFRetain(win);
                let win = Owned(win);
                // Messaging timeouts are per element: without this, reads on
                // a hung browser's windows use the ~6s system default.
                AXUIElementSetMessagingTimeout(win.0, 0.4);
                let mut strings = Vec::new();
                for attr in ["AXTitle", "AXDocument", "AXURL"] {
                    if let Some(val) = copy_attr(win.0, attr).and_then(|v| as_string(&v)) {
                        if !val.trim().is_empty() {
                            strings.push(val);
                        }
                    }
                }
                if confirm_controls && classify_browser_strings(&strings).is_some() {
                    let key = (target_pid, strings.first().cloned().unwrap_or_default());
                    let verdict = cached_controls(&key).unwrap_or_else(|| {
                        let v = find_call_controls(win.0);
                        store_controls(key, v);
                        v
                    });
                    if verdict == CallControls::Absent {
                        continue;
                    }
                }
                out.extend(strings);
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

#[cfg(not(target_os = "macos"))]
pub fn inspect_browser_windows_ax(_bundle_id: &str, _pid: i32) -> Vec<String> {
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
pub fn inspect_browser_call_windows_ax(_bundle_id: &str, _pid: i32) -> Vec<String> {
    Vec::new()
}

/// Checks live CoreAudio process holders and (for browsers) Accessibility
/// window titles to detect an active call on this Mac.
pub fn detect_active_call() -> Option<DetectedCall> {
    let holders = crate::mic_usage::active_audio_holders()?;
    detect_call_from_holders(&holders, inspect_browser_call_windows_ax)
}

/// Checks live CoreAudio process holders for an active call during an ongoing
/// meeting recording (permitting output-only audio holders when the local user
/// is muted in the call app).
pub fn detect_active_call_for_poller() -> Option<DetectedCall> {
    let holders = crate::mic_usage::active_audio_holders()?;
    detect_call_from_holders_for_poller(&holders, inspect_browser_windows_ax)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_bundle_id_strips_helper_suffixes() {
        assert_eq!(
            canonical_bundle_id("com.google.Chrome.helper.renderer"),
            "com.google.Chrome"
        );
        assert_eq!(
            canonical_bundle_id("com.tinyspeck.slackmacgap.helper"),
            "com.tinyspeck.slackmacgap"
        );
        assert_eq!(
            canonical_bundle_id("com.microsoft.teams2.modulehost"),
            "com.microsoft.teams2"
        );
        assert_eq!(
            canonical_bundle_id("com.apple.WebKit.GPU"),
            "com.apple.Safari"
        );
        assert_eq!(
            canonical_bundle_id("com.apple.avconferenced"),
            "com.apple.FaceTime"
        );
        assert_eq!(canonical_bundle_id("us.zoom.xos"), "us.zoom.xos");
    }

    #[test]
    fn native_conferencing_app_is_detected_when_mic_is_active() {
        let holders = vec![AudioProcessHolder {
            pid: 4200,
            bundle_id: Some("us.zoom.xos".into()),
            is_input: true,
            is_output: true,
        }];
        let detected = detect_call_from_holders(&holders, |_, _| Vec::new());
        assert_eq!(
            detected,
            Some(DetectedCall {
                platform: ConferencingPlatform::Zoom,
                app_name: "Zoom".into(),
                bundle_id: "us.zoom.xos".into(),
                pid: 4200,
            })
        );
    }

    #[test]
    fn idle_zoom_with_only_output_does_not_false_trigger() {
        let holders = vec![AudioProcessHolder {
            pid: 4200,
            bundle_id: Some("us.zoom.xos".into()),
            is_input: false,
            is_output: true,
        }];
        assert_eq!(detect_call_from_holders(&holders, |_, _| Vec::new()), None);
    }

    #[test]
    fn browser_holding_mic_requires_call_window_title_or_url() {
        let holders = vec![AudioProcessHolder {
            pid: 5100,
            bundle_id: Some("com.google.Chrome.helper".into()),
            is_input: true,
            is_output: true,
        }];
        // Ordinary voice search / non-call tab: not a meeting.
        assert_eq!(
            detect_call_from_holders(&holders, |_, _| vec!["GitHub - Pull Requests".into()]),
            None
        );
        // Google Meet tab title with code: detected as Google Meet.
        let meet = detect_call_from_holders(&holders, |_, _| {
            vec!["Meet - abc-defg-hij - Google Chrome".into()]
        });
        assert_eq!(
            meet,
            Some(DetectedCall {
                platform: ConferencingPlatform::GoogleMeet,
                app_name: "Google Chrome".into(),
                bundle_id: "com.google.Chrome".into(),
                pid: 5100,
            })
        );
        // Microsoft Teams web tab title: detected as Microsoft Teams.
        let teams = detect_call_from_holders(&holders, |_, _| {
            vec!["Sprint Planning | Microsoft Teams".into()]
        });
        assert_eq!(
            teams.map(|c| c.platform),
            Some(ConferencingPlatform::MicrosoftTeams)
        );
    }

    #[test]
    fn tracker_prompts_start_after_confirm_streak_and_respects_dismiss() {
        let mut tracker = AutoDetectTracker::default();
        let zoom = DetectedCall {
            platform: ConferencingPlatform::Zoom,
            app_name: "Zoom".into(),
            bundle_id: "us.zoom.xos".into(),
            pid: 100,
        };

        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::None
        );
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::None
        );
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::PromptStart(zoom.clone())
        );
        // Doesn't re-emit while the prompt is already up.
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::None
        );

        // User dismisses the prompt: stays quiet for the rest of this call.
        tracker.dismiss_start_prompt();
        for _ in 0..5 {
            assert_eq!(
                tracker.observe(Some(zoom.clone()), false, false),
                AutoDetectAction::None
            );
        }

        // Once the call ends, the dismiss latch clears so the next call prompts again.
        assert_eq!(tracker.observe(None, false, true), AutoDetectAction::None);
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::None
        );
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::None
        );
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::PromptStart(zoom)
        );
    }

    #[test]
    fn tracker_prompts_stop_when_bound_call_ends_and_system_audio_is_silent() {
        let mut tracker = AutoDetectTracker::default();
        let meet = DetectedCall {
            platform: ConferencingPlatform::GoogleMeet,
            app_name: "Google Chrome".into(),
            bundle_id: "com.google.Chrome".into(),
            pid: 200,
        };

        // Recording starts while on a Google Meet call.
        assert_eq!(
            tracker.observe(Some(meet.clone()), true, false),
            AutoDetectAction::None
        );

        // Call drops, but system audio is still playing: don't prompt stop yet.
        for _ in 0..CONFIRM_STOP_POLLS {
            assert_eq!(tracker.observe(None, true, false), AutoDetectAction::None);
        }

        // Call is gone and system audio is silent for CONFIRM_STOP_POLLS ticks.
        for _ in 0..(CONFIRM_STOP_POLLS - 1) {
            assert_eq!(tracker.observe(None, true, true), AutoDetectAction::None);
        }
        assert_eq!(
            tracker.observe(None, true, true),
            AutoDetectAction::PromptStop(meet)
        );
        // Emitted only once per recording.
        assert_eq!(tracker.observe(None, true, true), AutoDetectAction::None);
    }

    #[test]
    fn manually_stopping_recording_mid_call_does_not_reprompt_same_call() {
        let mut tracker = AutoDetectTracker::default();
        let zoom = DetectedCall {
            platform: ConferencingPlatform::Zoom,
            app_name: "Zoom".into(),
            bundle_id: "us.zoom.xos".into(),
            pid: 300,
        };

        // Call is bound while recording.
        assert_eq!(
            tracker.observe(Some(zoom.clone()), true, false),
            AutoDetectAction::None
        );

        // User stops the recording manually while the Zoom call is still active.
        for _ in 0..5 {
            assert_eq!(
                tracker.observe(Some(zoom.clone()), false, false),
                AutoDetectAction::None
            );
            assert_eq!(tracker.prompted_call(), None);
        }

        // Once the Zoom call ends, a subsequent call will prompt again.
        assert_eq!(tracker.observe(None, false, true), AutoDetectAction::None);
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::None
        );
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::None
        );
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::PromptStart(zoom.clone())
        );
        assert_eq!(tracker.prompted_call(), Some(&zoom));
    }

    fn zoom_call() -> DetectedCall {
        DetectedCall {
            platform: ConferencingPlatform::Zoom,
            app_name: "Zoom".into(),
            bundle_id: "us.zoom.xos".into(),
            pid: 4200,
        }
    }

    #[test]
    fn stop_prompt_is_withdrawn_and_rearmed_when_call_returns() {
        let mut tracker = AutoDetectTracker::default();
        let zoom = zoom_call();
        assert_eq!(
            tracker.observe(Some(zoom.clone()), true, false),
            AutoDetectAction::None
        );
        for _ in 0..CONFIRM_STOP_POLLS - 1 {
            assert_eq!(tracker.observe(None, true, true), AutoDetectAction::None);
        }
        assert_eq!(
            tracker.observe(None, true, true),
            AutoDetectAction::PromptStop(zoom.clone())
        );
        // Call drops back in (rejoin): the prompt is withdrawn...
        assert_eq!(
            tracker.observe(Some(zoom.clone()), true, false),
            AutoDetectAction::ClearStopPrompt
        );
        assert_eq!(
            tracker.observe(Some(zoom.clone()), true, false),
            AutoDetectAction::None
        );
        // ...and fires again when the call really ends.
        for _ in 0..CONFIRM_STOP_POLLS - 1 {
            assert_eq!(tracker.observe(None, true, true), AutoDetectAction::None);
        }
        assert_eq!(
            tracker.observe(None, true, true),
            AutoDetectAction::PromptStop(zoom)
        );
    }

    #[test]
    fn reset_drops_stale_recording_binding() {
        let mut tracker = AutoDetectTracker::default();
        let zoom = zoom_call();
        tracker.observe(Some(zoom.clone()), true, false);
        // Auto-detect switched off mid-recording, recording ends, then a new
        // call starts on the same app after re-enabling: it must still prompt.
        tracker.reset();
        for _ in 0..CONFIRM_START_POLLS - 1 {
            assert_eq!(
                tracker.observe(Some(zoom.clone()), false, false),
                AutoDetectAction::None
            );
        }
        assert_eq!(
            tracker.observe(Some(zoom.clone()), false, false),
            AutoDetectAction::PromptStart(zoom)
        );
    }

    #[test]
    fn current_webex_app_is_detected() {
        assert_eq!(
            classify_native_bundle("Cisco-Systems.Spark").map(|(p, _)| p),
            Some(ConferencingPlatform::Webex)
        );
    }

    #[test]
    fn browser_pages_that_merely_mention_a_call_service_are_not_calls() {
        for title in [
            "how to join a zoom meeting - Google Search",
            "https://meet.google.com/landing",
            "Google Meet",
        ] {
            assert_eq!(classify_browser_strings(&[title]), None, "{title}");
        }
        assert_eq!(
            classify_browser_strings(&["https://meet.google.com/abc-defg-hij"]),
            Some(ConferencingPlatform::GoogleMeet)
        );
        assert_eq!(
            classify_browser_strings(&["Zoom Meeting"]),
            Some(ConferencingPlatform::Zoom)
        );
        assert_eq!(
            classify_browser_strings(&["https://app.zoom.us/wc/123456789/join"]),
            Some(ConferencingPlatform::Zoom)
        );
    }

    #[test]
    fn browser_using_only_the_mic_is_not_a_call() {
        // A voice-search tab holds the mic; a Meet tab is open but silent.
        let holders = vec![AudioProcessHolder {
            pid: 5100,
            bundle_id: Some("com.google.Chrome.helper".into()),
            is_input: true,
            is_output: false,
        }];
        let inspector = |_: &str, _: i32| vec!["Meet - abc-defg-hij - Google Chrome".to_string()];
        assert_eq!(detect_call_from_holders(&holders, inspector), None);
        // Once recording, output alone still keeps the call alive (muted user).
        assert!(detect_call_from_holders_for_poller(&holders, inspector).is_some());
    }

    #[test]
    fn leave_controls_are_recognised_and_lookalikes_are_not() {
        for label in [
            "Leave call",
            "Leave",
            "Leave (⌘⇧H)",
            "Leave huddle",
            "Leave meeting",
            "Hang up",
            "End call",
        ] {
            assert!(is_leave_control("AXButton", label), "{label}");
        }
        for label in [
            "Leave a comment",
            "Leaves",
            "Join now",
            "Ask to join",
            "Calendar",
        ] {
            assert!(!is_leave_control("AXButton", label), "{label}");
        }
        assert!(!is_leave_control("AXStaticText", "Leave call"));
    }
}
