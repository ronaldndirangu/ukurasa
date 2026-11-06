//! Canonical page geometry and transforms (page boxes, /Rotate, /UserUnit).
//!
//! Display coordinates are points with a top-left origin, measured on the page as shown
//! after its crop box and `/Rotate` are applied. User coordinates are PDF user space.

/// An axis-aligned rectangle with `x0 <= x1` and `y0 <= y1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    /// US Letter, the box PDFium uses when a page has no usable `/MediaBox`.
    pub const LETTER: Rect = Rect {
        x0: 0.0,
        y0: 0.0,
        x1: 612.0,
        y1: 792.0,
    };

    /// Builds a normalized rectangle from two opposite corners.
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    pub fn is_empty(&self) -> bool {
        !(self.width() > 0.0 && self.height() > 0.0)
    }

    pub fn intersect(&self, other: &Rect) -> Rect {
        let rect = Rect {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        if rect.x0 > rect.x1 || rect.y0 > rect.y1 {
            Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 0.0,
                y1: 0.0,
            }
        } else {
            rect
        }
    }

    /// The smallest rectangle containing all `points`.
    pub fn bounding(points: &[(f64, f64)]) -> Rect {
        let mut rect = Rect {
            x0: f64::INFINITY,
            y0: f64::INFINITY,
            x1: f64::NEG_INFINITY,
            y1: f64::NEG_INFINITY,
        };
        for &(x, y) in points {
            rect.x0 = rect.x0.min(x);
            rect.y0 = rect.y0.min(y);
            rect.x1 = rect.x1.max(x);
            rect.y1 = rect.y1.max(y);
        }
        rect
    }
}

/// Clockwise page rotation applied for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    /// Interprets a `/Rotate` value the way PDFium does: whole quarter turns, any sign.
    pub fn from_degrees(degrees: i64) -> Self {
        match ((degrees / 90) % 4 + 4) % 4 {
            1 => Self::R90,
            2 => Self::R180,
            3 => Self::R270,
            _ => Self::R0,
        }
    }
}

/// The visible area of a page and how it is turned for display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    pub crop: Rect,
    pub rotation: Rotation,
}

impl PageGeometry {
    /// Resolves the visible box from raw page attributes, matching PDFium: an empty media box
    /// becomes US Letter, and the crop box is clipped to the media box.
    pub fn new(media_box: Option<Rect>, crop_box: Option<Rect>, rotate: i64) -> Self {
        let media = media_box.filter(|r| !r.is_empty()).unwrap_or(Rect::LETTER);
        let crop = match crop_box.filter(|r| !r.is_empty()) {
            Some(crop) => {
                let clipped = crop.intersect(&media);
                if clipped.is_empty() { media } else { clipped }
            }
            None => media,
        };
        Self {
            crop,
            rotation: Rotation::from_degrees(rotate),
        }
    }

    /// Width and height in points as displayed.
    pub fn display_size(&self) -> (f64, f64) {
        match self.rotation {
            Rotation::R0 | Rotation::R180 => (self.crop.width(), self.crop.height()),
            Rotation::R90 | Rotation::R270 => (self.crop.height(), self.crop.width()),
        }
    }

    /// Maps a display point to user space.
    pub fn to_user(&self, u: f64, v: f64) -> (f64, f64) {
        let Rect { x0, y0, x1, y1 } = self.crop;
        match self.rotation {
            Rotation::R0 => (x0 + u, y1 - v),
            Rotation::R90 => (x0 + v, y0 + u),
            Rotation::R180 => (x1 - u, y0 + v),
            Rotation::R270 => (x1 - v, y1 - u),
        }
    }

    /// Maps a user-space point to display coordinates; the inverse of [`Self::to_user`].
    pub fn to_display(&self, x: f64, y: f64) -> (f64, f64) {
        let Rect { x0, y0, x1, y1 } = self.crop;
        match self.rotation {
            Rotation::R0 => (x - x0, y1 - y),
            Rotation::R90 => (y - y0, x - x0),
            Rotation::R180 => (x1 - x, y - y0),
            Rotation::R270 => (y1 - y, x1 - x),
        }
    }

    /// Maps a display rectangle (top-left corner and size) to user space.
    pub fn rect_to_user(&self, x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect::bounding(&[
            self.to_user(x, y),
            self.to_user(x + width, y),
            self.to_user(x, y + height),
            self.to_user(x + width, y + height),
        ])
    }

    /// The form `/Matrix` that keeps an appearance drawn in display orientation upright on
    /// screen once the page is rotated.
    pub fn upright_matrix(&self) -> [f64; 6] {
        match self.rotation {
            Rotation::R0 => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            Rotation::R90 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
            Rotation::R180 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
            Rotation::R270 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Applies an affine matrix `[a b c d e f]` to a point.
pub fn transform(m: &[f64; 6], x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

#[cfg(test)]
mod tests {
    use super::*;

    const CROP: Rect = Rect {
        x0: 10.0,
        y0: 20.0,
        x1: 210.0,
        y1: 320.0,
    };

    fn geometry(rotate: i64) -> PageGeometry {
        PageGeometry::new(Some(Rect::new(0.0, 0.0, 400.0, 400.0)), Some(CROP), rotate)
    }

    #[test]
    fn rotation_normalizes_like_pdfium() {
        assert_eq!(Rotation::from_degrees(0), Rotation::R0);
        assert_eq!(Rotation::from_degrees(90), Rotation::R90);
        assert_eq!(Rotation::from_degrees(-90), Rotation::R270);
        assert_eq!(Rotation::from_degrees(450), Rotation::R90);
        assert_eq!(Rotation::from_degrees(45), Rotation::R0);
        assert_eq!(Rotation::from_degrees(-180), Rotation::R180);
    }

    #[test]
    fn boxes_fall_back_and_clip() {
        let letter = PageGeometry::new(None, None, 0);
        assert_eq!(letter.crop, Rect::LETTER);
        let empty_media = PageGeometry::new(Some(Rect::new(0.0, 0.0, 0.0, 0.0)), None, 0);
        assert_eq!(empty_media.crop, Rect::LETTER);
        let clipped = PageGeometry::new(
            Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
            Some(Rect::new(50.0, 50.0, 200.0, 200.0)),
            0,
        );
        assert_eq!(clipped.crop, Rect::new(50.0, 50.0, 100.0, 100.0));
        let outside = PageGeometry::new(
            Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
            Some(Rect::new(150.0, 150.0, 200.0, 200.0)),
            0,
        );
        assert_eq!(outside.crop, Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn display_size_swaps_for_quarter_turns() {
        assert_eq!(geometry(0).display_size(), (200.0, 300.0));
        assert_eq!(geometry(90).display_size(), (300.0, 200.0));
        assert_eq!(geometry(180).display_size(), (200.0, 300.0));
        assert_eq!(geometry(270).display_size(), (300.0, 200.0));
    }

    #[test]
    fn display_corners_map_to_crop_corners() {
        for rotate in [0, 90, 180, 270] {
            let g = geometry(rotate);
            let (w, h) = g.display_size();
            let corners = [
                g.to_user(0.0, 0.0),
                g.to_user(w, 0.0),
                g.to_user(0.0, h),
                g.to_user(w, h),
            ];
            assert_eq!(Rect::bounding(&corners), CROP, "rotate {rotate}");
        }
        assert_eq!(geometry(0).to_user(0.0, 0.0), (10.0, 320.0));
        assert_eq!(geometry(90).to_user(0.0, 0.0), (10.0, 20.0));
        assert_eq!(geometry(180).to_user(0.0, 0.0), (210.0, 20.0));
        assert_eq!(geometry(270).to_user(0.0, 0.0), (210.0, 320.0));
    }

    #[test]
    fn to_display_inverts_to_user() {
        for rotate in [0, 90, 180, 270] {
            let g = geometry(rotate);
            for (u, v) in [(0.0, 0.0), (12.5, 40.0), (150.0, 7.0)] {
                let (x, y) = g.to_user(u, v);
                assert_eq!(g.to_display(x, y), (u, v), "rotate {rotate}");
            }
        }
    }

    #[test]
    fn upright_matrix_maps_form_box_onto_user_rect() {
        for rotate in [0, 90, 180, 270] {
            let g = geometry(rotate);
            let (x, y, w, h) = (30.0, 40.0, 50.0, 20.0);
            let rect = g.rect_to_user(x, y, w, h);
            let m = g.upright_matrix();
            let form = Rect::bounding(&[
                transform(&m, 0.0, 0.0),
                transform(&m, w, 0.0),
                transform(&m, 0.0, h),
                transform(&m, w, h),
            ]);
            assert_eq!(form.width(), rect.width(), "rotate {rotate}");
            assert_eq!(form.height(), rect.height(), "rotate {rotate}");
            // The form's top-left (0, h) must land on the display rect's top-left corner.
            let (tx, ty) = transform(&m, 0.0, h);
            let mapped = (rect.x0 + tx - form.x0, rect.y0 + ty - form.y0);
            assert_eq!(mapped, g.to_user(x, y), "rotate {rotate}");
        }
    }
}
