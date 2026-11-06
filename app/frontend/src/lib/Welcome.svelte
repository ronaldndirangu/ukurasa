<script lang="ts">
  import { MOD, type RecentFile } from "./api";
  import Icon from "./Icon.svelte";

  interface Props {
    recent: RecentFile[];
    /** Name of the file being opened, if any. */
    loading: string | null;
    onopen: () => void;
    onopenrecent: (path: string) => void;
    onforget: (path: string) => void;
  }

  let { recent, loading, onopen, onopenrecent, onforget }: Props = $props();

  const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const date = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });

  let now = $state(Date.now());

  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 60_000);
    return () => clearInterval(timer);
  });

  function ago(time: number): string {
    const minutes = Math.round((now - time) / 60_000);
    if (minutes < 1) return "Just now";
    if (minutes < 60) return relative.format(-minutes, "minute");
    const hours = Math.round(minutes / 60);
    if (hours < 24) return relative.format(-hours, "hour");
    const days = Math.round(hours / 24);
    if (days < 7) return relative.format(-days, "day");
    return date.format(time);
  }

  function folder(path: string): string {
    const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return cut > 0 ? path.slice(0, cut) : path;
  }
</script>

<div class="welcome">
  <div class="intro">
    <svg class="mark" viewBox="0 0 48 48" aria-hidden="true">
      <path class="sheet" d="M13 4h16l11 11v27a2 2 0 0 1-2 2H13a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z" />
      <path class="fold" d="M29 4v9a2 2 0 0 0 2 2h9Z" />
      <path class="lines" d="M17 21h12M17 26.5h8" />
      <path class="signature" d="M17 36.5c2.5-4.5 4.5-5 5.5-2.5s2.5 2.5 4.5-.5 3.5-2.5 4.5 0" />
    </svg>
    <h1>Ukurasa</h1>
    <p class="tagline">Read, edit, and sign PDF files.</p>
    {#if loading}
      <p class="opening" role="status"><span class="spinner"></span>Opening {loading}…</p>
    {:else}
      <button class="primary" onclick={onopen}>
        <Icon name="open" />
        Open PDF
      </button>
      <p class="hint">or drop a file anywhere · <kbd>{MOD}O</kbd></p>
    {/if}
  </div>

  {#if recent.length > 0}
    <section aria-labelledby="recent-heading">
      <h2 id="recent-heading">Recent</h2>
      <ul>
        {#each recent as file (file.path)}
          <li class:missing={file.missing}>
            <button
              class="entry"
              title={file.missing ? `${file.path} (not found)` : file.path}
              onclick={() => onopenrecent(file.path)}
            >
              <span class="icon"><Icon name="file" /></span>
              <span class="text">
                <span class="name">{file.name}</span>
                <span class="path">{folder(file.path)}</span>
              </span>
              <span class="when">{file.missing ? "Not found" : ago(file.opened)}</span>
            </button>
            <button
              class="tool remove"
              aria-label="Remove {file.name} from recent files"
              title="Remove from list"
              onclick={() => onforget(file.path)}
            >
              <Icon name="close" size={16} />
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

<style>
  .welcome {
    display: flex;
    flex-direction: column;
    gap: 2.5rem;
    align-items: center;
    height: 100%;
    overflow-y: auto;
    padding: max(2rem, 12vh) 1.5rem 2rem;
    box-sizing: border-box;
  }

  .intro {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    align-items: center;
    text-align: center;
  }

  .mark {
    width: 4rem;
    height: 4rem;
    margin-bottom: 0.25rem;
  }

  .sheet {
    fill: var(--accent);
  }

  .fold {
    fill: rgb(255 255 255 / 0.4);
  }

  .lines,
  .signature {
    fill: none;
    stroke: #fff;
    stroke-width: 2.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .lines {
    stroke-opacity: 0.7;
  }

  h1 {
    margin: 0;
    font-size: 2.25rem;
    font-weight: 700;
    letter-spacing: -0.02em;
  }

  p {
    margin: 0;
  }

  .tagline {
    margin-bottom: 0.75rem;
    color: var(--muted);
    font-size: 1.0625rem;
  }

  .hint {
    color: var(--muted);
    font-size: 0.875rem;
  }

  .opening {
    display: flex;
    gap: 0.625rem;
    align-items: center;
    height: 2.25rem;
  }

  section {
    width: min(36rem, 100%);
  }

  h2 {
    margin: 0 0 0.5rem 0.75rem;
    color: var(--muted);
    font-size: 0.875rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  ul {
    margin: 0;
    padding: 0.25rem;
    border: 1px solid var(--border);
    border-radius: 0.75rem;
    background: var(--surface);
    list-style: none;
  }

  li {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: 0.5rem;
  }

  li:hover {
    background: var(--hover);
  }

  .entry {
    display: flex;
    flex: 1;
    gap: 0.75rem;
    align-items: center;
    min-width: 0;
    padding: 0.5rem 0.75rem 0.5rem 0.625rem;
    border: 0;
    border-radius: 0.5rem;
    background: none;
    text-align: left;
    cursor: pointer;
  }

  .icon {
    display: inline-flex;
    padding: 0.375rem;
    border-radius: 0.375rem;
    background: var(--selected);
    color: var(--selected-text);
  }

  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .name,
  .path {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .name {
    font-weight: 600;
  }

  .path,
  .when {
    color: var(--muted);
    font-size: 0.875rem;
  }

  .when {
    flex: none;
    font-variant-numeric: tabular-nums;
  }

  .remove {
    position: absolute;
    right: 0.375rem;
    color: var(--muted);
    opacity: 0;
  }

  li:hover .remove,
  .remove:focus-visible {
    opacity: 1;
  }

  li:hover .when,
  li:has(.remove:focus-visible) .when {
    visibility: hidden;
  }

  .missing .name,
  .missing .icon {
    opacity: 0.55;
  }
</style>
