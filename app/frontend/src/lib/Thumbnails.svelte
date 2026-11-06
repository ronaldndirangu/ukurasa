<script lang="ts">
  import { tick, untrack } from "svelte";
  import type { DocumentView } from "./api";
  import { editor } from "./edits.svelte";
  import { THUMB_WIDTH, display, indexAt, range } from "./layout.svelte";
  import PageImage from "./PageImage.svelte";

  const PAD = 14;
  const LABEL = 24;
  const GAP = 12;
  const OVERSCAN = 3;
  const PRIORITY = 4;

  interface Props {
    doc: DocumentView;
    page: number;
    ongoto: (index: number) => void;
  }

  let { doc, page, ongoto }: Props = $props();

  let list: HTMLDivElement;
  let scrollTop = $state(0);
  let height = $state(0);
  let focusIndex = $state(untrack(() => page));

  const layout = $derived.by(() => {
    const count = doc.pages.length;
    const tops = new Float64Array(count);
    const heights = new Float64Array(count);
    let y = PAD;
    for (let i = 0; i < count; i++) {
      const size = doc.pages[i];
      heights[i] = Math.max(1, Math.round((THUMB_WIDTH * size.height) / size.width));
      tops[i] = y;
      y += heights[i] + LABEL + GAP;
    }
    return { tops, heights, height: y - GAP + PAD };
  });

  const mounted = $derived(
    range(indexAt(layout.tops, scrollTop), indexAt(layout.tops, scrollTop + height), OVERSCAN, doc.pages.length),
  );

  $effect(() => {
    const index = page;
    untrack(() => {
      focusIndex = index;
      reveal(index);
    });
  });

  /** Centers the thumbnail when any part of it is out of view. */
  function reveal(index: number) {
    const top = layout.tops[index];
    const bottom = top + layout.heights[index] + LABEL;
    if (top >= list.scrollTop && bottom <= list.scrollTop + list.clientHeight) return;
    list.scrollTop = (top + bottom - list.clientHeight) / 2;
    scrollTop = list.scrollTop;
  }

  async function onkeydown(event: KeyboardEvent) {
    const last = doc.pages.length - 1;
    const steps: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, Home: -last, End: last };
    const step = steps[event.key];
    if (step === undefined) return;
    event.preventDefault();
    const target = Math.min(Math.max(focusIndex + step, 0), last);
    focusIndex = target;
    ongoto(target);
    reveal(target);
    await tick();
    list.querySelector<HTMLElement>(`[data-index="${target}"]`)?.focus();
  }
</script>

<div
  class="list"
  bind:this={list}
  bind:clientHeight={height}
  onscroll={() => (scrollTop = list.scrollTop)}
>
  <div class="content" style:height="{layout.height}px">
    {#each mounted as index (index)}
      <button
        class="thumb"
        class:current={index === page}
        style:top="{layout.tops[index]}px"
        style:width="{THUMB_WIDTH}px"
        aria-label="Page {index + 1}"
        aria-current={index === page ? "page" : undefined}
        tabindex={index === focusIndex ? 0 : -1}
        data-index={index}
        onclick={() => ongoto(index)}
        {onkeydown}
      >
        <span class="frame" style:height="{layout.heights[index]}px">
          <PageImage
            doc={doc.id}
            {index}
            renderWidth={display.thumbRenderWidth}
            priority={PRIORITY}
            revision={editor.revisions.get(index) ?? 0}
          />
        </span>
        <span class="label">{index + 1}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .list {
    height: 100%;
    overflow-y: auto;
    scrollbar-width: thin;
  }

  .content {
    position: relative;
  }

  .thumb {
    position: absolute;
    left: 50%;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    transform: translateX(-50%);
  }

  .thumb:focus-visible {
    outline: none;
  }

  .frame {
    position: relative;
    display: block;
    border-radius: 2px;
    box-shadow: var(--page-shadow);
    transition: box-shadow 120ms;
  }

  .thumb:hover .frame {
    box-shadow:
      0 0 0 2px var(--border),
      var(--page-shadow);
  }

  .current .frame,
  .current:hover .frame {
    box-shadow:
      0 0 0 2px var(--accent),
      var(--page-shadow);
  }

  .thumb:focus-visible .frame {
    box-shadow:
      0 0 0 2px var(--surface),
      0 0 0 4px var(--accent);
  }

  .label {
    display: block;
    height: 24px;
    padding-top: 5px;
    box-sizing: border-box;
    color: var(--muted);
    font-size: 0.875rem;
    font-variant-numeric: tabular-nums;
    line-height: 1;
    text-align: center;
  }

  .current .label {
    color: var(--selected-text);
    font-weight: 600;
  }
</style>
