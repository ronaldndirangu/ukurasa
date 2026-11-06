<script lang="ts" module>
  /** A scroll position that survives reopening the same pages. */
  export interface Position {
    page: number;
    /** Fraction of the page's height scrolled past its top. */
    offset: number;
    left: number;
  }
</script>

<script lang="ts">
  import { flushSync, onMount, tick, untrack } from "svelte";
  import type { DocumentView, Fit, FitMode } from "./api";
  import EditLayer from "./EditLayer.svelte";
  import { editor } from "./edits.svelte";
  import { PX_PER_PT, clamp, display, indexAt, range } from "./layout.svelte";
  import LinkLayer from "./LinkLayer.svelte";
  import PageImage from "./PageImage.svelte";

  const PAD = 16;
  const GAP = 12;
  const MIN_ZOOM = 0.1;
  const MAX_ZOOM = 8;
  const STEPS = [0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5, 6, 8];
  const OVERSCAN = 2;
  const MAX_RENDER_PIXELS = 16_000_000;
  const RENDER_DELAY_MS = 150;

  interface Props {
    doc: DocumentView;
    /** Zero-based page nearest the top third of the viewport. */
    page?: number;
    zoom?: number;
    fit?: Fit;
    /** Where to scroll once mounted. */
    start?: Position | null;
  }

  let { doc, page = $bindable(0), zoom = $bindable(1), fit = $bindable(null), start = null }: Props = $props();

  interface Layout {
    tops: Float64Array;
    widths: Float64Array;
    heights: Float64Array;
    width: number;
    height: number;
  }

  let scroller: HTMLDivElement;
  let viewportWidth = $state(0);
  let viewportHeight = $state(0);
  let scrollTop = $state(0);
  // Navigation keeps its target current, even when several small pages fit on screen, until
  // the view scrolls again.
  let pinned = $state.raw<{ page: number; top: number } | null>(null);
  // Pages re-render only after zooming pauses; until then their current images are scaled.
  let renderZoom = $state(0);

  const layout: Layout = $derived.by(() => {
    const scale = zoom * PX_PER_PT;
    const count = doc.pages.length;
    const tops = new Float64Array(count);
    const widths = new Float64Array(count);
    const heights = new Float64Array(count);
    let y = PAD;
    let maxWidth = 0;
    for (let i = 0; i < count; i++) {
      widths[i] = Math.max(1, Math.round(doc.pages[i].width * scale));
      heights[i] = Math.max(1, Math.round(doc.pages[i].height * scale));
      tops[i] = y;
      y += heights[i] + GAP;
      maxWidth = Math.max(maxWidth, widths[i]);
    }
    return {
      tops,
      widths,
      heights,
      width: Math.max(viewportWidth, maxWidth + 2 * PAD),
      height: y - GAP + PAD,
    };
  });

  const first = $derived(pageAt(scrollTop));
  const last = $derived(pageAt(scrollTop + viewportHeight));
  const mounted = $derived(range(first, last, OVERSCAN, doc.pages.length));
  const scale = $derived(zoom * PX_PER_PT);
  // The selected item's page sits above its neighbours so the item's floating bar stays visible.
  const raised = $derived(editor.get(editor.selected)?.edit.page ?? -1);
  const current = $derived.by(() => {
    if (pinned && Math.abs(scrollTop - pinned.top) < 1) return pinned.page;
    const maxScroll = layout.height - viewportHeight;
    if (maxScroll > 0 && scrollTop >= maxScroll - 1) return doc.pages.length - 1;
    return pageAt(scrollTop + viewportHeight / 3);
  });

  $effect(() => {
    page = current;
  });

  $effect(() => {
    const target = zoom;
    if (target === untrack(() => renderZoom)) return;
    const timer = setTimeout(() => (renderZoom = target), RENDER_DELAY_MS);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    const mode = fit;
    if (!mode || viewportWidth === 0 || viewportHeight === 0) return;
    // flushSync is not allowed inside effects, so the zoom change runs right after.
    queueMicrotask(() => {
      if (fit === mode) applyFit(mode);
    });
  });

  onMount(() => {
    viewportWidth = scroller.clientWidth;
    viewportHeight = scroller.clientHeight;
    if (fit) zoom = fitZoom(fit);
    renderZoom = zoom;
    scroller.focus({ preventScroll: true });
    scroller.addEventListener("wheel", onWheel, { passive: false });
    const target = untrack(() => start);
    if (target) tick().then(() => scrollTo(target));
    return () => scroller.removeEventListener("wheel", onWheel);
  });

  export function position(): Position {
    const index = pageAt(scroller.scrollTop);
    return {
      page: index,
      offset: (scroller.scrollTop - layout.tops[index]) / layout.heights[index],
      left: scroller.scrollLeft,
    };
  }

  function scrollTo(target: Position) {
    const index = Math.min(Math.max(target.page, 0), doc.pages.length - 1);
    scroller.scrollTop = layout.tops[index] + target.offset * layout.heights[index];
    scroller.scrollLeft = target.left;
    scrollTop = scroller.scrollTop;
  }

  function onpointerdown(event: PointerEvent) {
    if (!event.defaultPrevented && !(event.target as Element).closest("[data-edit]")) editor.select(null);
  }

  export function goToPage(index: number) {
    goToPoint(index, null);
  }

  /** Scrolls so `top`, in points from the top of the page, sits just below the viewport's top. */
  function goToPoint(index: number, top: number | null) {
    const target = clamp(index, 0, doc.pages.length - 1);
    const offset = top === null ? 0 : (top / doc.pages[target].height) * layout.heights[target];
    scroller.scrollTop = layout.tops[target] + offset - PAD;
    pinned = { page: target, top: scroller.scrollTop };
    scrollTop = scroller.scrollTop;
  }

  export function zoomIn() {
    fit = null;
    setZoom(STEPS.find((step) => step > zoom * 1.001) ?? MAX_ZOOM);
  }

  export function zoomOut() {
    fit = null;
    let next = MIN_ZOOM;
    for (const step of STEPS) if (step < zoom / 1.001) next = step;
    setZoom(next);
  }

  export function zoomTo(value: number) {
    fit = null;
    setZoom(value);
  }

  export function fitTo(mode: FitMode) {
    fit = mode;
    applyFit(mode);
  }

  function applyFit(mode: FitMode) {
    setZoom(fitZoom(mode), 0, 0);
    if (mode === "page") goToPage(page);
  }

  function fitZoom(mode: FitMode): number {
    const width = mode === "width" ? doc.pages.reduce((max, size) => Math.max(max, size.width), 0) : doc.pages[page].width;
    // One spare pixel keeps rounding from adding a horizontal scrollbar.
    const byWidth = (viewportWidth - 2 * PAD - 1) / (width * PX_PER_PT);
    if (mode === "width") return byWidth;
    return Math.min(byWidth, (viewportHeight - 2 * PAD - 1) / (doc.pages[page].height * PX_PER_PT));
  }

  /** Changes the zoom while keeping the point under (anchorX, anchorY) in place. */
  function setZoom(value: number, anchorX = viewportWidth / 2, anchorY = viewportHeight / 2) {
    const next = Math.min(Math.max(value, MIN_ZOOM), MAX_ZOOM);
    if (Math.abs(next - zoom) < 1e-6) return;
    const before = layout;
    const y = scroller.scrollTop + anchorY;
    const x = scroller.scrollLeft + anchorX;
    const index = pageAt(y);
    const fy = (y - before.tops[index]) / before.heights[index];
    const fx = (x - leftOf(before, index)) / before.widths[index];
    zoom = next;
    const after = layout;
    const top = Math.max(0, after.tops[index] + fy * after.heights[index] - anchorY);
    const left = Math.max(0, leftOf(after, index) + fx * after.widths[index] - anchorX);
    scrollTop = top;
    flushSync();
    scroller.scrollTop = top;
    scroller.scrollLeft = left;
    scrollTop = scroller.scrollTop;
  }

  function onWheel(event: WheelEvent) {
    if (!event.ctrlKey && !event.metaKey) return;
    event.preventDefault();
    const rect = scroller.getBoundingClientRect();
    const delta = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? event.deltaY * 16 : event.deltaY;
    fit = null;
    setZoom(zoom * Math.exp(-delta * 0.002), event.clientX - rect.left, event.clientY - rect.top);
  }

  function pageAt(y: number): number {
    return indexAt(layout.tops, y);
  }

  function leftOf(at: Layout, index: number): number {
    return Math.round((at.width - at.widths[index]) / 2);
  }

  function renderWidth(index: number): number {
    if (renderZoom <= 0) return 0;
    const size = doc.pages[index];
    const css = Math.round(size.width * renderZoom * PX_PER_PT) * display.dpr;
    const limit = Math.sqrt((MAX_RENDER_PIXELS * size.width) / size.height);
    return Math.max(1, Math.round(Math.min(css, limit)));
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="scroller"
  role="region"
  aria-label="Pages"
  tabindex="0"
  bind:this={scroller}
  bind:clientWidth={viewportWidth}
  bind:clientHeight={viewportHeight}
  onscroll={() => (scrollTop = scroller.scrollTop)}
  {onpointerdown}
>
  <div class="content" style:width="{layout.width}px" style:height="{layout.height}px">
    {#each mounted as index (index)}
      {@const visible = index >= first && index <= last}
      <div
        class="page"
        class:raised={index === raised}
        role="group"
        aria-label="Page {index + 1}"
        style:top="{layout.tops[index]}px"
        style:left="{leftOf(layout, index)}px"
        style:width="{layout.widths[index]}px"
        style:height="{layout.heights[index]}px"
      >
        <PageImage
          doc={doc.id}
          {index}
          renderWidth={renderWidth(index)}
          priority={visible ? 1 : 3}
          previewWidth={display.thumbRenderWidth}
          previewPriority={visible ? 0 : 2}
          revision={editor.revisions.get(index) ?? 0}
          onshow={(revision) => editor.painted(index, revision)}
        />
        {#if editor.tool === "none"}
          <LinkLayer doc={doc.id} page={index} {scale} onnavigate={goToPoint} />
        {/if}
        <EditLayer page={index} width={doc.pages[index].width} height={doc.pages[index].height} {scale} />
      </div>
    {/each}
  </div>
</div>

<style>
  .scroller {
    position: absolute;
    inset: 0;
    overflow: auto;
    scrollbar-gutter: stable;
    background: var(--canvas);
    outline: none;
  }

  .content {
    position: relative;
  }

  .page {
    position: absolute;
    box-shadow: var(--page-shadow);
  }

  .raised {
    z-index: 1;
  }
</style>
