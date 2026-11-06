<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { MOD, type DocumentFont, type Span, type Style } from "./api";
  import { LINE_HEIGHT, TEXT_SIZES, editor, type FieldHistory } from "./edits.svelte";
  import Icon from "./Icon.svelte";
  import { clamp } from "./layout.svelte";
  import { STANDARD_FONTS, fontFamily, fontLabel, fromHex, hex, spanCss } from "./lineStyle";
  import { roving } from "./roving";
  import { charsOfRows, rowsOf, styleAt, styleOf, type Char } from "./spans";

  interface Props {
    /** The text the field opens with. */
    rows: Span[][];
    /** Style for typing where the text has none. */
    base: Style;
    /** CSS pixels per point. */
    scale: number;
    /** Enter starts a new row, and rows are as high as their text, like the saved text. */
    multiline?: boolean;
    /** The field hides what is under it. */
    cover?: boolean;
    /** Inline CSS for the field, such as the line box it replaces. */
    fieldStyle?: string;
    /** The format bar goes below the field. */
    below?: boolean;
    /** Client point to put the caret at; it goes at the end otherwise. */
    caret?: { x: number; y: number } | null;
    onchange?: (rows: Span[][]) => void;
    /** Called once with the final text; `cancelled` when Escape closed the field. */
    onclose: (rows: Span[][], cancelled: boolean) => void;
  }

  let {
    rows: opened,
    base,
    scale,
    multiline = false,
    cover = false,
    fieldStyle = "",
    below = false,
    caret = null,
    onchange,
    onclose,
  }: Props = $props();

  const MAX_HISTORY = 100;
  /** Space kept between the format bar and the field, and between the bar and the view's edges. */
  const BAR_GAP_PX = 8;

  interface Snapshot {
    chars: Char[];
    start: number;
    end: number;
  }

  let fonts = $state.raw<DocumentFont[]>([]);
  let chars = $state.raw<Char[]>(untrack(() => charsOfRows(opened, base)));
  /** The selection in the field, as offsets into `chars`. */
  let start = $state(0);
  let end = $state(0);
  /** Style for text typed at the caret, set by formatting with nothing selected. */
  let pending = $state.raw<Style | null>(null);
  let field = $state<HTMLDivElement>();
  let root = $state<HTMLDivElement>();
  let bar = $state<HTMLDivElement>();
  /** The bar's left edge from the root's, once measured. */
  let barLeft = $state<number | null>(null);
  let barBelow = $state(untrack(() => below));
  /** The field's center when it opened, from the root's left edge, so the bar stays put while typing. */
  let anchor: number | null = null;
  /** Styles of the field's span elements, by their `data-i`, and of its empty rows, by `data-row`. */
  let painted: Style[] = [];
  let marks: Style[] = [];
  let history = $state.raw<Snapshot[]>([]);
  let future = $state.raw<Snapshot[]>([]);
  /** The kind of the last change, so typing and deleting merge into one undo step. */
  let lastKind: string | null = null;
  let composing = false;
  let closed = false;

  const rows = $derived(rowsOf(chars, base));
  const leading = $derived(multiline ? LINE_HEIGHT : "inherit");
  const current = $derived.by(() => {
    const selected = chars.slice(start, end).filter((char) => char.c !== "\n");
    const picked =
      start === end || selected.length === 0
        ? [pending ?? styleAt(chars, start, base)]
        : selected.map((char) => char.style);
    const same = <T,>(get: (style: Style) => T): T | null =>
      picked.every((style) => get(style) === get(picked[0])) ? get(picked[0]) : null;
    return {
      font: same((style) => style.font),
      size: same((style) => style.size),
      bold: picked.every((style) => style.bold),
      italic: picked.every((style) => style.italic),
      underline: picked.every((style) => style.underline),
      color: picked[0].color,
    };
  });
  const documentNames = $derived([
    ...new Set([...fonts.map((font) => font.name), ...opened.flat().map((span) => span.font), base.font]),
  ]);
  const standardNames = $derived(STANDARD_FONTS.filter((font) => !documentNames.includes(font.name)));
  /** Readable font names, with the exact name added where two would read the same. */
  const labels = $derived.by(() => {
    const names = [...documentNames, ...standardNames.map((font) => font.name)];
    const plain = names.map((name) => fontLabel(name));
    return new Map(
      names.map((name, i) => [
        name,
        plain.indexOf(plain[i]) !== plain.lastIndexOf(plain[i]) && plain[i] !== name
          ? `${plain[i]} (${name})`
          : plain[i],
      ]),
    );
  });
  const sizes = $derived(
    current.size !== null && !TEXT_SIZES.includes(current.size)
      ? [...TEXT_SIZES, current.size].sort((a, b) => a - b)
      : TEXT_SIZES,
  );

  $effect(() => {
    let live = true;
    editor.fonts().then((found) => {
      if (live) fonts = found;
    });
    return () => {
      live = false;
    };
  });

  $effect(() => {
    const onselectionchange = () => {
      const range = readRange();
      if (!range || (range.start === start && range.end === end)) return;
      start = range.start;
      end = range.end;
      pending = null;
      lastKind = null;
    };
    document.addEventListener("selectionchange", onselectionchange);
    return () => document.removeEventListener("selectionchange", onselectionchange);
  });

  $effect(() => {
    const changed = rows;
    untrack(() => onchange?.(changed));
  });

  // Span styles bake in the zoom, so zooming repaints them.
  $effect(() => {
    void scale;
    untrack(() => {
      repaint();
      anchor = null;
      placeBar();
    });
  });

  const fieldHistory: FieldHistory = {
    get canUndo() {
      return history.length > 0;
    },
    get canRedo() {
      return future.length > 0;
    },
    undo,
    redo,
  };

  onMount(() => {
    if (!field) return;
    paint();
    field.focus({ preventScroll: true });
    const hit = caret && document.caretRangeFromPoint?.(caret.x, caret.y);
    const at = hit && field.contains(hit.startContainer) ? offsetIn(hit.startContainer, hit.startOffset) : chars.length;
    select(at, at);
    editor.field = fieldHistory;
  });

  onMount(() => {
    const scroller = root?.closest<HTMLElement>(".scroller");
    let frame = 0;
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => placeBar());
    };
    scroller?.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    return () => {
      cancelAnimationFrame(frame);
      scroller?.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
    };
  });

  /** Centers the format bar over the field within the visible pages, below it when there's no room above. */
  function placeBar() {
    if (!bar || !field || !root) return;
    const scroller = root.closest<HTMLElement>(".scroller");
    const view = (scroller ?? document.documentElement).getBoundingClientRect();
    const right = view.left + (scroller?.clientWidth ?? view.width);
    const bottom = view.top + (scroller?.clientHeight ?? view.height);
    const own = root.getBoundingClientRect();
    const text = field.getBoundingClientRect();
    anchor ??= text.left + text.width / 2 - own.left;
    const width = bar.offsetWidth;
    const lowest = view.left + BAR_GAP_PX;
    const highest = Math.max(lowest, right - BAR_GAP_PX - width);
    barLeft = clamp(own.left + anchor - width / 2, lowest, highest) - own.left;
    const need = bar.offsetHeight + 2 * BAR_GAP_PX;
    const above = text.top - view.top;
    const under = bottom - text.bottom;
    barBelow = below ? under >= need || above < need : above < need && under > above;
  }

  // Scrolling far away unmounts the page; keep what was typed.
  onDestroy(() => close(false));

  /** Ends editing; the parent calls it before opening another field. */
  export function finish() {
    close(false);
  }

  function close(cancelled: boolean) {
    if (closed) return;
    closed = true;
    if (editor.field === fieldHistory) editor.field = null;
    onclose(rows, cancelled);
  }

  /** Field children are rows; the break between two rows counts as one character. */
  function rowElements(): HTMLElement[] {
    return field ? ([...field.children] as HTMLElement[]) : [];
  }

  function offsetIn(node: Node, offset: number): number {
    const elements = rowElements();
    const lengths = elements.map((row) => row.textContent?.length ?? 0);
    const rowStart = (r: number) => lengths.slice(0, r).reduce((sum, length) => sum + length + 1, 0);
    if (node === field) return offset < elements.length ? rowStart(offset) : Math.max(rowStart(elements.length) - 1, 0);
    const r = elements.findIndex((row) => row.contains(node));
    if (r < 0) return 0;
    const range = document.createRange();
    range.setStart(elements[r], 0);
    range.setEnd(node, offset);
    return rowStart(r) + range.toString().length;
  }

  function readRange(): { start: number; end: number } | null {
    const selection = getSelection();
    if (!field || !selection || selection.rangeCount === 0) return null;
    const range = selection.getRangeAt(0);
    if (!field.contains(range.startContainer) || !field.contains(range.endContainer)) return null;
    return toOffsets(range);
  }

  function toOffsets(range: AbstractRange): { start: number; end: number } {
    return { start: offsetIn(range.startContainer, range.startOffset), end: offsetIn(range.endContainer, range.endOffset) };
  }

  function position(offset: number): [Node, number] {
    const elements = rowElements();
    for (const row of elements) {
      const length = row.textContent?.length ?? 0;
      if (offset <= length) {
        const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
        let last: [Node, number] = [row, 0];
        for (let node = walker.nextNode(); node; node = walker.nextNode()) {
          const size = (node as Text).length;
          if (offset <= size) return [node, offset];
          offset -= size;
          last = [node, size];
        }
        return last;
      }
      offset -= length + 1;
    }
    return [field!, elements.length];
  }

  function select(from: number, to: number) {
    start = from;
    end = to;
    const selection = getSelection();
    if (!field || !selection) return;
    const range = document.createRange();
    range.setStart(...position(from));
    range.setEnd(...position(to));
    selection.removeAllRanges();
    selection.addRange(range);
  }

  /** Draws the text into the field as one element per row and span; empty rows keep their style. */
  function paint() {
    if (!field) return;
    painted = [];
    marks = [];
    field.replaceChildren(
      ...rows.map((row) => {
        const element = document.createElement("div");
        const shown = row.filter((span) => span.text);
        if (shown.length === 0) {
          const style = styleOf(row[0]);
          element.dataset.row = String(marks.push(style) - 1);
          element.style.cssText = spanCss(style, scale, leading);
          element.append(document.createElement("br"));
        }
        for (const span of shown) {
          const child = document.createElement("span");
          child.dataset.i = String(painted.push(styleOf(span)) - 1);
          child.style.cssText = spanCss(span, scale, leading);
          child.textContent = span.text;
          element.append(child);
        }
        return element;
      }),
    );
  }

  /** Paints the field again, keeping its selection. */
  function repaint() {
    if (composing) return;
    const range = readRange();
    paint();
    if (range) select(range.start, range.end);
  }

  /** Reads back text the browser changed in the field, such as after IME input. */
  function sync() {
    if (!field) return;
    const range = readRange();
    const breaks = chars.filter((char) => char.c === "\n").map((char) => char.style);
    const read: Char[] = [];
    rowElements().forEach((row, r) => {
      let style = marks[Number(row.dataset.row)] ?? read.at(-1)?.style ?? base;
      if (r > 0) read.push({ c: "\n", style: breaks[r - 1] ?? style });
      const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const owner = node.parentElement?.closest<HTMLElement>("[data-i]");
        if (owner && row.contains(owner)) style = painted[Number(owner.dataset.i)] ?? style;
        for (const c of (node as Text).data.replace(/\u00a0/g, " ").replace(/[\r\n]/g, "")) {
          for (let i = 0; i < c.length; i++) read.push({ c: c[i], style });
        }
      }
    });
    chars = read;
    paint();
    const at = Math.min(range?.end ?? read.length, read.length);
    select(Math.min(range?.start ?? at, at), at);
  }

  function record(kind: string) {
    if (kind !== lastKind || kind === "format") {
      history = [...history.slice(1 - MAX_HISTORY), { chars, ...(readRange() ?? { start, end }) }];
      future = [];
    }
    lastKind = kind;
  }

  function restore(snapshot: Snapshot) {
    chars = snapshot.chars;
    pending = null;
    lastKind = null;
    paint();
    select(snapshot.start, snapshot.end);
  }

  function undo() {
    const previous = history.at(-1);
    if (!previous) return;
    history = history.slice(0, -1);
    future = [...future, { chars, start, end }];
    restore(previous);
  }

  function redo() {
    const next = future.at(-1);
    if (!next) return;
    future = future.slice(0, -1);
    history = [...history, { chars, start, end }];
    restore(next);
  }

  /** Replaces the text between `from` and `to` with `text`, in the style typing there uses. */
  function replace(from: number, to: number, text: string, kind: string) {
    const style = pending ?? styleAt(chars, from, base);
    record(kind);
    const typed = [...Array(text.length).keys()].map((i) => ({ c: text[i], style }));
    chars = [...chars.slice(0, from), ...typed, ...chars.slice(to)];
    pending = text ? null : pending;
    paint();
    select(from + text.length, from + text.length);
  }

  /** Changes the style of the selected text, or of what is typed next when nothing is selected. */
  function format(change: (style: Style) => Style, kind = "format") {
    if (start === end) {
      pending = change(pending ?? styleAt(chars, start, base));
    } else {
      record(kind);
      chars = chars.map((char, i) => (i >= start && i < end ? { c: char.c, style: change(char.style) } : char));
      paint();
    }
    field?.focus({ preventScroll: true });
    select(start, end);
  }

  const toggle = (key: "bold" | "italic" | "underline") => {
    const on = !current[key];
    format((style) => ({ ...style, [key]: on }));
  };

  function setFont(name: string) {
    const family =
      [...fonts, ...STANDARD_FONTS].find((font) => font.name === name)?.family ??
      opened.flat().find((span) => span.font === name)?.family ??
      base.family;
    format((style) => ({ ...style, font: name, family }));
  }

  /** The text an edit replaces: its target range, else the selection or the character beside the caret. */
  function target(event: InputEvent, forward: boolean): { start: number; end: number } {
    const [range] = event.getTargetRanges();
    const offsets = range ? toOffsets(range) : (readRange() ?? { start, end });
    if (offsets.start !== offsets.end || !event.inputType.startsWith("delete")) return offsets;
    const at = offsets.start;
    // Steps over a whole surrogate pair.
    if (forward) return { start: at, end: Math.min(at + (/[\ud800-\udbff]/.test(chars[at]?.c ?? "") ? 2 : 1), chars.length) };
    return { start: Math.max(at - (/[\udc00-\udfff]/.test(chars[at - 1]?.c ?? "") ? 2 : 1), 0), end: at };
  }

  function clean(text: string): string {
    const normalized = text.replace(/\r\n?/g, "\n").replace(/\t/g, " ").replace(/\u00a0/g, " ");
    return multiline ? normalized : normalized.replace(/\n+/g, " ");
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    const mod = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();
    if (event.key === "Enter" && multiline && !mod) {
      const range = readRange() ?? { start, end };
      replace(range.start, range.end, "\n", "break");
    } else if (event.key === "Enter") close(false);
    else if (event.key === "Escape") close(true);
    else if (mod && key === "b") toggle("bold");
    else if (mod && key === "i") toggle("italic");
    else if (mod && key === "u") toggle("underline");
    else if (mod && (key === "y" || (key === "z" && event.shiftKey))) redo();
    else if (mod && key === "z") undo();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }

  function onbeforeinput(event: InputEvent) {
    if (event.isComposing || composing) return;
    event.preventDefault();
    const type = event.inputType;
    if (type === "insertText" || type === "insertReplacementText" || type === "insertFromPaste") {
      const text = event.data ?? event.dataTransfer?.getData("text/plain") ?? "";
      const range = target(event, false);
      replace(range.start, range.end, clean(text), "type");
    } else if ((type === "insertParagraph" || type === "insertLineBreak") && multiline) {
      const range = target(event, false);
      replace(range.start, range.end, "\n", "break");
    } else if (type.startsWith("delete") && type !== "deleteByDrag") {
      const range = target(event, type.endsWith("Forward"));
      if (range.start !== range.end) replace(range.start, range.end, "", "delete");
    }
  }

  function oncompositionend() {
    composing = false;
    record("type");
    sync();
  }

  // Moving to the format bar or another window keeps the field open.
  function onfocusout(event: FocusEvent) {
    const next = event.relatedTarget as Node | null;
    if ((next && root?.contains(next)) || !document.hasFocus()) return;
    close(false);
  }
</script>

<div class="rich" bind:this={root} {onfocusout}>
  <div
    class="field"
    class:cover
    class:multiline
    contenteditable="plaintext-only"
    role="textbox"
    aria-label="Text"
    aria-multiline={multiline}
    title={multiline ? `${MOD}Enter or Esc to finish` : "Enter to finish, Esc to cancel"}
    tabindex="0"
    spellcheck="false"
    bind:this={field}
    style={fieldStyle}
    {onkeydown}
    {onbeforeinput}
    oncompositionstart={() => (composing = true)}
    {oncompositionend}
    ondrop={(event) => event.preventDefault()}
  ></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="bar"
    class:below={barBelow}
    role="toolbar"
    aria-label="Text style"
    tabindex="-1"
    bind:this={bar}
    style:left={barLeft === null ? undefined : `${barLeft}px`}
    use:roving
    onpointerdown={(event) => event.stopPropagation()}
    ondblclick={(event) => event.stopPropagation()}
  >
    <select
      class="font"
      aria-label="Font"
      title={current.font ?? "Font"}
      value={current.font ?? ""}
      onchange={(event) => setFont(event.currentTarget.value)}
    >
      <option value="" hidden></option>
      <optgroup label="In this document">
        {#each documentNames as name (name)}
          <option value={name} title={name}>{labels.get(name)}</option>
        {/each}
      </optgroup>
      {#if standardNames.length > 0}
        <optgroup label="Standard">
          {#each standardNames as font (font.name)}
            <option value={font.name} style:font-family={fontFamily({ font: font.name, family: font.family })}>
              {labels.get(font.name)}
            </option>
          {/each}
        </optgroup>
      {/if}
    </select>
    <select
      class="size"
      aria-label="Font size"
      title="Font size"
      value={current.size === null ? "" : String(current.size)}
      onchange={(event) => {
        const size = Number(event.currentTarget.value);
        format((style) => ({ ...style, size }));
      }}
    >
      <option value="" hidden></option>
      {#each sizes as size (size)}
        <option value={String(size)}>{Math.round(size * 100) / 100}</option>
      {/each}
    </select>
    <span class="divider"></span>
    <button
      class="tool"
      aria-label="Bold"
      title="Bold ({MOD}B)"
      aria-pressed={current.bold}
      onmousedown={(event) => event.preventDefault()}
      onclick={() => toggle("bold")}><b>B</b></button
    >
    <button
      class="tool"
      aria-label="Italic"
      title="Italic ({MOD}I)"
      aria-pressed={current.italic}
      onmousedown={(event) => event.preventDefault()}
      onclick={() => toggle("italic")}><i>I</i></button
    >
    <button
      class="tool"
      aria-label="Underline"
      title="Underline ({MOD}U)"
      aria-pressed={current.underline}
      onmousedown={(event) => event.preventDefault()}
      onclick={() => toggle("underline")}><u>U</u></button
    >
    <span class="divider"></span>
    <input
      class="color"
      type="color"
      aria-label="Text color"
      title="Text color"
      value={hex(current.color)}
      oninput={(event) => {
        const color = fromHex(event.currentTarget.value);
        format((style) => ({ ...style, color }), "color");
      }}
    />
    <span class="divider"></span>
    <button
      class="tool"
      aria-label="Done"
      title={multiline ? `Done (${MOD}Enter)` : "Done (Enter)"}
      onmousedown={(event) => event.preventDefault()}
      onclick={() => close(false)}
    >
      <Icon name="check" />
    </button>
  </div>
</div>

<style>
  .rich {
    position: relative;
  }

  .field {
    display: block;
    box-sizing: content-box;
    width: max-content;
    outline: none;
    white-space: pre;
    caret-color: var(--accent);
    cursor: text;
    user-select: text;
  }

  /* Rows take their height from their text alone, as they do once saved. */
  .multiline {
    min-width: 2px;
    font-size: 0;
  }

  /* Matches the page patches in EditLayer so finishing an edit doesn't shift the text. */
  .cover {
    border-radius: 2px;
    outline: 2px solid var(--accent);
    outline-offset: 1px;
    background: #fff;
  }

  .bar {
    position: absolute;
    bottom: calc(100% + 0.5rem);
    left: -0.25rem;
    z-index: 2;
    display: flex;
    gap: 0.125rem;
    align-items: center;
    padding: 0.25rem;
    border: 1px solid var(--border);
    border-radius: 0.625rem;
    background: var(--popover);
    box-shadow: var(--popover-shadow);
    font-size: 0.875rem;
    white-space: nowrap;
    cursor: default;
  }

  .below {
    top: calc(100% + 0.5rem);
    bottom: auto;
  }

  .bar select {
    height: 2rem;
    padding: 0 0.25rem;
    border: 1px solid var(--border);
    border-radius: 0.375rem;
    background: var(--surface);
    color: inherit;
    font: inherit;
  }

  .font {
    width: 11.5rem;
  }

  .size {
    width: 4.25rem;
    font-variant-numeric: tabular-nums;
  }

  .bar .tool {
    min-width: 2rem;
    height: 2rem;
    padding: 0 0.25rem;
    font-size: 1rem;
  }

  .color {
    width: 2rem;
    height: 2rem;
    padding: 0.1875rem;
    border: 1px solid var(--border);
    border-radius: 0.375rem;
    background: var(--surface);
    cursor: pointer;
  }

  .divider {
    width: 1px;
    height: 1.25rem;
    margin: 0 0.25rem;
    background: var(--border);
  }
</style>
