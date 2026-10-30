//! Play screen. The centrepiece is a big dot flashing on every click (brass on beat 1).
//! Secondary, smaller and dimmer: a table metronome and the current pattern as a
//! pick-direction (down/up stroke) diagram with a playhead. Before a pattern switch the
//! next pattern is previewed below with a countdown bar. No accuracy feedback during play.

use bevy::color::Color;

use crate::game::Game;
use crate::paint::{Align, Paint, col};
use crate::patterns::{BEATS_PER_BAR, PATTERNS, Pattern, bar_seconds, beat_seconds, sixteenth_seconds};
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
    let title = if play.calibration { "latency calibration".to_string() } else { format!("{}  {} BPM", play.level.name(), bpm) };
    p.text(32.0, 40.0, title, 16.0, col::FAINT, Align::Left);

    // ---------- the click dot (centrepiece)
    let beats = t / beat_s;
    let running = t >= 0.0 && (waiting || t < total);
    let cur_beat = beats.floor() as i64;
    let since = ((beats - beats.floor()) * beat_s) as f32;
    let flash = if running { (-since / 0.12).exp() } else { 0.0 };
    let accent = cur_beat.rem_euclid(BEATS_PER_BAR as i64) == 0;
    let base = if accent { col::BRASS } else { col::TEXT };
    let (dx, dy, dr) = (720.0, 210.0, 100.0);
    p.disc(dx, dy, dr + 4.0, col::PANEL2);
    p.disc(dx, dy, dr, col::BG);
    if running {
        let r = dr * (0.92 + 0.08 * flash);
        p.disc(dx, dy, r, mix(col::BG, base, 0.12 + 0.88 * flash));
    }
    // Count-in status, subtle: whether it is waiting and how many clicks were caught.
    if let Some(gate) = &play.gate {
        p.text(720.0, 346.0, "play along to start", 14.0, col::FAINT, Align::Center);
        for i in 0..crate::countin::NEEDED {
            let c = if i < gate.streak() { col::DIM } else { col::PANEL2 };
            p.disc(696.0 + i as f32 * 16.0, 368.0, 4.0, c);
        }
    } else if t < start {
        p.text(720.0, 346.0, "starting next bar", 14.0, col::FAINT, Align::Center);
    }

    // ---------- small, dim metronome on the left
    metronome(p, 190.0, 70.0, 0.6, if running { beats } else { 0.5 }, bpm);

    // ---------- current pattern (secondary)
    let rel = t - start;
    let in_count_in = waiting || rel < 0.0;
    let bar_idx = if in_count_in { 0 } else { (rel / bar_s).floor() as i64 };
    let bar_t0 = start + bar_idx as f64 * bar_s;
    let pos16 = ((t - bar_t0) / s16) as f32;
    let cur = play.plan.pattern_at(bar_idx);
    p.text(80.0, 412.0, PATTERNS[cur].name, 15.0, col::DIM, Align::Left);
    let (pm_on, tab) = (play.level.pm_on(), play.level.tab());
    stroke_diagram(p, &PATTERNS[cur], 80.0, 430.0, 80.0, 64.0, if in_count_in { None } else { Some(pos16) }, true, pm_on, tab);

    // ---------- next pattern preview with countdown to the switch
    if !waiting {
        let next_switch = (bar_idx.max(0) + 1..i64::from(play.plan.scored_bars)).find(|&b| play.plan.pattern_at(b) != cur);
        if let Some(sb) = next_switch {
            let t_switch = start + sb as f64 * bar_s;
            let lead = 2.0 * bar_s;
            let remain = t_switch - t;
            if remain > 0.0 && remain <= lead {
                let next = play.plan.pattern_at(sb);
                let (x, w) = (400.0, 640.0);
                p.text(x, 676.0, format!("next: {}", PATTERNS[next].name), 14.0, col::DIM, Align::Left);
                let f = (remain / lead) as f32;
                p.rect(x, 690.0, w, 6.0, col::PANEL);
                p.rect(x + w * (1.0 - f), 690.0, w * f, 6.0, dim(col::BRASS, 0.8));
                stroke_diagram(p, &PATTERNS[next], x, 704.0, 40.0, 40.0, None, false, pm_on, tab);
            }
        }
    }

    p.text(720.0, 870.0, "Enter: end & score     Esc: abort", 13.0, col::FAINT, Align::Center);
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    let (a, b) = (a.to_srgba(), b.to_srgba());
    Color::srgb(a.red + (b.red - a.red) * k, a.green + (b.green - a.green) * k, a.blue + (b.blue - a.blue) * k)
}

/// One bar on the 16th grid: count labels, then either stroke arrows (played down, played
/// up, small faint ghost strokes) with duration bars, or with `tab` a six-string tab with
/// fret numbers and small pick arrows above. P.M. brackets mark palm-muted notes when
/// `pm_on`. Optional playhead position in sixteenths. Drawn at reduced contrast.
#[allow(clippy::too_many_arguments)]
fn stroke_diagram(p: &mut Paint, pat: &Pattern, x0: f32, top: f32, cell: f32, arrow: f32, playhead: Option<f32>, labels: bool, pm_on: bool, tab: bool) {
    let h = if tab { arrow * 2.6 } else { arrow + 34.0 };
    if labels {
        for k in 0..16 {
            let x = x0 + k as f32 * cell;
            let (lbl, size, c) = match k % 4 {
                0 => (format!("{}", k / 4 + 1), 18.0, col::DIM),
                1 => ("e".into(), 14.0, col::FAINT),
                2 => ("+".into(), 15.0, col::FAINT),
                _ => ("a".into(), 14.0, col::FAINT),
            };
            p.text(x + cell / 2.0, top, lbl, size, c, Align::Center);
        }
    }
    let ltop = top + if labels { 20.0 } else { 0.0 };
    // P.M. brackets above the notes.
    let hits = pat.hits();
    let pm_y = ltop + 6.0;
    let gtop = ltop + 16.0;
    if pm_on {
        let mut i = 0;
        while i < hits.len() {
            if !hits[i].pm {
                i += 1;
                continue;
            }
            let start = hits[i].pos;
            let mut j = i;
            while j + 1 < hits.len() && hits[j + 1].pm {
                j += 1;
            }
            let end = hits[j].pos + hits[j].len;
            let (xa, xb) = (x0 + start as f32 * cell + 2.0, x0 + end as f32 * cell - 4.0);
            let tsize = (cell * 0.18).clamp(9.0, 12.0);
            p.text(xa, pm_y, "P.M.", tsize, col::DIM, Align::Left);
            let mut x = xa + tsize * 2.6;
            while x < xb {
                p.rect(x, pm_y, 4.0_f32.min(xb - x), 1.0, col::DIM);
                x += 8.0;
            }
            p.rect(xb, pm_y, 1.0, 6.0, col::DIM);
            i = j + 1;
        }
    }
    for k in 0..=16 {
        p.rect(x0 + k as f32 * cell, gtop, 1.0, h, if k % 4 == 0 { col::LINE } else { col::PANEL });
    }
    if tab {
        // Six strings, high e on top; small pick arrows above.
        let sp = (h - arrow * 0.5 - 8.0) / 6.0;
        let s_top = gtop + arrow * 0.5 + 6.0;
        let string_y = |s: u8| s_top + (f32::from(s) - 0.5) * sp;
        for s in 1..=6u8 {
            p.rect(x0, string_y(s), 16.0 * cell, 1.0, col::LINE);
        }
        let fr = pat.frettings();
        for (n, f) in hits.iter().zip(&fr) {
            let cx = x0 + n.pos as f32 * cell + cell / 2.0;
            let active = playhead.is_some_and(|ph| ph >= n.pos as f32 && ph < (n.pos + n.len) as f32);
            let c = if active { dim(col::BRASS, 0.85) } else { dim(col::TEXT, 0.65) };
            p.stroke_arrow(cx, gtop + arrow * 0.25, arrow * 0.4, n.downstroke(), c);
            let size = (sp * 1.1).clamp(9.0, 16.0);
            let mut put = |s: u8, fret: u8| {
                let y = string_y(s);
                p.rect(cx - size * 0.45, y - size * 0.5, size * 0.9 * if fret >= 10 { 1.6 } else { 1.0 }, size, col::BG);
                p.text(cx, y, format!("{fret}"), size, c, Align::Center);
            };
            put(f.string, f.fret);
            if f.power {
                put(f.string - 1, f.fret + 2);
            }
        }
    } else {
        // Played notes: arrow at the onset plus a duration bar; ghost strokes (the hand
        // moves but misses the strings on purpose): small faint arrows.
        for st in pat.strokes() {
            let x = x0 + st.pos as f32 * cell;
            let cx = x + cell / 2.0;
            if !st.played {
                p.stroke_arrow(cx, gtop + 8.0 + arrow / 2.0, arrow * 0.42, st.down, col::LINE);
                continue;
            }
            let len = hits.iter().find(|h| (h.pos - st.pos).abs() < 1e-6).map_or(1.0, |h| h.len) as f32;
            let active = playhead.is_some_and(|ph| ph >= st.pos as f32 && ph < st.pos as f32 + len);
            let base = if st.down { col::TEXT } else { col::DIM };
            let c = if active { dim(col::BRASS, 0.8) } else { dim(base, 0.6) };
            p.stroke_arrow(cx, gtop + 8.0 + arrow / 2.0, arrow, st.down, c);
            p.rect(x + 4.0, gtop + arrow + 18.0, len * cell - 8.0, 3.0, if active { dim(col::BRASS, 0.8) } else { col::LINE });
        }
    }
    if let Some(ph) = playhead {
        let px = x0 + ph.clamp(0.0, 16.0) * cell;
        p.rect(px - 1.0, gtop, 2.0, h, dim(col::BRASS, 0.7));
    }
}

/// Classical table metronome: pyramid wooden body, inverted pendulum arm with a sliding
/// weight (lower = faster). The arm passes the centre on each click (one end-to-end swing
/// per beat), so the extremes fall on the off-beat eighths. Drawn small and dim.
fn metronome(p: &mut Paint, cx: f32, top: f32, s: f32, beats: f64, bpm: u32) {
    let k = 0.55; // dimming toward the panel colour
    let wood = dim(Color::srgb(0.43, 0.26, 0.15), k);
    let wood_dark = dim(Color::srgb(0.29, 0.17, 0.09), k);
    let face = dim(Color::srgb(0.55, 0.36, 0.21), k);
    let metal = dim(Color::srgb(0.80, 0.78, 0.72), k);
    let (top_y, bot_y) = (top, top + 260.0 * s);
    let (top_w, bot_w) = (64.0 * s, 230.0 * s);
    let mut y = top_y;
    while y < bot_y {
        let f = (y - top_y) / (bot_y - top_y);
        let w = top_w + (bot_w - top_w) * f;
        p.rect(cx - w / 2.0, y, w, 2.0, wood);
        let fw = (w - 34.0 * s).max(0.0);
        if y > top_y + 16.0 * s && y < bot_y - 34.0 * s {
            p.rect(cx - fw / 2.0, y, fw, 2.0, face);
        }
        y += 2.0;
    }
    p.rect(cx - bot_w / 2.0 - 12.0 * s, bot_y, bot_w + 24.0 * s, 16.0 * s, wood_dark);
    p.rect(cx - top_w / 2.0 - 4.0 * s, top_y - 8.0 * s, top_w + 8.0 * s, 10.0 * s, wood_dark);
    for i in 0..9 {
        let yy = top_y + (40.0 + i as f32 * 22.0) * s;
        p.rect(cx - 7.0 * s, yy, 14.0 * s, 1.5, wood_dark);
    }
    let (px, py, len) = (cx, bot_y - 46.0 * s, 250.0 * s);
    let theta = 0.36 * (std::f64::consts::PI * beats).sin() as f32;
    let dir = bevy::math::Vec2::new(theta.sin(), -theta.cos());
    let mid = bevy::math::Vec2::new(px, py) + dir * (len / 2.0);
    p.rot_rect(mid.x, mid.y, 6.0 * s, len, -theta, metal);
    let d = len * (0.32 + 0.55 * (180.0 - bpm as f32).max(0.0) / 140.0);
    let w = bevy::math::Vec2::new(px, py) + dir * d;
    p.rot_rect(w.x, w.y, 36.0 * s, 24.0 * s, -theta, wood_dark);
    p.rot_rect(w.x, w.y, 30.0 * s, 18.0 * s, -theta, metal);
    p.disc(px, py, 7.0 * s, wood_dark);
}
