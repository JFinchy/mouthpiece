use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::config;

/// The underlying formatting/refinement transform a mode applies. This is
/// the small, fixed set of *technical* behaviors (regex-based CLI
/// formatting, LLM prompt phrasing in `llm.rs::prompt_for_mode`) — several
/// user-facing named modes (see `ModeDefinition`) can share the same one.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Behavior {
    /// No transformation — pastes the raw Whisper transcript.
    Plain,
    /// No rule-based transform (behaves like Plain formatting-wise); the
    /// natural fit for modes that lean on LLM refinement for tone instead.
    Casual,
    /// Terminal apps: a handful of illustrative natural-language ->
    /// shell-command patterns. Intentionally narrow — general NL-to-CLI
    /// translation needs the LLM refinement pipeline, not regex.
    Cli,
}

/// A mode's LLM refinement choice — a mode can opt out entirely, ride
/// along with whatever the global default model is, or pin its own
/// specific model. Modeled as an enum rather than `Option<String>` because
/// "off" and "no override (use the global default)" are genuinely
/// different states, both of which need representing in the Settings UI.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum LlmRefinement {
    #[default]
    Off,
    Global,
    Model(String),
}

/// One app assigned to a mode.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct AppRef {
    pub bundle_id: String,
    pub app_name: String,
}

/// A named, user-editable mode: a formatting behavior plus the apps that
/// use it and its own model choices. Modes are a flat, uniform list (see
/// `config::AppConfig::modes`) — the handful shipped by default (Default,
/// Voice to Text, Messaging, CLI) are just pre-seeded entries, exactly as
/// renameable/deletable as anything a user creates.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ModeDefinition {
    pub name: String,
    pub behavior: Behavior,
    #[serde(default)]
    pub apps: Vec<AppRef>,
    /// Whisper model id (matching `models::CATALOG`) to use for this mode,
    /// overriding the globally-active model. `None` = use whatever's
    /// globally active.
    #[serde(default)]
    pub stt_model: Option<String>,
    #[serde(default)]
    pub llm_refinement: LlmRefinement,
    /// Free-form extra instructions folded into the LLM refinement prompt
    /// for this mode (see `llm::prompt_for_mode`) — e.g. "sign off messages
    /// with 'thanks, Jake'". Ignored when `llm_refinement` is `Off`, since
    /// there's no LLM call to steer.
    #[serde(default)]
    pub custom_instructions: Option<String>,
    /// Reorganise dictations of 80+ words into Goal / Context / Constraints
    /// / Done when for a coding agent (`structure.rs`), with the mode's LLM
    /// or, when its cleanup is off, the global one. Off by default.
    #[serde(default)]
    pub structure_long_prompts: bool,
    /// Exactly one mode should have this set — the fallback used when the
    /// frontmost app isn't assigned to any mode. A `name`-based lookup
    /// would break the moment a user renames their fallback mode; this
    /// survives that.
    #[serde(default)]
    pub is_default: bool,
}

/// Legacy one-app-per-rule shape, kept only so `config::load`'s one-time
/// migration can still deserialize a `config.json` saved before modes
/// became named/multi-app (see `config.rs`). Not used anywhere else.
#[derive(Serialize, Deserialize, Clone)]
pub struct AppModeRule {
    pub bundle_id: String,
    pub app_name: String,
    pub mode: Behavior,
    pub stt_model: Option<String>,
    #[serde(default)]
    pub use_llm_refinement: bool,
}

/// The 4 modes a fresh install (or a config migrating up from before named
/// modes existed) starts with. Pre-populating Messaging/CLI's `apps`
/// collapses what used to be a separate hardcoded bundle-id table
/// (`BUILTIN_DEFAULTS`) into real, visible, editable config data instead of
/// an invisible second resolution path.
pub fn seed_default_modes() -> Vec<ModeDefinition> {
    fn app(bundle_id: &str, app_name: &str) -> AppRef {
        AppRef {
            bundle_id: bundle_id.to_string(),
            app_name: app_name.to_string(),
        }
    }

    vec![
        ModeDefinition {
            name: "Default".to_string(),
            behavior: Behavior::Plain,
            apps: Vec::new(),
            stt_model: None,
            llm_refinement: LlmRefinement::Off,
            custom_instructions: None,
            structure_long_prompts: false,
            is_default: true,
        },
        ModeDefinition {
            name: "Voice to Text".to_string(),
            behavior: Behavior::Plain,
            apps: Vec::new(),
            stt_model: None,
            llm_refinement: LlmRefinement::Off,
            custom_instructions: None,
            structure_long_prompts: false,
            is_default: false,
        },
        ModeDefinition {
            name: "Messaging".to_string(),
            behavior: Behavior::Casual,
            apps: vec![
                app("com.apple.MobileSMS", "Messages"),
                app("com.tinyspeck.slackmacgap", "Slack"),
                app("com.hnc.Discord", "Discord"),
            ],
            stt_model: None,
            // Casual's rule-based path is a no-op (regex can't meaningfully
            // "sound casual"), so this is the one shipped mode worth
            // defaulting to LLM refinement rather than leaving the raw
            // transcript as-is.
            llm_refinement: LlmRefinement::Global,
            custom_instructions: None,
            structure_long_prompts: false,
            is_default: false,
        },
        ModeDefinition {
            name: "CLI".to_string(),
            behavior: Behavior::Cli,
            apps: vec![
                app("com.apple.Terminal", "Terminal"),
                app("com.googlecode.iterm2", "iTerm"),
                app("net.kovidgoyal.kitty", "kitty"),
                app("com.github.wez.wezterm", "WezTerm"),
                app("dev.warp.Warp-Stable", "Warp"),
            ],
            stt_model: None,
            llm_refinement: LlmRefinement::Off,
            custom_instructions: None,
            structure_long_prompts: false,
            is_default: false,
        },
    ]
}

pub struct ResolvedSettings {
    pub mode: Behavior,
    /// Per-mode Whisper model override, settable in Settings. Consumed by
    /// `recording::transcribe_and_paste`, which resolves it to a path and
    /// passes it to `WhisperEngine::transcribe_with_model` — the engine
    /// keeps a small LRU of warm contexts (see stt.rs) so switching models
    /// across recordings doesn't repay the multi-second Metal shader
    /// compile every time.
    pub stt_model: Option<String>,
    pub llm_refinement: LlmRefinement,
    /// Extra LLM prompt instructions carried over from the resolved mode's
    /// `custom_instructions`. Meaningless when `llm_refinement` is `Off`.
    pub custom_instructions: Option<String>,
    pub structure_long_prompts: bool,
}

fn settings_for(def: &ModeDefinition) -> ResolvedSettings {
    ResolvedSettings {
        mode: def.behavior,
        stt_model: def.stt_model.clone(),
        llm_refinement: def.llm_refinement.clone(),
        custom_instructions: def.custom_instructions.clone(),
        structure_long_prompts: def.structure_long_prompts,
    }
}

/// Full resolution (mode + per-mode overrides), used by the recording
/// pipeline. Looks for a mode whose `apps` includes the frontmost app;
/// falls back to whichever mode has `is_default: true`; falls back further
/// to a hardcoded Plain/no-refinement if even that's been deleted, so
/// dictation never has nothing to fall back to.
pub fn resolve_settings(bundle_id: Option<&str>, modes: &[ModeDefinition]) -> ResolvedSettings {
    let hardcoded_fallback = ResolvedSettings {
        mode: Behavior::Plain,
        stt_model: None,
        llm_refinement: LlmRefinement::Off,
        custom_instructions: None,
        structure_long_prompts: false,
    };
    let Some(bundle_id) = bundle_id else {
        return hardcoded_fallback;
    };
    if let Some(def) = modes
        .iter()
        .find(|m| m.apps.iter().any(|a| a.bundle_id == bundle_id))
    {
        return settings_for(def);
    }
    modes
        .iter()
        .find(|m| m.is_default)
        .map(settings_for)
        .unwrap_or(hardcoded_fallback)
}

/// Resolves a specific mode by name — used for the widget's one-off
/// "next dictation" override, which picks a mode directly rather than
/// going through the frontmost-app lookup above.
pub fn settings_for_name(name: &str, modes: &[ModeDefinition]) -> Option<ResolvedSettings> {
    modes.iter().find(|m| m.name == name).map(settings_for)
}

pub fn apply_mode(mode: Behavior, transcript: &str) -> String {
    match mode {
        Behavior::Plain | Behavior::Casual => transcript.to_string(),
        Behavior::Cli => format_as_cli(transcript),
    }
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text[prefix.len()..].trim())
}

fn normalize_cli_input(text: &str) -> &str {
    text.trim().trim_end_matches('.')
}

/// Whether `text` matches one of the literal shell-command directives
/// `format_as_cli` mechanically translates (`git commit ...`, `make
/// directory ...`, `change directory to ...`) — as opposed to falling
/// through to file-reference tagging, the natural-language case for
/// dictating into a terminal-based coding agent rather than a literal
/// shell command.
///
/// Used to keep those already-final commands away from LLM refinement,
/// which can only corrupt them.
pub fn is_literal_cli_directive(text: &str) -> bool {
    let trimmed = normalize_cli_input(text);
    strip_prefix_ci(trimmed, "git commit ").is_some()
        || strip_prefix_ci(trimmed, "make directory ").is_some()
        || strip_prefix_ci(trimmed, "change directory to ").is_some()
}

fn format_as_cli(text: &str) -> String {
    let trimmed = normalize_cli_input(text);

    if let Some(message) = strip_prefix_ci(trimmed, "git commit ") {
        return format!("git commit -m \"{message}\"");
    }
    if let Some(dir) = strip_prefix_ci(trimmed, "make directory ") {
        return format!("mkdir {dir}");
    }
    if let Some(dir) = strip_prefix_ci(trimmed, "change directory to ") {
        return format!("cd {dir}");
    }

    // Anything that isn't one of the literal shell directives above falls
    // through to file tagging rather than being pasted verbatim — this is
    // deliberately *not* run over the branches above (a file mention
    // inside an actual `git commit` message shouldn't get an `@` stuck on
    // it). See file_tagging.rs for why this only works for terminal
    // coding agents (Claude Code, OpenCode, Gemini CLI), not editors.
    crate::file_tagging::tag_file_references(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(name: &str, behavior: Behavior, apps: Vec<&str>) -> ModeDefinition {
        ModeDefinition {
            name: name.to_string(),
            behavior,
            apps: apps
                .into_iter()
                .map(|id| AppRef {
                    bundle_id: id.to_string(),
                    app_name: id.to_string(),
                })
                .collect(),
            stt_model: None,
            llm_refinement: LlmRefinement::Off,
            custom_instructions: None,
            structure_long_prompts: false,
            is_default: false,
        }
    }

    #[test]
    fn no_bundle_id_resolves_to_plain() {
        assert_eq!(resolve_settings(None, &[]).mode, Behavior::Plain);
    }

    #[test]
    fn unknown_app_with_no_modes_resolves_to_plain() {
        assert_eq!(
            resolve_settings(Some("com.example.unknown"), &[]).mode,
            Behavior::Plain
        );
    }

    #[test]
    fn app_assigned_to_a_mode_resolves_to_it() {
        let modes = vec![mode("CLI", Behavior::Cli, vec!["com.apple.Terminal"])];
        assert_eq!(
            resolve_settings(Some("com.apple.Terminal"), &modes).mode,
            Behavior::Cli
        );
    }

    #[test]
    fn multiple_apps_can_share_one_mode() {
        let modes = vec![mode(
            "Messaging",
            Behavior::Casual,
            vec!["com.tinyspeck.slackmacgap", "com.hnc.Discord"],
        )];
        assert_eq!(
            resolve_settings(Some("com.tinyspeck.slackmacgap"), &modes).mode,
            Behavior::Casual
        );
        assert_eq!(
            resolve_settings(Some("com.hnc.Discord"), &modes).mode,
            Behavior::Casual
        );
    }

    #[test]
    fn unassigned_app_falls_back_to_the_default_mode() {
        let mut fallback = mode("Default", Behavior::Plain, vec![]);
        fallback.is_default = true;
        let modes = vec![
            mode("CLI", Behavior::Cli, vec!["com.apple.Terminal"]),
            fallback,
        ];
        assert_eq!(
            resolve_settings(Some("com.example.unknown"), &modes).mode,
            Behavior::Plain
        );
    }

    #[test]
    fn no_default_mode_falls_back_to_hardcoded_plain() {
        let modes = vec![mode("CLI", Behavior::Cli, vec!["com.apple.Terminal"])];
        let settings = resolve_settings(Some("com.example.unknown"), &modes);
        assert_eq!(settings.mode, Behavior::Plain);
        assert_eq!(settings.llm_refinement, LlmRefinement::Off);
    }

    #[test]
    fn llm_refinement_resolves_per_mode() {
        let mut m = mode(
            "Messaging",
            Behavior::Casual,
            vec!["com.tinyspeck.slackmacgap"],
        );
        m.llm_refinement = LlmRefinement::Global;
        let modes = vec![m];
        assert_eq!(
            resolve_settings(Some("com.tinyspeck.slackmacgap"), &modes).llm_refinement,
            LlmRefinement::Global
        );
    }

    #[test]
    fn stt_model_resolves_per_mode() {
        let mut m = mode("CLI", Behavior::Cli, vec!["com.apple.Terminal"]);
        m.stt_model = Some("small.en".to_string());
        let modes = vec![m];
        assert_eq!(
            resolve_settings(Some("com.apple.Terminal"), &modes).stt_model,
            Some("small.en".to_string())
        );
    }

    #[test]
    fn custom_instructions_resolve_per_mode() {
        let mut m = mode(
            "Messaging",
            Behavior::Casual,
            vec!["com.tinyspeck.slackmacgap"],
        );
        m.custom_instructions = Some("Always sign off with 'thanks, Jake'".to_string());
        let modes = vec![m];
        assert_eq!(
            resolve_settings(Some("com.tinyspeck.slackmacgap"), &modes).custom_instructions,
            Some("Always sign off with 'thanks, Jake'".to_string())
        );
    }

    #[test]
    fn settings_for_name_finds_the_named_mode() {
        let modes = vec![mode("CLI", Behavior::Cli, vec![])];
        let settings = settings_for_name("CLI", &modes).expect("mode should resolve");
        assert_eq!(settings.mode, Behavior::Cli);
        assert!(settings_for_name("Nonexistent", &modes).is_none());
    }

    #[test]
    fn seed_default_modes_has_exactly_one_default() {
        let seeded = seed_default_modes();
        assert_eq!(seeded.iter().filter(|m| m.is_default).count(), 1);
        assert_eq!(seeded.len(), 4);
    }

    #[test]
    fn plain_and_casual_pass_transcript_through_unchanged() {
        let text = "  Hello,  world.  ";
        assert_eq!(apply_mode(Behavior::Plain, text), text);
        assert_eq!(apply_mode(Behavior::Casual, text), text);
    }

    #[test]
    fn cli_mode_formats_git_commit() {
        assert_eq!(
            apply_mode(Behavior::Cli, "git commit update readme"),
            "git commit -m \"update readme\""
        );
        // Case-insensitive prefix match.
        assert_eq!(
            apply_mode(Behavior::Cli, "Git Commit fix the bug."),
            "git commit -m \"fix the bug\""
        );
    }

    #[test]
    fn cli_mode_formats_mkdir_and_cd() {
        assert_eq!(apply_mode(Behavior::Cli, "make directory src"), "mkdir src");
        assert_eq!(
            apply_mode(Behavior::Cli, "change directory to src"),
            "cd src"
        );
    }

    #[test]
    fn is_literal_cli_directive_matches_the_three_shell_directives() {
        assert!(is_literal_cli_directive("git commit update readme"));
        assert!(is_literal_cli_directive("Git Commit fix the bug."));
        assert!(is_literal_cli_directive("make directory src"));
        assert!(is_literal_cli_directive("change directory to src"));
    }

    #[test]
    fn is_literal_cli_directive_rejects_agent_chat_prose() {
        // Real 2026-09-09 dictation into a coding-agent terminal: prose
        // must not be mistaken for a literal shell command.
        assert!(!is_literal_cli_directive(
            "Let's test and see what happens when we try and paste in here. \
             If it truly is just a problem with this terminal, we still need to work it out."
        ));
        assert!(!is_literal_cli_directive("this doesn't match any pattern"));
        assert!(!is_literal_cli_directive(
            "look at recording.rs and fix the bug"
        ));
    }

    #[test]
    fn cli_mode_passes_through_unmatched_text() {
        assert_eq!(
            apply_mode(Behavior::Cli, "this doesn't match any pattern"),
            "this doesn't match any pattern"
        );
    }

    #[test]
    fn behavior_serializes_as_snake_case() {
        assert_eq!(serde_json::to_string(&Behavior::Cli).unwrap(), "\"cli\"");
        assert_eq!(
            serde_json::to_string(&Behavior::Casual).unwrap(),
            "\"casual\""
        );
        assert_eq!(
            serde_json::to_string(&Behavior::Plain).unwrap(),
            "\"plain\""
        );
    }
}

#[tauri::command]
pub fn get_modes(app: AppHandle) -> Vec<ModeDefinition> {
    config::load(&app).modes
}

/// Full-list replace, mirroring `snippets::set_snippets` — the Settings UI
/// keeps the whole list client-side (renaming, adding/removing apps,
/// adding/deleting modes are all just array edits) and saves it back in
/// one shot rather than the backend exposing per-field mutation commands.
#[tauri::command]
pub fn set_modes(app: AppHandle, modes: Vec<ModeDefinition>) {
    let mut cfg = config::load(&app);
    cfg.modes = modes;
    let _ = config::save(&app, &cfg);
}

#[derive(Serialize)]
pub struct RunningAppPayload {
    pub bundle_id: String,
    pub name: String,
    pub icon_data_uri: Option<String>,
    pub is_running: bool,
}

/// Lists apps for the "add to mode" picker in Settings: currently-running
/// apps first (so adding a rule doesn't require switching to the target
/// app first), then common developer/everyday apps that are installed but
/// not currently running. Icons included for both. NSWorkspace requires
/// the main thread; the command's own thread blocks on a channel waiting
/// for that dispatch to run.
#[tauri::command]
pub fn list_running_apps(app: AppHandle) -> Vec<RunningAppPayload> {
    let own_bundle_id = app.config().identifier.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let dispatched = app.run_on_main_thread(move || {
        let running = crate::app_detect::list_running_apps(&own_bundle_id);
        let common = crate::app_detect::list_common_apps(&running);
        let _ = tx.send((running, common));
    });
    if dispatched.is_err() {
        return Vec::new();
    }
    let (running, common) = rx.recv().unwrap_or_default();

    running
        .into_iter()
        .map(|info| RunningAppPayload {
            bundle_id: info.bundle_id,
            name: info.name,
            icon_data_uri: info.icon_data_uri,
            is_running: true,
        })
        .chain(common.into_iter().map(|info| RunningAppPayload {
            bundle_id: info.bundle_id,
            name: info.name,
            icon_data_uri: info.icon_data_uri,
            is_running: false,
        }))
        .collect()
}
