import type { Span, Style } from "./api";

/** One UTF-16 code unit of styled text, so offsets match the DOM's. */
export interface Char {
  c: string;
  style: Style;
}

export const sameStyle = (a: Style, b: Style) =>
  a.font === b.font &&
  a.face === b.face &&
  a.family === b.family &&
  a.bold === b.bold &&
  a.italic === b.italic &&
  a.underline === b.underline &&
  a.size === b.size &&
  a.color.every((value, i) => value === b.color[i]);

export const sameSpans = (a: Span[], b: Span[]) =>
  a.length === b.length && a.every((span, i) => span.text === b[i].text && sameStyle(span, b[i]));

export function styleOf(span: Span): Style {
  const { text, ...style } = span;
  void text;
  return style;
}

export function charsOf(spans: Span[]): Char[] {
  return spans.flatMap((span) => {
    const style = styleOf(span);
    return [...Array(span.text.length).keys()].map((i) => ({ c: span.text[i], style }));
  });
}

/** Joins characters of the same style into spans; no text keeps `fallback` as an empty span. */
export function spansOf(chars: Char[], fallback: Style): Span[] {
  const spans: Span[] = [];
  for (const { c, style } of chars) {
    const last = spans.at(-1);
    if (last && sameStyle(last, style)) last.text += c;
    else spans.push({ ...style, text: c });
  }
  return spans.length > 0 ? spans : [{ ...fallback, text: "" }];
}

/**
 * The style typing at `offset` continues: the character before it, or the one after at the start
 * of a row. An empty row continues the row break before it.
 */
export function styleAt(chars: Char[], offset: number, fallback: Style): Style {
  const before = chars[offset - 1];
  const after = chars[offset];
  if (before && before.c !== "\n") return before.style;
  if (after && after.c !== "\n") return after.style;
  return before?.style ?? after?.style ?? fallback;
}

/** Splits text into rows at its row breaks; an empty row keeps the style typing in it would use. */
export function rowsOf(chars: Char[], fallback: Style): Span[][] {
  const rows: Span[][] = [];
  let start = 0;
  for (let i = 0; i <= chars.length; i++) {
    if (i < chars.length && chars[i].c !== "\n") continue;
    rows.push(
      i > start ? spansOf(chars.slice(start, i), fallback) : [{ ...styleAt(chars, start, fallback), text: "" }],
    );
    start = i + 1;
  }
  return rows;
}

/** Joins rows with row breaks, styled so `rowsOf` gives the rows back. */
export function charsOfRows(rows: Span[][], fallback: Style): Char[] {
  return rows.flatMap((row, r) => {
    const chars = charsOf(row);
    if (r === 0) return chars;
    const mark = chars.length === 0 ? row[0] : rows[r - 1].at(-1);
    return [{ c: "\n", style: mark ? styleOf(mark) : fallback }, ...chars];
  });
}

export const sameRows = (a: Span[][], b: Span[][]) =>
  a.length === b.length && a.every((row, i) => sameSpans(row, b[i]));

export const isBlank = (rows: Span[][]) => rows.every((row) => row.every((span) => !span.text.trim()));
