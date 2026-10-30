//! Play screen: time ring, pendulum + beat lights, live stats, the bar on a 16th grid
//! with playhead and hit markers, heat strip of the last bars, bias line.

use crate::game::Game;
use crate::paint::{Align, Paint, col};
use crate::patterns::{BEATS_PER_BAR, bar_seconds, beat_seconds, sixteenth_seconds};
use crate::ui_common::*;

pub fn draw(g: &mut Game, p: &mut Paint) {
    let (Some(engine), Some(play)) = (&g.engine, &g.play) else { return };
    let t = play.now(engine);
    let bpm = play.plan.bpm;
    let beat_s = beat_seconds(bpm);
    let bar_s = bar_seconds(bpm);
    let s16 = sixteenth_seconds(bpm);
    let total = play.plan.total_seconds();
    let start = play.plan.scoring_start();
    let win = play.window();

    // ---------- top panel
    p.panel(20.0, 16.0, 1400.0, 262.0);
    // time ring
    let (rcx, rcy, rr) = (150.0, 140.0, 96.0);
    let remaining = (total - t.max(0.0)).max(0.0);
    let frac = if t < start { 1.0 } else { (remaining / (total - start)).clamp(0.0, 1.0) };
    let dots = 60;
    for i in 0..dots {
        let a = std::f32::consts::TAU * i as f32 / dots as f32 - std::f32::consts::FRAC_PI_2;
        let lit = (i as f64) < frac * dots as f64;
        p.disc(rcx + rr * a.cos(), rcy + rr * a.sin(), 4.0, if lit { col::BRASS } else { col::LINE });
    }
    let rem = remaining.min(total - start).max(0.0) as u32;
    p.text(rcx, rcy - 8.0, format!("{}:{:02}", rem / 60, rem % 60), 36.0, col::TEXT, Align::Center);
    let bar_no = ((t - start) / bar_s).floor() as i64 + 1;
    let bar_txt = if t < start { "count-in".to_string() } else { format!("bar {} / {}", bar_no.min(i64::from(play.plan.scored_bars)), play.plan.scored_bars) };
    p.text(rcx, rcy + 26.0, bar_txt, 15.0, col::DIM, Align::Center);
    let title = if play.calibration { "Latency calibration".to_string() } else { format!("{} {}  {}", roman(play.pattern.tier), play.pattern.name, "PM") };
    p.text(270.0, 40.0, title, 22.0, col::TEXT, Align::Left);
    p.text(270.0, 66.0, format!("{bpm} BPM  4/4"), 16.0, col::DIM, Align::Left);

    // pendulum
    let beats = t / beat_s;
    let (pvx, pvy, len) = (720.0, 40.0, 150.0);
    let ang = if t >= 0.0 { 0.45 * (std::f64::consts::PI * beats).cos() as f32 } else { 0.45 };
    let (bx, by) = (pvx + len * ang.sin(), pvy + len * ang.cos());
    p.line(pvx, pvy, bx, by, col::DIM);
    p.line(pvx + 1.0, pvy, bx + 1.0, by, col::DIM);
    p.disc(pvx, pvy, 4.0, col::DIM);
    p.disc(bx, by, 11.0, col::TEXT);
    // beat lights
    let cur_beat = beats.floor() as i64;
    let phase = (beats - beats.floor()) as f32;
    for k in 0..BEATS_PER_BAR as i64 {
        let x = 630.0 + k as f32 * 60.0;
        let y = 236.0;
        let accent = k == 0;
        let r = if accent { 15.0 } else { 11.0 };
        let on = t >= 0.0 && t < total && cur_beat.rem_euclid(BEATS_PER_BAR as i64) == k;
        let base = if accent { col::BRASS } else { col::TEXT };
        let c = if on { dim(base, 1.0 - 0.7 * phase.min(1.0)) } else { col::LINE };
        p.disc(x, y, r, c);
    }

    // live stats
    let devs: Vec<f64> = play.note_dev.iter().flatten().copied().collect();
    let recent: Vec<f64> = devs.iter().rev().take(32).copied().collect();
    let mean = crate::dsp::mean(&recent);
    let sd = crate::dsp::std(&recent);
    let mut streak = 0;
    for (n, d) in play.plan.notes.iter().zip(&play.note_dev).rev() {
        if n.time > t - win {
            continue;
        }
        match d {
            Some(d) if d.abs() <= 10.0 => streak += 1,
            _ => break,
        }
    }
    let sx = 1010.0;
    // vertical input meter
    let mdb = g.meter_db;
    let mh = 200.0;
    let my = 40.0;
    let ypos = |d: f32| my + mh - mh * ((d + 60.0) / 60.0).clamp(0.0, 1.0);
    p.rect(sx, my, 16.0, mh, col::BG);
    p.rect(sx, ypos(-6.0), 16.0, ypos(-18.0) - ypos(-6.0), dim(col::GOOD, 0.35));
    p.rect(sx + 4.0, ypos(mdb), 8.0, my + mh - ypos(mdb), if mdb > -1.0 { col::MISS } else { col::GOOD });
    p.frame(sx, my, 16.0, mh, 1.0, col::LINE);
    p.text(sx + 8.0, my + mh + 14.0, "IN", 11.0, col::DIM, Align::Center);
    let stat = |p: &mut Paint, x: f32, label: &str, value: String, unit: &str, c| {
        p.label(x, 60.0, label);
        p.text(x, 100.0, value, 34.0, c, Align::Left);
        p.text(x, 130.0, unit, 13.0, col::DIM, Align::Left);
    };
    let mean_c = if recent.is_empty() { col::DIM } else { dev_color(mean) };
    stat(p, sx + 50.0, "offset", if recent.is_empty() { "-".into() } else { format!("{mean:+.0}") }, "ms", mean_c);
    stat(p, sx + 160.0, "sd", if recent.len() < 3 { "-".into() } else { format!("{sd:.0}") }, "ms", col::TEXT);
    stat(p, sx + 260.0, "streak", format!("{streak}"), "on time", col::BRASS);
    if let Some(e) = &g.engine_err {
        p.text(sx + 50.0, 200.0, e.clone(), 12.0, col::MISS, Align::Left);
    }
    p.text(sx + 50.0, 236.0, format!("dsp {:.0} %", engine.shared.dsp_load.load() * 100.0), 12.0, col::FAINT, Align::Left);

    // ---------- notation
    p.panel(20.0, 292.0, 1400.0, 290.0);
    let (nx, cell) = (80.0, 80.0);
    let rel = t - start;
    let bar_idx = if rel < 0.0 { 0 } else { (rel / bar_s).floor() as u32 };
    let in_count_in = rel < 0.0;
    let bar_t0 = start + f64::from(bar_idx) * bar_s;
    let pos16 = ((t - bar_t0) / s16) as f32;
    let bar = play.pattern.bar();
    // 16th labels and grid
    for k in 0..16 {
        let x = nx + k as f32 * cell;
        let lbl = match k % 4 {
            0 => format!("{}", k / 4 + 1),
            1 => "e".into(),
            2 => "+".into(),
            _ => "a".into(),
        };
        p.text(x + 6.0, 320.0, lbl, if k % 4 == 0 { 18.0 } else { 14.0 }, if k % 4 == 0 { col::TEXT } else { col::DIM }, Align::Left);
        p.rect(x, 362.0, 1.0, 96.0, if k % 4 == 0 { col::LINE } else { col::PANEL2 });
    }
    p.rect(nx + 16.0 * cell, 362.0, 1.0, 96.0, col::LINE);
    // P.M. bracket
    p.text(nx, 344.0, "P.M.", 13.0, col::DIM, Align::Left);
    p.dashed_frame(nx + 44.0, 343.0, 16.0 * cell - 48.0, 1.0, col::DIM);
    p.rect(nx + 16.0 * cell - 4.0, 343.0, 1.0, 8.0, col::DIM);
    // blocks
    let notes = &play.plan.notes;
    let first_idx = notes.partition_point(|n| n.bar < bar_idx);
    let mut slot = 0;
    for bn in &bar {
        let x = nx + bn.pos as f32 * cell;
        let wlen = bn.len as f32 * cell - 4.0;
        if bn.rest {
            p.dashed_frame(x + 2.0, 380.0, wlen, 44.0, col::LINE);
            continue;
        }
        let active = !in_count_in && pos16 >= bn.pos as f32 && pos16 < (bn.pos + bn.len) as f32;
        let c = if active { col::BRASS } else { dim(col::TEXT, 0.55) };
        p.rect(x + 2.0, 380.0, wlen, 44.0, dim(c, 0.35));
        p.rect(x + 2.0, 380.0, 8.0, 44.0, c);
        // result marker for this note
        let gi = first_idx + slot;
        if !in_count_in && let (Some(n), Some(d)) = (notes.get(gi), play.note_dev.get(gi)) {
            if n.bar == bar_idx {
                match d {
                    Some(d) => {
                        let mx = x + (*d / 1e3 / s16) as f32 * cell;
                        hit_marker(p, mx + 2.0, 486.0, 10.0, *d);
                        p.text(mx + 2.0, 510.0, format!("{d:+.0}"), 13.0, dev_color(*d), Align::Center);
                    }
                    None if t > n.time + win + 0.05 => p.cross(x + 2.0, 486.0, 8.0, col::MISS),
                    None => {}
                }
            }
        }
        slot += 1;
    }
    // extra (unmatched) hits in this bar
    for h in play.hits.iter().rev().take(64) {
        if h.note.is_none() && h.t >= bar_t0 && h.t < bar_t0 + bar_s && !in_count_in && h.level_db > -50.0 {
            let x = nx + ((h.t - bar_t0) / s16) as f32 * cell;
            p.disc(x, 540.0, 4.0, col::DIM);
        }
    }
    // playhead
    if !in_count_in || pos16 >= 0.0 {
        let px = nx + pos16.clamp(0.0, 16.0) * cell;
        p.rect(px - 1.0, 356.0, 3.0, 108.0, col::BRASS);
    }
    if in_count_in {
        let n = cur_beat.rem_euclid(BEATS_PER_BAR as i64) + 1;
        let txt = if t < 0.0 { "get ready".to_string() } else { format!("count-in  {n}") };
        p.rect(500.0, 390.0, 440.0, 70.0, col::PANEL);
        p.text(720.0, 425.0, txt, 40.0, col::BRASS, Align::Center);
    }
    // last hit readout
    if let Some(h) = play.hits.iter().rev().find(|h| h.note.is_some()) {
        p.label(1250.0, 320.0, "last hit");
        p.text(1250.0, 350.0, format!("{:+.0} ms", h.dev_ms), 26.0, dev_color(h.dev_ms), Align::Left);
    }

    // ---------- bottom panel
    p.panel(20.0, 596.0, 1400.0, 280.0);
    p.label(40.0, 620.0, "last 8 bars");
    let cur_bar = if in_count_in { 0 } else { bar_idx };
    let first_bar = cur_bar.saturating_sub(7);
    let nslots = play.pattern.hits().len();
    for (col_i, b) in (first_bar..first_bar + 8).enumerate() {
        let bx = 40.0 + col_i as f32 * 92.0;
        p.text(bx + 40.0, 642.0, format!("{}", b + 1), 11.0, col::FAINT, Align::Center);
        let base = notes.partition_point(|n| n.bar < b);
        let cw = 84.0 / nslots as f32;
        for s in 0..nslots {
            let i = base + s;
            let Some(n) = notes.get(i) else { continue };
            if n.bar != b {
                continue;
            }
            let c = match play.note_dev[i] {
                Some(d) => dev_color(d),
                None if t > n.time + win + 0.05 => col::MISS,
                None => col::PANEL2,
            };
            p.rect(bx + s as f32 * cw, 656.0, (cw - 1.0).max(1.0), 26.0, c);
        }
    }
    // legend
    let ly = 702.0;
    p.disc(46.0, ly, 5.0, col::TEXT);
    p.text(56.0, ly, "+/-10 ms", 12.0, col::DIM, Align::Left);
    p.tri(136.0, ly, 6.0, false, col::EARLY);
    p.text(146.0, ly, "early", 12.0, col::DIM, Align::Left);
    p.tri(196.0, ly, 6.0, true, col::LATE);
    p.text(206.0, ly, "late", 12.0, col::DIM, Align::Left);
    p.cross(250.0, ly, 5.0, col::MISS);
    p.text(260.0, ly, "miss", 12.0, col::DIM, Align::Left);
    // counts
    let due: Vec<usize> = (0..notes.len()).filter(|&i| notes[i].time < t - win - 0.05).collect();
    let n_hit = due.iter().filter(|&&i| play.note_dev[i].is_some()).count();
    let n_miss = due.len() - n_hit;
    let n_ok = due.iter().filter(|&&i| play.note_dev[i].is_some_and(|d| d.abs() <= 10.0)).count();
    let pct = if n_hit > 0 { 100.0 * n_ok as f64 / n_hit as f64 } else { 0.0 };
    let cx = 820.0;
    for (i, (label, v, c)) in [("hits", format!("{n_hit}"), col::TEXT), ("miss", format!("{n_miss}"), col::MISS), ("+/-10 ms", format!("{pct:.0} %"), col::BRASS)]
        .into_iter()
        .enumerate()
    {
        let x = cx + i as f32 * 130.0;
        p.label(x, 620.0, label);
        p.text(x, 656.0, v, 30.0, c, Align::Left);
    }
    // bias line
    let (bx0, bx1, by) = (60.0, 760.0, 790.0);
    p.label(40.0, 750.0, "bias");
    p.rect(bx0, by, bx1 - bx0, 2.0, col::LINE);
    let bpos = |ms: f64| bx0 + (bx1 - bx0) * ((ms + 60.0) / 120.0).clamp(0.0, 1.0) as f32;
    p.rect(bpos(0.0) - 1.0, by - 10.0, 2.0, 22.0, col::DIM);
    p.rect(bpos(-10.0), by - 4.0, bpos(10.0) - bpos(-10.0), 10.0, col::PANEL2);
    p.text(bx0, by + 24.0, "-60 early", 12.0, col::EARLY, Align::Left);
    p.text(bx1, by + 24.0, "late +60", 12.0, col::LATE, Align::Right);
    if !recent.is_empty() {
        p.disc(bpos(mean), by + 1.0, 9.0, dev_color(mean));
        let word = if mean < -5.0 {
            "rushing"
        } else if mean > 5.0 {
            "dragging"
        } else {
            "centred"
        };
        p.text(bpos(0.0), by - 26.0, word, 15.0, dev_color(mean), Align::Center);
    }
    p.text(820.0, 760.0, "Latency compensated live from your median offset;", 12.0, col::FAINT, Align::Left);
    p.text(820.0, 778.0, "the final score re-analyses the whole take.", 12.0, col::FAINT, Align::Left);
    let end_clicked = p.button(1120.0, 800.0, 130.0, 48.0, "End", true);
    let abort_clicked = p.button(1264.0, 800.0, 130.0, 48.0, "Abort (Esc)", false);
    if end_clicked {
        g.end_early();
    } else if abort_clicked {
        g.abort_play();
    }
}
