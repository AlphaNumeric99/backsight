import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api, type AddCameraRequest, type AddQuboCameraRequest, type Camera, type CameraGroup, type CameraId, type UpdateCameraRequest } from "@/ipc";
import { describeError, toApiError } from "@/lib/errors";
import { queryKeys } from "./keys";

export function useCameras() {
  return useQuery({ queryKey: queryKeys.cameras(), queryFn: () => api.listCameras() });
}

export function useCamera(id: CameraId) {
  const qc = useQueryClient();
  return useQuery({
    queryKey: queryKeys.camera(id),
    queryFn: () => api.getCamera(id),
    // Render straight away from the list when we have it.
    placeholderData: () => qc.getQueryData<Camera[]>(queryKeys.cameras())?.find((c) => c.id === id),
  });
}

export function useGroups() {
  return useQuery({ queryKey: queryKeys.groups(), queryFn: () => api.listGroups(), staleTime: 5 * 60_000 });
}

function patchCamera(camera: Camera, req: UpdateCameraRequest): Camera {
  return {
    ...camera,
    ...(req.name !== undefined && { name: req.name }),
    ...(req.groupIds !== undefined && { groupIds: req.groupIds }),
    ...(req.favorite !== undefined && { favorite: req.favorite }),
    ...(req.cameraAccount !== undefined && { hasCameraAccount: req.cameraAccount !== null }),
  };
}

/** Updates a camera, optimistically for name, groups and favourite. */
export function useUpdateCamera(options: { silent?: boolean } = {}) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, req }: { id: CameraId; req: UpdateCameraRequest }) => api.updateCamera(id, req),
    onMutate: async ({ id, req }) => {
      await qc.cancelQueries({ queryKey: queryKeys.cameras() });
      const list = qc.getQueryData<Camera[]>(queryKeys.cameras());
      const one = qc.getQueryData<Camera>(queryKeys.camera(id));
      if (list) qc.setQueryData<Camera[]>(queryKeys.cameras(), list.map((c) => (c.id === id ? patchCamera(c, req) : c)));
      if (one) qc.setQueryData<Camera>(queryKeys.camera(id), patchCamera(one, req));
      return { list, one };
    },
    onError: (error, { id }, context) => {
      if (context?.list) qc.setQueryData(queryKeys.cameras(), context.list);
      if (context?.one) qc.setQueryData(queryKeys.camera(id), context.one);
      if (!options.silent) {
        const copy = describeError(toApiError(error));
        toast.error(copy.title, { description: copy.body });
      }
    },
    onSuccess: (camera) => {
      qc.setQueryData<Camera>(queryKeys.camera(camera.id), camera);
      qc.setQueryData<Camera[]>(queryKeys.cameras(), (list) => list?.map((c) => (c.id === camera.id ? camera : c)));
    },
  });
}

export function useAddCamera() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (req: AddCameraRequest) => api.addCamera(req),
    onSuccess: (camera) => {
      qc.setQueryData<Camera[]>(queryKeys.cameras(), (list) => (list ? [...list.filter((c) => c.id !== camera.id), camera] : list));
      qc.setQueryData<Camera>(queryKeys.camera(camera.id), camera);
    },
  });
}

export function useRemoveCamera() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: CameraId) => api.removeCamera(id),
    onSuccess: (_, id) => {
      qc.setQueryData<Camera[]>(queryKeys.cameras(), (list) => list?.filter((c) => c.id !== id));
      qc.removeQueries({ queryKey: queryKeys.camera(id), exact: true });
    },
  });
}

export function useAddQuboCamera() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (req: AddQuboCameraRequest) => api.addQuboCamera(req),
    onSuccess: (camera) => {
      qc.setQueryData<Camera[]>(queryKeys.cameras(), (list) => list ? [...list.filter((c) => c.id !== camera.id), camera] : list);
      qc.setQueryData<Camera>(queryKeys.camera(camera.id), camera);
    },
  });
}

export function useSaveGroups() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (groups: CameraGroup[]) => api.saveGroups(groups),
    onMutate: async (groups) => {
      await qc.cancelQueries({ queryKey: queryKeys.groups() });
      const previous = qc.getQueryData<CameraGroup[]>(queryKeys.groups());
      qc.setQueryData(queryKeys.groups(), groups);
      return { previous };
    },
    onError: (error, _groups, context) => {
      if (context?.previous) qc.setQueryData(queryKeys.groups(), context.previous);
      const copy = describeError(toApiError(error));
      toast.error(copy.title, { description: copy.body });
    },
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.groups() });
      void qc.invalidateQueries({ queryKey: queryKeys.cameras() });
    },
  });
}
