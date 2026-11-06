<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMaximizeButton, setMaximizeButton, type ButtonState } from "./api";

  const appWindow = getCurrentWindow();

  let maximized = $state(false);
  // Windows reports the pointer over the maximize button in place of the page.
  let pointer = $state<ButtonState>("leave");
  let maximize: HTMLButtonElement;

  function place() {
    const { x, y, width, height } = maximize.getBoundingClientRect();
    setMaximizeButton({ x, y, width, height }).catch(() => {});
  }

  onMount(() => {
    const sync = () => appWindow.isMaximized().then((value) => (maximized = value));
    sync();
    place();
    window.addEventListener("resize", place);
    const unlisten = [appWindow.onResized(sync), onMaximizeButton((state) => (pointer = state))];
    return () => {
      window.removeEventListener("resize", place);
      unlisten.forEach((pending) => pending.then((stop) => stop()));
      setMaximizeButton(null).catch(() => {});
    };
  });
</script>

<div class="controls">
  <button class="caption" tabindex="-1" aria-label="Minimize" title="Minimize" onclick={() => appWindow.minimize()}>
    &#xE921;
  </button>
  <button
    bind:this={maximize}
    class="caption"
    class:hover={pointer === "hover"}
    class:press={pointer === "press"}
    tabindex="-1"
    aria-label={maximized ? "Restore" : "Maximize"}
    onclick={() => appWindow.toggleMaximize()}
  >
    {maximized ? "\uE923" : "\uE922"}
  </button>
  <button class="caption close" tabindex="-1" aria-label="Close" title="Close" onclick={() => appWindow.close()}>
    &#xE8BB;
  </button>
</div>

<style>
  .controls {
    display: flex;
    align-self: stretch;
    margin-left: 0.5rem;
  }

  .caption {
    width: 46px;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    font-size: 10px;
  }

  .caption:hover,
  .hover {
    background: var(--chrome-hover);
  }

  .caption:active,
  .press {
    background: var(--chrome-active);
  }

  .close:hover {
    background: #c42b1c;
    color: #ffffff;
  }

  .close:active {
    background: rgb(196 43 28 / 0.9);
    color: rgb(255 255 255 / 0.85);
  }
</style>
