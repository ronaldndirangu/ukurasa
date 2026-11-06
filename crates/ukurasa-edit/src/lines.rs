//! In-place text editing. Reads the lines of text a page draws from its content stream as spans of
//! styled text, and rewrites the show operators of a line with new styled text, leaving every
//! other operator on the page as it was.

mod faces;
mod read;
mod write;

use std::collections::{BTreeMap, HashMap, HashSet};

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use serde::{Deserialize, Serialize};

use crate::standard::{Family, Standard};
use crate::{Changes, EditError, TextBox};
use faces::{Faces, load_font, page_fonts};
use read::{Scan, check_images, decode_content, neighbors, read_ops, read_page, spans, text_line};
use write::{closing, draw_text, edit_line, isolate};

/// Text in one style. Sizes are display points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub text: String,
    /// Family name of a font in the document, or of a standard font.
    pub font: String,
    /// The font the text was read in, kept while the span's font and style still match it.
    #[serde(default)]
    pub face: Option<String>,
    /// Generic family, for a stand-in font while editing.
    pub family: Family,
    pub bold: bool,
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    pub size: f64,
    pub color: [u8; 3],
}

impl Span {
    fn same_style(&self, other: &Span) -> bool {
        self.font == other.font
            && self.face == other.face
            && self.bold == other.bold
            && self.italic == other.italic
            && self.underline == other.underline
            && self.color == other.color
            && (self.size - other.size).abs() <= 0.01 * self.size
    }
}

/// An editable line of text. Positions are display points from the page's top-left corner.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLine {
    /// Content operator indexes of the show operators that draw the line.
    pub runs: Vec<usize>,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub baseline: f64,
    pub spans: Vec<Span>,
}

/// Replaces the text of a line, identified by the content operators that draw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineEdit {
    pub page: usize,
    pub runs: Vec<usize>,
    /// The line's text when it was read, to detect a page that changed since.
    pub original: String,
    pub spans: Vec<Span>,
}

/// What happened to one line edit.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineReport {
    pub page: usize,
    pub runs: Vec<usize>,
    /// Characters the chosen font can't show, drawn in a standard font instead.
    pub fallback: String,
    /// How far the new text reaches past the end of the line it replaces, in display points.
    pub overflow: f64,
}

/// A font family in the document that lines can be edited in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentFont {
    pub name: String,
    pub family: Family,
}

fn page_id(doc: &Document, page: usize) -> Result<ObjectId, EditError> {
    u32::try_from(page + 1)
        .ok()
        .and_then(|number| doc.get_pages().get(&number).copied())
        .ok_or(EditError::NoPage(page + 1))
}

/// The editable lines of text on page `page` (zero-based).
pub fn text_lines(doc: &Document, page: usize) -> Result<Vec<TextLine>, EditError> {
    let page = read_page(doc, page_id(doc, page)?)?;
    Ok(page
        .lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .map(|line| text_line(line, &page))
        .collect())
}

/// The families of the fonts in the document that lines can be edited in, by name.
pub fn document_fonts(doc: &Document) -> Vec<DocumentFont> {
    let mut seen = HashSet::new();
    let mut families = BTreeMap::new();
    for page_id in doc.get_pages().into_values() {
        let Ok(fonts) = page_fonts(doc, page_id) else {
            continue;
        };
        for (id, dict) in fonts.into_values() {
            if seen.insert(id)
                && let Some(font) = load_font(doc, dict)
            {
                families.entry(font.family_name).or_insert(font.family);
            }
        }
    }
    families
        .into_iter()
        .map(|(name, family)| DocumentFont { name, family })
        .collect()
}

/// The largest font size a span may have, in display points.
const MAX_SIZE: f64 = 10_000.0;

/// Fails unless every span has a usable size.
pub(crate) fn check_spans(spans: &[Span]) -> Result<(), EditError> {
    if spans
        .iter()
        .all(|s| s.size.is_finite() && s.size > 0.0 && s.size <= MAX_SIZE)
    {
        Ok(())
    } else {
        Err(EditError::Invalid("a text size is out of range"))
    }
}

/// Rewrites the lines `edits` change on page `index`, draws `texts` on it, and returns a report
/// for each changed line.
pub(crate) fn apply_page(
    changes: &mut Changes,
    pages: &[ObjectId],
    index: usize,
    edits: &[&LineEdit],
    texts: &[&TextBox],
    standard: &mut HashMap<Standard, ObjectId>,
) -> Result<Vec<LineReport>, EditError> {
    for edit in edits {
        check_spans(&edit.spans)?;
    }
    let doc = changes.doc;
    let page_id = pages[index];
    let read::Page {
        ops,
        lines,
        fonts,
        ids,
        observed,
    } = read_ops(doc, page_id, decode_content(doc, page_id)?)?;
    let stale = || EditError::StaleText(index + 1);

    let mut targets = Vec::new();
    for edit in edits {
        let line = lines
            .iter()
            .find(|line| line.runs.iter().map(|r| r.op).eq(edit.runs.iter().copied()))
            .ok_or_else(stale)?;
        if line.text != edit.original {
            return Err(stale());
        }
        if spans(line, &fonts, &ids) != edit.spans {
            targets.push((*edit, line));
        }
    }
    if targets.is_empty() && texts.is_empty() {
        return Ok(Vec::new());
    }

    let mut scan = Scan::new(doc, page_id, neighbors(pages, index), observed);
    let mut faces = Faces::new(doc, pages, page_id, fonts, ids);
    let mut replaced: BTreeMap<usize, Vec<Operation>> = BTreeMap::new();
    let mut underlines = Vec::new();
    let mut reports = Vec::new();
    for (edit, line) in targets {
        let change = edit_line(line, &edit.spans, &ops, &mut faces, &mut scan)?;
        for (op, replacement) in change.ops {
            if replaced.insert(op, replacement).is_some() {
                return Err(EditError::Invalid("two edits change the same text"));
            }
        }
        underlines.extend(change.underlines);
        reports.push(LineReport {
            page: edit.page,
            runs: edit.runs.clone(),
            fallback: change.fallback,
            overflow: (change.overflow * 10.0).round() / 10.0,
        });
    }
    let geometry = crate::page_geometry(doc, page_id);
    let mut added = Vec::new();
    for text in texts {
        added.extend(draw_text(text, &geometry, &mut faces, &mut scan)?);
    }

    let contents = match content_streams(doc, page_id) {
        // Added text alone goes in a stream of its own, after the page's streams.
        Some(streams) if replaced.is_empty() => {
            if streams.is_empty() {
                Object::Reference(changes.add(content_stream(added)?)?)
            } else {
                let mut tail = closing(&ops)?;
                tail.extend(added);
                let head = changes.add(content_stream(vec![Operation::new("q", vec![])])?)?;
                let tail = changes.add(content_stream(tail)?)?;
                let mut array = vec![Object::Reference(head)];
                array.extend(streams.into_iter().map(Object::Reference));
                array.push(Object::Reference(tail));
                Object::Array(array)
            }
        }
        _ => {
            check_images(&ops)?;
            let mut operations = ops;
            for (op, replacement) in replaced.into_iter().rev() {
                operations.splice(op..=op, replacement);
            }
            if !underlines.is_empty() || !added.is_empty() {
                isolate(&mut operations)?;
                operations.extend(underlines);
                operations.extend(added);
            }
            Object::Reference(changes.add(content_stream(operations)?)?)
        }
    };
    changes
        .modify(page_id)?
        .as_dict_mut()?
        .set("Contents", contents);

    for (key, added) in faces.into_added() {
        let font = match added {
            faces::Added::Object(id) => id,
            faces::Added::Standard(face) => match standard.get(&face) {
                Some(&id) => id,
                None => {
                    let id = changes.add(face.dictionary())?;
                    standard.insert(face, id);
                    id
                }
            },
        };
        add_font(changes, page_id, &key, font)?;
    }
    Ok(reports)
}

/// The streams of the page's content, when every part of it is a stream.
fn content_streams(doc: &Document, page_id: ObjectId) -> Option<Vec<ObjectId>> {
    let streams = doc.get_page_contents(page_id);
    streams
        .iter()
        .all(|&id| matches!(doc.objects.get(&id), Some(Object::Stream(_))))
        .then_some(streams)
}

fn content_stream(operations: Vec<Operation>) -> Result<Stream, EditError> {
    let content = Content { operations }.encode()?;
    let mut stream = Stream::new(dictionary! {}, content);
    // An uncompressed stream is still valid, so a compression failure is not fatal.
    let _ = stream.compress();
    Ok(stream)
}

/// Adds font `font` to the page's resources as `key`. Other pages may share the resources and
/// font dictionaries the page has, so the page gets its own copies.
fn add_font(
    changes: &mut Changes,
    page_id: ObjectId,
    key: &[u8],
    font: ObjectId,
) -> Result<(), EditError> {
    let doc = changes.doc;
    let current = changes
        .modify(page_id)?
        .as_dict()?
        .get(b"Resources")
        .ok()
        .cloned();
    let mut resources = match current {
        Some(Object::Dictionary(resources)) => resources,
        Some(Object::Reference(id)) => doc.get_dictionary(id).cloned().unwrap_or_default(),
        _ => crate::inherited(doc, page_id, b"Resources")
            .and_then(|o| o.as_dict().ok())
            .cloned()
            .unwrap_or_default(),
    };
    let mut fonts = match resources.get(b"Font") {
        Ok(Object::Dictionary(fonts)) => fonts.clone(),
        Ok(&Object::Reference(id)) => doc.get_dictionary(id).cloned().unwrap_or_default(),
        _ => lopdf::Dictionary::new(),
    };
    fonts.set(key, font);
    resources.set("Font", fonts);
    changes
        .modify(page_id)?
        .as_dict_mut()?
        .set("Resources", resources);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::faces::{family_name, style};
    use super::*;

    #[test]
    fn counts_the_codes_cmap_ranges_map() {
        let small = b"2 beginbfrange <0000> <00FF> <0041>\n<0100> <0101> [<0042> <0043>]\n\
            endbfrange 1 beginbfchar <01> <41> endbfchar";
        assert_eq!(super::faces::cmap_codes(small), 258);
        let huge = [
            &small[..],
            b"\n1 beginbfrange <00000000> <FFFFFFFF> <0000> endbfrange",
        ]
        .concat();
        assert_eq!(super::faces::cmap_codes(&huge), 258 + (1 << 32));
    }

    #[test]
    fn style_reads_common_font_names() {
        let cases = [
            ("NimbusRomNo9L-Regu", (false, false, Family::Serif)),
            ("NimbusRomNo9L-Medi", (true, false, Family::Serif)),
            ("NimbusRomNo9L-ReguItal", (false, true, Family::Serif)),
            ("NimbusSanL-Bold", (true, false, Family::Sans)),
            ("SFRM1000", (false, false, Family::Serif)),
            ("SFTT1000", (false, false, Family::Mono)),
            ("CMBX12", (true, false, Family::Serif)),
            ("Arial-BoldItalicMT", (true, true, Family::Sans)),
            ("Unknown", (false, false, Family::Sans)),
        ];
        for (name, expected) in cases {
            assert_eq!(style(name, 0, None, None), expected, "{name}");
        }
    }

    #[test]
    fn family_names_drop_styles_and_vendor_suffixes() {
        let cases = [
            ("ArialMT", "Arial"),
            ("Arial-BoldItalicMT", "Arial"),
            ("TimesNewRomanPS-BoldMT", "TimesNewRoman"),
            ("TimesNewRomanPSMT", "TimesNewRoman"),
            ("Calibri,Bold", "Calibri"),
            ("Times-Roman", "Times"),
            ("Helvetica", "Helvetica"),
            ("NimbusRomNo9L-Medi", "NimbusRomNo9L"),
            ("", "Unnamed"),
        ];
        for (name, family) in cases {
            assert_eq!(family_name(name), family, "{name}");
        }
    }
}
