//! Level unlocks and best scores, persisted as JSON in the XDG data dir.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::patterns::{BASE_BPM, BPM_MAX, BPM_MIN, BPM_STEP, Level, LevelKind, PATTERNS};

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
        p.unlocked.insert(Level::track(1, BASE_BPM).key());
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
        p.migrate();
        p
    }

    /// Earlier versions unlocked single-pattern levels (`pattern@bpm`); those are drills
    /// now, and an unlocked one opens its tier's track at that tempo.
    fn migrate(&mut self) {
        let old: Vec<Level> = self.unlocked.iter().filter_map(|k| Level::from_key(k)).filter(|l| !l.is_track()).collect();
        for l in old {
            self.unlocked.remove(&l.key());
            self.unlocked.insert(Level::track(l.tier(), l.bpm).key());
        }
        self.fresh.retain(|k| k.starts_with("tier"));
        self.unlocked.insert(Level::track(1, BASE_BPM).key());
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

    /// Tracks are unlocked explicitly; a drill is open wherever its tier's track is.
    pub fn is_unlocked(&self, level: &Level) -> bool {
        self.unlocked.contains(&Level::track(level.tier(), level.bpm).key())
    }

    pub fn best(&self, level: &Level) -> Option<f64> {
        self.best.get(&level.key()).copied()
    }

    pub fn passed(&self, level: &Level) -> bool {
        self.best(level).is_some_and(|b| b >= PASS_THRESHOLD)
    }

    /// Record a result. Returns the levels newly unlocked by it. Only tracks unlock:
    /// the next tier's track at the same tempo and the same track at +/-20 BPM.
    pub fn record(&mut self, level: &Level, combined: f64) -> Vec<Level> {
        let best = self.best.entry(level.key()).or_insert(combined);
        if combined > *best {
            *best = combined;
        }
        self.fresh.remove(&level.key());
        let mut newly = Vec::new();
        let LevelKind::Track(tier) = level.kind else { return newly };
        if combined >= PASS_THRESHOLD {
            let bpm = level.bpm;
            let mut targets = Vec::new();
            if tier < max_tier() {
                targets.push(Level::track(tier + 1, bpm));
            }
            if bpm + BPM_STEP <= BPM_MAX {
                targets.push(Level::track(tier, bpm + BPM_STEP));
            }
            if bpm >= BPM_MIN + BPM_STEP {
                targets.push(Level::track(tier, bpm - BPM_STEP));
            }
            for t in targets {
                if self.unlocked.insert(t.key()) {
                    self.fresh.insert(t.key());
                    newly.push(t);
                }
            }
        }
        newly
    }
}

pub fn max_tier() -> u32 {
    PATTERNS.iter().map(|p| p.tier).max().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlock_rules() {
        let mut p = Progress::fresh();
        assert!(p.is_unlocked(&Level::track(1, 80)));
        assert!(p.is_unlocked(&Level::drill(0, 80)) && p.is_unlocked(&Level::drill(1, 80)));
        assert!(!p.is_unlocked(&Level::track(2, 80)) && !p.is_unlocked(&Level::drill(3, 80)));
        // Drills keep their own best but unlock nothing.
        assert!(p.record(&Level::drill(0, 80), 95.0).is_empty());
        assert_eq!(p.best(&Level::drill(0, 80)), Some(95.0));
        assert!(p.record(&Level::track(1, 80), 50.0).is_empty());
        let newly = p.record(&Level::track(1, 80), 85.0);
        assert_eq!(newly, vec![Level::track(2, 80), Level::track(1, 100), Level::track(1, 60)]);
        assert!(p.is_unlocked(&Level::drill(3, 80)), "tier-2 drills open with the tier-2 track");
        assert!(!p.is_unlocked(&Level::track(2, 100)));
        assert_eq!(p.best(&Level::track(1, 80)), Some(85.0));
        let newly = p.record(&Level::track(1, 180), 90.0);
        assert_eq!(newly, vec![Level::track(2, 180), Level::track(1, 160)]);
    }

    #[test]
    fn migrates_old_pattern_levels() {
        let mut p = Progress::default();
        p.unlocked.insert("gallop@120".into());
        p.best.insert("gallop@120".into(), 77.0);
        p.migrate();
        assert!(p.is_unlocked(&Level::track(2, 120)));
        assert!(p.is_unlocked(&Level::track(1, 80)));
        assert_eq!(p.best(&Level::drill(3, 120)), Some(77.0));
    }
}
