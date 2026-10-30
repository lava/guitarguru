//! Main screen ("Guitar Guru" style): the guru reacting to the chosen tempo and
//! ingredients, ingredient pills, a row of BPM circles with unlock lines, the Play button,
//! the guru's stages strip and a hairline drills list.

use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;

use crate::game::{Game, Screen, calibration_level};
use crate::mascot::{self, MascotImages, Mood, STAGE_NAMES};
use crate::paint::{Align, Font, Paint, col, fit};
use crate::patterns::{BPM_MAX, BPM_MIN, BPM_STEP, INGREDIENTS, Level, LevelKind, PATTERNS, bpm_steps, is_riff_continuation};
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
    let calib = match g.cfg.latency_ms {
        Some(l) => format!("Latency {l:+.0} ms"),
        None => "Calibrate latency".into(),
    };
    if link(p, 1200.0, 56.0, &calib) {
        g.start_level(calibration_level(), true);
        return;
    }
    if link(p, 1320.0, 56.0, "Setup") {
        g.refresh_lists();
        g.screen = Screen::Settings;
        return;
    }

    // ---------- the guru (left), nodding along at the selected tempo
    let mix = g.mix;
    let sel_bpm = g.selected.bpm;
    let stage = mascot::stage_for(sel_bpm, mix.count_ones());
    let beats = g.clock * f64::from(sel_bpm) / 60.0;
    let accent = (beats.round() as i64).rem_euclid(4) == 0;
    let pose = mascot::pose(stage, Some(beats - beats.floor()), accent);
    mascot::draw(p, imgs, 220.0, 196.0, 1.15, stage, Mood::Neutral, &pose);
    p.label_at(357.0, 536.0, "today's guru", Align::Center, col::DIM);
    p.serif(357.0, 566.0, STAGE_NAMES[stage as usize], 36.0, col::TEXT, Align::Center);
    for i in 0..5 {
        p.rect(281.0 + i as f32 * 32.0, 595.0, 26.0, 2.0, if i < stage { col::OCHRE } else { col::LINE });
    }

    // ---------- ingredients
    p.label(RX, 214.0, "ingredients");
    let unlocked_mask = g.progress.unlocked_mask();
    let mut x = RX;
    let mut hint = None;
    for (i, ing) in INGREDIENTS.iter().enumerate() {
        let short = if ing.code == "16" { "16ths" } else { ing.name };
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
        } else {
            p.dashed_frame(x, y, w, h, col::LINE);
            p.dashed_ring(x + 24.0, y + h / 2.0, 6.0, col::FAINT);
            p.text(x + 40.0, y + h / 2.0 - 1.0, short, 16.0, col::FAINT, Align::Left);
            if hint.is_none() && i == g.progress.ingredients {
                hint = Some(if unlocked_mask == 0 {
                    format!("{} opens after a pass with the basic mix at 80+ BPM.", ing.name)
                } else {
                    format!("{} opens after a pass with {} at 80+ BPM.", ing.name, ingredient_list(unlocked_mask))
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
        if g.progress.fresh.contains(&l.key()) {
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

    // ---------- stages strip
    p.hline(120.0, 634.0, 1200.0, col::LINE);
    p.label(120.0, 670.0, "the guru's stages");
    for s in 1..=5u32 {
        let x = 120.0 + (s - 1) as f32 * 135.0;
        let still = mascot::pose(s, None, false);
        mascot::draw(p, imgs, x + 12.0, 688.0, 0.42, s, Mood::Neutral, &still);
        p.text(x + 62.0, 832.0, STAGE_NAMES[s as usize], 13.0, col::TEXT, Align::Center);
        if s == stage {
            p.rect(x, 850.0, 125.0, 2.0, col::OCHRE);
        }
    }

    // ---------- drills (fit the toggled ingredients, at the selected tempo)
    let (dx, dw) = (843.0, 477.0);
    p.label(dx, 670.0, &format!("drills \u{b7} {} bpm", sel.bpm));
    let drills: Vec<usize> = (0..PATTERNS.len()).filter(|&i| !is_riff_continuation(i) && PATTERNS[i].drill_needs() & !mix == 0).collect();
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
