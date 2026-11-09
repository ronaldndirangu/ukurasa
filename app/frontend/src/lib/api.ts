import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface PageSize {
  width: number;
  height: number;
}

export interface OutlineItem {
  title: string;
  /** Zero-based target page, or null when the entry does not point into this document. */
  page: number | null;
  children?: OutlineItem[];
}

export interface DocumentView {
  id: number;
  name: string;
  pages: PageSize[];
  outline: OutlineItem[];
}

export interface RecentFile {
  path: string;
  name: string;
  /** Milliseconds since the Unix epoch. */
  opened: number;
  missing: boolean;
}

export interface Session {
  revision: number;
  loading: string | null;
  document: DocumentView | null;
  error: string | null;
  recent: RecentFile[];
}

export type FitMode = "width" | "page";
export type Fit = FitMode | null;

export const PLATFORM: "windows" | "macos" | "linux" = navigator.userAgent.includes("Windows")
  ? "windows"
  : navigator.userAgent.includes("Mac")
    ? "macos"
    : "linux";
export const MOD = PLATFORM === "macos" ? "⌘" : "Ctrl+";

export const getSession = () => invoke<Session>("session");
export const openDialog = () => invoke<void>("open_dialog");
export const openPath = (path: string) => invoke<void>("open_path", { path });
export const forgetRecent = (path: string) => invoke<void>("forget_recent", { path });
export const onSession = (handler: (session: Session) => void): Promise<UnlistenFn> =>
  listen<Session>("session-changed", (event) => handler(event.payload));

export type Color = [number, number, number];

/** An outline in its own coordinate space, with y growing downward. */
export interface VectorPath {
  d: string;
  width: number;
  height: number;
  fill: boolean;
  strokeWidth: number;
}

/**
 * Rows of styled text added to the page. Positions are in points from the top-left corner of the
 * page as displayed; `x` and `y` are the top-left corner of the first row.
 */
export interface TextEdit {
  kind: "text";
  page: number;
  x: number;
  y: number;
  /** Each row is as high as its largest style, empty spans included. */
  lines: Span[][];
}

export interface SignatureEdit {
  kind: "signature";
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  path: VectorPath;
  color: Color;
}

export type Family = "serif" | "sans" | "mono";

/** Text in one style. Sizes are display points. */
export interface Span {
  text: string;
  /** Family name of a font in the document, or of a standard font. */
  font: string;
  /** The font the text was read in, kept while the span's font and style still match it. */
  face: string | null;
  /** Generic family, for a stand-in font while editing. */
  family: Family;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  size: number;
  color: Color;
}

export type Style = Omit<Span, "text">;

/** Replaces the text of an existing line with styled text. */
export interface LineEdit {
  kind: "line";
  page: number;
  /** Identifies the line by its runs, as read by `textLines`. */
  runs: number[];
  original: string;
  spans: Span[];
}


/** Moves, resizes, or with null `bounds` removes a picture found by `pageImages`. */
export interface ImageEdit {
  kind: "image";
  page: number;
  /** Identifies the picture by its drawing operator, as read by `pageImages`. */
  op: number;
  /** Where the picture showed when it was read. */
  original: Rect;
  bounds: Rect | null;
}

export type Edit = TextEdit | SignatureEdit | LineEdit | ImageEdit;

/** A picture on a page that can be moved, resized, or removed, positioned like `TextEdit`. */
export interface Picture {
  op: number;
  /** The part of the picture that shows. */
  bounds: Rect;
  /** The area the picture can show in. */
  limits: Rect;
}

/** A line of the page's original text, positioned like `TextEdit`. */
export interface TextLine {
  runs: number[];
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  baseline: number;
  spans: Span[];
}

export interface LineReport {
  page: number;
  runs: number[];
  /** Characters the chosen font can't show, drawn in a standard font instead. */
  fallback: string;
  /** How far the new text reaches past the end of the line it replaces, in points. */
  overflow: number;
}

/** A font family in the document that lines can be edited in. */
export interface DocumentFont {
  name: string;
  family: Family;
}

export interface Preview {
  /** Pages whose rendering changed. */
  pages: number[];
  lines: LineReport[];
}

/**
 * Saves `edits` over the open file and reopens it; resolves to the file's name, or null when
 * cancelled.
 */
export const saveDocument = (doc: number, edits: Edit[]) =>
  invoke<string | null>("save_document", { doc, edits });
/** Saves a copy with `edits` applied and opens it; resolves to the saved name, or null when cancelled. */
export const saveCopy = (doc: number, edits: Edit[]) => invoke<string | null>("save_copy", { doc, edits });
export const textLines = (doc: number, page: number) => invoke<TextLine[]>("text_lines", { doc, page });
export const pageImages = (doc: number, page: number) => invoke<Picture[]>("page_images", { doc, page });
export const documentFonts = (doc: number) => invoke<DocumentFont[]>("document_fonts", { doc });
/** Renders the open document with `edits` applied, replacing the previous preview. */
export const previewEdits = (doc: number, edits: (LineEdit | TextEdit | ImageEdit)[]) =>
  invoke<Preview>("preview_edits", { doc, edits });
export const setUnsaved = (unsaved: boolean) => invoke<void>("set_unsaved", { unsaved });
export const signaturePath = (text: string) => invoke<VectorPath>("signature_path", { text });

type LinkTarget =
  /** `top` is the destination's distance in points from the top of the page, when known. */
  | { kind: "page"; page: number; top: number | null }
  | { kind: "uri"; uri: string };

/** A clickable area, positioned like `TextEdit`. */
export interface Link {
  x: number;
  y: number;
  width: number;
  height: number;
  target: LinkTarget;
}

let linkCache = { doc: -1, pages: new Map<number, Promise<Link[]>>() };

/** The links on a page, fetched once per document. Failures resolve to no links. */
export function pageLinks(doc: number, page: number): Promise<Link[]> {
  if (linkCache.doc !== doc) linkCache = { doc, pages: new Map() };
  const cache = linkCache.pages;
  let links = cache.get(page);
  if (!links) {
    links = invoke<Link[]>("page_links", { doc, page }).catch(() => {
      cache.delete(page);
      return [];
    });
    cache.set(page, links);
  }
  return links;
}

/** Opens a web or email link in the default app. */
export const openLink = (url: string) => invoke<void>("open_link", { url });

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Tells Windows where the maximize button is, so hovering it shows Snap Layouts. */
export const setMaximizeButton = (rect: Rect | null) => invoke<void>("set_maximize_button", { rect });
export type ButtonState = "hover" | "press" | "leave";
/** The pointer over the maximize button, which Windows reports in place of the page. */
export const onMaximizeButton = (handler: (state: ButtonState) => void): Promise<UnlistenFn> =>
  listen<ButtonState>("maximize-button", (event) => handler(event.payload));

export interface PageImage {
  id: number;
  url: string;
}

let nextId = 1;

/** Requests a page rendered at `width` device pixels; lower priorities render first. */
export function pageImage(doc: number, page: number, width: number, priority: number): PageImage {
  const id = nextId++;
  return { id, url: convertFileSrc(`${doc}-${page}-${width}-${priority}-${id}`, "pdf") };
}

let cancelled: number[] = [];

export function cancelRender(id: number) {
  if (cancelled.length === 0) {
    queueMicrotask(() => {
      const ids = cancelled;
      cancelled = [];
      invoke("cancel_renders", { ids }).catch(() => {});
    });
  }
  cancelled.push(id);
}
