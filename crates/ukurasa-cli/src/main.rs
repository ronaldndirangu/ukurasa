//! The `ukurasa` command-line tool.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Command-line tools for Ukurasa, the open source PDF reader and editor.
#[derive(Debug, Parser)]
#[command(name = "ukurasa", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render PDF pages to images.
    Render {
        /// PDF file to render.
        input: PathBuf,
    },
    /// Apply a digital signature to a PDF.
    Sign {
        /// PDF file to sign.
        input: PathBuf,
    },
    /// Verify the digital signatures in a PDF.
    Verify {
        /// PDF file to verify.
        input: PathBuf,
    },
    /// Convert a Markdown document to PDF.
    #[command(name = "md2pdf")]
    Md2Pdf {
        /// Markdown file to convert.
        input: PathBuf,
    },
    /// Import a PDF as Markdown (best effort; layout changes).
    #[command(name = "pdf2md")]
    Pdf2Md {
        /// PDF file to import.
        input: PathBuf,
    },
    /// Check a PDF for problems that affect reading, editing, or signing.
    Check {
        /// PDF file to check.
        input: PathBuf,
    },
}

impl Command {
    fn name_and_input(&self) -> (&'static str, &Path) {
        match self {
            Self::Render { input } => ("render", input),
            Self::Sign { input } => ("sign", input),
            Self::Verify { input } => ("verify", input),
            Self::Md2Pdf { input } => ("md2pdf", input),
            Self::Pdf2Md { input } => ("pdf2md", input),
            Self::Check { input } => ("check", input),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (name, input) = cli.command.name_and_input();
    eprintln!(
        "ukurasa {name}: not implemented yet (input: {})",
        input.display()
    );
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_every_subcommand() {
        for name in ["render", "sign", "verify", "md2pdf", "pdf2md", "check"] {
            let cli = Cli::try_parse_from(["ukurasa", name, "input"])
                .unwrap_or_else(|err| panic!("failed to parse `{name}`: {err}"));
            assert_eq!(cli.command.name_and_input().0, name);
        }
    }
}
