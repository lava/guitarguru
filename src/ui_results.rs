//! Score screen: gauge, timing/tone bars, unlocks, vs-best, three weakest sections with
//! listen-back, timing panel (scale, histogram, per-slot, drift) and tone panel.

use crate::game::{Game, ResultsState, Screen};
use crate::paint::{Align, Paint, col};
use crate::patterns::{BPM_MAX, BPM_STEP, Level, PATTERNS, bar_seconds, sixteenth_seconds};
use crate::progress::PASS_THRESHOLD;
use crate::score::ScoreReport;
use crate::session::Window;
use crate::ui_common::*;
use crate::ui_menu::truncate;

pub fn draw(g: &mut Game, p: &mut Paint) {
    let Some(r) = &g.results else {
        g.screen = Screen::Menu;
        return;
    };
    if r.calibration {
        draw_calibration(g, p);
        return;
    }
    let rep = &r.rep;
    let mut action = Action::None;

    // ---------- gauge
    p.panel(20.0, 16.0, 300.0, 290.0);
    let (cx, cy, rr) = (170.0, 150.0, 104.0);
    let dots = 72;
    for i in 0..dots {
        let a = std::f32::consts::TAU * i as f32 / dots as f32 - std::f32::consts::FRAC_PI_2;
        let lit = (i as f64) < rep.combined_pct / 100.0 * dots as f64;
        let tick80 = i == (dots as f64 * 0.8) as usize;
        let c = if tick80 { col::TEXT } else if lit { col::BRASS } else { col::LINE };
        p.disc(cx + rr * a.cos(), cy + rr * a.sin(), if tick80 { 5.0 } else { 4.0 }, c);
    }
    let pass = rep.combined_pct >= PASS_THRESHOLD;
    p.text(cx, cy - 10.0, format!("{:.0}", rep.combined_pct), 60.0, if pass { col::BRASS } else { col::TEXT }, Align::Center);
    p.text(cx, cy + 34.0, if pass { "PASSED".to_string() } else { format!("pass at {PASS_THRESHOLD:.0}") }, 15.0, col::DIM, Align::Center);
    p.text(cx, 284.0, crate::paint::fit(&format!("{} - {} BPM", r.run.level.name(), r.run.plan.bpm), 280.0, 14.0), 14.0, col::DIM, Align::Center);

    // ---------- timing & tone bars
    p.panel(330.0, 16.0, 430.0, 290.0);
    let bar = |p: &mut Paint, y: f32, label: &str, v: f64, c, detail: String| {
        p.label(350.0, y, label);
        p.text(740.0, y, format!("{v:.0} %"), 22.0, c, Align::Right);
        p.rect(350.0, y + 18.0, 390.0, 8.0, col::BG);
        p.rect(350.0, y + 18.0, 390.0 * (v / 100.0).clamp(0.0, 1.0) as f32, 8.0, c);
        p.text(350.0, y + 42.0, detail, 13.0, col::DIM, Align::Left);
    };
    bar(
        p,
        40.0,
        "timing  (60 %)",
        rep.timing_pct,
        col::BRASS,
        format!("sd {:.1} ms -> {:.0} %   x hits {:.2}   ({} missed, {} extra)", rep.spread_ms, rep.spread_pct, rep.hit_factor, rep.n_missed, rep.n_extra),
    );
    let cen = rep.centroid_std_oct.map_or("-".into(), |c| format!("{c:.2} oct"));
    bar(
        p,
        116.0,
        "tone  (40 %)",
        rep.tone_pct,
        col::TONE,
        format!("attack sd {:.1} dB -> {:.0} %   brightness sd {cen}", rep.level_std_detrended_db, rep.level_pct),
    );
    // unlock chips
    p.label(350.0, 196.0, "unlocked");
    let mut x = 350.0;
    if r.newly.is_empty() {
        p.text(350.0, 222.0, if pass { "nothing new" } else { "score 80 to unlock" }, 14.0, col::FAINT, Align::Left);
    }
    for k in &r.newly {
        let label = k.label();
        let w = 12.0 + label.len() as f32 * 8.0;
        if x + w > 750.0 {
            break;
        }
        p.rect(x, 210.0, w, 24.0, col::PANEL2);
        p.frame(x, 210.0, w, 24.0, 1.0, col::BRASS);
        p.text(x + w / 2.0, 222.0, label, 13.0, col::BRASS, Align::Center);
        x += w + 6.0;
    }
    p.text(350.0, 270.0, format!("latency {:+.1} ms  ({})", rep.latency_ms, if r.run.calibrated { "calibrated" } else { "uncalibrated" }), 12.0, col::FAINT, Align::Left);
    p.text(350.0, 288.0, format!("{} notes, {} quiet re-triggers ignored", rep.n_grid, rep.n_quiet_ignored), 12.0, col::FAINT, Align::Left);

    // ---------- vs best
    p.panel(770.0, 16.0, 410.0, 290.0);
    p.label(790.0, 36.0, "vs best");
    p.text(1000.0, 36.0, "now", 12.0, col::DIM, Align::Right);
    p.text(1080.0, 36.0, "best", 12.0, col::DIM, Align::Right);
    let b = r.prev_best.as_ref();
    let rows: [(&str, String, Option<String>, Option<f64>); 5] = [
        ("score", format!("{:.0} %", rep.combined_pct), b.map(|b| format!("{:.0} %", b.combined)), b.map(|b| rep.combined_pct - b.combined)),
        ("timing sd", format!("{:.1} ms", rep.spread_ms), b.map(|b| format!("{:.1} ms", b.spread_ms)), b.map(|b| b.spread_ms - rep.spread_ms)),
        (
            "offset",
            rep.mean_offset_ms.map_or("-".into(), |o| format!("{o:+.0} ms")),
            b.and_then(|b| b.mean_offset_ms).map(|o| format!("{o:+.0} ms")),
            None,
        ),
        ("attack sd", format!("{:.1} dB", rep.level_std_detrended_db), b.map(|b| format!("{:.1} dB", b.level_std_db)), b.map(|b| b.level_std_db - rep.level_std_detrended_db)),
        ("misses", format!("{}", rep.n_missed), b.map(|b| format!("{}", b.misses)), b.map(|b| b.misses as f64 - rep.n_missed as f64)),
    ];
    for (i, (label, now, best, delta)) in rows.iter().enumerate() {
        let y = 70.0 + i as f32 * 40.0;
        p.text(790.0, y, *label, 15.0, col::DIM, Align::Left);
        p.text(1000.0, y, now.clone(), 18.0, col::TEXT, Align::Right);
        p.text(1080.0, y, best.clone().unwrap_or("-".into()), 15.0, col::DIM, Align::Right);
        if let Some(d) = delta {
            let c = if *d > 0.0 { col::GOOD } else if *d < 0.0 { col::BAD } else { col::DIM };
            p.text(1160.0, y, if *d > 0.0 { "better" } else if *d < 0.0 { "worse" } else { "=" }, 13.0, c, Align::Right);
        }
    }
    if b.is_none() {
        p.text(790.0, 280.0, "first run of this level", 13.0, col::FAINT, Align::Left);
    }

    // ---------- buttons
    if p.button(1194.0, 16.0, 226.0, 52.0, "Retry  (Space)", true) {
        action = Action::Retry;
    }
    if let Some(l) = next_level(g, r) {
        let label = format!("Next: {} BPM", l.bpm);
        if p.button(1194.0, 78.0, 226.0, 44.0, &label, false) {
            action = Action::Start(l);
        }
    }
    if p.button(1194.0, 132.0, 226.0, 44.0, "Level select  (Esc)", false) {
        action = Action::Menu;
    }
    match &r.saved {
        Ok(path) => {
            p.text(1194.0, 196.0, "take saved:", 12.0, col::FAINT, Align::Left);
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            p.text(1194.0, 214.0, truncate(&name, 30), 11.0, col::FAINT, Align::Left);
        }
        Err(e) => {
            p.text(1194.0, 196.0, truncate(&format!("save failed: {e}"), 30), 12.0, col::MISS, Align::Left);
        }
    }

    // ---------- weakest sections
    let playing = g.engine.as_ref().and_then(|e| e.playback_pos());
    if r.listening.is_some() && playing.is_none() {
        // finished
    }
    for (i, w) in r.worst.iter().enumerate() {
        let x = 20.0 + i as f32 * 470.0;
        if let Some(a) = weak_card(p, r, i, w, x, 318.0, 460.0, 210.0, if r.listening == Some(i) { playing } else { None }) {
            action = a;
        }
    }
    if r.worst.is_empty() {
        p.text(40.0, 400.0, "not enough bars for a section breakdown", 14.0, col::FAINT, Align::Left);
    }

    // ---------- timing + tone panels
    timing_panel(p, r, 20.0, 540.0, 930.0, 345.0);
    tone_panel(p, r, 960.0, 540.0, 460.0, 345.0);

    match action {
        Action::None => {}
        Action::Retry => g.retry(),
        Action::Menu => {
            g.stop_listen();
            g.screen = Screen::Menu;
        }
        Action::Start(l) => {
            g.stop_listen();
            g.selected = l;
            g.start_level(l, false);
        }
        Action::Listen(i) => g.listen(i),
        Action::StopListen => g.stop_listen(),
    }
    if let Some(r) = &mut g.results
        && r.listening.is_some()
        && g.engine.as_ref().and_then(|e| e.playback_pos()).is_none()
    {
        r.listening = None;
    }
}

enum Action {
    None,
    Retry,
    Menu,
    Start(Level),
    Listen(usize),
    StopListen,
}

fn next_level(g: &Game, r: &ResultsState) -> Option<Level> {
    // The same combination 20 BPM faster, if open.
    let l = r.run.level;
    let faster = Level { kind: l.kind, bpm: l.bpm + BPM_STEP };
    (faster.bpm <= BPM_MAX && g.progress.is_unlocked(&faster)).then_some(faster)
}

fn window_problems(rep: &ScoreReport, w: &Window) -> Vec<(String, bevy::color::Color)> {
    let devs: Vec<f64> = w.notes.iter().filter_map(|&i| rep.notes[i].dev_ms).collect();
    let lv: Vec<f64> = w.notes.iter().filter_map(|&i| rep.notes[i].level_db).collect();
    let missed = w.notes.iter().filter(|&&i| rep.notes[i].onset_time.is_none()).count();
    let mut out = Vec::new();
    let m = crate::dsp::mean(&devs);
    if m > 8.0 {
        out.push((format!("late {m:+.0}"), col::LATE));
    } else if m < -8.0 {
        out.push((format!("early {m:+.0}"), col::EARLY));
    }
    let sd = crate::dsp::std(&devs);
    if sd > 15.0 {
        out.push((format!("loose sd {sd:.0}"), col::TEXT));
    }
    if missed > 0 {
        out.push((format!("missed {missed}"), col::MISS));
    }
    if w.n_extra > 0 {
        out.push((format!("extra {}", w.n_extra), col::DIM));
    }
    let lsd = crate::dsp::std(&lv);
    if lsd > 2.0 {
        out.push((format!("uneven {lsd:.1} dB"), col::TONE));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn weak_card(p: &mut Paint, r: &ResultsState, i: usize, w: &Window, x: f32, y: f32, cw: f32, ch: f32, playing: Option<(usize, usize)>) -> Option<Action> {
    let rep = &r.rep;
    let run = &r.run;
    let mut act = None;
    p.panel(x, y, cw, ch);
    p.text(x + 16.0, y + 22.0, format!("#{}", i + 1), 22.0, col::BRASS, Align::Left);
    let (s0, s1) = run.window_span(w);
    let t0 = s0 - run.plan.scoring_start();
    let tf = |t: f64| format!("{}:{:02}", (t.max(0.0) as u32) / 60, (t.max(0.0) as u32) % 60);
    let bars = if w.bars == 1 { format!("bar {}", w.start_bar + 1) } else { format!("bars {}-{}", w.start_bar + 1, w.start_bar + w.bars) };
    p.text(x + 56.0, y + 22.0, format!("{bars}   {}-{}", tf(t0), tf(s1 - run.plan.scoring_start())), 15.0, col::TEXT, Align::Left);
    p.text(x + cw - 16.0, y + 22.0, format!("err {:.0} ms", w.error), 13.0, col::DIM, Align::Right);
    let label = if playing.is_some() { "Stop" } else { "Listen" };
    if p.button(x + 16.0, y + 42.0, 84.0, 30.0, label, playing.is_none()) {
        act = Some(if playing.is_some() { Action::StopListen } else { Action::Listen(i) });
    }
    // playback progress
    let prog = playing.map(|(pos, len)| pos as f64 / len.max(1) as f64);
    p.text(x + 110.0, y + 50.0, crate::paint::fit(PATTERNS[w.pattern].name, cw - 270.0, 14.0), 14.0, col::TEXT, Align::Left);
    pattern_glyph(p, x + cw - 136.0, y + 44.0, 120.0, 10.0, &PATTERNS[w.pattern], col::DIM);
    p.rect(x + 110.0, y + 66.0, cw - 126.0, 4.0, col::LINE);
    if let Some(f) = prog {
        p.rect(x + 110.0, y + 66.0, (cw - 126.0) * f as f32, 4.0, col::BRASS);
    }
    // problem chips
    let mut cx = x + 16.0;
    for (txt, c) in window_problems(rep, w) {
        let wch = 14.0 + txt.len() as f32 * 7.5;
        if cx + wch > x + cw - 10.0 {
            break;
        }
        p.frame(cx, y + 82.0, wch, 22.0, 1.0, c);
        p.text(cx + wch / 2.0, y + 93.0, txt, 12.0, c, Align::Center);
        cx += wch + 6.0;
    }
    // hit strip over the window's bars
    let sx = x + 16.0;
    let sw = cw - 32.0;
    let six = sixteenth_seconds(run.plan.bpm);
    let wstart = run.plan.scoring_start() + f64::from(w.start_bar) * bar_seconds(run.plan.bpm);
    let total16 = 16.0 * w.bars as f32;
    let px16 = sw / total16;
    let sy = y + 118.0;
    for k in 0..(total16 as usize) {
        p.rect(sx + k as f32 * px16, sy, 1.0, 40.0, if k % 16 == 0 { col::DIM } else if k % 4 == 0 { col::LINE } else { col::PANEL2 });
    }
    let worst_note = w.notes.iter().copied().max_by(|&a, &b| note_err(rep, a).total_cmp(&note_err(rep, b)));
    for &ni in &w.notes {
        let n = &rep.notes[ni];
        let g = &run.plan.notes[ni];
        let nx = sx + ((g.time - wstart) / six) as f32 * px16;
        p.rect(nx, sy + 6.0, (px16 * 0.6).max(3.0), 12.0, dim(col::TEXT, 0.5));
        match n.dev_ms {
            Some(d) => hit_marker(p, nx + (d / 1e3 / six) as f32 * px16, sy + 30.0, 6.0, d),
            None => p.cross(nx + 2.0, sy + 30.0, 5.0, col::MISS),
        }
        if Some(ni) == worst_note {
            p.frame(nx - 4.0, sy + 2.0, (px16 * 0.6).max(3.0) + 8.0, 36.0, 1.0, col::MISS);
        }
    }
    if let Some((pos, _)) = playing {
        let t = s0 + pos as f64 / f64::from(run.sample_rate);
        let ph = sx + ((t - wstart) / six) as f32 * px16;
        if ph >= sx && ph <= sx + sw {
            p.rect(ph - 1.0, sy - 4.0, 2.0, 48.0, col::BRASS);
        }
    }
    // attack level bars relative to the take median
    let ly = y + 168.0;
    p.rect(sx, ly + 14.0, sw, 1.0, col::LINE);
    for &ni in &w.notes {
        let n = &rep.notes[ni];
        let g = &run.plan.notes[ni];
        let nx = sx + ((g.time - wstart) / six) as f32 * px16;
        if let Some(l) = n.level_db {
            let d = ((l - rep.level_median_db) * 3.0).clamp(-14.0, 14.0) as f32;
            if d >= 0.0 {
                p.rect(nx, ly + 14.0 - d, 3.0, d.max(1.0), col::TONE);
            } else {
                p.rect(nx, ly + 14.0, 3.0, -d, dim(col::TONE, 0.6));
            }
        }
    }
    p.text(x + cw - 16.0, ly + 30.0, "attack dB vs median", 10.0, col::FAINT, Align::Right);
    act
}

fn note_err(rep: &ScoreReport, i: usize) -> f64 {
    rep.notes[i].dev_ms.map_or(100.0, f64::abs)
}

fn timing_panel(p: &mut Paint, r: &ResultsState, x: f32, y: f32, w: f32, h: f32) {
    let rep = &r.rep;
    p.panel(x, y, w, h);
    p.label(x + 16.0, y + 20.0, "timing");
    // sd scale 0..40 ms with bands
    let (sx, sy, sw) = (x + 16.0, y + 52.0, 280.0);
    let pos = |ms: f64| sx + sw * (ms / 40.0).clamp(0.0, 1.0) as f32;
    p.rect(sx, sy, sw, 10.0, col::BG);
    p.rect(pos(5.0), sy, pos(10.0) - pos(5.0), 10.0, dim(col::BRASS, 0.6));
    p.rect(pos(10.0), sy, pos(15.0) - pos(10.0), 10.0, dim(col::BRASS, 0.3));
    p.text(pos(7.5), sy + 22.0, "pro", 11.0, col::DIM, Align::Center);
    p.text(pos(12.5), sy + 22.0, "good", 11.0, col::DIM, Align::Center);
    for t in [0.0, 20.0, 30.0, 40.0] {
        p.text(pos(t), sy + 22.0, format!("{t:.0}"), 11.0, col::FAINT, Align::Center);
    }
    p.disc(pos(rep.spread_ms), sy + 5.0, 7.0, col::TEXT);
    if let Some(b) = &r.prev_best {
        p.rect(pos(b.spread_ms) - 1.0, sy - 6.0, 2.0, 22.0, col::DIM);
    }
    p.text(sx, sy + 44.0, format!("sd {:.1} ms", rep.spread_ms), 20.0, col::TEXT, Align::Left);
    let off = rep.mean_offset_ms.map_or("offset: calibrate".to_string(), |o| {
        format!("{} {:+.1} ms", if o < 0.0 { "rushing" } else { "dragging" }, o)
    });
    let oc = rep.mean_offset_ms.map_or(col::DIM, dev_color);
    p.text(sx + 140.0, sy + 44.0, off, 15.0, oc, Align::Left);
    p.text(
        sx,
        sy + 70.0,
        format!("slot {:.1}  jitter {:.1}  wander {:.0} ms (not scored)", rep.slot_pattern_std_ms, rep.jitter_std_ms, rep.wander_ms),
        13.0,
        col::DIM,
        Align::Left,
    );

    // histogram -70..+70 ms, 5 ms bins
    let (hx, hy, hw, hh) = (x + 330.0, y + 36.0, 280.0, 90.0);
    let mut bins = [0usize; 28];
    for d in rep.notes.iter().filter_map(|n| n.dev_ms) {
        let b = ((d + 70.0) / 5.0).floor();
        if (0.0..28.0).contains(&b) {
            bins[b as usize] += 1;
        }
    }
    let maxb = *bins.iter().max().unwrap_or(&1).max(&1);
    let bw = hw / 28.0;
    for (i, c) in bins.iter().enumerate() {
        let center = -70.0 + 5.0 * i as f64 + 2.5;
        let hgt = hh * *c as f32 / maxb as f32;
        p.rect(hx + i as f32 * bw, hy + hh - hgt, bw - 1.0, hgt, dev_color(center));
    }
    p.rect(hx + hw / 2.0, hy, 1.0, hh, col::DIM);
    p.text(hx, hy + hh + 12.0, "-70", 11.0, col::FAINT, Align::Left);
    p.text(hx + hw / 2.0, hy + hh + 12.0, "0", 11.0, col::FAINT, Align::Center);
    p.text(hx + hw, hy + hh + 12.0, "+70 ms", 11.0, col::FAINT, Align::Right);

    // per-slot mean deviation, one row per pattern (slots differ between patterns)
    let (px, py, pw, ph) = (x + 640.0, y + 36.0, w - 660.0, 120.0);
    p.label(px, py - 16.0, "per slot, per pattern (mean ms)");
    let mut pats: Vec<(String, usize)> = Vec::new();
    for s in &rep.slots {
        match pats.iter_mut().find(|(id, _)| *id == s.pattern) {
            Some(e) => e.1 += s.count,
            None => pats.push((s.pattern.clone(), s.count)),
        }
    }
    pats.sort_by_key(|e| std::cmp::Reverse(e.1));
    pats.truncate(4);
    let rows = pats.len().max(1);
    let rh = ph / rows as f32;
    let (gx, gw) = (px + 110.0, pw - 110.0);
    for (ri, (id, _)) in pats.iter().enumerate() {
        let ry = py + ri as f32 * rh;
        let mid = ry + rh / 2.0;
        let name = PATTERNS.iter().find(|q| q.id == *id).map_or(id.as_str(), |q| q.name);
        p.text(px, mid, truncate(name, 13), 11.0, col::DIM, Align::Left);
        p.rect(gx, mid, gw, 1.0, col::LINE);
        for s in rep.slots.iter().filter(|s| s.pattern == *id) {
            let hgt = (s.mean_ms * 1.2).clamp(-(rh as f64) / 2.0 + 1.0, rh as f64 / 2.0 - 1.0) as f32;
            let c = if s.consistent { dev_color(s.mean_ms * 10.0) } else { col::DIM };
            let bx = gx + gw * (s.pos16 as f32 / 16.0);
            let bw = (gw / 16.0 - 3.0).max(2.0);
            if hgt >= 0.0 {
                p.rect(bx, mid, bw, hgt.max(1.0), c);
            } else {
                p.rect(bx, mid + hgt, bw, -hgt, c);
            }
        }
    }
    p.text(px, py + ph + 14.0, "16th grid; color = consistent bias", 11.0, col::FAINT, Align::Left);

    // drift line with jitter band and pins at weak sections
    let (dx, dy, dw, dh) = (x + 16.0, y + 190.0, w - 32.0, 130.0);
    p.label(dx, dy - 16.0, "timing over the take: wander (11-note average) and jitter band");
    p.rect(dx, dy, dw, dh, col::BG);
    // Total deviation from the constant latency: the slow part is the tracked wander.
    let pts: Vec<(f64, f64)> =
        rep.notes.iter().filter_map(|n| n.dev_ms.map(|d| (n.grid_time, d + n.offset_ms - rep.latency_ms))).collect();
    if pts.len() > 2 {
        let t0 = rep.notes.first().map_or(0.0, |n| n.grid_time);
        let t1 = rep.notes.last().map_or(1.0, |n| n.grid_time).max(t0 + 1e-3);
        let xs = |t: f64| dx + dw * ((t - t0) / (t1 - t0)) as f32;
        let ys = |ms: f64| dy + dh / 2.0 - (ms.clamp(-150.0, 150.0) / 150.0) as f32 * dh / 2.0;
        p.rect(dx, ys(0.0), dw, 1.0, col::LINE);
        let half = 5;
        let ma: Vec<f64> = (0..pts.len())
            .map(|i| crate::dsp::mean(&pts[i.saturating_sub(half)..(i + half + 1).min(pts.len())].iter().map(|p| p.1).collect::<Vec<_>>()))
            .collect();
        let jit = rep.jitter_std_ms;
        // jitter band as thin vertical strokes
        for (i, (t, _)) in pts.iter().enumerate().step_by((pts.len() / 300).max(1)) {
            let (a, b) = (ys(ma[i] + jit), ys(ma[i] - jit));
            p.rect(xs(*t), a, 1.5, b - a, dim(col::BRASS, 0.25));
        }
        for i in 1..pts.len() {
            p.line(xs(pts[i - 1].0), ys(ma[i - 1]), xs(pts[i].0), ys(ma[i]), col::BRASS);
        }
        for (k, w) in r.worst.iter().enumerate() {
            if let Some(&ni) = w.notes.first() {
                let px = xs(rep.notes[ni].grid_time);
                p.rect(px, dy, 1.0, dh, col::MISS);
                p.text(px + 4.0, dy + 10.0, format!("#{}", k + 1), 11.0, col::MISS, Align::Left);
            }
        }
        p.text(dx + 4.0, ys(150.0) + 8.0, "+150 ms late", 10.0, col::FAINT, Align::Left);
        p.text(dx + 4.0, ys(-150.0) - 8.0, "-150 ms early", 10.0, col::FAINT, Align::Left);
    }
}

fn tone_panel(p: &mut Paint, r: &ResultsState, x: f32, y: f32, w: f32, h: f32) {
    let rep = &r.rep;
    p.panel(x, y, w, h);
    p.label(x + 16.0, y + 20.0, "tone");
    let (sx, sy, sw) = (x + 16.0, y + 52.0, w - 32.0);
    let pos = |db: f64| sx + sw * (db / 5.0).clamp(0.0, 1.0) as f32;
    p.rect(sx, sy, sw, 10.0, col::BG);
    p.rect(pos(0.0), sy, pos(1.0) - pos(0.0), 10.0, dim(col::TONE, 0.5));
    for t in 0..=5 {
        p.text(pos(f64::from(t)), sy + 22.0, format!("{t}"), 11.0, col::FAINT, Align::Center);
    }
    p.disc(pos(rep.level_std_detrended_db), sy + 5.0, 7.0, col::TONE);
    p.text(sx, sy + 44.0, format!("attack sd {:.1} dB", rep.level_std_detrended_db), 20.0, col::TONE, Align::Left);
    p.text(sx + 220.0, sy + 44.0, format!("raw {:.1} dB, median {:.0} dBFS", rep.level_std_db, rep.level_median_db), 12.0, col::DIM, Align::Left);
    // Per group, as marked by the patterns (no P.M. detection): count-weighted into the score.
    let grp = |g: &crate::score::ToneGroup| {
        if g.n == 0 {
            "-".to_string()
        } else {
            format!(
                "{} notes  {:.1} dB  {}",
                g.n,
                g.level_std_db,
                g.centroid_std_oct.map_or("-".into(), |c| format!("{c:.2} oct")),
            )
        }
    };
    p.text(sx, sy + 66.0, format!("P.M.  {}", grp(&rep.pm)), 12.0, col::TONE, Align::Left);
    p.text(sx, sy + 82.0, format!("open  {}", grp(&rep.open)), 12.0, col::TONE, Align::Left);

    // attack level per bar relative to the median, +/-1 dB band
    let (bx, by, bw, bh) = (x + 16.0, y + 172.0, w - 32.0, 62.0);
    p.label(bx, by - 14.0, "attack per bar vs median (+/-1 dB band)");
    p.rect(bx, by, bw, bh, col::BG);
    let ys = |db: f64| by + bh / 2.0 - (db.clamp(-4.0, 4.0) / 4.0) as f32 * bh / 2.0;
    p.rect(bx, ys(1.0), bw, ys(-1.0) - ys(1.0), dim(col::TONE, 0.2));
    let nb = r.run.plan.scored_bars.max(1);
    let colw = bw / nb as f32;
    for b in 0..nb {
        let lv: Vec<f64> = rep
            .notes
            .iter()
            .zip(&r.run.plan.notes)
            .filter(|(_, g)| g.bar == b)
            .filter_map(|(n, _)| n.level_db)
            .collect();
        if lv.is_empty() {
            continue;
        }
        let d = crate::dsp::mean(&lv) - rep.level_median_db;
        let (a, c) = (ys(d), ys(0.0));
        let (top, hgt) = if a < c { (a, c - a) } else { (c, a - c) };
        p.rect(bx + b as f32 * colw, top, (colw - 1.0).max(1.0), hgt.max(1.0), col::TONE);
    }
    // palm-mute brightness (centroid per bar)
    let (cx, cy, cw, chh) = (x + 16.0, y + 268.0, w - 32.0, 56.0);
    p.label(cx, cy - 14.0, "attack brightness per bar (centroid)");
    p.rect(cx, cy, cw, chh, col::BG);
    let per_bar: Vec<f64> = (0..nb)
        .filter_map(|b| {
            let v: Vec<f64> = r
                .centroids
                .iter()
                .zip(&r.run.plan.notes)
                .filter(|(_, g)| g.bar == b)
                .filter_map(|(c, _)| c.map(f64::log2))
                .collect();
            (!v.is_empty()).then(|| crate::dsp::mean(&v))
        })
        .collect();
    if per_bar.len() > 1 {
        let (lo, hi) = per_bar.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
        let mid = (lo + hi) / 2.0;
        let span = (hi - lo).max(0.5); // at least half an octave visible
        let yy = |v: f64| cy + chh / 2.0 - ((v - mid) / span) as f32 * chh * 0.9;
        for i in 1..per_bar.len() {
            let x0 = cx + cw * (i - 1) as f32 / (per_bar.len() - 1) as f32;
            let x1 = cx + cw * i as f32 / (per_bar.len() - 1) as f32;
            p.line(x0, yy(per_bar[i - 1]), x1, yy(per_bar[i]), col::TONE);
        }
        p.text(cx + cw, cy + chh + 10.0, format!("~{:.0} Hz", 2f64.powf(mid)), 11.0, col::FAINT, Align::Right);
    }
}

fn draw_calibration(g: &mut Game, p: &mut Paint) {
    let Some(r) = &g.results else { return };
    let rep = &r.rep;
    p.panel(320.0, 180.0, 800.0, 440.0);
    p.text(720.0, 230.0, "LATENCY CALIBRATION", 30.0, col::BRASS, Align::Center);
    if rep.n_matched >= 6 {
        p.text(720.0, 320.0, format!("{:+.1} ms", rep.latency_ms), 64.0, col::TEXT, Align::Center);
        p.text(720.0, 380.0, format!("from {} of {} clicks, spread {:.1} ms", rep.n_matched, rep.n_grid, rep.spread_ms), 18.0, col::DIM, Align::Center);
        p.text(720.0, 410.0, "Saved. Future runs report rushing/dragging against this.", 15.0, col::DIM, Align::Center);
    } else {
        p.text(720.0, 330.0, "Not enough notes detected", 30.0, col::MISS, Align::Center);
        p.text(720.0, 370.0, format!("{} of {} clicks matched - check the input level in Settings", rep.n_matched, rep.n_grid), 16.0, col::DIM, Align::Center);
    }
    let retry = p.button(470.0, 520.0, 220.0, 50.0, "Again", false);
    let done = p.button(750.0, 520.0, 220.0, 50.0, "Done", true);
    if retry {
        g.retry();
    } else if done {
        g.screen = Screen::Menu;
    }
}
