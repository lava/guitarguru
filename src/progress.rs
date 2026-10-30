//! Level unlocks and best scores, persisted as JSON in the XDG data dir.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::patterns::{BASE_BPM, BPM_MAX, BPM_MIN, BPM_STEP, INGREDIENTS, LH, Level, LevelKind, PM, S16, first_ingredients};
#[cfg(test)]
use crate::patterns::SYNC;

/// Combined score (percent) needed to pass a level.
pub const PASS_THRESHOLD: f64 = 60.0;

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
    /// Legacy (tier model); migrated on load and then left empty.
    #[serde(default)]
    pub unlocked: BTreeSet<String>,
    /// Best combined score per level key (`mix:<set>@bpm` runs, `pattern@bpm` drills).
    #[serde(default)]
    pub best: BTreeMap<String, f64>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    /// Newly opened runs / ingredients not yet played (shown with a dot).
    #[serde(default)]
    pub fresh: BTreeSet<String>,
    /// How many ingredients (in `INGREDIENTS` order) are unlocked.
    #[serde(default)]
    pub ingredients: usize,
    /// Pattern ids of the last few runs per run key (newest last), so the next run of the
    /// same combination can pass over them.
    #[serde(default)]
    pub recent_patterns: BTreeMap<String, Vec<Vec<String>>>,
    /// Song suggestions already shown (`Song::label`).
    #[serde(default)]
    pub songs_shown: BTreeSet<String>,
    /// Layout version of this file (see `migrate`); 0 for files from before versioning.
    #[serde(default)]
    pub version: u32,
}

/// Current `Progress::version`. 1: Syncopation became the first ingredient.
pub const PROGRESS_VERSION: u32 = 1;

/// Runs per combination whose patterns the next run avoids.
pub const RECENT_RUNS: usize = 3;

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
    /// Mean pattern difficulty over the run's bars (see `generator::difficulty`); None in
    /// entries from before generated runs.
    #[serde(default)]
    pub difficulty: Option<f64>,
}

impl Progress {
    pub fn history_for(&self, key: &str) -> Vec<&HistoryEntry> {
        self.history.iter().filter(|h| h.key == key).collect()
    }
    pub fn best_entry(&self, key: &str) -> Option<&HistoryEntry> {
        self.history_for(key).into_iter().max_by(|a, b| a.combined.total_cmp(&b.combined))
    }
    /// Pattern ids played in the last `RECENT_RUNS` runs of this key.
    pub fn recent_patterns(&self, key: &str) -> std::collections::HashSet<String> {
        self.recent_patterns.get(key).into_iter().flatten().flatten().cloned().collect()
    }
    /// Remember the patterns of a finished run (keeps the last `RECENT_RUNS` per key).
    pub fn remember_patterns(&mut self, key: &str, ids: Vec<String>) {
        let runs = self.recent_patterns.entry(key.to_string()).or_default();
        runs.push(ids);
        let extra = runs.len().saturating_sub(RECENT_RUNS);
        runs.drain(..extra);
    }
}

/// Relative difference in mean pattern difficulty from which the results screen notes that
/// a run is not directly comparable with the best one.
pub const DIFFICULTY_NOTE: f64 = 0.12;

/// A note for the results screen when this run's patterns (mean difficulty `now`) were
/// notably harder or easier than those of the previous best `best` of the same level.
pub fn difficulty_note(now: f64, best: Option<&HistoryEntry>) -> Option<String> {
    let then = best?.difficulty?;
    if then <= 0.0 || (now / then - 1.0).abs() < DIFFICULTY_NOTE {
        return None;
    }
    let word = if now > then { "harder" } else { "easier" };
    Some(format!("{word} patterns than your best run (difficulty {now:.1} vs {then:.1})"))
}

/// What opened up after a result.
#[derive(Clone, Debug, PartialEq)]
pub enum Unlock {
    Ingredient(usize),
    Tempo(Level),
}

impl Unlock {
    pub fn label(&self) -> String {
        match self {
            Unlock::Ingredient(i) => format!("+ {}", INGREDIENTS[*i].name),
            Unlock::Tempo(l) => format!("{} {}", l.name(), l.bpm),
        }
    }
}

impl Progress {
    pub fn fresh() -> Self {
        Self { version: PROGRESS_VERSION, ..Self::default() }
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

    /// Earlier versions had tier levels (`tierN@bpm`: 1 basic, 2 + palm mutes, 3 + 16ths,
    /// 4 + note changes, cumulative). Their bests become run bests for that ingredient set,
    /// and the highest tier unlocked anywhere unlocks the matching ingredients. Drill bests
    /// (`pattern@bpm`) stay as they are; keys of removed patterns are dropped.
    fn migrate(&mut self) {
        let tier_mask = |t: u32| match t {
            1 => Some(0),
            2 => Some(PM),
            3 => Some(PM | S16),
            4 => Some(PM | S16 | LH),
            _ => None,
        };
        let parse_tier = |k: &str| -> Option<(u32, u32)> {
            let (a, b) = k.split_once('@')?;
            Some((a.strip_prefix("tier")?.parse().ok()?, b.parse().ok()?))
        };
        let mut max_tier = 0;
        for k in std::mem::take(&mut self.unlocked) {
            if let Some((t, _)) = parse_tier(&k) {
                max_tier = max_tier.max(t);
            }
        }
        let old: Vec<(String, f64)> = self.best.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (k, v) in old {
            if let Some((t, bpm)) = parse_tier(&k) {
                self.best.remove(&k);
                max_tier = max_tier.max(t);
                if let Some(m) = tier_mask(t) {
                    let e = self.best.entry(Level::run(m, bpm).key()).or_insert(v);
                    *e = e.max(v);
                }
            } else if Level::from_key(&k).is_none() {
                self.best.remove(&k);
            }
        }
        for h in &mut self.history {
            if let Some((t, bpm)) = parse_tier(&h.key)
                && let Some(m) = tier_mask(t)
            {
                h.key = Level::run(m, bpm).key();
            }
        }
        // Counted in the order before syncopation (palm mutes first).
        self.ingredients = self.ingredients.max((max_tier as usize).saturating_sub(1));
        // Version 1 put syncopation first in the unlock order: whoever had palm mutes keeps
        // everything they had and gets syncopation too.
        if self.version < 1 && self.ingredients > 0 {
            self.ingredients += 1;
            self.fresh.insert("ingredient:sync".into());
        }
        self.version = PROGRESS_VERSION;
        self.ingredients = self.ingredients.min(INGREDIENTS.len());
        self.fresh.retain(|k| Level::from_key(k).is_some() || k.starts_with("ingredient:"));
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

    /// Mask of the unlocked ingredients.
    pub fn unlocked_mask(&self) -> u8 {
        first_ingredients(self.ingredients)
    }

    pub fn ingredient_unlocked(&self, i: usize) -> bool {
        i < self.ingredients
    }

    /// Passed runs, as (bpm, ingredient mask).
    fn passed_runs(&self) -> impl Iterator<Item = (u32, u8)> + '_ {
        self.best.iter().filter(|(_, v)| **v >= PASS_THRESHOLD).filter_map(|(k, _)| {
            let l = Level::from_key(k)?;
            match l.kind {
                LevelKind::Run(m) => Some((l.bpm, m)),
                LevelKind::Drill(_) => None,
            }
        })
    }

    /// (b, S) is open if every ingredient in S is unlocked and either b is the base tempo or
    /// a passed run at b +/- 20 had a superset of S. Drills are open where the run with
    /// their ingredients is.
    pub fn is_unlocked(&self, level: &Level) -> bool {
        let mask = level.mask();
        if mask & !self.unlocked_mask() != 0 {
            return false;
        }
        if level.bpm == BASE_BPM {
            return true;
        }
        self.passed_runs().any(|(b, m)| (b + BPM_STEP == level.bpm || level.bpm + BPM_STEP == b) && m & mask == mask)
    }

    pub fn best(&self, level: &Level) -> Option<f64> {
        self.best.get(&level.key()).copied()
    }

    pub fn passed(&self, level: &Level) -> bool {
        self.best(level).is_some_and(|b| b >= PASS_THRESHOLD)
    }

    /// Record a result. Returns what it opened. Only runs unlock: tempos +/-20 for the run's
    /// ingredient set (and its subsets), and the next ingredient when the run had all
    /// unlocked ingredients active at >= 80 BPM.
    pub fn record(&mut self, level: &Level, combined: f64) -> Vec<Unlock> {
        let before: Vec<Level> = self.neighbour_runs(level).into_iter().filter(|l| self.is_unlocked(l)).collect();
        let best = self.best.entry(level.key()).or_insert(combined);
        if combined > *best {
            *best = combined;
        }
        self.fresh.remove(&level.key());
        let mut newly = Vec::new();
        let LevelKind::Run(mask) = level.kind else { return newly };
        if combined < PASS_THRESHOLD {
            return newly;
        }
        if mask == self.unlocked_mask() && level.bpm >= BASE_BPM && self.ingredients < INGREDIENTS.len() {
            self.ingredients += 1;
            self.fresh.insert(format!("ingredient:{}", INGREDIENTS[self.ingredients - 1].code));
            newly.push(Unlock::Ingredient(self.ingredients - 1));
        }
        for l in self.neighbour_runs(level) {
            if !before.contains(&l) && self.is_unlocked(&l) {
                self.fresh.insert(l.key());
                newly.push(Unlock::Tempo(l));
            }
        }
        newly
    }

    /// The same ingredient set at +/-20 BPM.
    fn neighbour_runs(&self, level: &Level) -> Vec<Level> {
        let m = level.mask();
        let mut v = Vec::new();
        if level.bpm + BPM_STEP <= BPM_MAX {
            v.push(Level::run(m, level.bpm + BPM_STEP));
        }
        if level.bpm >= BPM_MIN + BPM_STEP {
            v.push(Level::run(m, level.bpm - BPM_STEP));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::PATTERNS;

    #[test]
    fn unlock_rules() {
        let mut p = Progress::fresh();
        assert!(p.is_unlocked(&Level::run(0, 80)));
        assert!(!p.is_unlocked(&Level::run(0, 100)));
        assert!(!p.is_unlocked(&Level::run(SYNC, 80)), "syncopation still locked");
        let quarters = PATTERNS.iter().position(|q| q.id == "quarters").unwrap();
        let offbeats = PATTERNS.iter().position(|q| q.id == "offbeats").unwrap();
        assert!(p.is_unlocked(&Level::drill(quarters, 80)) && !p.is_unlocked(&Level::drill(offbeats, 80)));
        // Drills keep their own best but unlock nothing.
        assert!(p.record(&Level::drill(quarters, 80), 95.0).is_empty());
        assert!(p.record(&Level::run(0, 80), 50.0).is_empty());
        // Passing basic at 80 (all unlocked ingredients = none) unlocks syncopation and 60/100.
        let newly = p.record(&Level::run(0, 80), 85.0);
        assert_eq!(newly, vec![Unlock::Ingredient(0), Unlock::Tempo(Level::run(0, 100)), Unlock::Tempo(Level::run(0, 60))]);
        assert_eq!(INGREDIENTS[0].bit, SYNC, "syncopation comes first");
        assert!(p.is_unlocked(&Level::run(SYNC, 80)) && p.is_unlocked(&Level::drill(offbeats, 80)));
        assert!(!p.is_unlocked(&Level::run(PM, 80)), "palm mutes come next");
        assert!(!p.is_unlocked(&Level::run(SYNC, 100)), "tempos open per ingredient set");
        // A run without all unlocked ingredients does not unlock the next one.
        let newly = p.record(&Level::run(0, 100), 90.0);
        assert!(!newly.iter().any(|u| matches!(u, Unlock::Ingredient(_))), "{newly:?}");
        assert_eq!(p.ingredients, 1);
        // Below 80 BPM never unlocks ingredients.
        p.record(&Level::run(0, 60), 90.0);
        assert!(p.record(&Level::run(SYNC, 60), 0.0).is_empty());
        assert!(!p.is_unlocked(&Level::run(SYNC, 60)), "60 not open for syncopation yet");
    }

    #[test]
    fn superset_runs_open_subset_tempos() {
        let mut p = Progress { ingredients: 4, ..Progress::fresh() };
        p.record(&Level::run(PM | S16, 120), 85.0);
        for m in [0, PM, S16, PM | S16] {
            assert!(p.is_unlocked(&Level::run(m, 100)) && p.is_unlocked(&Level::run(m, 140)), "mask {m}");
        }
        assert!(!p.is_unlocked(&Level::run(LH, 140)), "LH was not part of the passed run");
        assert!(!p.is_unlocked(&Level::run(PM | S16 | LH, 100)));
        assert!(!p.is_unlocked(&Level::run(0, 160)), "only neighbours");
        // An ingredient that is still locked keeps everything with it closed.
        let p2 = Progress { ingredients: 2, best: p.best.clone(), ..Progress::fresh() };
        assert!(!p2.is_unlocked(&Level::run(S16, 100)));
        assert!(p2.is_unlocked(&Level::run(PM, 100)));
    }

    #[test]
    fn remembers_recent_patterns() {
        let mut p = Progress::fresh();
        for i in 0..5 {
            p.remember_patterns("mix:pm@100", vec![format!("p{i}"), "quarters".into()]);
        }
        let r = p.recent_patterns("mix:pm@100");
        assert_eq!(p.recent_patterns["mix:pm@100"].len(), RECENT_RUNS);
        assert!(r.contains("p4") && r.contains("p2") && !r.contains("p1"));
        assert!(p.recent_patterns("mix:pm@120").is_empty());
        let back: Progress = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.recent_patterns("mix:pm@100"), r);
        let old: Progress = serde_json::from_str(r#"{"best":{},"history":[{"key":"mix:basic@80","date":"","combined":1.0,"timing":0.0,"tone":0.0,"spread_ms":0.0,"level_std_db":0.0,"mean_offset_ms":null,"misses":0,"seconds":1.0}]}"#).unwrap();
        assert_eq!(old.history[0].difficulty, None);
    }

    #[test]
    fn notes_notable_difficulty_differences() {
        let best = |d| HistoryEntry { difficulty: d, ..Default::default() };
        assert_eq!(difficulty_note(3.0, None), None);
        assert_eq!(difficulty_note(3.0, Some(&best(None))), None, "old entries have no difficulty");
        assert_eq!(difficulty_note(3.3, Some(&best(Some(3.0)))), None, "within 12 %");
        assert!(difficulty_note(3.5, Some(&best(Some(3.0)))).unwrap().starts_with("harder"));
        assert!(difficulty_note(2.5, Some(&best(Some(3.0)))).unwrap().starts_with("easier"));
    }

    #[test]
    fn migrates_tier_progress() {
        let mut p = Progress::default();
        p.unlocked.insert("tier3@100".into());
        p.unlocked.insert("tier6@80".into());
        p.best.insert("tier1@80".into(), 88.0);
        p.best.insert("tier2@80".into(), 70.0);
        p.best.insert("gallop@120".into(), 77.0);
        p.best.insert("triplet_gallop@80".into(), 60.0);
        p.migrate();
        assert_eq!(p.best(&Level::run(0, 80)), Some(88.0));
        assert_eq!(p.best(&Level::run(PM, 80)), Some(70.0));
        assert_eq!(p.best.get("gallop@120"), Some(&77.0));
        assert!(!p.best.contains_key("triplet_gallop@80"));
        assert!(p.unlocked.is_empty());
        assert_eq!(p.ingredients, INGREDIENTS.len());
        assert!(p.is_unlocked(&Level::run(0, 100)), "derived from the passed basic run at 80");
    }

    #[test]
    fn syncopation_joins_existing_unlocks() {
        let load = |json: &str| {
            let mut p: Progress = serde_json::from_str(json).unwrap();
            p.migrate();
            p
        };
        // Palm mutes and 16ths unlocked before syncopation existed.
        let p = load(r#"{"best":{"mix:pm+16@100":70.0},"ingredients":2}"#);
        assert_eq!(p.unlocked_mask(), SYNC | PM | S16);
        assert!(p.fresh.contains("ingredient:sync"));
        assert_eq!(p.best(&Level::run(PM | S16, 100)), Some(70.0));
        assert_eq!(load(&serde_json::to_string(&p).unwrap()).ingredients, 3, "migrates once");
        // Nothing unlocked yet: syncopation is the next one.
        let p = load(r#"{"best":{"mix:basic@80":40.0},"ingredients":0}"#);
        assert_eq!(p.ingredients, 0);
        assert_eq!(p.version, PROGRESS_VERSION);
        // Everything unlocked stays everything.
        assert_eq!(load(r#"{"ingredients":4}"#).ingredients, INGREDIENTS.len());
        assert!(!Progress::fresh().fresh.contains("ingredient:sync"));
    }
}
