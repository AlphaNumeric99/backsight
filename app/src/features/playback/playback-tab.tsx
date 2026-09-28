import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  CalendarCheck,
  CalendarX,
  Camera as CameraIcon,
  CircleAlert,
  Keyboard,
  Maximize,
  Minimize,
  Pause,
  Play,
  Radio,
  Scissors,
  Volume2,
  VolumeX,
} from "lucide-react";
import type { Camera, EventType, LocalDate, StreamRequest } from "@/ipc";
import type { PlayerState, VideoSurfaceHandle } from "@/player/types";
import { VideoSurface } from "@/player/VideoSurface";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { describeError, toApiError } from "@/lib/errors";
import { EVENT_TYPES, eventLabel } from "@/lib/events";
import { addDays, dayRange, formatStamp, localOffsetMinutes, MINUTE, SECOND, toIso, todayIn } from "@/lib/time";
import type { ParsedEvent, ParsedSegment } from "@/queries/recordings";
import { useDayIndex } from "@/queries/recordings";
import { Button } from "@/components/ui/button";
import { Card, Kbd } from "@/components/ui/misc";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { IconButton } from "@/components/ui/icon-button";
import { EventDot } from "@/components/event-glyph";
import { useStatusInfo } from "@/components/camera-status";
import { useNow } from "@/hooks/use-now";
import { Timeline } from "@/features/timeline/timeline";
import { clamp, snapToFootage, stepSpeed, type Range } from "@/features/timeline/math";
import { OverlayButton, StageMessage, VideoStage, useFullscreen } from "@/features/camera/video-stage";
import { useSnapshot } from "@/features/camera/snapshot";
import { createClock, useClockSelector, useClockTime, type PlaybackClock } from "./clock";
import { DateChip } from "./date-chip";
import { EventsPanel } from "./events-panel";
import { SpeedMenu } from "./speed-menu";
import { ClipEditor } from "./clip-editor";
import { usePlaybackHotkeys } from "./use-playback-hotkeys";

const NO_SEGMENTS: ParsedSegment[] = [];
const NO_EVENTS: ParsedEvent[] = [];

export interface PlaybackTabProps {
  camera: Camera;
  /** From the URL; defaults to today. */
  date?: LocalDate;
  /** From the URL: where to start, epoch ms. Consumed once. */
  initialTime?: number;
  onDateChange: (date: LocalDate) => void;
  onInitialTimeConsumed: () => void;
  onGoLive: () => void;
}

export function PlaybackTab({
  camera,
  date: dateParam,
  initialTime,
  onDateChange,
  onInitialTimeConsumed,
  onGoLive,
}: PlaybackTabProps) {
  const offset = camera.utcOffsetMinutes ?? localOffsetMinutes();
  const wallNow = useNow(15_000);
  const today = todayIn(offset, wallNow);
  const date = dateParam && dateParam <= today ? dateParam : today;
  const isToday = date === today;
  const day = useMemo(() => dayRange(date, offset), [date, offset]);
  const info = useStatusInfo(camera.status);
  const index = useDayIndex(camera.id, date, { isToday, enabled: info.viewable });
  const segments = index.data?.segments ?? NO_SEGMENTS;
  const events = index.data?.events ?? NO_EVENTS;

  const [clock] = useState(() => createClock(initialTime ?? day.start));
  const [speed, setSpeed] = useState(1);
  const [paused, setPaused] = useState(false);
  /** Where the current stream started; a new object restarts the stream. */
  const [origin, setOrigin] = useState<{ ms: number; nonce: number } | null>(null);
  const [muted, setMuted] = useState(true);
  const [playerState, setPlayerState] = useState<PlayerState>({ kind: "idle" });
  const [poster, setPoster] = useState<string | undefined>();
  const [filters, setFilters] = useState<Set<EventType>>(() => new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [clip, setClip] = useState<Range | null>(null);
  const [simulated, setSimulated] = useState(false);
  const videoRef = useRef<VideoSurfaceHandle>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const [fullscreen, toggleFullscreen] = useFullscreen(stageRef);
  const snapshot = useSnapshot(camera, videoRef);
  const lastPlayerSignal = useRef(0);

  // --- Position --------------------------------------------------------------------------------

  const startAt = useCallback(
    (ms: number) => {
      clock.seek(ms);
      setOrigin((o) => ({ ms, nonce: (o?.nonce ?? 0) + 1 }));
      setPaused(false);
      setPoster(undefined);
    },
    [clock],
  );

  const seek = useCallback(
    (ms: number) => {
      if (segments.length === 0) {
        clock.seek(clamp(ms, day.start, day.end));
        return;
      }
      const t = snapToFootage(clamp(ms, day.start, day.end - SECOND), segments);
      startAt(clamp(t, day.start, day.end));
    },
    [clock, day, segments, startAt],
  );

  // Choose where to start once a day's index arrives. Until then, stop the previous day's
  // stream and park the playhead at the start of the new day.
  const positionedFor = useRef<string | null>(null);
  const parkedFor = useRef<string | null>(null);
  useEffect(() => {
    const data = index.data;
    if (positionedFor.current === date) return;
    if (!data) {
      if (parkedFor.current !== date && positionedFor.current !== null) {
        parkedFor.current = date;
        setOrigin(null);
        setSelectedId(null);
        clock.seek(day.start);
      }
      return;
    }
    positionedFor.current = date;
    const segs = data.segments;
    const wanted = initialTime !== undefined && initialTime >= day.start && initialTime < day.end ? initialTime : undefined;
    let t: number;
    if (wanted !== undefined) t = wanted;
    else if (segs.length === 0) t = day.start;
    else if (isToday) {
      const last = segs[segs.length - 1];
      t = Math.max(last.start, last.end - MINUTE);
    } else t = segs[0].start;

    setSelectedId(wanted !== undefined ? (data.events.find((e) => wanted >= e.start && wanted < e.end)?.id ?? null) : null);
    if (segs.length === 0) {
      clock.seek(t);
      setOrigin(null);
    } else {
      const s = snapToFootage(t, segs);
      startAt(s);
    }
    if (initialTime !== undefined) onInitialTimeConsumed();
  }, [index.data, date, day, isToday, initialTime, clock, startAt, onInitialTimeConsumed]);

  const source = useMemo<StreamRequest | null>(
    () =>
      origin && !paused && info.viewable
        ? { kind: "playback", cameraId: camera.id, start: toIso(origin.ms), speed }
        : null,
    [origin, paused, info.viewable, camera.id, speed],
  );

  const changeSpeed = (next: number) => {
    if (next === speed) return;
    const at = clock.now();
    clock.setSpeed(next);
    setSpeed(next);
    if (origin && !paused) setOrigin((o) => ({ ms: at, nonce: (o?.nonce ?? 0) + 1 }));
  };

  const togglePlay = async () => {
    if (!origin) {
      if (segments.length > 0) seek(clock.now());
      return;
    }
    if (paused) {
      startAt(clock.now());
      return;
    }
    clock.setPlaying(false);
    let frame: string | undefined;
    try {
      const png = await videoRef.current?.snapshot();
      if (png) frame = URL.createObjectURL(png);
    } catch {
      // No frame to freeze on (e.g. still connecting): pause without a poster.
    }
    setPoster(frame);
    setPaused(true);
  };

  useEffect(
    () => () => {
      if (poster) URL.revokeObjectURL(poster);
    },
    [poster],
  );

  // --- Player ----------------------------------------------------------------------------------

  const onTime = useCallback(
    (ms: number) => {
      lastPlayerSignal.current = performance.now();
      clock.report(ms);
    },
    [clock],
  );
  const onState = useCallback((s: PlayerState) => {
    lastPlayerSignal.current = performance.now();
    setPlayerState(s);
  }, []);

  useEffect(() => {
    setPlayerState({ kind: "idle" });
    setSimulated(false);
  }, [source]);

  useEffect(() => {
    clock.setPlaying(!paused && source !== null && (playerState.kind === "playing" || simulated));
  }, [clock, paused, source, playerState.kind, simulated]);

  useSimulatedPlayback({ source, clock, segments, lastPlayerSignal, setSimulated, setPlayerState });

  // --- Events ----------------------------------------------------------------------------------

  const playingId = useClockSelector(clock, (t) => {
    for (const e of events) if (t >= e.start && t < e.end) return e.id;
    return null;
  });

  const selectEvent = (e: ParsedEvent) => {
    setSelectedId(e.id);
    seek(e.start);
  };

  const onTimelineSeek = (ms: number) => {
    setSelectedId(events.find((e) => ms >= e.start && ms < e.end)?.id ?? null);
    seek(ms);
  };

  const openClip = (range?: Range) => {
    const at = clock.now();
    setClip(range ?? { start: at, end: at + 2 * MINUTE });
  };

  // --- Keyboard --------------------------------------------------------------------------------

  usePlaybackHotkeys(clip === null, (action) => {
    switch (action.type) {
      case "toggle-play":
        void togglePlay();
        break;
      case "seek-by":
        seek(clock.now() + action.ms);
        break;
      case "speed":
        changeSpeed(stepSpeed(speed, action.dir));
        break;
      case "seek-to":
        seek(action.where === "start" ? (segments[0]?.start ?? day.start) : (segments.at(-1)?.end ?? day.end) - 10 * SECOND);
        break;
      case "live":
        onGoLive();
        break;
    }
  });

  // --- Overlays --------------------------------------------------------------------------------

  let center: ReactNode = null;
  if (!info.viewable) {
    center = <StageMessage icon={info.icon} title={strings.playback.offlineTitle} body={info.detail} />;
  } else if (index.isError) {
    const copy = describeError(toApiError(index.error));
    center = (
      <StageMessage
        icon={CircleAlert}
        title={copy.title}
        body={copy.body}
        actions={<StageButton onClick={() => void index.refetch()}>{strings.common.retry}</StageButton>}
      />
    );
  } else if (index.data && segments.length === 0) {
    center = (
      <StageMessage icon={CalendarX} title={strings.playback.noRecordingsDay} body={strings.playback.noRecordingsDayBody} />
    );
  } else if (playerState.kind === "error") {
    center = (
      <StageMessage
        icon={CircleAlert}
        title={strings.live.streamError}
        body={playerState.code === "codec_unsupported" ? strings.live.codecUnsupported : playerState.message}
        actions={<StageButton onClick={() => startAt(clock.now())}>{strings.common.retry}</StageButton>}
      />
    );
  } else if (playerState.kind === "ended") {
    center = (
      <StageMessage
        icon={CalendarCheck}
        title={strings.playback.endOfDay}
        actions={
          date < today ? (
            <StageButton onClick={() => onDateChange(addDays(date, 1))}>{strings.playback.nextDayCta}</StageButton>
          ) : undefined
        }
      />
    );
  } else if (paused) {
    center = (
      <button
        type="button"
        onClick={() => void togglePlay()}
        aria-label={strings.playback.play}
        className="video-glass pointer-events-auto grid size-16 place-items-center rounded-full transition-transform hover:scale-105 active:scale-95"
      >
        <Play className="ml-1 size-7 fill-current" />
      </button>
    );
  } else if (index.isPending || playerState.kind === "connecting" || playerState.kind === "buffering") {
    center = <StageMessage spinner title={playerState.kind === "buffering" ? strings.live.buffering : strings.live.connecting} />;
  }

  // While a seek into an event connects, show that event's thumbnail as the poster.
  const originThumbnail = useMemo(() => {
    if (!origin) return undefined;
    return events.find((e) => origin.ms >= e.start && origin.ms < e.end)?.thumbnailUrl;
  }, [origin, events]);

  const presentTypes = useMemo(() => EVENT_TYPES.filter((t) => events.some((e) => e.types.includes(t))), [events]);
  const highlight = filters.size > 0 ? filters : null;
  const playing = source !== null && !paused;

  return (
    <div className="grid gap-5 [grid-template-columns:minmax(0,1fr)_340px] max-[1199px]:[grid-template-columns:minmax(0,1fr)_292px]">
      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <DateChip cameraId={camera.id} date={date} today={today} onChange={onDateChange} />
          <div className="flex-1" />
          <ShortcutsPopover />
          <Button variant="outline" size="sm" onClick={onGoLive} disabled={!info.viewable}>
            <Radio className="text-live" />
            {strings.playback.goLive}
            <Kbd className="ml-0.5 h-5 min-w-5 text-[10px]">{strings.keys.live}</Kbd>
          </Button>
        </div>

        <div className="mx-auto w-full" // Tall enough to fill the window without scrolling: header, toolbar and timeline take ~344 px.
          style={{ maxWidth: "calc((100dvh - 344px) * 16 / 9)", minWidth: "min(100%, 480px)" }}>
          <VideoStage
            stageRef={stageRef}
            fullscreen={fullscreen}
            pinControls={paused || !info.viewable || playerState.kind === "error" || playerState.kind === "ended"}
            stickyTopLeft
            onDoubleClick={toggleFullscreen}
            video={
              <VideoSurface
                ref={videoRef}
                source={source}
                muted={muted}
                fit="contain"
                posterUrl={poster ?? originThumbnail}
                onState={onState}
                onTime={onTime}
                className="size-full"
              />
            }
            center={center}
            topLeft={info.viewable && <TimestampOverlay clock={clock} offsetMinutes={offset} speed={speed} />}
            topRight={info.viewable && <SpeedMenu speed={speed} onChange={changeSpeed} />}
            bottomLeft={
              <>
                <OverlayButton
                  label={playing ? strings.playback.pause : strings.playback.play}
                  shortcut={strings.keys.space}
                  onClick={() => void togglePlay()}
                  disabled={!info.viewable || segments.length === 0}
                >
                  {playing ? <Pause className="fill-current" /> : <Play className="fill-current" />}
                </OverlayButton>
                <OverlayButton label={muted ? strings.playback.unmute : strings.playback.mute} onClick={() => setMuted((m) => !m)}>
                  {muted ? <VolumeX /> : <Volume2 />}
                </OverlayButton>
              </>
            }
            bottomRight={
              <>
                <OverlayButton label={strings.playback.snapshot} onClick={snapshot.take} disabled={!playing || snapshot.busy}>
                  <CameraIcon />
                </OverlayButton>
                <OverlayButton
                  label={strings.playback.clip}
                  onClick={() => (clip ? setClip(null) : openClip())}
                  disabled={!info.viewable || segments.length === 0}
                  aria-pressed={clip !== null}
                  className={cn(clip && "bg-white/20 text-white")}
                >
                  <Scissors />
                </OverlayButton>
                <OverlayButton
                  label={fullscreen ? strings.playback.exitFullscreen : strings.playback.fullscreen}
                  onClick={toggleFullscreen}
                >
                  {fullscreen ? <Minimize /> : <Maximize />}
                </OverlayButton>
              </>
            }
          />
        </div>

        <AnimatePresence mode="wait" initial={false}>
          {clip ? (
            <ClipEditor
              key="clip"
              camera={camera}
              day={day}
              offsetMinutes={offset}
              segments={segments}
              events={events}
              nowMs={isToday ? wallNow : null}
              initial={clip}
              onPreview={seek}
              onClose={() => setClip(null)}
            />
          ) : (
            <motion.div
              key="timeline"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0, transition: transitions.slow }}
              exit={{ opacity: 0, y: 6, transition: transitions.fast }}
            >
              <Card className="px-4 pb-2.5 pt-1">
                <Timeline
                  day={day}
                  offsetMinutes={offset}
                  segments={segments}
                  events={events}
                  highlightTypes={highlight}
                  selectedEventId={selectedId}
                  nowMs={isToday ? wallNow : null}
                  clock={clock}
                  onSeek={onTimelineSeek}
                  footer={
                    presentTypes.length > 0 && (
                      <ul className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px] text-fg-2">
                        <li className="inline-flex items-center gap-1.5">
                          <span className="h-2 w-3.5 rounded-sm" style={{ background: "var(--tl-band-strong)" }} aria-hidden />
                          {strings.playback.timeline.footage}
                        </li>
                        {presentTypes.map((t) => (
                          <li key={t} className="inline-flex items-center gap-1.5">
                            <EventDot type={t} />
                            {eventLabel(t)}
                          </li>
                        ))}
                      </ul>
                    )
                  }
                />
              </Card>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <aside className="relative min-h-[440px]">
        <EventsPanel
          events={events}
          loading={info.viewable && index.isPending}
          offsetMinutes={offset}
          filters={filters}
          onFiltersChange={setFilters}
          selectedId={selectedId}
          playingId={playingId}
          onSelect={selectEvent}
          onClip={(e) => openClip({ start: e.start - 2 * SECOND, end: e.end + 2 * SECOND })}
          message={!info.viewable ? info.detail : index.isError ? describeError(toApiError(index.error)).body : null}
        />
      </aside>
    </div>
  );
}

function StageButton({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-full bg-white px-4 py-2 text-sm font-semibold text-[#0f1729] transition-colors hover:bg-white/90"
    >
      {children}
    </button>
  );
}

function TimestampOverlay({ clock, offsetMinutes, speed }: { clock: PlaybackClock; offsetMinutes: number; speed: number }) {
  const t = useClockTime(clock);
  return (
    <span className="inline-flex items-center gap-2">
      <span className="rounded-lg bg-black/45 px-2.5 py-1 font-mono text-[12.5px] font-medium tabular-nums text-white backdrop-blur-sm">
        {formatStamp(t, offsetMinutes)}
      </span>
      {speed !== 1 && (
        <span className="rounded-full bg-brand px-2 py-0.5 text-[11px] font-bold tabular-nums text-white">
          {strings.playback.speedValue(speed)}
        </span>
      )}
    </span>
  );
}

function ShortcutsPopover() {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <span>
          <IconButton label={strings.playback.shortcuts} size="icon-sm" variant="ghost">
            <Keyboard />
          </IconButton>
        </span>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-0">
        <p className="border-b border-border px-4 py-3 text-sm font-semibold">{strings.playback.shortcuts}</p>
        <dl className="grid gap-2.5 px-4 py-3.5">
          {strings.playback.shortcutList.map(([keys, what]) => (
            <div key={keys} className="flex items-center justify-between gap-4 text-[13px]">
              <dt className="text-fg-2">{what}</dt>
              <dd className="flex shrink-0 gap-1">
                {keys.split(" ").map((k) => (
                  <Kbd key={k}>{k}</Kbd>
                ))}
              </dd>
            </div>
          ))}
        </dl>
      </PopoverContent>
    </Popover>
  );
}

/**
 * Development aid: until the player engine lands, the placeholder <VideoSurface/> never reports
 * time. If a stream gets no state or time callbacks for 1.5 s, advance the clock at the chosen
 * speed (~10 Hz, like the real player) so the timeline, overlays and "now playing" can be
 * exercised. It switches itself off as soon as a real player reports anything, and is compiled
 * out of production builds.
 */
function useSimulatedPlayback({
  source,
  clock,
  segments,
  lastPlayerSignal,
  setSimulated,
  setPlayerState,
}: {
  source: StreamRequest | null;
  clock: PlaybackClock;
  segments: readonly ParsedSegment[];
  lastPlayerSignal: RefObject<number>;
  setSimulated: (on: boolean) => void;
  setPlayerState: (s: PlayerState) => void;
}) {
  const segmentsRef = useRef(segments);
  useLayoutEffect(() => {
    segmentsRef.current = segments;
  }, [segments]);
  useEffect(() => {
    if (!import.meta.env.DEV || !source) return;
    const began = performance.now();
    let running = false;
    const timer = setInterval(() => {
      if (lastPlayerSignal.current > began) {
        if (running) setSimulated(false);
        clearInterval(timer);
        return;
      }
      if (!running) {
        if (performance.now() - began < 1500) return;
        running = true;
        setSimulated(true);
      }
      const s = clock.getState();
      const segs = segmentsRef.current;
      const last = segs[segs.length - 1];
      let next = s.timeMs + 100 * s.speed;
      if (last && next >= last.end) {
        clock.report(last.end);
        clearInterval(timer);
        setSimulated(false);
        setPlayerState({ kind: "ended" });
        return;
      }
      next = snapToFootage(next, segs);
      clock.report(next);
    }, 100);
    return () => clearInterval(timer);
  }, [source, clock, lastPlayerSignal, setSimulated, setPlayerState]);
}
