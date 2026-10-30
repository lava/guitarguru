//! Monitoring DSP ("amp sound"): input gain -> noise gate -> amp (NAM or fallback drive)
//! -> cab IR (uniformly partitioned FFT convolution) -> room -> output level.
//! Fixed sound presets: heavy (the Soldano capture into a V30 cab), a classic rock crunch
//! (the Soldano capture driven much softer) and a clean studio sound (no amp model:
//! compression, a touch of saturation, a warm cab and a small room), plus mute for players
//! with their own amp on a splitter (no monitoring, nothing loaded). The presets'
//! output trims match their loudness (see the `preset_loudness` test).
//! Everything allocates on construction only; `process` is allocation-free.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ASSET_DIR: &str = "tones";

/// The guitar sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoundPreset {
    /// High gain.
    #[default]
    Heavy,
    /// Classic rock'n'roll crunch.
    Crunch,
    /// Clean studio sound.
    Clean,
    /// No monitoring at all (the player hears their own amp); the DI is still recorded.
    Mute,
}

impl SoundPreset {
    pub const ALL: [SoundPreset; 4] = [SoundPreset::Heavy, SoundPreset::Crunch, SoundPreset::Clean, SoundPreset::Mute];
    pub fn name(self) -> &'static str {
        match self {
            SoundPreset::Heavy => "Heavy",
            SoundPreset::Crunch => "Crunch",
            SoundPreset::Clean => "Clean",
            SoundPreset::Mute => "Mute",
        }
    }
    /// Does this preset play the guitar through the monitoring chain?
    pub fn monitors(self) -> bool {
        self != SoundPreset::Mute
    }

    /// What the preset is built from (None for mute).
    fn recipe(self) -> Option<Recipe> {
        const SOLDANO: &str = "Lum-Soldano-SLO100-3.nam";
        const V30: &str = "1_Cookie_Monster.wav";
        match self {
            SoundPreset::Heavy => Some(Recipe { amp: RecipeAmp::Nam(SOLDANO), cab: V30, drive_db: 0.0, gate_offset_db: 0.0, trim_db: 0.0, room: false }),
            SoundPreset::Crunch => Some(Recipe { amp: RecipeAmp::Nam(SOLDANO), cab: V30, drive_db: -18.0, gate_offset_db: -6.0, trim_db: 5.0, room: false }),
            // A darker, mid-forward cab (DV-77).
            SoundPreset::Clean => {
                Some(Recipe { amp: RecipeAmp::Studio, cab: "2_Darth_Genocider.wav", drive_db: 0.0, gate_offset_db: -15.0, trim_db: 6.0, room: true })
            }
            SoundPreset::Mute => None,
        }
    }
}

enum RecipeAmp {
    /// A `.nam` file in the asset directory (the built-in drive if it fails to load).
    Nam(&'static str),
    /// No model: compressor and light saturation.
    Studio,
}

/// A preset's chain: amp, cab IR (in the asset directory), the amp's input drive, the gate
/// relative to the setting, the output trim that matches heavy's loudness (measured on DI
/// takes, see `preset_loudness`), and a small room.
struct Recipe {
    amp: RecipeAmp,
    cab: &'static str,
    drive_db: f32,
    gate_offset_db: f32,
    trim_db: f32,
    room: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneSettings {
    pub preset: SoundPreset,
    pub asset_dir: PathBuf,
    pub input_gain_db: f32,
    pub gate_db: f32,
    pub output_db: f32,
}

impl Default for ToneSettings {
    fn default() -> Self {
        Self {
            preset: SoundPreset::Heavy,
            asset_dir: PathBuf::from(DEFAULT_ASSET_DIR),
            input_gain_db: 3.0,
            gate_db: -55.0,
            output_db: -6.0,
        }
    }
}

fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// Simple noise gate on the input with ~1 ms attack, 50 ms hold, 30 ms release.
struct Gate {
    threshold: f32,
    env: f32,
    gain: f32,
    hold: u32,
    hold_len: u32,
    env_coef: f32,
    rel_coef: f32,
    att_coef: f32,
}

impl Gate {
    fn new(threshold_db: f32, sr: f32) -> Self {
        Self {
            threshold: db(threshold_db),
            env: 0.0,
            gain: 0.0,
            hold: 0,
            hold_len: (0.05 * sr) as u32,
            env_coef: (-1.0 / (0.005 * sr)).exp(),
            rel_coef: (-1.0 / (0.03 * sr)).exp(),
            att_coef: (-1.0 / (0.001 * sr)).exp(),
        }
    }
    fn process(&mut self, x: &mut [f32]) {
        for v in x {
            let a = v.abs();
            self.env = if a > self.env { a } else { self.env * self.env_coef };
            let open = if self.env > self.threshold {
                self.hold = self.hold_len;
                true
            } else if self.hold > 0 {
                self.hold -= 1;
                true
            } else {
                false
            };
            let target = if open { 1.0 } else { 0.0 };
            let c = if open { self.att_coef } else { self.rel_coef };
            self.gain = target + (self.gain - target) * c;
            *v *= self.gain;
        }
    }
}

/// Fallback amp: tight low cut, mid push, hard-ish tanh saturation, high cut.
struct Drive {
    hp_x: f32,
    hp_y: f32,
    hp_a: f32,
    lp1: f32,
    lp2: f32,
    lp_a: f32,
}

impl Drive {
    fn new(sr: f32) -> Self {
        let hp_a = (-2.0 * std::f32::consts::PI * 300.0 / sr).exp();
        let lp_a = (-2.0 * std::f32::consts::PI * 5000.0 / sr).exp();
        Self { hp_x: 0.0, hp_y: 0.0, hp_a, lp1: 0.0, lp2: 0.0, lp_a }
    }
    fn process(&mut self, x: &mut [f32]) {
        for v in x {
            let hp = self.hp_a * (self.hp_y + *v - self.hp_x);
            self.hp_x = *v;
            self.hp_y = hp;
            let s = (hp * 40.0).tanh();
            let s2 = (1.5 * s + 0.3 * s * s).tanh(); // some asymmetry
            self.lp1 = s2 + (self.lp1 - s2) * self.lp_a;
            self.lp2 = self.lp1 + (self.lp2 - self.lp1) * self.lp_a;
            *v = 0.3 * self.lp2;
        }
    }
}

/// One-pole high-pass.
struct HighPass {
    x: f32,
    y: f32,
    a: f32,
}

impl HighPass {
    fn new(hz: f32, sr: f32) -> Self {
        Self { x: 0.0, y: 0.0, a: (-2.0 * std::f32::consts::PI * hz / sr).exp() }
    }
    fn tick(&mut self, v: f32) -> f32 {
        self.y = self.a * (self.y + v - self.x);
        self.x = v;
        self.y
    }
}

/// Clean studio "amp": low cut, a feed-forward compressor (4:1 above -38 dBFS, 4 ms
/// attack, 150 ms release, make-up gain), then a light tube-ish saturation.
struct Studio {
    hp1: HighPass,
    hp2: HighPass,
    env: f32,
    att: f32,
    rel: f32,
}

impl Studio {
    const THRESHOLD_DB: f32 = -38.0;
    const RATIO: f32 = 6.0;
    const MAKEUP_DB: f32 = 20.0;
    fn new(sr: f32) -> Self {
        Self {
            hp1: HighPass::new(70.0, sr),
            hp2: HighPass::new(70.0, sr),
            env: 0.0,
            att: (-1.0 / (0.001 * sr)).exp(),
            rel: (-1.0 / (0.150 * sr)).exp(),
        }
    }
    fn process(&mut self, x: &mut [f32]) {
        for v in x {
            let h = self.hp2.tick(self.hp1.tick(*v));
            let a = h.abs();
            let c = if a > self.env { self.att } else { self.rel };
            self.env = a + (self.env - a) * c;
            let lvl = 20.0 * (self.env + 1e-9).log10();
            let over = (lvl - Self::THRESHOLD_DB).max(0.0);
            let gain_db = -over * (1.0 - 1.0 / Self::RATIO) + Self::MAKEUP_DB;
            let y = h * db(gain_db);
            // Soft, slightly asymmetric saturation; transparent at low levels.
            *v = (1.2 * y + 0.08 * y * y).tanh() / 1.2;
        }
    }
}

/// A small room (Schroeder/Freeverb style: four damped combs into two all-passes), mixed
/// in quietly.
struct Room {
    combs: [(Vec<f32>, usize, f32); 4],
    allpasses: [(Vec<f32>, usize); 2],
    feedback: f32,
    damp: f32,
    wet: f32,
}

impl Room {
    fn new(sr: f32, wet: f32) -> Self {
        let n = |len_44k: usize| ((len_44k as f32 * sr / 44100.0) as usize).max(1);
        let comb = |l: usize| (vec![0.0; n(l)], 0usize, 0.0f32);
        Self {
            combs: [comb(1116), comb(1188), comb(1277), comb(1356)],
            allpasses: [(vec![0.0; n(556)], 0), (vec![0.0; n(441)], 0)],
            feedback: 0.72,
            damp: 0.35,
            wet,
        }
    }
    fn process(&mut self, x: &mut [f32]) {
        for v in x {
            let input = *v * 0.25;
            let mut out = 0.0;
            for (buf, pos, lp) in &mut self.combs {
                let y = buf[*pos];
                *lp = y * (1.0 - self.damp) + *lp * self.damp;
                buf[*pos] = input + *lp * self.feedback;
                *pos = (*pos + 1) % buf.len();
                out += y;
            }
            for (buf, pos) in &mut self.allpasses {
                let b = buf[*pos];
                buf[*pos] = out + b * 0.5;
                out = b - out;
                *pos = (*pos + 1) % buf.len();
            }
            *v += self.wet * out;
        }
    }
}

/// Uniformly partitioned overlap-save convolution with block size `B` (adds B samples latency).
pub struct Convolver {
    b: usize,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    ir_parts: Vec<Vec<Complex<f32>>>,
    fdl: Vec<Vec<Complex<f32>>>,
    fdl_pos: usize,
    in_buf: Vec<f32>, // 2B: previous block + current
    fill: usize,
    out_block: Vec<f32>,
    out_pos: usize,
    time: Vec<f32>,
    spec: Vec<Complex<f32>>,
    acc: Vec<Complex<f32>>,
    scratch_f: Vec<Complex<f32>>,
}

impl Convolver {
    pub fn new(ir: &[f32], b: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let n = 2 * b;
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let parts = ir.len().div_ceil(b).max(1);
        let scale = 1.0 / n as f32;
        let mut ir_parts = Vec::with_capacity(parts);
        for p in 0..parts {
            let mut t = vec![0.0f32; n];
            for (i, v) in ir.iter().skip(p * b).take(b).enumerate() {
                t[i] = v * scale;
            }
            let mut s = fwd.make_output_vec();
            fwd.process(&mut t, &mut s).expect("fft");
            ir_parts.push(s);
        }
        let bins = n / 2 + 1;
        Self {
            b,
            fdl: vec![vec![Complex::default(); bins]; parts],
            fdl_pos: 0,
            ir_parts,
            in_buf: vec![0.0; n],
            fill: 0,
            out_block: vec![0.0; b],
            out_pos: 0,
            time: vec![0.0; n],
            spec: vec![Complex::default(); bins],
            acc: vec![Complex::default(); bins],
            scratch_f: vec![Complex::default(); bins],
            fwd,
            inv,
        }
    }

    fn block(&mut self) {
        let b = self.b;
        self.time.copy_from_slice(&self.in_buf);
        let _ = self.fwd.process(&mut self.time, &mut self.spec);
        let parts = self.ir_parts.len();
        self.fdl_pos = (self.fdl_pos + parts - 1) % parts;
        self.fdl[self.fdl_pos].copy_from_slice(&self.spec);
        self.acc.iter_mut().for_each(|c| *c = Complex::default());
        for p in 0..parts {
            let x = &self.fdl[(self.fdl_pos + p) % parts];
            for ((a, xv), hv) in self.acc.iter_mut().zip(x).zip(&self.ir_parts[p]) {
                *a += xv * hv;
            }
        }
        self.scratch_f.copy_from_slice(&self.acc);
        // DC/Nyquist imaginary parts must be zero for the inverse real FFT.
        self.scratch_f[0].im = 0.0;
        let last = self.scratch_f.len() - 1;
        self.scratch_f[last].im = 0.0;
        let _ = self.inv.process(&mut self.scratch_f, &mut self.time);
        self.out_block.copy_from_slice(&self.time[b..]);
        self.in_buf.copy_within(b.., 0);
    }

    pub fn process(&mut self, x: &mut [f32]) {
        let b = self.b;
        for v in x {
            self.in_buf[b + self.fill] = *v;
            self.fill += 1;
            *v = self.out_block[self.out_pos];
            self.out_pos += 1;
            if self.fill == b {
                self.block();
                self.fill = 0;
                self.out_pos = 0;
            }
        }
    }
}

pub fn load_ir(path: &Path, sr: u32, max_len: usize) -> Result<Vec<f32>, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().filter_map(Result::ok).map(|v| v as f32 * scale).collect()
        }
    };
    let mut ir: Vec<f32> = raw.chunks(ch).map(|c| c[0]).collect();
    if spec.sample_rate != sr {
        // Linear resample; good enough for a cab IR in a prototype.
        let ratio = f64::from(spec.sample_rate) / f64::from(sr);
        let n = (ir.len() as f64 / ratio) as usize;
        ir = (0..n)
            .map(|i| {
                let p = i as f64 * ratio;
                let k = p as usize;
                let f = (p - k as f64) as f32;
                ir[k] * (1.0 - f) + ir.get(k + 1).copied().unwrap_or(0.0) * f
            })
            .collect();
    }
    ir.truncate(max_len);
    let fade = (max_len / 8).min(ir.len());
    let n = ir.len();
    for i in 0..fade {
        ir[n - fade + i] *= 1.0 - i as f32 / fade as f32;
    }
    // Normalise to unit energy so different cabs are similarly loud.
    let e: f32 = ir.iter().map(|v| v * v).sum::<f32>().sqrt();
    if e > 0.0 {
        ir.iter_mut().for_each(|v| *v /= e);
    }
    Ok(ir)
}

enum Amp {
    /// Unprocessed (the mute chain, which never runs).
    Clean,
    Drive(Drive),
    /// Model and its output gain; `drive` scales its input.
    Nam(Box<nam_rs::Model>, f32),
    Studio(Studio),
}

pub struct Chain {
    in_gain: f32,
    /// Extra gain into the amp only (crunch drives the model softer).
    drive: f32,
    out_gain: f32,
    gate: Gate,
    amp: Amp,
    cab: Option<Convolver>,
    room: Option<Room>,
    pub description: String,
}

impl Chain {
    /// Build the chain (loads files; call off the audio thread). A model that fails to
    /// load falls back to the built-in drive; errors are reported in `description`.
    pub fn build(s: &ToneSettings, sr: u32) -> Self {
        let srf = sr as f32;
        let Some(r) = s.preset.recipe() else {
            // Never processed (monitoring is off); nothing is loaded.
            return Self {
                in_gain: 1.0,
                drive: 1.0,
                out_gain: 0.0,
                gate: Gate::new(s.gate_db, srf),
                amp: Amp::Clean,
                cab: None,
                room: None,
                description: "muted: no monitoring (your own amp)".into(),
            };
        };
        let mut notes = vec![s.preset.name().to_lowercase()];
        let amp = match r.amp {
            RecipeAmp::Studio => {
                notes.push("compressor, saturation".into());
                Amp::Studio(Studio::new(srf))
            }
            RecipeAmp::Nam(file) => match load_nam(&s.asset_dir.join(file), sr) {
                Ok((m, gain)) => {
                    notes.push(format!("NAM {file}"));
                    Amp::Nam(Box::new(m), gain)
                }
                Err(e) => {
                    notes.push(format!("NAM failed ({e}); using built-in drive"));
                    Amp::Drive(Drive::new(srf))
                }
            },
        };
        if r.drive_db != 0.0 {
            notes.push(format!("drive {:+.0} dB", r.drive_db));
        }
        let cab = match load_ir(&s.asset_dir.join(r.cab), sr, 4096) {
            Ok(ir) => {
                notes.push(format!("cab {}", r.cab));
                Some(Convolver::new(&ir, 64))
            }
            Err(e) => {
                notes.push(format!("cab failed: {e}"));
                None
            }
        };
        if r.room {
            notes.push("small room".into());
        }
        Self {
            in_gain: db(s.input_gain_db),
            drive: db(r.drive_db),
            out_gain: db(s.output_db + r.trim_db),
            gate: Gate::new(s.gate_db + r.gate_offset_db, srf),
            amp,
            cab,
            room: r.room.then(|| Room::new(srf, 0.18)),
            description: notes.join(", "),
        }
    }

    pub fn process(&mut self, x: &mut [f32]) {
        for v in x.iter_mut() {
            *v *= self.in_gain;
        }
        self.gate.process(x);
        match &mut self.amp {
            Amp::Clean => {}
            Amp::Drive(d) => d.process(x),
            Amp::Studio(st) => st.process(x),
            Amp::Nam(m, g) => {
                if self.drive != 1.0 {
                    for v in x.iter_mut() {
                        *v *= self.drive;
                    }
                }
                m.process_buffer(x);
                for v in x.iter_mut() {
                    *v *= *g;
                }
            }
        }
        if let Some(c) = &mut self.cab {
            c.process(x);
        }
        if let Some(r) = &mut self.room {
            r.process(x);
        }
        for v in x.iter_mut() {
            *v = soft_limit(*v * self.out_gain);
        }
    }
}

/// Linear up to 0.6 (-4.4 dBFS), then a smooth knee that never exceeds 1.0: stray peaks
/// round off instead of clipping hard.
fn soft_limit(v: f32) -> f32 {
    const K: f32 = 0.6;
    let a = v.abs();
    if a <= K { v } else { v.signum() * (K + (1.0 - K) * ((a - K) / (1.0 - K)).tanh()) }
}

fn load_nam(path: &Path, sr: u32) -> Result<(nam_rs::Model, f32), String> {
    let model = nam_rs::NamModel::from_file(path).map_err(|e| e.to_string())?;
    let want = model.expected_sample_rate();
    if (want - f64::from(sr)).abs() > 1.0 {
        return Err(format!("model wants {want} Hz, stream runs at {sr} Hz"));
    }
    // Normalise loudness to -18 dBFS like the NAM plugin.
    let gain = model.loudness().map_or(1.0, |l| db(-18.0 - l));
    let m = nam_rs::Model::from_nam(&model).map_err(|e| e.to_string())?;
    Ok((m, gain))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convolver_matches_direct() {
        let ir: Vec<f32> = (0..300).map(|i| ((i * 7919) % 13) as f32 / 13.0 - 0.5).collect();
        let x: Vec<f32> = (0..2000).map(|i| ((i * 104729) % 17) as f32 / 17.0 - 0.5).collect();
        let mut c = Convolver::new(&ir, 64);
        let mut y = x.clone();
        // irregular block sizes
        let mut pos = 0;
        for n in [5usize, 100, 37, 300, 1].iter().cycle() {
            if pos >= y.len() {
                break;
            }
            let e = (pos + n).min(y.len());
            c.process(&mut y[pos..e]);
            pos = e;
        }
        for i in 64..2000 {
            let k = i - 64; // latency B
            let d: f32 = (0..ir.len()).filter(|j| *j <= k).map(|j| ir[j] * x[k - j]).sum();
            assert!((y[i] - d).abs() < 1e-3, "{i}: {} vs {d}", y[i]);
        }
    }

    #[test]
    fn presets_load_and_stay_bounded() {
        let old: ToneSettings = serde_json::from_str(r#"{"amp":{"Nam":"x.nam"},"cab":null}"#).unwrap();
        assert_eq!(old.preset, SoundPreset::Heavy, "configs from before presets");
        // Configs with the removed amp, cab and monitoring keys still load.
        let cfg: crate::config::Config = serde_json::from_str(r#"{"audio":{"monitor":false,"buffer_frames":256},"tone":{"amp":"Clean","cab":"8_Big_Bubba.wav","preset":"Crunch","gate_db":-50.0}}"#).unwrap();
        assert_eq!((cfg.audio.buffer_frames, cfg.tone.preset, cfg.tone.gate_db), (256, SoundPreset::Crunch, -50.0));
        let back: ToneSettings = serde_json::from_str(&serde_json::to_string(&ToneSettings { preset: SoundPreset::Clean, ..old }).unwrap()).unwrap();
        assert_eq!(back.preset, SoundPreset::Clean);
        assert_eq!(soft_limit(0.5), 0.5);
        assert!(soft_limit(5.0) <= 1.0 && soft_limit(-5.0) >= -1.0 && soft_limit(0.7) > 0.6);
        // Clean without any asset files: compressor, saturation, room; no cab.
        let s = ToneSettings { preset: SoundPreset::Clean, asset_dir: PathBuf::from("/nonexistent"), ..ToneSettings::default() };
        let mut c = Chain::build(&s, 48000);
        let mut x: Vec<f32> = (0..48000).map(|i| 0.3 * (i as f32 * 0.02).sin() * (-(i % 12000) as f32 / 3000.0).exp()).collect();
        for b in x.chunks_mut(128) {
            c.process(b);
        }
        assert!(x.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
        assert!(x.iter().any(|v| v.abs() > 0.05), "makes sound");
        assert!(c.description.contains("small room"), "{}", c.description);
        // Mute loads nothing and is never monitored.
        let m = Chain::build(&ToneSettings { preset: SoundPreset::Mute, ..s }, 48000);
        assert!(matches!(m.amp, Amp::Clean) && m.cab.is_none() && !SoundPreset::Mute.monitors());
    }

    /// Integrated loudness (BS.1770 K-weighting at 48 kHz, 400 ms blocks, absolute and
    /// relative gates), LUFS.
    fn loudness(x: &[f32]) -> f64 {
        let biquad = |x: &[f64], b: [f64; 3], a: [f64; 2]| {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            x.iter()
                .map(|&v| {
                    let y = b[0] * v + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
                    (x2, x1, y2, y1) = (x1, v, y1, y);
                    y
                })
                .collect::<Vec<f64>>()
        };
        let v: Vec<f64> = x.iter().map(|&s| f64::from(s)).collect();
        let v = biquad(&v, [1.53512485958697, -2.69169618940638, 1.19839281085285], [-1.69065929318241, 0.73248077421585]);
        let v = biquad(&v, [1.0, -2.0, 1.0], [-1.99004745483398, 0.99007225036621]);
        let (blk, hop) = (19200, 4800);
        let ms: Vec<f64> = (0..v.len().saturating_sub(blk) / hop).map(|i| v[i * hop..i * hop + blk].iter().map(|s| s * s).sum::<f64>() / blk as f64).collect();
        let l = |m: f64| -0.691 + 10.0 * m.log10();
        let abs: Vec<f64> = ms.into_iter().filter(|&m| l(m) > -70.0).collect();
        let rel = l(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
        let g: Vec<f64> = abs.into_iter().filter(|&m| l(m) > rel).collect();
        l(g.iter().sum::<f64>() / g.len() as f64)
    }

    fn render(s: &ToneSettings, di: &[f32]) -> Vec<f32> {
        let mut chain = Chain::build(s, 48000);
        let mut y = di.to_vec();
        for b in y.chunks_mut(128) {
            chain.process(b);
        }
        y
    }

    /// Loudness of each preset over DI takes (`GG_DI_WAVS`, colon separated), relative to
    /// heavy, and a crunch drive sweep (loudness and THD of a 110 Hz sine at the takes'
    /// typical attack level). Renders go to `GG_RENDER_DIR` if set.
    #[test]
    #[ignore = "needs DI takes and the NAMix asset directory; prints preset loudness"]
    fn preset_loudness() {
        let paths = std::env::var("GG_DI_WAVS").expect("GG_DI_WAVS");
        let mut di = Vec::new();
        for p in paths.split(':') {
            let mut r = hound::WavReader::open(p).unwrap();
            let sp = r.spec();
            assert_eq!(sp.sample_rate, 48000);
            let v: Vec<f32> = match sp.sample_format {
                hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
                hound::SampleFormat::Int => {
                    let k = 1.0 / (1i64 << (sp.bits_per_sample - 1)) as f32;
                    r.samples::<i32>().map(|v| v.unwrap() as f32 * k).collect()
                }
            };
            di.extend(v.chunks(sp.channels as usize).map(|c| c[0]));
        }
        if let Some(sec) = std::env::var("GG_DI_SECONDS").ok().and_then(|v| v.parse::<usize>().ok()) {
            di.truncate(sec * 48000);
        }
        let mut peaks: Vec<f32> = di.chunks(4800).map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs()))).collect();
        peaks.sort_by(f32::total_cmp);
        let p90 = peaks[peaks.len() * 9 / 10];
        println!("DI: {:.0} s, 100 ms peaks p90 {:.1} dBFS, loudness {:.1} LUFS", di.len() as f64 / 48000.0, 20.0 * p90.log10(), loudness(&di));
        let base = ToneSettings::default();
        let heavy = loudness(&render(&base, &di));
        for preset in SoundPreset::ALL.into_iter().filter(|p| p.monitors()) {
            let s = ToneSettings { preset, ..base.clone() };
            let y = render(&s, &di);
            let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let l = loudness(&y);
            println!("{:7} {:+.1} LU vs heavy ({l:.1} LUFS), peak {:.1} dBFS, peak to loudness {:.1} dB", preset.name(), l - heavy, 20.0 * peak.log10(), 20.0 * f64::from(peak).log10() - l);
            if let Ok(dir) = std::env::var("GG_RENDER_DIR") {
                let spec = hound::WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
                let mut w = hound::WavWriter::create(format!("{dir}/{}.wav", preset.name().to_lowercase()), spec).unwrap();
                y.iter().take(48000 * 30).for_each(|v| w.write_sample(*v).unwrap());
                w.finalize().unwrap();
            }
        }
        if std::env::var_os("GG_SWEEP").is_none() {
            return;
        }
        // Crunch drive sweep, amp only (no cab), on a sine at the attack level after the
        // input gain.
        let amp_in = p90 * db(base.input_gain_db);
        for drive in [0.0f32, -6.0, -10.0, -13.0, -16.0, -19.0, -22.0, -26.0] {
            let (mut model, g) = load_nam(&base.asset_dir.join("Lum-Soldano-SLO100-3.nam"), 48000).unwrap();
            let n = 48000;
            let mut x: Vec<f32> = (0..n).map(|i| amp_in * db(drive) * (std::f32::consts::TAU * 110.0 * i as f32 / 48000.0).sin()).collect();
            for b in x.chunks_mut(128) {
                model.process_buffer(b);
            }
            let y: Vec<f64> = x[n / 2..].iter().map(|v| f64::from(*v * g)).collect();
            let (mut sc, mut cc) = (0.0, 0.0);
            for (i, v) in y.iter().enumerate() {
                let ph = std::f64::consts::TAU * 110.0 * (i + n / 2) as f64 / 48000.0;
                sc += v * ph.sin();
                cc += v * ph.cos();
            }
            let (a, b) = (2.0 * sc / y.len() as f64, 2.0 * cc / y.len() as f64);
            let mean = y.iter().sum::<f64>() / y.len() as f64;
            let total: f64 = y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / y.len() as f64;
            let fund = (a * a + b * b) / 2.0;
            let thd = ((total - fund).max(0.0) / fund).sqrt();
            let s = ToneSettings { preset: SoundPreset::Crunch, ..base.clone() };
            let mut chain = Chain::build(&s, 48000);
            chain.drive = db(drive);
            let mut yy = di.clone();
            for b in yy.chunks_mut(128) {
                chain.process(b);
            }
            println!("drive {drive:+5.0} dB: THD {:5.1} %, {:+.1} LU vs heavy", thd * 100.0, loudness(&yy) - heavy);
        }
    }

    #[test]
    #[ignore = "needs the NAMix asset directory; prints NAM CPU cost"]
    fn nam_cpu_cost() {
        let s = ToneSettings::default();
        let mut chain = Chain::build(&s, 48000);
        println!("{}", chain.description);
        let mut buf = vec![0.0f32; 128];
        let blocks = 48000 * 5 / 128;
        let t = std::time::Instant::now();
        for k in 0..blocks {
            for (i, v) in buf.iter_mut().enumerate() {
                *v = 0.1 * ((k * 128 + i) as f32 * 0.01).sin();
            }
            chain.process(&mut buf);
        }
        let el = t.elapsed().as_secs_f64();
        println!("5 s of audio in {el:.3} s = {:.1} % of one core", el / 5.0 * 100.0);
    }
}
