<script lang="ts" module>
  const PATHS = {
    open: "M2.5 5.5v9a1 1 0 0 0 1 1h13a1 1 0 0 0 1-1V7.5a1 1 0 0 0-1-1H10L8 4.5H3.5a1 1 0 0 0-1 1Z",
    file: "M5.5 2.5h6l4 4v10a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1ZM11.5 2.5v4h4",
    sidebar: "M3.5 3.5h13a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1h-13a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1ZM7.5 3.5v13",
    up: "m5 12.5 5-5 5 5",
    down: "m5 7.5 5 5 5-5",
    right: "m7.5 5 5 5-5 5",
    minus: "M5 10h10",
    plus: "M5 10h10M10 5v10",
    close: "m5.5 5.5 9 9M14.5 5.5l-9 9",
    check: "m4.5 10.5 3.5 3.5 7.5-8",
    clock: "M10 2.5a7.5 7.5 0 1 1 0 15 7.5 7.5 0 0 1 0-15ZM10 6v4l2.5 2.5",
    text: "M4.5 5V4h11v1M10 4v12M7.5 16h5",
    editText: "M3 6h7M3 10h5M3 14h7M14 4.5v11M12.5 4.5h3M12.5 15.5h3",
    sign: "M3 16.5h14M5 13l7.5-7.5 2 2L7 15H5v-2Z",
    save: "M4.5 3.5h9l3 3v9a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1ZM7 3.5v4h6v-4M6.5 16.5v-5h7v5",
    trash: "M4 6h12M8 6V4h4v2M5.5 6l.75 10.5h7.5L14.5 6",
    undo: "M7.5 5 4 8.5 7.5 12M4 8.5h8a4 4 0 0 1 0 8H9",
    redo: "M12.5 5 16 8.5 12.5 12M16 8.5H8a4 4 0 0 0 0 8h3",
    menu: "M3.5 5.5h13M3.5 10h13M3.5 14.5h13",
  };

  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  let { name, size = 20 }: { name: IconName; size?: number } = $props();
</script>

<svg viewBox="0 0 20 20" width={size} height={size} aria-hidden="true"><path d={PATHS[name]} /></svg>

<style>
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
