//! Play screen: only the metronome (pendulum + beat lights) and the current pattern as a
//! pick-direction (down/up stroke) diagram on the 16th grid with a playhead. No live
//! accuracy feedback: the take is scored afterwards.

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

    // Thin progress line along the top edge.
    let frac = ((t - start) / (total - start)).clamp(0.0, 1.0) as f32;
    p.rect(0.0, 0.0, 1440.0, 3.0, col::PANEL);
    p.rect(0.0, 0.0, 1440.0 * frac, 3.0, col::BRASS);
    let title = if play.calibration { "latency calibration".to_string() } else { format!("{}  {} BPM", play.pattern.name, bpm) };
    p.text(32.0, 32.0, title, 18.0, col::DIM, Align::Left);

    // ---------- metronome
    let beats = t / beat_s;
    let (pvx, pvy, len) = (720.0, 70.0, 190.0);
    let ang = if t >= 0.0 && t < total { 0.42 * (std::f64::consts::PI * beats).cos() as f32 } else { 0.42 };
    let (bx, by) = (pvx + len * ang.sin(), pvy + len * ang.cos());
    for o in [-1.0, 0.0, 1.0] {
        p.line(pvx + o, pvy, bx + o, by, col::DIM);
    }
    p.disc(pvx, pvy, 5.0, col::DIM);
    p.disc(bx, by, 14.0, col::TEXT);
    let cur_beat = beats.floor() as i64;
    let phase = (beats - beats.floor()) as f32;
    for k in 0..BEATS_PER_BAR as i64 {
        let x = 720.0 + (k as f32 - 1.5) * 80.0;
        let accent = k == 0;
        let r = if accent { 20.0 } else { 14.0 };
        let on = t >= 0.0 && t < total && cur_beat.rem_euclid(BEATS_PER_BAR as i64) == k;
        let base = if accent { col::BRASS } else { col::TEXT };
        let c = if on { dim(base, 1.0 - 0.75 * phase.min(1.0)) } else { col::LINE };
        p.disc(x, 320.0, r, c);
    }

    // ---------- pattern: stroke diagram
    let (nx, cell, top) = (80.0, 80.0, 420.0);
    let rel = t - start;
    let in_count_in = rel < 0.0;
    let bar_idx = if in_count_in { 0 } else { (rel / bar_s).floor() as u32 };
    let bar_t0 = start + f64::from(bar_idx) * bar_s;
    let pos16 = ((t - bar_t0) / s16) as f32;
    for k in 0..16 {
        let x = nx + k as f32 * cell;
        let (lbl, size, c) = match k % 4 {
            0 => (format!("{}", k / 4 + 1), 22.0, col::TEXT),
            1 => ("e".into(), 16.0, col::FAINT),
            2 => ("+".into(), 18.0, col::DIM),
            _ => ("a".into(), 16.0, col::FAINT),
        };
        p.text(x + cell / 2.0, top, lbl, size, c, Align::Center);
        p.rect(x, top + 24.0, 1.0, 200.0, if k % 4 == 0 { col::LINE } else { col::PANEL });
    }
    p.rect(nx + 16.0 * cell, top + 24.0, 1.0, 200.0, col::LINE);
    for bn in play.pattern.bar() {
        let x = nx + bn.pos as f32 * cell;
        let cx = x + cell / 2.0 * (bn.len as f32).min(1.0);
        if bn.rest {
            p.rect(cx - 12.0, top + 100.0, 24.0, 4.0, col::FAINT);
            continue;
        }
        let active = !in_count_in && pos16 >= bn.pos as f32 && pos16 < (bn.pos + bn.len) as f32;
        let c = if active { col::BRASS } else { col::TEXT };
        p.stroke_arrow(cx, top + 92.0, 76.0, bn.downstroke(), c);
        // duration bar
        p.rect(x + 6.0, top + 146.0, bn.len as f32 * cell - 12.0, 4.0, if active { col::BRASS } else { col::LINE });
    }
    // playhead
    if !in_count_in {
        let px = nx + pos16.clamp(0.0, 16.0) * cell;
        p.rect(px - 1.5, top + 30.0, 3.0, 190.0, col::BRASS);
    } else if t >= 0.0 {
        let n = cur_beat.rem_euclid(BEATS_PER_BAR as i64) + 1;
        let last_bar = rel > -bar_s;
        let c = if last_bar { col::BRASS } else { col::TEXT };
        p.text(720.0, 372.0, format!("count-in  {n}"), 26.0, c, Align::Center);
    } else {
        p.text(720.0, 372.0, "get ready", 26.0, col::DIM, Align::Center);
    }

    p.text(720.0, 870.0, "Enter: end & score     Esc: abort", 13.0, col::FAINT, Align::Center);
}
