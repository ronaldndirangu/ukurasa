<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { MOD, PLATFORM, type RecentFile } from "./api";
  import { editor } from "./edits.svelte";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import { roving } from "./roving";
  import WindowControls from "./WindowControls.svelte";

  const SAVE_AS = PLATFORM === "macos" ? "⇧⌘S" : "Ctrl+Shift+S";
  /** Windows and macOS draw no title bar of their own, so this one shows the name and moves the window. */
  const CUSTOM = PLATFORM !== "linux";

  interface Props {
    name: string | null;
    /** Horizontal center of the document area in CSS px, so the name sits over the page. */
    center: number | null;
    ready: boolean;
    /** An in-place save just finished. */
    saved: boolean;
    recent: RecentFile[];
    sidebarOpen: boolean;
    onsidebar: () => void;
    onopen: () => void;
    onopenrecent: (path: string) => void;
    onsave: () => void;
    onsaveas: () => void;
    onsign: () => void;
  }

  let {
    name,
    center,
    ready,
    saved,
    recent,
    sidebarOpen,
    onsidebar,
    onopen,
    onopenrecent,
    onsave,
    onsaveas,
    onsign,
  }: Props = $props();

  const announcement = $derived(editor.saving ? "Saving" : saved ? "Saved" : "");

  let active = $state(document.hasFocus());
  let width = $state(0);
  // Padding one side of the bar by twice the offset centers the middle column on `center`.
  const shift = $derived(center === null || width === 0 ? 0 : Math.round(2 * center - width));

  // Keeps focus in a field being typed in; a disabled button would otherwise blur it and
  // enable itself before the click lands.
  const keepFocus = (event: PointerEvent) => event.preventDefault();

  function drag(event: MouseEvent) {
    if (!CUSTOM || event.button !== 0) return;
    if ((event.target as Element).closest("button, input, a, [role='menu']")) return;
    const appWindow = getCurrentWindow();
    if (event.detail === 2) appWindow.toggleMaximize();
    else appWindow.startDragging();
  }
</script>

<svelte:window onfocus={() => (active = true)} onblur={() => (active = false)} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  class="titlebar"
  class:windows={PLATFORM === "windows"}
  class:mac={PLATFORM === "macos"}
  class:inactive={!active}
  style:--lead="{Math.max(0, shift)}px"
  style:--trail="{Math.max(0, -shift)}px"
  bind:clientWidth={width}
  onmousedown={drag}
>
  <div class="start" role="toolbar" aria-label="Document" use:roving>
    {#if ready}
      <button
        class="tool sidebar"
        aria-label="Sidebar"
        title="Sidebar"
        aria-pressed={sidebarOpen}
        onclick={onsidebar}
      >
        <Icon name="sidebar" />
      </button>
    {/if}
    <Menu label="File">
      {#snippet trigger()}
        <Icon name="open" />
        <span class="label file">File</span>
      {/snippet}
      <button role="menuitem" onclick={onopen}>
        <Icon name="open" size={16} />
        Open…
        <kbd class="menu-hint">{MOD}O</kbd>
      </button>
      <button role="menuitem" disabled={!editor.dirty || editor.saving} onclick={onsave}>
        <Icon name="save" size={16} />
        Save
        <kbd class="menu-hint">{MOD}S</kbd>
      </button>
      <button role="menuitem" disabled={!ready || editor.saving} onclick={onsaveas}>
        <Icon name="file" size={16} />
        Save as…
        <kbd class="menu-hint">{SAVE_AS}</kbd>
      </button>
      {#if recent.length > 0}
        <div role="separator"></div>
        <div class="menu-heading">Recent</div>
        {#each recent as file (file.path)}
          <button
            role="menuitem"
            class="recent"
            class:missing={file.missing}
            title={file.missing ? `${file.path} (not found)` : file.path}
            onclick={() => onopenrecent(file.path)}
          >
            <Icon name="clock" size={16} />
            <span class="truncate">{file.name}</span>
          </button>
        {/each}
      {/if}
    </Menu>
    {#if ready}
      <span class="divider"></span>
      <button
        class="tool"
        aria-label="Undo"
        title="Undo ({MOD}Z)"
        disabled={!editor.canUndo}
        onpointerdown={keepFocus}
        onclick={() => editor.undo()}
      >
        <Icon name="undo" />
      </button>
      <button
        class="tool"
        aria-label="Redo"
        title="Redo ({MOD}Y)"
        disabled={!editor.canRedo}
        onpointerdown={keepFocus}
        onclick={() => editor.redo()}
      >
        <Icon name="redo" />
      </button>
    {/if}
  </div>

  <div class="title">
    {#if CUSTOM}
      <span class="name" title={name ?? undefined}>{name ?? "Ukurasa"}</span>
    {/if}
    {#if ready && (editor.dirty || editor.saving)}
      <button
        class="save"
        aria-label={editor.saving ? "Saving" : "Save, edits aren't saved"}
        title={`Save (${MOD}S). Your edits aren't saved yet`}
        disabled={editor.saving}
        onclick={onsave}
      >
        {#if editor.saving}
          <span class="spinner"></span>
          Saving…
        {:else}
          Save
        {/if}
      </button>
    {:else if ready && saved}
      <span class="save done"><Icon name="check" size={16} />Saved</span>
    {/if}
    <span class="sr-only" aria-live="polite">{announcement}</span>
  </div>

  <div class="end">
    {#if ready}
      <div class="group" role="toolbar" aria-label="Edit" use:roving>
        <button
          class="tool"
          title="Add text"
          aria-pressed={editor.tool === "text"}
          onclick={() => editor.arm("text")}
        >
          <Icon name="text" />
          <span class="label">Add text</span>
        </button>
        <button class="tool" title="Sign" aria-pressed={editor.tool === "sign"} onclick={onsign}>
          <Icon name="sign" />
          <span class="label">Sign</span>
        </button>
      </div>
    {/if}
    {#if PLATFORM === "windows"}
      <WindowControls />
    {/if}
  </div>
</header>

<style>
  .titlebar {
    display: grid;
    grid-template-columns:
      var(--lead) minmax(max-content, 1fr) minmax(0, auto) minmax(max-content, 1fr)
      var(--trail);
    gap: 0.5rem;
    align-items: center;
    height: 40px;
    background: var(--chrome);
    color: var(--chrome-text);
    user-select: none;
  }

  .start {
    grid-column: 1 / 3;
    padding-left: 0.375rem;
  }

  .mac .start {
    padding-left: 78px;
  }

  .start,
  .end,
  .group {
    display: flex;
    gap: 0.125rem;
    align-items: center;
    height: 100%;
    white-space: nowrap;
  }

  .end {
    grid-column: 4 / 6;
    justify-content: flex-end;
  }

  .titlebar:not(.windows) .end {
    padding-right: 0.375rem;
  }

  .titlebar :global(.tool:hover:not(:disabled)),
  .titlebar :global(.tool[aria-expanded="true"]) {
    background: var(--chrome-hover);
  }

  .titlebar :global(.tool:active:not(:disabled)) {
    background: var(--chrome-active);
  }

  .titlebar :global(.tool[aria-pressed="true"]) {
    background: rgb(124 155 255 / 0.24);
    color: #c9d5ff;
  }

  /* The open sidebar already shows this toggle's state. */
  .titlebar .sidebar[aria-pressed="true"]:not(:hover) {
    background: none;
    color: inherit;
  }

  .titlebar :global(:focus-visible) {
    outline-color: #7c9bff;
  }

  .divider {
    flex: none;
    width: 1px;
    height: 1.25rem;
    margin: 0 0.25rem;
    background: var(--chrome-border);
  }

  .tool:has(.label) {
    padding-right: 0.625rem;
  }

  /* Matches the end padding of the labeled tools; the trigger button belongs to Menu. */
  .file {
    padding-right: 0.25rem;
  }

  @media (max-width: 960px) {
    .label {
      display: none;
    }

    .tool:has(.label) {
      padding-right: 0.375rem;
    }
  }

  .title {
    display: grid;
    grid-template-columns: 1fr minmax(0, auto) 1fr;
    gap: 0.625rem;
    align-items: center;
    min-width: 0;
  }

  .name {
    grid-column: 2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The empty first column mirrors this one, so the name stays put when Save appears. */
  .save {
    display: inline-flex;
    grid-column: 3;
    gap: 0.375rem;
    align-items: center;
    justify-self: start;
    height: 1.75rem;
    padding: 0 0.875rem;
    border: none;
    border-radius: 0.375rem;
    background: var(--accent);
    color: var(--accent-text);
    font: inherit;
    font-size: 0.875rem;
    font-weight: 600;
    white-space: nowrap;
  }

  .done {
    padding-left: 0.625rem;
    background: none;
    color: var(--chrome-muted);
    font-weight: 500;
    animation: fade 1.2s ease-in forwards;
  }

  .done :global(svg) {
    color: #8fd19e;
  }

  @keyframes fade {
    0%,
    60% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .done {
      animation: none;
    }
  }

  .title:not(:has(.name)) .save {
    grid-column: 2;
  }

  button.save:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 85%, white);
  }

  button.save:active:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 85%, black);
  }

  .save:focus-visible {
    outline: 2px solid #c9d5ff;
    outline-offset: 2px;
  }

  .save:disabled {
    opacity: 0.85;
  }

  .save .spinner {
    border-color: rgb(255 255 255 / 0.35);
    border-top-color: var(--accent-text);
  }

  .inactive .title,
  .inactive :global(.caption) {
    color: var(--chrome-muted);
  }

  .recent {
    max-width: 24rem;
  }

  .missing {
    color: var(--muted);
  }

  .truncate {
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
