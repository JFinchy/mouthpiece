/// Insights: a usage-and-adoption dashboard, loosely modeled on Wispr
/// Flow's "Your usage" tab but adapted for a local, single-user app —
/// Flow's words-per-minute card ranks you against their whole userbase
/// ("Top 0.1%") and their "fixes made" card counts cloud-LLM corrections;
/// neither has an honest local equivalent, so this drops the percentile
/// (there's no population to rank against) and reframes "fixes" as
/// "smart formatting applied" (deterministic passes that actually fired,
/// not LLM edits).
///
/// Also adds something Flow's version doesn't have: a feature-adoption
/// checklist. Most of Mouthpiece's power (Vocabulary, Snippets, Backtrack,
/// casing commands, per-app Modes) is opt-in and easy to forget exists —
/// this surfaces what's actually being used versus what's configured but
/// idle, with a one-line suggestion for each unused one.
///
/// Pure aggregation over `Vec<HistoryEntry>` + `AppConfig`, no Tauri
/// dependency in the core logic — same shape as the other deterministic
/// modules (`punctuation.rs`, `snippets.rs`), which makes it fully unit
/// testable without a real app handle or filesystem.
use serde::Serialize;
use std::collections::HashMap;

use crate::config::AppConfig;
use crate::history::HistoryEntry;

#[derive(Serialize)]
pub struct AppUsage {
    pub app_name: String,
    pub dictations: u64,
    pub percent: f64,
}

#[derive(Serialize)]
pub struct StreakInfo {
    pub current_days: u32,
    pub longest_days: u32,
    /// "YYYY-MM-DD" -> dictation count, UTC calendar days. The frontend
    /// lays these into a weekday-row/month-column grid itself; note this
    /// is UTC-day bucketed, not the user's local midnight, since Rust's
    /// std doesn't know the local UTC offset without a extra crate —
    /// close enough for a streak indicator, occasionally off by one
    /// dictation near a day boundary.
    pub daily_counts: HashMap<String, u32>,
}

#[derive(Serialize)]
pub struct AdoptionItem {
    pub key: String,
    pub label: String,
    pub done: bool,
    pub suggestion: String,
}

#[derive(Serialize)]
pub struct AdoptionInfo {
    pub score_percent: u32,
    pub items: Vec<AdoptionItem>,
}

#[derive(Serialize)]
pub struct InsightsPayload {
    pub total_dictations: u64,
    pub total_words: u64,
    pub smart_formatting_count: u64,
    /// `None` until at least one dictation has a recorded duration (older
    /// entries, or a wav whose header failed to read, won't have one).
    pub avg_words_per_minute: Option<f64>,
    pub personal_best_wpm: Option<f64>,
    /// Median and 90th-percentile time from releasing the hotkey to the
    /// text landing, over the most recent `LATENCY_WINDOW` dictations that
    /// recorded it. `None` until there's at least one.
    pub latency: Option<LatencyStats>,
    pub app_usage: Vec<AppUsage>,
    pub streak: StreakInfo,
    pub adoption: AdoptionInfo,
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Words actually *spoken* for an entry — `spoken_words` when present,
/// falling back to counting the delivered `text` for entries logged
/// before that field existed. See `HistoryEntry::spoken_words` for why
/// `text` alone (the delivered output, not the input) is the wrong thing
/// to count for entries that went through a snippet/casing/boilerplate
/// substitution.
fn spoken_word_count(entry: &HistoryEntry) -> u64 {
    entry
        .spoken_words
        .map(|n| n as u64)
        .unwrap_or_else(|| word_count(&entry.text) as u64)
}

/// Days since the Unix epoch (1970-01-01) for a UTC millisecond timestamp.
fn days_since_epoch(timestamp_ms: u64) -> i64 {
    timestamp_ms as i64 / 86_400_000
}

/// Proleptic-Gregorian (y, m, d) from a day count since 1970-01-01, via
/// Howard Hinnant's `civil_from_days` algorithm
/// (http://howardhinnant.github.io/date_algorithms.html) — pure integer
/// math, so this doesn't need to pull in a full date/time crate just to
/// label calendar days for a streak heatmap.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn date_key(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn compute_streak(days_with_activity: &std::collections::BTreeSet<i64>, today: i64) -> (u32, u32) {
    if days_with_activity.is_empty() {
        return (0, 0);
    }

    // Current streak: walk backward from today (or yesterday, if nothing
    // logged yet today) while consecutive days are present.
    let mut current = 0u32;
    let start = if days_with_activity.contains(&today) {
        today
    } else if days_with_activity.contains(&(today - 1)) {
        today - 1
    } else {
        // Neither today nor yesterday has activity — streak is broken.
        i64::MIN
    };
    if start != i64::MIN {
        let mut day = start;
        while days_with_activity.contains(&day) {
            current += 1;
            day -= 1;
        }
    }

    // Longest streak: longest run of consecutive integers in the sorted set.
    let mut longest = 0u32;
    let mut run = 0u32;
    let mut prev: Option<i64> = None;
    for &day in days_with_activity {
        match prev {
            Some(p) if day == p + 1 => run += 1,
            _ => run = 1,
        }
        longest = longest.max(run);
        prev = Some(day);
    }

    (current, longest)
}

#[derive(Serialize, Debug, PartialEq)]
pub struct LatencyStats {
    pub median_ms: u64,
    pub p90_ms: u64,
    pub samples: u32,
}

/// Recent enough to reflect the current model/LLM setup, large enough that
/// one slow cold start doesn't dominate the median.
const LATENCY_WINDOW: usize = 50;

fn compute_latency(entries: &[HistoryEntry]) -> Option<LatencyStats> {
    let mut recent: Vec<u64> = entries
        .iter()
        .rev()
        .filter_map(|e| e.latency_ms)
        .take(LATENCY_WINDOW)
        .collect();
    if recent.is_empty() {
        return None;
    }
    recent.sort_unstable();
    let at = |q: f64| recent[((recent.len() - 1) as f64 * q).round() as usize];
    Some(LatencyStats {
        median_ms: at(0.5),
        p90_ms: at(0.9),
        samples: recent.len() as u32,
    })
}

pub fn compute(entries: &[HistoryEntry], cfg: &AppConfig, now_ms: u64) -> InsightsPayload {
    let total_dictations = entries.len() as u64;
    let total_words: u64 = entries.iter().map(spoken_word_count).sum();

    let smart_formatting_count = entries
        .iter()
        .filter(|e| {
            !e.features_used.is_empty()
                || matches!(
                    e.mode.as_deref(),
                    Some("casing") | Some("snippet") | Some("boilerplate")
                )
        })
        .count() as u64;

    let wpm_samples: Vec<f64> = entries
        .iter()
        .filter_map(|e| {
            let duration_ms = e.duration_ms?;
            if duration_ms == 0 {
                return None;
            }
            let minutes = duration_ms as f64 / 60_000.0;
            Some(spoken_word_count(e) as f64 / minutes)
        })
        .collect();
    let avg_words_per_minute = if wpm_samples.is_empty() {
        None
    } else {
        Some(wpm_samples.iter().sum::<f64>() / wpm_samples.len() as f64)
    };
    let personal_best_wpm = wpm_samples
        .iter()
        .cloned()
        .fold(None, |best: Option<f64>, v| {
            Some(best.map_or(v, |b| b.max(v)))
        });

    let mut app_counts: HashMap<String, u64> = HashMap::new();
    for entry in entries {
        let name = entry
            .app_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        *app_counts.entry(name).or_insert(0) += 1;
    }
    let mut app_usage: Vec<AppUsage> = app_counts
        .into_iter()
        .map(|(app_name, dictations)| AppUsage {
            app_name,
            percent: if total_dictations > 0 {
                dictations as f64 / total_dictations as f64 * 100.0
            } else {
                0.0
            },
            dictations,
        })
        .collect();
    app_usage.sort_by(|a, b| {
        b.dictations
            .cmp(&a.dictations)
            .then_with(|| a.app_name.cmp(&b.app_name))
    });

    let mut daily_counts: HashMap<String, u32> = HashMap::new();
    let mut days_with_activity: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    for entry in entries {
        let day = days_since_epoch(entry.timestamp_ms);
        days_with_activity.insert(day);
        *daily_counts.entry(date_key(day)).or_insert(0) += 1;
    }
    let today = days_since_epoch(now_ms);
    let (current_days, longest_days) = compute_streak(&days_with_activity, today);

    let adoption = compute_adoption(entries, cfg);

    InsightsPayload {
        total_dictations,
        total_words,
        smart_formatting_count,
        avg_words_per_minute,
        personal_best_wpm,
        latency: compute_latency(entries),
        app_usage,
        streak: StreakInfo {
            current_days,
            longest_days,
            daily_counts,
        },
        adoption,
    }
}

fn used_feature(entries: &[HistoryEntry], key: &str) -> bool {
    entries
        .iter()
        .any(|e| e.features_used.iter().any(|f| f == key))
}

fn used_mode(entries: &[HistoryEntry], mode: &str) -> bool {
    entries.iter().any(|e| e.mode.as_deref() == Some(mode))
}

fn compute_adoption(entries: &[HistoryEntry], cfg: &AppConfig) -> AdoptionInfo {
    let items = vec![
        AdoptionItem {
            key: "vocabulary".to_string(),
            label: "Custom vocabulary".to_string(),
            done: cfg.vocabulary != crate::stt::default_vocabulary(),
            suggestion: "Add jargon Whisper mis-hears to Vocabulary so it's recognized correctly."
                .to_string(),
        },
        AdoptionItem {
            key: "snippets".to_string(),
            label: "Snippets".to_string(),
            done: used_mode(entries, "snippet"),
            suggestion:
                "Say a snippet trigger like \"pr checklist\" to paste a saved block of text instantly."
                    .to_string(),
        },
        AdoptionItem {
            key: "punctuation".to_string(),
            label: "Punctuation commands".to_string(),
            done: used_feature(entries, "punctuation") || used_feature(entries, "lists"),
            suggestion: "Say \"period\", \"new line\", or \"open paren\" to punctuate hands-free."
                .to_string(),
        },
        AdoptionItem {
            key: "backtrack".to_string(),
            label: "Backtrack".to_string(),
            done: used_feature(entries, "backtrack"),
            suggestion:
                "Misspoke? Say \"scratch that\" or \"..., actually ...\" to self-correct without editing by hand."
                    .to_string(),
        },
        AdoptionItem {
            key: "casing".to_string(),
            label: "Casing commands".to_string(),
            done: used_mode(entries, "casing"),
            suggestion: "Say \"snake case\" or \"camel case\" before a phrase for a properly-formatted identifier."
                .to_string(),
        },
        AdoptionItem {
            key: "modes".to_string(),
            label: "Per-app modes".to_string(),
            // Modes always has at least the 4 shipped defaults, so
            // "done" has to mean "changed from what shipped" (a new app
            // assigned, a new mode created, a model overridden) rather
            // than just non-empty.
            done: cfg.modes != crate::modes::seed_default_modes(),
            suggestion:
                "Set up per-app formatting in Modes so your terminal, Slack, and editor each get the right style."
                    .to_string(),
        },
    ];

    let done_count = items.iter().filter(|i| i.done).count();
    let score_percent = if items.is_empty() {
        0
    } else {
        (done_count * 100 / items.len()) as u32
    };

    AdoptionInfo {
        score_percent,
        items,
    }
}

// Off the main thread: sync commands run on it, and this one does I/O.
#[tauri::command(async)]
pub fn get_insights(app: tauri::AppHandle) -> InsightsPayload {
    let entries = crate::history::all_entries(&app);
    let cfg = crate::config::load(&app);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    compute(&entries, &cfg, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::AppRef;

    fn entry(
        text: &str,
        app_name: &str,
        mode: &str,
        days_ago: i64,
        duration_ms: Option<u64>,
    ) -> HistoryEntry {
        let now: u64 = 1_700_000_000_000; // fixed reference instant
        HistoryEntry {
            timestamp_ms: (now as i64 - days_ago * 86_400_000) as u64,
            text: text.to_string(),
            app_name: Some(app_name.to_string()),
            mode: Some(mode.to_string()),
            summary: None,
            duration_ms,
            features_used: Vec::new(),
            spoken_words: None,
            latency_ms: None,
            original_text: None,
        }
    }

    const NOW: u64 = 1_700_000_000_000;

    #[test]
    fn totals_words_and_dictations_across_entries() {
        let entries = vec![
            entry("one two three", "Terminal", "Plain", 0, None),
            entry("four five", "Terminal", "Plain", 1, None),
        ];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.total_dictations, 2);
        assert_eq!(payload.total_words, 5);
    }

    #[test]
    fn smart_formatting_counts_casing_snippet_boilerplate_and_feature_flags() {
        let mut with_backtrack = entry("hi", "Terminal", "Plain", 0, None);
        with_backtrack.features_used = vec!["backtrack".to_string()];
        let entries = vec![
            entry("a", "Terminal", "casing", 0, None),
            entry("b", "Terminal", "snippet", 0, None),
            entry("c", "Terminal", "boilerplate", 0, None),
            entry("d", "Terminal", "Plain", 0, None),
            with_backtrack,
        ];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.smart_formatting_count, 4);
    }

    #[test]
    fn average_wpm_ignores_entries_without_a_duration() {
        // 60 words in 30s = 120 wpm.
        let text = (0..60).map(|_| "word").collect::<Vec<_>>().join(" ");
        let entries = vec![
            entry(&text, "Terminal", "Plain", 0, Some(30_000)),
            entry("no duration here", "Terminal", "Plain", 0, None),
        ];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.avg_words_per_minute, Some(120.0));
        assert_eq!(payload.personal_best_wpm, Some(120.0));
    }

    #[test]
    fn no_durations_means_no_wpm_stat() {
        let entries = vec![entry("hello there", "Terminal", "Plain", 0, None)];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.avg_words_per_minute, None);
        assert_eq!(payload.personal_best_wpm, None);
    }

    #[test]
    fn app_usage_is_sorted_by_dictation_count_descending() {
        let entries = vec![
            entry("a", "Terminal", "Plain", 0, None),
            entry("b", "Slack", "Casual", 0, None),
            entry("c", "Terminal", "Plain", 0, None),
            entry("d", "Terminal", "Plain", 0, None),
        ];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.app_usage[0].app_name, "Terminal");
        assert_eq!(payload.app_usage[0].dictations, 3);
        assert!((payload.app_usage[0].percent - 75.0).abs() < 0.001);
        assert_eq!(payload.app_usage[1].app_name, "Slack");
    }

    #[test]
    fn missing_app_name_buckets_as_unknown() {
        let mut e = entry("a", "Terminal", "Plain", 0, None);
        e.app_name = None;
        let payload = compute(&[e], &AppConfig::default(), NOW);
        assert_eq!(payload.app_usage[0].app_name, "Unknown");
    }

    #[test]
    fn streak_counts_consecutive_days_ending_today() {
        let entries = vec![
            entry("a", "Terminal", "Plain", 0, None),
            entry("b", "Terminal", "Plain", 1, None),
            entry("c", "Terminal", "Plain", 2, None),
            // Gap at day 3, so this shouldn't extend the current streak.
            entry("d", "Terminal", "Plain", 5, None),
        ];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.streak.current_days, 3);
        assert_eq!(payload.streak.longest_days, 3);
    }

    #[test]
    fn streak_still_counts_if_nothing_logged_yet_today_but_yesterday_has_activity() {
        let entries = vec![entry("a", "Terminal", "Plain", 1, None)];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.streak.current_days, 1);
    }

    #[test]
    fn streak_is_zero_when_most_recent_activity_is_two_or_more_days_ago() {
        let entries = vec![entry("a", "Terminal", "Plain", 2, None)];
        let payload = compute(&entries, &AppConfig::default(), NOW);
        assert_eq!(payload.streak.current_days, 0);
        assert_eq!(payload.streak.longest_days, 1);
    }

    #[test]
    fn empty_history_has_zeroed_streak_and_no_adoption_crash() {
        let payload = compute(&[], &AppConfig::default(), NOW);
        assert_eq!(payload.streak.current_days, 0);
        assert_eq!(payload.streak.longest_days, 0);
        assert_eq!(payload.total_words, 0);
        assert_eq!(payload.adoption.score_percent, 0);
    }

    #[test]
    fn adoption_score_reflects_which_features_have_been_used() {
        let mut cfg = AppConfig::default();
        cfg.modes[0].apps.push(AppRef {
            bundle_id: "com.apple.Terminal".to_string(),
            app_name: "Terminal".to_string(),
        });
        let entries = vec![entry("a", "Terminal", "casing", 0, None)];
        let payload = compute(&entries, &cfg, NOW);

        let casing = payload
            .adoption
            .items
            .iter()
            .find(|i| i.key == "casing")
            .unwrap();
        assert!(casing.done);
        let modes = payload
            .adoption
            .items
            .iter()
            .find(|i| i.key == "modes")
            .unwrap();
        assert!(modes.done);
        let backtrack = payload
            .adoption
            .items
            .iter()
            .find(|i| i.key == "backtrack")
            .unwrap();
        assert!(!backtrack.done);
        // 2 of 6 default items done (vocabulary/snippets untouched, punctuation unused).
        assert_eq!(payload.adoption.score_percent, 33);
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(date_key(0), "1970-01-01");
    }

    #[test]
    fn latency_uses_recent_entries_that_recorded_it() {
        let mut entries: Vec<HistoryEntry> = (0..5)
            .map(|i| {
                let mut e = entry("hi", "Terminal", "Plain", i, None);
                e.latency_ms = Some([900, 300, 500, 400, 2000][i as usize]);
                e
            })
            .collect();
        entries.push(entry("old format", "Terminal", "Plain", 6, None));

        let stats = compute_latency(&entries).unwrap();
        assert_eq!(stats.samples, 5);
        assert_eq!(stats.median_ms, 500);
        assert_eq!(stats.p90_ms, 2000);
        assert_eq!(compute_latency(&entries[5..]), None);
    }
}
