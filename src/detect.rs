//! Attack (onset) detection on a mono DI signal. Port of the Python detector proven on
//! real palm-muted takes: high-passed 1 ms RMS envelope in dB, 5 ms rise ("flux"), peak
//! picking, then refinement to where the envelope first exceeds 30 % of the following peak.
//!
//! Dark, low-treble signals (open low strings through a bright-less pickup) put the
//! 1.5 kHz band near its noise floor during the sustain, where the 1 ms envelope spikes
//! once per pitch period. Those spikes can pass the contrast test about 105 ms (the
//! minimum peak distance) after a real attack. A re-attack within `REATTACK_S` of the
//! previous attack therefore also needs a real energy rise (`MIN_REATTACK_RISE_DB`) in at
//! least one lower band (full band, >300 Hz, >800 Hz): a new pick stroke raises at least
//! one of them, ringing doesn't.

use serde::{Deserialize, Serialize};

use crate::dsp::{butter_highpass, filtfilt, find_peaks, moving_rms};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Onset {
    /// Seconds from the start of the analysed buffer.
    pub time: f64,
    /// Attack level: peak |x| over the first 30 ms, dBFS.
    pub level_db: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct DetectParams {
    pub sample_rate: f64,
    /// One sixteenth note in seconds (sets the minimum peak distance).
    pub sixteenth: f64,
    /// Ignore onsets quieter than `max - dynamic_range_db` (envelope).
    pub dynamic_range_db: f64,
}

/// An attack must rise this far (dB) above the median envelope of the 40..5 ms before it.
/// Rejects re-triggers on ringing open strings (on real takes these cluster below ~8 dB,
/// real attacks above ~15 dB).
pub const MIN_CONTRAST_DB: f64 = 12.0;

/// Attacks closer than this to the previous one must show an energy rise in a lower band.
pub const REATTACK_S: f64 = 0.15;
/// Energy rise (dB) required of a re-attack: mean power over the 20 ms after the onset vs
/// the 30..5 ms before it, best of the full band and the >300 Hz and >800 Hz bands. On the
/// real takes, phantom re-triggers sit within +/-1 dB in all three; real re-attacks (also
/// quiet palm-muted notes right after an accent) rise by 7..30 dB in at least one.
pub const MIN_REATTACK_RISE_DB: f64 = 3.0;

/// Prefix sums of x^2 of the full band and two high-passed bands, for windowed power.
struct Bands(Vec<Vec<f64>>);

impl Bands {
    fn new(x: &[f32], sr: f64) -> Self {
        let full: Vec<f64> = x.iter().map(|&v| f64::from(v)).collect();
        let b300 = filtfilt(&butter_highpass(4, 300.0, sr), x);
        let b800 = filtfilt(&butter_highpass(4, 800.0, sr), x);
        Self(
            [full, b300, b800]
                .into_iter()
                .map(|y| {
                    let mut acc = 0.0;
                    std::iter::once(0.0)
                        .chain(y.iter().map(|v| {
                            acc += v * v;
                            acc
                        }))
                        .collect()
                })
                .collect(),
        )
    }

    /// Best rise (dB) over the bands: power in [i, i+post) vs [i-pre_a, i-pre_b).
    fn rise_db(&self, i: usize, post: usize, pre_a: usize, pre_b: usize) -> f64 {
        self.0
            .iter()
            .map(|cs| {
                let n = cs.len() - 1;
                let pw = |a: usize, b: usize| {
                    let (a, b) = (a.min(n), b.min(n));
                    if b > a { (cs[b] - cs[a]) / (b - a) as f64 } else { 0.0 }
                };
                let after = pw(i, i + post);
                let before = pw(i.saturating_sub(pre_a), i.saturating_sub(pre_b));
                10.0 * ((after + 1e-20) / (before + 1e-20)).log10()
            })
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

pub fn detect(x: &[f32], p: &DetectParams) -> Vec<Onset> {
    let sr = p.sample_rate;
    if x.len() < (0.05 * sr) as usize {
        return Vec::new();
    }
    let hp = filtfilt(&butter_highpass(4, 1500.0, sr), x);
    let env = moving_rms(&hp, (0.001 * sr) as usize);
    let loud: Vec<f64> = env.iter().map(|e| 20.0 * (e + 1e-9).log10()).collect();
    let lag = (0.005 * sr) as usize;
    let mut flux = vec![0.0; loud.len()];
    for i in lag..loud.len() {
        flux[i] = (loud[i] - loud[i - lag]).max(0.0);
    }
    let distance = ((0.55 * p.sixteenth * sr) as usize).max(1);
    let peaks = find_peaks(&flux, 6.0, distance, 4.0);
    let max_loud = loud.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let w30 = (0.03 * sr) as usize;
    let (pre_a, pre_b) = ((0.040 * sr) as usize, lag);
    let reattack = (REATTACK_S * sr) as usize;
    let (rise_post, rise_a) = ((0.020 * sr) as usize, (0.030 * sr) as usize);
    let mut bands: Option<Bands> = None;
    let mut out: Vec<Onset> = Vec::new();
    let mut last: Option<usize> = None;
    for pk in peaks.into_iter().filter(|&pk| loud[pk] > max_loud - p.dynamic_range_db) {
        let end = (pk + w30).min(env.len());
        let top = env[pk..end].iter().copied().fold(0.0, f64::max);
        let from = pk.saturating_sub(lag);
        let start = from + env[from..end].iter().position(|&e| e > 0.3 * top).unwrap_or(0);
        // Contrast against what was ringing just before.
        let (a, b) = (start.saturating_sub(pre_a), start.saturating_sub(pre_b));
        if b > a {
            let pre = crate::dsp::median(&env[a..b]).unwrap_or(0.0);
            if 20.0 * (top / (pre + 1e-12)).log10() < MIN_CONTRAST_DB {
                continue;
            }
        }
        if last.is_some_and(|l| start - l.min(start) < reattack) {
            let bands = bands.get_or_insert_with(|| Bands::new(x, sr));
            if bands.rise_db(start, rise_post, rise_a, lag) < MIN_REATTACK_RISE_DB {
                continue;
            }
        }
        last = Some(start);
        let aend = (start + w30).min(x.len());
        let peak = x[start..aend].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        out.push(Onset { time: start as f64 / sr, level_db: 20.0 * (f64::from(peak) + 1e-9).log10() });
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Tiny deterministic PRNG for test signals.
    pub struct Rng(u64);
    impl Rng {
        pub fn new(seed: u64) -> Self {
            Self(seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407))
        }
        pub fn next_f64(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        /// Approximately normal (sum of uniforms).
        pub fn gauss(&mut self) -> f64 {
            (0..12).map(|_| self.next_f64()).sum::<f64>() - 6.0
        }
    }

    /// Palm-mute-like hits: decaying noise bursts plus a low "string" tone, over a noise floor.
    pub fn synth(sr: f64, len_s: f64, hits: &[(f64, f64)], seed: u64) -> Vec<f32> {
        let mut rng = Rng::new(seed);
        let n = (len_s * sr) as usize;
        let mut x: Vec<f64> = (0..n).map(|_| 1e-4 * rng.gauss()).collect();
        for &(t, level_db) in hits {
            let amp = 10f64.powf(level_db / 20.0);
            let s = (t * sr) as usize;
            for i in 0..(0.5 * sr) as usize {
                if s + i >= n {
                    break;
                }
                let tt = i as f64 / sr;
                let burst = (rng.next_f64() * 2.0 - 1.0) * (-tt / 0.004).exp();
                let tone = (2.0 * std::f64::consts::PI * 82.4 * tt).sin() * (-tt / 0.05).exp();
                x[s + i] += amp * (0.6 * burst + 0.4 * tone);
            }
        }
        x.into_iter().map(|v| v as f32).collect()
    }

    #[test]
    fn detects_bursts_at_known_times() {
        let sr = 48000.0;
        let mut rng = Rng::new(7);
        let s16 = 60.0 / 120.0 / 4.0;
        let truth: Vec<(f64, f64)> =
            (0..64).map(|i| (0.5 + i as f64 * s16 + 0.004 * rng.gauss(), -18.0 + rng.gauss())).collect();
        let x = synth(sr, 0.5 + 64.0 * s16 + 0.5, &truth, 3);
        let on = detect(&x, &DetectParams { sample_rate: sr, sixteenth: s16, dynamic_range_db: 45.0 });
        if on.len() != truth.len() {
            for o in &on { eprintln!("{:.4} {:.1}", o.time, o.level_db); }
        }
        assert_eq!(on.len(), truth.len());
        for (o, (t, _)) in on.iter().zip(&truth) {
            assert!((o.time - t).abs() < 0.0015, "onset {} vs {}", o.time, t);
        }
    }

    /// Ringing open strings: slowly decaying, beating high partials between attacks must
    /// not re-trigger the detector.
    #[test]
    fn ringing_strings_do_not_retrigger() {
        let sr = 48000.0;
        let beat = 0.75;
        let hits: Vec<f64> = (0..12).map(|i| 0.5 + i as f64 * beat).collect();
        let n = ((0.5 + 12.0 * beat + 1.0) * sr) as usize;
        let mut rng = Rng::new(9);
        let mut x = vec![0.0f64; n];
        for v in x.iter_mut() {
            *v = 3e-4 * rng.gauss();
        }
        for &t in &hits {
            let s = (t * sr) as usize;
            for i in 0..(beat * sr) as usize {
                if s + i >= n {
                    break;
                }
                let tt = i as f64 / sr;
                let pick = (rng.next_f64() * 2.0 - 1.0) * (-tt / 0.003).exp() * 0.3;
                // Several high partials with close frequencies -> 6..14 Hz beating, slow decay.
                let ring: f64 = [(2010.0, 2016.0), (2730.0, 2741.0), (3420.0, 3434.0)]
                    .iter()
                    .map(|(a, b)| ((std::f64::consts::TAU * a * tt).sin() + (std::f64::consts::TAU * b * tt).sin()) * 0.03)
                    .sum::<f64>()
                    * (-tt / 0.6).exp();
                x[s + i] += pick + ring;
            }
        }
        let x: Vec<f32> = x.into_iter().map(|v| v as f32).collect();
        let on = detect(&x, &DetectParams { sample_rate: sr, sixteenth: beat / 4.0, dynamic_range_db: 45.0 });
        assert_eq!(on.len(), hits.len(), "{:?}", on.iter().map(|o| o.time).collect::<Vec<_>>());
    }

    /// The palm-muted reference takes the detector was first tuned on (skipped if absent):
    /// nearly every grid note is found, with few unmatched detections.
    #[test]
    #[ignore = "needs reference takes"]
    fn reference_takes_still_detect() {
        let root = std::path::Path::new("reference-takes");
        let Ok(sessions) = std::fs::read_dir(root) else { return };
        for sess in sessions.flatten() {
            let Ok(parts) = std::fs::read_dir(sess.path()) else { continue };
            for part in parts.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(" - Gtr I")) {
                let Ok(text) = std::fs::read_to_string(part.join("grid.json")) else { continue };
                let g: serde_json::Value = serde_json::from_str(&text).unwrap();
                let s16 = g["bar_seconds"].as_f64().unwrap() / 16.0;
                let mut grid: Vec<f64> = g["notes"].as_array().unwrap().iter().filter_map(|n| n["time"].as_f64()).collect();
                grid.sort_by(f64::total_cmp);
                grid.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
                let Ok(files) = std::fs::read_dir(part.join("Audio Files")) else { continue };
                for wav in files.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Guitar DI_"))) {
                    let mut r = hound::WavReader::open(&wav).unwrap();
                    let (sr, ch) = (f64::from(r.spec().sample_rate), r.spec().channels as usize);
                    let x: Vec<f32> = r.samples::<f32>().filter_map(Result::ok).step_by(ch).collect();
                    let on = detect(&x, &DetectParams { sample_rate: sr, sixteenth: s16, dynamic_range_db: 45.0 });
                    if on.len() < 10 {
                        continue;
                    }
                    let g: Vec<f64> = grid.iter().copied().filter(|t| *t < x.len() as f64 / sr - 0.5).collect();
                    let near = |t: f64| on.iter().map(|o| o.time - t).min_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap();
                    let mut d: Vec<f64> = g.iter().map(|&t| near(t)).filter(|d| d.abs() < 0.8 * s16).collect();
                    d.sort_by(f64::total_cmp);
                    let off = d[d.len() / 2];
                    let matched = g.iter().filter(|&&t| (near(t + off)).abs() < 0.4 * s16).count();
                    let span = |t: f64| t > g[0] + off - 0.4 * s16 && t < g[g.len() - 1] + off + 0.4 * s16;
                    let inside = on.iter().filter(|o| span(o.time)).count();
                    println!("{}: grid {} detected {} matched {}", wav.display(), g.len(), inside, matched);
                    assert!(matched * 100 >= g.len() * 92, "missed too many");
                    assert!(inside * 100 <= g.len() * 104, "too many detections");
                }
            }
        }
    }
}
