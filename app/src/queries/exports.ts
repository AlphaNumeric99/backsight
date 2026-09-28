import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type ExportJob, type ExportRequest } from "@/ipc";
import { queryKeys } from "./keys";

export const ACTIVE_EXPORT_STATES: ReadonlySet<ExportJob["state"]> = new Set(["queued", "running", "paused"]);

export function upsertJob(list: ExportJob[] | undefined, job: ExportJob): ExportJob[] | undefined {
  if (!list) return list;
  const i = list.findIndex((j) => j.id === job.id);
  if (i === -1) return [job, ...list];
  const next = list.slice();
  next[i] = job;
  return next;
}

export function useExports() {
  return useQuery({ queryKey: queryKeys.exports(), queryFn: () => api.listExports() });
}

export function useActiveExportCount(): number {
  const { data } = useQuery({
    queryKey: queryKeys.exports(),
    queryFn: () => api.listExports(),
    select: (jobs) => jobs.filter((j) => ACTIVE_EXPORT_STATES.has(j.state)).length,
  });
  return data ?? 0;
}

export function useStartExport() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (req: ExportRequest) => api.startExport(req),
    onSuccess: (job) => {
      qc.setQueryData<ExportJob[]>(queryKeys.exports(), (list) => {
        // A progress event may already have delivered a newer copy of this job.
        if (list?.some((j) => j.id === job.id)) return list;
        return upsertJob(list, job);
      });
    },
  });
}

export function useCancelExport() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.cancelExport(id),
    onSettled: () => qc.invalidateQueries({ queryKey: queryKeys.exports() }),
  });
}

export function useRevealExport() {
  return useMutation({ mutationFn: (id: string) => api.revealExport(id) });
}
