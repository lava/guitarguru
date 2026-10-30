//! Monitoring DSP ("amp sound"): input gain -> noise gate -> amp (NAM or fallback drive)
//! -> cab IR (uniformly partitioned FFT convolution) -> output level.
//! Everything allocates on construction only; `process` is allocation-free.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ASSET_DIR: &str = "tones";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AmpChoice {
    /// Plain DI.
    Clean,
    /// Built-in drive (pre-emphasis + tanh), the fallback without NAM.
    Drive,
    /// A `.nam` file name inside the asset directory.
    Nam(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneSettings {
    pub asset_dir: PathBuf,
    pub amp: AmpChoice,
    /// IR file name inside the asset directory, or None.
    pub cab: Option<String>,
    pub input_gain_db: f32,
    pub gate_db: f32,
    pub output_db: f32,
}

impl Default for ToneSettings {
    fn default() -> Self {
        Self {
            asset_dir: PathBuf::from(DEFAULT_ASSET_DIR),
            amp: AmpChoice::Nam("Lum-Soldano-SLO100-3.nam".into()),
            cab: Some("1_Cookie_Monster.wav".into()),
            input_gain_db: 3.0,
            gate_db: -55.0,
            output_db: -6.0,
        }
    }
}

pub fn list_assets(dir: &Path, ext: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.to_lowercase().ends_with(ext))
                .collect()
        })
        .unwrap_or_default();
    // Jester IRs are "N_Name.wav": sort numerically.
    v.sort_by_key(|n| (n.split('_').next().and_then(|p| p.parse::<u32>().ok()).unwrap_or(u32::MAX), n.clone()));
    v
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
    Clean,
    Drive(Drive),
    Nam(Box<nam_rs::Model>, f32),
}

pub struct Chain {
    in_gain: f32,
    out_gain: f32,
    gate: Gate,
    amp: Amp,
    cab: Option<Convolver>,
    pub description: String,
}

impl Chain {
    /// Build the chain (loads files; call off the audio thread). Errors fall back to
    /// the built-in drive and are reported in `description`.
    pub fn build(s: &ToneSettings, sr: u32) -> Self {
        let srf = sr as f32;
        let mut notes = Vec::new();
        let amp = match &s.amp {
            AmpChoice::Clean => Amp::Clean,
            AmpChoice::Drive => Amp::Drive(Drive::new(srf)),
            AmpChoice::Nam(file) => match load_nam(&s.asset_dir.join(file), sr) {
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
        if matches!(s.amp, AmpChoice::Drive) {
            notes.push("built-in drive".into());
        } else if matches!(s.amp, AmpChoice::Clean) {
            notes.push("clean DI".into());
        }
        let cab = s.cab.as_ref().and_then(|f| match load_ir(&s.asset_dir.join(f), sr, 4096) {
            Ok(ir) => {
                notes.push(format!("cab {f}"));
                Some(Convolver::new(&ir, 64))
            }
            Err(e) => {
                notes.push(format!("cab failed: {e}"));
                None
            }
        });
        Self {
            in_gain: db(s.input_gain_db),
            out_gain: db(s.output_db),
            gate: Gate::new(s.gate_db, srf),
            amp,
            cab,
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
            Amp::Nam(m, g) => {
                m.process_buffer(x);
                for v in x.iter_mut() {
                    *v *= *g;
                }
            }
        }
        if let Some(c) = &mut self.cab {
            c.process(x);
        }
        for v in x.iter_mut() {
            *v = (*v * self.out_gain).clamp(-1.0, 1.0);
        }
    }
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
