<script lang="ts">
  import { onMount } from "svelte";
  import { signaturePath, type Color, type VectorPath } from "./api";
  import {
    SIGNATURE_INKS,
    css,
    editor,
    forgetSignature,
    rememberSignature,
    sameColor,
    savedSignatures,
  } from "./edits.svelte";
  import Icon from "./Icon.svelte";
  import Ink from "./Ink.svelte";
  import { clamp } from "./layout.svelte";
  import { roving } from "./roving";

  const PAD_WIDTH = 480;
  const PAD_HEIGHT = 180;
  const STROKE_WIDTH = 2.5;
  const TYPE_DELAY_MS = 120;

  type Tab = "saved" | "draw" | "type";

  let { onclose }: { onclose: () => void } = $props();

  let dialog: HTMLDialogElement;
  let pad = $state<SVGSVGElement>();
  let saved = $state.raw(savedSignatures());
  let picked = $state.raw<VectorPath | null>(savedSignatures()[0] ?? null);
  let tab = $state<Tab>(savedSignatures().length > 0 ? "saved" : "draw");
  let color = $state.raw<Color>(editor.signature?.color ?? SIGNATURE_INKS[0].color);
  let remember = $state(true);
  /** Each stroke is a flat list of x, y pairs in pad units. */
  let strokes = $state.raw<number[][]>([]);
  let drawing: number | null = null;
  let name = $state("");
  let typed = $state.raw<VectorPath | null>(null);
  let typeError = $state<string | null>(null);
  let sequence = 0;

  const drawn = $derived(strokes.length > 0 ? normalize(strokes) : null);
  const result = $derived(tab === "draw" ? drawn : tab === "type" ? typed : picked);
  const tabs = $derived<{ id: Tab; label: string }[]>([
    ...(saved.length > 0 ? [{ id: "saved" as const, label: "Saved" }] : []),
    { id: "draw", label: "Draw" },
    { id: "type", label: "Type" },
  ]);

  onMount(() => dialog.showModal());

  $effect(() => {
    const text = name.trim();
    const id = ++sequence;
    if (!text) {
      typed = null;
      typeError = null;
      return;
    }
    const timer = setTimeout(() => {
      signaturePath(text)
        .then((path) => {
          if (id !== sequence) return;
          typed = path;
          typeError = null;
        })
        .catch((error) => {
          if (id !== sequence) return;
          typed = null;
          typeError = String(error);
        });
    }, TYPE_DELAY_MS);
    return () => clearTimeout(timer);
  });

  const round = (n: number) => +n.toFixed(1);

  /** Smooths a stroke with quadratic curves through the midpoints of its samples. */
  function strokePath(points: number[], dx = 0, dy = 0): string {
    const at = (i: number) => `${round(points[i] + dx)} ${round(points[i + 1] + dy)}`;
    const count = points.length / 2;
    if (count === 1) return `M${at(0)}L${round(points[0] + dx + 0.1)} ${round(points[1] + dy)}`;
    let d = `M${at(0)}`;
    for (let i = 2; i < points.length - 2; i += 2) {
      const midX = (points[i] + points[i + 2]) / 2 + dx;
      const midY = (points[i + 1] + points[i + 3]) / 2 + dy;
      d += `Q${at(i)} ${round(midX)} ${round(midY)}`;
    }
    return `${d}L${at(points.length - 2)}`;
  }

  /** Crops the drawing to its ink with room for the stroke. */
  function normalize(list: number[][]): VectorPath {
    let [minX, minY, maxX, maxY] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const stroke of list) {
      for (let i = 0; i < stroke.length; i += 2) {
        minX = Math.min(minX, stroke[i]);
        maxX = Math.max(maxX, stroke[i]);
        minY = Math.min(minY, stroke[i + 1]);
        maxY = Math.max(maxY, stroke[i + 1]);
      }
    }
    const margin = STROKE_WIDTH / 2 + 1;
    return {
      d: list.map((stroke) => strokePath(stroke, margin - minX, margin - minY)).join(""),
      width: round(maxX - minX + 2 * margin),
      height: round(maxY - minY + 2 * margin),
      fill: false,
      strokeWidth: STROKE_WIDTH,
    };
  }

  function padPoint(event: PointerEvent): [number, number] {
    const rect = pad!.getBoundingClientRect();
    return [
      clamp(((event.clientX - rect.left) * PAD_WIDTH) / rect.width, 0, PAD_WIDTH),
      clamp(((event.clientY - rect.top) * PAD_HEIGHT) / rect.height, 0, PAD_HEIGHT),
    ];
  }

  function ondown(event: PointerEvent) {
    if (event.button !== 0 || !pad) return;
    event.preventDefault();
    pad.setPointerCapture(event.pointerId);
    drawing = event.pointerId;
    strokes = [...strokes, padPoint(event)];
  }

  function onmove(event: PointerEvent) {
    if (event.pointerId !== drawing) return;
    const stroke = [...strokes[strokes.length - 1]];
    const samples = event.getCoalescedEvents?.() ?? [];
    for (const sample of samples.length > 0 ? samples : [event]) {
      const [x, y] = padPoint(sample);
      if (Math.hypot(x - stroke[stroke.length - 2], y - stroke[stroke.length - 1]) >= 1) stroke.push(x, y);
    }
    strokes = [...strokes.slice(0, -1), stroke];
  }

  function onup(event: PointerEvent) {
    if (event.pointerId === drawing) drawing = null;
  }

  function use(path: VectorPath, keep: boolean) {
    if (keep) rememberSignature(path);
    editor.prepareSignature(path, color);
    dialog.close();
  }

  function forget(path: VectorPath) {
    forgetSignature(path);
    saved = savedSignatures();
    if (picked?.d === path.d) picked = saved[0] ?? null;
    if (saved.length === 0) tab = "draw";
  }

  function pickKey(event: KeyboardEvent, path: VectorPath) {
    if (event.key !== "Enter") return;
    event.preventDefault();
    use(path, true);
  }

  function focus(node: HTMLElement) {
    node.focus();
  }
</script>

<dialog bind:this={dialog} aria-labelledby="signature-title" {onclose}>
  <header>
    <h2 id="signature-title">Add a signature</h2>
    <button class="tool" aria-label="Close" title="Close" onclick={() => dialog.close()}>
      <Icon name="close" />
    </button>
  </header>

  <div class="tabs" role="tablist" use:roving>
    {#each tabs as item (item.id)}
      <button role="tab" aria-selected={tab === item.id} onclick={() => (tab = item.id)}>{item.label}</button>
    {/each}
  </div>

  <div class="panel" role="tabpanel">
    {#if tab === "draw"}
      <svg
        class="paper pad"
        viewBox="0 0 {PAD_WIDTH} {PAD_HEIGHT}"
        role="img"
        aria-label="Signature drawing area"
        bind:this={pad}
        onpointerdown={ondown}
        onpointermove={onmove}
        onpointerup={onup}
        onpointercancel={onup}
      >
        <line class="baseline" x1="24" y1="132" x2={PAD_WIDTH - 24} y2="132" />
        {#each strokes as stroke, i (i)}
          <path d={strokePath(stroke)} stroke={css(color)} stroke-width={STROKE_WIDTH} />
        {/each}
      </svg>
      {#if strokes.length === 0}
        <p class="hint">Draw your signature with the mouse, a pen, or a finger.</p>
      {/if}
    {:else if tab === "type"}
      <input class="name" aria-label="Your name" placeholder="Type your name" maxlength="60" use:focus bind:value={name} />
      <div class="paper preview">
        {#if typed}
          <Ink path={typed} {color} stretch={false} />
        {:else if typeError}
          <p class="error">{typeError}</p>
        {:else}
          <p class="hint">Your signature appears here.</p>
        {/if}
      </div>
    {:else}
      <div class="saved" role="radiogroup" aria-label="Saved signatures" use:roving={"[role='radio']"}>
        {#each saved as path, i (path.d)}
          {@const selected = picked?.d === path.d}
          <div class="tile">
            <button
              class="paper pick"
              role="radio"
              aria-checked={selected}
              aria-label="Signature {i + 1}"
              title="Double-click to use"
              onclick={() => (picked = path)}
              ondblclick={() => use(path, true)}
              onkeydown={(event) => pickKey(event, path)}
            >
              <Ink {path} {color} stretch={false} />
            </button>
            <button
              class="tool forget"
              tabindex={selected ? 0 : -1}
              aria-label="Remove signature {i + 1}"
              title="Remove from this device"
              onclick={() => forget(path)}
            >
              <Icon name="trash" size={16} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div class="options">
    <span class="ink-label" id="ink-label">Ink</span>
    <div class="inks" role="radiogroup" aria-labelledby="ink-label" use:roving>
      {#each SIGNATURE_INKS as ink (ink.name)}
        <button
          class="swatch"
          role="radio"
          aria-checked={sameColor(ink.color, color)}
          aria-label={ink.name}
          title={ink.name}
          style:--ink={css(ink.color)}
          onclick={() => (color = ink.color)}
        ></button>
      {/each}
    </div>
    {#if tab !== "saved"}
      <label class="remember">
        <input type="checkbox" bind:checked={remember} />
        Remember on this device
      </label>
    {/if}
  </div>

  <footer>
    {#if tab === "draw"}
      <button class="secondary" disabled={strokes.length === 0} onclick={() => (strokes = [])}>Clear</button>
    {/if}
    <div class="actions">
      <button class="secondary" onclick={() => dialog.close()}>Cancel</button>
      <button class="primary" disabled={!result} onclick={() => result && use(result, tab === "saved" || remember)}>
        Use signature
      </button>
    </div>
  </footer>
</dialog>

<style>
  dialog {
    width: min(34rem, calc(100vw - 2rem));
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 0.75rem;
    background: var(--popover);
    box-shadow: var(--popover-shadow);
    color: inherit;
  }

  dialog::backdrop {
    background: rgb(15 23 42 / 0.35);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.75rem 0.75rem 0 1.25rem;
  }

  h2 {
    margin: 0;
    font-size: 1.0625rem;
  }

  .tabs {
    display: flex;
    gap: 0.25rem;
    margin: 0.75rem 1.25rem 0;
    border-bottom: 1px solid var(--border);
  }

  [role="tab"] {
    margin-bottom: -1px;
    padding: 0.5rem 0.75rem;
    border: 0;
    border-bottom: 2px solid transparent;
    background: none;
    color: var(--muted);
    font-weight: 600;
    cursor: pointer;
  }

  [role="tab"][aria-selected="true"] {
    border-bottom-color: var(--accent);
    color: inherit;
  }

  .panel {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-height: 11.25rem;
    padding: 1rem 1.25rem;
  }

  .paper {
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    background: #fff;
    color: #5f6b7a;
  }

  .pad {
    display: block;
    width: 100%;
    aspect-ratio: 480 / 180;
    cursor: crosshair;
    touch-action: none;
  }

  .pad path {
    fill: none;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .baseline {
    stroke: #cfd5dd;
    stroke-dasharray: 4 4;
  }

  .hint {
    margin: 0;
    color: var(--muted);
    font-size: 0.875rem;
    text-align: center;
  }

  .preview .hint,
  .preview .error {
    color: #5f6b7a;
  }

  .pad + .hint {
    position: absolute;
    top: 50%;
    right: 1.25rem;
    left: 1.25rem;
    pointer-events: none;
  }

  .name {
    height: 2.25rem;
    padding: 0 0.75rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    background: var(--field);
  }

  .preview {
    display: grid;
    place-items: center;
    height: 7.5rem;
    padding: 1rem 1.5rem;
  }

  .error {
    margin: 0;
    text-align: center;
  }

  .saved {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
    gap: 0.75rem;
  }

  .tile {
    position: relative;
  }

  .pick {
    display: block;
    width: 100%;
    height: 6rem;
    padding: 0.75rem 1rem;
    cursor: pointer;
  }

  .pick:hover {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
  }

  .pick[aria-checked="true"] {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .pick:focus-visible {
    outline-offset: 2px;
  }

  .forget {
    position: absolute;
    top: 0.25rem;
    right: 0.25rem;
    color: #5f6b7a;
  }

  .options {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    padding: 0 1.25rem 1rem;
  }

  .ink-label {
    color: var(--muted);
    font-size: 0.875rem;
  }

  footer {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    padding: 0.75rem 1.25rem 1rem;
    border-top: 1px solid var(--border);
  }

  .inks {
    display: flex;
    gap: 0.5rem;
  }

  .remember {
    display: flex;
    gap: 0.375rem;
    align-items: center;
    margin-left: auto;
    color: var(--muted);
    font-size: 0.875rem;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    margin-left: auto;
  }
</style>
