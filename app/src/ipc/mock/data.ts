import type { BacksightApi } from "../api";

/** Placeholder: fake cameras, recordings, events, exports and settings. */
export function createMockData(): Omit<BacksightApi, "openStream"> {
  const notReady = () => Promise.reject({ code: "internal", message: "mock not implemented" });
  return {
    listCameras: async () => [],
    getCamera: notReady,
    discover: async () => [],
    addCamera: notReady,
    updateCamera: notReady,
    removeCamera: notReady,
    listGroups: async () => [],
    saveGroups: async () => {},
    getDaysWithRecordings: async () => [],
    getDayIndex: notReady,
    startExport: notReady,
    listExports: async () => [],
    cancelExport: notReady,
    revealExport: notReady,
    saveSnapshot: notReady,
    getSettings: notReady,
    updateSettings: notReady,
    subscribe: () => () => {},
  };
}
