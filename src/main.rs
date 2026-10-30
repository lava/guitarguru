mod audio;
mod auto;
mod config;
mod countin;
mod detect;
mod dsp;
mod game;
mod generator;
mod mascot;
mod monitor;
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
