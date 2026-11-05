//! Optional sound cues for dictation: when recording starts, when it
//! stops, and when the text has been pasted (or the dictation failed).
//! Off by default (`AppConfig::dictation_sounds`). Uses the built-in macOS
//! system sounds, played with `afplay` like the agent-waiting sound.

use tauri::AppHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Start,
    Stop,
    Done,
    Error,
}

impl Cue {
    fn path(self) -> &'static str {
        match self {
            Cue::Start => "/System/Library/Sounds/Pop.aiff",
            Cue::Stop => "/System/Library/Sounds/Bottle.aiff",
            Cue::Done => "/System/Library/Sounds/Glass.aiff",
            Cue::Error => "/System/Library/Sounds/Basso.aiff",
        }
    }
}

/// Softer than full volume: these play on every dictation.
const VOLUME: &str = "0.5";

/// Plays `cue` if dictation sounds are on. Never blocks the caller.
pub fn play(app: &AppHandle, cue: Cue) {
    if !crate::config::load(app).dictation_sounds {
        return;
    }
    std::thread::spawn(move || {
        let _ = std::process::Command::new("/usr/bin/afplay")
            .args(["-v", VOLUME, cue.path()])
            .status();
    });
}

#[tauri::command]
pub fn get_dictation_sounds(app: AppHandle) -> bool {
    crate::config::load(&app).dictation_sounds
}

#[tauri::command]
pub fn set_dictation_sounds(app: AppHandle, enabled: bool) {
    let mut cfg = crate::config::load(&app);
    cfg.dictation_sounds = enabled;
    let _ = crate::config::save(&app, &cfg);
    // Let the user hear what they turned on.
    if enabled {
        play(&app, Cue::Start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_is_a_distinct_system_sound() {
        let cues = [Cue::Start, Cue::Stop, Cue::Done, Cue::Error];
        let paths: std::collections::HashSet<_> = cues.iter().map(|c| c.path()).collect();
        assert_eq!(paths.len(), cues.len());
        for cue in cues {
            assert!(cue.path().starts_with("/System/Library/Sounds/"));
            // Agent-waiting uses Tink; keep dictation cues distinguishable.
            assert!(!cue.path().ends_with("Tink.aiff"));
        }
    }
}
