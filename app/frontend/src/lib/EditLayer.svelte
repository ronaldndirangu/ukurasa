<script lang="ts">
  import { editor, isPlaced, lineKey, signatureSize } from "./edits.svelte";
  import EditItem from "./EditItem.svelte";
  import Ink from "./Ink.svelte";
  import { clamp } from "./layout.svelte";
  import LineLayer from "./LineLayer.svelte";
  import { CARET_ROOM, lineBox, spanCss } from "./lineStyle";
  import PictureLayer from "./PictureLayer.svelte";

  interface Props {
    page: number;
    /** Page size in points as displayed. */
    width: number;
    height: number;
    /** CSS pixels per point. */
    scale: number;
  }

  let { page, width, height, scale }: Props = $props();

  let layer: HTMLDivElement;
  let pointer = $state.raw<{ x: number; y: number } | null>(null);

  const items = $derived(editor.items.filter(isPlaced).filter((item) => item.edit.page === page));
  const patches = $derived(editor.patches.filter((patch) => patch.page === page));
  const pending = $derived(editor.tool === "sign" ? editor.signature : null);
  const ghost = $derived(pending && pointer ? place(pointer) : null);
  // Text editing leaves the page itself alone so only the text takes clicks.
  const placing = $derived(editor.tool === "text" || editor.tool === "sign");

  /** A new signature box centred on `at` and kept on the page. */
  function place(at: { x: number; y: number }) {
    const size = signatureSize(pending!.path);
    return {
      ...size,
      x: clamp(at.x - size.width / 2, 0, width - size.width),
      y: clamp(at.y - size.height / 2, 0, height - size.height),
    };
  }

  function locate(event: PointerEvent) {
    const rect = layer.getBoundingClientRect();
    return { x: (event.clientX - rect.left) / scale, y: (event.clientY - rect.top) / scale };
  }

  function onpointerdown(event: PointerEvent) {
    if (event.button !== 0 || event.target !== layer || !placing) return;
    event.preventDefault();
    (document.activeElement as HTMLElement | null)?.blur();
    const at = locate(event);
    if (editor.tool === "text") {
      editor.startText(page, at, { width, height });
    } else if (pending) {
      editor.placeSignature({ kind: "signature", page, ...place(at), path: pending.path, color: pending.color });
      pointer = null;
    }
  }

  // Keeps the scroller from taking focus away from a new text box.
  function onmousedown(event: MouseEvent) {
    if (event.target === layer && placing) event.preventDefault();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="layer"
  class:armed={placing}
  class:text={editor.tool === "text"}
  bind:this={layer}
  {onpointerdown}
  {onmousedown}
  onpointermove={(event) => (pointer = pending ? locate(event) : null)}
  onpointerleave={() => (pointer = null)}
>
  {#each patches as { line, spans } (lineKey(page, line.runs))}
    {@const box = lineBox(line, spans, scale, editor.lineWidth(page, line))}
    <div
      class="patch"
      style:left="{box.left}px"
      style:top="{box.top}px"
      style:min-width="{box.minWidth}px"
      style:min-height="{box.lineHeight}px"
      style:margin-left="{-CARET_ROOM}px"
      style:padding="0 {CARET_ROOM}px"
      style:font={box.font}
    >{#each spans as span, i (i)}<span style={spanCss(span, scale)}>{span.text}</span>{/each}</div>
  {/each}
  {#if editor.awake.has(page)}
    <PictureLayer {page} {width} {scale} />
    {#if editor.tool === "none"}
      <LineLayer {page} {scale} />
    {/if}
  {/if}
  {#each items as item (item.id)}
    <EditItem {item} {scale} pageWidth={width} pageHeight={height} />
  {/each}
  {#if ghost && pending}
    <div
      class="ghost"
      style:left="{ghost.x * scale}px"
      style:top="{ghost.y * scale}px"
      style:width="{ghost.width * scale}px"
      style:height="{ghost.height * scale}px"
    >
      <Ink path={pending.path} color={pending.color} />
    </div>
  {/if}
</div>

<style>
  .layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .armed {
    pointer-events: auto;
    cursor: crosshair;
  }

  .text {
    cursor: text;
  }

  .ghost {
    position: absolute;
    opacity: 0.55;
    outline: 1px dashed var(--accent);
    pointer-events: none;
  }

  /* Matches the text field in LineLayer so finishing an edit doesn't shift the text. */
  .patch {
    position: absolute;
    box-sizing: content-box;
    width: max-content;
    background: #fff;
    white-space: pre;
    pointer-events: none;
  }
</style>
