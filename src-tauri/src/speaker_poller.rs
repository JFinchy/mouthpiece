//! Offline active-speaker observation via macOS Accessibility (`AXUIElement`).
//!
//! During a call on Zoom, Google Meet, Microsoft Teams, or Slack Huddles, the
//! conferencing app visually highlights the active speaker's video tile right
//! next to their display name (and marks muted participants with a muted icon).
//! Because Mouthpiece already has Accessibility permission for pasting, we can
//! poll those participant tiles locally in the background — without screenshots,
//! Screen Recording permission, or network calls — and fuse the timestamped
//! active-speaker observations with `meeting.rs`'s acoustic voice clusters
//! after the call ends.
//!
//! Uses a two-tier anchored polling strategy (modeled after Wispr Flow's
//! `MeetingSpeakerPoller`):
//! 1. **Discovery pass (every ~5s or when unbound):** Walks the conferencing
//!    window's AX tree (bounded to 1,500 nodes) to locate participant tile
//!    roots, extract each participant's display name, and retain direct
//!    `AXUIElementRef` handles (`CachedTile`).
//! 2. **Compact poll (every 300ms):** Reads only the cached tile handles to
//!    check for active-speaker and mute markers, recording a
//!    `SpeakerObservation` whenever the speaking or muted set changes.

use crate::meeting_detect::ConferencingPlatform;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Google Meet DOM class on each participant's video tile root element.
pub const GOOGLE_MEET_TILE_ROOT_CLASSES: &[&str] = &["oZRSLe"];
/// Google Meet DOM class on the active-speaker indicator inside a tile.
pub const GOOGLE_MEET_MARKER_CLASSES: &[&str] = &["kssMZb"];

/// Slack Huddle DOM class on each peer tile root element.
pub const SLACK_TILE_ROOT_CLASSES: &[&str] = &["p-huddle_peer_tile"];
/// Slack Huddle DOM class on the active-speaker overlay element.
pub const SLACK_MARKER_CLASSES: &[&str] = &["p-huddle_peer_tile__overlay--active_speaker"];

/// Microsoft Teams DOM class on video tile frame root elements.
pub const TEAMS_TILE_ROOT_CLASSES: &[&str] = &["vdi-frame-occlusion"];

/// Interval between fast compact polls of cached participant tiles.
const COMPACT_POLL_INTERVAL: Duration = Duration::from_millis(300);
/// Interval between full AX tree discovery refreshes to pick up new joiners.
const DISCOVERY_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
/// How often to retry discovery while no tile session is anchored (no call
/// window, in-room meeting, or tiles not found). Each pass walks up to
/// thousands of AX nodes, so this must not run at the 300ms poll rate.
const DISCOVERY_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// One timestamped change in the set of active or muted speakers on screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerObservation {
    /// Milliseconds since the meeting recording started.
    pub offset_ms: u64,
    /// Display names of participants whose tiles currently show an active
    /// speaking indicator.
    pub speaking_names: Vec<String>,
    /// Display names of participants whose tiles currently show a muted
    /// microphone indicator (negative evidence during diarization fusion).
    pub muted_names: Vec<String>,
}

/// Snapshot of a participant tile's attributes extracted from Accessibility,
/// kept pure so platform classification rules are unit-testable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TileNodeData {
    pub role: String,
    pub title: String,
    pub description: String,
    pub value: String,
    pub dom_classes: Vec<String>,
    pub texts: Vec<String>,
}

/// Evaluated state of a single participant tile on a poll tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatedTile {
    pub name: String,
    pub is_speaking: bool,
    pub is_muted: bool,
    pub is_self: bool,
}

/// Whether `dom_classes` contains any class in `targets`.
pub fn has_any_class(dom_classes: &[String], targets: &[&str]) -> bool {
    dom_classes.iter().any(|c| targets.iter().any(|t| c == t))
}

/// Whether `raw` refers to the local user's own tile ("You", "Alice (You)", etc.).
pub fn is_self_label(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("you") || trimmed.eq_ignore_ascii_case("me") {
        return true;
    }
    let lower = trimmed.to_lowercase();
    lower.ends_with(" (you)")
        || lower.ends_with("(you)")
        || lower.ends_with(", you")
        || lower.starts_with("you ")
        || lower.contains(" (you, ")
}

/// Cleans status suffixes and role badges ("(Host)", ", muted", ", speaking",
/// etc.) out of a raw AX tile label so only the participant's human name
/// remains.
pub fn clean_participant_name(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return None;
    }

    // Strip common status suffixes after a comma or dash ("Alice Chen, speaking, unmuted").
    for sep in [", ", " - ", " – ", " — ", " · "] {
        if let Some((head, tail)) = s.split_once(sep) {
            let tail_lower = tail.to_lowercase();
            if tail_lower.contains("speaking")
                || tail_lower.contains("talking")
                || tail_lower.contains("active speaker")
                || tail_lower.contains("muted")
                || tail_lower.contains("unmuted")
                || tail_lower.contains("microphone")
                || tail_lower.contains("camera")
                || tail_lower.contains("video")
                || tail_lower.contains("organizer")
                || tail_lower.contains("presenter")
            {
                s = head.trim().to_string();
            }
        }
    }

    // Strip parenthetical badges at the end.
    for badge in [
        "(host)",
        "(co-host)",
        "(guest)",
        "(external)",
        "(organizer)",
        "(presenter)",
        "(speaking)",
        "(talking)",
        "(muted)",
        "(unmuted)",
    ] {
        if s.to_lowercase().ends_with(badge) {
            let new_len = s.len().saturating_sub(badge.len());
            s = s[..new_len].trim().to_string();
        }
    }

    // Collapse internal whitespace runs.
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || collapsed.len() > 80 {
        return None;
    }
    // Reject strings that are purely UI chrome rather than a person's name.
    let lower = collapsed.to_lowercase();
    const CHROME_LABELS: &[&str] = &[
        "mute",
        "unmute",
        "muted",
        "unmuted",
        "speaking",
        "talking",
        "active speaker",
        "turn on microphone",
        "turn off microphone",
        "turn on camera",
        "turn off camera",
        "share screen",
        "leave call",
        "leave meeting",
        "more options",
        "participants",
        "chat",
        "reactions",
        "raise hand",
        "pin",
        "unpin",
    ];
    if CHROME_LABELS.contains(&lower.as_str()) {
        return None;
    }
    Some(collapsed)
}

/// Evaluates a participant tile subtree (`root` + flattened descendant nodes)
/// for a given conferencing platform.
pub fn evaluate_tile_subtree(
    platform: ConferencingPlatform,
    root: &TileNodeData,
    descendants: &[TileNodeData],
) -> Option<EvaluatedTile> {
    let mut all_classes: Vec<String> = root.dom_classes.clone();
    let mut combined_meta = format!("{} {} {}", root.title, root.description, root.value);
    let mut candidate_names: Vec<String> = Vec::new();

    if !root.title.trim().is_empty() {
        candidate_names.push(root.title.clone());
    }
    if !root.description.trim().is_empty() {
        candidate_names.push(root.description.clone());
    }
    candidate_names.extend(root.texts.iter().cloned());

    for d in descendants {
        all_classes.extend(d.dom_classes.iter().cloned());
        if !d.title.is_empty() {
            combined_meta.push(' ');
            combined_meta.push_str(&d.title);
            candidate_names.push(d.title.clone());
        }
        if !d.description.is_empty() {
            combined_meta.push(' ');
            combined_meta.push_str(&d.description);
            candidate_names.push(d.description.clone());
        }
        if !d.value.is_empty() {
            combined_meta.push(' ');
            combined_meta.push_str(&d.value);
        }
        candidate_names.extend(d.texts.iter().cloned());
    }

    let meta_lower = combined_meta.to_lowercase();

    // Check if any candidate label marks this tile as "You".
    let is_self = candidate_names.iter().any(|c| is_self_label(c)) || is_self_label(&combined_meta);

    // Pick the first clean human participant name from the tile's text/title nodes.
    let name = candidate_names
        .iter()
        .filter(|c| !is_self_label(c))
        .find_map(|c| clean_participant_name(c))?;

    let class_speaking = match platform {
        ConferencingPlatform::GoogleMeet => has_any_class(&all_classes, GOOGLE_MEET_MARKER_CLASSES),
        ConferencingPlatform::Slack => has_any_class(&all_classes, SLACK_MARKER_CLASSES),
        _ => false,
    };

    let semantic_speaking = meta_lower.contains("active speaker")
        || meta_lower.contains("is speaking")
        || meta_lower.contains("is talking")
        || has_word_token(&meta_lower, "speaking")
        || has_word_token(&meta_lower, "talking");

    let is_muted = has_word_token(&meta_lower, "muted")
        || meta_lower.contains("microphone off")
        || meta_lower.contains("mic off");

    let is_speaking = (class_speaking || semantic_speaking) && !is_muted;

    Some(EvaluatedTile {
        name,
        is_speaking,
        is_muted,
        is_self,
    })
}

fn has_word_token(lower: &str, target: &str) -> bool {
    lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .any(|w| w == target)
}

/// Whether `node` looks like the root of a participant video tile on `platform`.
pub fn is_tile_root(platform: ConferencingPlatform, node: &TileNodeData) -> bool {
    match platform {
        ConferencingPlatform::GoogleMeet => {
            if has_any_class(&node.dom_classes, GOOGLE_MEET_TILE_ROOT_CLASSES) {
                return true;
            }
        }
        ConferencingPlatform::Slack
            if has_any_class(&node.dom_classes, SLACK_TILE_ROOT_CLASSES) =>
        {
            return true;
        }
        ConferencingPlatform::MicrosoftTeams
            if has_any_class(&node.dom_classes, TEAMS_TILE_ROOT_CLASSES) =>
        {
            return true;
        }
        _ => {}
    }

    // Semantic fallback for Zoom, Teams, or updated web DOMs: a group/region
    // whose title or description mentions a participant status ("speaking",
    // "talking", "muted", "unmuted", "active speaker").
    if matches!(
        node.role.as_str(),
        "AXGroup" | "AXGenericElement" | "AXButton" | "AXCell"
    ) {
        let combined = format!("{} {}", node.title, node.description).to_lowercase();
        if (has_word_token(&combined, "muted")
            || has_word_token(&combined, "unmuted")
            || has_word_token(&combined, "speaking")
            || has_word_token(&combined, "talking")
            || combined.contains("active speaker"))
            && clean_participant_name(&node.title)
                .or_else(|| clean_participant_name(&node.description))
                .is_some()
        {
            return true;
        }
    }
    false
}

/// Aggregates evaluated tiles from a single poll tick into sorted,
/// deduplicated `(speaking_names, muted_names)` lists, excluding self-tiles.
pub fn summarize_evaluated_tiles(tiles: &[EvaluatedTile]) -> (Vec<String>, Vec<String>) {
    let mut speaking = Vec::new();
    let mut muted = Vec::new();
    for tile in tiles {
        if tile.is_self {
            continue;
        }
        if tile.is_speaking && !speaking.contains(&tile.name) {
            speaking.push(tile.name.clone());
        }
        if tile.is_muted && !muted.contains(&tile.name) {
            muted.push(tile.name.clone());
        }
    }
    speaking.sort();
    muted.sort();
    (speaking, muted)
}

/// Handle to a running background speaker-observation poller for one meeting.
pub struct SpeakerPollerHandle {
    stop: Arc<AtomicBool>,
    observations: Arc<Mutex<Vec<SpeakerObservation>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SpeakerPollerHandle {
    /// Starts a background speaker poller tied to a meeting that started at
    /// `started_instant`. Safe to call even if Accessibility permission is off
    /// (it will simply record zero observations).
    pub fn start(started_instant: Instant) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let observations = Arc::new(Mutex::new(Vec::new()));
        let stop_flag = Arc::clone(&stop);
        let obs_out = Arc::clone(&observations);

        let thread = std::thread::spawn(move || {
            run_poller_loop(started_instant, stop_flag, obs_out);
        });

        Self {
            stop,
            observations,
            thread: Some(thread),
        }
    }

    /// Stops the background polling thread and returns all collected
    /// active-speaker observations for post-call cluster fusion.
    pub fn stop_and_take(mut self) -> Vec<SpeakerObservation> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        self.observations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl Drop for SpeakerPollerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(target_os = "macos")]
fn run_poller_loop(
    started_instant: Instant,
    stop: Arc<AtomicBool>,
    observations: Arc<Mutex<Vec<SpeakerObservation>>>,
) {
    let mut cached_session: Option<ax_poller::AnchoredSession> = None;
    let mut last_discovery = Instant::now()
        .checked_sub(DISCOVERY_REFRESH_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_state: Option<(Vec<String>, Vec<String>)> = None;
    // Apps whose `AXEnhancedUserInterface` we switched on, restored on exit.
    let mut enhanced_pids: Vec<i32> = Vec::new();
    let push_observation = |offset_ms: u64, speaking: Vec<String>, muted: Vec<String>| {
        if let Ok(mut log) = observations.lock() {
            log.push(SpeakerObservation {
                offset_ms,
                speaking_names: speaking,
                muted_names: muted,
            });
        }
    };

    while !stop.load(Ordering::Relaxed) {
        if !crate::paste::accessibility_trusted() {
            for _ in 0..20 {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            continue;
        }

        let refresh_after = if cached_session.is_some() {
            DISCOVERY_REFRESH_INTERVAL
        } else {
            DISCOVERY_RETRY_INTERVAL
        };
        if last_discovery.elapsed() >= refresh_after {
            let had_session = cached_session.is_some();
            cached_session =
                crate::meeting_detect::detect_active_call_for_poller().and_then(|call| {
                    ax_poller::AnchoredSession::discover(call, &mut enhanced_pids, &stop)
                });
            last_discovery = Instant::now();
            if had_session && cached_session.is_none() && !stop.load(Ordering::Relaxed) {
                mark_session_lost(&started_instant, &mut last_state, &push_observation);
            }
        }

        if let Some(session) = &cached_session {
            // Timestamp before the AX reads, which can take hundreds of ms.
            let offset_ms = started_instant.elapsed().as_millis() as u64;
            match session.poll_compact(&stop) {
                Some(tiles) => {
                    let (speaking, muted) = summarize_evaluated_tiles(&tiles);
                    let current = (speaking.clone(), muted.clone());
                    if last_state.as_ref() != Some(&current) {
                        last_state = Some(current);
                        push_observation(offset_ms, speaking, muted);
                    }
                }
                None => {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // Cached AX handles became invalid (window closed or DOM
                    // rebuilt); rediscover on the next tick.
                    cached_session = None;
                    last_discovery = Instant::now()
                        .checked_sub(DISCOVERY_RETRY_INTERVAL)
                        .unwrap_or_else(Instant::now);
                    mark_session_lost(&started_instant, &mut last_state, &push_observation);
                }
            }
        }

        for _ in 0..3 {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(COMPACT_POLL_INTERVAL / 3);
        }
    }

    ax_poller::restore_enhanced_ui(&enhanced_pids);
}

/// Logs an empty observation when the call window disappears, so the last
/// "X is speaking" state doesn't carry forward and support a wrong name.
#[cfg(target_os = "macos")]
fn mark_session_lost(
    started_instant: &Instant,
    last_state: &mut Option<(Vec<String>, Vec<String>)>,
    push_observation: &impl Fn(u64, Vec<String>, Vec<String>),
) {
    let empty = (Vec::new(), Vec::new());
    if last_state.as_ref().is_some_and(|s| *s != empty) {
        push_observation(
            started_instant.elapsed().as_millis() as u64,
            Vec::new(),
            Vec::new(),
        );
    }
    *last_state = Some(empty);
}

#[cfg(not(target_os = "macos"))]
fn run_poller_loop(
    _started_instant: Instant,
    _stop: Arc<AtomicBool>,
    _observations: Arc<Mutex<Vec<SpeakerObservation>>>,
) {
}

#[cfg(target_os = "macos")]
mod ax_poller {
    use super::*;
    use crate::meeting_detect::{canonical_bundle_id, DetectedCall};
    use core_foundation::array::{CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex};
    use core_foundation::base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::string::{CFString, CFStringGetTypeID, CFStringRef};
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
        fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> i32;
    }

    const MAX_DISCOVERY_NODES: usize = 1500;
    const MAX_DISCOVERY_DEPTH: usize = 24;
    const MAX_COMPACT_DESCENDANTS: usize = 40;
    /// Wall-clock cap on one discovery pass, so a slow or hung renderer can't
    /// stall the poller (and `stop_and_take`, which joins it) for long.
    const DISCOVERY_BUDGET: Duration = Duration::from_millis(750);
    /// Per-element AX messaging timeout. Timeouts don't propagate from the
    /// app element to windows/children, which otherwise use the ~6s default.
    const AX_TIMEOUT_SECS: f32 = 0.35;

    pub struct OwnedAx(pub AXUIElementRef);
    unsafe impl Send for OwnedAx {}
    impl Drop for OwnedAx {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn copy_attr(element: AXUIElementRef, name: &str) -> Option<OwnedAx> {
        let attr = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
        };
        (status == 0 && !value.is_null()).then(|| OwnedAx(value))
    }

    fn as_string(value: &OwnedAx) -> Option<String> {
        unsafe {
            if CFGetTypeID(value.0) != CFStringGetTypeID() {
                return None;
            }
            Some(CFString::wrap_under_get_rule(value.0 as CFStringRef).to_string())
        }
    }

    fn as_string_list(value: &OwnedAx) -> Vec<String> {
        unsafe {
            if CFGetTypeID(value.0) != CFArrayGetTypeID() {
                return Vec::new();
            }
            let count = CFArrayGetCount(value.0 as _).min(32);
            let mut out = Vec::new();
            for i in 0..count {
                let item = CFArrayGetValueAtIndex(value.0 as _, i);
                if !item.is_null() && CFGetTypeID(item) == CFStringGetTypeID() {
                    out.push(CFString::wrap_under_get_rule(item as CFStringRef).to_string());
                }
            }
            out
        }
    }

    fn children(element: AXUIElementRef) -> Vec<OwnedAx> {
        let Some(array) = copy_attr(element, "AXChildren") else {
            return Vec::new();
        };
        unsafe {
            if CFGetTypeID(array.0) != CFArrayGetTypeID() {
                return Vec::new();
            }
            let count = CFArrayGetCount(array.0 as _).min(64);
            (0..count)
                .filter_map(|i| {
                    let child = CFArrayGetValueAtIndex(array.0 as _, i);
                    (!child.is_null()).then(|| {
                        CFRetain(child);
                        AXUIElementSetMessagingTimeout(child, AX_TIMEOUT_SECS);
                        OwnedAx(child)
                    })
                })
                .collect()
        }
    }

    fn read_node_data(element: AXUIElementRef) -> TileNodeData {
        let role = copy_attr(element, "AXRole")
            .and_then(|v| as_string(&v))
            .unwrap_or_default();
        let title = copy_attr(element, "AXTitle")
            .and_then(|v| as_string(&v))
            .unwrap_or_default();
        let description = copy_attr(element, "AXDescription")
            .and_then(|v| as_string(&v))
            .unwrap_or_default();
        let value = copy_attr(element, "AXValue")
            .and_then(|v| as_string(&v))
            .unwrap_or_default();
        let dom_classes = copy_attr(element, "AXDOMClassList")
            .map(|v| as_string_list(&v))
            .unwrap_or_default();
        let mut texts = Vec::new();
        if role == "AXStaticText" || role == "AXHeading" {
            if !value.trim().is_empty() {
                texts.push(value.clone());
            } else if !title.trim().is_empty() {
                texts.push(title.clone());
            }
        }
        TileNodeData {
            role,
            title,
            description,
            value,
            dom_classes,
            texts,
        }
    }

    fn set_bool_attr(app: &OwnedAx, name: &str, value: bool) -> i32 {
        let key = CFString::new(name);
        let v = if value {
            CFBoolean::true_value()
        } else {
            CFBoolean::false_value()
        };
        unsafe { AXUIElementSetAttributeValue(app.0, key.as_concrete_TypeRef(), v.as_CFTypeRef()) }
    }

    fn bool_attr(app: &OwnedAx, name: &str) -> bool {
        copy_attr(app.0, name).is_some_and(|v| unsafe {
            CFGetTypeID(v.0) == CFBoolean::type_id()
                && CFBoolean::wrap_under_get_rule(v.0 as _) == CFBoolean::true_value()
        })
    }

    /// Asks a Chromium/Electron app to build its web accessibility tree so
    /// `AXDOMClassList` is exposed — once per process per poller run.
    ///
    /// Electron honours `AXManualAccessibility`, which has no side effects.
    /// Chrome-family browsers only honour `AXEnhancedUserInterface`, which is
    /// known to break window animations/moves for window managers, so we only
    /// set it if it was off and switch it back off when the poller stops.
    fn enable_dom_accessibility(
        app: &OwnedAx,
        pid: i32,
        canonical_bundle: &str,
        enhanced_pids: &mut Vec<i32>,
    ) {
        if enhanced_pids.contains(&pid) || enhanced_pids.contains(&-pid) {
            return;
        }
        let is_browser = crate::meeting_detect::is_browser_bundle(canonical_bundle).is_some();
        if !is_browser && set_bool_attr(app, "AXManualAccessibility", true) == 0 {
            // Negative = handled without touching AXEnhancedUserInterface.
            enhanced_pids.push(-pid);
            return;
        }
        if bool_attr(app, "AXEnhancedUserInterface") {
            // Already on (e.g. VoiceOver): leave it as the user had it.
            enhanced_pids.push(-pid);
            return;
        }
        if set_bool_attr(app, "AXEnhancedUserInterface", true) == 0 {
            enhanced_pids.push(pid);
        }
    }

    /// Switches `AXEnhancedUserInterface` back off on apps we turned it on for.
    pub fn restore_enhanced_ui(enhanced_pids: &[i32]) {
        for &pid in enhanced_pids.iter().filter(|p| **p > 0) {
            let raw = unsafe { AXUIElementCreateApplication(pid) };
            if raw.is_null() {
                continue;
            }
            let app = OwnedAx(raw);
            unsafe { AXUIElementSetMessagingTimeout(app.0, AX_TIMEOUT_SECS) };
            let _ = set_bool_attr(&app, "AXEnhancedUserInterface", false);
        }
    }

    pub struct AnchoredSession {
        platform: ConferencingPlatform,
        tiles: Vec<OwnedAx>,
    }

    impl AnchoredSession {
        pub fn discover(
            call: DetectedCall,
            enhanced_pids: &mut Vec<i32>,
            stop: &AtomicBool,
        ) -> Option<Self> {
            use std::collections::VecDeque;

            let deadline = Instant::now() + DISCOVERY_BUDGET;
            let mut pids = vec![call.pid];
            let canon = canonical_bundle_id(&call.bundle_id);
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

            for pid in pids {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                unsafe {
                    let app_raw = AXUIElementCreateApplication(pid);
                    if app_raw.is_null() {
                        continue;
                    }
                    let app = OwnedAx(app_raw);
                    AXUIElementSetMessagingTimeout(app.0, AX_TIMEOUT_SECS);
                    enable_dom_accessibility(&app, pid, canon, enhanced_pids);

                    let Some(windows) = copy_attr(app.0, "AXWindows") else {
                        continue;
                    };
                    if CFGetTypeID(windows.0) != CFArrayGetTypeID() {
                        continue;
                    }
                    let win_count = CFArrayGetCount(windows.0 as _).min(12);
                    let mut discovered_tiles = Vec::new();

                    for i in 0..win_count {
                        if stop.load(Ordering::Relaxed) {
                            return None;
                        }
                        let win = CFArrayGetValueAtIndex(windows.0 as _, i);
                        if win.is_null() {
                            continue;
                        }
                        CFRetain(win);
                        AXUIElementSetMessagingTimeout(win, AX_TIMEOUT_SECS);
                        let win_owned = OwnedAx(win);
                        let mut queue = VecDeque::from([(win_owned, 0usize)]);
                        let mut visited = 0usize;

                        while let Some((node, depth)) = queue.pop_front() {
                            if stop.load(Ordering::Relaxed) {
                                return None;
                            }
                            if visited >= MAX_DISCOVERY_NODES || Instant::now() >= deadline {
                                break;
                            }
                            visited += 1;
                            let data = read_node_data(node.0);
                            if is_tile_root(call.platform, &data) {
                                discovered_tiles.push(node);
                                if discovered_tiles.len() >= 48 {
                                    break;
                                }
                                continue;
                            }
                            if depth < MAX_DISCOVERY_DEPTH {
                                for child in children(node.0) {
                                    queue.push_back((child, depth + 1));
                                }
                            }
                        }
                        if !discovered_tiles.is_empty() {
                            return Some(Self {
                                platform: call.platform,
                                tiles: discovered_tiles,
                            });
                        }
                    }
                }
            }
            None
        }

        /// Fast compact poll over the cached tile roots. Returns `None` if all
        /// cached tile handles have become invalid so the caller can trigger a
        /// fresh discovery pass.
        pub fn poll_compact(&self, stop: &AtomicBool) -> Option<Vec<EvaluatedTile>> {
            use std::collections::VecDeque;

            let mut results = Vec::new();
            let mut valid_handles = 0usize;

            for tile in &self.tiles {
                if stop.load(Ordering::Relaxed) {
                    return None;
                }
                let root_data = read_node_data(tile.0);
                if root_data.role.is_empty() && root_data.dom_classes.is_empty() {
                    continue;
                }
                valid_handles += 1;

                let mut descendants = Vec::new();
                let mut queue: VecDeque<OwnedAx> = children(tile.0).into_iter().collect();
                while let Some(child) = queue.pop_front() {
                    if stop.load(Ordering::Relaxed) {
                        return None;
                    }
                    if descendants.len() >= MAX_COMPACT_DESCENDANTS {
                        break;
                    }
                    descendants.push(read_node_data(child.0));
                    if descendants.len() < MAX_COMPACT_DESCENDANTS {
                        for grandchild in children(child.0) {
                            queue.push_back(grandchild);
                        }
                    }
                }

                if let Some(evaluated) =
                    evaluate_tile_subtree(self.platform, &root_data, &descendants)
                {
                    results.push(evaluated);
                }
            }

            (valid_handles > 0).then_some(results)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_participant_name_strips_badges_and_status_suffixes() {
        assert_eq!(
            clean_participant_name("Alice Chen, speaking, unmuted"),
            Some("Alice Chen".into())
        );
        assert_eq!(
            clean_participant_name("Bob Smith (Host)"),
            Some("Bob Smith".into())
        );
        assert_eq!(
            clean_participant_name("  Carol   Danvers (Guest) "),
            Some("Carol Danvers".into())
        );
        assert_eq!(clean_participant_name("Mute"), None);
        assert_eq!(clean_participant_name("Turn off microphone"), None);
    }

    #[test]
    fn is_self_label_identifies_local_user_tiles() {
        assert!(is_self_label("You"));
        assert!(is_self_label("Jake Finch (You)"));
        assert!(is_self_label("Jake Finch, You"));
        assert!(!is_self_label("Alice Youmans"));
    }

    #[test]
    fn google_meet_tile_with_kss_marker_is_marked_speaking() {
        let root = TileNodeData {
            role: "AXGroup".into(),
            dom_classes: vec!["oZRSLe".into()],
            ..Default::default()
        };
        let descendants = vec![
            TileNodeData {
                role: "AXStaticText".into(),
                texts: vec!["Alice Chen".into()],
                ..Default::default()
            },
            TileNodeData {
                role: "AXGroup".into(),
                dom_classes: vec!["kssMZb".into()],
                ..Default::default()
            },
        ];

        assert!(is_tile_root(ConferencingPlatform::GoogleMeet, &root));
        let eval =
            evaluate_tile_subtree(ConferencingPlatform::GoogleMeet, &root, &descendants).unwrap();
        assert_eq!(
            eval,
            EvaluatedTile {
                name: "Alice Chen".into(),
                is_speaking: true,
                is_muted: false,
                is_self: false,
            }
        );
    }

    #[test]
    fn slack_huddle_tile_and_muted_tile_are_evaluated_accurately() {
        let speaking_root = TileNodeData {
            role: "AXGroup".into(),
            dom_classes: vec!["p-huddle_peer_tile".into()],
            texts: vec!["Bob Smith".into()],
            ..Default::default()
        };
        let speaking_desc = vec![TileNodeData {
            role: "AXGroup".into(),
            dom_classes: vec!["p-huddle_peer_tile__overlay--active_speaker".into()],
            ..Default::default()
        }];
        let muted_root = TileNodeData {
            role: "AXGroup".into(),
            dom_classes: vec!["p-huddle_peer_tile".into()],
            title: "Carol Danvers, muted".into(),
            ..Default::default()
        };
        let self_root = TileNodeData {
            role: "AXGroup".into(),
            dom_classes: vec!["p-huddle_peer_tile".into()],
            title: "Jake (You)".into(),
            ..Default::default()
        };

        let t1 = evaluate_tile_subtree(ConferencingPlatform::Slack, &speaking_root, &speaking_desc)
            .unwrap();
        let t2 = evaluate_tile_subtree(ConferencingPlatform::Slack, &muted_root, &[]).unwrap();
        let t3 = evaluate_tile_subtree(ConferencingPlatform::Slack, &self_root, &[]);
        // Self-only label has no non-self participant name, or if named is marked is_self.
        assert!(t3.is_none() || t3.unwrap().is_self);

        let (speaking, muted) = summarize_evaluated_tiles(&[t1, t2]);
        assert_eq!(speaking, vec!["Bob Smith".to_string()]);
        assert_eq!(muted, vec!["Carol Danvers".to_string()]);
    }

    #[test]
    fn microsoft_teams_vdi_tile_requires_semantic_speaking_indicator() {
        let silent_root = TileNodeData {
            role: "AXGroup".into(),
            dom_classes: vec!["vdi-frame-occlusion".into()],
            texts: vec!["Dave Grohl".into()],
            ..Default::default()
        };
        assert!(is_tile_root(
            ConferencingPlatform::MicrosoftTeams,
            &silent_root
        ));
        let eval_silent =
            evaluate_tile_subtree(ConferencingPlatform::MicrosoftTeams, &silent_root, &[]).unwrap();
        assert_eq!(
            eval_silent,
            EvaluatedTile {
                name: "Dave Grohl".into(),
                is_speaking: false,
                is_muted: false,
                is_self: false,
            }
        );

        let active_desc = vec![TileNodeData {
            role: "AXGroup".into(),
            description: "Dave Grohl, speaking".into(),
            ..Default::default()
        }];
        let eval_speaking = evaluate_tile_subtree(
            ConferencingPlatform::MicrosoftTeams,
            &silent_root,
            &active_desc,
        )
        .unwrap();
        assert!(eval_speaking.is_speaking);
    }
}
