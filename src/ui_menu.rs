//! Main screen: the level graph. Nodes are levels (pattern x BPM), laid out as tier bands
//! (rows) x tempo (columns); edges show what passing a level unlocks (same pattern at
//! +/-20 BPM, next tier at the same BPM). Click a node to select it, click again (or
//! Enter) to play.

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;

use crate::game::{CALIBRATION_BPM, Game, Screen, calibration_pattern};
use crate::paint::{Align, Paint, col};
use crate::patterns::{BPM_MAX, BPM_MIN, BPM_STEP, PATTERNS, bpm_steps};
use crate::progress::{PASS_THRESHOLD, level_key};
use crate::ui_common::*;

const X0: f32 = 330.0;
const COL_W: f32 = 148.0;
const Y0: f32 = 150.0;
const BAND_H: f32 = 112.0;
const R: f32 = 15.0;

/// Screen position of a level node.
fn node_pos(pi: usize, bpm: u32) -> (f32, f32) {
    let tier = PATTERNS[pi].tier;
    let in_tier: Vec<usize> = (0..PATTERNS.len()).filter(|&i| PATTERNS[i].tier == tier).collect();
    let k = in_tier.iter().position(|&i| i == pi).unwrap_or(0) as f32;
    let n = in_tier.len() as f32;
    let off = k - (n - 1.0) / 2.0;
    let col = ((bpm - BPM_MIN) / BPM_STEP) as f32;
    (X0 + col * COL_W + off * 34.0, Y0 + (tier - 1) as f32 * BAND_H + off * 44.0)
}

pub fn draw(g: &mut Game, p: &mut Paint, keys: &ButtonInput<KeyCode>) {
    let bpms: Vec<u32> = bpm_steps().collect();
    p.text(32.0, 40.0, "GUITARGURU", 30.0, col::BRASS, Align::Left);
    let calib = match g.cfg.latency_ms {
        Some(l) => format!("latency {l:+.0} ms"),
        None => "calibrate latency".into(),
    };
    if p.button(1090.0, 22.0, 190.0, 38.0, &calib, false) {
        g.start_level(calibration_pattern(), CALIBRATION_BPM, true);
        return;
    }
    if p.button(1292.0, 22.0, 116.0, 38.0, "settings", false) {
        g.refresh_lists();
        g.screen = Screen::Settings;
        return;
    }
    for (c, bpm) in bpms.iter().enumerate() {
        p.text(X0 + c as f32 * COL_W, 96.0, format!("{bpm}"), 18.0, col::DIM, Align::Center);
    }
    p.text(X0 + (bpms.len() as f32 - 0.5) * COL_W + 20.0, 96.0, "BPM", 12.0, col::FAINT, Align::Left);

    let passed = |pi: usize, b: u32| g.progress.best(PATTERNS[pi].id, b).is_some_and(|s| s >= PASS_THRESHOLD);
    let unlocked = |pi: usize, b: u32| g.progress.is_unlocked(PATTERNS[pi].id, b);

    // Tier bands with labels
    let mut tiers: Vec<u32> = PATTERNS.iter().map(|p| p.tier).collect();
    tiers.dedup();
    for t in &tiers {
        let y = Y0 + (*t - 1) as f32 * BAND_H;
        p.rect(32.0, y - BAND_H / 2.0 + 4.0, 1376.0, 1.0, col::PANEL);
        p.text(40.0, y, roman(*t), 28.0, col::BRASS, Align::Left);
        let names: Vec<usize> = (0..PATTERNS.len()).filter(|&i| PATTERNS[i].tier == *t).collect();
        for &pi in &names {
            let (_, ny) = node_pos(pi, BPM_MIN);
            p.text(96.0, ny, PATTERNS[pi].name, 15.0, col::DIM, Align::Left);
        }
    }

    // Edges (drawn first, nodes on top)
    for pi in 0..PATTERNS.len() {
        for &b in &bpms {
            let (x, y) = node_pos(pi, b);
            let src_passed = passed(pi, b);
            if b + BPM_STEP <= BPM_MAX {
                let (x2, y2) = node_pos(pi, b + BPM_STEP);
                let lit = src_passed || passed(pi, b + BPM_STEP);
                let near = unlocked(pi, b) || unlocked(pi, b + BPM_STEP);
                p.line(x + R, y, x2 - R, y2, if lit { col::BRASS } else if near { col::LINE } else { col::PANEL2 });
            }
            for qi in (0..PATTERNS.len()).filter(|&q| PATTERNS[q].tier == PATTERNS[pi].tier + 1) {
                let (x2, y2) = node_pos(qi, b);
                let c = if src_passed {
                    col::BRASS
                } else if unlocked(pi, b) {
                    col::LINE
                } else {
                    col::PANEL2
                };
                p.line(x, y + R, x2, y2 - R, c);
            }
        }
    }

    // Nodes
    let mut clicked: Option<(usize, u32)> = None;
    let mut hovered: Option<(usize, u32)> = None;
    for pi in 0..PATTERNS.len() {
        for &b in &bpms {
            let (x, y) = node_pos(pi, b);
            let open = unlocked(pi, b);
            let best = g.progress.best(PATTERNS[pi].id, b);
            let hov = p.cursor.is_some_and(|c| (c.x - x).powi(2) + (c.y - y).powi(2) <= (R + 4.0).powi(2));
            if hov {
                hovered = Some((pi, b));
            }
            if g.selected == (pi, b) {
                p.disc(x, y, R + 6.0, col::TEXT);
                p.disc(x, y, R + 4.0, col::BG);
            }
            if passed(pi, b) {
                p.disc(x, y, R, col::BRASS);
                p.text(x, y, format!("{:.0}", best.unwrap_or(0.0)), 12.0, col::BG, Align::Center);
            } else if open {
                p.disc(x, y, R, if hov { col::TEXT } else { col::DIM });
                p.disc(x, y, R - 2.0, if hov { col::PANEL2 } else { col::PANEL });
                if let Some(s) = best {
                    p.text(x, y, format!("{s:.0}"), 12.0, col::TEXT, Align::Center);
                }
            } else {
                p.disc(x, y, R * 0.45, col::LINE);
            }
            if g.progress.fresh.contains(&level_key(PATTERNS[pi].id, b)) {
                p.disc(x + R * 0.8, y - R * 0.8, 4.0, col::BRASS);
            }
            if hov && p.clicked && open {
                clicked = Some((pi, b));
            }
        }
    }
    if let Some((pi, b)) = clicked {
        if g.selected == (pi, b) {
            g.start_level(&PATTERNS[pi], b, false);
            return;
        }
        g.selected = (pi, b);
    }

    // Keyboard: arrows move between unlocked nodes, Enter plays.
    let (mut r, mut b) = g.selected;
    if keys.just_pressed(KeyCode::ArrowDown) {
        r = (r + 1).min(PATTERNS.len() - 1);
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        r = r.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        b = (b + BPM_STEP).min(BPM_MAX);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        b = b.saturating_sub(BPM_STEP).max(BPM_MIN);
    }
    if unlocked(r, b) {
        g.selected = (r, b);
    }
    if keys.just_pressed(KeyCode::Enter) {
        let (pi, b) = g.selected;
        g.start_level(&PATTERNS[pi], b, false);
        return;
    }

    // One line about the hovered (else selected) level.
    let (pi, b) = hovered.unwrap_or(g.selected);
    let pat = &PATTERNS[pi];
    let key = level_key(pat.id, b);
    let mut info = format!("{}  {} BPM", pat.name, b);
    if !unlocked(pi, b) {
        info.push_str("   locked");
    } else if let Some(h) = g.progress.best_entry(&key) {
        info.push_str(&format!("   best {:.0} %   sd {:.1} ms   attack {:.1} dB", h.combined, h.spread_ms, h.level_std_db));
    }
    pattern_glyph(p, 32.0, 822.0, 240.0, 16.0, pat, col::BRASS);
    p.text(292.0, 830.0, info, 17.0, col::TEXT, Align::Left);
    p.text(1408.0, 830.0, "click again or Enter to play", 13.0, col::FAINT, Align::Right);

    if let Some(e) = &g.engine_err {
        p.text(32.0, 872.0, format!("audio: {e}"), 14.0, col::MISS, Align::Left);
    } else if let Some(w) = g.engine.as_ref().and_then(|e| e.info.warning.clone()) {
        p.text(32.0, 872.0, w, 14.0, col::LATE, Align::Left);
    }
    if let Some(m) = &g.message {
        p.text(1408.0, 872.0, m.clone(), 14.0, col::MISS, Align::Right);
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "~" }
}
