//! Renderer trait, worker-process protocol, render scheduler, and page cache.

mod protocol;
mod service;
mod worker;

use std::fmt;
use std::path::Path;
use std::sync::Arc;

pub use protocol::{
    DocumentInfo, ErrorKind, Link, LinkTarget, OutlineItem, PageSize, Request, Response,
    read_frame, write_frame,
};
pub use service::{OpenedDocument, RenderRequest, RenderResult, RenderService};
pub use worker::{WorkerCommand, WorkerRenderer};

/// Opens one document at a time and renders its pages to PNG.
pub trait Renderer: Send {
    /// Opens `path`, replacing the current document only on success.
    fn open(&mut self, path: &Path) -> Result<DocumentInfo, RenderError>;
    /// Renders a zero-based page at the given pixel width and returns PNG bytes.
    fn render(&mut self, page: u32, width: u32) -> Result<Vec<u8>, RenderError>;
    /// Renders from `pdf`, an edited copy of the open document, until the next open or
    /// revision. `None` returns to the file.
    fn revise(&mut self, pdf: Option<Arc<[u8]>>) -> Result<(), RenderError>;
    /// Lists the links on a zero-based page of the document being rendered.
    fn links(&mut self, page: u32) -> Result<Vec<Link>, RenderError>;
    fn close(&mut self) -> Result<(), RenderError>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    /// The worker reported a failure for this request.
    Worker {
        kind: ErrorKind,
        message: String,
    },
    /// The worker process could not be started or stopped responding.
    Engine(String),
    Cancelled,
    /// The request referred to a document that is no longer open.
    Stale,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Worker { kind, message } => match kind {
                ErrorKind::Password => {
                    f.write_str("The file is password protected, which is not supported yet.")
                }
                ErrorKind::Format => f.write_str("The file is not a valid PDF or is damaged."),
                ErrorKind::Io => write!(f, "The file could not be read ({message})."),
                ErrorKind::Unsupported => {
                    write!(f, "The file uses an unsupported feature ({message}).")
                }
                ErrorKind::Page => write!(f, "The page could not be loaded ({message})."),
                ErrorKind::Internal => write!(f, "The PDF engine failed ({message})."),
            },
            Self::Engine(message) => write!(f, "The PDF engine stopped unexpectedly ({message})."),
            Self::Cancelled => f.write_str("The request was cancelled."),
            Self::Stale => f.write_str("The document is no longer open."),
        }
    }
}

impl std::error::Error for RenderError {}
