/**
 * Makes the buttons of a toolbar, tab list, or radio group one Tab stop: arrow keys, Home, and End
 * move focus between them, and Tab returns to the one focused last. Tabs and radios are selected
 * as focus reaches them. Fields and menus inside keep their own keys and Tab stops.
 */
export function roving(node: HTMLElement, selector = "button") {
  let current: HTMLElement | null = null;

  const items = () =>
    Array.from(node.querySelectorAll<HTMLElement>(selector)).filter(
      (item) => !(item as HTMLButtonElement).disabled && !item.closest("[role='menu']"),
    );
  const isSelected = (item: HTMLElement) =>
    item.getAttribute("aria-selected") === "true" || item.getAttribute("aria-checked") === "true";

  function refresh() {
    const list = items();
    const selected = list.find(isSelected);
    if (selected && !node.contains(document.activeElement)) current = selected;
    else if (!current || !list.includes(current)) current = selected ?? list[0] ?? null;
    for (const item of list) item.tabIndex = item === current ? 0 : -1;
  }

  function onfocusin(event: FocusEvent) {
    const target = event.target as HTMLElement;
    if (!items().includes(target)) return;
    current = target;
    refresh();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
    const list = items();
    const at = list.indexOf(event.target as HTMLElement);
    if (at < 0) return;
    const vertical = node.getAttribute("aria-orientation") === "vertical";
    let next: number;
    switch (event.key) {
      case vertical ? "ArrowUp" : "ArrowLeft":
        next = (at - 1 + list.length) % list.length;
        break;
      case vertical ? "ArrowDown" : "ArrowRight":
        next = (at + 1) % list.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = list.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    const item = list[next];
    item.focus();
    const role = item.getAttribute("role");
    if ((role === "tab" || role === "radio") && !isSelected(item)) item.click();
  }

  const observer = new MutationObserver(refresh);
  observer.observe(node, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["disabled", "aria-selected", "aria-checked"],
  });
  node.addEventListener("focusin", onfocusin);
  node.addEventListener("keydown", onkeydown);
  refresh();

  return {
    destroy() {
      observer.disconnect();
      node.removeEventListener("focusin", onfocusin);
      node.removeEventListener("keydown", onkeydown);
    },
  };
}
