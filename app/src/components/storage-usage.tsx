import type { CameraState, StorageInfo } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { formatBytes } from "@/lib/format";
import { SdCardIcon } from "./icons";

/** `state` is the camera's: without storage info yet, a reachable camera is still being checked. */
export function storageSummary(
  storage?: StorageInfo,
  state?: CameraState,
): {
  text: string;
  fraction: number;
  tone: "normal" | "warning" | "danger" | "muted";
} {
  if (!storage) {
    const checking = state === "connecting" || state === "online";
    return { text: checking ? strings.storage.checking : strings.storage.unknown, fraction: 0, tone: "muted" };
  }
  if (!storage.present || storage.status === "none") return { text: strings.storage.none, fraction: 0, tone: "muted" };
  if (storage.status === "unformatted") return { text: strings.storage.unformatted, fraction: 0, tone: "warning" };
  if (storage.status === "error") return { text: strings.storage.error, fraction: 0, tone: "danger" };
  const used = Math.max(0, storage.totalBytes - storage.freeBytes);
  const fraction = storage.totalBytes > 0 ? used / storage.totalBytes : 0;
  // With loop recording a full card is the steady state: the oldest footage is overwritten.
  if (storage.loopRecording && (storage.status === "full" || fraction >= 0.9)) {
    return { text: strings.storage.loop(formatBytes(storage.totalBytes)), fraction, tone: "normal" };
  }
  const text =
    storage.status === "full"
      ? strings.storage.full
      : strings.storage.usage(formatBytes(used), formatBytes(storage.totalBytes));
  return { text, fraction, tone: storage.status === "full" ? "danger" : fraction >= 0.9 ? "warning" : "normal" };
}

/** "SD card · 81.9 of 128 GB" with a slim usage bar. */
export function StorageUsage({
  storage,
  state,
  className,
}: {
  storage?: StorageInfo;
  state?: CameraState;
  className?: string;
}) {
  const s = storageSummary(storage, state);
  return (
    <div className={cn("grid gap-1.5", className)}>
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="inline-flex items-center gap-1.5 text-fg-2">
          <SdCardIcon size={14} />
          {strings.storage.label}
        </span>
        <span
          className={cn(
            "truncate tabular-nums",
            s.tone === "danger" ? "text-danger" : s.tone === "warning" ? "text-warning" : s.tone === "muted" ? "text-fg-3" : "text-fg-2",
          )}
        >
          {s.text}
        </span>
      </div>
      <div
        className="h-1 overflow-hidden rounded-full bg-sunken"
        role="meter"
        aria-label={strings.storage.label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(s.fraction * 100)}
        aria-valuetext={s.text}
      >
        <div
          className={cn(
            "h-full rounded-full transition-[width] duration-500 ease-standard",
            s.tone === "danger" ? "bg-danger-dot" : s.tone === "warning" ? "bg-warning-dot" : "bg-brand",
          )}
          style={{ width: `${Math.max(s.fraction > 0 ? 3 : 0, s.fraction * 100)}%` }}
        />
      </div>
    </div>
  );
}
