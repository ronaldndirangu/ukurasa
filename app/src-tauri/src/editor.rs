//! Editing commands: previews text edits and saves edits into the file or a copy.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use ukurasa_core::{Edit, EditError, Stamp};
use ukurasa_edit::lopdf::Document;
use ukurasa_edit::{DocumentFont, LineReport, PageImage, TextLine, VectorPath};

use crate::viewer::{self, Viewer};

const CLOSED: &str = "This document is no longer open.";

#[derive(Default)]
pub struct Editor {
    unsaved: AtomicBool,
    /// The open document parsed for text editing, loaded on first use.
    source: Mutex<Option<Source>>,
}

struct Source {
    doc: u64,
    bytes: Vec<u8>,
    document: Document,
    /// The edits in the current preview, by page, with line edits first sorted by runs.
    preview: BTreeMap<usize, Vec<Edit>>,
}

impl Source {
    fn load(viewer: &Viewer, doc: u64) -> Result<Self, String> {
        let path = viewer.document_path(doc).ok_or(CLOSED)?;
        let bytes =
            std::fs::read(&path).map_err(|error| format!("Couldn't read the PDF. {error}"))?;
        let document =
            Document::load_mem(&bytes).map_err(|error| EditError::from(error).to_string())?;
        Ok(Self {
            doc,
            bytes,
            document,
            preview: BTreeMap::new(),
        })
    }
}

impl Editor {
    /// Runs `task` on document `doc`, holding the lock so previews apply in order.
    fn with_source<T>(
        &self,
        viewer: &Viewer,
        doc: u64,
        task: impl FnOnce(&mut Source) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut source = self.source.lock().unwrap_or_else(PoisonError::into_inner);
        let loaded = match source.take() {
            Some(loaded) if loaded.doc == doc => loaded,
            _ => Source::load(viewer, doc)?,
        };
        task(source.insert(loaded))
    }
}

/// What a text preview changed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// Pages that render differently than before.
    pages: Vec<usize>,
    lines: Vec<LineReport>,
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    task: impl FnOnce(&AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || task(&app))
        .await
        .map_err(|_| "The edit failed unexpectedly.".to_owned())?
}

/// The editable lines of text on a page of the open document.
#[tauri::command]
pub async fn text_lines(app: AppHandle, doc: u64, page: usize) -> Result<Vec<TextLine>, String> {
    blocking(app, move |app| {
        let viewer = app.state::<Viewer>();
        app.state::<Editor>().with_source(&viewer, doc, |source| {
            ukurasa_edit::check_line_editing(&source.document)
                .and_then(|()| ukurasa_edit::text_lines(&source.document, page))
                .map_err(|error| error.to_string())
        })
    })
    .await
}

/// The pictures on a page of the open document that can be moved, resized, or removed.
#[tauri::command]
pub async fn page_images(app: AppHandle, doc: u64, page: usize) -> Result<Vec<PageImage>, String> {
    blocking(app, move |app| {
        let viewer = app.state::<Viewer>();
        app.state::<Editor>().with_source(&viewer, doc, |source| {
            ukurasa_edit::check_line_editing(&source.document)
                .and_then(|()| ukurasa_edit::page_images(&source.document, page))
                .map_err(|error| error.to_string())
        })
    })
    .await
}

/// The fonts the open document uses for text, offered when styling edited words.
#[tauri::command]
pub async fn document_fonts(app: AppHandle, doc: u64) -> Result<Vec<DocumentFont>, String> {
    blocking(app, move |app| {
        let viewer = app.state::<Viewer>();
        app.state::<Editor>().with_source(&viewer, doc, |source| {
            Ok(ukurasa_edit::document_fonts(&source.document))
        })
    })
    .await
}

/// Renders the open document with line, text, and picture `edits` applied, replacing the
/// previous preview. Signatures are left out, since the editor draws them over the page.
#[tauri::command]
pub async fn preview_edits(app: AppHandle, doc: u64, edits: Vec<Edit>) -> Result<Preview, String> {
    blocking(app, move |app| {
        let viewer = app.state::<Viewer>();
        app.state::<Editor>().with_source(&viewer, doc, |source| {
            let preview = by_page(edits);
            let pages: Vec<usize> = preview
                .keys()
                .chain(source.preview.keys())
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|page| preview.get(page) != source.preview.get(page))
                .collect();
            let (pdf, lines) = if preview.is_empty() {
                (None, Vec::new())
            } else {
                let edits: Vec<Edit> = preview.values().flatten().cloned().collect();
                let applied =
                    ukurasa_edit::apply_document(&source.bytes, source.document.clone(), &edits)
                        .map_err(|error| error.to_string())?;
                (Some(Arc::from(applied.bytes)), applied.lines)
            };
            let render_pages: Vec<u32> = pages
                .iter()
                .filter_map(|&page| page.try_into().ok())
                .collect();
            viewer.revise(doc, pdf, &render_pages)?;
            source.preview = preview;
            Ok(Preview { pages, lines })
        })
    })
    .await
}

fn by_page(edits: Vec<Edit>) -> BTreeMap<usize, Vec<Edit>> {
    let mut pages: BTreeMap<usize, Vec<Edit>> = BTreeMap::new();
    for edit in edits {
        if !matches!(edit, Edit::Signature(_)) {
            pages.entry(edit.page()).or_default().push(edit);
        }
    }
    for edits in pages.values_mut() {
        edits.sort_by_key(|edit| match edit {
            Edit::Line(line) => (false, line.runs.clone()),
            _ => (true, Vec::new()),
        });
    }
    pages
}

#[tauri::command]
pub fn set_unsaved(app: AppHandle, editor: State<'_, Editor>, unsaved: bool) {
    editor.unsaved.store(unsaved, Ordering::Relaxed);
    app.state::<Viewer>().show_unsaved(&app, unsaved);
}

#[tauri::command]
pub fn signature_path(text: String) -> Result<VectorPath, String> {
    ukurasa_edit::typed_signature(&text).map_err(|error| error.to_string())
}

/// Saves the edits into the open file itself and reopens it. Returns the file name, or `None`
/// when the user cancels.
#[tauri::command]
pub async fn save_document(
    app: AppHandle,
    window: WebviewWindow,
    doc: u64,
    edits: Vec<Edit>,
) -> Result<Option<String>, String> {
    let (path, stamp) = app.state::<Viewer>().document_file(doc).ok_or(CLOSED)?;
    run_save(move || {
        if !confirm_signatures(&window, &path, &edits)? {
            return Ok(None);
        }
        save_in_place(&app, &path, stamp, &edits).map(Some)
    })
    .await
}

/// Asks where to save, writes the document with its edits there, and opens it. Returns the
/// saved file name, or `None` when the user cancels.
#[tauri::command]
pub async fn save_copy(
    app: AppHandle,
    window: WebviewWindow,
    doc: u64,
    edits: Vec<Edit>,
) -> Result<Option<String>, String> {
    let (source, stamp) = app.state::<Viewer>().document_file(doc).ok_or(CLOSED)?;
    let mut dialog = app
        .dialog()
        .file()
        .add_filter("PDF documents", &["pdf"])
        .set_title("Save as")
        .set_file_name(save_name(&source, &edits))
        .set_parent(&window);
    if let Some(directory) = source.parent() {
        dialog = dialog.set_directory(directory);
    }
    run_save(move || {
        let Some(target) = dialog.blocking_save_file() else {
            return Ok(None);
        };
        let mut target = target.into_path().map_err(|error| error.to_string())?;
        if target.extension().is_none() {
            target.set_extension("pdf");
        }
        if !confirm_signatures(&window, &source, &edits)? {
            return Ok(None);
        }
        if ukurasa_core::same_file(&source, &target) {
            return save_in_place(&app, &source, stamp, &edits).map(Some);
        }
        ukurasa_core::save_as(&source, &target, &edits).map_err(|error| error.to_string())?;
        app.state::<Viewer>().open(&app, &target);
        Ok(Some(viewer::display_name(&target)))
    })
    .await
}

async fn run_save(
    task: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| "Saving failed unexpectedly.".to_owned())?
}

fn save_in_place(
    app: &AppHandle,
    path: &Path,
    stamp: Option<Stamp>,
    edits: &[Edit],
) -> Result<String, String> {
    let stamp = stamp
        .ok_or("Couldn't tell whether the file changed after it was opened. Save a copy instead")?;
    ukurasa_core::save(path, stamp, edits).map_err(|error| error.to_string())?;
    app.state::<Viewer>().open(app, path);
    Ok(viewer::display_name(path))
}

/// Returns whether to go on saving `edits` into `source`, asking first when they would
/// invalidate its signatures.
fn confirm_signatures(
    window: &WebviewWindow,
    source: &Path,
    edits: &[Edit],
) -> Result<bool, String> {
    if !ukurasa_core::breaks_signatures(source, edits).map_err(|error| error.to_string())? {
        return Ok(true);
    }
    Ok(window
        .dialog()
        .message("This PDF is digitally signed. Your edits change its page content, so its signatures will show as invalid after you save.")
        .title("Signed document")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Save anyway".into(),
            "Cancel".into(),
        ))
        .parent(window)
        .blocking_show())
}

pub fn has_unsaved(app: &AppHandle) -> bool {
    app.try_state::<Editor>()
        .is_some_and(|editor| editor.unsaved.load(Ordering::Relaxed))
}

/// Runs `then` now, or once the user agrees to discard unsaved edits.
pub fn confirm_discard(app: &AppHandle, then: impl FnOnce() + Send + 'static) {
    if !has_unsaved(app) {
        return then();
    }
    let mut dialog = app
        .dialog()
        .message("Your edits haven't been saved. If you continue, they'll be lost.")
        .title("Unsaved edits")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Discard edits".into(),
            "Keep editing".into(),
        ));
    if let Some(window) = app.get_webview_window(viewer::MAIN_WINDOW) {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |discard| {
        if discard {
            then();
        }
    });
}

/// Suggests a name for a saved copy that says what changed: `-signed`, `-edited`, or `-copy`.
fn save_name(source: &Path, edits: &[Edit]) -> String {
    let stem = source
        .file_stem()
        .map_or_else(|| "document".into(), |stem| stem.to_string_lossy());
    let base = ["-signed", "-edited", "-copy"]
        .iter()
        .find_map(|suffix| stem.strip_suffix(suffix))
        .unwrap_or(&stem);
    let suffix = if edits.iter().any(|edit| matches!(edit, Edit::Signature(_))) {
        "signed"
    } else if edits.is_empty() {
        "copy"
    } else {
        "edited"
    };
    let name = format!("{base}-{suffix}.pdf");
    if source
        .file_name()
        .is_some_and(|current| current == name.as_str())
    {
        format!("{base}-{suffix}-2.pdf")
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use ukurasa_edit::{LineEdit, Signature, TextBox};

    use super::*;

    #[test]
    fn groups_previewed_edits_by_page() {
        let line = |page, runs: &[usize]| {
            Edit::Line(LineEdit {
                page,
                runs: runs.to_vec(),
                original: "old".into(),
                spans: Vec::new(),
            })
        };
        let text = |page, x| {
            Edit::Text(TextBox {
                page,
                x,
                y: 0.0,
                lines: Vec::new(),
            })
        };
        let signature = Edit::Signature(Signature {
            page: 0,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            path: VectorPath {
                d: String::new(),
                width: 1.0,
                height: 1.0,
                fill: false,
                stroke_width: 1.0,
            },
            color: [0, 0, 0],
        });
        let pages = by_page(vec![
            text(2, 1.0),
            line(2, &[9]),
            signature,
            line(0, &[4]),
            text(2, 0.0),
            line(2, &[3, 5]),
        ]);
        assert_eq!(
            pages.into_iter().collect::<Vec<_>>(),
            [
                (0, vec![line(0, &[4])]),
                (
                    2,
                    vec![line(2, &[3, 5]), line(2, &[9]), text(2, 1.0), text(2, 0.0)]
                )
            ]
        );
    }

    #[test]
    fn names_saved_copies() {
        let stamp = [Edit::Signature(Signature {
            page: 0,
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            path: VectorPath {
                d: String::new(),
                width: 1.0,
                height: 1.0,
                fill: false,
                stroke_width: 1.0,
            },
            color: [0, 0, 0],
        })];
        let text = [Edit::Text(TextBox {
            page: 0,
            x: 0.0,
            y: 0.0,
            lines: Vec::new(),
        })];
        let name = |path, edits: &[Edit]| save_name(Path::new(path), edits);
        assert_eq!(name("/docs/lease.pdf", &stamp), "lease-signed.pdf");
        assert_eq!(name("/docs/lease.pdf", &text), "lease-edited.pdf");
        assert_eq!(name("/docs/lease.pdf", &[]), "lease-copy.pdf");
        assert_eq!(name("/docs/lease-edited.pdf", &stamp), "lease-signed.pdf");
        assert_eq!(name("/docs/lease-signed.pdf", &stamp), "lease-signed-2.pdf");
    }
}
