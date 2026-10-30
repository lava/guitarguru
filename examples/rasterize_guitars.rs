//! One-off tool: rasterise the sound choices on the main menu (hand-drawn, in the
//! mascot's ink-and-brush style) into PNGs.
//!
//!     cargo run --release --example rasterize_guitars
//!
//! Writes `assets/guitars/{heavy,crunch,clean,mute}.png` (viewBox 60x150 at 4x): a pointy
//! V for heavy, a single-cut for crunch, a double-cut with a pickguard for clean, and a
//! small amp for mute (the player's own amp).

use std::path::Path;

const SCALE: f32 = 4.0;
const W: u32 = 60;
const H: u32 = 150;

const INK: &str = "#2b211a";
const WOOD: &str = "#8a5a35";
const MAPLE: &str = "#e6c48e";
const CREAM: &str = "#fbf6ec";

/// Neck with a few frets, from `y0` to `y1`.
fn neck(fill: &str, y0: f32, y1: f32) -> String {
    let mut s = format!(r##"<rect x="27" y="{y0}" width="6" height="{}" fill="{fill}"/>"##, y1 - y0);
    let mut y = y0 + 8.0;
    while y < y1 - 4.0 {
        s += &format!(r##"<path d="M27.6 {y} h4.8" stroke-width=".7" opacity=".6"/>"##);
        y += 9.0;
    }
    s
}

/// Tuner pegs on both sides of a headstock between `y0` and `y1`.
fn pegs(xl: f32, xr: f32, ys: &[f32]) -> String {
    ys.iter().map(|y| format!(r##"<circle cx="{xl}" cy="{y}" r="1.3" fill="{CREAM}" stroke-width=".8"/><circle cx="{xr}" cy="{y}" r="1.3" fill="{CREAM}" stroke-width=".8"/>"##)).collect()
}

/// Two strings' worth of hairlines down the middle, from nut to bridge.
fn strings(y0: f32, y1: f32) -> String {
    format!(r##"<path d="M29 {y0} V{y1} M31 {y0} V{y1}" stroke="{CREAM}" stroke-width=".5" opacity=".9"/>"##)
}

fn heavy() -> String {
    let mut b = String::new();
    // Arrow headstock.
    b += r##"<path d="M30 1 L37 8 L34 21 L26 21 L23 8 Z" fill="#a9442e"/>"##;
    b += &pegs(22.5, 37.5, &[10.0, 15.0]);
    b += &neck(WOOD, 20.0, 82.0);
    // The V.
    b += r##"<path d="M25 78 L35 78 L55 140 L46 146 L30 106 L14 146 L5 140 Z" fill="#a9442e"/>"##;
    b += r##"<path d="M30 106 L 22 96 M30 106 L 38 96" stroke-width=".8" opacity=".5"/>"##;
    b += &format!(r##"<rect x="25.5" y="84" width="9" height="5" rx="1" fill="{CREAM}"/><rect x="25.5" y="93" width="9" height="5" rx="1" fill="{CREAM}"/>"##);
    b += &strings(21.0, 101.0);
    b += &format!(r##"<path d="M25.5 101 h9" stroke-width="1.4"/><circle cx="43" cy="122" r="1.6" fill="{CREAM}" stroke-width=".8"/><circle cx="47" cy="132" r="1.6" fill="{CREAM}" stroke-width=".8"/>"##);
    b
}

fn crunch() -> String {
    let mut b = String::new();
    // Open-book headstock.
    b += &format!(r##"<path d="M24 4 Q27 1 30 3 Q33 1 36 4 L35 19 L25 19 Z" fill="{WOOD}"/>"##);
    b += &pegs(23.5, 36.5, &[7.0, 11.5, 16.0]);
    b += &neck(WOOD, 18.0, 84.0);
    // Single cut: full upper bout on the bass side, cutaway on the treble side.
    b += r##"<path d="M25 80 C 15 78, 11 86, 14 95 C 16 101, 8 106, 8 119 C 8 136, 19 146, 30 146 C 41 146, 52 136, 52 119 C 52 107, 45 103, 46 96 C 47 90, 44 86, 39 88 C 36 89, 35 84, 35 80 Z" fill="#dca451"/>"##;
    b += r##"<path d="M10.5 119 C 11 133, 20 142.5, 30 142.5 C 40 142.5, 49 133, 49.5 119" fill="none" stroke-width=".7" opacity=".5"/>"##;
    b += &format!(r##"<rect x="24.5" y="96" width="11" height="6" rx="1.2" fill="{CREAM}"/><rect x="24.5" y="111" width="11" height="6" rx="1.2" fill="{CREAM}"/>"##);
    b += &strings(19.0, 124.0);
    b += r##"<path d="M24.5 121 h11 M25.5 126 h9" stroke-width="1.3"/>"##;
    for (x, y) in [(40.0, 130.0), (45.5, 124.0), (42.5, 137.0), (48.0, 131.0)] {
        b += &format!(r##"<circle cx="{x}" cy="{y}" r="1.7" fill="{CREAM}" stroke-width=".8"/>"##);
    }
    b
}

fn clean() -> String {
    let mut b = String::new();
    // Six-in-line headstock, pegs on one side.
    b += &format!(r##"<path d="M27 2 C 33 0, 38 4, 37 10 C 36 15, 33 16, 33 21 L 27 21 Z" fill="{MAPLE}"/>"##);
    for k in 0..4 {
        b += &format!(r##"<circle cx="25" cy="{}" r="1.2" fill="{CREAM}" stroke-width=".8"/>"##, 5.0 + k as f32 * 4.5);
    }
    b += &neck(MAPLE, 20.0, 84.0);
    // Double cut with horns.
    // Double cut: the bass-side horn reaches higher than the treble-side one.
    b += r##"<path d="M25 86 C 24 77, 20 69, 16 67 C 11 66, 10 73, 12 81 C 14 89, 13 95, 10 103 C 5 117, 8 136, 21 143 C 30 148, 42 146, 48 138 C 55 128, 54 112, 48 104 C 45 98, 45 92, 47 86 C 49 79, 47 73, 43 73 C 40 74, 37 80, 35 86 Z" fill="#1f7a6e"/>"##;
    // Pickguard and three single coils.
    b += &format!(r##"<path d="M25 88 C 20 90, 16 100, 17 111 C 18 123, 29 130, 39 125 L 37.5 112 C 36.5 104, 36 96, 35 88 Z" fill="{CREAM}"/>"##);
    for y in [95.0, 104.5, 114.0] {
        b += &format!(r##"<rect x="25.5" y="{y}" width="9" height="3.4" rx="1.4" fill="{CREAM}" stroke-width=".9"/>"##);
    }
    b += &strings(21.0, 126.0);
    b += r##"<path d="M25 126 h10" stroke-width="1.3"/>"##;
    for (x, y) in [(33.0, 121.5), (37.0, 117.0)] {
        b += &format!(r##"<circle cx="{x}" cy="{y}" r="1.5" fill="{CREAM}" stroke-width=".8"/>"##);
    }
    b
}

fn mute() -> String {
    let mut b = String::new();
    // A small combo amp: handle, control strip, grille with the speaker behind it.
    b += r##"<path d="M23 86 C 23 80, 37 80, 37 86" fill="none" stroke-width="2"/>"##;
    b += &format!(r##"<rect x="6" y="86" width="48" height="58" rx="4" fill="{WOOD}"/>"##);
    b += &format!(r##"<rect x="9.5" y="89.5" width="41" height="8" rx="1.5" fill="{CREAM}"/>"##);
    for x in [16.0, 23.0, 30.0, 37.0] {
        b += &format!(r##"<circle cx="{x}" cy="93.5" r="1.5" fill="{INK}" stroke="none"/>"##);
    }
    b += r##"<rect x="9.5" y="100.5" width="41" height="40" rx="2" fill="#ecd2b4"/>"##;
    b += r##"<circle cx="30" cy="120.5" r="14" fill="none" stroke-width="1" opacity=".55"/>"##;
    for k in 0..7 {
        let x = 12.0 + k as f32 * 6.0;
        b += &format!(r##"<path d="M{x} 102 L{} 139" stroke-width=".5" opacity=".35"/>"##, x + 4.0);
    }
    b
}

fn svg(body: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
<defs><filter id="gbrush" x="-5%" y="-5%" width="110%" height="110%"><feTurbulence type="fractalNoise" baseFrequency="0.12" numOctaves="2" seed="7"/><feDisplacementMap in="SourceGraphic" scale="0.7"/></filter></defs>
<g filter="url(#gbrush)" stroke="{INK}" stroke-width="1.7" stroke-linejoin="round" stroke-linecap="round">{body}</g></svg>"##
    )
}

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/guitars");
    std::fs::create_dir_all(&out).unwrap();
    let opt = resvg::usvg::Options::default();
    for (name, body) in [("heavy", heavy()), ("crunch", crunch()), ("clean", clean()), ("mute", mute())] {
        let tree = resvg::usvg::Tree::from_str(&svg(&body), &opt).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut pix = resvg::tiny_skia::Pixmap::new((W as f32 * SCALE) as u32, (H as f32 * SCALE) as u32).unwrap();
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(SCALE, SCALE), &mut pix.as_mut());
        pix.save_png(out.join(format!("{name}.png"))).unwrap();
    }
    println!("wrote 4 icons to {}", out.display());
}
