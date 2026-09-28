import type { ComponentProps } from "react";
import { Slider as SliderPrimitive } from "radix-ui";
import { cn } from "@/lib/utils";

export function Slider({ className, "aria-label": ariaLabel, ...props }: ComponentProps<typeof SliderPrimitive.Root>) {
  return (
    <SliderPrimitive.Root
      className={cn("relative flex h-5 w-full touch-none select-none items-center", className)}
      {...props}
    >
      <SliderPrimitive.Track className="relative h-1.5 grow overflow-hidden rounded-full bg-sunken">
        <SliderPrimitive.Range className="absolute h-full rounded-full bg-brand" />
      </SliderPrimitive.Track>
      <SliderPrimitive.Thumb
        aria-label={ariaLabel}
        className="block size-[18px] rounded-full border-2 border-brand bg-white shadow-[0_1px_4px_rgb(0_0_0/0.2)] transition-transform hover:scale-110"
      />
    </SliderPrimitive.Root>
  );
}
