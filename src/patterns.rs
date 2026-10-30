//! Rhythm patterns as data, and the note grid a play session is scored against.
//!
//! A pattern is a string of note tokens filling one bar of 4/4:
//! `W` whole (16 sixteenths), `H` half (8), `Q` quarter (4), `E` eighth (2), `S` sixteenth (1),
//! `T` sixteenth-triplet (2/3). A leading `-` makes the token a rest (`-Q`, `-E`, ...).

use serde::{Deserialize, Serialize};

pub const BPM_MIN: u32 = 40;
pub const BPM_MAX: u32 = 180;
pub const BPM_STEP: u32 = 20;
pub const BASE_BPM: u32 = 80;
pub const SIXTEENTHS_PER_BAR: f64 = 16.0;
pub const BEATS_PER_BAR: u32 = 4;

pub fn bpm_steps() -> impl Iterator<Item = u32> {
    (BPM_MIN..=BPM_MAX).step_by(BPM_STEP as usize)
}

#[derive(Clone, Debug)]
pub struct Pattern {
    pub id: &'static str,
    pub name: &'static str,
    pub tier: u32,
    pub notation: &'static str,
}

/// All patterns, ordered by tier. Add new ones here.
pub const PATTERNS: &[Pattern] = &[
    Pattern { id: "quarters", name: "Quarters", tier: 1, notation: "Q Q Q Q" },
    Pattern { id: "eighths", name: "Eighths", tier: 1, notation: "E E E E E E E E" },
    Pattern { id: "tremolo", name: "Tremolo 16ths", tier: 2, notation: "S S S S S S S S S S S S S S S S" },
    Pattern { id: "gallop", name: "Gallop", tier: 2, notation: "E S S E S S E S S E S S" },
    Pattern { id: "pursuit", name: "Pursuit of Vikings", tier: 3, notation: "E S S E E S S E S S E" },
    Pattern { id: "reverse_gallop", name: "Reverse gallop", tier: 4, notation: "S S E S S E S S E S S E" },
    Pattern { id: "gallop_rests", name: "Gallop + rests", tier: 4, notation: "E S S -Q E S S -Q" },
    Pattern { id: "mixed_gallop", name: "Gallop / reverse", tier: 5, notation: "E S S S S E E S S S S E" },
    Pattern { id: "tremolo_bursts", name: "Tremolo bursts", tier: 5, notation: "S S S S -Q S S S S -E E" },
    Pattern { id: "triplet_gallop", name: "Triplet gallop", tier: 6, notation: "T T T E T T T E T T T E T T T E" },
];

#[cfg(test)]
pub fn pattern_by_id(id: &str) -> Option<&'static Pattern> {
    PATTERNS.iter().find(|p| p.id == id)
}


/// One note of a parsed bar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarNote {
    /// Onset position in sixteenths from the start of the bar.
    pub pos: f64,
    /// Duration in sixteenths.
    pub len: f64,
    pub rest: bool,
}

pub fn parse(notation: &str) -> Result<Vec<BarNote>, String> {
    let mut pos = 0.0;
    let mut out = Vec::new();
    for tok in notation.split_whitespace() {
        let (rest, sym) = tok.strip_prefix('-').map_or((false, tok), |s| (true, s));
        let len = match sym {
            "W" => 16.0,
            "H" => 8.0,
            "Q" => 4.0,
            "E" => 2.0,
            "S" => 1.0,
            "T" => 2.0 / 3.0,
            _ => return Err(format!("unknown token `{tok}`")),
        };
        out.push(BarNote { pos, len, rest });
        pos += len;
    }
    if (pos - SIXTEENTHS_PER_BAR).abs() > 1e-6 {
        return Err(format!("bar has {pos} sixteenths, expected {SIXTEENTHS_PER_BAR}"));
    }
    Ok(out)
}

impl BarNote {
    /// Pick direction with strict alternate picking on the 16th grid (down on 1/+, up
    /// on e/a); 16th triplets alternate within each triplet group, starting down.
    pub fn downstroke(&self) -> bool {
        if (self.pos - self.pos.round()).abs() < 1e-6 && (self.len - self.len.round()).abs() < 1e-6 {
            (self.pos.round() as i64) % 2 == 0
        } else {
            ((self.pos * 1.5).round() as i64) % 2 == 0
        }
    }
}

impl Pattern {
    pub fn bar(&self) -> Vec<BarNote> {
        parse(self.notation).expect("built-in pattern must parse")
    }
    /// Played (non-rest) notes of one bar.
    pub fn hits(&self) -> Vec<BarNote> {
        self.bar().into_iter().filter(|n| !n.rest).collect()
    }
}

/// Bars per pattern segment in a main ("track") level.
pub const SEGMENT_BARS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LevelKind {
    /// Main level of a tier: alternates between the tier's patterns and earlier ones.
    Track(u32),
    /// One pattern only (index into `PATTERNS`).
    Drill(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Level {
    pub kind: LevelKind,
    pub bpm: u32,
}

impl Level {
    pub fn track(tier: u32, bpm: u32) -> Self {
        Self { kind: LevelKind::Track(tier), bpm }
    }
    pub fn drill(pattern: usize, bpm: u32) -> Self {
        Self { kind: LevelKind::Drill(pattern), bpm }
    }
    pub fn tier(&self) -> u32 {
        match self.kind {
            LevelKind::Track(t) => t,
            LevelKind::Drill(p) => PATTERNS[p].tier,
        }
    }
    /// Progress key. Drill keys keep the old `pattern@bpm` form, so earlier single-pattern
    /// results carry over as drill results.
    pub fn key(&self) -> String {
        match self.kind {
            LevelKind::Track(t) => format!("tier{t}@{}", self.bpm),
            LevelKind::Drill(p) => format!("{}@{}", PATTERNS[p].id, self.bpm),
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        let (a, b) = key.split_once('@')?;
        let bpm = b.parse().ok()?;
        if let Some(t) = a.strip_prefix("tier") {
            return Some(Self::track(t.parse().ok()?, bpm));
        }
        Some(Self::drill(PATTERNS.iter().position(|p| p.id == a)?, bpm))
    }
    pub fn name(&self) -> String {
        match self.kind {
            LevelKind::Track(t) => format!("Tier {t} track"),
            LevelKind::Drill(p) => format!("{} drill", PATTERNS[p].name),
        }
    }
    pub fn is_track(&self) -> bool {
        matches!(self.kind, LevelKind::Track(_))
    }
    /// Pattern (index into `PATTERNS`) for each scored bar.
    pub fn bar_patterns(&self, bars: u32) -> Vec<usize> {
        match self.kind {
            LevelKind::Drill(p) => vec![p; bars as usize],
            LevelKind::Track(t) => {
                let order = track_segments(t, bars.div_ceil(SEGMENT_BARS) as usize, &self.key());
                (0..bars).map(|b| order[(b / SEGMENT_BARS) as usize]).collect()
            }
        }
    }
}

/// Seeded segment order for a track level: about half the segments use the tier's own
/// patterns, the rest earlier tiers; never the same pattern twice in a row (if avoidable).
/// The seed comes from the level key, so every run of a level plays the same sequence.
pub fn track_segments(tier: u32, segments: usize, key: &str) -> Vec<usize> {
    let own: Vec<usize> = (0..PATTERNS.len()).filter(|&i| PATTERNS[i].tier == tier).collect();
    let earlier: Vec<usize> = (0..PATTERNS.len()).filter(|&i| PATTERNS[i].tier < tier).collect();
    let mut seed = key.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut out: Vec<usize> = Vec::with_capacity(segments);
    for k in 0..segments {
        // Start and end on the tier's own material; alternate in between, with some chance.
        let use_own = earlier.is_empty() || k == 0 || k + 1 == segments || rnd() % 2 == 0;
        let pool = if use_own && !own.is_empty() { &own } else { &earlier };
        let mut pick = pool[(rnd() % pool.len() as u64) as usize];
        if pool.len() > 1 && out.last() == Some(&pick) {
            pick = pool[(pool.iter().position(|&x| x == pick).unwrap_or(0) + 1) % pool.len()];
        }
        if out.last() == Some(&pick) && pool.len() == 1 {
            let other = if std::ptr::eq(pool, &own) { &earlier } else { &own };
            if !other.is_empty() {
                pick = other[(rnd() % other.len() as u64) as usize];
            }
        }
        out.push(pick);
    }
    out
}

/// A grid note to be played.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GridNote {
    /// Seconds from the session start (the first count-in click).
    pub time: f64,
    pub bar: u32,
    /// Pattern of this bar (index into `PATTERNS`).
    pub pattern: usize,
    /// Index of the note among the played notes of its pattern's bar (the "slot").
    pub slot: usize,
}

#[derive(Clone, Debug)]
pub struct SessionPlan {
    pub bpm: u32,
    pub count_in_bars: u32,
    pub scored_bars: u32,
    /// Pattern per scored bar.
    pub bar_patterns: Vec<usize>,
    pub notes: Vec<GridNote>,
}

impl SessionPlan {
    pub fn new(level: &Level, count_in_bars: u32, scored_seconds: f64) -> Self {
        let bpm = level.bpm;
        let bar_s = bar_seconds(bpm);
        let scored_bars = ((scored_seconds / bar_s).round() as u32).max(1);
        let bar_patterns = level.bar_patterns(scored_bars);
        let s16 = sixteenth_seconds(bpm);
        let mut notes = Vec::new();
        for b in 0..scored_bars {
            let pi = bar_patterns[b as usize];
            let bar_start = f64::from(count_in_bars + b) * bar_s;
            for (slot, n) in PATTERNS[pi].hits().iter().enumerate() {
                notes.push(GridNote { time: bar_start + n.pos * s16, bar: b, pattern: pi, slot });
            }
        }
        Self { bpm, count_in_bars, scored_bars, bar_patterns, notes }
    }
    /// Pattern of scored bar `b` (count-in bars show the first pattern).
    pub fn pattern_at(&self, bar: i64) -> usize {
        let i = bar.clamp(0, i64::from(self.scored_bars) - 1) as usize;
        self.bar_patterns[i]
    }
    /// Keep only the first `bars` scored bars.
    pub fn truncate(&mut self, bars: u32) {
        self.notes.retain(|n| n.bar < bars);
        self.bar_patterns.truncate(bars as usize);
        self.scored_bars = bars;
    }
    pub fn total_beats(&self) -> u32 {
        (self.count_in_bars + self.scored_bars) * BEATS_PER_BAR
    }
    pub fn total_seconds(&self) -> f64 {
        f64::from(self.total_beats()) * beat_seconds(self.bpm)
    }
    pub fn scoring_start(&self) -> f64 {
        f64::from(self.count_in_bars) * bar_seconds(self.bpm)
    }
}

pub fn beat_seconds(bpm: u32) -> f64 {
    60.0 / f64::from(bpm)
}
pub fn sixteenth_seconds(bpm: u32) -> f64 {
    beat_seconds(bpm) / 4.0
}
pub fn bar_seconds(bpm: u32) -> f64 {
    beat_seconds(bpm) * f64::from(BEATS_PER_BAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_patterns_fill_one_bar() {
        for p in PATTERNS {
            assert!(parse(p.notation).is_ok(), "{}", p.id);
        }
    }

    #[test]
    fn pursuit_positions() {
        let p = pattern_by_id("pursuit").unwrap();
        let pos: Vec<f64> = p.hits().iter().map(|n| n.pos).collect();
        assert_eq!(pos, vec![0.0, 2.0, 3.0, 4.0, 6.0, 8.0, 9.0, 10.0, 12.0, 13.0, 14.0]);
    }

    #[test]
    fn pick_directions() {
        let dirs = |id: &str| pattern_by_id(id).unwrap().hits().iter().map(|n| if n.downstroke() { 'D' } else { 'U' }).collect::<String>();
        assert_eq!(dirs("gallop"), "DDUDDUDDUDDU");
        assert_eq!(dirs("reverse_gallop"), "DUDDUDDUDDUD");
        assert_eq!(dirs("triplet_gallop"), "DUDDDUDDDUDDDUDD");
    }

    #[test]
    fn track_order_is_seeded_and_mixed() {
        let a = track_segments(3, 8, "tier3@120");
        assert_eq!(a, track_segments(3, 8, "tier3@120"), "same level, same order");
        assert_ne!(a, track_segments(3, 8, "tier3@140"));
        let pursuit = PATTERNS.iter().position(|p| p.id == "pursuit").unwrap();
        assert_eq!(a[0], pursuit);
        assert!(a.iter().any(|&p| PATTERNS[p].tier < 3), "{a:?}");
        assert!(a.windows(2).all(|w| w[0] != w[1]), "{a:?}");
        let l = Level::track(2, 100);
        let plan = SessionPlan::new(&l, 2, 120.0);
        assert_eq!(plan.bar_patterns.len(), 50);
        assert!(plan.bar_patterns.chunks(SEGMENT_BARS as usize).all(|c| c.iter().all(|&p| p == c[0])));
        assert_eq!(Level::from_key(&l.key()), Some(l));
        assert_eq!(Level::from_key("gallop@80"), Some(Level::drill(3, 80)));
    }

    #[test]
    fn plan_lengths() {
        let p = PATTERNS.iter().position(|p| p.id == "quarters").unwrap();
        let plan = SessionPlan::new(&Level::drill(p, 120), 2, 120.0);
        assert_eq!(plan.scored_bars, 60);
        assert_eq!(plan.notes.len(), 240);
        assert!((plan.notes[0].time - 4.0).abs() < 1e-9);
        assert!((plan.scoring_start() - 4.0).abs() < 1e-9);
    }
}
