import type { ComponentProps } from "react";
import { DropdownMenu as MenuPrimitive } from "radix-ui";
import { Check } from "lucide-react";
import { cn } from "@/lib/utils";
import { floatingPanelClass } from "./popover";

export const DropdownMenu = MenuPrimitive.Root;
export const DropdownMenuTrigger = MenuPrimitive.Trigger;
export const DropdownMenuGroup = MenuPrimitive.Group;
export const DropdownMenuRadioGroup = MenuPrimitive.RadioGroup;

export function DropdownMenuContent({
  className,
  sideOffset = 6,
  align = "end",
  ...props
}: ComponentProps<typeof MenuPrimitive.Content>) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Content
        sideOffset={sideOffset}
        align={align}
        collisionPadding={12}
        className={cn(
          floatingPanelClass,
          "min-w-44 origin-(--radix-dropdown-menu-content-transform-origin) rounded-xl p-1.5",
          className,
        )}
        {...props}
      />
    </MenuPrimitive.Portal>
  );
}

const itemClass = cn(
  "relative flex h-9 cursor-default select-none items-center gap-2.5 rounded-lg px-2.5 text-sm text-fg outline-none",
  "data-[highlighted]:bg-hover data-[disabled]:pointer-events-none data-[disabled]:opacity-45",
  "[&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-fg-2",
);

export function DropdownMenuItem({
  className,
  destructive,
  ...props
}: ComponentProps<typeof MenuPrimitive.Item> & { destructive?: boolean }) {
  return (
    <MenuPrimitive.Item
      className={cn(itemClass, destructive && "text-danger [&_svg]:text-danger data-[highlighted]:bg-danger-soft", className)}
      {...props}
    />
  );
}

export function DropdownMenuRadioItem({ className, children, ...props }: ComponentProps<typeof MenuPrimitive.RadioItem>) {
  return (
    <MenuPrimitive.RadioItem className={cn(itemClass, "pr-8", className)} {...props}>
      {children}
      <MenuPrimitive.ItemIndicator className="absolute right-2.5 flex items-center">
        <Check className="!text-brand-text" />
      </MenuPrimitive.ItemIndicator>
    </MenuPrimitive.RadioItem>
  );
}

export function DropdownMenuLabel({ className, ...props }: ComponentProps<typeof MenuPrimitive.Label>) {
  return (
    <MenuPrimitive.Label
      className={cn("px-2.5 pb-1 pt-1.5 text-[11px] font-semibold uppercase tracking-wider text-fg-3", className)}
      {...props}
    />
  );
}

export function DropdownMenuSeparator({ className, ...props }: ComponentProps<typeof MenuPrimitive.Separator>) {
  return <MenuPrimitive.Separator className={cn("-mx-1.5 my-1.5 h-px bg-border", className)} {...props} />;
}
