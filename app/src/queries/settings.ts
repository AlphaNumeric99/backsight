import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api, type Settings } from "@/ipc";
import { describeError, toApiError } from "@/lib/errors";
import { strings } from "@/lib/strings";
import { applyTheme } from "@/lib/theme";
import { queryKeys } from "./keys";

export function useSettings() {
  return useQuery({ queryKey: queryKeys.settings(), queryFn: () => api.getSettings(), staleTime: Infinity });
}

/** Saves a settings patch optimistically; the theme applies instantly. */
export function useUpdateSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: Partial<Settings>) => api.updateSettings(patch),
    onMutate: async (patch) => {
      await qc.cancelQueries({ queryKey: queryKeys.settings() });
      const previous = qc.getQueryData<Settings>(queryKeys.settings());
      if (previous) qc.setQueryData<Settings>(queryKeys.settings(), { ...previous, ...patch });
      if (patch.theme) applyTheme(patch.theme);
      return { previous };
    },
    onError: (error, _patch, context) => {
      if (context?.previous) {
        qc.setQueryData(queryKeys.settings(), context.previous);
        applyTheme(context.previous.theme);
      }
      const copy = describeError(toApiError(error));
      toast.error(strings.settings.saveFailed, { description: copy.body });
    },
    onSuccess: (settings) => {
      qc.setQueryData(queryKeys.settings(), settings);
    },
  });
}
