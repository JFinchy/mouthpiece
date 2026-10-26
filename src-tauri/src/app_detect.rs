use base64::Engine;
use objc2::AnyThread;
use objc2_app_kit::{
    NSApplicationActivationPolicy, NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

#[derive(Clone)]
pub struct AppInfo {
    pub bundle_id: String,
    pub name: String,
    /// `data:image/png;base64,...` — `None` if the icon couldn't be
    /// resolved/encoded (never fatal, callers just show a placeholder).
    pub icon_data_uri: Option<String>,
}

/// Bundle IDs for common macOS apps developers are likely to use, shown in
/// the mode-assignment picker even when not currently running (SuperWhisper
/// offers the same "pick from a broad app list", not just live processes).
/// Icons are resolved on demand via `icon_for_bundle_id`, same as running
/// apps — nothing here is hardcoded beyond the id/name pairing.
pub const COMMON_APPS: &[(&str, &str)] = &[
    ("com.apple.Terminal", "Terminal"),
    ("com.googlecode.iterm2", "iTerm"),
    ("dev.warp.Warp-Stable", "Warp"),
    ("com.github.wez.wezterm", "WezTerm"),
    ("net.kovidgoyal.kitty", "Kitty"),
    ("com.microsoft.VSCode", "Visual Studio Code"),
    ("com.todesktop.230313mzl4w4u92", "Cursor"),
    ("com.jetbrains.intellij", "IntelliJ IDEA"),
    ("com.sublimetext.4", "Sublime Text"),
    ("com.apple.dt.Xcode", "Xcode"),
    ("com.google.Chrome", "Google Chrome"),
    ("com.apple.Safari", "Safari"),
    ("company.thebrowser.Browser", "Arc"),
    ("org.mozilla.firefox", "Firefox"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("com.apple.MobileSMS", "Messages"),
    ("com.hnc.Discord", "Discord"),
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("notion.id", "Notion"),
    ("com.apple.mail", "Mail"),
    ("com.apple.Notes", "Notes"),
    ("com.apple.iChat", "TextEdit"),
];

const ICON_PX: f64 = 32.0;

/// Icons vended by NSWorkspace/NSRunningApplication apparently don't expose
/// pre-sized bitmap representations via `.representations()` the way a
/// static .icns file's reps array would (that approach returned zero
/// usable icons — presumably a lazily-rasterized single representation).
/// So: actually resample by drawing the source image into a freshly
/// allocated 32x32 canvas, then encode *that* canvas. This is also the fix
/// for the earlier bug where `NSBitmapImageRep::setSize` only changed size
/// *metadata* without resampling pixels, silently producing 1-4MB PNGs
/// from the full 512-1024px source instead of a genuinely small icon.
fn icon_to_data_uri(icon: &NSImage) -> Option<String> {
    let size = NSSize::new(ICON_PX, ICON_PX);
    let canvas = NSImage::initWithSize(NSImage::alloc(), size);

    // Deprecated in favor of block-based drawing APIs, but still fully
    // functional and far simpler to call from Rust than the block-based
    // replacement (which needs the `block2` machinery for an Objective-C
    // closure argument).
    #[allow(deprecated)]
    {
        canvas.lockFocus();
        icon.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), size));
        canvas.unlockFocus();
    }

    let tiff = canvas.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let rep = rep.downcast::<NSBitmapImageRep>().ok()?;

    let props = NSDictionary::new();
    let png =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
    let bytes = png.to_vec();
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn icon_for_bundle_id(bundle_id: &str) -> Option<String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let url = workspace
        .URLForApplicationWithBundleIdentifier(&objc2_foundation::NSString::from_str(bundle_id))?;
    let path = url.path()?;
    let icon = workspace.iconForFile(&path);
    icon_to_data_uri(&icon)
}

/// Reads the frontmost application via NSWorkspace. No special permission
/// needed (unlike Accessibility), but AppKit requires this run on the main
/// thread — callers should dispatch through `AppHandle::run_on_main_thread`.
pub fn frontmost_app_info() -> Option<AppInfo> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    let bundle_id = app.bundleIdentifier()?.to_string();
    let name = app
        .localizedName()
        .map(|n| n.to_string())
        .unwrap_or_else(|| bundle_id.clone());
    let icon_data_uri = app.icon().and_then(|icon| icon_to_data_uri(&icon));
    Some(AppInfo {
        bundle_id,
        name,
        icon_data_uri,
    })
}

/// Frontmost app's bundle id and name (no icon encoding), fetched from a
/// background thread by hopping to the main thread and waiting. `None` if
/// the main thread doesn't answer within a second rather than hanging the
/// caller.
pub fn frontmost_app_blocking(app: &tauri::AppHandle) -> Option<(String, String)> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let workspace = NSWorkspace::sharedWorkspace();
        let info = workspace.frontmostApplication().and_then(|a| {
            let bundle_id = a.bundleIdentifier()?.to_string();
            let name = a
                .localizedName()
                .map(|n| n.to_string())
                .unwrap_or_else(|| bundle_id.clone());
            Some((bundle_id, name))
        });
        let _ = tx.send(info);
    })
    .ok()?;
    rx.recv_timeout(std::time::Duration::from_secs(1))
        .ok()
        .flatten()
}

/// Process id of the frontmost application. Main thread only, like
/// `frontmost_app_info`.
pub fn frontmost_pid() -> Option<i32> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|a| a.processIdentifier())
}

/// Frontmost app's bundle id and process id, for reading its Accessibility
/// tree. Same hop to the main thread as `frontmost_app_blocking`.
pub fn frontmost_bundle_and_pid_blocking(app: &tauri::AppHandle) -> Option<(String, i32)> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let info = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .and_then(|a| Some((a.bundleIdentifier()?.to_string(), a.processIdentifier())));
        let _ = tx.send(info);
    })
    .ok()?;
    rx.recv_timeout(std::time::Duration::from_secs(1))
        .ok()
        .flatten()
}

pub fn frontmost_bundle_id_blocking(app: &tauri::AppHandle) -> Option<String> {
    frontmost_app_blocking(app).map(|(bundle_id, _)| bundle_id)
}

/// Lists currently-running regular (foreground, Dock-visible) apps via
/// NSWorkspace, for the Settings "assign a mode" picker. Excludes
/// background/agent processes (menu-bar-only helpers etc.) and
/// Mouthpiece itself. Same main-thread requirement as `frontmost_app_info`.
pub fn list_running_apps(own_bundle_id: &str) -> Vec<AppInfo> {
    let workspace = NSWorkspace::sharedWorkspace();
    let running = workspace.runningApplications();

    running
        .iter()
        .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
        .filter_map(|app| {
            let bundle_id = app.bundleIdentifier()?.to_string();
            if bundle_id == own_bundle_id {
                return None;
            }
            let name = app
                .localizedName()
                .map(|n| n.to_string())
                .unwrap_or_else(|| bundle_id.clone());
            let icon_data_uri = app.icon().and_then(|icon| icon_to_data_uri(&icon));
            Some(AppInfo {
                bundle_id,
                name,
                icon_data_uri,
            })
        })
        .collect()
}

/// Common apps (see `COMMON_APPS`) that aren't already in `exclude` (the
/// currently-running list), with icons resolved from their installed
/// bundle on disk. Skips anything not actually installed.
pub fn list_common_apps(exclude: &[AppInfo]) -> Vec<AppInfo> {
    COMMON_APPS
        .iter()
        .filter(|(id, _)| !exclude.iter().any(|a| a.bundle_id == *id))
        .filter_map(|(id, name)| {
            let icon_data_uri = icon_for_bundle_id(id);
            icon_data_uri.as_ref()?; // not installed / unresolvable — skip
            Some(AppInfo {
                bundle_id: id.to_string(),
                name: name.to_string(),
                icon_data_uri,
            })
        })
        .collect()
}

/// Process id of the frontmost app, fetched like `frontmost_app_blocking`.
/// Used to confirm a focus hop landed, and to know where to return to.
pub fn frontmost_pid_blocking(app: &tauri::AppHandle) -> Option<i32> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let pid = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|a| a.processIdentifier());
        let _ = tx.send(pid);
    })
    .ok()?;
    rx.recv_timeout(std::time::Duration::from_secs(1))
        .ok()
        .flatten()
}

/// A regular (Dock-visible) running app, by process id. Helper processes
/// and background agents return `None`, which is what lets the caller walk
/// up a process tree until it reaches the app that owns a terminal.
pub fn regular_app_for_pid(pid: i32) -> Option<(String, String)> {
    use objc2_app_kit::NSRunningApplication;
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    regular_app_parts(&app)
}

/// The first regular running app with this bundle id.
pub fn regular_app_for_bundle(bundle_id: &str) -> Option<(i32, String)> {
    use objc2_app_kit::NSRunningApplication;
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        &objc2_foundation::NSString::from_str(bundle_id),
    );
    apps.iter().find_map(|app| {
        let (_, name) = regular_app_parts(&app)?;
        Some((app.processIdentifier(), name))
    })
}

fn regular_app_parts(app: &objc2_app_kit::NSRunningApplication) -> Option<(String, String)> {
    if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
        return None;
    }
    let bundle_id = app.bundleIdentifier()?.to_string();
    let name = app
        .localizedName()
        .map(|n| n.to_string())
        .unwrap_or_else(|| bundle_id.clone());
    Some((bundle_id, name))
}
