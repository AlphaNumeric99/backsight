import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { AnimatePresence, motion } from "motion/react";
import { LoaderCircle, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { transitions } from "@/lib/motion";
import { Button, type ButtonProps } from "@/components/ui/button";
import { Tooltip } from "@/components/ui/tooltip";

/** Fullscreen state for an element, following Esc and other exits. */
export function useFullscreen(ref: RefObject<HTMLElement | null>): [boolean, () => void] {
  const [active, setActive] = useState(false);
  useEffect(() => {
    const onChange = () => setActive(document.fullscreenElement === ref.current && ref.current !== null);
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, [ref]);
  const toggle = useCallback(() => {
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
    else void ref.current?.requestFullscreen?.().catch(() => {});
  }, [ref]);
  return [active, toggle];
}

/** Shows overlay controls while the pointer moves or focus is inside, hides them when idle. */
function useControlsVisibility(forceVisible: boolean) {
  const [active, setActive] = useState(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const poke = useCallback(() => {
    setActive(true);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setActive(false), 2600);
  }, []);
  useEffect(() => {
    poke();
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [poke]);
  return { visible: forceVisible || active, poke };
}

export interface VideoStageProps {
  stageRef: RefObject<HTMLDivElement | null>;
  /** The <VideoSurface/>. */
  video: ReactNode;
  topLeft?: ReactNode;
  topRight?: ReactNode;
  bottomLeft?: ReactNode;
  bottomRight?: ReactNode;
  /** Centered overlay (spinner, error, paused…). */
  center?: ReactNode;
  /** Keep controls visible (paused, errors). */
  pinControls?: boolean;
  /** Keep the top-left overlay (e.g. the timestamp) visible while controls hide. */
  stickyTopLeft?: boolean;
  className?: string;
  fullscreen?: boolean;
  onDoubleClick?: () => void;
  onClick?: () => void;
}

export function VideoStage({
  stageRef,
  video,
  topLeft,
  topRight,
  bottomLeft,
  bottomRight,
  center,
  pinControls = false,
  stickyTopLeft = false,
  className,
  fullscreen = false,
  onDoubleClick,
  onClick,
}: VideoStageProps) {
  const { visible, poke } = useControlsVisibility(pinControls);
  return (
    <div
      ref={stageRef}
      onPointerMove={poke}
      onPointerDown={poke}
      onFocusCapture={poke}
      data-controls={visible ? "visible" : "hidden"}
      className={cn(
        "group/stage relative isolate overflow-hidden bg-video text-white",
        fullscreen ? "h-full w-full" : "aspect-video w-full rounded-card shadow-card ring-1 ring-card-border",
        !visible && "cursor-none",
        className,
      )}
    >
      <div className="absolute inset-0" onDoubleClick={onDoubleClick} onClick={onClick}>
        {video}
      </div>

      {center && <div className="pointer-events-none absolute inset-0 z-10 grid place-items-center p-6">{center}</div>}

      <div
        aria-hidden
        className={cn(
          "pointer-events-none absolute inset-x-0 top-0 z-20 h-20 bg-gradient-to-b from-black/55 via-black/20 to-transparent transition-opacity duration-(--dur-base)",
          visible ? "opacity-100" : "opacity-0",
        )}
      />
      <div className="pointer-events-none absolute inset-x-0 top-0 z-20 flex items-start justify-between gap-3 p-3.5">
        <div
          className={cn(
            "pointer-events-auto flex min-w-0 items-center gap-2 transition-opacity duration-(--dur-base)",
            visible || stickyTopLeft ? "opacity-100" : "opacity-0",
          )}
        >
          {topLeft}
        </div>
        <div
          className={cn(
            "flex items-center gap-2 transition-opacity duration-(--dur-base)",
            visible ? "pointer-events-auto opacity-100" : "opacity-0",
          )}
        >
          {topRight}
        </div>
      </div>

      <div
        className={cn(
          "pointer-events-none absolute inset-x-0 bottom-0 z-20 flex items-end justify-between gap-3 bg-gradient-to-t from-black/65 via-black/25 to-transparent p-3 pt-12 transition-opacity duration-(--dur-base)",
          visible ? "opacity-100" : "opacity-0 focus-within:opacity-100",
        )}
      >
        <div className="pointer-events-auto flex items-center gap-1">{bottomLeft}</div>
        <div className="pointer-events-auto flex items-center gap-1">{bottomRight}</div>
      </div>
    </div>
  );
}

/** A round, icon-only control on top of video. */
export function OverlayButton({
  label,
  shortcut,
  className,
  children,
  ...props
}: Omit<ButtonProps, "aria-label" | "variant" | "size"> & { label: string; shortcut?: string }) {
  return (
    <Tooltip content={label} shortcut={shortcut}>
      <Button
        variant="overlay-ghost"
        size="icon"
        aria-label={label}
        className={cn("[&_svg]:size-5", className)}
        {...props}
      >
        {children}
      </Button>
    </Tooltip>
  );
}

export function LiveBadge({ label }: { label: string }) {
  return (
    <span className="inline-flex h-6 items-center gap-1.5 rounded-full bg-live px-2.5 text-[11px] font-bold uppercase tracking-[0.06em] text-white shadow-[0_2px_8px_rgb(240_56_59/0.35)]">
      <span className="size-1.5 animate-live-dot rounded-full bg-white" />
      {label}
    </span>
  );
}

/** Spinner, or an icon with a message and actions, centred over the video. */
export function StageMessage({
  icon: Icon,
  spinner,
  title,
  body,
  actions,
}: {
  icon?: LucideIcon;
  spinner?: boolean;
  title: ReactNode;
  body?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0, scale: 0.97 }}
        animate={{ opacity: 1, scale: 1, transition: transitions.base }}
        className="pointer-events-auto flex max-w-sm flex-col items-center text-center"
      >
        <span className="video-glass mb-3.5 grid size-14 place-items-center rounded-full">
          {spinner ? (
            <LoaderCircle className="size-6 animate-spin" />
          ) : Icon ? (
            <Icon className="size-6" strokeWidth={1.8} />
          ) : null}
        </span>
        <p className="text-[15px] font-semibold text-white text-shadow-video">{title}</p>
        {body && <p className="mt-1 text-[13px] leading-relaxed text-white/75 text-shadow-video">{body}</p>}
        {actions && <div className="mt-4 flex gap-2">{actions}</div>}
      </motion.div>
    </AnimatePresence>
  );
}
