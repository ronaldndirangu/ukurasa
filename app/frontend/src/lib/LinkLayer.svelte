<script lang="ts">
  import { openLink, pageLinks, type Link } from "./api";
  import { editor } from "./edits.svelte";

  interface Props {
    doc: number;
    page: number;
    /** CSS pixels per point. */
    scale: number;
    onnavigate: (page: number, top: number | null) => void;
    /** Lets clicks through to the text and pictures under the links. */
    bypass?: boolean;
  }

  let { doc, page, scale, onnavigate, bypass = false }: Props = $props();

  let links = $state.raw<Link[]>([]);

  $effect(() => {
    let live = true;
    pageLinks(doc, page).then((found) => {
      if (live) links = found;
    });
    return () => {
      live = false;
    };
  });

  function label(link: Link): string {
    const { target } = link;
    return target.kind === "uri" ? target.uri.replace(/^mailto:/, "") : `Go to page ${target.page + 1}`;
  }

  function follow(link: Link) {
    const { target } = link;
    if (target.kind === "page") onnavigate(target.page, target.top);
    else openLink(target.uri).catch((error) => (editor.notice = String(error)));
  }
</script>

<div class="links" class:bypass>
  {#each links as link, index (index)}
    <!-- Mouse clicks leave focus on the page so Space keeps scrolling. -->
    <button
      type="button"
      class="link"
      title={label(link)}
      aria-label={label(link)}
      style:left="{link.x * scale}px"
      style:top="{link.y * scale}px"
      style:width="{link.width * scale}px"
      style:height="{link.height * scale}px"
      onmousedown={(event) => event.preventDefault()}
      onclick={() => follow(link)}
    ></button>
  {/each}
</div>

<style>
  /* Above the page's text and pictures, which a plain click on a link follows instead of editing. */
  .links {
    position: absolute;
    inset: 0;
    z-index: 1;
    pointer-events: none;
  }

  .link {
    position: absolute;
    padding: 0;
    border: 0;
    border-radius: 2px;
    background: transparent;
    cursor: pointer;
    pointer-events: auto;
  }

  .bypass .link {
    pointer-events: none;
  }

  .link:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }

  .link:focus-visible {
    outline: 2px solid var(--accent);
  }
</style>
