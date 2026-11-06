<script lang="ts">
  import { SIGNATURE_INKS, css, editor, sameColor, type Placed } from "./edits.svelte";
  import Icon from "./Icon.svelte";
  import Ink from "./Ink.svelte";
  import { clamp } from "./layout.svelte";
  import RichText from "./RichText.svelte";
  import { roving } from "./roving";
  import TextRows from "./TextRows.svelte";

  /** Room for the outline around text. */
  const TEXT_PADDING = 2;
  const MIN_SIGNATURE_WIDTH = 20;
  const DRAG_THRESHOLD_PX = 3;
  /** Room the floating bar needs above an item before it moves below instead. */
  const BAR_ROOM_PX = 48;

  interface Props {
    item: Placed;
    /** CSS pixels per point. */
    scale: number;
    pageWidth: number;
    pageHeight: number;
  }

  let { item, scale, pageWidth, pageHeight }: Props = $props();

  interface Gesture {
    pointer: number;
    kind: "move" | "resize";
    startX: number;
    startY: number;
    /** Measured item size in points. */
    width: number;
    height: number;
    moved: boolean;
  }

  let node: HTMLDivElement;
  let gesture: Gesture | null = null;
  // Gestures preview locally and commit a single undo step on release.
  let offset = $state.raw<{ x: number; y: number } | null>(null);
  let resized = $state<number | null>(null);
  /** Where the click that started typing was, for the caret. */
  let caret = $state.raw<{ x: number; y: number } | null>(null);

  const edit = $derived(item.edit);
  const selected = $derived(editor.selected === item.id);
  const editing = $derived(editor.editing === item.id);
  const x = $derived(edit.x + (offset?.x ?? 0));
  const y = $derived(edit.y + (offset?.y ?? 0));
  const busy = $derived(offset !== null || resized !== null);
  const pad = $derived(edit.kind === "text" ? TEXT_PADDING * scale : 0);
  // The page draws added text once it has rendered it; until then, and while it moves, the box does.
  const shown = $derived(edit.kind !== "text" || offset !== null || editor.unpainted.includes(edit));

  function type(event: MouseEvent) {
    caret = { x: event.clientX, y: event.clientY };
    editor.editText(item.id);
  }

  function onpointerdown(event: PointerEvent) {
    if (event.button !== 0 || editing) return;
    if (editor.tool === "text" && edit.kind === "text") {
      event.preventDefault();
      (document.activeElement as HTMLElement | null)?.blur();
      type(event);
      return;
    }
    editor.select(item.id);
    begin(event, "move");
  }

  function onresizedown(event: PointerEvent) {
    if (event.button !== 0) return;
    event.stopPropagation();
    event.preventDefault();
    begin(event, "resize");
  }

  function begin(event: PointerEvent, kind: Gesture["kind"]) {
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    gesture = {
      pointer: event.pointerId,
      kind,
      startX: event.clientX,
      startY: event.clientY,
      width: node.offsetWidth / scale,
      height: node.offsetHeight / scale,
      moved: false,
    };
  }

  function onpointermove(event: PointerEvent) {
    if (gesture?.pointer !== event.pointerId) return;
    const dxPx = event.clientX - gesture.startX;
    const dyPx = event.clientY - gesture.startY;
    if (!gesture.moved && Math.hypot(dxPx, dyPx) < DRAG_THRESHOLD_PX) return;
    if (!gesture.moved && edit.kind === "text") editor.hold(item.id);
    gesture.moved = true;
    if (gesture.kind === "move") {
      offset = {
        x: clamp(edit.x + dxPx / scale, 0, pageWidth - gesture.width) - edit.x,
        y: clamp(edit.y + dyPx / scale, 0, pageHeight - gesture.height) - edit.y,
      };
    } else if (edit.kind === "signature") {
      const aspect = edit.width / edit.height;
      const max = Math.min(pageWidth - edit.x, (pageHeight - edit.y) * aspect);
      resized = clamp(edit.width + dxPx / scale, Math.min(MIN_SIGNATURE_WIDTH, max), max);
    }
  }

  function onpointerup(event: PointerEvent) {
    if (gesture?.pointer !== event.pointerId) return;
    if (offset && (offset.x !== 0 || offset.y !== 0)) {
      editor.update(item.id, { x: edit.x + offset.x, y: edit.y + offset.y });
    }
    if (resized !== null && edit.kind === "signature" && resized !== edit.width) {
      editor.update(item.id, { width: resized, height: (resized * edit.height) / edit.width });
    }
    cancel();
  }

  function cancel() {
    // Moving first lets the page draw the box once, where it landed.
    if (gesture?.moved && edit.kind === "text") editor.hold(null);
    gesture = null;
    offset = null;
    resized = null;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="item"
  class:selected
  class:editing
  class:busy
  class:typing={editor.tool === "text" && edit.kind === "text"}
  data-edit
  bind:this={node}
  style:left="{x * scale}px"
  style:top="{y * scale}px"
  style:--pad="{pad}px"
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={cancel}
  ondblclick={(event) => edit.kind === "text" && !editing && type(event)}
>
  {#if edit.kind === "text"}
    {#if editing}
      <RichText
        rows={edit.lines}
        base={edit.lines[0][0]}
        {scale}
        multiline
        below={y * scale < BAR_ROOM_PX}
        {caret}
        onclose={(rows) => editor.finishText(item.id, rows)}
      />
    {:else}
      <TextRows rows={edit.lines} {scale} hidden={!shown} />
    {/if}
  {:else}
    {@const width = resized ?? edit.width}
    <div class="ink" style:width="{width * scale}px" style:height="{(width * edit.height * scale) / edit.width}px">
      <Ink path={edit.path} color={edit.color} />
    </div>
    {#if selected}
      <span class="handle" aria-hidden="true" onpointerdown={onresizedown}></span>
    {/if}
  {/if}

  {#if selected && !busy && !editing}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="bar"
      class:below={y * scale < BAR_ROOM_PX}
      role="toolbar"
      aria-label={edit.kind === "text" ? "Text" : "Signature"}
      tabindex="-1"
      use:roving
      onpointerdown={(event) => event.stopPropagation()}
      onmousedown={(event) => event.preventDefault()}
      ondblclick={(event) => event.stopPropagation()}
    >
      {#if edit.kind === "signature"}
        {#each SIGNATURE_INKS as ink (ink.name)}
          <button
            class="swatch"
            aria-label={ink.name}
            title={ink.name}
            aria-pressed={sameColor(ink.color, edit.color)}
            style:--ink={css(ink.color)}
            onclick={() => editor.update(item.id, { color: ink.color })}
          ></button>
        {/each}
        <span class="divider"></span>
      {/if}
      <button class="tool" aria-label="Delete" title="Delete (Del)" onclick={() => editor.remove(item.id)}>
        <Icon name="trash" size={16} />
      </button>
    </div>
  {/if}
</div>

<style>
  .item {
    position: absolute;
    pointer-events: auto;
    cursor: move;
    user-select: none;
    touch-action: none;
  }

  .item::after {
    position: absolute;
    inset: calc(-1 * var(--pad));
    border-radius: 2px;
    content: "";
    pointer-events: none;
  }

  .item:hover::after {
    outline: 1px dashed color-mix(in srgb, var(--accent) 70%, transparent);
  }

  .selected::after,
  .selected:hover::after {
    outline: 1.5px solid var(--accent);
  }

  .editing::after,
  .editing:hover::after {
    outline: 1.5px dashed var(--accent);
  }

  .typing,
  .editing {
    cursor: text;
  }

  .ink {
    display: block;
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
    bottom: calc(100% + var(--pad) + 0.5rem);
    left: calc(-1 * var(--pad));
    z-index: 2;
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
    top: calc(100% + var(--pad) + 0.5rem);
    bottom: auto;
  }

  .bar .swatch {
    margin: 0 0.25rem;
  }

  .bar .tool {
    min-width: 1.75rem;
    height: 1.75rem;
    padding: 0 0.25rem;
  }

  .divider {
    width: 1px;
    height: 1.25rem;
    margin: 0 0.25rem;
    background: var(--border);
  }
</style>
