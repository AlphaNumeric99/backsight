import { memo, useEffect, useMemo, useRef, type KeyboardEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { AnimatePresence, motion } from "motion/react";
import { toast } from "sonner";
import { Copy, EllipsisVertical, Play, Scissors } from "lucide-react";
import type { EventType } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { EVENT_TYPES, eventLabel, primaryEventType } from "@/lib/events";
import { formatClockDuration, formatHms, formatStamp } from "@/lib/time";
import type { ParsedEvent } from "@/queries/recordings";
import { Card, Chip, Skeleton } from "@/components/ui/misc";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EventDot, EventGlyph } from "@/components/event-glyph";

const ROW_HEIGHT = 84;

export interface EventsPanelProps {
  events: readonly ParsedEvent[];
  loading: boolean;
  offsetMinutes: number;
  filters: ReadonlySet<EventType>;
  onFiltersChange: (filters: Set<EventType>) => void;
  selectedId: string | null;
  playingId: string | null;
  onSelect: (event: ParsedEvent) => void;
  onClip: (event: ParsedEvent) => void;
  message?: string | null;
}

export function EventsPanel({
  events,
  loading,
  offsetMinutes,
  filters,
  onFiltersChange,
  selectedId,
  playingId,
  onSelect,
  onClip,
  message,
}: EventsPanelProps) {
  const counts = useMemo(() => {
    const m = new Map<EventType, number>();
    for (const e of events) for (const t of e.types) m.set(t, (m.get(t) ?? 0) + 1);
    return m;
  }, [events]);
  const present = EVENT_TYPES.filter((t) => counts.has(t));

  // Newest first, like the Tapo app.
  const visible = useMemo(() => {
    const list = filters.size === 0 ? events : events.filter((e) => e.types.some((t) => filters.has(t)));
    return [...list].reverse();
  }, [events, filters]);

  const toggle = (t: EventType) => {
    const next = new Set(filters);
    if (next.has(t)) next.delete(t);
    else next.add(t);
    onFiltersChange(next);
  };

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 6,
    getItemKey: (i) => visible[i].id,
  });

  // Bring the selected event into view when the selection changes (e.g. from a timeline
  // click), but not on every data refresh, so it never fights the user's own scrolling.
  const visibleRef = useRef(visible);
  useEffect(() => {
    visibleRef.current = visible;
  });
  useEffect(() => {
    if (!selectedId) return;
    const i = visibleRef.current.findIndex((e) => e.id === selectedId);
    if (i >= 0) virtualizer.scrollToIndex(i, { align: "auto" });
  }, [selectedId, virtualizer]);

  const focusRow = (index: number) => {
    const i = Math.max(0, Math.min(visible.length - 1, index));
    virtualizer.scrollToIndex(i, { align: "auto" });
    requestAnimationFrame(() => {
      scrollRef.current?.querySelector<HTMLButtonElement>(`[data-row="${i}"] [data-event-button]`)?.focus();
    });
  };

  const onListKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-row]");
    if (!row) return;
    const i = Number(row.dataset.row);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      focusRow(i + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      focusRow(i - 1);
    }
  };

  const filterKey = [...filters].sort().join(",");

  return (
    <Card className="absolute inset-0 flex flex-col overflow-hidden">
      <div className="border-b border-border px-4 pb-3.5 pt-4">
        <h2 className="text-[15px] font-semibold tracking-[-0.01em] text-fg">
          {strings.playback.events.title(loading ? 0 : events.length)}
        </h2>
        {present.length > 0 && (
          <div role="group" aria-label={strings.playback.events.filterLabel} className="mt-3 flex flex-wrap gap-1.5">
            <Chip selected={filters.size === 0} onClick={() => onFiltersChange(new Set())} count={events.length}>
              {strings.playback.events.all}
            </Chip>
            {present.map((t) => (
              <Chip
                key={t}
                selected={filters.has(t)}
                onClick={() => toggle(t)}
                count={counts.get(t)}
                leading={<EventDot type={t} className={filters.has(t) ? "ring-2 ring-background/40" : undefined} />}
              >
                {eventLabel(t)}
              </Chip>
            ))}
          </div>
        )}
      </div>

      <div
        ref={scrollRef}
        className="relative min-h-0 flex-1 overflow-y-auto px-2.5 py-2.5"
        onKeyDown={onListKeyDown}
        role="list"
        aria-label={strings.playback.events.listLabel}
      >
        {loading ? (
          <div className="grid gap-2">
            {Array.from({ length: 7 }, (_, i) => (
              <div key={i} className="flex items-center gap-3 p-2">
                <Skeleton className="aspect-video w-[118px] rounded-[10px]" />
                <div className="grid flex-1 gap-2">
                  <Skeleton className="h-4 w-20" />
                  <Skeleton className="h-3.5 w-24" />
                </div>
              </div>
            ))}
          </div>
        ) : message ? (
          <p className="px-4 py-10 text-center text-[13px] leading-relaxed text-fg-2">{message}</p>
        ) : visible.length === 0 ? (
          <div className="px-4 py-10 text-center">
            <p className="text-[13px] text-fg-2">
              {events.length === 0 ? strings.playback.events.empty : strings.playback.events.emptyFiltered}
            </p>
            {filters.size > 0 && (
              <button
                type="button"
                onClick={() => onFiltersChange(new Set())}
                className="mt-2 text-[13px] font-semibold text-brand-text hover:underline"
              >
                {strings.playback.events.clearFilters}
              </button>
            )}
          </div>
        ) : (
          <AnimatePresence mode="wait" initial={false}>
            <motion.div
              key={filterKey}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0, transition: transitions.base }}
              exit={{ opacity: 0, transition: transitions.fast }}
              style={{ height: virtualizer.getTotalSize(), position: "relative" }}
            >
              {virtualizer.getVirtualItems().map((item) => {
                const event = visible[item.index];
                return (
                  <div
                    key={item.key}
                    role="listitem"
                    data-row={item.index}
                    className="absolute inset-x-0 top-0"
                    style={{ height: ROW_HEIGHT, transform: `translateY(${item.start}px)` }}
                  >
                    <EventCard
                      event={event}
                      offsetMinutes={offsetMinutes}
                      selected={event.id === selectedId}
                      playing={event.id === playingId}
                      onSelect={onSelect}
                      onClip={onClip}
                    />
                  </div>
                );
              })}
            </motion.div>
          </AnimatePresence>
        )}
      </div>
    </Card>
  );
}

const EventCard = memo(function EventCard({
  event,
  offsetMinutes,
  selected,
  playing,
  onSelect,
  onClip,
}: {
  event: ParsedEvent;
  offsetMinutes: number;
  selected: boolean;
  playing: boolean;
  onSelect: (event: ParsedEvent) => void;
  onClip: (event: ParsedEvent) => void;
}) {
  const type = primaryEventType(event.types);
  const time = formatHms(event.start, offsetMinutes);
  const duration = formatClockDuration(event.end - event.start);
  return (
    <div
      className={cn(
        "group relative flex h-[76px] items-center gap-3 rounded-[14px] p-2 pr-1 transition-[background-color,box-shadow] duration-(--dur-fast)",
        selected ? "bg-brand-softer ring-2 ring-brand" : "hover:bg-hover",
      )}
    >
      <div className="relative aspect-video w-[108px] shrink-0 overflow-hidden rounded-[10px] bg-video">
        {event.thumbnailUrl && (
          <img src={event.thumbnailUrl} alt="" loading="lazy" decoding="async" className="size-full object-cover" />
        )}
        <span className="absolute bottom-1 right-1 rounded-md bg-black/65 px-1.5 py-[3px] font-mono text-[10px] font-semibold leading-none tabular-nums text-white">
          {duration}
        </span>
        {playing && (
          <span className="absolute left-1 top-1 inline-flex h-4 items-center gap-[2px] rounded bg-brand px-1" aria-hidden>
            {[0, 0.2, 0.4].map((d) => (
              <span key={d} className="w-[2px] animate-pulse-dot rounded-full bg-white" style={{ height: 8, animationDelay: `${d}s` }} />
            ))}
          </span>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="text-[14px] font-semibold tabular-nums text-fg">{time}</span>
          {playing && <span className="text-[11px] font-semibold text-brand-text">{strings.playback.events.nowPlaying}</span>}
        </div>
        <div className="mt-1.5 flex items-center gap-1">
          {event.types.map((t) => (
            <EventGlyph key={t} type={t} size="xs" />
          ))}
          <span className="ml-1 truncate text-xs text-fg-2">{eventLabel(type)}</span>
        </div>
      </div>
      <button
        type="button"
        data-event-button=""
        onClick={() => onSelect(event)}
        aria-label={`${eventLabel(type)}, ${time}, ${duration}`}
        aria-current={selected || undefined}
        className="absolute inset-0 rounded-[14px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={strings.playback.events.menu}
            className="relative z-10 grid size-8 shrink-0 place-items-center rounded-full text-fg-3 opacity-70 transition-[opacity,background-color,color] hover:bg-hover hover:text-fg group-hover:opacity-100 data-[state=open]:bg-hover data-[state=open]:opacity-100"
          >
            <EllipsisVertical className="size-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => onSelect(event)}>
            <Play />
            {strings.playback.events.playFrom}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => onClip(event)}>
            <Scissors />
            {strings.playback.events.clipEvent}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              void navigator.clipboard?.writeText(formatStamp(event.start, offsetMinutes));
              toast(strings.playback.events.timeCopied);
            }}
          >
            <Copy />
            {strings.playback.events.copyTime}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
});
