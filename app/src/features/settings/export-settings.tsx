import { useEffect, useId, useRef, useState } from "react";
import { toast } from "sonner";
import { FolderOpen } from "lucide-react";
import type { Settings } from "@/ipc";
import { strings } from "@/lib/strings";
import { EXPORT_NAME_TOKENS, renderExportName, type ExportNameToken } from "@/lib/export-name";
import { localOffsetMinutes, MINUTE } from "@/lib/time";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Segmented } from "@/components/ui/segmented";
import { Slider } from "@/components/ui/slider";
import { SettingRow, SettingsSection } from "./section";

const CACHE_STEPS_MB = [256, 512, 1024, 2048, 4096, 8192, 16384, 32768];

export function formatCacheSize(mb: number): string {
  return mb >= 1024 ? `${Number((mb / 1024).toFixed(1))} GB` : `${mb} MB`;
}

export function ExportSettingsSection({
  settings,
  sampleCameraName,
  onChange,
}: {
  settings: Settings;
  sampleCameraName: string;
  onChange: (patch: Partial<Settings>) => void;
}) {
  const nameId = useId();
  const [template, setTemplate] = useState(settings.exportNameTemplate);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const cacheIndex = Math.max(0, CACHE_STEPS_MB.findIndex((mb) => mb >= settings.cacheLimitMb));
  const [cache, setCache] = useState(cacheIndex);

  useEffect(() => setTemplate(settings.exportNameTemplate), [settings.exportNameTemplate]);
  useEffect(() => setCache(cacheIndex), [cacheIndex]);

  const commitTemplate = (value: string) => {
    if (!value.trim()) {
      setError(strings.settings.nameTemplateEmpty);
      return;
    }
    setError(null);
    if (value !== settings.exportNameTemplate) onChange({ exportNameTemplate: value });
  };

  const insertToken = (token: ExportNameToken) => {
    const el = inputRef.current;
    const text = `{${token}}`;
    const start = el?.selectionStart ?? template.length;
    const end = el?.selectionEnd ?? template.length;
    const next = template.slice(0, start) + text + template.slice(end);
    setTemplate(next);
    commitTemplate(next);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(start + text.length, start + text.length);
    });
  };

  const now = Date.now();
  const preview = template.trim()
    ? renderExportName(template, {
        camera: sampleCameraName,
        start: now - 6 * MINUTE - 4000,
        end: now,
        offsetMinutes: localOffsetMinutes(now),
      })
    : "—";

  return (
    <SettingsSection id="recordings" title={strings.settings.sections.recordings}>
      <SettingRow label={strings.settings.exportDir} help={strings.settings.exportDirHelp}>
        <div className="flex max-w-full items-center gap-2">
          <code className="flex h-9 min-w-0 max-w-[320px] items-center gap-2 truncate rounded-control border border-border bg-surface-2 px-3 font-mono text-[12.5px] text-fg">
            <FolderOpen className="size-4 shrink-0 text-fg-3" />
            <span className="truncate">{settings.exportDir}</span>
          </code>
          <Button variant="outline" size="sm" className="h-9" onClick={() => toast.info(strings.settings.chooseUnavailable)}>
            {strings.settings.choose}
          </Button>
        </div>
      </SettingRow>

      <SettingRow label={strings.settings.nameTemplate} help={strings.settings.nameTemplateHelp} htmlFor={nameId} stacked>
        <div className="grid max-w-[560px] gap-2.5">
          <div className="relative">
            <Input
              id={nameId}
              ref={inputRef}
              value={template}
              onChange={(e) => {
                setTemplate(e.target.value);
                if (error && e.target.value.trim()) setError(null);
              }}
              onBlur={() => commitTemplate(template)}
              onKeyDown={(e) => e.key === "Enter" && commitTemplate(template)}
              aria-invalid={Boolean(error) || undefined}
              spellCheck={false}
              className="pr-14 font-mono text-[13px]"
            />
            <span className="pointer-events-none absolute inset-y-0 right-3 grid place-items-center font-mono text-[12.5px] text-fg-3">
              .mp4
            </span>
          </div>
          {error && (
            <p className="text-[12.5px] text-danger" role="alert">
              {error}
            </p>
          )}
          <div className="flex flex-wrap items-center gap-1.5">
            {EXPORT_NAME_TOKENS.map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => insertToken(t)}
                className="inline-flex h-7 items-center gap-1 rounded-full border border-border bg-surface px-2.5 text-xs font-medium text-fg-2 transition-colors hover:border-brand/40 hover:bg-brand-softer hover:text-brand-text"
              >
                <span className="font-mono text-brand-text">+</span>
                {strings.settings.tokens[t]}
              </button>
            ))}
          </div>
          <p className="min-w-0 truncate text-xs text-fg-3">
            {strings.settings.preview}: <span className="font-mono text-fg-2">{preview}</span>
          </p>
        </div>
      </SettingRow>

      <SettingRow label={strings.settings.cacheLimit} help={strings.settings.cacheLimitHelp}>
        <div className="flex w-[260px] items-center gap-4">
          <Slider
            aria-label={strings.settings.cacheLimit}
            min={0}
            max={CACHE_STEPS_MB.length - 1}
            step={1}
            value={[cache]}
            onValueChange={([v]) => setCache(v)}
            onValueCommit={([v]) => onChange({ cacheLimitMb: CACHE_STEPS_MB[v] })}
          />
          <span className="w-14 shrink-0 text-right text-sm font-medium tabular-nums text-fg">
            {formatCacheSize(CACHE_STEPS_MB[cache])}
          </span>
        </div>
      </SettingRow>

      <SettingRow label={strings.settings.defaultQuality} help={strings.settings.defaultQualityHelp}>
        <Segmented
          aria-label={strings.settings.defaultQuality}
          value={settings.defaultLiveQuality}
          onValueChange={(q) => onChange({ defaultLiveQuality: q })}
          items={[
            { value: "hd", label: strings.live.hd },
            { value: "sd", label: strings.live.sd },
          ]}
        />
      </SettingRow>
    </SettingsSection>
  );
}
