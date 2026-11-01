//! Matching detected onsets to the note grid and turning the result into scores.
//!
//! Formulas (all percentages clamped to 0..100):
//! - global offset = median(onset - grid) over matched notes ("your latency + feel")
//! - uniformity    = sd(IOI error) / sqrt(2), IOI error = actual - expected interval of
//!                   consecutive matched attacks (lag-independent)
//! - uniformity %  = (40 ms - uniformity) / 30 ms                (10 ms -> 100 %, 40 ms -> 0 %)
//! - drift         = RMS of the 11-note moving average of (onset - grid - global offset)
//! - drift %       = (100 ms - drift) / 85 ms                    (15 ms -> 100 %, 100 ms -> 0 %)
//! - hit factor    = max(0, matched - min(0.5 * extra, grid / 4)) / grid notes
//! - timing %      = (0.75 * uniformity % + 0.25 * drift %) * hit factor
//! - tone %        = stroke consistency per pick direction and P.M./open group: volume,
//!                   attack, sustain and brightness spreads (see `tone`)
//! - combined %    = 0.7 * timing % + 0.3 * tone %;  pass at 60
//! Onsets are assigned to notes against a local offset track (sliding median over +/-4 s)
//! so matching survives a wandering player; all scores use the global offset.

use serde::{Deserialize, Serialize};

use crate::detect::Onset;
use crate::tone::{Feature, StrokeIn, ToneReport};
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
    /// The note is marked palm-muted.
    #[serde(default)]
    pub pm: bool,
    /// Local offset (latency + slow wander) the note was matched against, ms.
    #[serde(default)]
    pub offset_ms: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SlotStat {
    /// Flat slot id (see the run's slot table).
    pub slot: usize,
    /// Pattern id and slot within that pattern's bar (filled in by the session).
    pub pattern: String,
    pub slot_in_pattern: usize,
    pub pos16: f64,
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
    /// Note-to-note uniformity: sd of the inter-onset-interval errors / sqrt(2), ms
    /// (onset-equivalent; the headline timing number).
    pub spread_ms: f64,
    /// Spread around the global offset (includes slow drift), ms.
    #[serde(default)]
    pub spread_total_ms: f64,
    /// Spread of the local matching track (informational), ms.
    #[serde(default)]
    pub wander_ms: f64,
    /// Spread of the intervals between consecutive attacks vs. expected, ms.
    #[serde(default)]
    pub ioi_sd_ms: f64,
    /// RMS of the slow component (11-note average) of deviations from the global offset, ms.
    #[serde(default)]
    pub drift_rms_ms: f64,
    #[serde(default)]
    pub drift_pct: f64,
    pub slot_pattern_std_ms: f64,
    pub drift_std_ms: f64,
    pub jitter_std_ms: f64,
    pub slots: Vec<SlotStat>,
    pub level_median_db: f64,
    pub level_std_db: f64,
    /// Volume spread within the tone groups (after removing note means), count-weighted, dB.
    pub level_std_detrended_db: f64,
    /// Stroke consistency per (pick direction, P.M.) group.
    #[serde(default)]
    pub tone: ToneReport,
    pub hit_factor: f64,
    pub spread_pct: f64,
    pub timing_pct: f64,
    pub tone_pct: f64,
    pub combined_pct: f64,
    pub notes: Vec<NoteResult>,
}

pub struct GridIn {
    /// Grid time in the recording's timeline (seconds), already including any known latency.
    pub time: f64,
    pub slot: usize,
    /// Marked palm-muted in the pattern (and palm mutes are on).
    pub pm: bool,
    /// Pick direction.
    pub down: bool,
    /// Fretted note (MIDI) and string, to compare like with like.
    pub midi: u8,
    pub string: u8,
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

/// Half-width (seconds) of the sliding window for the local offset track.
pub const TRACK_HALF_S: f64 = 4.0;

/// Sliding median of `devs` (per grid note, None = no estimate) over grid notes within
/// +/-TRACK_HALF_S; falls back to `fallback` where a window has no estimates.
fn local_track(grid: &[GridIn], devs: &[Option<f64>], fallback: f64) -> Vec<f64> {
    let mut lo = 0;
    let mut hi = 0;
    (0..grid.len())
        .map(|i| {
            let t = grid[i].time;
            while grid[lo].time < t - TRACK_HALF_S {
                lo += 1;
            }
            while hi < grid.len() && grid[hi].time <= t + TRACK_HALF_S {
                hi += 1;
            }
            let v: Vec<f64> = devs[lo..hi].iter().flatten().copied().collect();
            if v.len() >= 3 { median(&v).unwrap_or(fallback) } else { fallback }
        })
        .collect()
}

/// Globally match grid notes to onsets: closest pairs first, each onset used once. Each
/// grid note is looked for around its own expected offset.
fn match_notes(grid: &[GridIn], onsets: &[Onset], offsets: &[f64], window: f64) -> Vec<Option<usize>> {
    let mut pairs = Vec::new();
    for (gi, g) in grid.iter().enumerate() {
        let t = g.time + offsets[gi];
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

/// Evenness score: 10 ms -> 100 %, 40 ms -> 0 %.
pub fn evenness_pct(spread_ms: f64) -> f64 {
    pct((40.0 - spread_ms) / 30.0)
}

/// Drift score: 15 ms -> 100 %, 100 ms -> 0 %.
pub fn drift_pct(drift_rms_ms: f64) -> f64 {
    pct((100.0 - drift_rms_ms) / 85.0)
}

/// Share of the grid that counts as hit: extras cost half a miss each, capped at a
/// quarter of the grid so a detector problem can't zero the score on its own.
pub fn hit_factor(n_grid: usize, n_matched: usize, n_extra: usize) -> f64 {
    if n_grid == 0 {
        return 0.0;
    }
    let extra_pen = (0.5 * n_extra as f64).min(n_grid as f64 / 4.0);
    (n_matched as f64 - extra_pen).max(0.0) / n_grid as f64
}

pub fn timing_pct(spread_ms: f64, drift_rms_ms: f64, hit: f64) -> f64 {
    (0.75 * evenness_pct(spread_ms) + 0.25 * drift_pct(drift_rms_ms)) * hit
}

/// Share of timing in the combined score; tone has the rest.
pub const TIMING_WEIGHT: f64 = 0.7;

pub fn combined_pct(timing: f64, tone: f64) -> f64 {
    TIMING_WEIGHT * timing + (1.0 - TIMING_WEIGHT) * tone
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
    let offset0 = median(&coarse).unwrap_or(0.0);
    // Local offset track: the heard click and the player's sense of it can wander slowly
    // (output latency changes, tempo drift). Estimate it with a sliding median of
    // unambiguous nearest-onset deviations, then match around it and refine twice.
    let near: Vec<Option<f64>> = grid
        .iter()
        .map(|g| nearest(onsets, g.time + offset0).map(|j| onsets[j].time - g.time).filter(|d| (d - offset0).abs() < 0.45 * inp.min_ioi))
        .collect();
    let mut track = local_track(grid, &near, offset0);
    let mut matched = match_notes(grid, onsets, &track, window);
    for _ in 0..2 {
        let d: Vec<Option<f64>> = grid.iter().zip(&matched).map(|(g, m)| m.map(|j| onsets[j].time - g.time)).collect();
        track = local_track(grid, &d, offset0);
        matched = match_notes(grid, onsets, &track, window);
    }
    let all: Vec<f64> = grid.iter().zip(&matched).filter_map(|(g, m)| m.map(|j| onsets[j].time - g.time)).collect();
    let offset = median(&all).unwrap_or(offset0);

    let mut rep = ScoreReport { n_grid: grid.len(), calibrated: inp.calibrated, latency_ms: offset * 1e3, ..Default::default() };
    let mut devs = Vec::new();
    let mut total_devs = Vec::new();
    let mut track_ms = Vec::new();
    let mut raw_devs = Vec::new();
    let mut levels = Vec::new();
    let mut slots_of = Vec::new();
    for ((g, m), tr) in grid.iter().zip(&matched).zip(&track) {
        let (onset_time, dev_ms, level_db) = match m {
            Some(j) => {
                let o = onsets[*j];
                // Deviation from the single global offset (the reference); the local track
                // only decided which onset belongs to which note.
                let d = (o.time - g.time - offset) * 1e3;
                devs.push(d);
                total_devs.push(d);
                track_ms.push((tr - offset) * 1e3);
                raw_devs.push((o.time - g.time) * 1e3);
                levels.push(o.level_db);
                slots_of.push(g.slot);
                (Some(o.time), Some(d), Some(o.level_db))
            }
            None => (None, None, None),
        };
        rep.notes.push(NoteResult { grid_time: g.time, slot: g.slot, onset_time, dev_ms, level_db, pm: g.pm, offset_ms: tr * 1e3 });
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
        let (lo, hi) = (first.time + track[0] - window, last.time + track[track.len() - 1] + window);
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

    // Main, lag-independent criterion: uniformity of the intervals between consecutive
    // matched attacks. IOI error = actual interval - expected interval; across misses the
    // next matched note is used with its (multi-note) expected interval. For independent
    // per-note jitter, sd(IOI error) = sqrt(2) * sd(onset), so divide by sqrt(2) to keep
    // the onset-based 10..40 ms scale.
    let ioi_err: Vec<f64> = devs.windows(2).map(|w| w[1] - w[0]).collect();
    rep.ioi_sd_ms = std(&ioi_err);
    rep.spread_ms = rep.ioi_sd_ms / std::f64::consts::SQRT_2;
    rep.spread_total_ms = std(&total_devs);
    rep.wander_ms = std(&track_ms);
    // Long-term drift against the global offset: RMS of the slow component (11-note
    // moving average) of onset - grid - global offset.
    let half = DRIFT_WINDOW / 2;
    let slow: Vec<f64> = (0..devs.len()).map(|i| mean(&devs[i.saturating_sub(half)..(i + half + 1).min(devs.len())])).collect();
    rep.drift_rms_ms = if slow.is_empty() { 0.0 } else { (slow.iter().map(|v| v * v).sum::<f64>() / slow.len() as f64).sqrt() };
    rep.mean_offset_ms = inp.calibrated.then(|| mean(&raw_devs));

    // Decomposition: per-slot pattern, slow drift, note-to-note jitter.
    let n_slots = inp.slots_per_bar.max(1);
    let mut slot_devs = vec![Vec::new(); n_slots];
    for (d, s) in devs.iter().zip(&slots_of) {
        slot_devs[*s % n_slots].push(*d);
    }
    rep.slots = slot_devs
        .iter()
        .enumerate()
        .map(|(slot, v)| {
            let (m, s) = (mean(v), std(v));
            let t = if s > 0.0 { m / (s / (v.len() as f64).sqrt()) } else { 0.0 };
            SlotStat {
                slot,
                count: v.len(),
                mean_ms: m,
                std_ms: s,
                consistent: v.len() >= 4 && t.abs() > 3.0 && m.abs() > 3.0,
                ..Default::default()
            }
        })
        .collect();
    let slot_mean: Vec<f64> = rep.slots.iter().map(|s| s.mean_ms).collect();
    let pattern_part: Vec<f64> = slots_of.iter().map(|s| slot_mean[*s % n_slots]).collect();
    let resid: Vec<f64> = devs.iter().zip(&pattern_part).map(|(d, p)| d - p).collect();
    let drift: Vec<f64> = (0..resid.len())
        .map(|i| mean(&resid[i.saturating_sub(half)..(i + half + 1).min(resid.len())]))
        .collect();
    let jitter: Vec<f64> = resid.iter().zip(&drift).map(|(r, d)| r - d).collect();
    rep.slot_pattern_std_ms = std(&pattern_part);
    rep.drift_std_ms = std(&drift);
    rep.jitter_std_ms = std(&jitter);

    // Tone consistency.
    rep.level_std_db = std(&levels);
    if let Some((x, sr)) = inp.audio {
        let strokes: Vec<StrokeIn> = grid
            .iter()
            .zip(&rep.notes)
            .enumerate()
            .filter_map(|(i, (g, n))| {
                Some(StrokeIn {
                    note: i,
                    onset: n.onset_time?,
                    level_db: n.level_db?,
                    down: g.down,
                    pm: g.pm,
                    midi: g.midi,
                    string: g.string,
                    position: g.slot,
                })
            })
            .collect();
        let times: Vec<f64> = onsets.iter().map(|o| o.time).collect();
        rep.tone = crate::tone::analyse(x, sr, &strokes, &times);
    }
    let lv: Vec<(f64, f64)> = rep.tone.groups.iter().filter_map(|g| g.sd_of(Feature::Level).map(|s| (g.n as f64, s))).collect();
    let w: f64 = lv.iter().map(|v| v.0).sum();
    rep.level_std_detrended_db = if w > 0.0 { lv.iter().map(|v| v.0 * v.1).sum::<f64>() / w } else { rep.level_std_db };

    let enough = devs.len() >= 4;
    rep.hit_factor = hit_factor(grid.len(), devs.len(), rep.n_extra);
    rep.spread_pct = if enough { evenness_pct(rep.spread_ms) } else { 0.0 };
    rep.drift_pct = if enough { drift_pct(rep.drift_rms_ms) } else { 0.0 };
    rep.timing_pct = if enough { timing_pct(rep.spread_ms, rep.drift_rms_ms, rep.hit_factor) } else { 0.0 };
    rep.tone_pct = if enough { rep.tone.pct.unwrap_or(0.0) } else { 0.0 };
    rep.combined_pct = combined_pct(rep.timing_pct, rep.tone_pct);
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
        (0..n).map(|i| GridIn { time: start + (i * step) as f64 * s16, slot: i % (16 / step), pm: true, down: true, midi: 40, string: 6 }).collect()
    }

    fn onsets_from(grid: &[GridIn], mut f: impl FnMut(usize) -> Option<(f64, f64)>) -> Vec<Onset> {
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
        assert_eq!(r.tone_pct, 0.0, "no audio, no tone");
        assert!((r.combined_pct - 100.0 * TIMING_WEIGHT).abs() < 1e-9);
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
        // hit factor (152-1.5)/160
        let expect = (0.75 * (40.0 - r.spread_ms) / 30.0 * 100.0 + 0.25 * r.drift_pct) * (152.0 - 1.5) / 160.0;
        assert!((r.timing_pct - expect).abs() < 1e-6);
    }

    #[test]
    fn ioi_uniformity_ignores_lag_and_separates_drift() {
        // Eighths at 120 BPM for 2 minutes, 120 ms constant lag, 8 ms jitter, and a slow
        // 60 ms-amplitude tempo wander (period 30 s).
        let s16 = 0.125;
        let g = grid_for(480, 1.0, s16, 2);
        let mut rng = Rng::new(5);
        let on = onsets_from(&g, |i| {
            let t = g[i].time;
            let wander = 0.060 * (std::f64::consts::TAU * t / 30.0).sin();
            Some((0.120 + wander + 0.008 * rng.gauss(), -17.0))
        });
        let r = score(&ScoreInput { grid: &g, onsets: &on, sixteenth: s16, slots_per_bar: 8, min_ioi: 2.0 * s16, calibrated: false, audio: None });
        assert_eq!(r.n_matched, 480, "matching follows the wander");
        assert!((r.latency_ms - 120.0).abs() < 15.0, "global offset {}", r.latency_ms);
        assert!(r.spread_ms > 6.5 && r.spread_ms < 10.0, "uniformity {} ~ jitter 8 ms", r.spread_ms);
        assert!((r.drift_rms_ms - 42.4).abs() < 6.0, "drift {} ~ 60/sqrt(2)", r.drift_rms_ms);
        assert!(r.spread_total_ms > 40.0);
        // Same playing with a different lag scores the same uniformity.
        let on2: Vec<Onset> = on.iter().map(|o| Onset { time: o.time - 0.07, ..*o }).collect();
        let r2 = score(&ScoreInput { grid: &g, onsets: &on2, sixteenth: s16, slots_per_bar: 8, min_ioi: 2.0 * s16, calibrated: false, audio: None });
        assert!((r2.spread_ms - r.spread_ms).abs() < 1e-6);
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
                g.push(GridIn { time: 0.5 + (bar as f64 * 16.0 + p) * s16, slot, pm: true, down: true, midi: 40, string: 6 });
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
        assert_eq!(r.tone.groups.len(), 1, "all downstrokes, palm-muted");
        assert!(r.tone.groups[0].sd_of(Feature::Brightness).is_some());
    }
}
