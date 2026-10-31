//! Songs and riffs to try, a list per rung of the tempo ladder. Light inspiration, not a
//! promise that the player can now play them (tempos and tunings differ).
//!
//! Every entry is hidden until a single run at exactly its tempo reaches its minimum score:
//! a run whose ingredients include `needs`, or the named drill. Bests are kept per level, so
//! this reads straight off `Progress::best` (no cumulative scoring). Entries revealed but
//! not yet looked at are marked on the main screen (`Progress::songs_shown` holds the
//! looked-at ones).
//!
//! Most tempos come from memory; check them before relying on them.

use crate::patterns::{LH, Level, LevelKind, PATTERNS, PM, S16, SYNC, TREM};
use crate::progress::Progress;

/// What reveals an entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Via {
    /// A run with at least these ingredients (bit mask; 0 = any run).
    Run(u8),
    /// The drill of this pattern id.
    Drill(&'static str),
}

pub struct Song {
    pub artist: &'static str,
    pub title: &'static str,
    /// Which part to play ("the main riff"), or empty.
    pub part: &'static str,
    /// The ladder rung.
    pub bpm: u32,
    pub via: Via,
    /// Minimum combined score of that one run.
    pub min: f64,
}

const B: f64 = 60.0;
const A: f64 = 75.0;
const S: f64 = 90.0;

const fn run(bpm: u32, artist: &'static str, title: &'static str, part: &'static str, needs: u8, min: f64) -> Song {
    Song { artist, title, part, bpm, via: Via::Run(needs), min }
}

const fn drill(bpm: u32, artist: &'static str, title: &'static str, part: &'static str, id: &'static str, min: f64) -> Song {
    Song { artist, title, part, bpm, via: Via::Drill(id), min }
}

pub const SONGS: &[Song] = &[
    // 40: doom, or 16ths in half time.
    run(40, "Electric Wizard", "Funeralopolis", "the opening", LH, B),
    run(40, "Sleep", "Dopesmoker", "the opening riff", LH, B),
    run(40, "Knocked Loose", "Counting Worms", "the closing breakdown", PM | S16 | SYNC, A),
    // 60
    drill(60, "Black Sabbath", "Black Sabbath", "the main riff", "quarters", B),
    run(60, "Sleep", "Dragonaut", "the main riff", 0, B),
    run(60, "Metallica", "The Thing That Should Not Be", "the main riff", PM | S16 | LH, A),
    // 80
    run(80, "Black Sabbath", "Iron Man", "the main riff", LH, B),
    drill(80, "Gojira", "The Chant", "", "alt_eighths", B),
    run(80, "Mastodon", "Oblivion", "the verse", LH, B),
    run(80, "Power Trip", "Executioner's Tax (Swing of the Axe)", "the breakdown", PM, B),
    run(80, "Alcest", "Autre Temps", "", S16 | TREM, B),
    run(80, "Spiritbox", "Holy Roller", "the main riff", SYNC | PM, A),
    run(80, "Metallica", "Sad But True", "the main riff", PM | S16 | LH, A),
    // 100
    drill(100, "Deftones", "My Own Summer (Shove It)", "the main riff", "alt_eighths", B),
    run(100, "Amon Amarth", "Guardians of Asgaard", "", PM | LH, B),
    run(100, "AC/DC", "Back in Black", "the chord riff", SYNC | LH, A),
    run(100, "Architects", "Animals", "the main riff", PM | SYNC, A),
    run(100, "Sepultura", "Roots Bloody Roots", "the main riff", PM | SYNC | LH, A),
    // 120
    run(120, "Rammstein", "Sonne", "the main riff", PM | SYNC, B),
    run(120, "Sabaton", "Primo Victoria", "the verse", PM | LH, B),
    run(120, "Dick Dale", "Misirlou", "at two thirds speed", S16 | TREM | LH, B),
    run(120, "Metallica", "Enter Sandman", "the main riff", PM | LH, A),
    run(120, "Rammstein", "Du Hast", "the main riff", PM | SYNC, A),
    run(120, "Meshuggah", "Bleed", "the main riff", PM | S16 | SYNC, S),
    // 140
    run(140, "Ghost", "Square Hammer", "the verse", LH, B),
    run(140, "Megadeth", "Symphony of Destruction", "the main riff", PM | LH, B),
    run(140, "Metallica", "Seek & Destroy", "the main riff", PM | SYNC | LH, A),
    drill(140, "Amon Amarth", "The Pursuit of Vikings", "the main riff", "gallop", A),
    // 160
    run(160, "Black Sabbath", "Paranoid", "the main riff", PM | LH, B),
    drill(160, "Powerwolf", "Army of the Night", "or any Powerwolf song", "gallop", B),
    drill(160, "Iron Maiden", "The Trooper", "the gallop", "gallop", A),
    run(160, "Amon Amarth", "Twilight of the Thunder God", "the main riff", PM | S16 | LH, A),
    run(160, "Mg\u{142}a", "Exercises in Futility V", "the tremolo riff", S16 | TREM | LH, A),
    // 180
    drill(180, "Iron Maiden", "Run to the Hills", "the verse gallop", "gallop", A),
    run(180, "Power Trip", "Executioner's Tax (Swing of the Axe)", "the main riff", PM | SYNC | LH, A),
    run(180, "Dick Dale", "Misirlou", "the main melody", S16 | TREM | LH, S),
    run(180, "Slayer", "Raining Blood", "the main riff", PM | S16 | LH, S),
];

impl Song {
    /// "Artist – Title".
    pub fn label(&self) -> String {
        format!("{} \u{2013} {}", self.artist, self.title)
    }

    /// Unique key (in `Progress::songs_shown`).
    pub fn key(&self) -> String {
        format!("{}@{} {}", self.label(), self.bpm, self.part)
    }

    /// Does `level` count for this entry (score aside)?
    pub fn counts(&self, level: &Level) -> bool {
        if level.bpm != self.bpm {
            return false;
        }
        match (self.via, level.kind) {
            (Via::Run(needs), LevelKind::Run(m)) => m & needs == needs,
            (Via::Drill(id), LevelKind::Drill(p)) => PATTERNS.get(p).is_some_and(|q| q.id == id),
            _ => false,
        }
    }

    /// Revealed by a single run in the bests?
    pub fn revealed(&self, progress: &Progress) -> bool {
        progress.best.iter().any(|(k, &v)| v >= self.min && Level::from_key(k).is_some_and(|l| self.counts(&l)))
    }
}

/// The entries of one ladder rung, in table order.
pub fn at(bpm: u32) -> impl Iterator<Item = &'static Song> {
    SONGS.iter().filter(move |s| s.bpm == bpm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::{bpm_steps, drills};
    use crate::progress::PASS_THRESHOLD;

    #[test]
    fn table_is_sound() {
        let rungs: Vec<u32> = bpm_steps().collect();
        let mut keys = std::collections::HashSet::new();
        for s in SONGS {
            assert!(rungs.contains(&s.bpm), "{} not on the ladder", s.key());
            assert!(s.min >= PASS_THRESHOLD, "{}", s.key());
            assert!(keys.insert(s.key()), "duplicate {}", s.key());
            match s.via {
                Via::Run(m) => assert!(m & TREM == 0 || m & S16 != 0, "tremolo needs 16ths: {}", s.key()),
                Via::Drill(id) => assert!(drills().iter().any(|&p| PATTERNS[p].id == id), "no drill {id}"),
            }
        }
        for b in rungs {
            assert!(at(b).count() >= 2, "{b} BPM has too few entries");
        }
    }

    #[test]
    fn one_run_at_exactly_the_tempo() {
        let iron_man = SONGS.iter().find(|s| s.title == "Iron Man").unwrap();
        let mut p = Progress::default();
        // Wrong tempo, too low a score, missing ingredient: still hidden.
        p.best.insert(Level::run(LH, 100).key(), 95.0);
        p.best.insert(Level::run(LH, 80).key(), 55.0);
        p.best.insert(Level::run(PM, 80).key(), 95.0);
        assert!(!iron_man.revealed(&p));
        // A superset of the needed ingredients at that tempo counts.
        p.best.insert(Level::run(LH | PM, 80).key(), 61.0);
        assert!(iron_man.revealed(&p));
        // Drills reveal their own entries only.
        let trooper = SONGS.iter().find(|s| s.title == "The Trooper").unwrap();
        let gallop = PATTERNS.iter().position(|q| q.id == "gallop").unwrap();
        p.best.insert(Level::run(PM | S16 | LH, 160).key(), 99.0);
        assert!(!trooper.revealed(&p));
        p.best.insert(Level::drill(gallop, 160).key(), 80.0);
        assert!(trooper.revealed(&p));
    }
}
