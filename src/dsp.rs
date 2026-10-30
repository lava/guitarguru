//! Small DSP helpers shared by the detector: Butterworth high-pass (as biquad sections),
//! zero-phase filtering, moving RMS, and a port of `scipy.signal.find_peaks`.

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    /// RBJ high-pass (bilinear transform with pre-warping, like scipy's `butter`).
    pub fn highpass(fc: f64, q: f64, sr: f64) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * fc / sr;
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + c) / 2.0 / a0,
            b1: -(1.0 + c) / a0,
            b2: (1.0 + c) / 2.0 / a0,
            a1: -2.0 * c / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    /// Process in place (direct form II transposed), starting from rest.
    pub fn run(&self, x: &mut [f64]) {
        let (mut z1, mut z2) = (0.0, 0.0);
        for v in x.iter_mut() {
            let i = *v;
            let o = self.b0 * i + z1;
            z1 = self.b1 * i - self.a1 * o + z2;
            z2 = self.b2 * i - self.a2 * o;
            *v = o;
        }
    }
}

/// Butterworth high-pass of even `order` as cascaded biquads.
pub fn butter_highpass(order: usize, fc: f64, sr: f64) -> Vec<Biquad> {
    let n = order as f64;
    (0..order / 2)
        .map(|k| {
            let theta = std::f64::consts::PI * (2.0 * k as f64 + 1.0) / (2.0 * n);
            let q = 1.0 / (2.0 * theta.sin());
            Biquad::highpass(fc, q, sr)
        })
        .collect()
}

/// Zero-phase filtering (forward then backward), with odd-extension padding like scipy.
pub fn filtfilt(sections: &[Biquad], x: &[f32]) -> Vec<f64> {
    if x.is_empty() {
        return Vec::new();
    }
    // Longer than scipy's default pad, to let the edge transient die out.
    let pad = 1000.min(x.len() - 1);
    let first = f64::from(x[0]);
    let last = f64::from(x[x.len() - 1]);
    let mut y: Vec<f64> = Vec::with_capacity(x.len() + 2 * pad);
    for i in (1..=pad).rev() {
        y.push(2.0 * first - f64::from(x[i]));
    }
    y.extend(x.iter().map(|&v| f64::from(v)));
    for i in 1..=pad {
        y.push(2.0 * last - f64::from(x[x.len() - 1 - i]));
    }
    for s in sections {
        s.run(&mut y);
    }
    y.reverse();
    for s in sections {
        s.run(&mut y);
    }
    y.reverse();
    y.drain(..pad);
    y.truncate(x.len());
    y
}

/// Centered moving RMS with window `w` (like `np.convolve(x*x, ones(w)/w, 'same')`).
pub fn moving_rms(x: &[f64], w: usize) -> Vec<f64> {
    let w = w.max(1);
    let n = x.len();
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0);
    let mut acc = 0.0;
    for v in x {
        acc += v * v;
        prefix.push(acc);
    }
    let left = (w - 1) / 2;
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(w - 1 - left);
            let hi = (i + left + 1).min(n);
            ((prefix[hi] - prefix[lo]) / w as f64).max(0.0).sqrt()
        })
        .collect()
}

/// Port of `scipy.signal.find_peaks(x, height, distance, prominence)`.
pub fn find_peaks(x: &[f64], height: f64, distance: usize, prominence: f64) -> Vec<usize> {
    // Local maxima, plateaus resolved to their middle.
    let mut peaks = Vec::new();
    let n = x.len();
    let mut i = 1;
    while n >= 3 && i < n - 1 {
        if x[i - 1] < x[i] {
            let mut ahead = i + 1;
            while ahead < n - 1 && x[ahead] == x[i] {
                ahead += 1;
            }
            if x[ahead] < x[i] {
                peaks.push((i + ahead - 1) / 2);
                i = ahead;
            }
        }
        i += 1;
    }
    peaks.retain(|&p| x[p] >= height);
    // Distance: keep higher peaks first, drop neighbours closer than `distance`.
    if distance > 1 && !peaks.is_empty() {
        let mut keep = vec![true; peaks.len()];
        let mut order: Vec<usize> = (0..peaks.len()).collect();
        order.sort_by(|&a, &b| x[peaks[a]].partial_cmp(&x[peaks[b]]).unwrap_or(std::cmp::Ordering::Equal));
        for &i in order.iter().rev() {
            if !keep[i] {
                continue;
            }
            let mut k = i;
            while k > 0 && peaks[i] - peaks[k - 1] < distance {
                keep[k - 1] = false;
                k -= 1;
            }
            let mut k = i + 1;
            while k < peaks.len() && peaks[k] - peaks[i] < distance {
                keep[k] = false;
                k += 1;
            }
        }
        peaks = peaks.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).collect();
    }
    peaks.retain(|&p| peak_prominence(x, p) >= prominence);
    peaks
}

fn peak_prominence(x: &[f64], p: usize) -> f64 {
    let h = x[p];
    let mut left_min = h;
    let mut i = p;
    while i > 0 {
        i -= 1;
        if x[i] > h {
            break;
        }
        left_min = left_min.min(x[i]);
    }
    let mut right_min = h;
    let mut i = p;
    while i + 1 < x.len() {
        i += 1;
        if x[i] > h {
            break;
        }
        right_min = right_min.min(x[i]);
    }
    h - left_min.max(right_min)
}

pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() { 0.0 } else { x.iter().sum::<f64>() / x.len() as f64 }
}

pub fn std(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return 0.0;
    }
    let m = mean(x);
    (x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / x.len() as f64).sqrt()
}

pub fn median(x: &[f64]) -> Option<f64> {
    if x.is_empty() {
        return None;
    }
    let mut v = x.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highpass_attenuates_low_passes_high() {
        let sr = 48000.0;
        let hp = butter_highpass(4, 1500.0, sr);
        let gain = |f: f64| {
            let x: Vec<f32> = (0..48000).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() as f32).collect();
            let y = filtfilt(&hp, &x);
            let rms = |v: &[f64]| (v.iter().map(|a| a * a).sum::<f64>() / v.len() as f64).sqrt();
            rms(&y[10000..38000]) / std::f64::consts::FRAC_1_SQRT_2
        };
        assert!(gain(100.0) < 1e-3);
        assert!((gain(1500.0) - 0.5).abs() < 0.02, "{}", gain(1500.0)); // -3 dB twice
        assert!((gain(8000.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn peaks_like_scipy() {
        let x = [0.0, 1.0, 0.0, 5.0, 5.0, 5.0, 0.0, 2.0, 1.0, 3.0, 0.0];
        assert_eq!(find_peaks(&x, 0.0, 1, 0.0), vec![1, 4, 7, 9]);
        assert_eq!(find_peaks(&x, 2.0, 1, 0.0), vec![4, 7, 9]);
        assert_eq!(find_peaks(&x, 0.0, 4, 0.0), vec![4, 9]);
        assert_eq!(find_peaks(&x, 0.0, 1, 1.5), vec![4, 9]);
    }
}
