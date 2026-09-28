import { Info } from "lucide-react";
import type { PlayerStats } from "@/player/types";
import { strings } from "@/lib/strings";
import { formatBitrate, formatCodec, formatResolution } from "@/lib/format";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Tooltip } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";

export function StatsPopover({ stats }: { stats: PlayerStats | null }) {
  const rows: [string, string | undefined][] = stats
    ? [
        [strings.live.statsRows.codec, formatCodec(stats.codec)],
        [strings.live.statsRows.resolution, formatResolution(stats.width, stats.height)],
        [strings.live.statsRows.fps, `${stats.fps.toFixed(stats.fps % 1 ? 1 : 0)} fps`],
        [strings.live.statsRows.bitrate, formatBitrate(stats.bitrate)],
        [strings.live.statsRows.dropped, String(stats.droppedFrames)],
        [strings.live.statsRows.queue, String(stats.decodeQueue)],
        [strings.live.statsRows.latency, stats.latencyMs !== undefined ? `${Math.round(stats.latencyMs)} ms` : undefined],
      ]
    : [];
  return (
    <Popover>
      <Tooltip content={strings.live.stats}>
        <PopoverTrigger asChild>
          <Button variant="overlay" size="icon-sm" aria-label={strings.live.stats}>
            <Info />
          </Button>
        </PopoverTrigger>
      </Tooltip>
      <PopoverContent align="end" className="w-64 p-0">
        <div className="border-b border-border px-4 py-3 text-sm font-semibold">{strings.live.stats}</div>
        {stats ? (
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 px-4 py-3 text-[13px]">
            {rows.map(([label, value]) => (
              <div key={label} className="contents">
                <dt className="text-fg-2">{label}</dt>
                <dd className="truncate text-right font-medium tabular-nums text-fg">{value ?? "—"}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="px-4 py-5 text-center text-[13px] text-fg-2">{strings.live.statsWaiting}</p>
        )}
      </PopoverContent>
    </Popover>
  );
}
