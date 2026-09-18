import type { CSSProperties } from "react";
import { Toaster as Sonner } from "sonner";
import { CircleAlert, CircleCheck, Info, LoaderCircle, TriangleAlert } from "lucide-react";
import { useResolvedTheme } from "@/lib/theme";

/** App-wide toasts (sonner), themed with Backsight tokens. */
export function Toaster() {
  const theme = useResolvedTheme();
  return (
    <Sonner
      theme={theme}
      position="bottom-right"
      offset={20}
      gap={10}
      visibleToasts={4}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--foreground)",
          "--normal-border": "var(--card-border)",
          "--border-radius": "16px",
          "--width": "368px",
        } as CSSProperties
      }
      icons={{
        success: <CircleCheck className="size-[18px] text-success-dot" />,
        error: <CircleAlert className="size-[18px] text-danger-dot" />,
        info: <Info className="size-[18px] text-brand-text" />,
        warning: <TriangleAlert className="size-[18px] text-warning-dot" />,
        loading: <LoaderCircle className="size-[18px] animate-spin text-fg-2" />,
      }}
      toastOptions={{
        classNames: {
          toast: "!shadow-overlay !px-4 !py-3.5 !gap-3 !font-sans !items-start",
          title: "!text-[13.5px] !font-semibold !leading-5",
          description: "!text-[12.5px] !leading-[1.45] !text-fg-2 !mt-0.5",
          icon: "!mt-[1px]",
          actionButton:
            "!h-7 !rounded-full !bg-brand-soft !px-3 !text-xs !font-semibold !text-brand-text hover:!bg-brand-soft-hover !self-center",
          cancelButton: "!h-7 !rounded-full !bg-surface-3 !px-3 !text-xs !font-medium !text-fg !self-center",
        },
      }}
    />
  );
}
