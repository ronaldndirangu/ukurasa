<script lang="ts" module>
  export type SidebarTab = "pages" | "outline";
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import type { DocumentView } from "./api";
  import Outline from "./Outline.svelte";
  import { roving } from "./roving";
  import Thumbnails from "./Thumbnails.svelte";

  interface Props {
    doc: DocumentView;
    page: number;
    tab: SidebarTab;
    ongoto: (index: number) => void;
  }

  let { doc, page, tab = $bindable(), ongoto }: Props = $props();

  // A single top-level entry is usually the document title, so its sections start expanded.
  const expanded = new SvelteSet<number>(
    untrack(() => (doc.outline.length === 1 && doc.outline[0].children?.length ? [0] : [])),
  );

  const hasOutline = $derived(doc.outline.length > 0);
  const current = $derived(hasOutline ? tab : "pages");
  const tabs = $derived<[SidebarTab, string][]>(
    hasOutline
      ? [
          ["pages", "Pages"],
          ["outline", "Outline"],
        ]
      : [["pages", "Pages"]],
  );
</script>

<aside aria-label="Sidebar">
  <div class="tabs" role="tablist" use:roving>
    {#each tabs as [id, label] (id)}
      <button
        role="tab"
        id="sidebar-tab-{id}"
        aria-selected={current === id}
        aria-controls="sidebar-panel"
        onclick={() => (tab = id)}
      >
        {label}
      </button>
    {/each}
  </div>
  <div class="panel" role="tabpanel" id="sidebar-panel" aria-labelledby="sidebar-tab-{current}">
    {#if current === "pages"}
      <Thumbnails {doc} {page} {ongoto} />
    {:else}
      <Outline outline={doc.outline} {page} {expanded} {ongoto} />
    {/if}
  </div>
</aside>

<style>
  aside {
    display: flex;
    flex: none;
    flex-direction: column;
    width: 15rem;
    border-right: 1px solid var(--border);
    background: var(--sidebar);
  }

  .tabs {
    display: flex;
    gap: 0.125rem;
    margin: 0.5rem 0.5rem 0;
    padding: 0.1875rem;
    border-radius: 0.5rem;
    background: var(--hover);
  }

  [role="tab"] {
    flex: 1;
    height: 1.75rem;
    border: 0;
    border-radius: 0.375rem;
    background: none;
    color: var(--muted);
    font-size: 0.875rem;
    font-weight: 600;
    cursor: pointer;
  }

  [role="tab"][aria-selected="true"] {
    background: var(--surface);
    box-shadow: 0 1px 2px rgb(15 23 42 / 0.12);
    color: inherit;
  }

  .panel {
    flex: 1;
    min-height: 0;
  }
</style>
