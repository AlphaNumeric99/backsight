import { useState } from "react";
import { Cctv } from "lucide-react";
import type { Camera } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";

/** The camera's preview (its latest captured frame), or a quiet placeholder when there isn't one. */
export function CameraSnapshot({
  camera,
  className,
  imgClassName,
  showPlaceholderLabel = true,
  placeholderLabel = strings.home.noPreview,
  bare = false,
}: {
  camera: Pick<Camera, "snapshotUrl" | "name">;
  className?: string;
  imgClassName?: string;
  showPlaceholderLabel?: boolean;
  placeholderLabel?: string;
  /** Placeholder without its icon and label, when something else is drawn on top. */
  bare?: boolean;
}) {
  // Remember which URL failed, so a newer preview gets its chance.
  const [failedUrl, setFailedUrl] = useState<string>();
  const url = camera.snapshotUrl !== failedUrl ? camera.snapshotUrl : undefined;
  return (
    <div className={cn("relative size-full overflow-hidden bg-video", className)}>
      {url ? (
        <img
          src={url}
          alt=""
          draggable={false}
          decoding="async"
          loading="lazy"
          onError={() => setFailedUrl(url)}
          className={cn("size-full object-cover", imgClassName)}
        />
      ) : (
        <SnapshotPlaceholder label={showPlaceholderLabel ? placeholderLabel : undefined} bare={bare} />
      )}
    </div>
  );
}

export function SnapshotPlaceholder({ label, className, bare = false }: { label?: string; className?: string; bare?: boolean }) {
  return (
    <div
      className={cn("absolute inset-0 grid place-items-center", className)}
      style={{
        background:
          "radial-gradient(120% 95% at 28% 18%, #283144 0%, #151a25 55%, #0b0d12 100%), #0b0d12",
      }}
    >
      <div
        aria-hidden
        className="absolute inset-0 opacity-[0.07]"
        style={{
          backgroundImage:
            "linear-gradient(rgb(255 255 255 / 0.9) 1px, transparent 1px), linear-gradient(90deg, rgb(255 255 255 / 0.9) 1px, transparent 1px)",
          backgroundSize: "28px 28px",
          maskImage: "radial-gradient(80% 70% at 50% 45%, #000 20%, transparent 75%)",
        }}
      />
      {!bare && (
        <div className="relative flex flex-col items-center gap-2 text-white/35">
          <Cctv className="size-8" strokeWidth={1.6} aria-hidden />
          {label && <span className="text-xs font-medium text-white/45">{label}</span>}
        </div>
      )}
    </div>
  );
}
