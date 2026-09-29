import { useEffect, useRef } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { api, type Camera, type ExportJob } from "@/ipc";
import { strings } from "@/lib/strings";
import { formatHm, localOffsetMinutes } from "@/lib/time";
import { queryKeys } from "./keys";
import { upsertJob } from "./exports";

/**
 * Keeps the query cache in sync with backend events: camera status and SD card changes,
 * camera list changes, new previews and export progress. Mounted once, inside the router.
 */
export function ApiEventBridge() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const navigateRef = useRef(navigate);
  useEffect(() => {
    navigateRef.current = navigate;
  }, [navigate]);

  useEffect(() => {
    return api.subscribe((event) => {
      switch (event.type) {
        case "camera-status": {
          const apply = (c: Camera) => (c.id === event.cameraId ? { ...c, status: event.status } : c);
          qc.setQueryData<Camera[]>(queryKeys.cameras(), (list) => list?.map(apply));
          qc.setQueryData<Camera>(queryKeys.camera(event.cameraId), (c) => (c ? apply(c) : c));
          break;
        }
        case "cameras-changed":
          void qc.invalidateQueries({ queryKey: queryKeys.cameras() });
          void qc.invalidateQueries({ queryKey: queryKeys.groups() });
          break;
        case "camera-info": {
          const { storage, utcOffsetMinutes } = event;
          const apply = (c: Camera) => (c.id === event.cameraId ? { ...c, storage, utcOffsetMinutes } : c);
          qc.setQueryData<Camera[]>(queryKeys.cameras(), (list) => list?.map(apply));
          qc.setQueryData<Camera>(queryKeys.camera(event.cameraId), (c) => (c ? apply(c) : c));
          break;
        }
        case "camera-preview": {
          const { snapshotUrl, snapshotAt } = event;
          const apply = (c: Camera) => (c.id === event.cameraId ? { ...c, snapshotUrl, snapshotAt } : c);
          qc.setQueryData<Camera[]>(queryKeys.cameras(), (list) => list?.map(apply));
          qc.setQueryData<Camera>(queryKeys.camera(event.cameraId), (c) => (c ? apply(c) : c));
          break;
        }
        case "export-progress": {
          const job = event.job;
          const list = qc.getQueryData<ExportJob[]>(queryKeys.exports());
          if (!list) {
            void qc.invalidateQueries({ queryKey: queryKeys.exports() });
            break;
          }
          const before = list.find((j) => j.id === job.id);
          qc.setQueryData<ExportJob[]>(queryKeys.exports(), (l) => upsertJob(l, job));
          if (before && before.state !== job.state) announce(job, qc, (to) => void navigateRef.current({ to }));
          break;
        }
      }
    });
  }, [qc]);

  return null;
}

function announce(job: ExportJob, qc: ReturnType<typeof useQueryClient>, go: (to: "/downloads") => void) {
  const camera = qc.getQueryData<Camera[]>(queryKeys.cameras())?.find((c) => c.id === job.cameraId);
  const offset = camera?.utcOffsetMinutes ?? localOffsetMinutes();
  const range = `${job.cameraName} · ${formatHm(Date.parse(job.start), offset)}–${formatHm(Date.parse(job.end), offset)}`;
  if (job.state === "done") {
    toast.success(strings.downloads.completedToast, {
      description: range,
      action: { label: strings.downloads.reveal, onClick: () => void api.revealExport(job.id) },
    });
  } else if (job.state === "failed") {
    toast.error(strings.downloads.failedToast, {
      description: job.error ?? range,
      action: { label: strings.clip.toastAction, onClick: () => go("/downloads") },
    });
  }
}
