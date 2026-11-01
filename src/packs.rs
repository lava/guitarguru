//! Practice packs: song lists the player adds, each with unlock requirements like the
//! built-in songs and optionally a riff to practise with a backing track. Nothing from a
//! pack ships with the app; packs live in `<data dir>/packs/<name>/pack.json`.
//!
//! ```json
//! {
//!   "name": "My pack",
//!   "songs": [
//!     {
//!       "id": "first-riff",
//!       "artist": "Some Band", "title": "Some Song", "part": "the main riff",
//!       "unlock": { "bpm": 80, "needs": ["pm"], "min": 60 },
//!       "riff": {
//!         "bpm": 84,
//!         "bars": [ { "notation": "16:d-d-D---d-d-D---", "frets": "E0 E0 E3p E0 E0 A2p" } ],
//!         "backing": { "generate": ["drums", "bass"], "gain_db": -6 }
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! - `unlock.bpm` is the tempo-ladder step the song is listed under; `needs` are
//!   ingredient codes (`sync`, `pm`, `16`, `lh`, `trem`), or `"drill": "<pattern id>"`
//!   instead; `min` is the minimum score of one run there.
//! - `riff.bpm` is the riff's own tempo; `bars` use the pattern notation (see `patterns`).
//! - `riff.backing` is either a WAV file next to `pack.json` (`"backing": "riff.wav"`,
//!   starting on the riff's first downbeat and as long as one pass of the riff; it loops)
//!   or parts the app synthesises from the riff (`drums`, `bass`, `guitar`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::patterns::{Fretting, INGREDIENTS, PATTERNS, Pattern, SessionPlan, parse};
use crate::songs::{Song, Via};

/// Passes of the riff in one practice loop.
pub const RIFF_REPS: u32 = 4;

#[derive(Deserialize)]
struct PackFile {
    #[serde(default)]
    name: String,
    songs: Vec<SongFile>,
}

#[derive(Deserialize)]
struct SongFile {
    id: String,
    artist: String,
    title: String,
    #[serde(default)]
    part: String,
    unlock: UnlockFile,
    #[serde(default)]
    riff: Option<RiffFile>,
}

#[derive(Deserialize)]
struct UnlockFile {
    bpm: u32,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    drill: Option<String>,
    min: f64,
}

#[derive(Deserialize)]
struct RiffFile {
    bpm: u32,
    bars: Vec<BarFile>,
    #[serde(default)]
    backing: Option<BackingFile>,
}

#[derive(Deserialize)]
struct BarFile {
    notation: String,
    #[serde(default)]
    frets: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BackingFile {
    Wav(String),
    Generate {
        generate: Vec<String>,
        #[serde(default)]
        gain_db: Option<f32>,
    },
}

/// A synthesised backing part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Drums,
    Bass,
    Guitar,
}

#[derive(Clone, Debug)]
pub enum Backing {
    Wav(PathBuf),
    Generate { parts: Vec<Part>, gain_db: f32 },
}

/// A riff to practise: one pass of its bars, at its own tempo.
#[derive(Debug)]
pub struct Riff {
    /// `<pack dir>/<song id>`, unique; the progress key is `riff:<id>@<bpm>`.
    pub id: String,
    pub title: String,
    pub bpm: u32,
    /// One pattern per bar.
    pub bars: Vec<Pattern>,
    pub backing: Option<Backing>,
}

struct Loaded {
    songs: Vec<Song>,
    riffs: Vec<Riff>,
}

/// The loaded packs. Each (re)load leaks a fresh table so songs and riffs can be handed
/// out as `'static`; reloads only happen on import, so the leak stays small.
static LOADED: RwLock<Option<&'static Loaded>> = RwLock::new(None);

pub fn packs_dir() -> PathBuf {
    crate::progress::data_dir().join("packs")
}

fn loaded() -> &'static Loaded {
    if let Some(l) = *LOADED.read().unwrap() {
        return l;
    }
    load_all()
}

/// Read every pack again (after an import).
pub fn reload() {
    load_all();
}

fn load_all() -> &'static Loaded {
    let mut l = Loaded { songs: Vec::new(), riffs: Vec::new() };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(packs_dir()).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.join("pack.json").is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    dirs.sort();
    for d in dirs {
        if let Err(e) = load_pack(&d, &mut l) {
            eprintln!("pack {}: {e}", d.display());
        }
    }
    let l: &'static Loaded = Box::leak(Box::new(l));
    *LOADED.write().unwrap() = Some(l);
    l
}

/// Import a dropped pack: a folder with a `pack.json`, the `pack.json` itself, or a
/// `.zip` / `.tar` / `.tar.gz` / `.tgz` archive holding one (at its root or in a single
/// top-level folder). The pack is copied into the packs directory, replacing one of the
/// same name, and the packs are reloaded. Returns the pack's name.
pub fn import(dropped: &Path) -> Result<String, String> {
    let name = import_into(dropped, &packs_dir())?;
    reload();
    Ok(name)
}

fn import_into(dropped: &Path, packs: &Path) -> Result<String, String> {
    let file = dropped.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let archive = [".zip", ".tar", ".tar.gz", ".tgz"].iter().find(|e| file.ends_with(*e));
    let name = if let Some(ext) = archive {
        let tmp = packs.join(".importing");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let res = extract(dropped, &tmp).and_then(|()| {
            let stem = file.strip_suffix(*ext).unwrap_or(&file).to_string();
            let (root, name) = pack_root(&tmp, stem)?;
            install(&root, &name, packs)
        });
        let _ = std::fs::remove_dir_all(&tmp);
        res?
    } else {
        let src = if dropped.is_dir() { dropped.to_path_buf() } else { dropped.parent().map(Path::to_path_buf).unwrap_or_default() };
        let name = src.file_name().map(|n| n.to_string_lossy().to_string()).ok_or("bad pack folder")?;
        install(&src, &name, packs)?
    };
    Ok(name)
}

/// Unpack an archive into `dest` (entries escaping `dest` are rejected by both readers).
fn extract(archive: &Path, dest: &Path) -> Result<(), String> {
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let name = archive.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        zip::ZipArchive::new(f).and_then(|mut z| z.extract(dest)).map_err(|e| format!("zip: {e}"))
    } else if name.ends_with(".tar") {
        tar::Archive::new(f).unpack(dest).map_err(|e| format!("tar: {e}"))
    } else {
        tar::Archive::new(flate2::read::GzDecoder::new(f)).unpack(dest).map_err(|e| format!("tar.gz: {e}"))
    }
}

/// The folder holding `pack.json` in an unpacked archive and the pack's folder name: the
/// archive root (named after the archive) or its single top-level folder.
fn pack_root(dir: &Path, stem: String) -> Result<(PathBuf, String), String> {
    if dir.join("pack.json").is_file() {
        return Ok((dir.to_path_buf(), stem));
    }
    let subs: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    match subs.as_slice() {
        [one] if one.join("pack.json").is_file() => Ok((one.clone(), one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(stem))),
        _ => Err("not a practice pack (no pack.json)".into()),
    }
}

/// Check `src/pack.json` parses and copy `src` to `<packs>/<name>`.
fn install(src: &Path, name: &str, packs: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(src.join("pack.json")).map_err(|_| "not a practice pack (no pack.json)".to_string())?;
    let pack: PackFile = serde_json::from_str(&text).map_err(|e| format!("pack.json: {e}"))?;
    if name.is_empty() || name.starts_with('.') {
        return Err("bad pack name".into());
    }
    let dest = packs.join(name);
    if dest != src {
        let _ = std::fs::remove_dir_all(&dest);
        copy_dir(src, &dest).map_err(|e| format!("copying the pack: {e}"))?;
    }
    Ok(if pack.name.is_empty() { name.to_string() } else { pack.name })
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dest.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

/// Songs from all packs.
pub fn songs() -> &'static [Song] {
    &loaded().songs
}

pub fn riffs() -> &'static [Riff] {
    &loaded().riffs
}

pub fn riff_by_id(id: &str) -> Option<usize> {
    riffs().iter().position(|r| r.id == id)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn load_pack(dir: &Path, l: &mut Loaded) -> Result<(), String> {
    let text = std::fs::read_to_string(dir.join("pack.json")).map_err(|e| e.to_string())?;
    let pack: PackFile = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let dir_name = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().to_string());
    for s in pack.songs {
        match song(dir, &dir_name, s, l.riffs.len()) {
            Ok((song, riff)) => {
                l.riffs.extend(riff);
                l.songs.push(song);
            }
            Err(e) => eprintln!("pack {dir_name}: {e}"),
        }
    }
    Ok(())
}

fn song(dir: &Path, pack: &str, s: SongFile, next_riff: usize) -> Result<(Song, Option<Riff>), String> {
    let via = match &s.unlock.drill {
        Some(id) => {
            let p = PATTERNS.iter().find(|p| p.id == id.as_str()).ok_or_else(|| format!("{}: unknown drill `{id}`", s.id))?;
            Via::Drill(leak(p.id.to_string()))
        }
        None => {
            let mut m = 0;
            for c in &s.unlock.needs {
                m |= INGREDIENTS.iter().find(|i| i.code == c).ok_or_else(|| format!("{}: unknown ingredient `{c}`", s.id))?.bit;
            }
            Via::Run(m)
        }
    };
    let riff = s.riff.map(|r| riff(dir, pack, &s.id, &s.title, r)).transpose()?;
    let song = Song {
        artist: leak(s.artist),
        title: leak(s.title),
        part: leak(s.part),
        bpm: s.unlock.bpm,
        via,
        min: s.unlock.min,
        riff: riff.as_ref().map(|_| next_riff),
    };
    Ok((song, riff))
}

fn riff(dir: &Path, pack: &str, id: &str, title: &str, r: RiffFile) -> Result<Riff, String> {
    if r.bars.is_empty() {
        return Err(format!("{id}: riff has no bars"));
    }
    let mut bars = Vec::new();
    for (i, b) in r.bars.into_iter().enumerate() {
        let hits = parse(&b.notation).map_err(|e| format!("{id} bar {}: {e}", i + 1))?.into_iter().filter(|n| !n.rest).count();
        if let Some(f) = &b.frets {
            let toks: Vec<&str> = f.split_whitespace().collect();
            if toks.len() != hits {
                return Err(format!("{id} bar {}: {} frettings for {hits} notes", i + 1, toks.len()));
            }
            for t in toks {
                Fretting::parse(t).map_err(|e| format!("{id} bar {}: {e}", i + 1))?;
            }
        }
        bars.push(Pattern {
            id: format!("{id}-{}", i + 1).into(),
            name: format!("bar {}", i + 1).into(),
            needs: 0,
            notation: b.notation.into(),
            frets: b.frets.map(Into::into),
        });
    }
    let backing = r.backing.map(|b| match b {
        BackingFile::Wav(f) => Backing::Wav(dir.join(f)),
        BackingFile::Generate { generate, gain_db } => Backing::Generate {
            parts: generate
                .iter()
                .filter_map(|p| match p.as_str() {
                    "drums" => Some(Part::Drums),
                    "bass" => Some(Part::Bass),
                    "guitar" => Some(Part::Guitar),
                    _ => {
                        eprintln!("{id}: unknown backing part `{p}`");
                        None
                    }
                })
                .collect(),
            gain_db: gain_db.unwrap_or(-6.0),
        },
    });
    Ok(Riff { id: format!("{pack}/{id}"), title: title.to_string(), bpm: r.bpm, bars, backing })
}

impl Riff {
    /// Ingredients the riff's markings use: palm mutes when it marks any, note changes
    /// (the tab view) when it has frettings.
    pub fn mask(&self) -> u8 {
        let pm = self.bars.iter().any(|b| b.hits().iter().any(|n| n.pm));
        let lh = self.bars.iter().any(|b| b.frets.is_some());
        (if pm { crate::patterns::PM } else { 0 }) | if lh { crate::patterns::LH } else { 0 }
    }

    /// One pass of the backing at 48 kHz (looped by the player), or None.
    pub fn backing_samples(&self) -> Option<Arc<Vec<f32>>> {
        let sr = crate::audio::SAMPLE_RATE;
        let pass = SessionPlan::from_sequence(self.bpm, 0, self.sequence(1), true);
        let len = (pass.total_seconds() * f64::from(sr)).round() as usize;
        let samples = match self.backing.as_ref()? {
            Backing::Wav(path) => match read_wav(path, sr) {
                Ok(mut s) => {
                    s.resize(len, 0.0);
                    s
                }
                Err(e) => {
                    eprintln!("backing {}: {e}", path.display());
                    return None;
                }
            },
            Backing::Generate { parts, gain_db } => {
                let mut s = crate::backing::render(&pass, parts, sr);
                let g = 10f32.powf(gain_db / 20.0);
                s.iter_mut().for_each(|v| *v *= g);
                s
            }
        };
        Some(Arc::new(samples))
    }

    /// The riff's bars, `reps` times.
    pub fn sequence(&self, reps: u32) -> crate::patterns::Sequence {
        let n = self.bars.len();
        crate::patterns::Sequence { patterns: self.bars.clone(), bar_patterns: (0..n * reps as usize).map(|b| b % n).collect(), seed: None }
    }
}

/// A WAV file as mono f32 at `sr` (linear resampling).
fn read_wav(path: &Path, sr: u32) -> Result<Vec<f32>, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = r.spec();
    let ch = usize::from(spec.channels.max(1));
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>().map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|s| s.map(|v| v as f32 * scale)).collect::<Result<_, _>>().map_err(|e| e.to_string())?
        }
    };
    let mono: Vec<f32> = raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    if spec.sample_rate == sr || mono.is_empty() {
        return Ok(mono);
    }
    let step = f64::from(spec.sample_rate) / f64::from(sr);
    let n = (mono.len() as f64 / step) as usize;
    Ok((0..n)
        .map(|i| {
            let x = i as f64 * step;
            let (a, f) = (x as usize, (x.fract()) as f32);
            let b = (a + 1).min(mono.len() - 1);
            mono[a] * (1.0 - f) + mono[b] * f
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_archives_and_folders() {
        let base = std::env::temp_dir().join(format!("gg-import-test-{}", std::process::id()));
        let (src, packs) = (base.join("src/mypack"), base.join("packs"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("pack.json"), r#"{"name":"Mine","songs":[]}"#).unwrap();
        std::fs::write(src.join("riff.wav"), b"x").unwrap();
        // A .tar.gz with the pack in a top-level folder.
        let tgz = base.join("dl.tar.gz");
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(std::fs::File::create(&tgz).unwrap(), flate2::Compression::default()));
        b.append_dir_all("mypack", &src).unwrap();
        b.into_inner().unwrap().finish().unwrap();
        assert_eq!(import_into(&tgz, &packs).unwrap(), "Mine");
        assert!(packs.join("mypack/pack.json").is_file() && packs.join("mypack/riff.wav").is_file());
        assert!(!packs.join(".importing").exists());
        // A plain .tar with pack.json at the root takes the archive's name.
        let t = base.join("flat.tar");
        let mut b = tar::Builder::new(std::fs::File::create(&t).unwrap());
        b.append_path_with_name(src.join("pack.json"), "pack.json").unwrap();
        b.finish().unwrap();
        import_into(&t, &packs).unwrap();
        assert!(packs.join("flat/pack.json").is_file());
        // A folder, or its pack.json.
        import_into(&src.join("pack.json"), &base.join("packs2")).unwrap();
        assert!(base.join("packs2/mypack/riff.wav").is_file());
        // Not a pack.
        std::fs::write(base.join("junk.zip"), b"not a zip").unwrap();
        assert!(import_into(&base.join("junk.zip"), &packs).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn parses_a_pack() {
        let dir = std::env::temp_dir().join(format!("gg-pack-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let json = r#"{"name":"t","songs":[
            {"id":"a","artist":"X","title":"A","unlock":{"bpm":80,"needs":["pm","sync"],"min":75}},
            {"id":"b","artist":"Y","title":"B","part":"riff","unlock":{"bpm":120,"drill":"gallop","min":60},
             "riff":{"bpm":124,"bars":[{"notation":"16:D---D-D---D-D-U-","frets":"A3 A3 A5 A3 A5 A7"},{"notation":"16:D-------D-------","frets":"A5 A3"}],
                     "backing":{"generate":["drums","bass","guitar"]}}},
            {"id":"bad","artist":"Z","title":"C","unlock":{"bpm":80,"min":60},"riff":{"bpm":90,"bars":[{"notation":"16:D---","frets":"A1"}]}}
        ]}"#;
        std::fs::write(dir.join("pack.json"), json).unwrap();
        let mut l = Loaded { songs: Vec::new(), riffs: Vec::new() };
        load_pack(&dir, &mut l).unwrap();
        assert_eq!(l.songs.len(), 2, "the bad song is skipped");
        assert!(matches!(l.songs[0].via, Via::Run(m) if m == crate::patterns::PM | crate::patterns::SYNC));
        assert_eq!((l.songs[1].riff, l.riffs.len()), (Some(0), 1));
        let r = &l.riffs[0];
        assert_eq!(r.mask(), crate::patterns::LH);
        let seq = r.sequence(RIFF_REPS);
        assert_eq!(seq.bar_patterns, vec![0, 1, 0, 1, 0, 1, 0, 1]);
        let b = r.backing_samples().unwrap();
        let pass = 2.0 * 4.0 * 60.0 / 124.0;
        assert_eq!(b.len(), (pass * 48000.0_f64).round() as usize);
        let peak = b.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.05 && peak <= 1.0, "peak {peak}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
