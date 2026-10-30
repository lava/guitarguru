//! Game state and screen flow. Each screen is an immediate-mode function drawing with
//! [`Paint`] from the state in [`Game`].

use std::sync::atomic::Ordering;

use bevy::prelude::*;

use crate::audio::{AudioEngine, ClickPlan, DeviceEntry, SAMPLE_RATE};
use crate::config::Config;
use crate::countin::CountInGate;
use crate::detect::Onset;
use crate::monitor::Chain;
use crate::paint::Paint;
use crate::patterns::{BEATS_PER_BAR, Level, PATTERNS, SessionPlan, beat_seconds};
use crate::progress::{HistoryEntry, Progress};
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

pub struct PlayState {
    pub level: Level,
    pub plan: SessionPlan,
    pub calibration: bool,
    pub click_start_frame: u64,
    /// input frame = output frame + delta
    pub delta_frames: f64,
    pub timestamps_used: bool,
    pub ended_early: Option<f64>,
    /// Open-ended count-in; None once the player has caught 4 clicks in a row.
    pub gate: Option<CountInGate>,
    pub countin_seed_ms: Option<f64>,
    last_gate_check: f64,
}

impl PlayState {
    pub fn waiting(&self) -> bool {
        self.gate.is_some()
    }

    pub fn now(&self, engine: &AudioEngine) -> f64 {
        let f = engine.shared.out_frames.load(Ordering::Relaxed) as f64;
        (f - self.click_start_frame as f64) / f64::from(SAMPLE_RATE)
    }
}

pub struct ResultsState {
    pub run: RunData,
    pub rep: ScoreReport,
    #[allow(dead_code)]
    pub onsets: Vec<Onset>,
    pub worst: Vec<Window>,
    pub newly: Vec<crate::progress::Unlock>,
    pub saved: Result<std::path::PathBuf, String>,
    pub prev_best: Option<HistoryEntry>,
    /// Set when this run's patterns were notably harder or easier than the previous best's.
    pub difficulty_note: Option<String>,
    pub listening: Option<usize>,
    pub calibration: bool,
    /// A song to try, suggested for the first time by this pass.
    pub song: Option<&'static crate::songs::Song>,
    /// Height of the results page as last drawn (design px).
    pub content_h: Option<f32>,
}

#[derive(Default)]
pub struct SettingsLists {
    pub inputs: Vec<DeviceEntry>,
    pub outputs: Vec<DeviceEntry>,
    pub nams: Vec<String>,
    pub irs: Vec<String>,
    pub input_scroll: usize,
    pub output_scroll: usize,
    pub cab_scroll: usize,
    pub show_all: bool,
}

#[derive(Resource)]
pub struct Game {
    pub screen: Screen,
    pub cfg: Config,
    pub progress: Progress,
    pub engine: Option<AudioEngine>,
    pub engine_err: Option<String>,
    pub engine_started: std::time::Instant,
    pub meter_db: f32,
    pub meter_peak_db: f32,
    pub meter_hold: f32,
    pub selected: Level,
    /// Ingredient toggles on the main screen (bit mask).
    pub mix: u8,
    /// Seconds since start (UI animation clock).
    pub clock: f64,
    pub last_screen: Screen,
    pub screen_since: f64,
    /// Vertical scroll of the current screen (design px, >= 0).
    pub scroll: f32,
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
            engine_started: std::time::Instant::now(),
            meter_db: -90.0,
            meter_peak_db: -90.0,
            meter_hold: 0.0,
            selected: Level::run(0, crate::patterns::BASE_BPM),
            mix: 0,
            clock: 0.0,
            last_screen: Screen::Menu,
            screen_since: 0.0,
            scroll: 0.0,
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
        self.engine_started = std::time::Instant::now();
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

    /// A status line about the input when something is wrong: open error, adapter missing,
    /// or an open stream that delivers no data (e.g. held exclusively elsewhere).
    pub fn input_problem(&self) -> Option<(String, bool)> {
        if let Some(e) = &self.engine_err {
            return Some((e.clone(), true));
        }
        let e = self.engine.as_ref()?;
        if self.engine_started.elapsed().as_secs_f32() > 2.0 && e.shared.in_frames.load(Ordering::Relaxed) == 0 {
            let why = crate::audio::alsa_card_with("BANDFUSE")
                .and_then(|c| crate::audio::alsa_capture_owner(&c))
                .filter(|(_, comm)| comm != "pipewire" && comm != "wireplumber")
                .map(|(pid, comm)| format!(": BANDFUSE opened directly by {comm} (pid {pid})"))
                .unwrap_or_else(|| " - the adapter may be held exclusively by another program".into());
            return Some((format!("No data from the input{why}"), true));
        }
        e.info.warning.clone().map(|w| (w, false))
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
        self.lists.inputs = crate::audio::list_devices(true, self.lists.show_all);
        self.lists.outputs = crate::audio::list_devices(false, self.lists.show_all);
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

    pub fn start_level(&mut self, level: Level, calibration: bool) {
        let bpm = level.bpm;
        let Some(engine) = &self.engine else {
            self.message = Some("No audio engine - check Settings".into());
            return;
        };
        let seconds = if calibration { 8.0 * crate::patterns::bar_seconds(bpm) } else { SCORED_SECONDS };
        // Placeholder plan: the real count-in length is set when the count-in gate opens.
        // A fresh draw every time, passing over the patterns of the last runs of this level.
        let avoid = self.progress.recent_patterns(&level.key());
        let seq = level.sequence(crate::patterns::scored_bars(bpm, seconds), crate::generator::random_seed(), &avoid);
        let plan = SessionPlan::from_sequence(bpm, 1, seq, level.pm_on());
        engine.start_recording();
        engine.start_clock_log();
        let sr = f64::from(SAMPLE_RATE);
        let start = engine.shared.out_frames.load(Ordering::Relaxed) + (0.4 * sr) as u64;
        let (in_f, ts) = engine.out_to_in_frame(start);
        let delta = in_f - start as f64;
        engine.set_click(Some(ClickPlan {
            start_frame: start,
            frames_per_beat: beat_seconds(bpm) * sr,
            beats: u32::MAX / 4, // until the count-in gate opens
            beats_per_bar: BEATS_PER_BAR,
        }));
        let allowance = if calibration { 0.0 } else { self.cfg.latency_ms.unwrap_or(0.0) / 1e3 };
        self.play = Some(PlayState {
            gate: Some(CountInGate::new(beat_seconds(bpm), allowance)),
            countin_seed_ms: None,
            last_gate_check: 0.0,
            level,
            plan,
            calibration,
            click_start_frame: start,
            delta_frames: delta,
            timestamps_used: ts,
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

    /// Per-frame play logic: end the session after the last bar. No live analysis: the
    /// take is scored afterwards so the global input lag can be estimated over the whole run.
    pub fn tick_play(&mut self) {
        let (Some(engine), Some(play)) = (&self.engine, &mut self.play) else { return };
        let now = play.now(engine);
        if play.gate.is_some() {
            if now - play.last_gate_check < 0.05 {
                return;
            }
            play.last_gate_check = now;
            let sr = f64::from(SAMPLE_RATE);
            let bpm = play.plan.bpm;
            let onsets: Vec<f64> = {
                let rec = engine.recording.lock().unwrap();
                let n = rec.samples.len();
                let a = n.saturating_sub((1.5 * sr) as usize);
                let off = (play.click_start_frame as f64 + play.delta_frames - rec.start_frame as f64) / sr;
                crate::countin::tail_onsets(&rec.samples[a..], sr, crate::patterns::sixteenth_seconds(bpm))
                    .into_iter()
                    .map(|t| t + a as f64 / sr - off)
                    .collect()
            };
            if let Some(seed) = play.gate.as_mut().and_then(|g| g.update(&onsets, now)) {
                // Start at the next bar line.
                let bar = crate::patterns::bar_seconds(bpm);
                let bars = ((now + 0.05) / bar).ceil().max(1.0) as u32;
                // Same patterns as the placeholder plan, now after the real count-in.
                play.plan = SessionPlan::from_sequence(bpm, bars, play.plan.sequence(), play.level.pm_on());
                play.gate = None;
                play.countin_seed_ms = Some(seed * 1e3);
                engine.set_click(Some(ClickPlan {
                    start_frame: play.click_start_frame,
                    frames_per_beat: beat_seconds(bpm) * sr,
                    beats: play.plan.total_beats(),
                    beats_per_bar: BEATS_PER_BAR,
                }));
            }
            return;
        }
        let end = play.ended_early.unwrap_or(play.plan.total_seconds() + 0.3);
        if now >= end {
            self.finish_play();
        }
    }

    pub fn end_early(&mut self) {
        let (Some(engine), Some(play)) = (&self.engine, &mut self.play) else { return };
        if play.waiting() {
            self.abort_play();
            return;
        }
        let now = play.now(engine);
        let bar = crate::patterns::bar_seconds(play.plan.bpm);
        let start = play.plan.scoring_start();
        let done_bars = ((now - start) / bar).floor().max(0.0) as u32;
        if done_bars < 2 {
            self.abort_play();
            return;
        }
        // Keep only fully played bars.
        play.plan.truncate(done_bars);
        play.ended_early = Some(now);
        engine.set_click(None);
        self.finish_play();
    }

    fn finish_play(&mut self) {
        let Some(play) = self.play.take() else { return };
        let Some(engine) = &self.engine else { return };
        engine.set_click(None);
        let (rec_start, samples) = engine.stop_recording();
        let (in_stamps, out_stamps) = engine.take_clock_log(25);
        let clock = crate::session::ClockLog {
            in_stamps,
            out_stamps,
            input_drops: engine.shared.xruns.load(Ordering::Relaxed),
            recorded_s: samples.len() as f64 / f64::from(SAMPLE_RATE),
            session_s: play.plan.total_seconds(),
        };
        let base = self.base_offset(&play, rec_start);
        let calib = if play.calibration { None } else { self.cfg.latency_ms };
        let run = RunData {
            level: play.level,
            plan: play.plan,
            sample_rate: SAMPLE_RATE,
            samples,
            grid_offset_s: base + calib.unwrap_or(0.0) / 1e3,
            calibrated: calib.is_some(),
            timestamps_used: play.timestamps_used,
            stream_latency_ms: play.delta_frames / f64::from(SAMPLE_RATE) * 1e3,
            countin_latency_ms: play.countin_seed_ms,
            clock,
        };
        let (rep, onsets) = run.analyse();
        let worst = run.worst_windows(&rep, &onsets, 3);
        let saved = run.save(&rep, &onsets, &worst);
        if let Err(e) = &saved {
            eprintln!("saving run failed: {e}");
        }
        let key = run.level.key();
        let prev_best = self.progress.best_entry(&key).cloned();
        let difficulty = run.avg_difficulty();
        let mut newly = Vec::new();
        let mut song = None;
        if play.calibration {
            if rep.n_matched >= 6 {
                self.cfg.latency_ms = Some(rep.latency_ms);
                self.cfg.save();
            }
        } else {
            newly = self.progress.record(&run.level, rep.combined_pct);
            if rep.combined_pct >= crate::progress::PASS_THRESHOLD {
                song = crate::songs::suggest(&mut self.progress, &run.level);
            }
            self.progress.history.push(HistoryEntry {
                key: key.clone(),
                date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
                combined: rep.combined_pct,
                timing: rep.timing_pct,
                tone: rep.tone_pct,
                spread_ms: rep.spread_ms,
                level_std_db: rep.level_std_detrended_db,
                mean_offset_ms: rep.mean_offset_ms,
                misses: rep.n_missed,
                seconds: run.plan.total_seconds(),
                difficulty: Some(difficulty),
            });
            if run.level.is_run() {
                self.progress.remember_patterns(&key, run.patterns_used().into_iter().map(|p| run.plan.patterns[p].id.to_string()).collect());
            }
            self.progress.save();
        }
        self.results = Some(ResultsState {
            run,
            rep,
            onsets,
            worst,
            newly,
            saved,
            difficulty_note: if play.calibration { None } else { crate::progress::difficulty_note(difficulty, prev_best.as_ref()) },
            prev_best,
            listening: None,
            calibration: play.calibration,
            song,
            content_h: None,
        });
        self.screen = Screen::Results;
    }

    /// Show the results of a saved run (re-analysed; nothing is saved or recorded). For
    /// checking the results screen against real takes (`GG_RESULTS`).
    pub fn show_saved(&mut self, json: &std::path::Path) {
        let (run, _) = match crate::session::load_run(json) {
            Ok(r) => r,
            Err(e) => {
                self.message = Some(format!("cannot load {}: {e}", json.display()));
                return;
            }
        };
        let (rep, onsets) = run.analyse();
        let worst = run.worst_windows(&rep, &onsets, 3);
        let prev_best = self.progress.best_entry(&run.level.key()).cloned();
        self.results = Some(ResultsState {
            run,
            rep,
            onsets,
            worst,
            newly: Vec::new(),
            saved: Ok(json.with_extension("wav")),
            prev_best,
            difficulty_note: None,
            listening: None,
            calibration: false,
            song: None,
            content_h: None,
        });
        self.screen = Screen::Results;
    }

    pub fn listen(&mut self, i: usize) {
        let (Some(engine), Some(r)) = (&self.engine, &mut self.results) else { return };
        let Some(w) = r.worst.get(i) else { return };
        // Replay: raw DI a bit quieter, click 4 dB louder than live, so the timing relation
        // is clearly audible.
        let gain = 10f32.powf((self.cfg.audio.click_volume_db + 4.0) / 20.0);
        let audio = r.run.render_listen(w, gain, r.rep.level_median_db);
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

    pub fn retry(&mut self) {
        if let Some(r) = &self.results {
            let (l, c) = (r.run.level, r.calibration);
            self.stop_listen();
            self.start_level(l, c);
        }
    }
}

pub fn calibration_level() -> Level {
    Level::drill(PATTERNS.iter().position(|p| p.id == "quarters").expect("quarters pattern"), CALIBRATION_BPM)
}

#[allow(clippy::too_many_arguments)]
pub fn game_system(
    mut game: ResMut<Game>,
    mut paint: ResMut<Paint>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mascot: Option<Res<crate::mascot::MascotImages>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mascot) = mascot else { return };
    let g = &mut *game;
    g.update_meter(time.delta_secs());
    g.clock += f64::from(time.delta_secs());
    // Calm ease-out fade on every screen change.
    if g.screen != g.last_screen {
        g.last_screen = g.screen;
        g.screen_since = g.clock;
        g.scroll = 0.0;
    }
    let t = ((g.clock - g.screen_since) / 0.35).clamp(0.0, 1.0) as f32;
    paint.fade = 1.0 - (1.0 - t).powi(3);
    let mascot = &*mascot;
    match g.screen {
        Screen::Menu => {
            if keys.just_pressed(KeyCode::Escape) {
                exit.write(AppExit::Success);
            }
            crate::ui_menu::draw(g, &mut paint, &keys, mascot);
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
            if keys.just_pressed(KeyCode::Enter) {
                g.end_early();
                return;
            }
            g.tick_play();
            if g.screen == Screen::Play {
                crate::ui_play::draw(g, &mut paint, mascot);
            }
        }
        Screen::Results => {
            if keys.just_pressed(KeyCode::Escape) {
                g.stop_listen();
                g.screen = Screen::Menu;
            } else if keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::KeyR) {
                g.retry();
            }
            crate::ui_results::draw(g, &mut paint, mascot);
        }
    }
}
