//! End-to-end dictation tests with real audio: macOS `say` speaks each
//! phrase in `tests/fixtures/audio_cases.json` to a 16kHz WAV, the real
//! Whisper model transcribes it, and the real pipeline formats it, exactly
//! as a push-to-talk recording would go (minus the mic and the paste).
//!
//! Ignored by default (needs macOS `say` and the downloaded model, and
//! takes ~30s). Run with `bun run test:audio`, which also prints a word
//! error rate and latency report usable for comparing models.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::config::AppConfig;
use crate::modes::{Behavior, LlmRefinement, ResolvedSettings};
use crate::pipeline::{self, Refiner};
use crate::stt::WhisperEngine;

#[derive(serde::Deserialize)]
struct AudioCase {
    name: String,
    say: String,
    mode: String,
    expect: String,
    /// Allowed word error rate against `expect` (case and punctuation
    /// ignored). 0 means the words must match exactly.
    #[serde(default)]
    max_wer: f64,
    #[serde(default)]
    press_enter_enabled: bool,
    expect_press_enter: Option<bool>,
    /// Why a tolerance is set, when it isn't 0. Documentation only.
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
}

struct NoLlm;

impl Refiner for NoLlm {
    fn refine(
        &self,
        _: Behavior,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: &[String],
    ) -> Result<String, String> {
        Err("LLM disabled in audio e2e".into())
    }
    fn boilerplate(&self, _: &str, _: &str) -> Result<String, String> {
        Err("LLM disabled in audio e2e".into())
    }
}

/// Lowercased words with surrounding punctuation stripped, keeping the
/// characters that carry meaning inside developer tokens (`_`, `@`, `.`).
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '@')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word error rate: word-level edit distance over the reference length.
fn wer(reference: &str, hypothesis: &str) -> f64 {
    let (r, h) = (words(reference), words(hypothesis));
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut cur = vec![i; h.len() + 1];
        for j in 1..=h.len() {
            let substitution = prev[j - 1] + usize::from(r[i - 1] != h[j - 1]);
            cur[j] = substitution.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len() as f64
}

fn speak(text: &str, out: &Path) -> Result<(), String> {
    speak_as("Samantha", text, out)
}

fn speak_as(voice: &str, text: &str, out: &Path) -> Result<(), String> {
    let status = Command::new("say")
        .args(["-v", voice, "--data-format=LEI16@16000", "-o"])
        .arg(out)
        .arg(text)
        .status()
        .map_err(|e| format!("couldn't run `say`: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("`say` exited with {status}"))
}

fn model_path() -> PathBuf {
    std::env::var_os("DW_E2E_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/ggml-base.en-q5_1.bin")
        })
}

#[test]
#[ignore = "needs macOS `say` and a Whisper model; run with `bun run test:audio`"]
fn dictation_from_real_audio() {
    let model = model_path();
    assert!(
        model.exists(),
        "no Whisper model at {} (run ./scripts/download-model.sh)",
        model.display()
    );
    let cases: Vec<AudioCase> =
        serde_json::from_str(include_str!("../tests/fixtures/audio_cases.json")).unwrap();

    let dir = std::env::temp_dir().join(format!("dw-audio-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let engine = WhisperEngine::new("e2e".into(), model.clone());
    engine.ensure_loaded().expect("model failed to load");

    let mut failures = Vec::new();
    let mut total_wer = 0.0;
    let mut stt_ms_total = 0u128;
    println!("\n{:<28} {:>6} {:>7}  output", "case", "WER", "stt ms");
    for case in &cases {
        let wav = dir.join(format!("{}.wav", case.name.replace(' ', "_")));
        speak(&case.say, &wav).unwrap();

        let started = Instant::now();
        let samples = crate::stt::load_samples_16k_mono(&wav).unwrap();
        let ticks = std::sync::Mutex::new(Vec::new());
        let raw = engine
            .transcribe_samples_with_progress(&samples, None, &|p| ticks.lock().unwrap().push(p))
            .unwrap();
        let ticks = ticks.into_inner().unwrap();
        assert!(
            ticks.windows(2).all(|w| w[0] < w[1]) && ticks.last().is_some_and(|&p| p < 100),
            "progress must rise monotonically and stay below 100 until done: {ticks:?}"
        );
        let stt_ms = started.elapsed().as_millis();
        stt_ms_total += stt_ms;

        let cleaned = crate::stt::strip_non_speech(&raw);
        let cfg = AppConfig {
            press_enter_enabled: case.press_enter_enabled,
            ..AppConfig::default()
        };
        let settings = ResolvedSettings {
            mode: match case.mode.as_str() {
                "cli" => Behavior::Cli,
                "casual" => Behavior::Casual,
                _ => Behavior::Plain,
            },
            stt_model: None,
            llm_refinement: LlmRefinement::Off,
            custom_instructions: None,
        };
        let out = pipeline::format(cleaned, &cfg, &settings, &NoLlm, &mut |_| {});

        let error_rate = wer(&case.expect, &out.text);
        total_wer += error_rate;
        println!(
            "{:<28} {:>6.3} {:>7}  {:?}  ({} progress updates, last {}%)",
            case.name,
            error_rate,
            stt_ms,
            out.text,
            ticks.len(),
            ticks.last().copied().unwrap_or(0)
        );

        let mut problems = Vec::new();
        if error_rate > case.max_wer {
            problems.push(format!(
                "WER {error_rate:.3} > {} (whisper heard {raw:?}, delivered {:?}, expected {:?})",
                case.max_wer, out.text, case.expect
            ));
        }
        if case.max_wer == 0.0 && out.text.contains('\n') != case.expect.contains('\n') {
            problems.push(format!(
                "line structure differs: {:?} vs {:?}",
                out.text, case.expect
            ));
        }
        if let Some(enter) = case.expect_press_enter {
            if out.press_enter != enter {
                problems.push(format!("press_enter {} expected {enter}", out.press_enter));
            }
        }
        if !problems.is_empty() {
            failures.push(format!("{}: {}", case.name, problems.join("; ")));
        }
    }
    println!(
        "\nmodel {}: mean WER {:.3}, mean stt {}ms over {} cases\n",
        model.file_name().unwrap().to_string_lossy(),
        total_wer / cases.len() as f64,
        stt_ms_total / cases.len() as u128,
        cases.len()
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "{} case(s) failed:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// Live chunking against real speech: three phrases separated by long
/// pauses go through the real Silero VAD and the chunker exactly as a
/// push-to-talk recording would, are transcribed chunk by chunk with the
/// previous text as context, and the joined result is compared with
/// transcribing the whole buffer. Prints both WERs and the work left at
/// stop.
#[test]
#[ignore = "needs macOS `say` and a Whisper model; run with `bun run test:audio`"]
fn chunked_dictation_with_long_pauses_matches_whole_buffer() {
    let model = model_path();
    assert!(model.exists(), "no Whisper model at {}", model.display());
    let vad_model = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/silero_vad.onnx");
    let phrases = [
        "I want to refactor the authentication middleware so that it validates tokens before hitting the database.",
        "Then run the migration script and check that the users table has the new column.",
        "Finally open a pull request and ask Alice to review it.",
    ];
    let pauses = [6.0f32, 20.0];

    let dir = std::env::temp_dir().join(format!("dw-audio-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut recording: Vec<f32> = Vec::new();
    for (i, phrase) in phrases.iter().enumerate() {
        let wav = dir.join(format!("phrase{i}.wav"));
        speak(phrase, &wav).unwrap();
        recording.extend(crate::stt::load_samples_16k_mono(&wav).unwrap());
        if let Some(pause) = pauses.get(i) {
            recording.extend(std::iter::repeat_n(0.0f32, (pause * 16_000.0) as usize));
        }
    }
    let expected = phrases.join(" ");
    let audio_s = recording.len() as f32 / 16_000.0;

    let engine = WhisperEngine::new("e2e".into(), model);
    engine.ensure_loaded().expect("model failed to load");

    let started = Instant::now();
    let whole = engine.transcribe_samples(&recording, None).unwrap();
    let whole_ms = started.elapsed().as_millis();

    let vad = crate::live::vad_from_model(&vad_model).expect("VAD failed to load");
    let mut is_speech = |window: &[f32]| {
        vad.accept_waveform(window);
        let speech = vad.detected();
        vad.clear();
        speech
    };
    let mut segmenter = crate::live::Segmenter::new();
    let mut texts: Vec<String> = Vec::new();
    for piece in recording.chunks(1600) {
        for chunk in segmenter.push(piece, &mut is_speech) {
            let context = crate::live::join_texts(&texts);
            let text = engine
                .transcribe_samples_in_context(&chunk, None, &context, &|_| {})
                .unwrap();
            texts.push(crate::stt::strip_non_speech(&text));
        }
    }
    let chunks = texts.len();
    let tail = segmenter.finish();
    let tail_started = Instant::now();
    if let Some(tail) = &tail {
        let context = crate::live::join_texts(&texts);
        let text = engine
            .transcribe_samples_in_context(tail, None, &context, &|_| {})
            .unwrap();
        texts.push(crate::stt::strip_non_speech(&text));
    }
    let tail_ms = tail_started.elapsed().as_millis();
    let chunked = crate::live::join_texts(&texts);
    let kept_s = segmenter.emitted_samples() as f32 / 16_000.0;

    let whole_wer = wer(&expected, &crate::stt::strip_non_speech(&whole));
    let chunked_wer = wer(&expected, &chunked);
    println!(
        "\nlive chunking: {audio_s:.1}s recording, {chunks} chunks + tail, {kept_s:.1}s transcribed\n  whole buffer: WER {whole_wer:.3}, {whole_ms}ms to text after stop\n  chunked:      WER {chunked_wer:.3}, {tail_ms}ms to text after stop\n  whole   {whole:?}\n  chunked {chunked:?}\n"
    );
    let _ = std::fs::remove_dir_all(&dir);

    assert!(chunks >= 1, "expected the long pauses to cut chunks");
    assert!(
        kept_s < audio_s * 0.6,
        "silence should have been skipped ({kept_s:.1}s of {audio_s:.1}s)"
    );
    assert!(
        chunked_wer <= whole_wer + 0.05,
        "chunked WER {chunked_wer:.3} is worse than whole-buffer {whole_wer:.3}"
    );
    assert!(chunked_wer <= 0.2, "chunked WER {chunked_wer:.3}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wer_ignores_case_and_punctuation() {
        assert_eq!(wer("Hello, world.", "hello world"), 0.0);
        assert_eq!(wer("a b c d", "a x c d"), 0.25);
        assert_eq!(wer("a b", ""), 1.0);
        assert_eq!(
            wer("look at @recording.rs", "look at recording.rs"),
            1.0 / 3.0
        );
    }
}

/// Live drafts: a long sentence spoken without a pause is only cut into a
/// chunk at its end, so without drafts nothing shows until then. Streams
/// the audio as if live (wall-clock time is simulated from audio time plus
/// measured inference time) and reports when text first appears, how fast
/// the quick draft pass is against a full pass, and the drafts' accuracy.
#[test]
#[ignore = "needs macOS `say` and a Whisper model; run with `bun run test:audio`"]
fn live_drafts_show_words_before_the_phrase_ends() {
    let model = model_path();
    assert!(model.exists(), "no Whisper model at {}", model.display());
    let vad_model = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/silero_vad.onnx");
    let sentence = "So what I'd like to do next is split the recording module into two parts, \
                    one that only captures audio and one that decides when to stop, \
                    and then add tests for both of them before we touch the widget again.";
    let dir = std::env::temp_dir().join(format!("dw-audio-draft-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("sentence.wav");
    speak(sentence, &wav).unwrap();
    let mut recording = crate::stt::load_samples_16k_mono(&wav).unwrap();
    recording.extend(std::iter::repeat_n(0.0f32, 16_000)); // a second of silence after
    let audio_s = recording.len() as f32 / 16_000.0;

    let engine = WhisperEngine::new("e2e".into(), model);
    engine.ensure_loaded().expect("model failed to load");
    let vad = crate::live::vad_from_model(&vad_model).expect("VAD failed to load");
    let mut is_speech = |window: &[f32]| {
        vad.accept_waveform(window);
        let speech = vad.detected();
        vad.clear();
        speech
    };

    let mut segmenter = crate::live::Segmenter::new();
    let mut first_final_at: Option<f32> = None;
    let mut first_draft_at: Option<f32> = None;
    let mut busy_until = 0.0f32; // simulated time the transcriber is free
    let mut last_draft = -10.0f32;
    let mut drafted_speech = 0usize;
    let mut drafts = 0;
    let (mut quick_ms, mut quick_audio) = (0u128, 0.0f32);
    let mut last_draft_text = String::new();
    for (i, piece) in recording.chunks(1600).enumerate() {
        let now = (i + 1) as f32 * 0.1;
        for chunk in segmenter.push(piece, &mut is_speech) {
            let started = Instant::now();
            let text = engine
                .transcribe_samples_in_context(&chunk, None, "", &|_| {})
                .unwrap();
            let done = now.max(busy_until) + started.elapsed().as_secs_f32();
            busy_until = done;
            if !crate::stt::strip_non_speech(&text).is_empty() {
                first_final_at.get_or_insert(done);
            }
        }
        let pending = segmenter.pending_speech();
        if now - last_draft >= 0.7
            && pending >= 9_600
            && pending >= drafted_speech + 6_400
            && now >= busy_until
        {
            if let Some(audio) = segmenter.pending_audio() {
                let started = Instant::now();
                let text = engine.transcribe_preview(&audio, None, "").unwrap();
                let elapsed = started.elapsed();
                quick_ms += elapsed.as_millis();
                quick_audio += audio.len() as f32 / 16_000.0;
                busy_until = now + elapsed.as_secs_f32();
                last_draft = now;
                drafted_speech = pending;
                drafts += 1;
                let text = crate::stt::strip_non_speech(&text);
                if !text.is_empty() {
                    first_draft_at.get_or_insert(busy_until);
                    last_draft_text = text;
                }
            }
        }
    }
    if let Some(tail) = segmenter.finish() {
        let started = Instant::now();
        let text = engine
            .transcribe_samples_in_context(&tail, None, "", &|_| {})
            .unwrap();
        if !crate::stt::strip_non_speech(&text).is_empty() {
            first_final_at.get_or_insert(audio_s + started.elapsed().as_secs_f32());
        }
    }

    // Full pass vs quick pass on the same clip (the last draft's audio).
    let clip = segmenter_clip(&recording);
    let started = Instant::now();
    let full_text = engine.transcribe_samples(&clip, None).unwrap();
    let full_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let quick_text = engine.transcribe_preview(&clip, None, "").unwrap();
    let quick_clip_ms = started.elapsed().as_millis();
    let draft_wer = wer(sentence, &crate::stt::strip_non_speech(&quick_text));
    let full_wer = wer(sentence, &crate::stt::strip_non_speech(&full_text));

    println!(
        "\nlive drafts: {audio_s:.1}s of continuous speech, {drafts} drafts ({:.0} ms per audio second)\n  first text without drafts: {:.1}s\n  first text with drafts:    {:.1}s\n  same {:.1}s clip: full pass {full_ms}ms (WER {full_wer:.3}), quick pass {quick_clip_ms}ms (WER {draft_wer:.3})\n  last draft {last_draft_text:?}\n",
        quick_ms as f32 / quick_audio.max(0.1),
        first_final_at.unwrap_or(f32::NAN),
        first_draft_at.unwrap_or(f32::NAN),
        clip.len() as f32 / 16_000.0,
    );
    let _ = std::fs::remove_dir_all(&dir);

    let first_draft = first_draft_at.expect("drafts should show text during the sentence");
    let first_final = first_final_at.expect("the sentence should be transcribed");
    assert!(
        first_draft < first_final - 2.0,
        "drafts should show words well before the phrase is cut ({first_draft:.1}s vs {first_final:.1}s)"
    );
    assert!(
        draft_wer <= 0.25,
        "draft WER {draft_wer:.3} is too high for a preview"
    );
}

/// The speech part of a recording (trailing silence dropped), for timing
/// one clip both ways.
fn segmenter_clip(recording: &[f32]) -> Vec<f32> {
    let end = recording
        .iter()
        .rposition(|s| s.abs() > 0.001)
        .map_or(recording.len(), |i| i + 1);
    recording[..end].to_vec()
}

/// Times the full and quick passes fairly: alternating, several times, at
/// several clip lengths (the first call on a model is slower). Drafts are
/// short clips, where the quick pass has to win.
#[test]
#[ignore = "needs macOS `say` and a Whisper model; run with `bun run test:audio`"]
fn quick_pass_is_faster_on_short_clips() {
    let model = model_path();
    let dir = std::env::temp_dir().join(format!("dw-audio-quick-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("s.wav");
    speak("So what I'd like to do next is split the recording module into two parts, one that only captures audio and one that decides when to stop, and then add tests for both of them.", &wav).unwrap();
    let audio = crate::stt::load_samples_16k_mono(&wav).unwrap();
    let engine = WhisperEngine::new("e2e".into(), model);
    engine.ensure_loaded().unwrap();
    let _ = engine.transcribe_samples(&audio[..32_000], None); // warm up
    for secs in [1.5f32, 3.0, 5.0, 8.0] {
        let clip = &audio[..((secs * 16_000.0) as usize).min(audio.len())];
        let (mut full, mut quick) = (Vec::new(), Vec::new());
        for _ in 0..4 {
            let t = Instant::now();
            let _ = engine.transcribe_samples(clip, None).unwrap();
            full.push(t.elapsed().as_millis());
            let t = Instant::now();
            let _ = engine.transcribe_preview(clip, None, "").unwrap();
            quick.push(t.elapsed().as_millis());
        }
        full.sort();
        quick.sort();
        println!(
            "clip {secs:>4.1}s: full median {}ms, quick median {}ms",
            full[2], quick[2]
        );
        if secs <= 5.0 {
            assert!(
                quick[2] < full[2],
                "quick pass not faster on a {secs}s clip"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs a scripted meeting (macOS voices taking turns) through meeting
/// mode's speaker separation and transcription. Each voice has to come out
/// as one speaker, different from the others, and the words have to match.
fn check_meeting(name: &str, script: &[(&str, &str)], speakers: u32) {
    let dir = std::env::temp_dir().join(format!("dw-meeting-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut samples = Vec::new();
    let mut truth = Vec::new(); // (start, end, voice)
    for (i, (voice, text)) in script.iter().enumerate() {
        let wav = dir.join(format!("{i}.wav"));
        speak_as(voice, text, &wav).unwrap();
        let start = samples.len() as f32 / 16_000.0;
        samples.extend(crate::stt::load_samples_16k_mono(&wav).unwrap());
        truth.push((start, samples.len() as f32 / 16_000.0, *voice));
        samples.extend(std::iter::repeat_n(0.0, 16_000 * 7 / 10));
    }

    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let models = crate::meeting::DiarizationModels {
        segmentation: resources.join("pyannote_segmentation_3_0.onnx"),
        embedding: resources.join("wespeaker_en_voxceleb_resnet34_LM.onnx"),
    };
    let started = Instant::now();
    let turns = crate::meeting::diarize(&samples, &models, speakers).unwrap();
    println!(
        "\n{name}: separated {:.0}s of audio in {}ms ({} turns)",
        samples.len() as f32 / 16_000.0,
        started.elapsed().as_millis(),
        turns.len()
    );

    // Which separated speaker covers most of each scripted line.
    let mut voice_to_speaker: std::collections::HashMap<&str, Vec<i32>> = Default::default();
    for &(start, end, voice) in &truth {
        let mut overlap: std::collections::HashMap<i32, f32> = Default::default();
        for t in &turns {
            let o = end.min(t.end) - start.max(t.start);
            if o > 0.0 {
                *overlap.entry(t.speaker).or_default() += o;
            }
        }
        let best = overlap
            .into_iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| s)
            .unwrap_or(-1);
        println!("  {voice:<9} {start:>5.1}-{end:>5.1}s -> speaker {best}");
        voice_to_speaker.entry(voice).or_default().push(best);
    }
    let mut assigned = Vec::new();
    for (voice, speakers) in &voice_to_speaker {
        assert!(
            speakers.iter().all(|s| *s == speakers[0] && *s >= 0),
            "{name}: {voice}'s lines went to different speakers: {speakers:?}"
        );
        assigned.push(speakers[0]);
    }
    assigned.sort();
    assigned.dedup();
    assert_eq!(
        assigned.len(),
        voice_to_speaker.len(),
        "{name}: voices weren't kept apart"
    );

    let engine = WhisperEngine::new("e2e".into(), model_path());
    engine.ensure_loaded().unwrap();
    let names = crate::meeting::speaker_names(&turns, None);
    let mut lines = Vec::new();
    for t in &turns {
        let a = (t.start * 16_000.0) as usize;
        let b = ((t.end * 16_000.0) as usize).min(samples.len());
        let text = engine.transcribe_samples(&samples[a..b], None).unwrap();
        lines.push(crate::meeting::Line {
            start: t.start,
            speaker: names[&t.speaker].clone(),
            text: crate::stt::strip_non_speech(&text).trim().to_string(),
        });
    }
    let detail = crate::meeting::meeting_detail(samples.len() as f32 / 16_000.0, &lines);
    println!("{}", crate::meeting::render_markdown(name, &detail, &lines));
    let heard: String = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let said: String = script.iter().map(|(_, t)| *t).collect::<Vec<_>>().join(" ");
    let error = wer(&said, &heard);
    println!("transcript WER {error:.3}");
    assert!(error < 0.15, "{name}: transcript WER {error:.3}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Auto speaker count, two clearly different voices.
#[test]
#[ignore = "needs macOS `say` and a Whisper model; run with `bun run test:audio`"]
fn meeting_finds_two_speakers_on_its_own() {
    check_meeting(
        "two speakers, auto",
        &[
            ("Samantha", "Okay, let's get started. The main thing today is the release on Friday."),
            ("Daniel", "Right. The build is green, but the installer still needs a final check on a clean machine."),
            ("Samantha", "Can you do that check this afternoon?"),
            ("Daniel", "Yes, I have a spare laptop with nothing installed on it. I will post the results in the channel."),
            ("Samantha", "Perfect. Let's meet again on Thursday to make the final call."),
        ],
        0,
    );
}

/// Three voices, two of them close (macOS's US and Australian female
/// voices score 0.78 alike, more than different people usually do): with
/// the speaker count given, all three are kept apart.
#[test]
#[ignore = "needs macOS `say` and a Whisper model; run with `bun run test:audio`"]
fn meeting_separates_three_speakers_when_told_how_many() {
    check_meeting(
        "three speakers, count given",
        &[
            ("Samantha", "Okay, let's get started. The main thing today is the release on Friday."),
            ("Daniel", "Right. The build is green, but the installer still needs a final check on a clean machine."),
            ("Karen", "I can do that this afternoon. I have a spare laptop with nothing installed on it."),
            ("Samantha", "Great, thanks. Daniel, is the migration ready as well?"),
            ("Daniel", "Yes, it moves the old settings over and leaves a link behind, and the tests pass."),
            ("Karen", "Then I think we are in good shape. I will post the results in the channel."),
            ("Samantha", "Perfect. Let's meet again on Thursday to make the final call."),
        ],
        3,
    );
}
