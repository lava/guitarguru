//! A tiny immediate-mode painter on top of Bevy: each frame the game pushes rectangles,
//! discs, triangles, text and lines in a fixed 1440x900 design space (origin top-left,
//! y down); `flush` maps them onto pooled sprites / Text2d entities / gizmos. Buttons are
//! hit-tested against the cursor. This keeps every screen a plain function of game state.

use bevy::asset::RenderAssetUsages;
use bevy::camera::ScalingMode;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use bevy::window::PrimaryWindow;

pub const W: f32 = 1440.0;
pub const H: f32 = 900.0;

pub mod col {
    use bevy::color::Color;
    const fn hex(r: u8, g: u8, b: u8) -> Color {
        Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }
    pub const BG: Color = hex(0x0e, 0x11, 0x14);
    pub const PANEL: Color = hex(0x16, 0x1b, 0x20);
    pub const PANEL2: Color = hex(0x1c, 0x23, 0x2a);
    pub const LINE: Color = hex(0x2a, 0x33, 0x3c);
    pub const TEXT: Color = hex(0xec, 0xe8, 0xde);
    pub const DIM: Color = hex(0x8b, 0x8f, 0x8f);
    pub const FAINT: Color = hex(0x55, 0x5c, 0x62);
    pub const BRASS: Color = hex(0xd1, 0xa1, 0x4a);
    pub const EARLY: Color = hex(0x3d, 0x8a, 0xe6);
    pub const LATE: Color = hex(0xe0, 0x70, 0x3a);
    pub const MISS: Color = hex(0xe0, 0x52, 0x52);
    pub const TONE: Color = hex(0x90, 0x85, 0xe9);
    pub const GOOD: Color = hex(0x5c, 0xb8, 0x6c);
    pub const BAD: Color = hex(0xd9, 0x77, 0x6f);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Disc,
    /// Triangle pointing right.
    TriRight,
    TriLeft,
}

struct ShapeCmd {
    shape: Shape,
    center: Vec2,
    size: Vec2,
    color: Color,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

struct TextCmd {
    text: String,
    pos: Vec2,
    size: f32,
    color: Color,
    align: Align,
}

struct LineCmd {
    a: Vec2,
    b: Vec2,
    color: Color,
}

#[derive(Resource, Default)]
pub struct Paint {
    shapes: Vec<ShapeCmd>,
    texts: Vec<TextCmd>,
    lines: Vec<LineCmd>,
    /// Cursor in design coordinates.
    pub cursor: Option<Vec2>,
    pub clicked: bool,
    pub mouse_down: bool,
    pub scroll: f32,
}

#[derive(Resource)]
struct ShapeImages {
    disc: Handle<Image>,
    tri: Handle<Image>,
}

#[derive(Component)]
struct PoolSprite;
#[derive(Component)]
struct PoolText;

impl Paint {
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.shapes.push(ShapeCmd { shape: Shape::Rect, center: Vec2::new(x + w / 2.0, y + h / 2.0), size: Vec2::new(w, h), color });
    }
    /// Outline of width `t`.
    pub fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, color: Color) {
        self.rect(x, y, w, t, color);
        self.rect(x, y + h - t, w, t, color);
        self.rect(x, y, t, h, color);
        self.rect(x + w - t, y, t, h, color);
    }
    pub fn dashed_frame(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let dash: f32 = 6.0;
        let mut i = 0.0;
        while i < w {
            let l = dash.min(w - i);
            self.rect(x + i, y, l, 1.0, color);
            self.rect(x + i, y + h - 1.0, l, 1.0, color);
            i += 2.0 * dash;
        }
        let mut i = 0.0;
        while i < h {
            let l = dash.min(h - i);
            self.rect(x, y + i, 1.0, l, color);
            self.rect(x + w - 1.0, y + i, 1.0, l, color);
            i += 2.0 * dash;
        }
    }
    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.shapes.push(ShapeCmd { shape: Shape::Disc, center: Vec2::new(cx, cy), size: Vec2::splat(2.0 * r), color });
    }
    pub fn tri(&mut self, cx: f32, cy: f32, r: f32, right: bool, color: Color) {
        let shape = if right { Shape::TriRight } else { Shape::TriLeft };
        self.shapes.push(ShapeCmd { shape, center: Vec2::new(cx, cy), size: Vec2::splat(2.0 * r), color });
    }
    pub fn cross(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.line(cx - r, cy - r, cx + r, cy + r, color);
        self.line(cx - r, cy + r, cx + r, cy - r, color);
    }
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color) {
        self.lines.push(LineCmd { a: Vec2::new(x0, y0), b: Vec2::new(x1, y1), color });
    }
    /// Text with its vertical centre at `y`.
    pub fn text(&mut self, x: f32, y: f32, s: impl Into<String>, size: f32, color: Color, align: Align) {
        self.texts.push(TextCmd { text: s.into(), pos: Vec2::new(x, y), size, color, align });
    }
    pub fn label(&mut self, x: f32, y: f32, s: &str) {
        self.text(x, y, s.to_uppercase(), 13.0, col::DIM, Align::Left);
    }
    pub fn hover(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.cursor.is_some_and(|c| c.x >= x && c.x < x + w && c.y >= y && c.y < y + h)
    }
    pub fn hit(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.clicked && self.hover(x, y, w, h)
    }
    /// A button; returns true when clicked. `primary` uses the brass accent.
    pub fn button(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, primary: bool) -> bool {
        let hov = self.hover(x, y, w, h);
        let (bg, fg) = if primary {
            (if hov { col::TEXT } else { col::BRASS }, col::BG)
        } else {
            (if hov { col::LINE } else { col::PANEL2 }, col::TEXT)
        };
        self.rect(x, y, w, h, bg);
        if !primary {
            self.frame(x, y, w, h, 1.0, col::LINE);
        }
        self.text(x + w / 2.0, y + h / 2.0, label, 18.0, fg, Align::Center);
        self.hit(x, y, w, h)
    }
    /// A selectable row/chip; returns true when clicked.
    pub fn choice(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, selected: bool) -> bool {
        let hov = self.hover(x, y, w, h);
        self.rect(x, y, w, h, if selected { col::PANEL2 } else if hov { col::PANEL } else { col::BG });
        self.frame(x, y, w, h, 1.0, if selected { col::BRASS } else { col::LINE });
        self.text(x + 10.0, y + h / 2.0, label, 16.0, if selected { col::TEXT } else { col::DIM }, Align::Left);
        self.hit(x, y, w, h)
    }
    pub fn panel(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.rect(x, y, w, h, col::PANEL);
        self.frame(x, y, w, h, 1.0, col::LINE);
    }
}

fn to_world(p: Vec2) -> Vec2 {
    Vec2::new(p.x - W / 2.0, H / 2.0 - p.y)
}

pub struct PaintPlugin;

impl Plugin for PaintPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Paint>()
            .insert_resource(ClearColor(col::BG))
            .add_systems(Startup, setup)
            .add_systems(PreUpdate, begin_frame)
            .add_systems(PostUpdate, flush);
    }
}

fn shape_image(size: u32, f: impl Fn(f32, f32) -> f32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // coverage via 4x4 supersampling
            let mut cov = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let u = (x as f32 + (sx as f32 + 0.5) / 4.0) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / 4.0) / size as f32;
                    cov += f(u, v);
                }
            }
            let a = (cov / 16.0 * 255.0) as u8;
            data.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin { min_width: W, min_height: H },
            ..OrthographicProjection::default_2d()
        }),
    ));
    let disc = images.add(shape_image(64, |u, v| {
        let (dx, dy) = (u - 0.5, v - 0.5);
        if dx * dx + dy * dy <= 0.25 { 1.0 } else { 0.0 }
    }));
    // Right-pointing triangle: apex at (1, 0.5), base at x = 0.
    let tri = images.add(shape_image(64, |u, v| if (v - 0.5).abs() <= 0.5 * (1.0 - u) { 1.0 } else { 0.0 }));
    commands.insert_resource(ShapeImages { disc, tri });
}

fn begin_frame(
    mut paint: ResMut<Paint>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<(&Camera, &GlobalTransform)>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
) {
    paint.shapes.clear();
    paint.texts.clear();
    paint.lines.clear();
    paint.cursor = None;
    if let (Ok(win), Ok((cam, gt))) = (windows.single(), cams.single())
        && let Some(c) = win.cursor_position()
        && let Ok(w) = cam.viewport_to_world_2d(gt, c)
    {
        paint.cursor = Some(Vec2::new(w.x + W / 2.0, H / 2.0 - w.y));
    }
    paint.clicked = mouse.just_pressed(MouseButton::Left);
    paint.mouse_down = mouse.pressed(MouseButton::Left);
    paint.scroll = wheel.read().map(|e| e.y).sum();
}

#[allow(clippy::too_many_arguments)]
fn flush(
    mut commands: Commands,
    mut paint: ResMut<Paint>,
    images: Res<ShapeImages>,
    mut sprites: Query<(&mut Sprite, &mut Transform, &mut Visibility), (With<PoolSprite>, Without<PoolText>)>,
    mut texts: Query<
        (&mut Text2d, &mut TextFont, &mut TextColor, &mut Anchor, &mut Transform, &mut Visibility),
        (With<PoolText>, Without<PoolSprite>),
    >,
    mut gizmos: Gizmos,
) {
    let paint = &mut *paint;
    // Shapes
    let mut it = sprites.iter_mut();
    for (i, s) in paint.shapes.iter().enumerate() {
        let z = i as f32 * 0.0005;
        let image = match s.shape {
            Shape::Rect => Handle::default(),
            Shape::Disc => images.disc.clone(),
            Shape::TriRight | Shape::TriLeft => images.tri.clone(),
        };
        let flip = s.shape == Shape::TriLeft;
        let pos = to_world(s.center).extend(z);
        match it.next() {
            Some((mut sp, mut tf, mut vis)) => {
                if sp.image != image {
                    sp.image = image;
                }
                sp.color = s.color;
                sp.custom_size = Some(s.size);
                sp.flip_x = flip;
                tf.translation = pos;
                *vis = Visibility::Visible;
            }
            None => {
                let mut sp = Sprite::from_color(s.color, s.size);
                sp.image = image;
                sp.flip_x = flip;
                commands.spawn((sp, Transform::from_translation(pos), Visibility::Visible, PoolSprite));
            }
        }
    }
    for (_, _, mut vis) in it {
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
    }
    // Texts
    let mut it = texts.iter_mut();
    for t in &paint.texts {
        let anchor = match t.align {
            Align::Left => Anchor::CENTER_LEFT,
            Align::Center => Anchor::CENTER,
            Align::Right => Anchor::CENTER_RIGHT,
        };
        let pos = to_world(t.pos).extend(10.0);
        match it.next() {
            Some((mut tx, mut font, mut color, mut anc, mut tf, mut vis)) => {
                if tx.0 != t.text {
                    tx.0.clone_from(&t.text);
                }
                if font.font_size != FontSize::Px(t.size) {
                    font.font_size = FontSize::Px(t.size);
                }
                if color.0 != t.color {
                    color.0 = t.color;
                }
                if *anc != anchor {
                    *anc = anchor;
                }
                if tf.translation != pos {
                    tf.translation = pos;
                }
                if *vis != Visibility::Visible {
                    *vis = Visibility::Visible;
                }
            }
            None => {
                commands.spawn((
                    Text2d::new(t.text.clone()),
                    TextFont::from_font_size(t.size),
                    TextColor(t.color),
                    anchor,
                    Transform::from_translation(pos),
                    Visibility::Visible,
                    PoolText,
                ));
            }
        }
    }
    // Unused texts are blanked rather than hidden: a hidden Text2d does not pick up
    // content changes when it is shown again.
    for (mut tx, _, _, _, _, _) in it {
        if !tx.0.is_empty() {
            tx.0.clear();
        }
    }
    for l in &paint.lines {
        gizmos.line_2d(to_world(l.a), to_world(l.b), l.color);
    }
}
