//! Level unlocks and best scores, persisted as JSON in the XDG data dir.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::patterns::{BASE_BPM, BPM_MAX, BPM_MIN, BPM_STEP, INGREDIENTS, LH, Level, LevelKind, PM, S16, first_ingredients};
#[cfg(test)]
use crate::patterns::SYNC;

/// Combined score (percent) needed to pass a level.
pub const PASS_THRESHOLD: f64 = 60.0;

/// Letter grade of a combined score: C below the pass mark, then B, A and S.
pub fn grade(score: f64) -> char {
    if score >= 90.0 {
        'S'
    } else if score >= 75.0 {
        'A'
    } else if score >= PASS_THRESHOLD {
        'B'
    } else {
        'C'
    }
}

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
    /// Legacy (before version 2): how many ingredients, in `INGREDIENTS` order, were
    /// unlocked. Migrated into `ingredient_mask`.
    #[serde(default, skip_serializing)]
    pub ingredients: usize,
    /// Unlocked ingredients (bit mask).
    #[serde(default)]
    pub ingredient_mask: u8,
    /// Experience from all finished runs (see `xp`).
    #[serde(default)]
    pub xp: u32,
    /// XP the main menu's bar last showed; it fills from here to `xp` on the next visit.
    #[serde(default)]
    pub xp_seen: u32,
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

/// Current `Progress::version`. 1: Syncopation became the first ingredient. 2: ingredients
/// are a free choice (mask + unlock tokens); XP.
pub const PROGRESS_VERSION: u32 = 2;

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
    /// An ingredient of the player's choice, picked on the main menu.
    Token,
    /// A newly reached tempo (for every unlocked ingredient set).
    Tempo(u32),
}

impl Unlock {
    pub fn label(&self) -> String {
        match self {
            Unlock::Token => "+ an ingredient".into(),
            Unlock::Tempo(b) => format!("{b} BPM"),
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
        // Version 2: the unlocked count becomes a mask, XP is seeded from the history (or
        // from the passed bests when there is none) and shows as already seen.
        if self.version < 2 {
            self.ingredient_mask |= first_ingredients(self.ingredients.min(INGREDIENTS.len()));
            self.xp = if self.history.is_empty() {
                self.best
                    .iter()
                    .filter_map(|(k, v)| Some(crate::xp::run_xp(&Level::from_key(k)?, *v, crate::game::SCORED_SECONDS)))
                    .sum()
            } else {
                self.history.iter().map(crate::xp::entry_xp).sum()
            };
            self.xp_seen = self.xp;
        }
        self.ingredients = 0;
        self.ingredient_mask &= INGREDIENTS.iter().fold(0, |m, i| m | i.bit);
        self.xp_seen = self.xp_seen.min(self.xp);
        self.version = PROGRESS_VERSION;
        // Fresh tempos were per run (`mix:<set>@bpm`); they are per tempo now.
        let old_fresh = std::mem::take(&mut self.fresh);
        for k in old_fresh {
            match Level::from_key(&k) {
                Some(l) if l.is_run() => {
                    self.fresh.insert(format!("tempo:{}", l.bpm));
                }
                Some(_) => {}
                None if k.starts_with("ingredient:") || k.starts_with("tempo:") => {
                    self.fresh.insert(k);
                }
                None => {}
            }
        }
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
        self.ingredient_mask
    }

    pub fn ingredient_unlocked(&self, i: usize) -> bool {
        self.ingredient_mask & INGREDIENTS[i].bit != 0
    }

    /// Are all ingredients unlocked?
    pub fn all_ingredients(&self) -> bool {
        INGREDIENTS.iter().all(|i| self.ingredient_mask & i.bit != 0)
    }

    /// Ingredients that have been part of a passed run.
    pub fn passed_mask(&self) -> u8 {
        self.passed_runs().fold(0, |m, (_, mask)| m | mask)
    }

    /// Unlocks earned and not yet spent (the player picks which ingredient). The first
    /// passed run earns one, and so does each ingredient's first passed run, whatever else
    /// was on and at any tempo; every unlocked ingredient has spent one.
    pub fn unlock_tokens(&self) -> u32 {
        if self.all_ingredients() {
            return 0;
        }
        let earned = u32::from(self.passed_runs().next().is_some()) + self.passed_mask().count_ones();
        earned.saturating_sub(self.ingredient_mask.count_ones())
    }

    /// Spend an unlock on ingredient `i`. False if there is none or it is open.
    pub fn unlock_ingredient(&mut self, i: usize) -> bool {
        if self.unlock_tokens() == 0 || self.ingredient_unlocked(i) {
            return false;
        }
        self.ingredient_mask |= INGREDIENTS[i].bit;
        self.fresh.insert(format!("ingredient:{}", INGREDIENTS[i].code));
        true
    }

    /// Passed runs, as (bpm, ingredient mask).
    fn passed_runs(&self) -> impl Iterator<Item = (u32, u8)> + '_ {
        self.best.iter().filter(|(_, v)| **v >= PASS_THRESHOLD).filter_map(|(k, _)| {
            let l = Level::from_key(k)?;
            match l.kind {
                LevelKind::Run(m) => Some((l.bpm, m)),
                LevelKind::Drill(_) | LevelKind::Riff(_) => None,
            }
        })
    }

    /// Tempo and ingredients are independent: a tempo is reached when it is the base tempo,
    /// was passed, or lies 20 BPM from a passed run with any ingredients.
    pub fn tempo_reached(&self, bpm: u32) -> bool {
        bpm == BASE_BPM || self.passed_runs().any(|(b, _)| b == bpm || b + BPM_STEP == bpm || bpm + BPM_STEP == b)
    }

    /// (b, S) is open if every ingredient in S is unlocked and tempo b is reached. Drills are
    /// open where the run with their ingredients is.
    pub fn is_unlocked(&self, level: &Level) -> bool {
        level.mask() & !self.unlocked_mask() == 0 && self.tempo_reached(level.bpm)
    }

    pub fn best(&self, level: &Level) -> Option<f64> {
        self.best.get(&level.key()).copied()
    }

    pub fn passed(&self, level: &Level) -> bool {
        self.best(level).is_some_and(|b| b >= PASS_THRESHOLD)
    }

    /// Record a result. Returns what it opened. Only runs unlock: the tempos +/-20 (for every
    /// unlocked ingredient set), and an ingredient of the player's choice (see
    /// `unlock_tokens`).
    pub fn record(&mut self, level: &Level, combined: f64) -> Vec<Unlock> {
        let before: Vec<u32> = neighbour_tempos(level.bpm).filter(|&b| self.tempo_reached(b)).collect();
        let tokens = self.unlock_tokens();
        let best = self.best.entry(level.key()).or_insert(combined);
        if combined > *best {
            *best = combined;
        }
        self.fresh.remove(&format!("tempo:{}", level.bpm));
        let mut newly = Vec::new();
        if !level.is_run() || combined < PASS_THRESHOLD {
            return newly;
        }
        if self.unlock_tokens() > tokens {
            newly.push(Unlock::Token);
        }
        for b in neighbour_tempos(level.bpm) {
            if !before.contains(&b) && self.tempo_reached(b) {
                self.fresh.insert(format!("tempo:{b}"));
                newly.push(Unlock::Tempo(b));
            }
        }
        newly
    }
}

/// The tempos 20 BPM faster and slower, within range.
fn neighbour_tempos(bpm: u32) -> impl Iterator<Item = u32> {
    [bpm + BPM_STEP, bpm.saturating_sub(BPM_STEP)].into_iter().filter(move |&b| b != bpm && (BPM_MIN..=BPM_MAX).contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::PATTERNS;

    #[test]
    fn grades_start_at_the_pass_mark() {
        assert_eq!(grade(40.0), 'C');
        assert_eq!(grade(PASS_THRESHOLD - 0.1), 'C');
        assert_eq!(grade(PASS_THRESHOLD), 'B');
        assert_eq!(grade(75.0), 'A');
        assert_eq!(grade(90.0), 'S');
    }

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
        // The first pass earns a token and opens 60/100.
        let newly = p.record(&Level::run(0, 80), 85.0);
        assert_eq!(newly, vec![Unlock::Token, Unlock::Tempo(100), Unlock::Tempo(60)]);
        assert!(p.fresh.contains("tempo:100") && p.fresh.contains("tempo:60"));
        assert_eq!(p.unlock_tokens(), 1);
        // Passing again earns nothing more.
        assert!(!p.record(&Level::run(0, 80), 90.0).contains(&Unlock::Token));
        assert_eq!(p.unlock_tokens(), 1);
        // The player picks palm mutes first.
        let pm = INGREDIENTS.iter().position(|i| i.bit == PM).unwrap();
        assert!(p.unlock_ingredient(pm));
        assert!(!p.unlock_ingredient(0), "no token left");
        assert_eq!((p.unlock_tokens(), p.unlocked_mask()), (0, PM));
        assert!(p.fresh.contains("ingredient:pm"));
        assert!(p.is_unlocked(&Level::run(PM, 80)) && !p.is_unlocked(&Level::run(SYNC, 80)));
        assert!(!p.is_unlocked(&Level::drill(offbeats, 80)), "offbeats need syncopation");
        assert!(p.is_unlocked(&Level::run(PM, 100)) && p.is_unlocked(&Level::run(PM, 60)), "reached tempos open for any unlocked set");
        assert!(!p.is_unlocked(&Level::run(PM, 120)), "120 not reached yet");
        // A run without a new ingredient earns no token.
        let newly = p.record(&Level::run(0, 100), 90.0);
        assert!(!newly.contains(&Unlock::Token), "{newly:?}");
        assert_eq!(p.unlock_tokens(), 0);
        // P.M.'s first pass does, below a pass it doesn't.
        assert!(p.record(&Level::run(PM, 80), 50.0).is_empty());
        assert!(p.record(&Level::run(PM, 80), 70.0).contains(&Unlock::Token));
        assert!(!p.record(&Level::run(PM, 100), 70.0).contains(&Unlock::Token), "P.M. has earned its token");
        assert!(p.unlock_ingredient(0));
        assert_eq!(p.unlocked_mask(), PM | SYNC);
        // Syncopation's first pass earns one at any tempo, with other ingredients on.
        assert_eq!(p.record(&Level::run(PM | SYNC, 60), 90.0), vec![Unlock::Token, Unlock::Tempo(40)]);
        assert_eq!(p.unlock_tokens(), 1);
        assert!(!p.record(&Level::run(PM | SYNC, 60), 95.0).contains(&Unlock::Token));
        assert_eq!(p.unlock_tokens(), 1);
        // Syncopation at 120 opens 140 for every set.
        p.record(&Level::run(0, 100), 90.0);
        assert_eq!(p.record(&Level::run(SYNC, 120), 70.0), vec![Unlock::Tempo(140)]);
        assert!(p.is_unlocked(&Level::run(PM, 140)) && p.is_unlocked(&Level::run(0, 140)));
    }

    #[test]
    fn tempos_are_independent_of_the_ingredient_set() {
        let mut p = Progress { ingredient_mask: SYNC | PM | S16 | LH, ..Progress::fresh() };
        p.record(&Level::run(PM | S16, 120), 85.0);
        for m in [0, PM, S16, LH, SYNC | LH, PM | S16 | LH] {
            for b in [80, 100, 120, 140] {
                assert!(p.is_unlocked(&Level::run(m, b)), "mask {m} at {b}");
            }
            assert!(!p.is_unlocked(&Level::run(m, 160)) && !p.is_unlocked(&Level::run(m, 60)), "only neighbours");
        }
        // A locked ingredient keeps everything with it closed.
        let p2 = Progress { ingredient_mask: SYNC | PM, best: p.best.clone(), ..Progress::fresh() };
        assert!(!p2.is_unlocked(&Level::run(S16, 100)));
        assert!(p2.is_unlocked(&Level::run(SYNC | PM, 140)));
        // Drills follow: the gallop (16ths + P.M.) is open wherever those are.
        let gallop = crate::patterns::PATTERNS.iter().position(|q| q.id == "gallop").unwrap();
        assert!(p.is_unlocked(&Level::drill(gallop, 140)) && !p2.is_unlocked(&Level::drill(gallop, 140)));
    }

    #[test]
    fn fresh_run_keys_become_tempos() {
        let mut p: Progress = serde_json::from_str(r#"{"version":2,"fresh":["mix:sync@100","ingredient:pm","gallop@120","junk"]}"#).unwrap();
        p.migrate();
        let f: Vec<&str> = p.fresh.iter().map(String::as_str).collect();
        assert_eq!(f, vec!["ingredient:pm", "tempo:100"]);
    }

    #[test]
    fn version_1_files_keep_ingredients_and_seed_xp() {
        let json = r#"{"version":1,"ingredients":2,"best":{"mix:sync@80":70.0},"history":[
            {"key":"mix:basic@80","date":"","combined":70.0,"timing":0.0,"tone":0.0,"spread_ms":0.0,"level_std_db":0.0,"mean_offset_ms":null,"misses":0,"seconds":125.0},
            {"key":"mix:sync@80","date":"","combined":40.0,"timing":0.0,"tone":0.0,"spread_ms":0.0,"level_std_db":0.0,"mean_offset_ms":null,"misses":0,"seconds":125.0}]}"#;
        let mut p: Progress = serde_json::from_str(json).unwrap();
        p.migrate();
        assert_eq!(p.unlocked_mask(), SYNC | PM);
        assert_eq!(p.unlock_tokens(), 0);
        assert_eq!(p.xp, 24 + 10);
        assert_eq!(p.xp_seen, p.xp, "no fill animation for old XP");
        let saved = serde_json::to_string(&p).unwrap();
        assert!(!saved.contains("\"ingredients\""));
        let mut again: Progress = serde_json::from_str(&saved).unwrap();
        again.xp += 5;
        again.migrate();
        assert_eq!((again.unlocked_mask(), again.xp, again.xp_seen), (SYNC | PM, 39, 34), "migrates once");
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
        assert!(p.all_ingredients());
        assert!(p.is_unlocked(&Level::run(0, 100)), "derived from the passed basic run at 80");
        // No history: XP from the bests (basic 80 at 88 %, P.M. 80 at 70 %, gallop 120 at 77 %).
        assert_eq!(p.xp, 28 + 30 + 57);
        assert_eq!(p.xp_seen, p.xp);
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
        assert_eq!(load(&serde_json::to_string(&p).unwrap()).unlocked_mask(), SYNC | PM | S16, "migrates once");
        // Nothing unlocked yet: syncopation is the next one.
        let p = load(r#"{"best":{"mix:basic@80":40.0},"ingredients":0}"#);
        assert_eq!(p.unlocked_mask(), 0);
        assert_eq!(p.version, PROGRESS_VERSION);
        // Everything unlocked stays everything.
        assert!(load(r#"{"ingredients":4}"#).all_ingredients());
        assert!(!Progress::fresh().fresh.contains("ingredient:sync"));
    }
}
