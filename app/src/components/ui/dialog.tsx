import type { ComponentProps, ReactNode } from "react";
import { AlertDialog as AlertPrimitive, Dialog as DialogPrimitive } from "radix-ui";
import { AnimatePresence, motion } from "motion/react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { Button } from "./button";

const overlayMotion = {
  initial: { opacity: 0 },
  animate: { opacity: 1, transition: transitions.base },
  exit: { opacity: 0, transition: transitions.fast },
};

const panelMotion = {
  initial: { opacity: 0, y: 14, scale: 0.97 },
  animate: { opacity: 1, y: 0, scale: 1, transition: transitions.spring },
  exit: { opacity: 0, y: 8, scale: 0.98, transition: transitions.fast },
};

const sizes = {
  sm: "max-w-[420px]",
  md: "max-w-[520px]",
  lg: "max-w-[640px]",
};

export interface DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  description?: ReactNode;
  /** Hide the visible header (the title is still announced). */
  hideHeader?: boolean;
  size?: keyof typeof sizes;
  className?: string;
  children?: ReactNode;
  /** Prevent closing by clicking outside, e.g. while a request is in flight. */
  dismissible?: boolean;
  onOpenAutoFocus?: (event: Event) => void;
}

/** A modal dialog that animates in and out. Controlled: pass `open` and `onOpenChange`. */
export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  hideHeader,
  size = "md",
  className,
  children,
  dismissible = true,
  onOpenAutoFocus,
}: DialogProps) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <AnimatePresence>
        {open && (
          <DialogPrimitive.Portal forceMount key="dialog">
            <DialogPrimitive.Overlay asChild forceMount>
              <motion.div className="fixed inset-0 z-50 bg-scrim backdrop-blur-[3px]" {...overlayMotion} />
            </DialogPrimitive.Overlay>
            <div className="pointer-events-none fixed inset-0 z-50 flex justify-center p-4">
              <DialogPrimitive.Content
                asChild
                forceMount
                {...(description ? {} : { "aria-describedby": undefined })}
                onOpenAutoFocus={onOpenAutoFocus}
                onPointerDownOutside={(e) => !dismissible && e.preventDefault()}
                onEscapeKeyDown={(e) => !dismissible && e.preventDefault()}
              >
                <motion.div
                  className={cn(
                    "pointer-events-auto relative my-auto flex max-h-full w-full flex-col rounded-dialog border border-card-border bg-popover text-fg shadow-overlay outline-none",
                    sizes[size],
                    className,
                  )}
                  {...panelMotion}
                >
                  {hideHeader ? (
                    <>
                      <DialogPrimitive.Title className="sr-only">{title}</DialogPrimitive.Title>
                      {description && (
                        <DialogPrimitive.Description className="sr-only">{description}</DialogPrimitive.Description>
                      )}
                    </>
                  ) : (
                    <div className="shrink-0 px-6 pb-1 pt-6 pr-14">
                      <DialogPrimitive.Title className="text-lg font-semibold tracking-[-0.01em]">
                        {title}
                      </DialogPrimitive.Title>
                      {description && (
                        <DialogPrimitive.Description className="mt-1 text-sm text-fg-2">
                          {description}
                        </DialogPrimitive.Description>
                      )}
                    </div>
                  )}
                  <div className="flex min-h-0 flex-1 flex-col">{children}</div>
                  <DialogPrimitive.Close asChild>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="absolute right-4 top-4"
                      aria-label={strings.common.close}
                      disabled={!dismissible}
                    >
                      <X />
                    </Button>
                  </DialogPrimitive.Close>
                </motion.div>
              </DialogPrimitive.Content>
            </div>
          </DialogPrimitive.Portal>
        )}
      </AnimatePresence>
    </DialogPrimitive.Root>
  );
}

/** Scrolls when the dialog would be taller than the window; header and footer stay put. */
export function DialogBody({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("min-h-0 flex-1 overflow-y-auto px-6 py-4", className)}>{children}</div>;
}

/** A form that lays out a DialogBody and DialogFooter like the dialog itself. */
export function DialogForm({ className, ...props }: ComponentProps<"form">) {
  return <form className={cn("flex min-h-0 flex-1 flex-col", className)} {...props} />;
}

export function DialogFooter({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("flex shrink-0 items-center justify-end gap-2 px-6 pb-6 pt-2", className)}>{children}</div>;
}

export interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  body: ReactNode;
  confirmLabel: string;
  destructive?: boolean;
  loading?: boolean;
  onConfirm: () => void;
}

/** A confirmation step for destructive actions. */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  body,
  confirmLabel,
  destructive,
  loading,
  onConfirm,
}: ConfirmDialogProps) {
  return (
    <AlertPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <AnimatePresence>
        {open && (
          <AlertPrimitive.Portal forceMount key="confirm">
            <AlertPrimitive.Overlay asChild forceMount>
              <motion.div className="fixed inset-0 z-50 bg-scrim backdrop-blur-[3px]" {...overlayMotion} />
            </AlertPrimitive.Overlay>
            <div className="pointer-events-none fixed inset-0 z-50 grid place-items-center p-4">
              <AlertPrimitive.Content asChild forceMount>
                <motion.div
                  className="pointer-events-auto w-full max-w-[420px] rounded-dialog border border-card-border bg-popover p-6 text-fg shadow-overlay outline-none"
                  {...panelMotion}
                >
                  <AlertPrimitive.Title className="text-lg font-semibold tracking-[-0.01em]">{title}</AlertPrimitive.Title>
                  <AlertPrimitive.Description className="mt-2 text-sm leading-relaxed text-fg-2">
                    {body}
                  </AlertPrimitive.Description>
                  <div className="mt-6 flex justify-end gap-2">
                    <AlertPrimitive.Cancel asChild>
                      <Button variant="ghost">{strings.common.cancel}</Button>
                    </AlertPrimitive.Cancel>
                    <Button
                      variant={destructive ? "danger" : "primary"}
                      loading={loading}
                      onClick={(e) => {
                        e.preventDefault();
                        onConfirm();
                      }}
                    >
                      {confirmLabel}
                    </Button>
                  </div>
                </motion.div>
              </AlertPrimitive.Content>
            </div>
          </AlertPrimitive.Portal>
        )}
      </AnimatePresence>
    </AlertPrimitive.Root>
  );
}
