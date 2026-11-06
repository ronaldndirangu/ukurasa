# Ukurasa

*Ukurasa* (oo-koo-RAH-sah) is Swahili for "page".

A fast, free, open source PDF reader and editor for Windows, macOS, and Linux, built with Rust and Tauri. Edit existing PDFs (add and replace text, fill forms, annotate, change words in place) and apply visual or certificate-based (PAdES) signatures.

Status: early development. See [docs/plan.md](docs/plan.md) for the architecture and roadmap, and [docs/infrastructure.md](docs/infrastructure.md) for how builds, CI, releases, and the runtime fit together.

## Install

Download the package for your platform from [Releases](https://github.com/ronaldndirangu/ukurasa/releases). While the repository is private, only its collaborators can see releases.

| Platform | Package | How to install |
|---|---|---|
| Windows 10 and 11 (x64) | `Ukurasa_<version>_x64-setup.exe` | Run it. Installs for the current user; no admin rights needed. |
| | `Ukurasa_<version>_x64_en-US.msi` | Run it. Installs for all users; needs admin rights. |
| macOS 11 or later | `Ukurasa_<version>_aarch64.dmg` (Apple silicon) or `Ukurasa_<version>_x64.dmg` (Intel) | Open the DMG and drag Ukurasa to Applications. |
| Debian, Ubuntu 22.04 or later | `Ukurasa_<version>_amd64.deb` | `sudo apt install ./Ukurasa_<version>_amd64.deb` |
| Fedora | `Ukurasa-<version>-1.x86_64.rpm` | `sudo dnf install ./Ukurasa-<version>-1.x86_64.rpm` |
| Other Linux | `Ukurasa_<version>_amd64.AppImage` | `chmod +x Ukurasa_<version>_amd64.AppImage`, then run it. Needs FUSE 2 (`libfuse2` on Ubuntu 22.04, `libfuse2t64` on 24.04). |

Each release lists the packages built for it. Build any missing package yourself with the steps in [Build a release](#build-a-release).

The installers are not code signed yet, so the first launch needs one extra step:

- **Windows**: SmartScreen shows "Windows protected your PC". Choose **More info**, then **Run anyway**.
- **macOS**: if macOS says Ukurasa "is damaged" or "can't be opened", run `xattr -dr com.apple.quarantine /Applications/Ukurasa.app` once, or open **System Settings > Privacy & Security** and choose **Open Anyway**.
- **Linux**: no extra step.

## Using Ukurasa

### Reading

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

## Build a release

A release build is optimized and packaged as installers. Build on each platform and architecture you want to ship: Tauri does not cross-compile installers, and `pnpm install` downloads the PDFium library for the machine it runs on.

### Prerequisites

On every platform, install [Rust](https://rustup.rs) (rustup reads the toolchain from `rust-toolchain.toml`), [Node.js](https://nodejs.org) 22 or later, and pnpm 10 (`npm install -g pnpm@10`). Then add the platform tools:

- **Windows 10 or 11**: [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload. WebView2 is already part of Windows 10 and 11.
- **macOS**: Xcode Command Line Tools (`xcode-select --install`).
- **Debian and Ubuntu 22.04 or later**:

  ```sh
  sudo apt update
  sudo apt install build-essential curl wget file libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  ```

- **Fedora**:

  ```sh
  sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel
  sudo dnf group install c-development
  ```

Other distributions are listed in the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

### Build

From the repository root:

```sh
cd app
pnpm install --frozen-lockfile   # also downloads PDFium into src-tauri/pdfium
pnpm tauri build
```

The first build takes several minutes, and on Windows it also downloads the NSIS and WiX installer tools. The packages land in `target/release/bundle/` at the repository root:

| Platform | Folders | Packages |
|---|---|---|
| Windows | `nsis/`, `msi/` | Setup program and MSI |
| macOS | `macos/`, `dmg/` | `Ukurasa.app` and DMG |
| Linux | `deb/`, `rpm/`, `appimage/` | deb, rpm, and AppImage |

To build only some packages, pass `--bundles`, for example `pnpm tauri build --bundles nsis` or `pnpm tauri build --bundles deb`.

The version comes from `version` in the root `Cargo.toml`. [docs/infrastructure.md](docs/infrastructure.md#releases) describes how to bump it and publish a release.

## Development

Development builds use the same prerequisites. `pnpm tauri dev` starts the app with hot reload of the frontend and rebuilds the Rust code when it changes.

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

The `ukurasa` command-line tool lives in `crates/ukurasa-cli`; run it with `cargo run -p ukurasa-cli -- --help`.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
