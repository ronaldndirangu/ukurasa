//! Client side of the worker process: spawns it, exchanges frames, and restarts it after a crash.

use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;

use crate::{
    DocumentInfo, ErrorKind, Link, RenderError, Renderer, Request, Response, read_frame,
    write_frame,
};

#[derive(Debug, Clone)]
pub struct WorkerCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

/// A [`Renderer`] backed by a worker process, so a PDFium crash cannot take down the caller.
pub struct WorkerRenderer {
    command: WorkerCommand,
    process: Option<WorkerProcess>,
    /// The open document and its current revision, restored after a restart.
    path: Option<PathBuf>,
    revision: Option<Arc<[u8]>>,
}

impl WorkerRenderer {
    /// Creates the renderer; the worker process starts on the first request.
    pub fn new(command: WorkerCommand) -> Self {
        Self {
            command,
            process: None,
            path: None,
            revision: None,
        }
    }

    /// Sends a request, restarting the worker once if it has exited or broken the protocol.
    fn call(
        &mut self,
        request: &Request,
        payload: &[u8],
    ) -> Result<(Response, Vec<u8>), RenderError> {
        self.try_call(request, payload)
            .or_else(|_| self.try_call(request, payload))
            .map_err(|error| RenderError::Engine(error.to_string()))
    }

    fn try_call(&mut self, request: &Request, payload: &[u8]) -> io::Result<(Response, Vec<u8>)> {
        let mut process = match self.process.take() {
            Some(process) => process,
            None => self.spawn(!matches!(request, Request::Open { .. }))?,
        };
        let reply = process.call(request, payload)?;
        self.process = Some(process);
        Ok(reply)
    }

    fn spawn(&self, restore: bool) -> io::Result<WorkerProcess> {
        let mut process = WorkerProcess::spawn(&self.command)?;
        if let (true, Some(path)) = (restore, &self.path) {
            process.call(&Request::Open { path: path.clone() }, &[])?;
            if let Some(revision) = &self.revision {
                process.call(&Request::Revise, revision)?;
            }
        }
        Ok(process)
    }
}

impl Renderer for WorkerRenderer {
    fn open(&mut self, path: &Path) -> Result<DocumentInfo, RenderError> {
        let request = Request::Open {
            path: path.to_path_buf(),
        };
        match self.call(&request, &[])? {
            (Response::Opened(info), _) => {
                self.path = Some(path.to_path_buf());
                self.revision = None;
                Ok(info)
            }
            (other, _) => Err(unexpected(other)),
        }
    }

    fn render(&mut self, page: u32, width: u32) -> Result<Vec<u8>, RenderError> {
        match self.call(&Request::Render { page, width }, &[])? {
            (Response::Rendered { .. }, png) => Ok(png),
            (other, _) => Err(unexpected(other)),
        }
    }

    fn revise(&mut self, pdf: Option<Arc<[u8]>>) -> Result<(), RenderError> {
        let payload = pdf.as_deref().unwrap_or_default();
        match self.call(&Request::Revise, payload)? {
            (Response::Revised, _) => {
                self.revision = pdf;
                Ok(())
            }
            (other, _) => Err(unexpected(other)),
        }
    }

    fn links(&mut self, page: u32) -> Result<Vec<Link>, RenderError> {
        match self.call(&Request::Links { page }, &[])? {
            (Response::Links { links }, _) => Ok(links),
            (other, _) => Err(unexpected(other)),
        }
    }

    fn close(&mut self) -> Result<(), RenderError> {
        self.path = None;
        self.revision = None;
        if self.process.is_none() {
            return Ok(());
        }
        match self.call(&Request::Close, &[])? {
            (Response::Closed, _) => Ok(()),
            (other, _) => Err(unexpected(other)),
        }
    }
}

fn unexpected(response: Response) -> RenderError {
    match response {
        Response::Error { kind, message } => RenderError::Worker { kind, message },
        other => RenderError::Worker {
            kind: ErrorKind::Internal,
            message: format!("unexpected worker response {other:?}"),
        },
    }
}

struct WorkerProcess {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl WorkerProcess {
    fn spawn(command: &WorkerCommand) -> io::Result<Self> {
        let mut builder = Command::new(&command.program);
        builder
            .args(&command.args)
            .envs(command.env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            builder.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = builder.spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("worker pipes are unavailable"));
        };
        Ok(Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
        })
    }

    fn call(&mut self, request: &Request, payload: &[u8]) -> io::Result<(Response, Vec<u8>)> {
        write_frame(&mut self.stdin, request, payload)?;
        read_frame(&mut self.stdout)
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
