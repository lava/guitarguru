//! Settings: input/output device, channel, level meter, amp + cab, click, latency.

use std::sync::atomic::Ordering;

use crate::game::{CALIBRATION_BPM, Game, Screen, calibration_pattern};
use crate::monitor::AmpChoice;
use crate::paint::{Align, Paint, col};
use crate::ui_common::*;
use crate::ui_menu::truncate;

pub fn draw(g: &mut Game, p: &mut Paint) {
    p.text(32.0, 42.0, "SETTINGS", 32.0, col::BRASS, Align::Left);
    if p.button(1292.0, 22.0, 116.0, 40.0, "Back", false) {
        g.screen = Screen::Menu;
        return;
    }
    if p.button(1150.0, 22.0, 130.0, 40.0, "Rescan", false) {
        g.refresh_lists();
    }

    // --- Input column
    let (x, w) = (32.0, 440.0);
    p.panel(x, 84.0, w, 790.0);
    p.label(x + 16.0, 104.0, "input device");
    let mut restart = false;
    let mut y = 120.0;
    if p.choice(x + 16.0, y, w - 32.0, 30.0, "Auto (first BANDFUSE, else default)", g.cfg.audio.input_device.is_none()) {
        g.cfg.audio.input_device = None;
        restart = true;
    }
    y += 34.0;
    // Show BANDFUSE devices first, then the rest; scroll with the wheel.
    let mut inputs = g.lists.inputs.clone();
    inputs.sort_by_key(|d| !d.label.contains("BANDFUSE"));
    let visible = 11;
    if p.hover(x, y, w, visible as f32 * 34.0) && p.scroll != 0.0 {
        let max = inputs.len().saturating_sub(visible);
        g.lists.input_scroll = if p.scroll < 0.0 { (g.lists.input_scroll + 1).min(max) } else { g.lists.input_scroll.saturating_sub(1) };
    }
    for d in inputs.iter().skip(g.lists.input_scroll).take(visible) {
        let sel = g.cfg.audio.input_device.as_deref() == Some(d.id.as_str());
        if p.choice(x + 16.0, y, w - 32.0, 30.0, &truncate(&d.label, 52), sel) {
            g.cfg.audio.input_device = Some(d.id.clone());
            restart = true;
        }
        y += 34.0;
    }
    if inputs.len() > visible {
        p.text(x + w - 20.0, 104.0, format!("scroll {}/{}", g.lists.input_scroll + 1, inputs.len() - visible + 1), 11.0, col::FAINT, Align::Right);
    }
    y = 556.0;
    p.label(x + 16.0, y, "input channel");
    for ch in 0..2u16 {
        if p.choice(x + 16.0 + ch as f32 * 90.0, y + 14.0, 80.0, 30.0, &format!("In {}", ch + 1), g.cfg.audio.input_channel == ch) {
            g.cfg.audio.input_channel = ch;
            restart = true;
        }
    }
    p.label(x + 16.0, y + 70.0, "level  (target -18..-6 dBFS)");
    level_meter(p, x + 16.0, y + 84.0, w - 32.0, 18.0, g.meter_db, g.meter_peak_db);
    if let Some(e) = &g.engine {
        p.text(x + 16.0, y + 132.0, format!("open: {}", truncate(&e.info.input_label, 50)), 12.0, col::DIM, Align::Left);
        p.text(x + 16.0, y + 150.0, format!("out: {}", truncate(&e.info.output_label, 50)), 12.0, col::DIM, Align::Left);
        p.text(
            x + 16.0,
            y + 168.0,
            format!(
                "{} Hz  in {}  out {}  dsp {:.0} %  drops {}",
                e.info.sample_rate,
                e.info.input_buffer,
                e.info.output_buffer,
                e.shared.dsp_load.load() * 100.0,
                e.shared.xruns.load(Ordering::Relaxed)
            ),
            12.0,
            col::DIM,
            Align::Left,
        );
    }
    if let Some(w) = g.engine.as_ref().and_then(|e| e.info.warning.clone()) {
        p.text(x + 16.0, y + 214.0, truncate(&w, 60), 12.0, col::LATE, Align::Left);
    }
    if let Some(e) = &g.engine_err {
        p.text(x + 16.0, y + 196.0, truncate(e, 60), 12.0, col::MISS, Align::Left);
    }

    // --- Tone column
    let (x, w) = (492.0, 440.0);
    p.panel(x, 84.0, w, 790.0);
    p.label(x + 16.0, 104.0, "amp");
    let mut rebuild = false;
    let mut amps = vec![("Clean DI".to_string(), AmpChoice::Clean), ("Built-in drive".to_string(), AmpChoice::Drive)];
    for n in &g.lists.nams {
        amps.push((n.trim_end_matches(".nam").replace("Lum-", "NAM "), AmpChoice::Nam(n.clone())));
    }
    let mut y = 120.0;
    for (i, (label, choice)) in amps.iter().enumerate() {
        let cx = x + 16.0 + (i % 2) as f32 * 206.0;
        if p.choice(cx, y, 200.0, 40.0, label, g.cfg.tone.amp == *choice) {
            g.cfg.tone.amp = choice.clone();
            rebuild = true;
        }
        if i % 2 == 1 {
            y += 46.0;
        }
    }
    if amps.len() % 2 == 1 {
        y += 46.0;
    }
    p.text(x + 16.0, y + 8.0, truncate(&g.chain_description(), 60), 12.0, col::DIM, Align::Left);
    y += 26.0;
    p.label(x + 16.0, y, "cab ir");
    y += 14.0;
    if p.choice(x + 16.0, y, 200.0, 26.0, "None", g.cfg.tone.cab.is_none()) {
        g.cfg.tone.cab = None;
        rebuild = true;
    }
    let irs = g.lists.irs.clone();
    for (i, ir) in irs.iter().enumerate() {
        let k = i + 1;
        let cx = x + 16.0 + (k % 2) as f32 * 206.0;
        let cy = y + (k / 2) as f32 * 30.0;
        let label = ir.trim_end_matches(".wav").replace('_', " ");
        if p.choice(cx, cy, 200.0, 26.0, &truncate(&label, 22), g.cfg.tone.cab.as_deref() == Some(ir.as_str())) {
            g.cfg.tone.cab = Some(ir.clone());
            rebuild = true;
        }
    }
    let y = y + (irs.len().div_ceil(2) as f32 + 0.5) * 30.0;
    let d = stepper(p, x + 16.0, y, "input gain", &format!("{:+.0} dB", g.cfg.tone.input_gain_db));
    if d != 0 {
        g.cfg.tone.input_gain_db = (g.cfg.tone.input_gain_db + d as f32).clamp(-24.0, 24.0);
        rebuild = true;
    }
    let d = stepper(p, x + 220.0, y, "noise gate", &format!("{:.0} dB", g.cfg.tone.gate_db));
    if d != 0 {
        g.cfg.tone.gate_db = (g.cfg.tone.gate_db + 5.0 * d as f32).clamp(-90.0, -20.0);
        rebuild = true;
    }
    let d = stepper(p, x + 16.0, y + 70.0, "amp volume", &format!("{:+.0} dB", g.cfg.tone.output_db));
    if d != 0 {
        g.cfg.tone.output_db = (g.cfg.tone.output_db + 2.0 * d as f32).clamp(-40.0, 12.0);
        rebuild = true;
    }
    if p.choice(x + 220.0, y + 98.0, 180.0, 32.0, if g.cfg.audio.monitor { "Monitoring ON" } else { "Monitoring OFF" }, g.cfg.audio.monitor) {
        g.cfg.audio.monitor = !g.cfg.audio.monitor;
        if let Some(e) = &g.engine {
            e.shared.monitor_on.store(g.cfg.audio.monitor, Ordering::Relaxed);
        }
        g.cfg.save();
    }

    // --- Output / click / latency column
    let (x, w) = (952.0, 456.0);
    p.panel(x, 84.0, w, 790.0);
    p.label(x + 16.0, 104.0, "output device");
    let mut y = 120.0;
    if p.choice(x + 16.0, y, w - 32.0, 30.0, "System default", g.cfg.audio.output_device.is_none()) {
        g.cfg.audio.output_device = None;
        restart = true;
    }
    y += 34.0;
    let outs = g.lists.outputs.clone();
    for d in outs.iter().filter(|d| !d.id.contains("null") && !d.label.contains("Rate Converter")).take(8) {
        let sel = g.cfg.audio.output_device.as_deref() == Some(d.id.as_str());
        if p.choice(x + 16.0, y, w - 32.0, 30.0, &truncate(&d.label, 54), sel) {
            g.cfg.audio.output_device = Some(d.id.clone());
            restart = true;
        }
        y += 34.0;
    }
    let y = 450.0;
    let d = stepper(p, x + 16.0, y, "click volume", &format!("{:.0} dB", g.cfg.audio.click_volume_db));
    if d != 0 {
        g.cfg.audio.click_volume_db = (g.cfg.audio.click_volume_db + 2.0 * d as f32).clamp(-40.0, 0.0);
        if let Some(e) = &g.engine {
            e.shared.click_gain.store(10f32.powf(g.cfg.audio.click_volume_db / 20.0));
        }
        g.cfg.save();
    }
    let d = stepper(p, x + 230.0, y, "count-in", &format!("{} bars", g.cfg.count_in()));
    if d != 0 {
        g.cfg.count_in_bars = Some((g.cfg.count_in() as i32 + d).clamp(1, 4) as u32);
        g.cfg.save();
    }
    let d = stepper(p, x + 16.0, y + 70.0, "buffer", &format!("{}", g.cfg.audio.buffer_frames));
    if d != 0 {
        let b = g.cfg.audio.buffer_frames;
        g.cfg.audio.buffer_frames = if d > 0 { (b * 2).min(1024) } else { (b / 2).max(32) };
        restart = true;
    }
    p.text(x + 16.0, y + 148.0, "click: accent 1760 Hz on beat 1, 1175 Hz others", 12.0, col::FAINT, Align::Left);
    let y = 640.0;
    p.label(x + 16.0, y, "latency calibration");
    let txt = match g.cfg.latency_ms {
        Some(l) => format!("{l:+.1} ms"),
        None => "not calibrated".into(),
    };
    p.text(x + 16.0, y + 30.0, txt, 24.0, col::TEXT, Align::Left);
    p.text(
        x + 16.0,
        y + 60.0,
        "Pick palm-muted quarters on the click (8 bars at 100 BPM).",
        13.0,
        col::DIM,
        Align::Left,
    );
    p.text(x + 16.0, y + 78.0, "Stream timestamps already cover the buffers; this", 13.0, col::DIM, Align::Left);
    p.text(x + 16.0, y + 96.0, "adds converter latency + your feel. Enables rush/drag.", 13.0, col::DIM, Align::Left);
    if p.button(x + 16.0, y + 116.0, 200.0, 40.0, "Calibrate now", true) {
        g.start_level(calibration_pattern(), CALIBRATION_BPM, true);
        return;
    }
    if g.cfg.latency_ms.is_some() && p.button(x + 230.0, y + 116.0, 120.0, 40.0, "Reset", false) {
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
