//! Main screen: the level graph. Big nodes are the main ("track") levels, tier x BPM, with
//! edges for what passing unlocks (next tier at the same BPM, same tier at +/-20 BPM).
//! Small satellite nodes under each track are its tier's single-pattern drills, open
//! wherever the track is. Click a node to select it, click again (or Enter) to play.

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;

use crate::game::{Game, Screen, calibration_level};
use crate::paint::{Align, Paint, col};
use crate::patterns::{BPM_MAX, BPM_MIN, BPM_STEP, Level, LevelKind, PATTERNS, bpm_steps};
use crate::progress::max_tier;
use crate::ui_common::*;

const X0: f32 = 330.0;
const COL_W: f32 = 148.0;
const Y0: f32 = 150.0;
const BAND_H: f32 = 112.0;
const R: f32 = 18.0;
const RS: f32 = 7.0;

fn col_x(bpm: u32) -> f32 {
    X0 + ((bpm - BPM_MIN) / BPM_STEP) as f32 * COL_W
}

fn node_pos(level: &Level) -> (f32, f32) {
    let x = col_x(level.bpm);
    let y = Y0 + (level.tier() - 1) as f32 * BAND_H;
    match level.kind {
        LevelKind::Track(_) => (x, y - 10.0),
        LevelKind::Drill(p) => {
            let tier = PATTERNS[p].tier;
            let in_tier: Vec<usize> = (0..PATTERNS.len()).filter(|&i| PATTERNS[i].tier == tier).collect();
            let k = in_tier.iter().position(|&i| i == p).unwrap_or(0) as f32;
            let n = in_tier.len() as f32;
            (x + (k - (n - 1.0) / 2.0) * 20.0, y + 30.0)
        }
    }
}

pub fn draw(g: &mut Game, p: &mut Paint, keys: &ButtonInput<KeyCode>) {
    let bpms: Vec<u32> = bpm_steps().collect();
    let tiers: Vec<u32> = (1..=max_tier()).collect();
    p.text(32.0, 40.0, "GUITARGURU", 30.0, col::BRASS, Align::Left);
    let calib = match g.cfg.latency_ms {
        Some(l) => format!("latency {l:+.0} ms"),
        None => "calibrate latency".into(),
    };
    if p.button(1090.0, 22.0, 190.0, 38.0, &calib, false) {
        g.start_level(calibration_level(), true);
        return;
    }
    if p.button(1292.0, 22.0, 116.0, 38.0, "settings", false) {
        g.refresh_lists();
        g.screen = Screen::Settings;
        return;
    }
    for bpm in &bpms {
        p.text(col_x(*bpm), 96.0, format!("{bpm}"), 18.0, col::DIM, Align::Center);
    }
    p.text(col_x(BPM_MAX) + 50.0, 96.0, "BPM", 12.0, col::FAINT, Align::Left);

    // Tier bands with labels: track + the tier's drills.
    for &t in &tiers {
        let y = Y0 + (t - 1) as f32 * BAND_H;
        p.rect(32.0, y - BAND_H / 2.0 + 4.0, 1376.0, 1.0, col::PANEL);
        p.text(40.0, y - 10.0, roman(t), 28.0, col::BRASS, Align::Left);
        p.text(100.0, y - 10.0, "track", 15.0, col::DIM, Align::Left);
        let names: Vec<&str> = PATTERNS.iter().filter(|q| q.tier == t).map(|q| q.name).collect();
        p.text(100.0, y + 16.0, "drills", 11.0, col::FAINT, Align::Left);
        p.text(100.0, y + 32.0, truncate(&names.join(", "), 30), 11.0, col::FAINT, Align::Left);
    }

    let unlocked = |l: &Level| g.progress.is_unlocked(l);
    let passed = |l: &Level| g.progress.passed(l);
    // Edges between tracks
    for &t in &tiers {
        for &b in &bpms {
            let l = Level::track(t, b);
            let (x, y) = node_pos(&l);
            if b + BPM_STEP <= BPM_MAX {
                let r = Level::track(t, b + BPM_STEP);
                let (x2, y2) = node_pos(&r);
                let c = if passed(&l) || passed(&r) {
                    col::BRASS
                } else if unlocked(&l) || unlocked(&r) {
                    col::LINE
                } else {
                    col::PANEL2
                };
                p.line(x + R, y, x2 - R, y2, c);
            }
            if t < max_tier() {
                let d = Level::track(t + 1, b);
                let (x2, y2) = node_pos(&d);
                let c = if passed(&l) {
                    col::BRASS
                } else if unlocked(&l) {
                    col::LINE
                } else {
                    col::PANEL2
                };
                p.line(x, y + R + 32.0, x2, y2 - R, c);
            }
        }
    }

    // Nodes
    let mut clicked: Option<Level> = None;
    let mut hovered: Option<Level> = None;
    for &t in &tiers {
        for &b in &bpms {
            let mut levels = vec![Level::track(t, b)];
            levels.extend((0..PATTERNS.len()).filter(|&i| PATTERNS[i].tier == t).map(|i| Level::drill(i, b)));
            for l in levels {
                let (x, y) = node_pos(&l);
                let r = if l.is_track() { R } else { RS };
                let open = unlocked(&l);
                let best = g.progress.best(&l);
                let hov = p.cursor.is_some_and(|c| (c.x - x).powi(2) + (c.y - y).powi(2) <= (r + 4.0).powi(2));
                if hov {
                    hovered = Some(l);
                }
                if g.selected == l {
                    p.disc(x, y, r + 5.0, col::TEXT);
                    p.disc(x, y, r + 3.0, col::BG);
                }
                if passed(&l) {
                    p.disc(x, y, r, col::BRASS);
                    if l.is_track() {
                        p.text(x, y, format!("{:.0}", best.unwrap_or(0.0)), 13.0, col::BG, Align::Center);
                    }
                } else if open {
                    p.disc(x, y, r, if hov { col::TEXT } else { col::DIM });
                    p.disc(x, y, r - 2.0, if hov { col::PANEL2 } else { col::PANEL });
                    if let (true, Some(s)) = (l.is_track(), best) {
                        p.text(x, y, format!("{s:.0}"), 13.0, col::TEXT, Align::Center);
                    }
                } else {
                    p.disc(x, y, r * 0.45, col::LINE);
                }
                if l.is_track() && g.progress.fresh.contains(&l.key()) {
                    p.disc(x + r * 0.8, y - r * 0.8, 4.0, col::BRASS);
                }
                if hov && p.clicked && open {
                    clicked = Some(l);
                }
            }
        }
    }
    if let Some(l) = clicked {
        if g.selected == l {
            g.start_level(l, false);
            return;
        }
        g.selected = l;
    }

    // Keyboard: arrows move between unlocked tracks, Enter plays the selection.
    let (mut t, mut b) = (g.selected.tier(), g.selected.bpm);
    if keys.just_pressed(KeyCode::ArrowDown) {
        t = (t + 1).min(max_tier());
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        t = t.saturating_sub(1).max(1);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        b = (b + BPM_STEP).min(BPM_MAX);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        b = b.saturating_sub(BPM_STEP).max(BPM_MIN);
    }
    if (t, b) != (g.selected.tier(), g.selected.bpm) && unlocked(&Level::track(t, b)) {
        g.selected = Level::track(t, b);
    }
    if keys.just_pressed(KeyCode::Enter) {
        g.start_level(g.selected, false);
        return;
    }

    // One line about the hovered (else selected) level.
    let l = hovered.unwrap_or(g.selected);
    let mut info = format!("{}  {} BPM", l.name(), l.bpm);
    if !unlocked(&l) {
        info.push_str("   locked");
    } else if let Some(h) = g.progress.best_entry(&l.key()) {
        info.push_str(&format!("   best {:.0} %   sd {:.1} ms   attack {:.1} dB", h.combined, h.spread_ms, h.level_std_db));
    }
    if let LevelKind::Drill(pi) = l.kind {
        pattern_glyph(p, 32.0, 822.0, 240.0, 16.0, &PATTERNS[pi], col::BRASS);
    } else {
        let names: Vec<&str> = PATTERNS.iter().filter(|q| q.tier <= l.tier()).map(|q| q.name).collect();
        p.text(32.0, 806.0, format!("mixes {} in {}-bar sections", names.join(", "), crate::patterns::SEGMENT_BARS), 12.0, col::FAINT, Align::Left);
    }
    p.text(292.0, 830.0, info, 17.0, col::TEXT, Align::Left);
    p.text(1408.0, 830.0, "click again or Enter to play", 13.0, col::FAINT, Align::Right);

    if let Some((msg, bad)) = g.input_problem() {
        p.text(32.0, 872.0, truncate(&msg, 120), 14.0, if bad { col::MISS } else { col::LATE }, Align::Left);
        if p.button(1300.0, 856.0, 108.0, 32.0, "Retry", false) {
            g.restart_engine();
        }
    }
    if let Some(m) = &g.message {
        p.text(1408.0, 800.0, m.clone(), 14.0, col::MISS, Align::Right);
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "~" }
}
