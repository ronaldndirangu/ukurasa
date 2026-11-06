import { SvelteMap } from "svelte/reactivity";
import {
  documentFonts,
  previewEdits,
  textLines,
  type Color,
  type DocumentFont,
  type Edit,
  type LineEdit,
  type LineReport,
  type SignatureEdit,
  type Span,
  type Style,
  type TextEdit,
  type TextLine,
  type VectorPath,
} from "./api";
import { clamp } from "./layout.svelte";
import { isBlank, sameRows, sameSpans, styleOf } from "./spans";

export type Tool = "none" | "text" | "sign" | "line";

export interface Item {
  id: number;
  edit: Edit;
}

/** An item placed on the page as a box, as opposed to text edited in place. */
export type Placed = Item & { edit: TextEdit | SignatureEdit };
export const isPlaced = (item: Item): item is Placed => item.edit.kind !== "line";

/** A line's new text, drawn over the page until its image shows the change. */
export interface LinePatch {
  page: number;
  line: TextLine;
  spans: Span[];
}

export const lineKey = (page: number, runs: number[]) => `${page}:${runs.join(",")}`;

/** Undo and redo of an open text field. */
export interface FieldHistory {
  readonly canUndo: boolean;
  readonly canRedo: boolean;
  undo(): void;
  redo(): void;
}
/** Text reaching past the space it has by more than this many points gets a notice. */
const OVERFLOW_NOTICE = 0.5;

/** Must match the line spacing of saved text rows. */
export const LINE_HEIGHT = 1.2;
export const TEXT_SIZES = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 60, 72];
/** Added text takes this style where the page has no text to take it from. */
const TEXT_STYLE: Style = {
  font: "Helvetica",
  face: null,
  family: "sans",
  bold: false,
  italic: false,
  underline: false,
  size: 12,
  color: [0, 0, 0],
};
export const SIGNATURE_INKS: { name: string; color: Color }[] = [
  { name: "Black", color: [0, 0, 0] },
  { name: "Blue", color: [29, 63, 187] },
];

const MAX_HISTORY = 100;
const SAVED_KEY = "ukurasa.signatures";
const MAX_SAVED = 4;

export const css = ([r, g, b]: Color) => `rgb(${r}, ${g}, ${b})`;
export const sameColor = (a: Color, b: Color) => a.every((value, i) => value === b[i]);

/** Size of a newly placed signature in points: about 48 pt tall and at most 200 pt wide. */
export function signatureSize(path: VectorPath): { width: number; height: number } {
  const aspect = path.width / path.height;
  const height = Math.min(48, 200 / aspect);
  return { width: height * aspect, height };
}

let nextId = 1;

class Editor {
  /** The open document's id. */
  doc = $state<number | null>(null);
  items = $state.raw<Item[]>([]);
  selected = $state<number | null>(null);
  /** Text item whose content is being typed. */
  editing = $state<number | null>(null);
  tool = $state<Tool>("none");
  /** Signature waiting to be placed with the sign tool. */
  signature = $state.raw<{ path: VectorPath; color: Color } | null>(null);
  saving = $state(false);
  /** Counts how often each page's rendering changed, so its image can be requested again. */
  revisions = new SvelteMap<number, number>();
  /** A text preview is rendering. */
  previewing = $state(false);
  patches = $state.raw<LinePatch[]>([]);
  /** Added text the pages don't show yet, drawn over them until they do. */
  unpainted = $state.raw<TextEdit[]>([]);
  /** Added text being moved, drawn over its page instead of in it. */
  #held: number | null = null;
  /** The revision of each page the viewer shows. */
  #painted = new Map<number, number>();
  /** Lines edited so far, so undo and redo can patch them too. */
  #edited = new Map<string, { page: number; line: TextLine }>();
  /** How far each edited line's text on the pages reaches past its original end, in points. */
  #reach = $state.raw(new Map<string, number>());
  /** Something to tell the user about the latest text edit; the app clears it once shown. */
  notice = $state<string | null>(null);
  error = $state<string | null>(null);
  #undo = $state.raw<Item[][]>([]);
  #redo = $state.raw<Item[][]>([]);
  /** The item being edited was just created, so its typing belongs to the same undo step. */
  #fresh: number | null = null;
  #lines = new Map<number, Promise<TextLine[]>>();
  #fonts: Promise<DocumentFont[]> | null = null;
  /** Line and text items the pages currently show. */
  #previewed: Item[] = [];
  /** The edits most recently sent for preview. */
  #sent = "[]";
  /** The line edited last, whose report becomes a notice. */
  #latest: string | null = null;
  /** The latest line or text commit and the redo steps it cleared, so a failed preview can take it back. */
  #lastCommit: { items: Item[]; redo: Item[][] } | null = null;
  /** The line field open in Edit text, by the token `openLine` gave it. */
  #line = $state<number | null>(null);
  /** The open line's text or style differs from what it showed when opened. */
  #lineChanged = $state(false);
  #lineTokens = 0;
  /** Line tokens up to this one belong to a document that has closed. */
  #lineFloor = 0;

  /** History of the text field being typed in, which Undo and Redo act on while it is open. */
  field = $state.raw<FieldHistory | null>(null);

  get canUndo() {
    if (this.field) return this.field.canUndo;
    return this.#undo.length > 0 && this.editing === null && this.#line === null;
  }

  get canRedo() {
    if (this.field) return this.field.canRedo;
    return this.#redo.length > 0 && this.editing === null && this.#line === null;
  }

  get dirty() {
    return this.items.length > 0 || this.#lineChanged;
  }

  get(id: number | null): Item | undefined {
    return this.items.find((item) => item.id === id);
  }

  reset(doc: number | null) {
    this.doc = doc;
    this.items = [];
    this.#undo = [];
    this.#redo = [];
    this.selected = null;
    this.editing = null;
    this.tool = "none";
    this.signature = null;
    this.#fresh = null;
    this.revisions.clear();
    this.notice = null;
    this.error = null;
    this.#lines.clear();
    this.#fonts = null;
    this.#previewed = [];
    this.#sent = "[]";
    this.#latest = null;
    this.#lastCommit = null;
    this.#line = null;
    this.#lineChanged = false;
    this.#lineFloor = this.#lineTokens;
    this.patches = [];
    this.unpainted = [];
    this.#held = null;
    this.#painted.clear();
    this.#edited.clear();
    this.#reach = new Map();
  }

  arm(tool: Tool) {
    this.tool = this.tool === tool ? "none" : tool;
    if (this.tool !== "sign") this.signature = null;
    if (this.tool !== "none") this.selected = null;
  }

  /** Arms the sign tool with a signature to place. */
  prepareSignature(path: VectorPath, color: Color) {
    this.signature = { path, color };
    this.tool = "sign";
    this.selected = null;
  }

  placeSignature(edit: Edit) {
    this.select(this.#add(edit));
    this.tool = "none";
    this.signature = null;
  }

  /** Adds a text box at `at`, typed in the style of the page's nearest line, and starts typing in it. */
  async startText(page: number, at: { x: number; y: number }, size: { width: number; height: number }) {
    this.tool = "none";
    const doc = this.doc;
    const lines = await this.lines(page).catch(() => []);
    if (doc !== this.doc || this.editing !== null) return;
    const style = this.#nearestStyle(page, lines, at);
    const row = style.size * LINE_HEIGHT;
    const id = nextId++;
    const edit: TextEdit = {
      kind: "text",
      page,
      x: clamp(at.x, 0, size.width - style.size),
      y: clamp(at.y - row / 2, 0, size.height - row),
      lines: [[{ ...style, text: "" }]],
    };
    this.selected = id;
    this.editing = id;
    this.#fresh = id;
    this.#commit([...this.items, { id, edit }]);
  }

  editText(id: number) {
    if (this.editing === id || this.get(id)?.edit.kind !== "text") return;
    this.selected = id;
    this.editing = id;
    this.tool = "none";
    // The box shows the text while it's typed in, so the pages leave it out.
    this.#preview();
  }

  /** Ends typing in a text box: blank text removes it, and the pages draw the rest. */
  finishText(id: number, rows: Span[][]) {
    const fresh = this.#fresh === id;
    if (fresh) this.#fresh = null;
    if (this.editing === id) this.editing = null;
    const item = this.get(id);
    if (item?.edit.kind !== "text") return;
    if (fresh && isBlank(rows)) {
      const commit = this.#lastCommit;
      if (commit?.items === this.items) this.#redo = commit.redo;
      this.#lastCommit = null;
      this.#setItems(this.items.filter((other) => other.id !== id));
      this.#undo = this.#undo.slice(0, -1);
      if (this.selected === id) this.selected = null;
    } else if (fresh) {
      const next = this.#replace(id, { lines: rows });
      if (this.#lastCommit?.items === this.items) this.#lastCommit = { ...this.#lastCommit, items: next };
      this.#setItems(next);
      this.#preview();
    } else if (isBlank(rows)) {
      this.remove(id);
    } else if (!sameRows(rows, item.edit.lines)) {
      this.update(id, { lines: rows });
    } else {
      this.#unpaint(item.edit);
      this.#preview();
    }
  }

  /** Leaves added text out of the pages while it moves, or puts it back with `null`. */
  hold(id: number | null) {
    const held = this.get(this.#held);
    this.#held = id;
    if (held?.edit.kind === "text") this.#unpaint(held.edit);
    this.#preview();
  }

  select(id: number | null) {
    if (this.selected !== id) this.selected = id;
  }

  /** The editable lines of a page's original text. */
  lines(page: number): Promise<TextLine[]> {
    let lines = this.#lines.get(page);
    if (!lines) {
      lines = this.doc === null ? Promise.resolve([]) : textLines(this.doc, page);
      this.#lines.set(page, lines);
    }
    return lines;
  }

  /** The fonts the document uses for text. Failures resolve to none. */
  fonts(): Promise<DocumentFont[]> {
    if (!this.#fonts) {
      this.#fonts = this.doc === null ? Promise.resolve([]) : documentFonts(this.doc).catch(() => []);
    }
    return this.#fonts;
  }

  /** Stops text editing with a message, such as when the PDF doesn't allow it. */
  linesFailed(message: string) {
    this.error = message;
    if (this.tool === "line") this.tool = "none";
  }

  /** The styled text `line` shows with edits applied. */
  lineSpans(page: number, line: TextLine, items = this.items): Span[] {
    return this.#lineItem(page, line.runs, items)?.edit.spans ?? line.spans;
  }

  isEdited(page: number, line: TextLine): boolean {
    return this.#lineItem(page, line.runs) !== undefined;
  }

  /** The width `line` takes on the pages, which an edit that lengthened it extends. */
  lineWidth(page: number, line: TextLine): number {
    return line.width + (this.#reach.get(lineKey(page, line.runs)) ?? 0);
  }

  /** Opens a line field in Edit text and returns the token that closes it. */
  openLine(): number {
    this.#line = ++this.#lineTokens;
    this.#lineChanged = false;
    return this.#line;
  }

  /** Records whether the open line field changed, which makes the document count as unsaved. */
  changeLine(token: number, changed: boolean) {
    if (token === this.#line) this.#lineChanged = changed;
  }

  /** Closes a line field. False when its document has closed since, so its text must be dropped. */
  closeLine(token: number): boolean {
    if (token === this.#line) {
      this.#line = null;
      this.#lineChanged = false;
    }
    return token > this.#lineFloor;
  }

  /** Replaces the styled text of `line`; its original text removes the edit. */
  editLine(page: number, line: TextLine, spans: Span[]) {
    const existing = this.#lineItem(page, line.runs);
    if (sameSpans(spans, existing?.edit.spans ?? line.spans)) return;
    const key = lineKey(page, line.runs);
    this.#latest = key;
    this.#edited.set(key, { page, line });
    this.#patch([{ page, line, spans }]);
    if (sameSpans(spans, line.spans)) {
      this.#commit(this.items.filter((item) => item !== existing));
    } else {
      const edit: LineEdit = { kind: "line", page, runs: line.runs, original: line.text, spans };
      this.#commit(
        existing
          ? this.items.map((item) => (item === existing ? { id: item.id, edit } : item))
          : [...this.items, { id: nextId++, edit }],
      );
    }
  }

  update(id: number, patch: Partial<Edit>) {
    const next = this.#replace(id, patch);
    if (this.#fresh === id) this.#setItems(next);
    else this.#commit(next);
  }

  remove(id: number) {
    this.#commit(this.items.filter((item) => item.id !== id));
    if (this.selected === id) this.selected = null;
  }

  undo() {
    if (this.field) return this.field.undo();
    const previous = this.#undo.at(-1);
    if (!previous || !this.canUndo) return;
    this.#undo = this.#undo.slice(0, -1);
    this.#redo = [...this.#redo, this.items];
    this.#restore(previous);
  }

  redo() {
    if (this.field) return this.field.redo();
    const next = this.#redo.at(-1);
    if (!next || !this.canRedo) return;
    this.#redo = this.#redo.slice(0, -1);
    this.#undo = [...this.#undo, this.items];
    this.#restore(next);
  }

  /** Disarms the active tool, or clears the selection. Returns whether anything changed. */
  escape(): boolean {
    if (this.tool !== "none") {
      this.arm("none");
      return true;
    }
    if (this.selected !== null) {
      this.selected = null;
      return true;
    }
    return false;
  }

  #add(edit: Edit): number {
    const id = nextId++;
    this.#commit([...this.items, { id, edit }]);
    return id;
  }

  #replace(id: number, patch: Partial<Edit>): Item[] {
    return this.items.map((item) => (item.id === id ? { id, edit: { ...item.edit, ...patch } as Edit } : item));
  }

  #commit(items: Item[]) {
    const redo = this.#redo;
    this.#undo = [...this.#undo.slice(1 - MAX_HISTORY), this.items];
    this.#redo = [];
    this.#setItems(items);
    this.#lastCommit = { items, redo };
    this.#preview();
  }

  #restore(items: Item[]) {
    const before = this.items;
    this.#setItems(items);
    if (!this.get(this.selected)) this.selected = null;
    this.#patch(
      [...this.#edited.values()]
        .map(({ page, line }) => ({ page, line, spans: this.lineSpans(page, line) }))
        .filter(({ page, line, spans }) => !sameSpans(spans, this.lineSpans(page, line, before))),
    );
    this.#preview();
  }

  /** Sets the items, drawing text they add or change over the pages until the pages show it. */
  #setItems(items: Item[]) {
    const known = new Set(this.items.map((item) => item.edit));
    this.items = items;
    for (const { edit } of items) if (edit.kind === "text" && !known.has(edit)) this.#unpaint(edit);
  }

  #unpaint(edit: TextEdit) {
    if (!this.unpainted.includes(edit)) this.unpainted = [...this.unpainted, edit];
  }

  /** The item is drawn into the pages, as opposed to over them or not at all. */
  #drawn(item: Item): item is Item & { edit: LineEdit | TextEdit } {
    const { kind } = item.edit;
    return kind === "line" || (kind === "text" && item.id !== this.editing && item.id !== this.#held);
  }

  /** The style of the line nearest `at`, so added text matches the text around it. */
  #nearestStyle(page: number, lines: TextLine[], at: { x: number; y: number }): Style {
    let nearest: Span | undefined;
    let best = Infinity;
    for (const line of lines) {
      const dx = Math.max(line.x - at.x, 0, at.x - line.x - line.width);
      const dy = Math.max(line.y - at.y, 0, at.y - line.y - line.height);
      const distance = Math.hypot(dx, dy);
      const spans = this.lineSpans(page, line);
      const span = spans.find((candidate) => candidate.text.trim()) ?? spans[0];
      if (span && distance < best) {
        best = distance;
        nearest = span;
      }
    }
    return nearest ? { ...styleOf(nearest), underline: false } : TEXT_STYLE;
  }

  /** Records which revision of `page` the viewer shows, retiring patches it no longer needs. */
  painted(page: number, revision: number) {
    this.#painted.set(page, revision);
    this.#prune();
  }

  #patch(patches: LinePatch[]) {
    if (patches.length === 0) return;
    const key = ({ page, line }: LinePatch) => lineKey(page, line.runs);
    const keys = new Set(patches.map(key));
    this.patches = [...this.patches.filter((patch) => !keys.has(key(patch))), ...patches];
  }

  /** Drops patches and text overlays on pages that show their latest rendering. */
  #prune() {
    if (this.previewing) return;
    const stale = (page: number) => (this.#painted.get(page) ?? 0) < (this.revisions.get(page) ?? 0);
    const patches = this.patches.filter(({ page }) => stale(page));
    if (patches.length < this.patches.length) this.patches = patches;
    const unpainted = this.unpainted.filter(({ page }) => stale(page));
    if (unpainted.length < this.unpainted.length) this.unpainted = unpainted;
  }

  #lineItem(page: number, runs: number[], items = this.items): (Item & { edit: LineEdit }) | undefined {
    const key = lineKey(page, runs);
    return items.find(
      (item): item is Item & { edit: LineEdit } =>
        item.edit.kind === "line" && lineKey(item.edit.page, item.edit.runs) === key,
    );
  }

  /** Renders the pages with the current line and text edits, one preview at a time, skipping stale ones. */
  async #preview() {
    if (this.previewing) return;
    this.previewing = true;
    try {
      for (;;) {
        const doc = this.doc;
        const state = this.items;
        const items = state.filter((item) => this.#drawn(item));
        const edits = items.map((item) => item.edit);
        const sent = JSON.stringify(edits);
        if (doc === null || sent === this.#sent) break;
        this.#sent = sent;
        try {
          const preview = await previewEdits(doc, edits);
          if (doc !== this.doc) continue;
          this.#previewed = items;
          if (this.#lastCommit?.items === state) this.#lastCommit = null;
          for (const page of preview.pages) this.revisions.set(page, (this.revisions.get(page) ?? 0) + 1);
          this.#reach = new Map(
            preview.lines.map((line) => [lineKey(line.page, line.runs), Math.max(0, line.overflow)]),
          );
          this.#report(preview.lines);
        } catch (failure) {
          if (doc !== this.doc) continue;
          this.error = String(failure);
          this.#latest = null;
          // The pages still show the text from before; the failed edit can't be saved either.
          this.#sent = JSON.stringify(this.#previewed.map((item) => item.edit));
          this.#discard(state);
        }
      }
    } finally {
      this.previewing = false;
      this.#prune();
    }
  }

  /** Takes back edits whose preview failed, from the pages and the history alike. */
  #discard(failed: Item[]) {
    const commit = this.#lastCommit;
    this.#lastCommit = null;
    const previous = this.#undo.at(-1);
    if (previous && commit?.items === failed && this.items === failed) {
      this.#undo = this.#undo.slice(0, -1);
      this.#redo = commit.redo;
      this.#restore(previous);
    } else {
      const kept = this.items.filter((item) => !this.#drawn(item));
      const ids = new Set(kept.map((item) => item.id));
      this.#restore([...kept, ...this.#previewed.filter((item) => !ids.has(item.id))]);
    }
  }

  #report(reports: LineReport[]) {
    const latest = this.#latest;
    this.#latest = null;
    const report = reports.find((line) => lineKey(line.page, line.runs) === latest);
    if (!report || latest === null) return;
    if (report.fallback) {
      this.notice = `This font doesn't have ${report.fallback}, so a standard font shows it instead`;
    } else if (report.overflow > OVERFLOW_NOTICE) {
      this.notice = "The new text is longer than the original line";
    }
  }
}

export const editor = new Editor();

function isPath(value: unknown): value is VectorPath {
  const path = value as VectorPath;
  return (
    typeof path?.d === "string" &&
    typeof path.width === "number" &&
    typeof path.height === "number" &&
    typeof path.fill === "boolean" &&
    typeof path.strokeWidth === "number"
  );
}

export function savedSignatures(): VectorPath[] {
  try {
    const list: unknown = JSON.parse(localStorage.getItem(SAVED_KEY) ?? "[]");
    return Array.isArray(list) ? list.filter(isPath).slice(0, MAX_SAVED) : [];
  } catch {
    return [];
  }
}

export function rememberSignature(path: VectorPath) {
  const list = [path, ...savedSignatures().filter((saved) => saved.d !== path.d)];
  localStorage.setItem(SAVED_KEY, JSON.stringify(list.slice(0, MAX_SAVED)));
}

export function forgetSignature(path: VectorPath) {
  const list = savedSignatures().filter((saved) => saved.d !== path.d);
  localStorage.setItem(SAVED_KEY, JSON.stringify(list));
}
