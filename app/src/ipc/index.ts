import { isTauri } from "@tauri-apps/api/core";
import type { BacksightApi } from "./api";
import { createMockApi } from "./mock";
import { createTauriApi } from "./tauri";

/**
 * The real backend inside the desktop app; in a plain browser (`npm run dev`) or with
 * `VITE_BACKEND=mock`, in-memory fake cameras.
 */
export const api: BacksightApi =
  isTauri() && import.meta.env.VITE_BACKEND !== "mock" ? createTauriApi() : createMockApi();

export type * from "./api";
