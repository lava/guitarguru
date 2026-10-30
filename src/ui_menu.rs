//! Level select: tier rows x BPM columns, stats header, selected-level side panel.

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;

use crate::game::{Game, Screen, calibration_pattern, CALIBRATION_BPM};
use crate::paint::{Align, Paint, col};
use crate::patterns::{PATTERNS, bpm_steps};
use crate::progress::{PASS_THRESHOLD, level_key};
use crate::ui_common::*;

pub fn draw(g: &mut Game, p: &mut Paint, keys: &ButtonInput<KeyCode>) {
    let bpms: Vec<u32> = bpm_steps().collect();
    // Header
    p.text(32.0, 42.0, "GUITARGURU", 38.0, col::BRASS, Align::Left);
    p.text(34.0, 74.0, "metal rhythm trainer", 14.0, col::DIM, Align::Left);
    let total = PATTERNS.len() * bpms.len();
    let passed = g.progress.best.values().filter(|v| **v >= PASS_THRESHOLD).count();
    let hist = &g.progress.history;
    let recent: Vec<&crate::progress::HistoryEntry> = hist.iter().rev().take(12).rev().collect();
    let mut sx = 380.0;
    p.label(sx, 30.0, "levels passed");
    p.text(sx, 58.0, format!("{passed} / {total}"), 26.0, col::TEXT, Align::Left);
    sx += 170.0;
    p.label(sx, 30.0, "timing sd");
    let last_sd = recent.last().map_or("-".into(), |h| format!("{:.1} ms", h.spread_ms));
    p.text(sx, 58.0, last_sd, 26.0, col::TEXT, Align::Left);
    sparkline(p, sx + 100.0, 46.0, 70.0, 22.0, &recent.iter().map(|h| -h.spread_ms).collect::<Vec<_>>(), col::BRASS);
    sx += 210.0;
    p.label(sx, 30.0, "attack sd");
    let last_lv = recent.last().map_or("-".into(), |h| format!("{:.1} dB", h.level_std_db));
    p.text(sx, 58.0, last_lv, 26.0, col::TONE, Align::Left);
    sparkline(p, sx + 100.0, 46.0, 70.0, 22.0, &recent.iter().map(|h| -h.level_std_db).collect::<Vec<_>>(), col::TONE);
    sx += 210.0;
    p.label(sx, 30.0, "practice");
    let secs = g.progress.practice_seconds() as u64;
    p.text(sx, 58.0, format!("{}h {:02}m", secs / 3600, (secs / 60) % 60), 26.0, col::TEXT, Align::Left);

    let calib = match g.cfg.latency_ms {
        Some(l) => format!("Latency {l:+.0} ms"),
        None => "Calibrate latency".into(),
    };
    if p.button(1110.0, 26.0, 170.0, 40.0, &calib, false) {
        g.start_level(calibration_pattern(), CALIBRATION_BPM, true);
        return;
    }
    if p.button(1292.0, 26.0, 116.0, 40.0, "Settings", false) {
        g.refresh_lists();
        g.screen = Screen::Settings;
        return;
    }

    // Grid
    let (gx, gy, head_w, cell_w, row_h) = (32.0, 112.0, 300.0, 92.0, 66.0);
    for (c, bpm) in bpms.iter().enumerate() {
        p.text(gx + head_w + c as f32 * cell_w + cell_w / 2.0, gy, format!("{bpm}"), 18.0, col::DIM, Align::Center);
    }
    p.text(gx + head_w + bpms.len() as f32 * cell_w + 8.0, gy, "BPM", 12.0, col::FAINT, Align::Left);
    let y0 = gy + 18.0;
    let mut last_tier = 0;
    for (r, pat) in PATTERNS.iter().enumerate() {
        let y = y0 + r as f32 * row_h;
        if pat.tier != last_tier {
            p.rect(gx, y, head_w + bpms.len() as f32 * cell_w, 1.0, col::LINE);
            p.text(gx + 4.0, y + row_h / 2.0, roman(pat.tier), 28.0, col::BRASS, Align::Left);
            last_tier = pat.tier;
        }
        p.text(gx + 60.0, y + 20.0, pat.name, 18.0, col::TEXT, Align::Left);
        p.text(gx + 250.0, y + 20.0, "PM", 11.0, col::DIM, Align::Left);
        pattern_glyph(p, gx + 60.0, y + 36.0, 220.0, 14.0, pat, col::TEXT);
        for (c, bpm) in bpms.iter().enumerate() {
            let (cx, cy, cw, ch) = (gx + head_w + c as f32 * cell_w + 4.0, y + 6.0, cell_w - 8.0, row_h - 12.0);
            let key = level_key(pat.id, *bpm);
            let unlocked = g.progress.is_unlocked(pat.id, *bpm);
            let best = g.progress.best(pat.id, *bpm);
            let selected = g.selected == (r, *bpm);
            if unlocked {
                let pass = best.is_some_and(|b| b >= PASS_THRESHOLD);
                let hov = p.hover(cx, cy, cw, ch);
                p.rect(cx, cy, cw, ch, if pass { col::PANEL2 } else if hov { col::PANEL2 } else { col::PANEL });
                p.frame(cx, cy, cw, ch, 1.0, if pass { col::BRASS } else { col::LINE });
                let txt = best.map_or("-".to_string(), |b| format!("{b:.0}"));
                p.text(cx + cw / 2.0, cy + ch / 2.0 - 5.0, txt, 22.0, if pass { col::BRASS } else { col::TEXT }, Align::Center);
                // best-score bar with 80 % tick
                let bw = cw - 12.0;
                p.rect(cx + 6.0, cy + ch - 9.0, bw, 3.0, col::LINE);
                if let Some(b) = best {
                    p.rect(cx + 6.0, cy + ch - 9.0, bw * (b / 100.0) as f32, 3.0, if pass { col::BRASS } else { col::DIM });
                }
                p.rect(cx + 6.0 + bw * 0.8, cy + ch - 12.0, 1.0, 9.0, col::TEXT);
                if g.progress.fresh.contains(&key) {
                    p.disc(cx + cw - 8.0, cy + 8.0, 4.0, col::BRASS);
                }
                if p.hit(cx, cy, cw, ch) {
                    if selected {
                        g.start_level(pat, *bpm, false);
                        return;
                    }
                    g.selected = (r, *bpm);
                }
            } else {
                p.dashed_frame(cx, cy, cw, ch, col::LINE);
                // lock icon
                let (lx, ly) = (cx + cw / 2.0, cy + ch / 2.0);
                p.frame(lx - 5.0, ly - 10.0, 10.0, 9.0, 2.0, col::FAINT);
                p.rect(lx - 8.0, ly - 2.0, 16.0, 12.0, col::FAINT);
            }
            if selected {
                p.frame(cx - 2.0, cy - 2.0, cw + 4.0, ch + 4.0, 2.0, col::TEXT);
            }
        }
    }

    // Keyboard navigation
    let (mut r, mut b) = g.selected;
    if keys.just_pressed(KeyCode::ArrowDown) {
        r = (r + 1).min(PATTERNS.len() - 1);
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        r = r.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        b = (b + crate::patterns::BPM_STEP).min(crate::patterns::BPM_MAX);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        b = b.saturating_sub(crate::patterns::BPM_STEP).max(crate::patterns::BPM_MIN);
    }
    if PATTERNS.get(r).is_some_and(|pt| g.progress.is_unlocked(pt.id, b)) {
        g.selected = (r, b);
    }

    // Side panel
    let (px, py, pw, ph) = (1110.0, 112.0, 298.0, 690.0);
    p.panel(px, py, pw, ph);
    let pat = g.selected_pattern();
    let bpm = g.selected.1;
    let key = level_key(pat.id, bpm);
    p.label(px + 18.0, py + 24.0, &format!("tier {}", roman(pat.tier)));
    p.text(px + 18.0, py + 56.0, pat.name, 28.0, col::TEXT, Align::Left);
    p.text(px + 18.0, py + 88.0, format!("{bpm} BPM  -  4/4  -  palm-muted"), 16.0, col::DIM, Align::Left);
    pattern_glyph(p, px + 18.0, py + 108.0, pw - 36.0, 26.0, pat, col::BRASS);
    p.text(px + 18.0, py + 150.0, pat.notation, 13.0, col::FAINT, Align::Left);
    let best = g.progress.best_entry(&key).cloned();
    p.label(px + 18.0, py + 186.0, "best");
    p.text(px + 18.0, py + 216.0, best.as_ref().map_or("-".into(), |b| format!("{:.0} %", b.combined)), 30.0, col::BRASS, Align::Left);
    p.label(px + 150.0, py + 186.0, "timing sd");
    p.text(px + 150.0, py + 216.0, best.as_ref().map_or("-".into(), |b| format!("{:.1} ms", b.spread_ms)), 22.0, col::TEXT, Align::Left);
    p.label(px + 18.0, py + 258.0, "history");
    let hist: Vec<crate::progress::HistoryEntry> = g.progress.history_for(&key).into_iter().rev().take(10).cloned().collect();
    for (i, h) in hist.iter().enumerate() {
        let y = py + 284.0 + i as f32 * 26.0;
        p.text(px + 18.0, y, &h.date, 14.0, col::DIM, Align::Left);
        p.text(px + 190.0, y, format!("{:.1} ms", h.spread_ms), 14.0, col::DIM, Align::Right);
        let c = if h.combined >= PASS_THRESHOLD { col::BRASS } else { col::TEXT };
        p.text(px + pw - 18.0, y, format!("{:.0} %", h.combined), 16.0, c, Align::Right);
    }
    if hist.is_empty() {
        p.text(px + 18.0, py + 284.0, "no runs yet", 14.0, col::FAINT, Align::Left);
    }
    // input meter
    p.label(px + 18.0, py + ph - 140.0, "input");
    level_meter(p, px + 18.0, py + ph - 126.0, pw - 36.0, 14.0, g.meter_db, g.meter_peak_db);
    if let Some(e) = &g.engine_err {
        p.text(px + 18.0, py + ph - 88.0, format!("audio: {}", truncate(e, 34)), 13.0, col::MISS, Align::Left);
    } else if let Some(e) = &g.engine {
        p.text(px + 18.0, py + ph - 88.0, truncate(&e.info.input_label, 40), 12.0, col::FAINT, Align::Left);
        if let Some(w) = &e.info.warning {
            p.text(32.0, 840.0, w.clone(), 16.0, col::LATE, Align::Left);
        }
    }
    if p.button(px + 18.0, py + ph - 64.0, pw - 36.0, 48.0, "Play  (Enter)", true) || keys.just_pressed(KeyCode::Enter) {
        g.start_level(pat, bpm, false);
    }
    if let Some(m) = &g.message {
        p.text(32.0, 860.0, m.clone(), 16.0, col::MISS, Align::Left);
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "~" }
}
