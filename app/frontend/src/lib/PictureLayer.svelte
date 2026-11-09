<script lang="ts">
  import type { Picture, Rect } from "./api";
  import { editor, pictureKey } from "./edits.svelte";
  import Icon from "./Icon.svelte";
  import { clamp } from "./layout.svelte";
  import { roving } from "./roving";

  const DRAG_THRESHOLD_PX = 3;
  /** Room the floating bar needs above a picture before it moves below instead. */
  const BAR_ROOM_PX = 48;
  /** The shortest side a picture can be resized to, in points. */
  const MIN_SIDE = 8;

  interface Props {
    page: number;
    /** Page width in points as displayed. */
    width: number;
    /** CSS pixels per point. */
    scale: number;
  }

  let { page, width, scale }: Props = $props();

  /** A picture's pixels as the page shows them, and where. */
  interface Snapshot {
    pixels: HTMLCanvasElement;
    from: Rect;
  }

  interface Drag {
    picture: Picture;
    pointer: number;
    kind: "move" | "resize";
    startX: number;
    startY: number;
    start: Rect;
    moved: boolean;
    snapshot: Snapshot | null;
  }

  let layer: HTMLDivElement;
  let pictures = $state.raw<Picture[]>([]);
  let drag: Drag | null = null;
  // Gestures preview locally and commit a single undo step on release.
  let preview = $state.raw<{ op: number; to: Rect; snapshot: Snapshot | null } | null>(null);

  const interactive = $derived(editor.tool === "none");
  const dragging = $derived(preview !== null);
  const ghosts = $derived(editor.ghosts.filter((ghost) => ghost.page === page && ghost.op !== preview?.op));

  $effect(() => {
    let live = true;
    editor.pictures(page).then((found) => {
      if (live) pictures = found;
    });
    return () => {
      live = false;
    };
  });

  // Escape puts a dragged picture back without closing anything else.
  $effect(() => {
    if (!dragging) return;
    const onkeydown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      cancel();
    };
    window.addEventListener("keydown", onkeydown, true);
    return () => window.removeEventListener("keydown", onkeydown, true);
  });

  const within = (value: number, min: number, max: number) => clamp(value, Math.min(min, max), Math.max(min, max));

  /** Where a picture can go and stay visible, including where it started. */
  function room({ limits, bounds }: Picture): Rect {
    const x = Math.min(limits.x, bounds.x);
    const y = Math.min(limits.y, bounds.y);
    return {
      x,
      y,
      width: Math.max(limits.x + limits.width, bounds.x + bounds.width) - x,
      height: Math.max(limits.y + limits.height, bounds.y + bounds.height) - y,
    };
  }

  function moved(picture: Picture, from: Rect, dx: number, dy: number): Rect {
    const area = room(picture);
    return {
      ...from,
      x: within(from.x + dx, area.x, area.x + area.width - from.width),
      y: within(from.y + dy, area.y, area.y + area.height - from.height),
    };
  }

  /** Resizes from the bottom-right corner, keeping the picture's proportions. */
  function resized(picture: Picture, from: Rect, dx: number, dy: number): Rect {
    const { width: w, height: h } = from;
    if (w <= 0 || h <= 0) return from;
    const area = room(picture);
    // Follows the pointer along the picture's diagonal.
    const k = ((w + dx) * w + (h + dy) * h) / (w * w + h * h);
    const min = Math.min(1, MIN_SIDE / Math.min(w, h));
    const max = Math.max(min, Math.min((area.x + area.width - from.x) / w, (area.y + area.height - from.y) / h));
    const fit = clamp(k, min, max);
    return { ...from, width: w * fit, height: h * fit };
  }

  /** Copies the picture from the page image, to draw it where it goes until the page shows it there. */
  function snapshot(picture: Picture, bounds: Rect): Snapshot | null {
    const ghost = editor.ghosts.find((other) => other.page === page && other.op === picture.op);
    if (ghost) return { pixels: ghost.pixels, from: ghost.from };
    const image = layer.closest(".page")?.querySelector<HTMLImageElement>("img:not(.hidden)");
    if (!image?.complete || image.naturalWidth === 0) return null;
    const k = image.naturalWidth / width;
    const pixels = document.createElement("canvas");
    pixels.width = Math.max(1, Math.round(bounds.width * k));
    pixels.height = Math.max(1, Math.round(bounds.height * k));
    pixels
      .getContext("2d")
      ?.drawImage(image, bounds.x * k, bounds.y * k, bounds.width * k, bounds.height * k, 0, 0, pixels.width, pixels.height);
    return { pixels, from: bounds };
  }

  function paint(canvas: HTMLCanvasElement, source: HTMLCanvasElement) {
    const draw = (from: HTMLCanvasElement) => {
      canvas.width = from.width;
      canvas.height = from.height;
      canvas.getContext("2d")?.drawImage(from, 0, 0);
    };
    draw(source);
    return { update: draw };
  }

  function onpointerdown(event: PointerEvent, picture: Picture, kind: Drag["kind"]) {
    if (event.button !== 0 || !interactive) return;
    const bounds = editor.pictureBounds(page, picture);
    if (!bounds) return;
    const node = event.currentTarget as HTMLElement;
    if (kind === "resize") {
      event.stopPropagation();
      event.preventDefault();
    }
    node.closest<HTMLElement>(".picture")?.focus({ preventScroll: true });
    editor.selectPicture(page, picture.op);
    node.setPointerCapture(event.pointerId);
    drag = {
      picture,
      pointer: event.pointerId,
      kind,
      startX: event.clientX,
      startY: event.clientY,
      start: bounds,
      moved: false,
      snapshot: null,
    };
  }

  function onpointermove(event: PointerEvent) {
    if (drag?.pointer !== event.pointerId) return;
    const dxPx = event.clientX - drag.startX;
    const dyPx = event.clientY - drag.startY;
    if (!drag.moved && Math.hypot(dxPx, dyPx) < DRAG_THRESHOLD_PX) return;
    if (!drag.moved) drag.snapshot = snapshot(drag.picture, drag.start);
    drag.moved = true;
    const shift = drag.kind === "move" ? moved : resized;
    preview = {
      op: drag.picture.op,
      to: shift(drag.picture, drag.start, dxPx / scale, dyPx / scale),
      snapshot: drag.snapshot,
    };
  }

  function onpointerup(event: PointerEvent) {
    if (drag?.pointer !== event.pointerId) return;
    if (preview) editor.movePicture(page, drag.picture, preview.to, preview.snapshot);
    cancel();
  }

  function cancel() {
    drag = null;
    preview = null;
  }

  function remove(picture: Picture) {
    const bounds = editor.pictureBounds(page, picture);
    if (bounds) editor.movePicture(page, picture, null, snapshot(picture, bounds));
  }

  function onkeydown(event: KeyboardEvent, picture: Picture) {
    if (event.target !== event.currentTarget || drag || event.ctrlKey || event.metaKey || event.altKey) return;
    const bounds = editor.pictureBounds(page, picture);
    if (!bounds || editor.picture !== pictureKey(page, picture.op)) return;
    const step = event.shiftKey ? 10 : 1;
    const nudges: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    if (event.key === "Delete" || event.key === "Backspace") {
      remove(picture);
    } else if (event.key in nudges) {
      const [dx, dy] = nudges[event.key];
      editor.movePicture(page, picture, moved(picture, bounds, dx, dy), snapshot(picture, bounds));
    } else {
      return;
    }
    event.preventDefault();
  }
</script>

<div class="pictures" bind:this={layer}>
  {#each ghosts as ghost (ghost.op)}
    <div
      class="cover"
      style:left="{ghost.from.x * scale}px"
      style:top="{ghost.from.y * scale}px"
      style:width="{ghost.from.width * scale}px"
      style:height="{ghost.from.height * scale}px"
    ></div>
    {#if ghost.to}
      <canvas
        class="moved"
        use:paint={ghost.pixels}
        style:left="{ghost.to.x * scale}px"
        style:top="{ghost.to.y * scale}px"
        style:width="{ghost.to.width * scale}px"
        style:height="{ghost.to.height * scale}px"
      ></canvas>
    {/if}
  {/each}
  {#if preview?.snapshot}
    {@const { from, pixels } = preview.snapshot}
    <div
      class="cover"
      style:left="{from.x * scale}px"
      style:top="{from.y * scale}px"
      style:width="{from.width * scale}px"
      style:height="{from.height * scale}px"
    ></div>
    <canvas
      class="moved"
      use:paint={pixels}
      style:left="{preview.to.x * scale}px"
      style:top="{preview.to.y * scale}px"
      style:width="{preview.to.width * scale}px"
      style:height="{preview.to.height * scale}px"
    ></canvas>
  {/if}
  {#if interactive}
    {#each pictures as picture (picture.op)}
      {@const shown = preview?.op === picture.op ? preview.to : editor.pictureBounds(page, picture)}
      {#if shown}
        {@const selected = editor.picture === pictureKey(page, picture.op)}
        <div
          class="picture"
          class:selected
          data-edit
          role="button"
          tabindex="-1"
          aria-label="Picture"
          aria-pressed={selected}
          style:left="{shown.x * scale}px"
          style:top="{shown.y * scale}px"
          style:width="{shown.width * scale}px"
          style:height="{shown.height * scale}px"
          onpointerdown={(event) => onpointerdown(event, picture, "move")}
          {onpointermove}
          {onpointerup}
          onpointercancel={cancel}
          onkeydown={(event) => onkeydown(event, picture)}
        >
          {#if selected}
            <span class="handle" aria-hidden="true" onpointerdown={(event) => onpointerdown(event, picture, "resize")}
            ></span>
            {#if !preview}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="bar"
                class:below={shown.y * scale < BAR_ROOM_PX}
                role="toolbar"
                aria-label="Picture"
                tabindex="-1"
                use:roving
                onpointerdown={(event) => event.stopPropagation()}
                onmousedown={(event) => event.preventDefault()}
              >
                <button class="tool" aria-label="Delete" title="Delete (Del)" onclick={() => remove(picture)}>
                  <Icon name="trash" size={16} />
                </button>
              </div>
            {/if}
          {/if}
        </div>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .pictures {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  /* The outline hides the anti-aliased edge pixels the render leaves past fractional bounds. */
  .cover {
    position: absolute;
    background: #fff;
    outline: 1px solid #fff;
  }

  .moved {
    position: absolute;
    display: block;
  }

  .picture {
    position: absolute;
    border-radius: 1px;
    outline: none;
    cursor: move;
    user-select: none;
    touch-action: none;
    pointer-events: auto;
  }

  .picture:hover {
    outline: 1px dashed color-mix(in srgb, var(--accent) 70%, transparent);
  }

  /* Above text and links, so the whole picture, its handle, and its bar take clicks. */
  .selected,
  .selected:hover {
    z-index: 2;
    outline: 1.5px solid var(--accent);
  }

  .handle {
    position: absolute;
    right: -5px;
    bottom: -5px;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--accent);
    border-radius: 2px;
    background: #fff;
    cursor: nwse-resize;
  }

  .bar {
    position: absolute;
    bottom: calc(100% + 0.5rem);
    left: 0;
    display: flex;
    gap: 0.125rem;
    align-items: center;
    padding: 0.25rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
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

  .bar .tool {
    min-width: 1.75rem;
    height: 1.75rem;
    padding: 0 0.25rem;
  }
</style>
