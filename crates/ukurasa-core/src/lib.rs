//! Document session, edit-operation log, journal, save pipeline, and capability policy.

use std::ffi::OsString;
use std::fs::{self, File, Metadata, Permissions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use ukurasa_edit::{Edit, EditError};

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("Save as needs a different file than the one it copies")]
    SameFile,
    #[error(
        "{} changed on disk after it was opened. Save a copy instead, or reopen it",
        name(path)
    )]
    Changed { path: PathBuf },
    #[error("{} is read-only. Save a copy instead", name(path))]
    ReadOnly { path: PathBuf },
    #[error("Couldn't read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("Couldn't write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Edit(#[from] EditError),
}

/// A file's size and modification time, to tell whether something else changed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    pub fn of(path: &Path) -> io::Result<Self> {
        fs::metadata(path).map(|metadata| Self::from(&metadata))
    }
}

impl From<&Metadata> for Stamp {
    fn from(metadata: &Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

/// Saves `edits` into `path` itself. The edits are appended as an incremental update, so the
/// original bytes stay intact. `opened` is the file's stamp from when it was opened; a file
/// changed since then is left alone. The file is replaced atomically, so a failed save leaves
/// it untouched.
pub fn save(path: &Path, opened: Stamp, edits: &[Edit]) -> Result<(), SaveError> {
    let metadata = fs::metadata(path).map_err(|error| read_error(path, error))?;
    if metadata.permissions().readonly() {
        return Err(SaveError::ReadOnly { path: path.into() });
    }
    if Stamp::from(&metadata) != opened {
        return Err(SaveError::Changed { path: path.into() });
    }
    if edits.is_empty() {
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|error| read_error(path, error))?;
    let output = ukurasa_edit::apply(&bytes, edits)?;
    write_atomic(path, &output, Some(metadata.permissions())).map_err(|error| SaveError::Write {
        path: path.into(),
        source: error,
    })
}

/// Saves `source` with `edits` applied as `target`, which must be a different file. The target
/// is replaced atomically, so a failed save leaves any existing file untouched.
pub fn save_as(source: &Path, target: &Path, edits: &[Edit]) -> Result<(), SaveError> {
    if same_file(source, target) {
        return Err(SaveError::SameFile);
    }
    let bytes = fs::read(source).map_err(|error| read_error(source, error))?;
    let output = if edits.is_empty() {
        bytes
    } else {
        ukurasa_edit::apply(&bytes, edits)?
    };
    write_atomic(target, &output, None).map_err(|error| SaveError::Write {
        path: target.into(),
        source: error,
    })
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// Whether saving `edits` into `path` invalidates signatures already in it; see
/// [`ukurasa_edit::breaks_signatures`].
pub fn breaks_signatures(path: &Path, edits: &[Edit]) -> Result<bool, SaveError> {
    if edits.is_empty() {
        return Ok(false);
    }
    let bytes = fs::read(path).map_err(|error| read_error(path, error))?;
    Ok(ukurasa_edit::breaks_signatures(&bytes, edits)?)
}

fn read_error(path: &Path, source: io::Error) -> SaveError {
    SaveError::Read {
        path: path.into(),
        source,
    }
}

fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Writes `bytes` to a temporary file beside `target`, reads them back to check them, and
/// renames the file over `target`.
fn write_atomic(target: &Path, bytes: &[u8], permissions: Option<Permissions>) -> io::Result<()> {
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
    let mut temp = OsString::from(".");
    temp.push(name);
    temp.push(format!(".{}.tmp", std::process::id()));
    let temp = target.with_file_name(temp);
    let result = File::create(&temp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| {
            if fs::read(&temp)? == bytes {
                Ok(())
            } else {
                Err(io::Error::other("the written file didn't match"))
            }
        })
        .and_then(|()| {
            permissions.map_or(Ok(()), |permissions| {
                fs::set_permissions(&temp, permissions)
            })
        })
        .and_then(|()| fs::rename(&temp, target));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
