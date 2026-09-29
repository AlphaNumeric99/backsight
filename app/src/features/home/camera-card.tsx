import { Link } from "@tanstack/react-router";
import { motion } from "motion/react";
import { History, Play, Star } from "lucide-react";
import type { Camera, CameraGroup } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { listItemVariants } from "@/lib/motion";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/misc";
import { Tooltip } from "@/components/ui/tooltip";
import { CameraSnapshot } from "@/components/camera-snapshot";
import { StatusPill, useStatusInfo } from "@/components/camera-status";
import { StorageUsage } from "@/components/storage-usage";
import { useUpdateCamera } from "@/queries/cameras";

export function CameraCard({
  camera,
  groups,
  capturing = false,
}: {
  camera: Camera;
  groups: CameraGroup[];
  /** A preview is being grabbed for this camera right now. */
  capturing?: boolean;
}) {
  const info = useStatusInfo(camera.status);
  const update = useUpdateCamera();
  const groupNames = camera.groupIds
    .map((id) => groups.find((g) => g.id === id)?.name)
    .filter(Boolean)
    .join(", ");
  const StateIcon = info.icon;

  return (
    <motion.article
      layout
      variants={listItemVariants}
      initial="initial"
      animate="animate"
      exit="exit"
      className={cn(
        "group relative flex flex-col overflow-hidden rounded-card border border-card-border bg-surface shadow-card",
        "transition-shadow duration-(--dur-base) hover:shadow-raised",
        "has-[[data-card-link]:focus-visible]:ring-2 has-[[data-card-link]:focus-visible]:ring-ring has-[[data-card-link]:focus-visible]:ring-offset-2 has-[[data-card-link]:focus-visible]:ring-offset-background",
      )}
    >
      <div className="relative aspect-video overflow-hidden bg-video">
        <CameraSnapshot
          camera={camera}
          placeholderLabel={capturing ? strings.home.gettingPreview : strings.home.noPreview}
          bare={!info.viewable && Boolean(StateIcon)}
          imgClassName={cn(
            "transition-[transform,opacity,filter] duration-700 ease-out-expo group-hover:scale-[1.035]",
            !info.viewable && "opacity-50 saturate-50",
          )}
        />
        <div className="pointer-events-none absolute inset-0 bg-gradient-to-b from-black/40 via-black/0 to-black/50" />

        {!info.viewable && StateIcon && (
          <div className="pointer-events-none absolute inset-0 grid place-items-center">
            <span className="video-glass grid size-11 place-items-center rounded-full">
              <StateIcon className="size-5" strokeWidth={1.8} />
            </span>
          </div>
        )}

        <div className="pointer-events-none absolute left-3 right-14 top-3">
          <StatusPill status={camera.status} variant="overlay" />
        </div>

        <Tooltip content={camera.favorite ? strings.home.unfavorite : strings.home.favorite}>
          <button
            type="button"
            aria-pressed={camera.favorite}
            aria-label={camera.favorite ? strings.home.unfavorite : strings.home.favorite}
            onClick={() => update.mutate({ id: camera.id, req: { favorite: !camera.favorite } })}
            className="video-glass absolute right-2.5 top-2.5 z-10 grid size-8 place-items-center rounded-full transition-transform duration-(--dur-fast) hover:scale-105 active:scale-90"
          >
            <motion.span
              key={String(camera.favorite)}
              initial={{ scale: camera.favorite ? 0.6 : 1 }}
              animate={{ scale: 1 }}
              transition={{ type: "spring", stiffness: 600, damping: 18 }}
              className="grid place-items-center"
            >
              <Star
                className={cn("size-4", camera.favorite ? "fill-[#ffc53d] text-[#ffc53d]" : "text-white")}
                strokeWidth={2}
              />
            </motion.span>
          </button>
        </Tooltip>

        {info.viewable && (
          <div
            className={cn(
              "absolute inset-x-3 bottom-3 z-10 flex gap-2 transition-[opacity,transform] duration-(--dur-base) ease-standard",
              "translate-y-1.5 opacity-0 group-focus-within:translate-y-0 group-focus-within:opacity-100 group-hover:translate-y-0 group-hover:opacity-100",
            )}
          >
            <Button asChild size="sm" variant="overlay">
              <Link to="/cameras/$cameraId" params={{ cameraId: camera.id }} search={{ tab: "live" }}>
                <Play className="fill-current" />
                {strings.home.live}
              </Link>
            </Button>
            <Button asChild size="sm" variant="overlay">
              <Link to="/cameras/$cameraId" params={{ cameraId: camera.id }} search={{ tab: "playback" }}>
                <History />
                {strings.home.playback}
              </Link>
            </Button>
          </div>
        )}
      </div>

      <div className="flex flex-1 flex-col gap-3.5 p-4">
        <div className="min-w-0">
          <div className="flex items-center justify-between gap-2">
            <h3 className="min-w-0 truncate text-[15px] font-semibold tracking-[-0.01em] text-fg">
              <Link
                to="/cameras/$cameraId"
                params={{ cameraId: camera.id }}
                data-card-link=""
                aria-label={strings.home.open(camera.name)}
                className="outline-none after:absolute after:inset-0 after:content-['']"
              >
                {camera.name}
              </Link>
            </h3>
            {camera.model && (
              <span className="shrink-0 rounded-md bg-surface-3 px-1.5 py-[3px] font-mono text-[10.5px] font-medium leading-none text-fg-2">
                {camera.model}
              </span>
            )}
          </div>
          <p className="mt-1 truncate text-xs text-fg-3">
            {camera.host}
            {groupNames && ` · ${groupNames}`}
          </p>
        </div>
        <StorageUsage storage={camera.storage} className="mt-auto" />
      </div>
    </motion.article>
  );
}

export function CameraCardSkeleton() {
  return (
    <div className="overflow-hidden rounded-card border border-card-border bg-surface shadow-card">
      <Skeleton className="aspect-video rounded-none" />
      <div className="grid gap-3 p-4">
        <Skeleton className="h-4 w-2/5" />
        <Skeleton className="h-3 w-3/5" />
        <Skeleton className="mt-1 h-1 w-full rounded-full" />
      </div>
    </div>
  );
}
