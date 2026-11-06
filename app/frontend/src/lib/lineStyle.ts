import type { Color, Family, Span, Style, TextLine } from "./api";
import { css } from "./edits.svelte";

const FAMILIES: Record<Family, string> = {
  serif: '"Times New Roman", "Liberation Serif", Times, serif',
  sans: 'Arial, "Liberation Sans", Helvetica, sans-serif',
  mono: '"Courier New", "Liberation Mono", Courier, monospace',
};
/** The standard PDF fonts, which every reader can show without embedding them. */
export const STANDARD_FONTS: { name: string; family: Family }[] = [
  { name: "Helvetica", family: "sans" },
  { name: "Times", family: "serif" },
  { name: "Courier", family: "mono" },
];
/** Room for the caret on each side of the text, in CSS pixels; boxes need it as padding. */
export const CARET_ROOM = 2;
/** Boxes are at least this many font sizes tall, so they cover the original glyphs. */
const MIN_LEADING = 1.2;

const measure = document.createElement("canvas").getContext("2d")!;

/** CSS font families for a PDF font: its own name when installed, then a stand-in. */
export function fontFamily({ font, family }: Pick<Style, "font" | "family">): string {
  if (STANDARD_FONTS.some((standard) => standard.name === font)) return FAMILIES[family];
  const name = font.replace(/["\\]/g, "");
  if (!name) return FAMILIES[family];
  // PostScript names like "TimesNewRomanPSMT" become the installed family "Times New Roman".
  const spaced = name.replace(/(PS)?MT$/, "").replace(/([a-z])([A-Z])/g, "$1 $2");
  const names = spaced === name ? `"${name}"` : `"${name}", "${spaced}"`;
  return `${names}, ${FAMILIES[family]}`;
}

function fontCss(style: Style, scale: number, lineHeight?: number): string {
  const size = `${style.size * scale}px${lineHeight ? `/${lineHeight}px` : ""}`;
  return `${style.italic ? "italic " : ""}${style.bold ? 700 : 400} ${size} ${fontFamily(style)}`;
}

export const hex = (color: Color) => `#${color.map((value) => value.toString(16).padStart(2, "0")).join("")}`;
export const fromHex = (value: string): Color => [1, 3, 5].map((i) => parseInt(value.slice(i, i + 2), 16)) as Color;

const FONT_ALIASES: [RegExp, string][] = [
  [/^NimbusRomNo9L$/, "Nimbus Roman"],
  [/^NimbusSanL$/, "Nimbus Sans"],
  [/^NimbusMonL$/, "Nimbus Mono"],
  [/^LMRoman\d*$/, "Latin Modern Roman"],
  [/^LMSans\d*$/, "Latin Modern Sans"],
  [/^LMMono\d*$/, "Latin Modern Mono"],
  [/^CMR\d*$/, "Computer Modern"],
  [/^CMBX\d*$/, "Computer Modern Bold"],
  [/^CMTI\d*$/, "Computer Modern Italic"],
  [/^CMSL\d*$/, "Computer Modern Slanted"],
  [/^CMTT\d*$/, "Computer Modern Typewriter"],
  [/^CMSS\d*$/, "Computer Modern Sans"],
  [/^CMMI\d*$/, "Computer Modern Math Italic"],
  [/^CMSY\d*$/, "Computer Modern Symbols"],
  [/^CMEX\d*$/, "Computer Modern Extension"],
];
const STYLE_WORDS: Record<string, string> = {
  Regu: "",
  Regular: "",
  Roman: "",
  Medi: "Medium",
  Ital: "Italic",
  It: "Italic",
  Obli: "Oblique",
  Bd: "Bold",
};

const words = (part: string) =>
  part
    .replace(/PS$|(PS)?MT$/, "")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .trim();

/** A readable name for a PDF font, such as "Times New Roman Bold" for "TimesNewRomanPS-BoldMT". */
export function fontLabel(name: string): string {
  const [family, style = ""] = name.replace(/^[A-Z]{6}\+/, "").split(/[-,]/, 2);
  const alias = FONT_ALIASES.find(([pattern]) => pattern.test(family))?.[1];
  const styles = words(style)
    .split(" ")
    .map((word) => STYLE_WORDS[word] ?? word)
    .filter(Boolean);
  return [alias ?? words(family), ...styles].join(" ").trim() || name;
}

/** Inline CSS for a span: its line height is a multiple of its size, or the line box's. */
export const spanCss = (style: Style, scale: number, leading: number | "inherit" = "inherit") =>
  `font: ${fontCss(style, scale)}; line-height: ${leading}; color: ${css(style.color)}; ` +
  `text-decoration: ${style.underline ? "underline" : "none"}`;

/**
 * The CSS box that shows `spans` in place of the line and hides its original glyphs, placed so
 * the text sits on the line's baseline.
 */
export function lineBox(line: TextLine, spans: Span[], scale: number, width = line.width) {
  const strut = spans[0] ?? line.spans[0];
  const sizes = [...spans, ...line.spans].map((span) => span.size);
  const lineHeight = Math.max(line.height, Math.max(...sizes) * MIN_LEADING) * scale;
  // CSS centres each font's ascent and descent in the line height, then aligns baselines.
  let above = 0;
  for (const style of [strut, ...spans.filter((span) => span.text)]) {
    measure.font = fontCss(style, scale);
    const metrics = measure.measureText("Hg");
    above = Math.max(above, (lineHeight + metrics.fontBoundingBoxAscent - metrics.fontBoundingBoxDescent) / 2);
  }
  return {
    left: line.x * scale,
    top: line.baseline * scale - above,
    minWidth: width * scale,
    lineHeight,
    font: fontCss(strut, scale, lineHeight),
  };
}
