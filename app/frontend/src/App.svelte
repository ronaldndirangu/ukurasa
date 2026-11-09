<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import {
    forgetRecent,
    getSession,
    onSession,
    openDialog,
    openPath,
    saveCopy,
    saveDocument,
    setUnsaved,
    type Edit,
    type Fit,
    type Session,
  } from "./lib/api";
  import { editor, isPlaced } from "./lib/edits.svelte";
  import Icon from "./lib/Icon.svelte";
  import { clamp } from "./lib/layout.svelte";
  import PageControls from "./lib/PageControls.svelte";
  import Sidebar, { type SidebarTab } from "./lib/Sidebar.svelte";
  import SignatureDialog from "./lib/SignatureDialog.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import Viewer, { type Position } from "./lib/Viewer.svelte";
  import Welcome from "./lib/Welcome.svelte";

  const SIDEBAR_KEY = "ukurasa.sidebar";
  const TAB_KEY = "ukurasa.sidebarTab";
  const NOTICE_MS = 3000;
  const SAVED_MS = 1200;
  /** A line preview shows progress only when it takes longer than this. */
  const SLOW_PREVIEW_MS = 400;
  /** Below this page-area width, a centered toast would run into the page controls. */
  const CROWDED_PX = 1280;

  let session = $state.raw<Session>({ revision: -1, loading: null, document: null, error: null, recent: [] });
  let started = $state(false);
  let dismissed = $state(-1);
  let dragging = $state(false);
  let viewer = $state<ReturnType<typeof Viewer>>();
  let page = $state(0);
  let zoom = $state(1);
  let fit = $state<Fit>(null);
  let sidebarOpen = $state(localStorage.getItem(SIDEBAR_KEY) !== "closed");
  let sidebarTab = $state<SidebarTab>(localStorage.getItem(TAB_KEY) === "outline" ? "outline" : "pages");
  let signing = $state(false);
  let notice = $state<string | null>(null);
  /** An in-place save just finished, so the title bar confirms it. */
  let saved = $state(false);
  let saveError = $state<string | null>(null);
  let slowPreview = $state(false);
  let start = $state.raw<Position | null>(null);
  let docCenter = $state<number | null>(null);
  let docWidth = $state(0);
  // The view and tool to keep when a save reopens the document.
  let restore: { position: Position | null; zoom: number; fit: Fit; tool: typeof editor.tool } | null = null;

  const doc = $derived(session.document);
  const ready = $derived(doc !== null && doc.pages.length > 0);
  const statusLeft = $derived(docCenter === null ? undefined : `${docCenter}px`);
  const raised = $derived(ready && docWidth < CROWDED_PX);
  const error = $derived(session.revision > dismissed ? session.error : null);
  const hint = $derived(
    editor.tool === "text"
      ? "Click on a page to add text"
      : editor.tool === "sign"
        ? "Click on a page to place your signature"
        : editor.picture !== null
          ? "Drag the picture to move it, or its corner to resize it"
          : null,
  );

  $effect(() => localStorage.setItem(SIDEBAR_KEY, sidebarOpen ? "open" : "closed"));
  $effect(() => localStorage.setItem(TAB_KEY, sidebarTab));
  $effect(() => {
    setUnsaved(editor.dirty).catch(() => {});
  });
  $effect(() => {
    if (!notice) return;
    const timer = setTimeout(() => (notice = null), NOTICE_MS);
    return () => clearTimeout(timer);
  });
  $effect(() => {
    if (!saved) return;
    const timer = setTimeout(() => (saved = false), SAVED_MS);
    return () => clearTimeout(timer);
  });
  $effect(() => {
    if (!editor.notice) return;
    notice = editor.notice;
    editor.notice = null;
  });
  $effect(() => {
    if (!editor.previewing) {
      slowPreview = false;
      return;
    }
    const timer = setTimeout(() => (slowPreview = true), SLOW_PREVIEW_MS);
    return () => clearTimeout(timer);
  });

  onMount(() => {
    const unlisten = [
      onSession(apply),
      getCurrentWebview().onDragDropEvent(({ payload }) => {
        dragging = payload.type === "enter" || payload.type === "over";
      }),
    ];
    getSession()
      .then(apply)
      .finally(() => (started = true));
    return () => unlisten.forEach((pending) => pending.then((stop) => stop()));
  });

  function apply(next: Session) {
    if (next.revision <= session.revision) return;
    if (next.document?.id !== session.document?.id) {
      editor.reset(next.document?.id ?? null);
      signing = false;
      if (restore && next.document) {
        start = restore.position;
        page = restore.position?.page ?? 0;
        zoom = restore.zoom;
        fit = restore.fit;
        editor.tool = restore.tool;
      } else {
        start = null;
        page = 0;
        zoom = 1;
        fit = null;
      }
      restore = null;
    }
    session = next;
  }

  const goTo = (index: number) => viewer?.goToPage(index);

  /** Saves over the open file when `copy` is false, otherwise asks where to save a copy. */
  async function save(copy: boolean) {
    if (!doc || editor.saving) return;
    // Commits a text box that is still being typed in.
    (document.activeElement as HTMLElement | null)?.blur();
    if (!copy && !editor.dirty) return;
    editor.saving = true;
    saveError = null;
    // A signature waiting to be placed does not survive the reopen, so the sign tool is dropped.
    const tool = editor.tool === "sign" ? "none" : editor.tool;
    restore = { position: viewer?.position() ?? null, zoom, fit, tool };
    const edits: Edit[] = editor.items.map((item) => item.edit);
    try {
      const name = await (copy ? saveCopy(doc.id, edits) : saveDocument(doc.id, edits));
      if (name === null) restore = null;
      else if (copy) notice = `Saved ${name}`;
      else saved = true;
    } catch (failure) {
      restore = null;
      saveError = String(failure);
    } finally {
      editor.saving = false;
    }
  }

  function sign() {
    if (editor.tool === "sign") editor.arm("sign");
    else signing = true;
  }

  function nudge(dx: number, dy: number): boolean {
    const item = editor.get(editor.selected);
    const size = item && doc?.pages[item.edit.page];
    if (!item || !isPlaced(item) || !size) return false;
    const { edit } = item;
    const [width, height] = edit.kind === "signature" ? [edit.width, edit.height] : [0, 0];
    editor.update(item.id, {
      x: clamp(edit.x + dx, 0, size.width - width),
      y: clamp(edit.y + dy, 0, size.height - height),
    });
    return true;
  }

  function typing(target: EventTarget | null): boolean {
    return target instanceof HTMLElement && (target.isContentEditable || target.matches("input, textarea, select"));
  }

  /** The key belongs to the title bar, sidebar, page controls, or a menu rather than the page. */
  function elsewhere(event: KeyboardEvent): boolean {
    const target = event.target;
    if (event.defaultPrevented) return true;
    if (!(target instanceof Element)) return false;
    // Escape still puts away a tool just armed from the title bar, where focus stays.
    if (event.key === "Escape" && editor.tool !== "none" && target.closest("header")) return false;
    return target.closest("header, aside, [role='menu'], [data-page-controls]") !== null;
  }

  function onkeydown(event: KeyboardEvent) {
    if (signing || event.altKey) return;
    const inField = typing(event.target);
    if (event.ctrlKey || event.metaKey) {
      switch (event.key.toLowerCase()) {
        case "o":
          openDialog();
          break;
        case "s":
          save(event.shiftKey);
          break;
        case "z":
          if (inField) return;
          if (event.shiftKey) editor.redo();
          else editor.undo();
          break;
        case "y":
          if (inField) return;
          editor.redo();
          break;
        case "=":
        case "+":
          viewer?.zoomIn();
          break;
        case "-":
          viewer?.zoomOut();
          break;
        case "0":
          viewer?.zoomTo(1);
          break;
        default:
          return;
      }
      event.preventDefault();
      return;
    }
    if (inField || !ready || elsewhere(event)) return;
    const step = event.shiftKey ? 10 : 1;
    const selected = editor.get(editor.selected);
    switch (event.key) {
      case "Escape":
        if (!editor.escape()) return;
        break;
      case "Delete":
      case "Backspace":
        if (!selected) return;
        editor.remove(selected.id);
        break;
      case "Enter":
        if (selected?.edit.kind !== "text") return;
        editor.editText(selected.id);
        break;
      case "ArrowLeft":
        if (!nudge(-step, 0)) return;
        break;
      case "ArrowRight":
        if (!nudge(step, 0)) return;
        break;
      case "ArrowUp":
        if (!nudge(0, -step)) return;
        break;
      case "ArrowDown":
        if (!nudge(0, step)) return;
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  /** Follows the center of the area the pages are centered in, excluding its scrollbar. */
  function trackCenter(node: HTMLElement) {
    const scroller = node.querySelector<HTMLElement>(".scroller") ?? node;
    const observer = new ResizeObserver(() => {
      node.style.setProperty("--gutter", `${node.clientWidth - scroller.clientWidth}px`);
      docWidth = scroller.clientWidth;
      docCenter = node.getBoundingClientRect().left + scroller.clientWidth / 2;
    });
    observer.observe(node);
    observer.observe(scroller);
    return {
      destroy() {
        observer.disconnect();
        docCenter = null;
        docWidth = 0;
      },
    };
  }
</script>

<svelte:window {onkeydown} />

<div class="app">
  <TitleBar
    name={doc?.name ?? null}
    center={docCenter}
    {ready}
    {saved}
    recent={session.recent}
    {sidebarOpen}
    onsidebar={() => (sidebarOpen = !sidebarOpen)}
    onopen={openDialog}
    onopenrecent={openPath}
    onsave={() => save(false)}
    onsaveas={() => save(true)}
    onsign={sign}
  />

  {#if error}
    <div class="banner" role="alert">
      <span>{error}</span>
      <button class="tool" aria-label="Dismiss" title="Dismiss" onclick={() => (dismissed = session.revision)}>
        <Icon name="close" />
      </button>
    </div>
  {/if}
  {#if saveError}
    <div class="banner" role="alert">
      <span>Couldn't save: {saveError}</span>
      <button class="tool" aria-label="Dismiss" title="Dismiss" onclick={() => (saveError = null)}>
        <Icon name="close" />
      </button>
    </div>
  {/if}
  {#if editor.error}
    <div class="banner" role="alert">
      <span>Couldn't edit the text: {editor.error}</span>
      <button class="tool" aria-label="Dismiss" title="Dismiss" onclick={() => (editor.error = null)}>
        <Icon name="close" />
      </button>
    </div>
  {/if}

  <div class="body">
    {#if doc && ready}
      {#key doc.id}
        {#if sidebarOpen}
          <Sidebar {doc} {page} bind:tab={sidebarTab} ongoto={goTo} />
        {/if}
        <main use:trackCenter>
          <Viewer bind:this={viewer} {doc} {start} bind:page bind:zoom bind:fit />
          <PageControls
            {page}
            pageCount={doc.pages.length}
            {zoom}
            {fit}
            ongoto={goTo}
            onzoomin={() => viewer?.zoomIn()}
            onzoomout={() => viewer?.zoomOut()}
            onzoom={(value) => viewer?.zoomTo(value)}
            onfit={(mode) => viewer?.fitTo(mode)}
          />
        </main>
      {/key}
    {:else}
      <main>
        {#if doc}
          <p class="empty">{doc.name} has no pages.</p>
        {:else if started}
          <Welcome
            recent={session.recent}
            loading={session.loading}
            onopen={openDialog}
            onopenrecent={openPath}
            onforget={forgetRecent}
          />
        {/if}
      </main>
    {/if}

    {#if session.loading && ready}
      <div class="status" class:raised style:left={statusLeft} role="status">
        <span class="spinner"></span>Opening {session.loading}…
      </div>
    {:else if notice}
      <div class="status" class:raised style:left={statusLeft} role="status">
        <Icon name="check" size={16} />{notice}
      </div>
    {:else if slowPreview}
      <div class="status" class:raised style:left={statusLeft} role="status">
        <span class="spinner"></span>Updating the page…
      </div>
    {:else if hint && ready}
      <div class="status hint" class:raised style:left={statusLeft} role="status">
        {hint}
        <kbd>Esc</kbd>
        <span>to cancel</span>
      </div>
    {/if}
    {#if dragging}
      <div class="drop">Drop a PDF to open it</div>
    {/if}
  </div>
</div>

{#if signing}
  <SignatureDialog onclose={() => (signing = false)} />
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .body {
    position: relative;
    display: flex;
    flex: 1;
    min-height: 0;
  }

  main {
    position: relative;
    flex: 1;
    min-width: 0;
    background: var(--canvas);
  }

  .banner {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    padding: 0.25rem 0.375rem 0.25rem 0.875rem;
    border-bottom: 1px solid var(--error-border);
    background: var(--error-bg);
  }

  .banner span {
    flex: 1;
  }

  .empty {
    display: grid;
    place-content: center;
    height: 100%;
    margin: 0;
    color: var(--muted);
  }

  .status {
    position: absolute;
    bottom: 1.25rem;
    left: 50%;
    display: flex;
    gap: 0.625rem;
    align-items: center;
    padding: 0.5rem 1rem;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--popover);
    box-shadow: var(--popover-shadow);
    transform: translateX(-50%);
  }

  .status :global(svg) {
    color: var(--accent);
  }

  /* Stacks above the page controls. */
  .raised {
    bottom: 4rem;
  }

  .hint {
    gap: 0.375rem;
    pointer-events: none;
  }

  .hint kbd {
    margin-left: 0.375rem;
    padding: 0 0.375rem;
    border: 1px solid var(--border);
    border-radius: 0.25rem;
    background: var(--surface);
  }

  .hint span {
    color: var(--muted);
  }

  .drop {
    position: absolute;
    inset: 0.75rem;
    display: grid;
    place-content: center;
    border: 2px dashed var(--accent);
    border-radius: 0.75rem;
    background: color-mix(in srgb, var(--canvas) 88%, transparent);
    color: var(--selected-text);
    font-size: 1.25rem;
    font-weight: 600;
    pointer-events: none;
  }
</style>
