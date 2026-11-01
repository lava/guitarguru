//! Songs and riffs to try, a list per rung of the tempo ladder, all from practice packs
//! (see `packs`).
//!
//! Every entry is hidden until a single run at exactly its tempo reaches its minimum score:
//! a run whose ingredients include `needs`, or the named drill. Bests are kept per level, so
//! this reads straight off `Progress::best` (no cumulative scoring). Entries revealed but
//! not yet looked at are marked on the main screen (`Progress::songs_shown` holds the
//! looked-at ones).

use crate::patterns::{Level, LevelKind, PATTERNS};
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
    /// The riff to practise, for pack songs that have one (index into `packs::riffs`).
    pub riff: Option<usize>,
}

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

/// The entries of one ladder rung, in pack order.
pub fn at(bpm: u32) -> impl Iterator<Item = &'static Song> {
    crate::packs::songs().iter().filter(move |s| s.bpm == bpm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::{LH, PM, S16};

    fn song(bpm: u32, via: Via) -> Song {
        Song { artist: "A", title: "T", part: "", bpm, via, min: 60.0, riff: None }
    }

    #[test]
    fn one_run_at_exactly_the_tempo() {
        let iron_man = song(80, Via::Run(LH));
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
        let trooper = song(160, Via::Drill("gallop"));
        let gallop = PATTERNS.iter().position(|q| q.id == "gallop").unwrap();
        p.best.insert(Level::run(PM | S16 | LH, 160).key(), 99.0);
        assert!(!trooper.revealed(&p));
        p.best.insert(Level::drill(gallop, 160).key(), 80.0);
        assert!(trooper.revealed(&p));
    }
}
