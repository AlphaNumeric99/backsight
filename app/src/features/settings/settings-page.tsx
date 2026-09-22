import { useEffect } from "react";
import { useSearch } from "@tanstack/react-router";
import { strings } from "@/lib/strings";
import { Card, Skeleton } from "@/components/ui/misc";
import { useCameras, useGroups } from "@/queries/cameras";
import { useSettings, useUpdateSettings } from "@/queries/settings";
import { AppearanceSection } from "./appearance";
import { ExportSettingsSection } from "./export-settings";
import { CamerasSettingsSection, GroupsSettingsSection } from "./cameras-settings";
import { AboutSection } from "./about";

const NAV = [
  { id: "appearance", label: strings.settings.sections.appearance },
  { id: "recordings", label: strings.settings.sections.recordings },
  { id: "cameras", label: strings.settings.sections.cameras },
  { id: "groups", label: strings.settings.sections.groups },
  { id: "about", label: strings.settings.sections.about },
] as const;

export function SettingsPage() {
  const settings = useSettings();
  const update = useUpdateSettings();
  const cameras = useCameras();
  const groups = useGroups();
  const { section } = useSearch({ from: "/settings" });

  useEffect(() => {
    document.title = `${strings.settings.title} — ${strings.app.name}`;
  }, []);

  // Jump to a section when linked to one (e.g. "Change folder" in Downloads).
  const ready = Boolean(settings.data && cameras.data && groups.data);
  useEffect(() => {
    if (!section || !ready) return;
    const el = document.getElementById(`settings-${section}`);
    el?.scrollIntoView({ block: "start", behavior: "smooth" });
  }, [section, ready]);

  const scrollTo = (id: string) =>
    document.getElementById(`settings-${id}`)?.scrollIntoView({ block: "start", behavior: "smooth" });

  return (
    <div className="mx-auto flex w-full max-w-[1080px] flex-1 gap-10 px-8 pb-16 pt-9 max-[1099px]:px-6">
      <nav aria-label={strings.settings.title} className="sticky top-9 hidden h-fit w-44 shrink-0 min-[1180px]:block">
        <ul className="grid gap-0.5">
          {NAV.map((n) => (
            <li key={n.id}>
              <button
                type="button"
                onClick={() => scrollTo(n.id)}
                className="w-full rounded-lg px-3 py-2 text-left text-[13.5px] font-medium text-fg-2 transition-colors hover:bg-hover hover:text-fg"
              >
                {n.label}
              </button>
            </li>
          ))}
        </ul>
      </nav>

      <div className="min-w-0 flex-1">
        <header className="mb-8">
          <h1 className="text-[30px] font-semibold leading-[1.15] tracking-[-0.03em] text-fg">{strings.settings.title}</h1>
          <p className="mt-1.5 text-sm text-fg-2">{strings.settings.subtitle}</p>
        </header>

        {!settings.data ? (
          <div className="grid gap-8">
            {[0, 1].map((i) => (
              <Card key={i} className="grid gap-4 p-5">
                <Skeleton className="h-4 w-1/4" />
                <Skeleton className="h-10 w-full" />
                <Skeleton className="h-10 w-2/3" />
              </Card>
            ))}
          </div>
        ) : (
          <div className="grid gap-9">
            <AppearanceSection theme={settings.data.theme} onThemeChange={(theme) => update.mutate({ theme })} />
            <ExportSettingsSection
              settings={settings.data}
              sampleCameraName={cameras.data?.[0]?.name ?? "Front Door"}
              onChange={(patch) => update.mutate(patch)}
            />
            <CamerasSettingsSection cameras={cameras.data ?? []} groups={groups.data ?? []} />
            <GroupsSettingsSection groups={groups.data ?? []} cameras={cameras.data ?? []} />
            <AboutSection />
          </div>
        )}
      </div>
    </div>
  );
}
