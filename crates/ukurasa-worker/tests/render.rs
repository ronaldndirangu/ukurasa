//! Drives the real worker binary with PDFium through [`WorkerRenderer`].

use std::io::Write;
use std::path::{Path, PathBuf};

use ukurasa_edit::{Edit, Family, Signature, Span, TextBox, VectorPath};
use ukurasa_render::{
    ErrorKind, Link, LinkTarget, OutlineItem, PageSize, RenderError, Renderer, WorkerCommand,
    WorkerRenderer,
};
use ukurasa_worker::PDFIUM_PATH_ENV;

fn worker() -> WorkerRenderer {
    let pdfium = std::env::var_os(PDFIUM_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src-tauri/pdfium")
        });
    assert!(
        pdfium.exists(),
        "PDFium not found at {}; run `pnpm install` in app/ or set {PDFIUM_PATH_ENV}",
        pdfium.display()
    );
    WorkerRenderer::new(WorkerCommand {
        program: env!("CARGO_BIN_EXE_ukurasa-worker").into(),
        args: Vec::new(),
        env: vec![(PDFIUM_PATH_ENV.into(), pdfium.into())],
    })
}

/// A two-page PDF: a portrait letter page with a blue square, and a landscape A4 page. Its
/// outline mixes a direct destination, a GoTo action, and a nested entry.
fn sample_pdf() -> Vec<u8> {
    let first = "0 0 1 rg 50 50 100 100 re f BT /F1 24 Tf 72 700 Td (Ukurasa) Tj ET";
    let second = "BT /F1 24 Tf 72 500 Td (Page two) Tj ET";
    build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
        page_object(612, 792, 5),
        page_object(842, 595, 6),
        stream_object(first),
        stream_object(second),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        "<< /Type /Outlines /First 9 0 R /Last 10 0 R /Count 2 >>".to_owned(),
        "<< /Title (Intro) /Parent 8 0 R /Next 10 0 R /Dest [3 0 R /Fit] \
         /First 11 0 R /Last 11 0 R /Count 1 >>"
            .to_owned(),
        "<< /Title (Second page) /Parent 8 0 R /Prev 9 0 R \
         /A << /S /GoTo /D [4 0 R /XYZ 0 595 0] >> >>"
            .to_owned(),
        r"<< /Title (Nested\nentry) /Parent 9 0 R >>".to_owned(),
    ])
}

/// A one-page PDF whose only bookmark names itself as its next sibling.
fn cyclic_outline_pdf() -> Vec<u8> {
    build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_owned(),
        "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>".to_owned(),
        "<< /Title (Loop) /Parent 4 0 R /Next 5 0 R /Dest [3 0 R /Fit] >>".to_owned(),
    ])
}

fn build_pdf(objects: &[String]) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        write!(pdf, "{} 0 obj\n{object}\nendobj\n", index + 1).unwrap();
    }
    let xref = pdf.len();
    write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).unwrap();
    for offset in offsets {
        writeln!(pdf, "{offset:010} 00000 n ").unwrap();
    }
    write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .unwrap();
    pdf
}

fn page_object(width: u32, height: u32, contents: u32) -> String {
    format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents {contents} 0 R \
         /Resources << /Font << /F1 7 0 R >> >> >>"
    )
}

fn stream_object(content: &str) -> String {
    format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn decode_png(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let start = (y * width + x) as usize * 3;
    pixels[start..start + 3].try_into().unwrap()
}

#[test]
fn opens_and_renders_pages() {
    let path = temp_file("sample.pdf", &sample_pdf());
    let mut worker = worker();

    let info = worker.open(&path).unwrap();
    assert_eq!(
        info.pages,
        [
            PageSize {
                width: 612.0,
                height: 792.0
            },
            PageSize {
                width: 842.0,
                height: 595.0
            },
        ]
    );
    let entry = |title: &str, page, children| OutlineItem {
        title: title.into(),
        page,
        children,
    };
    assert_eq!(
        info.outline,
        [
            entry("Intro", Some(0), vec![entry("Nested entry", None, vec![])]),
            entry("Second page", Some(1), vec![]),
        ]
    );

    let (width, height, pixels) = decode_png(&worker.render(0, 306).unwrap());
    assert_eq!((width, height), (306, 396));
    assert_eq!(pixel(&pixels, width, 50, 346), [0, 0, 255], "blue square");
    assert_eq!(pixel(&pixels, width, 0, 0), [255, 255, 255]);
    assert_eq!(pixel(&pixels, width, 305, 395), [255, 255, 255]);

    let (width, height, _) = decode_png(&worker.render(1, 842).unwrap());
    assert_eq!((width, height), (842, 595));

    let missing = worker.render(2, 100).unwrap_err();
    assert!(
        matches!(
            missing,
            RenderError::Worker {
                kind: ErrorKind::Page,
                ..
            }
        ),
        "{missing:?}"
    );

    worker.close().unwrap();
}

#[test]
fn failed_open_reports_the_cause_and_keeps_the_document() {
    let path = temp_file("kept.pdf", &sample_pdf());
    let garbage = temp_file("garbage.pdf", b"this is not a PDF");
    let mut worker = worker();
    worker.open(&path).unwrap();

    let error = worker.open(&garbage).unwrap_err();
    assert!(
        matches!(
            error,
            RenderError::Worker {
                kind: ErrorKind::Format,
                ..
            }
        ),
        "{error:?}"
    );

    let error = worker
        .open(&path.with_file_name("missing.pdf"))
        .unwrap_err();
    assert!(
        matches!(
            error,
            RenderError::Worker {
                kind: ErrorKind::Io,
                ..
            }
        ),
        "{error:?}"
    );

    assert!(worker.render(0, 100).is_ok());
}

#[test]
fn renders_edits_where_they_were_placed() {
    let page = |rotate: u32| {
        format!("<< /Type /Page /Parent 2 0 R /CropBox [10 20 210 320] /Rotate {rotate} >>")
    };
    let source = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 612 792] >>".to_owned(),
        page(0),
        page(90),
        page(270),
    ]);
    let block = |page| {
        Edit::Signature(Signature {
            page,
            x: 30.0,
            y: 40.0,
            width: 50.0,
            height: 20.0,
            path: VectorPath {
                d: "M 0 0 L 10 0 L 10 10 L 0 10 Z".into(),
                width: 10.0,
                height: 10.0,
                fill: true,
                stroke_width: 0.0,
            },
            color: [0, 0, 0],
        })
    };
    let row = |text: &str| {
        vec![Span {
            text: text.into(),
            font: "Helvetica".into(),
            face: None,
            family: Family::Sans,
            bold: false,
            italic: false,
            underline: false,
            size: 20.0,
            color: [0, 0, 0],
        }]
    };
    let text = Edit::Text(TextBox {
        page: 0,
        x: 20.0,
        y: 100.0,
        lines: vec![row("Hello"), row("World")],
    });
    let edited = ukurasa_edit::apply(&source, &[text, block(1), block(2)]).unwrap();
    let path = temp_file("edited.pdf", &edited);
    let mut worker = worker();
    worker.open(&path).unwrap();

    let (width, height, pixels) = decode_png(&worker.render(0, 200).unwrap());
    assert_eq!((width, height), (200, 300));
    let dark: Vec<(u32, u32)> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&pixels, width, x, y)[0] < 128)
        .collect();
    assert!(dark.len() > 100, "text was not drawn");
    let (right, bottom) = (20.0 + 0.722 * 20.0 * 5.0, 100.0 + 2.0 * 1.2 * 20.0);
    for &(x, y) in &dark {
        assert!(
            (20..=right as u32).contains(&x) && (100..=bottom as u32).contains(&y),
            "ink at ({x}, {y}) is outside the text box"
        );
    }

    for page in [1, 2] {
        let (width, height, pixels) = decode_png(&worker.render(page, 300).unwrap());
        assert_eq!((width, height), (300, 200), "page {page}");
        let at = |x, y| pixel(&pixels, width, x, y);
        assert_eq!(at(55, 50), [0, 0, 0], "page {page} inside");
        for (x, y) in [(25, 50), (85, 50), (55, 35), (55, 65)] {
            assert_eq!(at(x, y), [255, 255, 255], "page {page} at ({x}, {y})");
        }
    }
}

#[test]
fn revisions_replace_the_rendered_document_until_reverted() {
    let path = temp_file("revised.pdf", &sample_pdf());
    let mut worker = worker();
    worker.open(&path).unwrap();
    let square = |worker: &mut WorkerRenderer| {
        let (width, _, pixels) = decode_png(&worker.render(0, 306).unwrap());
        pixel(&pixels, width, 50, 346)
    };
    assert_eq!(square(&mut worker), [0, 0, 255]);

    let red = String::from_utf8(sample_pdf())
        .unwrap()
        .replace("0 0 1 rg", "1 0 0 rg");
    worker.revise(Some(red.into_bytes().into())).unwrap();
    assert_eq!(square(&mut worker), [255, 0, 0]);

    let error = worker
        .revise(Some(cyclic_outline_pdf().into()))
        .unwrap_err();
    assert!(
        matches!(
            error,
            RenderError::Worker {
                kind: ErrorKind::Format,
                ..
            }
        ),
        "{error:?}"
    );
    assert_eq!(
        square(&mut worker),
        [255, 0, 0],
        "a failed revision keeps the last"
    );

    worker.revise(None).unwrap();
    assert_eq!(square(&mut worker), [0, 0, 255]);
}

#[test]
fn cyclic_outlines_are_bounded() {
    let path = temp_file("cyclic.pdf", &cyclic_outline_pdf());
    let info = worker().open(&path).unwrap();
    assert!(!info.outline.is_empty());
    assert!(info.outline.len() <= 10_000, "{}", info.outline.len());
    assert!(info.outline.iter().all(|item| item.page == Some(0)));
}

#[test]
fn lists_web_and_internal_links() {
    let link = |rect: &str, target: &str| {
        format!("<< /Type /Annot /Subtype /Link /Rect [{rect}] {target} >>")
    };
    let pdf = build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /Dests << /second [4 0 R /XYZ 0 495 0] >> >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R 7 0 R 8 0 R] >>"
            .to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 842 595] >>".to_owned(),
        link(
            "72 700 172 712",
            "/A << /S /URI /URI (https://example.com/cv) >>",
        ),
        link("72 600 172 612", "/Dest /second"),
        link("72 500 172 512", "/A << /S /GoTo /D [4 0 R /FitH 395] >>"),
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] >>".to_owned(),
    ]);
    let path = temp_file("links.pdf", &pdf);
    let mut worker = worker();
    worker.open(&path).unwrap();

    let area = |y| Link {
        x: 72.0,
        y,
        width: 100.0,
        height: 12.0,
        target: LinkTarget::Uri { uri: String::new() },
    };
    let to = |y, page, top| Link {
        target: LinkTarget::Page { page, top },
        ..area(y)
    };
    let web = Link {
        target: LinkTarget::Uri {
            uri: "https://example.com/cv".into(),
        },
        ..area(80.0)
    };
    assert_eq!(
        worker.links(0).unwrap(),
        [web, to(180.0, 1, Some(100.0)), to(280.0, 1, Some(200.0))]
    );
    assert_eq!(worker.links(1).unwrap(), []);
    assert!(matches!(
        worker.links(2),
        Err(RenderError::Worker {
            kind: ErrorKind::Page,
            ..
        })
    ));
}
