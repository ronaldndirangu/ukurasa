//! The File menu in the macOS menu bar, which mirrors the one in the title bar. Its items ask
//! the window to act, so they take the same path as the title bar and keyboard shortcuts.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tauri::menu::{Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::recent::RecentFile;
use crate::viewer::MAIN_WINDOW;

const EVENT: &str = "menu-action";
const OPEN: &str = "open";
const SAVE: &str = "save";
const SAVE_AS: &str = "save-as";
const RECENT: &str = "recent:";

pub struct FileMenu {
    recent: Submenu<Wry>,
    save: MenuItem<Wry>,
    save_as: MenuItem<Wry>,
    /// The paths the Open Recent submenu lists.
    shown: Mutex<Vec<PathBuf>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
enum Action {
    Open,
    OpenRecent { path: PathBuf },
    Save,
    SaveAs,
}

/// Adds Open, Open Recent, Save, and Save As to the File menu of Tauri's default menu bar.
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    let file = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(submenu) if submenu.text().is_ok_and(|text| text == "File") => {
            Some(submenu)
        }
        _ => None,
    });
    let Some(file) = file else {
        return Ok(());
    };
    let open = MenuItem::with_id(app, OPEN, "Open…", true, Some("CmdOrCtrl+O"))?;
    let recent = Submenu::new(app, "Open Recent", false)?;
    let save = MenuItem::with_id(app, SAVE, "Save", false, Some("CmdOrCtrl+S"))?;
    let save_as = MenuItem::with_id(app, SAVE_AS, "Save As…", false, Some("CmdOrCtrl+Shift+S"))?;
    file.insert_items(
        &[
            &open,
            &recent,
            &PredefinedMenuItem::separator(app)?,
            &save,
            &save_as,
            &PredefinedMenuItem::separator(app)?,
        ],
        0,
    )?;
    app.set_menu(menu)?;
    app.manage(FileMenu {
        recent,
        save,
        save_as,
        shown: Mutex::default(),
    });
    Ok(())
}

pub fn handle(app: &AppHandle, event: MenuEvent) {
    if let Some(action) = action(event.id().as_ref()) {
        let _ = app.emit_to(MAIN_WINDOW, EVENT, action);
    }
}

fn action(id: &str) -> Option<Action> {
    match id {
        OPEN => Some(Action::Open),
        SAVE => Some(Action::Save),
        SAVE_AS => Some(Action::SaveAs),
        _ => id
            .strip_prefix(RECENT)
            .map(|path| Action::OpenRecent { path: path.into() }),
    }
}

pub fn show_unsaved(app: &AppHandle, unsaved: bool) {
    if let Some(menu) = app.try_state::<FileMenu>() {
        let _ = menu.save.set_enabled(unsaved);
    }
}

pub fn show_session(app: &AppHandle, open: bool, recent: &[RecentFile]) {
    let Some(menu) = app.try_state::<FileMenu>() else {
        return;
    };
    let _ = menu.save_as.set_enabled(open);
    let paths: Vec<PathBuf> = recent.iter().map(|file| file.path.clone()).collect();
    let mut shown = menu.shown.lock().unwrap_or_else(PoisonError::into_inner);
    if *shown == paths {
        return;
    }
    while let Ok(Some(_)) = menu.recent.remove_at(0) {}
    for file in recent {
        let Some(path) = file.path.to_str() else {
            continue;
        };
        // A single `&` would mark a mnemonic and disappear from the name.
        let name = file.name.replace('&', "&&");
        if let Ok(item) =
            MenuItem::with_id(app, format!("{RECENT}{path}"), name, true, None::<&str>)
        {
            let _ = menu.recent.append(&item);
        }
    }
    let _ = menu.recent.set_enabled(!recent.is_empty());
    *shown = paths;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_items_name_their_actions() {
        assert_eq!(action(OPEN), Some(Action::Open));
        assert_eq!(action(SAVE), Some(Action::Save));
        assert_eq!(action(SAVE_AS), Some(Action::SaveAs));
        assert_eq!(
            action("recent:/docs/a:b.pdf"),
            Some(Action::OpenRecent {
                path: "/docs/a:b.pdf".into()
            })
        );
        assert_eq!(action("close"), None);
    }

    #[test]
    fn actions_reach_the_window_as_tagged_json() {
        let json = |action| serde_json::to_value(action).unwrap();
        assert_eq!(
            json(Action::SaveAs),
            serde_json::json!({ "action": "save-as" })
        );
        assert_eq!(
            json(Action::OpenRecent {
                path: "/a.pdf".into()
            }),
            serde_json::json!({ "action": "open-recent", "path": "/a.pdf" })
        );
    }
}
