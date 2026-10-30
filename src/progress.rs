//! Level unlocks and best scores, persisted as JSON in the XDG data dir.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::patterns::{BASE_BPM, BPM_MAX, BPM_MIN, BPM_STEP, PATTERNS, Pattern};

/// Combined score (percent) needed to pass a level.
pub const PASS_THRESHOLD: f64 = 80.0;

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("GG_DATA_DIR") {
        return PathBuf::from(d);
    }
    directories::ProjectDirs::from("", "", "guitarguru")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".guitarguru"))
}

pub fn level_key(pattern_id: &str, bpm: u32) -> String {
    format!("{pattern_id}@{bpm}")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    pub unlocked: BTreeSet<String>,
    /// Best combined score per level.
    pub best: BTreeMap<String, f64>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// Levels unlocked but not yet played (shown with a dot).
    #[serde(default)]
    pub fresh: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub key: String,
    pub date: String,
    pub combined: f64,
    pub timing: f64,
    pub tone: f64,
    pub spread_ms: f64,
    pub level_std_db: f64,
    pub mean_offset_ms: Option<f64>,
    pub misses: usize,
    pub seconds: f64,
}

impl Progress {
    pub fn history_for(&self, key: &str) -> Vec<&HistoryEntry> {
        self.history.iter().filter(|h| h.key == key).collect()
    }
    pub fn best_entry(&self, key: &str) -> Option<&HistoryEntry> {
        self.history_for(key).into_iter().max_by(|a, b| a.combined.total_cmp(&b.combined))
    }
}

impl Progress {
    pub fn fresh() -> Self {
        let mut p = Self::default();
        for pat in PATTERNS.iter().filter(|p| p.tier == 1) {
            p.unlocked.insert(level_key(pat.id, BASE_BPM));
        }
        p
    }

    pub fn path() -> PathBuf {
        data_dir().join("progress.json")
    }

    pub fn load() -> Self {
        let mut p = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str::<Progress>(&s).ok())
            .unwrap_or_else(Self::fresh);
        // New tier-1 patterns added later are always available.
        for pat in PATTERNS.iter().filter(|p| p.tier == 1) {
            p.unlocked.insert(level_key(pat.id, BASE_BPM));
        }
        p
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self)
            && let Err(e) = std::fs::write(&path, s)
        {
            eprintln!("cannot save progress to {}: {e}", path.display());
        }
    }

    pub fn is_unlocked(&self, pattern_id: &str, bpm: u32) -> bool {
        self.unlocked.contains(&level_key(pattern_id, bpm))
    }

    pub fn best(&self, pattern_id: &str, bpm: u32) -> Option<f64> {
        self.best.get(&level_key(pattern_id, bpm)).copied()
    }

    /// Record a result. Returns the levels newly unlocked by it.
    pub fn record(&mut self, pattern: &Pattern, bpm: u32, combined: f64) -> Vec<String> {
        let key = level_key(pattern.id, bpm);
        let best = self.best.entry(key).or_insert(combined);
        if combined > *best {
            *best = combined;
        }
        let mut newly = Vec::new();
        if combined >= PASS_THRESHOLD {
            let mut targets: Vec<String> =
                PATTERNS.iter().filter(|p| p.tier == pattern.tier + 1).map(|p| level_key(p.id, bpm)).collect();
            if bpm + BPM_STEP <= BPM_MAX {
                targets.push(level_key(pattern.id, bpm + BPM_STEP));
            }
            if bpm >= BPM_MIN + BPM_STEP {
                targets.push(level_key(pattern.id, bpm - BPM_STEP));
            }
            for t in targets {
                if self.unlocked.insert(t.clone()) {
                    self.fresh.insert(t.clone());
                    newly.push(t);
                }
            }
        }
        newly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::pattern_by_id;

    #[test]
    fn unlock_rules() {
        let mut p = Progress::fresh();
        assert!(p.is_unlocked("quarters", 80));
        assert!(p.is_unlocked("eighths", 80));
        assert!(!p.is_unlocked("gallop", 80));
        let q = pattern_by_id("quarters").unwrap();
        assert!(p.record(q, 80, 50.0).is_empty());
        let newly = p.record(q, 80, 85.0);
        assert!(p.is_unlocked("gallop", 80) && p.is_unlocked("tremolo", 80));
        assert!(p.is_unlocked("quarters", 100) && p.is_unlocked("quarters", 60));
        assert!(!p.is_unlocked("gallop", 100));
        assert_eq!(newly.len(), 4);
        assert_eq!(p.best("quarters", 80), Some(85.0));
        let newly = p.record(q, 180, 90.0);
        assert_eq!(newly, vec!["tremolo@180", "gallop@180", "quarters@160"]);
    }
}
