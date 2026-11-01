mod audio;
mod auto;
mod backing;
mod config;
mod countin;
mod detect;
mod dsp;
mod game;
mod generator;
mod mascot;
mod monitor;
mod packs;
mod paint;
mod patterns;
mod progress;
mod score;
mod session;
mod songs;
mod tone;
mod ui_menu;
mod ui_play;
mod ui_results;
mod ui_settings;
mod xp;

use bevy::prelude::*;
use bevy::window::WindowResolution;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--backing") {
        // `--backing <pack>/<song id> out.wav`: one pass of a riff's backing track.
        let (Some(id), Some(out)) = (args.get(1), args.get(2)) else {
            eprintln!("usage: guitarguru --backing <pack>/<song id> out.wav");
            return;
        };
        let Some(r) = packs::riff_by_id(id).map(|i| &packs::riffs()[i]) else {
            eprintln!("no riff `{id}`; have: {}", packs::riffs().iter().map(|r| r.id.as_str()).collect::<Vec<_>>().join(", "));
            return;
        };
        let Some(s) = r.backing_samples() else {
            eprintln!("`{id}` has no backing");
            return;
        };
        let spec = hound::WavSpec { channels: 1, sample_rate: audio::SAMPLE_RATE, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
        let mut w = hound::WavWriter::create(out, spec).expect("create wav");
        for _ in 0..packs::RIFF_REPS {
            s.iter().for_each(|v| w.write_sample(*v).expect("write"));
        }
        w.finalize().expect("finish wav");
        return;
    }
    if args.first().map(String::as_str) == Some("--rescore") {
        session::rescore_cli(&args[1..]);
        return;
    }
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Guitar Guru".into(),
                        resolution: WindowResolution::new(1440, 900),
                        ..default()
                    }),
                    ..default()
                })
                // Assets (fonts, mascot layers) live next to Cargo.toml.
                .set(AssetPlugin { file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(), ..default() }),
        )
        .add_systems(Startup, mascot::load)
        .add_plugins(paint::PaintPlugin)
        .insert_resource(game::Game::new())
        .insert_resource(auto::Auto::from_env())
        .add_systems(Update, (auto::auto_system, game::game_system).chain())
        .run();
}
