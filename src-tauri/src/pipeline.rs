//! The text half of dictation: everything between "Whisper produced this
//! transcript" and "this exact string gets delivered". Pure apart from the
//! LLM, which sits behind [`Refiner`] so tests can swap in a fake and run
//! the whole pipeline deterministically (see the golden cases below and
//! `tests/pipeline_cases.json`). Delivery (paste/copy, clipboard append,
//! history, webhook) stays in `recording.rs`.

use std::time::Instant;

use crate::config::AppConfig;
use crate::modes::ResolvedSettings;

/// The LLM operations the pipeline needs. `OllamaRefiner` is the real one.
pub trait Refiner {
    fn refine(
        &self,
        mode: crate::modes::Behavior,
        text: &str,
        model: &str,
        custom_instructions: Option<&str>,
        vocabulary: &[String],
    ) -> Result<String, String>;
    fn boilerplate(&self, request: &str, model: &str) -> Result<String, String>;
    /// Reorganises a long dictation (see `structure.rs`).
    fn structure(&self, _text: &str, _model: &str) -> Result<String, String> {
        Err("structuring isn't available here".to_string())
    }
    /// The user asked to paste as-is before refinement started.
    fn skipped(&self) -> bool {
        false
    }
}

/// The real LLM, streamed (see `llm::refine_streaming`): `on_progress`
/// gets the exact fraction written, and `cancel` (the user's "paste
/// as-is", by click or shortcut) stops it immediately or, if set before
/// refinement starts, skips it.
pub struct OllamaRefiner<'a> {
    pub on_progress: &'a dyn Fn(u8),
    pub cancel: &'a std::sync::atomic::AtomicBool,
}

impl Refiner for OllamaRefiner<'_> {
    fn refine(
        &self,
        mode: crate::modes::Behavior,
        text: &str,
        model: &str,
        custom_instructions: Option<&str>,
        vocabulary: &[String],
    ) -> Result<String, String> {
        crate::llm::refine_streaming(
            crate::llm::RefineRequest {
                mode,
                text,
                model,
                custom_instructions,
                vocabulary,
            },
            self.on_progress,
            self.cancel,
        )
    }

    fn boilerplate(&self, request: &str, model: &str) -> Result<String, String> {
        crate::llm::generate_boilerplate(request, model)
    }

    fn structure(&self, text: &str, model: &str) -> Result<String, String> {
        crate::llm::structure_prompt(text, model)
    }

    fn skipped(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// "Structure long prompts": a long dictation is reorganised by the LLM,
/// and the result is used only if `structure::check` finds nothing lost or
/// added.
fn structure_if_long(
    text: String,
    cfg: &AppConfig,
    settings: &crate::modes::ResolvedSettings,
    refiner: &dyn Refiner,
    on_refining: &mut dyn FnMut(&str),
    unrefined: &mut Option<String>,
    llm_ms: &mut Option<u64>,
    features_used: &mut Vec<String>,
) -> String {
    if !settings.structure_long_prompts || !crate::structure::long_enough(&text) || refiner.skipped() {
        return text;
    }
    let model = match &settings.llm_refinement {
        crate::modes::LlmRefinement::Model(id) => id.clone(),
        _ => cfg.llm_model.clone(),
    };
    on_refining(&model);
    let started = Instant::now();
    let result = refiner.structure(&text, &model);
    *llm_ms = Some(llm_ms.unwrap_or(0) + started.elapsed().as_millis() as u64);
    match result {
        Ok(structured) => match crate::structure::check(&text, &structured) {
            Ok(()) => {
                crate::applog!("structure: {model} structured {} words", text.split_whitespace().count());
                features_used.push("structured".to_string());
                if unrefined.is_none() {
                    *unrefined = Some(text);
                }
                structured
            }
            Err(why) => {
                crate::applog!("structure: rejected {model}'s version ({why}), pasting as spoken");
                text
            }
        },
        Err(err) => {
            crate::applog!("structure: {model} failed ({err}), pasting as spoken");
            text
        }
    }
}

/// Why a transcript would benefit from LLM cleanup, or `None` if it's
/// already clean. Deliberately about things Whisper leaves in (filler,
/// stutters, an unpunctuated run-on), not style: the deterministic passes
/// already handle punctuation commands, lists, casing and corrections.
pub fn refinement_needed(text: &str) -> Option<&'static str> {
    const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "er", "erm", "hmm", "mm"];
    const FILLER_PHRASES: &[&str] = &["you know,", "i mean,"];
    // Doubled words that are often correct English ("had had", "that that").
    const LEGIT_DOUBLES: &[&str] = &["had", "that", "is"];

    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();

    if words.iter().any(|w| FILLERS.contains(&w.as_str())) {
        return Some("filler words");
    }
    let lower = text.to_lowercase();
    if FILLER_PHRASES.iter().any(|p| lower.contains(p)) {
        return Some("filler words");
    }
    if words.windows(2).any(|pair| {
        pair[0] == pair[1] && pair[0].len() > 1 && !LEGIT_DOUBLES.contains(&pair[0].as_str())
    }) {
        return Some("repeated words");
    }
    if words.len() >= 12 && !text.contains(['.', ',', '?', '!', ';', ':']) {
        return Some("no punctuation");
    }
    None
}

/// What the pipeline decided to deliver, plus what the caller needs to
/// finish delivery and record it.
#[derive(Debug, Clone, PartialEq)]
pub struct Formatted {
    pub text: String,
    /// Which branch produced `text` ("snippet", "casing", "boilerplate",
    /// or the mode behavior, e.g. "Cli"). Recorded in history.
    pub mode_label: String,
    pub press_enter: bool,
    /// "Append clipboard" was spoken; the caller appends the clipboard's
    /// text, since reading it is a side effect.
    pub append_clipboard: bool,
    /// "Attach the error" was spoken; the caller appends the most recent
    /// error output as a fenced block (reading it is a side effect).
    pub attach_error: bool,
    /// "Attach the selection / the diff", "look at this": what else the
    /// caller appends (see `attachments.rs`), in spoken order.
    pub attachments: Vec<crate::attachments::Kind>,
    /// The whole dictation was "scratch that" / "undo that": no text, the
    /// caller undoes the last paste instead.
    pub undo_last: bool,
    pub features_used: Vec<String>,
    pub spoken_words: Option<u32>,
    /// Time spent waiting on the LLM, if it was called.
    pub llm_ms: Option<u64>,
    /// The text before the LLM rewrote it (after lists, punctuation, mode
    /// formatting), set only when refinement ran and changed the result.
    /// Chosen over the raw Whisper transcript because it is what "paste
    /// as-is" delivers: restoring it gives a usable entry, where the raw
    /// transcript would bring back spoken "comma"/"new line" commands.
    pub unrefined: Option<String>,
}

/// Runs a cleaned transcript through lists, punctuation, backtrack, voice
/// commands, snippets, casing, boilerplate, mode formatting and LLM
/// refinement. `on_refining` fires right before an LLM call, with the
/// model's name, so the widget can switch to "Refining…".
#[cfg(test)]
pub fn format(
    text: String,
    cfg: &AppConfig,
    settings: &ResolvedSettings,
    refiner: &dyn Refiner,
    on_refining: &mut dyn FnMut(&str),
) -> Formatted {
    format_in_project(text, cfg, settings, None, refiner, on_refining)
}

/// [`format`] with the names from the project being dictated into (see
/// `project.rs`): spoken names in the transcript become the real
/// identifiers and file paths before mode formatting, and the project's top
/// names go to the LLM so refinement doesn't "correct" them.
pub fn format_in_project(
    text: String,
    cfg: &AppConfig,
    settings: &ResolvedSettings,
    project: Option<&crate::project::Project>,
    refiner: &dyn Refiner,
    on_refining: &mut dyn FnMut(&str),
) -> Formatted {
    // Tracks which of the passes below actually changed the text
    // (as opposed to running as a no-op pass-through), for the
    // Insights feature-adoption checklist — "have you tried
    // Backtrack?" needs to know whether it's ever fired, not just
    // that the function was called on every dictation.
    let mut features_used: Vec<String> = Vec::new();
    let mut llm_ms = None;
    let mut unrefined = None;

    // The user's learned replacements (see `corrections.rs`) fix known
    // mis-hearings first, so every later pass sees the corrected word.
    let text = crate::corrections::apply_replacements(&text, &cfg.replacements);

    // A standalone "scratch that" undoes the last paste. Checked first, on
    // the raw transcript: backtrack (below) only handles the mid-sentence
    // form and leaves a lone trigger as text.
    if crate::undo::is_undo_command(&text) {
        return Formatted {
            text: String::new(),
            mode_label: "undo".to_string(),
            press_enter: false,
            append_clipboard: false,
            attach_error: false,
            attachments: Vec::new(),
            undo_last: true,
            features_used: vec!["undo".to_string()],
            spoken_words: Some(0),
            llm_ms: None,
            unrefined: None,
        };
    }

    // Spoken numbered lists ("one... two...") are expanded before
    // punctuation commands — it needs to see the raw marker words
    // ("one", "two") before anything else touches them, and it
    // hands back a real newline-separated list for the
    // punctuation pass (and everything downstream) to treat as
    // ordinary already-formatted text.
    let before_lists = text.clone();
    let text = crate::punctuation::expand_lists(&text);
    if text != before_lists {
        features_used.push("lists".to_string());
    }

    // Named punctuation commands ("period", "open paren", "new
    // line") are expanded next, ahead of everything else — like
    // casing commands below, they're cross-cutting rather than
    // mode-gated, and downstream steps (casing extraction, LLM
    // refinement) all work better against already-punctuated text
    // than against the literal spoken words.
    let before_punct = text.clone();
    let text = crate::punctuation::expand_punctuation(&text);
    if text != before_punct {
        features_used.push("punctuation".to_string());
    }

    // Backtrack ("...at two, actually three") collapses a
    // self-correction down to just the corrected tail. Runs after
    // punctuation expansion, not before — its "actually" trigger
    // requires a literal preceding comma, which only exists once
    // a spoken "comma" (or Whisper's own natural comma insertion)
    // has already been resolved to the character.
    let before_backtrack = text.clone();
    let text = crate::backtrack::try_backtrack(&text);
    if text != before_backtrack {
        features_used.push("backtrack".to_string());
    }

    // "Press enter": stripped before casing/boilerplate/mode so
    // none of those see the trailing control phrase as content.
    // Gated behind a Settings toggle (default off) — an
    // unexpected Enter keystroke is a worse failure mode than an
    // unexpected paste, so unlike the other punctuation commands
    // above this one needs an explicit opt-in.
    let (text, should_press_enter) = if cfg.press_enter_enabled {
        crate::punctuation::extract_press_enter(&text)
    } else {
        (text, false)
    };
    if should_press_enter {
        features_used.push("press_enter".to_string());
    }

    // "Attach the error": stripped the same way, after "press enter" (the
    // later-spoken phrase goes first). Always on: the phrases are
    // deliberate, and nothing happens beyond appending a block.
    // Other attachments ("look at this, attach the diff") can sit on
    // either side of "attach the error", so strip them before and after.
    let (text, mut attachments) = crate::attachments::extract(&text);
    let (text, should_attach_error) = crate::error_context::try_extract_trigger(&text);
    let (text, earlier) = crate::attachments::extract(&text);
    let earlier: Vec<_> = earlier.into_iter().filter(|k| !attachments.contains(k)).collect();
    attachments.splice(0..0, earlier);
    if should_attach_error {
        features_used.push("attach_error".to_string());
    }
    for kind in &attachments {
        let feature = kind.feature().to_string();
        if !features_used.contains(&feature) {
            features_used.push(feature);
        }
    }

    // Word count of what was actually spoken, captured here (post
    // punctuation/backtrack/press-enter, pre snippet/casing/
    // boilerplate) rather than derived from `formatted` below —
    // see the doc comment on `HistoryEntry::spoken_words` for why.
    let spoken_words = Some(text.split_whitespace().count() as u32);

    // "Append clipboard": stripped the same way as "press enter"
    // above (and checked after it, so "...append clipboard press
    // enter" strips the later-spoken phrase first). Always on,
    // unlike "press enter" — the trigger phrases are multi-word and
    // deliberate, not something said by accident the way a bare
    // Enter keystroke risk would warrant an opt-in.
    let (text, should_append_clipboard) = crate::clipboard::try_extract_trigger(&text);

    // Snippets ("pr checklist", "standup update") are the most
    // explicit, intentional signal of the pre-LLM checks — a
    // literal saved macro the user (or a shipped default) defined
    // themselves — so they're checked first, ahead of casing
    // commands and boilerplate requests, in case of a coincidental
    // overlap. Like those, skips mode formatting and LLM
    // refinement entirely: the output is already fully resolved.
    let (formatted, mode_label) =
        if let Some(expanded) = crate::snippets::try_expand(&text, &cfg.snippets) {
            crate::applog!(
                "snippets: trigger matched, transcript={}",
                crate::logging::said(&text)
            );
            (expanded, "snippet".to_string())
        }
        // Casing directives ("snake case error response handler") are a
        // cross-cutting syntax command, not gated behind a Mode — they
        // apply no matter which app/mode is active, and skip both
        // rule-based formatting and LLM refinement entirely since the
        // mechanical transform already fully resolves the output.
        else if let Some(cased) = crate::syntax::try_apply_casing_command(&text) {
            crate::applog!(
                "syntax: casing command matched, transcript={} output={}",
                crate::logging::said(&text),
                crate::logging::said(&cased)
            );
            (cased, "casing".to_string())
        } else if let Some(request) = crate::boilerplate::try_extract_request(&text) {
            crate::applog!(
                "boilerplate: request matched, transcript={} request={}",
                crate::logging::said(&text),
                crate::logging::said(&request)
            );
            on_refining(&cfg.llm_model);
            let boilerplate_started = Instant::now();
            let result = refiner.boilerplate(&request, &cfg.llm_model);
            llm_ms = Some(boilerplate_started.elapsed().as_millis() as u64);
            match result {
                Ok(code) => (code, "boilerplate".to_string()),
                Err(err) => {
                    // Falls back to normal mode formatting rather than
                    // pasting nothing — a down/missing Ollama shouldn't
                    // eat the user's dictation.
                    crate::applog!(
                        "boilerplate: generation failed, falling back to plain formatting: {err}"
                    );
                    let formatted = crate::modes::apply_mode(settings.mode, &text);
                    (formatted, format!("{:?}", settings.mode))
                }
            }
        } else {
            // Names from the project (code and file names), for terminals and
            // editors only: not Casual messaging, and not the literal shell
            // directives, whose arguments are already final.
            let project = project.filter(|_| {
                settings.mode != crate::modes::Behavior::Casual
                    && !(settings.mode == crate::modes::Behavior::Cli
                        && crate::modes::is_literal_cli_directive(&text))
            });
            let named = project.map(|p| {
                p.index
                    .apply(&text, settings.mode == crate::modes::Behavior::Cli)
            });
            let formatted = match named {
                Some((named, n)) if n > 0 => {
                    crate::applog!("project: replaced {n} spoken names");
                    crate::modes::apply_mode(settings.mode, &named)
                }
                _ => crate::modes::apply_mode(settings.mode, &text),
            };
            crate::applog!(
                "modes: transcript={} formatted={}",
                crate::logging::said(&text),
                crate::logging::said(&formatted)
            );

            let llm_model = match &settings.llm_refinement {
                crate::modes::LlmRefinement::Off => None,
                crate::modes::LlmRefinement::Global => Some(cfg.llm_model.as_str()),
                crate::modes::LlmRefinement::Model(id) => Some(id.as_str()),
            };
            // A literal spoken command (`git commit ...`, `make directory ...`)
            // is already a finished shell command after `apply_mode`; sending it
            // through the LLM only risks corrupting it (observed: `mkdir for
            // the new service` came back as bare `mkdir`).
            let llm_model = if settings.mode == crate::modes::Behavior::Cli
                && crate::modes::is_literal_cli_directive(&text)
            {
                None
            } else {
                llm_model
            };
            // Refinement costs seconds and usually hands back the text
            // unchanged, so a clean transcript skips it. Tone-changing modes
            // (Casual) and modes with custom instructions always refine: there
            // the rewrite is the point.
            let llm_model = llm_model.filter(|model| {
                if refiner.skipped() {
                    crate::applog!("llm: pasting as-is (skipped {model} at the user's request)");
                    return false;
                }
                let always = !cfg.llm_only_when_needed
                    || settings.mode == crate::modes::Behavior::Casual
                    || settings
                        .custom_instructions
                        .as_deref()
                        .is_some_and(|i| !i.trim().is_empty());
                if always {
                    return true;
                }
                match refinement_needed(&formatted) {
                    Some(reason) => {
                        crate::applog!("llm: refining with {model} ({reason})");
                        true
                    }
                    None => {
                        crate::applog!("llm: transcript already clean, skipping {model}");
                        false
                    }
                }
            });
            let formatted = if let Some(model) = llm_model {
                on_refining(model);
                let refine_started = Instant::now();
                // The user's own terms first, then the project's top names.
                let mut vocabulary = cfg.vocabulary.clone();
                if let Some(project) = project {
                    vocabulary.extend(project.terms.iter().cloned());
                }
                let result = refiner.refine(
                    settings.mode,
                    &formatted,
                    model,
                    settings.custom_instructions.as_deref(),
                    &vocabulary,
                );
                llm_ms = Some(refine_started.elapsed().as_millis() as u64);
                match result {
                    Ok(refined) => match crate::llm::degenerate_output(&formatted, &refined) {
                        None => {
                            crate::applog!(
                                "llm: {model} refined {} -> {}",
                                crate::logging::said(&formatted),
                                crate::logging::said(&refined)
                            );
                            // The model sometimes drops the `@` that marks a
                            // file reference for terminal coding agents; tagging
                            // skips tokens already tagged, so re-running it is safe.
                            let refined = if settings.mode == crate::modes::Behavior::Cli {
                                crate::file_tagging::tag_file_references(&refined)
                            } else {
                                refined
                            };
                            if refined != formatted {
                                unrefined = Some(formatted.clone());
                            }
                            refined
                        }
                        Some(reason) => {
                            crate::applog!(
                                "llm: rejected {model} refinement ({reason}): {}; pasting {}",
                                crate::logging::said(&refined),
                                crate::logging::said(&formatted)
                            );
                            formatted
                        }
                    },
                    Err(err) => {
                        crate::applog!("llm: refinement failed, pasting unrefined text: {err}");
                        formatted
                    }
                }
            } else {
                formatted
            };
            let formatted = structure_if_long(
                formatted,
                cfg,
                settings,
                refiner,
                on_refining,
                &mut unrefined,
                &mut llm_ms,
                &mut features_used,
            );
            (formatted, format!("{:?}", settings.mode))
        };

    Formatted {
        text: formatted,
        mode_label,
        press_enter: should_press_enter,
        append_clipboard: should_append_clipboard,
        attach_error: should_attach_error,
        attachments,
        undo_last: false,
        features_used,
        spoken_words,
        llm_ms,
        unrefined,
    }
}

/// Golden cases: what a spoken phrase should deliver, end to end through
/// the pipeline, with a scripted LLM. Add a case to
/// `tests/fixtures/pipeline_cases.json` for every dictation bug fixed.
#[cfg(test)]
mod golden {
    use super::*;
    use crate::modes::{Behavior, LlmRefinement};
    use std::cell::Cell;

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        input: String,
        mode: String,
        #[serde(default)]
        press_enter_enabled: bool,
        /// Turns off the "only when needed" gate, for cases about what the
        /// LLM returns rather than whether it runs.
        #[serde(default)]
        always_refine: bool,
        /// Turns on "Structure long prompts" for the case's mode.
        #[serde(default)]
        structure: bool,
        llm: Option<ScriptedLlm>,
        expect: Option<String>,
        expect_prefix: Option<String>,
        expect_label: Option<String>,
        expect_press_enter: Option<bool>,
        expect_append_clipboard: Option<bool>,
        expect_attach_error: Option<bool>,
        /// e.g. ["screenshot", "diff"], in spoken order.
        expect_attachments: Option<Vec<String>>,
        expect_undo_last: Option<bool>,
        expect_llm_called: Option<bool>,
        /// Names in the dictated-into project, as `"file:src/paste.rs"` /
        /// `"id:paste_text"` / `"dep:serde"`; tags a name `"recent:"` to
        /// rank it as touched in the last commits.
        #[serde(default)]
        project: Vec<String>,
        /// Terms the LLM must be given (user vocabulary plus these).
        expect_llm_terms: Option<Vec<String>>,
        /// The text before refinement, when refinement changed it.
        expect_unrefined: Option<String>,
        /// Whether `unrefined` should be set at all.
        expect_refined: Option<bool>,
        /// Learned `[from, to]` replacements applied before formatting.
        #[serde(default)]
        replacements: Vec<(String, String)>,
    }

    fn fake_project(names: &[String]) -> Option<crate::project::Project> {
        use crate::names::{Kind, NameEntry, NameIndex, SCORE_RECENT_COMMIT, SCORE_TRACKED};
        if names.is_empty() {
            return None;
        }
        let entries: Vec<NameEntry> = names
            .iter()
            .map(|n| {
                let (recent, n) = n
                    .strip_prefix("recent:")
                    .map_or((false, n.as_str()), |r| (true, r));
                let (kind, text) = match n.split_once(':') {
                    Some(("file", t)) => (Kind::File, t),
                    Some(("dep", t)) => (Kind::Dependency, t),
                    Some(("id", t)) => (Kind::Identifier, t),
                    _ => panic!("bad project name {n:?}"),
                };
                NameEntry {
                    text: text.to_string(),
                    kind,
                    score: if recent {
                        SCORE_RECENT_COMMIT
                    } else {
                        SCORE_TRACKED
                    },
                }
            })
            .collect();
        let index = NameIndex::new(entries);
        let terms = index.top_terms(40);
        Some(crate::project::Project {
            name: "fake".into(),
            index,
            terms,
            root: "/fake".into(),
        })
    }

    #[derive(serde::Deserialize, Clone)]
    #[serde(rename_all = "lowercase")]
    enum ScriptedLlm {
        Returns(String),
        Error(String),
    }

    struct FakeRefiner {
        script: Option<ScriptedLlm>,
        calls: Cell<u32>,
        vocabulary: std::cell::RefCell<Vec<String>>,
    }

    impl FakeRefiner {
        fn answer(&self) -> Result<String, String> {
            self.calls.set(self.calls.get() + 1);
            match &self.script {
                Some(ScriptedLlm::Returns(text)) => Ok(text.clone()),
                Some(ScriptedLlm::Error(err)) => Err(err.clone()),
                None => panic!("LLM called but the case scripts no answer"),
            }
        }
    }

    impl Refiner for FakeRefiner {
        fn refine(
            &self,
            _: Behavior,
            _: &str,
            _: &str,
            _: Option<&str>,
            vocabulary: &[String],
        ) -> Result<String, String> {
            *self.vocabulary.borrow_mut() = vocabulary.to_vec();
            self.answer()
        }
        fn boilerplate(&self, _: &str, _: &str) -> Result<String, String> {
            self.answer()
        }
        fn structure(&self, _: &str, _: &str) -> Result<String, String> {
            self.answer()
        }
    }

    fn behavior(name: &str) -> Behavior {
        match name {
            "plain" => Behavior::Plain,
            "casual" => Behavior::Casual,
            "cli" => Behavior::Cli,
            other => panic!("unknown mode {other:?}"),
        }
    }

    #[test]
    fn pipeline_golden_cases() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../tests/fixtures/pipeline_cases.json")).unwrap();
        assert!(cases.len() >= 25, "golden suite shrank to {}", cases.len());

        let mut failures = Vec::new();
        for case in &cases {
            let cfg = AppConfig {
                press_enter_enabled: case.press_enter_enabled,
                llm_only_when_needed: !case.always_refine,
                replacements: case
                    .replacements
                    .iter()
                    .map(|(from, to)| crate::corrections::Replacement {
                        from: from.clone(),
                        to: to.clone(),
                    })
                    .collect(),
                ..AppConfig::default()
            };
            let settings = ResolvedSettings {
                mode: behavior(&case.mode),
                stt_model: None,
                llm_refinement: if case.llm.is_some() {
                    LlmRefinement::Global
                } else {
                    LlmRefinement::Off
                },
                custom_instructions: None,
                structure_long_prompts: case.structure,
            };
            let refiner = FakeRefiner {
                script: case.llm.clone(),
                calls: Cell::new(0),
                vocabulary: Default::default(),
            };
            let mut refining_events = 0;
            let project = fake_project(&case.project);
            let out = format_in_project(
                case.input.clone(),
                &cfg,
                &settings,
                project.as_ref(),
                &refiner,
                &mut |_| refining_events += 1,
            );

            let mut problems = Vec::new();
            if let Some(expect) = &case.expect {
                if &out.text != expect {
                    problems.push(format!("text {:?}, expected {expect:?}", out.text));
                }
            }
            if let Some(prefix) = &case.expect_prefix {
                if !out.text.starts_with(prefix.as_str()) {
                    problems.push(format!("text {:?} doesn't start with {prefix:?}", out.text));
                }
            }
            if let Some(label) = &case.expect_label {
                if &out.mode_label != label {
                    problems.push(format!("label {:?}, expected {label:?}", out.mode_label));
                }
            }
            if let Some(enter) = case.expect_press_enter {
                if out.press_enter != enter {
                    problems.push(format!("press_enter {}, expected {enter}", out.press_enter));
                }
            }
            if let Some(append) = case.expect_append_clipboard {
                if out.append_clipboard != append {
                    problems.push(format!(
                        "append_clipboard {}, expected {append}",
                        out.append_clipboard
                    ));
                }
            }
            if let Some(expected) = &case.expect_attachments {
                let got: Vec<String> = out
                    .attachments
                    .iter()
                    .map(|k| format!("{k:?}").to_lowercase())
                    .collect();
                if &got != expected {
                    failures.push(format!("{}: attachments {got:?}, expected {expected:?}", case.name));
                }
            }
            if let Some(attach) = case.expect_attach_error {
                if out.attach_error != attach {
                    problems.push(format!(
                        "attach_error {}, expected {attach}",
                        out.attach_error
                    ));
                }
            }
            if let Some(undo) = case.expect_undo_last {
                if out.undo_last != undo {
                    problems.push(format!("undo_last {}, expected {undo}", out.undo_last));
                }
            }
            if let Some(called) = case.expect_llm_called {
                if (refiner.calls.get() > 0) != called {
                    problems.push(format!(
                        "LLM called {} times, expected called={called}",
                        refiner.calls.get()
                    ));
                }
            }
            if let Some(terms) = &case.expect_llm_terms {
                let given = refiner.vocabulary.borrow();
                for term in terms {
                    if !given.contains(term) {
                        problems.push(format!("LLM wasn't given {term:?}"));
                    }
                }
            }
            if let Some(expect) = &case.expect_unrefined {
                if out.unrefined.as_ref() != Some(expect) {
                    problems.push(format!(
                        "unrefined {:?}, expected {expect:?}",
                        out.unrefined
                    ));
                }
            }
            if let Some(refined) = case.expect_refined {
                if out.unrefined.is_some() != refined {
                    problems.push(format!(
                        "unrefined {:?}, expected refined={refined}",
                        out.unrefined
                    ));
                }
            }
            if refining_events != refiner.calls.get() {
                problems.push(format!(
                    "{refining_events} refining events for {} LLM calls",
                    refiner.calls.get()
                ));
            }
            if !problems.is_empty() {
                failures.push(format!("{}:\n    {}", case.name, problems.join("\n    ")));
            }
        }
        assert!(
            failures.is_empty(),
            "{} golden case(s) failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }

    #[test]
    fn refinement_needed_flags_what_whisper_leaves_in() {
        assert_eq!(
            refinement_needed("Um, can you check this?"),
            Some("filler words")
        );
        assert_eq!(
            refinement_needed("It's, you know, fine."),
            Some("filler words")
        );
        assert_eq!(
            refinement_needed("Check the the logs."),
            Some("repeated words")
        );
        assert_eq!(
            refinement_needed(
                "so then we deploy it and check the logs and restart the service again"
            ),
            Some("no punctuation")
        );
        for clean in [
            "I love all these ideas though, I'm good with you proceeding with them.",
            "He had had enough.",
            "Look at @recording.rs and fix the crash",
            "ship it",
        ] {
            assert_eq!(refinement_needed(clean), None, "{clean}");
        }
    }

    #[test]
    fn llm_time_is_reported_only_when_the_llm_runs() {
        let cfg = AppConfig::default();
        let mut settings = ResolvedSettings {
            mode: Behavior::Casual,
            stt_model: None,
            llm_refinement: LlmRefinement::Off,
            custom_instructions: None,
            structure_long_prompts: false,
        };
        let refiner = FakeRefiner {
            script: Some(ScriptedLlm::Returns("Ship it.".into())),
            calls: Cell::new(0),
            vocabulary: Default::default(),
        };
        assert_eq!(
            format("ship it".into(), &cfg, &settings, &refiner, &mut |_| {}).llm_ms,
            None
        );
        settings.llm_refinement = LlmRefinement::Global;
        assert!(
            format("ship it".into(), &cfg, &settings, &refiner, &mut |_| {})
                .llm_ms
                .is_some()
        );
    }
}
