//! Main menu ("Guitar Guru" style). A left rail holds the logo, the guru keeping time at the
//! chosen tempo (his stage from XP, with a small bar under him that fills after each run),
//! the mode switch between the tempo ladder and drills, and Setup. The main area shows the
//! tempo ladder (BPM circles with unlock lines and a grade on every played tempo), the
//! options of the mode (ingredient checkboxes for runs, where locked ones can be picked with
//! an unlock token; the drill pattern for drills), the songs of the selected tempo (hidden
//! ones as question marks, their requirements on hover) and the Play button bottom right.

use bevy::color::Alpha;
use bevy::input::ButtonInput;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::Color;

use crate::game::{Game, Screen};
use crate::mascot::{self, MascotImages, Mood, STAGE_NAMES};
use crate::paint::{Align, Font, H, Paint, W, col, fit};
use crate::patterns::{BPM_MAX, BPM_MIN, BPM_STEP, INGREDIENTS, Level, LevelKind, PATTERNS, bpm_steps, drills};
use crate::progress::{PASS_THRESHOLD, grade};
use crate::songs::{self, Song, Via};

const RAIL_W: f32 = 360.0;
/// Main area, left and right edge.
const MX: f32 = RAIL_W + 88.0;
const MR: f32 = W - 88.0;
const NODE_R: f32 = 40.0;
const LADDER_Y: f32 = 262.0;
const OPTIONS_Y: f32 = 412.0;
const TILE_H: f32 = 48.0;
const SONGS_Y: f32 = OPTIONS_Y + TILE_H + 56.0;
const SONG_ROW_H: f32 = 46.0;
const S_FILL: Color = Color::srgb(0xe9 as f32 / 255.0, 0xc8 as f32 / 255.0, 0x77 as f32 / 255.0);

fn node_x(bpm: u32) -> f32 {
    let steps = ((BPM_MAX - BPM_MIN) / BPM_STEP).max(1) as f32;
    MX + NODE_R + ((bpm - BPM_MIN) / BPM_STEP) as f32 * (MR - MX - 2.0 * NODE_R) / steps
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

pub fn draw(g: &mut Game, p: &mut Paint, keys: &ButtonInput<KeyCode>, imgs: &MascotImages) {
    if rail(g, p, imgs) {
        g.refresh_lists();
        g.screen = Screen::Settings;
        return;
    }

    let kind = g.selected.kind;
    let title = if matches!(kind, LevelKind::Drill(_)) { "Drills" } else { "Tempo ladder" };
    p.serif(MX, 118.0, title, 44.0, col::TEXT, Align::Left);

    let mut clicked = ladder(g, p, kind);

    p.hline(MX, OPTIONS_Y - 32.0, MR - MX, col::LINE);
    match kind {
        LevelKind::Run(_) | LevelKind::Riff(_) => ingredients(g, p),
        LevelKind::Drill(_) => {
            if let Some(d) = drill_options(g, p) {
                g.selected = d;
            }
        }
    }
    song_list(g, p);

    // ---------- play button, bottom right
    let sel = g.selected;
    let can_play = g.progress.is_unlocked(&sel);
    let (bw, bh) = (200.0, 76.0);
    let (bx, by) = (MR - bw, H - 64.0 - bh);
    let hov = p.hover(bx, by, bw, bh);
    p.rect(bx, by, bw, bh, if !can_play { col::FAINT } else if hov { col::DIM } else { col::TEXT });
    p.serif(bx + bw / 2.0, by + bh / 2.0 - 3.0, "Play", 36.0, col::BG, Align::Center);
    if can_play && (p.hit(bx, by, bw, bh) || keys.just_pressed(KeyCode::Enter)) {
        g.start_level(sel, false);
        return;
    }

    if let Some(l) = clicked.take() {
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

    // ---------- problems, bottom left
    let text_w = bx - 40.0 - MX;
    if let Some((msg, bad)) = g.input_problem() {
        p.text(MX, by + 14.0, fit(&msg, text_w, 13.0), 13.0, if bad { col::LATE } else { col::DIM }, Align::Left);
        if link(p, MX + 44.0, by + 42.0, "Retry") {
            g.restart_engine();
        }
    }
    if let Some(m) = &g.message {
        p.text(MX, by + 70.0, fit(m, text_w, 13.0), 13.0, col::LATE, Align::Left);
    } else if let Some((m, at)) = &g.notice
        && g.clock - at < 5.0
    {
        p.text(MX, by + 70.0, fit(m, text_w, 13.0), 13.0, col::ONTIME, Align::Left);
    }
}

/// The left rail. Returns true when Setup was clicked.
fn rail(g: &mut Game, p: &mut Paint, imgs: &MascotImages) -> bool {
    p.rect(0.0, 0.0, RAIL_W, H, col::PANEL);
    lotus(p, 58.0, 64.0, 0.85);
    p.italic(88.0, 58.0, "Guitar Guru", 38.0, col::TEXT, Align::Left);

    let bpm = g.selected.bpm;
    guru(g, p, imgs, bpm);

    // Mode switch.
    let drill_mode = matches!(g.selected.kind, LevelKind::Drill(_));
    let (nx, nw, nh) = (40.0, RAIL_W - 80.0, 48.0);
    for (k, active) in [(0, !drill_mode), (1, drill_mode)] {
        let y = 520.0 + k as f32 * (nh + 4.0);
        let hov = p.hover(nx, y, nw, nh);
        if active {
            p.rect(nx, y, nw, nh, col::TEXT);
        } else if hov {
            p.rect(nx, y, nw, nh, col::PANEL2);
        }
        let c = if active { col::BG } else { col::TEXT };
        let (ix, cy) = (nx + 26.0, y + nh / 2.0);
        if k == 0 {
            for (i, h) in [6.0f32, 10.0, 14.0, 18.0].into_iter().enumerate() {
                p.rect(ix - 9.0 + i as f32 * 5.5, cy + 9.0 - h, 2.4, h, c);
            }
        } else {
            p.ring(ix, cy, 9.0, c);
            p.ring(ix, cy, 8.4, c);
            p.disc(ix, cy, 3.5, c);
        }
        p.styled(nx + 50.0, cy - 1.0, if k == 0 { "Tempo ladder" } else { "Drills" }, 16.0, c, Align::Left, Font::SansBold, 0.0);
        if p.hit(nx, y, nw, nh) && !active {
            g.selected = if k == 0 { Level::run(g.mix, bpm) } else { first_drill(g, bpm) };
        }
    }

    // Setup, at the bottom.
    let (sx, sy) = (40.0, H - 56.0);
    let hov = p.hover(sx, sy - 22.0, 120.0, 44.0);
    let c = if hov { col::OCHRE } else { col::TEXT };
    gear(p, sx + 9.0, sy, c);
    p.styled(sx + 28.0, sy - 1.0, "Setup", 15.0, c, Align::Left, Font::SansBold, 0.0);
    p.clicked && hov
}

/// The drill to show when switching to drills: the first one open at this tempo.
fn first_drill(g: &Game, bpm: u32) -> Level {
    let all = drills();
    let open = all.iter().map(|&pi| Level::drill(pi, bpm)).find(|d| g.progress.is_unlocked(d));
    open.unwrap_or_else(|| Level::drill(all.first().copied().unwrap_or(0), bpm))
}

/// The tempo ladder for levels of `kind`. Returns the clicked (open) level.
fn ladder(g: &mut Game, p: &mut Paint, kind: LevelKind) -> Option<Level> {
    let bpms: Vec<u32> = bpm_steps().collect();
    let (r, cy) = (NODE_R, LADDER_Y);
    let at = |b: u32| Level { kind, bpm: b };
    for &b in &bpms {
        if b + BPM_STEP > BPM_MAX {
            continue;
        }
        let (l, rr) = (at(b), at(b + BPM_STEP));
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
    let mut clicked = None;
    for &b in &bpms {
        let l = at(b);
        let x = node_x(b);
        let open = g.progress.is_unlocked(&l);
        let passed = g.progress.passed(&l);
        let best = g.progress.best(&l);
        let hov = p.hover_circle(x, cy, r + 3.0);
        if g.selected == l {
            p.disc(x, cy, r, col::TEXT);
            p.serif(x, cy - 2.0, format!("{b}"), 30.0, col::BG, Align::Center);
        } else if passed {
            outlined(p, x, cy, r, col::ONTIME, if hov { col::PANEL } else { col::BG }, 1.5);
            p.serif(x, cy - 2.0, format!("{b}"), 30.0, col::ONTIME, Align::Center);
        } else if open {
            outlined(p, x, cy, r, col::TEXT, if hov { col::PANEL } else { col::BG }, 1.5);
            p.serif(x, cy - 2.0, format!("{b}"), 30.0, col::TEXT, Align::Center);
        } else {
            p.dashed_ring(x, cy, r, col::LINE);
            p.serif(x, cy - 2.0, format!("{b}"), 30.0, col::FAINT, Align::Center);
        }
        if let Some(s) = best {
            grade_badge(p, x + r * 0.74, cy - r * 0.74, grade(s), 14.0);
        }
        if g.progress.fresh.contains(&format!("tempo:{b}")) {
            p.disc(x - r * 0.74, cy - r * 0.74, 4.5, col::OCHRE);
        }
        let below = if !open {
            "locked".to_string()
        } else if let Some(s) = best {
            format!("{s:.0}%")
        } else {
            "open".into()
        };
        p.text(x, cy + r + 22.0, below, 14.0, col::DIM, Align::Center);
        if hov && p.clicked && open {
            clicked = Some(l);
        }
    }
    clicked
}

fn grade_badge(p: &mut Paint, cx: f32, cy: f32, g: char, r: f32) {
    let (fill, text, edge) = match g {
        'S' => (S_FILL, col::TEXT, col::GOLD),
        'A' => (col::ONTIME, Color::WHITE, col::ONTIME),
        'B' => (col::TEAL_TINT, col::ONTIME, col::ONTIME),
        _ => (col::PANEL2, col::DIM, col::FAINT),
    };
    p.disc(cx, cy, r + 3.0, col::BG);
    outlined(p, cx, cy, r, edge, fill, 1.5);
    p.styled(cx, cy, g.to_string(), r, text, Align::Center, Font::SansBold, 0.0);
}

/// A filled disc with an edge, both in the shape layer so later shapes cover it (a
/// `ring` is drawn above every shape).
fn outlined(p: &mut Paint, cx: f32, cy: f32, r: f32, edge: Color, fill: Color, w: f32) {
    p.disc(cx, cy, r, edge);
    p.disc(cx, cy, r - w, fill);
}

/// Look of an option tile.
#[derive(Clone, Copy, PartialEq)]
enum Tile {
    On,
    Off,
    /// Locked, but an unlock token can open it.
    Unlockable,
    Locked,
}

/// Width of a tile with this label (and a lock icon when locked).
fn tile_w(label: &str, t: Tile) -> f32 {
    let lock = if matches!(t, Tile::Unlockable | Tile::Locked) { 24.0 } else { 0.0 };
    14.0 + 22.0 + 12.0 + label.len() as f32 * 8.0 + lock + 18.0
}

/// An option tile with a checkbox (`radio` = false) or a radio dot. Returns true when
/// clicked (any state but plain locked).
fn tile(p: &mut Paint, x: f32, y: f32, label: &str, t: Tile, radio: bool) -> bool {
    let (w, h) = (tile_w(label, t), TILE_H);
    let hov = p.hover(x, y, w, h);
    let (bx, cy) = (x + 14.0, y + h / 2.0);
    let label_c = match t {
        Tile::On => {
            p.rect(x, y, w, h, col::TEAL_TINT);
            p.frame(x, y, w, h, 1.5, col::ONTIME);
            col::ONTIME
        }
        Tile::Off => {
            if hov {
                p.rect(x, y, w, h, col::PANEL);
            }
            p.frame(x, y, w, h, 1.5, col::TEXT);
            col::TEXT
        }
        Tile::Unlockable => {
            if hov {
                p.rect(x, y, w, h, col::OCHRE_TINT);
            }
            p.dashed_frame(x, y, w, h, col::OCHRE);
            col::DIM
        }
        Tile::Locked => {
            p.dashed_frame(x, y, w, h, col::LINE);
            col::FAINT
        }
    };
    // The box (or radio) itself.
    if radio {
        let rc = bx + 11.0;
        match t {
            Tile::On => {
                p.disc(rc, cy, 10.0, col::ONTIME);
                p.disc(rc, cy, 4.0, Color::WHITE);
            }
            Tile::Off => {
                p.disc(rc, cy, 10.0, col::TEXT);
                p.disc(rc, cy, 8.5, col::BG);
            }
            Tile::Unlockable => p.dashed_ring(rc, cy, 10.0, col::OCHRE),
            Tile::Locked => p.dashed_ring(rc, cy, 10.0, col::LINE),
        }
    } else {
        let (s, by) = (22.0, cy - 11.0);
        match t {
            Tile::On => {
                p.rect(bx, by, s, s, col::ONTIME);
                tick(p, bx, by, Color::WHITE);
            }
            Tile::Off => {
                p.rect(bx, by, s, s, col::BG);
                p.frame(bx, by, s, s, 1.5, col::TEXT);
            }
            Tile::Unlockable => p.dashed_frame(bx, by, s, s, col::OCHRE),
            Tile::Locked => p.dashed_frame(bx, by, s, s, col::LINE),
        }
    }
    let font = if t == Tile::On { Font::SansBold } else { Font::SansMedium };
    p.styled(bx + 34.0, cy - 1.0, label, 16.0, label_c, Align::Left, font, 0.0);
    if matches!(t, Tile::Unlockable | Tile::Locked) {
        let c = if t == Tile::Unlockable { col::OCHRE } else { col::FAINT };
        lock(p, x + w - 26.0, cy, c);
    }
    t != Tile::Locked && p.clicked && hov
}

/// Ingredient checkboxes (runs).
fn ingredients(g: &mut Game, p: &mut Paint) {
    let mut x = MX;
    let mut tip = false;
    for (i, ing) in INGREDIENTS.iter().enumerate() {
        let t = if g.progress.ingredient_unlocked(i) {
            if g.mix & ing.bit != 0 { Tile::On } else { Tile::Off }
        } else if g.progress.unlock_tokens() > 0 {
            Tile::Unlockable
        } else {
            Tile::Locked
        };
        let w = tile_w(ing.short, t);
        if x + w > MR {
            break;
        }
        let fresh_key = format!("ingredient:{}", ing.code);
        if tile(p, x, OPTIONS_Y, ing.short, t, false) {
            match t {
                Tile::On | Tile::Off => {
                    g.mix ^= ing.bit;
                    g.progress.fresh.remove(&fresh_key);
                }
                Tile::Unlockable => {
                    if g.progress.unlock_ingredient(i) {
                        g.progress.save();
                        // Straight into the mix, at the selected tempo.
                        g.mix |= ing.bit;
                    }
                }
                Tile::Locked => {}
            }
            g.selected = Level::run(g.mix, g.selected.bpm);
        }
        if g.progress.fresh.contains(&fresh_key) {
            p.disc(x + w - 6.0, OPTIONS_Y + 6.0, 4.0, col::OCHRE);
        }
        if t == Tile::Locked && p.hover(x, OPTIONS_Y, w, TILE_H) {
            tip = true;
        }
        x += w + 12.0;
    }
    // The next unlock comes from the first pass with an unlocked ingredient not passed
    // yet, or from any first pass (`Progress::unlock_tokens`).
    if tip {
        let todo = g.progress.unlocked_mask() & !g.progress.passed_mask();
        let with = if todo == 0 { "any run".into() } else { mask_names_with(todo, " or ") };
        p.tooltip("locked ingredient", format!("{with} \u{2265} {PASS_THRESHOLD:.0}%"));
    }
}

/// "Sync + P.M." for a mask.
fn mask_names(m: u8) -> String {
    mask_names_with(m, " + ")
}

fn mask_names_with(m: u8, sep: &str) -> String {
    INGREDIENTS.iter().filter(|i| m & i.bit != 0).map(|i| i.short).collect::<Vec<_>>().join(sep)
}

/// Drill pattern choice (drills). Returns the newly picked drill.
fn drill_options(g: &Game, p: &mut Paint) -> Option<Level> {
    let bpm = g.selected.bpm;
    let mut x = MX;
    let mut picked = None;
    for pi in drills() {
        let d = Level::drill(pi, bpm);
        let name = PATTERNS[pi].name.as_ref();
        let t = if g.selected == d {
            Tile::On
        } else if g.progress.is_unlocked(&d) {
            Tile::Off
        } else {
            Tile::Locked
        };
        let w = tile_w(name, t);
        if x + w > MR {
            break;
        }
        if tile(p, x, OPTIONS_Y, name, t, true) && t == Tile::Off {
            picked = Some(d);
        }
        x += w + 12.0;
    }
    picked
}

/// The songs of the selected tempo in two columns: revealed ones by name (a dot until
/// hovered once), hidden ones as a question mark that shows its requirements on hover.
fn song_list(g: &mut Game, p: &mut Paint) {
    let list: Vec<&Song> = songs::at(g.selected.bpm).collect();
    if list.is_empty() {
        return;
    }
    p.hline(MX, SONGS_Y - 28.0, MR - MX, col::LINE);
    let col_w = (MR - MX - 40.0) / 2.0;
    let rows = list.len().div_ceil(2);
    let mut practise = None;
    for (i, s) in list.into_iter().enumerate() {
        let (x, y) = (MX + (i / rows) as f32 * (col_w + 40.0), SONGS_Y + (i % rows) as f32 * SONG_ROW_H);
        let cy = y + SONG_ROW_H / 2.0;
        let (mx, tx) = (x + 14.0, x + 40.0);
        let hov = p.hover(x, y, col_w, SONG_ROW_H);
        let revealed = s.revealed(&g.progress);
        let req = requirement(s);
        if revealed && let Some(r) = s.riff {
            // A pack riff: click to practise it.
            let c = if hov { col::OCHRE } else { col::DIM };
            p.ring(mx, cy, 14.0, if hov { col::OCHRE } else { col::FAINT });
            p.tri(mx + 1.5, cy, 6.5, true, c);
            if hov && p.clicked {
                practise = crate::packs::riffs().get(r).map(|rf| Level { kind: LevelKind::Riff(r), bpm: rf.bpm });
            }
        } else if revealed {
            p.ring(mx, cy, 14.0, col::FAINT);
            note(p, mx, cy, col::DIM);
        }
        if revealed {
            let has_part = !s.part.is_empty();
            let ty = if has_part { cy - 9.0 } else { cy - 1.0 };
            let tw = col_w - 50.0;
            p.styled(tx, ty, fit(&s.label(), tw, 16.0), 16.0, col::TEXT, Align::Left, Font::SansMedium, 0.0);
            if has_part {
                p.text(tx, cy + 11.0, fit(s.part, tw, 13.0), 13.0, col::DIM, Align::Left);
            }
            let key = s.key();
            if !g.progress.songs_shown.contains(&key) {
                if hov {
                    g.progress.songs_shown.insert(key);
                    g.progress.save();
                } else {
                    p.disc(mx + 11.0, cy - 11.0, 4.0, col::OCHRE);
                }
            }
        } else {
            p.ring(mx, cy, 14.0, if hov { col::OCHRE } else { col::FAINT });
            p.serif(mx, cy - 2.0, "?", 20.0, if hov { col::OCHRE } else { col::DIM }, Align::Center);
        }
        if !revealed {
            p.text(tx, cy - 1.0, fit(&req, col_w - 50.0, 14.0), 14.0, col::FAINT, Align::Left);
        } else if hov {
            p.tooltip(s.key(), req);
        }
    }
    if let Some(l) = practise {
        g.start_level(l, false);
    }
}

/// What reveals a song: "P.M. + 16ths ≥ 75%", "any mix ≥ 60%" or "Gallop drill ≥ 90%".
fn requirement(s: &Song) -> String {
    let what = match s.via {
        Via::Run(0) => "any mix".into(),
        Via::Run(m) => mask_names(m),
        Via::Drill(id) => PATTERNS.iter().find(|q| q.id == id).map_or_else(|| id.to_string(), |q| format!("{} drill", q.name)),
    };
    format!("{what} \u{2265} {:.0}%", s.min)
}

/// A tick inside a 22 px box at (x, y).
fn tick(p: &mut Paint, x: f32, y: f32, c: Color) {
    for o in [-0.7f32, 0.0, 0.7] {
        p.line(x + 6.0, y + 11.5 + o, x + 9.5, y + 15.0 + o, c);
        p.line(x + 9.5, y + 15.0 + o, x + 16.0, y + 7.5 + o, c);
    }
}

/// An eighth note centred at (cx, cy).
fn note(p: &mut Paint, cx: f32, cy: f32, c: Color) {
    p.ellipse(cx - 2.5, cy + 5.0, 9.0, 6.5, 0.45, c);
    p.rect(cx + 1.2, cy - 8.0, 1.6, 13.0, c);
    p.rot_rect(cx + 4.6, cy - 5.5, 1.6, 7.0, 0.6, c);
}

/// A small padlock centred at (cx, cy).
fn lock(p: &mut Paint, cx: f32, cy: f32, c: Color) {
    p.rect(cx - 5.5, cy - 1.0, 11.0, 8.0, c);
    let (r, top) = (3.4f32, cy - 4.5);
    for o in [0.0f32, 0.6] {
        p.line(cx - r - o, cy - 1.0, cx - r - o, top, c);
        p.line(cx + r + o, cy - 1.0, cx + r + o, top, c);
        let n = 8;
        for i in 0..n {
            let (a0, a1) = (std::f32::consts::PI * i as f32 / n as f32, std::f32::consts::PI * (i + 1) as f32 / n as f32);
            let rr = r + o;
            p.line(cx - rr * a0.cos(), top - rr * a0.sin(), cx - rr * a1.cos(), top - rr * a1.sin(), c);
        }
    }
}

/// A small gear centred at (cx, cy).
fn gear(p: &mut Paint, cx: f32, cy: f32, c: Color) {
    p.ring(cx, cy, 5.5, c);
    p.ring(cx, cy, 5.0, c);
    p.ring(cx, cy, 2.2, c);
    for k in 0..8 {
        let a = std::f32::consts::TAU * k as f32 / 8.0;
        p.rot_rect(cx + 7.0 * a.cos(), cy + 7.0 * a.sin(), 3.2, 2.6, -a, c);
    }
}

/// The guru at the selected tempo, his stage from XP, with the XP bar under him. After a
/// run the bar fills from the XP last shown to the current total; crossing a stage
/// threshold cross-fades him into the next stage inside a widening gold ring.
fn guru(g: &mut Game, p: &mut Paint, imgs: &MascotImages, bpm: u32) {
    let gs = 1.0;
    let (gx, gy) = ((RAIL_W - 240.0 * gs) / 2.0, 130.0);
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
                    let r = 100.0 + 70.0 * v;
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
    let (bx, by, bw) = (54.0, 450.0, RAIL_W - 108.0);
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
