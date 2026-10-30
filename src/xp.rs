//! Experience: every finished run earns XP, and the guru's stage follows the total.
//!
//! A run earns `(20 * score + 10 if passed) * bpm / 80 * (1 + 0.25 * ingredients)`, scaled
//! down for runs ended early. A typical first pass (basic, 80 BPM, ~70 %) is worth 24 XP;
//! a pass at 120 BPM with two ingredients ~56.

use crate::game::SCORED_SECONDS;
use crate::patterns::{BASE_BPM, Level};
use crate::progress::{HistoryEntry, PASS_THRESHOLD};

/// Total XP at which each stage starts (stage 1 at 0). Steps 120, 240, 420, 720.
pub const STAGE_XP: [u32; 5] = [0, 120, 360, 780, 1500];
pub const MAX_STAGE: u32 = STAGE_XP.len() as u32;

/// XP for one run of `level` scoring `combined` percent over `seconds` (the run's
/// length, count-in included).
pub fn run_xp(level: &Level, combined: f64, seconds: f64) -> u32 {
    let score = combined.clamp(0.0, 100.0) / 100.0 * 20.0;
    let pass = if combined >= PASS_THRESHOLD { 10.0 } else { 0.0 };
    let tempo = f64::from(level.bpm) / f64::from(BASE_BPM);
    let mix = 1.0 + 0.25 * f64::from(level.mask().count_ones());
    let played = (seconds / SCORED_SECONDS).clamp(0.0, 1.0);
    ((score + pass) * tempo * mix * played).round() as u32
}

/// XP a history entry earned (for seeding XP from history kept before XP existed).
pub fn entry_xp(h: &HistoryEntry) -> u32 {
    Level::from_key(&h.key).map_or(0, |l| run_xp(&l, h.combined, h.seconds))
}

/// Stage (1..=MAX_STAGE) at this total.
pub fn stage(xp: u32) -> u32 {
    STAGE_XP.iter().filter(|&&s| xp >= s).count() as u32
}

/// Fraction of the way through the current stage (1.0 at the top stage).
pub fn fill(xp: f64) -> f64 {
    let s = stage(xp as u32) as usize;
    if s >= STAGE_XP.len() {
        return 1.0;
    }
    let (a, b) = (f64::from(STAGE_XP[s - 1]), f64::from(STAGE_XP[s]));
    ((xp - a) / (b - a)).clamp(0.0, 1.0)
}

/// Seconds the bar takes to fill once, and to show a stage-up.
const FILL_S: f64 = 1.4;
const STAGE_UP_S: f64 = 1.3;

/// The menu bar's animation from `from` to `to` XP, `t` seconds in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    /// XP the bar shows.
    pub xp: f64,
    /// During a stage-up: (new stage, progress 0..1); the bar sits full meanwhile.
    pub stage_up: Option<(u32, f64)>,
    pub done: bool,
}

fn ease(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

pub fn animate(from: u32, to: u32, t: f64) -> Shown {
    if to <= from {
        return Shown { xp: f64::from(to), stage_up: None, done: true };
    }
    let mut t = t.max(0.0);
    let mut cur = from;
    loop {
        let s = stage(cur) as usize;
        // The end of this leg: the next stage threshold or the target.
        let next = STAGE_XP.get(s).copied().filter(|&n| n <= to);
        let end = next.unwrap_or(to);
        let span = if s < STAGE_XP.len() { f64::from(STAGE_XP[s] - STAGE_XP[s - 1]) } else { f64::from(to - from).max(1.0) };
        let dur = (f64::from(end - cur) / span * FILL_S).max(0.25);
        if t < dur {
            let xp = f64::from(cur) + f64::from(end - cur) * ease(t / dur);
            // Stay just under the threshold until the leg ends, so the bar reads full.
            return Shown { xp: if next.is_some() { xp.min(f64::from(end) - 1e-6) } else { xp }, stage_up: None, done: false };
        }
        t -= dur;
        let Some(n) = next else { return Shown { xp: f64::from(to), stage_up: None, done: true } };
        if t < STAGE_UP_S {
            return Shown { xp: f64::from(n) - 1e-6, stage_up: Some((s as u32 + 1, t / STAGE_UP_S)), done: false };
        }
        t -= STAGE_UP_S;
        cur = n;
        if cur >= to {
            return Shown { xp: f64::from(to), stage_up: None, done: true };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::{PM, S16, SYNC};

    #[test]
    fn earns() {
        assert_eq!(run_xp(&Level::run(0, 80), 70.0, 125.0), 24);
        assert_eq!(run_xp(&Level::run(0, 80), 45.0, 125.0), 9, "no pass bonus");
        assert_eq!(run_xp(&Level::run(SYNC | PM, 120), 70.0, 125.0), 54);
        assert_eq!(run_xp(&Level::run(0, 80), 70.0, 12.0), 2, "ended early");
        assert!(run_xp(&Level::run(SYNC | PM | S16, 180), 100.0, 125.0) > 100);
    }

    #[test]
    fn stages() {
        assert_eq!(stage(0), 1);
        assert_eq!(stage(119), 1);
        assert_eq!(stage(120), 2);
        assert_eq!(stage(1500), 5);
        assert_eq!(stage(99999), 5);
        assert!((fill(60.0) - 0.5).abs() < 1e-9);
        assert_eq!(fill(2000.0), 1.0);
        // About five first passes (plus a few misses) reach stage 2.
        assert!((4..=6).contains(&(STAGE_XP[1] / 24)));
    }

    #[test]
    fn animation_passes_through_stage_up() {
        let a = animate(100, 150, 0.0);
        assert_eq!((a.xp, a.done), (100.0, false));
        // 20 of 120 to the threshold: the minimum leg, then the stage-up.
        let up = animate(100, 150, 0.3);
        assert_eq!(up.stage_up.map(|s| s.0), Some(2));
        assert_eq!(stage(up.xp as u32), 1, "bar full in the old stage");
        let after = animate(100, 150, 0.25 + STAGE_UP_S + 0.1);
        assert!(after.stage_up.is_none() && after.xp > 120.0 && after.xp < 150.0);
        assert!(animate(100, 150, 10.0).done);
        assert_eq!(animate(100, 150, 10.0).xp, 150.0);
        assert!(animate(150, 150, 0.0).done);
        // Two stage-ups in one go.
        let ups: Vec<u32> = (0..200).filter_map(|i| animate(100, 400, f64::from(i) * 0.05).stage_up.map(|s| s.0)).collect();
        assert!(ups.contains(&2) && ups.contains(&3));
        // Beyond the top stage.
        assert!(animate(1600, 1700, 10.0).done);
    }
}
