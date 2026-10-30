//! Rule-based pattern generator and run sequencing.
//!
//! A generated bar is either a beat-figure composition or a whole-bar idiom:
//! - Beat figures fill one beat each (quarter, 8ths, offbeat, gallop `x-xx`, reverse gallop
//!   `xxx-`, four 16ths, dotted 8th `x--x`, 16th pickup `--xx`, offbeat 16ths `-x-x`, rest),
//!   arranged in a form that repeats: all the same, A-B-A-B, A-A-A-B (variation on 4), A-A-B-A,
//!   or a freer bar that still repeats one figure. The name follows from the form
//!   ("Gallop on 1 & 3", "8ths, rest on 4", "Reverse gallops").
//! - Idioms are fixed bars with their own name (tresillo 3-3-2, 3-3-2 16ths, tremolo figures).
//!
//! Every played note keeps strict alternate picking: the hand moves on the 8th grid (down on
//! the beat, up on the "+") unless the bar has 16th positions, then on the 16th grid (down on
//! beat and "+", up on "e" and "a"). Layers on top, only with their ingredient: palm mutes
//! (mostly muted chugs with a few open accents) and note changes (pedal-tone riffs on the
//! low E or A string, simple power chords, a walking line; frets 0-7, changes timed so the
//! left hand can make them at the run's tempo).
//!
//! A run draws from a pool of curated (`PATTERNS`) and generated bars, following a
//! difficulty curve that depends only on the ingredient set and tempo.

use std::borrow::Cow;
use std::collections::HashSet;

use crate::patterns::{Fretting, LH, PATTERNS, PM, Pattern, S16, SEGMENT_BARS, SYNC, Sequence, TREM, sixteenth_seconds};

// ---------------------------------------------------------------- randomness

/// SplitMix64: small, fast, good enough for musical choices.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    pub fn weighted<T: Copy>(&mut self, items: &[(T, f64)]) -> T {
        let total: f64 = items.iter().map(|i| i.1).sum();
        let mut x = self.unit() * total;
        for &(t, w) in items {
            if x < w {
                return t;
            }
            x -= w;
        }
        items[items.len() - 1].0
    }
}

/// A fresh seed per run (`GG_SEED` overrides it, for reproducible checks).
pub fn random_seed() -> u64 {
    if let Some(s) = std::env::var("GG_SEED").ok().and_then(|v| v.parse().ok()) {
        return s;
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    Rng::new(t ^ (u64::from(std::process::id()) << 32)).next_u64()
}

/// FNV-1a, for seeds derived from text.
pub fn hash_str(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

// ---------------------------------------------------------------- beat figures

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fig {
    Quarter,
    Eighths,
    Offbeat,
    Rest,
    Gallop,
    Reverse,
    Sixteens,
    Dotted,
    Pickup,
    Off16,
}

impl Fig {
    fn cells(self) -> [bool; 4] {
        let s = match self {
            Fig::Quarter => "x---",
            Fig::Eighths => "x-x-",
            Fig::Offbeat => "--x-",
            Fig::Rest => "----",
            Fig::Gallop => "x-xx",
            Fig::Reverse => "xxx-",
            Fig::Sixteens => "xxxx",
            Fig::Dotted => "x--x",
            Fig::Pickup => "--xx",
            Fig::Off16 => "-x-x",
        };
        let mut c = [false; 4];
        for (i, ch) in s.chars().enumerate() {
            c[i] = ch == 'x';
        }
        c
    }
    fn on_beat(self) -> bool {
        self.cells()[0]
    }
    /// Name of one beat of it ("gallop on 1 & 3").
    fn one(self) -> &'static str {
        match self {
            Fig::Quarter => "quarter",
            Fig::Eighths => "8ths",
            Fig::Offbeat => "offbeat",
            Fig::Rest => "rest",
            Fig::Gallop => "gallop",
            Fig::Reverse => "reverse gallop",
            Fig::Sixteens => "16ths",
            Fig::Dotted => "dotted 8th",
            Fig::Pickup => "16th pickup",
            Fig::Off16 => "offbeat 16ths",
        }
    }
    /// Name of a bar made of it.
    fn many(self) -> &'static str {
        match self {
            Fig::Quarter => "Quarters",
            Fig::Eighths => "8ths",
            Fig::Offbeat => "Offbeat 8ths",
            Fig::Rest => "Rests",
            Fig::Gallop => "Gallops",
            Fig::Reverse => "Reverse gallops",
            Fig::Sixteens => "Tremolo",
            Fig::Dotted => "Dotted 8ths",
            Fig::Pickup => "16th pickups",
            Fig::Off16 => "Offbeat 16ths",
        }
    }
}

/// Beat figures with their weights and the ingredients they need. Figures whose beat starts
/// silent (offbeats, 16th pickups, offbeat 16ths) are syncopation.
const FIGS: &[(Fig, f64, u8)] = &[
    (Fig::Quarter, 3.0, 0),
    (Fig::Eighths, 3.0, 0),
    (Fig::Offbeat, 1.2, SYNC),
    (Fig::Gallop, 3.5, S16),
    (Fig::Reverse, 2.5, S16),
    (Fig::Sixteens, 1.2, S16),
    (Fig::Dotted, 1.0, S16),
    (Fig::Pickup, 1.0, S16 | SYNC),
    (Fig::Off16, 0.4, S16 | SYNC),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Uniform,
    Abab,
    Aaab,
    Aaba,
    Free,
}

/// "1", "1 & 3", "2, 3 & 4".
fn beat_list(beats: &[usize]) -> String {
    let b: Vec<String> = beats.iter().map(|i| (i + 1).to_string()).collect();
    match b.len() {
        0 => String::new(),
        1 => b[0].clone(),
        n => format!("{} & {}", b[..n - 1].join(", "), b[n - 1]),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// Readable name of a bar of four beat figures.
fn figure_name(figs: &[Fig; 4]) -> String {
    if figs.iter().all(|&f| f == figs[0]) {
        return figs[0].many().to_string();
    }
    let count = |f: Fig| figs.iter().filter(|&&g| g == f).count();
    // Base: the most frequent figure; ties go to quarters, then 8ths, then the earliest.
    let mut base = figs[0];
    for &f in figs {
        let better = count(f) > count(base)
            || (count(f) == count(base) && f != base && (f == Fig::Quarter || (f == Fig::Eighths && base != Fig::Quarter)));
        if better {
            base = f;
        }
    }
    let mut parts: Vec<String> = Vec::new();
    let others: Vec<Fig> = figs.iter().copied().filter(|&f| f != base).collect();
    let only_rest = others.iter().all(|&f| f == Fig::Rest);
    if base != Fig::Quarter || count(base) > 2 || only_rest {
        parts.push(base.many().to_string());
    }
    let mut done: Vec<Fig> = Vec::new();
    for &f in &others {
        if done.contains(&f) {
            continue;
        }
        done.push(f);
        let beats: Vec<usize> = (0..4).filter(|&i| figs[i] == f).collect();
        parts.push(format!("{} on {}", f.one(), beat_list(&beats)));
    }
    capitalise(&parts.join(", "))
}

/// A generated rhythm before the palm-mute and fretting layers.
struct Rhythm {
    on: [bool; 16],
    name: String,
    /// Beat 1 may be silent (offbeat figures).
    off_start: bool,
}

/// Whole-bar idioms: cells, name, ingredients needed.
const IDIOMS: &[(&str, &str, u8)] = &[
    ("x-----x-----x---", "Tresillo (3-3-2)", SYNC),
    ("x-----x-----x-x-", "Tresillo + 8ths", SYNC),
    ("x---x-x---x-x---", "Push on the + of 3", SYNC),
    ("x-x---x-x---x-x-", "Push on the + of 2", SYNC),
    ("x--x--x-x--x--x-", "3-3-2 16ths", S16 | SYNC),
    ("x--x--x-x-xxx-xx", "3-3-2, then gallops", S16 | SYNC),
    ("x-xxx-xxx--x--x-", "Gallops, then 3-3-2", S16 | SYNC),
    ("xxxxxxxxxxxxxxxx", "Tremolo", S16 | TREM),
    ("xxxxxxxxx---x---", "Tremolo, then quarters", S16 | TREM),
    ("x---x---xxxxxxxx", "Quarters, then tremolo", S16 | TREM),
    ("xxxxxxxxx-xxx-xx", "Tremolo, then gallops", S16 | TREM),
    ("xxxxxxxxxxxx----", "Tremolo, rest on 4", S16 | TREM),
    ("xxxxxxxx--x---x-", "Tremolo, offbeats on 3 & 4", S16 | TREM | SYNC),
];

fn rhythm(rng: &mut Rng, mask: u8) -> Rhythm {
    let s16 = mask & S16 != 0;
    let sync = mask & SYNC != 0;
    let idioms: Vec<(usize, f64)> = IDIOMS
        .iter()
        .enumerate()
        .filter(|(_, (_, _, needs))| needs & !mask == 0)
        .map(|(i, (_, _, needs))| (i, if needs & TREM != 0 { 1.5 } else { 1.0 }))
        .collect();
    if !idioms.is_empty() && rng.chance(if mask & TREM != 0 { 0.3 } else { 0.18 }) {
        let (cells, name, _) = IDIOMS[rng.weighted(&idioms)];
        let mut on = [false; 16];
        for (i, c) in cells.chars().enumerate() {
            on[i] = c == 'x';
        }
        return Rhythm { on, name: name.to_string(), off_start: false };
    }
    let figs: Vec<(Fig, f64)> = FIGS.iter().filter(|f| f.2 & !mask == 0).map(|f| (f.0, f.1)).collect();
    let pick = |rng: &mut Rng, ok: &dyn Fn(Fig) -> bool| -> Fig {
        let c: Vec<(Fig, f64)> = figs.iter().copied().filter(|&(f, _)| ok(f)).collect();
        rng.weighted(&c)
    };
    let form = rng.weighted(&[(Form::Uniform, 1.5), (Form::Abab, 3.0), (Form::Aaab, 3.0), (Form::Aaba, 1.0), (Form::Free, 1.5)]);
    // Without syncopation the only silent beat is beat 4 (`last`).
    let with_rest = |rng: &mut Rng, ok: &dyn Fn(Fig) -> bool, last: bool| if (sync || last) && rng.chance(0.18) { Fig::Rest } else { pick(rng, ok) };
    let bar: [Fig; 4] = match form {
        Form::Uniform => {
            let a = pick(rng, &|_| true);
            [a; 4]
        }
        Form::Abab => {
            let a = pick(rng, &Fig::on_beat);
            let b = pick(rng, &|f| f != a);
            [a, b, a, b]
        }
        Form::Aaab => {
            let a = pick(rng, &Fig::on_beat);
            let b = with_rest(rng, &|f| f != a, true);
            [a, a, a, b]
        }
        Form::Aaba => {
            let a = pick(rng, &Fig::on_beat);
            let b = with_rest(rng, &|f| f != a, false);
            [a, a, b, a]
        }
        Form::Free => {
            let a = pick(rng, &Fig::on_beat);
            let mut bar = [a; 4];
            for (i, slot) in bar.iter_mut().enumerate().skip(1) {
                *slot = if rng.chance(0.45) { a } else { with_rest(rng, &|_| true, i == 3) };
            }
            bar
        }
    };
    let mut on = [false; 16];
    for (b, f) in bar.iter().enumerate() {
        for (i, c) in f.cells().iter().enumerate() {
            on[b * 4 + i] = *c;
        }
    }
    // Readable and idiomatic: one rest at most, two different figures at most (so one repeats).
    let rests = bar.iter().filter(|&&f| f == Fig::Rest).count();
    let mut kinds: Vec<Fig> = bar.to_vec();
    kinds.dedup();
    kinds.sort_by_key(|f| *f as u8);
    kinds.dedup();
    // On the 8th grid a third figure is still easy to read (and widens the small basic pool).
    if rests > 1 || kinds.len() > if s16 { 2 } else { 3 } {
        on = [false; 16]; // rejected by validation
    }
    Rhythm { on, name: figure_name(&bar), off_start: form == Form::Uniform && matches!(bar[0], Fig::Offbeat | Fig::Off16) }
}

// ---------------------------------------------------------------- layers

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PmStyle {
    All,
    Open1,
    Open13,
    Accents,
    Open,
}

/// Palm-mute markings: mostly muted chugs with a few open accents. Returns the suffix.
fn palm_mutes(rng: &mut Rng, on: &[bool; 16], tremolo: bool) -> (Vec<bool>, &'static str) {
    let pos: Vec<usize> = (0..16).filter(|&i| on[i]).collect();
    let style = if tremolo {
        rng.weighted(&[(PmStyle::All, 0.3), (PmStyle::Open, 0.5), (PmStyle::Open1, 0.2)])
    } else {
        rng.weighted(&[(PmStyle::All, 0.35), (PmStyle::Open1, 0.2), (PmStyle::Open13, 0.1), (PmStyle::Accents, 0.2), (PmStyle::Open, 0.15)])
    };
    let syncopated = |p: usize| !p.is_multiple_of(4) && !on[(p / 4 + 1) * 4 % 16];
    let open: Vec<bool> = pos
        .iter()
        .enumerate()
        .map(|(k, &p)| match style {
            PmStyle::All => false,
            PmStyle::Open => true,
            PmStyle::Open1 => p == 0,
            PmStyle::Open13 => p == 0 || p == 8,
            PmStyle::Accents => syncopated(p) || (k == pos.len() - 1 && !pos.iter().any(|&q| syncopated(q))),
        })
        .collect();
    let n_open = open.iter().filter(|&&o| o).count();
    if style == PmStyle::Open {
        return (vec![false; pos.len()], "");
    }
    // Mostly muted: too many accents fall back to all muted.
    if n_open == 0 || n_open * 2 > pos.len() {
        return (vec![true; pos.len()], "P.M.");
    }
    let suffix = match style {
        PmStyle::Open1 => "P.M. + open 1",
        PmStyle::Open13 => "P.M. + open 1 & 3",
        _ => "P.M. + accents",
    };
    (open.iter().map(|o| !o).collect(), suffix)
}

const SCALES: &[&[u8]] = &[&[0, 2, 3, 5, 7], &[0, 1, 3, 5, 6, 7], &[0, 3, 5, 7]];

fn single(string: u8, fret: u8) -> Fretting {
    Fretting { string, fret, power: false }
}

/// Next scale degree near `i`: a step or two up or down, staying in the scale.
fn step(rng: &mut Rng, scale: &[u8], i: usize, nonzero: bool) -> usize {
    for _ in 0..8 {
        let d = rng.weighted(&[(-2i32, 1.0), (-1, 2.0), (1, 2.0), (2, 1.0)]);
        let j = i as i32 + d;
        if j >= 0 && (j as usize) < scale.len() && !(nonzero && scale[j as usize] == 0) {
            return j as usize;
        }
    }
    if nonzero { 1.max(i).min(scale.len() - 1) } else { i }
}

/// Fretting per played note (notes at `pos`, open accents from the palm-mute layer).
fn frettings(rng: &mut Rng, pos: &[usize], accents: &[bool]) -> (Vec<Fretting>, &'static str) {
    let scale = SCALES[rng.below(SCALES.len())];
    let style = rng.weighted(&[(0u8, 0.35), (1, 0.15), (2, 0.3), (3, 0.2)]);
    match style {
        // Pedal tone: open-string chugs with fretted melody notes on accents / beats.
        0 | 1 => {
            let string = if style == 0 { 6 } else { 5 };
            let mut melody: Vec<bool> = if accents.iter().any(|&a| a) && accents.iter().any(|&a| !a) {
                accents.to_vec()
            } else {
                pos.iter().map(|&p| p % 4 == 0 && rng.chance(0.55)).collect()
            };
            if !melody.iter().any(|&m| m) {
                let k = pos.iter().rposition(|&p| p % 4 == 0).unwrap_or(pos.len() - 1);
                melody[k] = true;
            }
            if melody.iter().all(|&m| m) {
                melody[0] = false;
            }
            let mut i = 1 + rng.below(scale.len() - 1);
            let mut out = Vec::new();
            for &m in &melody {
                if m {
                    out.push(single(string, scale[i]));
                    i = step(rng, scale, i, true);
                } else {
                    out.push(single(string, 0));
                }
            }
            (out, if string == 6 { "E pedal" } else { "A pedal" })
        }
        // Power chords: change on beats only, roots on E or A, small moves.
        2 => {
            let mut string = if rng.chance(0.7) { 6 } else { 5 };
            let mut i = rng.below(scale.len());
            let mut last_beat = usize::MAX;
            let mut out = Vec::new();
            for &p in pos {
                let beat = p / 4;
                if beat != last_beat && last_beat != usize::MAX && p % 4 == 0 && rng.chance(0.5) {
                    i = step(rng, scale, i, false);
                    if rng.chance(0.15) {
                        string = 11 - string;
                    }
                }
                last_beat = beat;
                out.push(Fretting { string, fret: scale[i], power: true });
            }
            (out, "power chords")
        }
        // Walking line: a new note on each beat.
        _ => {
            let string = if rng.chance(0.75) { 6 } else { 5 };
            let mut i = rng.below(scale.len());
            let mut last_beat = usize::MAX;
            let mut out = Vec::new();
            for &p in pos {
                let beat = p / 4;
                if beat != last_beat && last_beat != usize::MAX {
                    i = step(rng, scale, i, false);
                }
                last_beat = beat;
                out.push(single(string, scale[i]));
            }
            (out, if string == 6 { "walk on E" } else { "walk on A" })
        }
    }
}

// ---------------------------------------------------------------- analysis

/// Positions (in sixteenths) of the played notes.
fn positions(p: &Pattern) -> Vec<usize> {
    p.hits().iter().map(|n| n.pos.round() as usize).collect()
}

/// Longest run of notes a 16th apart (the bar repeats).
fn max_run16(pos: &[usize]) -> usize {
    if pos.len() == 16 {
        return 16;
    }
    let on: Vec<bool> = (0..16).map(|i| pos.contains(&i)).collect();
    let mut best = 0;
    for start in 0..16 {
        if on[start] && !on[(start + 15) % 16] {
            let mut n = 0;
            while n < 16 && on[(start + n) % 16] {
                n += 1;
            }
            best = best.max(n);
        }
    }
    best
}

/// Seconds the left hand needs to go from `a` to `b`.
fn change_time(a: Fretting, b: Fretting) -> f64 {
    if a == b {
        return 0.0;
    }
    if a.power || b.power {
        let small = a.power == b.power && a.string == b.string && a.fret.abs_diff(b.fret) <= 2;
        return if small { 0.12 } else { 0.16 };
    }
    if a.fret == 0 || b.fret == 0 {
        return if a.string == b.string { 0.08 } else { 0.10 };
    }
    if a.string == b.string && a.fret.abs_diff(b.fret) <= 3 { 0.12 } else { 0.18 }
}

/// Ingredients a pattern actually uses (its declared needs plus what its notes show).
pub fn features(p: &Pattern) -> u8 {
    let hits = p.hits();
    let pos = positions(p);
    let mut f = p.needs;
    if hits.iter().any(|h| h.pm) {
        f |= PM;
    }
    if pos.iter().any(|q| q % 2 == 1) {
        f |= S16;
    }
    if max_run16(&pos) >= 8 {
        f |= TREM | S16;
    }
    if p.frettings().iter().any(|x| *x != Fretting::OPEN_E) {
        f |= LH;
    }
    if syncopated(&pos) {
        f |= SYNC;
    }
    f
}

/// Syncopation: a beat with notes but a silent downbeat (offbeat- or upstroke-led, pushes,
/// 3-3-2), a silent beat 1, or a lone 16th on the "a" ringing into the next beat
/// (anticipation; the end of a 16th burst before a rest doesn't count). Plain bars with a rest on 4 or a pickup on the "+" of 4 are not
/// syncopated. (Rests on beats 2 and 3 aren't either, but basic bars avoid them anyway.)
pub fn syncopated(pos: &[usize]) -> bool {
    let on = |q: usize| pos.contains(&(q % 16));
    let offbeat_led = (0..4).any(|b| !on(b * 4) && (1..4).any(|i| on(b * 4 + i)));
    let anticipation = pos.iter().any(|&q| q % 4 == 3 && !on(q + 1) && !on(q - 1));
    offbeat_led || !on(0) || anticipation
}

/// Can the bar be played at this tempo with these ingredients? Caps 16th runs (tremolo
/// needs its ingredient; above 150 BPM, without tremolo, at most 4 notes a 16th apart and
/// 8 such notes per bar) and checks every fret change has the time it needs.
pub fn playable(p: &Pattern, bpm: u32, mask: u8) -> bool {
    let pos = positions(p);
    if pos.is_empty() {
        return false;
    }
    let s16 = sixteenth_seconds(bpm);
    let fast = bpm > 150;
    let run = max_run16(&pos);
    let n = pos.len();
    let ioi = |k: usize| (pos[(k + 1) % n] + if k + 1 == n { 16 } else { 0 } - pos[k]) as f64;
    if mask & TREM == 0 {
        let limit = if fast { 4 } else { 5 };
        if run > limit {
            return false;
        }
        if fast && (0..n).filter(|&k| ioi(k) < 1.5).count() > 8 {
            return false;
        }
    }
    let fr = p.frettings();
    (0..n).all(|k| change_time(fr[k], fr[(k + 1) % n]) <= ioi(k) * s16 + 1e-9)
}

/// Difficulty of one bar at a tempo: note density, syncopations, upstrokes, notes on the
/// "e"/"a", palm-mute switches, string/fret changes and shifts, and speed (notes closer
/// than 0.2 s add load, so 16ths cost more the faster the run). Quarters ~0.6, offbeat
/// 8ths ~3, gallops at 120 ~5, a tremolo bar at 120 ~9.
pub fn difficulty(p: &Pattern, bpm: u32, pm_on: bool) -> f64 {
    let hits = p.hits();
    let pos = positions(p);
    let n = pos.len();
    if n == 0 {
        return 0.0;
    }
    let on = |q: usize| pos.contains(&(q % 16));
    let sync = pos.iter().filter(|&&q| q % 4 != 0 && !on((q / 4 + 1) * 4)).count();
    let up = hits.iter().filter(|h| !h.downstroke()).count();
    let odd = pos.iter().filter(|&&q| q % 2 == 1).count();
    let pm_switch = if pm_on { (0..n).filter(|&k| hits[k].pm != hits[(k + 1) % n].pm).count() } else { 0 };
    let fr = p.frettings();
    let (mut frets, mut strings, mut shifts) = (0, 0, 0);
    for k in 0..n {
        let (a, b) = (fr[k], fr[(k + 1) % n]);
        if a == b {
            continue;
        }
        if a.string != b.string {
            strings += 1;
        } else if a.fret != 0 && b.fret != 0 && a.fret.abs_diff(b.fret) > 3 {
            shifts += 1;
        } else {
            frets += 1;
        }
    }
    let s16 = sixteenth_seconds(bpm);
    let speed: f64 = (0..n)
        .map(|k| {
            let gap = (pos[(k + 1) % n] + if k + 1 == n { 16 } else { 0 } - pos[k]) as f64 * s16;
            ((0.2 - gap) / 0.1).max(0.0)
        })
        .sum();
    0.15 * n as f64
        + 0.5 * sync as f64
        + 0.1 * up as f64
        + 0.25 * odd as f64
        + 0.2 * pm_switch as f64
        + 0.25 * frets as f64
        + 0.45 * strings as f64
        + 0.6 * shifts as f64
        + 0.3 * speed
}

// ---------------------------------------------------------------- generation

/// One generated bar for this ingredient set, or None if the attempt broke a rule.
pub fn generate(rng: &mut Rng, mask: u8, bpm: u32) -> Option<Pattern> {
    let r = rhythm(rng, mask);
    let on = r.on;
    let pos: Vec<usize> = (0..16).filter(|&i| on[i]).collect();
    // Shape rules: enough notes, beat 1 played (unless an offbeat figure), no long silence.
    if pos.len() < 3 || (!on[0] && !(r.off_start && mask & SYNC != 0)) {
        return None;
    }
    let gap = (0..pos.len()).map(|k| (pos[(k + 1) % pos.len()] + 16 - pos[k] - 1) % 16 + 1).max().unwrap_or(16);
    if gap > 8 {
        return None;
    }
    let tremolo = max_run16(&pos) >= 8;
    // Name: the rhythm plus one tag (the fretting style if any, else the palm-mute style).
    let name = r.name;
    let (pm, pm_name) = if mask & PM != 0 { palm_mutes(rng, &on, tremolo) } else { (vec![false; pos.len()], "") };
    let accents: Vec<bool> = pm.iter().map(|m| !m).collect();
    let mut frets = None;
    if mask & LH != 0 && rng.chance(0.8) {
        for _ in 0..4 {
            let (fr, lh_name) = frettings(rng, &pos, &accents);
            let cand = build(&on, &pm, Some(&fr), &name, mask);
            if playable(&cand, bpm, mask) && fr.iter().any(|f| *f != Fretting::OPEN_E) {
                frets = Some((fr, lh_name));
                break;
            }
        }
    }
    let p = match &frets {
        Some((fr, lh_name)) => build(&on, &pm, Some(fr), &format!("{name}, {lh_name}"), mask),
        None if !pm_name.is_empty() => build(&on, &pm, None, &format!("{name}, {pm_name}"), mask),
        None => build(&on, &pm, None, &name, mask),
    };
    (playable(&p, bpm, mask) && features(&p) & !mask == 0).then_some(p)
}

/// The pattern for played cells `on` with palm mutes and frettings per played note.
fn build(on: &[bool; 16], pm: &[bool], frets: Option<&[Fretting]>, name: &str, mask: u8) -> Pattern {
    let eighths = (0..16).all(|i| i % 2 == 0 || !on[i]);
    let (step, prefix) = if eighths { (2, "8:") } else { (1, "16:") };
    let mut notation = String::from(prefix);
    let mut k = 0;
    for cell in 0..16 / step {
        let i = cell * step;
        if on[i] {
            let c = if cell % 2 == 0 { 'D' } else { 'U' };
            notation.push(if pm[k] { c.to_ascii_lowercase() } else { c });
            k += 1;
        } else {
            notation.push('-');
        }
    }
    let frets = frets.map(|f| f.iter().map(Fretting::token).collect::<Vec<_>>().join(" "));
    let id = match &frets {
        Some(f) => format!("g:{notation}|{f}"),
        None => format!("g:{notation}"),
    };
    let mut p = Pattern {
        id: Cow::Owned(id),
        name: Cow::Owned(name.to_string()),
        needs: 0,
        notation: Cow::Owned(notation),
        in_tracks: true,
        frets: frets.map(Cow::Owned),
        then: None,
    };
    p.needs = features(&p) & mask;
    p
}

// ---------------------------------------------------------------- pool and sequencing

/// A pattern a run can use, with its difficulty at the run's tempo.
#[derive(Clone, Debug)]
pub struct Entry {
    pub pattern: Pattern,
    pub difficulty: f64,
    /// Uses one of the run's active ingredients (other than palm mutes, if there are others).
    pub featured: bool,
}

/// Generation attempts per pool (valid, distinct results are fewer).
pub const POOL_ATTEMPTS: usize = 400;

/// Material for a run: the curated patterns with these ingredients plus generated bars,
/// deduplicated, all playable at `bpm`. Plain quarters (the opener) is left out.
pub fn pool(mask: u8, bpm: u32, seed: u64, attempts: usize) -> Vec<Entry> {
    let pm_on = mask & PM != 0;
    // The ingredients that make a bar "featured": palm mutes only when they are the only one
    // (with more, nearly every bar is muted, so the others tell bars apart).
    let main = if mask & !PM != 0 { mask & !PM } else { mask };
    // Patterns sound the same if their notation (with P.M. only where it applies) and frets match.
    let key = |p: &Pattern| {
        let n = if pm_on { p.notation.to_string() } else { p.notation.to_uppercase() };
        (n, p.frets.as_ref().map(ToString::to_string))
    };
    let mut seen: HashSet<(String, Option<String>)> = HashSet::new();
    let quarters = PATTERNS.iter().find(|p| p.id == "quarters").expect("quarters pattern");
    seen.insert(key(quarters));
    seen.insert(("8:D-D-D-D-".into(), None));
    let mut out = Vec::new();
    let mut add = |p: Pattern, out: &mut Vec<Entry>| {
        if playable(&p, bpm, mask) && seen.insert(key(&p)) {
            let difficulty = difficulty(&p, bpm, pm_on);
            let featured = features(&p) & main != 0;
            out.push(Entry { pattern: p, difficulty, featured });
        }
    };
    for p in PATTERNS.iter().filter(|p| p.in_tracks && p.needs & !mask == 0) {
        add(p.clone(), &mut out);
    }
    let mut rng = Rng::new(seed);
    for _ in 0..attempts {
        if let Some(p) = generate(&mut rng, mask, bpm) {
            add(p, &mut out);
        }
    }
    out
}

/// Number of segments of a run of `bars` bars.
pub fn segments(bars: u32) -> usize {
    bars.div_ceil(SEGMENT_BARS) as usize
}

/// Target difficulty per segment. Depends only on the ingredient set and tempo (taken from
/// a reference pool seeded by them), so every run of a combination climbs the same curve:
/// segment 0 is quarters, then a rise from the easy end of the pool (12th percentile) to
/// its hard end (85th), with a breather (back to about a third of the climb) every fourth
/// segment except the last.
pub fn curve(mask: u8, bpm: u32, segs: usize) -> Vec<f64> {
    let reference = pool(mask, bpm, hash_str(&format!("curve:{mask}@{bpm}")), POOL_ATTEMPTS);
    let mut d: Vec<f64> = reference.iter().map(|e| e.difficulty).collect();
    d.sort_by(f64::total_cmp);
    let pct = |q: f64| if d.is_empty() { 1.0 } else { d[((d.len() - 1) as f64 * q).round() as usize] };
    let (lo, hi) = (pct(0.12), pct(0.85));
    let quarters = PATTERNS.iter().find(|p| p.id == "quarters").map_or(0.6, |q| difficulty(q, bpm, false));
    (0..segs)
        .map(|k| {
            if k == 0 {
                return quarters;
            }
            let frac = if segs <= 2 { 1.0 } else { (k - 1) as f64 / (segs - 2) as f64 };
            let ramp = lo + (hi - lo) * frac;
            if k % 4 == 3 && k + 1 < segs { lo + 0.35 * (ramp - lo) } else { ramp }
        })
        .collect()
}

/// The patterns of a run: 8 bars of quarters, then one pattern per 8-bar segment drawn from
/// the pool near the curve's target. Never the same pattern twice in a row; patterns already
/// played this run or in `avoid` (recent runs of the same combination) are passed over while
/// others are close; patterns with the active ingredients are favoured in the second half
/// (and held back a little in the first).
pub fn run_sequence(mask: u8, bpm: u32, bars: u32, seed: u64, avoid: &HashSet<String>) -> Sequence {
    let segs = segments(bars).max(1);
    let targets = curve(mask, bpm, segs);
    let pool = pool(mask, bpm, seed, POOL_ATTEMPTS);
    let mut rng = Rng::new(seed ^ 0x05ee_d5e9u64);
    let quarters = PATTERNS.iter().find(|p| p.id == "quarters").expect("quarters pattern").clone();
    let mut chosen: Vec<usize> = Vec::new(); // pool indices, segments 1..
    // Error diffusion: how far the picks so far are above their targets. The next target is
    // lowered by it, so every run's mean difficulty stays close to the curve's (fair
    // comparison between runs of one combination) while the single picks stay random.
    let mut carry = 0.0f64;
    for (k, &curve_target) in targets.iter().enumerate().skip(1) {
        let target = curve_target - carry.clamp(-1.0, 1.0);
        let second_half = k * 2 >= segs;
        let mut scored: Vec<(f64, usize)> = pool
            .iter()
            .enumerate()
            .filter(|(i, _)| chosen.last() != Some(i) || pool.len() == 1)
            .map(|(i, e)| {
                let mut cost = (e.difficulty - target).abs();
                if mask != 0 {
                    cost += match (second_half, e.featured) {
                        (true, false) => 0.8,
                        (false, true) => 0.3,
                        _ => 0.0,
                    };
                }
                if avoid.contains(&*e.pattern.id) {
                    cost += 2.5;
                }
                if chosen.contains(&i) {
                    cost += 4.0;
                }
                (cost, i)
            })
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        if scored.is_empty() {
            break;
        }
        // Anything about as close as the best (within 0.4), at least the 4 closest.
        let best = scored[0].0;
        let near = scored.iter().take_while(|c| c.0 <= best + 0.4).count().max(4).min(scored.len());
        let pick = scored[rng.below(near)].1;
        carry += pool[pick].difficulty - curve_target;
        chosen.push(pick);
    }
    let mut patterns = vec![quarters];
    let mut seg_index = vec![0];
    for &i in &chosen {
        let p = &pool[i].pattern;
        let idx = patterns.iter().position(|q| q.id == p.id).unwrap_or_else(|| {
            patterns.push(p.clone());
            patterns.len() - 1
        });
        seg_index.push(idx);
    }
    let bar_patterns = (0..bars).map(|b| seg_index[((b / SEGMENT_BARS) as usize).min(seg_index.len() - 1)]).collect();
    Sequence { patterns, bar_patterns, seed: Some(seed) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::{BPM_MAX, BPM_MIN, bpm_steps, parse};

    const MASKS: [u8; 13] = [
        0,
        SYNC,
        PM,
        S16,
        PM | S16,
        SYNC | S16,
        SYNC | PM | S16,
        LH,
        PM | LH,
        PM | S16 | LH,
        S16 | TREM,
        PM | S16 | LH | TREM,
        SYNC | PM | S16 | LH | TREM,
    ];

    fn samples(mask: u8, bpm: u32, n: usize) -> Vec<Pattern> {
        let mut rng = Rng::new(hash_str(&format!("{mask}:{bpm}")));
        (0..n).filter_map(|_| generate(&mut rng, mask, bpm)).collect()
    }

    #[test]
    fn runs_of_a_combination_are_equally_hard() {
        for mask in MASKS {
            for bpm in [60, 140, 180] {
                let bars = crate::patterns::scored_bars(bpm, 120.0);
                let avg = |s: &Sequence| s.bar_patterns.iter().map(|&i| difficulty(&s.patterns[i], bpm, mask & PM != 0)).sum::<f64>() / f64::from(bars);
                let first = run_sequence(mask, bpm, bars, 0, &HashSet::new());
                let avoid: HashSet<String> = first.patterns.iter().map(|p| p.id.to_string()).collect();
                let none = HashSet::new();
                let v: Vec<f64> = (0..12).map(|seed| avg(&run_sequence(mask, bpm, bars, seed, if seed % 2 == 0 { &avoid } else { &none }))).collect();
                let m = v.iter().sum::<f64>() / v.len() as f64;
                // Small pools at slow tempos have fewer difficulty steps to pick from.
                let tol = if bpm < 100 { 0.15 } else { 0.12 };
                assert!(v.iter().all(|d| (d / m - 1.0).abs() < tol), "mask {mask} @{bpm}: {v:.2?}");
            }
        }
    }

    /// `cargo test --release sample_runs -- --ignored --nocapture`: pool sizes, examples
    /// and a few drawn runs per ingredient set, for eyeballing the material.
    #[test]
    #[ignore = "prints sample material"]
    fn sample_runs() {
        for mask in MASKS {
            let bpm = 120;
            let p = pool(mask, bpm, 3, POOL_ATTEMPTS);
            let generated = p.iter().filter(|e| e.pattern.id.starts_with("g:")).count();
            println!("\n== {} @{bpm}: pool {} ({generated} generated)", crate::patterns::mask_name(mask), p.len());
            let mut rng = Rng::new(9);
            for _ in 0..8 {
                let e = &p[rng.below(p.len())];
                println!("  {:5.2}  {:<20} {:<28} {}", e.difficulty, e.pattern.notation, e.pattern.frets.as_deref().unwrap_or(""), e.pattern.name);
            }
            let bars = crate::patterns::scored_bars(bpm, 120.0);
            for seed in [1, 2] {
                let s = run_sequence(mask, bpm, bars, seed, &HashSet::new());
                println!("  run seed {seed}:");
                for &i in s.bar_patterns.iter().step_by(SEGMENT_BARS as usize) {
                    let q = &s.patterns[i];
                    println!("    {:5.2}  {:<20} {:<28} {}", difficulty(q, bpm, mask & PM != 0), q.notation, q.frets.as_deref().unwrap_or(""), q.name);
                }
            }
        }
    }

    #[test]
    fn generated_bars_follow_the_rules() {
        for mask in MASKS {
            for bpm in [BPM_MIN, 100, 140, BPM_MAX] {
                let pats = samples(mask, bpm, 300);
                assert!(pats.len() > 100, "mask {mask} @{bpm}: only {} valid", pats.len());
                for p in &pats {
                    let what = format!("{} `{}` mask {mask} @{bpm}", p.name, p.notation);
                    // One bar on a strict alternate-picking grid.
                    let (step, cells) = if let Some(c) = p.notation.strip_prefix("8:") { (2, c) } else { (1, p.notation.strip_prefix("16:").expect("grid")) };
                    assert_eq!(cells.len() * step, 16, "{what}");
                    for (i, c) in cells.chars().enumerate() {
                        match c {
                            'D' | 'd' => assert!(i % 2 == 0, "downstroke off the grid: {what}"),
                            'U' | 'u' => assert!(i % 2 == 1, "upstroke off the grid: {what}"),
                            '-' => {}
                            _ => panic!("{what}"),
                        }
                    }
                    assert!(parse(&p.notation).is_ok(), "{what}");
                    let hits = p.hits();
                    assert_eq!(p.frettings().len(), hits.len(), "{what}");
                    // Ingredient gating.
                    let pos = positions(p);
                    if mask & S16 == 0 {
                        assert!(p.notation.starts_with("8:") && pos.iter().all(|q| q % 2 == 0), "16ths without the ingredient: {what}");
                    }
                    if mask & PM == 0 {
                        assert!(!p.notation.chars().any(|c| c.is_lowercase()), "P.M. without the ingredient: {what}");
                    } else if hits.iter().any(|h| h.pm) {
                        assert!(hits.iter().filter(|h| h.pm).count() * 2 >= hits.len(), "mostly muted: {what}");
                    }
                    if mask & LH == 0 {
                        assert!(p.frets.is_none(), "{what}");
                    }
                    for f in p.frettings() {
                        assert!((f.string == 5 || f.string == 6) && f.fret <= 7, "{what}");
                    }
                    if mask & TREM == 0 {
                        assert!(max_run16(&pos) <= 5, "tremolo without the ingredient: {what}");
                    }
                    if mask & SYNC == 0 {
                        assert!(!syncopated(&pos), "syncopation without the ingredient: {what}");
                        // Every beat starts with a downstroke; only beat 4 may rest.
                        for b in 0..3 {
                            assert!(pos.contains(&(b * 4)), "beat {} silent: {what}", b + 1);
                        }
                        assert!(pos.iter().all(|&q| q % 4 == 0 || pos.contains(&(q / 4 * 4))), "{what}");
                    }
                    // Musical shape and playability.
                    assert!(pos.contains(&0) || p.name.starts_with("Offbeat"), "beat 1: {what}");
                    assert!(pos.len() >= 3, "{what}");
                    assert!(playable(p, bpm, mask), "{what}");
                    assert_eq!(features(p) & !mask, 0, "{what}");
                    assert!(!p.name.is_empty() && p.name.len() <= 72, "{what}");
                }
            }
        }
    }

    #[test]
    fn fast_tempos_cap_sixteenths() {
        for p in samples(PM | S16 | LH, BPM_MAX, 400) {
            assert!(max_run16(&positions(&p)) <= 4, "{} {}", p.name, p.notation);
        }
        let tremolo = PATTERNS.iter().find(|p| p.id == "tremolo").unwrap();
        assert!(playable(tremolo, BPM_MAX, S16 | TREM));
        assert!(!playable(tremolo, 100, S16));
        // A position shift on a 16th is not playable fast; on a quarter it is.
        let mut p = tremolo.clone();
        p.notation = "16:D-DUD-D-D-D-D-D-".into();
        p.frets = Some("E0 E2 E7 E7 E7 E7 E7 E7 E7".into());
        assert!(!playable(&p, 160, S16 | LH));
        p.notation = "8:D-D-D-D-".into();
        p.frets = Some("E2 E7 E2 E7".into());
        assert!(playable(&p, 160, LH));
    }

    #[test]
    fn syncopation_is_recognised() {
        let sync = |n: &str| syncopated(&positions(&Pattern { notation: n.to_string().into(), ..PATTERNS[0].clone() }));
        for plain in ["8:D-D-D-D-", "8:DUDUDU--", "8:D-D-D-DU", "8:D-DUD-D-", "16:D-DUD-DUD-DUD-DU", "16:DUD-DUD-DUD-DUD-", "16:D--UD--UD--UD---"] {
            assert!(!sync(plain), "{plain}");
        }
        for s in ["8:-U-U-U-U", "8:D--U--D-", "8:DU-UD-DU", "16:D--U--D-D--U--D-", "16:D---D---D-----DU", "16:D--U----D-D-D-D-"] {
            assert!(sync(s), "{s}");
        }
        // Curated patterns declare it.
        for p in PATTERNS.iter().filter(|p| p.in_tracks) {
            assert_eq!(features(p) & SYNC, p.needs & SYNC, "{}", p.id);
        }
    }

    #[test]
    fn names_read_well() {
        use Fig::*;
        assert_eq!(figure_name(&[Gallop, Quarter, Gallop, Quarter]), "Gallop on 1 & 3");
        assert_eq!(figure_name(&[Offbeat; 4]), "Offbeat 8ths");
        assert_eq!(figure_name(&[Quarter, Quarter, Quarter, Rest]), "Quarters, rest on 4");
        assert_eq!(figure_name(&[Eighths, Eighths, Eighths, Gallop]), "8ths, gallop on 4");
        assert_eq!(figure_name(&[Reverse, Eighths, Reverse, Eighths]), "8ths, reverse gallop on 1 & 3");
        assert_eq!(figure_name(&[Quarter, Quarter, Quarter, Pickup]), "Quarters, 16th pickup on 4");
    }

    #[test]
    fn difficulty_orders_familiar_patterns() {
        let d = |id: &str, bpm: u32| difficulty(PATTERNS.iter().find(|p| p.id == id).unwrap(), bpm, true);
        assert!(d("quarters", 120) < d("q_ee", 120));
        assert!(d("q_ee", 120) < d("offbeats", 120));
        assert!(d("alt_eighths", 120) < d("gallop", 120));
        assert!(d("gallop", 120) < d("tremolo", 120));
        assert!(d("gallop", 80) < d("gallop", 180), "faster is harder");
        assert!(d("lh_two", 120) > d("quarters", 120), "note changes count");
    }

    #[test]
    fn pools_are_large_and_varied() {
        for mask in MASKS {
            let p = pool(mask, 120, 7, POOL_ATTEMPTS);
            let ids: HashSet<&str> = p.iter().map(|e| &*e.pattern.id).collect();
            assert_eq!(ids.len(), p.len(), "distinct ids");
            let min = match mask {
                0 => 12,
                SYNC | PM => 30,
                _ => 60,
            };
            assert!(p.len() >= min, "mask {mask}: {}", p.len());
            assert!(p.iter().any(|e| e.featured) || mask == 0);
        }
    }

    #[test]
    fn sequences_ramp_vary_and_avoid_repeats() {
        for mask in MASKS {
            for bpm in bpm_steps() {
                let bars = crate::patterns::scored_bars(bpm, 120.0);
                let segs = segments(bars);
                let a = run_sequence(mask, bpm, bars, 1, &HashSet::new());
                let b = run_sequence(mask, bpm, bars, 2, &HashSet::new());
                assert_eq!(a, run_sequence(mask, bpm, bars, 1, &HashSet::new()), "same seed, same run");
                assert_eq!(a.bar_patterns.len(), bars as usize);
                for s in [&a, &b] {
                    assert!(s.bar_patterns[..SEGMENT_BARS.min(bars) as usize].iter().all(|&i| s.patterns[i].id == "quarters"), "starts with quarters");
                    let seg: Vec<usize> = s.bar_patterns.iter().step_by(SEGMENT_BARS as usize).copied().collect();
                    assert_eq!(seg.len(), segs);
                    assert!(seg.windows(2).all(|w| w[0] != w[1]), "no immediate repeats: mask {mask} @{bpm}");
                    assert!(s.patterns.iter().all(|p| features(p) & !mask == 0 || PATTERNS.iter().any(|q| q.id == p.id)), "ingredients");
                }
                if segs > 2 {
                    let ids = |s: &Sequence| s.bar_patterns.iter().map(|&i| s.patterns[i].id.to_string()).collect::<Vec<_>>();
                    let distinct: HashSet<Vec<String>> = (10..16).map(|seed| ids(&run_sequence(mask, bpm, bars, seed, &HashSet::new()))).collect();
                    assert!(distinct.len() >= 5, "seeds differ: mask {mask} @{bpm}: {}", distinct.len());
                }
            }
        }
    }

    #[test]
    fn difficulty_rises_over_the_run() {
        for mask in [0, PM | S16, PM | S16 | LH | TREM] {
            let bpm = 140;
            let bars = crate::patterns::scored_bars(bpm, 120.0);
            let segs = segments(bars);
            let c = curve(mask, bpm, segs);
            assert_eq!(c, curve(mask, bpm, segs), "the curve is fixed per combination");
            assert!(c[segs - 1] > 1.5 * c[1], "{c:?}");
            assert!((4..segs - 1).any(|k| c[k] < c[k - 1]), "a breather: {c:?}");
            // Averaged over seeds, the second half is harder than the first.
            let (mut first, mut second) = (0.0, 0.0);
            for seed in 0..20 {
                let s = run_sequence(mask, bpm, bars, seed, &HashSet::new());
                let d: Vec<f64> = s.bar_patterns.iter().step_by(SEGMENT_BARS as usize).map(|&i| difficulty(&s.patterns[i], bpm, mask & PM != 0)).collect();
                first += d[1..segs / 2].iter().sum::<f64>() / (segs / 2 - 1) as f64;
                second += d[segs / 2..].iter().sum::<f64>() / (segs - segs / 2) as f64;
            }
            // The basic pool is deliberately plain, so its ramp is flatter.
            assert!(second > if mask == 0 { 1.1 } else { 1.2 } * first, "mask {mask}: {first} vs {second}");
        }
    }

    #[test]
    fn recent_patterns_are_avoided_and_featured_later() {
        let (mask, bpm) = (PM | S16, 120);
        let bars = crate::patterns::scored_bars(bpm, 120.0);
        let a = run_sequence(mask, bpm, bars, 11, &HashSet::new());
        let avoid: HashSet<String> = a.patterns.iter().filter(|p| p.id != "quarters").map(|p| p.id.to_string()).collect();
        let b = run_sequence(mask, bpm, bars, 11, &avoid);
        let again = b.patterns.iter().filter(|p| avoid.contains(&*p.id)).count();
        assert_eq!(again, 0, "with a large pool nothing recent comes back");
        let segs = segments(bars);
        let seg: Vec<&Pattern> = a.bar_patterns.iter().step_by(SEGMENT_BARS as usize).map(|&i| &a.patterns[i]).collect();
        let feat = |ps: &[&Pattern]| ps.iter().filter(|p| features(p) & mask != 0).count() as f64 / ps.len() as f64;
        assert!(feat(&seg[segs / 2..]) >= 0.75, "ingredients in the second half");
        let _ = BPM_MAX;
    }
}
