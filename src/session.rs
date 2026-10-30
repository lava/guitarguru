//! A finished run: analysis, worst-section search, listen-back rendering and saving.
//! Independent of Bevy so it can be tested headlessly.

use std::path::PathBuf;

use serde::Serialize;

use crate::audio::{ClickPlan, render_click};
use crate::detect::{DetectParams, Onset, detect};
use crate::monitor::{Chain, ToneSettings};
use crate::patterns::{BEATS_PER_BAR, GridNote, Level, PATTERNS, SessionPlan, bar_seconds, beat_seconds, sixteenth_seconds};
use crate::progress::data_dir;
use crate::score::{GridIn, ScoreInput, ScoreReport, score};

pub struct RunData {
    pub level: Level,
    pub plan: SessionPlan,
    pub sample_rate: u32,
    /// Raw DI, starting at the recording start.
    pub samples: Vec<f32>,
    /// Recording time = session time + `grid_offset_s` (includes the measured stream
    /// latency and the manual calibration, if any).
    pub grid_offset_s: f64,
    pub calibrated: bool,
    pub timestamps_used: bool,
    pub stream_latency_ms: f64,
    /// Median offset of the 4 count-in hits that opened the gate (latency seed), ms.
    pub countin_latency_ms: Option<f64>,
    /// Stream diagnostics: (frame, timestamp ns) samples of both streams, input drops.
    pub clock: ClockLog,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ClockLog {
    pub in_stamps: Vec<(u64, u128)>,
    pub out_stamps: Vec<(u64, u128)>,
    pub input_drops: u32,
    /// Recording length vs the session's own length (seconds), to spot lost samples.
    pub recorded_s: f64,
    pub session_s: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Window {
    pub start_bar: u32,
    pub bars: u32,
    /// Mean error of the window (ms-equivalent; see `note_error`).
    pub error: f64,
    /// Indices into `ScoreReport::notes`.
    pub notes: Vec<usize>,
    pub n_extra: usize,
    /// Pattern played in this window (index into `PATTERNS`); windows never span a switch.
    pub pattern: usize,
    pub pattern_id: &'static str,
    /// Local offset (latency + wander) of the window's notes, ms, relative to the grid.
    pub offset_ms: f64,
}

impl RunData {
    pub fn detect_params(&self) -> DetectParams {
        DetectParams { sample_rate: f64::from(self.sample_rate), sixteenth: sixteenth_seconds(self.plan.bpm), dynamic_range_db: 45.0 }
    }

    pub fn analyse(&self) -> (ScoreReport, Vec<Onset>) {
        let onsets = detect(&self.samples, &self.detect_params());
        let rep = self.score_onsets(&onsets);
        (rep, onsets)
    }

    /// Patterns used in this run, in order of first appearance.
    pub fn patterns_used(&self) -> Vec<usize> {
        let mut v: Vec<usize> = Vec::new();
        for &p in &self.plan.bar_patterns {
            if !v.contains(&p) {
                v.push(p);
            }
        }
        v
    }

    /// Flat slot table: one entry per (pattern, slot within the pattern's bar). Per-slot
    /// statistics are per pattern, since a slot means something different in each.
    pub fn slot_table(&self) -> Vec<(usize, usize)> {
        self.patterns_used().into_iter().flat_map(|p| (0..PATTERNS[p].hits().len()).map(move |s| (p, s))).collect()
    }

    pub fn score_onsets(&self, onsets: &[Onset]) -> ScoreReport {
        let table = self.slot_table();
        let flat = |p: usize, s: usize| table.iter().position(|&e| e == (p, s)).unwrap_or(0);
        let grid: Vec<GridIn> =
            self.plan.notes.iter().map(|n| GridIn { time: n.time + self.grid_offset_s, slot: flat(n.pattern, n.slot), pm: n.pm }).collect();
        let min_len = self
            .patterns_used()
            .iter()
            .flat_map(|&p| PATTERNS[p].hits())
            .map(|n| n.len)
            .fold(f64::INFINITY, f64::min);
        let mut rep = score(&ScoreInput {
            grid: &grid,
            onsets,
            sixteenth: sixteenth_seconds(self.plan.bpm),
            slots_per_bar: table.len(),
            min_ioi: min_len * sixteenth_seconds(self.plan.bpm),
            calibrated: self.calibrated,
            audio: Some((&self.samples, f64::from(self.sample_rate))),
        });
        for st in &mut rep.slots {
            if let Some(&(p, s)) = table.get(st.slot) {
                st.pattern = PATTERNS[p].id.to_string();
                st.slot_in_pattern = s;
                st.pos16 = PATTERNS[p].hits()[s].pos;
            }
        }
        rep
    }

    /// Bars per listen-back window: two bars, or one when bars are long.
    pub fn window_bars(&self) -> u32 {
        if bar_seconds(self.plan.bpm) > 3.5 { 1 } else { 2 }
    }

    /// The `n` worst non-overlapping windows by mean per-note error.
    pub fn worst_windows(&self, rep: &ScoreReport, onsets: &[Onset], n: usize) -> Vec<Window> {
        let wb = self.window_bars();
        let nslots = self.slot_table().len().max(1);
        // Mean level per slot, for tone error.
        let mut slot_lv = vec![(0.0, 0usize); nslots];
        for note in &rep.notes {
            if let Some(l) = note.level_db {
                slot_lv[note.slot].0 += l;
                slot_lv[note.slot].1 += 1;
            }
        }
        let slot_mean: Vec<f64> = slot_lv.iter().map(|(s, c)| if *c > 0 { s / *c as f64 } else { 0.0 }).collect();
        let note_err = |i: usize| {
            let note = &rep.notes[i];
            match (note.dev_ms, note.level_db) {
                (Some(d), Some(l)) => d.abs() + 5.0 * (l - slot_mean[note.slot]).abs(),
                _ => 60.0,
            }
        };
        let matched: std::collections::HashSet<u64> =
            rep.notes.iter().filter_map(|n| n.onset_time).map(f64::to_bits).collect();
        let quiet = rep.level_median_db - crate::score::EXTRA_LEVEL_MARGIN_DB;
        let bar_s = bar_seconds(self.plan.bpm);
        let mut cands = Vec::new();
        let mut start = 0;
        while start + wb <= self.plan.scored_bars {
            // Windows stay within one segment (one pattern in a run; a multi-bar riff in a drill).
            let pat = self.plan.bar_patterns[start as usize];
            let seg = start / crate::patterns::SEGMENT_BARS;
            let same_seg = (start..start + wb).all(|b| b / crate::patterns::SEGMENT_BARS == seg);
            let same_pat = (start..start + wb).all(|b| self.plan.bar_patterns[b as usize] == pat);
            if !(same_pat || (!self.level.is_run() && same_seg)) {
                start += 1;
                continue;
            }
            let idx: Vec<usize> = (0..rep.notes.len())
                .filter(|&i| self.plan.notes[i].bar >= start && self.plan.notes[i].bar < start + wb)
                .collect();
            if !idx.is_empty() {
                let offs: Vec<f64> = idx.iter().map(|&i| rep.notes[i].offset_ms).collect();
                let offset_ms = crate::dsp::median(&offs).unwrap_or(rep.latency_ms);
                let t0 = self.plan.scoring_start() + f64::from(start) * bar_s + self.grid_offset_s + offset_ms / 1e3;
                let t1 = t0 + f64::from(wb) * bar_s;
                let w = 0.4 * sixteenth_seconds(self.plan.bpm);
                let extra = onsets
                    .iter()
                    .filter(|o| o.time >= t0 - w && o.time < t1 - w && !matched.contains(&o.time.to_bits()) && o.level_db >= quiet)
                    .count();
                let err = (idx.iter().map(|&i| note_err(i)).sum::<f64>() + 30.0 * extra as f64) / idx.len() as f64;
                cands.push(Window { start_bar: start, bars: wb, error: err, notes: idx, n_extra: extra, pattern: pat, pattern_id: PATTERNS[pat].id, offset_ms });
            }
            start += 1;
        }
        cands.sort_by(|a, b| b.error.partial_cmp(&a.error).unwrap_or(std::cmp::Ordering::Equal));
        let mut out: Vec<Window> = Vec::new();
        for c in cands {
            if out.iter().all(|o| c.start_bar + c.bars <= o.start_bar || o.start_bar + o.bars <= c.start_bar) {
                out.push(c);
                if out.len() == n {
                    break;
                }
            }
        }
        out
    }

    /// Session-time span of a window plus a lead-in beat and a short tail.
    pub fn window_span(&self, w: &Window) -> (f64, f64) {
        let bar_s = bar_seconds(self.plan.bpm);
        let t0 = self.plan.scoring_start() + f64::from(w.start_bar) * bar_s;
        (t0 - beat_seconds(self.plan.bpm), t0 + f64::from(w.bars) * bar_s + 0.5 * beat_seconds(self.plan.bpm))
    }

    /// Render a window of the DI through the amp chain, with the click mixed in.
    /// The click is placed where the player heard it: shifted by the window's local offset
    /// (latency and slow wander as tracked by the scorer).
    pub fn render_listen(&self, w: &Window, tone: &ToneSettings, click_gain: f32) -> Vec<f32> {
        let sr = f64::from(self.sample_rate);
        let (s0, s1) = self.window_span(w);
        let shift = self.grid_offset_s + w.offset_ms / 1e3;
        let a = ((s0 + shift) * sr).max(0.0) as usize;
        let b = (((s1 + shift) * sr) as usize).min(self.samples.len());
        if a >= b {
            return Vec::new();
        }
        let mut chain = Chain::build(tone, self.sample_rate);
        // Warm the amp up on the preceding half second.
        let warm = a.saturating_sub((0.5 * sr) as usize);
        let mut tmp = self.samples[warm..a].to_vec();
        for c in tmp.chunks_mut(256) {
            chain.process(c);
        }
        let mut out = self.samples[a..b].to_vec();
        for c in out.chunks_mut(256) {
            chain.process(c);
        }
        let plan = ClickPlan {
            start_frame: 0,
            frames_per_beat: beat_seconds(self.plan.bpm) * sr,
            beats: self.plan.total_beats(),
            beats_per_bar: BEATS_PER_BAR,
        };
        for (i, v) in out.iter_mut().enumerate() {
            // Session time of this sample, minus the latency the player compensated for.
            let session_t = (a + i) as f64 / sr - shift;
            if session_t >= 0.0 {
                *v += click_gain * render_click(&plan, (session_t * sr) as u64, sr);
            }
        }
        out
    }

    pub fn save(&self, rep: &ScoreReport, onsets: &[Onset], worst: &[Window]) -> Result<PathBuf, String> {
        let dir = data_dir().join("runs");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let base = dir.join(format!("{stamp}_{}", self.level.key().replace('@', "_")));
        let wav = base.with_extension("wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: self.sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(&wav, spec).map_err(|e| e.to_string())?;
        for s in &self.samples {
            w.write_sample(*s).map_err(|e| e.to_string())?;
        }
        w.finalize().map_err(|e| e.to_string())?;
        #[derive(Serialize)]
        struct PatternInfo {
            id: &'static str,
            name: &'static str,
            notation: &'static str,
        }
        #[derive(Serialize)]
        struct SlotInfo {
            pattern: &'static str,
            slot: usize,
            pos16: f64,
        }
        #[derive(Serialize)]
        struct Out<'a> {
            level: String,
            level_name: String,
            /// Patterns used, and the pattern id of every scored bar.
            patterns: Vec<PatternInfo>,
            bar_patterns: Vec<&'static str>,
            /// Flat slot ids used by `report.slots` and `report.notes[].slot`.
            slots: Vec<SlotInfo>,
            bpm: u32,
            sample_rate: u32,
            count_in_bars: u32,
            scored_bars: u32,
            grid_offset_s: f64,
            calibrated: bool,
            timestamps_used: bool,
            stream_latency_ms: f64,
            countin_latency_ms: Option<f64>,
            clock: &'a ClockLog,
            grid: &'a [GridNote],
            onsets: &'a [Onset],
            worst_windows: &'a [Window],
            report: &'a ScoreReport,
        }
        let out = Out {
            level: self.level.key(),
            level_name: self.level.name(),
            patterns: self
                .patterns_used()
                .into_iter()
                .map(|p| PatternInfo { id: PATTERNS[p].id, name: PATTERNS[p].name, notation: PATTERNS[p].notation })
                .collect(),
            bar_patterns: self.plan.bar_patterns.iter().map(|&p| PATTERNS[p].id).collect(),
            slots: self
                .slot_table()
                .into_iter()
                .map(|(p, s)| SlotInfo { pattern: PATTERNS[p].id, slot: s, pos16: PATTERNS[p].hits()[s].pos })
                .collect(),
            bpm: self.plan.bpm,
            sample_rate: self.sample_rate,
            count_in_bars: self.plan.count_in_bars,
            scored_bars: self.plan.scored_bars,
            grid_offset_s: self.grid_offset_s,
            calibrated: self.calibrated,
            timestamps_used: self.timestamps_used,
            stream_latency_ms: self.stream_latency_ms,
            countin_latency_ms: self.countin_latency_ms,
            clock: &self.clock,
            grid: &self.plan.notes,
            onsets,
            worst_windows: worst,
            report: rep,
        };
        let json = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
        std::fs::write(base.with_extension("json"), json).map_err(|e| e.to_string())?;
        Ok(wav)
    }
}

/// Load a saved run (JSON + WAV) for offline re-scoring. Understands the JSON of every
/// earlier version (single `pattern`, tier `level`, and `mix:` levels with `bar_patterns`).
pub fn load_run(json_path: &std::path::Path) -> Result<(RunData, serde_json::Value), String> {
    let text = std::fs::read_to_string(json_path).map_err(|e| e.to_string())?;
    let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let num = |k: &str| j.get(k).and_then(serde_json::Value::as_f64).ok_or(format!("missing {k}"));
    let bpm = num("bpm")? as u32;
    let count_in = num("count_in_bars")? as u32;
    let scored = num("scored_bars")? as usize;
    let pidx = |id: &str| PATTERNS.iter().position(|p| p.id == id);
    let bar_patterns: Vec<usize> = match j.get("bar_patterns").and_then(|v| v.as_array()) {
        Some(a) => a.iter().map(|v| v.as_str().and_then(pidx).ok_or(format!("unknown pattern {v}"))).collect::<Result<_, _>>()?,
        None => {
            let id = j.get("pattern").and_then(|v| v.as_str()).ok_or("no pattern")?;
            vec![pidx(id).ok_or(format!("unknown pattern {id}"))?; scored]
        }
    };
    let level = j
        .get("level")
        .and_then(|v| v.as_str())
        .and_then(Level::from_key)
        .unwrap_or_else(|| if bar_patterns.iter().all(|&p| p == bar_patterns[0]) { Level::drill(bar_patterns[0], bpm) } else { Level::run(0, bpm) });
    let plan = SessionPlan::from_bars(bpm, count_in, bar_patterns, level.pm_on());
    // The saved grid is authoritative for timing: check the rebuilt plan agrees.
    if let Some(g) = j.get("grid").and_then(|v| v.as_array()) {
        let times: Vec<f64> = g.iter().filter_map(|n| n.get("time").and_then(serde_json::Value::as_f64)).collect();
        let same = times.len() == plan.notes.len() && times.iter().zip(&plan.notes).all(|(a, b)| (a - b.time).abs() < 1e-6);
        if !same {
            return Err(format!("rebuilt grid differs from the saved one ({} vs {} notes)", plan.notes.len(), times.len()));
        }
    }
    let wav = json_path.with_extension("wav");
    let mut r = hound::WavReader::open(&wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let sample_rate = r.spec().sample_rate;
    let samples: Vec<f32> = r.samples::<f32>().filter_map(Result::ok).collect();
    let run = RunData {
        level,
        plan,
        sample_rate,
        samples,
        grid_offset_s: num("grid_offset_s")?,
        calibrated: j.get("calibrated").and_then(|v| v.as_bool()).unwrap_or(false),
        timestamps_used: j.get("timestamps_used").and_then(|v| v.as_bool()).unwrap_or(false),
        stream_latency_ms: num("stream_latency_ms").unwrap_or(0.0),
        countin_latency_ms: j.get("countin_latency_ms").and_then(serde_json::Value::as_f64),
        clock: ClockLog::default(),
    };
    let old = j.get("report").cloned().unwrap_or_default();
    Ok((run, old))
}

/// `guitarguru --rescore <run.json>...`: re-analyse saved runs and print old vs new numbers.
pub fn rescore_cli(paths: &[String]) {
    println!("{:<40} {:>5} {:>13} {:>13} {:>9} {:>11} {:>11} {:>11}", "run", "grid", "matched", "extra", "missed", "spread ms", "level dB", "combined");
    for p in paths {
        let path = std::path::Path::new(p);
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        match load_run(path) {
            Ok((run, old)) => {
                let (rep, _) = run.analyse();
                let o = |k: &str| old.get(k).and_then(serde_json::Value::as_f64).unwrap_or(f64::NAN);
                println!(
                    "{:<40} {:>5} {:>6.0} -> {:<4} {:>6.0} -> {:<4} {:>3.0} -> {:<3} {:>4.1} -> {:<4.1} {:>4.1} -> {:<4.1} {:>4.1} -> {:<4.1}",
                    name, rep.n_grid, o("n_matched"), rep.n_matched, o("n_extra"), rep.n_extra, o("n_missed"), rep.n_missed,
                    o("spread_ms"), rep.spread_ms, o("level_std_detrended_db"), rep.level_std_detrended_db, o("combined_pct"), rep.combined_pct
                );
                println!(
                    "    latency {:.0} ms  wander {:.1} ms  total spread {:.1} ms  timing {:.0} %  tone {:.0} %  quiet {}",
                    rep.latency_ms, rep.wander_ms, rep.spread_total_ms, rep.timing_pct, rep.tone_pct, rep.n_quiet_ignored
                );
            }
            Err(e) => println!("{name:<40} skipped: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::tests::{Rng, synth};

    #[test]
    fn full_run_from_synth_signal() {
        let level = Level::drill(PATTERNS.iter().position(|p| p.id == "pursuit").unwrap(), 120);
        let plan = SessionPlan::new(&level, 2, 20.0);
        let offset = 0.250; // recording started 250 ms before the first click (+ latency)
        let mut rng = Rng::new(1);
        // Bar 4 is sloppy: 40 ms random errors.
        let hits: Vec<(f64, f64)> = plan
            .notes
            .iter()
            .map(|n| {
                let sd = if n.bar == 4 { 0.03 } else { 0.004 };
                (n.time + offset + 0.015 + sd * rng.gauss(), -17.0 + 0.5 * rng.gauss())
            })
            .collect();
        let samples = synth(48000.0, plan.total_seconds() + offset + 1.0, &hits, 2);
        let run = RunData { level, plan, sample_rate: 48000, samples, grid_offset_s: offset, calibrated: false, timestamps_used: false, stream_latency_ms: 0.0, countin_latency_ms: None, clock: ClockLog::default() };
        let (rep, onsets) = run.analyse();
        assert!(rep.n_missed <= 4, "{}", rep.n_missed);
        for (n, g) in rep.notes.iter().zip(&run.plan.notes) {
            assert!(n.onset_time.is_some() || g.bar == 4, "miss outside the sloppy bar");
        }
        assert!((rep.latency_ms - 15.0).abs() < 3.0);
        let worst = run.worst_windows(&rep, &onsets, 3);
        assert_eq!(worst.len(), 3);
        assert_eq!(run.window_bars(), 2);
        assert!(worst[0].start_bar == 3 || worst[0].start_bar == 4, "{worst:?}");
        let audio = run.render_listen(&worst[0], &ToneSettings { amp: crate::monitor::AmpChoice::Drive, cab: None, ..Default::default() }, 0.3);
        let expect = (4.0 + 0.5 + 0.25) * 48000.0;
        assert_eq!(worst[0].pattern_id, "pursuit"); // 2 bars + 1 beat lead-in + half-beat tail
        assert!((audio.len() as f64 - expect).abs() < 2.0, "{}", audio.len());
    }

    #[test]
    fn track_run_has_per_pattern_slots_and_clean_windows() {
        let level = Level::run(crate::patterns::PM | crate::patterns::S16, 160);
        let plan = SessionPlan::new(&level, 1, 40.0);
        let mut rng = Rng::new(3);
        let hits: Vec<(f64, f64)> = plan.notes.iter().map(|n| (n.time + 0.2 + 0.01 + 0.004 * rng.gauss(), -18.0)).collect();
        let samples = synth(48000.0, plan.total_seconds() + 1.5, &hits, 7);
        let run = RunData { level, plan, sample_rate: 48000, samples, grid_offset_s: 0.2, calibrated: false, timestamps_used: false, stream_latency_ms: 0.0, countin_latency_ms: None, clock: ClockLog::default() };
        assert!(run.patterns_used().len() >= 2, "{:?}", run.plan.bar_patterns);
        let (rep, onsets) = run.analyse();
        assert_eq!(rep.slots.len(), run.slot_table().len());
        assert!(rep.slots.iter().all(|s| !s.pattern.is_empty()));
        assert!(rep.n_missed <= 2, "{}", rep.n_missed);
        assert_eq!(rep.pm.n + rep.open.n, rep.n_matched);
        assert!(rep.open.n > 0, "quarters segment is open");
        for w in run.worst_windows(&rep, &onsets, 3) {
            assert!(w.notes.iter().all(|&i| run.plan.notes[i].pattern == w.pattern), "runs: one pattern per window");
        }
    }

    /// Regression check on the real takes in the user's data dir (skipped if absent):
    /// detections stay close to the number of notes and few extras are counted.
    #[test]
    #[ignore = "needs real runs in ~/.local/share/guitarguru/runs"]
    fn real_runs_rescore_sanely() {
        let dir = crate::progress::data_dir().join("runs");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Ok((run, _)) = load_run(&p) else { continue };
            let (rep, onsets) = run.analyse();
            if onsets.len() < 10 {
                continue; // silent take
            }
            println!("{}: grid {} onsets {} matched {} extra {} spread {:.1} wander {:.1} combined {:.1}",
                p.display(), rep.n_grid, onsets.len(), rep.n_matched, rep.n_extra, rep.spread_ms, rep.wander_ms, rep.combined_pct);
            assert!((onsets.len() as f64) < 1.3 * rep.n_grid as f64, "too many detections");
            assert!(rep.n_extra * 3 < rep.n_grid, "too many extras");
        }
    }
}
