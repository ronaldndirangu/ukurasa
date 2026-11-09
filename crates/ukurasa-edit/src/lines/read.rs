//! Reads the lines of text a page's content stream draws, and the styled spans of each line.

use std::collections::{BTreeMap, HashMap, HashSet};

use lopdf::content::{Content, Operation};
use lopdf::{DecompressError, Document, Object, ObjectId};
use ukurasa_geom::{Rect, transform};

use super::faces::{FaceId, Font, load_font, page_fonts};
use super::{Span, TextLine};
use crate::{EditError, page_geometry};

/// A TJ adjustment at or below this, in thousandths of an em, reads as a word space.
const KERN_SPACE: f64 = -180.0;

/// Pages on each side of an edited page searched for glyphs of a subset font.
const SCAN_PAGES: usize = 20;

/// The most decoded content a page may have for its text to be edited.
pub(crate) const MAX_CONTENT: usize = 64 << 20;

pub(crate) type Matrix = [f64; 6];

pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m × n`: applies `m`, then `n`.
pub(crate) fn mul(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

pub(super) fn translate(x: f64, y: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

pub(crate) fn num(o: &Object) -> Option<f64> {
    match *o {
        Object::Integer(i) => Some(i as f64),
        Object::Real(r) => Some(f64::from(r)),
        _ => None,
    }
}

pub(super) fn real(v: f64) -> Object {
    Object::Real(((v * 1000.0).round() / 1000.0) as f32)
}

pub(crate) fn deref<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    doc.dereference(o).ok().map(|(_, o)| o)
}

pub(super) fn deref_num(doc: &Document, o: &Object) -> Option<f64> {
    deref(doc, o).and_then(num)
}

pub(super) fn code_value(code: &[u8]) -> u32 {
    code.iter().fold(0, |value, b| value << 8 | u32::from(*b))
}

/// A font size in display points, as spans report it.
pub(super) fn display_size(size: f64) -> f64 {
    (size * 100.0).round() / 100.0
}

pub(crate) fn matrix(operands: &[Object]) -> Option<Matrix> {
    let values: Vec<f64> = operands.iter().map(num).collect::<Option<_>>()?;
    values.try_into().ok()
}

fn color(operands: &[Object]) -> Option<[u8; 3]> {
    let values: Vec<f64> = operands.iter().map(num).collect::<Option<_>>()?;
    let to = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    match values[..] {
        [g] => Some([to(g); 3]),
        [r, g, b] => Some([to(r), to(g), to(b)]),
        [c, m, y, k] => Some([
            to((1.0 - c) * (1.0 - k)),
            to((1.0 - m) * (1.0 - k)),
            to((1.0 - y) * (1.0 - k)),
        ]),
        _ => None,
    }
}

/// Records a color operator in `ops`, the operators that set the current color. A color space
/// or device color stays, as the operators after it depend on it.
fn set_color(ops: &mut Vec<Operation>, op: &Operation) {
    match op.operator.as_str() {
        "sc" | "scn" | "SC" | "SCN" => {
            ops.truncate(1);
            if ops
                .first()
                .is_some_and(|o| matches!(o.operator.as_str(), "sc" | "scn" | "SC" | "SCN"))
            {
                ops.clear();
            }
            ops.push(op.clone());
        }
        _ => *ops = vec![op.clone()],
    }
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Option<Vec<u8>>,
    tfs: f64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    rise: f64,
    mode: i64,
    color: [u8; 3],
    fill: Vec<Operation>,
    stroke: Vec<Operation>,
    line_width: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            ctm: IDENTITY,
            font: None,
            tfs: 0.0,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            rise: 0.0,
            mode: 0,
            color: [0, 0, 0],
            fill: Vec::new(),
            stroke: Vec::new(),
            line_width: 1.0,
        }
    }
}

/// One show operator of a line. Advances are in thousandths of an em at the run's font size.
pub(super) struct Run {
    pub(super) op: usize,
    /// The font's resource name.
    pub(super) font: Vec<u8>,
    pub(super) tfs: f64,
    pub(super) tc: f64,
    pub(super) tw: f64,
    pub(super) th: f64,
    pub(super) color: [u8; 3],
    /// Operators that set the fill and stroke colors, empty for the initial black.
    pub(super) fill: Vec<Operation>,
    pub(super) stroke: Vec<Operation>,
    pub(super) line_width: f64,
    pub(super) text: String,
    /// Sum of the TJ adjustments before the first glyph.
    pub(super) lead: f64,
    /// Total advance, including character and word spacing and TJ adjustments.
    pub(super) advance: f64,
    /// TJ adjustments read as word spaces.
    pub(super) space_kerns: Vec<f64>,
    pub(super) bounds: Rect,
    /// Display x of the first glyph's origin and of the last glyph's end, along the baseline.
    pub(super) start: f64,
    pub(super) end: f64,
    pub(super) baseline: f64,
    /// Display points per em, vertically and horizontally.
    pub(super) size: f64,
    pub(super) em: f64,
    /// The text matrix where the run starts, alone and times the CTM.
    pub(super) tm: Matrix,
    pub(super) matrix: Matrix,
    /// The text line matrix where the run starts.
    pub(super) tlm: Matrix,
    pub(super) rise: f64,
    pub(super) mode: i64,
    /// The text matrix slants the glyphs, as for a synthesized italic.
    pub(super) slant: bool,
    /// The line's text has a space before the run that the run doesn't show.
    pub(super) gap: bool,
}

pub(super) struct Line {
    pub(super) runs: Vec<Run>,
    pub(super) text: String,
}

type Observed = HashMap<Vec<u8>, HashMap<char, Vec<u8>>>;

pub(super) struct Page<'a> {
    pub(super) ops: Vec<Operation>,
    pub(super) lines: Vec<Line>,
    /// Fonts the page's text uses, by resource name.
    pub(super) fonts: HashMap<Vec<u8>, Option<Font<'a>>>,
    /// Every font the page can use, by resource name.
    pub(super) ids: BTreeMap<Vec<u8>, FaceId>,
    /// The code each base font uses for each character this page shows.
    pub(super) observed: Observed,
}

/// Glyphs measured from one show operator.
#[derive(Default)]
struct Measured {
    text: String,
    decodable: bool,
    glyphs: usize,
    lead: f64,
    advance: f64,
    start: f64,
    end: f64,
    space_kerns: Vec<f64>,
    observed: Vec<(char, Vec<u8>)>,
}

fn measure(font: &Font, state: &State, items: &[Object]) -> Measured {
    let Some(encoding) = &font.encoding else {
        return Measured::default();
    };
    let spacing = |space: bool| {
        if state.tfs == 0.0 {
            0.0
        } else {
            (state.tc + if space { state.tw } else { 0.0 }) * 1000.0 / state.tfs
        }
    };
    let mut m = Measured {
        decodable: true,
        ..Measured::default()
    };
    let mut pen = 0.0;
    let mut pending_space = false;
    for item in items {
        match item {
            Object::String(bytes, _) => {
                let step = if font.cid { 2 } else { 1 };
                if bytes.len() % step != 0 {
                    m.decodable = false;
                }
                for code in bytes.chunks(step) {
                    let value = code_value(code);
                    let decoded = Document::decode_text(encoding, code).unwrap_or_default();
                    if decoded.is_empty()
                        || decoded.chars().any(|c| c == '\u{fffd}' || c.is_control())
                    {
                        m.decodable = false;
                    }
                    if pending_space
                        && !m.text.ends_with(char::is_whitespace)
                        && !decoded.starts_with(char::is_whitespace)
                    {
                        m.text.push(' ');
                    }
                    pending_space = false;
                    let mut chars = decoded.chars();
                    if let (Some(c), None) = (chars.next(), chars.next()) {
                        m.observed.push((c, code.to_vec()));
                    }
                    m.text.push_str(&decoded);
                    if m.glyphs == 0 {
                        m.start = pen;
                    }
                    pen += font.metrics.width(value) + spacing(!font.cid && value == 32);
                    m.end = pen;
                    m.glyphs += 1;
                }
            }
            other => {
                let Some(n) = num(other) else { continue };
                if m.glyphs == 0 {
                    m.lead += n;
                } else if n <= KERN_SPACE {
                    pending_space = true;
                    m.space_kerns.push(n);
                }
                pen -= n;
            }
        }
    }
    m.advance = pen;
    m
}

pub(super) fn read_page(doc: &Document, page_id: ObjectId) -> Result<Page<'_>, EditError> {
    read_ops(doc, page_id, page_operations(doc, page_id)?)
}

/// Reads the lines of text that the page's decoded content `ops` shows.
pub(super) fn read_ops(
    doc: &Document,
    page_id: ObjectId,
    ops: Vec<Operation>,
) -> Result<Page<'_>, EditError> {
    let dicts = page_fonts(doc, page_id)?;
    let font_states = font_states(doc, page_id);
    let geometry = page_geometry(doc, page_id);
    let mut fonts: HashMap<Vec<u8>, Option<Font>> = HashMap::new();
    let mut observed = Observed::new();
    let mut lines: Vec<Line> = Vec::new();
    // Whether the next run may continue the last line.
    let mut open = false;
    // Text in a font Ukurasa can't measure moved the text matrix by an unknown amount.
    let mut lost = false;
    let mut state = State::default();
    let mut stack = Vec::new();
    let (mut tm, mut tlm) = (IDENTITY, IDENTITY);

    for (index, op) in ops.iter().enumerate() {
        let operands = op.operands.as_slice();
        let n = |i: usize| operands.get(i).and_then(num);
        let mut next_line = |tl: f64| {
            tlm = mul(&translate(0.0, -tl), &tlm);
            tm = tlm;
            lost = false;
        };
        let items = match op.operator.as_str() {
            "q" => {
                stack.push(state.clone());
                continue;
            }
            "Q" => {
                if let Some(saved) = stack.pop() {
                    state = saved;
                }
                continue;
            }
            "cm" => {
                if let Some(m) = matrix(operands) {
                    state.ctm = mul(&m, &state.ctm);
                }
                continue;
            }
            "BT" => {
                (tm, tlm) = (IDENTITY, IDENTITY);
                lost = false;
                continue;
            }
            "Tf" => {
                state.font = operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .map(<[u8]>::to_vec);
                state.tfs = n(1).unwrap_or(0.0);
                continue;
            }
            "gs" => {
                let sets_font = operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .is_some_and(|name| font_states.contains(name));
                if sets_font {
                    state.font = None;
                }
                continue;
            }
            "Tc" | "Tw" | "Tz" | "TL" | "Ts" | "Tr" | "w" => {
                if let Some(v) = n(0) {
                    match op.operator.as_str() {
                        "Tc" => state.tc = v,
                        "Tw" => state.tw = v,
                        "Tz" => state.th = v / 100.0,
                        "TL" => state.tl = v,
                        "Ts" => state.rise = v,
                        "w" => state.line_width = v,
                        _ => state.mode = v as i64,
                    }
                }
                continue;
            }
            "Td" | "TD" => {
                if let (Some(x), Some(y)) = (n(0), n(1)) {
                    if op.operator == "TD" {
                        state.tl = -y;
                    }
                    tlm = mul(&translate(x, y), &tlm);
                    tm = tlm;
                    lost = false;
                }
                continue;
            }
            "Tm" => {
                if let Some(m) = matrix(operands) {
                    (tm, tlm) = (m, m);
                    lost = false;
                }
                continue;
            }
            "T*" => {
                next_line(state.tl);
                continue;
            }
            "rg" | "g" | "k" | "sc" | "scn" => {
                if let Some(c) = color(operands) {
                    state.color = c;
                }
                set_color(&mut state.fill, op);
                continue;
            }
            "cs" => {
                state.color = [0, 0, 0];
                set_color(&mut state.fill, op);
                continue;
            }
            "RG" | "G" | "K" | "SC" | "SCN" | "CS" => {
                set_color(&mut state.stroke, op);
                continue;
            }
            "Tj" | "TJ" => operands.first(),
            "'" => {
                next_line(state.tl);
                operands.first()
            }
            "\"" => {
                if let (Some(aw), Some(ac)) = (n(0), n(1)) {
                    (state.tw, state.tc) = (aw, ac);
                }
                next_line(state.tl);
                operands.get(2)
            }
            // A line rewritten in its first show operator would move later text under what
            // these paint.
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "sh" | "Do" | "BI" | "EI" => {
                open = false;
                continue;
            }
            _ => continue,
        };
        let items = match items {
            Some(Object::Array(items)) => items.as_slice(),
            Some(string @ Object::String(..)) => std::slice::from_ref(string),
            _ => &[],
        };
        let font = match state.font.clone() {
            Some(name) => {
                let font = fonts
                    .entry(name.clone())
                    .or_insert_with(|| dicts.get(&name).and_then(|(_, d)| load_font(doc, d)));
                font.as_ref().map(|font| (name, font))
            }
            None => None,
        };
        let Some((name, font)) = font else {
            lost |= !items.is_empty();
            open = false;
            continue;
        };
        let m = measure(font, &state, items);
        let start_tm = tm;
        let base = mul(&tm, &state.ctm);
        let trm = mul(
            &[state.tfs * state.th, 0.0, 0.0, state.tfs, 0.0, state.rise],
            &base,
        );
        tm = mul(
            &translate(m.advance / 1000.0 * state.tfs * state.th, 0.0),
            &tm,
        );
        if m.glyphs == 0 {
            continue;
        }
        if m.decodable {
            let codes = observed.entry(font.base.clone()).or_default();
            for (c, code) in m.observed {
                codes.entry(c).or_insert(code);
            }
        }
        let point = |x: f64, y: f64| {
            let (ux, uy) = transform(&trm, x / 1000.0, y / 1000.0);
            geometry.to_display(ux, uy)
        };
        let origin = point(0.0, 0.0);
        let (right, up) = (point(1000.0, 0.0), point(0.0, 1000.0));
        let (gx, gy) = (
            (right.0 - origin.0, right.1 - origin.1),
            (up.0 - origin.0, up.1 - origin.1),
        );
        let horizontal = gx.0 > 1e-6 && gx.1.abs() <= 0.02 * gx.0 && gy.1 < 0.0;
        if lost || !(m.decodable && horizontal && !matches!(state.mode, 3 | 7)) {
            open = false;
            continue;
        }
        let (start, end) = (point(m.start, 0.0), point(m.end, 0.0));
        let mut run = Run {
            op: index,
            font: name,
            tfs: state.tfs,
            tc: state.tc,
            tw: state.tw,
            th: state.th,
            color: state.color,
            fill: state.fill.clone(),
            stroke: state.stroke.clone(),
            line_width: state.line_width,
            text: m.text,
            lead: m.lead,
            advance: m.advance,
            space_kerns: m.space_kerns,
            bounds: Rect::bounding(&[
                point(m.start, font.descent),
                point(m.end, font.descent),
                point(m.start, font.ascent),
                point(m.end, font.ascent),
            ]),
            start: start.0,
            end: end.0,
            baseline: start.1,
            size: -gy.1,
            em: gx.0,
            tm: start_tm,
            matrix: base,
            tlm,
            rise: state.rise,
            mode: state.mode,
            slant: gy.0 > 0.1 * -gy.1,
            gap: false,
        };
        match lines.last_mut() {
            Some(line) if open && joins(line.runs.last().expect("lines have runs"), &run) => {
                let last = line.runs.last().expect("lines have runs");
                if run.start - last.end > 0.15 * last.size
                    && !line.text.ends_with(char::is_whitespace)
                    && !run.text.starts_with(char::is_whitespace)
                {
                    line.text.push(' ');
                    run.gap = true;
                }
                line.text.push_str(&run.text);
                line.runs.push(run);
            }
            _ => lines.push(Line {
                text: run.text.clone(),
                runs: vec![run],
            }),
        }
        open = true;
    }
    Ok(Page {
        ops,
        lines,
        fonts,
        ids: dicts.into_iter().map(|(key, (id, _))| (key, id)).collect(),
        observed,
    })
}

/// Whether run `b` continues the line that ends with run `a`: it sits on the same baseline,
/// close after it, in any font, size, or color, and spaces and paints its glyphs as `a` does,
/// as the line is rewritten in the text state of its first run.
fn joins(a: &Run, b: &Run) -> bool {
    let size = a.size.max(b.size);
    let gap = b.start - a.end;
    let tfs = a.tfs.abs().max(b.tfs.abs());
    let near = |x: f64, y: f64, tolerance: f64| (x - y).abs() <= tolerance;
    let fill = |mode: i64| match mode {
        2 => 0,
        6 => 4,
        mode => mode,
    };
    (a.baseline - b.baseline).abs() <= 0.1 * size
        && (-0.3 * size..=size).contains(&gap)
        && near(a.tc, b.tc, 0.02 * tfs)
        && near(a.tw, b.tw, 0.02 * tfs)
        && near(a.th, b.th, 0.001)
        && near(a.rise, b.rise, 0.005 * tfs)
        && fill(a.mode) == fill(b.mode)
}

/// Decodes the page's content, failing when rewriting it would drop part of it.
fn page_operations(doc: &Document, page_id: ObjectId) -> Result<Vec<Operation>, EditError> {
    let ops = decode_content(doc, page_id)?;
    check_images(&ops)?;
    Ok(ops)
}

/// Decodes the page's content, failing when part of it can't be read.
pub(crate) fn decode_content(
    doc: &Document,
    page_id: ObjectId,
) -> Result<Vec<Operation>, EditError> {
    let data = doc
        .get_page_content_with_limit(page_id, MAX_CONTENT)
        .map_err(|error| match error {
            lopdf::Error::Decompress(DecompressError::MemoryLimitExceeded { .. }) => {
                EditError::LargeContent
            }
            error => error.into(),
        })?;
    Ok(Content::decode_strict(&data)
        .map_err(|_| EditError::UnsafeContent)?
        .operations)
}

/// Fails when encoding `ops` again would drop an inline image, which lopdf reads as a BI
/// operator with no image when it can't decode it.
pub(crate) fn check_images(ops: &[Operation]) -> Result<(), EditError> {
    let images_kept = ops
        .iter()
        .filter(|op| op.operator == "BI")
        .all(|op| matches!(op.operands.as_slice(), [Object::Stream(_)]));
    if images_kept {
        Ok(())
    } else {
        Err(EditError::UnsafeContent)
    }
}

/// The names of the page's graphics state parameter dictionaries that set a font.
fn font_states(doc: &Document, page_id: ObjectId) -> HashSet<Vec<u8>> {
    let Ok((direct, inherited)) = doc.get_page_resources(page_id) else {
        return HashSet::new();
    };
    let resources = direct.into_iter().chain(
        inherited
            .into_iter()
            .filter_map(|id| doc.get_dictionary(id).ok()),
    );
    let mut names = HashSet::new();
    for resources in resources {
        let Some(states) = resources
            .get(b"ExtGState")
            .ok()
            .and_then(|o| deref(doc, o))
            .and_then(|o| o.as_dict().ok())
        else {
            continue;
        };
        for (name, state) in states.iter() {
            if deref(doc, state)
                .and_then(|s| s.as_dict().ok())
                .is_some_and(|s| s.has(b"Font"))
            {
                names.insert(name.clone());
            }
        }
    }
    names
}

/// The line's text as spans of one style each.
pub(super) fn spans(
    line: &Line,
    fonts: &HashMap<Vec<u8>, Option<Font>>,
    ids: &BTreeMap<Vec<u8>, FaceId>,
) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for run in &line.runs {
        let Some(font) = fonts.get(&run.font).and_then(Option::as_ref) else {
            continue;
        };
        if run.gap
            && let Some(last) = spans.last_mut()
        {
            last.text.push(' ');
        }
        let span = Span {
            text: run.text.clone(),
            font: font.family_name.clone(),
            face: ids.get(&run.font).map(FaceId::encode),
            family: font.family,
            bold: font.bold || matches!(run.mode, 2 | 6),
            italic: font.italic || run.slant,
            underline: false,
            size: display_size(run.size),
            color: run.color,
        };
        match spans.last_mut() {
            Some(last) if last.same_style(&span) => last.text.push_str(&span.text),
            _ => spans.push(span),
        }
    }
    spans
}

pub(super) fn text_line(line: &Line, page: &Page) -> TextLine {
    let corners: Vec<(f64, f64)> = line
        .runs
        .iter()
        .flat_map(|r| [(r.bounds.x0, r.bounds.y0), (r.bounds.x1, r.bounds.y1)])
        .collect();
    let bounds = Rect::bounding(&corners);
    TextLine {
        runs: line.runs.iter().map(|r| r.op).collect(),
        text: line.text.clone(),
        x: bounds.x0,
        y: bounds.y0,
        width: bounds.width(),
        height: bounds.height(),
        baseline: line.runs[0].baseline,
        spans: spans(line, &page.fonts, &page.ids),
    }
}

/// Finds the codes subset fonts use for characters, reading other pages on demand.
pub(super) struct Scan<'a> {
    doc: &'a Document,
    /// Pages still to read, next last.
    pages: Vec<ObjectId>,
    read: HashSet<ObjectId>,
    observed: Observed,
}

impl<'a> Scan<'a> {
    pub(super) fn new(
        doc: &'a Document,
        page_id: ObjectId,
        pages: Vec<ObjectId>,
        observed: Observed,
    ) -> Self {
        Scan {
            doc,
            pages,
            read: HashSet::from([page_id]),
            observed,
        }
    }

    /// Reads `page` next, as a page known to use a font.
    pub(super) fn include(&mut self, page: ObjectId) {
        if !self.read.contains(&page) && self.pages.last() != Some(&page) {
            self.pages.retain(|p| *p != page);
            self.pages.push(page);
        }
    }

    pub(super) fn find(&mut self, base: &[u8], c: char) -> Option<Vec<u8>> {
        loop {
            if let Some(code) = self.observed.get(base).and_then(|codes| codes.get(&c)) {
                return Some(code.clone());
            }
            let page_id = self.pages.pop()?;
            if !self.read.insert(page_id) {
                continue;
            }
            let Ok(page) = read_page(self.doc, page_id) else {
                continue;
            };
            for (base, codes) in page.observed {
                let known = self.observed.entry(base).or_default();
                for (c, code) in codes {
                    known.entry(c).or_insert(code);
                }
            }
        }
    }
}

/// The pages within [`SCAN_PAGES`] of page `index`, nearest last.
pub(super) fn neighbors(pages: &[ObjectId], index: usize) -> Vec<ObjectId> {
    let mut order = Vec::new();
    for distance in 1..=SCAN_PAGES {
        if let Some(i) = index.checked_sub(distance) {
            order.push(pages[i]);
        }
        if let Some(page) = pages.get(index + distance) {
            order.push(*page);
        }
    }
    order.reverse();
    order
}
