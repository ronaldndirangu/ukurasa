# Ukurasa

*Ukurasa* (oo-koo-RAH-sah) is Swahili for "page".

A fast, free, open source PDF reader and editor for Windows, macOS, and Linux, built with Rust and Tauri. Edit existing PDFs (add and replace text, fill forms, annotate, change words in place) and apply visual or certificate-based (PAdES) signatures.

Status: early development. See [docs/plan.md](docs/plan.md) for the architecture and roadmap.

## Development

Prerequisites: [Rust](https://rustup.rs) (the stable toolchain is pinned in `rust-toolchain.toml`), Node.js 22, pnpm 10, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```sh
# Rust workspace: crates/ and app/src-tauri
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Desktop app (Svelte 5 frontend in app/frontend)
cd app
pnpm install    # also downloads the PDFium library; rerun it with `pnpm pdfium`
pnpm tauri dev
```

PDFium renders pages in a separate worker process (the app executable started with `--ukurasa-worker`), so a malformed PDF cannot crash the window.

### Using the reader

- Open a PDF with **Open** (Ctrl+O), by dropping it on the window, from the recent files list, or by passing its path on the command line.
- The sidebar shows page thumbnails and the document outline. Toggle it with the button at the left of the title bar.
- Zoom with Ctrl+= and Ctrl+-, or Ctrl+scroll. Ctrl+0 sets 100%; the zoom menu also offers fit width, fit page, and presets.
- Type a page number in the page controls at the bottom right and press Enter to jump to it.

### Editing

- **Edit text** changes existing text in its own font, one line at a time. Click a line and type; select words to change their font, size, bold, italic, underline, or color. Press Enter to finish or Escape to cancel.
- When the font lacks a character you typed, Helvetica shows that text instead and the editor says so.
- **Add text** places new text anywhere on the page in the style of the text nearby; Enter starts a new line. **Sign** places a drawn or typed signature.
- Undo and redo with Ctrl+Z and Ctrl+Y.
- **Save** (Ctrl+S) updates the file in place, and a Save button appears beside the file name while edits are unsaved. Ukurasa appends edits as an incremental update when the file allows it, so the original bytes stay intact. **Save as** (Ctrl+Shift+S) writes a copy.
- Editing the text of a signed PDF invalidates its signatures, so Ukurasa asks before saving.

The `ukurasa` command-line tool lives in `crates/ukurasa-cli`; run it with `cargo run -p ukurasa-cli -- --help`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
