<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { cancelRender, pageImage, type PageImage } from "./api";

  interface Props {
    doc: number;
    index: number;
    /** Device pixel width to render at; zero waits. */
    renderWidth: number;
    priority: number;
    /** A cheaper width to show first while the page is blank; zero skips it. */
    previewWidth?: number;
    previewPriority?: number;
    /** Changes when the page's content changes, to render it again. */
    revision?: number;
    /** Called with the revision of each image once it is on screen. */
    onshow?: (revision: number) => void;
  }

  let {
    doc,
    index,
    renderWidth,
    priority,
    previewWidth = 0,
    previewPriority = priority,
    revision = 0,
    onshow,
  }: Props = $props();

  interface Shot extends PageImage {
    width: number;
    revision: number;
  }

  // Requested images stay hidden until they load, so zooming never flashes a blank page.
  let shown = $state.raw<Shot | null>(null);
  let preview = $state.raw<Shot | null>(null);
  let pending = $state.raw<Shot | null>(null);
  let failed = $state(false);
  const shots = $derived([shown, preview, pending].filter((shot): shot is Shot => shot !== null));

  $effect(() => {
    const target = renderWidth;
    const content = revision;
    untrack(() => request(target, content));
  });

  onDestroy(() => {
    for (const shot of [preview, pending]) if (shot) cancelRender(shot.id);
  });

  function request(target: number, content: number) {
    const current = (shot: Shot | null) => shot?.width === target && shot.revision === content;
    if (target <= 0 || current(pending)) return;
    if (pending) cancelRender(pending.id);
    pending = null;
    if (current(shown)) return;
    if (!shown && !preview && previewWidth > 0 && previewWidth < target) {
      preview = shot(previewWidth, previewPriority, content);
    }
    pending = shot(target, priority, content);
  }

  function shot(width: number, at: number, content: number): Shot {
    return { ...pageImage(doc, index, width, at), width, revision: content };
  }

  function loaded(shot: Shot) {
    if (shot.id === pending?.id) {
      shown = shot;
      pending = null;
      if (preview) cancelRender(preview.id);
      preview = null;
    } else if (shot.id === preview?.id) {
      preview = null;
      shown ??= shot;
    } else {
      return;
    }
    failed = false;
    onshow?.(shown.revision);
  }

  function errored(shot: Shot) {
    if (shot.id === pending?.id) pending = null;
    else if (shot.id === preview?.id) preview = null;
    else return;
    failed = !shown && !preview && !pending;
  }
</script>

<span class="image" class:loading={!shown && !failed}>
  {#each shots as shot (shot.id)}
    <img
      src={shot.url}
      alt=""
      draggable="false"
      decoding="async"
      class:hidden={shot.id !== shown?.id}
      onload={() => loaded(shot)}
      onerror={() => errored(shot)}
    />
  {/each}
  {#if failed}
    <span class="failure">Page {index + 1} could not be displayed.</span>
  {/if}
</span>

<style>
  .image {
    display: block;
    position: absolute;
    inset: 0;
    overflow: hidden;
    background: #fff;
  }

  .loading {
    background: linear-gradient(100deg, transparent 35%, rgb(15 23 42 / 0.06) 50%, transparent 65%) #fff;
    background-size: 250% 100%;
    animation: shimmer 1.5s ease-in-out infinite;
  }

  @keyframes shimmer {
    from {
      background-position: 100% 0;
    }
    to {
      background-position: 0 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .loading {
      animation: none;
    }
  }

  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    user-select: none;
  }

  .hidden {
    visibility: hidden;
  }

  .failure {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    margin: 0;
    padding: 0.5rem;
    color: #5f6b7a;
    font-size: 0.875em;
    text-align: center;
  }
</style>
