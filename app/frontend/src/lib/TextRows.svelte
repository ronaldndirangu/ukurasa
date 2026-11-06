<script lang="ts">
  import type { Span } from "./api";
  import { LINE_HEIGHT } from "./edits.svelte";
  import { spanCss } from "./lineStyle";

  interface Props {
    rows: Span[][];
    /** CSS pixels per point. */
    scale: number;
    /** Keeps the text's room without showing it, such as when the page draws it. */
    hidden?: boolean;
  }

  let { rows, scale, hidden = false }: Props = $props();
</script>

<!-- Matches the rows of RichText so finishing an edit doesn't shift the text. -->
<div class="rows" class:hidden>
  {#each rows as row, r (r)}
    {#if row.some((span) => span.text)}
      <div class="row">{#each row as span, i (i)}{#if span.text}<span style={spanCss(span, scale, LINE_HEIGHT)}>{span.text}</span>{/if}{/each}</div>
    {:else if row[0]}
      <div class="row" style={spanCss(row[0], scale, LINE_HEIGHT)}><br /></div>
    {/if}
  {/each}
</div>

<style>
  .rows {
    width: max-content;
    min-width: 2px;
    font-size: 0;
  }

  .row {
    white-space: pre;
  }

  .hidden {
    visibility: hidden;
  }
</style>
