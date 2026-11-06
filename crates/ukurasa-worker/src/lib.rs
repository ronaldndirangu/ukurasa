//! Worker process that hosts PDFium for rendering, text extraction, and search indexing.
//!
//! The worker reads [`Request`] frames from stdin and writes [`Response`] frames to stdout,
//! so a crash inside PDFium only takes down this process.

use std::collections::HashMap;
use std::env;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pdfium_render::prelude::*;
use ukurasa_geom::{PageGeometry, Rect, Rotation};
use ukurasa_render::{
    DocumentInfo, ErrorKind, Link, LinkTarget, OutlineItem, PageSize, Request, Response,
    read_frame, write_frame,
};

/// Command-line flag that makes the desktop app executable run as a worker.
pub const WORKER_FLAG: &str = "--ukurasa-worker";
/// Directory or file path of the PDFium library.
pub const PDFIUM_PATH_ENV: &str = "UKURASA_PDFIUM_PATH";

const MAX_PIXELS: f32 = 32_000_000.0;
const MAX_SIDE: f32 = 16_384.0;
const MAX_OUTLINE_ITEMS: usize = 10_000;
const MAX_OUTLINE_DEPTH: usize = 32;
const MAX_TITLE_CHARS: usize = 300;
const MAX_LINKS: usize = 2_000;
const MAX_URI_LEN: usize = 4_096;

type Failure = (ErrorKind, String);

pub fn run() -> ExitCode {
    let input = BufReader::new(io::stdin().lock());
    let output = BufWriter::new(io::stdout().lock());
    match serve(input, output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ukurasa-worker: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Answers requests until `input` ends.
pub fn serve(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
    let pdfium = load_pdfium();
    let mut open = None;
    loop {
        let (request, payload) = match read_frame(&mut input) {
            Ok(frame) => frame,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        };
        let reply = match &pdfium {
            Ok(pdfium) => handle(pdfium, &mut open, request, payload),
            Err(message) => Err((ErrorKind::Internal, message.clone())),
        };
        let (response, payload) =
            reply.unwrap_or_else(|(kind, message)| (Response::Error { kind, message }, Vec::new()));
        write_frame(&mut output, &response, &payload)?;
    }
}

struct Open<'a> {
    path: PathBuf,
    /// The file, or an edited copy of it after a revision.
    document: PdfDocument<'a>,
}

fn handle<'a>(
    pdfium: &'a Pdfium,
    open: &mut Option<Open<'a>>,
    request: Request,
    payload: Vec<u8>,
) -> Result<(Response, Vec<u8>), Failure> {
    match request {
        Request::Open { path } => {
            let document = pdfium.load_pdf_from_file(&path, None).map_err(classify)?;
            let pages = page_sizes(&document)?;
            let outline = outline(&document, pages.len());
            *open = Some(Open { path, document });
            Ok((
                Response::Opened(DocumentInfo { pages, outline }),
                Vec::new(),
            ))
        }
        Request::Render { page, width } => {
            let open = open.as_ref().ok_or_else(nothing_open)?;
            render(&open.document, page, width)
        }
        Request::Revise => {
            let open = open.as_mut().ok_or_else(nothing_open)?;
            let revised = if payload.is_empty() {
                pdfium.load_pdf_from_file(&open.path, None)
            } else {
                pdfium.load_pdf_from_byte_vec(payload, None)
            }
            .map_err(classify)?;
            if revised.pages().len() != open.document.pages().len() {
                return Err((
                    ErrorKind::Format,
                    "the revision changes the page count".to_owned(),
                ));
            }
            open.document = revised;
            Ok((Response::Revised, Vec::new()))
        }
        Request::Links { page } => {
            let open = open.as_ref().ok_or_else(nothing_open)?;
            let links = page_links(&open.document, page)?;
            Ok((Response::Links { links }, Vec::new()))
        }
        Request::Close => {
            *open = None;
            Ok((Response::Closed, Vec::new()))
        }
    }
}

fn nothing_open() -> Failure {
    (ErrorKind::Internal, "no document is open".to_owned())
}

fn page_sizes(document: &PdfDocument) -> Result<Vec<PageSize>, Failure> {
    let sizes = document.pages().page_sizes().map_err(classify)?;
    Ok(sizes
        .iter()
        .map(|rect| PageSize {
            width: rect.width().value,
            height: rect.height().value,
        })
        .collect())
}

/// Reads the bookmark tree. The limits stop cyclic or hostile outlines from stalling the worker.
fn outline(document: &PdfDocument, page_count: usize) -> Vec<OutlineItem> {
    let mut budget = MAX_OUTLINE_ITEMS;
    document.bookmarks().root().map_or_else(Vec::new, |first| {
        outline_level(first, 1, page_count, &mut budget)
    })
}

fn outline_level(
    first: PdfBookmark,
    depth: usize,
    page_count: usize,
    budget: &mut usize,
) -> Vec<OutlineItem> {
    let mut items = Vec::new();
    let mut next = Some(first);
    while let Some(bookmark) = next {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let children = match bookmark.first_child() {
            Some(child) if depth < MAX_OUTLINE_DEPTH => {
                outline_level(child, depth + 1, page_count, budget)
            }
            _ => Vec::new(),
        };
        items.push(OutlineItem {
            title: clean_title(&bookmark.title().unwrap_or_default()),
            page: bookmark_page(&bookmark).filter(|page| (*page as usize) < page_count),
            children,
        });
        next = bookmark.next_sibling();
    }
    items
}

fn bookmark_page(bookmark: &PdfBookmark) -> Option<u32> {
    let index = match bookmark.destination() {
        Some(destination) => destination.page_index().ok()?,
        None => bookmark
            .action()?
            .as_local_destination_action()?
            .destination()
            .ok()?
            .page_index()
            .ok()?,
    };
    u32::try_from(index).ok()
}

fn clean_title(title: &str) -> String {
    title
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// Reads the link annotations on a page. Internal targets are resolved by PDFium, which
/// covers `/Dest`, GoTo actions, and named destinations.
fn page_links(document: &PdfDocument, page: u32) -> Result<Vec<Link>, Failure> {
    let index = PdfPageIndex::try_from(page).map_err(|_| page_missing(page))?;
    let pages = document.pages();
    let page_count = pages.len();
    let source = pages.get(index).map_err(classify)?;
    let geometry = page_geometry(&source);
    let mut targets = HashMap::new();
    let mut links = Vec::new();
    for annotation in source.annotations().iter() {
        if links.len() == MAX_LINKS {
            break;
        }
        let Some(link) = annotation
            .as_link_annotation()
            .and_then(|annotation| annotation.link().ok())
        else {
            continue;
        };
        let Some(area) = link
            .rect()
            .ok()
            .and_then(|rect| display_rect(&geometry, rect))
        else {
            continue;
        };
        let uri = link
            .action()
            .and_then(|action| action.as_uri_action()?.uri().ok());
        let target = match uri {
            Some(uri) => {
                let uri = uri.trim();
                (!uri.is_empty() && uri.len() <= MAX_URI_LEN).then(|| LinkTarget::Uri {
                    uri: uri.to_owned(),
                })
            }
            None => link.destination().and_then(|destination| {
                let index = destination.page_index().ok()?;
                if !(0..page_count).contains(&index) {
                    return None;
                }
                let top = destination.view_settings().ok().and_then(|view| {
                    let (x, y) = view_point(&view)?;
                    let geometry = targets
                        .entry(index)
                        .or_insert_with(|| pages.get(index).ok().map(|page| page_geometry(&page)))
                        .as_ref()?;
                    destination_top(geometry, x, y)
                });
                Some(LinkTarget::Page {
                    page: u32::try_from(index).ok()?,
                    top,
                })
            }),
        };
        if let Some(target) = target {
            let (x, y, width, height) = area;
            links.push(Link {
                x,
                y,
                width,
                height,
                target,
            });
        }
    }
    Ok(links)
}

fn page_geometry(page: &PdfPage) -> PageGeometry {
    let boundaries = page.boundaries();
    let media = boundaries
        .media()
        .ok()
        .map(|boundary| rect(boundary.bounds));
    let crop = boundaries.crop().ok().map(|boundary| rect(boundary.bounds));
    let rotate = match page.rotation() {
        Ok(PdfPageRenderRotation::Degrees90) => 90,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
        _ => 0,
    };
    PageGeometry::new(media, crop, rotate)
}

fn rect(bounds: PdfRect) -> Rect {
    Rect::new(
        f64::from(bounds.left().value),
        f64::from(bounds.bottom().value),
        f64::from(bounds.right().value),
        f64::from(bounds.top().value),
    )
}

/// Maps a user-space link rectangle to the displayed page as `(x, y, width, height)`,
/// clipped to the page. Returns `None` when nothing visible remains.
fn display_rect(geometry: &PageGeometry, bounds: PdfRect) -> Option<(f32, f32, f32, f32)> {
    let user = rect(bounds);
    let corners = [(user.x0, user.y0), (user.x1, user.y1)].map(|(x, y)| geometry.to_display(x, y));
    let (width, height) = geometry.display_size();
    let visible = Rect::bounding(&corners).intersect(&Rect::new(0.0, 0.0, width, height));
    if visible.is_empty() || !visible.x0.is_finite() || !visible.y1.is_finite() {
        return None;
    }
    Some((
        visible.x0 as f32,
        visible.y0 as f32,
        visible.width() as f32,
        visible.height() as f32,
    ))
}

/// The user-space point a destination scrolls to, when it names one.
fn view_point(view: &PdfDestinationViewSettings) -> Option<(Option<f32>, Option<f32>)> {
    let value = |points: &Option<PdfPoints>| points.map(|points| points.value);
    match view {
        PdfDestinationViewSettings::SpecificCoordinatesAndZoom(x, y, _) => {
            Some((value(x), value(y)))
        }
        PdfDestinationViewSettings::FitPageHorizontallyToWindow(top)
        | PdfDestinationViewSettings::FitBoundsHorizontallyToWindow(top) => {
            Some((None, value(top)))
        }
        PdfDestinationViewSettings::FitPageVerticallyToWindow(left)
        | PdfDestinationViewSettings::FitBoundsVerticallyToWindow(left) => {
            Some((value(left), None))
        }
        PdfDestinationViewSettings::FitPageToRectangle(rect) => {
            Some((Some(rect.left().value), Some(rect.top().value)))
        }
        _ => None,
    }
}

/// Distance from the top of the displayed page to a destination point. Only the coordinate
/// that runs down the page after rotation matters; without it there is no top.
fn destination_top(geometry: &PageGeometry, x: Option<f32>, y: Option<f32>) -> Option<f32> {
    let crop = geometry.crop;
    let (x, y) = match geometry.rotation {
        Rotation::R0 | Rotation::R180 => (crop.x0, f64::from(y?)),
        Rotation::R90 | Rotation::R270 => (f64::from(x?), crop.y0),
    };
    let (_, top) = geometry.to_display(x, y);
    let (_, height) = geometry.display_size();
    top.is_finite().then(|| top.clamp(0.0, height) as f32)
}

fn render(document: &PdfDocument, page: u32, width: u32) -> Result<(Response, Vec<u8>), Failure> {
    let index = PdfPageIndex::try_from(page).map_err(|_| page_missing(page))?;
    let pages = document.pages();
    let size = pages.page_size(index).map_err(classify)?;
    let width = clamp_width(width, size.width().value, size.height().value)
        .ok_or((ErrorKind::Page, format!("page {} has no area", page + 1)))?;

    let config = PdfRenderConfig::new()
        .set_target_width(width)
        .render_form_data(true);
    let page = pages.get(index).map_err(classify)?;
    let bitmap = page.render_with_config(&config).map_err(classify)?;
    let (width, height) = (bitmap.width() as u32, bitmap.height() as u32);
    let rgba = bitmap.as_rgba_bytes();
    if rgba.len() != width as usize * height as usize * 4 {
        return Err((ErrorKind::Internal, "unexpected bitmap layout".to_owned()));
    }
    let png = encode_png(&rgba, width, height)
        .map_err(|error| (ErrorKind::Internal, format!("PNG encoding failed: {error}")))?;
    Ok((Response::Rendered { width, height }, png))
}

/// Limits the bitmap so neither side exceeds [`MAX_SIDE`] and the area stays under [`MAX_PIXELS`].
fn clamp_width(requested: u32, page_width: f32, page_height: f32) -> Option<Pixels> {
    if !(page_width > 0.0 && page_height > 0.0) {
        return None;
    }
    let aspect = page_height / page_width;
    let limit = MAX_SIDE
        .min(MAX_SIDE / aspect)
        .min((MAX_PIXELS / aspect).sqrt());
    Some((requested as f32).min(limit).max(1.0) as Pixels)
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, png::EncodingError> {
    let (pixels, _) = rgba.as_chunks::<4>();
    let rgb: Vec<u8> = pixels.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect();
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgb)?;
    writer.finish()?;
    Ok(png)
}

fn classify(error: PdfiumError) -> Failure {
    match error {
        PdfiumError::IoError(error) => (ErrorKind::Io, error.to_string()),
        PdfiumError::PdfiumLibraryInternalError(internal) => match internal {
            PdfiumInternalError::FileError => {
                (ErrorKind::Io, "the file could not be opened".into())
            }
            PdfiumInternalError::FormatError => (ErrorKind::Format, "invalid PDF structure".into()),
            PdfiumInternalError::PasswordError => {
                (ErrorKind::Password, "a password is required".into())
            }
            PdfiumInternalError::SecurityError => (
                ErrorKind::Unsupported,
                "unsupported security handler".into(),
            ),
            PdfiumInternalError::PageError => (ErrorKind::Page, "the page is damaged".into()),
            other => (ErrorKind::Internal, format!("{other:?}")),
        },
        PdfiumError::PageIndexOutOfBounds => (ErrorKind::Page, "no such page".into()),
        other => (ErrorKind::Internal, other.to_string()),
    }
}

fn page_missing(page: u32) -> Failure {
    (
        ErrorKind::Page,
        format!("page {} does not exist", u64::from(page) + 1),
    )
}

fn load_pdfium() -> Result<Pdfium, String> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os(PDFIUM_PATH_ENV).map(PathBuf::from) {
        candidates.push(library_path(path));
    }
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        candidates.push(library_path(dir));
    }

    let mut errors = Vec::new();
    for candidate in candidates.iter().filter(|path| path.is_file()) {
        match Pdfium::bind_to_library(candidate) {
            Ok(bindings) => return Ok(Pdfium::new(bindings)),
            Err(error) => errors.push(format!("{}: {error}", candidate.display())),
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        Err(error) => {
            errors.push(format!("system library: {error}"));
            Err(format!(
                "PDFium could not be loaded ({}); set {PDFIUM_PATH_ENV} to its location",
                errors.join("; ")
            ))
        }
    }
}

fn library_path(path: PathBuf) -> PathBuf {
    if path.is_dir() {
        Pdfium::pdfium_platform_library_name_at_path(&path)
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_width_respects_limits() {
        assert_eq!(clamp_width(800, 612.0, 792.0), Some(800));
        assert_eq!(clamp_width(0, 612.0, 792.0), Some(1));
        assert_eq!(clamp_width(100_000, 612.0, 792.0), Some(4972));
        assert_eq!(clamp_width(2_000, 100.0, 10_000.0), Some(163));
        assert_eq!(clamp_width(800, 0.0, 792.0), None);
    }

    #[test]
    fn link_rects_map_to_the_displayed_page() {
        let letter = PageGeometry::new(None, None, 0);
        let bounds = PdfRect::new_from_values(700.0, 72.0, 712.0, 172.0);
        assert_eq!(
            display_rect(&letter, bounds),
            Some((72.0, 80.0, 100.0, 12.0))
        );

        let turned = PageGeometry::new(None, None, 90);
        assert_eq!(
            display_rect(&turned, bounds),
            Some((700.0, 72.0, 12.0, 100.0))
        );

        let outside = PdfRect::new_from_values(900.0, 72.0, 950.0, 172.0);
        assert_eq!(display_rect(&letter, outside), None);
    }

    #[test]
    fn destination_tops_follow_rotation() {
        let letter = PageGeometry::new(None, None, 0);
        assert_eq!(destination_top(&letter, None, Some(692.0)), Some(100.0));
        assert_eq!(destination_top(&letter, Some(10.0), None), None);
        assert_eq!(destination_top(&letter, None, Some(2000.0)), Some(0.0));

        let turned = PageGeometry::new(None, None, 90);
        assert_eq!(
            destination_top(&turned, Some(100.0), Some(5.0)),
            Some(100.0)
        );
        assert_eq!(destination_top(&turned, None, Some(5.0)), None);
    }

    #[test]
    fn titles_collapse_whitespace_and_controls() {
        assert_eq!(clean_title("  Part\r\n1\u{0}\tIntro  "), "Part 1 Intro");
        assert_eq!(clean_title(""), "");
        assert_eq!(
            clean_title(&"x".repeat(1000)).chars().count(),
            MAX_TITLE_CHARS
        );
    }
}
