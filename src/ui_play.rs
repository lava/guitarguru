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

    let waiting = play.waiting();
    // Time left: a solid bar anchored at the right edge; its left edge moves right.
    let left = if waiting || t < start { 1.0 } else { (1.0 - (t - start) / (total - start)).clamp(0.0, 1.0) as f32 };
    p.rect(0.0, 0.0, 1440.0, 10.0, col::PANEL);
    p.rect(1440.0 * (1.0 - left), 0.0, 1440.0 * left, 10.0, col::BRASS);
    let title = if play.calibration { "latency calibration".to_string() } else { format!("{}  {} BPM", play.pattern.name, bpm) };
    p.text(32.0, 40.0, title, 18.0, col::DIM, Align::Left);

    // ---------- metronome
    let beats = t / beat_s;
    let running = t >= 0.0 && (waiting || t < total);
    metronome(p, 720.0, if running { beats } else { 0.5 }, bpm);
    // Flash dot on each click, brass on beat 1.
    let cur_beat = beats.floor() as i64;
    let since = ((beats - beats.floor()) * beat_s) as f32;
    let flash = if running { (-since / 0.09).exp() } else { 0.0 };
    let base = if cur_beat.rem_euclid(BEATS_PER_BAR as i64) == 0 { col::BRASS } else { col::TEXT };
    p.disc(720.0, 382.0, 13.0, col::PANEL2);
    if flash > 0.02 {
        p.disc(720.0, 382.0, 13.0, dim(base, flash.max(0.25)));
    }
    // Subtle count-in status: no accuracy feedback, just whether it is waiting.
    if let Some(gate) = &play.gate {
        p.text(560.0, 382.0, "play along to start", 14.0, col::FAINT, Align::Right);
        for i in 0..crate::countin::NEEDED {
            let c = if i < gate.streak() { col::DIM } else { col::PANEL2 };
            p.disc(780.0 + i as f32 * 16.0, 382.0, 4.0, c);
        }
    } else if t < start {
        p.text(560.0, 382.0, "starting next bar", 14.0, col::FAINT, Align::Right);
    }

    // ---------- pattern: stroke diagram
    let (nx, cell, top) = (80.0, 80.0, 420.0);
    let rel = t - start;
    let in_count_in = waiting || rel < 0.0;
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
    }

    p.text(720.0, 870.0, "Enter: end & score     Esc: abort", 13.0, col::FAINT, Align::Center);
}

/// Classical table metronome: pyramid wooden body, inverted pendulum arm with a sliding
/// weight (lower = faster). The arm passes the centre on each click (one end-to-end swing
/// per beat), so the extremes fall on the off-beat eighths.
fn metronome(p: &mut Paint, cx: f32, beats: f64, bpm: u32) {
    use bevy::color::Color;
    let wood = Color::srgb(0.43, 0.26, 0.15);
    let wood_dark = Color::srgb(0.29, 0.17, 0.09);
    let face = Color::srgb(0.55, 0.36, 0.21);
    let metal = Color::srgb(0.80, 0.78, 0.72);
    let (top_y, bot_y) = (70.0, 330.0);
    let (top_w, bot_w) = (64.0, 230.0);
    // body as horizontal strips
    let mut y = top_y;
    while y < bot_y {
        let f = (y - top_y) / (bot_y - top_y);
        let w = top_w + (bot_w - top_w) * f;
        p.rect(cx - w / 2.0, y, w, 2.0, wood);
        let fw = (w - 34.0).max(0.0);
        if y > top_y + 16.0 && y < bot_y - 34.0 {
            p.rect(cx - fw / 2.0, y, fw, 2.0, face);
        }
        y += 2.0;
    }
    p.rect(cx - bot_w / 2.0 - 12.0, bot_y, bot_w + 24.0, 16.0, wood_dark);
    p.rect(cx - top_w / 2.0 - 4.0, top_y - 8.0, top_w + 8.0, 10.0, wood_dark);
    // tempo scale on the face
    for k in 0..9 {
        let yy = top_y + 40.0 + k as f32 * 22.0;
        p.rect(cx - 7.0, yy, 14.0, 1.5, wood_dark);
    }
    // arm: inverted pendulum pivoting near the bottom
    let (px, py, len) = (cx, bot_y - 46.0, 250.0);
    let theta = 0.36 * (std::f64::consts::PI * beats).sin() as f32;
    let dir = bevy::math::Vec2::new(theta.sin(), -theta.cos());
    let mid = bevy::math::Vec2::new(px, py) + dir * (len / 2.0);
    p.rot_rect(mid.x, mid.y, 6.0, len, -theta, metal);
    // sliding weight: higher up for slower tempi
    let d = len * (0.32 + 0.55 * (180.0 - bpm as f32).max(0.0) / 140.0);
    let w = bevy::math::Vec2::new(px, py) + dir * d;
    p.rot_rect(w.x, w.y, 36.0, 24.0, -theta, wood_dark);
    p.rot_rect(w.x, w.y, 30.0, 18.0, -theta, metal);
    p.disc(px, py, 7.0, wood_dark);
}
