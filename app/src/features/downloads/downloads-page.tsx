import { useEffect, useMemo } from "react";
import { Link } from "@tanstack/react-router";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import { toast } from "sonner";
import { CircleAlert, CircleCheck, Clock, Download, FolderOpen, LoaderCircle, Pause, RotateCw, X } from "lucide-react";
import type { Camera, ExportJob, ExportState } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { describeError, toApiError } from "@/lib/errors";
import { formatBytes, formatEta, formatPercent, formatRelative } from "@/lib/format";
import { formatClockDuration, formatHms, formatLocalDateLabel, localDateOf, localOffsetMinutes } from "@/lib/time";
import { Button } from "@/components/ui/button";
import { Badge, Card, Progress, Skeleton } from "@/components/ui/misc";
import { EmptyState } from "@/components/ui/empty-state";
import { Tooltip } from "@/components/ui/tooltip";
import { useCameras } from "@/queries/cameras";
import { useSettings } from "@/queries/settings";
import { ACTIVE_EXPORT_STATES, useCancelExport, useExports, useRevealExport, useStartExport } from "@/queries/exports";

const STATE_TONE: Record<ExportState, "neutral" | "brand" | "warning" | "success" | "danger"> = {
  queued: "neutral",
  running: "brand",
  paused: "warning",
  done: "success",
  failed: "danger",
  cancelled: "neutral",
};

export function DownloadsPage() {
  const exports = useExports();
  const cameras = useCameras();
  const settings = useSettings();

  useEffect(() => {
    document.title = `${strings.downloads.title} — ${strings.app.name}`;
  }, []);

  const jobs = useMemo(
    () => [...(exports.data ?? [])].sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt)),
    [exports.data],
  );
  const active = jobs.filter((j) => ACTIVE_EXPORT_STATES.has(j.state));
  const finished = jobs.filter((j) => !ACTIVE_EXPORT_STATES.has(j.state));
  const cameraById = useMemo(() => new Map((cameras.data ?? []).map((c) => [c.id, c])), [cameras.data]);

  return (
    <div className="mx-auto w-full max-w-[1080px] flex-1 px-8 pb-14 pt-9 max-[1099px]:px-6">
      <header className="mb-8 flex flex-wrap items-end justify-between gap-4">
        <div className="min-w-0">
          <h1 className="text-[30px] font-semibold leading-[1.15] tracking-[-0.03em] text-fg">{strings.downloads.title}</h1>
          {settings.data && (
            <p className="mt-1.5 flex min-w-0 items-center gap-1.5 text-sm text-fg-2">
              <FolderOpen className="size-4 shrink-0 text-fg-3" />
              <span className="shrink-0">{strings.downloads.savedTo}</span>
              <code className="truncate rounded-md bg-surface-3 px-1.5 py-0.5 font-mono text-[12px] text-fg">
                {settings.data.exportDir}
              </code>
            </p>
          )}
        </div>
        <Button asChild variant="outline" size="sm">
          <Link to="/settings" search={{ section: "recordings" }}>
            {strings.downloads.changeFolder}
          </Link>
        </Button>
      </header>

      {exports.isPending ? (
        <div className="grid gap-3">
          {Array.from({ length: 3 }, (_, i) => (
            <Card key={i} className="flex items-center gap-4 p-4">
              <Skeleton className="size-11 rounded-xl" />
              <div className="grid flex-1 gap-2">
                <Skeleton className="h-4 w-1/3" />
                <Skeleton className="h-3 w-1/2" />
              </div>
            </Card>
          ))}
        </div>
      ) : jobs.length === 0 ? (
        <Card className="px-8">
          <EmptyState
            art={<EmptyDownloadsArt />}
            title={strings.downloads.emptyTitle}
            body={strings.downloads.emptyBody}
            actions={
              <Button asChild variant="primary">
                <Link to="/">{strings.downloads.goToCameras}</Link>
              </Button>
            }
          />
        </Card>
      ) : (
        <LayoutGroup>
          <Section title={strings.downloads.inProgress} count={active.length} hidden={active.length === 0}>
            {active.map((job) => (
              <JobRow key={job.id} job={job} camera={cameraById.get(job.cameraId)} />
            ))}
          </Section>
          <Section title={strings.downloads.finished} count={finished.length} hidden={finished.length === 0}>
            {finished.map((job) => (
              <JobRow key={job.id} job={job} camera={cameraById.get(job.cameraId)} />
            ))}
          </Section>
        </LayoutGroup>
      )}
    </div>
  );
}

function Section({
  title,
  count,
  hidden,
  children,
}: {
  title: string;
  count: number;
  hidden: boolean;
  children: React.ReactNode;
}) {
  return (
    <AnimatePresence initial={false}>
      {!hidden && (
        <motion.section
          layout
          key={title}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1, transition: transitions.base }}
          exit={{ opacity: 0, transition: transitions.fast }}
          className="mb-8"
        >
          <motion.h2 layout="position" className="mb-3 flex items-center gap-2 text-[13px] font-semibold uppercase tracking-wider text-fg-3">
            {title}
            <span className="tabular-nums">{count}</span>
          </motion.h2>
          <ul className="grid gap-3">
            <AnimatePresence initial={false} mode="popLayout">
              {children}
            </AnimatePresence>
          </ul>
        </motion.section>
      )}
    </AnimatePresence>
  );
}

function StateIcon({ state }: { state: ExportState }) {
  switch (state) {
    case "queued":
      return <Clock />;
    case "running":
      return <LoaderCircle className="animate-spin" />;
    case "paused":
      return <Pause />;
    case "done":
      return <CircleCheck />;
    case "failed":
      return <CircleAlert />;
    case "cancelled":
      return <X />;
  }
}

function JobRow({ job, camera }: { job: ExportJob; camera?: Camera }) {
  const cancel = useCancelExport();
  const reveal = useRevealExport();
  const restart = useStartExport();
  const offset = camera?.utcOffsetMinutes ?? localOffsetMinutes();
  const start = Date.parse(job.start);
  const end = Date.parse(job.end);
  const isActive = ACTIVE_EXPORT_STATES.has(job.state);
  const tone = STATE_TONE[job.state];
  const date = formatLocalDateLabel(localDateOf(start, offset), { weekday: "short", day: "numeric", month: "short" });
  const fileName = job.outputPath?.split(/[\\/]/).pop();

  const onReveal = () =>
    reveal.mutate(job.id, {
      onError: (err) => toast.error(strings.downloads.revealFailed, { description: describeError(toApiError(err)).body }),
    });
  const onRetry = () =>
    restart.mutate(
      { cameraId: job.cameraId, start: job.start, end: job.end },
      { onError: (err) => toast.error(strings.clip.failed, { description: describeError(toApiError(err)).body }) },
    );
  const onCancel = () =>
    cancel.mutate(job.id, {
      onSuccess: () => toast(strings.downloads.cancelledToast),
      onError: (err) => toast.error(describeError(toApiError(err)).title),
    });

  return (
    <motion.li
      layout
      layoutId={job.id}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0, transition: transitions.slow }}
      exit={{ opacity: 0, scale: 0.98, transition: transitions.fast }}
      transition={transitions.softSpring}
    >
      <Card className="flex items-center gap-4 p-4">
        <span
          className={cn(
            "grid size-11 shrink-0 place-items-center rounded-xl [&_svg]:size-5",
            tone === "success" && "bg-success-soft text-success",
            tone === "danger" && "bg-danger-soft text-danger",
            tone === "warning" && "bg-warning-soft text-warning",
            tone === "brand" && "bg-brand-soft text-brand-text",
            tone === "neutral" && "bg-surface-3 text-fg-2",
          )}
        >
          <StateIcon state={job.state} />
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <p className="truncate text-[14.5px] font-semibold text-fg">{job.cameraName}</p>
            <Badge tone={tone} size="sm">
              {strings.downloads.state[job.state]}
            </Badge>
          </div>
          <p className="mt-0.5 truncate text-[13px] tabular-nums text-fg-2">
            {date} · {formatHms(start, offset)} – {formatHms(end, offset)}
            <span className="text-fg-3"> · {formatClockDuration(end - start)}</span>
          </p>

          {isActive ? (
            <div className="mt-2.5 flex items-center gap-3">
              <Progress
                value={job.progress}
                indeterminate={job.state === "queued"}
                tone={job.state === "paused" ? "neutral" : "brand"}
                label={`${job.cameraName} ${strings.downloads.state[job.state]}`}
                className="max-w-md"
              />
              <span className="shrink-0 text-xs tabular-nums text-fg-2">
                {job.state === "paused"
                  ? strings.downloads.pausedHint
                  : job.state === "queued"
                    ? strings.downloads.state.queued
                    : [
                        strings.downloads.progress(formatPercent(job.progress), formatBytes(job.bytesWritten)),
                        job.etaSeconds !== undefined ? strings.downloads.eta(formatEta(job.etaSeconds)) : null,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
              </span>
            </div>
          ) : job.state === "failed" ? (
            <p className="mt-1 truncate text-[13px] text-danger">{job.error ?? strings.downloads.failedToast}</p>
          ) : job.state === "done" ? (
            <p className="mt-1 flex min-w-0 items-center gap-1.5 text-xs text-fg-3">
              <span className="tabular-nums">{formatBytes(job.bytesWritten)}</span>
              {fileName && (
                <Tooltip content={job.outputPath}>
                  <span className="truncate font-mono">{fileName}</span>
                </Tooltip>
              )}
              <span>· {formatRelative(Date.parse(job.createdAt))}</span>
            </p>
          ) : (
            <p className="mt-1 text-xs text-fg-3">{formatRelative(Date.parse(job.createdAt))}</p>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {isActive && (
            <Button variant="ghost" size="sm" onClick={onCancel} loading={cancel.isPending}>
              {strings.downloads.cancel}
            </Button>
          )}
          {job.state === "done" && (
            <Button variant="outline" size="sm" onClick={onReveal} loading={reveal.isPending}>
              <FolderOpen />
              {strings.downloads.reveal}
            </Button>
          )}
          {(job.state === "failed" || job.state === "cancelled") && (
            <Button variant="outline" size="sm" onClick={onRetry} loading={restart.isPending}>
              <RotateCw />
              {strings.downloads.retry}
            </Button>
          )}
        </div>
      </Card>
    </motion.li>
  );
}

function EmptyDownloadsArt() {
  return (
    <div className="relative grid size-20 place-items-center">
      <span className="absolute inset-0 rounded-[26px] bg-brand-soft" />
      <span className="absolute inset-2 rounded-[20px] border border-dashed border-brand/35" />
      <Download className="relative size-8 text-brand-text" strokeWidth={1.8} />
    </div>
  );
}
