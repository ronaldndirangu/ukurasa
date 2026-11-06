<script lang="ts">
  import type { Span, TextLine } from "./api";
  import { editor, lineKey } from "./edits.svelte";
  import { CARET_ROOM, lineBox } from "./lineStyle";
  import RichText from "./RichText.svelte";
  import { charsOf, sameSpans, spansOf } from "./spans";

  interface Props {
    page: number;
    /** CSS pixels per point. */
    scale: number;
  }

  let { page, scale }: Props = $props();

  /** Room the format bar needs above a line before it moves below instead. */
  const BAR_ROOM_PX = 48;

  interface Open {
    line: TextLine;
    /** The text the field opened with. */
    spans: Span[];
    caret: { x: number; y: number };
    /** The token `editor.openLine` gave the field. */
    token: number;
  }

  let lines = $state.raw<TextLine[]>([]);
  let active = $state.raw<Open | null>(null);
  /** The open line's text as typed so far. */
  let spans = $state.raw<Span[]>([]);
  let field = $state<ReturnType<typeof RichText>>();

  const box = $derived(active ? lineBox(active.line, spans, scale, editor.lineWidth(page, active.line)) : null);

  $effect(() => {
    let live = true;
    editor.lines(page).then(
      (found) => {
        if (live) lines = found;
      },
      (failure) => {
        if (live) editor.linesFailed(String(failure));
      },
    );
    return () => {
      live = false;
    };
  });

  function open(event: PointerEvent, line: TextLine) {
    if (event.button !== 0) return;
    event.preventDefault();
    field?.finish();
    const shown = editor.lineSpans(page, line);
    const opened = spansOf(charsOf(shown), line.spans[0]);
    spans = opened;
    active = { line, spans: opened, caret: { x: event.clientX, y: event.clientY }, token: editor.openLine() };
  }

  function change(open: Open, rows: Span[][]) {
    spans = rows[0];
    editor.changeLine(open.token, !sameSpans(rows[0], open.spans));
  }

  function close(open: Open, rows: Span[][], cancelled: boolean) {
    if (active === open) active = null;
    if (editor.closeLine(open.token) && !cancelled) editor.editLine(page, open.line, rows[0]);
  }
</script>

{#each lines as line (lineKey(page, line.runs))}
  {#if line !== active?.line}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="line"
      class:edited={editor.isEdited(page, line)}
      data-edit
      style:left="{line.x * scale - 2}px"
      style:top="{line.y * scale - 1}px"
      style:width="{editor.lineWidth(page, line) * scale + 4}px"
      style:height="{line.height * scale + 2}px"
      onpointerdown={(event) => open(event, line)}
      onmousedown={(event) => event.preventDefault()}
    ></div>
  {/if}
{/each}
{#if active && box}
  {@const opened = active}
  {#key opened.token}
    <div class="editor" data-edit style:left="{box.left}px" style:top="{box.top}px">
      <RichText
        bind:this={field}
        rows={[opened.spans]}
        base={opened.line.spans[0]}
        {scale}
        cover
        fieldStyle={`min-width: ${box.minWidth}px; min-height: ${box.lineHeight}px; ` +
          `margin-left: ${-CARET_ROOM}px; padding: 0 ${CARET_ROOM}px; font: ${box.font}`}
        below={box.top < BAR_ROOM_PX}
        caret={opened.caret}
        onchange={(rows) => change(opened, rows)}
        onclose={(rows, cancelled) => close(opened, rows, cancelled)}
      />
    </div>
  {/key}
{/if}

<style>
  .line {
    position: absolute;
    border-radius: 2px;
    cursor: text;
    pointer-events: auto;
  }

  .line:hover {
    outline: 1px solid color-mix(in srgb, var(--accent) 60%, transparent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  .edited {
    box-shadow: inset 0 -2px 0 color-mix(in srgb, var(--accent) 55%, transparent);
  }

  .editor {
    position: absolute;
    pointer-events: auto;
  }
</style>