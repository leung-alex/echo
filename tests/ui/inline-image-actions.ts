import type { Locator } from "@playwright/test";

export type InlineImageActionState = {
  actionCount: number;
  visibility: string;
  opacity: string;
  pointerEvents: string;
  transitionProperty: string;
  minContrast: number;
};

export async function readInlineImageActionState(
  rail: Locator,
): Promise<InlineImageActionState> {
  return rail.evaluate((element) => {
    const parseColor = (value: string) => {
      const components = value.match(/[\d.]+/g)?.map(Number) ?? [];
      return {
        red: components[0] ?? 0,
        green: components[1] ?? 0,
        blue: components[2] ?? 0,
        alpha: components[3] ?? 1,
      };
    };
    const blend = (
      foreground: ReturnType<typeof parseColor>,
      background: ReturnType<typeof parseColor>,
    ) => ({
      red:
        foreground.red * foreground.alpha +
        background.red * (1 - foreground.alpha),
      green:
        foreground.green * foreground.alpha +
        background.green * (1 - foreground.alpha),
      blue:
        foreground.blue * foreground.alpha +
        background.blue * (1 - foreground.alpha),
      alpha: 1,
    });
    const channelLuminance = (channel: number) => {
      const normalized = channel / 255;
      return normalized <= 0.03928
        ? normalized / 12.92
        : ((normalized + 0.055) / 1.055) ** 2.4;
    };
    const luminance = (color: ReturnType<typeof parseColor>) =>
      0.2126 * channelLuminance(color.red) +
      0.7152 * channelLuminance(color.green) +
      0.0722 * channelLuminance(color.blue);
    const contrast = (
      foreground: ReturnType<typeof parseColor>,
      background: ReturnType<typeof parseColor>,
    ) => {
      const foregroundLuminance = luminance(foreground);
      const backgroundLuminance = luminance(background);
      return (
        (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
        (Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
      );
    };

    const railStyle = getComputedStyle(element);
    const railBackground = blend(parseColor(railStyle.backgroundColor), {
      red: 0,
      green: 0,
      blue: 0,
      alpha: 1,
    });
    const buttonContrasts = Array.from(element.querySelectorAll("button")).map(
      (button) => {
        const buttonStyle = getComputedStyle(button);
        return contrast(
          parseColor(buttonStyle.color),
          blend(parseColor(buttonStyle.backgroundColor), railBackground),
        );
      },
    );

    return {
      actionCount: buttonContrasts.length,
      visibility: railStyle.visibility,
      opacity: railStyle.opacity,
      pointerEvents: railStyle.pointerEvents,
      transitionProperty: railStyle.transitionProperty,
      minContrast: Math.min(...buttonContrasts),
    };
  });
}
