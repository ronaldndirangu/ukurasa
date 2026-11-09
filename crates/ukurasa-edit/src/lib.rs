//! PDF writer for annotations, the AcroForm model, appearance streams, and page operations.
//!
//! Edits are saved as an incremental update: the original bytes stay intact and the new and
//! changed objects are appended after them. Removing a picture rewrites the whole file instead,
//! so no earlier revision keeps it.

mod images;
mod lines;
mod path;
mod script;
mod standard;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub use images::{Bounds, ImageEdit, PageImage, page_images};
pub use lines::{DocumentFont, LineEdit, LineReport, Span, TextLine, document_fonts, text_lines};
pub use lopdf;
use lopdf::{
    Dictionary, Document, IncrementalDocument, Object, ObjectId, Permissions, Stream, dictionary,
};
pub use script::typed_signature;
use serde::{Deserialize, Serialize};
pub use standard::Family;
use ukurasa_geom::{PageGeometry, Rect};

/// One change to place on a page. Positions are display points from the page's top-left
/// corner, as the page is shown after cropping and rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Edit {
    Text(TextBox),
    Signature(Signature),
    Line(LineEdit),
    Image(ImageEdit),
}

impl Edit {
    pub fn page(&self) -> usize {
        match self {
            Edit::Text(t) => t.page,
            Edit::Signature(s) => s.page,
            Edit::Line(l) => l.page,
            Edit::Image(i) => i.page,
        }
    }
}

/// Rows of styled text drawn into a page's content, each row a line high in its largest font.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextBox {
    /// Zero-based page index.
    pub page: usize,
    /// Top-left corner of the first row.
    pub x: f64,
    pub y: f64,
    /// The rows from top to bottom. Tabs show as spaces. A row's spans set its height even when
    /// empty; a row without spans is as high as the one above.
    pub lines: Vec<Vec<Span>>,
}

/// A drawn or typed signature, scaled to fill its box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub page: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub path: VectorPath,
    pub color: [u8; 3],
}

/// Vector artwork in a `width` by `height` box with a top-left origin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorPath {
    /// SVG path data restricted to absolute `M`, `L`, `Q`, `C`, and `Z` commands.
    pub d: String,
    pub width: f64,
    pub height: f64,
    /// Fill the outlines, or stroke them with round caps and joins.
    pub fill: bool,
    pub stroke_width: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("Couldn't read the PDF: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("Couldn't write the PDF: {0}")]
    Io(#[from] std::io::Error),
    #[error("This PDF needs a password, so edits can't be saved yet")]
    PasswordRequired,
    #[error("The owner of this PDF doesn't allow adding signatures")]
    Restricted,
    #[error("The owner of this PDF doesn't allow changing its content")]
    RestrictedContent,
    #[error("This encrypted PDF is damaged, so edits can't be saved")]
    DamagedEncrypted,
    #[error("Ukurasa can't edit PDFs encrypted this way yet")]
    UnsupportedEncryption,
    #[error("Page {0} doesn't exist")]
    NoPage(usize),
    #[error("Neither the text's font nor the fallback font has these characters: {0}")]
    UnsupportedLineText(String),
    #[error("The text on page {0} no longer matches this edit")]
    StaleText(usize),
    #[error("The picture on page {0} no longer matches this edit")]
    StaleImage(usize),
    #[error("Ukurasa can't remove pictures from an encrypted PDF yet")]
    EncryptedImageRemoval,
    #[error("Ukurasa can't change the text on this page without losing some of its content")]
    UnsafeContent,
    #[error("This page's content is too large to edit")]
    LargeContent,
    #[error("The signature font doesn't have these characters: {0}")]
    UnsupportedSignatureText(String),
    #[error("Type a name to sign")]
    EmptySignature,
    #[error("Invalid signature path: {0}")]
    InvalidPath(String),
    #[error("Invalid edit: {0}")]
    Invalid(&'static str),
    #[error("The edited PDF didn't pass verification, so it wasn't saved")]
    Verify,
}

/// Applies `edits` to the PDF in `source` and returns the bytes of the saved file. When the
/// edits remove a picture, the file is rewritten without the objects nothing uses anymore, so
/// the picture can't be recovered from an earlier revision.
pub fn apply(source: &[u8], edits: &[Edit]) -> Result<Vec<u8>, EditError> {
    let doc = Document::load_mem(source)?;
    let pages = doc.get_pages().len();
    let applied = apply_edits(source, doc, edits, removes_images(edits))?;
    match Document::load_mem(&applied.bytes) {
        Ok(saved) if saved.get_pages().len() == pages => Ok(applied.bytes),
        _ => Err(EditError::Verify),
    }
}

/// The result of [`apply_document`].
#[derive(Debug)]
pub struct Applied {
    pub bytes: Vec<u8>,
    pub lines: Vec<LineReport>,
}

/// Applies `edits` to `doc`, loaded from `source`, without verifying the output. The result is
/// an incremental update, quick to preview, that keeps the pictures the edits remove in the
/// original revision.
pub fn apply_document(source: &[u8], doc: Document, edits: &[Edit]) -> Result<Applied, EditError> {
    apply_edits(source, doc, edits, false)
}

fn removes_images(edits: &[Edit]) -> bool {
    edits
        .iter()
        .any(|edit| matches!(edit, Edit::Image(ImageEdit { bounds: None, .. })))
}

/// Applies `edits`, and with `purge` rewrites the file without the objects nothing uses.
fn apply_edits(
    source: &[u8],
    doc: Document,
    edits: &[Edit],
    purge: bool,
) -> Result<Applied, EditError> {
    let mut by_page: BTreeMap<usize, lines::PageEdits> = BTreeMap::new();
    for edit in edits {
        match edit {
            Edit::Line(line) => by_page.entry(line.page).or_default().lines.push(line),
            Edit::Text(text) => {
                if check_text(text)? {
                    by_page.entry(text.page).or_default().texts.push(text);
                }
            }
            Edit::Image(image) => by_page.entry(image.page).or_default().images.push(image),
            Edit::Signature(_) => {}
        }
    }
    let signatures = edits.iter().any(|e| matches!(e, Edit::Signature(_)));
    check_encryption(&doc, !by_page.is_empty(), signatures)?;
    if removes_images(edits) && doc.encryption_state.is_some() {
        return Err(EditError::EncryptedImageRemoval);
    }
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    if let Some(edit) = edits.iter().find(|e| e.page() >= pages.len()) {
        return Err(EditError::NoPage(edit.page() + 1));
    }
    let mut changes = Changes {
        doc: &doc,
        objects: BTreeMap::new(),
        max_id: doc.max_id,
        removed: HashSet::new(),
    };
    let mut lines = Vec::new();
    let mut standard_fonts = HashMap::new();
    for (&page, page_edits) in &by_page {
        lines.extend(lines::apply_page(
            &mut changes,
            &pages,
            page,
            page_edits,
            &mut standard_fonts,
            purge,
        )?);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    for (index, edit) in edits.iter().enumerate() {
        let Edit::Signature(signature) = edit else {
            continue;
        };
        let annotation = signature_annotation(signature)?;
        let name = format!("ukurasa-{}-{index}", now.as_nanos());
        changes.add_annotation(pages[signature.page], annotation, &name, now.as_secs())?;
    }
    let Changes {
        objects,
        max_id,
        removed,
        ..
    } = changes;
    let bytes = write(source, doc, objects, max_id, purge.then_some(&removed))?;
    Ok(Applied { bytes, lines })
}

/// Fails unless `text` can be drawn, and returns whether it shows anything.
fn check_text(text: &TextBox) -> Result<bool, EditError> {
    if !(text.x.is_finite() && text.y.is_finite()) {
        return Err(EditError::Invalid("text position must be finite"));
    }
    for row in &text.lines {
        lines::check_spans(row)?;
        if row.iter().any(|s| s.text.contains(['\n', '\r'])) {
            return Err(EditError::Invalid("a row of text has a line break"));
        }
    }
    Ok(text
        .lines
        .iter()
        .flatten()
        .any(|s| s.text.chars().any(|c| !c.is_whitespace())))
}

/// Fails when a password or the owner's permissions keep line edits from being saved, before
/// [`text_lines`] reads the document.
pub fn check_line_editing(doc: &Document) -> Result<(), EditError> {
    check_encryption(doc, true, false)
}

/// The update to an encrypted PDF is encrypted with the document's key, which needs the
/// owner's permission to modify `content` and to add `annotations`, and an intact
/// cross-reference chain to append to.
fn check_encryption(doc: &Document, content: bool, annotations: bool) -> Result<(), EditError> {
    if doc.is_encrypted() {
        return Err(EditError::PasswordRequired);
    }
    let Some(state) = &doc.encryption_state else {
        // lopdf reads only an `/Encrypt` reference; with a direct dictionary it loads no objects.
        if doc.trailer.has(b"Encrypt") {
            return Err(EditError::UnsupportedEncryption);
        }
        return Ok(());
    };
    if content && !state.permissions().contains(Permissions::MODIFIABLE) {
        return Err(EditError::RestrictedContent);
    }
    if annotations && !state.permissions().contains(Permissions::ANNOTABLE) {
        return Err(EditError::Restricted);
    }
    if state.encrypt_object_id().is_none() || needs_rewrite(doc) {
        return Err(EditError::DamagedEncrypted);
    }
    Ok(())
}

/// A file lopdf had to repair has no reliable cross-reference chain to append to.
fn needs_rewrite(doc: &Document) -> bool {
    doc.xref_start == 0 || doc.xref_recovery.is_some()
}

/// Whether saving `edits` into the PDF in `source` invalidates signatures already in it.
/// Signature stamps are appended as annotations and keep them valid; edits that change page
/// content, or rewriting a damaged file, break them.
pub fn breaks_signatures(source: &[u8], edits: &[Edit]) -> Result<bool, EditError> {
    if edits.is_empty() {
        return Ok(false);
    }
    let doc = Document::load_mem(source)?;
    let content = edits.iter().any(|edit| !matches!(edit, Edit::Signature(_)));
    Ok((content || needs_rewrite(&doc)) && is_signed(&doc))
}

/// Whether a signature field in `doc` holds a signature.
pub fn is_signed(doc: &Document) -> bool {
    let fields = doc
        .catalog()
        .and_then(|catalog| catalog.get_deref(b"AcroForm", doc))
        .and_then(Object::as_dict)
        .and_then(|form| form.get_deref(b"Fields", doc))
        .and_then(Object::as_array);
    let Ok(fields) = fields else {
        return false;
    };
    let mut pending: Vec<(&Object, bool)> = fields.iter().map(|field| (field, false)).collect();
    let mut seen = HashSet::new();
    while let Some((field, inherited)) = pending.pop() {
        if let Object::Reference(id) = field
            && !seen.insert(*id)
        {
            continue;
        }
        let Ok(field) = doc
            .dereference(field)
            .and_then(|(_, field)| field.as_dict())
        else {
            continue;
        };
        let signature = match field.get_deref(b"FT", doc).and_then(Object::as_name) {
            Ok(kind) => kind == b"Sig",
            Err(_) => inherited,
        };
        if signature
            && field
                .get_deref(b"V", doc)
                .is_ok_and(|value| !matches!(value, Object::Null))
        {
            return true;
        }
        if let Ok(kids) = field.get_deref(b"Kids", doc).and_then(Object::as_array) {
            pending.extend(kids.iter().map(|kid| (kid, signature)));
        }
    }
    false
}

/// The page box and rotation PDFium uses to display page `index`.
pub fn page_geometry(doc: &Document, page_id: ObjectId) -> PageGeometry {
    let rect = |key: &[u8]| inherited(doc, page_id, key).and_then(|o| rect(doc, o));
    let rotate = match inherited(doc, page_id, b"Rotate") {
        Some(Object::Integer(r)) => *r,
        Some(Object::Real(r)) => *r as i64,
        _ => 0,
    };
    PageGeometry::new(rect(b"MediaBox"), rect(b"CropBox"), rotate)
}

fn inherited<'a>(doc: &'a Document, page_id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut dict = doc.get_dictionary(page_id).ok()?;
    for _ in 0..64 {
        if let Ok(value) = dict.get(key) {
            return doc.dereference(value).ok().map(|(_, o)| o);
        }
        let parent = dict.get(b"Parent").and_then(Object::as_reference).ok()?;
        dict = doc.get_dictionary(parent).ok()?;
    }
    None
}

fn rect(doc: &Document, object: &Object) -> Option<Rect> {
    let values = object.as_array().ok()?;
    if values.len() != 4 {
        return None;
    }
    let mut n = [0.0; 4];
    for (slot, value) in n.iter_mut().zip(values) {
        *slot = f64::from(doc.dereference(value).ok()?.1.as_float().ok()?);
    }
    Some(Rect::new(n[0], n[1], n[2], n[3]))
}

/// An annotation to add, with its appearance drawn in a display-oriented box.
struct Annotation {
    subtype: &'static str,
    /// Display rectangle: top-left corner and size.
    bounds: (f64, f64, f64, f64),
    appearance: String,
    resources: Option<Dictionary>,
    extra: Dictionary,
}

fn signature_annotation(signature: &Signature) -> Result<Annotation, EditError> {
    let Signature {
        x,
        y,
        width,
        height,
        ref path,
        color,
        ..
    } = *signature;
    let positive = |n: f64| n.is_finite() && n > 0.0;
    if !(x.is_finite() && y.is_finite() && positive(width) && positive(height)) {
        return Err(EditError::Invalid(
            "signature box must be positive and finite",
        ));
    }
    if !(positive(path.width) && positive(path.height)) {
        return Err(EditError::InvalidPath(
            "the path box must be positive".into(),
        ));
    }
    let segments = path::parse(&path.d)?;
    let color = rgb(color);
    let (sx, sy) = (width / path.width, height / path.height);

    let mut ap = String::from("q\n");
    if path.fill {
        writeln!(ap, "{color} rg").unwrap();
    } else {
        writeln!(
            ap,
            "{color} RG\n1 J 1 j {} w",
            path::number(path.stroke_width)
        )
        .unwrap();
    }
    writeln!(
        ap,
        "{} 0 0 {} 0 {} cm",
        path::number(sx),
        path::number(-sy),
        path::number(height)
    )
    .unwrap();
    path::write_pdf(&segments, &mut ap);
    ap.push_str(if path.fill { "f\nQ\n" } else { "S\nQ\n" });

    Ok(Annotation {
        subtype: "Stamp",
        bounds: (x, y, width, height),
        appearance: ap,
        resources: None,
        extra: dictionary! { "Contents" => lopdf::text_string("Signature") },
    })
}

fn rgb(color: [u8; 3]) -> String {
    let [r, g, b] = color.map(|c| path::number(f64::from(c) / 255.0));
    format!("{r} {g} {b}")
}

fn reals(values: &[f64]) -> Object {
    Object::Array(
        values
            .iter()
            .map(|v| Object::Real(((v * 1000.0).round() / 1000.0) as f32))
            .collect(),
    )
}

/// New and changed objects, computed against the unchanged source document.
struct Changes<'a> {
    doc: &'a Document,
    objects: BTreeMap<ObjectId, Object>,
    max_id: u32,
    /// The XObjects of the pictures the edits take off a page.
    removed: HashSet<ObjectId>,
}

impl Changes<'_> {
    fn add(&mut self, object: impl Into<Object>) -> Result<ObjectId, EditError> {
        self.max_id = self
            .max_id
            .checked_add(1)
            .ok_or(EditError::Pdf(lopdf::Error::ObjectIdOverflow))?;
        let id = (self.max_id, 0);
        self.objects.insert(id, object.into());
        Ok(id)
    }

    fn modify(&mut self, id: ObjectId) -> Result<&mut Object, EditError> {
        if !self.objects.contains_key(&id) {
            let object = self.doc.get_object(id)?.clone();
            self.objects.insert(id, object);
        }
        Ok(self.objects.get_mut(&id).expect("inserted above"))
    }

    fn add_annotation(
        &mut self,
        page_id: ObjectId,
        annotation: Annotation,
        name: &str,
        now: u64,
    ) -> Result<(), EditError> {
        let geometry = page_geometry(self.doc, page_id);
        let (x, y, w, h) = annotation.bounds;
        let mut form = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => reals(&[0.0, 0.0, w, h]),
            "Matrix" => reals(&geometry.upright_matrix()),
        };
        if let Some(resources) = annotation.resources {
            form.set("Resources", resources);
        }
        let mut stream = Stream::new(form, annotation.appearance.into_bytes());
        // Uncompressed appearances are still valid, so a compression failure is not fatal.
        let _ = stream.compress();
        let appearance = self.add(stream)?;

        let user = geometry.rect_to_user(x, y, w, h);
        let mut dict = dictionary! {
            "Type" => "Annot",
            "Subtype" => annotation.subtype,
            "Rect" => reals(&[user.x0, user.y0, user.x1, user.y1]),
            "F" => 4,
            "P" => page_id,
            "NM" => lopdf::text_string(name),
            "M" => Object::string_literal(pdf_date(now)),
            "AP" => dictionary! { "N" => appearance },
        };
        dict.extend(&annotation.extra);
        let annot = self.add(dict)?;

        // Other pages may share an indirect array, so the page gets its own.
        let doc = self.doc;
        let page = self.modify(page_id)?.as_dict_mut()?;
        let mut annots = match page.get(b"Annots") {
            Ok(Object::Array(annots)) => annots.clone(),
            Ok(&Object::Reference(id)) => doc
                .get_object(id)
                .and_then(Object::as_array)
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        annots.push(annot.into());
        page.set("Annots", annots);
        Ok(())
    }
}

/// Writes `objects` into `doc` as an incremental update, or rewrites the whole file when its
/// cross-reference chain is damaged or `purge` asks to drop the objects nothing uses, including
/// the pictures it lists that no page draws anymore.
fn write(
    source: &[u8],
    mut doc: Document,
    objects: BTreeMap<ObjectId, Object>,
    max_id: u32,
    purge: Option<&HashSet<ObjectId>>,
) -> Result<Vec<u8>, EditError> {
    let mut trailer = Dictionary::new();
    for key in [b"Root".as_slice(), b"Info", b"ID"] {
        if let Ok(value) = doc.trailer.get(key) {
            trailer.set(key, value.clone());
        }
    }
    let mut out = Vec::with_capacity(source.len() + 64 * 1024);
    if purge.is_some() || needs_rewrite(&doc) {
        doc.trailer = trailer;
        doc.objects.extend(objects);
        doc.max_id = max_id;
        if let Some(removed) = purge {
            images::forget(&mut doc, removed);
            doc.prune_objects();
        }
        doc.save_to(&mut out)?;
    } else {
        let mut incremental = IncrementalDocument::create_from(source.to_vec(), doc);
        let update = &mut incremental.new_document;
        if let Ok(prev) = update.trailer.get(b"Prev") {
            trailer.set("Prev", prev.clone());
        }
        update.trailer = trailer;
        update.objects.extend(objects);
        update.max_id = max_id;
        incremental.save_to(&mut out)?;
    }
    Ok(out)
}

/// Formats seconds since the Unix epoch as a PDF date in UTC.
fn pdf_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days, after Howard Hinnant's date algorithms.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "D:{year:04}{month:02}{day:02}{:02}{:02}{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests;
