//! Writes styled text: in place of the show operators of a line, or as new text on a page.

use lopdf::content::Operation;
use lopdf::{Object, StringFormat};
use ukurasa_geom::PageGeometry;

use super::Span;
use super::faces::{Faces, Font};
use super::read::{Line, Matrix, Run, Scan, code_value, display_size, mul, real, translate};
use crate::standard::Standard;
use crate::{EditError, TextBox};

/// Word space, in thousandths of an em, for a font without a space glyph on a line that has no
/// word space to copy.
const DEFAULT_SPACE: f64 = -250.0;

/// Width of a synthesized bold stroke, in ems.
const BOLD_STROKE: f64 = 0.03;

/// Shear of a synthesized italic: tan 12°.
const ITALIC_SLANT: f64 = 0.2126;

/// Distance of an underline's top below the baseline, and its thickness, in ems.
const UNDERLINE_OFFSET: f64 = 0.1;
const UNDERLINE_THICKNESS: f64 = 0.05;

/// The operators that replace a line's show operators.
pub(super) struct LineChange {
    pub(super) ops: Vec<(usize, Vec<Operation>)>,
    /// Underlines to draw after the page's content.
    pub(super) underlines: Vec<Operation>,
    /// Characters drawn in a standard font because the chosen font lacks them.
    pub(super) fallback: String,
    /// How far the new text reaches past the end of the line, in display points.
    pub(super) overflow: f64,
}

enum Piece {
    Glyph(Vec<u8>),
    Kern(f64),
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    face: usize,
    tfs: f64,
    color: [u8; 3],
    bold: bool,
    italic: bool,
    underline: bool,
}

struct Segment {
    style: Style,
    pieces: Vec<Piece>,
}

fn add(segments: &mut Vec<Segment>, style: Style, piece: Piece) {
    match segments.last_mut() {
        Some(last) if last.style == style || matches!(piece, Piece::Kern(_)) => {
            last.pieces.push(piece)
        }
        _ => segments.push(Segment {
            style,
            pieces: vec![piece],
        }),
    }
}

fn extend_unique(target: &mut String, c: char) {
    if !target.contains(c) {
        target.push(c);
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied()
}

/// The text state that new text is sized and spaced in.
struct Basis {
    /// The font size in text space units, and the size in display points it shows at.
    tfs: f64,
    size: f64,
    /// The text matrix already slants the glyphs.
    slant: bool,
    /// Word space, in thousandths of an em, for a font without a space glyph.
    space_kern: f64,
}

impl Basis {
    /// Text in a text space of display points.
    const PLAIN: Basis = Basis {
        tfs: 1.0,
        size: 1.0,
        slant: false,
        space_kern: DEFAULT_SPACE,
    };

    /// The text state of a line's first run, with the line's typical word space.
    fn of(line: &Line) -> Basis {
        let first = &line.runs[0];
        let space_kern = median(
            line.runs
                .iter()
                .flat_map(|r| r.space_kerns.iter().copied())
                .collect(),
        )
        .unwrap_or(DEFAULT_SPACE);
        Basis {
            tfs: first.tfs,
            size: first.size,
            slant: first.slant,
            space_kern,
        }
    }
}

/// Splits `spans` into segments of one face and style, choosing the faces that draw them.
fn segments(
    basis: &Basis,
    spans: &[Span],
    faces: &mut Faces,
    scan: &mut Scan,
    fallback: &mut String,
) -> Result<Vec<Segment>, EditError> {
    let mut segments = Vec::new();
    let mut missing = String::new();
    for span in spans.iter().filter(|s| !s.text.is_empty()) {
        let choice = faces.resolve(span, scan);
        let tfs = if (span.size - display_size(basis.size)).abs() < 1e-9 {
            basis.tfs
        } else {
            f64::from((basis.tfs * span.size / basis.size) as f32)
        };
        if !tfs.is_finite() {
            return Err(EditError::Invalid("a text size is out of range"));
        }
        let style = Style {
            face: choice.face,
            tfs,
            color: span.color,
            bold: choice.bold,
            italic: choice.italic && !basis.slant,
            underline: span.underline,
        };
        for c in span.text.chars() {
            if let Some(code) = faces.code(choice.face, c, scan) {
                add(&mut segments, style, Piece::Glyph(code));
                continue;
            }
            if c == ' ' {
                add(&mut segments, style, Piece::Kern(basis.space_kern));
                continue;
            }
            let standard = faces.standard(Standard {
                family: faces.font(choice.face).family,
                bold: span.bold,
                italic: span.italic,
            });
            match faces.code(standard, c, scan) {
                Some(code) => {
                    extend_unique(fallback, c);
                    let style = Style {
                        face: standard,
                        bold: false,
                        italic: false,
                        ..style
                    };
                    add(&mut segments, style, Piece::Glyph(code));
                }
                None => extend_unique(&mut missing, c),
            }
        }
    }
    if missing.is_empty() {
        Ok(segments)
    } else {
        Err(EditError::UnsupportedLineText(missing))
    }
}

fn op(operator: &str, operands: Vec<Object>) -> Operation {
    Operation::new(operator, operands)
}

fn matrix_operands(m: &Matrix) -> Vec<Object> {
    m.iter().map(|&v| Object::Real(v as f32)).collect()
}

fn fill_rgb(color: [u8; 3]) -> Operation {
    op(
        "rg",
        color.iter().map(|&c| real(f64::from(c) / 255.0)).collect(),
    )
}

fn stroke_rgb(color: [u8; 3]) -> Operation {
    op("RG", fill_rgb(color).operands)
}

/// A `TJ` that shows `pieces` from `pen`, moving it by `advance` for each glyph and by kerns
/// scaled by `scale`, and where the last glyph ends.
fn show(
    pieces: &[Piece],
    advance: impl Fn(&[u8]) -> f64,
    scale: f64,
    pen: &mut f64,
) -> (Operation, f64) {
    let mut items: Vec<Object> = Vec::new();
    let mut end = *pen;
    for piece in pieces {
        match piece {
            Piece::Glyph(code) => {
                *pen += advance(code);
                end = *pen;
                match items.last_mut() {
                    Some(Object::String(string, _)) => string.extend(code),
                    _ => items.push(Object::String(code.clone(), StringFormat::Hexadecimal)),
                }
            }
            Piece::Kern(n) => {
                *pen -= n / 1000.0 * scale;
                items.push(real(*n));
            }
        }
    }
    (op("TJ", vec![Object::Array(items)]), end)
}

/// Sets the font at the exact size, so text after a rewritten run doesn't shift.
fn set_font(name: &[u8], tfs: f64) -> Operation {
    op(
        "Tf",
        vec![Object::Name(name.to_vec()), Object::Real(tfs as f32)],
    )
}

fn stroked(mode: i64) -> bool {
    matches!(mode, 1 | 2 | 5 | 6)
}

/// The rendering mode of a segment in the style of a line in `mode`.
fn mode(mode: i64, bold: bool) -> i64 {
    match (mode, bold) {
        (0, true) => 2,
        (4, true) => 6,
        (2, false) => 0,
        (6, false) => 4,
        (mode, _) => mode,
    }
}

/// The fill and stroke color operators of a line.
struct Colors<'a> {
    runs: &'a [Run],
}

impl Colors<'_> {
    /// Operators that set the fill to `color`, copied from a run of the line in that color.
    fn fill(&self, color: [u8; 3]) -> Vec<Operation> {
        match self.runs.iter().find(|r| r.color == color) {
            Some(run) => original(&run.fill, "g"),
            None => vec![fill_rgb(color)],
        }
    }

    fn stroke(&self, color: [u8; 3]) -> Vec<Operation> {
        self.fill(color)
            .into_iter()
            .map(|o| {
                let operator = match o.operator.as_str() {
                    "cs" => "CS",
                    "sc" => "SC",
                    "scn" => "SCN",
                    "rg" => "RG",
                    "g" => "G",
                    "k" => "K",
                    other => other,
                };
                op(operator, o.operands)
            })
            .collect()
    }
}

/// Emits a line's segments, tracking the text state and the pen position along the baseline
/// in the first run's text space.
struct Writer<'a> {
    first: &'a Run,
    colors: Colors<'a>,
    ops: Vec<Operation>,
    pen: f64,
    font: Vec<u8>,
    tfs: f64,
    color: [u8; 3],
    mode: i64,
    /// The stroke color and line width, while they differ from the line's own.
    stroke: Option<[u8; 3]>,
    width: Option<f64>,
    italic: bool,
    /// A `Tm` changed the text line matrix.
    moved: bool,
}

impl Writer<'_> {
    fn style(&mut self, style: &Style, key: Vec<u8>) {
        let first = self.first;
        if key != self.font || style.tfs != self.tfs {
            self.ops.push(set_font(&key, style.tfs));
            (self.font, self.tfs) = (key, style.tfs);
        }
        if style.color != self.color {
            self.ops.extend(self.colors.fill(style.color));
            self.color = style.color;
        }
        let mode = mode(first.mode, style.bold);
        if mode != self.mode {
            self.ops.push(op("Tr", vec![Object::Integer(mode)]));
            self.mode = mode;
        }
        if stroked(mode) && !stroked(first.mode) {
            let [a, b, c, d, ..] = first.tm;
            let width = BOLD_STROKE * style.tfs * (a * d - b * c).abs().sqrt();
            if self.width != Some(width) {
                self.ops.push(op("w", vec![real(width)]));
                self.width = Some(width);
            }
        }
        if stroked(mode) {
            let wanted =
                (style.color != first.color || !stroked(first.mode)).then_some(style.color);
            if wanted != self.stroke {
                let stroke = match wanted {
                    Some(color) => self.colors.stroke(color),
                    None => original_stroke(first),
                };
                self.ops.extend(stroke);
                self.stroke = wanted;
            }
        }
        if style.italic != self.italic {
            let at = mul(&translate(self.pen, 0.0), &first.tm);
            let tm = if style.italic {
                mul(&[1.0, 0.0, ITALIC_SLANT, 1.0, 0.0, 0.0], &at)
            } else {
                at
            };
            self.ops.push(op("Tm", matrix_operands(&tm)));
            self.italic = style.italic;
            self.moved = true;
        }
    }

    /// Shows `pieces` in the current state, returning where the last glyph ends.
    fn show(&mut self, pieces: &[Piece], font: &Font) -> f64 {
        let (first, tfs) = (self.first, self.tfs);
        let advance = |code: &[u8]| {
            let space = !font.cid && code == [32];
            (font.metrics.width(code_value(code)) / 1000.0 * tfs
                + first.tc
                + if space { first.tw } else { 0.0 })
                * first.th
        };
        let (show, end) = show(pieces, advance, tfs * first.th, &mut self.pen);
        self.ops.push(show);
        end
    }

    /// Moves the pen by `distance` in the first run's font.
    fn advance(&mut self, distance: f64) {
        let first = self.first;
        let n = -distance * 1000.0 / (first.tfs * first.th);
        if n.abs() > 0.0005 {
            self.ops.push(op("TJ", vec![Object::Array(vec![real(n)])]));
        }
    }

    /// Restores the first run's text state, and leaves the pen where the run left it.
    fn finish(mut self) -> Vec<Operation> {
        let first = self.first;
        if self.font != first.font || self.tfs != first.tfs {
            self.ops.push(set_font(&first.font, first.tfs));
        }
        if self.color != first.color {
            self.ops.extend(original(&first.fill, "g"));
        }
        if self.mode != first.mode {
            self.ops.push(op("Tr", vec![Object::Integer(first.mode)]));
        }
        if self.width.is_some() {
            self.ops.push(op("w", vec![real(first.line_width)]));
        }
        if self.stroke.is_some() {
            self.ops.extend(original_stroke(first));
        }
        let end = first.advance / 1000.0 * first.tfs * first.th;
        if self.moved {
            self.ops.push(op("Tm", matrix_operands(&first.tlm)));
            self.advance(start_offset(&first.tm, &first.tlm) + end);
        } else {
            let pen = self.pen;
            self.advance(end - pen);
        }
        self.ops
    }
}

/// The color operators `ops`, or black in `gray`, the initial color, when there are none.
fn original(ops: &[Operation], gray: &str) -> Vec<Operation> {
    if ops.is_empty() {
        vec![op(gray, vec![Object::Integer(0)])]
    } else {
        ops.to_vec()
    }
}

fn original_stroke(first: &Run) -> Vec<Operation> {
    original(&first.stroke, "G")
}

/// How far along the text line `tm` is from `tlm`, which differ by a horizontal move.
fn start_offset(tm: &Matrix, tlm: &Matrix) -> f64 {
    if tlm[0].abs() >= tlm[1].abs() {
        if tlm[0] == 0.0 {
            0.0
        } else {
            (tm[4] - tlm[4]) / tlm[0]
        }
    } else {
        (tm[5] - tlm[5]) / tlm[1]
    }
}

/// The line moves that `'` and `"` make before showing text, which a rewritten run must keep.
fn prefix(operation: &Operation) -> Vec<Operation> {
    match operation.operator.as_str() {
        "'" => vec![op("T*", vec![])],
        "\"" => {
            let operand = |i: usize| {
                operation
                    .operands
                    .get(i)
                    .cloned()
                    .unwrap_or(Object::Integer(0))
            };
            vec![
                op("Tw", vec![operand(0)]),
                op("Tc", vec![operand(1)]),
                op("T*", vec![]),
            ]
        }
        _ => Vec::new(),
    }
}

/// Shows `spans` in place of `line`: in the line's first show operator, from where its first
/// glyph was, leaving the text state and pen as the operator left them. The line's other show
/// operators keep their advance but show nothing, so text after the line stays where it was.
pub(super) fn edit_line(
    line: &Line,
    spans: &[Span],
    content: &[Operation],
    faces: &mut Faces,
    scan: &mut Scan,
) -> Result<LineChange, EditError> {
    let first = &line.runs[0];
    let last = line.runs.last().expect("lines have runs");
    if first.tfs * first.th == 0.0 {
        return Err(EditError::Invalid("the line has no size"));
    }
    let mut fallback = String::new();
    let segments = segments(&Basis::of(line), spans, faces, scan, &mut fallback)?;

    let mut writer = Writer {
        first,
        colors: Colors { runs: &line.runs },
        ops: prefix(&content[first.op]),
        pen: 0.0,
        font: first.font.clone(),
        tfs: first.tfs,
        color: first.color,
        mode: first.mode,
        stroke: None,
        width: None,
        italic: false,
        moved: false,
    };
    writer.advance(-first.lead / 1000.0 * first.tfs * first.th);
    writer.pen = -first.lead / 1000.0 * first.tfs * first.th;
    let mut glyph_end = writer.pen;
    let mut underlines = Vec::new();
    for segment in &segments {
        let key = faces.key(segment.style.face);
        writer.style(&segment.style, key);
        let start = writer.pen;
        glyph_end = writer.show(&segment.pieces, faces.font(segment.style.face));
        if segment.style.underline {
            underlines.extend(underline(
                &first.matrix,
                first.rise,
                writer.colors.fill(segment.style.color),
                segment.style.tfs,
                start,
                glyph_end,
            ));
        }
    }

    let origin = first.start + first.lead / 1000.0 * first.em;
    let new_end = origin + glyph_end * first.em / (first.tfs * first.th);
    let mut ops = vec![(first.op, writer.finish())];
    for run in &line.runs[1..] {
        let mut replacement = prefix(&content[run.op]);
        replacement.push(op("TJ", vec![Object::Array(vec![real(-run.advance)])]));
        ops.push((run.op, replacement));
    }
    Ok(LineChange {
        ops,
        underlines,
        fallback,
        overflow: new_end - last.end,
    })
}

/// A filled rectangle under text from pen position `from` to `to`, in text space `matrix` with
/// the text raised by `rise`.
fn underline(
    matrix: &Matrix,
    rise: f64,
    fill: Vec<Operation>,
    tfs: f64,
    from: f64,
    to: f64,
) -> Vec<Operation> {
    let top = rise - UNDERLINE_OFFSET * tfs;
    let thickness = UNDERLINE_THICKNESS * tfs;
    let mut ops = vec![op("q", vec![]), op("cm", matrix_operands(matrix))];
    ops.extend(fill);
    ops.push(op(
        "re",
        vec![
            real(from),
            real(top - thickness),
            real(to - from),
            real(thickness),
        ],
    ));
    ops.push(op("f", vec![]));
    ops.push(op("Q", vec![]));
    ops
}

/// The operators that end what `operations` leave open, when a `q` comes before them: a text
/// object, a path that wasn't painted, and the saved graphics states. Operators added after
/// these start from the page's initial state. Fails when `operations` restore a state they
/// didn't save, which viewers ignore and a `q` before them would not.
pub(super) fn closing(operations: &[Operation]) -> Result<Vec<Operation>, EditError> {
    let mut depth = 0usize;
    let (mut text, mut path) = (false, false);
    for operation in operations {
        match operation.operator.as_str() {
            "q" => depth += 1,
            "Q" => depth = depth.checked_sub(1).ok_or(EditError::UnsafeContent)?,
            "BT" => text = true,
            "ET" => text = false,
            "m" | "l" | "c" | "v" | "y" | "h" | "re" | "W" | "W*" => path = true,
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => path = false,
            _ => {}
        }
    }
    let mut ops = Vec::new();
    if text {
        ops.push(op("ET", vec![]));
    }
    if path {
        ops.push(op("n", vec![]));
    }
    ops.extend((0..=depth).map(|_| op("Q", vec![])));
    Ok(ops)
}

/// Wraps `operations` so operators added after them start from the page's initial state.
pub(super) fn isolate(operations: &mut Vec<Operation>) -> Result<(), EditError> {
    let closing = closing(operations)?;
    operations.insert(0, op("q", vec![]));
    operations.extend(closing);
    Ok(())
}

/// Line height of added text, as a multiple of the font size, matching the editor's CSS.
const LEADING: f64 = 1.2;

/// Shear of a synthesized italic, as a text matrix.
const SKEW: Matrix = [1.0, 0.0, ITALIC_SLANT, 1.0, 0.0, 0.0];

/// The height of a row of text in `size` above and below its baseline, as CSS lays out a line
/// of `LEADING` in a font with `ascent` and `descent` in thousandths of an em.
fn extent(size: f64, ascent: f64, descent: f64) -> (f64, f64) {
    let a = Some(ascent / 1000.0)
        .filter(|a| *a > 0.0 && *a <= 1.5)
        .unwrap_or(0.8);
    let d = Some(descent / 1000.0)
        .filter(|d| *d < 0.0 && *d >= -0.6)
        .unwrap_or(-0.2);
    (
        size * (LEADING + a + d) / 2.0,
        size * (LEADING - a - d) / 2.0,
    )
}

/// The text state of added text, which starts from the page's initial state.
struct Pen {
    font: Vec<u8>,
    tfs: f64,
    color: [u8; 3],
    bold: bool,
    stroke: Option<[u8; 3]>,
    width: Option<f64>,
}

impl Pen {
    fn style(&mut self, ops: &mut Vec<Operation>, style: &Style, key: Vec<u8>) {
        if key != self.font || style.tfs != self.tfs {
            ops.push(set_font(&key, style.tfs));
            (self.font, self.tfs) = (key, style.tfs);
        }
        if style.color != self.color {
            ops.push(fill_rgb(style.color));
            self.color = style.color;
        }
        if style.bold != self.bold {
            ops.push(op(
                "Tr",
                vec![Object::Integer(if style.bold { 2 } else { 0 })],
            ));
            self.bold = style.bold;
        }
        if style.bold {
            let width = BOLD_STROKE * style.tfs;
            if self.width != Some(width) {
                ops.push(op("w", vec![real(width)]));
                self.width = Some(width);
            }
            if self.stroke != Some(style.color) {
                ops.push(stroke_rgb(style.color));
                self.stroke = Some(style.color);
            }
        }
    }
}

/// Draws `text` on a page shown in `geometry`, in its own graphics state.
pub(super) fn draw_text(
    text: &TextBox,
    geometry: &PageGeometry,
    faces: &mut Faces,
    scan: &mut Scan,
) -> Result<Vec<Operation>, EditError> {
    let [a, b, c, d, ..] = geometry.upright_matrix();
    let mut ops = vec![op("q", vec![])];
    let mut pen = Pen {
        font: Vec::new(),
        tfs: 0.0,
        color: [0, 0, 0],
        bold: false,
        stroke: None,
        width: None,
    };
    let mut top = text.y;
    let mut previous = None;
    for row in &text.lines {
        let spans: Vec<Span> = row
            .iter()
            .filter(|s| !s.text.is_empty())
            .map(|s| Span {
                text: s.text.replace('\t', " "),
                ..s.clone()
            })
            .collect();
        // Empty spans count, so an empty row is as high as the style it was typed in.
        let mut row_extent: Option<(f64, f64)> = None;
        for span in row {
            let face = faces.resolve(span, scan).face;
            let font = faces.font(face);
            let (above, below) = extent(span.size, font.ascent, font.descent);
            row_extent = Some(match row_extent {
                Some((a, b)) => (a.max(above), b.max(below)),
                None => (above, below),
            });
        }
        let (above, below) = row_extent
            .or(previous)
            .unwrap_or_else(|| extent(12.0, 718.0, -207.0));
        previous = Some((above, below));
        let baseline = top + above;
        top = baseline + below;
        if spans.iter().all(|s| s.text.trim().is_empty()) {
            continue;
        }

        let (x, y) = geometry.to_user(text.x, baseline);
        let matrix = [a, b, c, d, x, y];
        let segments = segments(&Basis::PLAIN, &spans, faces, scan, &mut String::new())?;
        ops.push(op("BT", vec![]));
        let (mut at, mut italic) = (0.0, None);
        let mut underlines = Vec::new();
        for segment in &segments {
            let style = &segment.style;
            let key = faces.key(style.face);
            pen.style(&mut ops, style, key);
            if italic != Some(style.italic) {
                let tm = mul(&translate(at, 0.0), &matrix);
                let tm = if style.italic { mul(&SKEW, &tm) } else { tm };
                ops.push(op("Tm", matrix_operands(&tm)));
                italic = Some(style.italic);
            }
            let font = faces.font(style.face);
            let advance = |code: &[u8]| font.metrics.width(code_value(code)) / 1000.0 * style.tfs;
            let start = at;
            let (show, end) = show(&segment.pieces, advance, style.tfs, &mut at);
            ops.push(show);
            if style.underline {
                underlines.extend(underline(
                    &matrix,
                    0.0,
                    vec![fill_rgb(style.color)],
                    style.tfs,
                    start,
                    end,
                ));
            }
        }
        ops.push(op("ET", vec![]));
        ops.extend(underlines);
    }
    ops.push(op("Q", vec![]));
    Ok(ops)
}
