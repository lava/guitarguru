//! A finished run: analysis, worst-section search, listen-back rendering and saving.
//! Independent of Bevy so it can be tested headlessly.

use std::path::PathBuf;

use serde::Serialize;

use crate::audio::{ClickPlan, render_click};
use crate::detect::{DetectParams, Onset, detect};
use crate::generator::difficulty;
use crate::patterns::{BEATS_PER_BAR, GridNote, Level, PATTERNS, Pattern, Sequence, SessionPlan, bar_seconds, beat_seconds, sixteenth_seconds};
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
    /// The window scored with the run's own formulas (timing from evenness, drift and
    /// hits; tone from the attack-level spread), percent. Windows are ranked by it.
    pub score_pct: f64,
    /// Note-to-note evenness inside the window: sd(IOI error) / sqrt(2), ms (needs three
    /// matched notes).
    pub evenness_ms: Option<f64>,
    /// Mean deviation from the run's global offset, ms (a steady offset in this window).
    pub drift_ms: f64,
    /// Attack-level spread after removing each slot's mean, count-weighted over the
    /// palm-muted and open groups, dB.
    pub level_sd_db: f64,
    /// Indices into `ScoreReport::notes`.
    pub notes: Vec<usize>,
    pub n_missed: usize,
    pub n_extra: usize,
    /// Pattern played in this window (index into the plan's patterns); windows never span
    /// a switch.
    pub pattern: usize,
    pub pattern_id: String,
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

    /// Mean difficulty of the run's bars (see `generator::difficulty`).
    pub fn avg_difficulty(&self) -> f64 {
        let bars = &self.plan.bar_patterns;
        let d: Vec<f64> = self.plan.patterns.iter().map(|p| difficulty(p, self.plan.bpm, self.level.pm_on())).collect();
        if bars.is_empty() { 0.0 } else { bars.iter().map(|&b| d[b]).sum::<f64>() / bars.len() as f64 }
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
        self.patterns_used().into_iter().flat_map(|p| (0..self.plan.patterns[p].hits().len()).map(move |s| (p, s))).collect()
    }

    pub fn score_onsets(&self, onsets: &[Onset]) -> ScoreReport {
        let table = self.slot_table();
        let flat = |p: usize, s: usize| table.iter().position(|&e| e == (p, s)).unwrap_or(0);
        let grid: Vec<GridIn> =
            self.plan.notes.iter().map(|n| GridIn { time: n.time + self.grid_offset_s, slot: flat(n.pattern, n.slot), pm: n.pm }).collect();
        let min_len = self
            .patterns_used()
            .iter()
            .flat_map(|&p| self.plan.patterns[p].hits())
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
                let pat = &self.plan.patterns[p];
                st.pattern = pat.id.to_string();
                st.slot_in_pattern = s;
                st.pos16 = pat.hits()[s].pos;
            }
        }
        rep
    }

    /// Bars per listen-back window: two bars, or one when bars are long.
    pub fn window_bars(&self) -> u32 {
        if bar_seconds(self.plan.bpm) > 3.5 { 1 } else { 2 }
    }

    /// The `n` worst non-overlapping windows, scored like the whole run (see `Window`).
    pub fn worst_windows(&self, rep: &ScoreReport, onsets: &[Onset], n: usize) -> Vec<Window> {
        use crate::score::{combined_pct, hit_factor, level_pct, timing_pct};
        let wb = self.window_bars();
        let nslots = self.slot_table().len().max(1);
        // Mean level per slot, as in the run's tone score.
        let mut slot_lv = vec![(0.0, 0usize); nslots];
        for note in &rep.notes {
            if let Some(l) = note.level_db {
                slot_lv[note.slot].0 += l;
                slot_lv[note.slot].1 += 1;
            }
        }
        let slot_mean: Vec<f64> = slot_lv.iter().map(|(s, c)| if *c > 0 { s / *c as f64 } else { 0.0 }).collect();
        let matched: std::collections::HashSet<u64> =
            rep.notes.iter().filter_map(|n| n.onset_time).map(f64::to_bits).collect();
        let quiet = rep.level_median_db - crate::score::EXTRA_LEVEL_MARGIN_DB;
        let bar_s = bar_seconds(self.plan.bpm);
        let mut cands = Vec::new();
        let mut start = 0;
        while start + wb <= self.plan.scored_bars {
            // Windows stay within one segment (one pattern).
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
                let devs: Vec<f64> = idx.iter().filter_map(|&i| rep.notes[i].dev_ms).collect();
                let ioi: Vec<f64> = devs.windows(2).map(|d| d[1] - d[0]).collect();
                let enough = devs.len() >= 3;
                let evenness_ms = enough.then(|| crate::dsp::std(&ioi) / std::f64::consts::SQRT_2);
                let drift_ms = crate::dsp::mean(&devs);
                let group_sd = |pm: bool| {
                    let r: Vec<f64> = idx
                        .iter()
                        .filter(|&&i| rep.notes[i].pm == pm)
                        .filter_map(|&i| rep.notes[i].level_db.map(|l| l - slot_mean[rep.notes[i].slot]))
                        .collect();
                    (crate::dsp::std(&r), r.len())
                };
                let ((pm_sd, n_pm), (open_sd, n_open)) = (group_sd(true), group_sd(false));
                let level_sd_db = (n_pm as f64 * pm_sd + n_open as f64 * open_sd) / ((n_pm + n_open) as f64).max(1.0);
                let hit = hit_factor(idx.len(), devs.len(), extra);
                let timing = evenness_ms.map_or(0.0, |e| timing_pct(e, drift_ms.abs(), hit));
                let tone = if enough { level_pct(pm_sd, n_pm, open_sd, n_open) } else { 0.0 };
                cands.push(Window {
                    start_bar: start,
                    bars: wb,
                    score_pct: combined_pct(timing, tone),
                    evenness_ms,
                    drift_ms,
                    level_sd_db,
                    n_missed: idx.len() - devs.len(),
                    notes: idx,
                    n_extra: extra,
                    pattern: pat,
                    pattern_id: self.plan.patterns[pat].id.to_string(),
                    offset_ms,
                });
            }
            start += 1;
        }
        // Ties (several windows at 0 %) go to the one with more missed and extra notes.
        cands.sort_by(|a, b| a.score_pct.total_cmp(&b.score_pct).then((b.n_missed + b.n_extra).cmp(&(a.n_missed + a.n_extra))));
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
    /// Raw DI (no amp/cab), levelled so attacks sit around -24 dBFS, with the click
    /// (`click_gain`, already boosted by the caller) where the player heard it.
    pub fn render_listen(&self, w: &Window, click_gain: f32, attack_level_db: f64) -> Vec<f32> {
        let sr = f64::from(self.sample_rate);
        let (s0, s1) = self.window_span(w);
        let shift = self.grid_offset_s + w.offset_ms / 1e3;
        let a = ((s0 + shift) * sr).max(0.0) as usize;
        let b = (((s1 + shift) * sr) as usize).min(self.samples.len());
        if a >= b {
            return Vec::new();
        }
        let di_gain = 10f64.powf((-24.0 - attack_level_db) / 20.0).clamp(0.1, 10.0) as f32;
        let mut out: Vec<f32> = self.samples[a..b].iter().map(|v| v * di_gain).collect();
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
        self.save_in(&data_dir().join("runs"), rep, onsets, worst)
    }

    /// Save the take (`<stamp>_<level>.wav` + `.json`) into `dir`; returns the WAV path.
    pub fn save_in(&self, dir: &std::path::Path, rep: &ScoreReport, onsets: &[Onset], worst: &[Window]) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let base = dir.join(format!("{stamp}_{}", self.level.key().replace('@', "_")));
        let wav = base.with_extension("wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: self.sample_rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(&wav, spec).map_err(|e| e.to_string())?;
        for s in &self.samples {
            w.write_sample(*s).map_err(|e| e.to_string())?;
        }
        w.finalize().map_err(|e| e.to_string())?;
        /// The full definition, so generated patterns can be rebuilt (format 3).
        #[derive(Serialize)]
        struct PatternInfo<'a> {
            #[serde(flatten)]
            pattern: &'a Pattern,
            difficulty: f64,
        }
        #[derive(Serialize)]
        struct SlotInfo<'a> {
            pattern: &'a str,
            slot: usize,
            pos16: f64,
        }
        #[derive(Serialize)]
        struct Out<'a> {
            /// 3: `patterns` hold full definitions and `seed` the run's draw. Earlier files
            /// have no format (2: `patterns`/`bar_patterns` ids of built-in patterns, 1: one
            /// `pattern` id).
            format: u32,
            level: String,
            level_name: String,
            seed: Option<u64>,
            /// Mean pattern difficulty over the scored bars.
            avg_difficulty: f64,
            /// Patterns used, and the pattern id of every scored bar.
            patterns: Vec<PatternInfo<'a>>,
            bar_patterns: Vec<&'a str>,
            /// Flat slot ids used by `report.slots` and `report.notes[].slot`.
            slots: Vec<SlotInfo<'a>>,
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
        let pats = &self.plan.patterns;
        let out = Out {
            format: 3,
            level: self.level.key(),
            level_name: self.level.name(),
            seed: self.plan.seed,
            avg_difficulty: self.avg_difficulty(),
            patterns: self
                .patterns_used()
                .into_iter()
                .map(|p| PatternInfo { pattern: &pats[p], difficulty: difficulty(&pats[p], self.plan.bpm, self.level.pm_on()) })
                .collect(),
            bar_patterns: self.plan.bar_patterns.iter().map(|&p| &*pats[p].id).collect(),
            slots: self
                .slot_table()
                .into_iter()
                .map(|(p, s)| SlotInfo { pattern: &pats[p].id, slot: s, pos16: pats[p].hits()[s].pos })
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

/// The patterns of a saved run. Format 3 stores full definitions (generated patterns
/// included) and the seed; older files name built-in patterns by id, per bar
/// (`bar_patterns`) or once for the whole take (`pattern`).
pub fn saved_sequence(j: &serde_json::Value) -> Result<Sequence, String> {
    let scored = j.get("scored_bars").and_then(serde_json::Value::as_u64).ok_or("missing scored_bars")? as usize;
    let format = j.get("format").and_then(serde_json::Value::as_u64).unwrap_or(1);
    let bar_ids: Vec<String> = match j.get("bar_patterns").and_then(|v| v.as_array()) {
        Some(a) => a.iter().map(|v| v.as_str().map(str::to_string).ok_or(format!("bad pattern id {v}"))).collect::<Result<_, _>>()?,
        None => vec![j.get("pattern").and_then(|v| v.as_str()).ok_or("no pattern")?.to_string(); scored],
    };
    if format >= 3 {
        let defs = j.get("patterns").and_then(|v| v.as_array()).ok_or("missing pattern definitions")?;
        let mut patterns: Vec<Pattern> = Vec::new();
        for d in defs {
            let p: Pattern = serde_json::from_value(d.clone()).map_err(|e| format!("bad pattern definition: {e}"))?;
            crate::patterns::parse(&p.notation)?;
            if let Some(f) = &p.frets {
                let n = f.split_whitespace().map(crate::patterns::Fretting::parse).collect::<Result<Vec<_>, _>>()?.len();
                if n != p.hits().len() {
                    return Err(format!("pattern {}: {n} frettings for {} notes", p.id, p.hits().len()));
                }
            }
            patterns.push(p);
        }
        let bar_patterns = bar_ids
            .iter()
            .map(|id| patterns.iter().position(|p| p.id == *id).ok_or(format!("pattern {id} not defined")))
            .collect::<Result<_, _>>()?;
        let seed = j.get("seed").and_then(serde_json::Value::as_u64);
        return Ok(Sequence { patterns, bar_patterns, seed });
    }
    let pidx = |id: &str| PATTERNS.iter().position(|p| p.id == id).ok_or(format!("unknown pattern {id}"));
    let bars: Vec<usize> = bar_ids.iter().map(|id| pidx(id)).collect::<Result<_, _>>()?;
    Ok(Sequence::from_static(&bars))
}

/// Load a saved run (JSON + WAV) for offline re-scoring. Understands the JSON of every
/// version (see `saved_sequence`; tier `level` keys of format 2 fall back to a basic run).
pub fn load_run(json_path: &std::path::Path) -> Result<(RunData, serde_json::Value), String> {
    let text = std::fs::read_to_string(json_path).map_err(|e| e.to_string())?;
    let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let num = |k: &str| j.get(k).and_then(serde_json::Value::as_f64).ok_or(format!("missing {k}"));
    let bpm = num("bpm")? as u32;
    let count_in = num("count_in_bars")? as u32;
    let seq = saved_sequence(&j)?;
    let level = j.get("level").and_then(|v| v.as_str()).and_then(Level::from_key).unwrap_or_else(|| {
        let first = &seq.patterns[seq.bar_patterns[0]];
        match PATTERNS.iter().position(|p| p.id == first.id) {
            Some(p) if seq.patterns.len() == 1 => Level::drill(p, bpm),
            _ => Level::run(0, bpm),
        }
    });
    let plan = SessionPlan::from_sequence(bpm, count_in, seq, level.pm_on());
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
    println!("{:<40} {:>5} {:>13} {:>13} {:>9} {:>11} {:>11} {:>11}", "run", "grid", "matched", "extra", "missed", "evenness ms", "level dB", "combined");
    for p in paths {
        let path = std::path::Path::new(p);
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        match load_run(path) {
            Ok((run, old)) => {
                let (rep, onsets) = run.analyse();
                let o = |k: &str| old.get(k).and_then(serde_json::Value::as_f64).unwrap_or(f64::NAN);
                println!(
                    "{:<40} {:>5} {:>6.0} -> {:<4} {:>6.0} -> {:<4} {:>3.0} -> {:<3} {:>4.1} -> {:<4.1} {:>4.1} -> {:<4.1} {:>4.1} -> {:<4.1}",
                    name, rep.n_grid, o("n_matched"), rep.n_matched, o("n_extra"), rep.n_extra, o("n_missed"), rep.n_missed,
                    o("spread_ms"), rep.spread_ms, o("level_std_detrended_db"), rep.level_std_detrended_db, o("combined_pct"), rep.combined_pct
                );
                println!(
                    "    evenness {:.1} ms (IOI sd {:.1})  drift {:.0} ms rms  global offset {:.0} ms  timing {:.0} %  tone {:.0} %  pass {}",
                    rep.spread_ms, rep.ioi_sd_ms, rep.drift_rms_ms, rep.latency_ms, rep.timing_pct, rep.tone_pct,
                    if rep.combined_pct >= crate::progress::PASS_THRESHOLD { "yes" } else { "no" }
                );
                println!("    onsets {}  close pairs (< 140 ms) {}", onsets.len(), close_pairs(&rep, &onsets).len());
                for w in run.worst_windows(&rep, &onsets, 3) {
                    let (s0, _) = run.window_span(&w);
                    let t = s0 - run.plan.scoring_start() + beat_seconds(run.plan.bpm);
                    println!(
                        "    window bars {}-{} ({}:{:02}) {}: {:.0} %  evenness {} ms  drifted {:+.0} ms  missed {}  extra {}  attacks {:.1} dB",
                        w.start_bar + 1, w.start_bar + w.bars, t as u32 / 60, t as u32 % 60, w.pattern_id, w.score_pct,
                        w.evenness_ms.map_or("-".into(), |e| format!("{e:.0}")),
                        w.drift_ms, w.n_missed, w.n_extra, w.level_sd_db
                    );
                }
            }
            Err(e) => println!("{name:<40} skipped: {e}"),
        }
    }
}

/// Onsets inside the scored span that follow the previous onset by less than 140 ms (a
/// sign of double detections at the slow tempos the real takes use), as recording times.
pub fn close_pairs(rep: &ScoreReport, onsets: &[Onset]) -> Vec<f64> {
    let (Some(first), Some(last)) = (rep.notes.first(), rep.notes.last()) else { return Vec::new() };
    let (lo, hi) = (first.grid_time + first.offset_ms / 1e3 - 0.1, last.grid_time + last.offset_ms / 1e3 + 0.1);
    onsets.windows(2).filter(|w| w[1].time >= lo && w[1].time <= hi && w[1].time - w[0].time < 0.14).map(|w| w[1].time).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::tests::{Rng, synth};

    #[test]
    fn full_run_from_synth_signal() {
        let level = Level::drill(PATTERNS.iter().position(|p| p.id == "gallop").unwrap(), 120);
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
        let audio = run.render_listen(&worst[0], 0.3, rep.level_median_db);
        let expect = (4.0 + 0.5 + 0.25) * 48000.0;
        assert_eq!(worst[0].pattern_id, "gallop"); // 2 bars + 1 beat lead-in + half-beat tail
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

    #[test]
    fn saved_runs_round_trip_and_old_formats_load() {
        use crate::patterns::{LH, PM, S16};
        let level = Level::run(PM | S16 | LH, 120);
        let plan = SessionPlan::new(&level, 1, 60.0);
        assert!(plan.patterns.iter().any(|p| p.id.starts_with("g:")), "uses generated patterns");
        let mut rng = Rng::new(5);
        let hits: Vec<(f64, f64)> = plan.notes.iter().map(|n| (n.time + 0.2 + 0.01 + 0.004 * rng.gauss(), -18.0)).collect();
        let samples = synth(48000.0, plan.total_seconds() + 1.5, &hits, 7);
        let run = RunData { level, plan, sample_rate: 48000, samples, grid_offset_s: 0.2, calibrated: false, timestamps_used: false, stream_latency_ms: 0.0, countin_latency_ms: None, clock: ClockLog::default() };
        let (rep, onsets) = run.analyse();
        let dir = std::env::temp_dir().join(format!("guitarguru-test-{}", std::process::id()));
        let wav = run.save_in(&dir, &rep, &onsets, &[]).unwrap();
        let loaded = load_run(&wav.with_extension("json"));
        let _ = std::fs::remove_dir_all(&dir);
        let (back, old) = loaded.unwrap();
        assert_eq!(back.level, level);
        assert_eq!(back.plan.sequence(), run.plan.sequence(), "patterns, bars and seed survive");
        assert_eq!(back.plan.seed, Some(1));
        assert!((back.avg_difficulty() - run.avg_difficulty()).abs() < 1e-9);
        let again = back.analyse().0.combined_pct;
        assert!((again - rep.combined_pct).abs() < 1e-9);
        assert!((old["combined_pct"].as_f64().unwrap() - again).abs() < 1e-9);

        // Format 2: ids of built-in patterns per bar; format 1: one pattern for the take.
        let ids = |s: &Sequence| s.bar_patterns.iter().map(|&i| s.patterns[i].id.to_string()).collect::<Vec<_>>();
        let v2 = serde_json::json!({"scored_bars": 3, "bar_patterns": ["quarters", "gallop", "gallop"],
            "patterns": [{"id": "quarters", "name": "Quarters", "notation": "8:D-D-D-D-"}]});
        let s2 = saved_sequence(&v2).unwrap();
        assert_eq!(ids(&s2), ["quarters", "gallop", "gallop"]);
        assert_eq!((s2.patterns.len(), s2.seed), (2, None));
        let s1 = saved_sequence(&serde_json::json!({"scored_bars": 2, "pattern": "eighths"})).unwrap();
        assert_eq!(ids(&s1), ["eighths", "eighths"]);
        assert!(saved_sequence(&serde_json::json!({"scored_bars": 1, "pattern": "nope"})).is_err());
        // Format 3 checks the definitions it rebuilds.
        let bad = serde_json::json!({"format": 3, "scored_bars": 1, "bar_patterns": ["g:x"],
            "patterns": [{"id": "g:x", "name": "x", "needs": 4, "notation": "8:D-D-D-D-", "frets": "E0 E3"}]});
        assert!(saved_sequence(&bad).is_err());
    }

    /// Regression check on the real takes in the user's data dir (skipped if absent):
    /// detections stay close to the number of notes and there are few double detections.
    /// Extras aren't asserted in general: in some takes the player leaves the grid (the
    /// 18:58 take plays steady ~0.5 s quarters against the 0.75 s grid for most of the run).
    #[test]
    #[ignore = "needs real runs in ~/.local/share/guitarguru/runs"]
    fn real_runs_rescore_sanely() {
        let dir = crate::progress::data_dir().join("runs");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        paths.sort();
        for p in paths {
            let Ok((run, _)) = load_run(&p) else { continue };
            let (rep, onsets) = run.analyse();
            if onsets.len() < 10 {
                continue; // silent take
            }
            let pairs = close_pairs(&rep, &onsets);
            println!("{}: grid {} onsets {} matched {} extra {} pairs {} spread {:.1} combined {:.1}",
                p.display(), rep.n_grid, onsets.len(), rep.n_matched, rep.n_extra, pairs.len(), rep.spread_ms, rep.combined_pct);
            assert!((onsets.len() as f64) < 1.3 * rep.n_grid as f64, "too many detections");
            assert!(pairs.len() * 10 < rep.n_grid, "too many close pairs: {pairs:?}");
        }
    }

    /// The take behind the phantom-notes report (dark open strings at 80 BPM): no double
    /// detections, and the reported window (bars 27-28, q_and2) has no extra notes.
    #[test]
    #[ignore = "needs the 18:34 run in ~/.local/share/guitarguru/runs"]
    fn real_run_1834_has_no_phantom_notes() {
        let p = crate::progress::data_dir().join("runs/20261030-183434_mix:basic_80.json");
        let Ok((run, _)) = load_run(&p) else { return };
        let (rep, onsets) = run.analyse();
        let pairs = close_pairs(&rep, &onsets);
        assert!(pairs.len() <= 1, "close pairs at {pairs:?}");
        assert!(rep.n_extra <= 15, "{} extras", rep.n_extra);
        let win: Vec<usize> = (0..rep.notes.len()).filter(|&i| (26..28).contains(&run.plan.notes[i].bar)).collect();
        let (t0, t1) = (rep.notes[win[0]].grid_time - 0.1, rep.notes[*win.last().unwrap()].grid_time + 0.6);
        let matched: Vec<f64> = rep.notes.iter().filter_map(|n| n.onset_time).collect();
        let extra: Vec<f64> = onsets
            .iter()
            .filter(|o| o.time > t0 && o.time < t1 && !matched.contains(&o.time) && o.level_db >= rep.level_median_db - crate::score::EXTRA_LEVEL_MARGIN_DB)
            .map(|o| o.time)
            .collect();
        assert!(extra.len() <= 1, "extras in bars 27-28 at {extra:?}");
    }
}
