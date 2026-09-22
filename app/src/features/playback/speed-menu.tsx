import { Gauge } from "lucide-react";
import { strings } from "@/lib/strings";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip } from "@/components/ui/tooltip";
import { SPEEDS } from "@/features/timeline/math";

export function SpeedMenu({ speed, onChange }: { speed: number; onChange: (speed: number) => void }) {
  return (
    <DropdownMenu>
      <Tooltip content={strings.playback.speed} shortcut="[ ]">
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={`${strings.playback.speed}: ${strings.playback.speedValue(speed)}`}
            className="video-glass inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-xs font-semibold tabular-nums transition-colors hover:bg-black/65"
          >
            <Gauge className="size-3.5" />
            {strings.playback.speed} {strings.playback.speedValue(speed)}
          </button>
        </DropdownMenuTrigger>
      </Tooltip>
      <DropdownMenuContent align="end" className="min-w-36">
        <DropdownMenuLabel>{strings.playback.speed}</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={String(speed)} onValueChange={(v) => onChange(Number(v))}>
          {SPEEDS.map((s) => (
            <DropdownMenuRadioItem key={s} value={String(s)} className="tabular-nums">
              {strings.playback.speedValue(s)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
