# Ukurasa: plan for an open source PDF reader and editor (Rust + Tauri)

**Name:** *ukurasa* (oo-koo-RAH-sah) is Swahili for "page". Every PDF is a set of pages, and the app exists to read, edit, and sign them.

## Goal

Build a free, open source desktop app for Windows, macOS, and Linux (all three from the first release) that:

1. Opens and renders PDFs very fast, including 1,000+ page files.
2. Makes editing existing PDFs easy: add and replace text, fill forms, annotate, rearrange pages, and edit existing text lines and paragraphs in their original fonts.
3. Signs any PDF, visually and with certificate-based digital signatures (PAdES), and verifies existing signatures.
4. Optional, after the core ships: write documents in Markdown and export them as PDFs.

Non-goals for v1: OCR, moving content outside an edited paragraph, collaboration, cloud sync, mobile.

## Key product decision: how existing PDFs are edited

Editing existing PDFs and signing them is the core of the app. A PDF stores positioned glyphs with no paragraphs, so edits work at three levels, and the UI shows which level applies.

| Level | Phase | What the user does | How it is stored | Layout |
|---|---|---|---|---|
| Overlay edits | 2 | Add text boxes, whiteout and replace, fill forms, annotate, sign | Annotations and form fields with generated appearances; incremental update | Preserved |
| In-place line edits | 3 | Click an existing text line and change its words | Rewritten page content stream; incremental update | Preserved; changes stay within the line, with no reflow |
| In-place paragraph edits | 3b | Click a paragraph and edit it like a word processor text box | Rewritten page content stream; incremental update | Lines rewrap within the paragraph's width; content outside the paragraph never moves |

- Whiteout hides the original text without removing it from the file. The UI says so and points to in-place editing or redaction when the old text must be gone.
- Markdown mode (optional, Phase 6) is for writing new documents. It never edits an existing PDF in place.

## Architecture

```
app/ (Tauri v2)
  frontend/ (Svelte 5 + TypeScript)
    viewer: virtualized page list, canvas tiles, DOM text layer, SVG overlay for pending edits
    edit-ui: text boxes, whiteout, form fields, in-place line and paragraph editor
    sign-ui: signature pad, certificate picker, verification panel
    md-editor (optional): CodeMirror 6 with live Typst preview
  src-tauri/ (thin command layer, no business logic)
crates/
  ukurasa-core    document session, edit-op log, journal, save pipeline, capability policy
  ukurasa-render  Renderer trait, worker-process protocol, scheduler, tile cache
  ukurasa-worker  separate binary hosting PDFium (render, text, search index)
  ukurasa-geom    canonical page geometry and transforms (boxes, /Rotate, /UserUnit)
  ukurasa-edit    lopdf writer: annotations, AcroForm model, appearance streams, page ops, content-stream text edits
  ukurasa-sign    PAdES signing, verification, signature policy engine
  ukurasa-md      (optional) comrak (GFM) to Typst, Typst World, PDF export, source package
  ukurasa-import  (optional) best-effort PDF to Markdown from PDFium text and glyph geometry
  ukurasa-cli     `ukurasa` binary: render, sign, verify, md2pdf, pdf2md, check (tests and power users)
```

The repository contains only the crates in use. `ukurasa-sign`, `ukurasa-md`, `ukurasa-import`, and `ukurasa-cli` are created when the phase that needs them starts.

### Process model and rendering

- PDFium is C++ parsing untrusted input and is not thread-safe. It never runs in the main app process. `ukurasa-worker` processes host PDFium; a crash kills a worker, not the app, and the scheduler restarts it.
- Worker pool: one or more render workers (visible pages and prefetch) plus a separate index worker (text extraction, search index, thumbnails), so indexing never blocks scrolling. Pool size scales with CPU cores and memory budget.
- Workers return pixels through shared memory; the main process serves tiles to the webview through a Tauri custom URI scheme (`pdf://tile/...`). No JSON for binary data.
- Scheduler: priority queue (visible tiles, then prefetch, then thumbnails), cancellation of stale requests, per-page render timeout so a pathological page cannot stall the queue.
- Tiles (for example 512x512) at the current zoom; a low-resolution full-page image appears instantly while sharp tiles load. LRU tile cache keyed by (doc revision, page, zoom bucket, tile) with a fixed memory budget.
- `Renderer` trait allows a future in-process `hayro` backend (pure Rust, thread-safe) as an experiment.
- OS-level sandbox hardening of workers (Windows restricted token or AppContainer, macOS sandbox profile, Linux seccomp/Landlock) follows process isolation in Phase 5.

### Text layer and accessibility

- Each page has a DOM text layer of positioned, transparent spans built from PDFium glyph geometry (same approach as pdf.js). It provides selection, copy, find highlighting, and screen-reader text.
- Keyboard navigation across pages, outline, and annotations from Phase 1. ARIA labels on all controls.

### Geometry contract

- All edit operations store positions in canonical PDF user space for the target page.
- `ukurasa-geom` computes the page-to-view transform from MediaBox, CropBox (including inherited and non-zero origins), `/Rotate`, and `/UserUnit`. The frontend overlay uses only transforms produced by this crate.
- Test fixtures cover rotated, cropped, offset-origin, and UserUnit pages for every annotation type.

### Editing and save pipeline

- Source of truth: the original file (memory-mapped, never edited in place) plus an ordered list of edit operations.
- Pending operations render in the frontend SVG overlay, so edits feel instant without PDFium reloads.
- Every operation persists to a sidecar journal immediately. After a crash, the app offers to restore unsaved edits. Undo/redo works on the journal.
- Appearance streams: `ukurasa-edit` generates an `/AP` for every annotation, widget, and visual signature it writes. Text appearances are laid out by krilla or Typst (shaping, font fallback, CJK, RTL, subsetting), produced as a one-page PDF, and imported as a Form XObject. Other viewers never need to synthesize appearances.
- In-place line edits (approach validated by the feasibility spike; see Phase 0):
  - The worker reports the text objects, bounds, and fonts of the selected line from PDFium. The k-th PDFium text object on a page is the k-th non-empty show operator (`Tj`, `TJ`, `'`, `"`) in the page content.
  - `ukurasa-edit` rewrites only those show operators with lopdf. It re-encodes the new string through the font's encoding or `ToUnicode` CMap, keeps all surrounding text state (`Tc`, `Tw`, `Tz`, `Tm`), and saves the page content as an incremental update.
  - PDFium content regeneration (`FPDFText_SetText` plus `FPDFPage_GenerateContent`) is not used. It re-serializes the whole page and, in the spike, dropped word spacing across a page, rotated a form line, and shifted vector paths.
  - A trailing `TJ` adjustment, computed from `/Widths` or CID `/W`, keeps the advance of the edited run unchanged, so text after it on the same line stays in place. Standard 14 fonts without `/Widths` use bundled AFM metrics.
  - Glyph coverage: an embedded font accepts only characters already shown with that font in the document; a non-embedded font accepts any character in its encoding. Otherwise the edit falls back to whiteout and replace with the closest bundled font, and the UI says so.
  - Fonts without a space glyph (common in TeX output) get spaces as `TJ` adjustments.
  - A line in the editor groups adjacent show operators with the same font and baseline, because generators split lines into several objects, sometimes mid-word.
  - Longer text may run into neighbouring content on the same line; the editor shows the overflow and offers to shrink the text to fit.
  - Edited text stays selectable and searchable as the new string, because it only uses codes with existing `ToUnicode` mappings. A covering `/ActualText` span is updated to match.
  - Not yet covered: text inside form XObjects (often shared between pages), Type 3 fonts, vertical writing, and CID fonts with CMaps other than Identity-H.
- In-place paragraph edits build on line edits:
  - Detection: lines with the same font, size, and color whose baselines step down by a steady leading (1.0 to 1.8 font sizes) and whose left edges align form a paragraph block. Lines that share a baseline with text in another font (a bold word, a right-aligned date) end the block and stay line-editable.
  - Hard breaks: a line that ends well short of the block's right edge, or a line that starts with a bullet or list number, keeps a line break in the editable text, so list items and short paragraphs inside a block stay separate.
  - Alignment: left, centered, right, or justified, read from the line edges. The first-line indent is kept.
  - Reflow: the new text wraps greedily within the block width using the font's own glyph widths, character and word spacing, and the block's word-space adjustment. Justified lines spread the remaining width across their spaces.
  - Placement: each new line reuses the next original line slot, rewritten as in a line edit with a leading `TJ` shift for alignment and indent. Unused slots are emptied. Extra lines continue below the block at its leading in a new text object, appended to the page content after wrapping the original content in `q`/`Q`.
  - Content below the block never moves. When the paragraph grows past its original height, the editor reports how many lines were added so the user can shorten the text.
  - Single-line blocks don't wrap and behave like line edits.
  - Not yet covered: mixed fonts within a paragraph (bold or italic words), hyphenation, and paragraphs that continue across columns or pages.
- Capability policy classifies each input file and operation before save:
  - Incremental-safe: annotations, form fill, and signatures on a well-formed file.
  - Content-changing: in-place line and paragraph edits. Saved incrementally, but they invalidate existing signatures, and the UI says so before save.
  - Full-rewrite-required: page delete, reorder, insert, broken or repaired xref, and redaction. These invalidate existing signatures, and the UI says so before save.
  - Read-only: unsupported encryption or files that fail validation.
- Save writes to a temp file, validates it (reopen in a worker, `qpdf --check` in CI and debug builds, object-level comparison), then atomically replaces the target. The app detects external modification of the original before saving.
  - Status: Save replaces the opened file and Save as writes a copy. Edits are appended as an incremental update; the output is reparsed and its page count checked, the temp file is read back and compared, and a rename replaces the target. A file changed or made read-only since it was opened is left alone, and a save that would invalidate signatures asks first. Remaining: worker reopen, `qpdf --check`, and object-level comparison.
- After save, workers reload the new file and the doc revision increments, which invalidates the tile cache.

### Forms

- `ukurasa-edit` contains an AcroForm model from Phase 0: field hierarchy, widgets, appearances, signature fields, and FieldMDP locks. Signing and verification depend on it.
- Form-filling UI ships in Phase 2 with signing, since filling and signing a form is the most common editing task. Flattening ships in Phase 4.

### Encryption

- Open password-protected PDFs (user password prompt) in Phase 1, honoring permission flags.
- Incremental saves to encrypted files must use the existing security handler. Supported revisions are decided in Phase 0. Unsupported encrypted files open read-only with a clear banner.

### Markdown pipeline (optional, Phase 6)

- Parse with `comrak` (GFM: tables, task lists, footnotes, strikethrough).
- Convert the AST to Typst markup with editable Typst templates (default, report, letter).
- Embed Typst as a library: implement `World` with bundled OFL fonts and a resolver restricted to the package directory.
- Live preview renders Typst output to SVG or PNG directly; export uses `typst-pdf`.
- Source package (embedded associated file): manifest version, Markdown source, assets, template, font list, Typst and converter versions, SHA-256 per file, MIME types, and safe relative paths. Extraction rejects path traversal, enforces size limits (decompression-bomb guard), and runs in a temp directory.
- The package is embedded before any signature is applied. Editing source of a signed PDF produces a new unsigned PDF.
- Reopening an app-created PDF offers "Edit source" with a lossless round-trip. Other PDFs get a best-effort "Import as Markdown" that creates a new document and warns that layout will change.

### Signing and verification

| Level | Type | Phase |
|---|---|---|
| Visual | Drawn, typed, or image signature with generated appearance | 2 |
| PAdES B-B | Certificate signature from PKCS#12 file | 2 |
| PAdES B-T | Adds RFC 3161 timestamp from a user-configured TSA (opt-in network call) | 2 |
| Advanced | OS cert stores (Windows CNG, macOS Keychain), PKCS#11 tokens, B-LT/B-LTA | 5 |

- Implementation choice is a Phase 0 go/no-go on `pdf_signing`. A custom writer is a large project: stable ByteRange offsets, CMS size estimation with retry for long chains and TSA tokens, signature field and widget, appearance, subfilters. If `pdf_signing` fails, use OpenSSL CMS (`openssl` crate, vendored) for the cryptography and keep only PDF structure work in-house, or ship certificate signing after Phase 2 rather than ship it broken.
- Verification covers digest integrity, certificate chain, trust store (OS roots plus user-added roots), revocation where available, and signing time.
- Signature policy engine: parses DocMDP (`P=1/2/3`) and FieldMDP for every existing signature and classifies each pending operation as permitted, invalidating, or forbidden before save.
- Verification panel shows, per signature: signer, time, integrity, trust, and one of "unchanged since signing", "changed in permitted ways", or "changed in ways not permitted".
- Signing is always the final step; the UI warns before any edit to a signed file.

## Phases and exit criteria

### Phase 0: Feasibility spikes
- Tauri shell renders a 1,000-page PDF through a PDFium worker process, shared memory, and the custom URI scheme.
- Benchmark tile transport (raw RGBA vs WebP) on Windows (WebView2), macOS (WKWebView), Linux (WebKitGTK) while indexing runs concurrently.
- Full sign-and-verify lifecycle with `pdf_signing`: PKCS#12, TSA timestamp with large response, two sequential signatures, an annotation added after signing. Validate in Acrobat Reader, EU DSS, pyHanko, and poppler `pdfsig`.
- Appearance generation: FreeText and typed signature in Latin, CJK, and Arabic, rendered identically in PDFium, Acrobat, pdf.js, and macOS Preview, on rotated and cropped pages.
- Encrypted PDFs: open with password, read permissions, incremental save of an annotation, and signing.
- In-place line edit: change words on text lines in 20 real-world PDFs (subset TrueType, CFF, Type 1, Type 3, and CID fonts; `TJ` kerning arrays; rotated text), save incrementally, and check the result in Acrobat, Chrome, and Preview. Record how often the embedded font covers the new characters.
  - Done for 6 PDFs (Chromium print with CID TrueType subsets, IRS W-9 and 1040, two LaTeX papers with Type 1 subsets, and the PDF 32000 spec), 90 edits on 3 pages each. All 71 accepted edits extracted as the new text with the old text gone, and changed 0 pixels outside the edited line in both PDFium and MuPDF. The coverage rule refused the other 19; the PDFium regeneration run showed that such characters render as boxes or vanish. Reordering existing characters was always accepted; a deliberately unusual test string was accepted 26 of 45 times.
  - Incremental growth is the size of the recompressed page content (1 to 12 KB per edited page). An edit plus a verification reload takes 4 to 60 ms on small files and about 0.7 s on the 22 MB spec.
  - Remaining: Type 3 fonts, rotated text, form XObjects, encrypted files, and checks in Acrobat and Preview.
- Printing: print a 100-page document at 300 DPI on each OS (webview print of rasterized pages vs native path).
- CI matrix and release pipeline produce signed installers for Windows, macOS, and Linux from the spike build.
- Exit: go/no-go recorded for `pdf_signing`, tile transport, printing path, supported encryption revisions, and in-place edit font coverage; signed spike installers run on all three platforms.

### Phase 1: Fast, accessible reader (first public release)
- Worker-process rendering, scheduler, tile cache, crash recovery of workers.
- Open, scroll, zoom, fit modes, rotate view, dark-mode page inversion.
- Text layer: selection, copy, find, screen-reader text; keyboard navigation.
- Full-text search with background indexing, outline, thumbnails, links.
- Password-protected PDFs (read), printing, recent files, file association, single-instance.
- Exit: performance targets met at p95 on reference machines; zero crashes of the main process across the test corpus; screen reader reads page text on Windows (NVDA), macOS (VoiceOver), and Linux (Orca); signed installers published for all three platforms.

### Phase 2: Edit and sign
- Text boxes, whiteout and replace, highlight, underline, strikeout, ink, sticky note, shapes, image stamp, all with generated appearance streams.
- Form fill: text, checkbox, radio, combo, and list fields.
- Journal, autosave recovery, atomic save, capability policy, external-change detection.
- Visual signature: draw, type, upload image; saved signatures stored locally.
- PAdES B-B and B-T with PKCS#12; verification panel; signature policy engine.
- Exit: signed output validates in Acrobat, EU DSS, and pyHanko; annotations and filled forms render identically in Acrobat, Chrome, Firefox, and Preview; IME text entry works for CJK in text boxes and form fields.

### Phase 3: In-place line editing
- Click an existing text line, change its words, keep its font, size, and position.
- Glyph coverage check with fallback to whiteout and replace; overflow indicator with shrink to fit.
- Signature warning before saving content changes to a signed file.
- Exit: edits on the Phase 0 corpus render identically in Acrobat, Chrome, and Preview; edited text is selectable and searchable as the new string; missing glyphs always trigger the fallback instead of producing boxes or wrong characters.

### Phase 3b: In-place paragraph editing
- Click a paragraph and edit it in a multi-line box that wraps like the page; the text keeps its font, size, color, alignment, and leading.
- Paragraph detection with hard breaks for list items and short lines; single-line blocks behave like line edits.
- Reflow within the paragraph width, including justified text; lines added below the paragraph are reported so the user can shorten the text.
- The page shows the new text immediately while the exact rendering loads.
- Status: detection, reflow, and the editor UI are implemented; grouping is checked on one LaTeX paper. Remaining: the corpus run and checks in Acrobat, Chrome, and Preview.
- Exit: paragraph edits on 20 text-heavy corpus PDFs (papers, letters, CVs, contracts) render identically in Acrobat, Chrome, and Preview; pixels outside the paragraph's area don't change; the new text extracts in reading order.

### Phase 4: Pages and forms
- Merge, split, reorder, rotate, delete, insert blank pages, extract pages (full rewrite, with signature warning).
- Form flattening.
- Exit: page operations preserve annotations, links, and outline; flattened forms display correctly in Acrobat and Chrome.

### Phase 5: Advanced
- OS certificate stores and PKCS#11 tokens; PAdES B-LT and B-LTA.
- OS-level sandboxing of worker processes.
- Redaction (true content removal), optional OCR via Tesseract.

### Phase 6 (optional): Markdown mode
- Split-view editor with live preview, templates, images, tables, footnotes, Typst math.
- Export with source package; "Edit source" on reopen.
- Best-effort "Import as Markdown" (headings from font size, lists, paragraphs, simple tables, images), with an optional hook for external converters such as Marker or Docling.
- Exit: round-trip of app-created PDFs is byte-identical in source; import produces readable Markdown on 50 sample text-heavy PDFs.

## Performance targets (p95, warm start, reference mid-range laptop per OS)

| Metric | Target |
|---|---|
| App launch to window | < 400 ms |
| Open 10 MB PDF to first page visible | < 250 ms |
| Scroll 1,000-page PDF while indexing | 60 fps with placeholders; sharp tiles within 150 ms of stopping |
| Memory, 1,000-page PDF open (all processes) | < 500 MB, enforced by per-component budgets |
| Search 1,000 pages after indexing | < 300 ms |
| Markdown preview after keystroke (Phase 6) | < 100 ms for a 20-page document |

Benchmarks run on dedicated reference machines, not shared CI runners. CI tracks relative regressions only.

## Testing and quality

- Corpus: pdf.js test PDFs, PDFium and qpdf test files, plus real-world PDFs (forms, scans, encrypted, broken xref, linearized, object streams, rotated and cropped pages).
- Visual regression: `ukurasa render` output compared against stored images.
- Cross-viewer checks for generated annotations: PDFium, pdf.js, Acrobat, Preview.
- Fuzzing: `cargo-fuzz` on our parsing paths (lopdf usage, content-stream rewriting, signature parsing, and the optional import heuristics and source-package extraction).
- Save validation: reopen, `qpdf --check`, and differential comparison after every save in tests.
- In-place edit tests: render before and after in PDFium and MuPDF and require zero changed pixels outside the edited line or paragraph area, plus text extraction of the new string.
- Signature interoperability: pyHanko and `pdfsig` in CI; EU DSS and Acrobat per release.
- i18n and IME tests (CJK, RTL) before any text-entry feature ships.

## Distribution and licensing

- License: MIT OR Apache-2.0. Core dependencies are permissive: PDFium (BSD-3/Apache-2.0), pdfium-render, lopdf, Typst and krilla (Apache-2.0/MIT), Tauri, comrak, OpenSSL 3 (Apache-2.0). Bundled fonts must be OFL or similar. Avoid MuPDF (AGPL).
- PDFium binaries from `bblanchon/pdfium-binaries`, bundled per platform and updated promptly for security fixes.
- Installers via Tauri bundler (see platform matrix); auto-update via `tauri-plugin-updater` with signed update manifests.

### Platform matrix

| Platform | Architectures | Webview | Packages | Code signing |
|---|---|---|---|---|
| Windows 10 and 11 | x64, arm64 | WebView2 (Evergreen, bootstrapped by installer) | MSI, NSIS, winget | Authenticode via SignPath Foundation (free for open source) or Azure Trusted Signing |
| macOS 11+ | Apple Silicon, Intel (universal binary) | WKWebView | DMG, Homebrew cask | Developer ID and notarization (Apple Developer Program, USD 99 per year) |
| Linux (glibc 2.35+) | x64, arm64 | WebKitGTK 4.1 | AppImage, deb, rpm, Flathub | Signed repositories; Flathub builds |

- PDFium binaries exist for every target above; the worker binary ships per architecture.
- CI: GitHub Actions matrix (Windows, macOS arm64 and x64, Ubuntu 22.04 for the oldest supported glibc) builds, tests, and packages every commit. A release pipeline producing signed installers for all three platforms is a Phase 0 deliverable, so every phase ships everywhere.
- Platform-specific work tracked per phase: default PDF handler registration, printing path, HiDPI and fractional scaling, Wayland and X11 on Linux, screen readers (NVDA, VoiceOver, Orca), and OS certificate stores (Phase 5).
- Webview hardening: strict CSP, no remote content, minimal Tauri capability allowlist, no shell or filesystem access from the frontend beyond scoped commands.
- Privacy: no telemetry by default; network calls limited to opt-in TSA timestamping and update checks.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| PDFium memory-safety bug on untrusted input | App compromise or crash | Worker processes from Phase 1; OS sandbox in Phase 5; prompt PDFium updates |
| Rendering contention | Misses fps targets | Separate render and index workers, cancellation, per-page timeouts, tiles |
| `pdf_signing` immature | Invalid signatures | Phase 0 lifecycle go/no-go; OpenSSL CMS fallback; delay rather than ship broken |
| Edits silently break signatures | User trust lost | Signature policy engine and explicit warnings before save |
| PDFium and lopdf interpret a file differently | Corrupted output | Capability policy, read-only for unsupported files, save validation |
| Annotations render differently across viewers | Edits look wrong elsewhere | Generated appearance streams, cross-viewer tests |
| In-place edit needs glyphs missing from a subset font | Boxes or wrong characters in edited text | Accept only characters already shown with the font; fallback to whiteout and replace |
| Content edit damages the rest of the page | Broken layout outside the edit | Rewrite only the edited show operators, never regenerate the page; tests require zero pixel change outside the edited line or paragraph |
| Paragraph detection groups the wrong lines (lists, tables, columns) | An edit scrambles the layout | Hard breaks at short lines and bullets; one font per block; instant preview and undo before save |
| Whiteout leaves the original text in the file | Hidden text leaks | UI states that whiteout only hides; in-place edits and redaction remove text |
| Webview transport slow on WebKitGTK | Poor Linux experience | Phase 0 benchmark; native wgpu surface as fallback |
| Crash during edit or save | Lost work or corrupt file | Journal, autosave, atomic save |
| PDF to Markdown import quality | User disappointment | Optional feature, labelled as import, external converter hook |
| Scope creep toward full Acrobat | Never ships | Phase gates; Phase 1 ships as a reader; paragraph edits reflow only within the paragraph and never move other content |

## Open questions

1. Frontend framework (Svelte 5 proposed; React has a larger ecosystem).
2. Solo project or recruiting contributors early (affects docs and CI investment in Phase 0).
3. Register `ukurasa.app`, `ukurasa.dev`, or `ukurasa.io` (all appeared unregistered on 2026-11-06; `ukurasa.com` is taken), and reserve the `ukurasa` name on crates.io and GitHub. Note: Kurasa (mykurasa.com) is an established Kenyan school-software brand with a similar name, so check for a trademark conflict before the first release.
