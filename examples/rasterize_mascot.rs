//! One-off tool: rasterise the Guitar Guru mascot (from the GuruMascot design, SVG with a
//! hand-drawn "brush" displacement filter) into PNG layers per stage and mood, so the game
//! can animate head, body and hair as separate sprites.
//!
//!     cargo run --release --example rasterize_mascot
//!
//! Writes `assets/mascot/s{stage}_{mood}_{layer}.png` (viewBox 240x280 at 2x).
//! Layers, back to front: back (fire, cloud, lightning), hairback, fly (flying hair),
//! torso, head, front (guitar), arm (raised horns hand, every stage but sad), over (notes,
//! petals, lotus).
//! Runtime transforms (see `src/mascot.rs`): hairback/fly/head follow the head bob
//! (pivot 120,128), torso/front follow the body at 20 %, fly is also stretched about
//! (120,70), arm swings about the shoulder (`ARM_PIVOT`, 166,136) like a metronome.

use std::path::Path;

const SCALE: f32 = 2.0;
const W: u32 = 240;
const H: u32 = 280;

#[derive(Clone, Copy, PartialEq)]
enum Mood {
    Neutral,
    Happy,
    Sad,
}

fn layer(stage: u32, mood: Mood, name: &str) -> String {
    let s = stage;
    let happy = mood == Mood::Happy;
    let sad = mood == Mood::Sad;
    let robe = if s <= 2 { "#cf8a43" } else { "#a9442e" };
    let face = if s >= 4 { "#efe9df" } else { "#e9c39b" };
    let blush = if s >= 4 { 0.0 } else { 0.5 };
    let (cloud_fill, cloud_line) = match s {
        4 => ("#6b6b7d", "#2b2a36"),
        5 => ("#ecd2b4", "#b5452f"),
        _ => ("#fbf6ec", "#1f7a6e"),
    };
    let mut b = String::new();
    match name {
        "back" => {
            if s == 5 {
                b += r##"<g opacity=".9"><path d="M42 238 C 26 214, 42 198, 40 178 C 52 192, 58 174, 58 160 C 74 180, 76 204, 70 234 Z" fill="#c4582f"/>
<path d="M198 238 C 214 214, 198 198, 202 176 C 188 190, 184 172, 186 158 C 168 180, 166 204, 172 234 Z" fill="#c4582f"/>
<path d="M54 234 C 47 218, 56 208, 56 196 C 63 208, 66 218, 62 234 Z" fill="#dca451" stroke-width="1.2"/>
<path d="M186 234 C 193 218, 184 208, 186 196 C 178 208, 176 218, 180 234 Z" fill="#dca451" stroke-width="1.2"/></g>"##;
            }
            b += &format!(
                r##"<path d="M26 230 C 12 232, 6 220, 14 214 C 21 210, 28 217, 23 222" fill="none" stroke="{cl}"/>
<path d="M34 226 C 32 208, 58 202, 74 211 C 80 194, 115 189, 125 205 C 136 192, 170 194, 172 211 C 190 202, 214 210, 211 228 C 213 242, 193 248, 180 243 L 62 243 C 44 248, 32 239, 34 226 Z" fill="{cf}" stroke="{cl}"/>
<path d="M72 231 C 63 231, 61 222, 68 220 C 73 219, 75 225, 71 226" fill="none" stroke="{cl}" stroke-width="1.3"/>
<path d="M120 235 C 111 235, 109 226, 116 224 C 121 223, 123 229, 119 230" fill="none" stroke="{cl}" stroke-width="1.3"/>
<path d="M166 231 C 157 231, 155 222, 162 220 C 167 219, 169 225, 165 226" fill="none" stroke="{cl}" stroke-width="1.3"/>
<path d="M212 226 C 225 226, 230 215, 223 210 C 218 206, 212 213, 218 217" fill="none" stroke="{cl}"/>"##,
                cl = cloud_line,
                cf = cloud_fill
            );
            if s == 4 {
                b += r##"<path d="M60 245 L 51 260 L 59 259 L 53 274 L 70 255 L 62 256 L 68 245 Z" fill="#dca451" stroke-width="1.2"/>
<path d="M176 245 L 169 258 L 177 257 L 173 271 L 187 253 L 179 254 L 185 245 Z" fill="#dca451" stroke-width="1.2"/>"##;
            }
        }
        "hairback" => {
            if s >= 3 {
                b += r##"<path d="M83 88 C 78 48, 162 48, 157 88 C 166 120, 169 148, 160 166 C 151 150, 153 132, 149 117 L 91 117 C 87 132, 89 150, 80 166 C 71 148, 74 120, 83 88 Z" fill="#2b211a"/>"##;
            }
        }
        "fly" => {
            if s == 5 {
                b += r##"<path d="M88 70 C 64 44, 46 52, 34 34 C 58 40, 72 34, 96 56 Z M152 70 C 176 44, 194 52, 208 32 C 184 40, 168 34, 144 56 Z" fill="#2b211a"/>"##;
            }
        }
        "torso" => {
            b += &format!(
                r##"<ellipse cx="120" cy="200" rx="62" ry="19" fill="{robe}"/>
<circle cx="120" cy="158" r="46" fill="{robe}"/>
<path d="M88 128 C 104 150, 112 176, 112 202" fill="none" stroke-width="1.2" opacity=".6"/>
<circle cx="122" cy="170" r="24" fill="#e9c39b"/>
<path d="M119 174 q3 3 6 0" fill="none" stroke-width="1.2"/>"##
            );
            if s >= 3 {
                b += r##"<path d="M78 140 C 80 121, 96 113, 104 117 L 100 195 C 87 195, 76 180, 74 160 Z" fill="#2b211a"/>
<path d="M162 140 C 160 121, 144 113, 136 117 L 142 195 C 155 195, 164 180, 166 160 Z" fill="#2b211a"/>
<circle cx="89" cy="141" r="1.8" fill="#cfc6b8" stroke="none"/><circle cx="91" cy="160" r="1.8" fill="#cfc6b8" stroke="none"/><circle cx="151" cy="141" r="1.8" fill="#cfc6b8" stroke="none"/><circle cx="149" cy="160" r="1.8" fill="#cfc6b8" stroke="none"/>"##;
            }
            if s >= 4 {
                b += r##"<path d="M78 125 L 69 114 L 84 119 Z M85 117 L 82 103 L 93 113 Z M162 125 L 171 114 L 156 119 Z M155 117 L 158 103 L 147 113 Z" fill="#cfc6b8" stroke-width="1.2"/>"##;
            }
        }
        "head" => {
            b += &format!(
                r##"<ellipse cx="87" cy="98" rx="8" ry="13" fill="#e9c39b"/>
<ellipse cx="153" cy="98" rx="8" ry="13" fill="#e9c39b"/>
<circle cx="120" cy="90" r="34" fill="{face}"/>
<ellipse cx="101" cy="104" rx="6" ry="3.2" fill="#d98f7a" stroke="none" opacity="{blush}"/>
<ellipse cx="139" cy="104" rx="6" ry="3.2" fill="#d98f7a" stroke="none" opacity="{blush}"/>"##
            );
            if s >= 3 {
                b += r##"<path d="M87 84 C 89 52, 151 52, 153 84 C 139 68, 101 68, 87 84 Z" fill="#2b211a"/>"##;
            }
            if s == 2 {
                b += r##"<path d="M87 72 C 104 63, 136 63, 153 72 L 153 78 C 136 70, 104 70, 87 78 Z" fill="#b5452f" stroke-width="1.2"/><path d="M153 75 L 167 70 M153 77 L 165 84" fill="none" stroke="#b5452f" stroke-width="3"/>"##;
            }
            if s >= 4 {
                b += r##"<path d="M99 81 L 112 85 L 110 101 L 105 96 Z M141 81 L 128 85 L 130 101 L 135 96 Z" fill="#1d1612" stroke-width="1"/>
<path d="M113 116 L 111 123 M120 117 L 120 125 M127 116 L 129 123" fill="none" stroke-width="1.4"/>"##;
            }
            let eyes_closed = s == 1 && !sad;
            let eyes_open = (s >= 3 && !(s == 3 && happy)) || (s == 1 && sad);
            let shades = s == 2 || (s == 3 && happy);
            if eyes_closed {
                b += r##"<path d="M103 92 q6 4.5 12 0 M125 92 q6 4.5 12 0" fill="none" stroke-width="1.8"/>"##;
            }
            if eyes_open {
                b += r##"<ellipse cx="109" cy="92" rx="2.4" ry="2.9" fill="#2b211a" stroke="none"/><ellipse cx="131" cy="92" rx="2.4" ry="2.9" fill="#2b211a" stroke="none"/>"##;
            }
            if shades {
                b += r##"<path d="M97 87 h21 v6 q0 8 -10.5 8 q-10.5 0 -10.5 -8 Z M122 87 h21 v6 q0 8 -10.5 8 q-10.5 0 -10.5 -8 Z M118 89 h4" fill="#1d1612" stroke-width="1.2"/>"##;
            }
            if sad {
                b += r##"<path d="M101 81 l11 3 M139 81 l-11 3" fill="none" stroke-width="1.5"/><path d="M139 101 c-2.5 5 2.5 7.5 3.5 3.5 c0 -1.6 -1.6 -3.5 -3.5 -3.5 Z" fill="#9cc3cc" stroke-width="1"/>"##;
            }
            let smile = !sad && !happy && s <= 2;
            let grin = (happy && s <= 3) || (!sad && s == 3);
            let shout = !sad && s >= 4;
            if smile {
                b += r##"<path d="M110 108 q10 7 20 0" fill="none" stroke-width="1.8"/>"##;
            }
            if grin {
                b += r##"<path d="M108 106 q12 13 24 0 Z" fill="#6e2a1e" stroke-width="1.4"/>"##;
            }
            if shout {
                b += r##"<ellipse cx="120" cy="112" rx="7.5" ry="9" fill="#6e2a1e" stroke-width="1.4"/>"##;
            }
            if sad {
                b += r##"<path d="M113 112 q7 4 14 0" fill="none" stroke-width="1.6"/>"##;
            }
        }
        "front" => {
            match s {
                1 => {
                    b += r##"<path d="M60 150 L 132 172" stroke-width="7" stroke="#6b4423"/>
<path d="M50 145 L 63 149 L 61 156 L 48 152 Z" fill="#6b4423" stroke-width="1.2"/>
<circle cx="134" cy="174" r="19" fill="#b9824e"/><circle cx="157" cy="188" r="25" fill="#b9824e"/>
<circle cx="146" cy="180" r="6.5" fill="#3a2a1e"/><circle cx="66" cy="152" r="7" fill="#e9c39b"/><circle cx="150" cy="196" r="7" fill="#e9c39b"/>"##
                }
                2..=4 => {
                    let gc = if s == 4 { "#2b2a36" } else { "#2f6f6a" };
                    b += &format!(
                        r##"<path d="M56 148 L 136 174" stroke-width="6" stroke="#4e3420"/>
<path d="M44 141 L 59 147 L 57 154 L 42 148 Z" fill="#1d1612" stroke-width="1.2"/>
<path d="M128 164 C 140 151, 165 160, 169 174 C 185 178, 183 203, 164 205 C 150 213, 129 203, 133 190 C 121 184, 121 172, 128 164 Z" fill="{gc}"/>
<path d="M141 177 L 161 184 M139 186 L 159 193" fill="none" stroke="#efe5d3" stroke-width="1.6"/>
<circle cx="62" cy="150" r="7" fill="#e9c39b"/><circle cx="150" cy="200" r="7" fill="#e9c39b"/>"##
                    )
                }
                _ => {
                    b += r##"<path d="M58 131 L 138 172" stroke-width="6" stroke="#4e3420"/>
<path d="M46 123 L 61 129 L 57 136 L 42 130 Z" fill="#1d1612" stroke-width="1.2"/>
<path d="M128 162 L 195 160 L 183 175 L 150 179 L 189 203 L 170 211 L 124 182 Z" fill="#b5452f"/>
<path d="M150 170 C 160 164, 168 172, 176 166 M146 184 C 156 186, 162 196, 172 196" fill="none" stroke="#dca451" stroke-width="1.6"/>
<circle cx="64" cy="133" r="7" fill="#e9c39b"/><circle cx="148" cy="190" r="7" fill="#e9c39b"/>"##
                }
            }
        }
        "arm" => {
            if !sad {
                // Sleeve from the shoulder (166,136); on stages 1-2 it starts inside the
                // round robe. Wrist: red sweatband (stage 2, like the headband), studded
                // black band (stages 4-5).
                let from = if s <= 2 { "M160 142" } else { "M166 136" };
                b += &format!(
                    r##"<path d="{from} L 182 94" stroke-width="8" stroke="{robe}"/>
<circle cx="184" cy="88" r="8.5" fill="#e9c39b"/>
<path d="M179 82 L 175 64 M190 82 L 195 64" stroke-width="5" stroke="#e9c39b"/>
<path d="M176.5 81 L 173 64 M192.5 81 L 197 64" fill="none" stroke-width="1"/>"##
                );
                if s == 2 {
                    b += r##"<path d="M171.5 104.7 L 182.7 108.9 L 184.5 104.3 L 173.3 100.1 Z" fill="#b5452f" stroke-width="1.2"/>"##;
                }
                if s >= 4 {
                    b += r##"<path d="M171.5 104.7 L 182.7 108.9 L 184.5 104.3 L 173.3 100.1 Z" fill="#1d1612" stroke-width="1.2"/>
<circle cx="174.6" cy="103.4" r="1.3" fill="#cfc6b8" stroke="none"/><circle cx="178" cy="104.6" r="1.3" fill="#cfc6b8" stroke="none"/><circle cx="181.4" cy="105.9" r="1.3" fill="#cfc6b8" stroke="none"/>"##;
                }
            }
        }
        "over" => {
            if s <= 2 && !sad {
                // Floating note on the left: the raised hand is on the right.
                b += r##"<path transform="translate(-160 0)" d="M196 72 v-16 l9 -2.5 v16 M196 72 a3.5 2.6 0 1 1 -1 -1 M205 69.5 a3.5 2.6 0 1 1 -1 -1" fill="#1f7a6e" stroke="#1f7a6e" stroke-width="1.3"/>"##;
            }
            if happy {
                b += r##"<g stroke-width="1"><path d="M34 44 c4 -6 10 -4 8 2 c-2 5 -8 4 -8 -2 Z" fill="#dca451"/>
<path d="M204 40 c4 -6 10 -4 8 2 c-2 5 -8 4 -8 -2 Z" fill="#c27a2c"/><path d="M62 18 c3 -4 7 -3 6 1 c-1 4 -6 3 -6 -1 Z" fill="#b5452f"/>
<path d="M182 16 c3 -4 7 -3 6 1 c-1 4 -6 3 -6 -1 Z" fill="#dca451"/><path d="M20 118 c3 -4 7 -3 6 1 c-1 4 -6 3 -6 -1 Z" fill="#c27a2c"/>
<path d="M218 116 c3 -4 7 -3 6 1 c-1 4 -6 3 -6 -1 Z" fill="#dca451"/></g>"##;
            }
            if sad {
                b += r##"<path d="M190 150 C 185 139, 190 130, 196 126 C 202 130, 207 139, 202 150 Z" fill="#e3b3bf"/>
<path d="M190 150 C 181 146, 178 137, 180 133 C 187 135, 192 141, 196 150 Z M202 150 C 211 146, 214 137, 212 133 C 205 135, 200 141, 196 150 Z" fill="#cf8ea0"/>
<path d="M182 152 h28" stroke="#1f7a6e" stroke-width="2.2"/>"##;
            }
        }
        _ => unreachable!(),
    }
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
<defs><filter id="gbrush" x="-5%" y="-5%" width="110%" height="110%"><feTurbulence type="fractalNoise" baseFrequency="0.05" numOctaves="2" seed="7"/><feDisplacementMap in="SourceGraphic" scale="1.1"/></filter></defs>
<g filter="url(#gbrush)" stroke="#2b211a" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round">{b}</g></svg>"##
    )
}

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mascot");
    std::fs::create_dir_all(&out).unwrap();
    let opt = resvg::usvg::Options::default();
    let mut n = 0;
    for stage in 1..=5 {
        for (mood, mname) in [(Mood::Neutral, "neutral"), (Mood::Happy, "happy"), (Mood::Sad, "sad")] {
            for name in ["back", "hairback", "fly", "torso", "head", "front", "arm", "over"] {
                let svg = layer(stage, mood, name);
                let tree = resvg::usvg::Tree::from_str(&svg, &opt).unwrap_or_else(|e| panic!("{stage} {mname} {name}: {e}"));
                let mut pix = resvg::tiny_skia::Pixmap::new((W as f32 * SCALE) as u32, (H as f32 * SCALE) as u32).unwrap();
                resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(SCALE, SCALE), &mut pix.as_mut());
                pix.save_png(out.join(format!("s{stage}_{mname}_{name}.png"))).unwrap();
                n += 1;
            }
        }
    }
    println!("wrote {n} layers to {}", out.display());
}
