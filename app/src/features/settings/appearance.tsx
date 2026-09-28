import { RadioGroup } from "radix-ui";
import { Check } from "lucide-react";
import type { Settings } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { SettingRow, SettingsSection } from "./section";

type Theme = Settings["theme"];

const THEMES: Theme[] = ["system", "light", "dark"];

function Preview({ tone }: { tone: "light" | "dark" }) {
  const c =
    tone === "light"
      ? { bg: "#f4f5f7", card: "#ffffff", line: "#e3e6eb", text: "#c9ced6", side: "#fafbfc" }
      : { bg: "#0c0e12", card: "#171a21", line: "#262a33", text: "#353a45", side: "#101216" };
  return (
    <svg viewBox="0 0 120 72" className="h-full w-full" aria-hidden>
      <rect width="120" height="72" fill={c.bg} />
      <rect width="28" height="72" fill={c.side} />
      <rect x="6" y="8" width="8" height="8" rx="2.5" fill="#3461f4" />
      <rect x="6" y="24" width="16" height="3" rx="1.5" fill={c.text} />
      <rect x="6" y="32" width="12" height="3" rx="1.5" fill={c.text} />
      <rect x="36" y="10" width="36" height="5" rx="2.5" fill={c.text} />
      <rect x="36" y="22" width="36" height="40" rx="5" fill={c.card} stroke={c.line} />
      <rect x="78" y="22" width="36" height="40" rx="5" fill={c.card} stroke={c.line} />
      <rect x="36" y="22" width="36" height="22" rx="5" fill={tone === "light" ? "#dfe6f5" : "#1f2533"} />
      <rect x="78" y="22" width="36" height="22" rx="5" fill={tone === "light" ? "#dfe6f5" : "#1f2533"} />
      <rect x="40" y="50" width="20" height="3" rx="1.5" fill={c.text} />
      <rect x="82" y="50" width="16" height="3" rx="1.5" fill={c.text} />
      <rect x="92" y="9" width="22" height="7" rx="3.5" fill="#3461f4" />
    </svg>
  );
}

export function AppearanceSection({ theme, onThemeChange }: { theme: Theme; onThemeChange: (t: Theme) => void }) {
  return (
    <SettingsSection id="appearance" title={strings.settings.sections.appearance}>
      <SettingRow label={strings.settings.theme} help={strings.settings.themeHelp} stacked>
        <RadioGroup.Root
          value={theme}
          onValueChange={(v) => onThemeChange(v as Theme)}
          aria-label={strings.settings.theme}
          className="grid grid-cols-3 gap-3 max-w-[480px]"
        >
          {THEMES.map((t) => (
            <RadioGroup.Item
              key={t}
              value={t}
              className="group text-left outline-none"
            >
              <span
                className={cn(
                  "relative block aspect-[5/3] overflow-hidden rounded-xl border-2 transition-[border-color,box-shadow] duration-(--dur-fast)",
                  "group-data-[state=checked]:border-brand group-data-[state=checked]:shadow-[0_0_0_4px_var(--brand-soft)]",
                  "border-border group-hover:border-border-strong group-focus-visible:ring-2 group-focus-visible:ring-ring group-focus-visible:ring-offset-2 group-focus-visible:ring-offset-surface",
                )}
              >
                {t === "system" ? (
                  <>
                    <span className="absolute inset-0">
                      <Preview tone="light" />
                    </span>
                    <span className="absolute inset-0 [clip-path:polygon(62%_0,100%_0,100%_100%,38%_100%)]">
                      <Preview tone="dark" />
                    </span>
                  </>
                ) : (
                  <Preview tone={t} />
                )}
                <RadioGroup.Indicator className="absolute right-1.5 top-1.5 grid size-5 place-items-center rounded-full bg-brand text-white shadow-xs">
                  <Check className="size-3" strokeWidth={3} />
                </RadioGroup.Indicator>
              </span>
              <span className="mt-2 block text-center text-[13px] font-medium text-fg-2 group-data-[state=checked]:text-fg">
                {strings.settings.themes[t]}
              </span>
            </RadioGroup.Item>
          ))}
        </RadioGroup.Root>
      </SettingRow>
    </SettingsSection>
  );
}
