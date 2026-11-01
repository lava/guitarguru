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
#[serde(default)]
pub struct AudioSettings {
    /// cpal device id string; None = auto (first device whose name contains "BANDFUSE",
    /// else the default input).
    pub input_device: Option<String>,
    pub input_channel: u16,
    /// None = default output.
    pub output_device: Option<String>,
    pub buffer_frames: u32,
    pub click_volume_db: f32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { input_device: None, input_channel: 0, output_device: None, buffer_frames: 128, click_volume_db: -4.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// Through PipeWire/PulseAudio: shared with other programs.
    Shared,
    /// One ALSA hardware device per card (exclusive, lowest latency).
    Direct,
    /// ALSA aliases and plugin pseudo-devices (only with "show all").
    Other,
}

#[derive(Clone, Debug)]
pub struct DeviceEntry {
    /// Host-qualified cpal id, e.g. `pulseaudio:alsa_input.usb-...` or `alsa:plughw:CARD=Adapter,DEV=0`.
    pub id: String,
    pub label: String,
    pub kind: DeviceKind,
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_owned()).unwrap_or_else(|_| "?".into())
}

fn device_label(d: &cpal::Device) -> String {
    let id = device_id(d).unwrap_or_default();
    format!("{} [{id}]", device_name(d))
}

/// Host-qualified id string (`host:device`).
fn device_id(d: &cpal::Device) -> Option<String> {
    d.id().ok().map(|i| i.to_string())
}

/// Settings written before ids were host-qualified hold bare ALSA ids.
fn qualify(id: &str) -> String {
    if id.starts_with("alsa:") || id.starts_with("pulseaudio:") || id.starts_with("jack:") || id.starts_with("pipewire:") {
        id.to_string()
    } else {
        format!("alsa:{id}")
    }
}

fn hosts() -> Vec<cpal::Host> {
    cpal::available_hosts().into_iter().filter_map(|h| cpal::host_from_id(h).ok()).collect()
}

fn all_devices(input: bool) -> Vec<cpal::Device> {
    let mut v = Vec::new();
    for h in hosts() {
        if let Ok(ds) = if input { h.input_devices() } else { h.output_devices() } {
            v.extend(ds);
        }
    }
    v
}

fn card_of(alsa_id: &str) -> Option<&str> {
    let rest = alsa_id.split("CARD=").nth(1)?;
    Some(rest.split(',').next().unwrap_or(rest))
}

/// Classify a device: PulseAudio sources/sinks are shared (monitors are hidden), ALSA
/// devices collapse to one `plughw:CARD=<name>` entry per card, everything else is "other".
fn kind_of(id: &str, name: &str, chosen_per_card: &std::collections::HashMap<String, String>) -> Option<DeviceKind> {
    if let Some(pid) = id.strip_prefix("pulseaudio:") {
        return if pid.ends_with(".monitor") { None } else { Some(DeviceKind::Shared) };
    }
    if chosen_per_card.get(name).is_some_and(|c| c == id) {
        return Some(DeviceKind::Direct);
    }
    Some(DeviceKind::Other)
}

/// Per card name, the ALSA id to show as its direct entry: plughw with a named card, else hw.
fn direct_choices(devs: &[cpal::Device]) -> std::collections::HashMap<String, String> {
    let mut best: std::collections::HashMap<String, (u8, String)> = std::collections::HashMap::new();
    for d in devs {
        let Some(id) = device_id(d) else { continue };
        let Some(aid) = id.strip_prefix("alsa:") else { continue };
        let Some(card) = card_of(aid) else { continue };
        let named = !card.chars().all(|c| c.is_ascii_digit());
        let rank = match (aid.starts_with("plughw:"), aid.starts_with("hw:"), named) {
            (true, _, true) => 0,
            (_, true, true) => 1,
            (true, _, false) => 2,
            (_, true, false) => 3,
            _ => continue,
        };
        let name = device_name(d);
        if best.get(&name).is_none_or(|(r, _)| rank < *r) {
            best.insert(name, (rank, id));
        }
    }
    best.into_iter().map(|(k, (_, id))| (k, id)).collect()
}

pub fn list_devices(input: bool, show_all: bool) -> Vec<DeviceEntry> {
    let devs = all_devices(input);
    let chosen = direct_choices(&devs);
    let mut out: Vec<DeviceEntry> = devs
        .iter()
        .filter_map(|d| {
            let id = device_id(d)?;
            let name = device_name(d);
            let kind = kind_of(&id, &name, &chosen)?;
            if kind == DeviceKind::Other && !show_all {
                return None;
            }
            let label = match kind {
                DeviceKind::Shared => format!("shared  {name}"),
                DeviceKind::Direct => format!("direct  {}", name.trim_end_matches(", USB Audio")),
                DeviceKind::Other => format!("{name} [{id}]"),
            };
            Some(DeviceEntry { id, label, kind })
        })
        .collect();
    out.sort_by_key(|e| (e.kind as u8, !e.label.contains("BANDFUSE")));
    out
}

/// Who holds an ALSA card's capture device open, from /proc/asound (pid, program name).
pub fn alsa_capture_owner(card: &str) -> Option<(u32, String)> {
    for e in std::fs::read_dir("/proc/asound").ok()?.flatten() {
        let path = e.path();
        let id = std::fs::read_to_string(path.join("id")).unwrap_or_default();
        let num = e.file_name().to_string_lossy().trim_start_matches("card").to_string();
        if id.trim() != card && num != card {
            continue;
        }
        for pcm in std::fs::read_dir(&path).ok()?.flatten() {
            let n = pcm.file_name().to_string_lossy().to_string();
            if !(n.starts_with("pcm") && n.ends_with('c')) {
                continue;
            }
            for sub in std::fs::read_dir(pcm.path()).ok()?.flatten() {
                let st = std::fs::read_to_string(sub.path().join("status")).unwrap_or_default();
                let pid = st.lines().find_map(|l| l.strip_prefix("owner_pid").map(|v| v.trim_start_matches([' ', ':']).trim().to_string()));
                if let Some(pid) = pid.and_then(|p| p.parse::<u32>().ok()) {
                    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default().trim().to_string();
                    return Some((pid, comm));
                }
            }
        }
    }
    None
}

/// ALSA card id (e.g. `Adapter`) of the first card whose description contains `needle`.
pub fn alsa_card_with(needle: &str) -> Option<String> {
    let cards = std::fs::read_to_string("/proc/asound/cards").ok()?;
    let line = cards.lines().find(|l| l.contains(needle))?;
    let num = line.split_whitespace().next()?;
    std::fs::read_to_string(format!("/proc/asound/card{num}/id")).ok().map(|s| s.trim().to_string())
}

/// Explain why a device that should be there could not be used, from /proc/asound.
pub fn explain_missing(alsa_card: &str, label: &str) -> String {
    match alsa_capture_owner(alsa_card) {
        Some((pid, comm)) if comm != "pipewire" && comm != "wireplumber" => {
            format!("{label} busy: opened directly by {comm} (pid {pid}); close it there or use the shared device")
        }
        Some((pid, comm)) => format!("{label} busy: held by {comm} (pid {pid}); use the shared device"),
        None => format!("{label} not available"),
    }
}

/// Input candidates in the order to try. Auto: a known USB guitar cable (the BANDFUSE
/// adapter) through PipeWire (shared), then direct ALSA; other interfaces are picked in
/// Setup. Returns (candidates, adapter present?).
fn find_input(want: Option<&str>) -> (Vec<cpal::Device>, bool) {
    let all = all_devices(true);
    if let Some(w) = want {
        let w = qualify(w);
        let v: Vec<cpal::Device> = all.into_iter().filter(|d| device_id(d).as_deref() == Some(w.as_str())).collect();
        let present = !v.is_empty();
        return (v, present);
    }
    let chosen = direct_choices(&all);
    let rank = |d: &cpal::Device| {
        let id = device_id(d).unwrap_or_default();
        if id.starts_with("pulseaudio:") && !id.ends_with(".monitor") {
            Some(0)
        } else if chosen.values().any(|c| *c == id) {
            Some(1)
        } else if id.starts_with("alsa:hw:") {
            Some(2)
        } else {
            None
        }
    };
    let mut cands: Vec<(u8, cpal::Device)> =
        all.into_iter().filter(|d| device_name(d).contains("BANDFUSE")).filter_map(|d| rank(&d).map(|r| (r, d))).collect();
    cands.sort_by_key(|(r, _)| *r);
    let present = !cands.is_empty();
    (cands.into_iter().map(|(_, d)| d).collect(), present)
}

fn describe_open_error(dev: &cpal::Device, e: &str, busy: bool) -> String {
    let name = device_name(dev).replace(", USB Audio", "");
    let id = device_id(dev).unwrap_or_default();
    if busy || e.to_lowercase().contains("busy") {
        let owner = id
            .strip_prefix("alsa:")
            .and_then(card_of)
            .and_then(alsa_capture_owner)
            .map(|(pid, comm)| format!(" by {comm} (pid {pid})"))
            .unwrap_or_default();
        format!("{name} busy: in use{owner}")
    } else {
        format!("{name} [{id}]: {e}")
    }
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

/// A backing track looped under a session: one pass of `samples` from `start_frame`
/// (output frames), repeated until `end_frame`. The click is ducked while it plays.
#[derive(Clone)]
pub struct BackingPlan {
    pub samples: Arc<Vec<f32>>,
    pub start_frame: u64,
    pub end_frame: u64,
}

/// Click gain while a backing track plays (-6 dB).
const BACKING_CLICK_DUCK: f32 = 0.5;

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
    pub backing: Mutex<Option<BackingPlan>>,
    pub xruns: AtomicU32,
    /// Per-callback (frame, timestamp ns) logs during a session, for diagnosing clocks.
    pub log_on: AtomicBool,
    pub in_log: Mutex<Vec<(u64, u128)>>,
    pub out_log: Mutex<Vec<(u64, u128)>>,
}

const LOG_CAP: usize = 250_000;

fn log_stamp(on: &AtomicBool, log: &Mutex<Vec<(u64, u128)>>, frame: u64, ns: u128) {
    if on.load(Ordering::Relaxed)
        && ns != 0
        && let Ok(mut l) = log.try_lock()
        && l.len() < l.capacity()
    {
        l.push((frame, ns));
    }
}

/// How far a rolling recording may grow past what it keeps before it is cut back.
const KEEP_SLACK: usize = SAMPLE_RATE as usize * 30;

#[derive(Default)]
pub struct Recording {
    pub active: bool,
    /// Input frame index of `samples[0]`.
    pub start_frame: u64,
    pub samples: Vec<f32>,
    /// Keep only about the last this many samples (open-ended free play).
    pub keep: Option<usize>,
}

pub struct EngineInfo {
    pub sample_rate: u32,
    pub input_label: String,
    #[allow(dead_code)]
    pub output_label: String,
    pub input_buffer: String,
    pub output_buffer: String,
    pub warning: Option<String>,
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

enum InputHandle {
    Device(#[allow(dead_code)] cpal::Stream),
    /// Synthesised guitar for headless testing (`GG_FAKE_INPUT=1`).
    Fake(Arc<AtomicBool>),
}

impl Drop for InputHandle {
    fn drop(&mut self) {
        if let InputHandle::Fake(stop) = self {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

enum OutputHandle {
    Device(#[allow(dead_code)] cpal::Stream),
    Fake(Arc<AtomicBool>),
}

impl Drop for OutputHandle {
    fn drop(&mut self) {
        if let OutputHandle::Fake(stop) = self {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

struct Streams {
    _input: InputHandle,
    _output: OutputHandle,
}

impl AudioEngine {
    pub fn start(settings: &AudioSettings, tone: &ToneSettings) -> Result<Self, String> {
        let shared = Arc::new(Shared::default());
        shared.click_gain.store(10f32.powf(settings.click_volume_db / 20.0));
        shared.monitor_on.store(tone.preset.monitors(), Ordering::Relaxed);
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
                            // Drop the oldest audio in big steps, so it moves rarely.
                            if let Some(k) = r.keep
                                && r.samples.len() > k + KEEP_SLACK
                            {
                                let cut = r.samples.len() - k;
                                r.samples.drain(..cut);
                                r.start_frame += cut as u64;
                            }
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

    /// Start a recording; any previous one is discarded. With `keep_seconds`, only about
    /// the last that many seconds are kept.
    pub fn start_recording(&self, keep_seconds: Option<f64>) {
        let mut r = self.recording.lock().unwrap();
        r.samples.clear();
        let keep = keep_seconds.map(|s| (s * f64::from(SAMPLE_RATE)) as usize);
        r.keep = keep;
        r.samples.reserve(keep.map_or(SAMPLE_RATE as usize * 180, |k| k + KEEP_SLACK));
        r.active = true;
    }

    /// Start logging stream timestamps (pre-allocated, so the callbacks never allocate).
    pub fn start_clock_log(&self) {
        for l in [&self.shared.in_log, &self.shared.out_log] {
            let mut v = l.lock().unwrap();
            v.clear();
            v.reserve(LOG_CAP);
        }
        self.shared.log_on.store(true, Ordering::Relaxed);
    }

    /// Stop logging; returns (input log, output log) decimated to every `step`-th entry.
    pub fn take_clock_log(&self, step: usize) -> (Vec<(u64, u128)>, Vec<(u64, u128)>) {
        self.shared.log_on.store(false, Ordering::Relaxed);
        let take = |l: &Mutex<Vec<(u64, u128)>>| std::mem::take(&mut *l.lock().unwrap()).into_iter().step_by(step.max(1)).collect();
        (take(&self.shared.in_log), take(&self.shared.out_log))
    }

    pub fn stop_recording(&self) -> (u64, Vec<f32>) {
        let mut r = self.recording.lock().unwrap();
        r.active = false;
        (r.start_frame, std::mem::take(&mut r.samples))
    }

    pub fn set_click(&self, plan: Option<ClickPlan>) {
        *self.shared.click.lock().unwrap() = plan;
    }

    pub fn set_backing(&self, plan: Option<BackingPlan>) {
        *self.shared.backing.lock().unwrap() = plan;
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
    let mut errors: Vec<String> = Vec::new();

    // Input: try candidates until one opens.
    let mut last_err = String::from("no input device found");
    let mut input = None;
    let mut raw_tx = Some(raw_tx);
    let mut mon_tx = Some(mon_tx);
    let fake = std::env::var_os("GG_FAKE_INPUT").is_some();
    let (candidates, present) = if fake { (Vec::new(), true) } else { find_input(s.input_device.as_deref()) };
    for dev in candidates {
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
            Err((e, busy, st)) => {
                last_err = describe_open_error(&dev, &e, busy);
                errors.push(last_err.clone());
                raw_tx = Some(st.raw);
                mon_tx = Some(st.mon);
            }
        }
    }
    let (in_stream, in_label, in_buffer) = if fake {
        let state = InState { shared: Arc::clone(shared), raw: raw_tx.take().unwrap(), mon: mon_tx.take().unwrap(), scratch: vec![0.0; 16384], frame: 0 };
        (InputHandle::Fake(spawn_fake_input(state)), "FAKE INPUT (synthesised eighths)".to_string(), "128".to_string())
    } else {
        match input {
            Some((st, label, cfg)) => (InputHandle::Device(st), label, format!("{:?}", cfg.buffer_size)),
            None if s.input_device.is_none() && !present => {
                // No adapter at all: open the default input for the meter, monitoring muted.
                let dev = host.default_input_device().ok_or("no guitar adapter found and no default input")?;
                let cfgs = dev.supported_input_configs().map_err(|e| e.to_string())?;
                let (cfg, bs) = pick_config(cfgs.collect(), SAMPLE_RATE, 1, 1).ok_or("default input: no 48 kHz config")?;
                let mut sc: cpal::StreamConfig = cfg.config();
                sc.buffer_size = buffer_size(s.buffer_frames, &bs);
                let ch = sc.channels as usize;
                let state = InState { shared: Arc::clone(shared), raw: raw_tx.take().unwrap(), mon: mon_tx.take().unwrap(), scratch: vec![0.0; 16384], frame: 0 };
                let r = match cfg.sample_format() {
                    cpal::SampleFormat::F32 => build_in::<f32>(&dev, sc.clone(), ch, 0, state),
                    cpal::SampleFormat::I32 => build_in::<i32>(&dev, sc.clone(), ch, 0, state),
                    _ => build_in::<i16>(&dev, sc.clone(), ch, 0, state),
                };
                let st = r.map_err(|(e, _, _)| format!("default input: {e}"))?;
                (InputHandle::Device(st), device_label(&dev), format!("{:?}", sc.buffer_size))
            }
            None => {
                let msg = if !errors.is_empty() {
                    errors.join("; ")
                } else if let Some(card) = s.input_device.as_deref().map(qualify).as_deref().and_then(|i| i.strip_prefix("alsa:")).and_then(card_of) {
                    // ALSA hides a busy device from enumeration.
                    explain_missing(card, &format!("ALSA device {card}"))
                } else {
                    last_err
                };
                return Err(msg);
            }
        }
    };
    let mut warning = None;
    if !fake && s.input_device.is_none() && !present {
        warning = Some("No known guitar adapter found - using the default input, monitoring muted; pick your interface in Setup".to_string());
        shared.monitor_on.store(false, Ordering::Relaxed);
    }
    let input_is_pulse = in_label.contains("[pulseaudio:");

    let fake_out = fake || std::env::var_os("GG_FAKE_OUTPUT").is_some();
    let (out_handle, out_label, out_buffer) = if fake_out {
        let st = OutState { shared: Arc::clone(shared), mon: mon_rx, frame: 0, scratch: vec![0.0; 16384] };
        (OutputHandle::Fake(spawn_fake_output(st)), "FAKE OUTPUT (silent)".to_string(), "128".to_string())
    } else {
        // Default output: same host as the input, so both streams share one clock domain.
        let default_out = || {
            if input_is_pulse {
                cpal::host_from_id(cpal::HostId::PulseAudio).ok().and_then(|h| h.default_output_device()).or_else(|| host.default_output_device())
            } else {
                host.default_output_device()
            }
        };
        let out_dev = match &s.output_device {
            Some(id) => {
                let id = qualify(id);
                all_devices(false).into_iter().find(|d| device_id(d).as_deref() == Some(id.as_str())).or_else(default_out)
            }
            None => default_out(),
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
        out_stream.play().map_err(|e| format!("output play: {e}"))?;
        (OutputHandle::Device(out_stream), device_label(&out_dev), format!("{:?}", sc.buffer_size))
    };
    let info = EngineInfo {
        sample_rate: SAMPLE_RATE,
        input_label: in_label,
        output_label: out_label,
        input_buffer: in_buffer,
        warning,
        output_buffer: out_buffer,
    };
    Ok((Streams { _input: in_stream, _output: out_handle }, info))
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
        if stamp != 0
            && let Ok(mut r) = sh.in_ref.try_lock()
        {
            *r = Some(StampRef { frame: self.frame, nanos: stamp });
        }
        log_stamp(&sh.log_on, &sh.in_log, self.frame, stamp);
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
            sh.dsp_load.store(0.0);
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
) -> Result<cpal::Stream, (String, bool, InState)>
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
    let err = match r {
        Ok(s) => match s.play() {
            Ok(()) => return Ok(s),
            Err(e) => {
                drop(s);
                e
            }
        },
        Err(e) => e,
    };
    let busy = err.kind() == cpal::ErrorKind::DeviceBusy;
    let st = slot.lock().ok().and_then(|mut s| s.take());
    match st {
        Some(st) => Err((err.to_string(), busy, st)),
        None => panic!("input state lost after failed open: {err}"),
    }
}

/// Feeds synthesised palm-muted eighths, following the click plan (+15 ms, 6 ms jitter,
/// every 13th note missed), through the normal input path. For headless testing.
fn spawn_fake_input(mut st: InState) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    std::thread::spawn(move || {
        let sr = f64::from(SAMPLE_RATE);
        let block = 128usize;
        let t0 = std::time::Instant::now();
        let mut seed: u64 = 0x9e3779b97f4a7c15;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut next_hit: Option<(u64, f64)> = None; // (frame, amplitude)
        let mut env = 0.0f64;
        let mut tone_phase = 0.0f64;
        let mut note_k: i64 = -1;
        while !stop2.load(Ordering::Relaxed) {
            let due = (t0.elapsed().as_secs_f64() * sr) as u64;
            while st.frame + (block as u64) <= due {
                let plan = st.shared.click.lock().ok().and_then(|c| *c);
                for i in 0..block {
                    let f = st.frame + i as u64;
                    if let Some(p) = plan {
                        let half = p.frames_per_beat / 2.0;
                        let rel = f as f64 - p.start_frame as f64 - 0.015 * sr;
                        let k = (rel / half).floor() as i64;
                        if k > note_k && k >= 0 && (k as u32) < p.beats * 2 {
                            note_k = k;
                            if k % 13 != 12 {
                                let jitter = ((0..6).map(|_| rnd()).sum::<f64>() - 3.0) * 0.006 * 1.41;
                                let at = (p.start_frame as f64 + 0.015 * sr + k as f64 * half + jitter * sr).max(0.0) as u64;
                                next_hit = Some((at.max(f), 0.12 * (1.0 + 0.1 * (rnd() - 0.5))));
                            }
                        }
                    }
                    if let Some((at, amp)) = next_hit
                        && f >= at
                    {
                        env = amp;
                        next_hit = None;
                    }
                    tone_phase += 82.4 / sr;
                    let noise = rnd() * 2.0 - 1.0;
                    let v = env * (0.6 * noise + 0.4 * (std::f64::consts::TAU * tone_phase).sin()) + 1e-4 * (rnd() - 0.5);
                    env *= (-1.0 / (0.006 * sr)).exp();
                    st.scratch[i] = v as f32;
                }
                st.block(block, 0);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    stop
}

/// Real-time paced, silent output for headless testing.
fn spawn_fake_output(mut st: OutState) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = Arc::clone(&stop);
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        while !stop2.load(Ordering::Relaxed) {
            let due = (t0.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as u64;
            while st.frame + 128 <= due {
                st.block(128, 0);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    stop
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
        if stamp != 0
            && let Ok(mut r) = sh.out_ref.try_lock()
        {
            *r = Some(StampRef { frame: self.frame, nanos: stamp });
        }
        log_stamp(&sh.log_on, &sh.out_log, self.frame, stamp);
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
        let mut backing_from = u64::MAX;
        if let Ok(b) = sh.backing.try_lock()
            && let Some(b) = b.as_ref()
            && !b.samples.is_empty()
        {
            backing_from = b.start_frame;
            let len = b.samples.len() as u64;
            for (i, v) in x.iter_mut().enumerate() {
                let f = self.frame + i as u64;
                if f >= b.start_frame && f < b.end_frame {
                    *v += b.samples[((f - b.start_frame) % len) as usize];
                }
            }
        }
        if let Ok(c) = sh.click.try_lock()
            && let Some(plan) = c.as_ref()
        {
            let g = sh.click_gain.load();
            for (i, v) in x.iter_mut().enumerate() {
                let f = self.frame + i as u64;
                let duck = if f >= backing_from { BACKING_CLICK_DUCK } else { 1.0 };
                *v += g * duck * render_click(plan, f, f64::from(SAMPLE_RATE));
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
