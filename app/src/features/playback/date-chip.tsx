import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CalendarDays, ChevronDown, ChevronLeft, ChevronRight, LoaderCircle } from "lucide-react";
import { api, type CameraId, type LocalDate } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import {
  addDays,
  addMonths,
  formatLocalDateLabel,
  formatMonthLabel,
  localeWeekStart,
  monthGrid,
  monthOf,
  parseLocalDate,
  weekdayLabels,
} from "@/lib/time";
import { IconButton } from "@/components/ui/icon-button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { queryKeys } from "@/queries/keys";
import { useDaysWithRecordings } from "@/queries/recordings";

export function dateChipLabel(date: LocalDate, today: LocalDate): string {
  if (date === today) return strings.playback.today;
  if (date === addDays(today, -1)) return strings.playback.yesterday;
  const sameYear = date.slice(0, 4) === today.slice(0, 4);
  return formatLocalDateLabel(
    date,
    sameYear ? { weekday: "short", day: "numeric", month: "short" } : { day: "numeric", month: "short", year: "numeric" },
  );
}

/** "‹ Tue, 29 Sep ›" with a month calendar marking days that have footage. */
export function DateChip({
  cameraId,
  date,
  today,
  onChange,
}: {
  cameraId: CameraId;
  date: LocalDate;
  today: LocalDate;
  onChange: (date: LocalDate) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="inline-flex h-10 items-center gap-0.5 rounded-full border border-card-border bg-surface p-1 shadow-xs">
      <IconButton label={strings.playback.prevDay} size="icon-sm" onClick={() => onChange(addDays(date, -1))}>
        <ChevronLeft />
      </IconButton>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <button
            type="button"
            aria-label={`${strings.playback.chooseDate}: ${formatLocalDateLabel(date, { dateStyle: "full" })}`}
            className="flex h-8 items-center gap-2 rounded-full px-3 text-sm font-semibold text-fg transition-colors hover:bg-hover"
          >
            <CalendarDays className="size-4 text-brand-text" />
            <span className="tabular-nums">{dateChipLabel(date, today)}</span>
            <ChevronDown className={cn("size-3.5 text-fg-3 transition-transform", open && "rotate-180")} />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-[312px] p-3"
          // The calendar focuses the selected day itself.
          onOpenAutoFocus={(e) => e.preventDefault()}
        >
          <MonthCalendar
            cameraId={cameraId}
            selected={date}
            today={today}
            onSelect={(d) => {
              onChange(d);
              setOpen(false);
            }}
          />
        </PopoverContent>
      </Popover>
      <IconButton
        label={strings.playback.nextDay}
        size="icon-sm"
        disabled={date >= today}
        onClick={() => onChange(addDays(date, 1))}
      >
        <ChevronRight />
      </IconButton>
    </div>
  );
}

/** A keyboard-navigable month grid (arrows, Home/End, PageUp/PageDown) with recording dots. */
export function MonthCalendar({
  cameraId,
  selected,
  today,
  onSelect,
}: {
  cameraId: CameraId;
  selected: LocalDate;
  today: LocalDate;
  onSelect: (date: LocalDate) => void;
}) {
  const [month, setMonth] = useState(monthOf(selected));
  const [focused, setFocused] = useState<LocalDate>(selected);
  const weekStart = useMemo(() => localeWeekStart(), []);
  const days = useDaysWithRecordings(cameraId, month);
  const qc = useQueryClient();
  const refs = useRef(new Map<LocalDate, HTMLButtonElement>());
  // Focus the selected day when the calendar opens.
  const shouldFocus = useRef(true);

  // Warm the neighbouring month so paging feels instant.
  useEffect(() => {
    const prev = addMonths(month, -1);
    void qc.prefetchQuery({
      queryKey: queryKeys.days(cameraId, prev),
      queryFn: () => api.getDaysWithRecordings(cameraId, prev),
      staleTime: 5 * 60_000,
    });
  }, [qc, cameraId, month]);

  useEffect(() => {
    if (!shouldFocus.current) return;
    shouldFocus.current = false;
    refs.current.get(focused)?.focus();
  }, [focused, month]);

  const weeks = monthGrid(month, weekStart);
  const has = days.data;
  const enabled = (d: LocalDate) => d <= today && (d === selected || d === today || (has?.has(d) ?? false));

  const moveFocus = (d: LocalDate) => {
    if (d > today) d = today;
    shouldFocus.current = true;
    setFocused(d);
    if (monthOf(d) !== month) setMonth(monthOf(d));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const dow = (new Date(Date.UTC(parseLocalDate(focused).year, parseLocalDate(focused).month - 1, parseLocalDate(focused).day)).getUTCDay() - weekStart + 7) % 7;
    const map: Record<string, () => LocalDate> = {
      ArrowLeft: () => addDays(focused, -1),
      ArrowRight: () => addDays(focused, 1),
      ArrowUp: () => addDays(focused, -7),
      ArrowDown: () => addDays(focused, 7),
      Home: () => addDays(focused, -dow),
      End: () => addDays(focused, 6 - dow),
      PageUp: () => shiftMonth(focused, -1),
      PageDown: () => shiftMonth(focused, 1),
    };
    const next = map[e.key];
    if (next) {
      e.preventDefault();
      moveFocus(next());
    }
  };

  const nextMonth = addMonths(month, 1);
  return (
    <div>
      <div className="mb-2 flex items-center justify-between px-1">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-fg" aria-live="polite">
            {formatMonthLabel(month)}
          </span>
          {days.isFetching && <LoaderCircle className="size-3.5 animate-spin text-fg-3" aria-hidden />}
        </div>
        <div className="flex items-center gap-0.5">
          <IconButton label={strings.playback.prevMonth} size="icon-xs" onClick={() => setMonth(addMonths(month, -1))}>
            <ChevronLeft />
          </IconButton>
          <IconButton
            label={strings.playback.nextMonth}
            size="icon-xs"
            disabled={nextMonth > monthOf(today)}
            onClick={() => setMonth(nextMonth)}
          >
            <ChevronRight />
          </IconButton>
        </div>
      </div>

      <div role="grid" aria-label={formatMonthLabel(month)} onKeyDown={onKeyDown}>
        <div role="row" className="grid grid-cols-7">
          {weekdayLabels(weekStart, "narrow").map((w, i) => (
            <span key={i} role="columnheader" className="grid h-8 place-items-center text-[11px] font-semibold text-fg-3">
              {w}
            </span>
          ))}
        </div>
        {weeks.map((week, wi) => (
          <div role="row" key={wi} className="grid grid-cols-7">
            {week.map((d, di) =>
              d === null ? (
                <span key={di} role="gridcell" className="h-10" />
              ) : (
                <span key={d} role="gridcell" aria-selected={d === selected} className="grid h-10 place-items-center">
                  <button
                    ref={(el) => {
                      if (el) refs.current.set(d, el);
                      else refs.current.delete(d);
                    }}
                    type="button"
                    tabIndex={d === focused ? 0 : -1}
                    disabled={!enabled(d)}
                    onClick={() => onSelect(d)}
                    onFocus={() => setFocused(d)}
                    aria-label={`${formatLocalDateLabel(d, { dateStyle: "full" })}${has?.has(d) ? `, ${strings.playback.hasRecordings}` : ""}`}
                    aria-current={d === today ? "date" : undefined}
                    className={cn(
                      "relative grid size-9 place-items-center rounded-full text-[13px] font-medium tabular-nums transition-colors duration-(--dur-fast)",
                      d === selected
                        ? "bg-brand text-white shadow-xs"
                        : d === today
                          ? "text-brand-text hover:bg-brand-soft"
                          : "text-fg hover:bg-hover",
                      "disabled:cursor-default disabled:text-fg-3/60 disabled:hover:bg-transparent",
                    )}
                  >
                    {parseLocalDate(d).day}
                    {has?.has(d) && (
                      <span
                        aria-hidden
                        className={cn(
                          "absolute bottom-[5px] size-1 rounded-full",
                          d === selected ? "bg-white" : "bg-brand",
                        )}
                      />
                    )}
                  </button>
                </span>
              ),
            )}
          </div>
        ))}
      </div>

      <div className="mt-2 flex items-center justify-between border-t border-border px-1 pt-2.5">
        <span className="inline-flex items-center gap-1.5 text-xs text-fg-2">
          <span className="size-1.5 rounded-full bg-brand" aria-hidden />
          {strings.playback.recordingsLegend}
        </span>
        <button
          type="button"
          onClick={() => onSelect(today)}
          className="rounded-md px-1.5 py-0.5 text-xs font-semibold text-brand-text hover:bg-brand-soft"
        >
          {strings.playback.today}
        </button>
      </div>
    </div>
  );
}

function shiftMonth(date: LocalDate, n: number): LocalDate {
  const { day } = parseLocalDate(date);
  const target = addMonths(monthOf(date), n);
  const [y, m] = target.split("-").map(Number);
  const last = new Date(Date.UTC(y, m, 0)).getUTCDate();
  return `${target}-${String(Math.min(day, last)).padStart(2, "0")}`;
}
