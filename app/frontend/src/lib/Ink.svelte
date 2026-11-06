<script lang="ts">
  import type { Color, VectorPath } from "./api";
  import { css } from "./edits.svelte";

  interface Props {
    path: VectorPath;
    color: Color;
    /** Stretch to fill the box, as saved signatures do; otherwise keep the aspect ratio. */
    stretch?: boolean;
  }

  let { path, color, stretch = true }: Props = $props();

  const paint = $derived(css(color));
</script>

<svg
  viewBox="0 0 {path.width} {path.height}"
  preserveAspectRatio={stretch ? "none" : "xMidYMid meet"}
  aria-hidden="true"
>
  <path
    d={path.d}
    fill={path.fill ? paint : "none"}
    stroke={path.fill ? "none" : paint}
    stroke-width={path.strokeWidth}
  />
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: 100%;
    overflow: visible;
    pointer-events: none;
  }

  path {
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
