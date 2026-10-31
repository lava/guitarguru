//! A tiny immediate-mode painter on top of Bevy: each frame the game pushes rectangles,
//! discs, petals, images, text, lines and circles in a fixed 1440x900 design space (origin
//! top-left, y down); `flush` maps them onto pooled sprites / Text2d entities / gizmos.
//! Buttons are hit-tested against the cursor. Every screen is a plain function of game
//! state. `offset_y` scrolls everything drawn afterwards; `fade` scales all alpha.
//!
//! Style ("Guitar Guru"): warm paper, ink lines, one ochre accent, gold for ornament,
//! Cormorant Garamond for display and numbers, Mukta for text.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, ScalingMode};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use bevy::text::{FontFeatureTag, FontFeatures, FontStyle, FontWeight, LetterSpacing};
use bevy::window::PrimaryWindow;

pub const W: f32 = 1440.0;
pub const H: f32 = 900.0;

pub mod col {
    use bevy::color::Color;
    const fn hex(r: u8, g: u8, b: u8) -> Color {
        Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }
    /// Paper.
    pub const BG: Color = hex(0xf6, 0xef, 0xe2);
    pub const PANEL: Color = hex(0xef, 0xe5, 0xd3);
    pub const PANEL2: Color = hex(0xe9, 0xdd, 0xc7);
    /// Hairlines.
    pub const LINE: Color = hex(0xd9, 0xcc, 0xb6);
    /// Ink.
    pub const TEXT: Color = hex(0x2b, 0x21, 0x1a);
    pub const DIM: Color = hex(0x5e, 0x4e, 0x40);
    pub const FAINT: Color = hex(0x7a, 0x6a, 0x5a);
    /// The single UI accent (selection, playhead, primary).
    pub const BRASS: Color = hex(0xc2, 0x7a, 0x2c);
    pub const OCHRE: Color = BRASS;
    pub const OCHRE_TINT: Color = hex(0xf0, 0xd9, 0xb5);
    /// Ornament only.
    pub const GOLD: Color = hex(0xb8, 0x91, 0x4a);
    pub const EARLY: Color = hex(0x3a, 0x4a, 0x8c);
    pub const ONTIME: Color = hex(0x1f, 0x7a, 0x6e);
    pub const TEAL_TINT: Color = hex(0xd6, 0xe8, 0xe2);
    pub const LATE: Color = hex(0xb5, 0x45, 0x2f);
    pub const TONE: Color = hex(0x7b, 0x4a, 0x9a);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Disc,
    /// Triangle pointing right (rotated by `rot`).
    Tri,
    /// A pointed leaf (mandala petal), long axis vertical.
    Petal,
    Grain,
}

#[derive(Clone)]
enum Tex {
    Shape(Shape),
    Image(Handle<Image>),
}

struct ShapeCmd {
    tex: Tex,
    center: Vec2,
    size: Vec2,
    color: Color,
    /// Rotation in radians, counter-clockwise on screen.
    rot: f32,
    /// Pivot within the sprite, -0.5..0.5 (x right, y up).
    anchor: Vec2,
    overlay: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Font {
    /// Cormorant Garamond.
    Serif,
    SerifItalic,
    /// Mukta.
    Sans,
    SansMedium,
    SansBold,
}

struct TextCmd {
    text: String,
    pos: Vec2,
    size: f32,
    color: Color,
    align: Align,
    font: Font,
    spacing: f32,
    overlay: bool,
}

enum LineCmd {
    Line(Vec2, Vec2, Color),
    Circle(Vec2, f32, Color),
}

#[derive(Resource)]
pub struct Paint {
    shapes: Vec<ShapeCmd>,
    texts: Vec<TextCmd>,
    lines: Vec<LineCmd>,
    /// Cursor in design coordinates (screen space, not scrolled).
    pub cursor: Option<Vec2>,
    pub clicked: bool,
    pub mouse_down: bool,
    pub scroll: f32,
    /// Added to every y coordinate drawn (negative = content scrolled up).
    pub offset_y: f32,
    /// Alpha multiplier for everything (screen fades).
    pub fade: f32,
    /// Draw on the overlay camera, above everything including ink lines (tooltips).
    overlay: bool,
    /// The tooltip asked for this frame: (target id, text).
    tip: Option<(String, String)>,
    /// The tooltip target under the cursor, and since when.
    tip_since: Option<(String, std::time::Instant)>,
}

/// How long the cursor rests on a target before its tooltip shows.
const TIP_DELAY: f32 = 0.8;

impl Default for Paint {
    fn default() -> Self {
        Self {
            shapes: Vec::new(),
            texts: Vec::new(),
            lines: Vec::new(),
            cursor: None,
            clicked: false,
            mouse_down: false,
            scroll: 0.0,
            offset_y: 0.0,
            fade: 1.0,
            overlay: false,
            tip: None,
            tip_since: None,
        }
    }
}

#[derive(Resource)]
struct ShapeImages {
    disc: Handle<Image>,
    tri: Handle<Image>,
    petal: Handle<Image>,
    grain: Handle<Image>,
}

#[derive(Resource)]
struct Fonts {
    serif: Handle<bevy::text::Font>,
    serif_italic: Handle<bevy::text::Font>,
    sans: Handle<bevy::text::Font>,
    sans_medium: Handle<bevy::text::Font>,
    sans_bold: Handle<bevy::text::Font>,
}

#[derive(Component)]
struct PoolSprite;
#[derive(Component)]
struct PoolText;
#[derive(Component)]
struct OverlayCam;

impl Paint {
    fn push(&mut self, tex: Tex, center: Vec2, size: Vec2, color: Color, rot: f32, anchor: Vec2) {
        let center = Vec2::new(center.x, center.y + self.offset_y);
        self.shapes.push(ShapeCmd { tex, center, size, color, rot, anchor, overlay: self.overlay });
    }
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.push(Tex::Shape(Shape::Rect), Vec2::new(x + w / 2.0, y + h / 2.0), Vec2::new(w, h), color, 0.0, Vec2::ZERO);
    }
    /// Rectangle centred at (cx, cy), rotated by `rot` radians (counter-clockwise on screen).
    pub fn rot_rect(&mut self, cx: f32, cy: f32, w: f32, h: f32, rot: f32, color: Color) {
        self.push(Tex::Shape(Shape::Rect), Vec2::new(cx, cy), Vec2::new(w, h), color, rot, Vec2::ZERO);
    }
    /// Outline of width `t`.
    pub fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, color: Color) {
        self.rect(x, y, w, t, color);
        self.rect(x, y + h - t, w, t, color);
        self.rect(x, y, t, h, color);
        self.rect(x + w - t, y, t, h, color);
    }
    /// Dashed outline (locked things).
    pub fn dashed_frame(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let dash = 5.0f32;
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
    pub fn hline(&mut self, x: f32, y: f32, w: f32, color: Color) {
        self.rect(x, y, w, 1.0, color);
    }
    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.push(Tex::Shape(Shape::Disc), Vec2::new(cx, cy), Vec2::splat(2.0 * r), color, 0.0, Vec2::ZERO);
    }
    /// Thin ink circle outline.
    pub fn ring(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.lines.push(LineCmd::Circle(Vec2::new(cx, cy + self.offset_y), r, color));
    }
    /// Dashed circle outline.
    pub fn dashed_ring(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        let n = ((std::f32::consts::TAU * r) / 7.0).max(12.0) as usize;
        for i in (0..n).step_by(2) {
            let (a0, a1) = (std::f32::consts::TAU * i as f32 / n as f32, std::f32::consts::TAU * (i + 1) as f32 / n as f32);
            self.line(cx + r * a0.cos(), cy + r * a0.sin(), cx + r * a1.cos(), cy + r * a1.sin(), color);
        }
    }
    /// Ring of fine dots.
    pub fn dotted_ring(&mut self, cx: f32, cy: f32, r: f32, dot: f32, color: Color) {
        let n = ((std::f32::consts::TAU * r) / (dot * 4.0)).max(12.0) as usize;
        for i in 0..n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            self.disc(cx + r * a.cos(), cy + r * a.sin(), dot, color);
        }
    }
    pub fn tri(&mut self, cx: f32, cy: f32, r: f32, right: bool, color: Color) {
        let rot = if right { 0.0 } else { std::f32::consts::PI };
        self.push(Tex::Shape(Shape::Tri), Vec2::new(cx, cy), Vec2::splat(2.0 * r), color, rot, Vec2::ZERO);
    }
    /// Mandala petal centred at (cx, cy), length `len`, width `w`, pointing at angle `rot`
    /// (radians, 0 = up, counter-clockwise on screen).
    pub fn petal(&mut self, cx: f32, cy: f32, len: f32, w: f32, rot: f32, color: Color) {
        self.push(Tex::Shape(Shape::Petal), Vec2::new(cx, cy), Vec2::new(w, len), color, rot, Vec2::ZERO);
    }
    /// An image (e.g. a mascot layer). `pivot` is the point (in design units, absolute)
    /// that `rot` and `scale` act around; the image's own pivot is `anchor` (-0.5..0.5).
    #[allow(clippy::too_many_arguments)]
    pub fn image(&mut self, img: &Handle<Image>, pivot: Vec2, size: Vec2, anchor: Vec2, rot: f32, color: Color) {
        self.push(Tex::Image(img.clone()), pivot, size, color, rot, anchor);
    }
    /// Thin pick-direction arrow: a shaft with an open arrowhead (ink lines).
    pub fn stroke_arrow(&mut self, cx: f32, cy: f32, len: f32, down: bool, color: Color) {
        let (y0, y1) = (cy - len / 2.0, cy + len / 2.0);
        let (tip, tail) = if down { (y1, y0) } else { (y0, y1) };
        let head = (len * 0.28).clamp(4.0, 11.0);
        let dir = if down { -1.0 } else { 1.0 };
        for o in [-0.6f32, 0.0, 0.6] {
            self.line(cx + o, tail, cx + o, tip, color);
        }
        for o in [0.0f32, 0.6] {
            self.line(cx + o, tip, cx - head * 0.62 + o, tip + dir * head, color);
            self.line(cx - o, tip, cx + head * 0.62 - o, tip + dir * head, color);
        }
    }
    /// Small pick-direction mark: "v" for a downstroke, "^" for an upstroke, `w` wide and
    /// `h` tall around (`cx`, `cy`), in slightly thickened ink lines.
    pub fn chevron(&mut self, cx: f32, cy: f32, w: f32, h: f32, down: bool, color: Color) {
        let (tip, ends) = if down { (cy + h / 2.0, cy - h / 2.0) } else { (cy - h / 2.0, cy + h / 2.0) };
        for o in [-0.5f32, 0.0, 0.5] {
            self.line(cx - w / 2.0 + o, ends, cx + o, tip, color);
            self.line(cx + w / 2.0 + o, ends, cx + o, tip, color);
        }
    }
    pub fn cross(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.line(cx - r, cy - r, cx + r, cy + r, color);
        self.line(cx - r, cy + r, cx + r, cy - r, color);
    }
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color) {
        let o = self.offset_y;
        self.lines.push(LineCmd::Line(Vec2::new(x0, y0 + o), Vec2::new(x1, y1 + o), color));
    }
    /// Sans text with its vertical centre at `y`.
    pub fn text(&mut self, x: f32, y: f32, s: impl Into<String>, size: f32, color: Color, align: Align) {
        self.styled(x, y, s, size, color, align, Font::Sans, 0.0);
    }
    /// Serif (display / numbers).
    pub fn serif(&mut self, x: f32, y: f32, s: impl Into<String>, size: f32, color: Color, align: Align) {
        self.styled(x, y, s, size, color, align, Font::Serif, 0.0);
    }
    pub fn italic(&mut self, x: f32, y: f32, s: impl Into<String>, size: f32, color: Color, align: Align) {
        self.styled(x, y, s, size, color, align, Font::SerifItalic, 0.0);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn styled(&mut self, x: f32, y: f32, s: impl Into<String>, size: f32, color: Color, align: Align, font: Font, spacing: f32) {
        let pos = Vec2::new(x, y + self.offset_y);
        self.texts.push(TextCmd { text: s.into(), pos, size, color, align, font, spacing, overlay: self.overlay });
    }
    /// Small uppercase letter-spaced label.
    pub fn label(&mut self, x: f32, y: f32, s: &str) {
        self.styled(x, y, s.to_uppercase(), 11.0, col::DIM, Align::Left, Font::SansBold, 1.6);
    }
    pub fn label_at(&mut self, x: f32, y: f32, s: &str, align: Align, color: Color) {
        self.styled(x, y, s.to_uppercase(), 11.0, color, align, Font::SansBold, 1.6);
    }
    /// Show `text` at the cursor once it has rested on target `id` for a moment. Call it
    /// every frame the target is hovered.
    pub fn tooltip(&mut self, id: impl Into<String>, text: impl Into<String>) {
        self.tip = Some((id.into(), text.into()));
    }
    fn draw_tooltip(&mut self) {
        let req = self.tip.take();
        let (Some((id, text)), Some(c)) = (req, self.cursor) else {
            self.tip_since = None;
            return;
        };
        match &self.tip_since {
            Some((cur, t)) if *cur == id => {
                if t.elapsed().as_secs_f32() < TIP_DELAY {
                    return;
                }
            }
            _ => {
                self.tip_since = Some((id, std::time::Instant::now()));
                return;
            }
        }
        let (w, h) = (text.chars().count() as f32 * 6.4 + 24.0, 30.0);
        let x = (c.x + 12.0).min(W - w - 8.0);
        let y = if c.y + 22.0 + h > H - 8.0 { c.y - 10.0 - h } else { c.y + 22.0 };
        let off = std::mem::take(&mut self.offset_y);
        self.overlay = true;
        self.rect(x, y, w, h, col::TEXT);
        self.text(x + 12.0, y + h / 2.0 - 1.0, text, 14.0, col::BG, Align::Left);
        self.overlay = false;
        self.offset_y = off;
    }
    /// Ellipse centred at (cx, cy), rotated by `rot` radians.
    pub fn ellipse(&mut self, cx: f32, cy: f32, w: f32, h: f32, rot: f32, color: Color) {
        self.push(Tex::Shape(Shape::Disc), Vec2::new(cx, cy), Vec2::new(w, h), color, rot, Vec2::ZERO);
    }
    /// Is the cursor over this (scrolled) rectangle?
    pub fn hover(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let y = y + self.offset_y;
        self.cursor.is_some_and(|c| c.x >= x && c.x < x + w && c.y >= y && c.y < y + h)
    }
    pub fn hover_circle(&self, cx: f32, cy: f32, r: f32) -> bool {
        let cy = cy + self.offset_y;
        self.cursor.is_some_and(|c| (c.x - cx).powi(2) + (c.y - cy).powi(2) <= r * r)
    }
    pub fn hit(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.clicked && self.hover(x, y, w, h)
    }
    /// A button; returns true when clicked. Primary = ink fill with paper text; secondary
    /// = ink outline.
    pub fn button(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, primary: bool) -> bool {
        let hov = self.hover(x, y, w, h);
        if primary {
            self.rect(x, y, w, h, if hov { col::DIM } else { col::TEXT });
            self.serif(x + w / 2.0, y + h / 2.0 - 2.0, label, (h * 0.46).min(24.0), col::BG, Align::Center);
        } else {
            if hov {
                self.rect(x, y, w, h, col::PANEL);
            }
            self.frame(x, y, w, h, 1.2, col::TEXT);
            self.serif(x + w / 2.0, y + h / 2.0 - 2.0, label, (h * 0.46).min(22.0), col::TEXT, Align::Center);
        }
        self.hit(x, y, w, h)
    }
    /// Round icon button (outline circle) with a short glyph; returns true when clicked.
    pub fn round_button(&mut self, cx: f32, cy: f32, r: f32, glyph: &str) -> bool {
        let hov = self.hover_circle(cx, cy, r);
        if hov {
            self.disc(cx, cy, r, col::PANEL);
        }
        self.ring(cx, cy, r, col::TEXT);
        match glyph {
            "x" => self.cross(cx, cy, r * 0.22, col::TEXT),
            "flag" => {
                let (x, y, h) = (cx - r * 0.18, cy - r * 0.3, r * 0.6);
                self.line(x, y, x, y + h, col::TEXT);
                self.line(x, y, x + r * 0.4, y, col::TEXT);
                self.line(x + r * 0.4, y, x + r * 0.4, y + h * 0.45, col::TEXT);
                self.line(x, y + h * 0.45, x + r * 0.4, y + h * 0.45, col::TEXT);
            }
            "play" => self.tri(cx + r * 0.08, cy, r * 0.3, true, col::TEXT),
            "stop" => self.rect(cx - r * 0.22, cy - r * 0.22, r * 0.44, r * 0.44, col::TEXT),
            "-" => self.rect(cx - r * 0.3, cy - 0.6, r * 0.6, 1.2, col::TEXT),
            "+" => {
                self.rect(cx - r * 0.3, cy - 0.6, r * 0.6, 1.2, col::TEXT);
                self.rect(cx - 0.6, cy - r * 0.3, 1.2, r * 0.6, col::TEXT);
            }
            g => self.text(cx, cy - 1.0, g, r * 0.8, col::TEXT, Align::Center),
        }
        self.clicked && hov
    }
    /// A hairline list row with a radio dot; returns true when clicked.
    pub fn radio_row(&mut self, x: f32, y: f32, w: f32, h: f32, label: &str, note: &str, selected: bool) -> bool {
        let hov = self.hover(x, y, w, h);
        if hov {
            self.rect(x, y, w, h - 1.0, col::PANEL);
        }
        let cy = y + h / 2.0;
        if selected {
            self.disc(x + 8.0, cy, 6.0, col::OCHRE);
        } else {
            self.ring(x + 8.0, cy, 6.0, col::DIM);
        }
        let font = if selected { Font::SansBold } else { Font::Sans };
        self.styled(x + 26.0, cy - 1.0, fit(label, w - 140.0, 15.0), 15.0, col::TEXT, Align::Left, font, 0.0);
        self.text(x + w - 2.0, cy - 1.0, fit(note, 120.0, 12.0), 12.0, col::FAINT, Align::Right);
        self.hline(x, y + h - 1.0, w, col::LINE);
        self.clicked && hov
    }
}

/// Truncate `s` to fit `w` design pixels at font `size` (proportional fonts, ~0.5 em per
/// glyph on average).
pub fn fit(s: &str, w: f32, size: f32) -> String {
    let n = (w / (size * 0.52)).floor().max(2.0) as usize;
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "\u{2026}" }
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

/// Faint paper grain: low-alpha ink speckle, tiled over the background.
fn grain_image(size: u32) -> Image {
    let mut seed: u32 = 0x1234_5678;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for _ in 0..size * size {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let r = (seed % 1000) as f32 / 1000.0;
        let a = if r > 0.985 { 26 } else if r > 0.9 { 9 } else { 0 };
        data.extend_from_slice(&[43, 33, 26, a]);
    }
    Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, assets: Res<AssetServer>, mut gizmo_cfg: ResMut<GizmoConfigStore>) {
    commands.spawn((
        Camera2d,
        // No MSAA: shapes come from pre-antialiased textures; saves a lot of GPU memory.
        Msaa::Off,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin { min_width: W, min_height: H },
            ..OrthographicProjection::default_2d()
        }),
    ));
    // Tooltips: a second camera on layer 1, drawn after the main one (and its gizmos).
    commands.spawn((
        Camera2d,
        Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
        Msaa::Off,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin { min_width: W, min_height: H },
            ..OrthographicProjection::default_2d()
        }),
        RenderLayers::layer(1),
        OverlayCam,
    ));
    let (cfg, _) = gizmo_cfg.config_mut::<DefaultGizmoConfigGroup>();
    cfg.line.width = 2.4;
    let disc = images.add(shape_image(128, |u, v| {
        let (dx, dy) = (u - 0.5, v - 0.5);
        if dx * dx + dy * dy <= 0.25 { 1.0 } else { 0.0 }
    }));
    let tri = images.add(shape_image(64, |u, v| if (v - 0.5).abs() <= 0.5 * (1.0 - u) { 1.0 } else { 0.0 }));
    // Petal: two circular arcs meeting in points at top and bottom.
    let petal = images.add(shape_image(96, |u, v| {
        let (x, y) = ((u - 0.5) * 2.0, (v - 0.5) * 2.0);
        // lens: |x| <= w(y), w(y) = sqrt(R^2 - y^2) - (R - 1), R chosen so w(0) = 1
        // Pointed at both ends, fuller towards the inner end.
        let w = (1.0 - y * y).max(0.0).powf(0.9) * (1.0 - 0.12 * y);
        if x.abs() <= w * 0.98 { 1.0 } else { 0.0 }
    }));
    let grain = images.add(grain_image(256));
    commands.insert_resource(ShapeImages { disc, tri, petal, grain });
    commands.insert_resource(Fonts {
        serif: assets.load("fonts/CormorantGaramond.ttf"),
        serif_italic: assets.load("fonts/CormorantGaramond-Italic.ttf"),
        sans: assets.load("fonts/Mukta-Regular.ttf"),
        sans_medium: assets.load("fonts/Mukta-Medium.ttf"),
        sans_bold: assets.load("fonts/Mukta-SemiBold.ttf"),
    });
}

fn begin_frame(
    mut paint: ResMut<Paint>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cams: Query<(&Camera, &GlobalTransform), Without<OverlayCam>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
) {
    paint.shapes.clear();
    paint.texts.clear();
    paint.lines.clear();
    paint.offset_y = 0.0;
    paint.cursor = None;
    if let (Ok(win), Ok((cam, gt))) = (windows.single(), cams.single())
        && let Some(c) = win.cursor_position()
        && let Ok(w) = cam.viewport_to_world_2d(gt, c)
    {
        paint.cursor = Some(Vec2::new(w.x + W / 2.0, H / 2.0 - w.y));
    }
    paint.clicked = mouse.just_pressed(MouseButton::Left);
    paint.mouse_down = mouse.pressed(MouseButton::Left);
    paint.scroll = wheel
        .read()
        .map(|e| match e.unit {
            bevy::input::mouse::MouseScrollUnit::Line => e.y * 40.0,
            bevy::input::mouse::MouseScrollUnit::Pixel => e.y,
        })
        .sum();
    // Paper grain under everything.
    for gy in 0..4 {
        for gx in 0..6 {
            paint.shapes.push(ShapeCmd {
                tex: Tex::Shape(Shape::Grain),
                center: Vec2::new(128.0 + gx as f32 * 256.0, 128.0 + gy as f32 * 256.0),
                size: Vec2::splat(256.0),
                color: Color::WHITE,
                rot: 0.0,
                anchor: Vec2::ZERO,
                overlay: false,
            });
        }
    }
}

fn make_font(handle: &Handle<bevy::text::Font>, size: f32, weight: FontWeight, style: FontStyle) -> TextFont {
    let mut f = TextFont::from(handle.clone()).with_font_size(size).with_font_weight(weight);
    f.style = style;
    // Lining figures (Cormorant defaults to old-style numerals).
    f.font_features = FontFeatures::builder().enable(FontFeatureTag::LINING_FIGURES).build();
    f
}

fn faded(c: Color, k: f32) -> Color {
    if k >= 0.999 {
        return c;
    }
    let a = c.alpha();
    c.with_alpha(a * k)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn flush(
    mut commands: Commands,
    mut paint: ResMut<Paint>,
    images: Res<ShapeImages>,
    fonts: Res<Fonts>,
    mut sprites: Query<(&mut Sprite, &mut Transform, &mut Visibility, &mut Anchor, &mut RenderLayers), (With<PoolSprite>, Without<PoolText>)>,
    mut texts: Query<
        (&mut Text2d, &mut TextFont, &mut TextColor, &mut Anchor, &mut Transform, &mut LetterSpacing, &mut RenderLayers),
        (With<PoolText>, Without<PoolSprite>),
    >,
    mut gizmos: Gizmos,
) {
    let paint = &mut *paint;
    paint.draw_tooltip();
    let k = paint.fade.clamp(0.0, 1.0);
    let mut it = sprites.iter_mut();
    for (i, s) in paint.shapes.iter().enumerate() {
        let z = i as f32 * 0.0005 + if s.overlay { 20.0 } else { 0.0 };
        let image = match &s.tex {
            Tex::Shape(Shape::Rect) => Handle::default(),
            Tex::Shape(Shape::Disc) => images.disc.clone(),
            Tex::Shape(Shape::Tri) => images.tri.clone(),
            Tex::Shape(Shape::Petal) => images.petal.clone(),
            Tex::Shape(Shape::Grain) => images.grain.clone(),
            Tex::Image(h) => h.clone(),
        };
        let pos = to_world(s.center).extend(z);
        let layers = RenderLayers::layer(usize::from(s.overlay));
        let anchor = Anchor(s.anchor);
        let color = if matches!(s.tex, Tex::Shape(Shape::Grain)) { s.color } else { faded(s.color, k) };
        match it.next() {
            Some((mut sp, mut tf, mut vis, mut anc, mut layer)) => {
                if *layer != layers {
                    *layer = layers;
                }
                if sp.image != image {
                    sp.image = image;
                }
                sp.color = color;
                sp.custom_size = Some(s.size);
                tf.translation = pos;
                tf.rotation = Quat::from_rotation_z(s.rot);
                if *anc != anchor {
                    *anc = anchor;
                }
                *vis = Visibility::Visible;
            }
            None => {
                let mut sp = Sprite::from_color(color, s.size);
                sp.image = image;
                let tf = Transform::from_translation(pos).with_rotation(Quat::from_rotation_z(s.rot));
                commands.spawn((sp, tf, anchor, Visibility::Visible, layers, PoolSprite));
            }
        }
    }
    for (_, _, mut vis, _, _) in it {
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
    }
    let mut it = texts.iter_mut();
    for t in &paint.texts {
        let anchor = match t.align {
            Align::Left => Anchor::CENTER_LEFT,
            Align::Center => Anchor::CENTER,
            Align::Right => Anchor::CENTER_RIGHT,
        };
        let (handle, weight, style) = match t.font {
            Font::Serif => (&fonts.serif, FontWeight(500), FontStyle::Normal),
            Font::SerifItalic => (&fonts.serif_italic, FontWeight(400), FontStyle::Italic),
            Font::Sans => (&fonts.sans, FontWeight(400), FontStyle::Normal),
            Font::SansMedium => (&fonts.sans_medium, FontWeight(500), FontStyle::Normal),
            Font::SansBold => (&fonts.sans_bold, FontWeight(600), FontStyle::Normal),
        };
        let pos = to_world(t.pos).extend(if t.overlay { 30.0 } else { 10.0 });
        let layers = RenderLayers::layer(usize::from(t.overlay));
        let color = faded(t.color, k);
        let spacing = LetterSpacing::Px(t.spacing);
        match it.next() {
            Some((mut tx, mut font, mut tc, mut anc, mut tf, mut ls, mut layer)) => {
                if *layer != layers {
                    *layer = layers;
                }
                if tx.0 != t.text {
                    tx.0.clone_from(&t.text);
                }
                let want = TextFont::from(handle.clone()).with_font_size(t.size).with_font_weight(weight);
                if font.font != want.font || font.font_size != want.font_size || font.weight != weight || font.style != style {
                    *font = make_font(handle, t.size, weight, style);
                }
                if tc.0 != color {
                    tc.0 = color;
                }
                if *anc != anchor {
                    *anc = anchor;
                }
                if tf.translation != pos {
                    tf.translation = pos;
                }
                if *ls != spacing {
                    *ls = spacing;
                }
            }
            None => {
                let font = make_font(handle, t.size, weight, style);
                commands.spawn((Text2d::new(t.text.clone()), font, TextColor(color), anchor, Transform::from_translation(pos), spacing, layers, PoolText));
            }
        }
    }
    // Unused texts are blanked rather than hidden: a hidden Text2d does not pick up
    // content changes when it is shown again.
    for (mut tx, _, _, _, _, _, _) in it {
        if !tx.0.is_empty() {
            tx.0.clear();
        }
    }
    for l in &paint.lines {
        match l {
            LineCmd::Line(a, b, c) => gizmos.line_2d(to_world(*a), to_world(*b), faded(*c, k)),
            LineCmd::Circle(c, r, color) => {
                gizmos.circle_2d(Isometry2d::from_translation(to_world(*c)), *r, faded(*color, k)).resolution(((*r) as u32).clamp(24, 160));
            }
        }
    }
}
