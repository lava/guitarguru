//! cpal audio engine: mono input (one channel of the chosen device), stereo output.
//!
//! - The input callback records the raw DI (via a ring buffer drained by the engine
//!   thread), and runs the monitoring chain whose output goes through a second ring to
//!   the output callback.
//! - The output callback mixes monitor signal, the metronome click (sample-accurate, from
//!   the output frame counter) and an optional playback buffer (listen-back).
//! - Both callbacks publish (frame counter, stream timestamp) pairs so a click scheduled at
//!   an output frame can be mapped into the input (recording) timeline.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};

use crate::monitor::{Chain, ToneSettings};

pub const SAMPLE_RATE: u32 = 48000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioSettings {
    /// cpal device id string; None = auto (first device whose name contains "BANDFUSE",
    /// else the default input).
    pub input_device: Option<String>,
    pub input_channel: u16,
    /// None = default output.
    pub output_device: Option<String>,
    pub buffer_frames: u32,
    pub click_volume_db: f32,
    pub monitor: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { input_device: None, input_channel: 0, output_device: None, buffer_frames: 128, click_volume_db: -10.0, monitor: true }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceEntry {
    pub id: String,
    pub label: String,
}

fn device_label(d: &cpal::Device) -> String {
    match d.description() {
        Ok(desc) => {
            let mut s = desc.name().to_owned();
            if let Some(extra) = desc.extended().nth(1) {
                s.push_str(" - ");
                s.push_str(extra);
            }
            s
        }
        Err(_) => "?".into(),
    }
}

fn device_id(d: &cpal::Device) -> Option<String> {
    d.id().ok().map(|i| i.id().to_owned())
}

pub fn list_devices(input: bool) -> Vec<DeviceEntry> {
    let host = cpal::default_host();
    let devs = if input { host.input_devices() } else { host.output_devices() };
    devs.map(|ds| {
        ds.filter_map(|d| Some(DeviceEntry { id: device_id(&d)?, label: format!("{} [{}]", device_label(&d), device_id(&d)?) }))
            .collect()
    })
    .unwrap_or_default()
}

fn find_input(host: &cpal::Host, want: Option<&str>) -> Vec<cpal::Device> {
    let all: Vec<cpal::Device> = host.input_devices().map(|d| d.collect()).unwrap_or_default();
    if let Some(w) = want {
        return all.into_iter().filter(|d| device_id(d).as_deref() == Some(w)).collect();
    }
    let mut cands: Vec<cpal::Device> = all.into_iter().filter(|d| device_label(d).contains("BANDFUSE")).collect();
    let rank = |d: &cpal::Device| {
        let id = device_id(d).unwrap_or_default();
        if id.starts_with("hw:") {
            0
        } else if id.starts_with("plughw:") {
            1
        } else {
            2
        }
    };
    cands.sort_by_key(rank);
    cands.extend(host.default_input_device());
    cands
}

/// f32 stored in an AtomicU32.
#[derive(Default)]
pub struct AtomicF32(AtomicU32);
impl AtomicF32 {
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
    /// For non-negative values only.
    pub fn fetch_max(&self, v: f32) {
        self.0.fetch_max(v.max(0.0).to_bits(), Ordering::Relaxed);
    }
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StampRef {
    pub frame: u64,
    pub nanos: u128,
}

/// Click schedule: beats at `start + round(k * frames_per_beat)`, k in 0..beats.
#[derive(Clone, Copy, Debug)]
pub struct ClickPlan {
    pub start_frame: u64,
    pub frames_per_beat: f64,
    pub beats: u32,
    pub beats_per_bar: u32,
}

pub struct Playback {
    pub samples: Vec<f32>,
    pub pos: usize,
}

#[derive(Default)]
pub struct Shared {
    pub in_peak: AtomicF32,
    pub out_peak: AtomicF32,
    pub in_frames: AtomicU64,
    pub out_frames: AtomicU64,
    pub in_ref: Mutex<Option<StampRef>>,
    pub out_ref: Mutex<Option<StampRef>>,
    pub click: Mutex<Option<ClickPlan>>,
    pub click_gain: AtomicF32,
    pub monitor_on: AtomicBool,
    pub chain: Mutex<Option<Box<Chain>>>,
    /// DSP time / block time of the monitoring chain (smoothed).
    pub dsp_load: AtomicF32,
    pub playback: Mutex<Option<Playback>>,
    pub xruns: AtomicU32,
}

#[derive(Default)]
pub struct Recording {
    pub active: bool,
    /// Input frame index of `samples[0]`.
    pub start_frame: u64,
    pub samples: Vec<f32>,
}

pub struct EngineInfo {
    pub sample_rate: u32,
    pub input_label: String,
    pub output_label: String,
    pub input_buffer: String,
    pub output_buffer: String,
}

pub struct AudioEngine {
    pub shared: Arc<Shared>,
    pub recording: Arc<Mutex<Recording>>,
    pub info: EngineInfo,
    stop: mpsc::Sender<()>,
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

fn pick_config(
    configs: Vec<cpal::SupportedStreamConfigRange>,
    sr: u32,
    min_channels: u16,
    want_channels: u16,
) -> Option<(cpal::SupportedStreamConfig, cpal::SupportedBufferSize)> {
    let rank = |f: cpal::SampleFormat| match f {
        cpal::SampleFormat::F32 => Some(0),
        cpal::SampleFormat::I32 => Some(1),
        cpal::SampleFormat::I16 => Some(2),
        _ => None,
    };
    configs
        .into_iter()
        .filter(|c| c.channels() >= min_channels && rank(c.sample_format()).is_some())
        .filter_map(|c| {
            let bs = *c.buffer_size();
            let key = (c.channels().abs_diff(want_channels), rank(c.sample_format()).unwrap_or(9));
            c.try_with_sample_rate(sr).map(|s| (key, s, bs))
        })
        .min_by_key(|(k, _, _)| *k)
        .map(|(_, s, b)| (s, b))
}

fn buffer_size(want: u32, sup: &cpal::SupportedBufferSize) -> cpal::BufferSize {
    match sup {
        cpal::SupportedBufferSize::Range { min, max } => cpal::BufferSize::Fixed(want.clamp(*min, *max)),
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Fixed(want),
    }
}

struct Streams {
    _input: cpal::Stream,
    _output: cpal::Stream,
}

impl AudioEngine {
    pub fn start(settings: &AudioSettings, tone: &ToneSettings) -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        shared.click_gain.store(10f32.powf(settings.click_volume_db / 20.0));
        shared.monitor_on.store(settings.monitor, Ordering::Relaxed);
        *shared.chain.lock().unwrap() = Some(Box::new(Chain::build(tone, SAMPLE_RATE)));
        let recording = Arc::new(Mutex::new(Recording::default()));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<EngineInfo, String>>();
        let sh = Arc::clone(&shared);
        let rec = Arc::clone(&recording);
        let settings = settings.clone();
        std::thread::Builder::new()
            .name("audio-owner".into())
            .spawn(move || {
                let (raw_tx, mut raw_rx) = match rtrb::RingBuffer::<f32>::new(SAMPLE_RATE as usize * 10) {
                    (p, c) => (p, c),
                };
                let built = build_streams(&settings, &sh, raw_tx);
                let _streams = match built {
                    Ok((streams, info)) => {
                        let _ = ready_tx.send(Ok(info));
                        streams
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                // Drain raw input into the recording until told to stop.
                let mut frame: u64 = 0;
                loop {
                    match stop_rx.recv_timeout(Duration::from_millis(5)) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        _ => break,
                    }
                    let n = raw_rx.slots();
                    if n == 0 {
                        continue;
                    }
                    let Ok(chunk) = raw_rx.read_chunk(n) else { continue };
                    let (a, b) = chunk.as_slices();
                    if let Ok(mut r) = rec.lock() {
                        if r.active {
                            if r.samples.is_empty() {
                                r.start_frame = frame;
                            }
                            r.samples.extend_from_slice(a);
                            r.samples.extend_from_slice(b);
                        }
                    }
                    frame += n as u64;
                    chunk.commit_all();
                }
            })
            .map_err(|e| e.to_string())?;
        let info = ready_rx.recv().map_err(|_| "audio thread died".to_string())??;
        Ok(Self { shared, recording, info, stop: stop_tx })
    }

    pub fn set_chain(&self, chain: Chain) {
        let old = self.shared.chain.lock().map(|mut c| c.replace(Box::new(chain)));
        drop(old);
    }

    /// Start a recording; any previous one is discarded.
    pub fn start_recording(&self) {
        let mut r = self.recording.lock().unwrap();
        r.samples.clear();
        r.samples.reserve(SAMPLE_RATE as usize * 180);
        r.active = true;
    }

    pub fn stop_recording(&self) -> (u64, Vec<f32>) {
        let mut r = self.recording.lock().unwrap();
        r.active = false;
        (r.start_frame, std::mem::take(&mut r.samples))
    }

    pub fn set_click(&self, plan: Option<ClickPlan>) {
        *self.shared.click.lock().unwrap() = plan;
    }

    pub fn play(&self, samples: Vec<f32>) {
        *self.shared.playback.lock().unwrap() = Some(Playback { samples, pos: 0 });
    }

    pub fn stop_playback(&self) {
        *self.shared.playback.lock().unwrap() = None;
    }

    /// Playback progress: (position, length) in samples.
    pub fn playback_pos(&self) -> Option<(usize, usize)> {
        self.shared.playback.lock().ok()?.as_ref().map(|p| (p.pos, p.samples.len()))
    }

    /// Map an output frame to an input frame. Uses the stream timestamps when they look sane,
    /// else assumes both counters advance together (no latency information).
    pub fn out_to_in_frame(&self, out_frame: u64) -> (f64, bool) {
        let sr = f64::from(self.info.sample_rate);
        let o = *self.shared.out_ref.lock().unwrap();
        let i = *self.shared.in_ref.lock().unwrap();
        let naive = out_frame as f64 + self.shared.in_frames.load(Ordering::Relaxed) as f64
            - self.shared.out_frames.load(Ordering::Relaxed) as f64;
        if let (Some(o), Some(i)) = (o, i) {
            let t_ns = o.nanos as f64 + (out_frame as f64 - o.frame as f64) / sr * 1e9;
            let in_f = i.frame as f64 + (t_ns - i.nanos as f64) * 1e-9 * sr;
            if (in_f - naive).abs() < 0.5 * sr {
                return (in_f, true);
            }
        }
        (naive, false)
    }
}

fn build_streams(
    s: &AudioSettings,
    shared: &Arc<Shared>,
    raw_tx: rtrb::Producer<f32>,
) -> Result<(Streams, EngineInfo), String> {
    let host = cpal::default_host();
    let (mon_tx, mon_rx) = rtrb::RingBuffer::<f32>::new(8192);

    // Input: try candidates until one opens.
    let mut last_err = String::from("no input device found");
    let mut input = None;
    let mut raw_tx = Some(raw_tx);
    let mut mon_tx = Some(mon_tx);
    for dev in find_input(&host, s.input_device.as_deref()) {
        let Ok(cfgs) = dev.supported_input_configs() else { continue };
        let Some((cfg, bs)) = pick_config(cfgs.collect(), SAMPLE_RATE, s.input_channel + 1, 1) else {
            last_err = format!("{}: no {SAMPLE_RATE} Hz config with {} channels", device_label(&dev), s.input_channel + 1);
            continue;
        };
        let mut sc: cpal::StreamConfig = cfg.config();
        sc.buffer_size = buffer_size(s.buffer_frames, &bs);
        let ch = sc.channels as usize;
        let sel = s.input_channel as usize;
        let state = InState { shared: Arc::clone(shared), raw: raw_tx.take().unwrap(), mon: mon_tx.take().unwrap(), scratch: vec![0.0; 16384], frame: 0 };
        let r = match cfg.sample_format() {
            cpal::SampleFormat::F32 => build_in::<f32>(&dev, sc.clone(), ch, sel, state),
            cpal::SampleFormat::I32 => build_in::<i32>(&dev, sc.clone(), ch, sel, state),
            _ => build_in::<i16>(&dev, sc.clone(), ch, sel, state),
        };
        match r {
            Ok(stream) => {
                input = Some((stream, device_label(&dev), sc));
                break;
            }
            Err((e, st)) => {
                last_err = format!("{}: {e}", device_label(&dev));
                raw_tx = Some(st.raw);
                mon_tx = Some(st.mon);
            }
        }
    }
    let (in_stream, in_label, in_cfg) = input.ok_or(last_err)?;

    let out_dev = match &s.output_device {
        Some(id) => host
            .output_devices()
            .ok()
            .and_then(|mut ds| ds.find(|d| device_id(d).as_deref() == Some(id)))
            .or_else(|| host.default_output_device()),
        None => host.default_output_device(),
    }
    .ok_or("no output device")?;
    let cfgs = out_dev.supported_output_configs().map_err(|e| e.to_string())?;
    let (cfg, bs) = pick_config(cfgs.collect(), SAMPLE_RATE, 1, 2).ok_or("output device: no 48 kHz config")?;
    let mut sc: cpal::StreamConfig = cfg.config();
    sc.buffer_size = buffer_size(s.buffer_frames, &bs);
    let ch = sc.channels as usize;
    let st = OutState { shared: Arc::clone(shared), mon: mon_rx, frame: 0, scratch: vec![0.0; 16384] };
    let out_stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => build_out::<f32>(&out_dev, sc.clone(), ch, st),
        cpal::SampleFormat::I32 => build_out::<i32>(&out_dev, sc.clone(), ch, st),
        _ => build_out::<i16>(&out_dev, sc.clone(), ch, st),
    }
    .map_err(|e| format!("output: {e}"))?;
    in_stream.play().map_err(|e| format!("input play: {e}"))?;
    out_stream.play().map_err(|e| format!("output play: {e}"))?;
    let info = EngineInfo {
        sample_rate: SAMPLE_RATE,
        input_label: in_label,
        output_label: device_label(&out_dev),
        input_buffer: format!("{:?}", in_cfg.buffer_size),
        output_buffer: format!("{:?}", sc.buffer_size),
    };
    Ok((Streams { _input: in_stream, _output: out_stream }, info))
}

struct InState {
    shared: Arc<Shared>,
    raw: rtrb::Producer<f32>,
    mon: rtrb::Producer<f32>,
    scratch: Vec<f32>,
    frame: u64,
}

impl InState {
    fn block(&mut self, n: usize, stamp: u128) {
        let sh = &self.shared;
        if let Ok(mut r) = sh.in_ref.try_lock() {
            *r = Some(StampRef { frame: self.frame, nanos: stamp });
        }
        let x = &mut self.scratch[..n];
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        sh.in_peak.fetch_max(peak);
        if let Ok(mut chunk) = self.raw.write_chunk_uninit(n.min(self.raw.slots())) {
            let k = chunk.len();
            let (a, b) = chunk.as_mut_slices();
            for (d, s) in a.iter_mut().chain(b.iter_mut()).zip(x.iter()) {
                d.write(*s);
            }
            unsafe { chunk.commit_all() };
            if k < n {
                sh.xruns.fetch_add(1, Ordering::Relaxed);
            }
        }
        if sh.monitor_on.load(Ordering::Relaxed) {
            let t = std::time::Instant::now();
            match sh.chain.try_lock() {
                Ok(mut c) => match c.as_mut() {
                    Some(c) => c.process(x),
                    None => x.iter_mut().for_each(|v| *v = 0.0),
                },
                Err(_) => x.iter_mut().for_each(|v| *v = 0.0),
            }
            let load = t.elapsed().as_secs_f32() / (n as f32 / SAMPLE_RATE as f32);
            let old = sh.dsp_load.load();
            sh.dsp_load.store(old + 0.05 * (load - old));
        } else {
            x.iter_mut().for_each(|v| *v = 0.0);
        }
        let m = n.min(self.mon.slots());
        if let Ok(chunk) = self.mon.write_chunk_uninit(m) {
            chunk.fill_from_iter(x.iter().copied());
        }
        self.frame += n as u64;
        sh.in_frames.store(self.frame, Ordering::Relaxed);
    }
}

fn build_in<T>(
    dev: &cpal::Device,
    cfg: cpal::StreamConfig,
    ch: usize,
    sel: usize,
    st: InState,
) -> Result<cpal::Stream, (String, InState)>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    use cpal::Sample;
    // The state must survive a failed build attempt, so keep it in a shared slot.
    let slot = Arc::new(Mutex::new(Some(st)));
    let cb_slot = Arc::clone(&slot);
    let mut state: Option<InState> = None;
    let r = dev.build_input_stream(
        cfg,
        move |data: &[T], info: &cpal::InputCallbackInfo| {
            if state.is_none() {
                state = cb_slot.lock().ok().and_then(|mut s| s.take());
            }
            let Some(st) = state.as_mut() else { return };
            let stamp = info.timestamp().capture.as_nanos();
            let frames = data.len() / ch;
            let max = st.scratch.len();
            let mut done = 0;
            while done < frames {
                let n = (frames - done).min(max);
                for i in 0..n {
                    st.scratch[i] = f32::from_sample(data[(done + i) * ch + sel]);
                }
                let st_stamp = stamp + (done as u128 * 1_000_000_000 / u128::from(SAMPLE_RATE));
                st.block(n, st_stamp);
                done += n;
            }
        },
        |e| eprintln!("input stream error: {e}"),
        None,
    );
    match r {
        Ok(s) => Ok(s),
        Err(e) => {
            let st = slot.lock().ok().and_then(|mut s| s.take());
            match st {
                Some(st) => Err((e.to_string(), st)),
                None => panic!("input state lost after failed build: {e}"),
            }
        }
    }
}

struct OutState {
    shared: Arc<Shared>,
    mon: rtrb::Consumer<f32>,
    frame: u64,
    scratch: Vec<f32>,
}

/// Synthesised click: sine blip with ~6 ms exponential decay.
fn click_sample(t: f64, accent: bool) -> f32 {
    if !(0.0..0.04).contains(&t) {
        return 0.0;
    }
    let f = if accent { 1760.0 } else { 1175.0 };
    let a = if accent { 1.0 } else { 0.7 };
    let attack = (t / 0.0005).min(1.0);
    (a * attack * (2.0 * std::f64::consts::PI * f * t).sin() * (-t / 0.006).exp()) as f32
}

pub fn render_click(plan: &ClickPlan, frame: u64, sr: f64) -> f32 {
    if frame < plan.start_frame {
        return 0.0;
    }
    let rel = (frame - plan.start_frame) as f64;
    let k = (rel / plan.frames_per_beat).floor() as i64;
    if k < 0 || k >= i64::from(plan.beats) {
        return 0.0;
    }
    let beat_start = (k as f64 * plan.frames_per_beat).round();
    let t = (rel - beat_start) / sr;
    click_sample(t, k % i64::from(plan.beats_per_bar.max(1)) == 0)
}

impl OutState {
    fn block(&mut self, n: usize, stamp: u128) {
        let sh = &self.shared;
        if let Ok(mut r) = sh.out_ref.try_lock() {
            *r = Some(StampRef { frame: self.frame, nanos: stamp });
        }
        let x = &mut self.scratch[..n];
        // Monitor: keep the ring short (bounded latency), fill gaps with silence.
        let avail = self.mon.slots();
        let slack = 256;
        if avail > n + slack {
            if let Ok(c) = self.mon.read_chunk(avail - n - slack / 2) {
                c.commit_all();
            }
        }
        let take = n.min(self.mon.slots());
        x.iter_mut().for_each(|v| *v = 0.0);
        if let Ok(c) = self.mon.read_chunk(take) {
            let (a, b) = c.as_slices();
            for (d, s) in x.iter_mut().zip(a.iter().chain(b.iter())) {
                *d = *s;
            }
            c.commit_all();
        }
        if let Ok(mut pb) = sh.playback.try_lock()
            && let Some(p) = pb.as_mut()
        {
            for v in x.iter_mut() {
                if p.pos < p.samples.len() {
                    *v += p.samples[p.pos];
                    p.pos += 1;
                }
            }
            if p.pos >= p.samples.len() {
                *pb = None;
            }
        }
        if let Ok(c) = sh.click.try_lock()
            && let Some(plan) = c.as_ref()
        {
            let g = sh.click_gain.load();
            for (i, v) in x.iter_mut().enumerate() {
                *v += g * render_click(plan, self.frame + i as u64, f64::from(SAMPLE_RATE));
            }
        }
        let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        sh.out_peak.fetch_max(peak);
        self.frame += n as u64;
        sh.out_frames.store(self.frame, Ordering::Relaxed);
    }
}

fn build_out<T>(dev: &cpal::Device, cfg: cpal::StreamConfig, ch: usize, mut st: OutState) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    dev.build_output_stream(
        cfg,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            let stamp = info.timestamp().playback.as_nanos();
            let frames = data.len() / ch;
            let max = st.scratch.len();
            let mut done = 0;
            while done < frames {
                let n = (frames - done).min(max);
                let st_stamp = stamp + (done as u128 * 1_000_000_000 / u128::from(SAMPLE_RATE));
                st.block(n, st_stamp);
                for i in 0..n {
                    let v = T::from_sample(st.scratch[i].clamp(-1.0, 1.0));
                    for c in 0..ch {
                        data[(done + i) * ch + c] = if c < 2 { v } else { T::from_sample(0.0f32) };
                    }
                }
                done += n;
            }
        },
        |e| eprintln!("output stream error: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_lands_on_beats() {
        let plan = ClickPlan { start_frame: 1000, frames_per_beat: 24000.0, beats: 8, beats_per_bar: 4 };
        let sr = 48000.0;
        assert_eq!(render_click(&plan, 999, sr), 0.0);
        // energy right after each beat start, none before.
        for k in 0..8u64 {
            let b = 1000 + k * 24000;
            let e: f32 = (b..b + 300).map(|f| render_click(&plan, f, sr).abs()).sum();
            assert!(e > 1.0);
            assert_eq!(render_click(&plan, b - 1, sr), 0.0);
        }
        assert_eq!(render_click(&plan, 1000 + 8 * 24000 + 10, sr), 0.0);
    }
}
