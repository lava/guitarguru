//! Main menu ("Guitar Guru" style): the guru keeping time at the chosen tempo, his stage
//! from XP with a small bar under him that fills after each run, ingredient pills (locked
//! ones can be picked with an unlock token), a row of BPM circles with unlock lines, the
//! Play button and a hairline drills list.

use bevy::color::Alpha;
use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;

use crate::game::{Game, Screen};
use crate::mascot::{self, MascotImages, Mood, STAGE_NAMES};
use crate::paint::{Align, Font, Paint, col, fit};
use crate::patterns::{BPM_MAX, BPM_MIN, BPM_STEP, INGREDIENTS, Level, LevelKind, PATTERNS, bpm_steps, drills};
use crate::progress::PASS_THRESHOLD;

const RX: f32 = 653.0;

fn node_x(bpm: u32) -> f32 {
    694.0 + ((bpm - BPM_MIN) / BPM_STEP) as f32 * 83.5
}

/// Header: lotus, title, gold block-print band (main screen) or a hairline.
pub fn header(p: &mut Paint, title: &str, band: bool) {
    lotus(p, 137.0, 62.0, 1.0);
    p.italic(170.0, 56.0, title, 46.0, col::TEXT, Align::Left);
    if band {
        let (y0, y1) = (97.0, 111.0);
        p.hline(120.0, y0, 1200.0, col::GOLD);
        p.hline(120.0, y1, 1200.0, col::GOLD);
        let mut x = 122.0;
        let mut k = 0;
        while x < 1320.0 {
            if k % 2 == 0 {
                p.rot_rect(x, 104.0, 5.0, 5.0, std::f32::consts::FRAC_PI_4, col::GOLD);
                p.rot_rect(x, 104.0, 2.4, 2.4, std::f32::consts::FRAC_PI_4, col::BG);
            } else {
                p.disc(x, 104.0, 1.3, col::GOLD);
            }
            x += 14.0;
            k += 1;
        }
    } else {
        p.hline(120.0, 97.0, 1200.0, col::LINE);
    }
}

/// Small lotus logo (petal outlines in ochre).
pub fn lotus(p: &mut Paint, cx: f32, cy: f32, s: f32) {
    for (dx, rot, len) in [(-12.0f32, 1.1f32, 14.0f32), (12.0, -1.1, 14.0), (-7.0, 0.55, 18.0), (7.0, -0.55, 18.0), (0.0, 0.0, 22.0)] {
        let (x, y) = (cx + dx * s, cy - len * 0.32 * s);
        p.petal(x, y, len * s, len * 0.45 * s, rot, col::OCHRE);
        p.petal(x, y + 0.8 * s, len * s * 0.8, len * 0.45 * s * 0.6, rot, col::BG);
    }
    p.hline(cx - 14.0 * s, cy + 2.0 * s, 28.0 * s, col::OCHRE);
}

/// Underlined text link (right-aligned at x); returns true when clicked.
pub fn link(p: &mut Paint, x: f32, y: f32, s: &str) -> bool {
    let w = s.len() as f32 * 8.2;
    let hov = p.hover(x - w, y - 12.0, w, 26.0);
    p.styled(x, y, s, 15.0, if hov { col::OCHRE } else { col::TEXT }, Align::Right, Font::SansBold, 0.0);
    p.hline(x - w, y + 11.0, w, if hov { col::OCHRE } else { col::TEXT });
    p.clicked && hov
}

fn ingredient_list(mask: u8) -> String {
    let names: Vec<String> = INGREDIENTS.iter().filter(|i| mask & i.bit != 0).map(|i| i.name.to_lowercase()).collect();
    names.join(", ")
}

pub fn draw(g: &mut Game, p: &mut Paint, keys: &ButtonInput<KeyCode>, imgs: &MascotImages) {
    header(p, "Guitar Guru", true);
    if link(p, 1320.0, 56.0, "Setup") {
        g.refresh_lists();
        g.screen = Screen::Settings;
        return;
    }

    // ---------- the guru (left), a metronome at the selected tempo, his stage from XP
    let mix = g.mix;
    let sel_bpm = g.selected.bpm;
    guru(g, p, imgs, sel_bpm);

    // ---------- songs to try (reached milestones), under the guru
    let songs = crate::songs::reached(&g.progress);
    if !songs.is_empty() {
        p.label(170.0, 690.0, "songs to try");
        for (i, s) in songs.iter().rev().take(6).enumerate() {
            p.text(170.0, 716.0 + i as f32 * 22.0, fit(&s.label(), 420.0, 13.0), 13.0, col::DIM, Align::Left);
        }
    }

    // ---------- ingredients
    p.label(RX, 214.0, "ingredients");
    let unlocked_mask = g.progress.unlocked_mask();
    let mut x = RX;
    let mut hint = None;
    for (i, ing) in INGREDIENTS.iter().enumerate() {
        let short = ing.short;
        let w = 62.0 + short.len() as f32 * 8.4;
        let (y, h) = (236.0, 42.0);
        let on = mix & ing.bit != 0;
        if g.progress.ingredient_unlocked(i) {
            let hov = p.hover(x, y, w, h);
            if on {
                p.rect(x, y, w, h, col::OCHRE_TINT);
                p.frame(x, y, w, h, 1.0, col::OCHRE);
                p.disc(x + 24.0, y + h / 2.0, 6.0, col::OCHRE);
            } else {
                if hov {
                    p.rect(x, y, w, h, col::PANEL);
                }
                p.frame(x, y, w, h, 1.0, col::TEXT);
                p.ring(x + 24.0, y + h / 2.0, 6.0, col::TEXT);
            }
            p.styled(x + 40.0, y + h / 2.0 - 1.0, short, 16.0, col::TEXT, Align::Left, Font::SansMedium, 0.0);
            if g.progress.fresh.contains(&format!("ingredient:{}", ing.code)) {
                p.disc(x + w - 6.0, y + 6.0, 4.0, col::OCHRE);
            }
            if p.hit(x, y, w, h) {
                g.mix ^= ing.bit;
                g.progress.fresh.remove(&format!("ingredient:{}", ing.code));
                g.selected = Level::run(g.mix, g.selected.bpm);
            }
        } else if g.progress.unlock_tokens > 0 {
            // An unlock to spend: every locked ingredient can be picked.
            let hov = p.hover(x, y, w, h);
            if hov {
                p.rect(x, y, w, h, col::OCHRE_TINT);
            }
            p.dashed_frame(x, y, w, h, col::OCHRE);
            p.dashed_ring(x + 24.0, y + h / 2.0, 6.0, col::OCHRE);
            p.text(x + 40.0, y + h / 2.0 - 1.0, short, 16.0, col::DIM, Align::Left);
            p.text(x + w / 2.0, y + h + 14.0, "click to unlock", 11.0, if hov { col::OCHRE } else { col::DIM }, Align::Center);
            if p.hit(x, y, w, h) && g.progress.unlock_ingredient(i) {
                g.progress.save();
            }
        } else {
            p.dashed_frame(x, y, w, h, col::LINE);
            p.dashed_ring(x + 24.0, y + h / 2.0, 6.0, col::FAINT);
            p.text(x + 40.0, y + h / 2.0 - 1.0, short, 16.0, col::FAINT, Align::Left);
            if hint.is_none() {
                hint = Some(if unlocked_mask == 0 {
                    "Pass the basic mix at 80+ BPM to unlock an ingredient of your choice.".to_string()
                } else {
                    format!("Pass with {} at 80+ BPM to unlock another ingredient of your choice.", ingredient_list(unlocked_mask))
                });
            }
        }
        x += w + 10.0;
    }
    if let Some(h) = hint {
        p.text(RX, 296.0, fit(&h, 680.0, 13.0), 13.0, col::DIM, Align::Left);
    }

    // ---------- tempo row
    p.label(RX, 352.0, "tempo \u{b7} bpm");
    let bpms: Vec<u32> = bpm_steps().collect();
    let r = 26.0;
    let cy = 402.0;
    for &b in &bpms {
        if b + BPM_STEP > BPM_MAX {
            continue;
        }
        let (l, rr) = (Level::run(mix, b), Level::run(mix, b + BPM_STEP));
        let (x0, x1) = (node_x(b) + r, node_x(b + BPM_STEP) - r);
        if g.progress.passed(&l) || g.progress.passed(&rr) {
            p.rect(x0, cy - 0.75, x1 - x0, 1.5, col::ONTIME);
        } else if g.progress.is_unlocked(&l) && g.progress.is_unlocked(&rr) {
            p.rect(x0, cy - 0.5, x1 - x0, 1.0, col::LINE);
        } else {
            let mut xx = x0 + 2.0;
            while xx < x1 - 2.0 {
                p.disc(xx, cy, 0.8, col::LINE);
                xx += 6.0;
            }
        }
    }
    let mut clicked: Option<Level> = None;
    let mut hovered: Option<Level> = None;
    for &b in &bpms {
        let l = Level::run(mix, b);
        let x = node_x(b);
        let open = g.progress.is_unlocked(&l);
        let passed = g.progress.passed(&l);
        let best = g.progress.best(&l);
        let hov = p.hover_circle(x, cy, r + 3.0);
        if hov {
            hovered = Some(l);
        }
        if g.selected == l {
            p.disc(x, cy, r, col::TEXT);
            p.serif(x, cy - 2.0, format!("{b}"), 22.0, col::BG, Align::Center);
        } else if passed {
            p.disc(x, cy, r, col::TEAL_TINT);
            p.ring(x, cy, r, col::ONTIME);
            p.serif(x, cy - 2.0, format!("{b}"), 22.0, col::ONTIME, Align::Center);
        } else if open {
            if hov {
                p.disc(x, cy, r, col::PANEL);
            }
            p.ring(x, cy, r, col::TEXT);
            p.serif(x, cy - 2.0, format!("{b}"), 22.0, col::TEXT, Align::Center);
        } else {
            p.dashed_ring(x, cy, r, col::LINE);
            p.serif(x, cy - 2.0, format!("{b}"), 22.0, col::FAINT, Align::Center);
        }
        if g.progress.fresh.contains(&format!("tempo:{b}")) {
            p.disc(x + r * 0.75, cy - r * 0.75, 4.0, col::OCHRE);
        }
        let below = if !open {
            "locked".to_string()
        } else if let Some(s) = best {
            format!("{s:.0}%")
        } else {
            "open".into()
        };
        p.text(x, cy + 42.0, below, 13.0, if passed { col::ONTIME } else { col::DIM }, Align::Center);
        if hov && p.clicked && open {
            clicked = Some(l);
        }
    }

    // ---------- play button
    let sel = g.selected;
    let can_play = g.progress.is_unlocked(&sel);
    let (main, sub) = match sel.kind {
        LevelKind::Run(m) => {
            (format!("Play {} BPM", sel.bpm), if m == 0 { "basic".to_string() } else { format!("with {}", ingredient_list(m)) })
        }
        LevelKind::Drill(pi) => (format!("Drill {} BPM", sel.bpm), PATTERNS[pi].name.to_string()),
    };
    let (bx, by, bh) = (RX, 488.0, 56.0);
    let main_w = main.len() as f32 * 12.6;
    let bw = 52.0 + main_w + sub.len() as f32 * 6.8;
    let hov = p.hover(bx, by, bw, bh);
    p.rect(bx, by, bw, bh, if !can_play { col::FAINT } else if hov { col::DIM } else { col::TEXT });
    p.serif(bx + 26.0, by + bh / 2.0 - 3.0, &main, 28.0, col::BG, Align::Left);
    p.text(bx + 36.0 + main_w, by + bh / 2.0 + 3.0, &sub, 13.0, col::BG, Align::Left);
    if can_play && (p.hit(bx, by, bw, bh) || keys.just_pressed(KeyCode::Enter)) {
        g.start_level(sel, false);
        return;
    }

    // ---------- drills (at the selected tempo; locked until their ingredients are)
    let (dx, dw) = (843.0, 477.0);
    p.label(dx, 670.0, &format!("drills \u{b7} {} bpm", sel.bpm));
    let drills = drills();
    let (row_h, visible) = (40.0, 5usize);
    let max_first = drills.len().saturating_sub(visible);
    if p.hover(dx, 686.0, dw, row_h * visible as f32) && p.scroll != 0.0 {
        let cur = (g.scroll / row_h) as usize;
        let next = if p.scroll < 0.0 { (cur + 1).min(max_first) } else { cur.saturating_sub(1) };
        g.scroll = next as f32 * row_h;
    }
    let first = ((g.scroll / row_h) as usize).min(max_first);
    for (k, &pi) in drills.iter().skip(first).take(visible).enumerate() {
        let d = Level::drill(pi, sel.bpm);
        let y = 686.0 + k as f32 * row_h;
        let open = g.progress.is_unlocked(&d);
        let hov = p.hover(dx, y, dw, row_h);
        if hov {
            hovered = Some(d);
            p.rect(dx, y, dw, row_h - 1.0, col::PANEL);
        }
        if g.selected == d {
            p.rect(dx - 10.0, y + 8.0, 2.0, row_h - 16.0, col::OCHRE);
        }
        let c = if open { col::TEXT } else { col::FAINT };
        p.styled(dx, y + row_h / 2.0, fit(&PATTERNS[pi].name, dw - 150.0, 16.0), 16.0, c, Align::Left, Font::SansMedium, 0.0);
        p.text(dx + dw - 48.0, y + row_h / 2.0 + 2.0, if open { format!("{} BPM", sel.bpm) } else { "locked".into() }, 12.0, col::DIM, Align::Right);
        let best = g.progress.best(&d).map_or("\u{2013}".to_string(), |b| format!("{b:.0}"));
        p.serif(dx + dw, y + row_h / 2.0 - 2.0, best, 22.0, if g.progress.passed(&d) { col::ONTIME } else { col::TEXT }, Align::Right);
        p.hline(dx, y + row_h - 1.0, dw, col::LINE);
        if hov && p.clicked && open {
            clicked = Some(d);
        }
    }
    if drills.len() > visible {
        p.text(dx + dw, 670.0, format!("{}\u{2013}{} of {} \u{b7} scroll", first + 1, (first + visible).min(drills.len()), drills.len()), 11.0, col::FAINT, Align::Right);
    }

    if let Some(l) = clicked {
        if g.selected == l {
            g.start_level(l, false);
            return;
        }
        g.selected = l;
    }
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

    // ---------- status lines
    if let Some(l) = hovered
        && let Some(h) = g.progress.best_entry(&l.key())
    {
        let pass = if h.combined >= PASS_THRESHOLD { "passed" } else { "not passed yet" };
        p.text(RX, 578.0, format!("{} {} BPM \u{b7} best {:.0}% ({pass}) \u{b7} evenness {:.1} ms", l.name(), l.bpm, h.combined, h.spread_ms), 13.0, col::DIM, Align::Left);
    }
    if let Some((msg, bad)) = g.input_problem() {
        p.text(RX, 604.0, fit(&msg, 560.0, 13.0), 13.0, if bad { col::LATE } else { col::DIM }, Align::Left);
        if link(p, 1320.0, 604.0, "Retry") {
            g.restart_engine();
        }
    }
    if let Some(m) = &g.message {
        p.text(RX, 620.0, m.clone(), 13.0, col::LATE, Align::Left);
    }
}

/// The guru at the selected tempo, his stage from XP, with the XP bar under him. After a
/// run the bar fills from the XP last shown to the current total; crossing a stage
/// threshold cross-fades him into the next stage inside a widening gold ring.
fn guru(g: &mut Game, p: &mut Paint, imgs: &MascotImages, bpm: u32) {
    let (gx, gy, gs) = (220.0, 196.0, 1.15);
    let (from, to) = (g.progress.xp_seen, g.progress.xp);
    let shown = crate::xp::animate(from, to, g.clock - g.screen_since - 0.6);
    if shown.done && from != to {
        g.progress.xp_seen = to;
        g.progress.save();
    }
    let stage = crate::xp::stage(shown.xp as u32);
    let beats = g.clock * f64::from(bpm) / 60.0;
    let mood = if shown.done { Mood::Neutral } else { Mood::Happy };
    let (cx, cy) = (gx + 120.0 * gs, gy + 140.0 * gs);
    match shown.stage_up {
        Some((next, u)) => {
            let u = u as f32;
            for (k, delay) in [(0, 0.0f32), (1, 0.18)] {
                let v = ((u - delay) / (1.0 - delay)).clamp(0.0, 1.0);
                if v > 0.0 && v < 1.0 {
                    let a = (1.0 - v) * 0.9;
                    let r = 110.0 + 120.0 * v;
                    if k == 0 {
                        p.ring(cx, cy, r, col::GOLD.with_alpha(a));
                    } else {
                        p.dotted_ring(cx, cy, r, 1.2, col::GOLD.with_alpha(a));
                    }
                }
            }
            let fade_in = (u / 0.55).clamp(0.0, 1.0);
            let fade_in = fade_in * fade_in * (3.0 - 2.0 * fade_in);
            if fade_in < 1.0 {
                mascot::draw(p, imgs, gx, gy, gs, stage, mood, &mascot::pose(stage, Some(beats)));
            }
            mascot::draw_faded(p, imgs, gx, gy, gs, next, mood, &mascot::pose(next, Some(beats)), fade_in);
        }
        None => mascot::draw(p, imgs, gx, gy, gs, stage, mood, &mascot::pose(stage, Some(beats))),
    }

    // The bar: a hairline track between two small gold diamonds, the fill in ochre.
    let (bx, by, bw) = (cx - 96.0, 512.0, 192.0);
    let full = shown.stage_up.is_some();
    let f = if full { 1.0 } else { crate::xp::fill(shown.xp) as f32 };
    p.rect(bx, by - 0.5, bw, 1.0, col::LINE);
    p.rect(bx, by - 1.5, bw * f, 3.0, if stage >= crate::xp::MAX_STAGE { col::GOLD } else { col::OCHRE });
    for x in [bx - 6.0, bx + bw + 6.0] {
        p.rot_rect(x, by, 5.0, 5.0, std::f32::consts::FRAC_PI_4, col::GOLD);
    }
    let name_stage = shown.stage_up.map_or(stage, |(n, _)| n);
    let name_col = if full { col::OCHRE } else { col::DIM };
    p.label_at(bx - 8.0, by + 20.0, STAGE_NAMES[name_stage as usize], Align::Left, name_col);
    if stage < crate::xp::MAX_STAGE {
        let (a, b) = (crate::xp::STAGE_XP[stage as usize - 1], crate::xp::STAGE_XP[stage as usize]);
        let have = if full { b - a } else { (shown.xp as u32).saturating_sub(a) };
        p.text(bx + bw + 8.0, by + 20.0, format!("{have} / {} xp", b - a), 11.0, col::FAINT, Align::Right);
    }
}
