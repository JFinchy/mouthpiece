//! Voice enrollment: records a short clip of the primary user speaking,
//! computes a speaker embedding from it, and persists it as
//! `voice_profile.json` for `isolate.rs`'s embedding-based masking path.
//!
//! Enrollment and dictation share the same underlying recorder
//! (`RecordingState::audio`); `capture.rs` keeps the two mutually exclusive,
//! so a hotkey press mid-enrollment can't misfire `transcribe_and_paste` on
//! the enrollment clip.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use crate::capture::{Request, Transition};
use crate::recording::RecordingState;
use crate::stt::WHISPER_SAMPLE_RATE;

/// A user has to talk for at least this long (of actual detected speech,
/// not clip duration) before enrollment is trusted enough to fingerprint —
/// mirrors `audio.rs`'s "no audio captured" style friendly-error rather
/// than silently enrolling off a fraction-of-a-second clip.
const MIN_VOICED_SAMPLES: usize = WHISPER_SAMPLE_RATE as usize * 3;

/// Identifies which embedding model produced a persisted profile, so a
/// future model swap can detect a stale profile and force re-enrollment
/// instead of silently comparing incompatible vectors.
const MODEL_ID: &str = "wespeaker_en_voxceleb_resnet34_LM";

pub struct VoiceIsolationState {
    extractor: Mutex<Option<SpeakerEmbeddingExtractor>>,
    /// In-memory cache of the persisted embedding, loaded at startup (see
    /// `lib.rs`) so `isolate::apply` never has to hit disk per recording.
    enrolled: Mutex<Option<Vec<f32>>>,
}

impl VoiceIsolationState {
    pub fn new() -> Self {
        Self {
            extractor: Mutex::new(None),
            enrolled: Mutex::new(None),
        }
    }

    /// Loads the persisted profile (if any) into the in-memory cache. Called
    /// once at startup — `isolate::apply` reads via `enrolled_embedding()`.
    pub fn load_persisted(&self, app: &AppHandle) {
        if let Some(profile) = read_profile(app) {
            *self.enrolled.lock().unwrap() = Some(profile.embedding);
        }
    }

    pub fn enrolled_embedding(&self) -> Option<Vec<f32>> {
        self.enrolled.lock().unwrap().clone()
    }

    fn ensure_loaded(&self, app: &AppHandle) -> Result<(), String> {
        let mut guard = self.extractor.lock().unwrap();
        if guard.is_some() {
            return Ok(());
        }
        let model_path = resolve_model_path(app)?;
        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(model_path.to_string_lossy().to_string()),
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        };
        let extractor = SpeakerEmbeddingExtractor::create(&config)
            .ok_or_else(|| "failed to load the speaker embedding model".to_string())?;
        *guard = Some(extractor);
        Ok(())
    }

    /// Computes one embedding from the voiced portion of `samples` (per
    /// `voiced`, from `isolate::energy_gate`) — used both for enrollment and
    /// for scoring a candidate segment against the enrolled profile.
    pub fn compute_embedding(
        &self,
        app: &AppHandle,
        samples: &[f32],
        voiced: &[(usize, usize)],
    ) -> Result<Vec<f32>, String> {
        self.ensure_loaded(app)?;
        let guard = self.extractor.lock().unwrap();
        let extractor = guard
            .as_ref()
            .expect("ensure_loaded just populated this or returned Err");

        let mut voiced_only = Vec::new();
        for &(start, end) in voiced {
            let start = start.min(samples.len());
            let end = end.min(samples.len());
            voiced_only.extend_from_slice(&samples[start..end]);
        }

        let stream = extractor
            .create_stream()
            .ok_or_else(|| "failed to create an embedding stream".to_string())?;
        stream.accept_waveform(WHISPER_SAMPLE_RATE as i32, &voiced_only);
        stream.input_finished();
        extractor
            .compute(&stream)
            .ok_or_else(|| "failed to compute a speaker embedding".to_string())
    }
}

fn resource_dir_model_path(app: &AppHandle) -> Option<PathBuf> {
    let path = app
        .path()
        .resource_dir()
        .ok()?
        .join("models")
        .join(format!("{MODEL_ID}.onnx"));
    path.exists().then_some(path)
}

/// Dev convenience: fall back to the repo-local `resources/` copy if the
/// bundled resource dir doesn't resolve it yet (e.g. `cargo tauri dev`
/// before a full bundled build) — mirrors `models.rs::dev_fallback_path`.
fn dev_fallback_model_path() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(format!("{MODEL_ID}.onnx"));
    path.exists().then_some(path)
}

pub(crate) fn resolve_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    resource_dir_model_path(app)
        .or_else(dev_fallback_model_path)
        .ok_or_else(|| "speaker embedding model not found".to_string())
}

#[derive(Serialize, Deserialize)]
struct VoiceProfile {
    embedding: Vec<f32>,
    model_id: String,
    enrolled_at_ms: u64,
}

fn profile_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("voice_profile.json"))
}

/// Silently ignores a missing/corrupt/stale-model-id profile rather than
/// erroring — the caller treats `None` as "not enrolled", the same state a
/// fresh install starts in.
fn read_profile(app: &AppHandle) -> Option<VoiceProfile> {
    let path = profile_path(app).ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    let profile: VoiceProfile = serde_json::from_str(&contents).ok()?;
    (profile.model_id == MODEL_ID).then_some(profile)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn save_profile(app: &AppHandle, embedding: &[f32]) -> Result<u64, String> {
    let enrolled_at_ms = now_ms();
    let profile = VoiceProfile {
        embedding: embedding.to_vec(),
        model_id: MODEL_ID.to_string(),
        enrolled_at_ms,
    };
    let path = profile_path(app)?;
    let json = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(enrolled_at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: loads the real bundled WeSpeaker ONNX model and computes
    /// an embedding on a synthetic tone, confirming sherpa-onnx is wired up
    /// end-to-end. Skips rather than fails if the resource file isn't
    /// present in this checkout — mirrors
    /// `stt.rs::transcribes_without_panicking`'s skip-if-missing pattern.
    #[test]
    fn computes_a_dim_length_embedding_on_a_synthetic_tone() {
        let model_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(format!("{MODEL_ID}.onnx"));
        if !model_path.exists() {
            eprintln!("skipping: {model_path:?} not present");
            return;
        }

        let config = SpeakerEmbeddingExtractorConfig {
            model: Some(model_path.to_string_lossy().to_string()),
            num_threads: 1,
            debug: false,
            provider: Some("cpu".to_string()),
        };
        let extractor =
            SpeakerEmbeddingExtractor::create(&config).expect("failed to create extractor");

        let samples: Vec<f32> = (0..WHISPER_SAMPLE_RATE * 2)
            .map(|i| {
                (i as f32 * 220.0 * std::f32::consts::TAU / WHISPER_SAMPLE_RATE as f32).sin() * 0.3
            })
            .collect();

        let stream = extractor.create_stream().expect("failed to create stream");
        stream.accept_waveform(WHISPER_SAMPLE_RATE as i32, &samples);
        stream.input_finished();

        let embedding = extractor.compute(&stream).expect("compute returned None");
        assert_eq!(embedding.len(), extractor.dim() as usize);
    }
}

#[tauri::command]
pub fn start_voice_enrollment(app: AppHandle) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    let _op = state.capture_op.lock().unwrap();
    match state.capture.request(Request::StartEnrollment) {
        Transition::StartedEnrollment { .. } => {
            state.audio.start();
            let _ = app.emit("enrollment-started", ());
            Ok(())
        }
        Transition::Rejected(reason) => Err(reason.to_string()),
        other => Err(format!("unexpected capture transition: {other:?}")),
    }
}

#[tauri::command]
pub fn stop_voice_enrollment(app: AppHandle) {
    let state = app.state::<RecordingState>();
    let _op = state.capture_op.lock().unwrap();
    if !matches!(
        state.capture.request(Request::StopEnrollment),
        Transition::StoppedEnrollment { .. }
    ) {
        return;
    }

    match state.audio.stop() {
        Ok(path) => {
            let app = app.clone();
            std::thread::spawn(move || {
                process_enrollment(&app, &path);
                let _ = std::fs::remove_file(&path);
            });
        }
        Err(err) => {
            let _ = app.emit("enrollment-error", err);
        }
    }
}

fn process_enrollment(app: &AppHandle, wav_path: &Path) {
    let samples = match crate::stt::load_samples_16k_mono(wav_path) {
        Ok(samples) => samples,
        Err(err) => {
            let _ = app.emit("enrollment-error", err);
            return;
        }
    };

    let voiced = crate::isolate::energy_gate(&samples);
    let voiced_samples: usize = voiced.iter().map(|&(start, end)| end - start).sum();
    if voiced_samples < MIN_VOICED_SAMPLES {
        let _ = app.emit(
            "enrollment-error",
            "not enough clear speech captured — try again in a quieter spot and speak for a few more seconds".to_string(),
        );
        return;
    }

    let vi_state = app.state::<VoiceIsolationState>();
    let embedding = match vi_state.compute_embedding(app, &samples, &voiced) {
        Ok(embedding) => embedding,
        Err(err) => {
            let _ = app.emit("enrollment-error", err);
            return;
        }
    };

    let enrolled_at_ms = match save_profile(app, &embedding) {
        Ok(ms) => ms,
        Err(err) => {
            let _ = app.emit("enrollment-error", err);
            return;
        }
    };

    *vi_state.enrolled.lock().unwrap() = Some(embedding);

    let mut cfg = crate::config::load(app);
    cfg.voice_enrolled = true;
    let _ = crate::config::save(app, &cfg);

    let _ = app.emit("enrollment-complete", enrolled_at_ms);
}

#[derive(Serialize)]
pub struct EnrollmentStatus {
    pub enrolled: bool,
    pub enrolled_at_ms: Option<u64>,
}

/// Self-heals against actual `voice_profile.json` presence rather than
/// trusting `cfg.voice_enrolled` blindly — e.g. if the file was deleted out
/// from under the app, this corrects the config back to `false` instead of
/// leaving the toggle claiming an embedding path that no longer exists.
#[tauri::command]
pub fn get_voice_enrollment_status(app: AppHandle) -> EnrollmentStatus {
    let profile = read_profile(&app);

    let mut cfg = crate::config::load(&app);
    let should_be_enrolled = profile.is_some();
    if cfg.voice_enrolled != should_be_enrolled {
        cfg.voice_enrolled = should_be_enrolled;
        let _ = crate::config::save(&app, &cfg);
    }

    EnrollmentStatus {
        enrolled: should_be_enrolled,
        enrolled_at_ms: profile.map(|p| p.enrolled_at_ms),
    }
}
