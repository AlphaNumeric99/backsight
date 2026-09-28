import type { ComponentProps } from "react";
import { Switch as SwitchPrimitive } from "radix-ui";
import { cn } from "@/lib/utils";

export function Switch({ className, ...props }: ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      className={cn(
        "peer inline-flex h-6 w-10 shrink-0 items-center rounded-full border border-transparent bg-sunken p-0.5 transition-colors duration-(--dur-base)",
        "data-[state=checked]:bg-brand disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb className="block size-[18px] rounded-full bg-white shadow-[0_1px_3px_rgb(0_0_0/0.25)] transition-transform duration-(--dur-base) ease-standard data-[state=checked]:translate-x-4" />
    </SwitchPrimitive.Root>
  );
}
