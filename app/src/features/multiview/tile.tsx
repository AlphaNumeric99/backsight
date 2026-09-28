import {
  memo,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEventHandler,
  type PointerEventHandler,
  type ReactNode,
  type Ref,
} from "react";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, GripVertical, Maximize2, Minimize2, Volume2, VolumeX } from "lucide-react";
import type { Camera, StreamQuality, StreamRequest } from "@/ipc";
import type { PlayerStats } from "@/player/types";
import { VideoSurface } from "@/player/VideoSurface";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { formatBitrate } from "@/lib/format";
import { useStatusInfo } from "@/components/camera-status";
import { SnapshotPlaceholder } from "@/components/camera-snapshot";
import { Tooltip } from "@/components/ui/tooltip";

export interface TileProps {
  camera: Camera;
  quality: StreamQuality;
  muted: boolean;
  onToggleMute: () => void;
  focused: boolean;
  onToggleFocus: () => void;
  /** Size tier for type and controls. */
  compact: boolean;
  className?: string;
  style?: CSSProperties;
  nodeRef?: Ref<HTMLDivElement>;
  handleRef?: Ref<HTMLButtonElement>;
  dragging?: boolean;
  onPointerDown?: PointerEventHandler;
  handleProps?: { onKeyDown?: KeyboardEventHandler } & Record<string, unknown>;
}

function TileButton({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <Tooltip content={label}>
      <button
        type="button"
        aria-label={label}
        onClick={(e) => {
          e.stopPropagation();
          onClick();
        }}
        onDoubleClick={(e) => e.stopPropagation()}
        className="grid size-8 place-items-center rounded-full bg-black/55 text-white backdrop-blur transition-colors hover:bg-black/75 [&_svg]:size-4"
      >
        {children}
      </button>
    </Tooltip>
  );
}

export const Tile = memo(function Tile({
  camera,
  quality,
  muted,
  onToggleMute,
  focused,
  onToggleFocus,
  compact,
  className,
  style,
  nodeRef,
  handleRef,
  dragging,
  onPointerDown,
  handleProps,
}: TileProps) {
  const info = useStatusInfo(camera.status);
  const [bitrate, setBitrate] = useState<number | null>(null);
  const last = useRef(0);
  const source = useMemo<StreamRequest | null>(
    () => (info.viewable ? { kind: "live", cameraId: camera.id, quality } : null),
    [info.viewable, camera.id, quality],
  );
  const onStats = (s: PlayerStats) => {
    const now = performance.now();
    if (now - last.current < 1000) return;
    last.current = now;
    setBitrate(s.bitrate);
  };
  const Icon = info.icon;

  return (
    <div
      ref={nodeRef}
      style={style}
      onPointerDown={onPointerDown}
      onDoubleClick={onToggleFocus}
      className={cn(
        "group/tile relative isolate overflow-hidden rounded-[6px] bg-[#07080b] text-white outline-none",
        dragging && "z-30 shadow-[0_24px_48px_-12px_rgb(0_0_0/0.8)] ring-2 ring-brand",
        className,
      )}
    >
      <VideoSurface
        source={source}
        muted={muted}
        fit="cover"
        posterUrl={camera.snapshotUrl}
        onStats={onStats}
        className="size-full"
      />
      {!camera.snapshotUrl && !source && <SnapshotPlaceholder bare />}

      {!info.viewable && (
        <div className="absolute inset-0 grid place-items-center bg-black/55 p-3 text-center backdrop-blur-[2px]">
          <div className="flex max-w-[260px] flex-col items-center">
            {Icon && (
              <span className={cn("mb-2 grid place-items-center rounded-full bg-white/10", compact ? "size-8" : "size-11")}>
                <Icon className={compact ? "size-4" : "size-5"} strokeWidth={1.8} />
              </span>
            )}
            <p className={cn("font-semibold text-shadow-video", compact ? "text-xs" : "text-sm")}>{info.label}</p>
            {!compact && info.detail && <p className="mt-1 text-xs text-white/70 text-shadow-video">{info.detail}</p>}
          </div>
        </div>
      )}

      <div className="pointer-events-none absolute inset-x-0 top-0 flex items-start justify-between gap-2 bg-gradient-to-b from-black/60 to-transparent p-2 pb-6">
        <div className="flex min-w-0 items-center gap-1.5">
          {info.viewable && (
            <span className="inline-flex h-5 shrink-0 items-center gap-1 rounded-[5px] bg-live px-1.5 text-[10px] font-bold uppercase tracking-[0.06em]">
              <span className="size-1.5 rounded-full bg-white" />
              {strings.multiview.live}
            </span>
          )}
          <span className={cn("truncate font-semibold text-shadow-video", compact ? "text-[11.5px]" : "text-[13px]")}>
            {camera.name}
          </span>
        </div>
        {bitrate !== null && (
          <span className="shrink-0 rounded-[5px] bg-black/45 px-1.5 py-0.5 font-mono text-[10.5px] tabular-nums text-white/85">
            {formatBitrate(bitrate)}
          </span>
        )}
      </div>

      {/* Hover and focus controls. */}
      <div
        className={cn(
          "absolute inset-x-0 bottom-0 flex items-end justify-between gap-2 bg-gradient-to-t from-black/60 to-transparent p-2 pt-8 transition-opacity duration-(--dur-fast)",
          "opacity-0 group-hover/tile:opacity-100 group-focus-within/tile:opacity-100",
          !muted && "opacity-100",
        )}
      >
        <button
          ref={handleRef}
          type="button"
          aria-label={strings.multiview.dragHandle(camera.name)}
          className="grid size-8 cursor-grab place-items-center rounded-full bg-black/55 text-white/85 backdrop-blur hover:bg-black/75 active:cursor-grabbing"
          {...handleProps}
        >
          <GripVertical className="size-4" />
        </button>
        <div className="flex items-center gap-1.5">
          {info.viewable && (
            <TileButton label={muted ? strings.multiview.unmute(camera.name) : strings.multiview.mute(camera.name)} onClick={onToggleMute}>
              {muted ? <VolumeX /> : <Volume2 className="text-[#8fb0ff]" />}
            </TileButton>
          )}
          <TileButton label={focused ? strings.multiview.exitFocus : strings.multiview.focus(camera.name)} onClick={onToggleFocus}>
            {focused ? <Minimize2 /> : <Maximize2 />}
          </TileButton>
          <Tooltip content={strings.multiview.openCamera}>
            <Link
              to="/cameras/$cameraId"
              params={{ cameraId: camera.id }}
              aria-label={`${strings.multiview.openCamera}: ${camera.name}`}
              onDoubleClick={(e) => e.stopPropagation()}
              className="grid size-8 place-items-center rounded-full bg-black/55 text-white backdrop-blur hover:bg-black/75"
            >
              <ArrowUpRight className="size-4" />
            </Link>
          </Tooltip>
        </div>
      </div>
    </div>
  );
});
