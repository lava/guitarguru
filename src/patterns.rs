//! Rhythm patterns as data, and the note grid a play session is scored against.
//!
//! Two notations fill one bar of 4/4:
//! - Stroke grids: `8:` or `16:` followed by one character per eighth / sixteenth of the
//!   strict alternate-picking grid: `D` played downstroke, `U` played upstroke, lowercase
//!   `d`/`u` the same but palm-muted, `-` a stroke the hand makes without hitting the
//!   strings (ghost). E.g. `8:D-DUD-DU`, `16:d-dud-dud-dud-du`.
//! - Durations: `W` whole (16 sixteenths), `H` half (8), `Q` quarter (4), `E` eighth (2),
//!   `S` sixteenth (1), `T` sixteenth-triplet (2/3); a leading `-` makes a rest (`-Q`).
//!   Pick directions follow alternate picking on the 16th grid.

use serde::{Deserialize, Serialize};

pub const BPM_MIN: u32 = 40;
pub const BPM_MAX: u32 = 180;
pub const BPM_STEP: u32 = 20;
pub const BASE_BPM: u32 = 80;
pub const SIXTEENTHS_PER_BAR: f64 = 16.0;
pub const BEATS_PER_BAR: u32 = 4;

pub fn bpm_steps() -> impl Iterator<Item = u32> {
    (BPM_MIN..=BPM_MAX).step_by(BPM_STEP as usize)
}

#[derive(Clone, Debug)]
pub struct Pattern {
    pub id: &'static str,
    pub name: &'static str,
    /// Ingredients this pattern needs (bit mask, see `INGREDIENTS`); 0 = basic.
    pub needs: u8,
    pub notation: &'static str,
    /// Used in runs (false: drill only, e.g. named riffs).
    pub in_tracks: bool,
    /// Fretting per played note, space separated: string letter (`E A D G B e`, low to
    /// high) + fret, `p` suffix for a 5th power chord (root + the next string two frets up),
    /// e.g. `E0 E3 A5p`. None = every note on the open low E.
    pub frets: Option<&'static str>,
    /// For multi-bar riffs: the pattern id of the next bar (drills alternate through them).
    pub then: Option<&'static str>,
}

/// Where a note is played. Strings are numbered 1 (high e) to 6 (low E), E standard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fretting {
    pub string: u8,
    pub fret: u8,
    pub power: bool,
}

impl Fretting {
    pub const OPEN_E: Fretting = Fretting { string: 6, fret: 0, power: false };
    /// MIDI note of the (root) note.
    pub fn midi(&self) -> u8 {
        const OPEN: [u8; 7] = [0, 64, 59, 55, 50, 45, 40];
        OPEN[self.string as usize] + self.fret
    }
    pub fn parse(tok: &str) -> Result<Self, String> {
        let mut chars = tok.chars();
        let string = match chars.next() {
            Some('E') => 6,
            Some('A') => 5,
            Some('D') => 4,
            Some('G') => 3,
            Some('B') => 2,
            Some('e') => 1,
            _ => return Err(format!("bad fretting `{tok}`")),
        };
        let rest: String = chars.collect();
        let (num, power) = rest.strip_suffix('p').map_or((rest.as_str(), false), |n| (n, true));
        let fret = num.parse().map_err(|_| format!("bad fret in `{tok}`"))?;
        if power && string == 1 {
            return Err(format!("no string above for a power chord: `{tok}`"));
        }
        Ok(Self { string, fret, power })
    }
}

/// Mix-in ingredients, in unlock order. A run is (BPM, set of active ingredients); its
/// pattern pool is the basic material plus every pattern whose needs are all active.
/// Add a new ingredient here (next bit) plus patterns that need it.
pub struct Ingredient {
    pub bit: u8,
    pub code: &'static str,
    pub name: &'static str,
    pub about: &'static str,
}

pub const PM: u8 = 1;
pub const S16: u8 = 2;
pub const LH: u8 = 4;
pub const TREM: u8 = 8;

pub const INGREDIENTS: &[Ingredient] = &[
    Ingredient { bit: PM, code: "pm", name: "Palm mutes", about: "the basic rhythms, partly palm-muted" },
    Ingredient { bit: S16, code: "16", name: "16th notes", about: "gallops, reverse gallops, 16ths with gaps, offbeat 16ths" },
    Ingredient { bit: LH, code: "lh", name: "Note changes", about: "low E and A string notes, then power chords (tab view)" },
    Ingredient { bit: TREM, code: "trem", name: "Tremolo", about: "sustained 16th tremolo: bursts and full bars" },
];

/// Mask of the first `n` ingredients.
pub fn first_ingredients(n: usize) -> u8 {
    INGREDIENTS.iter().take(n).fold(0, |m, i| m | i.bit)
}

pub fn mask_name(mask: u8) -> String {
    if mask == 0 {
        return "Basic".into();
    }
    INGREDIENTS.iter().filter(|i| mask & i.bit != 0).map(|i| i.name).collect::<Vec<_>>().join(" + ")
}

const fn pat(id: &'static str, name: &'static str, needs: u8, notation: &'static str) -> Pattern {
    Pattern { id, name, needs, notation, in_tracks: true, frets: None, then: None }
}

/// A pattern with changing notes (left hand).
const fn lh(id: &'static str, name: &'static str, needs: u8, notation: &'static str, frets: &'static str) -> Pattern {
    Pattern { id, name, needs: needs | LH, notation, in_tracks: true, frets: Some(frets), then: None }
}

/// All patterns. Add new ones here. Every run starts with `quarters`; the rest of its
/// pool is ordered by distance from quarters.
pub const PATTERNS: &[Pattern] = &[
    // Basic: the eighth grid (down on the beat, up on the "+"), open.
    pat("quarters", "Quarters", 0, "8:D-D-D-D-"),
    pat("q_pickup", "Quarters + pickup", 0, "8:D-D-D-DU"),
    pat("q_and2", "Quarters + and-of-2", 0, "8:D-DUD-D-"),
    pat("q_rest4", "Quarters, rest on 4", 0, "8:D-D-D---"),
    pat("q_ee", "Quarter, eighth, eighth", 0, "8:D-DUD-DU"),
    pat("ee_q", "Eighth, eighth, quarter", 0, "8:DUD-DUD-"),
    pat("mixed_bar", "Mixed bar", 0, "8:D-DUDUD-"),
    pat("mixed_bar2", "Mixed bar 2", 0, "8:DUD-D-DU"),
    pat("dotted", "Dotted quarter + eighth", 0, "8:D--UD--U"),
    pat("alt_eighths", "Eighths (alternate)", 0, "8:DUDUDUDU"),
    pat("eighths", "Eighths (all down)", 0, "8:DDDDDDDD"),
    pat("skip_beat", "Skip 2 and 4", 0, "8:DU-UDU-U"),
    pat("offbeats", "Offbeats", 0, "8:-U-U-U-U"),
    // Palm mutes: the basic rhythms, partly or fully palm-muted.
    pat("pm_quarters", "P.M. quarters", PM, "8:d-d-d-d-"),
    pat("pm_q_open4", "P.M. quarters, open 4", PM, "8:d-d-d-D-"),
    pat("pm_eighths", "P.M. eighths (all down)", PM, "8:dddddddd"),
    pat("pm_open1", "Open 1, P.M. eighths", PM, "8:Dddddddd"),
    pat("pm_q_ee", "P.M. quarter, eighth, eighth", PM, "8:d-dud-du"),
    pat("pm_ee_q", "P.M. eighth, eighth, quarter", PM, "8:dud-dud-"),
    pat("pm_then_open", "P.M. half, open half", PM, "8:d-dudUD-"),
    pat("pm_alt", "P.M. eighths (alternate)", PM, "8:dudududu"),
    pat("pm_dotted", "P.M. dotted + open pickup", PM, "8:d--Ud--U"),
    // 16th notes. P.M. markings here only apply when palm mutes are active too.
    pat("one_gallop", "One gallop", S16, "16:d-dud---d---d---"),
    pat("gallop_half", "Gallops on 1 and 3", S16, "16:d-dud---d-dud---"),
    pat("gallop", "Gallop", S16, "16:d-dud-dud-dud-du"),
    pat("open_gallop", "Open gallop", S16, "16:D-DUD-DUD-DUD-DU"),
    pat("reverse_gallop", "Reverse gallop", S16, "16:dud-dud-dud-dud-"),
    pat("gallop_reverse", "Gallop + reverse", S16, "16:d-dudud-d-dudud-"),
    pat("sixteenths_eighths", "16ths + eighths", S16, "16:dudud-d-dudud-d-"),
    pat("skip_and", "16ths, skip the +", S16, "16:du-udu-udu-udu-u"),
    pat("offbeat16", "Offbeat 16ths", S16, "16:-u-u-u-u-u-u-u-u"),
    pat("bursts", "Tremolo bursts", S16 | TREM, "16:dudu----dudu----"),
    pat("tremolo", "Tremolo 16ths", S16 | TREM, "16:dudududududududu"),
    // Note changes (E standard, low E and A strings, then power chords).
    lh("lh_two", "Two notes per bar", 0, "8:D-D-D-D-", "E0 E0 E3 E3"),
    lh("lh_e_to_a", "Low E to A", 0, "8:D-D-D-D-", "E0 E0 A0 A0"),
    lh("lh_walk", "Quarter walk", 0, "8:D-D-D-D-", "E0 E2 E3 E5"),
    lh("lh_pedal8", "Pedal eighths", 0, "8:dudududu", "E0 E3 E0 E5 E0 E3 E0 E2"),
    lh("lh_pedal_q_ee", "Pedal quarter-eighths", 0, "8:D-dud-du", "E5 E0 E0 E3 E0 E0"),
    lh("lh_pedal_gallop", "Pedal gallop", S16, "16:D-duD-duD-duD-du", "E3 E0 E0 E5 E0 E0 E3 E0 E0 E2 E0 E0"),
    lh("lh_a_pedal", "A-string pedal", S16, "16:dud-dud-dud-dud-", "A0 A0 A3 A0 A0 A5 A0 A0 A3 A0 A0 A2"),
    lh("lh_tremolo_walk", "Tremolo walk", S16 | TREM, "16:dudududududududu", "E0 E0 E0 E0 E2 E2 E2 E2 E3 E3 E3 E3 E5 E5 E5 E5"),
    lh("lh_chords", "Power chords", 0, "8:D-D-D-D-", "E0p E0p E3p E5p"),
    lh("lh_chords_pm", "P.M. power chords", PM, "8:dddddddd", "E0p E0p E0p E0p E3p E3p E5p E5p"),
    // Drill-only named riffs (Pursuit of Vikings, Amon Amarth; frets as in E standard).
    Pattern { id: "pursuit", name: "Pursuit of Vikings: intro", needs: S16, notation: "16:d-dud-d-dud-dud-", in_tracks: false, frets: None, then: None },
    Pattern {
        id: "pursuit_verse",
        name: "Pursuit of Vikings: verse",
        needs: S16 | LH,
        notation: "16:D-dud-D-D-d-D-d-",
        in_tracks: false,
        frets: Some("A7 E0 E0 E0 A9 A10 E0 A7 E0"),
        then: Some("pursuit_verse_b"),
    },
    Pattern {
        id: "pursuit_verse_b",
        name: "Pursuit of Vikings: verse (bar 2)",
        needs: S16 | LH,
        notation: "16:D-dud-d-dud-dud-",
        in_tracks: false,
        frets: Some("D9 E0 E0 E0 E0 E0 E0 E0 E0 E0 E0"),
        then: Some("pursuit_verse"),
    },
];

/// Patterns only reachable as the later bars of a multi-bar riff (not listed as drills).
pub fn is_riff_continuation(i: usize) -> bool {
    PATTERNS[..i].iter().any(|p| p.then == Some(PATTERNS[i].id))
}

#[cfg(test)]
pub fn pattern_by_id(id: &str) -> Option<&'static Pattern> {
    PATTERNS.iter().find(|p| p.id == id)
}


/// One note of a parsed bar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarNote {
    /// Onset position in sixteenths from the start of the bar.
    pub pos: f64,
    /// Duration in sixteenths.
    pub len: f64,
    pub rest: bool,
    /// Pick direction given by a stroke-grid notation (None: derived from the position).
    #[serde(default)]
    pub down: Option<bool>,
    /// Palm-muted (lowercase stroke in a grid notation).
    #[serde(default)]
    pub pm: bool,
}

/// A stroke of the picking hand: played or not (ghost).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub pos: f64,
    pub down: bool,
    pub played: bool,
    pub pm: bool,
}

fn parse_grid(cells: &str, step: f64) -> Result<Vec<BarNote>, String> {
    let cells: Vec<char> = cells.chars().filter(|c| !c.is_whitespace()).collect();
    if (cells.len() as f64 * step - SIXTEENTHS_PER_BAR).abs() > 1e-6 {
        return Err(format!("stroke grid has {} cells, expected {}", cells.len(), SIXTEENTHS_PER_BAR / step));
    }
    let played: Vec<(usize, bool, bool)> = cells
        .iter()
        .enumerate()
        .filter_map(|(i, c)| match c {
            'D' => Some(Ok((i, true, false))),
            'U' => Some(Ok((i, false, false))),
            'd' => Some(Ok((i, true, true))),
            'u' => Some(Ok((i, false, true))),
            '-' => None,
            _ => Some(Err(format!("unknown stroke `{c}`"))),
        })
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    let first = played.first().map_or(cells.len(), |p| p.0);
    if first > 0 {
        out.push(BarNote { pos: 0.0, len: first as f64 * step, rest: true, down: None, pm: false });
    }
    for (k, &(i, down, pm)) in played.iter().enumerate() {
        let end = played.get(k + 1).map_or(cells.len(), |n| n.0);
        out.push(BarNote { pos: i as f64 * step, len: (end - i) as f64 * step, rest: false, down: Some(down), pm });
    }
    Ok(out)
}

pub fn parse(notation: &str) -> Result<Vec<BarNote>, String> {
    if let Some(g) = notation.strip_prefix("8:") {
        return parse_grid(g, 2.0);
    }
    if let Some(g) = notation.strip_prefix("16:") {
        return parse_grid(g, 1.0);
    }
    let mut pos = 0.0;
    let mut out = Vec::new();
    for tok in notation.split_whitespace() {
        let (rest, sym) = tok.strip_prefix('-').map_or((false, tok), |s| (true, s));
        let len = match sym {
            "W" => 16.0,
            "H" => 8.0,
            "Q" => 4.0,
            "E" => 2.0,
            "S" => 1.0,
            "T" => 2.0 / 3.0,
            _ => return Err(format!("unknown token `{tok}`")),
        };
        out.push(BarNote { pos, len, rest, down: None, pm: false });
        pos += len;
    }
    if (pos - SIXTEENTHS_PER_BAR).abs() > 1e-6 {
        return Err(format!("bar has {pos} sixteenths, expected {SIXTEENTHS_PER_BAR}"));
    }
    Ok(out)
}

impl BarNote {
    /// Pick direction with strict alternate picking on the 16th grid (down on 1/+, up
    /// on e/a); 16th triplets alternate within each triplet group, starting down.
    pub fn downstroke(&self) -> bool {
        if let Some(d) = self.down {
            return d;
        }
        if (self.pos - self.pos.round()).abs() < 1e-6 && (self.len - self.len.round()).abs() < 1e-6 {
            (self.pos.round() as i64) % 2 == 0
        } else {
            ((self.pos * 1.5).round() as i64) % 2 == 0
        }
    }
}

impl Pattern {
    pub fn bar(&self) -> Vec<BarNote> {
        parse(self.notation).expect("built-in pattern must parse")
    }
    /// The picking grid in sixteenths per stroke: 2 for `8:` patterns, 1 for 16th-based
    /// ones, None for triplet patterns (no ghost strokes shown).
    pub fn grid_step(&self) -> Option<f64> {
        if self.notation.starts_with("8:") {
            Some(2.0)
        } else if self.bar().iter().all(|n| (n.pos - n.pos.round()).abs() < 1e-6) {
            Some(1.0)
        } else {
            None
        }
    }

    /// Every stroke of the alternate-picking hand in one bar, played or ghost.
    pub fn strokes(&self) -> Vec<Stroke> {
        let hits = self.hits();
        let Some(step) = self.grid_step() else {
            return hits.iter().map(|n| Stroke { pos: n.pos, down: n.downstroke(), played: true, pm: n.pm }).collect();
        };
        let n = (SIXTEENTHS_PER_BAR / step) as usize;
        (0..n)
            .map(|i| {
                let pos = i as f64 * step;
                match hits.iter().find(|h| (h.pos - pos).abs() < 1e-6) {
                    Some(h) => Stroke { pos, down: h.downstroke(), played: true, pm: h.pm },
                    None => Stroke { pos, down: i % 2 == 0, played: false, pm: false },
                }
            })
            .collect()
    }

    /// How different this pattern is from plain quarters: strokes added or removed on the
    /// 16th grid. Orders a track's progression from simple to varied.
    pub fn distance_from_quarters(&self) -> usize {
        let on: Vec<i64> = self.hits().iter().map(|n| (n.pos * 3.0).round() as i64).collect();
        let quarters = [0i64, 12, 24, 36];
        let added = on.iter().filter(|p| !quarters.contains(p)).count();
        let removed = quarters.iter().filter(|q| !on.contains(q)).count();
        added + removed
    }

    /// Fretting of each played note (defaults to the open low E).
    pub fn frettings(&self) -> Vec<Fretting> {
        let n = self.hits().len();
        match self.frets {
            None => vec![Fretting::OPEN_E; n],
            Some(f) => f.split_whitespace().map(|t| Fretting::parse(t).expect("built-in fretting must parse")).collect(),
        }
    }

    /// Played (non-rest) notes of one bar.
    pub fn hits(&self) -> Vec<BarNote> {
        self.bar().into_iter().filter(|n| !n.rest).collect()
    }
}

/// Bars per pattern segment in a run.
pub const SEGMENT_BARS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LevelKind {
    /// A main run with this set of active ingredients (bit mask).
    Run(u8),
    /// One pattern only (index into `PATTERNS`).
    Drill(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Level {
    pub kind: LevelKind,
    pub bpm: u32,
}

impl Pattern {
    /// Ingredients a drill of this pattern needs: its own needs, plus palm mutes if it
    /// marks any (drills keep their markings).
    pub fn drill_needs(&self) -> u8 {
        self.needs | if self.hits().iter().any(|n| n.pm) { PM } else { 0 }
    }
}

impl Level {
    pub fn run(mask: u8, bpm: u32) -> Self {
        Self { kind: LevelKind::Run(mask), bpm }
    }
    pub fn drill(pattern: usize, bpm: u32) -> Self {
        Self { kind: LevelKind::Drill(pattern), bpm }
    }
    /// The ingredient set this level needs.
    pub fn mask(&self) -> u8 {
        match self.kind {
            LevelKind::Run(m) => m,
            LevelKind::Drill(p) => PATTERNS[p].drill_needs(),
        }
    }
    /// Are palm-mute markings in effect (runs without palm mutes play everything open).
    pub fn pm_on(&self) -> bool {
        self.mask() & PM != 0
    }
    /// Show the tab view (note changes on).
    pub fn tab(&self) -> bool {
        self.mask() & LH != 0
    }
    /// Progress key: `mix:basic@80`, `mix:pm+16@120` for runs; drills keep `pattern@bpm`.
    pub fn key(&self) -> String {
        match self.kind {
            LevelKind::Run(m) => {
                let codes: Vec<&str> = INGREDIENTS.iter().filter(|i| m & i.bit != 0).map(|i| i.code).collect();
                let set = if codes.is_empty() { "basic".to_string() } else { codes.join("+") };
                format!("mix:{set}@{}", self.bpm)
            }
            LevelKind::Drill(p) => format!("{}@{}", PATTERNS[p].id, self.bpm),
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        let (a, b) = key.split_once('@')?;
        let bpm = b.parse().ok()?;
        if let Some(set) = a.strip_prefix("mix:") {
            let mut m = 0;
            if set != "basic" {
                for c in set.split('+') {
                    m |= INGREDIENTS.iter().find(|i| i.code == c)?.bit;
                }
            }
            return Some(Self::run(m, bpm));
        }
        Some(Self::drill(PATTERNS.iter().position(|p| p.id == a)?, bpm))
    }
    pub fn is_run(&self) -> bool {
        matches!(self.kind, LevelKind::Run(_))
    }
    pub fn name(&self) -> String {
        match self.kind {
            LevelKind::Run(m) => mask_name(m),
            LevelKind::Drill(p) => format!("{} drill", PATTERNS[p].name),
        }
    }
    /// Pattern (index into `PATTERNS`) for each scored bar.
    pub fn bar_patterns(&self, bars: u32) -> Vec<usize> {
        match self.kind {
            LevelKind::Drill(p) => {
                // Multi-bar riffs cycle through their bars.
                let mut out = Vec::with_capacity(bars as usize);
                let mut cur = p;
                for _ in 0..bars {
                    out.push(cur);
                    cur = PATTERNS[cur].then.and_then(|id| PATTERNS.iter().position(|q| q.id == id)).unwrap_or(p);
                }
                out
            }
            LevelKind::Run(m) => {
                let order = run_segments(m, bars.div_ceil(SEGMENT_BARS) as usize, &self.key());
                (0..bars).map(|b| order[(b / SEGMENT_BARS) as usize]).collect()
            }
        }
    }
}

/// Seeded segment order for a run. The first segment is always plain quarters. After that
/// the pool (basic material plus patterns whose needs are all active) is walked as a ramp:
/// early segments pick among the patterns closest to quarters (one stroke added or
/// removed), later ones may use anything; unused patterns are preferred, patterns using
/// the active ingredients are favoured in the second half, and the same pattern never
/// plays twice in a row. The seed comes from (BPM, ingredient set), so runs of the same
/// combination are comparable.
pub fn run_segments(mask: u8, segments: usize, key: &str) -> Vec<usize> {
    let quarters = PATTERNS.iter().position(|p| p.id == "quarters").unwrap_or(0);
    let mut pool: Vec<usize> =
        (0..PATTERNS.len()).filter(|&i| PATTERNS[i].in_tracks && PATTERNS[i].needs & !mask == 0 && i != quarters).collect();
    pool.sort_by_key(|&i| (PATTERNS[i].distance_from_quarters(), PATTERNS[i].needs));
    let mut seed = key.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut out = vec![quarters];
    for k in 1..segments {
        let frac = k as f64 / (segments.max(2) - 1) as f64;
        let reach = ((pool.len() as f64 * frac).ceil() as usize).clamp(3.min(pool.len()), pool.len());
        let mut cands: Vec<usize> = pool[..reach].to_vec();
        if mask != 0 && k * 2 >= segments {
            let featured: Vec<usize> = pool.iter().copied().filter(|&i| PATTERNS[i].needs & mask != 0 && !out.contains(&i)).collect();
            if !featured.is_empty() {
                cands = featured;
            }
        }
        let unused: Vec<usize> = cands.iter().copied().filter(|i| !out.contains(i)).collect();
        if !unused.is_empty() {
            cands = unused;
        }
        let last = *out.last().unwrap_or(&quarters);
        if cands.len() > 1 {
            cands.retain(|&i| i != last);
        }
        let pick = if cands.is_empty() { pool[(rnd() % pool.len() as u64) as usize] } else { cands[(rnd() % cands.len() as u64) as usize] };
        out.push(pick);
    }
    out
}

/// A grid note to be played.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct GridNote {
    /// Seconds from the session start (the first count-in click).
    pub time: f64,
    pub bar: u32,
    /// Pattern of this bar (index into `PATTERNS`).
    pub pattern: usize,
    /// Index of the note among the played notes of its pattern's bar (the "slot").
    pub slot: usize,
    /// Marked palm-muted.
    #[serde(default)]
    pub pm: bool,
    /// Where the note is played and its (root) pitch, for later pitch checking.
    pub fretting: Fretting,
    pub midi: u8,
}

#[derive(Clone, Debug)]
pub struct SessionPlan {
    pub bpm: u32,
    pub count_in_bars: u32,
    pub scored_bars: u32,
    /// Pattern per scored bar.
    pub bar_patterns: Vec<usize>,
    pub notes: Vec<GridNote>,
}

impl SessionPlan {
    pub fn new(level: &Level, count_in_bars: u32, scored_seconds: f64) -> Self {
        let scored_bars = ((scored_seconds / bar_seconds(level.bpm)).round() as u32).max(1);
        Self::from_bars(level.bpm, count_in_bars, level.bar_patterns(scored_bars), level.pm_on())
    }

    /// A plan from an explicit pattern per scored bar (e.g. reloaded from a saved run).
    pub fn from_bars(bpm: u32, count_in_bars: u32, bar_patterns: Vec<usize>, pm_on: bool) -> Self {
        let bar_s = bar_seconds(bpm);
        let scored_bars = bar_patterns.len() as u32;
        let s16 = sixteenth_seconds(bpm);
        let mut notes = Vec::new();
        for b in 0..scored_bars {
            let pi = bar_patterns[b as usize];
            let bar_start = f64::from(count_in_bars + b) * bar_s;
            let fr = PATTERNS[pi].frettings();
            for (slot, n) in PATTERNS[pi].hits().iter().enumerate() {
                let fretting = fr.get(slot).copied().unwrap_or(Fretting::OPEN_E);
                let pm = n.pm && pm_on;
                notes.push(GridNote { time: bar_start + n.pos * s16, bar: b, pattern: pi, slot, pm, fretting, midi: fretting.midi() });
            }
        }
        Self { bpm, count_in_bars, scored_bars, bar_patterns, notes }
    }
    /// Pattern of scored bar `b` (count-in bars show the first pattern).
    pub fn pattern_at(&self, bar: i64) -> usize {
        let i = bar.clamp(0, i64::from(self.scored_bars) - 1) as usize;
        self.bar_patterns[i]
    }
    /// Keep only the first `bars` scored bars.
    pub fn truncate(&mut self, bars: u32) {
        self.notes.retain(|n| n.bar < bars);
        self.bar_patterns.truncate(bars as usize);
        self.scored_bars = bars;
    }
    pub fn total_beats(&self) -> u32 {
        (self.count_in_bars + self.scored_bars) * BEATS_PER_BAR
    }
    pub fn total_seconds(&self) -> f64 {
        f64::from(self.total_beats()) * beat_seconds(self.bpm)
    }
    pub fn scoring_start(&self) -> f64 {
        f64::from(self.count_in_bars) * bar_seconds(self.bpm)
    }
}

pub fn beat_seconds(bpm: u32) -> f64 {
    60.0 / f64::from(bpm)
}
pub fn sixteenth_seconds(bpm: u32) -> f64 {
    beat_seconds(bpm) / 4.0
}
pub fn bar_seconds(bpm: u32) -> f64 {
    beat_seconds(bpm) * f64::from(BEATS_PER_BAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_patterns_fill_one_bar() {
        for p in PATTERNS {
            assert!(parse(p.notation).is_ok(), "{}", p.id);
            if let Some(f) = p.frets {
                let n = f.split_whitespace().map(Fretting::parse).collect::<Result<Vec<_>, _>>().unwrap();
                assert_eq!(n.len(), p.hits().len(), "{}: one fretting per played note", p.id);
            }
            if let Some(t) = p.then {
                assert!(PATTERNS.iter().any(|q| q.id == t), "{}: unknown next bar {t}", p.id);
            }
        }
        let verse = PATTERNS.iter().position(|p| p.id == "pursuit_verse").unwrap();
        let bars = Level::drill(verse, 120).bar_patterns(4);
        assert_eq!(bars.iter().map(|&i| PATTERNS[i].id).collect::<Vec<_>>(), ["pursuit_verse", "pursuit_verse_b", "pursuit_verse", "pursuit_verse_b"]);
        assert_eq!(Fretting::parse("A5p").unwrap().midi(), 50);
        assert_eq!(Fretting::OPEN_E.midi(), 40);
        let b = PATTERNS.iter().position(|p| p.id == "pursuit_verse_b").unwrap();
        assert!(is_riff_continuation(b) && !is_riff_continuation(verse));
    }

    #[test]
    fn pursuit_positions() {
        let p = pattern_by_id("pursuit").unwrap();
        let pos: Vec<f64> = p.hits().iter().map(|n| n.pos).collect();
        assert_eq!(pattern_by_id("q_ee").unwrap().hits().iter().map(|n| n.pos).collect::<Vec<_>>(), vec![0.0, 4.0, 6.0, 8.0, 12.0, 14.0]);
        assert_eq!(pos, vec![0.0, 2.0, 3.0, 4.0, 6.0, 8.0, 9.0, 10.0, 12.0, 13.0, 14.0]);
        assert!(p.hits().iter().all(|n| n.pm));
        assert!(!pattern_by_id("quarters").unwrap().hits()[0].pm);
    }

    #[test]
    fn pick_directions() {
        let dirs = |id: &str| pattern_by_id(id).unwrap().hits().iter().map(|n| if n.downstroke() { 'D' } else { 'U' }).collect::<String>();
        assert_eq!(dirs("gallop"), "DDUDDUDDUDDU");
        assert_eq!(dirs("reverse_gallop"), "DUDDUDDUDDUD");
        assert_eq!(dirs("pursuit"), "DDUDDDUDDUD");
        assert_eq!(dirs("q_ee"), "DDUDDU");
        assert_eq!(dirs("offbeats"), "UUUU");
        assert_eq!(dirs("eighths"), "DDDDDDDD");
        let ghosts = |id: &str| pattern_by_id(id).unwrap().strokes().iter().map(|s| match (s.played, s.down) {
            (true, true) => 'D',
            (true, false) => 'U',
            (false, true) => 'd',
            (false, false) => 'u',
        }).collect::<String>();
        assert_eq!(ghosts("dotted"), "DudUDudU");
        assert_eq!(ghosts("gallop"), "DuDUDuDUDuDUDuDU");
        assert_eq!(pattern_by_id("quarters").unwrap().distance_from_quarters(), 0);
        assert_eq!(pattern_by_id("q_pickup").unwrap().distance_from_quarters(), 1);
        assert_eq!(pattern_by_id("offbeats").unwrap().distance_from_quarters(), 8);
    }

    #[test]
    fn run_order_is_seeded_and_mixed() {
        let idx = |id: &str| PATTERNS.iter().position(|p| p.id == id).unwrap();
        let a = run_segments(PM | S16, 8, "mix:pm+16@120");
        assert_eq!(a, run_segments(PM | S16, 8, "mix:pm+16@120"), "same combination, same order");
        assert_ne!(a, run_segments(PM | S16, 8, "mix:pm+16@140"));
        assert!(!a.contains(&idx("pursuit")), "drill-only riffs stay out of runs");
        assert!(a.iter().all(|&p| PATTERNS[p].needs & !(PM | S16) == 0), "only active ingredients");
        assert_eq!(a[0], idx("quarters"), "runs start with quarters");
        assert!(PATTERNS[a[1]].distance_from_quarters() <= 2, "{}", PATTERNS[a[1]].id);
        assert!(a[4..].iter().any(|&p| PATTERNS[p].needs != 0), "ingredients featured later: {a:?}");
        for i in INGREDIENTS {
            assert!(PATTERNS.iter().any(|p| p.needs & i.bit != 0 && p.in_tracks), "{} has patterns", i.name);
        }
        assert!(a.windows(2).all(|w| w[0] != w[1]), "{a:?}");
        let t1 = run_segments(0, 12, "mix:basic@180");
        assert_eq!(t1[0], idx("quarters"));
        assert!(PATTERNS[t1[1]].distance_from_quarters() == 1, "{}", PATTERNS[t1[1]].id);
        assert!(t1.iter().all(|&p| PATTERNS[p].needs == 0));
        assert!(t1.iter().collect::<std::collections::HashSet<_>>().len() >= 8, "varied: {t1:?}");
        let l = Level::run(S16, 100);
        let plan = SessionPlan::new(&l, 2, 120.0);
        assert_eq!(plan.bar_patterns.len(), 50);
        assert!(plan.notes.iter().all(|n| !n.pm), "no palm mutes without the ingredient");
        assert!(plan.bar_patterns.chunks(SEGMENT_BARS as usize).all(|c| c.iter().all(|&p| p == c[0])));
        assert_eq!(Level::from_key(&l.key()), Some(l));
        assert_eq!(l.key(), "mix:16@100");
        assert_eq!(Level::from_key("mix:basic@80"), Some(Level::run(0, 80)));
        assert_eq!(Level::from_key("gallop@80"), Some(Level::drill(idx("gallop"), 80)));
        assert_eq!(PATTERNS[idx("pursuit")].drill_needs(), S16 | PM);
    }

    #[test]
    fn plan_lengths() {
        let p = PATTERNS.iter().position(|p| p.id == "quarters").unwrap();
        let plan = SessionPlan::new(&Level::drill(p, 120), 2, 120.0);
        assert_eq!(plan.scored_bars, 60);
        assert_eq!(plan.notes.len(), 240);
        assert!((plan.notes[0].time - 4.0).abs() < 1e-9);
        assert!((plan.scoring_start() - 4.0).abs() < 1e-9);
    }
}
