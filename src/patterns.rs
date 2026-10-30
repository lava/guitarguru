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

/// A grid note to be played.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GridNote {
    /// Seconds from the session start (the first count-in click).
    pub time: f64,
    pub bar: u32,
    /// Index of the note among the played notes of one bar (the "slot").
    pub slot: usize,
}

#[derive(Clone, Debug)]
pub struct SessionPlan {
    pub bpm: u32,
    pub count_in_bars: u32,
    pub scored_bars: u32,
    pub notes: Vec<GridNote>,
}

impl SessionPlan {
    pub fn new(pattern: &Pattern, bpm: u32, count_in_bars: u32, scored_seconds: f64) -> Self {
        let bar_s = bar_seconds(bpm);
        let scored_bars = ((scored_seconds / bar_s).round() as u32).max(1);
        let s16 = sixteenth_seconds(bpm);
        let hits = pattern.hits();
        let mut notes = Vec::new();
        for b in 0..scored_bars {
            let bar_start = f64::from(count_in_bars + b) * bar_s;
            for (slot, n) in hits.iter().enumerate() {
                notes.push(GridNote { time: bar_start + n.pos * s16, bar: b, slot });
            }
        }
        Self { bpm, count_in_bars, scored_bars, notes }
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
    fn plan_lengths() {
        let p = pattern_by_id("quarters").unwrap();
        let plan = SessionPlan::new(p, 120, 2, 120.0);
        assert_eq!(plan.scored_bars, 60);
        assert_eq!(plan.notes.len(), 240);
        assert!((plan.notes[0].time - 4.0).abs() < 1e-9);
        assert!((plan.scoring_start() - 4.0).abs() < 1e-9);
    }
}
