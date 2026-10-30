//! Results ("Guitar Guru" style, scrollable): a 32-petal mandala whose petals fill to the
//! score, the guru (happy or sad) with a short verdict, fact columns, the whole run (drift
//! against the global offset, weak-spot pins, section letters), "Listen again" for the
//! three weakest windows, the timing section (evenness headline, global offset, interval
//! errors per kind of interval, deviations, per-pattern slots) and the tone section (stroke
//! envelopes per pick direction and P.M. group).

use bevy::color::{Alpha, Color};

use crate::game::{Game, ResultsState, Screen};
use crate::mascot::{self, MascotImages, Mood};
use crate::paint::{Align, Font, Paint, col, fit};
use crate::patterns::{BPM_MAX, BPM_STEP, INGREDIENTS, Level, LevelKind, PATTERNS, bar_seconds, sixteenth_seconds};
use crate::progress::PASS_THRESHOLD;
use crate::score::ScoreReport;
use crate::session::Window;
use crate::tone::Feature;
use crate::ui_menu::link;

/// Content height before the first frame has measured it.
const CONTENT_H: f32 = 2510.0;

enum Action {
    None,
    Retry,
    Menu,
    Start(Level),
    Listen(usize),
    StopListen,
}

fn dev_color(d: f64) -> Color {
    if d.abs() <= 10.0 {
        col::ONTIME
    } else if d < 0.0 {
        col::EARLY
    } else {
        col::LATE
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

pub fn draw(g: &mut Game, p: &mut Paint, imgs: &MascotImages) {
    let Some(r) = &g.results else {
        g.screen = Screen::Menu;
        return;
    };
    if r.calibration {
        draw_calibration(g, p);
        return;
    }
    // Scrolling
    let content_h = r.content_h.unwrap_or(CONTENT_H);
    if p.scroll != 0.0 {
        g.scroll = (g.scroll - p.scroll).clamp(0.0, (content_h - 900.0).max(0.0));
    }
    p.offset_y = -g.scroll;
    let since = (g.clock - g.screen_since) as f32;
    let r = g.results.as_ref().unwrap();
    let rep = &r.rep;
    let mut action = Action::None;
    let playing = g.engine.as_ref().and_then(|e| e.playback_pos());

    // ---------- top bar
    if link(p, 192.0, 54.0, "Tempo path") {
        action = Action::Menu;
    }
    let lvl = r.run.level;
    let mut desc = vec![format!("{} BPM", lvl.bpm)];
    match lvl.kind {
        LevelKind::Run(m) => desc.extend(INGREDIENTS.iter().filter(|i| m & i.bit != 0).map(|i| i.name.to_string())),
        LevelKind::Drill(pi) => desc.push(format!("drill \u{b7} {}", PATTERNS[pi].name)),
    }
    p.label_at(207.0, 56.0, &desc.join(" \u{b7} "), Align::Left, col::DIM);
    if p.button(965.0, 34.0, 92.0, 44.0, "Again", false) {
        action = Action::Retry;
    }
    if let Some(n) = next_level(g, r) {
        if p.button(1073.0, 34.0, 170.0, 44.0, &format!("Next: {} BPM", n.bpm), true) {
            action = Action::Start(n);
        }
    } else if p.button(1073.0, 34.0, 170.0, 44.0, "Tempo path", true) {
        action = Action::Menu;
    }

    // ---------- mandala
    let pass = rep.combined_pct >= PASS_THRESHOLD;
    let shown = rep.combined_pct as f32 * ease(since / 1.4);
    mandala(p, 405.0, 330.0, shown, r.prev_best.as_ref().map(|b| b.combined as f32));
    p.serif(400.0, 318.0, format!("{shown:.0}"), 100.0, col::TEXT, Align::Center);
    p.serif(462.0, 340.0, "%", 26.0, col::TEXT, Align::Left);
    p.italic(405.0, 380.0, if pass { "passed" } else { "not yet" }, 26.0, if pass { col::ONTIME } else { col::LATE }, Align::Center);

    // ---------- guru + verdict
    let stage = mascot::stage_for(lvl.bpm, lvl.mask().count_ones());
    let better = r.prev_best.as_ref().is_none_or(|b| rep.combined_pct > b.combined);
    let mood = if pass { Mood::Happy } else { Mood::Sad };
    mascot::draw(p, imgs, 760.0, 120.0, 0.62, stage, mood, &mascot::pose(stage, None));
    let quote = verdict(r, pass, better);
    let lines = wrap(&quote, 40);
    for (i, l) in lines.iter().enumerate() {
        p.italic(930.0, 228.0 + i as f32 * 28.0, l, 21.0, col::TEXT, Align::Left);
    }
    // fact columns
    let fy = 330.0;
    p.hline(752.0, fy, 490.0, col::LINE);
    let cols: [(&str, String, Color); 4] = [
        ("timing", format!("{:.0}%", rep.timing_pct), col::EARLY),
        ("tone", format!("{:.0}%", rep.tone_pct), col::TONE),
        (
            "before",
            r.prev_best.as_ref().map_or("\u{2013}".into(), |b| format!("{:.0}", b.combined)),
            col::TEXT,
        ),
        ("hit \u{b7} miss \u{b7} extra", format!("{} \u{b7} {} \u{b7} {}", rep.n_matched, rep.n_missed, rep.n_extra), col::TEXT),
    ];
    for (i, (lab, v, c)) in cols.iter().enumerate() {
        let x = 752.0 + i as f32 * 124.0;
        if i > 0 {
            p.rect(x - 8.0, fy, 1.0, 120.0, col::LINE);
        }
        p.label(x, fy + 20.0, lab);
        let size = if i == 3 { 20.0 } else { 30.0 };
        p.serif(x, fy + 52.0, v.clone(), size, *c, Align::Left);
        if i == 2
            && let Some(b) = &r.prev_best
        {
            let d = rep.combined_pct - b.combined;
            p.text(x + 42.0, fy + 58.0, format!("{d:+.0}"), 12.0, if d >= 0.0 { col::ONTIME } else { col::LATE }, Align::Left);
        }
    }
    if let Some(note) = &r.difficulty_note {
        p.text(752.0, fy + 136.0, fit(note, 490.0, 12.0), 12.0, col::DIM, Align::Left);
    }
    if !r.newly.is_empty() {
        let txt: Vec<String> = r.newly.iter().map(|u| u.label()).collect();
        p.label(752.0, 482.0, "opened");
        p.serif(816.0, 478.0, fit(&txt.join(", "), 420.0, 22.0), 22.0, col::ONTIME, Align::Left);
    }
    match &r.saved {
        Ok(path) => {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            p.text(752.0, 520.0, format!("saved \u{b7} {}", fit(&name, 440.0, 11.0)), 11.0, col::FAINT, Align::Left);
        }
        Err(e) => p.text(752.0, 520.0, format!("save failed: {e}"), 11.0, col::LATE, Align::Left),
    }

    // ---------- the whole run
    run_timeline(p, r, 113.0, 580.0);

    // ---------- listen again
    let ly = 830.0;
    p.italic(113.0, ly, "Listen again", 34.0, col::TEXT, Align::Left);
    p.hline(113.0, ly + 24.0, 1130.0, col::LINE);
    for (i, w) in r.worst.iter().enumerate() {
        let x = 113.0 + i as f32 * 390.0;
        if i > 0 {
            p.rect(x - 22.0, ly + 40.0, 1.0, 170.0, col::LINE);
        }
        let this_playing = if r.listening == Some(i) { playing } else { None };
        if let Some(a) = listen_card(p, r, i, w, x, ly + 40.0, 355.0, this_playing) {
            action = a;
        }
    }
    if r.worst.is_empty() {
        p.text(113.0, ly + 60.0, "\u{2013}", 14.0, col::FAINT, Align::Left);
    }

    // ---------- timing
    let bottom = timing_section(p, r, 113.0, 1100.0);
    // ---------- tone
    let end = tone_section(p, r, 113.0, bottom + 60.0);

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
    if let Some(r) = &mut g.results {
        r.content_h = Some(end + 60.0);
    }
    if let Some(r) = &mut g.results
        && r.listening.is_some()
        && g.engine.as_ref().and_then(|e| e.playback_pos()).is_none()
    {
        r.listening = None;
    }
}

fn next_level(g: &Game, r: &ResultsState) -> Option<Level> {
    let l = r.run.level;
    let faster = Level { kind: l.kind, bpm: l.bpm + BPM_STEP };
    (faster.bpm <= BPM_MAX && g.progress.is_unlocked(&faster)).then_some(faster)
}

/// 32 petals around the score: filled (gold) up to the score, clockwise from the top;
/// ticks for the pass mark and the previous best.
fn mandala(p: &mut Paint, cx: f32, cy: f32, score: f32, prev: Option<f32>) {
    p.ring(cx, cy, 201.0, col::GOLD);
    p.dotted_ring(cx, cy, 193.0, 0.7, col::GOLD);
    let n = 32;
    let filled = (score / 100.0 * n as f32).round() as usize;
    for i in 0..n {
        let a = std::f32::consts::TAU * (i as f32 + 0.5) / n as f32; // clockwise from top
        let (s, c) = a.sin_cos();
        let (px, py) = (cx + 162.0 * s, cy - 162.0 * c);
        let rot = -a; // petal points outwards
        p.petal(px, py, 46.0, 22.0, rot, col::TEXT);
        p.petal(px, py, 43.0, 19.5, rot, if i < filled { Color::srgb(0.86, 0.64, 0.32) } else { col::BG });
    }
    p.disc(cx, cy, 134.0, Color::srgb(0.98, 0.96, 0.925));
    p.ring(cx, cy, 134.0, col::TEXT);
    p.ring(cx, cy, 126.0, col::LINE);
    let tick = |p: &mut Paint, v: f32, c: Color, len: f32| {
        let a = std::f32::consts::TAU * v / 100.0;
        let (s, co) = a.sin_cos();
        p.line(cx + 128.0 * s, cy - 128.0 * co, cx + (128.0 + len) * s, cy - (128.0 + len) * co, c);
        p.line(cx + 128.0 * s + 0.8, cy - 128.0 * co, cx + (128.0 + len) * s + 0.8, cy - (128.0 + len) * co, c);
    };
    tick(p, PASS_THRESHOLD as f32, col::TEXT, 64.0);
    if let Some(b) = prev {
        let a = std::f32::consts::TAU * b / 100.0;
        p.disc(cx + 134.0 * a.sin(), cy - 134.0 * a.cos(), 5.0, col::BG);
        p.ring(cx + 134.0 * a.sin(), cy - 134.0 * a.cos(), 5.0, col::TEXT);
    }
}

fn wrap(s: &str, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for w in s.split_whitespace() {
        if cur.len() + w.len() + 1 > n && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A short, honest verdict from the numbers.
fn verdict(r: &ResultsState, pass: bool, better: bool) -> String {
    let rep = &r.rep;
    let head = match (pass, better && r.prev_best.is_some()) {
        (true, true) => "A new best.",
        (true, false) => "Passed.",
        (false, true) => "Better than before, not there yet.",
        (false, false) => "Not yet.",
    };
    let slot = rep.slots.iter().filter(|s| s.consistent).max_by(|a, b| a.mean_ms.abs().total_cmp(&b.mean_ms.abs()));
    let body = if rep.n_missed * 5 > rep.n_grid {
        format!("{} of {} notes missed.", rep.n_missed, rep.n_grid)
    } else if rep.drift_rms_ms > 40.0 {
        format!("Drift {:.0} ms.", rep.drift_rms_ms)
    } else if let Some(s) = slot {
        let name = r.run.plan.patterns.iter().find(|p| p.id == s.pattern).map_or("the pattern", |p| &*p.name);
        format!("In {name}, {} comes {} ({:+.0} ms).", count_name(s.pos16), if s.mean_ms < 0.0 { "early" } else { "late" }, s.mean_ms)
    } else if rep.spread_ms > 25.0 {
        format!("Evenness {:.0} ms.", rep.spread_ms)
    } else if let Some((f, sd)) = rep.tone.worst().filter(|_| rep.tone_pct < 60.0) {
        let what = match f {
            Feature::Level => "Stroke volume",
            Feature::Attack => "Pick attack",
            Feature::Sustain => "Sustain",
            Feature::Brightness => "Brightness",
        };
        format!("{what} varies by {}.", f.fmt(sd))
    } else {
        format!("Evenness {:.0} ms.", rep.spread_ms)
    };
    format!("\u{201c}{head} {body}\u{201d}")
}

fn count_name(pos16: f64) -> String {
    let k = pos16.round() as i64;
    match k % 4 {
        0 => format!("beat {}", k / 4 + 1),
        1 => format!("the \u{201c}e\u{201d} of {}", k / 4 + 1),
        2 => format!("the \u{201c}+\u{201d} of {}", k / 4 + 1),
        _ => format!("the \u{201c}a\u{201d} of {}", k / 4 + 1),
    }
}

/// What went wrong in a window, worst first by what it cost the window's score: uneven
/// intervals, a steady offset against the run's global offset ("drifted"), missed and
/// extra notes, uneven attacks.
fn window_problems(w: &Window) -> Vec<(String, Color)> {
    let mut out: Vec<(f64, String, Color)> = Vec::new();
    if let Some(e) = w.evenness_ms.filter(|e| *e > 15.0) {
        out.push((0.75 * (e - 10.0) / 30.0, format!("uneven \u{b1}{e:.0} ms"), col::LATE));
    }
    if w.drift_ms.abs() > 15.0 {
        out.push((0.25 * (w.drift_ms.abs() - 15.0) / 85.0, format!("drifted {:+.0} ms", w.drift_ms), dev_color(w.drift_ms)));
    }
    let n = w.notes.len().max(1) as f64;
    if w.n_missed > 0 {
        out.push((w.n_missed as f64 / n, format!("{} missed", w.n_missed), col::TEXT));
    }
    if w.n_extra > 0 {
        out.push((0.5 * w.n_extra as f64 / n, format!("{} extra", w.n_extra), col::DIM));
    }
    if let Some((f, sd, cost)) = crate::tone::worst_feature(&w.tone_sd).filter(|w| w.2 > 0.1) {
        out.push((0.4 * cost, format!("{} {}", f.name(), f.fmt(sd)), col::TONE));
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().map(|(_, t, c)| (t, c)).collect()
}

#[allow(clippy::too_many_arguments)]
fn listen_card(p: &mut Paint, r: &ResultsState, i: usize, w: &Window, x: f32, y: f32, cw: f32, playing: Option<(usize, usize)>) -> Option<Action> {
    let rep = &r.rep;
    let run = &r.run;
    let mut act = None;
    p.serif(x, y + 22.0, format!("{}", i + 1), 32.0, col::OCHRE, Align::Left);
    let glyph = if playing.is_some() { "stop" } else { "play" };
    if p.round_button(x + 52.0, y + 22.0, 20.0, glyph) {
        act = Some(if playing.is_some() { Action::StopListen } else { Action::Listen(i) });
    }
    let (s0, s1) = run.window_span(w);
    let st = run.plan.scoring_start();
    let tf = |t: f64| format!("{}:{:02}", (t.max(0.0) as u32) / 60, (t.max(0.0) as u32) % 60);
    p.serif(x + 86.0, y + 14.0, format!("{}\u{2013}{}", tf(s0 - st), tf(s1 - st)), 22.0, col::TEXT, Align::Left);
    let bars = if w.bars == 1 { format!("bar {}", w.start_bar + 1) } else { format!("bars {}\u{2013}{}", w.start_bar + 1, w.start_bar + w.bars) };
    p.text(x + 86.0, y + 36.0, bars, 12.0, col::DIM, Align::Left);
    let wscore = w.score_pct;
    p.serif(x + cw, y + 22.0, format!("{wscore:.0}"), 30.0, col::LATE, Align::Right);
    // progress line while playing
    p.hline(x, y + 58.0, cw, col::LINE);
    if let Some((pos, len)) = playing {
        p.rect(x, y + 57.0, cw * pos as f32 / len.max(1) as f32, 2.0, col::OCHRE);
    }
    p.label_at(x, y + 80.0, &format!("{} \u{b7} {}", letter_of(run, w.pattern), fit(&run.plan.patterns[w.pattern].name, 300.0, 13.0)), Align::Left, col::DIM);
    // notes: arrow row and coloured dot row on a 16th grid over the window
    let six = sixteenth_seconds(run.plan.bpm);
    let wstart = st + f64::from(w.start_bar) * bar_seconds(run.plan.bpm);
    let px16 = cw / (16.0 * w.bars as f32);
    let worst = w.notes.iter().copied().max_by(|&a, &b| err(rep, a).total_cmp(&err(rep, b)));
    for &ni in &w.notes {
        let n = &rep.notes[ni];
        let gnote = &run.plan.notes[ni];
        let pat = &run.plan.patterns[gnote.pattern];
        let down = pat.hits().get(gnote.slot).is_none_or(|h| h.downstroke());
        let nx = x + ((gnote.time - wstart) / six) as f32 * px16 + 3.0;
        p.stroke_arrow(nx, y + 106.0, 12.0, down, col::TEXT);
        match n.dev_ms {
            Some(d) => p.disc(nx, y + 128.0, 4.0, dev_color(d)),
            None => p.ring(nx, y + 128.0, 4.0, col::TEXT),
        }
        if Some(ni) == worst {
            p.ring(nx, y + 128.0, 8.0, col::OCHRE);
        }
    }
    if let Some((pos, _)) = playing {
        let t = s0 + pos as f64 / f64::from(run.sample_rate);
        let ph = x + ((t - wstart) / six) as f32 * px16;
        if ph >= x && ph <= x + cw {
            p.rect(ph - 0.75, y + 92.0, 1.5, 46.0, col::OCHRE);
        }
    }
    let mut cx = x;
    for (txt, c) in window_problems(w) {
        p.disc(cx + 3.0, y + 156.0, 3.0, c);
        p.text(cx + 11.0, y + 155.0, &txt, 13.0, col::TEXT, Align::Left);
        cx += 22.0 + txt.len() as f32 * 7.0;
        if cx > x + cw - 60.0 {
            break;
        }
    }
    act
}

fn err(rep: &ScoreReport, i: usize) -> f64 {
    rep.notes[i].dev_ms.map_or(100.0, f64::abs)
}

fn letter_of(run: &crate::session::RunData, pat: usize) -> char {
    let used = run.patterns_used();
    let i = used.iter().position(|&x| x == pat).unwrap_or(0);
    (b'A' + (i as u8 % 26)) as char
}

fn section_header(p: &mut Paint, x: f32, y: f32, title: &str, label: &str) {
    p.italic(x, y, title, 34.0, col::TEXT, Align::Left);
    if !label.is_empty() {
        p.label(x + 14.0 + title.len() as f32 * 13.0, y + 6.0, label);
    }
    p.hline(x, y + 24.0, 1130.0, col::LINE);
}

/// Returns the bottom y.
fn timing_section(p: &mut Paint, r: &ResultsState, x: f32, y: f32) -> f32 {
    let rep = &r.rep;
    section_header(p, x, y, "Timing", "");
    // legend
    for (i, (c, t)) in [(col::EARLY, "early"), (col::ONTIME, "within 10 ms"), (col::LATE, "late")].iter().enumerate() {
        let lx = 1000.0 + [0.0, 64.0, 168.0][i];
        p.disc(lx, y + 4.0, 3.5, *c);
        p.text(lx + 8.0, y + 4.0, *t, 12.0, col::DIM, Align::Left);
    }
    // headline numbers
    let ny = y + 70.0;
    let prev = r.prev_best.as_ref();
    let cols: [(&str, String, &str, String, Color); 3] = [
        ("evenness", format!("{:.0}", rep.spread_ms), "ms", prev.map_or(format!("IOI sd {:.0} ms", rep.ioi_sd_ms), |b| format!("was {:.0}", b.spread_ms)), col::TEXT),
        ("drift", format!("{:.0}", rep.drift_rms_ms), "ms", String::new(), col::TEXT),
        ("latency + feel", format!("{:+.0}", rep.latency_ms), "ms", String::new(), col::DIM),
    ];
    for (i, (lab, v, unit, sub, c)) in cols.iter().enumerate() {
        let cx = x + i as f32 * 150.0;
        p.label(cx, ny, lab);
        p.serif(cx, ny + 38.0, v.clone(), 44.0, *c, Align::Left);
        p.text(cx + 12.0 + v.len() as f32 * 20.0, ny + 46.0, *unit, 13.0, col::DIM, Align::Left);
        p.text(cx, ny + 72.0, sub.clone(), 12.0, col::DIM, Align::Left);
    }
    // scale 0..40 ms with bands
    let (sx, sy, sw) = (x, ny + 120.0, 540.0);
    let pos = |ms: f64| sx + sw * (ms / 40.0).clamp(0.0, 1.0) as f32;
    p.hline(sx, sy, sw, col::TEXT);
    p.rect(pos(5.0), sy - 3.0, pos(10.0) - pos(5.0), 6.0, Color::srgba(0.12, 0.48, 0.43, 0.55));
    p.rect(pos(10.0), sy - 3.0, pos(15.0) - pos(10.0), 6.0, Color::srgba(0.12, 0.48, 0.43, 0.25));
    p.text(pos(7.5), sy + 18.0, "pros", 12.0, col::DIM, Align::Center);
    p.text(pos(12.5), sy + 18.0, "good", 12.0, col::DIM, Align::Center);
    for t in [0.0, 10.0, 20.0, 30.0, 40.0] {
        p.text(pos(t), sy + 42.0, if t == 40.0 { "40 ms".to_string() } else { format!("{t:.0}") }, 11.0, col::DIM, Align::Center);
    }
    p.rect(pos(rep.spread_ms) - 0.75, sy - 12.0, 1.5, 24.0, col::TEXT);
    p.italic(pos(rep.spread_ms), sy - 20.0, "you", 13.0, col::TEXT, Align::Center);
    if let Some(b) = prev {
        p.disc(pos(b.spread_ms), sy, 4.0, col::BG);
        p.ring(pos(b.spread_ms), sy, 4.0, col::TEXT);
    }
    // histogram of deviations from the global offset
    let (hx, hy, hw, hh) = (x + 587.0, y + 70.0, 543.0, 150.0);
    p.serif(hx, hy, "Where the notes landed", 22.0, col::TEXT, Align::Left);
    let mut bins = [0usize; 24];
    for d in rep.notes.iter().filter_map(|n| n.dev_ms) {
        let b = ((d + 60.0) / 5.0).floor();
        if (0.0..24.0).contains(&b) {
            bins[b as usize] += 1;
        }
    }
    let maxb = (*bins.iter().max().unwrap_or(&1)).max(1);
    let bw = hw / 24.0;
    let base = hy + 30.0 + hh;
    for (i, c) in bins.iter().enumerate() {
        let center = -60.0 + 5.0 * i as f64 + 2.5;
        let hgt = hh * *c as f32 / maxb as f32;
        p.rect(hx + i as f32 * bw + 1.0, base - hgt, bw - 2.0, hgt, dev_color(center));
    }
    p.hline(hx, base, hw, col::TEXT);
    p.text(hx + bw, base + 14.0, "\u{2212}60", 11.0, col::DIM, Align::Center);
    p.text(hx + hw / 2.0, base + 14.0, "0", 11.0, col::DIM, Align::Center);
    p.text(hx + hw - bw, base + 14.0, "+60 ms", 11.0, col::DIM, Align::Center);

    // interval errors, per kind of interval
    let iy = intervals_section(p, r, x, y + 320.0, 1130.0);

    // per-pattern slots (lollipops)
    let py = iy + 30.0;
    p.serif(x, py, "Each sixteenth, pattern by pattern", 22.0, col::TEXT, Align::Left);
    let mut pats: Vec<(String, usize)> = Vec::new();
    for s in &rep.slots {
        match pats.iter_mut().find(|(id, _)| *id == s.pattern) {
            Some(e) => e.1 += s.count,
            None => pats.push((s.pattern.clone(), s.count)),
        }
    }
    pats.sort_by_key(|e| std::cmp::Reverse(e.1));
    pats.truncate(3);
    for (ri, (id, _)) in pats.iter().enumerate() {
        let ry = py + 30.0 + ri as f32 * 120.0;
        p.hline(x, ry, 1130.0, col::LINE);
        let pi = r.run.plan.patterns.iter().position(|q| q.id == *id).unwrap_or(0);
        p.label(x, ry + 20.0, &format!("pattern {}", letter_of(&r.run, pi)));
        p.serif(x, ry + 46.0, fit(&r.run.plan.patterns[pi].name, 250.0, 20.0), 20.0, col::TEXT, Align::Left);
        let (gx, gw) = (x + 260.0, 870.0);
        let mid = ry + 58.0;
        p.hline(gx, mid, gw, col::LINE);
        p.text(gx + gw + 6.0, mid - 26.0, "late", 10.0, col::FAINT, Align::Left);
        p.text(gx + gw + 6.0, mid + 26.0, "early", 10.0, col::FAINT, Align::Left);
        for k in 0..16 {
            let lx = gx + gw * (k as f32 + 0.5) / 16.0;
            let lbl = ["1", "e", "+", "a"][k % 4];
            let lbl = if k % 4 == 0 { format!("{}", k / 4 + 1) } else { lbl.to_string() };
            p.serif(lx, ry + 102.0, lbl, 14.0, if k % 4 == 0 { col::TEXT } else { col::FAINT }, Align::Center);
        }
        for s in rep.slots.iter().filter(|s| s.pattern == *id) {
            let lx = gx + gw * (s.pos16 as f32 + 0.5) / 16.0;
            let hgt = (s.mean_ms * 1.6).clamp(-34.0, 34.0) as f32;
            let c = if s.mean_ms.abs() <= 10.0 && !s.consistent { col::ONTIME } else { dev_color(s.mean_ms) };
            p.rect(lx - 0.5, mid - hgt.max(0.0), 1.0, hgt.abs(), col::DIM);
            p.disc(lx, mid - hgt, 4.0, c);
            if s.mean_ms.abs() >= 8.0 {
                let ty = if hgt > 0.0 { mid - hgt - 12.0 } else { mid - hgt + 12.0 };
                p.text(lx, ty, format!("{:+.0}", s.mean_ms), 11.0, col::DIM, Align::Center);
            }
        }
    }

    py + 30.0 + pats.len().max(1) as f32 * 120.0
}

/// Drift over the whole run: the 11-note average of the deviations from the global offset
/// with a jitter band, section letters (shaded when not A) and pins for the weak windows.
fn run_timeline(p: &mut Paint, r: &ResultsState, x: f32, y: f32) {
    let rep = &r.rep;
    let wy = y;
    p.serif(x, wy, "The whole run", 22.0, col::TEXT, Align::Left);
    p.label_at(x + 1130.0, wy + 3.0, &format!("drift \u{b7} jitter \u{b1}{:.0} ms", rep.jitter_std_ms), Align::Right, col::DIM);
    let (dx, dy, dw, dh) = (x, wy + 34.0, 1130.0, 150.0);
    let pts: Vec<(f64, f64)> = rep.notes.iter().filter_map(|n| n.dev_ms.map(|d| (n.grid_time, d))).collect();
    if pts.len() > 2 {
        let t0 = rep.notes.first().map_or(0.0, |n| n.grid_time);
        let t1 = rep.notes.last().map_or(1.0, |n| n.grid_time).max(t0 + 1e-3);
        let xs = |t: f64| dx + dw * ((t - t0) / (t1 - t0)) as f32;
        let ys = |ms: f64| dy + dh / 2.0 - (ms.clamp(-120.0, 120.0) / 120.0) as f32 * dh / 2.0;
        // section shading (alternate patterns)
        let bar_s = bar_seconds(r.run.plan.bpm);
        let st = r.run.plan.scoring_start() + r.run.grid_offset_s;
        let mut b = 0usize;
        while b < r.run.plan.bar_patterns.len() {
            let pat = r.run.plan.bar_patterns[b];
            let mut e = b;
            while e + 1 < r.run.plan.bar_patterns.len() && r.run.plan.bar_patterns[e + 1] == pat {
                e += 1;
            }
            let (xa, xb) = (xs(st + b as f64 * bar_s), xs(st + (e + 1) as f64 * bar_s));
            if letter_of(&r.run, pat) != 'A' {
                p.rect(xa.max(dx), dy, (xb.min(dx + dw) - xa.max(dx)).max(0.0), dh, Color::srgba(0.86, 0.64, 0.32, 0.12));
            }
            p.italic(xa.max(dx) + 8.0, dy + 12.0, letter_of(&r.run, pat).to_string(), 14.0, col::DIM, Align::Left);
            b = e + 1;
        }
        let half = 5;
        let ma: Vec<f64> = (0..pts.len())
            .map(|i| crate::dsp::mean(&pts[i.saturating_sub(half)..(i + half + 1).min(pts.len())].iter().map(|q| q.1).collect::<Vec<_>>()))
            .collect();
        let j = rep.jitter_std_ms;
        for (i, (t, _)) in pts.iter().enumerate().step_by((pts.len() / 400).max(1)) {
            let (a, bb) = (ys(ma[i] + j), ys(ma[i] - j));
            p.rect(xs(*t), a, 2.0, bb - a, Color::srgba(0.48, 0.42, 0.35, 0.16));
        }
        p.hline(dx, ys(0.0), dw, col::LINE);
        for i in 1..pts.len() {
            p.line(xs(pts[i - 1].0), ys(ma[i - 1]), xs(pts[i].0), ys(ma[i]), col::EARLY);
        }
        for (k, w) in r.worst.iter().enumerate() {
            if let Some(&ni) = w.notes.first() {
                let px = xs(rep.notes[ni].grid_time);
                let mut yy = dy;
                while yy < dy + dh {
                    p.rect(px, yy, 1.0, 3.0, col::OCHRE);
                    yy += 6.0;
                }
                p.text(px + 4.0, dy - 8.0, format!("{}", k + 1), 12.0, col::OCHRE, Align::Left);
            }
        }
        p.text(dx, dy + dh + 16.0, "0:00", 11.0, col::DIM, Align::Left);
        let dur = t1 - t0;
        p.text(dx + dw, dy + dh + 16.0, format!("{}:{:02}", (dur as u32) / 60, (dur as u32) % 60), 11.0, col::DIM, Align::Right);
        p.text(dx + dw + 6.0, ys(120.0) + 6.0, "late", 10.0, col::FAINT, Align::Left);
        p.text(dx + dw + 6.0, ys(-120.0) - 6.0, "early", 10.0, col::FAINT, Align::Left);
    }
}

/// Stroke tone per (pick direction, P.M.) group: every stroke's envelope drawn faintly
/// over the group's median, then the spread of each feature (the group's costliest one in
/// violet). Returns the bottom y.
fn tone_section(p: &mut Paint, r: &ResultsState, x: f32, y: f32) -> f32 {
    use crate::tone::{CAP_MS, Feature};
    let tone = &r.rep.tone;
    section_header(p, x, y, "Tone", "");
    if tone.groups.is_empty() {
        p.text(x, y + 64.0, "\u{2013}", 14.0, col::FAINT, Align::Left);
        return y + 90.0;
    }
    const PW: f32 = 543.0;
    const PH: f32 = 130.0;
    const ROW: f32 = 300.0;
    const TOP_DB: f32 = 6.0;
    const BOT_DB: f32 = -42.0;
    let ms_max = CAP_MS as f32;
    for (gi, g) in tone.groups.iter().enumerate() {
        let gx = x + (gi % 2) as f32 * 587.0;
        let gy = y + 64.0 + (gi / 2) as f32 * ROW;
        let (dir, mute) = (if g.down { "Downstrokes" } else { "Upstrokes" }, if g.pm { "palm-muted" } else { "open" });
        p.serif(gx, gy, dir, 22.0, col::TEXT, Align::Left);
        p.italic(gx + 8.0 + dir.len() as f32 * 10.0, gy + 2.0, mute, 17.0, col::DIM, Align::Left);
        let right = match g.match_db {
            Some(m) => format!("{} strokes \u{b7} envelope \u{b1}{m:.1} dB", g.n),
            None => format!("{} strokes", g.n),
        };
        p.text(gx + PW, gy + 3.0, right, 12.0, col::DIM, Align::Right);
        // Overlay, in dB relative to the median envelope's peak.
        let (top, base) = (gy + 24.0, gy + 24.0 + PH);
        let reference = g.median_env.iter().take(30).copied().fold(f32::MIN, f32::max);
        let xs = |ms: f32| gx + PW * (ms / ms_max).min(1.0);
        let ys = |db: f32| top + PH * ((TOP_DB - (db - reference).clamp(BOT_DB, TOP_DB)) / (TOP_DB - BOT_DB));
        p.rect(gx, top, PW, PH, col::PANEL);
        p.rect(gx, ys(reference) - 0.5, PW, 1.0, col::LINE);
        let hx = xs(g.horizon_ms as f32);
        let mut yy = top;
        while yy < base {
            p.rect(hx - 0.5, yy, 1.0, 3.0, col::FAINT);
            yy += 7.0;
        }
        let faint = col::TONE.with_alpha((6.0 / g.n as f32).clamp(0.04, 0.25));
        const STEP: usize = 3;
        for s in tone.strokes.iter().filter(|s| s.group == gi) {
            let mut prev: Option<(f32, f32)> = None;
            for k in (0..s.env.len()).step_by(STEP) {
                let pt = (xs(k as f32), ys(s.env[k]));
                if let Some(q) = prev {
                    p.line(q.0, q.1, pt.0, pt.1, faint);
                }
                prev = Some(pt);
            }
        }
        for k in (STEP..g.median_env.len()).step_by(STEP) {
            p.line(xs((k - STEP) as f32), ys(g.median_env[k - STEP]), xs(k as f32), ys(g.median_env[k]), col::TEXT);
        }
        for ms in [0.0, 100.0, 200.0, 300.0, 400.0] {
            let lbl = if ms == ms_max { format!("{ms:.0} ms") } else { format!("{ms:.0}") };
            let al = if ms == 0.0 { Align::Left } else if ms == ms_max { Align::Right } else { Align::Center };
            p.text(xs(ms), base + 13.0, lbl, 11.0, col::DIM, al);
        }
        p.text(gx - 6.0, ys(reference), "0", 11.0, col::DIM, Align::Right);
        p.text(gx - 6.0, ys(reference - 30.0), "\u{2212}30", 11.0, col::DIM, Align::Right);
        // Feature spreads.
        let worst = crate::tone::worst_feature(&g.sd).filter(|w| w.2 > 0.1).map(|w| w.0);
        for (fi, f) in Feature::ALL.iter().enumerate() {
            let fx = gx + fi as f32 * 136.0;
            p.label(fx, base + 44.0, f.name());
            let (v, c) = match g.sd_of(*f) {
                Some(sd) => (f.fmt(sd), if worst == Some(*f) { col::TONE } else { col::TEXT }),
                None => ("\u{2013}".into(), col::FAINT),
            };
            p.serif(fx, base + 72.0, v, 24.0, c, Align::Left);
        }
    }
    y + 64.0 + tone.groups.len().div_ceil(2) as f32 * ROW
}

fn draw_calibration(g: &mut Game, p: &mut Paint) {
    let Some(r) = &g.results else { return };
    let rep = &r.rep;
    crate::ui_menu::header(p, "Latency", false);
    if rep.n_matched >= 6 {
        p.serif(720.0, 380.0, format!("{:+.1} ms", rep.latency_ms), 96.0, col::TEXT, Align::Center);
        p.text(720.0, 450.0, format!("from {} of {} clicks \u{b7} evenness {:.1} ms", rep.n_matched, rep.n_grid, rep.spread_ms), 16.0, col::DIM, Align::Center);
        p.italic(720.0, 490.0, "saved", 22.0, col::DIM, Align::Center);
    } else {
        p.italic(720.0, 380.0, "Not enough notes heard", 44.0, col::LATE, Align::Center);
        p.text(720.0, 440.0, format!("{} of {} clicks matched \u{b7} check the input level in Setup", rep.n_matched, rep.n_grid), 16.0, col::DIM, Align::Center);
    }
    let again = p.button(560.0, 560.0, 140.0, 48.0, "Again", false);
    let done = p.button(740.0, 560.0, 140.0, 48.0, "Done", true);
    if again {
        g.retry();
    } else if done {
        g.screen = Screen::Menu;
    }
    let _ = Font::Sans;
}

/// Musical name of a nominal interval in sixteenths (thirds of a sixteenth for triplets).
fn interval_name(sixteenths: f64) -> String {
    let named = [
        (2.0 / 3.0, "16th triplet"),
        (1.0, "16th"),
        (4.0 / 3.0, "8th triplet"),
        (2.0, "8th"),
        (3.0, "dotted 8th"),
        (8.0 / 3.0, "quarter triplet"),
        (4.0, "quarter"),
        (5.0, "quarter + 16th"),
        (6.0, "dotted quarter"),
        (8.0, "half"),
        (12.0, "dotted half"),
        (16.0, "whole"),
    ];
    named
        .iter()
        .find(|(v, _)| (v - sixteenths).abs() < 0.05)
        .map_or_else(|| format!("{} sixteenths", (sixteenths * 100.0).round() / 100.0), |(_, n)| n.to_string())
}

/// Interval errors (actual minus nominal time between consecutive matched notes, ms),
/// grouped by nominal length in sixteenths, shortest first. Intervals across a missed
/// note are left out.
fn interval_groups(rep: &ScoreReport, sixteenth: f64) -> Vec<(f64, Vec<f64>)> {
    let mut groups: Vec<(f64, Vec<f64>)> = Vec::new();
    for w in rep.notes.windows(2) {
        let (Some(a), Some(b)) = (w[0].dev_ms, w[1].dev_ms) else { continue };
        let nominal = ((w[1].grid_time - w[0].grid_time) / sixteenth * 3.0).round() / 3.0;
        match groups.iter_mut().find(|g| (g.0 - nominal).abs() < 1e-6) {
            Some(g) => g.1.push(b - a),
            None => groups.push((nominal, vec![b - a])),
        }
    }
    groups.retain(|g| g.1.len() >= 3);
    groups.sort_by(|a, b| a.0.total_cmp(&b.0));
    groups
}

/// Intervals, one small histogram per nominal interval: how far each played interval was
/// from its correct length (centre line), on a shared ms scale, with the count and the
/// spread (sd); the ochre band is mean +/- sd. Returns the bottom y.
fn intervals_section(p: &mut Paint, r: &ResultsState, x: f32, y: f32, w: f32) -> f32 {
    let rep = &r.rep;
    p.serif(x, y, "Intervals", 22.0, col::TEXT, Align::Left);
    p.label_at(x + w - 64.0, y + 3.0, "evenness", Align::Right, col::DIM);
    p.serif(x + w - 56.0, y, format!("{:.0}", rep.spread_ms), 26.0, col::OCHRE, Align::Left);
    p.text(x + w, y + 4.0, "ms", 13.0, col::DIM, Align::Right);
    let groups = interval_groups(rep, sixteenth_seconds(r.run.plan.bpm));
    if groups.is_empty() {
        p.text(x, y + 40.0, "\u{2013}", 14.0, col::FAINT, Align::Left);
        return y + 60.0;
    }
    // Shared scale: wide enough for the 95th percentile of all errors, 60..120 ms.
    let mut all: Vec<f64> = groups.iter().flat_map(|g| g.1.iter().map(|e| e.abs())).collect();
    all.sort_by(f64::total_cmp);
    let p95 = all[((all.len() - 1) as f64 * 0.95) as usize];
    let range = ((p95 / 20.0).ceil() * 20.0).clamp(60.0, 120.0);
    const BIN: f64 = 5.0;
    let nb = (2.0 * range / BIN) as usize;
    const PER_ROW: usize = 3;
    const GAP: f32 = 40.0;
    const H: f32 = 90.0;
    const ROW: f32 = 170.0;
    let pw = (w - GAP * (PER_ROW - 1) as f32) / PER_ROW as f32;
    for (gi, (nominal, errs)) in groups.iter().enumerate() {
        let gx = x + (gi % PER_ROW) as f32 * (pw + GAP);
        let gy = y + 40.0 + (gi / PER_ROW) as f32 * ROW;
        let (m, sd) = (crate::dsp::mean(errs), crate::dsp::std(errs));
        let name = interval_name(*nominal);
        p.serif(gx, gy, &name, 19.0, col::TEXT, Align::Left);
        p.text(gx + 10.0 + name.len() as f32 * 8.5, gy + 2.0, format!("\u{d7}{}", errs.len()), 12.0, col::DIM, Align::Left);
        p.serif(gx + pw - 22.0, gy, format!("\u{b1}{sd:.0}"), 19.0, col::OCHRE, Align::Right);
        p.text(gx + pw, gy + 2.0, "ms", 12.0, col::DIM, Align::Right);
        let (top, base) = (gy + 18.0, gy + 18.0 + H);
        let xs = |ms: f64| gx + pw * ((ms.clamp(-range, range) + range) / (2.0 * range)) as f32;
        p.rect(xs(m - sd), top, xs(m + sd) - xs(m - sd), H, col::OCHRE_TINT);
        let mut bins = vec![0usize; nb];
        for e in errs {
            bins[((e + range) / BIN).floor().clamp(0.0, nb as f64 - 1.0) as usize] += 1;
        }
        // At least 5 per full bar, so a handful of intervals doesn't look like a lot.
        let maxb = bins.iter().copied().max().unwrap_or(0).max(5);
        let bw = pw / nb as f32;
        for (i, c) in bins.iter().enumerate() {
            if *c > 0 {
                let hgt = (H - 6.0) * *c as f32 / maxb as f32;
                p.rect(gx + i as f32 * bw + 0.5, base - hgt, (bw - 1.0).max(1.0), hgt, col::DIM);
            }
        }
        p.hline(gx, base, pw, col::TEXT);
        p.rect(xs(0.0) - 0.75, top - 4.0, 1.5, H + 4.0, col::TEXT);
        let lbl = |v: f64| if v == 0.0 { "0".to_string() } else { format!("{v:+.0}").replace('-', "\u{2212}") };
        p.text(gx, base + 13.0, lbl(-range), 11.0, col::DIM, Align::Left);
        p.text(xs(0.0), base + 13.0, "0", 11.0, col::DIM, Align::Center);
        p.text(gx + pw, base + 13.0, format!("{} ms", lbl(range)), 11.0, col::DIM, Align::Right);
    }
    y + 40.0 + groups.len().div_ceil(PER_ROW) as f32 * ROW
}
