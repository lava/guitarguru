//! Game state and screen flow. Each screen is an immediate-mode function drawing with
//! [`Paint`] from the state in [`Game`].

use std::sync::atomic::Ordering;

use bevy::prelude::*;

use crate::audio::{AudioEngine, ClickPlan, DeviceEntry, SAMPLE_RATE};
use crate::config::Config;
use crate::detect::{DetectParams, LiveDetector, Onset};
use crate::monitor::Chain;
use crate::paint::Paint;
use crate::patterns::{BEATS_PER_BAR, PATTERNS, Pattern, SessionPlan, beat_seconds, pattern_by_id, sixteenth_seconds};
use crate::progress::{HistoryEntry, Progress, level_key};
use crate::score::ScoreReport;
use crate::session::{RunData, Window};

pub const SCORED_SECONDS: f64 = 120.0;
pub const CALIBRATION_BPM: u32 = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Menu,
    Settings,
    Play,
    Results,
}

#[derive(Clone, Copy, Debug)]
pub struct LiveHit {
    /// Session time of the onset.
    pub t: f64,
    pub level_db: f64,
    /// Nearest grid note and the latency-compensated deviation (ms).
    pub note: Option<usize>,
    pub dev_ms: f64,
}

pub struct PlayState {
    pub pattern: &'static Pattern,
    pub plan: SessionPlan,
    pub calibration: bool,
    pub click_start_frame: u64,
    /// input frame = output frame + delta
    pub delta_frames: f64,
    pub timestamps_used: bool,
    pub live: LiveDetector,
    pub hits: Vec<LiveHit>,
    /// Per grid note: Some(dev ms) when hit live.
    pub note_dev: Vec<Option<f64>>,
    raw_devs: Vec<f64>,
    pub ended_early: Option<f64>,
}

impl PlayState {
    pub fn now(&self, engine: &AudioEngine) -> f64 {
        let f = engine.shared.out_frames.load(Ordering::Relaxed) as f64;
        (f - self.click_start_frame as f64) / f64::from(SAMPLE_RATE)
    }
    pub fn live_latency(&self) -> f64 {
        let n = self.raw_devs.len();
        if n < 4 {
            return 0.0;
        }
        crate::dsp::median(&self.raw_devs[n.saturating_sub(24)..]).unwrap_or(0.0)
    }
    pub fn window(&self) -> f64 {
        0.4 * sixteenth_seconds(self.plan.bpm)
    }
}

pub struct ResultsState {
    pub run: RunData,
    pub rep: ScoreReport,
    #[allow(dead_code)]
    pub onsets: Vec<Onset>,
    pub worst: Vec<Window>,
    pub newly: Vec<String>,
    pub saved: Result<std::path::PathBuf, String>,
    pub prev_best: Option<HistoryEntry>,
    pub listening: Option<usize>,
    pub calibration: bool,
    /// Spectral centroid (Hz) per report note (None if missed).
    pub centroids: Vec<Option<f64>>,
}

#[derive(Default)]
pub struct SettingsLists {
    pub inputs: Vec<DeviceEntry>,
    pub outputs: Vec<DeviceEntry>,
    pub nams: Vec<String>,
    pub irs: Vec<String>,
    pub input_scroll: usize,
}

#[derive(Resource)]
pub struct Game {
    pub screen: Screen,
    pub cfg: Config,
    pub progress: Progress,
    pub engine: Option<AudioEngine>,
    pub engine_err: Option<String>,
    pub meter_db: f32,
    pub meter_peak_db: f32,
    pub meter_hold: f32,
    pub selected: (usize, u32),
    pub play: Option<PlayState>,
    pub results: Option<ResultsState>,
    pub lists: SettingsLists,
    pub message: Option<String>,
}

impl Game {
    pub fn new() -> Self {
        let cfg = Config::load();
        let progress = Progress::load();
        let mut g = Self {
            screen: Screen::Menu,
            cfg,
            progress,
            engine: None,
            engine_err: None,
            meter_db: -90.0,
            meter_peak_db: -90.0,
            meter_hold: 0.0,
            selected: (0, crate::patterns::BASE_BPM),
            play: None,
            results: None,
            lists: SettingsLists::default(),
            message: None,
        };
        g.restart_engine();
        g
    }

    pub fn restart_engine(&mut self) {
        self.engine = None; // release the devices first
        match AudioEngine::start(&self.cfg.audio, &self.cfg.tone) {
            Ok(e) => {
                self.engine_err = None;
                self.engine = Some(e);
            }
            Err(e) => {
                eprintln!("audio: {e}");
                self.engine_err = Some(e);
            }
        }
    }

    pub fn rebuild_chain(&mut self) {
        if let Some(e) = &self.engine {
            e.set_chain(Chain::build(&self.cfg.tone, SAMPLE_RATE));
        }
        self.cfg.save();
    }

    pub fn chain_description(&self) -> String {
        self.engine
            .as_ref()
            .and_then(|e| e.shared.chain.lock().ok().and_then(|c| c.as_ref().map(|c| c.description.clone())))
            .unwrap_or_default()
    }

    pub fn refresh_lists(&mut self) {
        self.lists.inputs = crate::audio::list_devices(true);
        self.lists.outputs = crate::audio::list_devices(false);
        self.lists.nams = crate::monitor::list_assets(&self.cfg.tone.asset_dir, ".nam");
        self.lists.irs = crate::monitor::list_assets(&self.cfg.tone.asset_dir, ".wav");
    }

    pub fn update_meter(&mut self, dt: f32) {
        let Some(e) = &self.engine else { return };
        let p = e.shared.in_peak.take();
        let db = 20.0 * (p + 1e-9).log10();
        // fast attack, ~20 dB/s release
        self.meter_db = if db > self.meter_db { db } else { (self.meter_db - 20.0 * dt).max(db) };
        if db >= self.meter_peak_db {
            self.meter_peak_db = db;
            self.meter_hold = 1.5;
        } else {
            self.meter_hold -= dt;
            if self.meter_hold < 0.0 {
                self.meter_peak_db -= 10.0 * dt;
            }
        }
    }

    pub fn start_level(&mut self, pattern: &'static Pattern, bpm: u32, calibration: bool) {
        let Some(engine) = &self.engine else {
            self.message = Some("No audio engine - check Settings".into());
            return;
        };
        let (count_in, seconds) = if calibration { (1, 8.0 * crate::patterns::bar_seconds(bpm)) } else { (self.cfg.count_in(), SCORED_SECONDS) };
        let plan = SessionPlan::new(pattern, bpm, count_in, seconds);
        engine.start_recording();
        let sr = f64::from(SAMPLE_RATE);
        let start = engine.shared.out_frames.load(Ordering::Relaxed) + (0.4 * sr) as u64;
        let (in_f, ts) = engine.out_to_in_frame(start);
        let delta = in_f - start as f64;
        engine.set_click(Some(ClickPlan {
            start_frame: start,
            frames_per_beat: beat_seconds(bpm) * sr,
            beats: plan.total_beats(),
            beats_per_bar: BEATS_PER_BAR,
        }));
        let n = plan.notes.len();
        self.play = Some(PlayState {
            pattern,
            live: LiveDetector::new(DetectParams { sample_rate: sr, sixteenth: sixteenth_seconds(bpm), dynamic_range_db: 45.0 }),
            plan,
            calibration,
            click_start_frame: start,
            delta_frames: delta,
            timestamps_used: ts,
            hits: Vec::new(),
            note_dev: vec![None; n],
            raw_devs: Vec::new(),
            ended_early: None,
        });
        self.results = None;
        self.screen = Screen::Play;
    }

    /// Recording time of session time 0 (seconds), without manual calibration.
    fn base_offset(&self, play: &PlayState, rec_start: u64) -> f64 {
        (play.click_start_frame as f64 + play.delta_frames - rec_start as f64) / f64::from(SAMPLE_RATE)
    }

    pub fn abort_play(&mut self) {
        if let Some(e) = &self.engine {
            e.set_click(None);
            e.stop_recording();
        }
        self.play = None;
        self.screen = Screen::Menu;
    }

    /// Per-frame play logic: live detection and end-of-session handling.
    pub fn tick_play(&mut self) {
        let (Some(engine), Some(play)) = (&self.engine, &mut self.play) else { return };
        let now = play.now(engine);
        let calib_s = if play.calibration { 0.0 } else { self.cfg.latency_ms.unwrap_or(0.0) / 1e3 };
        // Live onsets
        let new = {
            let rec = engine.recording.lock().unwrap();
            if rec.samples.is_empty() {
                Vec::new()
            } else {
                let off = (play.click_start_frame as f64 + play.delta_frames - rec.start_frame as f64) / f64::from(SAMPLE_RATE);
                play.live.update(&rec.samples).into_iter().map(|o| (o, off)).collect::<Vec<_>>()
            }
        };
        let win = play.window();
        for (o, off) in new {
            let t = o.time - off - calib_s;
            let notes = &play.plan.notes;
            let i = notes.partition_point(|n| n.time < t);
            let cand = [i.wrapping_sub(1), i]
                .into_iter()
                .filter(|&j| j < notes.len())
                .min_by(|&a, &b| (notes[a].time - t).abs().total_cmp(&(notes[b].time - t).abs()));
            let mut hit = LiveHit { t, level_db: o.level_db, note: None, dev_ms: 0.0 };
            if let Some(j) = cand {
                let raw = t - notes[j].time;
                if raw.abs() < win {
                    play.raw_devs.push(raw);
                }
                let dev = raw - play.live_latency();
                if dev.abs() <= win && play.note_dev[j].is_none() {
                    play.note_dev[j] = Some(dev * 1e3);
                    hit.note = Some(j);
                    hit.dev_ms = dev * 1e3;
                }
            }
            play.hits.push(hit);
        }
        let end = play.ended_early.unwrap_or(play.plan.total_seconds() + 0.3);
        if now >= end {
            self.finish_play();
        }
    }

    pub fn end_early(&mut self) {
        let (Some(engine), Some(play)) = (&self.engine, &mut self.play) else { return };
        let now = play.now(engine);
        let bar = crate::patterns::bar_seconds(play.plan.bpm);
        let start = play.plan.scoring_start();
        let done_bars = ((now - start) / bar).floor().max(0.0) as u32;
        if done_bars < 2 {
            self.abort_play();
            return;
        }
        // Keep only fully played bars.
        play.plan.notes.retain(|n| n.bar < done_bars);
        play.note_dev.truncate(play.plan.notes.len());
        play.plan.scored_bars = done_bars;
        play.ended_early = Some(now);
        engine.set_click(None);
        self.finish_play();
    }

    fn finish_play(&mut self) {
        let Some(play) = self.play.take() else { return };
        let Some(engine) = &self.engine else { return };
        engine.set_click(None);
        let (rec_start, samples) = engine.stop_recording();
        let base = self.base_offset(&play, rec_start);
        let calib = if play.calibration { None } else { self.cfg.latency_ms };
        let run = RunData {
            pattern: play.pattern,
            plan: play.plan,
            sample_rate: SAMPLE_RATE,
            samples,
            grid_offset_s: base + calib.unwrap_or(0.0) / 1e3,
            calibrated: calib.is_some(),
            timestamps_used: play.timestamps_used,
            stream_latency_ms: play.delta_frames / f64::from(SAMPLE_RATE) * 1e3,
        };
        let (rep, onsets) = run.analyse();
        let worst = run.worst_windows(&rep, &onsets, 3);
        let centroids = rep
            .notes
            .iter()
            .map(|n| {
                n.onset_time.and_then(|t| {
                    crate::score::centroids(&run.samples, f64::from(run.sample_rate), &[t]).first().copied()
                })
            })
            .collect();
        let saved = run.save(&rep, &onsets, &worst);
        if let Err(e) = &saved {
            eprintln!("saving run failed: {e}");
        }
        let key = level_key(run.pattern.id, run.plan.bpm);
        let prev_best = self.progress.best_entry(&key).cloned();
        let mut newly = Vec::new();
        if play.calibration {
            if rep.n_matched >= 6 {
                self.cfg.latency_ms = Some(rep.latency_ms);
                self.cfg.save();
            }
        } else {
            newly = self.progress.record(run.pattern, run.plan.bpm, rep.combined_pct);
            self.progress.fresh.remove(&key);
            self.progress.history.push(HistoryEntry {
                key,
                date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
                combined: rep.combined_pct,
                timing: rep.timing_pct,
                tone: rep.tone_pct,
                spread_ms: rep.spread_ms,
                level_std_db: rep.level_std_detrended_db,
                mean_offset_ms: rep.mean_offset_ms,
                misses: rep.n_missed,
                seconds: run.plan.total_seconds(),
            });
            self.progress.save();
        }
        self.results = Some(ResultsState {
            run,
            rep,
            onsets,
            worst,
            newly,
            saved,
            prev_best,
            listening: None,
            calibration: play.calibration,
            centroids,
        });
        self.screen = Screen::Results;
    }

    pub fn listen(&mut self, i: usize) {
        let (Some(engine), Some(r)) = (&self.engine, &mut self.results) else { return };
        let Some(w) = r.worst.get(i) else { return };
        let gain = 10f32.powf(self.cfg.audio.click_volume_db / 20.0);
        let audio = r.run.render_listen(w, &self.cfg.tone, gain, r.rep.latency_ms);
        engine.play(audio);
        r.listening = Some(i);
    }

    pub fn stop_listen(&mut self) {
        if let Some(e) = &self.engine {
            e.stop_playback();
        }
        if let Some(r) = &mut self.results {
            r.listening = None;
        }
    }

    pub fn selected_pattern(&self) -> &'static Pattern {
        &PATTERNS[self.selected.0.min(PATTERNS.len() - 1)]
    }

    pub fn retry(&mut self) {
        if let Some(r) = &self.results {
            let (p, b, c) = (r.run.pattern, r.run.plan.bpm, r.calibration);
            self.stop_listen();
            self.start_level(p, b, c);
        }
    }
}

pub fn calibration_pattern() -> &'static Pattern {
    pattern_by_id("quarters").expect("quarters pattern")
}

pub fn game_system(mut game: ResMut<Game>, mut paint: ResMut<Paint>, time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    let g = &mut *game;
    g.update_meter(time.delta_secs());
    match g.screen {
        Screen::Menu => {
            if keys.just_pressed(KeyCode::Escape) {
                exit.write(AppExit::Success);
            }
            crate::ui_menu::draw(g, &mut paint, &keys);
        }
        Screen::Settings => {
            if keys.just_pressed(KeyCode::Escape) {
                g.screen = Screen::Menu;
            }
            crate::ui_settings::draw(g, &mut paint);
        }
        Screen::Play => {
            if keys.just_pressed(KeyCode::Escape) {
                g.abort_play();
                return;
            }
            g.tick_play();
            if g.screen == Screen::Play {
                crate::ui_play::draw(g, &mut paint);
            }
        }
        Screen::Results => {
            if keys.just_pressed(KeyCode::Escape) {
                g.stop_listen();
                g.screen = Screen::Menu;
            } else if keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::KeyR) {
                g.retry();
            }
            crate::ui_results::draw(g, &mut paint);
        }
    }
}
