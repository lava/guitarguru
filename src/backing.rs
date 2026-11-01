//! Synthesised backing for a riff: a rock beat, a bass following the riff's roots an
//! octave down, and a second distorted guitar doubling the riff. Plucked strings are
//! Karplus-Strong; drums are swept sines and filtered noise. Rendered once per pass of
//! the riff and looped by the output.

use crate::generator::Rng;
use crate::packs::Part;
use crate::patterns::{SessionPlan, beat_seconds};

fn hz(midi: f64) -> f64 {
    440.0 * 2f64.powf((midi - 69.0) / 12.0)
}

/// One-pole low-pass coefficient for cutoff `fc`.
fn lp_coef(fc: f64, sr: f64) -> f32 {
    (1.0 - (-std::f64::consts::TAU * fc / sr).exp()) as f32
}

/// A plucked string at `f` Hz for `dur` seconds into `out` from `start` (Karplus-Strong;
/// `bright` 0..1 shapes the pluck, `damp` < 1 shortens the decay).
#[allow(clippy::too_many_arguments)]
fn pluck(out: &mut [f32], start: usize, f: f64, dur: f64, sr: f64, amp: f32, bright: f32, damp: f32, rng: &mut Rng) {
    let n = ((sr / f).round() as usize).max(2);
    let mut buf: Vec<f32> = (0..n).map(|_| rng.unit() as f32 * 2.0 - 1.0).collect();
    // A darker pluck: low-pass the initial noise.
    let mut s = 0.0;
    for v in &mut buf {
        s += bright * (*v - s);
        *v = s;
    }
    let len = (dur * sr) as usize;
    let fade = (0.012 * sr) as usize;
    let mut i = 0;
    for k in 0..len {
        let Some(o) = out.get_mut(start + k) else { break };
        let j = (i + 1) % n;
        let y = 0.5 * (buf[i] + buf[j]) * damp;
        buf[i] = y;
        i = j;
        let env = if k + fade > len { (len - k) as f32 / fade as f32 } else { 1.0 };
        *o += amp * y * env;
    }
}

fn kick(out: &mut [f32], start: usize, sr: f64, amp: f32) {
    let len = (0.35 * sr) as usize;
    let mut ph = 0.0f64;
    for k in 0..len {
        let Some(o) = out.get_mut(start + k) else { break };
        let t = k as f64 / sr;
        let f = 45.0 + 110.0 * (-t / 0.03).exp();
        ph += std::f64::consts::TAU * f / sr;
        *o += amp * (ph.sin() * (-t / 0.18).exp()) as f32;
    }
}

fn snare(out: &mut [f32], start: usize, sr: f64, amp: f32, rng: &mut Rng) {
    let len = (0.22 * sr) as usize;
    let (mut lp, mut prev) = (0.0f32, 0.0f32);
    let c = lp_coef(5000.0, sr);
    for k in 0..len {
        let Some(o) = out.get_mut(start + k) else { break };
        let t = k as f64 / sr;
        let noise = rng.unit() as f32 * 2.0 - 1.0;
        lp += c * (noise - lp);
        let hp = lp - prev; // crude high-pass keeps the rattle bright
        prev = lp;
        let tone = (std::f64::consts::TAU * 190.0 * t).sin() * (-t / 0.05).exp();
        *o += amp * (0.9 * hp * 3.0 * (-t / 0.08).exp() as f32 + 0.6 * tone as f32);
    }
}

fn hat(out: &mut [f32], start: usize, sr: f64, amp: f32, rng: &mut Rng) {
    let len = (0.05 * sr) as usize;
    let mut prev = 0.0f32;
    for k in 0..len {
        let Some(o) = out.get_mut(start + k) else { break };
        let t = k as f64 / sr;
        let noise = rng.unit() as f32 * 2.0 - 1.0;
        let hp = noise - prev;
        prev = noise;
        *o += amp * hp * (-t / 0.012).exp() as f32;
    }
}

/// Distortion and a speaker-ish band-pass, in place.
fn amp_sim(x: &mut [f32], sr: f64, drive: f32) {
    let (c1, c2) = (lp_coef(3800.0, sr), lp_coef(3800.0, sr));
    let hp = lp_coef(90.0, sr);
    let (mut l1, mut l2, mut low) = (0.0f32, 0.0f32, 0.0f32);
    for v in x.iter_mut() {
        low += hp * (*v - low);
        let y = (drive * (*v - low)).tanh();
        l1 += c1 * (y - l1);
        l2 += c2 * (l1 - l2);
        *v = l2;
    }
}

/// One pass of the backing for `plan` (no count-in), mono at `sr`, peak about 0.8.
pub fn render(plan: &SessionPlan, parts: &[Part], sr: u32) -> Vec<f32> {
    let sr = f64::from(sr);
    let len = (plan.total_seconds() * sr).round() as usize;
    let mut rng = Rng::new(7);
    let mut mix = vec![0.0f32; len];
    let end = plan.total_seconds();
    let dur = |i: usize| plan.notes.get(i + 1).map_or(end, |n| n.time) - plan.notes[i].time;
    if parts.contains(&Part::Guitar) {
        let mut g = vec![0.0f32; len];
        for (i, n) in plan.notes.iter().enumerate() {
            let at = (n.time * sr) as usize;
            let d = if n.pm { dur(i).min(0.16) } else { dur(i) };
            let damp = if n.pm { 0.985 } else { 0.998 };
            let root = f64::from(n.midi);
            pluck(&mut g, at, hz(root), d, sr, 0.5, 0.8, damp, &mut rng);
            if n.fretting.power {
                pluck(&mut g, at + 30, hz(root + 7.0), d, sr, 0.45, 0.8, damp, &mut rng);
                pluck(&mut g, at + 60, hz(root + 12.0), d, sr, 0.3, 0.8, damp, &mut rng);
            }
        }
        amp_sim(&mut g, sr, 9.0);
        mix.iter_mut().zip(&g).for_each(|(m, v)| *m += 0.32 * v);
    }
    if parts.contains(&Part::Bass) {
        let mut b = vec![0.0f32; len];
        for (i, n) in plan.notes.iter().enumerate() {
            let at = (n.time * sr) as usize;
            let d = if n.pm { dur(i).min(0.22) } else { dur(i) };
            pluck(&mut b, at, hz(f64::from(n.midi) - 12.0), d, sr, 0.9, 0.3, 0.9995, &mut rng);
        }
        let c = lp_coef(900.0, sr);
        let mut l = 0.0f32;
        for (m, v) in mix.iter_mut().zip(&b) {
            l += c * ((1.5 * v).tanh() - l);
            *m += 0.55 * l;
        }
    }
    if parts.contains(&Part::Drums) {
        let beat = beat_seconds(plan.bpm);
        let beats = (end / beat).round() as usize;
        for k in 0..beats {
            let at = (k as f64 * beat * sr) as usize;
            let half = (((k as f64 + 0.5) * beat) * sr) as usize;
            match k % 4 {
                0 | 2 => kick(&mut mix, at, sr, 0.8),
                _ => snare(&mut mix, at, sr, 0.45, &mut rng),
            }
            hat(&mut mix, at, sr, 0.18, &mut rng);
            hat(&mut mix, half, sr, 0.12, &mut rng);
        }
    }
    let peak = mix.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak > 0.0 {
        let g = 0.8 / peak;
        mix.iter_mut().for_each(|v| *v *= g);
    }
    mix
}
