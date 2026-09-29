import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { Camera as CameraIcon, Circle, Clock, Gauge, Maximize, Minimize, Square, Volume2, VolumeX, Wifi } from "lucide-react";
import type { Camera, StreamQuality, StreamRequest } from "@/ipc";
import type { PlayerState, PlayerStats, VideoSurfaceHandle } from "@/player/types";
import { VideoSurface } from "@/player/VideoSurface";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { describeError, toApiError } from "@/lib/errors";
import { formatBitrate, formatBytes, formatPercent, formatResolution } from "@/lib/format";
import { eventLabel, primaryEventType } from "@/lib/events";
import {
  formatClockDuration,
  formatHm,
  formatHms,
  formatUtcOffset,
  localOffsetMinutes,
  SECOND,
  toIso,
  todayIn,
} from "@/lib/time";
import { Card, Skeleton } from "@/components/ui/misc";
import { Segmented } from "@/components/ui/segmented";
import { Tooltip } from "@/components/ui/tooltip";
import { EventGlyph } from "@/components/event-glyph";
import { SdCardIcon } from "@/components/icons";
import { storageSummary } from "@/components/storage-usage";
import { useStatusInfo } from "@/components/camera-status";
import { useNow } from "@/hooks/use-now";
import { useSettings } from "@/queries/settings";
import { useDayIndex } from "@/queries/recordings";
import { useStartExport } from "@/queries/exports";
import { useLivePreview } from "@/features/previews/use-live-preview";
import { LiveBadge, OverlayButton, StageMessage, VideoStage, useFullscreen } from "./video-stage";
import { StatsPopover } from "./stats-popover";
import { useSnapshot } from "./snapshot";

export function LiveTab({ camera }: { camera: Camera }) {
  const settings = useSettings();
  const [quality, setQuality] = useState<StreamQuality>(settings.data?.defaultLiveQuality ?? "hd");
  const qualityTouched = useRef(false);
  useEffect(() => {
    if (!qualityTouched.current && settings.data) setQuality(settings.data.defaultLiveQuality);
  }, [settings.data]);

  const [muted, setMuted] = useState(true);
  const [playerState, setPlayerState] = useState<PlayerState>({ kind: "idle" });
  const [stats, setStats] = useState<PlayerStats | null>(null);
  const statsRef = useRef<{ at: number }>({ at: 0 });
  const [retry, setRetry] = useState(0);
  const videoRef = useRef<VideoSurfaceHandle>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const [fullscreen, toggleFullscreen] = useFullscreen(stageRef);
  const info = useStatusInfo(camera.status);
  const snapshot = useSnapshot(camera, videoRef);
  useLivePreview(camera.id, videoRef, playerState.kind === "playing");
  const recording = useLiveRecording(camera);
  const offset = camera.utcOffsetMinutes ?? localOffsetMinutes();
  const now = useNow(1000);

  const source = useMemo<StreamRequest | null>(
    () => (info.viewable ? { kind: "live", cameraId: camera.id, quality } : null),
    // `retry` is a dependency on purpose: bumping it restarts the stream after an error.
    [info.viewable, camera.id, quality, retry],
  );

  useEffect(() => {
    setStats(null);
    setPlayerState({ kind: "idle" });
  }, [source]);

  const onStats = (s: PlayerStats) => {
    const t = performance.now();
    if (t - statsRef.current.at < 1000) return;
    statsRef.current.at = t;
    setStats(s);
  };

  let center: ReactNode = null;
  if (!info.viewable) {
    center = (
      <StageMessage
        icon={info.icon}
        title={info.label}
        body={info.detail}
      />
    );
  } else if (playerState.kind === "connecting" || playerState.kind === "buffering") {
    center = <StageMessage spinner title={playerState.kind === "connecting" ? strings.live.connecting : strings.live.buffering} />;
  } else if (playerState.kind === "error") {
    center = (
      <StageMessage
        icon={Wifi}
        title={strings.live.streamError}
        body={playerState.code === "codec_unsupported" ? strings.live.codecUnsupported : playerState.message}
        actions={
          <button
            type="button"
            onClick={() => setRetry((n) => n + 1)}
            className="rounded-full bg-white px-4 py-2 text-sm font-semibold text-[#0f1729] hover:bg-white/90"
          >
            {strings.live.reconnect}
          </button>
        }
      />
    );
  }

  const recordAvailable = info.viewable && camera.storage?.present && camera.storage.recordingMode === "continuous";

  return (
    <div className="grid gap-5 [grid-template-columns:minmax(0,1fr)_320px] max-[1199px]:[grid-template-columns:minmax(0,1fr)_280px]">
      <div className="flex min-w-0 flex-col gap-4">
        <div className="mx-auto w-full" style={{ maxWidth: "calc((100dvh - 330px) * 16 / 9)", minWidth: "min(100%, 480px)" }}>
          <VideoStage
            stageRef={stageRef}
            fullscreen={fullscreen}
            pinControls={!info.viewable || playerState.kind === "error"}
            // LIVE, REC and the camera clock stay on screen, like a camera's own overlay.
            stickyTopLeft
            onDoubleClick={toggleFullscreen}
            video={
              <VideoSurface
                ref={videoRef}
                source={source}
                muted={muted}
                fit="contain"
                posterUrl={camera.snapshotUrl}
                onState={setPlayerState}
                onStats={onStats}
                className="size-full"
              />
            }
            center={center}
            topLeft={
              <>
                {info.viewable && <LiveBadge label={strings.live.badge} />}
                {recording.active && (
                  <span className="inline-flex h-6 items-center gap-1.5 rounded-full bg-black/55 px-2.5 text-[11px] font-semibold tabular-nums text-white">
                    <span className="size-2 animate-pulse-dot rounded-full bg-live" />
                    {strings.live.rec} {formatClockDuration(now - (recording.startedAt ?? now))}
                  </span>
                )}
                <span className="truncate text-[13px] font-medium tabular-nums text-white/90 text-shadow-video">
                  {formatHms(now, offset)}
                </span>
              </>
            }
            topRight={info.viewable && <StatsPopover stats={stats} />}
            bottomLeft={
              <>
                <OverlayButton label={muted ? strings.live.unmute : strings.live.mute} onClick={() => setMuted((m) => !m)}>
                  {muted ? <VolumeX /> : <Volume2 />}
                </OverlayButton>
                <Segmented
                  aria-label={strings.live.quality}
                  tone="overlay"
                  size="sm"
                  value={quality}
                  onValueChange={(q) => {
                    qualityTouched.current = true;
                    setQuality(q);
                  }}
                  items={[
                    { value: "hd", label: strings.live.hd },
                    { value: "sd", label: strings.live.sd },
                  ]}
                  className="ml-1"
                />
              </>
            }
            bottomRight={
              <>
                <OverlayButton label={strings.live.snapshot} onClick={snapshot.take} disabled={!info.viewable || snapshot.busy}>
                  <CameraIcon />
                </OverlayButton>
                <OverlayButton label={fullscreen ? strings.live.exitFullscreen : strings.live.fullscreen} onClick={toggleFullscreen}>
                  {fullscreen ? <Minimize /> : <Maximize />}
                </OverlayButton>
              </>
            }
          />
        </div>

        <Card className="flex items-center justify-center gap-10 px-6 py-4 max-[1099px]:gap-7">
          <ActionButton
            label={strings.live.snapshot}
            onClick={snapshot.take}
            disabled={!info.viewable || snapshot.busy}
            icon={<CameraIcon />}
          />
          <Tooltip content={strings.live.recordUnavailable} disabled={Boolean(recordAvailable)}>
            <span>
              <ActionButton
                label={recording.active ? strings.live.stopRecording : strings.live.record}
                onClick={recording.toggle}
                disabled={!recordAvailable || recording.saving}
                active={recording.active}
                icon={recording.active ? <Square className="fill-current" /> : <Circle className="fill-current" />}
                tone="record"
              />
            </span>
          </Tooltip>
          <ActionButton
            label={strings.live.sound}
            onClick={() => setMuted((m) => !m)}
            disabled={!info.viewable}
            active={!muted}
            icon={muted ? <VolumeX /> : <Volume2 />}
          />
          <ActionButton
            label={strings.live.fullscreen}
            onClick={toggleFullscreen}
            disabled={!info.viewable}
            icon={<Maximize />}
          />
        </Card>

        <StatusTiles camera={camera} quality={quality} stats={stats} now={now} />
      </div>

      <TodayPanel camera={camera} />
    </div>
  );
}

function ActionButton({
  label,
  icon,
  onClick,
  disabled,
  active,
  tone = "default",
}: {
  label: string;
  icon: ReactNode;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  tone?: "default" | "record";
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-pressed={active}
      className="group flex w-[72px] flex-col items-center gap-2 disabled:opacity-40"
    >
      <span
        className={cn(
          "grid size-12 place-items-center rounded-full transition-[background-color,color,transform] duration-(--dur-fast) group-active:scale-95 [&_svg]:size-[21px]",
          tone === "record"
            ? active
              ? "bg-live text-white shadow-[0_4px_14px_rgb(240_56_59/0.35)]"
              : "bg-danger-soft text-danger group-hover:bg-danger-soft/80 [&_svg]:size-4"
            : active
              ? "bg-brand text-white"
              : "bg-surface-3 text-fg group-hover:bg-surface-3-hover",
          tone === "record" && active && "[&_svg]:size-4",
        )}
      >
        {icon}
      </span>
      <span className="text-xs font-medium text-fg-2">{label}</span>
    </button>
  );
}

function StatusTiles({
  camera,
  quality,
  stats,
  now,
}: {
  camera: Camera;
  quality: StreamQuality;
  stats: PlayerStats | null;
  now: number;
}) {
  const info = useStatusInfo(camera.status);
  const storage = storageSummary(camera.storage, camera.status.state);
  const offset = camera.utcOffsetMinutes ?? localOffsetMinutes();
  const resolution = formatResolution(stats?.width, stats?.height);
  const hasCard = Boolean(camera.storage?.present && camera.storage.status === "normal");
  const tiles = [
    {
      icon: <Wifi />,
      label: strings.live.tiles.connection,
      value: info.viewable ? info.label : strings.status[camera.status.state === "auth_failed" ? "authFailed" : "offline"],
      sub: camera.host,
      tone: info.tone,
    },
    {
      icon: <Gauge />,
      label: strings.live.tiles.stream,
      value: `${quality.toUpperCase()}${resolution ? ` · ${resolution.replace(" × ", "×")}` : ""}`,
      sub: stats ? `${Math.round(stats.fps)} fps · ${formatBitrate(stats.bitrate)}` : strings.live.tiles.noVideo,
    },
    {
      icon: <SdCardIcon />,
      label: strings.live.tiles.storage,
      value: !hasCard
        ? storage.text
        : camera.storage?.loopRecording && storage.fraction >= 0.9
          ? strings.live.tiles.loop
          : strings.live.tiles.used(formatPercent(storage.fraction)),
      sub: [
        camera.storage?.recordingMode ? strings.storage.mode[camera.storage.recordingMode] : undefined,
        hasCard ? formatBytes(camera.storage!.totalBytes) : undefined,
      ]
        .filter(Boolean)
        .join(" · "),
      title: storage.text,
    },
    {
      icon: <Clock />,
      label: strings.live.tiles.localTime,
      value: formatHm(now, offset),
      sub: formatUtcOffset(offset),
      title: camera.timeZone?.replace(/_/g, " "),
    },
  ];
  return (
    <div className="@container">
      <div className="grid grid-cols-2 gap-3 @3xl:grid-cols-4">
      {tiles.map((t) => (
        <Card key={t.label} className="flex min-w-0 items-start gap-3 p-3.5" title={t.title}>
          <span
            className={cn(
              "grid size-9 shrink-0 place-items-center rounded-xl [&_svg]:size-[18px]",
              t.tone === "success"
                ? "bg-success-soft text-success"
                : t.tone === "danger"
                  ? "bg-danger-soft text-danger"
                  : t.tone === "warning"
                    ? "bg-warning-soft text-warning"
                    : "bg-brand-soft text-brand-text",
            )}
          >
            {t.icon}
          </span>
          <div className="min-w-0">
            <p className="text-[11.5px] font-medium text-fg-3">{t.label}</p>
            <p className="truncate text-[13.5px] font-semibold text-fg">{t.value}</p>
            <p className="truncate text-xs text-fg-2">{t.sub}</p>
          </div>
        </Card>
      ))}
      </div>
    </div>
  );
}

function TodayPanel({ camera }: { camera: Camera }) {
  const info = useStatusInfo(camera.status);
  const offset = camera.utcOffsetMinutes ?? localOffsetMinutes();
  const now = useNow(60_000);
  const today = todayIn(offset, now);
  const index = useDayIndex(camera.id, today, { isToday: true, enabled: info.viewable });
  const events = useMemo(() => [...(index.data?.events ?? [])].reverse(), [index.data]);

  return (
    <Card className="relative flex min-h-[360px] flex-col overflow-hidden">
      <div className="flex items-center justify-between px-4 pb-2 pt-4">
        <h2 className="text-[15px] font-semibold text-fg">
          {strings.live.todayTitle}
          {index.data && <span className="ml-1.5 font-normal tabular-nums text-fg-3">{events.length}</span>}
        </h2>
        <Link
          to="/cameras/$cameraId"
          params={{ cameraId: camera.id }}
          search={{ tab: "playback", date: today }}
          className="rounded-md text-xs font-semibold text-brand-text hover:underline"
        >
          {strings.live.seeAll}
        </Link>
      </div>
      <div className="absolute inset-x-0 bottom-0 top-12 overflow-y-auto px-2 pb-2">
        {!info.viewable ? (
          <p className="px-3 py-8 text-center text-[13px] text-fg-2">{info.detail}</p>
        ) : index.isPending ? (
          <div className="grid gap-1 px-1">
            {Array.from({ length: 6 }, (_, i) => (
              <div key={i} className="flex items-center gap-3 px-2 py-2">
                <Skeleton className="aspect-video w-[76px] rounded-lg" />
                <div className="grid flex-1 gap-1.5">
                  <Skeleton className="h-3.5 w-16" />
                  <Skeleton className="h-3 w-24" />
                </div>
              </div>
            ))}
          </div>
        ) : index.isError ? (
          <p className="px-3 py-8 text-center text-[13px] text-fg-2">{describeError(toApiError(index.error)).body}</p>
        ) : events.length === 0 ? (
          <p className="px-3 py-8 text-center text-[13px] text-fg-2">{strings.live.todayEmpty}</p>
        ) : (
          <ul className="grid gap-0.5">
            {events.map((e) => {
              const type = primaryEventType(e.types);
              return (
                <li key={e.id}>
                  <Link
                    to="/cameras/$cameraId"
                    params={{ cameraId: camera.id }}
                    search={{ tab: "playback", date: today, t: e.start }}
                    className="flex items-center gap-3 rounded-xl px-2 py-1.5 transition-colors hover:bg-hover"
                  >
                    <span className="relative aspect-video w-[76px] shrink-0 overflow-hidden rounded-lg bg-video">
                      {e.thumbnailUrl && <img src={e.thumbnailUrl} alt="" loading="lazy" className="size-full object-cover" />}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block text-[13px] font-semibold tabular-nums text-fg">{formatHms(e.start, offset)}</span>
                      <span className="mt-0.5 flex items-center gap-1.5 text-xs text-fg-2">
                        <EventGlyph type={type} size="xs" labelled={false} />
                        <span className="truncate">{eventLabel(type)}</span>
                        <span className="ml-auto shrink-0 tabular-nums text-fg-3">{formatClockDuration(e.end - e.start)}</span>
                      </span>
                    </span>
                  </Link>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </Card>
  );
}

/**
 * "Record" on live view: marks a stretch of time, then exports it from the SD card, which
 * records continuously. Saves re-encoding the live stream and keeps the original quality.
 */
function useLiveRecording(camera: Camera) {
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const start = useStartExport();
  const navigate = useNavigate();
  const toggle = () => {
    if (startedAt === null) {
      setStartedAt(Date.now());
      toast(strings.live.recordStarted);
      return;
    }
    const from = startedAt - 3 * SECOND;
    const to = Date.now();
    setStartedAt(null);
    start.mutate(
      { cameraId: camera.id, start: toIso(from), end: toIso(to) },
      {
        onSuccess: () =>
          toast.success(strings.live.recordSaved, {
            description: `${camera.name} · ${formatClockDuration(to - from)}`,
            action: { label: strings.clip.toastAction, onClick: () => void navigate({ to: "/downloads" }) },
          }),
        onError: (err) => toast.error(strings.clip.failed, { description: describeError(toApiError(err)).body }),
      },
    );
  };
  return { active: startedAt !== null, startedAt, toggle, saving: start.isPending };
}
