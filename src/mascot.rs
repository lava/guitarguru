//! The guru mascot: rasterised PNG layers (see `examples/rasterize_mascot.rs`) animated as
//! sprites. It is a metronome: the raised horns hand swings like a pendulum, one full
//! cycle per two beats, passing the middle of its travel (its fastest point) on every
//! click with the turning points on the off-beat eighths; the head moves with it, wider
//! on beat one. Phase comes from the caller's clock (the audio clock while playing). Five
//! stages from serene to full metal.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::paint::Paint;
use crate::patterns::{BPM_MIN, BPM_STEP};

pub const STAGE_NAMES: [&str; 6] = ["", "Serene", "Plugged in", "Horns up", "Storm", "Full metal"];
const LAYERS: [&str; 8] = ["back", "hairback", "fly", "torso", "head", "front", "arm", "over"];
/// Shoulder the raised arm swings about (viewBox units).
const ARM_PIVOT: Vec2 = Vec2::new(166.0, 136.0);
/// The horns hand's centre at rest (the middle of the swing).
const ARM_HAND: Vec2 = Vec2::new(184.0, 88.0);
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

/// Head and arm motion.
pub struct Pose {
    dy: f32,
    sy: f32,
    rot_deg: f32,
    lift: f32,
    /// Arm rotation about the shoulder, degrees (positive = clockwise on screen, down).
    arm_deg: f32,
    /// Draw the click guide: a faint dashed line where the arm is in the middle of its
    /// swing, i.e. on every click.
    pub guide: bool,
}

/// Pendulum position at `beats` (beat count from the clock, beat one of a bar at multiples
/// of 4): sin(pi * beats), 0 on every click, +/-1 on the off-beat eighths; one full cycle
/// per two beats. Positive = head down / arm lowered.
pub fn swing(beats: f64) -> f64 {
    (std::f64::consts::PI * beats).sin()
}

/// Pose at `beats` (None = still, arm and head in the middle).
pub fn pose(stage: u32, beats: Option<f64>) -> Pose {
    const AMP: [f32; 6] = [0.0, 1.5, 2.5, 4.0, 6.0, 9.0];
    const SQ: [f32; 6] = [0.0, 0.02, 0.03, 0.05, 0.07, 0.10];
    const ARM: [f32; 6] = [0.0, 12.0, 14.0, 16.0, 19.0, 23.0];
    let s = stage.clamp(1, 5) as usize;
    let u = beats.map_or(0.0, swing) as f32;
    // Beat-one emphasis: a smooth bump (x1.35 at its peak) over the beat either side of
    // each bar's first click, so the amplitude never jumps.
    let k = beats.map_or(1.0, |b| {
        let d = (b - 4.0 * (b / 4.0).round()).abs();
        if d < 1.0 { 1.0 + 0.35 * (0.5 + 0.5 * (std::f64::consts::PI * d).cos()) as f32 } else { 1.0 }
    });
    let down = u.max(0.0);
    Pose {
        dy: AMP[s] * u * k,
        sy: 1.0 - SQ[s] * down * k,
        rot_deg: if s == 5 { -12.0 - 6.0 * u * k } else { 0.0 },
        lift: 0.5 - 0.5 * u,
        arm_deg: ARM[s] * u * k,
        guide: false,
    }
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
    let layer_about = |p: &mut Paint, l: &'static str, sy: f32, py: f32, rot_deg: f32, dy: f32, piv: Vec2| {
        let Some(h) = get(l) else { return };
        let a = Vec2::new(piv.x, py);
        let th = rot_deg.to_radians();
        let v = a - piv;
        // y-down rotation: positive angle turns clockwise on screen.
        let rv = Vec2::new(v.x * th.cos() - v.y * th.sin(), v.x * th.sin() + v.y * th.cos());
        let pos = piv + rv + Vec2::new(0.0, dy);
        let anchor = Vec2::new(a.x / VW - 0.5, 0.5 - a.y / VH);
        p.image(h, to(pos.x, pos.y), Vec2::new(size.x, size.y * sy), anchor, -th, Color::WHITE);
    };
    let layer = |p: &mut Paint, l: &'static str, sy: f32, py: f32, rot_deg: f32, dy: f32| {
        layer_about(p, l, sy, py, rot_deg, dy, Vec2::new(120.0, 128.0));
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
    if ps.guide {
        click_guide(p, &to, body_dy);
    }
    layer_about(p, "arm", 1.0, ARM_PIVOT.y, ps.arm_deg, body_dy, ARM_PIVOT);
    layer(p, "over", 1.0, 128.0, 0.0, 0.0);
}

/// Faint ink dashes along the arm's middle position, from the shoulder to just past the
/// horns' tips, moving with the body like the arm's pivot. Behind the arm layer, so the
/// arm covers it as it swings through on the click.
fn click_guide(p: &mut Paint, to: &impl Fn(f32, f32) -> Vec2, body_dy: f32) {
    let dir = (ARM_HAND - ARM_PIVOT).normalize();
    let (len, dash, gap) = (84.0, 5.0, 4.0);
    let ink = Color::srgba(0.17, 0.13, 0.10, 0.3);
    let mut d = 14.0;
    while d + dash <= len {
        let (a, b) = (ARM_PIVOT + dir * d, ARM_PIVOT + dir * (d + dash));
        let (a, b) = (to(a.x, a.y + body_dy), to(b.x, b.y + body_dy));
        p.line(a.x, a.y, b.x, b.y, ink);
        d += dash + gap;
    }
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
        let still = pose(5, None);
        assert_eq!(still.rot_deg, -12.0);
        assert_eq!(still.arm_deg, 0.0);
    }

    /// The arm passes the middle of its travel on every click (at full speed, alternating
    /// direction) and turns on the off-beat eighths.
    #[test]
    fn arm_is_a_metronome() {
        for beat in 0..8 {
            let b = f64::from(beat);
            let on = pose(3, Some(b));
            assert!(on.arm_deg.abs() < 1e-3 && on.dy.abs() < 1e-3, "beat {beat}: {}", on.arm_deg);
            let (before, after) = (pose(3, Some(b - 0.02)).arm_deg, pose(3, Some(b + 0.02)).arm_deg);
            assert!(before.signum() == -after.signum() && after.abs() > 0.5, "moving through the middle");
            let dir = if beat % 2 == 0 { 1.0 } else { -1.0 };
            assert!(after * dir > 0.0, "direction alternates per beat");
            let ext = pose(3, Some(b + 0.5)).arm_deg;
            assert!((ext.abs() - 16.0).abs() < 16.0 * 0.36, "extreme on the off-beat: {ext}");
            assert!((pose(3, Some(b + 0.49)).arm_deg.abs() - ext.abs()).abs() < 0.1);
        }
        // Head moves with the arm.
        let p = pose(4, Some(0.3));
        assert!(p.dy > 0.0 && p.arm_deg > 0.0);
    }
}
