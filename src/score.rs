//! Matching detected onsets to the note grid and turning the result into scores.
//!
//! Formulas (all percentages clamped to 0..100):
//! - spread score  = (35 ms - std) / 30 ms                       (5 ms -> 100 %, 35 ms -> 0 %)
//! - hit factor    = max(0, matched - 0.5 * extra) / grid notes
//! - timing %      = spread score * hit factor
//! - level score   = (4 dB - level std) / 3 dB                   (1 dB -> 100 %, 4 dB -> 0 %)
//!   (level std after removing each slot's mean level, so intentional accents don't count)
//! - centroid score= (0.5 oct - centroid std) / 0.4 oct          (0.1 oct -> 100 %, 0.5 -> 0 %)
//! - tone %        = 0.75 * level score + 0.25 * centroid score
//! - combined %    = 0.6 * timing % + 0.4 * tone %

use serde::{Deserialize, Serialize};

use crate::detect::Onset;
use crate::dsp::{mean, median, std};

pub const MATCH_WINDOW_SIXTEENTHS: f64 = 0.4;
/// Onsets this far below the median attack level aren't counted as extra notes
/// (re-trigger detections at ~-28 dB vs ~-17 dB real attacks were observed).
pub const EXTRA_LEVEL_MARGIN_DB: f64 = 8.0;
pub const DRIFT_WINDOW: usize = 11;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteResult {
    pub grid_time: f64,
    pub slot: usize,
    pub onset_time: Option<f64>,
    /// Deviation after latency compensation, ms (positive = late).
    pub dev_ms: Option<f64>,
    pub level_db: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SlotStat {
    pub slot: usize,
    pub count: usize,
    pub mean_ms: f64,
    pub std_ms: f64,
    /// Mean deviation is significant (|t| > 3 and > 3 ms).
    pub consistent: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScoreReport {
    pub n_grid: usize,
    pub n_matched: usize,
    pub n_missed: usize,
    pub n_extra: usize,
    pub n_quiet_ignored: usize,
    /// Constant offset subtracted (median of onset - grid), ms.
    pub latency_ms: f64,
    pub calibrated: bool,
    /// Rushing (<0) / dragging (>0) relative to the calibrated latency, if any.
    pub mean_offset_ms: Option<f64>,
    pub spread_ms: f64,
    pub slot_pattern_std_ms: f64,
    pub drift_std_ms: f64,
    pub jitter_std_ms: f64,
    pub slots: Vec<SlotStat>,
    pub level_median_db: f64,
    pub level_std_db: f64,
    pub level_std_detrended_db: f64,
    pub centroid_std_oct: Option<f64>,
    pub hit_factor: f64,
    pub spread_pct: f64,
    pub timing_pct: f64,
    pub level_pct: f64,
    pub centroid_pct: Option<f64>,
    pub tone_pct: f64,
    pub combined_pct: f64,
    pub notes: Vec<NoteResult>,
}

pub struct GridIn {
    /// Grid time in the recording's timeline (seconds), already including any known latency.
    pub time: f64,
    pub slot: usize,
}

pub struct ScoreInput<'a> {
    pub grid: &'a [GridIn],
    pub onsets: &'a [Onset],
    pub sixteenth: f64,
    pub slots_per_bar: usize,
    /// Smallest inter-onset interval of the pattern, seconds.
    pub min_ioi: f64,
    /// True if `grid` times already include a measured round-trip latency.
    pub calibrated: bool,
    /// The raw DI and its rate, for the spectral centroid (optional).
    pub audio: Option<(&'a [f32], f64)>,
}

fn nearest(onsets: &[Onset], t: f64) -> Option<usize> {
    if onsets.is_empty() {
        return None;
    }
    let i = onsets.partition_point(|o| o.time < t);
    let mut best = None;
    for j in [i.wrapping_sub(1), i] {
        if j < onsets.len() && best.is_none_or(|b: usize| (onsets[j].time - t).abs() < (onsets[b].time - t).abs()) {
            best = Some(j);
        }
    }
    best
}

/// Globally match grid notes to onsets: closest pairs first, each onset used once.
fn match_notes(grid: &[GridIn], onsets: &[Onset], offset: f64, window: f64) -> Vec<Option<usize>> {
    let mut pairs = Vec::new();
    for (gi, g) in grid.iter().enumerate() {
        let t = g.time + offset;
        let lo = onsets.partition_point(|o| o.time < t - window);
        for (oi, o) in onsets.iter().enumerate().skip(lo) {
            if o.time > t + window {
                break;
            }
            pairs.push(((o.time - t).abs(), gi, oi));
        }
    }
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut g_used = vec![None; grid.len()];
    let mut o_used = vec![false; onsets.len()];
    for (_, gi, oi) in pairs {
        if g_used[gi].is_none() && !o_used[oi] {
            g_used[gi] = Some(oi);
            o_used[oi] = true;
        }
    }
    g_used
}

fn pct(x: f64) -> f64 {
    (x * 100.0).clamp(0.0, 100.0)
}

pub fn score(inp: &ScoreInput) -> ScoreReport {
    let window = MATCH_WINDOW_SIXTEENTHS * inp.sixteenth;
    let grid = inp.grid;
    let onsets = inp.onsets;
    // Coarse latency: nearest onsets that are unambiguous (within half the smallest IOI).
    let coarse: Vec<f64> = grid
        .iter()
        .filter_map(|g| nearest(onsets, g.time).map(|j| onsets[j].time - g.time))
        .filter(|d| d.abs() < 0.5 * inp.min_ioi)
        .collect();
    let mut offset = median(&coarse).unwrap_or(0.0);
    let mut matched = match_notes(grid, onsets, offset, window);
    for _ in 0..2 {
        let d: Vec<f64> =
            grid.iter().zip(&matched).filter_map(|(g, m)| m.map(|j| onsets[j].time - g.time)).collect();
        offset = median(&d).unwrap_or(offset);
        matched = match_notes(grid, onsets, offset, window);
    }

    let mut rep = ScoreReport { n_grid: grid.len(), calibrated: inp.calibrated, latency_ms: offset * 1e3, ..Default::default() };
    let mut devs = Vec::new();
    let mut raw_devs = Vec::new();
    let mut levels = Vec::new();
    let mut slots_of = Vec::new();
    for (g, m) in grid.iter().zip(&matched) {
        let (onset_time, dev_ms, level_db) = match m {
            Some(j) => {
                let o = onsets[*j];
                let d = (o.time - g.time - offset) * 1e3;
                devs.push(d);
                raw_devs.push((o.time - g.time) * 1e3);
                levels.push(o.level_db);
                slots_of.push(g.slot);
                (Some(o.time), Some(d), Some(o.level_db))
            }
            None => (None, None, None),
        };
        rep.notes.push(NoteResult { grid_time: g.time, slot: g.slot, onset_time, dev_ms, level_db });
    }
    rep.n_matched = devs.len();
    rep.n_missed = grid.len() - devs.len();
    rep.level_median_db = median(&levels).unwrap_or(-120.0);

    // Extras: unmatched onsets inside the scored span and not much quieter than real attacks.
    let mut used = vec![false; onsets.len()];
    for j in matched.iter().flatten() {
        used[*j] = true;
    }
    if let (Some(first), Some(last)) = (grid.first(), grid.last()) {
        let (lo, hi) = (first.time + offset - window, last.time + offset + window);
        for (o, u) in onsets.iter().zip(&used) {
            if !u && o.time >= lo && o.time <= hi {
                if o.level_db >= rep.level_median_db - EXTRA_LEVEL_MARGIN_DB && !levels.is_empty() {
                    rep.n_extra += 1;
                } else {
                    rep.n_quiet_ignored += 1;
                }
            }
        }
    }

    rep.spread_ms = std(&devs);
    rep.mean_offset_ms = inp.calibrated.then(|| mean(&raw_devs));

    // Decomposition: per-slot pattern, slow drift, note-to-note jitter.
    let n_slots = inp.slots_per_bar.max(1);
    let mut slot_devs = vec![Vec::new(); n_slots];
    let mut slot_levels = vec![Vec::new(); n_slots];
    for ((d, l), s) in devs.iter().zip(&levels).zip(&slots_of) {
        slot_devs[*s % n_slots].push(*d);
        slot_levels[*s % n_slots].push(*l);
    }
    rep.slots = slot_devs
        .iter()
        .enumerate()
        .map(|(slot, v)| {
            let (m, s) = (mean(v), std(v));
            let t = if s > 0.0 { m / (s / (v.len() as f64).sqrt()) } else { 0.0 };
            SlotStat { slot, count: v.len(), mean_ms: m, std_ms: s, consistent: v.len() >= 4 && t.abs() > 3.0 && m.abs() > 3.0 }
        })
        .collect();
    let slot_mean: Vec<f64> = rep.slots.iter().map(|s| s.mean_ms).collect();
    let pattern_part: Vec<f64> = slots_of.iter().map(|s| slot_mean[*s % n_slots]).collect();
    let resid: Vec<f64> = devs.iter().zip(&pattern_part).map(|(d, p)| d - p).collect();
    let half = DRIFT_WINDOW / 2;
    let drift: Vec<f64> = (0..resid.len())
        .map(|i| mean(&resid[i.saturating_sub(half)..(i + half + 1).min(resid.len())]))
        .collect();
    let jitter: Vec<f64> = resid.iter().zip(&drift).map(|(r, d)| r - d).collect();
    rep.slot_pattern_std_ms = std(&pattern_part);
    rep.drift_std_ms = std(&drift);
    rep.jitter_std_ms = std(&jitter);

    // Tone consistency.
    rep.level_std_db = std(&levels);
    let slot_lvl_mean: Vec<f64> = slot_levels.iter().map(|v| mean(v)).collect();
    let lvl_resid: Vec<f64> = levels.iter().zip(&slots_of).map(|(l, s)| l - slot_lvl_mean[*s % n_slots]).collect();
    rep.level_std_detrended_db = std(&lvl_resid);
    if let Some((x, sr)) = inp.audio {
        let times: Vec<f64> = rep.notes.iter().filter_map(|n| n.onset_time).collect();
        let c: Vec<f64> = centroids(x, sr, &times).into_iter().map(f64::log2).collect();
        if c.len() >= 4 {
            rep.centroid_std_oct = Some(std(&c));
        }
    }

    let enough = devs.len() >= 4;
    rep.hit_factor = if grid.is_empty() { 0.0 } else { (devs.len() as f64 - 0.5 * rep.n_extra as f64).max(0.0) / grid.len() as f64 };
    rep.spread_pct = if enough { pct((35.0 - rep.spread_ms) / 30.0) } else { 0.0 };
    rep.timing_pct = rep.spread_pct * rep.hit_factor;
    rep.level_pct = if enough { pct((4.0 - rep.level_std_detrended_db) / 3.0) } else { 0.0 };
    rep.centroid_pct = rep.centroid_std_oct.map(|s| pct((0.5 - s) / 0.4));
    rep.tone_pct = match rep.centroid_pct {
        Some(c) => 0.75 * rep.level_pct + 0.25 * c,
        None => rep.level_pct,
    };
    rep.combined_pct = 0.6 * rep.timing_pct + 0.4 * rep.tone_pct;
    rep
}

/// Spectral centroid (Hz) of the first 30 ms after each onset.
pub fn centroids(x: &[f32], sr: f64, times: &[f64]) -> Vec<f64> {
    use realfft::RealFftPlanner;
    let n = (0.03 * sr) as usize;
    let fft_len = n.next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(fft_len);
    let mut buf = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let hann: Vec<f64> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect();
    let bin_hz = sr / fft_len as f64;
    let lo_bin = (80.0 / bin_hz) as usize;
    let mut out = Vec::new();
    for &t in times {
        let s = (t * sr) as usize;
        if s + n > x.len() {
            continue;
        }
        buf.iter_mut().for_each(|v| *v = 0.0);
        for i in 0..n {
            buf[i] = f64::from(x[s + i]) * hann[i];
        }
        if fft.process(&mut buf, &mut spec).is_err() {
            continue;
        }
        let (mut num, mut den) = (0.0, 0.0);
        for (k, c) in spec.iter().enumerate().skip(lo_bin) {
            let m = c.norm();
            num += m * k as f64 * bin_hz;
            den += m;
        }
        if den > 0.0 {
            out.push(num / den);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::tests::{Rng, synth};
    use crate::detect::{DetectParams, detect};

    fn grid_for(n: usize, start: f64, s16: f64, step: usize) -> Vec<GridIn> {
        (0..n).map(|i| GridIn { time: start + (i * step) as f64 * s16, slot: i % (16 / step) }).collect()
    }

    fn onsets_from(grid: &[GridIn], f: impl Fn(usize) -> Option<(f64, f64)>) -> Vec<Onset> {
        grid.iter()
            .enumerate()
            .filter_map(|(i, g)| f(i).map(|(d, l)| Onset { time: g.time + d, level_db: l }))
            .collect()
    }

    #[test]
    fn perfect_take_with_latency_scores_full() {
        let s16 = 0.125;
        let g = grid_for(64, 1.0, s16, 2);
        let on = onsets_from(&g, |_| Some((0.023, -17.0)));
        let r = score(&ScoreInput { grid: &g, onsets: &on, sixteenth: s16, slots_per_bar: 8, min_ioi: 2.0 * s16, calibrated: false, audio: None });
        assert_eq!(r.n_matched, 64);
        assert!((r.latency_ms - 23.0).abs() < 1e-6);
        assert!(r.spread_ms < 1e-6);
        assert!((r.timing_pct - 100.0).abs() < 1e-9);
        assert!((r.tone_pct - 100.0).abs() < 1e-9);
        assert!((r.combined_pct - 100.0).abs() < 1e-9);
    }

    #[test]
    fn jitter_misses_extras_and_slot_bias() {
        let s16 = 0.125;
        let mut rng = Rng::new(11);
        let g = grid_for(160, 1.0, s16, 2); // eighths, 8 slots
        let noise: Vec<f64> = (0..g.len()).map(|_| 0.010 * rng.gauss()).collect();
        let mut on = onsets_from(&g, |i| {
            if i % 20 == 5 {
                return None; // 8 misses
            }
            // slot 3 consistently 15 ms late
            let bias = if g[i].slot == 3 { 0.015 } else { 0.0 };
            Some((0.030 + bias + noise[i], -17.0 + 0.5 * (i % 3) as f64))
        });
        // 3 loud extras between notes, and 5 quiet re-triggers
        for k in 0..3 {
            on.push(Onset { time: g[10 + k * 40].time + s16 + 0.03, level_db: -18.0 });
        }
        for k in 0..5 {
            on.push(Onset { time: g[30 + k * 20].time + s16 + 0.03, level_db: -29.0 });
        }
        on.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
        let r = score(&ScoreInput { grid: &g, onsets: &on, sixteenth: s16, slots_per_bar: 8, min_ioi: 2.0 * s16, calibrated: true, audio: None });
        assert_eq!(r.n_missed, 8);
        assert_eq!(r.n_extra, 3);
        assert_eq!(r.n_quiet_ignored, 5);
        assert!(r.spread_ms > 9.0 && r.spread_ms < 13.0, "{}", r.spread_ms);
        assert!(r.slots[3].consistent && r.slots[3].mean_ms > 10.0, "{:?}", r.slots[3]);
        assert!(r.slots.iter().filter(|s| s.slot != 3).all(|s| s.mean_ms < r.slots[3].mean_ms - 10.0));
        assert!(r.jitter_std_ms > 6.0 && r.jitter_std_ms < 11.0, "{}", r.jitter_std_ms);
        let mo = r.mean_offset_ms.unwrap();
        assert!((mo - 31.9).abs() < 3.0, "{mo}");
        assert!(r.level_std_detrended_db > 0.3 && r.level_std_detrended_db < 0.5);
        // spread ~11 ms -> ~80 %, hit factor (152-1.5)/160
        let expect = (35.0 - r.spread_ms) / 30.0 * 100.0 * (152.0 - 1.5) / 160.0;
        assert!((r.timing_pct - expect).abs() < 1e-6);
    }

    #[test]
    fn end_to_end_synth_signal() {
        // Gallop at 140 BPM with 8 ms jitter, detected from audio and scored.
        let sr = 48000.0;
        let bpm = 140.0;
        let s16 = 60.0 / bpm / 4.0;
        let pos = [0.0, 2.0, 3.0, 4.0, 6.0, 7.0, 8.0, 10.0, 11.0, 12.0, 14.0, 15.0];
        let mut g = Vec::new();
        for bar in 0..12 {
            for (slot, p) in pos.iter().enumerate() {
                g.push(GridIn { time: 0.5 + (bar as f64 * 16.0 + p) * s16, slot });
            }
        }
        let mut rng = Rng::new(42);
        let hits: Vec<(f64, f64)> = g.iter().map(|n| (n.time + 0.012 + 0.008 * rng.gauss(), -17.0 + 1.5 * rng.gauss())).collect();
        let x = synth(sr, g.last().unwrap().time + 1.0, &hits, 9);
        let on = detect(&x, &DetectParams { sample_rate: sr, sixteenth: s16, dynamic_range_db: 45.0 });
        let r = score(&ScoreInput { grid: &g, onsets: &on, sixteenth: s16, slots_per_bar: 12, min_ioi: s16, calibrated: false, audio: Some((&x, sr)) });
        assert_eq!(r.n_matched, g.len());
        assert_eq!(r.n_extra, 0);
        assert!((r.latency_ms - 12.0).abs() < 3.0, "{}", r.latency_ms);
        assert!(r.spread_ms > 6.0 && r.spread_ms < 10.0, "{}", r.spread_ms);
        assert!(r.level_std_db > 1.0 && r.level_std_db < 2.0, "{}", r.level_std_db);
        assert!(r.centroid_std_oct.is_some());
    }
}
