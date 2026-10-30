//! Drawing helpers shared by several screens.

use bevy::color::Color;

use crate::paint::{Align, Paint, col};
use crate::patterns::Pattern;

/// 16-cell glyph of a pattern bar: onsets bright, sustain dim, rests empty.
pub fn pattern_glyph(p: &mut Paint, x: f32, y: f32, w: f32, h: f32, pattern: &Pattern, on: Color) {
    let cell = w / 16.0;
    let bar = pattern.bar();
    for i in 0..16 {
        let cx = x + i as f32 * cell;
        let c = if i % 4 == 0 { col::LINE } else { col::PANEL2 };
        p.rect(cx + 1.0, y, cell - 2.0, h, c);
    }
    for n in &bar {
        if n.rest {
            continue;
        }
        let x0 = x + n.pos as f32 * cell;
        let wlen = (n.len as f32 * cell - 2.0).max(2.0);
        p.rect(x0 + 1.0, y, wlen, h, dim(on, 0.45));
        p.rect(x0 + 1.0, y, (cell * 0.45).min(wlen), h, on);
    }
}

pub fn dim(c: Color, k: f32) -> Color {
    let a = c.to_srgba();
    let b = col::PANEL.to_srgba();
    Color::srgb(b.red + (a.red - b.red) * k, b.green + (a.green - b.green) * k, b.blue + (a.blue - b.blue) * k)
}


/// Classify a deviation for colour coding.
pub fn dev_color(dev_ms: f64) -> Color {
    if dev_ms.abs() <= 10.0 {
        col::TEXT
    } else if dev_ms < 0.0 {
        col::EARLY
    } else {
        col::LATE
    }
}

/// Hit marker: white dot (on time), blue left triangle (early), orange right triangle (late).
pub fn hit_marker(p: &mut Paint, x: f32, y: f32, r: f32, dev_ms: f64) {
    if dev_ms.abs() <= 10.0 {
        p.disc(x, y, r * 0.7, col::TEXT);
    } else {
        p.tri(x, y, r, dev_ms > 0.0, dev_color(dev_ms));
    }
}

/// Horizontal level meter in dBFS (-60..0) with the -18..-6 target zone and clip zone.
pub fn level_meter(p: &mut Paint, x: f32, y: f32, w: f32, h: f32, db: f32, peak_db: f32) {
    let pos = |d: f32| x + w * ((d + 60.0) / 60.0).clamp(0.0, 1.0);
    p.rect(x, y, w, h, col::BG);
    p.rect(pos(-18.0), y, pos(-6.0) - pos(-18.0), h, dim(col::GOOD, 0.35));
    p.rect(pos(-1.0), y, pos(0.0) - pos(-1.0), h, dim(col::MISS, 0.5));
    let c = if db > -1.0 {
        col::MISS
    } else if db > -18.0 {
        col::GOOD
    } else {
        col::DIM
    };
    p.rect(x, y + h * 0.25, pos(db) - x, h * 0.5, c);
    p.rect(pos(peak_db) - 1.0, y, 2.0, h, col::TEXT);
    p.frame(x, y, w, h, 1.0, col::LINE);
    for d in [-48.0, -36.0, -24.0, -18.0, -12.0, -6.0, 0.0] {
        p.text(pos(d), y + h + 9.0, format!("{d:.0}"), 11.0, col::FAINT, Align::Center);
    }
}

pub fn stepper(p: &mut Paint, x: f32, y: f32, label: &str, value: &str) -> i32 {
    p.label(x, y + 14.0, label);
    let mut d = 0;
    if p.button(x, y + 28.0, 36.0, 32.0, "-", false) {
        d = -1;
    }
    p.text(x + 90.0, y + 44.0, value, 18.0, col::TEXT, Align::Center);
    if p.button(x + 144.0, y + 28.0, 36.0, 32.0, "+", false) {
        d = 1;
    }
    d
}
