//! Moves an install from the app's old name (Dev Whisper,
//! `com.devwhisper.app`) to Mouthpiece (`dev.mouthpiece`).
//!
//! The in-app updater replaces the bundle at its old path, so an updated
//! copy first runs as `Dev Whisper.app` with the new identifier, which
//! looks for its data in a new, empty folder. This carries over:
//! - the data folder (settings, history, models, hook token), leaving a
//!   link at the old path so `~/.claude/settings.json` hook entries that
//!   point into it keep working;
//! - the app's name in its Applications folder;
//! - the launch-at-login item, which is named after the app.
//!
//! macOS permission grants (Accessibility, Microphone) belong to the old
//! identifier and can't be carried over; the setup checklist asks again.
//! Every step is best effort: a failure is logged and the app carries on.

use std::path::{Path, PathBuf};

pub const OLD_IDENTIFIER: &str = "com.devwhisper.app";
const OLD_BUNDLE: &str = "Dev Whisper.app";
const NEW_BUNDLE: &str = "Mouthpiece.app";
const OLD_LOGIN_ITEM: &str = "Dev Whisper.plist";

/// The `.app` folder this executable runs from, if it runs from one.
fn bundle_path(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.parent().map(Path::to_path_buf))
        .flatten()
}

/// Where the bundle should move, if it still has the old name and nothing
/// is in the way.
pub fn bundle_destination(exe: &Path) -> Option<(PathBuf, PathBuf)> {
    let bundle = bundle_path(exe)?;
    if bundle.file_name()? != OLD_BUNDLE {
        return None;
    }
    let dest = bundle.with_file_name(NEW_BUNDLE);
    (!dest.exists()).then_some((bundle, dest))
}

/// Renames `Dev Whisper.app` to `Mouthpiece.app` and reopens it from there.
/// Returns true when the caller should exit so the reopened copy takes
/// over. Runs before anything else, so the hotkey, the login item and the
/// single-instance lock all belong to the copy at the new path.
pub fn move_bundle() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some((bundle, dest)) = bundle_destination(&exe) else {
        return false;
    };
    if let Err(err) = std::fs::rename(&bundle, &dest) {
        // Usually App Management or a read-only folder. The app works
        // fine under its old file name.
        crate::applog!("rename: couldn't rename {}: {err}", bundle.display());
        return false;
    }
    // Wait a moment so this copy has exited (and released the
    // single-instance lock) before the new one starts.
    let reopened = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("sleep 1; /usr/bin/open \"$1\"")
        .arg("sh")
        .arg(&dest)
        .spawn();
    match reopened {
        Ok(_) => true,
        Err(err) => {
            // Can't reopen: put it back rather than exit with nothing
            // running.
            crate::applog!("rename: couldn't reopen from {}: {err}", dest.display());
            let _ = std::fs::rename(&dest, &bundle);
            false
        }
    }
}

/// Moves the old data folder to `new_dir` and leaves a link in its place.
/// Does nothing once the new folder exists, or if there's no old one.
pub fn move_data_dir(new_dir: &Path) {
    let Some(old_dir) = new_dir.parent().map(|p| p.join(OLD_IDENTIFIER)) else {
        return;
    };
    match move_data_dir_from(&old_dir, new_dir) {
        Ok(true) => crate::applog!("rename: moved data from {}", old_dir.display()),
        Ok(false) => {}
        Err(err) => crate::applog!("rename: couldn't move {}: {err}", old_dir.display()),
    }
}

pub fn move_data_dir_from(old_dir: &Path, new_dir: &Path) -> std::io::Result<bool> {
    let old_is_real_dir = std::fs::symlink_metadata(old_dir).is_ok_and(|m| m.is_dir());
    if !old_is_real_dir || new_dir.exists() {
        return Ok(false);
    }
    std::fs::rename(old_dir, new_dir)?;
    std::os::unix::fs::symlink(new_dir, old_dir)?;
    Ok(true)
}

/// Replaces the old "Dev Whisper" login item with one for this app.
pub fn move_login_item(app: &tauri::AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let old = Path::new(&home)
        .join("Library/LaunchAgents")
        .join(OLD_LOGIN_ITEM);
    if !old.exists() {
        return;
    }
    if let Err(err) = std::fs::remove_file(&old) {
        crate::applog!("rename: couldn't remove the old login item: {err}");
        return;
    }
    match app.autolaunch().enable() {
        Ok(()) => crate::applog!("rename: moved the login item"),
        Err(err) => crate::applog!("rename: couldn't add the login item: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_old_named_bundle_moves() {
        let exe = Path::new("/nonexistent/Apps/Dev Whisper.app/Contents/MacOS/tauri-app");
        assert_eq!(
            bundle_destination(exe),
            Some((
                PathBuf::from("/nonexistent/Apps/Dev Whisper.app"),
                PathBuf::from("/nonexistent/Apps/Mouthpiece.app")
            ))
        );
        let new = Path::new("/nonexistent/Apps/Mouthpiece.app/Contents/MacOS/tauri-app");
        assert_eq!(bundle_destination(new), None);
        // A dev build runs from target/, not a bundle.
        assert_eq!(
            bundle_destination(Path::new("/src/target/debug/tauri-app")),
            None
        );
    }

    #[test]
    fn data_moves_once_and_leaves_a_link() {
        let root = std::env::temp_dir().join(format!("mp-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join(OLD_IDENTIFIER);
        let new = root.join("dev.mouthpiece");
        std::fs::create_dir_all(old.join("bin")).unwrap();
        std::fs::write(old.join("config.json"), "{}").unwrap();

        assert!(move_data_dir_from(&old, &new).unwrap());
        assert_eq!(
            std::fs::read_to_string(new.join("config.json")).unwrap(),
            "{}"
        );
        // The old path still reaches the same files.
        assert!(std::fs::symlink_metadata(&old).unwrap().is_symlink());
        assert!(old.join("bin").is_dir());
        // Second launch: nothing to do.
        assert!(!move_data_dir_from(&old, &new).unwrap());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn existing_new_folder_is_left_alone() {
        let root = std::env::temp_dir().join(format!("mp-rename-both-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join(OLD_IDENTIFIER);
        let new = root.join("dev.mouthpiece");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        assert!(!move_data_dir_from(&old, &new).unwrap());
        assert!(!std::fs::symlink_metadata(&old).unwrap().is_symlink());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
