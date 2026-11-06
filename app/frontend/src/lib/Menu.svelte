<script lang="ts">
  import { tick, type Snippet } from "svelte";

  interface Props {
    label: string;
    title?: string;
    align?: "start" | "end";
    side?: "below" | "above";
    disabled?: boolean;
    trigger: Snippet;
    children: Snippet;
  }

  let { label, title = label, align = "start", side = "below", disabled = false, trigger, children }: Props =
    $props();

  const ITEM = '[role^="menuitem"]:not(:disabled)';

  let open = $state(false);
  let root: HTMLDivElement;
  let button: HTMLButtonElement;
  let menu = $state<HTMLDivElement>();

  $effect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!root.contains(event.target as Node)) hide(false);
    };
    window.addEventListener("pointerdown", outside, true);
    return () => window.removeEventListener("pointerdown", outside, true);
  });

  async function show() {
    open = true;
    await tick();
    const list = items();
    (list.find((item) => item.getAttribute("aria-checked") === "true") ?? list[0])?.focus();
  }

  function hide(restoreFocus = true) {
    open = false;
    if (restoreFocus) button.focus();
  }

  function items(): HTMLElement[] {
    return menu ? Array.from(menu.querySelectorAll<HTMLElement>(ITEM)) : [];
  }

  function onmenukeydown(event: KeyboardEvent) {
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLElement);
    let next: number;
    switch (event.key) {
      case "ArrowDown":
        next = (at + 1) % list.length;
        break;
      case "ArrowUp":
        next = (Math.max(at, 0) - 1 + list.length) % list.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = list.length - 1;
        break;
      case "Escape":
        event.preventDefault();
        hide();
        return;
      case "Tab":
        hide(false);
        return;
      default:
        return;
    }
    event.preventDefault();
    list[next]?.focus();
  }

  function onmenuclick(event: MouseEvent) {
    if ((event.target as Element).closest(ITEM)) hide();
  }

  function ontriggerkeydown(event: KeyboardEvent) {
    if (!open && event.key === "ArrowDown") {
      event.preventDefault();
      show();
    }
  }
</script>

<div class="root" bind:this={root}>
  <button
    bind:this={button}
    class="tool"
    aria-label={label}
    {title}
    aria-haspopup="menu"
    aria-expanded={open}
    {disabled}
    onclick={() => (open ? hide() : show())}
    onkeydown={ontriggerkeydown}
  >
    {@render trigger()}
  </button>
  {#if open}
    <div
      class="menu"
      class:end={align === "end"}
      class:above={side === "above"}
      role="menu"
      tabindex="-1"
      aria-label={label}
      bind:this={menu}
      onkeydown={onmenukeydown}
      onclick={onmenuclick}
    >
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .root {
    position: relative;
    display: inline-flex;
  }

  .menu {
    position: absolute;
    top: calc(100% + 0.25rem);
    left: 0;
    z-index: 20;
    display: flex;
    flex-direction: column;
    min-width: 12rem;
    max-width: 26rem;
    max-height: min(32rem, calc(100vh - 4rem));
    overflow-y: auto;
    padding: 0.25rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    background: var(--popover);
    box-shadow: var(--popover-shadow);
    color: var(--text);
    outline: none;
  }

  .end {
    right: 0;
    left: auto;
  }

  .above {
    top: auto;
    bottom: calc(100% + 0.25rem);
  }

  .menu :global([role^="menuitem"]) {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    min-height: 2rem;
    padding: 0.25rem 0.625rem 0.25rem 0.5rem;
    border: 0;
    border-radius: 0.3125rem;
    background: transparent;
    text-align: left;
    white-space: nowrap;
    cursor: pointer;
  }

  .menu :global([role^="menuitem"]:hover),
  .menu :global([role^="menuitem"]:focus-visible) {
    background: var(--hover);
    outline: none;
  }

  .menu :global([role^="menuitem"]:disabled) {
    color: var(--muted);
    cursor: default;
  }

  .menu :global([role="separator"]) {
    height: 1px;
    margin: 0.25rem 0.375rem;
    background: var(--border);
  }

  .menu :global(.menu-heading) {
    padding: 0.375rem 0.625rem 0.25rem;
    color: var(--muted);
    font-size: 0.875rem;
    font-weight: 600;
  }

  .menu :global(.menu-hint) {
    margin-left: auto;
    padding-left: 1.5rem;
    color: var(--muted);
    font-size: 0.875rem;
  }
</style>
