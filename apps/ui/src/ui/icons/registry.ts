import * as AppsSdkIcons from "@openai/apps-sdk-ui/components/Icon";
import type { ComponentType, SVGProps } from "react";

export type EchoIconComponent = ComponentType<SVGProps<SVGSVGElement>>;
export type AppsSdkIconExportKey = keyof typeof AppsSdkIcons;

/**
 * The package barrel is the public Apps SDK icon catalog. Keeping the
 * namespace here gives the Favorite picker a single, verified export surface
 * while semantic UI code uses the smaller registry below.
 */
export const appsSdkIconRegistry = AppsSdkIcons as unknown as Record<
  string,
  EchoIconComponent
>;

const semanticIcons = {
  back: AppsSdkIcons.ArrowLeft,
  check: AppsSdkIcons.Check,
  close: AppsSdkIcons.X,
  copy: AppsSdkIcons.Copy,
  delete: AppsSdkIcons.Trash,
  edit: AppsSdkIcons.Pencil,
  favorite: AppsSdkIcons.Star,
  favoriteFilled: AppsSdkIcons.StarFilled,
  history: AppsSdkIcons.History,
  pin: AppsSdkIcons.Pin,
  pinFilled: AppsSdkIcons.PinFilled,
  plus: AppsSdkIcons.Plus,
  search: AppsSdkIcons.Search,
  settings: AppsSdkIcons.Settings,
  selectChecked: AppsSdkIcons.SquareCheckCheckboxChecked,
  selectUnchecked: AppsSdkIcons.SquareCheckboxUnchecked,
  themeDark: AppsSdkIcons.DarkMode,
  themeLight: AppsSdkIcons.Sun,
  themeSystem: AppsSdkIcons.SystemMode,
  unpin: AppsSdkIcons.Unpin,
} satisfies Record<string, EchoIconComponent>;

export type EchoIconName = keyof typeof semanticIcons;

export const echoIconRegistry: Readonly<
  Record<EchoIconName, EchoIconComponent>
> = semanticIcons;

export function getEchoIcon(name: string): EchoIconComponent | null {
  return echoIconRegistry[name as EchoIconName] ?? null;
}
