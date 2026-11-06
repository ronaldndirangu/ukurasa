//! The fonts a line can be drawn in, those in the document and the standard fonts, and how a
//! span of styled text picks one.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};

use lopdf::{Dictionary, Document, Encoding, Object, ObjectId};

use super::Span;
use super::read::{Scan, code_value, deref, deref_num, num};
use crate::EditError;
use crate::standard::{self, Family, Standard};

/// Prefix of the resource names Ukurasa gives the fonts it adds to a page.
const FONT_KEY: &str = "UkF";

/// The most decoded bytes a font's ToUnicode CMap may have.
const MAX_CMAP_BYTES: usize = 8 << 20;

/// The most codes a ToUnicode CMap's ranges may map, as lopdf lists each one.
const MAX_CMAP_CODES: u64 = 1 << 18;

/// A font: an indirect font object, or a font dictionary written directly in a page's
/// resources, by page and resource name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum FaceId {
    Object(ObjectId),
    Direct(ObjectId, Vec<u8>),
}

impl FaceId {
    pub(super) fn encode(&self) -> String {
        match self {
            FaceId::Object((number, generation)) => format!("{number} {generation}"),
            FaceId::Direct((number, generation), key) => {
                format!("{number} {generation} {}", String::from_utf8_lossy(key))
            }
        }
    }

    fn decode(text: &str) -> Option<FaceId> {
        let mut parts = text.splitn(3, ' ');
        let id = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
        Some(match parts.next() {
            Some(key) => FaceId::Direct(id, key.as_bytes().to_vec()),
            None => FaceId::Object(id),
        })
    }
}

/// The fonts page `page_id` can use, by resource name, as [`Document::get_page_fonts`] finds
/// them.
pub(super) fn page_fonts(
    doc: &Document,
    page_id: ObjectId,
) -> Result<BTreeMap<Vec<u8>, (FaceId, &Dictionary)>, EditError> {
    let (direct, inherited) = doc.get_page_resources(page_id)?;
    let resources = direct.into_iter().chain(
        inherited
            .into_iter()
            .filter_map(|id| doc.get_dictionary(id).ok()),
    );
    let mut fonts = BTreeMap::new();
    for resources in resources {
        let Some(dict) = resources
            .get(b"Font")
            .ok()
            .and_then(|o| deref(doc, o))
            .and_then(|o| o.as_dict().ok())
        else {
            continue;
        };
        for (name, value) in dict.iter() {
            if fonts.contains_key(name) {
                continue;
            }
            let font = match value {
                Object::Reference(id) => doc
                    .get_dictionary(*id)
                    .ok()
                    .map(|d| (FaceId::Object(*id), d)),
                Object::Dictionary(d) => Some((FaceId::Direct(page_id, name.clone()), d)),
                _ => None,
            };
            if let Some(font) = font {
                fonts.insert(name.clone(), font);
            }
        }
    }
    Ok(fonts)
}

/// Glyph advance widths in thousandths of an em.
pub(super) enum Metrics {
    Simple {
        first: u32,
        widths: Vec<f64>,
        missing: f64,
    },
    Cid {
        default: f64,
        /// Ranges by first code: last code and width.
        ranges: BTreeMap<u32, (u32, f64)>,
    },
    /// A standard font, or a font without `/Widths` measured as one.
    Standard(Standard),
}

impl Metrics {
    /// The width the font defines for `code`, if any.
    pub(super) fn get(&self, code: u32) -> Option<f64> {
        match self {
            Metrics::Simple { first, widths, .. } => code
                .checked_sub(*first)
                .and_then(|i| widths.get(i as usize).copied()),
            Metrics::Cid { default, ranges } => Some(
                ranges
                    .range(..=code)
                    .next_back()
                    .filter(|(_, (last, _))| code <= *last)
                    .map_or(*default, |(_, (_, width))| *width),
            ),
            Metrics::Standard(face) => u8::try_from(code)
                .ok()
                .map(|code| face.code_width(code))
                .filter(|w| *w > 0.0),
        }
    }

    pub(super) fn width(&self, code: u32) -> f64 {
        self.get(code).unwrap_or(match self {
            Metrics::Simple { missing, .. } => *missing,
            _ => 0.0,
        })
    }
}

/// A font text can be drawn in.
pub(super) struct Font<'a> {
    /// `None` for Ukurasa's standard fonts, which use WinAnsiEncoding.
    pub(super) encoding: Option<Encoding<'a>>,
    pub(super) metrics: Metrics,
    /// Two-byte Identity-H codes rather than one-byte codes.
    pub(super) cid: bool,
    /// The font program is in the file, so it may hold only the glyphs the file uses.
    pub(super) embedded: bool,
    pub(super) ascent: f64,
    pub(super) descent: f64,
    pub(super) base: Vec<u8>,
    pub(super) family_name: String,
    pub(super) bold: bool,
    pub(super) italic: bool,
    pub(super) family: Family,
}

fn descriptor<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    dict.get_deref(b"FontDescriptor", doc)
        .and_then(Object::as_dict)
        .ok()
}

pub(super) fn load_font<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<Font<'a>> {
    let subtype = dict.get(b"Subtype").and_then(Object::as_name).ok()?;
    let descendant = match subtype {
        b"Type3" => return None,
        b"Type0" => {
            let horizontal =
                dict.get(b"Encoding").and_then(Object::as_name).ok() == Some(b"Identity-H");
            let mapped = dict
                .get_deref(b"ToUnicode", doc)
                .and_then(Object::as_stream)
                .is_ok();
            if !(horizontal && mapped) {
                return None;
            }
            let fonts = dict
                .get_deref(b"DescendantFonts", doc)
                .and_then(Object::as_array)
                .ok()?;
            Some(deref(doc, fonts.first()?)?.as_dict().ok()?)
        }
        _ => None,
    };
    let base = dict
        .get(b"BaseFont")
        .and_then(Object::as_name)
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    let name = strip_subset(&String::from_utf8_lossy(&base)).to_string();
    let descriptor = descriptor(doc, descendant.unwrap_or(dict));
    let number = |key: &[u8]| {
        descriptor
            .and_then(|d| d.get(key).ok())
            .and_then(|o| deref_num(doc, o))
    };
    let flags = number(b"Flags").unwrap_or(0.0) as i64;
    let (bold, italic, family) = style(&name, flags, number(b"FontWeight"), number(b"ItalicAngle"));
    let fallback = Standard {
        family,
        bold,
        italic,
    };
    let metrics = metrics(doc, dict, descendant, &name, fallback)?;
    if !small_to_unicode(doc, dict) {
        return None;
    }
    let encoding = dict
        .get_font_encoding_with_limit(doc, MAX_CMAP_BYTES)
        .ok()?;
    let embedded = descriptor.is_some_and(|d| {
        [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
            .iter()
            .any(|key| d.has(key))
    });
    Some(Font {
        encoding: Some(encoding),
        metrics,
        cid: descendant.is_some(),
        embedded,
        ascent: number(b"Ascent")
            .filter(|a| (500.0..=1100.0).contains(a))
            .unwrap_or(750.0),
        descent: number(b"Descent")
            .filter(|d| (-500.0..=-50.0).contains(d))
            .unwrap_or(-250.0),
        base,
        family_name: family_name(&name),
        bold,
        italic,
        family,
    })
}

fn metrics(
    doc: &Document,
    dict: &Dictionary,
    descendant: Option<&Dictionary>,
    name: &str,
    fallback: Standard,
) -> Option<Metrics> {
    if let Some(cid) = descendant {
        let default = cid
            .get(b"DW")
            .ok()
            .and_then(|o| deref_num(doc, o))
            .unwrap_or(1000.0);
        let mut ranges = BTreeMap::new();
        if let Ok(w) = cid.get_deref(b"W", doc).and_then(Object::as_array) {
            let mut i = 0;
            while i + 1 < w.len() {
                let start = deref_num(doc, &w[i])? as u32;
                match deref(doc, &w[i + 1])? {
                    Object::Array(list) => {
                        for (j, width) in list.iter().enumerate() {
                            let code = start.checked_add(u32::try_from(j).ok()?)?;
                            ranges.insert(code, (code, deref_num(doc, width)?));
                        }
                        i += 2;
                    }
                    last => {
                        let width = deref_num(doc, w.get(i + 2)?)?;
                        ranges.insert(start, (num(last)? as u32, width));
                        i += 3;
                    }
                }
            }
        }
        return Some(Metrics::Cid { default, ranges });
    }
    if let Ok(widths) = dict.get_deref(b"Widths", doc).and_then(Object::as_array) {
        let missing = descriptor(doc, dict)
            .and_then(|d| d.get(b"MissingWidth").ok())
            .and_then(|o| deref_num(doc, o))
            .unwrap_or(0.0);
        let first = dict
            .get(b"FirstChar")
            .ok()
            .and_then(|o| deref_num(doc, o))? as u32;
        let widths = widths
            .iter()
            .map(|w| deref_num(doc, w).unwrap_or(missing))
            .collect();
        return Some(Metrics::Simple {
            first,
            widths,
            missing,
        });
    }
    let lower = name.to_ascii_lowercase();
    (!lower.contains("symbol") && !lower.contains("dingbat")).then_some(Metrics::Standard(fallback))
}

/// Whether the font has no ToUnicode CMap, or one small enough to read.
fn small_to_unicode(doc: &Document, dict: &Dictionary) -> bool {
    let Ok(stream) = dict
        .get_deref(b"ToUnicode", doc)
        .and_then(Object::as_stream)
    else {
        return true;
    };
    stream
        .get_plain_content_with_limit(MAX_CMAP_BYTES)
        .is_ok_and(|data| cmap_codes(&data) <= MAX_CMAP_CODES)
}

/// How many codes the `bfrange` sections of a CMap map.
pub(super) fn cmap_codes(data: &[u8]) -> u64 {
    const BEGIN: &[u8] = b"beginbfrange";
    let find = |data: &[u8], word: &[u8]| data.windows(word.len()).position(|w| w == word);
    let mut total = 0u64;
    let mut rest = data;
    while let Some(at) = find(rest, BEGIN) {
        rest = &rest[at + BEGIN.len()..];
        let end = find(rest, b"endbfrange").unwrap_or(rest.len());
        total = total.saturating_add(range_codes(&rest[..end]));
        rest = &rest[end..];
    }
    total
}

/// How many codes the `<start> <end> target` entries of a `bfrange` section map.
fn range_codes(section: &[u8]) -> u64 {
    let close = |from: usize, byte: u8| {
        section[from..]
            .iter()
            .position(|&b| b == byte)
            .map_or(section.len(), |p| from + p)
    };
    // Each top-level item: the value of a hex string, or `None` for an array.
    let mut items = Vec::new();
    let mut i = 0;
    while i < section.len() {
        match section[i] {
            b'<' => {
                let end = close(i, b'>');
                let value = section[i + 1..end]
                    .iter()
                    .filter_map(|b| (*b as char).to_digit(16))
                    .fold(0u64, |v, d| {
                        v.saturating_mul(16).saturating_add(u64::from(d))
                    });
                items.push(Some(value));
                i = end + 1;
            }
            b'[' => {
                items.push(None);
                i = close(i, b']') + 1;
            }
            _ => i += 1,
        }
    }
    items
        .chunks(3)
        .map(|entry| match entry {
            [Some(start), Some(end), _] if end >= start => (end - start).saturating_add(1),
            _ => 0,
        })
        .fold(0, u64::saturating_add)
}

/// A standard font as Ukurasa adds it to a page.
fn standard_font(face: Standard) -> Font<'static> {
    Font {
        encoding: None,
        metrics: Metrics::Standard(face),
        cid: false,
        embedded: false,
        ascent: 718.0,
        descent: -207.0,
        base: face.base_font().as_bytes().to_vec(),
        family_name: face.family_name().to_owned(),
        bold: face.bold,
        italic: face.italic,
        family: face.family,
    }
}

/// Drops the `ABCDEF+` tag subset fonts carry.
fn strip_subset(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
    }
}

/// The family a font belongs to, its name without style and vendor suffixes, so the faces of a
/// family list once.
pub(super) fn family_name(name: &str) -> String {
    if name.is_empty() {
        return "Unnamed".to_owned();
    }
    let stem = name.split([',', '-']).next().unwrap_or(name);
    let stem = stem.strip_suffix("MT").unwrap_or(stem);
    let stem = stem.strip_suffix("PS").unwrap_or(stem);
    if stem.is_empty() { name } else { stem }.to_owned()
}

/// Guesses weight, slant, and generic family from the font name and descriptor.
pub(super) fn style(
    name: &str,
    flags: i64,
    weight: Option<f64>,
    angle: Option<f64>,
) -> (bool, bool, Family) {
    let lower = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let tex = |prefixes: &[&str]| prefixes.iter().any(|p| lower.starts_with(p));
    let bold = has(&["bold", "black", "heavy", "semibold", "demi", "-medi"])
        || tex(&["cmb", "sfbx"])
        || weight.is_some_and(|w| w >= 600.0)
        || flags & (1 << 18) != 0;
    let italic = has(&["ital", "oblique"])
        || tex(&["cmti", "cmsl", "cmmi", "sfti", "sfsl"])
        || angle.is_some_and(|a| a.abs() > 1.0)
        || flags & (1 << 6) != 0;
    let family = if has(&["mono", "courier", "consol", "code"])
        || tex(&["cmtt", "sftt"])
        || flags & 1 != 0
    {
        Family::Mono
    } else if has(&[
        "sans",
        "arial",
        "helvet",
        "verdana",
        "calibri",
        "segoe",
        "tahoma",
        "nimbussan",
        "roboto",
        "lato",
        "futura",
        "gill",
        "franklin",
        "frutiger",
        "myriad",
        "univers",
        "avenir",
    ]) || tex(&["cmss", "sfss"])
    {
        Family::Sans
    } else if has(&[
        "serif",
        "times",
        "roman",
        "georgia",
        "garamond",
        "cambria",
        "palatino",
        "minion",
        "book",
        "nimbusrom",
        "termes",
        "pagella",
        "schola",
        "century",
        "baskerville",
        "caslon",
        "charter",
        "utopia",
        "libertine",
    ]) || tex(&["cm", "lm", "sfrm"])
        || flags & 2 != 0
    {
        Family::Serif
    } else {
        Family::Sans
    };
    (bold, italic, family)
}

/// The code that shows `c` in `font`, if the font has that glyph. A subset font only has the
/// glyphs the file shows, so its codes come from text the file already draws with it.
fn font_code(font: &Font, c: char, scan: &mut Scan) -> Option<Vec<u8>> {
    let Some(encoding) = &font.encoding else {
        let code = standard::encode(c)?;
        return font.metrics.get(code.into()).map(|_| vec![code]);
    };
    let code = if font.embedded {
        scan.find(&font.base, c)?
    } else {
        Document::encode_text(encoding, &c.to_string())
    };
    let length = if font.cid { 2 } else { 1 };
    let decoded = Document::decode_text(encoding, &code).ok()?;
    let defined = font.metrics.get(code_value(&code)).is_some_and(|w| w > 0.0);
    (code.len() == length && decoded.chars().eq([c]) && defined).then_some(code)
}

/// A font added to a page under a new resource name.
pub(super) enum Added {
    Object(ObjectId),
    Standard(Standard),
}

enum Source {
    Id(FaceId),
    Standard(Standard),
}

struct Face<'a> {
    font: Font<'a>,
    source: Source,
    /// The font's resource name on the edited page.
    key: Option<Vec<u8>>,
    /// A page that uses the font, to search for the glyphs of a subset.
    home: Option<ObjectId>,
}

/// The face that draws a span, and the styles to synthesize because the face lacks them.
pub(super) struct Choice {
    pub(super) face: usize,
    pub(super) bold: bool,
    pub(super) italic: bool,
}

impl Choice {
    fn plain(face: usize) -> Self {
        Choice {
            face,
            bold: false,
            italic: false,
        }
    }
}

/// The fonts an edited page can draw in, loaded as spans ask for them.
pub(super) struct Faces<'a> {
    doc: &'a Document,
    pages: &'a [ObjectId],
    page_id: ObjectId,
    list: Vec<Face<'a>>,
    /// Every font in the document is in `list`.
    complete: bool,
    keys: HashMap<FaceId, Vec<u8>>,
    names: HashSet<Vec<u8>>,
    added: Vec<(Vec<u8>, Added)>,
    codes: HashMap<(usize, char), Option<Vec<u8>>>,
}

impl<'a> Faces<'a> {
    pub(super) fn new(
        doc: &'a Document,
        pages: &'a [ObjectId],
        page_id: ObjectId,
        fonts: HashMap<Vec<u8>, Option<Font<'a>>>,
        ids: BTreeMap<Vec<u8>, FaceId>,
    ) -> Self {
        let mut keys = HashMap::new();
        for (key, id) in &ids {
            keys.entry(id.clone()).or_insert_with(|| key.clone());
        }
        let mut faces = Faces {
            doc,
            pages,
            page_id,
            list: Vec::new(),
            complete: false,
            keys,
            names: ids.keys().cloned().collect(),
            added: Vec::new(),
            codes: HashMap::new(),
        };
        let mut fonts: Vec<_> = fonts.into_iter().collect();
        fonts.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, font) in fonts {
            if let (Some(font), Some(id)) = (font, ids.get(&key))
                && faces.find(id).is_none()
            {
                faces.push(font, Source::Id(id.clone()), Some(page_id));
            }
        }
        faces
    }

    fn push(&mut self, font: Font<'a>, source: Source, home: Option<ObjectId>) -> usize {
        let key = match &source {
            Source::Id(id) => self.keys.get(id).cloned(),
            Source::Standard(_) => None,
        };
        self.list.push(Face {
            font,
            source,
            key,
            home,
        });
        self.list.len() - 1
    }

    fn find(&self, id: &FaceId) -> Option<usize> {
        self.list
            .iter()
            .position(|face| matches!(&face.source, Source::Id(i) if i == id))
    }

    pub(super) fn font(&self, face: usize) -> &Font<'a> {
        &self.list[face].font
    }

    pub(super) fn standard(&mut self, standard: Standard) -> usize {
        match self
            .list
            .iter()
            .position(|face| matches!(face.source, Source::Standard(s) if s == standard))
        {
            Some(face) => face,
            None => self.push(standard_font(standard), Source::Standard(standard), None),
        }
    }

    /// The face a span was read in, loaded if this page doesn't use it.
    fn hinted(&mut self, id: &FaceId) -> Option<usize> {
        if let Some(face) = self.find(id) {
            return Some(face);
        }
        let dict = match id {
            FaceId::Object(object) => self.doc.get_dictionary(*object).ok()?,
            FaceId::Direct(page, key) if *page == self.page_id => {
                page_fonts(self.doc, *page).ok()?.remove(key)?.1
            }
            FaceId::Direct(..) => return None,
        };
        let font = load_font(self.doc, dict)?;
        Some(self.push(font, Source::Id(id.clone()), None))
    }

    /// Adds every font in the document. Fonts written directly in another page's resources
    /// can't be shared, so they're left out.
    fn load_all(&mut self) {
        if std::mem::replace(&mut self.complete, true) {
            return;
        }
        for &page in self.pages {
            let Ok(fonts) = page_fonts(self.doc, page) else {
                continue;
            };
            for (id, dict) in fonts.into_values() {
                if matches!(&id, FaceId::Direct(p, _) if *p != self.page_id) {
                    continue;
                }
                match self.find(&id) {
                    Some(face) => {
                        self.list[face].home.get_or_insert(page);
                    }
                    None => {
                        if let Some(font) = load_font(self.doc, dict) {
                            self.push(font, Source::Id(id), Some(page));
                        }
                    }
                }
            }
        }
    }

    /// The code that shows `c` in `face`, if it can.
    pub(super) fn code(&mut self, face: usize, c: char, scan: &mut Scan) -> Option<Vec<u8>> {
        if let Some(code) = self.codes.get(&(face, c)) {
            return code.clone();
        }
        let entry = &self.list[face];
        if entry.font.embedded
            && let Some(home) = entry.home
        {
            scan.include(home);
        }
        let code = font_code(&entry.font, c, scan);
        self.codes.insert((face, c), code.clone());
        code
    }

    /// How many characters of `text` `face` shows. Spaces count, as a word space stands in for
    /// a missing space glyph.
    fn coverage(&mut self, face: usize, text: &str, scan: &mut Scan) -> usize {
        text.chars()
            .filter(|&c| c == ' ' || self.code(face, c, scan).is_some())
            .count()
    }

    /// The candidate closest to the `wanted` bold and italic styles that shows the most of
    /// `text`, preferring the face the text was read in, then fonts already on the page.
    fn best(
        &mut self,
        candidates: &[usize],
        hint: Option<usize>,
        wanted: (bool, bool),
        text: &str,
        scan: &mut Scan,
    ) -> Option<usize> {
        let mut best = None;
        for &face in candidates {
            let covered = self.coverage(face, text, scan);
            let entry = &self.list[face];
            let (bold, italic) = (entry.font.bold, entry.font.italic);
            let rank = (
                !(bold && !wanted.0 || italic && !wanted.1),
                usize::from(bold == wanted.0) + usize::from(italic == wanted.1),
                covered,
                Some(face) == hint,
                entry.key.is_some(),
                Reverse(face),
            );
            if best.as_ref().is_none_or(|(top, _)| rank > *top) {
                best = Some((rank, face));
            }
        }
        best.map(|(_, face)| face)
    }

    /// Picks the face that draws `span`: the face it was read in while its font and style still
    /// match, then a face of its font in its style, then a standard font of that name, then the
    /// closest face of its font with the missing styles synthesized, and last a standard font
    /// of its generic family.
    pub(super) fn resolve(&mut self, span: &Span, scan: &mut Scan) -> Choice {
        let total = span.text.chars().count();
        let wanted = (span.bold, span.italic);
        let hint = span
            .face
            .as_deref()
            .and_then(FaceId::decode)
            .and_then(|id| self.hinted(&id));
        if let Some(face) = hint {
            let font = &self.list[face].font;
            if font.family_name == span.font
                && (font.bold, font.italic) == wanted
                && self.coverage(face, &span.text, scan) == total
            {
                return Choice::plain(face);
            }
        }
        self.load_all();
        let family: Vec<usize> = (0..self.list.len())
            .filter(|&face| self.list[face].font.family_name == span.font)
            .collect();
        let styled: Vec<usize> = family
            .iter()
            .copied()
            .filter(|&face| {
                let font = &self.list[face].font;
                (font.bold, font.italic) == wanted
            })
            .collect();
        let standard = Standard::named(&span.font);
        if let Some(face) = self.best(&styled, hint, wanted, &span.text, scan)
            && (standard.is_none() || self.coverage(face, &span.text, scan) == total)
        {
            return Choice::plain(face);
        }
        if let Some(family) = standard {
            let face = self.standard(Standard {
                family,
                bold: span.bold,
                italic: span.italic,
            });
            return Choice::plain(face);
        }
        if let Some(face) = self.best(&family, hint, wanted, &span.text, scan) {
            let font = &self.list[face].font;
            return Choice {
                face,
                bold: span.bold && !font.bold,
                italic: span.italic && !font.italic,
            };
        }
        let face = self.standard(Standard {
            family: span.family,
            bold: span.bold,
            italic: span.italic,
        });
        Choice::plain(face)
    }

    /// The resource name `face` has on the edited page, adding it under a new name if needed.
    pub(super) fn key(&mut self, face: usize) -> Vec<u8> {
        if let Some(key) = &self.list[face].key {
            return key.clone();
        }
        let key = (1..)
            .map(|i| format!("{FONT_KEY}{i}").into_bytes())
            .find(|key| !self.names.contains(key))
            .expect("an unused name exists");
        self.names.insert(key.clone());
        let entry = &mut self.list[face];
        entry.key = Some(key.clone());
        let added = match &entry.source {
            Source::Id(FaceId::Object(id)) => Added::Object(*id),
            Source::Id(FaceId::Direct(..)) => unreachable!("direct fonts on the page have names"),
            Source::Standard(standard) => Added::Standard(*standard),
        };
        if let Source::Id(id) = &entry.source {
            self.keys.insert(id.clone(), key.clone());
        }
        self.added.push((key.clone(), added));
        key
    }

    /// The fonts to add to the page's resources, by name.
    pub(super) fn into_added(self) -> Vec<(Vec<u8>, Added)> {
        self.added
    }
}
