//! Main screen: ingredient toggles, a row of BPM nodes for the toggled combination (with
//! unlock edges between neighbours), and the drills that fit the combination.
//! Click to select, click again (or Enter) to play.

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;

use crate::game::{Game, Screen, calibration_level};
use crate::paint::{Align, Paint, col, fit};
use crate::patterns::{BPM_MAX, BPM_MIN, BPM_STEP, INGREDIENTS, Level, LevelKind, PATTERNS, bpm_steps, is_riff_continuation, mask_name};
use crate::ui_common::*;

const NODE_Y: f32 = 330.0;
const R: f32 = 34.0;

fn node_x(bpm: u32) -> f32 {
    230.0 + ((bpm - BPM_MIN) / BPM_STEP) as f32 * 140.0
}

pub fn draw(g: &mut Game, p: &mut Paint, keys: &ButtonInput<KeyCode>) {
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

    // ---------- ingredient toggles
    p.label(32.0, 124.0, "mix in");
    let unlocked_mask = g.progress.unlocked_mask();
    for (i, ing) in INGREDIENTS.iter().enumerate() {
        let (x, y, w, h) = (120.0 + i as f32 * 270.0, 100.0, 256.0, 48.0);
        let on = g.mix & ing.bit != 0;
        if g.progress.ingredient_unlocked(i) {
            let hov = p.hover(x, y, w, h);
            p.rect(x, y, w, h, if on { col::BRASS } else if hov { col::PANEL2 } else { col::PANEL });
            p.frame(x, y, w, h, 1.0, if on { col::BRASS } else { col::LINE });
            p.text(x + 16.0, y + h / 2.0, ing.name, 18.0, if on { col::BG } else { col::TEXT }, Align::Left);
            p.text(x + w - 14.0, y + h / 2.0, if on { "on" } else { "off" }, 13.0, if on { col::BG } else { col::FAINT }, Align::Right);
            if g.progress.fresh.contains(&format!("ingredient:{}", ing.code)) {
                p.disc(x + w - 6.0, y + 6.0, 5.0, col::BRASS);
            }
            if p.hit(x, y, w, h) {
                g.mix ^= ing.bit;
                g.progress.fresh.remove(&format!("ingredient:{}", ing.code));
                g.selected = Level::run(g.mix, g.selected.bpm);
            }
        } else {
            p.rect(x, y, w, h, col::BG);
            p.frame(x, y, w, h, 1.0, col::PANEL2);
            p.text(x + 16.0, y + h / 2.0, ing.name, 18.0, col::FAINT, Align::Left);
            let hint = if i == g.progress.ingredients {
                if unlocked_mask == 0 {
                    "unlock: pass basic at 80+ BPM".to_string()
                } else {
                    format!("unlock: pass with all {} on, 80+ BPM", unlocked_mask.count_ones())
                }
            } else {
                "locked".into()
            };
            p.text(x, y + h + 14.0, fit(&hint, w, 11.0), 11.0, col::FAINT, Align::Left);
        }
        p.text(x, y - 12.0, fit(ing.about, w, 10.0), 10.0, col::FAINT, Align::Left);
    }

    // ---------- BPM nodes for the toggled combination
    let mix = g.mix;
    let bpms: Vec<u32> = bpm_steps().collect();
    p.label(32.0, NODE_Y - 70.0, &format!("{}  -  tempo", mask_name(mix)));
    for &b in &bpms {
        if b + BPM_STEP > BPM_MAX {
            continue;
        }
        let (l, r) = (Level::run(mix, b), Level::run(mix, b + BPM_STEP));
        let c = if g.progress.passed(&l) || g.progress.passed(&r) {
            col::BRASS
        } else if g.progress.is_unlocked(&l) && g.progress.is_unlocked(&r) {
            col::LINE
        } else {
            col::PANEL2
        };
        p.rect(node_x(b) + R, NODE_Y - 1.0, 140.0 - 2.0 * R, 2.0, c);
    }
    let mut clicked: Option<Level> = None;
    let mut hovered: Option<Level> = None;
    for &b in &bpms {
        let l = Level::run(mix, b);
        let (x, y) = (node_x(b), NODE_Y);
        let open = g.progress.is_unlocked(&l);
        let best = g.progress.best(&l);
        let hov = p.cursor.is_some_and(|c| (c.x - x).powi(2) + (c.y - y).powi(2) <= (R + 4.0).powi(2));
        if hov {
            hovered = Some(l);
        }
        if g.selected == l {
            p.disc(x, y, R + 6.0, col::TEXT);
            p.disc(x, y, R + 3.0, col::BG);
        }
        if g.progress.passed(&l) {
            p.disc(x, y, R, col::BRASS);
            p.text(x, y, format!("{:.0}", best.unwrap_or(0.0)), 20.0, col::BG, Align::Center);
        } else if open {
            p.disc(x, y, R, if hov { col::TEXT } else { col::DIM });
            p.disc(x, y, R - 2.0, if hov { col::PANEL2 } else { col::PANEL });
            if let Some(s) = best {
                p.text(x, y, format!("{s:.0}"), 20.0, col::TEXT, Align::Center);
            }
        } else {
            p.disc(x, y, R * 0.35, col::LINE);
        }
        if g.progress.fresh.contains(&l.key()) {
            p.disc(x + R * 0.75, y - R * 0.75, 5.0, col::BRASS);
        }
        p.text(x, y + R + 18.0, format!("{b}"), 16.0, if open { col::DIM } else { col::FAINT }, Align::Center);
        if hov && p.clicked && open {
            clicked = Some(l);
        }
    }
    p.text(node_x(BPM_MAX) + R + 14.0, NODE_Y + R + 18.0, "BPM", 11.0, col::FAINT, Align::Left);

    // ---------- drills fitting the toggled combination, at the selected tempo
    let bpm = g.selected.bpm;
    let drills: Vec<usize> =
        (0..PATTERNS.len()).filter(|&i| !is_riff_continuation(i) && PATTERNS[i].drill_needs() & !mix == 0).collect();
    let dy0 = 470.0;
    p.label(32.0, dy0, &format!("drills at {bpm} BPM"));
    let (cw, chh, per_row) = (224.0, 30.0, 6usize);
    for (k, &pi) in drills.iter().enumerate() {
        let d = Level::drill(pi, bpm);
        let (cx, cy) = (32.0 + (k % per_row) as f32 * (cw + 6.0), dy0 + 16.0 + (k / per_row) as f32 * (chh + 5.0));
        let open = g.progress.is_unlocked(&d);
        let best = g.progress.best(&d).map_or(String::new(), |b| format!("{b:.0}"));
        let hov = p.hover(cx, cy, cw, chh);
        if hov {
            hovered = Some(d);
        }
        let sel = g.selected == d;
        p.rect(cx, cy, cw, chh, if sel || hov { col::PANEL2 } else { col::PANEL });
        p.frame(cx, cy, cw, chh, 1.0, if sel { col::TEXT } else if g.progress.passed(&d) { col::BRASS } else { col::LINE });
        let c = if !open {
            col::FAINT
        } else if g.progress.passed(&d) {
            col::BRASS
        } else {
            col::DIM
        };
        p.text(cx + 8.0, cy + chh / 2.0, fit(PATTERNS[pi].name, cw - 46.0, 13.0), 13.0, c, Align::Left);
        p.text(cx + cw - 8.0, cy + chh / 2.0, best, 13.0, c, Align::Right);
        if hov && p.clicked && open {
            clicked = Some(d);
        }
    }

    if let Some(l) = clicked {
        if g.selected == l {
            g.start_level(l, false);
            return;
        }
        g.selected = l;
    }

    // Keyboard: left/right change tempo, Enter plays the selection.
    let mut b = g.selected.bpm;
    if keys.just_pressed(KeyCode::ArrowRight) {
        b = (b + BPM_STEP).min(BPM_MAX);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        b = b.saturating_sub(BPM_STEP).max(BPM_MIN);
    }
    if b != g.selected.bpm {
        let l = Level { kind: g.selected.kind, bpm: b };
        if g.progress.is_unlocked(&l) {
            g.selected = l;
        }
    }
    if keys.just_pressed(KeyCode::Enter) && g.progress.is_unlocked(&g.selected) {
        g.start_level(g.selected, false);
        return;
    }

    // ---------- info line about the hovered (else selected) level, and Play
    let l = hovered.unwrap_or(g.selected);
    let mut info = format!("{}  {} BPM", l.name(), l.bpm);
    if !g.progress.is_unlocked(&l) {
        info.push_str("   locked");
    } else if let Some(h) = g.progress.best_entry(&l.key()) {
        info.push_str(&format!("   best {:.0} %   sd {:.1} ms   attack {:.1} dB", h.combined, h.spread_ms, h.level_std_db));
    }
    if let LevelKind::Drill(pi) = l.kind {
        pattern_glyph(p, 32.0, 834.0, 200.0, 16.0, &PATTERNS[pi], col::BRASS);
    } else {
        p.text(32.0, 842.0, format!("quarters, then {}-bar sections", crate::patterns::SEGMENT_BARS), 12.0, col::FAINT, Align::Left);
    }
    p.text(260.0, 842.0, fit(&info, 840.0, 17.0), 17.0, col::TEXT, Align::Left);
    if g.progress.is_unlocked(&g.selected) && p.button(1250.0, 818.0, 158.0, 46.0, "Play", true) {
        g.start_level(g.selected, false);
        return;
    }

    if let Some((msg, bad)) = g.input_problem() {
        p.text(32.0, 884.0, truncate(&msg, 120), 13.0, if bad { col::MISS } else { col::LATE }, Align::Left);
        if p.button(1130.0, 818.0, 108.0, 46.0, "Retry", false) {
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
