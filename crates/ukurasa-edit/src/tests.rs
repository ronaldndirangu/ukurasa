use lopdf::{Document, Object, Stream, dictionary};

use super::*;
use crate::standard::Standard;

fn sample(pages: &[(i64, Option<[f64; 4]>)]) -> Document {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for &(rotate, crop) in pages {
        let content = doc.add_object(Stream::new(dictionary! {}, b"0 0 m 10 10 l S".to_vec()));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content,
            "Rotate" => rotate,
        };
        if let Some(crop) = crop {
            page.set("CropBox", reals(&crop));
        }
        kids.push(doc.add_object(page).into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
            "MediaBox" => reals(&[0.0, 0.0, 612.0, 792.0]),
        }
        .into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc
}

fn save(doc: &mut Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

/// A one-page sample encrypted with an empty user password, using RC4 or AES-128.
fn encrypted(permissions: Permissions, aes: bool) -> Vec<u8> {
    use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
    use lopdf::{EncryptionState, EncryptionVersion};
    use std::sync::Arc;

    let mut doc = sample(&[(0, None)]);
    let id = Object::string_literal(b"0123456789abcdef".to_vec());
    doc.trailer.set("ID", vec![id.clone(), id]);
    let version = if aes {
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
        EncryptionVersion::V4 {
            document: &doc,
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: "owner",
            user_password: "",
            permissions,
        }
    } else {
        EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner",
            user_password: "",
            key_length: 128,
            permissions,
        }
    };
    let state = EncryptionState::try_from(version).unwrap();
    doc.encrypt(&state).unwrap();
    save(&mut doc)
}

fn helvetica(text: &str) -> Span {
    Span {
        text: text.into(),
        font: "Helvetica".into(),
        face: None,
        family: Family::Sans,
        bold: false,
        italic: false,
        underline: false,
        size: 12.0,
        color: [0, 0, 0],
    }
}

fn text_box(page: usize, x: f64, y: f64, lines: Vec<Vec<Span>>) -> Edit {
    Edit::Text(TextBox { page, x, y, lines })
}

/// Rows of 12pt Helvetica at (72, 100), one for each line of `text`.
fn text(page: usize, text: &str) -> Edit {
    let rows = text
        .split('\n')
        .map(|row| match row {
            "" => Vec::new(),
            row => vec![helvetica(row)],
        })
        .collect();
    text_box(page, 72.0, 100.0, rows)
}

fn signature(page: usize, x: f64, y: f64, width: f64, height: f64) -> Edit {
    Edit::Signature(Signature {
        page,
        x,
        y,
        width,
        height,
        path: VectorPath {
            d: "M 1 1 Q 5 9 9 1".into(),
            width: 10.0,
            height: 10.0,
            fill: false,
            stroke_width: 1.5,
        },
        color: [0, 0, 255],
    })
}

fn annotations(doc: &Document, page: u32) -> Vec<&Dictionary> {
    let id = doc.get_pages()[&page];
    doc.get_page_annotations(id).unwrap()
}

fn numbers(object: &Object) -> Vec<f64> {
    object
        .as_array()
        .unwrap()
        .iter()
        .map(|n| f64::from(n.as_float().unwrap()))
        .collect()
}

fn appearance<'a>(doc: &'a Document, annot: &Dictionary) -> &'a Stream {
    let ap = annot.get(b"AP").unwrap().as_dict().unwrap();
    let id = ap.get(b"N").unwrap().as_reference().unwrap();
    doc.get_object(id).unwrap().as_stream().unwrap()
}

#[test]
fn appends_an_incremental_update() {
    let source = save(&mut sample(&[(0, None)]));
    let output = apply(&source, &[signature(0, 10.0, 10.0, 100.0, 40.0)]).unwrap();
    assert!(output.starts_with(&source));
    let tail = String::from_utf8_lossy(&output[source.len()..]);
    assert!(tail.contains("/Prev"), "{tail}");
    assert!(!tail.contains("%PDF"));

    let doc = Document::load_mem(&output).unwrap();
    assert_eq!(doc.get_pages().len(), 1);
    assert_eq!(annotations(&doc, 1).len(), 1);
}

#[test]
fn keeps_the_encryption_of_encrypted_sources() {
    let permissions = Permissions::PRINTABLE | Permissions::ANNOTABLE | Permissions::MODIFIABLE;
    for aes in [false, true] {
        let source = encrypted(permissions, aes);
        let edits = [text(0, "Hello"), signature(0, 10.0, 10.0, 100.0, 40.0)];
        let output = apply(&source, &edits).unwrap();
        assert!(output.starts_with(&source));
        let tail = String::from_utf8_lossy(&output[source.len()..]);
        assert!(tail.contains("/Encrypt"), "{tail}");

        let doc = Document::load_mem(&output).unwrap();
        assert!(doc.was_encrypted());
        assert_eq!(annotations(&doc, 1).len(), 1);
        assert_eq!(lines_of(&output)[0].text, "Hello");
    }
}

#[test]
fn explains_encryption_it_cannot_read() {
    let mut doc = sample(&[(0, None)]);
    doc.trailer.set(
        "Encrypt",
        dictionary! { "Filter" => "Standard", "V" => 4, "R" => 4 },
    );
    let source = save(&mut doc);
    let loaded = Document::load_mem(&source).unwrap();
    assert!(matches!(
        check_line_editing(&loaded),
        Err(EditError::UnsupportedEncryption)
    ));
    assert!(matches!(
        apply(&source, &[signature(0, 10.0, 10.0, 100.0, 40.0)]),
        Err(EditError::UnsupportedEncryption)
    ));
}

#[test]
fn refuses_when_the_owner_restricts_annotations() {
    let source = encrypted(Permissions::PRINTABLE | Permissions::MODIFIABLE, false);
    assert!(matches!(
        apply(&source, &[signature(0, 10.0, 10.0, 100.0, 40.0)]),
        Err(EditError::Restricted)
    ));
}

#[test]
fn maps_rotated_and_cropped_pages() {
    let crop = Some([10.0, 20.0, 210.0, 320.0]);
    let source = save(&mut sample(&[
        (0, crop),
        (90, crop),
        (180, crop),
        (270, crop),
    ]));
    let edits: Vec<Edit> = (0..4)
        .map(|p| signature(p, 30.0, 40.0, 50.0, 20.0))
        .collect();
    let doc = Document::load_mem(&apply(&source, &edits).unwrap()).unwrap();
    let expected = [
        ([40.0, 260.0, 90.0, 280.0], [1.0, 0.0, 0.0, 1.0]),
        ([50.0, 50.0, 70.0, 100.0], [0.0, 1.0, -1.0, 0.0]),
        ([130.0, 60.0, 180.0, 80.0], [-1.0, 0.0, 0.0, -1.0]),
        ([150.0, 240.0, 170.0, 290.0], [0.0, -1.0, 1.0, 0.0]),
    ];
    for (page, (rect, matrix)) in expected.iter().enumerate() {
        let annots = annotations(&doc, page as u32 + 1);
        let annot = annots[0];
        assert_eq!(annot.get(b"Subtype").unwrap().as_name().unwrap(), b"Stamp");
        assert_eq!(&numbers(annot.get(b"Rect").unwrap()), rect, "page {page}");
        let form = &appearance(&doc, annot).dict;
        assert_eq!(&numbers(form.get(b"Matrix").unwrap())[..4], matrix);
        assert_eq!(numbers(form.get(b"BBox").unwrap()), [0.0, 0.0, 50.0, 20.0]);
    }
}

#[test]
fn keeps_existing_annotations() {
    let mut doc = sample(&[(0, None), (0, None)]);
    let pages = doc.get_pages();
    let link = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => reals(&[0.0, 0.0, 10.0, 10.0]) });
    let shared = doc.add_object(vec![Object::Reference(link)]);
    doc.get_dictionary_mut(pages[&1])
        .unwrap()
        .set("Annots", vec![Object::Reference(link)]);
    doc.get_dictionary_mut(pages[&2])
        .unwrap()
        .set("Annots", Object::Reference(shared));
    let source = save(&mut doc);

    let edits = [
        signature(0, 10.0, 10.0, 50.0, 20.0),
        signature(1, 10.0, 10.0, 50.0, 20.0),
        signature(1, 10.0, 40.0, 50.0, 20.0),
    ];
    let doc = Document::load_mem(&apply(&source, &edits).unwrap()).unwrap();
    assert_eq!(annotations(&doc, 1).len(), 2);
    assert_eq!(annotations(&doc, 2).len(), 3);
}

#[test]
fn updates_files_with_cross_reference_streams() {
    let mut doc = sample(&[(0, None)]);
    let mut source = Vec::new();
    doc.save_modern(&mut source).unwrap();
    let output = apply(&source, &[signature(0, 10.0, 10.0, 100.0, 40.0)]).unwrap();
    assert!(output.starts_with(&source));
    let doc = Document::load_mem(&output).unwrap();
    assert_eq!(annotations(&doc, 1).len(), 1);
}

#[test]
fn chains_repeated_saves() {
    let source = save(&mut sample(&[(0, None)]));
    let first = apply(&source, &[text(0, "First")]).unwrap();
    let second = apply(
        &first,
        &[text_box(0, 72.0, 200.0, vec![vec![helvetica("Second")]])],
    )
    .unwrap();
    assert!(second.starts_with(&first));
    let texts: Vec<_> = lines_of(&second).into_iter().map(|l| l.text).collect();
    assert_eq!(texts, ["First", "Second"]);
}

#[test]
fn tells_which_edits_break_existing_signatures() {
    let unsigned = save(&mut sample(&[(0, None)]));
    let mut doc = sample(&[(0, None)]);
    let value = doc.add_object(dictionary! { "Type" => "Sig", "Filter" => "Adobe.PPKLite" });
    let signed_field =
        doc.add_object(dictionary! { "T" => Object::string_literal("signed"), "V" => value });
    let empty_field =
        doc.add_object(dictionary! { "T" => Object::string_literal("empty"), "FT" => "Sig" });
    let parent = doc.add_object(dictionary! { "FT" => "Sig", "Kids" => vec![signed_field.into()] });
    let form = doc.add_object(dictionary! {
        "Fields" => vec![empty_field.into(), parent.into()],
        "SigFlags" => 3,
    });
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.get_dictionary_mut(root).unwrap().set("AcroForm", form);
    let signed = save(&mut doc);

    assert!(is_signed(&Document::load_mem(&signed).unwrap()));
    assert!(!is_signed(&Document::load_mem(&unsigned).unwrap()));
    let stamp = [signature(0, 10.0, 10.0, 50.0, 20.0)];
    let content = [text(0, "Changed")];
    let picture = [Edit::Image(ImageEdit {
        page: 0,
        op: 0,
        original: bounds(0.0, 0.0, 1.0, 1.0),
        bounds: Some(bounds(5.0, 5.0, 1.0, 1.0)),
    })];
    assert!(breaks_signatures(&signed, &content).unwrap());
    assert!(breaks_signatures(&signed, &picture).unwrap());
    assert!(!breaks_signatures(&signed, &stamp).unwrap());
    assert!(!breaks_signatures(&signed, &[]).unwrap());
    assert!(!breaks_signatures(&unsigned, &content).unwrap());
}

#[test]
fn rejects_bad_edits() {
    let source = save(&mut sample(&[(0, None)]));
    assert!(matches!(
        apply(&source, &[text(3, "x")]),
        Err(EditError::NoPage(4))
    ));
    match apply(&source, &[text(0, "Hi \u{4e2d}\u{4e2d}")]) {
        Err(EditError::UnsupportedLineText(chars)) => assert_eq!(chars, "\u{4e2d}"),
        other => panic!("{other:?}"),
    }
    for bad in [
        text_box(0, f64::NAN, 100.0, vec![vec![helvetica("x")]]),
        text_box(0, 72.0, 100.0, vec![vec![helvetica("x\ny")]]),
    ] {
        assert!(matches!(apply(&source, &[bad]), Err(EditError::Invalid(_))));
    }
    assert!(matches!(
        apply(&source, &[signature(0, 0.0, 0.0, 0.0, 10.0)]),
        Err(EditError::Invalid(_))
    ));
}

#[test]
fn skips_empty_text() {
    let source = save(&mut sample(&[(0, None)]));
    let contents = |source: &[u8]| {
        let doc = Document::load_mem(source).unwrap();
        doc.get_page_contents(doc.get_pages()[&1])
    };
    let output = apply(&source, &[text(0, " \n "), text(0, "")]).unwrap();
    assert_eq!(contents(&output), contents(&source));
    assert!(annotations(&Document::load_mem(&output).unwrap(), 1).is_empty());
}

#[test]
fn deserializes_frontend_json() {
    let json = r#"[
        {"kind":"text","page":0,"x":1,"y":2,"lines":[[{"text":"Hi","font":"Helvetica",
         "family":"sans","bold":false,"italic":false,"size":12,"color":[0,0,0]}],[]]},
        {"kind":"signature","page":0,"x":1,"y":2,"width":3,"height":4,"color":[0,0,0],
         "path":{"d":"M 0 0 L 1 1","width":1,"height":1,"fill":false,"strokeWidth":0.1}}
    ]"#;
    let edits: Vec<Edit> = serde_json::from_str(json).unwrap();
    assert_eq!(edits.len(), 2);
    let Edit::Text(text) = &edits[0] else {
        panic!("{edits:?}")
    };
    assert_eq!(text.lines.len(), 2);
}

#[test]
fn formats_pdf_dates() {
    assert_eq!(pdf_date(0), "D:19700101000000Z");
    assert_eq!(pdf_date(1_700_000_000), "D:20231114221320Z");
    assert_eq!(pdf_date(951_782_400), "D:20000229000000Z");
}

fn simple_font(base: &str, embedded: bool) -> impl FnOnce(&mut Document) -> Object {
    let base = base.to_owned();
    move |doc| {
        let mut font = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => base.as_str(),
            "Encoding" => "WinAnsiEncoding",
            "FirstChar" => 32,
            "LastChar" => 255,
            "Widths" => (32..=255u8)
                .map(|c| Object::Integer(Standard::HELVETICA.code_width(c) as i64))
                .collect::<Vec<_>>(),
        };
        if embedded {
            let file = doc.add_object(Stream::new(dictionary! {}, vec![0]));
            let descriptor = doc.add_object(dictionary! {
                "Type" => "FontDescriptor",
                "FontName" => base.as_str(),
                "Flags" => 32,
                "Ascent" => 718,
                "Descent" => -207,
                "FontFile" => file,
            });
            font.set("FontDescriptor", descriptor);
        }
        doc.add_object(font).into()
    }
}

const IDENTITY_CMAP: &[u8] = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
    /CMapName /Adobe-Identity-UCS def /CMapType 2 def\n\
    1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
    3 beginbfchar <0001> <0048> <0002> <0069> <0003> <0021> endbfchar\n\
    endcmap CMapName currentdict /CMap defineresource pop end end";

/// A Type0 font whose codes 1, 2, and 3 show "H", "i", and "!".
fn identity_font(doc: &mut Document) -> Object {
    let widths = vec![1.into(), vec![722.into(), 222.into(), 278.into()].into()];
    cid_font(doc, widths, IDENTITY_CMAP)
}

fn cid_font(doc: &mut Document, widths: Vec<Object>, cmap: &[u8]) -> Object {
    let to_unicode = doc.add_object(Stream::new(dictionary! {}, cmap.to_vec()));
    let file = doc.add_object(Stream::new(dictionary! {}, vec![0]));
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "ABCDEF+Arial",
        "Flags" => 32,
        "FontFile2" => file,
    });
    let cid = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => "ABCDEF+Arial",
        "DW" => 1000,
        "W" => widths,
        "FontDescriptor" => descriptor,
    });
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "ABCDEF+Arial",
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![cid.into()],
        "ToUnicode" => to_unicode,
    })
    .into()
}

type FontMaker = Box<dyn FnOnce(&mut Document) -> Object>;

/// A one-page PDF that draws `content` with fonts `/F1`, `/F2`, and so on.
fn fonts_page(rotate: i64, content: &str, fonts: Vec<FontMaker>) -> Vec<u8> {
    let mut doc = sample(&[(rotate, None)]);
    let page_id = doc.get_pages()[&1];
    let mut dict = Dictionary::new();
    for (i, font) in fonts.into_iter().enumerate() {
        let font = font(&mut doc);
        dict.set(format!("F{}", i + 1), font);
    }
    let content = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
    let page = doc.get_dictionary_mut(page_id).unwrap();
    page.set("Contents", content);
    page.set("Resources", dictionary! { "Font" => dict });
    save(&mut doc)
}

fn text_page(
    rotate: i64,
    content: &str,
    font: impl FnOnce(&mut Document) -> Object + 'static,
) -> Vec<u8> {
    fonts_page(rotate, content, vec![Box::new(font)])
}

fn plain(content: &str) -> Vec<u8> {
    text_page(0, content, simple_font("Helvetica", false))
}

fn standard_font(family: Family, bold: bool, italic: bool) -> FontMaker {
    let face = Standard {
        family,
        bold,
        italic,
    };
    Box::new(move |doc| doc.add_object(face.dictionary()).into())
}

/// Helvetica as `/F1`, Helvetica-Bold as `/F2`, and Times-Roman as `/F3`.
fn styled(content: &str) -> Vec<u8> {
    fonts_page(
        0,
        content,
        vec![
            standard_font(Family::Sans, false, false),
            standard_font(Family::Sans, true, false),
            standard_font(Family::Serif, false, false),
        ],
    )
}

fn lines_of(source: &[u8]) -> Vec<TextLine> {
    text_lines(&Document::load_mem(source).unwrap(), 0).unwrap()
}

fn spans_edit(line: &TextLine, spans: Vec<Span>) -> Edit {
    Edit::Line(LineEdit {
        page: 0,
        runs: line.runs.clone(),
        original: line.text.clone(),
        spans,
    })
}

/// Replaces the line's text, in the style of its first span.
fn line_edit(line: &TextLine, text: &str) -> Edit {
    let span = Span {
        text: text.into(),
        ..line.spans[0].clone()
    };
    spans_edit(line, vec![span])
}

/// `text` in the style of `span`, changed by `change`.
fn restyle(span: &Span, text: &str, change: impl FnOnce(&mut Span)) -> Span {
    let mut span = Span {
        text: text.into(),
        ..span.clone()
    };
    change(&mut span);
    span
}

fn content_of(source: &[u8]) -> String {
    let doc = Document::load_mem(source).unwrap();
    String::from_utf8_lossy(&doc.get_page_content(doc.get_pages()[&1])).into_owned()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn reads_lines_of_text() {
    let lines = lines_of(&plain(
        "BT /F1 12 Tf 72 700 Td (Hello) Tj 0 -20 Td 0 0 1 rg (Next) Tj ET",
    ));
    assert_eq!(lines.len(), 2);
    let hello = &lines[0];
    assert_eq!(
        (hello.text.as_str(), hello.runs.as_slice()),
        ("Hello", [3].as_slice())
    );
    assert!(near(hello.x, 72.0) && near(hello.baseline, 92.0));
    assert!(near(hello.width, 27.336) && near(hello.y, 83.0) && near(hello.height, 12.0));
    let span = &hello.spans[0];
    assert_eq!(
        (
            span.text.as_str(),
            span.font.as_str(),
            span.family,
            span.size
        ),
        ("Hello", "Helvetica", Family::Sans, 12.0)
    );
    assert!(!span.bold && !span.italic && !span.underline && span.face.is_some());
    assert_eq!(lines[1].text, "Next");
    assert!(near(lines[1].baseline, 112.0));
    assert_eq!(lines[1].spans[0].color, [0, 0, 255]);
}

#[test]
fn joins_runs_and_reads_kerned_spaces() {
    let lines = lines_of(&plain(
        "BT /F1 12 Tf 72 700 Td [(Hello) -250 (world)] TJ ( again) Tj [-3000 (X)] TJ ET",
    ));
    let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["Hello world again", "X"]);
    assert_eq!(lines[0].runs, [3, 4]);
    assert_eq!(lines[0].spans.len(), 1);
}

#[test]
fn reads_a_line_in_several_styles_as_spans() {
    let source = styled(
        "BT /F1 12 Tf 72 700 Td (Plain ) Tj /F2 12 Tf (bold) Tj /F3 12 Tf [-250 (serif)] TJ \
         1 0 0 rg /F1 12 Tf (red) Tj 2 Tr 0 g (heavy) Tj ET \
         BT /F1 12 Tf 1 0 0.3 1 72 650 Tm (Lean) Tj ET",
    );
    let lines = lines_of(&source);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].text, "Plain bold serifredheavy");
    let styles: Vec<_> = lines[0]
        .spans
        .iter()
        .map(|s| (s.text.as_str(), s.font.as_str(), s.bold, s.color))
        .collect();
    assert_eq!(
        styles,
        [
            ("Plain ", "Helvetica", false, [0, 0, 0]),
            ("bold ", "Helvetica", true, [0, 0, 0]),
            ("serif", "Times", false, [0, 0, 0]),
            ("red", "Helvetica", false, [255, 0, 0]),
            ("heavy", "Helvetica", true, [0, 0, 0]),
        ]
    );
    assert_eq!(lines[0].spans[2].family, Family::Serif);
    assert!(lines[1].spans[0].italic);
}

#[test]
fn lists_the_document_fonts() {
    let doc = Document::load_mem(&styled("BT /F1 12 Tf (A) Tj ET")).unwrap();
    let fonts: Vec<_> = document_fonts(&doc)
        .into_iter()
        .map(|f| (f.name, f.family))
        .collect();
    assert_eq!(
        fonts,
        [
            ("Helvetica".to_owned(), Family::Sans),
            ("Times".to_owned(), Family::Serif)
        ]
    );
}

#[test]
fn replaces_a_line_and_keeps_text_after_it_in_place() {
    let source =
        plain("BT /F1 12 Tf 72 700 Td [(Hello) -250 (world)] TJ ( again) Tj [-3000 (X)] TJ ET");
    let before = lines_of(&source);
    let output = apply(&source, &[line_edit(&before[0], "Hi there")]).unwrap();
    assert!(output.starts_with(&source));
    let after = lines_of(&output);
    let texts: Vec<_> = after.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["Hi there", "X"]);
    assert!(near(after[0].x, 72.0) && near(after[0].baseline, 92.0));
    assert!(near(after[1].x, before[1].x));
}

#[test]
fn reports_lines_that_grow() {
    let source = plain("BT /F1 12 Tf 72 700 Td (Price) Tj [-2000 (42)] TJ ET");
    let doc = Document::load_mem(&source).unwrap();
    let before = text_lines(&doc, 0).unwrap();
    assert_eq!(before.len(), 2);
    let applied = apply_document(&source, doc, &[line_edit(&before[0], "Cost")]).unwrap();
    assert!(applied.lines[0].overflow < 0.0);
    let after = lines_of(&applied.bytes);
    assert_eq!(after[0].text, "Cost");
    assert!(near(after[1].x, before[1].x));

    let doc = Document::load_mem(&source).unwrap();
    let applied = apply_document(&source, doc, &[line_edit(&before[0], "Prices")]).unwrap();
    assert!(near(applied.lines[0].overflow, 6.0));
    assert!(applied.lines[0].fallback.is_empty());
}

#[test]
fn styles_words_with_the_standard_faces_of_a_standard_font() {
    let source = plain(
        "BT /F1 12 Tf 72 700 Td (Hello brave world) Tj ET BT /F1 12 Tf 72 680 Td (Next) Tj ET",
    );
    let before = lines_of(&source);
    let base = &before[0].spans[0];
    let spans = vec![
        restyle(base, "Hello ", |_| {}),
        restyle(base, "brave", |s| s.bold = true),
        restyle(base, " new ", |s| s.italic = true),
        restyle(base, "world", |s| s.underline = true),
    ];
    let doc = Document::load_mem(&source).unwrap();
    let applied = apply_document(&source, doc, &[spans_edit(&before[0], spans)]).unwrap();
    assert!(applied.lines[0].fallback.is_empty());
    let after = lines_of(&applied.bytes);
    assert_eq!(after[0].text, "Hello brave new world");
    assert!(near(after[0].x, 72.0) && near(after[0].baseline, 92.0));
    assert!(near(after[1].x, before[1].x) && near(after[1].baseline, before[1].baseline));
    let styles: Vec<_> = after[0]
        .spans
        .iter()
        .map(|s| (s.text.as_str(), s.font.as_str(), s.bold, s.italic))
        .collect();
    assert_eq!(
        styles,
        [
            ("Hello ", "Helvetica", false, false),
            ("brave", "Helvetica", true, false),
            (" new ", "Helvetica", false, true),
            ("world", "Helvetica", false, false),
        ]
    );

    let doc = Document::load_mem(&applied.bytes).unwrap();
    let fonts = doc.get_page_fonts(doc.get_pages()[&1]).unwrap();
    let mut bases: Vec<_> = fonts
        .values()
        .map(|f| {
            String::from_utf8_lossy(f.get(b"BaseFont").unwrap().as_name().unwrap()).into_owned()
        })
        .collect();
    bases.sort();
    assert_eq!(bases, ["Helvetica", "Helvetica-Bold", "Helvetica-Oblique"]);
    let content = content_of(&applied.bytes);
    assert!(content.contains(" re\nf\n"), "{content}");
    assert!(!content.contains(" Tr"), "{content}");
}

#[test]
fn changes_the_font_size_and_color_of_words() {
    let source =
        styled("BT /F1 12 Tf 72 700 Td (Hello world) Tj ET BT /F1 12 Tf 72 680 Td (Next) Tj ET");
    let before = lines_of(&source);
    let base = &before[0].spans[0];
    let spans = vec![
        restyle(base, "Hello ", |_| {}),
        restyle(base, "world", |s| {
            s.font = "Times".into();
            s.family = Family::Serif;
            s.face = None;
            s.size = 18.0;
            s.color = [255, 0, 0];
        }),
    ];
    let output = apply(&source, &[spans_edit(&before[0], spans)]).unwrap();
    let after = lines_of(&output);
    let world = &after[0].spans[1];
    assert_eq!(
        (
            world.text.as_str(),
            world.font.as_str(),
            world.size,
            world.color
        ),
        ("world", "Times", 18.0, [255, 0, 0])
    );
    assert!(near(after[0].baseline, 92.0));
    assert_eq!(after[1].spans[0].color, [0, 0, 0]);
    assert!(near(after[1].x, before[1].x) && near(after[1].baseline, before[1].baseline));
    let content = content_of(&output);
    assert!(content.contains("/F3 18.0 Tf\n1.0 0.0 0.0 rg"), "{content}");
}

#[test]
fn synthesizes_styles_a_font_lacks() {
    let source = text_page(
        0,
        "BT /F1 12 Tf 72 700 Td <000100020003> Tj ET",
        identity_font,
    );
    let before = lines_of(&source);
    let base = &before[0].spans[0];
    assert_eq!(base.font, "Arial");
    let spans = vec![
        restyle(base, "H", |s| s.bold = true),
        restyle(base, "i!", |s| s.italic = true),
    ];
    let output = apply(&source, &[spans_edit(&before[0], spans)]).unwrap();
    let content = content_of(&output);
    assert!(
        content.contains("2 Tr") && content.contains(" w\n"),
        "{content}"
    );
    let after = lines_of(&output);
    let styles: Vec<_> = after[0]
        .spans
        .iter()
        .map(|s| (s.text.as_str(), s.bold, s.italic))
        .collect();
    assert_eq!(styles, [("H", true, false), ("i!", false, true)]);
    assert!(near(after[0].x, 72.0) && near(after[0].baseline, 92.0));

    let plain = restyle(&after[0].spans[0], "Hi!", |s| s.bold = false);
    let output = apply(&output, &[spans_edit(&after[0], vec![plain])]).unwrap();
    let again = lines_of(&output);
    assert_eq!(again[0].text, "Hi!");
    assert!(!again[0].spans[0].bold && !again[0].spans[0].italic);
}

#[test]
fn draws_glyphs_a_subset_font_lacks_in_a_standard_font() {
    let source = text_page(
        0,
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        simple_font("ABCDEF+Arial", true),
    );
    let doc = Document::load_mem(&source).unwrap();
    let line = &text_lines(&doc, 0).unwrap()[0];
    assert_eq!(line.spans[0].font, "Arial");
    let applied = apply_document(&source, doc, &[line_edit(line, "Hello Joe")]).unwrap();
    assert_eq!(applied.lines[0].fallback, "J");
    let doc = Document::load_mem(&applied.bytes).unwrap();
    let fonts = doc.get_page_fonts(doc.get_pages()[&1]).unwrap();
    assert_eq!(
        fonts[b"UkF1".as_slice()]
            .get(b"BaseFont")
            .unwrap()
            .as_name()
            .unwrap(),
        b"Helvetica"
    );
    let lines = text_lines(&doc, 0).unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "Hello Joe");
    let fonts: Vec<_> = lines[0].spans.iter().map(|s| s.font.as_str()).collect();
    assert_eq!(fonts, ["Arial", "Helvetica", "Arial"]);

    let applied = apply_document(&applied.bytes, doc, &[line_edit(&lines[0], "Jello")]).unwrap();
    assert_eq!(applied.lines[0].fallback, "J");
    assert_eq!(lines_of(&applied.bytes)[0].text, "Jello");
}

#[test]
fn uses_the_standard_font_of_a_standard_name_for_missing_glyphs() {
    let source = text_page(
        0,
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        simple_font("ABCDEF+Helvetica", true),
    );
    let line = &lines_of(&source)[0];
    let doc = Document::load_mem(&source).unwrap();
    let applied = apply_document(&source, doc, &[line_edit(line, "Hello Joe")]).unwrap();
    assert!(applied.lines[0].fallback.is_empty());
    let after = lines_of(&applied.bytes);
    assert_eq!(after[0].text, "Hello Joe");
    assert_eq!(after[0].spans.len(), 1);
}

#[test]
fn edits_identity_encoded_fonts() {
    let source = text_page(
        0,
        "BT /F1 12 Tf 72 700 Td <000100020003> Tj ET",
        identity_font,
    );
    let line = &lines_of(&source)[0];
    assert_eq!(line.text, "Hi!");
    let output = apply(&source, &[line_edit(line, "!iH")]).unwrap();
    let content = content_of(&output);
    assert!(content.contains("<000300020001>"), "{content}");
    assert_eq!(lines_of(&output)[0].text, "!iH");
}

#[test]
fn rejects_stale_and_unsupported_line_edits() {
    let source = plain("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let line = &lines_of(&source)[0];
    let mut stale = line.clone();
    stale.text = "Goodbye".into();
    assert!(matches!(
        apply(&source, &[line_edit(&stale, "Hi")]),
        Err(EditError::StaleText(1))
    ));
    stale.text = line.text.clone();
    stale.runs = vec![2];
    assert!(matches!(
        apply(&source, &[line_edit(&stale, "Hi")]),
        Err(EditError::StaleText(1))
    ));
    match apply(&source, &[line_edit(line, "Hi \u{4e2d}")]) {
        Err(EditError::UnsupportedLineText(chars)) => assert_eq!(chars, "\u{4e2d}"),
        other => panic!("{other:?}"),
    }
    let unchanged = apply(&source, &[line_edit(line, "Hello")]).unwrap();
    assert_eq!(lines_of(&unchanged)[0].runs, line.runs);
}

#[test]
fn reads_text_on_rotated_pages() {
    let content = "BT /F1 12 Tf 0 1 -1 0 300 100 Tm (Up) Tj ET";
    let rotated = text_page(90, content, simple_font("Helvetica", false));
    let lines = lines_of(&rotated);
    assert_eq!(lines.len(), 1);
    assert!(near(lines[0].x, 100.0) && near(lines[0].baseline, 300.0));
    let output = apply(&rotated, &[line_edit(&lines[0], "Down")]).unwrap();
    let after = lines_of(&output);
    assert_eq!(after[0].text, "Down");
    assert!(near(after[0].x, 100.0) && near(after[0].baseline, 300.0));

    assert!(lines_of(&plain(content)).is_empty());
}

#[test]
fn skips_invisible_text() {
    assert!(lines_of(&plain("BT /F1 12 Tf 3 Tr 72 700 Td (Hidden) Tj ET")).is_empty());
}

#[test]
fn keeps_line_moves_of_quote_operators() {
    let source = plain("BT /F1 12 Tf 14 TL 72 700 Td (A) Tj (Second) ' ET");
    let lines = lines_of(&source);
    assert_eq!(lines[1].runs, [5]);
    assert!(near(lines[1].baseline, 106.0));
    let output = apply(&source, &[line_edit(&lines[1], "2nd")]).unwrap();
    let after = lines_of(&output);
    assert_eq!(after[1].text, "2nd");
    assert!(near(after[1].baseline, 106.0) && near(after[1].x, 72.0));
}

#[test]
fn combines_line_edits_with_added_text_and_annotations() {
    let source = plain("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let line = &lines_of(&source)[0];
    let edits = [
        line_edit(line, "Hi"),
        text(0, "Note"),
        signature(0, 10.0, 10.0, 100.0, 40.0),
    ];
    let output = apply(&source, &edits).unwrap();
    let doc = Document::load_mem(&output).unwrap();
    assert_eq!(annotations(&doc, 1).len(), 1);
    assert_eq!(doc.get_page_contents(doc.get_pages()[&1]).len(), 1);
    let texts: Vec<_> = lines_of(&output).into_iter().map(|l| l.text).collect();
    assert_eq!(texts, ["Hi", "Note"]);
}

#[test]
fn refuses_line_edits_when_the_owner_restricts_changes() {
    let source = encrypted(Permissions::ANNOTABLE, false);
    let edit = Edit::Line(LineEdit {
        page: 0,
        runs: vec![0],
        original: "a".into(),
        spans: Vec::new(),
    });
    assert!(matches!(
        apply(&source, &[edit]),
        Err(EditError::RestrictedContent)
    ));
}

#[test]
fn deserializes_line_edits() {
    let json = r#"{"kind":"line","page":0,"runs":[3,4],"original":"Hello","spans":[
        {"text":"Hi","font":"Helvetica","family":"sans","bold":true,"italic":false,
         "size":12,"color":[0,0,0]}]}"#;
    let edit: Edit = serde_json::from_str(json).unwrap();
    let Edit::Line(line) = edit else {
        panic!("{edit:?}")
    };
    assert_eq!(line.runs, [3, 4]);
    let span = &line.spans[0];
    assert!(span.bold && !span.underline && span.face.is_none());
}

#[test]
fn refuses_pages_whose_content_a_rewrite_would_lose() {
    for content in [
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET BI /W 1 /H 1 /BPC 8 /CS /G /F /AHx ID 41> EI",
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET } 0 0 m",
    ] {
        let doc = Document::load_mem(&plain(content)).unwrap();
        assert!(
            matches!(text_lines(&doc, 0), Err(EditError::UnsafeContent)),
            "{content}"
        );
    }

    let source = plain(
        "q 9 0 0 9 300 300 cm BI /W 1 /H 1 /BPC 8 /CS /G ID A EI Q \
         BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
    );
    let line = &lines_of(&source)[0];
    let output = apply(&source, &[line_edit(line, "Hi")]).unwrap();
    assert!(content_of(&output).contains("ID A\nEI"));
    assert_eq!(lines_of(&output)[0].text, "Hi");
}

#[test]
fn gives_a_page_its_own_resources_before_adding_a_font() {
    let mut doc = sample(&[(0, None), (0, None)]);
    let pages = doc.get_pages();
    let helvetica = Standard {
        family: Family::Sans,
        bold: false,
        italic: false,
    };
    let font = doc.add_object(helvetica.dictionary());
    let fonts = doc.add_object(dictionary! { "F1" => font });
    let resources = doc.add_object(dictionary! { "Font" => fonts });
    for page in [1, 2] {
        let content = b"BT /F1 12 Tf 72 700 Td (Hello world) Tj ET".to_vec();
        let content = doc.add_object(Stream::new(dictionary! {}, content));
        let dict = doc.get_dictionary_mut(pages[&page]).unwrap();
        dict.set("Contents", content);
        dict.set("Resources", resources);
    }
    let source = save(&mut doc);

    let line = &lines_of(&source)[0];
    let base = &line.spans[0];
    let spans = vec![
        restyle(base, "Hello ", |_| {}),
        restyle(base, "world", |s| {
            s.font = "Times".into();
            s.family = Family::Serif;
            s.face = None;
        }),
    ];
    let output = apply(&source, &[spans_edit(line, spans)]).unwrap();
    let doc = Document::load_mem(&output).unwrap();
    let pages = doc.get_pages();
    let names = |page: u32| -> Vec<Vec<u8>> {
        doc.get_page_fonts(pages[&page])
            .unwrap()
            .into_keys()
            .collect()
    };
    assert_eq!(names(1), [b"F1".to_vec(), b"UkF1".to_vec()]);
    assert_eq!(names(2), [b"F1".to_vec()]);
    assert_eq!(doc.get_dictionary(fonts).unwrap().len(), 1);
    assert_eq!(
        doc.get_dictionary(resources).unwrap().get(b"Font").unwrap(),
        &Object::Reference(fonts)
    );
    assert_eq!(lines_of(&output)[0].text, "Hello world");
    assert_eq!(text_lines(&doc, 1).unwrap()[0].text, "Hello world");
}

#[test]
fn never_changes_an_annotation_array_pages_share() {
    let mut doc = sample(&[(0, None), (0, None)]);
    let pages = doc.get_pages();
    let note = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Text",
        "Rect" => reals(&[0.0, 0.0, 10.0, 10.0]),
    });
    let shared = doc.add_object(vec![Object::Reference(note)]);
    for page in [1, 2] {
        doc.get_dictionary_mut(pages[&page])
            .unwrap()
            .set("Annots", shared);
    }
    let source = save(&mut doc);

    let edit = signature(0, 10.0, 10.0, 50.0, 20.0);
    let doc = Document::load_mem(&apply(&source, &[edit]).unwrap()).unwrap();
    assert_eq!(annotations(&doc, 1).len(), 2);
    assert_eq!(annotations(&doc, 2).len(), 1);
    assert_eq!(doc.get_object(shared).unwrap().as_array().unwrap().len(), 1);
}

#[test]
fn ends_lines_where_paint_or_text_state_would_change_them() {
    let texts = |content: &str| -> Vec<String> {
        lines_of(&plain(content))
            .into_iter()
            .map(|l| l.text)
            .collect()
    };
    let start = "BT /F1 12 Tf 72 700 Td (Left) Tj";
    assert_eq!(
        texts(&format!("{start} ET BT /F1 12 Tf 100 700 Td (Right) Tj ET")).len(),
        1
    );
    assert_eq!(texts(&format!("{start} 0.1 Tc (Right) Tj ET")).len(), 1);
    assert_eq!(texts(&format!("{start} 2 Tr (Right) Tj ET")).len(), 1);
    for change in [
        "ET 100 695 10 10 re f BT /F1 12 Tf 100 700 Td",
        "2 Tc",
        "3 Tw",
        "90 Tz",
        "1 Tr",
        "0.5 Ts",
    ] {
        assert_eq!(
            texts(&format!("{start} {change} (Right) Tj ET")),
            ["Left", "Right"],
            "{change}"
        );
    }
}

#[test]
fn skips_text_drawn_where_the_pen_is_unknown() {
    let lines = lines_of(&plain(
        "BT /F9 12 Tf 72 700 Td (Ghost) Tj /F1 12 Tf (Lost) Tj 0 -20 Td (Found) Tj ET",
    ));
    let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["Found"]);
    assert!(near(lines[0].x, 72.0) && near(lines[0].baseline, 112.0));

    let mut doc = Document::load_mem(&plain(
        "BT /F1 12 Tf 72 700 Td /GS1 gs (Hidden) Tj ET \
         BT /F1 12 Tf 72 680 Td /GS2 gs (Shown) Tj ET",
    ))
    .unwrap();
    let page_id = doc.get_pages()[&1];
    let resources = doc
        .get_dictionary_mut(page_id)
        .unwrap()
        .get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap();
    let font = resources
        .get(b"Font")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"F1");
    let font = font.unwrap().clone();
    resources.set(
        "ExtGState",
        dictionary! {
            "GS1" => dictionary! { "Font" => vec![font, 12.into()] },
            "GS2" => dictionary! { "CA" => 1 },
        },
    );
    let texts: Vec<_> = lines_of(&save(&mut doc))
        .into_iter()
        .map(|l| l.text)
        .collect();
    assert_eq!(texts, ["Shown"]);
}

#[test]
fn skips_fonts_with_unreasonable_tables() {
    let huge = b"1 begincodespacerange <00000000> <FFFFFFFF> endcodespacerange\n\
        1 beginbfrange <00000000> <FFFFFFFF> <0000> endbfrange";
    let content = "BT /F1 12 Tf 72 700 Td <000100020003> Tj ET";
    let widths = vec![1.into(), vec![722.into(), 222.into(), 278.into()].into()];
    let source = text_page(0, content, move |doc| cid_font(doc, widths, huge));
    assert!(lines_of(&source).is_empty());

    let widths = vec![
        Object::Integer(u32::MAX.into()),
        vec![500.into(), 500.into()].into(),
    ];
    let source = text_page(0, content, move |doc| cid_font(doc, widths, IDENTITY_CMAP));
    assert!(lines_of(&source).is_empty());
}

#[test]
fn rejects_text_sizes_out_of_range() {
    let source = plain("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let line = &lines_of(&source)[0];
    for size in [0.0, -1.0, 1e300, f64::NAN, f64::INFINITY] {
        let span = restyle(&line.spans[0], "Hi", |s| s.size = size);
        assert!(
            matches!(
                apply(&source, &[spans_edit(line, vec![span])]),
                Err(EditError::Invalid(_))
            ),
            "{size}"
        );
    }
}

#[test]
fn needs_only_the_permissions_the_edits_use() {
    let source = encrypted(Permissions::MODIFIABLE, false);
    let edit = Edit::Line(LineEdit {
        page: 0,
        runs: vec![0],
        original: "a".into(),
        spans: Vec::new(),
    });
    assert!(matches!(
        apply(&source, &[edit]),
        Err(EditError::StaleText(_))
    ));
    assert_eq!(
        lines_of(&apply(&source, &[text(0, "Hello")]).unwrap())[0].text,
        "Hello"
    );
    assert!(matches!(
        apply(&source, &[signature(0, 10.0, 10.0, 50.0, 20.0)]),
        Err(EditError::Restricted)
    ));
    let source = encrypted(Permissions::ANNOTABLE, false);
    assert!(matches!(
        apply(&source, &[text(0, "Hello")]),
        Err(EditError::RestrictedContent)
    ));
}

/// The operators of each content stream of the first page.
fn streams_of(source: &[u8]) -> Vec<Vec<String>> {
    let doc = Document::load_mem(source).unwrap();
    doc.get_page_contents(doc.get_pages()[&1])
        .into_iter()
        .map(|id| {
            let stream = doc.get_object(id).unwrap().as_stream().unwrap();
            let content = lopdf::content::Content::decode(&stream.get_plain_content().unwrap());
            let operations = content.unwrap().operations.into_iter();
            operations.map(|o| o.operator).collect()
        })
        .collect()
}

#[test]
fn draws_added_text_after_the_original_content() {
    let source = save(&mut sample(&[(0, None)]));
    let original = Document::load_mem(&source).unwrap();
    let original = original.get_page_contents(original.get_pages()[&1]);
    let output = apply(&source, &[text(0, "Hello")]).unwrap();
    let doc = Document::load_mem(&output).unwrap();
    let contents = doc.get_page_contents(doc.get_pages()[&1]);
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[1], original[0]);
    let streams = streams_of(&output);
    assert_eq!(streams[0], ["q"]);
    assert_eq!(streams[2][..2], ["Q", "q"]);

    let lines = lines_of(&output);
    assert_eq!(lines[0].text, "Hello");
    assert!(near(lines[0].x, 72.0) && near(lines[0].baseline, 110.266));
}

#[test]
fn spaces_rows_of_added_text_like_the_editor() {
    let source = save(&mut sample(&[(0, None)]));
    let lines = lines_of(&apply(&source, &[text(0, "One\n\nThree")]).unwrap());
    let rows: Vec<_> = lines
        .iter()
        .map(|l| (l.text.as_str(), l.baseline))
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(near(rows[0].1, 110.266) && near(rows[1].1, 110.266 + 28.8));

    let big = Span {
        size: 24.0,
        ..helvetica("Big")
    };
    let edit = text_box(
        0,
        72.0,
        100.0,
        vec![vec![big, helvetica(" small")], vec![helvetica("Next")]],
    );
    let lines = lines_of(&apply(&source, &[edit]).unwrap());
    assert_eq!(lines[0].text, "Big small");
    assert!(near(lines[0].baseline, 100.0 + 24.0 * 0.8555));
    assert!(near(
        lines[1].baseline - lines[0].baseline,
        24.0 * 0.3445 + 10.266
    ));

    let blank = Span {
        size: 24.0,
        ..helvetica("")
    };
    let edit = text_box(
        0,
        72.0,
        100.0,
        vec![
            vec![helvetica("One")],
            vec![blank],
            vec![helvetica("Three")],
        ],
    );
    let lines = lines_of(&apply(&source, &[edit]).unwrap());
    assert!(near(lines[1].baseline - lines[0].baseline, 14.4 + 28.8));
}

#[test]
fn closes_what_the_original_content_leaves_open() {
    let source = plain("q 2 0 0 2 0 0 cm BT /F1 12 Tf 72 300 Td (Hello) Tj");
    let output = apply(&source, &[text(0, "Added")]).unwrap();
    assert_eq!(streams_of(&output)[2][..4], ["ET", "Q", "Q", "q"]);
    let lines = lines_of(&output);
    let added = lines.iter().find(|l| l.text == "Added").unwrap();
    // The page's Helvetica has no descriptor, so rows use the default ascent and descent.
    assert!(near(added.x, 72.0) && near(added.baseline, 100.0 + 12.0 * 1.7 / 2.0));

    let output = apply(&plain("0 0 m 10 10 l"), &[text(0, "Added")]).unwrap();
    assert_eq!(streams_of(&output)[2][..3], ["n", "Q", "q"]);

    let source = plain("Q BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    assert!(matches!(
        apply(&source, &[text(0, "Added")]),
        Err(EditError::UnsafeContent)
    ));
}

#[test]
fn draws_styles_in_added_text() {
    let source = text_page(
        0,
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        simple_font("Calibri", false),
    );
    let base = Span {
        font: "Calibri".into(),
        ..helvetica("")
    };
    let row = vec![
        restyle(&base, "B", |s| s.bold = true),
        restyle(&base, "I", |s| s.italic = true),
        restyle(&base, "U", |s| s.underline = true),
    ];
    let output = apply(&source, &[text_box(0, 72.0, 100.0, vec![row])]).unwrap();
    let content = content_of(&output);
    for needle in ["2 Tr", " w\n", " RG\n", "re\nf"] {
        assert!(content.contains(needle), "{needle} in {content}");
    }
    let lines = lines_of(&output);
    let added = lines.iter().find(|l| l.text == "BIU").unwrap();
    let styles: Vec<_> = added
        .spans
        .iter()
        .map(|s| (s.text.as_str(), s.font.as_str(), s.bold, s.italic))
        .collect();
    assert_eq!(
        styles,
        [
            ("B", "Calibri", true, false),
            ("I", "Calibri", false, true),
            ("U", "Calibri", false, false)
        ]
    );
}

#[test]
fn draws_added_text_upright_on_rotated_pages() {
    for rotate in [90, 180, 270] {
        let source = save(&mut sample(&[(rotate, None)]));
        let lines = lines_of(&apply(&source, &[text(0, "Hello")]).unwrap());
        assert_eq!(lines.len(), 1, "{rotate}");
        assert!(
            near(lines[0].x, 72.0) && near(lines[0].baseline, 110.266),
            "{rotate}: {} {}",
            lines[0].x,
            lines[0].baseline
        );
    }
}

/// A one-page PDF that draws `content` with Helvetica as `/F1`, `images` 2 by 2 gray images as
/// `/Im1`, `/Im2`, and so on, each filled with its number, and a form as `/Fm1`.
fn picture_page(rotate: i64, content: &str, images: u8) -> Vec<u8> {
    let mut doc = sample(&[(rotate, None)]);
    let page_id = doc.get_pages()[&1];
    let mut xobjects = Dictionary::new();
    for i in 1..=images {
        xobjects.set(format!("Im{i}"), gray_image(&mut doc, i));
    }
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        },
        b"0 0 10 10 re f".to_vec(),
    ));
    xobjects.set("Fm1", form);
    let font = simple_font("Helvetica", false)(&mut doc);
    let content = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
    let page = doc.get_dictionary_mut(page_id).unwrap();
    page.set("Contents", content);
    page.set(
        "Resources",
        dictionary! { "Font" => dictionary! { "F1" => font }, "XObject" => xobjects },
    );
    save(&mut doc)
}

/// A 2 by 2 gray image filled with `fill`.
fn gray_image(doc: &mut Document, fill: u8) -> ObjectId {
    doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 2,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        vec![fill; 4],
    ))
}

/// A PDF whose pages draw `contents` from one set of resources with images `/Im1` and `/Im2`
/// and a form `/Fm1` without resources of its own that draws `form`. The pages share the
/// resources, or with `inherit` take them from the page tree.
fn shared_pictures(contents: &[&str], form: &str, inherit: bool) -> Vec<u8> {
    let mut doc = sample(&vec![(0, None); contents.len()]);
    let mut xobjects = Dictionary::new();
    for i in 1..=2 {
        xobjects.set(format!("Im{i}"), gray_image(&mut doc, i));
    }
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
        },
        form.as_bytes().to_vec(),
    ));
    xobjects.set("Fm1", form);
    let pages = doc.get_pages();
    for (&page_id, content) in pages.values().zip(contents) {
        let content = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        doc.get_dictionary_mut(page_id)
            .unwrap()
            .set("Contents", content);
    }
    if inherit {
        let xobjects = doc.add_object(xobjects);
        let root = doc
            .catalog()
            .unwrap()
            .get(b"Pages")
            .unwrap()
            .as_reference()
            .unwrap();
        doc.get_dictionary_mut(root)
            .unwrap()
            .set("Resources", dictionary! { "XObject" => xobjects });
    } else {
        let resources = doc.add_object(dictionary! { "XObject" => xobjects });
        for &page_id in pages.values() {
            doc.get_dictionary_mut(page_id)
                .unwrap()
                .set("Resources", resources);
        }
    }
    save(&mut doc)
}

fn images_of(source: &[u8]) -> Vec<PageImage> {
    page_images(&Document::load_mem(source).unwrap(), 0).unwrap()
}

fn bounds(x: f64, y: f64, width: f64, height: f64) -> Bounds {
    Bounds {
        x,
        y,
        width,
        height,
    }
}

fn near_bounds(a: Bounds, b: Bounds) -> bool {
    near(a.x, b.x) && near(a.y, b.y) && near(a.width, b.width) && near(a.height, b.height)
}

fn image_edit(image: &PageImage, to: Option<Bounds>) -> Edit {
    Edit::Image(ImageEdit {
        page: 0,
        op: image.op,
        original: image.bounds,
        bounds: to,
    })
}

/// The image XObjects in the saved file, by the number they are filled with.
fn image_objects(source: &[u8]) -> Vec<u8> {
    let doc = Document::load_mem(source).unwrap();
    let mut images: Vec<u8> = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image"))
        .map(|s| s.content[0])
        .collect();
    images.sort();
    images
}

#[test]
fn lists_pictures_but_not_backgrounds_or_forms() {
    let source = picture_page(
        0,
        "q 612 0 0 792 0 0 cm /Im2 Do Q /Fm1 Do q 100 0 0 50 72 642 cm /Im1 Do Q \
         q 20 0 0 20 300 300 cm BI /W 2 /H 2 /CS /G /BPC 8 ID abcd EI Q",
        2,
    );
    let images = images_of(&source);
    assert_eq!(images.len(), 2, "{images:?}");
    assert_eq!(images[0].op, 7);
    assert!(near_bounds(
        images[0].bounds,
        bounds(72.0, 100.0, 100.0, 50.0)
    ));
    assert!(near_bounds(
        images[0].limits,
        bounds(0.0, 0.0, 612.0, 792.0)
    ));
    assert!(near_bounds(
        images[1].bounds,
        bounds(300.0, 472.0, 20.0, 20.0)
    ));
}

#[test]
fn lists_moves_and_removes_figures() {
    let mut doc = Document::load_mem(&picture_page(0, "q 1 0 0 1 72 600 cm /Fg1 Do Q", 0)).unwrap();
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 2,
            "Height" => 2,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
        },
        vec![7; 4],
    ));
    let figure = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 20.into(), 10.into()],
            "Matrix" => vec![2.into(), 0.into(), 0.into(), 2.into(), 0.into(), 0.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im" => image } },
        },
        b"q 20 0 0 10 0 0 cm /Im Do Q".to_vec(),
    ));
    let page_id = doc.get_pages()[&1];
    let page = doc.get_dictionary_mut(page_id).unwrap();
    let resources = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    let xobjects = resources
        .get_mut(b"XObject")
        .unwrap()
        .as_dict_mut()
        .unwrap();
    xobjects.set("Fg1", figure);
    let source = save(&mut doc);

    let images = images_of(&source);
    assert_eq!(images.len(), 1, "{images:?}");
    assert_eq!(images[0].op, 2);
    assert!(near_bounds(
        images[0].bounds,
        bounds(72.0, 172.0, 40.0, 20.0)
    ));

    let to = bounds(100.0, 300.0, 80.0, 40.0);
    let moved = images_of(&apply(&source, &[image_edit(&images[0], Some(to))]).unwrap());
    assert!(near_bounds(moved[0].bounds, to), "{moved:?}");

    let removed = apply(&source, &[image_edit(&images[0], None)]).unwrap();
    assert!(images_of(&removed).is_empty());
    assert!(image_objects(&removed).is_empty());
    let figures = Document::load_mem(&removed)
        .unwrap()
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.has(b"Matrix"))
        .count();
    assert_eq!(figures, 0);
}

#[test]
fn moves_and_resizes_pictures() {
    for rotate in [0, 90, 180, 270] {
        let source = picture_page(
            rotate,
            "q 100 0 0 50 72 642 cm /Im1 Do Q 2 0 0 2 0 0 cm q 10 0 0 10 200 200 cm \
             BI /W 2 /H 2 /CS /G /BPC 8 ID abcd EI Q",
            1,
        );
        let images = images_of(&source);
        let moved = bounds(
            images[0].bounds.x + 30.0,
            images[0].bounds.y + 40.0,
            50.0,
            25.0,
        );
        let output = apply(
            &source,
            &[
                image_edit(&images[0], Some(moved)),
                image_edit(&images[1], Some(bounds(10.0, 20.0, 60.0, 60.0))),
            ],
        )
        .unwrap();
        let after = images_of(&output);
        assert_eq!(after.len(), 2, "{rotate}: {after:?}");
        assert!(near_bounds(after[0].bounds, moved), "{rotate}: {after:?}");
        assert!(
            near_bounds(after[1].bounds, bounds(10.0, 20.0, 60.0, 60.0)),
            "{rotate}: {after:?}"
        );
        assert_eq!(image_objects(&output), [1]);
    }
}

#[test]
fn moves_the_clipping_a_picture_group_sets_with_it() {
    let source = picture_page(
        0,
        "q 72 642 50 50 re W n q 100 0 0 50 72 642 cm /Im1 Do Q Q 0 0 m 10 10 l S",
        1,
    );
    let image = &images_of(&source)[0];
    assert_eq!(image.op, 6);
    assert!(near_bounds(image.bounds, bounds(72.0, 100.0, 50.0, 50.0)));
    let moved = bounds(272.0, 300.0, 50.0, 50.0);
    let output = apply(&source, &[image_edit(image, Some(moved))]).unwrap();
    assert!(near_bounds(images_of(&output)[0].bounds, moved));
    let content = content_of(&output);
    assert!(
        content.starts_with("q\n1.0 0.0 0.0 1.0 200.0 -200.0 cm\n72 642 50 50 re"),
        "{content}"
    );
}

#[test]
fn keeps_pictures_inside_the_clipping_around_them() {
    let source = picture_page(
        0,
        "72 600 50 100 re W n q 100 0 0 100 72 600 cm /Im1 Do Q 0 0 m 10 10 l S",
        1,
    );
    let image = &images_of(&source)[0];
    assert!(near_bounds(image.bounds, bounds(72.0, 92.0, 100.0, 100.0)));
    assert!(near_bounds(image.limits, bounds(72.0, 92.0, 50.0, 100.0)));
}

#[test]
fn wraps_a_picture_its_group_shares_with_other_paint() {
    let source = picture_page(0, "q 100 0 0 50 72 642 cm /Im1 Do 0 0 m 1 1 l S Q", 1);
    let image = &images_of(&source)[0];
    let moved = bounds(82.0, 100.0, 100.0, 50.0);
    let output = apply(&source, &[image_edit(image, Some(moved))]).unwrap();
    assert!(near_bounds(images_of(&output)[0].bounds, moved));
    let content = content_of(&output);
    assert!(
        content.contains("q\n1.0 0.0 0.0 1.0 0.1 0.0 cm\n/Im1 Do\nQ\n0 0 m"),
        "{content}"
    );
}

#[test]
fn removes_pictures_from_the_saved_file() {
    let source = picture_page(
        0,
        "q 100 0 0 50 72 642 cm /Im1 Do Q q 100 0 0 50 72 400 cm /Im2 Do Q \
         q 50 0 0 50 300 300 cm /Im2 Do Q q 20 0 0 20 300 100 cm \
         BI /W 2 /H 2 /CS /G /BPC 8 ID wxyz EI Q",
        2,
    );
    let images = images_of(&source);
    let removed: Vec<Edit> = [0, 1, 3]
        .iter()
        .map(|&i| image_edit(&images[i], None))
        .collect();

    let preview = apply_document(&source, Document::load_mem(&source).unwrap(), &removed)
        .unwrap()
        .bytes;
    assert!(preview.starts_with(&source));
    assert_eq!(images_of(&preview).len(), 1);

    let output = apply(&source, &removed).unwrap();
    assert!(!output.windows(4).any(|w| w == b"wxyz"));
    assert_eq!(image_objects(&output), [2]);
    let doc = Document::load_mem(&output).unwrap();
    assert!(doc.trailer.get(b"Prev").is_err());
    let (resources, _) = doc.get_page_resources(doc.get_pages()[&1]).unwrap();
    let xobjects = resources
        .unwrap()
        .get(b"XObject")
        .unwrap()
        .as_dict()
        .unwrap();
    assert!(!xobjects.has(b"Im1") && xobjects.has(b"Im2") && xobjects.has(b"Fm1"));
    let after = images_of(&output);
    assert_eq!(after.len(), 1);
    assert!(near_bounds(after[0].bounds, images[2].bounds));
}

fn remove_first_picture(source: &[u8]) -> Vec<u8> {
    let output = apply(source, &[image_edit(&images_of(source)[0], None)]).unwrap();
    assert!(
        Document::load_mem(&output)
            .unwrap()
            .trailer
            .get(b"Prev")
            .is_err()
    );
    output
}

fn pictures_on(source: &[u8], page: usize) -> usize {
    page_images(&Document::load_mem(source).unwrap(), page)
        .unwrap()
        .len()
}

#[test]
fn removes_pictures_from_shared_and_inherited_resources() {
    for inherit in [false, true] {
        let source = shared_pictures(
            &[
                "q 100 0 0 50 72 642 cm /Im1 Do Q",
                "q 100 0 0 50 72 642 cm /Im2 Do Q /Fm1 Do",
            ],
            "0 0 10 10 re f",
            inherit,
        );
        let output = remove_first_picture(&source);
        assert_eq!(image_objects(&output), [2], "inherit: {inherit}");
        assert_eq!(pictures_on(&output, 0), 0);
        assert_eq!(pictures_on(&output, 1), 1);
    }
}

#[test]
fn keeps_shared_pictures_other_pages_still_draw() {
    for inherit in [false, true] {
        for (other, form) in [
            ("q 50 0 0 50 300 300 cm /Im1 Do Q", ""),
            ("/Fm1 Do", "q 10 0 0 10 0 0 cm /Im1 Do Q"),
        ] {
            let source =
                shared_pictures(&["q 100 0 0 50 72 642 cm /Im1 Do Q", other], form, inherit);
            let output = remove_first_picture(&source);
            assert_eq!(
                image_objects(&output),
                [1, 2],
                "inherit: {inherit}, other: {other}"
            );
            assert_eq!(pictures_on(&output, 0), 0);
            let doc = Document::load_mem(&output).unwrap();
            let page_id = doc.get_pages()[&2];
            let (resources, ids) = doc.get_page_resources(page_id).unwrap();
            let has_im1 = resources
                .into_iter()
                .chain(ids.iter().filter_map(|&id| doc.get_dictionary(id).ok()))
                .filter_map(|r| r.get(b"XObject").ok())
                .filter_map(|x| match x {
                    Object::Reference(id) => doc.get_dictionary(*id).ok(),
                    x => x.as_dict().ok(),
                })
                .any(|x| x.has(b"Im1"));
            assert!(has_im1, "inherit: {inherit}, other: {other}");
        }
    }
}

#[test]
fn combines_picture_and_line_edits_on_a_page() {
    let source = picture_page(
        0,
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET q 100 0 0 50 72 500 cm /Im1 Do Q",
        1,
    );
    let image = &images_of(&source)[0];
    let line = &lines_of(&source)[0];
    let moved = bounds(200.0, 300.0, 100.0, 50.0);
    let output = apply(
        &source,
        &[line_edit(line, "Hi"), image_edit(image, Some(moved))],
    )
    .unwrap();
    assert_eq!(lines_of(&output)[0].text, "Hi");
    assert!(near_bounds(images_of(&output)[0].bounds, moved));
}

#[test]
fn rejects_stale_and_invalid_picture_edits() {
    let source = picture_page(0, "q 100 0 0 50 72 642 cm /Im1 Do Q", 1);
    let image = &images_of(&source)[0];
    let mut shifted = image.clone();
    shifted.bounds.x += 1.0;
    let mut missing = image.clone();
    missing.op = 1;
    for edit in [image_edit(&shifted, None), image_edit(&missing, None)] {
        assert!(matches!(
            apply(&source, &[edit]),
            Err(EditError::StaleImage(1))
        ));
    }
    let flat = image_edit(image, Some(bounds(0.0, 0.0, 10.0, 0.0)));
    assert!(matches!(
        apply(&source, &[flat]),
        Err(EditError::Invalid(_))
    ));
}

#[test]
fn refuses_to_remove_pictures_from_encrypted_files() {
    let source = encrypted(Permissions::all(), false);
    let edit = Edit::Image(ImageEdit {
        page: 0,
        op: 0,
        original: bounds(0.0, 0.0, 1.0, 1.0),
        bounds: None,
    });
    assert!(matches!(
        apply(&source, &[edit]),
        Err(EditError::EncryptedImageRemoval)
    ));
}

#[test]
fn deserializes_picture_edits() {
    let json = r#"{"kind":"image","page":2,"op":7,
        "original":{"x":1,"y":2,"width":3,"height":4},"bounds":null}"#;
    let edit: Edit = serde_json::from_str(json).unwrap();
    let Edit::Image(image) = edit else {
        panic!("{edit:?}")
    };
    assert_eq!((image.page, image.op), (2, 7));
    assert!(image.bounds.is_none());
}
