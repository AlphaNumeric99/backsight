import { Button, type ButtonProps } from "./button";
import { Tooltip } from "./tooltip";

export type IconButtonProps = Omit<ButtonProps, "aria-label"> & {
  /** Accessible name, also shown as the tooltip. */
  label: string;
  tooltipSide?: "top" | "right" | "bottom" | "left";
  shortcut?: string;
  /** Set false where a visible label already explains the button. */
  showTooltip?: boolean;
};

/** An icon-only button with an accessible name and a matching tooltip. */
export function IconButton({
  label,
  tooltipSide = "top",
  shortcut,
  showTooltip = true,
  size = "icon",
  variant = "ghost",
  children,
  ...props
}: IconButtonProps) {
  const button = (
    <Button size={size} variant={variant} aria-label={label} {...props}>
      {children}
    </Button>
  );
  return (
    <Tooltip content={label} side={tooltipSide} shortcut={shortcut} disabled={!showTooltip}>
      {button}
    </Tooltip>
  );
}
