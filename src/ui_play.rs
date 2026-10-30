//! Play screen ("Guitar Guru" style). The guru keeps time with a head bob phase-locked to
//! the click (audio clock); the dominant click dot flashes sindoor on beat one, indigo on
//! the others. Below: the current pattern as thin pick arrows (played strokes only) or a
//! tab, with a violet P.M. bracket and an ochre playhead, its name to the right; the next
//! pattern comes in below it, at the same size, during the last two bars before a switch,
//! with a countdown line during the very last one. No accuracy feedback during play.

use bevy::color::{Alpha, Color};

use crate::game::Game;
use crate::mascot::{self, MascotImages, Mood};
use crate::paint::{Align, Font, Paint, col, fit};
use crate::patterns::{BEATS_PER_BAR, INGREDIENTS, PATTERNS, Pattern, bar_seconds, beat_seconds, sixteenth_seconds};

/// Bars on the play screen (main and preview): left edge, 16th cell width, arrow length.
const BAR_X: f32 = 141.0;
const BAR_CELL: f32 = 53.0;
const BAR_ARROW: f32 = 36.0;
/// The column right of the bars with the section letter and pattern name.
const NAME_X: f32 = 1030.0;
const NAME_W: f32 = 290.0;
const NAME_SIZE: f32 = 22.0;
/// The preview is drawn a little lighter than the bar being played.
const PREVIEW_ALPHA: f32 = 0.75;

pub fn draw(g: &mut Game, p: &mut Paint, imgs: &MascotImages) {
    let mut end_now = false;
    let mut abort = false;
    {
        let (Some(engine), Some(play)) = (&g.engine, &g.play) else { return };
        let t = play.now(engine);
        let bpm = play.plan.bpm;
        let beat_s = beat_seconds(bpm);
        let bar_s = bar_seconds(bpm);
        let s16 = sixteenth_seconds(bpm);
        let total = play.plan.total_seconds();
        let start = play.plan.scoring_start();
        let waiting = play.waiting();

        // Time left: 4 px ochre bar anchored at the right edge.
        let left = if waiting || t < start { 1.0 } else { (1.0 - (t - start) / (total - start)).clamp(0.0, 1.0) as f32 };
        p.rect(1440.0 * (1.0 - left), 0.0, 1440.0 * left, 4.0, col::OCHRE);

        // Header
        p.serif(120.0, 50.0, format!("{bpm} BPM"), 44.0, col::TEXT, Align::Left);
        let sub: Vec<String> = if play.calibration {
            vec!["latency calibration".into()]
        } else {
            match play.level.kind {
                crate::patterns::LevelKind::Run(m) => {
                    let v: Vec<String> = INGREDIENTS.iter().filter(|i| m & i.bit != 0).map(|i| i.name.to_string()).collect();
                    if v.is_empty() { vec!["basic".into()] } else { v }
                }
                crate::patterns::LevelKind::Drill(pi) => vec![format!("drill \u{b7} {}", PATTERNS[pi].name)],
            }
        };
        p.label_at(284.0, 58.0, &sub.join(" \u{b7} "), Align::Left, col::DIM);
        let bar_no = ((t - start) / bar_s).floor() as i64 + 1;
        let bars = if t < start || waiting { "\u{2013}".to_string() } else { format!("{} / {}", bar_no.min(i64::from(play.plan.scored_bars)), play.plan.scored_bars) };
        let rem = if waiting { (total - start) as u32 } else { (total - t.max(start)).max(0.0) as u32 };
        p.label_at(944.0, 58.0, "bar", Align::Right, col::DIM);
        p.serif(966.0, 54.0, bars, 26.0, col::TEXT, Align::Left);
        p.label_at(1076.0, 58.0, "left", Align::Right, col::DIM);
        p.serif(1097.0, 54.0, format!("{}:{:02}", rem / 60, rem % 60), 26.0, col::TEXT, Align::Left);
        if p.round_button(1238.0, 52.0, 22.0, "x") {
            abort = true;
        }
        if p.round_button(1298.0, 52.0, 22.0, "flag") {
            end_now = true;
        }

        // ---------- the guru keeps time (phase from the audio clock)
        let beats = t / beat_s;
        let running = t >= 0.0 && (waiting || t < total);
        let stage = crate::xp::stage(g.progress.xp);
        let mut pose = mascot::pose(stage, running.then_some(beats));
        pose.guide = true;
        mascot::draw(p, imgs, 266.0, 168.0, 0.95, stage, Mood::Neutral, &pose);
        p.italic(946.0, 276.0, "play any note or melody.", 22.0, col::TEXT, Align::Left);
        p.text(946.0, 306.0, "example: E 2 - 5 - 4 - 2 - 0 - 3", 14.0, col::DIM, Align::Left);

        // ---------- the click dot (dominant), fine double ring
        let (dx, dy, dr) = (720.0, 293.0, 163.0);
        let cur_beat = beats.floor() as i64;
        let since = ((beats - beats.floor()) * beat_s) as f32;
        let flash = if running { flash_level(f64::from(since), beat_s) } else { 0.0 };
        let accent = cur_beat.rem_euclid(BEATS_PER_BAR as i64) == 0;
        let hue = if accent { col::LATE } else { col::EARLY };
        p.ring(dx, dy, dr + 22.0, col::LINE);
        let paper = Color::srgb(0.98, 0.96, 0.925);
        p.disc(dx, dy, dr, mix(paper, hue, flash));
        p.ring(dx, dy, dr, col::TEXT);
        p.ring(dx, dy, dr + 1.0, col::TEXT);
        p.dotted_ring(dx, dy, dr - 11.0, 0.9, if flash > 0.3 { paper } else { col::OCHRE });
        if let Some(gate) = &play.gate {
            p.italic(dx, dy - 22.0, "Play along", 34.0, if flash > 0.5 { paper } else { col::TEXT }, Align::Center);
            for i in 0..crate::countin::NEEDED {
                let cx = dx - 36.0 + i as f32 * 24.0;
                if i < gate.streak() {
                    p.disc(cx, dy + 24.0, 7.0, col::OCHRE);
                }
                p.ring(cx, dy + 24.0, 7.0, if flash > 0.5 { paper } else { col::TEXT });
            }
        }

        // ---------- current pattern, the bar on the left and its name to the right
        // The session keeps recording a short tail after the last bar (see
        // `Game::tick_play`); hold the last bar, playhead at its end, until results show.
        let td = if waiting { t } else { t.min(total) };
        let rel = td - start;
        let in_count_in = waiting || rel < 0.0;
        let last_bar = i64::from(play.plan.scored_bars) - 1;
        let bar_idx = if in_count_in { 0 } else { ((rel / bar_s).floor() as i64).min(last_bar) };
        let bar_t0 = start + bar_idx as f64 * bar_s;
        let pos16 = (((td - bar_t0) / s16) as f32).min(16.0);
        let cur = play.plan.pattern_at(bar_idx);
        let (pm_on, tab) = (play.level.pm_on(), play.level.tab());
        // Main bar and next-pattern preview share one size; the tab view is taller, so
        // its bars start right below the click dot.
        let top = if tab { 492.0 } else { 520.0 };
        let gap = if tab { 30.0 } else { 44.0 };
        let cur_pat = &play.plan.patterns[cur];
        let playhead = if in_count_in { None } else { Some(pos16) };
        let bottom = stroke_diagram(p, cur_pat, BAR_X, top, BAR_CELL, BAR_ARROW, playhead, pm_on, tab, 1.0);
        p.label_at(NAME_X, top + 8.0, &section_letter(&play.plan.bar_patterns, cur), Align::Left, col::DIM);
        pattern_name(p, &cur_pat.name, top + 36.0, col::TEXT);

        // ---------- next pattern: comes in below during the last two bars before a switch
        // (never on the song's last bar); the countdown line runs during the last bar only.
        if !waiting {
            let next_switch = (bar_idx.max(0) + 1..i64::from(play.plan.scored_bars)).find(|&b| play.plan.pattern_at(b) != cur);
            if let Some(sb) = next_switch {
                let remain = start + sb as f64 * bar_s - t;
                let lead = 2.0 * bar_s;
                if remain > 0.0 && remain <= lead {
                    let next_pat = &play.plan.patterns[play.plan.pattern_at(sb)];
                    let fade = (((lead - remain) / (0.5 * bar_s)) as f32).clamp(0.0, 1.0);
                    let alpha = PREVIEW_ALPHA * fade;
                    let y = bottom + gap + (1.0 - fade) * 16.0;
                    stroke_diagram(p, next_pat, BAR_X, y, BAR_CELL, BAR_ARROW, None, pm_on, tab, alpha);
                    let a = |c: Color| c.with_alpha(alpha);
                    p.label_at(NAME_X, y + 8.0, "next", Align::Left, a(col::DIM));
                    let name_bottom = pattern_name(p, &next_pat.name, y + 36.0, a(col::TEXT));
                    if remain <= bar_s {
                        let k = (remain / bar_s) as f32;
                        let lw = NAME_W;
                        let ly = name_bottom + 6.0;
                        p.rect(NAME_X, ly, lw, 1.0, a(col::LINE));
                        p.rect(NAME_X + lw * (1.0 - k), ly - 0.5, lw * k, 2.0, a(col::OCHRE));
                    }
                }
            }
        }
        p.text(1320.0, 880.0, "Enter: end & score \u{b7} Esc: abort", 12.0, col::FAINT, Align::Right);
    }
    if abort {
        g.abort_play();
    } else if end_now {
        g.end_early();
    }
}

/// A pattern name in the name column, wrapped onto up to three lines from `y` (baseline
/// of the first). Returns the y below the last line.
fn pattern_name(p: &mut Paint, name: &str, y: f32, color: Color) -> f32 {
    let lines = wrap(name, NAME_W, NAME_SIZE, 3);
    let lh = NAME_SIZE * 1.15;
    for (i, l) in lines.iter().enumerate() {
        p.serif(NAME_X, y + i as f32 * lh, l.clone(), NAME_SIZE, color, Align::Left);
    }
    y + (lines.len() as f32 - 1.0) * lh + NAME_SIZE * 0.5
}

/// Greedy word wrap with `fit`'s width estimate; the last line is truncated to fit.
fn wrap(s: &str, w: f32, size: f32, max_lines: usize) -> Vec<String> {
    let n = (w / (size * 0.52)).floor().max(2.0) as usize;
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > n {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > max_lines {
        let rest = lines.split_off(max_lines - 1).join(" ");
        lines.push(rest);
    }
    if let Some(last) = lines.last_mut() {
        *last = fit(last, w, size);
    }
    lines
}

/// Section letter (A, B, ...) of a pattern by its first appearance in the run.
fn section_letter(bars: &[usize], pat: usize) -> String {
    let mut seen: Vec<usize> = Vec::new();
    for &b in bars {
        if !seen.contains(&b) {
            seen.push(b);
        }
    }
    let i = seen.iter().position(|&x| x == pat).unwrap_or(0);
    ((b'A' + (i as u8 % 26)) as char).to_string()
}

fn mix(a: Color, b: Color, k: f32) -> Color {
    let (a, b) = (a.to_srgba(), b.to_srgba());
    Color::srgb(a.red + (b.red - a.red) * k, a.green + (b.green - a.green) * k, a.blue + (b.blue - a.blue) * k)
}

/// One bar on the 16th grid: serif "1 + 2 +" counts on top, a violet P.M. bracket over
/// palm-muted notes, thin pick arrows for the played strokes (current in ochre, palm-muted
/// in violet; small "v"/"^" marks in the tab view); with `tab`
/// six hairline strings with serif fret numbers below. Returns the bottom y.
#[allow(clippy::too_many_arguments)]
pub fn stroke_diagram(p: &mut Paint, pat: &Pattern, x0: f32, top: f32, cell: f32, arrow: f32, playhead: Option<f32>, pm_on: bool, tab: bool, alpha: f32) -> f32 {
    let a = |c: Color| c.with_alpha(alpha);
    let hits = pat.hits();
    // Counts on the eighths only (no "e"/"a"); the current eighth's count lights up.
    let cur8 = playhead.map(|ph| (ph / 2.0).floor() as i32);
    for k in (0..16).step_by(2) {
        let x = x0 + k as f32 * cell + cell / 2.0;
        let c = if cur8 == Some(k / 2) { col::OCHRE } else if k % 4 == 0 { col::TEXT } else { col::FAINT };
        p.serif(x, top + 8.0, count_label(k), if k % 4 == 0 { 17.0 } else { 14.0 }, a(c), Align::Center);
    }
    let label_top = top;
    let top = top + 24.0;
    // P.M. brackets
    if pm_on {
        let mut i = 0;
        while i < hits.len() {
            if !hits[i].pm {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < hits.len() && hits[j + 1].pm {
                j += 1;
            }
            let (xa, xb) = (x0 + hits[i].pos as f32 * cell + 2.0, x0 + (hits[j].pos + hits[j].len) as f32 * cell - 8.0);
            p.styled(xa, top - 2.0, "P.M.", 10.0, a(col::TONE), Align::Left, Font::SerifItalic, 0.0);
            p.rect(xa, top + 6.0, xb - xa, 1.0, a(col::TONE));
            p.rect(xb, top + 6.0, 1.0, 6.0, a(col::TONE));
            i = j + 1;
        }
    }
    // The tab view marks pick directions with small "v"/"^" instead of arrows and keeps its
    // rows tight, so that two bars (current and preview) fit below the click dot.
    let mark_h = 10.0;
    let arrow_y = top + 16.0 + if tab { mark_h / 2.0 + 2.0 } else { arrow / 2.0 };
    let active = |pos: f64, len: f64| playhead.is_some_and(|ph| ph >= pos as f32 && ph < (pos + len) as f32);
    // Arrows: ochre while sounding, violet when palm-muted (like the P.M. bracket), else ink.
    let arrow_col = |n: &crate::patterns::BarNote| {
        if active(n.pos, n.len) {
            col::OCHRE
        } else if pm_on && n.pm {
            col::TONE
        } else {
            col::TEXT
        }
    };
    let bottom;
    if tab {
        let names = ["e", "B", "G", "D", "A", "E"];
        let sp = 16.0;
        let s_top = arrow_y + mark_h / 2.0 + 16.0;
        let string_y = |s: u8| s_top + f32::from(s - 1) * sp;
        for s in 1..=6u8 {
            p.styled(x0 - 13.0, string_y(s), names[s as usize - 1], 12.0, a(col::FAINT), Align::Center, Font::SerifItalic, 0.0);
            p.rect(x0, string_y(s), 16.0 * cell, 1.0, a(col::DIM));
        }
        let fr = pat.frettings();
        for (n, f) in hits.iter().zip(&fr) {
            let cx = x0 + n.pos as f32 * cell + cell / 2.0;
            let c = if active(n.pos, n.len) { col::OCHRE } else { col::TEXT };
            p.chevron(cx, arrow_y, 13.0, mark_h, n.downstroke(), a(arrow_col(n)));
            let mut put = |s: u8, fret: u8| {
                let y = string_y(s);
                let w = if fret >= 10 { 22.0 } else { 14.0 };
                p.rect(cx - w / 2.0, y - 8.0, w, 16.0, a(col::BG));
                p.serif(cx, y - 2.0, format!("{fret}"), 18.0, a(c), Align::Center);
            };
            put(f.string, f.fret);
            if f.power {
                put(f.string - 1, f.fret + 2);
            }
        }
        bottom = string_y(6) + 14.0;
    } else {
        for n in &hits {
            let cx = x0 + n.pos as f32 * cell + cell / 2.0;
            p.stroke_arrow(cx, arrow_y, arrow, n.downstroke(), a(arrow_col(n)));
        }
        bottom = arrow_y + arrow / 2.0 + 12.0;
    }
    if let Some(ph) = playhead {
        let px = x0 + ph.clamp(0.0, 16.0) * cell;
        p.rect(px - 0.75, label_top + 18.0, 1.5, bottom - label_top - 22.0, a(col::OCHRE));
    }
    bottom
}

/// Count of an eighth within the bar ("1 + 2 +"), by its sixteenth `k` (even).
fn count_label(k: i32) -> String {
    if k % 4 == 0 { format!("{}", k / 4 + 1) } else { "+".into() }
}

/// Click-dot flash, `since` seconds after a click: full colour for up to 50 ms, then a
/// smooth fade that is over 120 ms later or at 35 % of the beat, whichever comes first.
/// Each flash ends well before the next click, so colours never blend between clicks.
fn flash_level(since: f64, beat_s: f64) -> f32 {
    let end = (0.05 + 0.12f64).min(0.35 * beat_s);
    let hold = 0.05f64.min(0.4 * end);
    if since < 0.0 || since >= end {
        0.0
    } else if since < hold {
        1.0
    } else {
        let x = (since - hold) / (end - hold);
        (1.0 - x * x * (3.0 - 2.0 * x)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_is_short_and_never_overlaps() {
        for bpm in [40.0, 80.0, 120.0, 180.0, 240.0] {
            let beat = 60.0 / bpm;
            assert_eq!(flash_level(0.0, beat), 1.0);
            assert_eq!(flash_level(0.35 * beat, beat), 0.0, "{bpm} BPM: over by 35 % of the beat");
            assert_eq!(flash_level(0.171, beat), 0.0, "{bpm} BPM: over within 170 ms");
        }
        assert_eq!(flash_level(0.045, 0.75), 1.0);
        assert!(flash_level(0.1, 0.75) > 0.1 && flash_level(0.1, 0.75) < 0.9);
    }
}
