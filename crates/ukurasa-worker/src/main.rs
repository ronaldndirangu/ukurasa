//! Standalone PDFium worker binary; the desktop app runs the same code via `--ukurasa-worker`.

use std::process::ExitCode;

fn main() -> ExitCode {
    ukurasa_worker::run()
}
