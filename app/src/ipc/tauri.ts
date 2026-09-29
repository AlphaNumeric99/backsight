import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppEvent,
  BacksightApi,
  BatchListener,
  CameraId,
  StreamHandle,
  StreamRequest,
} from "./api";

/** Must match `EVENT_NAME` in `app/src-tauri/src/cameras.rs`. */
const EVENT_NAME = "backsight://event";

/** `BacksightApi` backed by the Rust commands in `app/src-tauri/src/commands.rs`. */
export function createTauriApi(): BacksightApi {
  return {
    listCameras: () => invoke("list_cameras"),
    getCamera: (id) => invoke("get_camera", { id }),
    discover: (timeoutMs) => invoke("discover", { timeoutMs }),
    addCamera: (req) => invoke("add_camera", { req }),
    updateCamera: (id, req) => invoke("update_camera", { id, req }),
    removeCamera: (id) => invoke("remove_camera", { id }),

    listGroups: () => invoke("list_groups"),
    saveGroups: (groups) => invoke("save_groups", { groups }),

    getDaysWithRecordings: (cameraId, month) =>
      invoke("get_days_with_recordings", { cameraId, month }),
    getDayIndex: (cameraId, date) => invoke("get_day_index", { cameraId, date }),

    openStream: async (req: StreamRequest, onBatch: BatchListener): Promise<StreamHandle> => {
      const channel = new Channel<ArrayBuffer>();
      channel.onmessage = (batch) => onBatch(batch);
      const id = await invoke<string>("open_stream", { req, channel });
      return {
        id,
        close: () => invoke("close_stream", { id }),
      };
    },

    startExport: (req) => invoke("start_export", { req }),
    listExports: () => invoke("list_exports"),
    cancelExport: (id) => invoke("cancel_export", { id }),
    revealExport: (id) => invoke("reveal_export", { id }),

    saveSnapshot: async (cameraId: CameraId, png: Blob) =>
      invoke<string>("save_snapshot", new Uint8Array(await png.arrayBuffer()), {
        headers: { "x-camera-id": cameraId },
      }),
    savePreview: async (cameraId: CameraId, jpeg: Blob) =>
      invoke<void>("save_preview", new Uint8Array(await jpeg.arrayBuffer()), {
        headers: { "x-camera-id": cameraId },
      }),

    getSettings: () => invoke("get_settings"),
    updateSettings: (patch) => invoke("update_settings", { patch }),

    subscribe: (listener: (event: AppEvent) => void) => {
      let unlisten: (() => void) | undefined;
      let cancelled = false;
      void listen<AppEvent>(EVENT_NAME, (event) => listener(event.payload)).then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
      return () => {
        cancelled = true;
        unlisten?.();
      };
    },
  };
}
