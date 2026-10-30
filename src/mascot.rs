//! The guru mascot: rasterised PNG layers (see `examples/rasterize_mascot.rs`) animated as
//! sprites. It keeps time by a head bob phase-locked to the click (driven by the audio
//! clock), deeper on beat one; five stages from serene to full metal.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::paint::Paint;
use crate::patterns::{BPM_MIN, BPM_STEP};

pub const STAGE_NAMES: [&str; 6] = ["", "Serene", "Plugged in", "Horns up", "Storm", "Full metal"];
const LAYERS: [&str; 7] = ["back", "hairback", "fly", "torso", "head", "front", "over"];
/// Source viewBox of the layers.
const VW: f32 = 240.0;
const VH: f32 = 280.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Mood {
    Neutral,
    Happy,
    Sad,
}

impl Mood {
    fn name(self) -> &'static str {
        match self {
            Mood::Neutral => "neutral",
            Mood::Happy => "happy",
            Mood::Sad => "sad",
        }
    }
}

/// Stage from tempo and ingredients: points = BPM step (0..7) + 2 per active ingredient;
/// stage = 1 + min(4, points / 3).
pub fn stage_for(bpm: u32, ingredients: u32) -> u32 {
    let step = bpm.saturating_sub(BPM_MIN) / BPM_STEP;
    let points = step + 2 * ingredients;
    1 + (points / 3).min(4)
}


#[derive(Resource)]
pub struct MascotImages {
    layers: HashMap<(u32, Mood, &'static str), Handle<Image>>,
}

pub fn load(mut commands: Commands, assets: Res<AssetServer>) {
    let mut layers = HashMap::new();
    for s in 1..=5 {
        for m in [Mood::Neutral, Mood::Happy, Mood::Sad] {
            for l in LAYERS {
                layers.insert((s, m, l), assets.load(format!("mascot/s{s}_{}_{l}.png", m.name())));
            }
        }
    }
    commands.insert_resource(MascotImages { layers });
}

/// Head motion at click phase `phase` (0 = the click; None = still).
pub struct Pose {
    dy: f32,
    sy: f32,
    rot_deg: f32,
    lift: f32,
}

pub fn pose(stage: u32, phase: Option<f64>, accent: bool) -> Pose {
    const AMP: [f32; 6] = [0.0, 1.5, 2.5, 4.0, 6.0, 9.0];
    const SQ: [f32; 6] = [0.0, 0.02, 0.03, 0.05, 0.07, 0.10];
    let s = stage.clamp(1, 5) as usize;
    let mut c = phase.map_or(0.0, |ph| (0.5 + 0.5 * (std::f64::consts::TAU * ph).cos()) as f32);
    if phase.is_some() && s >= 4 {
        c = c.powf(2.2);
    }
    let k = if accent { 1.35 } else { 1.0 };
    let rot_deg = if s == 5 {
        if phase.is_some() { -6.0 - 6.0 * c } else { -14.0 }
    } else {
        0.0
    };
    Pose { dy: AMP[s] * c * k, sy: 1.0 - SQ[s] * c * k, rot_deg, lift: if phase.is_some() { 1.0 - c } else { 1.0 } }
}

/// Draw the guru with its top-left at (x, y), `scale` design px per viewBox unit.
pub fn draw(p: &mut Paint, imgs: &MascotImages, x: f32, y: f32, scale: f32, stage: u32, mood: Mood, ps: &Pose) {
    let s = stage.clamp(1, 5);
    let to = |vx: f32, vy: f32| Vec2::new(x + vx * scale, y + vy * scale);
    let size = Vec2::new(VW * scale, VH * scale);
    let get = |l: &'static str| imgs.layers.get(&(s, mood, l));
    let body_dy = ps.dy * 0.2;
    // A layer scaled in y by `sy` about viewBox y `py`, then rotated by `rot` about the head
    // pivot (120,128) and moved down by `dy` (all in viewBox units, y down).
    let layer = |p: &mut Paint, l: &'static str, sy: f32, py: f32, rot_deg: f32, dy: f32| {
        let Some(h) = get(l) else { return };
        let a = Vec2::new(120.0, py);
        let piv = Vec2::new(120.0, 128.0);
        let th = rot_deg.to_radians();
        let v = a - piv;
        // y-down rotation: positive angle turns clockwise on screen.
        let rv = Vec2::new(v.x * th.cos() - v.y * th.sin(), v.x * th.sin() + v.y * th.cos());
        let pos = piv + rv + Vec2::new(0.0, dy);
        let anchor = Vec2::new(a.x / VW - 0.5, 0.5 - a.y / VH);
        p.image(h, to(pos.x, pos.y), Vec2::new(size.x, size.y * sy), anchor, -th, Color::WHITE);
    };
    let head_dy = ps.dy + body_dy;
    layer(p, "back", 1.0, 128.0, 0.0, 0.0);
    layer(p, "hairback", ps.sy, 128.0, ps.rot_deg, head_dy);
    if s == 5 {
        // Flying hair: stretched by 0.5 + 0.7 * lift about y = 70, inside the head motion.
        let f = 0.5 + 0.7 * ps.lift;
        let k = ps.sy * f;
        let y0 = if (1.0 - k).abs() > 1e-4 { (128.0 - 58.0 * ps.sy - 70.0 * ps.sy * f) / (1.0 - k) } else { 128.0 };
        layer(p, "fly", k, y0, ps.rot_deg, head_dy);
    }
    layer(p, "torso", 1.0, 128.0, 0.0, body_dy);
    layer(p, "head", ps.sy, 128.0, ps.rot_deg, head_dy);
    layer(p, "front", 1.0, 128.0, 0.0, body_dy);
    layer(p, "over", 1.0, 128.0, 0.0, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages() {
        assert_eq!(stage_for(40, 0), 1);
        assert_eq!(stage_for(80, 0), 1); // 2 points
        assert_eq!(stage_for(100, 0), 2); // 3
        assert_eq!(stage_for(120, 2), 3); // 4 + 4 = 8
        assert_eq!(stage_for(160, 2), 4); // 6 + 4 = 10
        assert_eq!(stage_for(180, 4), 5);
        let still = pose(5, None, false);
        assert_eq!(still.rot_deg, -14.0);
        let on_click = pose(3, Some(0.0), true);
        assert!((on_click.dy - 4.0 * 1.35).abs() < 1e-5);
        let off = pose(3, Some(0.5), false);
        assert!(off.dy.abs() < 1e-5);
    }
}
