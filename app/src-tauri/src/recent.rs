//! Recently opened documents, kept as JSON in the app data directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "recent.json";
const LIMIT: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFile {
    pub path: PathBuf,
    pub name: String,
    /// Milliseconds since the Unix epoch.
    pub opened: u64,
    #[serde(default, skip_deserializing)]
    pub missing: bool,
}

/// Moves `path` to the front of `list`, keeping at most `LIMIT` entries.
pub fn touch(list: &mut Vec<RecentFile>, path: &Path, name: &str, opened: u64) {
    if path.to_str().is_none() {
        return;
    }
    forget(list, path);
    list.insert(
        0,
        RecentFile {
            path: path.to_owned(),
            name: name.to_owned(),
            opened,
            missing: false,
        },
    );
    list.truncate(LIMIT);
}

pub fn forget(list: &mut Vec<RecentFile>, path: &Path) {
    list.retain(|file| !same_path(&file.path, path));
}

pub fn refresh(list: &mut [RecentFile]) {
    for file in list {
        file.missing = !file.path.is_file();
    }
}

/// Reads the list at `path`, treating a missing or unreadable file as empty.
pub fn load(path: &Path) -> Vec<RecentFile> {
    let mut list: Vec<RecentFile> = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    list.truncate(LIMIT);
    refresh(&mut list);
    list
}

/// Writes through a temporary file so a crash cannot leave a truncated list behind.
pub fn save(path: &Path, list: &[RecentFile]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(list)?)?;
    fs::rename(&temp, path)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[RecentFile]) -> Vec<&str> {
        list.iter().map(|file| file.name.as_str()).collect()
    }

    #[test]
    fn touching_moves_files_to_the_front_and_caps_the_list() {
        let mut list = Vec::new();
        for index in 0..20 {
            touch(
                &mut list,
                Path::new(&format!("/docs/{index}.pdf")),
                &index.to_string(),
                index,
            );
        }
        assert_eq!(list.len(), LIMIT);
        assert_eq!(list[0].name, "19");

        touch(&mut list, Path::new("/docs/12.pdf"), "12", 99);
        assert_eq!(list.len(), LIMIT);
        assert_eq!(names(&list[..3]), ["12", "19", "18"]);
        assert_eq!(list[0].opened, 99);
    }

    #[test]
    fn forgetting_removes_only_the_named_file() {
        let mut list = Vec::new();
        touch(&mut list, Path::new("/a.pdf"), "a", 1);
        touch(&mut list, Path::new("/b.pdf"), "b", 2);
        forget(&mut list, Path::new("/a.pdf"));
        assert_eq!(names(&list), ["b"]);
    }

    #[test]
    fn saved_lists_load_back_and_flag_missing_files() {
        let dir = std::env::temp_dir().join(format!("ukurasa-recent-{}", std::process::id()));
        let existing = dir.join("here.pdf");
        let file = dir.join(FILE_NAME);
        fs::create_dir_all(&dir).unwrap();
        fs::write(&existing, b"%PDF").unwrap();

        let mut list = Vec::new();
        touch(&mut list, &dir.join("gone.pdf"), "gone", 1);
        touch(&mut list, &existing, "here", 2);
        save(&file, &list).unwrap();
        let loaded = load(&file);

        assert_eq!(names(&loaded), ["here", "gone"]);
        assert_eq!(
            loaded.iter().map(|file| file.missing).collect::<Vec<_>>(),
            [false, true]
        );
        assert!(load(&dir.join("absent.json")).is_empty());
        fs::write(&file, b"not json").unwrap();
        assert!(load(&file).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
