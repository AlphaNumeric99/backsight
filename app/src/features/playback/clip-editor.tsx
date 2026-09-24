import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type Ref,
} from "react";
import { useNavigate } from "@tanstack/react-router";
import { motion } from "motion/react";
import { toast } from "sonner";
import { Scissors, TriangleAlert, X } from "lucide-react";
import type { Camera } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { describeError, toApiError } from "@/lib/errors";
import { formatClockDuration, formatHms, formatStamp, HOUR, MINUTE, SECOND, toIso } from "@/lib/time";
import { onThemeChange } from "@/lib/theme";
import type { ParsedEvent, ParsedSegment } from "@/queries/recordings";
import { useStartExport } from "@/queries/exports";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { drawTimeline, type TimelineLayout } from "@/features/timeline/draw";
import {
  MAX_SPAN_MS,
  MIN_SPAN_MS,
  clamp,
  msPerPx,
  panByPx,
  timeToX,
  zoomAt,
  type Range,
  type TimelineView,
} from "@/features/timeline/math";
import { readPalette, type TimelinePalette } from "@/features/timeline/palette";

const HEIGHT = 84;
const LAYOUT: TimelineLayout = { labelY: 15, tickTop: 21, bandTop: 40, bandHeight: 30 };
const LONG_CLIP_MS = HOUR;
const MIN_CLIP_MS = SECOND;

type DragKind = "start" | "end" | "move" | "pan";

export interface ClipEditorProps {
  camera: Camera;
  day: Range;
  offsetMinutes: number;
  segments: readonly ParsedSegment[];
  events: readonly ParsedEvent[];
  /** Wall-clock now when viewing today (clips can't reach past the recorded edge). */
  nowMs: number | null;
  initial: Range;
  /** Seek the player so the frame at the selection edge is visible. */
  onPreview: (ms: number) => void;
  onClose: () => void;
}

export function ClipEditor({
  camera,
  day,
  offsetMinutes,
  segments,
  events,
  nowMs,
  initial,
  onPreview,
  onClose,
}: ClipEditorProps) {
  const limitEnd = nowMs !== null ? Math.min(day.end, nowMs - 20 * SECOND) : day.end;
  const [sel, setSel] = useState<Range>(() => {
    const start = clamp(Math.round(initial.start / SECOND) * SECOND, day.start, limitEnd - MIN_CLIP_MS);
    const end = clamp(Math.round(initial.end / SECOND) * SECOND, start + MIN_CLIP_MS, limitEnd);
    return { start, end };
  });
  const [view, setView] = useState<TimelineView>(() => ({
    centerMs: (sel.start + sel.end) / 2,
    // Zoomed so the selection fills about a quarter of the ruler.
    spanMs: clamp((sel.end - sel.start) * 4, MIN_SPAN_MS, MAX_SPAN_MS),
    widthPx: 0,
  }));
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const paletteRef = useRef<TimelinePalette | null>(null);
  const drag = useRef<{ kind: DragKind; pointerId: number; startX: number; sel: Range; view: TimelineView } | null>(null);
  const [dragging, setDragging] = useState<DragKind | null>(null);
  const start = useStartExport();
  const navigate = useNavigate();

  const length = sel.end - sel.start;
  const long = length > LONG_CLIP_MS;

  // Size and palette.
  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    const canvas = canvasRef.current;
    if (!wrap || !canvas) return;
    paletteRef.current = readPalette(canvas);
    const resize = () => setView((v) => ({ ...v, widthPx: wrap.clientWidth }));
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(wrap);
    const offTheme = onThemeChange(() => {
      paletteRef.current = readPalette(canvas);
      setView((v) => ({ ...v }));
    });
    return () => {
      observer.disconnect();
      offTheme();
    };
  }, []);

  // Draw (coalesced to one frame).
  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    const palette = paletteRef.current;
    if (!canvas || !ctx || !palette || view.widthPx <= 0) return;
    const raf = requestAnimationFrame(() => {
      const dpr = window.devicePixelRatio || 1;
      const cw = Math.round(view.widthPx * dpr);
      const ch = Math.round(HEIGHT * dpr);
      if (canvas.width !== cw || canvas.height !== ch) {
        canvas.width = cw;
        canvas.height = ch;
      }
      drawTimeline(ctx, {
        width: view.widthPx,
        height: HEIGHT,
        dpr,
        view,
        day,
        offsetMinutes,
        segments,
        events,
        highlight: null,
        selectedId: null,
        hoverId: null,
        nowMs,
        playheadMs: null,
        hoverX: null,
        palette,
        layout: LAYOUT,
        selection: sel,
      });
    });
    return () => cancelAnimationFrame(raf);
  }, [view, sel, day, offsetMinutes, segments, events, nowMs]);

  // Esc closes the editor from anywhere on the page (menus and dialogs handle their own Esc).
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  });
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest("[role=dialog],[role=alertdialog],[role=menu],[role=listbox]")) return;
      onCloseRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Keyboard users land on the start handle.
  const startHandleRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    startHandleRef.current?.focus({ preventScroll: true });
  }, []);

  // Wheel zooms around the cursor, Shift+wheel pans.
  useEffect(() => {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = wrap.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? rect.width : 1;
      let dx = e.deltaX * unit;
      let dy = e.deltaY * unit;
      if (e.shiftKey && Math.abs(dx) < Math.abs(dy)) {
        dx = dy;
        dy = 0;
      }
      setView((v) =>
        Math.abs(dx) > Math.abs(dy)
          ? panByPx(v, -dx, day)
          : zoomAt(v, x, Math.exp(dy * (e.ctrlKey ? 0.01 : 0.0016)), { bounds: day }),
      );
    };
    wrap.addEventListener("wheel", onWheel, { passive: false });
    return () => wrap.removeEventListener("wheel", onWheel);
  }, [day]);

  const localX = (e: ReactPointerEvent) => e.clientX - wrapRef.current!.getBoundingClientRect().left;

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const handle = (e.target as HTMLElement).closest<HTMLElement>("[data-handle]")?.dataset.handle as DragKind | undefined;
    const x = localX(e);
    const inside = x >= timeToX(view, sel.start) && x <= timeToX(view, sel.end);
    const kind: DragKind = handle ?? (inside ? "move" : "pan");
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { kind, pointerId: e.pointerId, startX: x, sel, view };
    setDragging(kind);
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    const dx = localX(e) - d.startX;
    const dt = dx * msPerPx(d.view);
    const snap = (t: number) => Math.round(t / SECOND) * SECOND;
    if (d.kind === "pan") {
      setView(panByPx(d.view, dx, day));
    } else if (d.kind === "start") {
      setSel({ start: clamp(snap(d.sel.start + dt), day.start, d.sel.end - MIN_CLIP_MS), end: d.sel.end });
    } else if (d.kind === "end") {
      setSel({ start: d.sel.start, end: clamp(snap(d.sel.end + dt), d.sel.start + MIN_CLIP_MS, limitEnd) });
    } else {
      const len = d.sel.end - d.sel.start;
      const s = clamp(snap(d.sel.start + dt), day.start, limitEnd - len);
      setSel({ start: s, end: s + len });
    }
  };

  const onPointerUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || d.pointerId !== e.pointerId) return;
    drag.current = null;
    setDragging(null);
    if (d.kind === "start" || d.kind === "move") onPreview(sel.start);
    else if (d.kind === "end") onPreview(Math.max(sel.start, sel.end - 5 * SECOND));
  };

  /** Keeps a time on screen after keyboard nudges. */
  const reveal = (t: number) =>
    setView((v) => {
      const x = timeToX(v, t);
      const margin = 40;
      if (x >= margin && x <= v.widthPx - margin) return v;
      return { ...v, centerMs: t };
    });

  const onHandleKey = (edge: "start" | "end") => (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key.startsWith("Page") ? MINUTE : e.shiftKey ? 10 * SECOND : SECOND;
    const dir = e.key === "ArrowLeft" || e.key === "ArrowDown" || e.key === "PageDown" ? -1 : e.key === "ArrowRight" || e.key === "ArrowUp" || e.key === "PageUp" ? 1 : 0;
    let next: Range | null = null;
    if (dir !== 0) {
      next =
        edge === "start"
          ? { start: clamp(sel.start + dir * step, day.start, sel.end - MIN_CLIP_MS), end: sel.end }
          : { start: sel.start, end: clamp(sel.end + dir * step, sel.start + MIN_CLIP_MS, limitEnd) };
    } else if (e.key === "Home") {
      next = edge === "start" ? { start: day.start, end: sel.end } : { start: sel.start, end: sel.start + MIN_CLIP_MS };
    } else if (e.key === "End") {
      next = edge === "start" ? { start: sel.end - MIN_CLIP_MS, end: sel.end } : { start: sel.start, end: limitEnd };
    }
    if (!next) return;
    e.preventDefault();
    e.stopPropagation();
    setSel(next);
    reveal(edge === "start" ? next.start : next.end);
  };

  const submit = () => {
    start.mutate(
      { cameraId: camera.id, start: toIso(sel.start), end: toIso(sel.end) },
      {
        onSuccess: () => {
          toast.success(strings.clip.toastTitle, {
            description: `${camera.name} · ${formatHms(sel.start, offsetMinutes)}–${formatHms(sel.end, offsetMinutes)}`,
            action: { label: strings.clip.toastAction, onClick: () => void navigate({ to: "/downloads" }) },
          });
          onClose();
        },
        onError: (err) => toast.error(strings.clip.failed, { description: describeError(toApiError(err)).body }),
      },
    );
  };

  const x0 = timeToX(view, sel.start);
  const x1 = timeToX(view, sel.end);

  return (
    <motion.section
      data-theme="dark"
      aria-label={strings.clip.title}
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0, transition: transitions.slow }}
      exit={{ opacity: 0, y: 8, transition: transitions.fast }}
      className="theme-scope rounded-card border border-card-border bg-[#101319] p-4 pb-5 shadow-raised"
    >
      <div className="flex items-center gap-3">
        <span className="grid size-8 place-items-center rounded-full bg-brand-soft text-brand-text">
          <Scissors className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-[15px] font-semibold text-fg">{strings.clip.title}</h2>
          <p className="truncate text-xs text-fg-2">{strings.clip.hint}</p>
        </div>
        <div className="text-right">
          <p className="text-[11px] font-medium uppercase tracking-wider text-fg-3">{strings.clip.selectedLength}</p>
          <p className="text-[22px] font-semibold leading-tight tabular-nums text-fg" aria-live="polite">
            {formatClockDuration(length)}
          </p>
        </div>
        <IconButton label={strings.clip.close} onClick={onClose} className="-mr-1 self-start">
          <X />
        </IconButton>
      </div>

      <div className="mt-2 flex justify-between text-xs tabular-nums text-fg-2">
        <span>{formatStamp(sel.start, offsetMinutes)}</span>
        <span>{formatStamp(sel.end, offsetMinutes)}</span>
      </div>

      <div
        ref={wrapRef}
        className={cn(
          "relative mt-1.5 touch-none overflow-hidden rounded-xl bg-white/[0.03]",
          dragging === "pan" ? "cursor-grabbing" : "cursor-grab",
        )}
        style={{ height: HEIGHT }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        <canvas ref={canvasRef} className="absolute inset-0 size-full" />
        <div
          className={cn(
            "absolute bottom-1 top-[34px] border-y-[3px] border-brand bg-brand/10",
            dragging === "move" ? "cursor-grabbing" : "cursor-move",
          )}
          style={{ left: x0, width: Math.max(0, x1 - x0) }}
        >
          <ClipHandle
            handleRef={startHandleRef}
            edge="start"
            valueMs={sel.start}
            min={day.start}
            max={sel.end - MIN_CLIP_MS}
            offsetMinutes={offsetMinutes}
            onKeyDown={onHandleKey("start")}
            active={dragging === "start"}
          />
          <ClipHandle
            edge="end"
            valueMs={sel.end}
            min={sel.start + MIN_CLIP_MS}
            max={limitEnd}
            offsetMinutes={offsetMinutes}
            onKeyDown={onHandleKey("end")}
            active={dragging === "end"}
          />
        </div>
      </div>

      {long && (
        <motion.p
          initial={{ opacity: 0, y: -4 }}
          animate={{ opacity: 1, y: 0 }}
          role="status"
          className="mt-3 flex items-start gap-2 rounded-xl bg-warning-soft px-3 py-2 text-[12.5px] text-warning"
        >
          <TriangleAlert className="mt-px size-4 shrink-0" />
          {strings.clip.longWarning}
        </motion.p>
      )}

      <Button variant="primary" size="lg" className="mt-4 w-full" loading={start.isPending} onClick={submit}>
        {start.isPending ? strings.clip.submitting : strings.clip.submit}
      </Button>
    </motion.section>
  );
}

function ClipHandle({
  handleRef,
  edge,
  valueMs,
  min,
  max,
  offsetMinutes,
  onKeyDown,
  active,
}: {
  handleRef?: Ref<HTMLDivElement>;
  edge: "start" | "end";
  valueMs: number;
  min: number;
  max: number;
  offsetMinutes: number;
  onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => void;
  active: boolean;
}) {
  return (
    <div
      ref={handleRef}
      role="slider"
      tabIndex={0}
      data-handle={edge}
      aria-label={edge === "start" ? strings.clip.start : strings.clip.end}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={valueMs}
      aria-valuetext={formatHms(valueMs, offsetMinutes)}
      onKeyDown={onKeyDown}
      className={cn(
        "absolute -bottom-[3px] -top-[3px] grid w-4 cursor-ew-resize place-items-center bg-brand outline-none transition-[filter] duration-(--dur-fast) hover:brightness-110",
        "focus-visible:ring-2 focus-visible:ring-white focus-visible:ring-offset-2 focus-visible:ring-offset-[#101319]",
        edge === "start" ? "-left-4 rounded-l-[9px]" : "-right-4 rounded-r-[9px]",
        active && "brightness-110",
      )}
    >
      <span className="h-5 w-[3px] rounded-full bg-white/90" aria-hidden />
    </div>
  );
}
