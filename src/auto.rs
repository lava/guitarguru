//! Scripted runs for headless verification, driven by environment variables:
//! `GG_AUTOPLAY=mix:pm+16@120` (run) or `GG_AUTOPLAY=gallop@120` (drill) starts that level, `GG_SHOTS=3,9.5` saves screenshots at those
//! seconds to `GG_SHOT_DIR` (default `.`), `GG_END_AT=8` ends the run early (scores the
//! full bars played so far), `GG_LISTEN_AT=10` starts listen-back of the worst section,
//! `GG_FAKE_OUTPUT=1` replaces only the output with a silent one, `GG_EXIT_AT=12` quits, `GG_SCREEN=settings` opens the settings screen, `GG_SCROLLS=17:700` scrolls to 700 px at 17 s.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::game::Game;
use crate::patterns::Level;

#[derive(Resource)]
pub struct Auto {
    t: f32,
    autoplay: Option<(String, u32)>,
    shots: Vec<f32>,
    end_at: Option<f32>,
    listen_at: Option<f32>,
    exit_at: Option<f32>,
    dir: String,
    scrolls: Vec<(f32, f32)>,
}

fn num(name: &str) -> Option<f32> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

impl Auto {
    pub fn from_env() -> Self {
        let autoplay = std::env::var("GG_AUTOPLAY").ok().and_then(|v| {
            let (p, b) = v.split_once('@')?;
            Some((p.to_string(), b.parse().ok()?))
        });
        let shots = std::env::var("GG_SHOTS").map(|v| v.split(',').filter_map(|s| s.parse().ok()).collect()).unwrap_or_default();
        Self {
            t: 0.0,
            autoplay,
            shots,
            end_at: num("GG_END_AT"),
            listen_at: num("GG_LISTEN_AT"),
            exit_at: num("GG_EXIT_AT"),
            dir: std::env::var("GG_SHOT_DIR").unwrap_or_else(|_| ".".into()),
            scrolls: std::env::var("GG_SCROLLS")
                .map(|v| v.split(',').filter_map(|p| p.split_once(':')).filter_map(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).collect())
                .unwrap_or_default(),
        }
    }
}

pub fn auto_system(mut auto: ResMut<Auto>, mut game: ResMut<Game>, time: Res<Time>, mut commands: Commands, mut exit: MessageWriter<AppExit>) {
    let prev = auto.t;
    auto.t += time.delta_secs();
    let now = auto.t;
    let crossed = |at: f32| prev < at && now >= at;
    if prev == 0.0
        && let Some((p, b)) = auto.autoplay.clone()
        && let Some(level) = Level::from_key(&format!("{p}@{b}"))
    {
        game.start_level(level, false);
    }
    if prev == 0.0 && std::env::var("GG_SCREEN").as_deref() == Ok("settings") {
        game.refresh_lists();
        game.screen = crate::game::Screen::Settings;
    }
    for (at, px) in auto.scrolls.clone() {
        if crossed(at) {
            game.scroll = px;
        }
    }
    if auto.end_at.is_some_and(crossed) {
        game.end_early();
    }
    if auto.listen_at.is_some_and(crossed) {
        game.listen(0);
    }
    let shots: Vec<f32> = auto.shots.iter().copied().filter(|s| crossed(*s)).collect();
    for s in shots {
        let path = format!("{}/shot_{s:05.1}.png", auto.dir);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
    if auto.exit_at.is_some_and(crossed) {
        exit.write(AppExit::Success);
    }
}
