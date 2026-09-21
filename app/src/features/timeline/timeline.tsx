import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { ChevronLeft, ChevronRight, Crosshair, Minus, Plus } from "lucide-react";
import type { EventType } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { eventLabel, primaryEventType } from "@/lib/events";
import { formatClockDuration, formatHms, formatStamp, HOUR } from "@/lib/time";
import { onThemeChange } from "@/lib/theme";
import type { ParsedEvent, ParsedSegment } from "@/queries/recordings";
import { EventGlyph } from "@/components/event-glyph";
import { IconButton } from "@/components/ui/icon-button";
import { extrapolate, type PlaybackClock } from "@/features/playback/clock";
import { DEFAULT_LAYOUT, drawTimeline } from "./draw";
import {
  MAX_SPAN_MS,
  MIN_SPAN_MS,
  clamp,
  decayVelocity,
  eventAtX,
  panByPx,
  releaseVelocity,
  snapToFootage,
  timeToX,
  xToTime,
  zoomAroundTime,
  zoomAt,
  type Range,
  type TimelineView,
} from "./math";
import { readPalette } from "./palette";

export interface TimelineProps {
  /** The selected camera-local day, as UTC epoch ms. */
  day: Range;
  offsetMinutes: number;
  segments: readonly ParsedSegment[];
  events: readonly ParsedEvent[];
  /** Event types to emphasise; others are dimmed. Empty or null shows all. */
  highlightTypes?: ReadonlySet<EventType> | null;
  selectedEventId?: string | null;
  /** Wall-clock now, for today's marker. */
  nowMs?: number | null;
  clock: PlaybackClock;
  onSeek: (ms: number) => void;
  /** Shown under the ruler, left of the zoom controls (e.g. a legend). */
  footer?: ReactNode;
  className?: string;
}

const HEIGHT = 74;
const DEFAULT_SPAN_MS = 3 * HOUR;
const AUTO_FOLLOW_AFTER_MS = 6000;
const CLICK_SLOP_PX = 4;
const MIN_FLING_VELOCITY = 0.05;
/** px/ms; a hard flick coasts about 1300 px. */
const MAX_FLING_VELOCITY = 4;

const easeOutCubic = (k: number) => 1 - (1 - k) ** 3;

interface Glide {
  from: number;
  to: number;
  t0: number;
  dur: number;
  /** Track the moving playhead and resume following when done. */
  thenFollow: boolean;
}

interface ZoomAnim {
  from: number;
  to: number;
  anchorMs: number;
  t0: number;
  dur: number;
}

interface Drag {
  pointerId: number;
  startX: number;
  lastX: number;
  moved: boolean;
  samples: { x: number; t: number }[];
}

interface Mutable {
  view: TimelineView;
  follow: boolean;
  glide: Glide | null;
  zoom: ZoomAnim | null;
  velocity: number;
  drag: Drag | null;
  hoverX: number | null;
  hoverY: number | null;
  hoverId: string | null;
  pointerInside: boolean;
  lastInteraction: number;
  seekSeq: number;
  edge: "left" | "right" | null;
  detached: boolean;
  request: () => void;
}

/**
 * The playback ruler. At rest the view follows the playhead, which sits fixed at the centre
 * with its time bubble while the footage scrolls beneath it. Wheel zooms around the cursor;
 * dragging (with inertia) or Shift+wheel pans to look around; clicking seeks, and the ruler
 * glides back so the new position sits under the centre playhead again.
 */
export function Timeline(props: TimelineProps) {
  const { clock, day, className } = props;
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const bubbleRef = useRef<HTMLDivElement>(null);
  const bubbleTextRef = useRef<HTMLSpanElement>(null);
  const hoverRef = useRef<HTMLDivElement>(null);
  const hoverTimeRef = useRef<HTMLSpanElement>(null);
  const propsRef = useRef(props);
  propsRef.current = props;

  const [hoverEvent, setHoverEvent] = useState<ParsedEvent | null>(null);
  const [detached, setDetached] = useState(false);
  const [edge, setEdge] = useState<"left" | "right" | null>(null);
  const [st] = useState<Mutable>(() => ({
    view: { centerMs: clock.now(), spanMs: DEFAULT_SPAN_MS, widthPx: 0 },
    follow: true,
    glide: null,
    zoom: null,
    velocity: 0,
    drag: null,
    hoverX: null,
    hoverY: null,
    hoverId: null,
    pointerInside: false,
    lastInteraction: 0,
    seekSeq: clock.getState().seekSeq,
    edge: null,
    detached: false,
    request: () => {},
  }));

  const limits = () => ({ bounds: propsRef.current.day, minSpan: MIN_SPAN_MS, maxSpan: MAX_SPAN_MS });

  // --- Render loop ---------------------------------------------------------------------------
  useEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    const ctx = canvas.getContext("2d");
    let palette = readPalette(canvas);
    let raf: number | null = null;
    let lastFrame = 0;
    let lastSecond = Number.NaN;

    const request = () => {
      if (raf === null) raf = requestAnimationFrame(frame);
    };
    st.request = request;

    const startGlide = (now: number, to: number, thenFollow: boolean, dur = 380) => {
      st.velocity = 0;
      st.glide = { from: st.view.centerMs, to, t0: now, dur, thenFollow };
      st.follow = false;
    };

    const updateHover = () => {
      const el = hoverRef.current;
      if (!el) return;
      const p = propsRef.current;
      if (st.hoverX === null || st.drag?.moved) {
        el.style.opacity = "0";
        if (st.hoverId !== null) {
          st.hoverId = null;
          setHoverEvent(null);
        }
        return;
      }
      const t = xToTime(st.view, st.hoverX);
      const overBand =
        st.hoverY !== null &&
        st.hoverY >= DEFAULT_LAYOUT.bandTop - 12 &&
        st.hoverY <= DEFAULT_LAYOUT.bandTop + DEFAULT_LAYOUT.bandHeight + 12;
      const event = overBand ? eventAtX(st.view, st.hoverX, p.events) : undefined;
      const id = event?.id ?? null;
      if (id !== st.hoverId) {
        st.hoverId = id;
        setHoverEvent(event ?? null);
      }
      if (hoverTimeRef.current) hoverTimeRef.current.textContent = formatHms(t, p.offsetMinutes);
      const half = el.offsetWidth / 2;
      const x = clamp(st.hoverX, half, Math.max(half, st.view.widthPx - half));
      el.style.transform = `translateX(${x - half}px)`;
      el.style.opacity = "1";
    };

    const frame = (now: number) => {
      raf = null;
      const p = propsRef.current;
      const dt = lastFrame ? Math.min(64, now - lastFrame) : 16;
      lastFrame = now;
      const cs = clock.getState();
      const playhead = extrapolate(cs, now);
      const bounds = p.day;
      let animating = false;

      // An explicit seek (click, keys, event card): glide so it lands under the centre line.
      if (cs.seekSeq !== st.seekSeq) {
        st.seekSeq = cs.seekSeq;
        if (!st.drag) startGlide(now, clamp(playhead, bounds.start, bounds.end), true);
      }

      if (st.zoom) {
        const z = st.zoom;
        const k = Math.min(1, (now - z.t0) / z.dur);
        const span = z.from + (z.to - z.from) * easeOutCubic(k);
        st.view = zoomAroundTime(st.view, z.anchorMs, span / st.view.spanMs, limits());
        if (k >= 1) st.zoom = null;
        else animating = true;
      }

      if (st.glide) {
        const g = st.glide;
        if (g.thenFollow) g.to = clamp(playhead, bounds.start, bounds.end);
        const k = Math.min(1, (now - g.t0) / g.dur);
        st.view = { ...st.view, centerMs: g.from + (g.to - g.from) * easeOutCubic(k) };
        if (k >= 1) {
          st.glide = null;
          if (g.thenFollow) st.follow = true;
        } else animating = true;
      } else if (st.velocity !== 0 && !st.drag) {
        const before = st.view.centerMs;
        st.view = panByPx(st.view, st.velocity * dt, bounds);
        st.velocity = decayVelocity(st.velocity, dt);
        if (Math.abs(st.velocity) < 0.012 || st.view.centerMs === before) st.velocity = 0;
        else animating = true;
      } else if (st.follow) {
        st.view = { ...st.view, centerMs: clamp(playhead, bounds.start, bounds.end) };
      }

      // Drift back to the playhead after looking around, unless the pointer is still here.
      if (
        !st.follow &&
        !st.glide &&
        !st.drag &&
        st.velocity === 0 &&
        !st.pointerInside &&
        cs.playing &&
        now - st.lastInteraction > AUTO_FOLLOW_AFTER_MS
      ) {
        startGlide(now, clamp(playhead, bounds.start, bounds.end), true, 600);
        animating = true;
      }

      const width = st.view.widthPx;
      if (ctx && width > 0) {
        const dpr = window.devicePixelRatio || 1;
        const cw = Math.round(width * dpr);
        const ch = Math.round(HEIGHT * dpr);
        if (canvas.width !== cw || canvas.height !== ch) {
          canvas.width = cw;
          canvas.height = ch;
        }
        drawTimeline(ctx, {
          width,
          height: HEIGHT,
          dpr,
          view: st.view,
          day: bounds,
          offsetMinutes: p.offsetMinutes,
          segments: p.segments,
          events: p.events,
          highlight: p.highlightTypes ?? null,
          selectedId: p.selectedEventId ?? null,
          hoverId: st.hoverId,
          nowMs: p.nowMs ?? null,
          playheadMs: playhead,
          hoverX: st.drag?.moved ? null : st.hoverX,
          palette,
        });
      }

      // DOM overlays, updated directly: no React renders per frame.
      const px = timeToX(st.view, playhead);
      const inView = px >= 0 && px <= width;
      const bubble = bubbleRef.current;
      if (bubble) {
        bubble.style.transform = `translateX(${clamp(px, 0, width)}px)`;
        bubble.style.opacity = inView ? "1" : "0";
      }
      const second = Math.floor(playhead / 1000);
      if (second !== lastSecond) {
        lastSecond = second;
        if (bubbleTextRef.current) bubbleTextRef.current.textContent = formatHms(playhead, p.offsetMinutes);
        wrap.setAttribute("aria-valuenow", String(Math.round(playhead)));
        wrap.setAttribute("aria-valuetext", formatStamp(playhead, p.offsetMinutes));
      }
      const nextEdge = inView ? null : px < 0 ? "left" : "right";
      if (nextEdge !== st.edge) {
        st.edge = nextEdge;
        setEdge(nextEdge);
      }
      const nextDetached = !st.follow && !(st.glide?.thenFollow ?? false);
      if (nextDetached !== st.detached) {
        st.detached = nextDetached;
        setDetached(nextDetached);
      }
      updateHover();

      if (animating || cs.playing) request();
    };

    const resize = () => {
      st.view = { ...st.view, widthPx: wrap.clientWidth };
      request();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(wrap);
    resize();

    const offClock = clock.subscribe(request);
    const offTheme = onThemeChange(() => {
      palette = readPalette(canvas);
      request();
    });
    void document.fonts?.ready.then(() => {
      palette = readPalette(canvas);
      request();
    });

    // Wheel: zoom around the cursor; Shift+wheel or horizontal scrolling pans.
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
      st.lastInteraction = performance.now();
      st.velocity = 0;
      st.zoom = null;
      if (Math.abs(dx) > Math.abs(dy)) {
        st.follow = false;
        st.glide = null;
        st.view = panByPx(st.view, -dx, propsRef.current.day);
      } else if (dy !== 0) {
        const factor = Math.exp(dy * (e.ctrlKey ? 0.01 : 0.0016));
        const playhead = clock.now();
        const nearPlayhead = Math.abs(x - timeToX(st.view, playhead)) < 24;
        if (st.follow && !st.glide && nearPlayhead) {
          st.view = zoomAroundTime(st.view, playhead, factor, limits());
        } else {
          st.follow = false;
          st.glide = null;
          st.view = zoomAt(st.view, x, factor, limits());
        }
      }
      request();
    };
    wrap.addEventListener("wheel", onWheel, { passive: false });

    return () => {
      if (raf !== null) cancelAnimationFrame(raf);
      observer.disconnect();
      offClock();
      offTheme();
      wrap.removeEventListener("wheel", onWheel);
      st.request = () => {};
    };
  }, [clock, st]);

  // A new day: centre on the playhead again.
  useEffect(() => {
    st.follow = true;
    st.glide = null;
    st.velocity = 0;
    st.view = { ...st.view, centerMs: clamp(clock.now(), day.start, day.end) };
    st.request();
  }, [day.start, day.end, clock, st]);

  // Redraw when data or emphasis changes.
  useEffect(() => {
    st.request();
  }, [props.segments, props.events, props.highlightTypes, props.selectedEventId, props.nowMs, props.offsetMinutes, st]);

  // --- Pointer input -------------------------------------------------------------------------
  const localPoint = (e: ReactPointerEvent) => {
    const rect = wrapRef.current!.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const { x } = localPoint(e);
    e.currentTarget.setPointerCapture(e.pointerId);
    st.drag = { pointerId: e.pointerId, startX: x, lastX: x, moved: false, samples: [{ x, t: e.timeStamp }] };
    st.velocity = 0;
    st.zoom = null;
    st.lastInteraction = performance.now();
    st.request();
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const { x, y } = localPoint(e);
    st.hoverX = x;
    st.hoverY = y;
    st.pointerInside = true;
    const d = st.drag;
    if (d && e.pointerId === d.pointerId) {
      const dx = x - d.lastX;
      d.lastX = x;
      if (!d.moved && Math.abs(x - d.startX) > CLICK_SLOP_PX) {
        d.moved = true;
        st.follow = false;
        st.glide = null;
      }
      if (d.moved) st.view = panByPx(st.view, dx, propsRef.current.day);
      d.samples.push({ x, t: e.timeStamp });
      if (d.samples.length > 24) d.samples.shift();
      st.lastInteraction = performance.now();
    }
    st.request();
  };

  const endDrag = (e: ReactPointerEvent<HTMLDivElement>, cancelled: boolean) => {
    const d = st.drag;
    if (!d || e.pointerId !== d.pointerId) return;
    st.drag = null;
    st.lastInteraction = performance.now();
    if (!cancelled && !d.moved) {
      const p = propsRef.current;
      const t = xToTime(st.view, localPoint(e).x);
      const target = snapToFootage(clamp(t, p.day.start, p.day.end - 1000), p.segments);
      p.onSeek(clamp(target, p.day.start, p.day.end));
    } else if (!cancelled) {
      const v = releaseVelocity(d.samples);
      st.velocity = Math.abs(v) >= MIN_FLING_VELOCITY ? clamp(v, -MAX_FLING_VELOCITY, MAX_FLING_VELOCITY) : 0;
    }
    st.request();
  };

  const onPointerLeave = () => {
    st.pointerInside = false;
    st.hoverX = null;
    st.hoverY = null;
    st.request();
  };

  // --- Buttons -------------------------------------------------------------------------------
  const zoomBy = (factor: number) => {
    const playhead = clock.now();
    const px = timeToX(st.view, playhead);
    const anchorMs = px >= 0 && px <= st.view.widthPx ? playhead : st.view.centerMs;
    const target = clamp(st.view.spanMs * factor, MIN_SPAN_MS, MAX_SPAN_MS);
    st.zoom = { from: st.view.spanMs, to: target, anchorMs, t0: performance.now(), dur: 240 };
    st.request();
  };

  const backToPlayhead = () => {
    st.velocity = 0;
    st.glide = {
      from: st.view.centerMs,
      to: clock.now(),
      t0: performance.now(),
      dur: 420,
      thenFollow: true,
    };
    st.follow = false;
    st.request();
  };

  const hoverType = hoverEvent ? primaryEventType(hoverEvent.types) : null;

  return (
    <div className={cn("relative select-none", className)}>
      {/* Time bubble over the playhead. */}
      <div className="relative h-8">
        <div
          ref={bubbleRef}
          className="pointer-events-none absolute bottom-0 left-0 z-10 will-change-transform"
          style={{ transform: "translateX(-9999px)" }}
          aria-hidden
        >
          <div className="flex -translate-x-1/2 flex-col items-center">
            <span
              ref={bubbleTextRef}
              className="rounded-full bg-brand px-2.5 py-[5px] text-[12px] font-semibold leading-none tabular-nums text-white shadow-raised"
            >
              --:--:--
            </span>
            <span className="h-0 w-0 border-x-[5px] border-t-[5px] border-x-transparent border-t-brand" />
          </div>
        </div>
      </div>

      <div
        ref={wrapRef}
        role="slider"
        tabIndex={0}
        aria-label={strings.playback.timeline.label}
        aria-valuemin={day.start}
        aria-valuemax={day.end}
        aria-orientation="horizontal"
        data-playback-timeline=""
        className={cn(
          "relative touch-none rounded-lg outline-none",
          "cursor-grab active:cursor-grabbing focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface",
        )}
        style={{ height: HEIGHT }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={(e) => endDrag(e, false)}
        onPointerCancel={(e) => endDrag(e, true)}
        onPointerLeave={onPointerLeave}
      >
        <canvas ref={canvasRef} className="absolute inset-0 size-full" />

        {/* Hover: the time, and the event's thumbnail when over one. */}
        <div
          ref={hoverRef}
          className="pointer-events-none absolute bottom-[calc(100%+34px)] left-0 z-30 opacity-0 transition-opacity duration-100"
          aria-hidden
        >
          <div className="flex flex-col items-center gap-1.5">
            {hoverEvent && hoverType && (
              <div className="w-[184px] overflow-hidden rounded-xl border border-card-border bg-popover shadow-overlay">
                {hoverEvent.thumbnailUrl ? (
                  <img src={hoverEvent.thumbnailUrl} alt="" className="aspect-video w-full object-cover" />
                ) : (
                  <div className="aspect-video w-full bg-video" />
                )}
                <div className="flex items-center gap-1.5 px-2.5 py-2">
                  <EventGlyph type={hoverType} size="xs" labelled={false} />
                  <span className="truncate text-xs font-medium text-fg">{eventLabel(hoverType)}</span>
                  <span className="ml-auto text-[11px] tabular-nums text-fg-3">
                    {formatClockDuration(hoverEvent.end - hoverEvent.start)}
                  </span>
                </div>
              </div>
            )}
            <span
              ref={hoverTimeRef}
              className="rounded-md bg-tooltip px-2 py-1 text-[11.5px] font-medium tabular-nums leading-none text-tooltip-foreground shadow-overlay"
            />
          </div>
        </div>
      </div>

      {/* Controls under the ruler. */}
      <div className="mt-1 flex min-h-7 items-center gap-1">
        <div className="min-w-0 flex-1">{props.footer}</div>
        {detached && (
          <button
            type="button"
            onClick={backToPlayhead}
            className="mr-1 inline-flex shrink-0 h-7 items-center gap-1.5 rounded-full bg-brand-soft px-2.5 text-xs font-semibold text-brand-text transition-colors hover:bg-brand-soft-hover"
          >
            {edge === "left" ? (
              <ChevronLeft className="size-3.5" />
            ) : edge === "right" ? (
              <ChevronRight className="size-3.5 order-last" />
            ) : (
              <Crosshair className="size-3.5" />
            )}
            {strings.playback.timeline.backToPlayhead}
          </button>
        )}
        <IconButton label={strings.playback.timeline.zoomOut} size="icon-xs" onClick={() => zoomBy(2)}>
          <Minus />
        </IconButton>
        <IconButton label={strings.playback.timeline.zoomIn} size="icon-xs" onClick={() => zoomBy(0.5)}>
          <Plus />
        </IconButton>
      </div>
    </div>
  );
}
