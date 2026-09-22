import { ShieldAlert } from "lucide-react";
import { version } from "../../../package.json";
import { strings } from "@/lib/strings";
import { BrandGlyph } from "@/components/brand";
import { SettingsSection } from "./section";

const CREDITS = [
  { name: "pytapo", by: "Juraj Nyíri", url: "github.com/JurajNyiri/pytapo" },
  { name: "Home Assistant Tapo integration", by: "Juraj Nyíri", url: "github.com/JurajNyiri/HomeAssistant-Tapo-Control" },
  { name: "go2rtc", by: "Alexey Khit", url: "github.com/AlexxIT/go2rtc" },
  { name: "tapo (Rust crate)", by: "Mihai Dinculescu", url: "github.com/mihai-dinculescu/tapo" },
  { name: "tapo-v4-protocol", by: "freeKC", url: "github.com/freeKC/tapo-v4-protocol" },
];

export const APP_VERSION: string = version;

export function AboutSection() {
  return (
    <SettingsSection id="about" title={strings.settings.sections.about}>
      <div className="flex items-start gap-4 px-5 py-5">
        <BrandGlyph className="size-12" />
        <div className="min-w-0">
          <p className="text-[17px] font-semibold tracking-[-0.02em] text-fg">
            {strings.app.name}
            <span className="ml-2 align-middle text-xs font-medium text-fg-3">{strings.settings.version(APP_VERSION)}</span>
          </p>
          <p className="mt-1 max-w-xl text-[13.5px] leading-relaxed text-fg-2">{strings.settings.aboutBody}</p>
          <p className="mt-1 text-[13px] text-fg-3">{strings.settings.license}</p>
        </div>
      </div>
      <div className="px-5 py-4">
        <p className="text-sm font-medium text-fg">{strings.settings.credits}</p>
        <p className="mt-0.5 text-[13px] text-fg-2">{strings.settings.creditsBody}</p>
        <ul className="mt-3 grid gap-2 sm:grid-cols-2">
          {CREDITS.map((c) => (
            <li key={c.name} className="rounded-xl bg-surface-2 px-3 py-2.5">
              <p className="text-[13px] font-medium text-fg">
                {c.name} <span className="font-normal text-fg-3">— {c.by}</span>
              </p>
              <p className="mt-0.5 truncate font-mono text-[11.5px] text-fg-3">{c.url}</p>
            </li>
          ))}
        </ul>
      </div>
      <div className="px-5 py-4">
        <div className="flex gap-3 rounded-xl bg-neutral-soft p-4">
          <ShieldAlert className="mt-0.5 size-[18px] shrink-0 text-neutral" />
          <div>
            <p className="text-[13px] font-semibold text-fg">{strings.settings.disclaimerTitle}</p>
            <p className="mt-1 text-[13px] leading-relaxed text-fg-2">{strings.settings.disclaimer}</p>
          </div>
        </div>
      </div>
    </SettingsSection>
  );
}
