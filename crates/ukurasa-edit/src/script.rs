//! Typed signatures: text set in a bundled script font and converted to vector outlines.

use rustybuzz::ttf_parser::{GlyphId, OutlineBuilder};

use crate::path::{self, Segment};
use crate::{EditError, VectorPath};

/// Dancing Script, licensed under the SIL Open Font License 1.1 (see `fonts/OFL.txt`).
static FONT: &[u8] = include_bytes!("../fonts/DancingScript.ttf");

/// Nominal font size of the returned outlines, in points.
const SIZE: f64 = 72.0;

/// Shapes `text` and returns its glyph outlines as a filled path in a y-down box.
pub fn typed_signature(text: &str) -> Result<VectorPath, EditError> {
    let text = text.trim();
    let face = rustybuzz::Face::from_slice(FONT, 0).expect("bundled font parses");
    let missing: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && face.glyph_index(*c).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(EditError::UnsupportedSignatureText(missing));
    }

    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    let shaped = rustybuzz::shape(&face, &[], buffer);
    let scale = SIZE / f64::from(face.units_per_em());
    let ascender = f64::from(face.ascender());

    let mut outline = Outline {
        segments: Vec::new(),
        origin: (0.0, 0.0),
        scale,
        ascender,
    };
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut pen = 0.0;
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        outline.origin = (pen + f64::from(pos.x_offset), f64::from(pos.y_offset));
        let glyph = GlyphId(info.glyph_id as u16);
        if let Some(b) = face.outline_glyph(glyph, &mut outline) {
            let (left, top) = outline.map(f64::from(b.x_min), f64::from(b.y_max));
            let (right, bottom) = outline.map(f64::from(b.x_max), f64::from(b.y_min));
            x0 = x0.min(left);
            y0 = y0.min(top);
            x1 = x1.max(right);
            y1 = y1.max(bottom);
        }
        pen += f64::from(pos.x_advance);
    }
    if outline.segments.is_empty() {
        return Err(EditError::EmptySignature);
    }

    let margin = SIZE * 0.05;
    let segments = path::translate(&outline.segments, margin - x0, margin - y0);
    Ok(VectorPath {
        d: path::to_svg(&segments),
        width: x1 - x0 + 2.0 * margin,
        height: y1 - y0 + 2.0 * margin,
        fill: true,
        stroke_width: 0.0,
    })
}

struct Outline {
    segments: Vec<Segment>,
    origin: (f64, f64),
    scale: f64,
    ascender: f64,
}

impl Outline {
    fn map(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (self.origin.0 + x) * self.scale,
            (self.ascender - self.origin.1 - y) * self.scale,
        )
    }
}

impl OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x.into(), y.into());
        self.segments.push(Segment::Move(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x.into(), y.into());
        self.segments.push(Segment::Line(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1.into(), y1.into());
        let (x, y) = self.map(x.into(), y.into());
        self.segments.push(Segment::Quad(x1, y1, x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1.into(), y1.into());
        let (x2, y2) = self.map(x2.into(), y2.into());
        let (x, y) = self.map(x.into(), y.into());
        self.segments.push(Segment::Cubic(x1, y1, x2, y2, x, y));
    }

    fn close(&mut self) {
        self.segments.push(Segment::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlines_a_name() {
        let path = typed_signature("Jane Doe").unwrap();
        assert!(path.fill);
        assert!(path.width > path.height && path.height > 20.0);
        let segments = path::parse(&path.d).unwrap();
        assert!(segments.len() > 20);
        for segment in segments {
            if let Segment::Move(x, y) | Segment::Line(x, y) = segment {
                assert!((0.0..=path.width).contains(&x), "x {x}");
                assert!((0.0..=path.height).contains(&y), "y {y}");
            }
        }
    }

    #[test]
    fn rejects_empty_and_unsupported_text() {
        assert!(matches!(
            typed_signature("   "),
            Err(EditError::EmptySignature)
        ));
        assert!(matches!(
            typed_signature("\u{4e2d}\u{6587}"),
            Err(EditError::UnsupportedSignatureText(_))
        ));
    }
}
