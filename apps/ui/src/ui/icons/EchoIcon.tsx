import type { SVGProps } from "react";

import {
  getFavoriteIconComponent,
  type FavoriteIconKey,
} from "./favorite-icon-catalog";
import { getEchoIcon, type EchoIconName } from "./registry";

type SharedIconProps = Omit<SVGProps<SVGSVGElement>, "name"> & {
  size?: number | string;
};

export interface EchoIconProps extends SharedIconProps {
  name: EchoIconName;
}

/** Render a semantic Echo icon without leaking Apps SDK choices to features. */
export function EchoIcon({
  name,
  size = 16,
  className,
  width,
  height,
  ...props
}: EchoIconProps) {
  const Icon = getEchoIcon(name);
  if (!Icon) return null;

  return (
    <Icon
      {...props}
      className={["echo-icon", className].filter(Boolean).join(" ")}
      width={width ?? size}
      height={height ?? size}
    />
  );
}

export interface FavoriteIconProps extends Omit<SharedIconProps, "name"> {
  iconKey: FavoriteIconKey | string | null | undefined;
}

/**
 * Unknown or intentionally empty persisted keys return no element. This is
 * deliberate: callers can omit the icon slot and never leave a layout gap.
 */
export function FavoriteIcon({
  iconKey,
  size = 16,
  className,
  width,
  height,
  ...props
}: FavoriteIconProps) {
  const Icon = getFavoriteIconComponent(iconKey);
  if (!Icon) return null;

  return (
    <Icon
      {...props}
      className={["echo-icon", className].filter(Boolean).join(" ")}
      width={width ?? size}
      height={height ?? size}
    />
  );
}
