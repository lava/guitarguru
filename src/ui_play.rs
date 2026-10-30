//! Play screen ("Guitar Guru" style). The guru keeps time with a head bob phase-locked to
//! the click (audio clock); the dominant click dot flashes sindoor on beat one, indigo on
//! the others. Below: the current pattern as thin pick arrows (played strokes only) or a
//! tab, with a violet P.M. bracket and an ochre playhead; the next pattern fades in below
//! it during the last two bars before a switch, with a countdown line. No accuracy
//! feedback during play.

use bevy::color::{Alpha, Color};

use crate::game::Game;
use crate::mascot::{self, MascotImages, Mood, STAGE_NAMES};
use crate::paint::{Align, Font, Paint, col, fit};
use crate::patterns::{BEATS_PER_BAR, INGREDIENTS, PATTERNS, Pattern, bar_seconds, beat_seconds, sixteenth_seconds};

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
        p.label_at(946.0, 285.0, &format!("stage {stage} \u{b7} {}", STAGE_NAMES[stage as usize]), Align::Left, col::DIM);

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

        // ---------- current pattern
        p.hline(120.0, 545.0, 1200.0, col::LINE);
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
        p.label_at(120.0, 583.0, &section_letter(&play.plan.bar_patterns, cur), Align::Left, col::DIM);
        let cur_pat = &play.plan.patterns[cur];
        p.serif(141.0, 576.0, fit(&cur_pat.name, 1150.0, 30.0), 30.0, col::TEXT, Align::Left);
        let playhead = if in_count_in { None } else { Some(pos16) };
        let bottom = stroke_diagram(p, cur_pat, 141.0, 600.0, 53.0, 36.0, playhead, pm_on, tab, 1.0);

        // ---------- next pattern: fades in below during the last two bars
        if !waiting {
            let next_switch = (bar_idx.max(0) + 1..i64::from(play.plan.scored_bars)).find(|&b| play.plan.pattern_at(b) != cur);
            if let Some(sb) = next_switch {
                let remain = start + sb as f64 * bar_s - t;
                let lead = 2.0 * bar_s;
                if remain > 0.0 && remain <= lead {
                    let next = play.plan.pattern_at(sb);
                    let f = (remain / lead) as f32;
                    let alpha = ((1.0 - f) * 4.0).clamp(0.0, 1.0);
                    let y = bottom + 18.0;
                    let a = |c: Color| c.with_alpha(alpha);
                    p.label_at(141.0, y, &format!("next \u{b7} {} bars", (remain / bar_s).ceil()), Align::Left, a(col::DIM));
                    let next_pat = &play.plan.patterns[next];
                    p.serif(141.0, y + 26.0, fit(&next_pat.name, 440.0, 22.0), 22.0, a(col::TEXT), Align::Left);
                    let (lx, lw) = (141.0, 400.0);
                    p.rect(lx, y + 44.0, lw, 1.0, a(col::LINE));
                    p.rect(lx + lw * (1.0 - f), y + 43.5, lw * f, 2.0, a(col::OCHRE));
                    // Count above, then small arrows of the next pattern (played strokes only).
                    for k in 0..16 {
                        let c = if k % 4 == 0 { col::DIM } else { col::FAINT };
                        p.serif(600.0 + k as f32 * 26.0, y + 2.0, count_label(k), if k % 4 == 0 { 13.0 } else { 11.0 }, a(c), Align::Center);
                    }
                    for n in &next_pat.hits() {
                        let x = 600.0 + n.pos as f32 * 26.0;
                        p.stroke_arrow(x, y + 30.0, 16.0, n.downstroke(), a(col::DIM));
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

/// One bar on the 16th grid: serif "1 e + a" labels on top, a violet P.M. bracket over
/// palm-muted notes, thin pick arrows for the played strokes (current in ochre); with `tab`
/// six hairline strings with serif fret numbers below. Returns the bottom y.
#[allow(clippy::too_many_arguments)]
pub fn stroke_diagram(p: &mut Paint, pat: &Pattern, x0: f32, top: f32, cell: f32, arrow: f32, playhead: Option<f32>, pm_on: bool, tab: bool, alpha: f32) -> f32 {
    let a = |c: Color| c.with_alpha(alpha);
    let hits = pat.hits();
    // Count labels
    let cur16 = playhead.map(|ph| ph.floor() as i32);
    for k in 0..16 {
        let x = x0 + k as f32 * cell + cell / 2.0;
        let c = if cur16 == Some(k) { col::OCHRE } else if k % 4 == 0 { col::TEXT } else { col::FAINT };
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
    let arrow_y = top + 16.0 + arrow / 2.0;
    let active = |pos: f64, len: f64| playhead.is_some_and(|ph| ph >= pos as f32 && ph < (pos + len) as f32);
    let bottom;
    if tab {
        let names = ["e", "B", "G", "D", "A", "E"];
        let sp = 20.0;
        let s_top = arrow_y + arrow / 2.0 + 16.0;
        let string_y = |s: u8| s_top + f32::from(s - 1) * sp;
        for s in 1..=6u8 {
            p.styled(x0 - 13.0, string_y(s), names[s as usize - 1], 12.0, a(col::FAINT), Align::Center, Font::SerifItalic, 0.0);
            p.rect(x0, string_y(s), 16.0 * cell, 1.0, a(col::DIM));
        }
        let fr = pat.frettings();
        for (n, f) in hits.iter().zip(&fr) {
            let cx = x0 + n.pos as f32 * cell + cell / 2.0;
            let c = if active(n.pos, n.len) { col::OCHRE } else { col::TEXT };
            p.stroke_arrow(cx, arrow_y, arrow * 0.7, n.downstroke(), a(c));
            let mut put = |s: u8, fret: u8| {
                let y = string_y(s);
                let w = if fret >= 10 { 22.0 } else { 14.0 };
                p.rect(cx - w / 2.0, y - 9.0, w, 18.0, a(col::BG));
                p.serif(cx, y - 2.0, format!("{fret}"), 20.0, a(c), Align::Center);
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
            let c = if active(n.pos, n.len) { col::OCHRE } else { col::TEXT };
            p.stroke_arrow(cx, arrow_y, arrow, n.downstroke(), a(c));
        }
        bottom = arrow_y + arrow / 2.0 + 12.0;
    }
    if let Some(ph) = playhead {
        let px = x0 + ph.clamp(0.0, 16.0) * cell;
        p.rect(px - 0.75, label_top + 18.0, 1.5, bottom - label_top - 22.0, a(col::OCHRE));
    }
    bottom
}

/// "1 e + a" label of a sixteenth within the bar.
fn count_label(k: i32) -> String {
    match k % 4 {
        0 => format!("{}", k / 4 + 1),
        1 => "e".into(),
        2 => "+".into(),
        _ => "a".into(),
    }
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
