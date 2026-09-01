import type { BacksightApi } from "../api";
import { createMockData } from "./data";
import { openMockStream } from "./stream";

/** A `BacksightApi` backed by in-memory fixtures, for browser development and tests. */
export function createMockApi(): BacksightApi {
  return {
    ...createMockData(),
    openStream: openMockStream,
  };
}
