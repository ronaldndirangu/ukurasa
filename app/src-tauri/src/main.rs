// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chrome;
mod editor;
mod recent;
mod viewer;

use std::env;
use std::process::ExitCode;

use tauri::{DragDropEvent, Manager, WindowEvent};

fn main() -> ExitCode {
    if env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == ukurasa_worker::WORKER_FLAG)
    {
        return ukurasa_worker::run();
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol(viewer::SCHEME, viewer::serve_page)
        .invoke_handler(tauri::generate_handler![
            viewer::session,
            viewer::open_dialog,
            viewer::open_path,
            viewer::forget_recent,
            viewer::cancel_renders,
            viewer::page_links,
            viewer::open_link,
            editor::save_document,
            editor::save_copy,
            editor::set_unsaved,
            editor::signature_path,
            editor::text_lines,
            editor::page_images,
            editor::document_fonts,
            editor::preview_edits,
            chrome::set_maximize_button,
        ])
        .on_window_event(|window, event| match event {
            WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                viewer::open_first_pdf(window.app_handle(), paths);
            }
            WindowEvent::ScaleFactorChanged { .. } => chrome::rescale(window),
            WindowEvent::CloseRequested { api, .. } if editor::has_unsaved(window.app_handle()) => {
                api.prevent_close();
                let target = window.clone();
                editor::confirm_discard(window.app_handle(), move || {
                    let _ = target.destroy();
                });
            }
            _ => {}
        })
        .setup(|app| {
            app.manage(viewer::Viewer::new(app.handle())?);
            app.manage(editor::Editor::default());
            if let Some(window) = app.get_webview_window(viewer::MAIN_WINDOW) {
                chrome::attach(&window);
            }
            if let Some(path) = viewer::launch_path() {
                viewer::open_in_background(app.handle(), path);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Ukurasa");
    ExitCode::SUCCESS
}
