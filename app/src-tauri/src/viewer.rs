//! Reader session: opens documents and serves rendered pages through the `pdf` URI scheme.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use serde::Serialize;
use tauri::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use tauri::http::{Request, Response, StatusCode};
use tauri::{
    AppHandle, Emitter, Manager, State, UriSchemeContext, UriSchemeResponder, Url, WebviewWindow,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use ukurasa_core::Stamp;
use ukurasa_render::{
    Link, LinkTarget, OutlineItem, PageSize, RenderError, RenderRequest, RenderResult,
    RenderService, WorkerCommand, WorkerRenderer,
};
use ukurasa_worker::{PDFIUM_PATH_ENV, WORKER_FLAG};

use crate::editor;
use crate::recent::{self, RecentFile};

pub const SCHEME: &str = "pdf";
const SESSION_EVENT: &str = "session-changed";
pub const MAIN_WINDOW: &str = "main";

pub struct Viewer {
    service: RenderService,
    session: Mutex<Session>,
    recent_file: Option<PathBuf>,
    recent_write: Mutex<()>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Increases with every change so the window can ignore out-of-order snapshots.
    revision: u64,
    /// Name of the file being opened.
    loading: Option<String>,
    document: Option<DocumentView>,
    error: Option<String>,
    recent: Vec<RecentFile>,
    #[serde(skip)]
    last_open: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    id: u64,
    name: String,
    pages: Vec<PageSize>,
    outline: Vec<OutlineItem>,
    #[serde(skip)]
    path: PathBuf,
    /// The file as it was when opened, so a save can tell whether something else changed it.
    #[serde(skip)]
    stamp: Option<Stamp>,
}

impl Viewer {
    /// Runs PDFium in a copy of this executable so an engine crash cannot close the window.
    pub fn new(app: &AppHandle) -> io::Result<Self> {
        let mut command = WorkerCommand {
            program: env::current_exe()?,
            args: vec![WORKER_FLAG.into()],
            env: Vec::new(),
        };
        if env::var_os(PDFIUM_PATH_ENV).is_none()
            && let Ok(resources) = app.path().resource_dir()
        {
            command
                .env
                .push((PDFIUM_PATH_ENV.into(), resources.join("pdfium").into()));
        }
        let recent_file = app
            .path()
            .app_data_dir()
            .ok()
            .map(|dir| dir.join(recent::FILE_NAME));
        let recent = recent_file.as_deref().map(recent::load).unwrap_or_default();
        Ok(Self {
            service: RenderService::new(WorkerRenderer::new(command)),
            session: Mutex::new(Session {
                recent,
                ..Session::default()
            }),
            recent_file,
            recent_write: Mutex::default(),
        })
    }

    fn session(&self) -> MutexGuard<'_, Session> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the current list; the write lock keeps an older list from landing last.
    fn save_recent(&self) {
        let Some(file) = &self.recent_file else {
            return;
        };
        let _write = self
            .recent_write
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let recent = self.session().recent.clone();
        let _ = recent::save(file, &recent);
    }

    pub fn document_path(&self, id: u64) -> Option<PathBuf> {
        self.document_file(id).map(|(path, _)| path)
    }

    /// The path of document `id` and its stamp from when it was opened.
    pub fn document_file(&self, id: u64) -> Option<(PathBuf, Option<Stamp>)> {
        let session = self.session();
        let document = session
            .document
            .as_ref()
            .filter(|document| document.id == id)?;
        Some((document.path.clone(), document.stamp))
    }

    /// Shows `pdf`, an edited copy of document `doc`, in place of the file; see
    /// [`RenderService::revise`].
    pub fn revise(&self, doc: u64, pdf: Option<Arc<[u8]>>, pages: &[u32]) -> Result<(), String> {
        self.service
            .revise(doc, pdf, pages)
            .map_err(|error| match error {
                RenderError::Stale => "This document is no longer open.".to_owned(),
                error => format!("The preview couldn't be updated. {error}"),
            })
    }

    fn update<T>(&self, app: &AppHandle, change: impl FnOnce(&mut Session) -> T) -> T {
        let (snapshot, result) = {
            let mut session = self.session();
            let result = change(&mut session);
            session.revision += 1;
            (session.clone(), result)
        };
        let _ = app.emit(SESSION_EVENT, snapshot);
        result
    }

    pub fn open(&self, app: &AppHandle, path: &Path) {
        let name = display_name(path);
        let ticket = self.update(app, |session| {
            session.last_open += 1;
            session.loading = Some(name.clone());
            session.error = None;
            session.last_open
        });
        let stamp = Stamp::of(path).ok();
        let result = self.service.open(path);
        if result == Err(RenderError::Cancelled) {
            return;
        }
        let title = self.update(app, |session| {
            let latest = session.last_open == ticket;
            if latest {
                session.loading = None;
            }
            match result {
                Ok(opened) if session.document.as_ref().is_none_or(|d| d.id < opened.id) => {
                    session.document = Some(DocumentView {
                        id: opened.id,
                        name: name.clone(),
                        pages: opened.pages,
                        outline: opened.outline,
                        path: path.to_owned(),
                        stamp,
                    });
                    recent::touch(&mut session.recent, path, &name, recent::now());
                    Some(window_title(Some(&name), false))
                }
                Ok(_) => None,
                Err(error) => {
                    if latest {
                        session.error = Some(format!("Could not open {name}. {error}"));
                        recent::refresh(&mut session.recent);
                    }
                    None
                }
            }
        });
        if let Some(title) = title {
            self.save_recent();
            set_window_title(app, &title);
        }
    }

    /// Titles the window with the open document's name, marked while it has unsaved edits.
    pub fn show_unsaved(&self, app: &AppHandle, unsaved: bool) {
        let name = self.session().document.as_ref().map(|d| d.name.clone());
        set_window_title(app, &window_title(name.as_deref(), unsaved));
    }

    fn fail(&self, app: &AppHandle, message: &str) {
        self.update(app, |session| session.error = Some(message.to_owned()));
    }
}

/// Opens `path` on a background thread; progress is reported through session events.
pub fn open_in_background(app: &AppHandle, path: PathBuf) {
    let app = app.clone();
    thread::spawn(move || {
        if let Some(viewer) = app.try_state::<Viewer>() {
            viewer.open(&app, &path);
        }
    });
}

/// Opens the first PDF in `paths` once any unsaved edits are dealt with, or reports that none
/// was given.
pub fn open_first_pdf(app: &AppHandle, paths: &[PathBuf]) {
    match paths.iter().find(|path| is_pdf(path)) {
        Some(path) => {
            let (handle, path) = (app.clone(), path.clone());
            editor::confirm_discard(app, move || open_in_background(&handle, path));
        }
        None => {
            if let Some(viewer) = app.try_state::<Viewer>() {
                viewer.fail(app, "Only PDF files can be opened.");
            }
        }
    }
}

/// Returns the first file named on the command line.
pub fn launch_path() -> Option<PathBuf> {
    env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .find(|path| !path.to_string_lossy().starts_with('-') && path.is_file())
        .and_then(|path| std::path::absolute(path).ok())
}

#[tauri::command]
pub fn session(viewer: State<'_, Viewer>) -> Session {
    viewer.session().clone()
}

#[tauri::command]
pub fn open_dialog(app: AppHandle, window: WebviewWindow) {
    let dialog = app
        .dialog()
        .file()
        .add_filter("PDF documents", &["pdf"])
        .set_parent(&window);
    dialog.pick_file(move |file| {
        if let Some(path) = file.and_then(|file| file.into_path().ok()) {
            open_first_pdf(&app, &[path]);
        }
    });
}

#[tauri::command]
pub fn open_path(app: AppHandle, path: PathBuf) {
    open_first_pdf(&app, &[path]);
}

#[tauri::command]
pub fn forget_recent(app: AppHandle, viewer: State<'_, Viewer>, path: PathBuf) {
    viewer.update(&app, |session| recent::forget(&mut session.recent, &path));
    viewer.save_recent();
}

#[tauri::command]
pub fn cancel_renders(viewer: State<'_, Viewer>, ids: Vec<u64>) {
    viewer.service.cancel(&ids);
}

/// The links on a page. Web targets are normalized, and any that cannot be opened safely are
/// left out.
#[tauri::command]
pub async fn page_links(app: AppHandle, doc: u64, page: u32) -> Result<Vec<Link>, String> {
    let links = tauri::async_runtime::spawn_blocking(move || {
        app.state::<Viewer>().service.links(doc, page)
    })
    .await
    .map_err(|_| "The links couldn't be read.".to_owned())?
    .map_err(|error| error.to_string())?;
    Ok(links
        .into_iter()
        .filter_map(|mut link| {
            if let LinkTarget::Uri { uri } = &mut link.target {
                *uri = web_link(uri)?;
            }
            Some(link)
        })
        .collect())
}

/// Opens a web or email link in the default app.
#[tauri::command]
pub fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    let url = web_link(&url).ok_or("Only web and email links can be opened.")?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| format!("The link couldn't be opened. {error}"))
}

/// Accepts http, https, and mailto URLs. Links written without a scheme, as in
/// `example.com/page` or `name@example.com`, get one.
fn web_link(uri: &str) -> Option<String> {
    let uri = uri.trim();
    let url = match Url::parse(uri) {
        Ok(url) if !url.scheme().contains('.') => url,
        _ if uri.starts_with(|c: char| c.is_alphanumeric()) => {
            let scheme = if uri.contains('@') && !uri.contains('/') {
                "mailto:"
            } else {
                "https://"
            };
            Url::parse(&format!("{scheme}{uri}")).ok()?
        }
        _ => return None,
    };
    let openable = match url.scheme() {
        "http" | "https" => url.host_str().is_some_and(|host| !host.is_empty()),
        "mailto" => !url.path().is_empty(),
        _ => false,
    };
    openable.then(|| url.into())
}

/// Serves `/{doc}-{page}-{width}-{priority}-{id}` as a PNG.
pub fn serve_page(
    ctx: UriSchemeContext<'_, tauri::Wry>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let Some(viewer) = ctx.app_handle().try_state::<Viewer>() else {
        return responder.respond(empty(StatusCode::SERVICE_UNAVAILABLE));
    };
    let Some(render) = parse_page_path(request.uri().path()) else {
        return responder.respond(empty(StatusCode::BAD_REQUEST));
    };
    viewer.service.render(render, move |result| {
        responder.respond(page_response(result))
    });
}

fn parse_page_path(path: &str) -> Option<RenderRequest> {
    let fields = path
        .trim_start_matches('/')
        .split('-')
        .map(|field| field.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    let [doc, page, width, priority, id] = fields[..] else {
        return None;
    };
    Some(RenderRequest {
        id,
        doc,
        page: page.try_into().ok()?,
        width: u32::try_from(width).ok().filter(|width| *width > 0)?,
        priority: priority.try_into().ok()?,
    })
}

fn page_response(result: RenderResult) -> Response<Vec<u8>> {
    match result {
        Ok(png) => {
            let mut response = Response::new(png.to_vec());
            let headers = response.headers_mut();
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
            headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(RenderError::Cancelled) => empty(StatusCode::SERVICE_UNAVAILABLE),
        Err(RenderError::Stale) => empty(StatusCode::GONE),
        Err(_) => empty(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn empty(status: StatusCode) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = status;
    response
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn window_title(name: Option<&str>, unsaved: bool) -> String {
    match name {
        Some(name) if unsaved => format!("● {name} · Ukurasa"),
        Some(name) => format!("{name} · Ukurasa"),
        None => "Ukurasa".to_owned(),
    }
}

fn set_window_title(app: &AppHandle, title: &str) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.set_title(title);
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_page_paths() {
        let request = parse_page_path("/3-12-800-1-42").unwrap();
        assert_eq!(
            request,
            RenderRequest {
                id: 42,
                doc: 3,
                page: 12,
                width: 800,
                priority: 1,
            }
        );
        for invalid in [
            "/",
            "/3-12-800-1",
            "/3-12-0-1-42",
            "/3-12-800-300-42",
            "/a-1-2-3-4",
            "/1-2-3-4-5-6",
        ] {
            assert_eq!(parse_page_path(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn marks_unsaved_documents_in_the_title() {
        assert_eq!(window_title(Some("a.pdf"), false), "a.pdf · Ukurasa");
        assert_eq!(window_title(Some("a.pdf"), true), "● a.pdf · Ukurasa");
        assert_eq!(window_title(None, true), "Ukurasa");
    }

    #[test]
    fn web_links_are_normalized_and_restricted() {
        for (uri, expected) in [
            ("https://example.com/cv", Some("https://example.com/cv")),
            (" HTTP://Example.com ", Some("http://example.com/")),
            (
                "www.linkedin.com/in/someone",
                Some("https://www.linkedin.com/in/someone"),
            ),
            ("github.com", Some("https://github.com/")),
            ("someone@example.com", Some("mailto:someone@example.com")),
            (
                "mailto:someone@example.com",
                Some("mailto:someone@example.com"),
            ),
            ("javascript:alert(1)", None),
            ("file:///C:/Windows/notepad.exe", None),
            ("ms-settings:", None),
            ("mailto:", None),
            ("/relative/path", None),
            ("#section", None),
            ("", None),
        ] {
            assert_eq!(web_link(uri).as_deref(), expected, "{uri}");
        }
    }

    #[test]
    fn recognizes_pdf_extensions() {
        assert!(is_pdf(Path::new("report.PDF")));
        assert!(!is_pdf(Path::new("notes.txt")));
        assert!(!is_pdf(Path::new("pdf")));
    }
}
