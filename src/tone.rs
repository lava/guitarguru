//! Tone consistency: how alike the strokes of one kind sound.
//!
//! Strokes are grouped by pick direction and palm muting (down/up x P.M./open); groups
//! with fewer than `MIN_GROUP` matched strokes are skipped. For every stroke the amplitude
//! envelope (10 ms RMS, about one period of the low E, in 1 ms steps, dB) is taken from
//! its onset up to `END_MARGIN_MS` before the next detected attack, or `CAP_MS`. Per stroke:
//! - volume     = attack peak |x| over the first 30 ms (the onset's level), dB
//! - attack     = envelope peak (first 30 ms) minus its mean over 40..70 ms, dB: how much
//!                the pick transient stands out of the note
//! - sustain    = envelope peak minus its mean over the 20 ms before the group's horizon,
//!                dB: how much the note has died away (muted, choked or ringing). The
//!                horizon is the group's 20th-percentile free gap (time to the next attack
//!                minus `SUSTAIN_MARGIN_MS`, capped), so most strokes are compared over
//!                the same span; strokes cut short by the next note (or damped by the
//!                approaching pick) before the horizon have no sustain value
//! - brightness = spectral centroid of the first 30 ms, octaves
//! Different notes (frets, strings) sound different on their own, so each feature has
//! the mean of its (group, note) removed when that note has at least 3 strokes.
//! Envelope match: per stroke, the RMS difference (dB) of its peak-normalised envelope to
//! the group's median one, over the stroke's own window; the readout is the group mean.
//!
//! Scores (linear, clamped 0..100 %; calibrated on the real takes, see `Feature::pct`):
//! volume sd 2 -> 100 %, 7 dB -> 0 %; attack sd 1.5 -> 100 %, 6 dB -> 0 %; sustain sd
//! 2.5 -> 100 %, 9 dB -> 0 %; brightness sd 0.15 -> 100 %, 0.6 oct -> 0 %. A group's score
//! is 0.4 volume + 0.2 attack + 0.2 sustain + 0.2 brightness (missing features drop out
//! of the weights); tone % is the count-weighted mean over the groups.

use serde::{Deserialize, Serialize};

use crate::dsp::{mean, median, std};

/// Smallest group that gets judged.
pub const MIN_GROUP: usize = 8;
/// Longest envelope window, ms.
pub const CAP_MS: usize = 400;
/// Shortest window that still gets an attack value (needs 40..70 ms), ms.
const ATTACK_MIN_MS: usize = 72;
const RMS_MS: f64 = 10.0;
/// The pick (or a muting hand) damps a ringing string well before the next attack is
/// detected; sustain is only read this long before the next attack, ms.
const SUSTAIN_MARGIN_MS: usize = 60;
/// Envelopes end this long before the next detected attack (the RMS window looks 5 ms
/// ahead and the detector marks an attack at 30 % of its peak), ms.
const END_MARGIN_MS: usize = 12;

/// One matched stroke going in.
#[derive(Clone, Copy, Debug)]
pub struct StrokeIn {
    /// Index into the report's notes.
    pub note: usize,
    pub onset: f64,
    pub level_db: f64,
    pub down: bool,
    pub pm: bool,
    /// MIDI note (with string) of what was fretted, to compare like with like.
    pub midi: u8,
    pub string: u8,
}

/// One stroke's features (raw, before removing the note means).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StrokeTone {
    pub note: usize,
    pub group: usize,
    pub level_db: f64,
    pub attack_db: Option<f64>,
    pub sustain_db: Option<f64>,
    pub bright_oct: Option<f64>,
    /// RMS distance of the peak-normalised envelope to the group's median one, dB.
    pub match_db: Option<f64>,
    /// Envelope, dB, 1 ms steps from the onset (not saved).
    #[serde(skip)]
    pub env: Vec<f32>,
}

/// Which feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Feature {
    Level,
    Attack,
    Sustain,
    Brightness,
}

impl Feature {
    pub const ALL: [Feature; 4] = [Feature::Level, Feature::Attack, Feature::Sustain, Feature::Brightness];
    pub fn name(self) -> &'static str {
        match self {
            Feature::Level => "volume",
            Feature::Attack => "attack",
            Feature::Sustain => "sustain",
            Feature::Brightness => "brightness",
        }
    }
    pub fn weight(self) -> f64 {
        match self {
            Feature::Level => 0.4,
            _ => 0.2,
        }
    }
    /// Score of a spread (dB, or octaves for brightness).
    pub fn pct(self, sd: f64) -> f64 {
        let (good, bad) = match self {
            Feature::Level => (2.0, 7.0),
            Feature::Attack => (1.5, 6.0),
            Feature::Sustain => (2.5, 9.0),
            Feature::Brightness => (0.15, 0.6),
        };
        ((bad - sd) / (bad - good) * 100.0).clamp(0.0, 100.0)
    }
    /// The spread as the UI shows it: "+/-2.1 dB", brightness as "+/-12 %".
    pub fn fmt(self, sd: f64) -> String {
        match self {
            Feature::Brightness => format!("\u{b1}{:.0} %", (2f64.powf(sd) - 1.0) * 100.0),
            _ => format!("\u{b1}{sd:.1} dB"),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToneGroup {
    pub down: bool,
    pub pm: bool,
    pub n: usize,
    /// Spread (sd after removing note means) per feature, in `Feature::ALL` order; None
    /// when fewer than `MIN_GROUP` strokes have the feature.
    pub sd: [Option<f64>; 4],
    /// Mean envelope match, dB.
    pub match_db: Option<f64>,
    /// Sustain horizon, ms.
    pub horizon_ms: usize,
    /// Median envelope (dB, 1 ms steps) and median level of the group.
    pub median_env: Vec<f32>,
    pub median_level_db: f64,
    pub pct: f64,
}

impl ToneGroup {
    pub fn name(&self) -> String {
        format!("{}{}", if self.down { "Downstrokes" } else { "Upstrokes" }, if self.pm { ", P.M." } else { ", open" })
    }
    pub fn sd_of(&self, f: Feature) -> Option<f64> {
        self.sd[f as usize]
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToneReport {
    pub groups: Vec<ToneGroup>,
    pub strokes: Vec<StrokeTone>,
    /// Count-weighted over the groups, percent (None: no group big enough).
    pub pct: Option<f64>,
}

impl ToneReport {
    /// The feature costing the most score over all groups (count-weighted), with the
    /// count-weighted spread.
    pub fn worst(&self) -> Option<(Feature, f64)> {
        let total: usize = self.groups.iter().map(|g| g.n).sum();
        Feature::ALL
            .iter()
            .filter_map(|&f| {
                let v: Vec<(f64, f64)> = self.groups.iter().filter_map(|g| g.sd_of(f).map(|s| (g.n as f64, s))).collect();
                let w: f64 = v.iter().map(|x| x.0).sum();
                (w > 0.0).then(|| {
                    let sd = v.iter().map(|x| x.0 * x.1).sum::<f64>() / w;
                    (f, sd, f.weight() * (100.0 - f.pct(sd)) * w / total.max(1) as f64)
                })
            })
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .filter(|w| w.2 > 0.0)
            .map(|(f, s, _)| (f, s))
    }
}

/// Envelope of `x` from sample `i`, `len` 1 ms steps, dB.
fn envelope(x: &[f32], sr: f64, i: usize, len: usize) -> Vec<f32> {
    let hop = sr / 1000.0;
    let w = (RMS_MS * sr / 1000.0) as usize;
    (0..len)
        .map(|k| {
            let c = i + (k as f64 * hop) as usize;
            let (a, b) = (c.saturating_sub(w / 2), (c + w / 2).min(x.len()));
            let p = if b > a { x[a..b].iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / (b - a) as f64 } else { 0.0 };
            (10.0 * (p + 1e-10).log10()) as f32
        })
        .collect()
}

fn mean_f32(v: &[f32]) -> f64 {
    if v.is_empty() { f64::NAN } else { v.iter().map(|x| f64::from(*x)).sum::<f64>() / v.len() as f64 }
}

fn quantile(v: &[usize], q: f64) -> usize {
    let mut s = v.to_vec();
    s.sort_unstable();
    s.get(((s.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0)
}

/// Residuals of a feature after removing the mean of each note with >= 3 values (the
/// group mean otherwise).
fn residuals(vals: &[(u16, f64)]) -> Vec<f64> {
    let all: Vec<f64> = vals.iter().map(|v| v.1).collect();
    let gm = mean(&all);
    vals.iter()
        .map(|&(k, v)| {
            let same: Vec<f64> = vals.iter().filter(|o| o.0 == k).map(|o| o.1).collect();
            v - if same.len() >= 3 { mean(&same) } else { gm }
        })
        .collect()
}

/// Analyse the matched strokes. `onsets` are all detected attack times (they end each
/// stroke's window).
pub fn analyse(x: &[f32], sr: f64, strokes: &[StrokeIn], onsets: &[f64]) -> ToneReport {
    let mut rep = ToneReport::default();
    let keys = [(true, true), (true, false), (false, true), (false, false)];
    for (down, pm) in keys {
        let members: Vec<&StrokeIn> = strokes.iter().filter(|s| s.down == down && s.pm == pm).collect();
        if members.len() < MIN_GROUP {
            continue;
        }
        let gi = rep.groups.len();
        let mut st: Vec<StrokeTone> = Vec::new();
        let mut wins = Vec::new();
        let mut notes = Vec::new();
        for s in &members {
            let i = (s.onset * sr) as usize;
            if i >= x.len() {
                continue;
            }
            let next = onsets.iter().copied().find(|&t| t > s.onset + 0.03);
            let gap_ms = next.map_or(CAP_MS, |t| (((t - s.onset) * 1e3) as usize).saturating_sub(END_MARGIN_MS));
            let avail = (x.len() - i) * 1000 / sr as usize;
            let win = gap_ms.min(CAP_MS).min(avail);
            if win < 30 {
                continue;
            }
            let env = envelope(x, sr, i, win);
            let pk = env[..30].iter().copied().fold(f32::MIN, f32::max);
            let attack = (win >= ATTACK_MIN_MS).then(|| f64::from(pk) - mean_f32(&env[40..70]));
            let bright = crate::score::centroids(x, sr, &[s.onset]).first().map(|c| c.log2());
            wins.push(gap_ms.min(avail).saturating_sub(SUSTAIN_MARGIN_MS - END_MARGIN_MS).min(win));
            notes.push(u16::from(s.midi) | (u16::from(s.string) << 8));
            st.push(StrokeTone { note: s.note, group: gi, level_db: s.level_db, attack_db: attack, sustain_db: None, bright_oct: bright, match_db: None, env });
        }
        if st.len() < MIN_GROUP {
            continue;
        }
        // Sustain at the group's horizon.
        let horizon = quantile(&wins, 0.2).clamp(ATTACK_MIN_MS, CAP_MS);
        for (t, free) in st.iter_mut().zip(&wins) {
            if *free >= horizon {
                let pk = t.env[..30].iter().copied().fold(f32::MIN, f32::max);
                t.sustain_db = Some(f64::from(pk) - mean_f32(&t.env[horizon - 20..horizon]));
            }
        }
        // Median envelope (absolute) and median shape (peak-normalised), per ms while at
        // least half the group still has data.
        let peak = |e: &[f32]| e[..30].iter().copied().fold(f32::MIN, f32::max);
        let mut median_env = Vec::new();
        let mut median_shape = Vec::new();
        for k in 0..CAP_MS {
            let col: Vec<f64> = st.iter().filter(|t| t.env.len() > k).map(|t| f64::from(t.env[k])).collect();
            if col.len() * 2 < st.len() {
                break;
            }
            let shp: Vec<f64> = st.iter().filter(|t| t.env.len() > k).map(|t| f64::from(t.env[k] - peak(&t.env))).collect();
            median_env.push(median(&col).unwrap_or(0.0) as f32);
            median_shape.push(median(&shp).unwrap_or(0.0) as f32);
        }
        for t in &mut st {
            let p = peak(&t.env);
            let n = t.env.len().min(median_shape.len());
            if n >= 30 {
                let ss: f64 = (0..n).map(|k| f64::from(t.env[k] - p - median_shape[k]).powi(2)).sum();
                t.match_db = Some((ss / n as f64).sqrt());
            }
        }
        let feat = |f: Feature, t: &StrokeTone| match f {
            Feature::Level => Some(t.level_db),
            Feature::Attack => t.attack_db,
            Feature::Sustain => t.sustain_db,
            Feature::Brightness => t.bright_oct,
        };
        let mut sd = [None; 4];
        for f in Feature::ALL {
            let vals: Vec<(u16, f64)> = st.iter().zip(&notes).filter_map(|(t, k)| feat(f, t).map(|v| (*k, v))).collect();
            if vals.len() >= MIN_GROUP {
                sd[f as usize] = Some(std(&residuals(&vals)));
            }
        }
        let matches: Vec<f64> = st.iter().filter_map(|t| t.match_db).collect();
        let levels: Vec<f64> = st.iter().map(|t| t.level_db).collect();
        rep.groups.push(ToneGroup {
            down,
            pm,
            n: st.len(),
            sd,
            match_db: (!matches.is_empty()).then(|| mean(&matches)),
            horizon_ms: horizon,
            median_env,
            median_level_db: median(&levels).unwrap_or(0.0),
            pct: features_pct(&sd).unwrap_or(0.0),
        });
        rep.strokes.extend(st);
    }
    let n: usize = rep.groups.iter().map(|g| g.n).sum();
    if n > 0 {
        rep.pct = Some(rep.groups.iter().map(|g| g.n as f64 * g.pct).sum::<f64>() / n as f64);
    }
    rep
}

/// Score of a set of feature spreads (`Feature::ALL` order): weighted mean of the
/// features present.
pub fn features_pct(sd: &[Option<f64>; 4]) -> Option<f64> {
    let (mut num, mut den) = (0.0, 0.0);
    for f in Feature::ALL {
        if let Some(s) = sd[f as usize] {
            num += f.weight() * f.pct(s);
            den += f.weight();
        }
    }
    (den > 0.0).then(|| num / den)
}

/// The feature costing the most tone score in a set of spreads, with its spread and the
/// cost (share of the tone score lost, 0..1).
pub fn worst_feature(sd: &[Option<f64>; 4]) -> Option<(Feature, f64, f64)> {
    Feature::ALL
        .iter()
        .filter_map(|&f| sd[f as usize].map(|s| (f, s, f.weight() * (100.0 - f.pct(s)) / 100.0)))
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .filter(|w| w.2 > 0.0)
}

/// "sustain +/-4.2 dB": the worst feature of a window, if it costs anything.
pub fn window_label(sd: &[Option<f64>; 4]) -> Option<String> {
    worst_feature(sd).map(|(f, s, _)| format!("{} {}", f.name(), f.fmt(s)))
}

/// Feature spreads of a subset of strokes (e.g. a listen-back window), each feature's
/// values taken relative to its group's mean so groups can be pooled. Returns, per
/// feature, the pooled sd (None with fewer than 4 values).
pub fn window_spreads(rep: &ToneReport, notes: &[usize]) -> [Option<f64>; 4] {
    let mut out = [None; 4];
    for f in Feature::ALL {
        let get = |t: &StrokeTone| match f {
            Feature::Level => Some(t.level_db),
            Feature::Attack => t.attack_db,
            Feature::Sustain => t.sustain_db,
            Feature::Brightness => t.bright_oct,
        };
        let mut resid = Vec::new();
        for (gi, _) in rep.groups.iter().enumerate() {
            let all: Vec<f64> = rep.strokes.iter().filter(|t| t.group == gi).filter_map(get).collect();
            let gm = mean(&all);
            resid.extend(rep.strokes.iter().filter(|t| t.group == gi && notes.contains(&t.note)).filter_map(get).map(|v| v - gm));
        }
        if resid.len() >= 4 {
            out[f as usize] = Some(std(&resid));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decaying tone-plus-noise strokes `(time, level dB, decay dB/s)`; each stops at the next.
    fn synth(sr: f64, secs: f64, hits: &[(f64, f64, f64)]) -> Vec<f32> {
        let mut x = vec![0f32; (secs * sr) as usize];
        let mut seed = 7u64;
        for (h, &(t, lv, dec)) in hits.iter().enumerate() {
            let a = 10f64.powf(lv / 20.0);
            let (i0, i1) = ((t * sr) as usize, hits.get(h + 1).map_or(x.len(), |n| (n.0 * sr) as usize));
            for (k, v) in x[i0..i1].iter_mut().enumerate() {
                let tt = k as f64 / sr;
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let n = (seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
                let tone = (std::f64::consts::TAU * 110.0 * tt).sin() * 0.7 + n * 0.3;
                *v += (a * 10f64.powf(-dec * tt / 20.0) * tone) as f32;
            }
        }
        x
    }

    #[test]
    fn steady_strokes_score_high_and_sloppy_decay_is_named() {
        let sr = 48000.0;
        let mut hits = Vec::new();
        let mut strokes = Vec::new();
        for i in 0..40 {
            let t = 0.5 + i as f64 * 0.375;
            let down = i % 2 == 0;
            // Upstrokes: every other one choked (fast decay).
            let dec = if !down && i % 4 == 1 { 120.0 } else { 20.0 };
            hits.push((t, -20.0, dec));
            strokes.push(StrokeIn { note: i, onset: t, level_db: -20.0, down, pm: false, midi: 40, string: 6 });
        }
        let x = synth(sr, 16.0, &hits);
        let onsets: Vec<f64> = hits.iter().map(|h| h.0).collect();
        let r = analyse(&x, sr, &strokes, &onsets);
        assert_eq!(r.groups.len(), 2);
        let down = r.groups.iter().find(|g| g.down).unwrap();
        let up = r.groups.iter().find(|g| !g.down).unwrap();
        assert!(down.pct > 95.0, "{down:?}");
        assert!(up.sd_of(Feature::Sustain).unwrap() > 10.0, "{:?}", up.sd);
        assert_eq!(worst_feature(&up.sd).map(|w| w.0), Some(Feature::Sustain));
        assert!(up.match_db.unwrap() > 2.0 * down.match_db.unwrap());
        assert_eq!(r.worst().map(|w| w.0), Some(Feature::Sustain));
        // Too-small groups are skipped.
        let r = analyse(&x, sr, &strokes[..10], &onsets);
        assert!(r.groups.is_empty() && r.pct.is_none());
    }
}
