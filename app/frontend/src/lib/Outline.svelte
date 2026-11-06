<script lang="ts">
  import { tick } from "svelte";
  import type { SvelteSet } from "svelte/reactivity";
  import type { OutlineItem } from "./api";
  import Icon from "./Icon.svelte";

  interface Props {
    outline: OutlineItem[];
    page: number;
    /** Ids of expanded entries; owned by the caller so it survives tab switches. */
    expanded: SvelteSet<number>;
    ongoto: (index: number) => void;
  }

  let { outline, page, expanded, ongoto }: Props = $props();

  interface Node {
    title: string;
    page: number | null;
    depth: number;
    parent: number;
    /** One past the id of the last descendant. */
    end: number;
  }

  let tree: HTMLDivElement;
  let focused = $state(-1);

  const nodes = $derived(flatten(outline));

  const visible = $derived.by(() => {
    const ids: number[] = [];
    for (let id = 0; id < nodes.length; id = expanded.has(id) ? id + 1 : nodes[id].end) ids.push(id);
    return ids;
  });

  /** The entry for the furthest section at or before the current page, or its nearest shown ancestor. */
  const active = $derived.by(() => {
    let best = -1;
    for (let id = 0; id < nodes.length; id++) {
      const target = nodes[id].page;
      if (target !== null && target <= page && (best < 0 || target >= (nodes[best].page ?? 0))) best = id;
    }
    while (best >= 0 && !shown(best)) best = nodes[best].parent;
    return best;
  });

  const tabStop = $derived(visible.includes(focused) ? focused : active >= 0 ? active : (visible[0] ?? -1));

  $effect(() => {
    const id = active;
    if (id < 0) return;
    const row = tree.querySelector<HTMLElement>(`[data-id="${id}"]`);
    if (!row) return;
    const top = row.offsetTop;
    const bottom = top + row.offsetHeight;
    if (top < tree.scrollTop) tree.scrollTop = top - 8;
    else if (bottom > tree.scrollTop + tree.clientHeight) tree.scrollTop = bottom - tree.clientHeight + 8;
  });

  function flatten(items: OutlineItem[]): Node[] {
    const out: Node[] = [];
    const walk = (list: OutlineItem[], depth: number, parent: number) => {
      for (const item of list) {
        const node: Node = { title: item.title || "Untitled", page: item.page, depth, parent, end: 0 };
        const id = out.push(node) - 1;
        if (item.children) walk(item.children, depth + 1, id);
        node.end = out.length;
      }
    };
    walk(items, 0, -1);
    return out;
  }

  function shown(id: number): boolean {
    for (let at = nodes[id].parent; at >= 0; at = nodes[at].parent) if (!expanded.has(at)) return false;
    return true;
  }

  function isBranch(id: number): boolean {
    return nodes[id].end > id + 1;
  }

  function toggle(id: number) {
    if (expanded.has(id)) expanded.delete(id);
    else expanded.add(id);
  }

  function select(id: number) {
    const target = nodes[id].page;
    if (target !== null) ongoto(target);
  }

  async function focus(id: number | undefined) {
    if (id === undefined || id < 0) return;
    focused = id;
    await tick();
    tree.querySelector<HTMLElement>(`[data-id="${id}"] .entry`)?.focus();
  }

  function onkeydown(event: KeyboardEvent, id: number) {
    const at = visible.indexOf(id);
    switch (event.key) {
      case "ArrowDown":
        focus(visible[at + 1]);
        break;
      case "ArrowUp":
        focus(visible[at - 1]);
        break;
      case "Home":
        focus(visible[0]);
        break;
      case "End":
        focus(visible[visible.length - 1]);
        break;
      case "ArrowRight":
        if (!isBranch(id)) return;
        if (expanded.has(id)) focus(id + 1);
        else expanded.add(id);
        break;
      case "ArrowLeft":
        if (expanded.has(id)) expanded.delete(id);
        else focus(nodes[id].parent);
        break;
      default:
        return;
    }
    event.preventDefault();
  }
</script>

<div class="tree" bind:this={tree}>
  {#each visible as id (id)}
    {@const node = nodes[id]}
    {@const branch = isBranch(id)}
    <div class="row" class:active={id === active} style:padding-left="{node.depth * 14 + 4}px" data-id={id}>
      {#if branch}
        <button
          class="twisty"
          class:open={expanded.has(id)}
          tabindex="-1"
          aria-label={expanded.has(id) ? "Collapse" : "Expand"}
          onclick={() => toggle(id)}
        >
          <Icon name="right" size={14} />
        </button>
      {:else}
        <span class="twisty"></span>
      {/if}
      <button
        class="entry"
        title={node.title}
        aria-expanded={branch ? expanded.has(id) : undefined}
        aria-current={id === active ? "location" : undefined}
        aria-disabled={node.page === null}
        tabindex={id === tabStop ? 0 : -1}
        onclick={() => select(id)}
        onfocus={() => (focused = id)}
        onkeydown={(event) => onkeydown(event, id)}
      >
        <span class="title">{node.title}</span>
        {#if node.page !== null}
          <span class="number">{node.page + 1}</span>
        {/if}
      </button>
    </div>
  {/each}
</div>

<style>
  .tree {
    position: relative;
    height: 100%;
    overflow-y: auto;
    padding: 0.375rem;
    box-sizing: border-box;
    scrollbar-width: thin;
  }

  .row {
    display: flex;
    align-items: center;
    border-radius: 0.375rem;
  }

  .row:hover {
    background: var(--hover);
  }

  .row.active {
    background: var(--selected);
    color: var(--selected-text);
  }

  .twisty {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    padding: 0;
    border: 0;
    border-radius: 0.25rem;
    background: none;
    color: var(--muted);
    cursor: pointer;
  }

  button.twisty:hover {
    background: var(--hover);
  }

  .twisty :global(svg) {
    transition: transform 120ms;
  }

  .open :global(svg) {
    transform: rotate(90deg);
  }

  .entry {
    display: flex;
    flex: 1;
    gap: 0.5rem;
    align-items: baseline;
    min-width: 0;
    padding: 0.3125rem 0.375rem 0.3125rem 0.125rem;
    border: 0;
    border-radius: 0.375rem;
    background: none;
    text-align: left;
    cursor: pointer;
  }

  .entry[aria-disabled="true"] {
    color: var(--muted);
    cursor: default;
  }

  .title {
    flex: 1;
    overflow: hidden;
    line-height: 1.3;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .number {
    flex: none;
    color: var(--muted);
    font-size: 0.875rem;
    font-variant-numeric: tabular-nums;
  }

  .active .number {
    color: inherit;
  }

  @media (prefers-reduced-motion: reduce) {
    .twisty :global(svg) {
      transition: none;
    }
  }
</style>
