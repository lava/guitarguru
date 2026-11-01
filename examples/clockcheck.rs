//! Measure frame counts vs wall clock for a PulseAudio input and the default output.
//! Usage: `cargo run --example clockcheck -- <part of the input's name>`.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
fn main() {
    let host = cpal::host_from_id(cpal::HostId::PulseAudio).unwrap();
    let want = std::env::args().nth(1).expect("usage: clockcheck <part of the input's name>");
    let indev = host.input_devices().unwrap().find(|d| d.description().map(|x| x.name().contains(&want)).unwrap_or(false)).expect("no such input");
    let outdev = host.default_output_device().unwrap();
    let inn = Arc::new(AtomicU64::new(0));
    let out = Arc::new(AtomicU64::new(0));
    let lat = Arc::new(AtomicU64::new(0));
    let cfg = |ch| cpal::StreamConfig { channels: ch, sample_rate: 48000, buffer_size: cpal::BufferSize::Fixed(128) };
    let i2 = inn.clone();
    let is = indev.build_input_stream(cfg(1), move |d: &[f32], _: &cpal::InputCallbackInfo| { i2.fetch_add(d.len() as u64, Ordering::Relaxed); }, |e| eprintln!("in err {e}"), None).unwrap();
    let (o2, l2) = (out.clone(), lat.clone());
    let os = outdev.build_output_stream(cfg(2), move |d: &mut [f32], info: &cpal::OutputCallbackInfo| {
        d.iter_mut().for_each(|v| *v = 0.0);
        o2.fetch_add(d.len() as u64 / 2, Ordering::Relaxed);
        let ts = info.timestamp();
        let l = ts.playback.as_nanos().saturating_sub(ts.callback.as_nanos());
        l2.store(l as u64, Ordering::Relaxed);
    }, |e| eprintln!("out err {e}"), None).unwrap();
    is.play().unwrap(); os.play().unwrap();
    let t0 = std::time::Instant::now();
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let el = t0.elapsed().as_secs_f64() * 48000.0;
        let (i, o) = (inn.load(Ordering::Relaxed) as f64, out.load(Ordering::Relaxed) as f64);
        println!("t={:5.1}s  in-wall {:+7.0}  out-wall {:+7.0}  out-in {:+7.0} frames  playback latency {:.1} ms", el / 48000.0, i - el, o - el, o - i, lat.load(Ordering::Relaxed) as f64 / 1e6);
    }
}
