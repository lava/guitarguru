//! Open-ended count-in: the click keeps going until the player has played along with
//! `NEEDED` consecutive clicks (an onset within +/-`WINDOW` of each click). The median
//! offset of those hits seeds the latency estimate.

use crate::detect::{DetectParams, detect};

pub const NEEDED: usize = 4;
pub const WINDOW: f64 = 0.120;
/// Extra wait after a click's window closes before judging it (detector settling).
const SETTLE: f64 = 0.08;

pub struct CountInGate {
    beat_s: f64,
    /// Expected offset of hits relative to clicks (manual calibration), seconds.
    allowance: f64,
    /// Next click index to judge.
    next: u64,
    /// Offsets of the current run of caught clicks (seconds).
    run: Vec<f64>,
}

impl CountInGate {
    pub fn new(beat_s: f64, allowance: f64) -> Self {
        Self { beat_s, allowance, next: 0, run: Vec::new() }
    }

    /// Clicks caught in a row so far.
    pub fn streak(&self) -> usize {
        self.run.len()
    }

    /// Judge every click whose window has closed by `now` (session seconds), given onset
    /// times in session seconds. Returns the median offset (s) once `NEEDED` in a row match.
    pub fn update(&mut self, onsets: &[f64], now: f64) -> Option<f64> {
        loop {
            let click = self.next as f64 * self.beat_s;
            if now < click + self.allowance + WINDOW + SETTLE {
                return None;
            }
            let expect = click + self.allowance;
            let hit = onsets
                .iter()
                .map(|o| o - expect)
                .filter(|d| d.abs() <= WINDOW)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()));
            match hit {
                Some(d) => self.run.push(d + self.allowance),
                None => self.run.clear(),
            }
            self.next += 1;
            if self.run.len() >= NEEDED {
                let n = self.run.len();
                return crate::dsp::median(&self.run[n - NEEDED..]);
            }
        }
    }
}

/// Onsets (seconds, relative to the start of `tail`) for the count-in gate: the detector on
/// the recent recording, ignoring very quiet detections.
pub fn tail_onsets(tail: &[f32], sample_rate: f64, sixteenth: f64) -> Vec<f64> {
    detect(tail, &DetectParams { sample_rate, sixteenth, dynamic_range_db: 45.0 })
        .into_iter()
        .filter(|o| o.level_db > -60.0)
        .map(|o| o.time)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_four_in_a_row() {
        let beat = 0.5;
        let mut g = CountInGate::new(beat, 0.0);
        // Hits on clicks 0, 1, (miss 2), 3, 4, 5, 6 with +20 ms latency.
        let onsets: Vec<f64> = [0, 1, 3, 4, 5, 6].iter().map(|k| *k as f64 * beat + 0.020).collect();
        assert_eq!(g.update(&onsets, 0.9), None);
        assert_eq!(g.streak(), 2);
        assert_eq!(g.update(&onsets, 1.4), None);
        assert_eq!(g.streak(), 0, "click 2 was missed");
        assert_eq!(g.update(&onsets, 2.8), None);
        assert_eq!(g.streak(), 3);
        let seed = g.update(&onsets, 3.3).expect("4 in a row");
        assert!((seed - 0.020).abs() < 1e-9);
    }

    #[test]
    fn far_off_hits_dont_count() {
        let beat = 0.5;
        let mut g = CountInGate::new(beat, 0.0);
        let onsets: Vec<f64> = (0..8).map(|k| k as f64 * beat + 0.25).collect(); // on the off-beats
        assert_eq!(g.update(&onsets, 5.0), None);
        assert_eq!(g.streak(), 0);
    }

    #[test]
    fn detects_onsets_in_tail() {
        let sr = 48000.0;
        let hits: Vec<(f64, f64)> = (0..4).map(|k| (0.2 + k as f64 * 0.5, -20.0)).collect();
        let x = crate::detect::tests::synth(sr, 2.2, &hits, 4);
        let on = tail_onsets(&x, sr, 0.125);
        assert_eq!(on.len(), 4);
    }
}
