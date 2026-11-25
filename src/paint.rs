//! The cube's six-face painting: the palette, the brush, the payload's digit alphabet, and the
//! thin slabs the client draws each run of pixels with. A painting is 32 by 32 pixels per face,
//! one digit per pixel in the payload and in the block entity, sixteen colours plus bare.

use crate::gui_paint;
use crate::{Config, inside};
use mod_api::{BlockEntityDrawContext, Compound, CubeFace, Tag};

/// The white sprite painted pixels are tinted from: flat white, so a run of pixels takes the
/// palette's colour and nothing else.
pub(crate) const PAINT_SPRITE: &str = "strange_structure:block/paint_white";
/// The painting is this many pixels square on each of the cube's six faces.
pub(crate) const CANVAS: usize = 32;
/// One painted pixel per canvas cell per face.
pub(crate) const PAINT_LEN: usize = CANVAS * CANVAS * 6;
/// How far outside a face a painted pixel stands, in blocks, so it wins the depth test.
pub(crate) const PAINT_LIFT: f32 = 0.01;
/// The palette, 0xRRGGBB: sixteen colours, the greys and the hues.
pub(crate) const COLORS: [u32; 16] = [
    0xFF_FF_FF, 0xC6_C6_C6, 0x7F_7F_7F, 0x20_20_20, 0xE0_3C_3C, 0xF0_8A_28, 0xF5_D7_40, 0x7A_CB_4A,
    0x2E_8B_3D, 0x35_C9_C4, 0x5B_9B_F0, 0x2A_4B_D7, 0x8A_4B_D7, 0xD6_4B_B0, 0xF2_A2_C0, 0x7A_4A_24,
];
/// The six faces, in the order the painting stores them: north, south, west, east, up, down.
pub(crate) const FACE_NAMES: [&str; 6] = ["北", "南", "西", "东", "上", "下"];
/// How many pixels across one press paints.
pub(crate) const BRUSHES: [i32; 3] = [1, 3, 5];
pub(crate) const BRUSH_NAMES: [&str; 3] = ["细", "中", "粗"];
/// The paint payload's alphabet: `0` is a bare face, `1` to `g` the palette's colours.
pub(crate) const PAINT_DIGITS: [u8; 17] = *b"0123456789abcdefg";
/// One painted pixel's size on screen: the canvas is CANVAS cells across.
pub(crate) const CANVAS_CELL: f32 = gui_paint::CANVAS[2] / CANVAS as f32;

/// The page's canvas rectangle.
pub(crate) fn canvas_rect(x: f32, y: f32) -> [f32; 4] {
    gui_paint::at(x, y, gui_paint::CANVAS)
}

/// The pixel under the cursor on the canvas, if it is over one.
pub(crate) fn cell_at(rect: [f32; 4], at: [f32; 2]) -> Option<[i32; 2]> {
    if !inside(rect, at) {
        return None;
    }
    let column = ((at[0] - rect[0]) / CANVAS_CELL) as i32;
    let row = ((at[1] - rect[1]) / CANVAS_CELL) as i32;
    Some([
        column.clamp(0, CANVAS as i32 - 1),
        row.clamp(0, CANVAS as i32 - 1),
    ])
}

/// Stamps one dot of the brush, keeping the square inside the face.
pub(crate) fn paint_dot(paint: &mut [u8], face: usize, x: i32, y: i32, brush: i32, color: u8) {
    let half = brush / 2;
    let base = face * CANVAS * CANVAS;
    for offset_y in -half..=half {
        for offset_x in -half..=half {
            let (px, py) = (x + offset_x, y + offset_y);
            if px < 0 || py < 0 || px >= CANVAS as i32 || py >= CANVAS as i32 {
                continue;
            }
            paint[base + py as usize * CANVAS + px as usize] = color;
        }
    }
}

/// Stamps the brush along the line between two cells, so a fast drag leaves no gaps.
pub(crate) fn paint_line(
    paint: &mut [u8],
    face: usize,
    from: [i32; 2],
    to: [i32; 2],
    brush: i32,
    color: u8,
) {
    let (mut x, mut y) = (from[0], from[1]);
    let (end_x, end_y) = (to[0], to[1]);
    let span_x = (end_x - x).abs();
    let sign_x = if x < end_x { 1 } else { -1 };
    let span_y = -(end_y - y).abs();
    let sign_y = if y < end_y { 1 } else { -1 };
    let mut error = span_x + span_y;
    loop {
        paint_dot(paint, face, x, y, brush, color);
        if x == end_x && y == end_y {
            break;
        }
        let doubled = 2 * error;
        if doubled >= span_y {
            error += span_y;
            x += sign_x;
        }
        if doubled <= span_x {
            error += span_x;
            y += sign_y;
        }
    }
}

/// The six faces' text of one config, in face order.
pub(crate) fn paint_text(config: &Config) -> [&str; 6] {
    [
        &config.p0, &config.p1, &config.p2, &config.p3, &config.p4, &config.p5,
    ]
}

/// The painting as the per-face text the block entity and the payload store, one digit per
/// pixel and face.
pub(crate) fn encode_paint(paint: &[u8]) -> [String; 6] {
    std::array::from_fn(|face| {
        let pixels = &paint[face * CANVAS * CANVAS..(face + 1) * CANVAS * CANVAS];
        let mut out = String::with_capacity(CANVAS * CANVAS);
        for &value in pixels {
            let digit = PAINT_DIGITS[(value as usize).min(PAINT_DIGITS.len() - 1)];
            out.push(char::from(digit));
        }
        out
    })
}

/// Reads stored text back, answering a blank painting when it is not ours.
pub(crate) fn decode_paint(faces: [&str; 6]) -> Vec<u8> {
    let mut paint = vec![0u8; PAINT_LEN];
    if !paint_valid(faces) {
        return paint;
    }
    for (face, text) in faces.iter().enumerate() {
        let pixels = &mut paint[face * CANVAS * CANVAS..(face + 1) * CANVAS * CANVAS];
        for (slot, byte) in pixels.iter_mut().zip(text.bytes()) {
            *slot = PAINT_DIGITS
                .iter()
                .position(|digit| *digit == byte)
                .unwrap_or(0) as u8;
        }
    }
    paint
}

/// Whether every face is a painting this mod wrote: one digit per pixel and nothing else.
pub(crate) fn paint_valid(faces: [&str; 6]) -> bool {
    faces.iter().all(|text| {
        text.len() == CANVAS * CANVAS && text.bytes().all(|byte| PAINT_DIGITS.contains(&byte))
    })
}

/// The painting's pixels as rectangles: each row's run is merged, then grown down while the rows
/// match, so a face costs a few quads instead of a thousand.
pub(crate) fn runs(pixels: &[u8]) -> Vec<(usize, usize, usize, usize, u8)> {
    let mut used = [false; CANVAS * CANVAS];
    let mut rects = Vec::new();
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let color = pixels[y * CANVAS + x];
            if used[y * CANVAS + x] || color == 0 {
                continue;
            }
            let mut width = 1;
            while x + width < CANVAS {
                let index = y * CANVAS + x + width;
                if used[index] || pixels[index] != color {
                    break;
                }
                width += 1;
            }
            let mut height = 1;
            'grow: while y + height < CANVAS {
                for step in 0..width {
                    let index = (y + height) * CANVAS + x + step;
                    if used[index] || pixels[index] != color {
                        break 'grow;
                    }
                }
                height += 1;
            }
            for row in 0..height {
                for step in 0..width {
                    used[(y + row) * CANVAS + x + step] = true;
                }
            }
            rects.push((x, y, width, height, color));
        }
    }
    rects
}

/// Reads the painting out of the block entity data, if it has one.
pub(crate) fn read_paint(data: &Compound) -> Option<Vec<u8>> {
    let mut paint = vec![0u8; PAINT_LEN];
    for face in 0..6 {
        let Some(Tag::String(text)) = data.get(&format!("p{face}").as_str().into()) else {
            continue;
        };
        let text = text.to_string_lossy();
        if text.len() != CANVAS * CANVAS {
            continue;
        }
        for (slot, byte) in paint[face * CANVAS * CANVAS..(face + 1) * CANVAS * CANVAS]
            .iter_mut()
            .zip(text.bytes())
        {
            if let Some(index) = PAINT_DIGITS.iter().position(|&digit| digit == byte) {
                *slot = index as u8;
            }
        }
    }
    paint.iter().any(|&value| value != 0).then_some(paint)
}

/// Draws a painting's pixels over the six faces of the unit box the pose scales: each run of
/// pixels is a thin slab just outside its face, textured with the plain white sprite and tinted,
/// so the game bakes it as it bakes a model element and no face is lit from behind.
pub(crate) fn draw_paint(draw: &mut dyn BlockEntityDrawContext, paint: &[u8]) {
    let step = 16.0 / CANVAS as f32;
    let lift = PAINT_LIFT * 16.0;
    for face in 0..6 {
        let pixels = &paint[face * CANVAS * CANVAS..(face + 1) * CANVAS * CANVAS];
        for (px, py, width, height, color) in runs(pixels) {
            let u0 = px as f32 * step;
            let u1 = (px + width) as f32 * step;
            let v0 = py as f32 * step;
            let v1 = (py + height) as f32 * step;
            // Each face reads the painting as someone standing outside it does: x to the right,
            // y down. Down, up, north, south, west, east is the order a cube takes its faces in.
            let (from, to, side) = match face {
                0 => (
                    [16.0 - u1, 16.0 - v1, -lift],
                    [16.0 - u0, 16.0 - v0, 0.0],
                    2,
                ),
                1 => ([u0, 16.0 - v1, 16.0], [u1, 16.0 - v0, 16.0 + lift], 3),
                2 => ([-lift, 16.0 - v1, u0], [0.0, 16.0 - v0, u1], 4),
                3 => (
                    [16.0, 16.0 - v1, 16.0 - u1],
                    [16.0 + lift, 16.0 - v0, 16.0 - u0],
                    5,
                ),
                4 => ([u0, 16.0, v0], [u1, 16.0 + lift, v1], 1),
                _ => ([u0, -lift, 16.0 - v1], [u1, 0.0, 16.0 - v0], 0),
            };
            let mut faces: [Option<CubeFace<'_>>; 6] = [None; 6];
            faces[side] = Some(CubeFace::new(PAINT_SPRITE));
            let _ = draw.cube(from, to, faces, COLORS[color as usize - 1]);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn six(faces: &[String; 6]) -> [&str; 6] {
        [
            faces[0].as_str(),
            faces[1].as_str(),
            faces[2].as_str(),
            faces[3].as_str(),
            faces[4].as_str(),
            faces[5].as_str(),
        ]
    }

    #[test]
    fn a_painting_survives_the_round_trip() {
        let mut paint = vec![0u8; PAINT_LEN];
        paint_dot(&mut paint, 0, 3, 4, 1, 5);
        paint_dot(&mut paint, 5, 31, 0, 1, 16);
        let faces = encode_paint(&paint);
        assert!(paint_valid(six(&faces)));
        assert_eq!(decode_paint(six(&faces)), paint);
    }

    #[test]
    fn a_painting_of_the_wrong_shape_is_bare() {
        let bad = ["0"; 6];
        assert!(!paint_valid(bad));
        assert_eq!(decode_paint(bad), vec![0u8; PAINT_LEN]);
    }

    #[test]
    fn a_brush_is_clipped_at_the_edge() {
        let mut paint = vec![0u8; PAINT_LEN];
        paint_dot(&mut paint, 0, 0, 0, 5, 3);
        let painted = paint.iter().filter(|value| **value == 3).count();
        assert_eq!(painted, 9);
    }

    #[test]
    fn a_face_costs_one_rectangle_per_block_of_colour() {
        let mut pixels = vec![0u8; CANVAS * CANVAS];
        for y in 2..7 {
            for x in 4..12 {
                pixels[y * CANVAS + x] = 5;
            }
        }
        assert_eq!(runs(&pixels), vec![(4, 2, 8, 5, 5)]);
        pixels[5 * CANVAS + 8] = 9;
        let rects = runs(&pixels);
        assert!(rects.len() > 1);
        assert!(rects.iter().all(|(_, _, width, height, _)| *width > 0 && *height > 0));
    }
}

