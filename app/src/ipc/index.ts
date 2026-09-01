import type { BacksightApi } from "./api";
import { createMockApi } from "./mock";

// TODO(M3): switch to the Tauri implementation when running inside the desktop app.
export const api: BacksightApi = createMockApi();

export type * from "./api";
