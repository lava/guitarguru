//! Song suggestions: real songs to try once a milestone is passed. Light inspiration, not
//! a promise that the player can now play them (tempos and tunings differ).
//!
//! An entry is reached by passing a run at `min_bpm` or faster whose ingredients include
//! `needs` (drills don't count), or, for gallop entries, by passing the gallop drill at
//! `min_bpm` or faster. Each suggestion is shown once on the results screen (remembered in
//! `Progress::songs_shown`); the main screen lists every reached one.

use crate::patterns::{LH, Level, LevelKind, PATTERNS, PM, S16, SYNC, TREM};
use crate::progress::{PASS_THRESHOLD, Progress};

pub struct Song {
    pub artist: &'static str,
    pub title: &'static str,
    /// Ingredients the run must have had (bit mask; 0 = basic).
    pub needs: u8,
    /// Reached through the gallop drill instead of a run.
    pub gallop: bool,
    pub min_bpm: u32,
    /// A short hint, or empty.
    pub advice: &'static str,
}

const fn song(artist: &'static str, title: &'static str, needs: u8, min_bpm: u32, advice: &'static str) -> Song {
    Song { artist, title, needs, gallop: false, min_bpm, advice }
}

const fn gallop(artist: &'static str, title: &'static str, min_bpm: u32, advice: &'static str) -> Song {
    Song { artist, title, needs: S16, gallop: true, min_bpm, advice }
}

/// In order of suggestion within each milestone.
pub const SONGS: &[Song] = &[
    // Basic quarters, slow.
    song("Sleep", "Dragonaut", 0, 80, ""),
    song("Electric Wizard", "Funeralopolis", 0, 80, "the opening"),
    song("Gojira", "The Chant", 0, 80, ""),
    // Basic eighths.
    song("Deftones", "My Own Summer (Shove It)", 0, 100, ""),
    song("Ghost", "Square Hammer", 0, 100, ""),
    song("Amon Amarth", "Guardians of Asgaard", 0, 100, ""),
    // Syncopation.
    song("Rammstein", "Du Hast", SYNC, 80, ""),
    song("Rammstein", "Sonne", SYNC, 80, ""),
    song("Spiritbox", "Holy Roller", SYNC, 80, ""),
    song("Knocked Loose", "Counting Worms", SYNC, 80, ""),
    // Palm mutes.
    song("Architects", "Animals", PM, 80, ""),
    song("Sabaton", "Primo Victoria", PM, 80, ""),
    song("Power Trip", "Executioner's Tax (Swing of the Axe)", PM, 80, ""),
    // Gallop drill.
    gallop("Powerwolf", "Army of the Night", 100, "or any Powerwolf song"),
    gallop("Amon Amarth", "The Pursuit of Vikings", 140, ""),
    // Note changes, tremolo.
    song("Mastodon", "Oblivion", LH, 80, "the verse"),
    song("Alcest", "Autre Temps", TREM, 80, ""),
];

impl Song {
    /// "Artist – Title", also the key in `Progress::songs_shown`.
    pub fn label(&self) -> String {
        format!("{} \u{2013} {}", self.artist, self.title)
    }

    /// Does passing `level` reach this entry?
    pub fn reached_by(&self, level: &Level) -> bool {
        if level.bpm < self.min_bpm {
            return false;
        }
        match level.kind {
            LevelKind::Drill(p) => self.gallop && PATTERNS[p].id == "gallop",
            LevelKind::Run(m) => !self.gallop && m & self.needs == self.needs,
        }
    }
}

/// Every entry reached by a passed level, in table order.
pub fn reached(progress: &Progress) -> Vec<&'static Song> {
    let passed: Vec<Level> =
        progress.best.iter().filter(|(_, v)| **v >= PASS_THRESHOLD).filter_map(|(k, _)| Level::from_key(k)).collect();
    SONGS.iter().filter(|s| passed.iter().any(|l| s.reached_by(l))).collect()
}

/// After a pass of `level`: the first entry it reaches that hasn't been suggested yet,
/// marked as shown. One per pass, so suggestions arrive one at a time.
pub fn suggest(progress: &mut Progress, level: &Level) -> Option<&'static Song> {
    let s = SONGS.iter().find(|s| s.reached_by(level) && !progress.songs_shown.contains(&s.label()))?;
    progress.songs_shown.insert(s.label());
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones_and_once_only() {
        let mut p = Progress::default();
        let basic80 = Level::run(0, 80);
        assert_eq!(suggest(&mut p, &basic80).map(|s| s.title), Some("Dragonaut"));
        assert_eq!(suggest(&mut p, &basic80).map(|s| s.title), Some("Funeralopolis"));
        assert_eq!(suggest(&mut p, &basic80).map(|s| s.title), Some("The Chant"));
        assert!(suggest(&mut p, &basic80).is_none(), "each appears once");
        // A palm-mute run at 100 also reaches the basic eighths entries first.
        assert_eq!(suggest(&mut p, &Level::run(PM | SYNC, 100)).map(|s| s.title), Some("My Own Summer (Shove It)"));
        // Drills only reach gallop entries, and only the gallop drill.
        let idx = |id: &str| PATTERNS.iter().position(|q| q.id == id).unwrap();
        assert!(suggest(&mut p, &Level::drill(idx("quarters"), 180)).is_none());
        assert!(suggest(&mut p, &Level::drill(idx("gallop"), 80)).is_none(), "too slow");
        assert_eq!(suggest(&mut p, &Level::drill(idx("gallop"), 140)).map(|s| s.artist), Some("Powerwolf"));
        assert_eq!(suggest(&mut p, &Level::drill(idx("gallop"), 140)).map(|s| s.title), Some("The Pursuit of Vikings"));
        // Reached list from the bests.
        p.best.insert("mix:sync@80".into(), 70.0);
        p.best.insert("mix:pm@80".into(), 40.0);
        let r: Vec<&str> = reached(&p).iter().map(|s| s.title).collect();
        assert!(r.contains(&"Dragonaut") && r.contains(&"Du Hast") && !r.contains(&"Animals"), "{r:?}");
    }
}
