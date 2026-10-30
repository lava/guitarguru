//! Attack (onset) detection on a mono DI signal. Port of the Python detector proven on
//! real palm-muted takes: high-passed 1 ms RMS envelope in dB, 5 ms rise ("flux"), peak
//! picking, then refinement to where the envelope first exceeds 30 % of the following peak.

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
    peaks
        .into_iter()
        .filter(|&pk| loud[pk] > max_loud - p.dynamic_range_db)
        .map(|pk| {
            let end = (pk + w30).min(env.len());
            let top = env[pk..end].iter().copied().fold(0.0, f64::max);
            let from = pk.saturating_sub(lag);
            let start = from + env[from..end].iter().position(|&e| e > 0.3 * top).unwrap_or(0);
            let aend = (start + w30).min(x.len());
            let peak = x[start..aend].iter().fold(0.0f32, |m, v| m.max(v.abs()));
            Onset { time: start as f64 / sr, level_db: 20.0 * (f64::from(peak) + 1e-9).log10() }
        })
        .collect()
}

/// Incremental wrapper for live feedback: re-runs the detector over the tail of the
/// growing recording and reports onsets once they are old enough to be stable.
pub struct LiveDetector {
    params: DetectParams,
    /// Index into the recording up to which onsets have been reported.
    reported_until: f64,
    last_onset: f64,
}

impl LiveDetector {
    pub fn new(params: DetectParams) -> Self {
        Self { params, reported_until: 0.0, last_onset: f64::NEG_INFINITY }
    }

    /// `rec` is the full recording so far. Returns newly detected onsets (absolute times).
    pub fn update(&mut self, rec: &[f32]) -> Vec<Onset> {
        let sr = self.params.sample_rate;
        let window = (1.5 * sr) as usize;
        let settle = 0.06; // seconds an onset must be in the past before it's reported
        let start = rec.len().saturating_sub(window);
        let t0 = start as f64 / sr;
        let t_end = rec.len() as f64 / sr - settle;
        if t_end <= self.reported_until {
            return Vec::new();
        }
        let min_gap = 0.55 * self.params.sixteenth;
        let mut out = Vec::new();
        for mut o in detect(&rec[start..], &self.params) {
            o.time += t0;
            // Skip the start of the window (filter edge) except at the very beginning.
            if start > 0 && o.time < t0 + 0.05 {
                continue;
            }
            // Live view only: ignore noise-floor wiggles when nothing is played.
            if o.level_db < -60.0 {
                continue;
            }
            if o.time > self.reported_until && o.time <= t_end && o.time - self.last_onset >= min_gap {
                self.last_onset = o.time;
                out.push(o);
            }
        }
        self.reported_until = t_end;
        out
    }
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

    #[test]
    fn live_matches_offline() {
        let sr = 48000.0;
        let s16 = 60.0 / 100.0 / 4.0;
        let truth: Vec<(f64, f64)> = (0..30).map(|i| (0.3 + i as f64 * 2.0 * s16, -20.0)).collect();
        let x = synth(sr, 0.3 + 60.0 * s16 + 0.5, &truth, 5);
        let p = DetectParams { sample_rate: sr, sixteenth: s16, dynamic_range_db: 45.0 };
        let mut live = LiveDetector::new(p);
        let mut got = Vec::new();
        for end in (0..x.len()).step_by(800) {
            got.extend(live.update(&x[..end]));
        }
        got.extend(live.update(&x));
        assert_eq!(got.len(), truth.len());
        for (o, (t, _)) in got.iter().zip(&truth) {
            assert!((o.time - t).abs() < 0.0015);
        }
    }
}
