import {
  appsSdkIconRegistry,
  type AppsSdkIconExportKey,
  type EchoIconComponent,
} from "./registry";

/** Persisted value used by the Favorite editor for an intentionally empty icon. */
export const NO_ICON_KEY = "none" as const;

export type FavoriteIconKey = AppsSdkIconExportKey | typeof NO_ICON_KEY;

/**
 * Common clipboard-reuse choices shown before the complete Apps SDK catalog.
 * Every key is checked against the pinned package's actual barrel exports.
 */
export const CURATED_FAVORITE_ICON_KEYS = [
  "Mail",
  "Phone",
  "MapPin",
  "Terminal",
  "Code",
  "Link",
  "File",
  "Folder",
  "Document",
  "Clipboard",
  "Star",
  "Tag",
  "Key",
  "Calendar",
  "User",
  "Home",
  "Suitcase",
] as const satisfies readonly AppsSdkIconExportKey[];

const publicIconKeys = Object.entries(appsSdkIconRegistry)
  .filter(([, component]) => typeof component === "function")
  .map(([key]) => key as AppsSdkIconExportKey)
  .sort();
const curatedIconKeys =
  CURATED_FAVORITE_ICON_KEYS as readonly AppsSdkIconExportKey[];

/**
 * All public Apps SDK export keys, with No icon first and curated choices
 * promoted ahead of the searchable remainder. Components are resolved only
 * for the selected key; the catalog itself does not render SVGs.
 */
export const FAVORITE_ICON_KEYS: readonly FavoriteIconKey[] = [
  NO_ICON_KEY,
  ...curatedIconKeys,
  ...publicIconKeys.filter((key) => !curatedIconKeys.includes(key)),
];

export function getFavoriteIconComponent(
  iconKey: string | null | undefined,
): EchoIconComponent | null {
  if (!iconKey || iconKey === NO_ICON_KEY) return null;
  const component = appsSdkIconRegistry[iconKey];
  return typeof component === "function" ? component : null;
}

export function isFavoriteIconKey(
  iconKey: string | null | undefined,
): iconKey is FavoriteIconKey {
  return (
    iconKey === NO_ICON_KEY ||
    Boolean(iconKey && getFavoriteIconComponent(iconKey))
  );
}
