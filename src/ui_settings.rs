//! Setup ("Guitar Guru" style): hairline lists with radio dots, a thin level meter, round
//! -/+ buttons. Input device and channel, output, the guitar sound (fixed presets drawn
//! as guitars, or mute for your own amp), click, latency calibration.

use std::sync::atomic::Ordering;

use bevy::color::{Alpha, Color};
use bevy::math::Vec2;

use crate::game::{Game, Screen, calibration_level};
use crate::mascot::MascotImages;
use crate::monitor::SoundPreset;
use crate::paint::{Align, Font, Paint, col, fit};
use crate::ui_menu::{header, link};

const LX: f32 = 170.0;
const RX: f32 = 748.0;
const CW: f32 = 522.0;
const ROW: f32 = 40.0;

/// Round -/+ stepper; returns -1, 0 or +1.
fn stepper(p: &mut Paint, x: f32, y: f32, label: &str, value: &str) -> i32 {
    p.label(x, y, label);
    let bx = x + 6.0 + label.len() as f32 * 8.5;
    let mut d = 0;
    if p.round_button(bx + 20.0, y, 18.0, "-") {
        d = -1;
    }
    p.serif(bx + 76.0, y - 2.0, value, 24.0, col::TEXT, Align::Center);
    if p.round_button(bx + 132.0, y, 18.0, "+") {
        d = 1;
    }
    d
}

/// A scrollable radio list. Returns the clicked index.
fn radio_list(p: &mut Paint, scroll: &mut usize, x: f32, y: f32, rows: &[(String, String, bool)], visible: usize) -> Option<usize> {
    let max = rows.len().saturating_sub(visible);
    if p.hover(x, y, CW, ROW * visible as f32) && p.scroll != 0.0 {
        *scroll = if p.scroll < 0.0 { (*scroll + 1).min(max) } else { scroll.saturating_sub(1) };
    }
    *scroll = (*scroll).min(max);
    let mut hit = None;
    for (k, (label, note, sel)) in rows.iter().enumerate().skip(*scroll).take(visible) {
        let ry = y + (k - *scroll) as f32 * ROW;
        if p.radio_row(x, ry, CW, ROW, label, note, *sel) {
            hit = Some(k);
        }
    }
    if rows.len() > visible {
        p.text(x + CW, y - 12.0, format!("{}\u{2013}{} of {} \u{b7} scroll", *scroll + 1, (*scroll + visible).min(rows.len()), rows.len()), 11.0, col::FAINT, Align::Right);
    }
    hit
}

/// The guitar sound: hand-drawn guitars (and an amp for mute) in a row from `x0`, tops
/// at `y`, a one-word label under each. The chosen one is in full colour with a
/// gold diamond under its label; returns a new choice.
fn sound_choice(p: &mut Paint, imgs: &MascotImages, x0: f32, y: f32, cur: SoundPreset) -> Option<SoundPreset> {
    let (step, w, h) = (88.0, 26.0, 65.0);
    let mut pick = None;
    for (k, s) in SoundPreset::ALL.into_iter().enumerate() {
        let x = x0 + step / 2.0 + k as f32 * step;
        let on = s == cur;
        let hov = p.hover(x - step / 2.0 + 4.0, y - 4.0, step - 8.0, h + 30.0);
        let alpha = if on { 1.0 } else if hov { 0.85 } else { 0.5 };
        if let Some(img) = imgs.sounds.get(k) {
            p.image(img, Vec2::new(x, y + h / 2.0), Vec2::new(w, h), Vec2::ZERO, 0.0, Color::WHITE.with_alpha(alpha));
        }
        let c = if on { col::TEXT } else if hov { col::OCHRE } else { col::DIM };
        p.styled(x, y + h + 14.0, s.name(), 13.0, c, Align::Center, if on { Font::SansMedium } else { Font::Sans }, 0.0);
        if on {
            p.rot_rect(x, y + h + 30.0, 4.5, 4.5, std::f32::consts::FRAC_PI_4, col::GOLD);
        }
        if hov && p.clicked && !on {
            pick = Some(s);
        }
    }
    pick
}

fn radio_inline(p: &mut Paint, x: f32, y: f32, label: &str, on: bool, size: f32) -> bool {
    if on {
        p.disc(x + 8.0, y, 6.0, col::OCHRE);
    } else {
        p.ring(x + 8.0, y, 6.0, col::DIM);
    }
    p.text(x + 22.0, y - 1.0, label, size, col::TEXT, Align::Left);
    p.hit(x, y - 14.0, 26.0 + label.len() as f32 * size * 0.5, 28.0)
}

pub fn draw(g: &mut Game, p: &mut Paint, imgs: &MascotImages) {
    header(p, "Setup", false);
    if link(p, 1270.0, 66.0, "Main menu") {
        g.screen = Screen::Menu;
        return;
    }
    if link(p, 1120.0, 66.0, "Rescan devices") {
        g.refresh_lists();
    }
    let mut restart = false;
    let mut rebuild = false;

    // ---------- your guitar
    p.serif(LX, 139.0, "Your guitar", 28.0, col::TEXT, Align::Left);
    let mut rows = vec![("Auto".to_string(), "shared, else direct".to_string(), g.cfg.audio.input_device.is_none())];
    let inputs = g.lists.inputs.clone();
    for d in &inputs {
        let (kind, name) = d.label.split_once("  ").unwrap_or(("", d.label.as_str()));
        rows.push((name.to_string(), kind.to_string(), g.cfg.audio.input_device.as_deref() == Some(d.id.as_str())));
    }
    let mut sc = g.lists.input_scroll;
    if let Some(k) = radio_list(p, &mut sc, LX, 160.0, &rows, 5) {
        g.cfg.audio.input_device = if k == 0 { None } else { Some(inputs[k - 1].id.clone()) };
        restart = true;
    }
    g.lists.input_scroll = sc;
    let y = 160.0 + 5.0 * ROW + 26.0;
    for ch in 0..2u16 {
        let sel = g.cfg.audio.input_channel == ch;
        if radio_inline(p, LX + ch as f32 * 90.0, y, &format!("In {}", ch + 1), sel, 15.0) && !sel {
            g.cfg.audio.input_channel = ch;
            restart = true;
        }
    }
    if radio_inline(p, LX + 240.0, y, "show all ALSA devices", g.lists.show_all, 13.0) {
        g.lists.show_all = !g.lists.show_all;
        g.lists.input_scroll = 0;
        g.refresh_lists();
    }
    // thin meter -48..0 dBFS with ticks at -18 and -6
    let my = y + 44.0;
    let pos = |d: f32| LX + CW * ((d + 48.0) / 48.0).clamp(0.0, 1.0);
    p.rect(LX, my - 2.0, CW, 4.0, col::PANEL2);
    let mc = if g.meter_db > -1.0 { col::LATE } else { col::ONTIME };
    p.rect(LX, my - 2.0, pos(g.meter_db) - LX, 4.0, mc);
    p.rect(pos(g.meter_peak_db) - 0.75, my - 6.0, 1.5, 12.0, col::TEXT);
    for (d, t) in [(-48.0, "\u{2212}48"), (-18.0, "\u{2212}18"), (-6.0, "\u{2212}6"), (0.0, "0 dBFS")] {
        if d > -48.0 && d < 0.0 {
            p.rect(pos(d) - 0.5, my - 8.0, 1.0, 16.0, col::TEXT);
        }
        let al = if d == 0.0 {
            Align::Right
        } else if d == -48.0 {
            Align::Left
        } else {
            Align::Center
        };
        p.text(pos(d), my + 20.0, t, 11.0, col::DIM, al);
    }
    let d = stepper(p, LX, my + 64.0, "gain", &format!("{:+.0}", g.cfg.tone.input_gain_db));
    if d != 0 {
        g.cfg.tone.input_gain_db = (g.cfg.tone.input_gain_db + d as f32).clamp(-24.0, 24.0);
        rebuild = true;
    }
    p.label(LX + 230.0, my + 64.0, "db");
    if let Some(e) = &g.engine {
        p.text(LX, my + 100.0, fit(&format!("open: {}", e.info.input_label), CW, 12.0), 12.0, col::FAINT, Align::Left);
        p.text(
            LX,
            my + 118.0,
            format!(
                "{} Hz \u{b7} in {} \u{b7} out {} \u{b7} dsp {:.0}% \u{b7} drops {}",
                e.info.sample_rate,
                e.info.input_buffer,
                e.info.output_buffer,
                e.shared.dsp_load.load() * 100.0,
                e.shared.xruns.load(Ordering::Relaxed)
            ),
            12.0,
            col::FAINT,
            Align::Left,
        );
    }
    if let Some((msg, bad)) = g.input_problem() {
        p.text(LX, my + 138.0, fit(&msg, CW - 70.0, 12.0), 12.0, if bad { col::LATE } else { col::DIM }, Align::Left);
        if link(p, LX + CW, my + 138.0, "Retry") {
            restart = true;
        }
    }

    // ---------- output
    let oy = 680.0;
    p.serif(LX, oy, "Output", 28.0, col::TEXT, Align::Left);
    let mut orows = vec![("System default".to_string(), String::new(), g.cfg.audio.output_device.is_none())];
    let outs = g.lists.outputs.clone();
    for d in &outs {
        let (kind, name) = d.label.split_once("  ").unwrap_or(("", d.label.as_str()));
        orows.push((name.to_string(), kind.to_string(), g.cfg.audio.output_device.as_deref() == Some(d.id.as_str())));
    }
    let mut osc = g.lists.output_scroll;
    if let Some(k) = radio_list(p, &mut osc, LX, oy + 20.0, &orows, 4) {
        g.cfg.audio.output_device = if k == 0 { None } else { Some(outs[k - 1].id.clone()) };
        restart = true;
    }
    g.lists.output_scroll = osc;

    // ---------- sound: fixed presets (or mute), then gate and volume
    p.serif(RX, 139.0, "Sound", 28.0, col::TEXT, Align::Left);
    if let Some(preset) = sound_choice(p, imgs, RX - 22.0, 164.0, g.cfg.tone.preset) {
        g.set_preset(preset);
    }
    let ty = 310.0;
    let d = stepper(p, RX, ty, "gate", &format!("{:.0}", g.cfg.tone.gate_db));
    if d != 0 {
        g.cfg.tone.gate_db = (g.cfg.tone.gate_db + 5.0 * d as f32).clamp(-90.0, -20.0);
        rebuild = true;
    }
    let d = stepper(p, RX + 270.0, ty, "volume", &format!("{:+.0}", g.cfg.tone.output_db));
    if d != 0 {
        g.cfg.tone.output_db = (g.cfg.tone.output_db + 2.0 * d as f32).clamp(-40.0, 12.0);
        rebuild = true;
    }
    p.text(RX, ty + 38.0, fit(&g.chain_description(), CW, 12.0), 12.0, col::FAINT, Align::Left);

    // ---------- click
    let ky = 680.0;
    p.serif(RX, ky, "Click", 28.0, col::TEXT, Align::Left);
    let d = stepper(p, RX, ky + 40.0, "volume", &format!("{:.0}", g.cfg.audio.click_volume_db));
    if d != 0 {
        g.cfg.audio.click_volume_db = (g.cfg.audio.click_volume_db + 2.0 * d as f32).clamp(-40.0, 0.0);
        if let Some(e) = &g.engine {
            e.shared.click_gain.store(10f32.powf(g.cfg.audio.click_volume_db / 20.0));
        }
        g.cfg.save();
    }
    let d = stepper(p, RX + 270.0, ky + 40.0, "buffer", &format!("{}", g.cfg.audio.buffer_frames));
    if d != 0 {
        let b = g.cfg.audio.buffer_frames;
        g.cfg.audio.buffer_frames = if d > 0 { (b * 2).min(1024) } else { (b / 2).max(32) };
        restart = true;
    }

    // ---------- latency
    let ly = 800.0;
    p.serif(RX, ly, "Latency", 28.0, col::TEXT, Align::Left);
    let txt = g.cfg.latency_ms.map_or("\u{2013}".to_string(), |l| format!("{l:+.0} ms"));
    p.serif(RX + 130.0, ly, txt, 30.0, col::TEXT, Align::Left);
    if p.button(RX + 260.0, ly - 22.0, 130.0, 44.0, "Calibrate", true) {
        g.start_level(calibration_level(), true);
        return;
    }
    if g.cfg.latency_ms.is_some() && link(p, RX + CW, ly, "Reset") {
        g.cfg.latency_ms = None;
        g.cfg.save();
    }

    if rebuild {
        g.rebuild_chain();
    }
    if restart {
        g.cfg.save();
        g.restart_engine();
    }
}
