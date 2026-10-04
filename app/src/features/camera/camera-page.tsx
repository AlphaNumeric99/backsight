import { useCallback, useEffect } from "react";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { motion } from "motion/react";
import { ArrowLeft, EllipsisVertical, History, Radio, Settings, Star, VideoOff } from "lucide-react";
import type { Camera, LocalDate } from "@/ipc";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { toApiError } from "@/lib/errors";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/ui/empty-state";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { StatusPill } from "@/components/camera-status";
import { useCamera, useUpdateCamera } from "@/queries/cameras";
import { PlaybackTab } from "@/features/playback/playback-tab";
import { LiveTab } from "./live-tab";

type Tab = "live" | "playback";

export function CameraPage() {
  const { cameraId } = useParams({ from: "/cameras/$cameraId" });
  const search = useSearch({ from: "/cameras/$cameraId" });
  const navigate = useNavigate({ from: "/cameras/$cameraId" });
  const camera = useCamera(cameraId);
  const tab: Tab = camera.data?.brand === "qubo" ? "live" : search.tab ?? "live";

  useEffect(() => {
    if (camera.data) document.title = `${camera.data.name} — ${strings.app.name}`;
  }, [camera.data]);

  const setTab = useCallback(
    (next: string) => void navigate({ search: (s) => ({ ...s, tab: next as Tab, t: undefined }), replace: true }),
    [navigate],
  );
  const setDate = useCallback(
    (date: LocalDate) => void navigate({ search: (s) => ({ ...s, date, t: undefined }), replace: true }),
    [navigate],
  );
  const consumeTime = useCallback(
    () => void navigate({ search: (s) => ({ ...s, t: undefined }), replace: true }),
    [navigate],
  );

  if (camera.isError && !camera.data) {
    const notFound = toApiError(camera.error).code === "not_found";
    return (
      <div className="grid flex-1 place-items-center p-8">
        <EmptyState
          art={
            <div className="grid size-16 place-items-center rounded-2xl bg-surface-3 text-fg-2">
              <VideoOff className="size-7" strokeWidth={1.6} />
            </div>
          }
          title={notFound ? strings.camera.notFoundTitle : strings.errors.internal.title}
          body={notFound ? strings.camera.notFoundBody : toApiError(camera.error).message}
          actions={
            <Button asChild variant="primary">
              <Link to="/">{strings.camera.backHome}</Link>
            </Button>
          }
        />
      </div>
    );
  }

  const cam = camera.data;
  return (
    <Tabs value={tab} onValueChange={setTab} className="flex flex-1 flex-col px-8 pb-8 pt-6 max-[1099px]:px-6">
      <header className="mb-5 flex flex-wrap items-center gap-x-4 gap-y-3">
        <IconButton label={strings.camera.back} variant="outline" asChild tooltipSide="bottom">
          <Link to="/">
            <ArrowLeft />
          </Link>
        </IconButton>
        {cam ? (
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-3">
              <h1 className="truncate text-[24px] font-semibold leading-tight tracking-[-0.025em] text-fg">{cam.name}</h1>
              <StatusPill status={cam.status} />
            </div>
            <p className="mt-0.5 truncate text-[13px] text-fg-3">
              {[cam.model, cam.brand === "qubo" ? strings.qubo.cloud : cam.host, cam.videoCodec?.toUpperCase().replace("H", "H.")].filter(Boolean).join(" · ")}
            </p>
          </div>
        ) : (
          <div className="grid flex-1 gap-2">
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-3.5 w-64" />
          </div>
        )}
        <TabsList aria-label={strings.camera.tabsLabel}>
          <TabsTrigger value="live">
            <Radio className="size-4" />
            {strings.camera.tabs.live}
          </TabsTrigger>
          <TabsTrigger value="playback" disabled={cam?.brand === "qubo"} title={cam?.brand === "qubo" ? strings.qubo.playbackUnavailable : undefined}>
            <History className="size-4" />
            {strings.camera.tabs.playback}
          </TabsTrigger>
        </TabsList>
        {cam && <CameraMenu camera={cam} />}
      </header>

      {cam ? (
        <>
          <TabsContent value="live" className="flex-1">
            <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0, transition: transitions.base }}>
              <LiveTab camera={cam} />
            </motion.div>
          </TabsContent>
          {cam.brand !== "qubo" && <TabsContent value="playback" className="flex-1">
            <motion.div initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0, transition: transitions.base }}>
              <PlaybackTab
                key={cam.id}
                camera={cam}
                date={search.date}
                initialTime={search.t}
                onDateChange={setDate}
                onInitialTimeConsumed={consumeTime}
                onGoLive={() => setTab("live")}
              />
            </motion.div>
          </TabsContent>}
        </>
      ) : (
        <div className="grid gap-5 [grid-template-columns:minmax(0,1fr)_320px]">
          <Skeleton className="aspect-video w-full rounded-card" />
          <Skeleton className="h-full min-h-[320px] rounded-card" />
        </div>
      )}
    </Tabs>
  );
}

function CameraMenu({ camera }: { camera: Camera }) {
  const update = useUpdateCamera();
  const navigate = useNavigate();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <span>
          <IconButton label={strings.camera.menu} variant="ghost" tooltipSide="bottom">
            <EllipsisVertical />
          </IconButton>
        </span>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => update.mutate({ id: camera.id, req: { favorite: !camera.favorite } })}>
          <Star className={camera.favorite ? "fill-[#ffc53d] !text-[#ffc53d]" : undefined} />
          {camera.favorite ? strings.home.unfavorite : strings.home.favorite}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => void navigate({ to: "/settings", search: { section: "cameras" } })}>
          <Settings />
          {strings.camera.settings}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
