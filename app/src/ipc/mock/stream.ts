import type { BatchListener, StreamHandle, StreamRequest } from "../api";

/** Placeholder: streams fixture video in the wire format. */
export async function openMockStream(
  _req: StreamRequest,
  _onBatch: BatchListener,
): Promise<StreamHandle> {
  return { id: "mock", close: async () => {} };
}
