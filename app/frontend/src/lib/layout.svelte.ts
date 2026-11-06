export const PX_PER_PT = 96 / 72;

/** CSS width of sidebar thumbnails. Fast-scroll previews reuse it so both share cached renders. */
export const THUMB_WIDTH = 120;

let ratio = $state(window.devicePixelRatio || 1);
window.addEventListener("resize", () => (ratio = window.devicePixelRatio || 1));

export const display = {
  get dpr() {
    return ratio;
  },
  get thumbRenderWidth() {
    return Math.round(THUMB_WIDTH * ratio);
  },
};

/** Returns the last index whose top is at or above `y`, or 0 when none is. */
export function indexAt(tops: ArrayLike<number>, y: number): number {
  let low = 0;
  let high = tops.length - 1;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (tops[mid] <= y) low = mid;
    else high = mid - 1;
  }
  return low;
}

/** Indices from `first` to `last`, widened by `overscan` and clamped to `count`. */
export function range(first: number, last: number, overscan: number, count: number): number[] {
  const start = Math.max(0, first - overscan);
  const end = Math.min(count - 1, last + overscan);
  return Array.from({ length: Math.max(0, end - start + 1) }, (_, i) => start + i);
}

/** Limits `value` to `min..max`, preferring `min` when the range is empty. */
export const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(value, max));
