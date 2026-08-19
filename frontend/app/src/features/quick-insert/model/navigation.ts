export type ActiveResultPopupRole = "grid" | "listbox";

export function activeResultId(popupId: string, key: string): string {
  const encodedKey = encodeURIComponent(key)
    .replaceAll("_", "_5F")
    .replaceAll("%", "_");
  return `${popupId}-result-${encodedKey || "empty"}`;
}

export function nextSelection(
  current: number,
  count: number,
  direction: -1 | 1,
): number {
  if (count === 0) return -1;
  if (current < 0) return 0;
  return Math.max(
    0,
    Math.min((current < 0 ? 0 : current) + direction, count - 1),
  );
}

export function selectionForKey(key: string, count: number): number | null {
  if (!/^[1-9]$/.test(key)) return null;
  const index = Number(key) - 1;
  return index < count ? index : null;
}

export function scrollActiveResultIntoView(element: Element | null): void {
  element?.scrollIntoView({
    block: "nearest",
    inline: "nearest",
    behavior: "auto",
  });
}
