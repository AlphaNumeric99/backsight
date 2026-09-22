import type { CameraId, MultiviewLayout, StreamQuality } from "@/ipc";

export interface LayoutSpec {
  id: MultiviewLayout;
  cols: number;
  rows: number;
  /** Tiles per page. */
  tiles: number;
  /** Index of a tile spanning 2×2 (the "1" in "1+5"). */
  feature?: number;
}

export const LAYOUT_ORDER: MultiviewLayout[] = ["1", "2", "4", "1+5", "9", "16"];

export const LAYOUTS: Record<MultiviewLayout, LayoutSpec> = {
  "1": { id: "1", cols: 1, rows: 1, tiles: 1 },
  "2": { id: "2", cols: 2, rows: 1, tiles: 2 },
  "4": { id: "4", cols: 2, rows: 2, tiles: 4 },
  "1+5": { id: "1+5", cols: 3, rows: 3, tiles: 6, feature: 0 },
  "9": { id: "9", cols: 3, rows: 3, tiles: 9 },
  "16": { id: "16", cols: 4, rows: 4, tiles: 16 },
};

export function pageCount(cameraCount: number, spec: LayoutSpec): number {
  return Math.max(1, Math.ceil(cameraCount / spec.tiles));
}

export function pageSlice<T>(items: readonly T[], spec: LayoutSpec, page: number): T[] {
  return items.slice(page * spec.tiles, (page + 1) * spec.tiles);
}

/**
 * The saved order, minus cameras that no longer exist, plus any new cameras at the end in
 * their list order.
 */
export function normalizeOrder(saved: readonly CameraId[], cameras: readonly { id: CameraId }[]): CameraId[] {
  const known = new Set(cameras.map((c) => c.id));
  const out: CameraId[] = [];
  const seen = new Set<CameraId>();
  for (const id of saved) {
    if (known.has(id) && !seen.has(id)) {
      out.push(id);
      seen.add(id);
    }
  }
  for (const c of cameras) if (!seen.has(c.id)) out.push(c.id);
  return out;
}

/** Moves `id` to where `overId` is, returning a new order. */
export function moveInOrder(order: readonly CameraId[], id: CameraId, overId: CameraId): CameraId[] {
  const from = order.indexOf(id);
  const to = order.indexOf(overId);
  if (from === -1 || to === -1 || from === to) return [...order];
  const next = [...order];
  next.splice(from, 1);
  next.splice(to, 0, id);
  return next;
}

/** The largest grid of 16:9 tiles that fits the container. */
export function fitGrid(
  container: { width: number; height: number },
  spec: Pick<LayoutSpec, "cols" | "rows">,
  gap = 3,
): { width: number; height: number } {
  if (container.width <= 0 || container.height <= 0) return { width: 0, height: 0 };
  const innerW = (w: number) => w - gap * (spec.cols - 1);
  const innerH = (h: number) => h - gap * (spec.rows - 1);
  // Height needed for a given width, with every tile at 16:9.
  const heightFor = (w: number) => (innerW(w) / spec.cols) * (9 / 16) * spec.rows + gap * (spec.rows - 1);
  let width = container.width;
  let height = heightFor(width);
  if (height > container.height) {
    height = container.height;
    width = (innerH(height) / spec.rows) * (16 / 9) * spec.cols + gap * (spec.cols - 1);
  }
  return { width: Math.floor(width), height: Math.floor(height) };
}

/** Large tiles get HD; small ones SD, which is lighter on the camera and the network. */
export function tileQuality(spec: LayoutSpec, index: number): StreamQuality {
  if (spec.tiles <= 2) return "hd";
  if (spec.feature === index) return "hd";
  return "sd";
}

export const formatPage = (n: number) => String(n).padStart(2, "0");
