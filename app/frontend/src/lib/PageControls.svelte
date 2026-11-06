<script lang="ts">
  import { MOD, type Fit, type FitMode } from "./api";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import { roving } from "./roving";

  const LEVELS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4];

  interface Props {
    page: number;
    pageCount: number;
    zoom: number;
    fit: Fit;
    ongoto: (page: number) => void;
    onzoomin: () => void;
    onzoomout: () => void;
    onzoom: (zoom: number) => void;
    onfit: (mode: FitMode) => void;
  }

  let { page, pageCount, zoom, fit, ongoto, onzoomin, onzoomout, onzoom, onfit }: Props = $props();

  const percent = $derived(Math.round(zoom * 100));

  function commit(event: Event & { currentTarget: HTMLInputElement }) {
    const value = Number(event.currentTarget.value);
    if (Number.isInteger(value) && value >= 1 && value <= pageCount) ongoto(value - 1);
    event.currentTarget.value = String(page + 1);
  }
</script>

{#snippet choice(label: string, checked: boolean, select: () => void, hint?: string)}
  <button role="menuitemradio" aria-checked={checked} onclick={select}>
    <span class="check">
      {#if checked}<Icon name="check" size={16} />{/if}
    </span>
    {label}
    {#if hint}<kbd class="menu-hint">{hint}</kbd>{/if}
  </button>
{/snippet}

<div class="controls" role="toolbar" aria-label="Pages and zoom" data-page-controls use:roving>
  <button
    class="tool"
    aria-label="Previous page"
    title="Previous page"
    disabled={page <= 0}
    onclick={() => ongoto(page - 1)}
  >
    <Icon name="up" size={18} />
  </button>
  <input
    aria-label="Page number"
    inputmode="numeric"
    value={page + 1}
    onchange={commit}
    onfocus={(event) => event.currentTarget.select()}
  />
  <span class="count">of {pageCount}</span>
  <button
    class="tool"
    aria-label="Next page"
    title="Next page"
    disabled={page >= pageCount - 1}
    onclick={() => ongoto(page + 1)}
  >
    <Icon name="down" size={18} />
  </button>
  <span class="divider"></span>
  <button class="tool" aria-label="Zoom out" title="Zoom out ({MOD}-)" onclick={onzoomout}>
    <Icon name="minus" size={18} />
  </button>
  <Menu label="Zoom {percent}%" title="Zoom options" align="end" side="above">
    {#snippet trigger()}
      <span class="percent">{percent}%</span>
    {/snippet}
    {@render choice("Fit width", fit === "width", () => onfit("width"))}
    {@render choice("Fit page", fit === "page", () => onfit("page"))}
    <div role="separator"></div>
    {#each LEVELS as level (level)}
      {@render choice(
        `${Math.round(level * 100)}%`,
        fit === null && Math.abs(zoom - level) < 1e-3,
        () => onzoom(level),
        level === 1 ? `${MOD}0` : undefined,
      )}
    {/each}
  </Menu>
  <button class="tool" aria-label="Zoom in" title="Zoom in ({MOD}+)" onclick={onzoomin}>
    <Icon name="plus" size={18} />
  </button>
</div>

<style>
  /* Clears the scrollbar; `--gutter` is its width. */
  .controls {
    position: absolute;
    right: calc(var(--gutter, 0px) + 1rem);
    bottom: 1rem;
    z-index: 5;
    display: flex;
    gap: 0.125rem;
    align-items: center;
    padding: 0.25rem;
    border: 1px solid var(--border);
    border-radius: 0.625rem;
    background: var(--popover);
    box-shadow: var(--popover-shadow);
  }

  .controls :global(.tool) {
    min-width: 1.875rem;
    height: 1.875rem;
  }

  input {
    width: 2.75rem;
    height: 1.75rem;
    margin: 0 0.125rem 0 0.25rem;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 0.375rem;
    background: var(--field);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .count {
    margin: 0 0.25rem;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .divider {
    flex: none;
    width: 1px;
    height: 1.25rem;
    margin: 0 0.25rem;
    background: var(--border);
  }

  .percent {
    min-width: 2.75rem;
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .check {
    display: inline-flex;
    width: 1rem;
    color: var(--accent);
  }
</style>
