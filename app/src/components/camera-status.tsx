import type { ReactNode } from "react";
import { Ban, EyeOff, KeyRound, Lock, WifiOff, type LucideIcon } from "lucide-react";
import type { CameraStatus } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { formatRelative } from "@/lib/format";
import { minutesUntil } from "@/lib/time";
import { useNow } from "@/hooks/use-now";

export type StatusTone = "success" | "warning" | "danger" | "neutral";

export interface StatusInfo {
  tone: StatusTone;
  label: string;
  /** A sentence explaining the state, for overlays. */
  detail?: string;
  icon?: LucideIcon;
  pulse?: boolean;
  /** Whether video can be shown. */
  viewable: boolean;
}

/** Human copy for a camera state, e.g. locked → "Locked — retry in 23 min". */
export function describeStatus(status: CameraStatus, now: number = Date.now()): StatusInfo {
  switch (status.state) {
    case "online":
      return { tone: "success", label: strings.status.online, viewable: true };
    case "connecting":
      return {
        tone: "warning",
        label: strings.status.connecting,
        detail: strings.statusDetail.connecting,
        pulse: true,
        viewable: true,
      };
    case "offline":
      return {
        tone: "neutral",
        label: status.lastSeen
          ? strings.status.offlineSince(formatRelative(Date.parse(status.lastSeen), now))
          : strings.status.offline,
        detail: strings.statusDetail.offline,
        icon: WifiOff,
        viewable: false,
      };
    case "privacy":
      return {
        tone: "neutral",
        label: strings.status.privacy,
        detail: strings.statusDetail.privacy,
        icon: EyeOff,
        viewable: false,
      };
    case "locked": {
      const mins = status.lockedUntil ? minutesUntil(status.lockedUntil, now) : 30;
      return {
        tone: "danger",
        label: strings.status.locked(mins),
        detail: strings.statusDetail.locked(mins),
        icon: Lock,
        viewable: false,
      };
    }
    case "auth_failed":
      return {
        tone: "danger",
        label: strings.status.authFailed,
        detail: strings.statusDetail.authFailed,
        icon: KeyRound,
        viewable: false,
      };
    case "unsupported":
      return {
        tone: "neutral",
        label: strings.status.unsupported,
        detail: strings.statusDetail.unsupported,
        icon: Ban,
        viewable: false,
      };
  }
}

/** Re-evaluates every 30 s so countdowns and "seen … ago" stay current. */
export function useStatusInfo(status: CameraStatus): StatusInfo {
  const now = useNow(30_000);
  return describeStatus(status, now);
}

const dotTone: Record<StatusTone, string> = {
  success: "bg-success-dot",
  warning: "bg-warning-dot",
  danger: "bg-danger-dot",
  neutral: "bg-neutral-dot",
};

const softTone: Record<StatusTone, string> = {
  success: "bg-success-soft text-success",
  warning: "bg-warning-soft text-warning",
  danger: "bg-danger-soft text-danger",
  neutral: "bg-neutral-soft text-neutral",
};

export function StatusDot({ tone, pulse, className }: { tone: StatusTone; pulse?: boolean; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn("inline-block size-2 shrink-0 rounded-full", dotTone[tone], pulse && "animate-pulse-dot", className)}
    />
  );
}

export function StatusPill({
  status,
  variant = "soft",
  className,
  trailing,
}: {
  status: CameraStatus;
  variant?: "soft" | "overlay" | "plain";
  className?: string;
  trailing?: ReactNode;
}) {
  const info = useStatusInfo(status);
  const Icon = info.icon;
  return (
    <span
      className={cn(
        "inline-flex h-6 max-w-full items-center gap-1.5 truncate rounded-full px-2.5 text-xs font-medium leading-none",
        variant === "soft" && softTone[info.tone],
        variant === "overlay" && "video-glass",
        variant === "plain" && "px-0 text-fg-2",
        className,
      )}
    >
      {Icon ? (
        <Icon className={cn("size-3.5 shrink-0", variant === "overlay" && info.tone === "danger" && "text-[#ff8a8e]")} aria-hidden />
      ) : (
        <StatusDot tone={info.tone} pulse={info.pulse} />
      )}
      <span className="truncate">{info.label}</span>
      {trailing}
    </span>
  );
}
