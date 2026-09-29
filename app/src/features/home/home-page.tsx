import { useEffect, useMemo } from "react";
import { AnimatePresence, LayoutGroup } from "motion/react";
import { Check, Plus, RotateCw, Star } from "lucide-react";
import type { Camera } from "@/ipc";
import { strings } from "@/lib/strings";
import { formatLocalDateLabel, localDateOf, localOffsetMinutes } from "@/lib/time";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/misc";
import { EmptyState } from "@/components/ui/empty-state";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useNow } from "@/hooks/use-now";
import { useCameras, useGroups } from "@/queries/cameras";
import { useUiStore } from "@/state/ui";
import { usePreviewRefresh } from "@/features/previews/use-preview-refresh";
import { CameraCard, CameraCardSkeleton } from "./camera-card";
import { EmptyCamerasArt } from "./empty-art";

const NEEDS_ATTENTION = new Set<Camera["status"]["state"]>(["locked", "auth_failed", "offline", "unsupported"]);

export function HomePage() {
  const cameras = useCameras();
  const groups = useGroups();
  const group = useUiStore((s) => s.homeGroup);
  const setGroup = useUiStore((s) => s.setHomeGroup);
  const openAdd = useUiStore((s) => s.setAddCameraOpen);
  const now = useNow(60_000);

  useEffect(() => {
    document.title = `${strings.home.title} — ${strings.app.name}`;
  }, []);

  const list = cameras.data ?? [];
  const groupList = groups.data ?? [];
  const capturing = usePreviewRefresh(list);
  const validGroup = group === "all" || group === "favorites" || groupList.some((g) => g.id === group) ? group : "all";

  const filtered = useMemo(() => {
    if (validGroup === "all") return list;
    if (validGroup === "favorites") return list.filter((c) => c.favorite);
    return list.filter((c) => c.groupIds.includes(validGroup));
  }, [list, validGroup]);

  const online = list.filter((c) => c.status.state === "online").length;
  const attention = list.filter((c) => NEEDS_ATTENTION.has(c.status.state)).length;
  const dateLabel = formatLocalDateLabel(localDateOf(now, localOffsetMinutes(now)), {
    weekday: "long",
    day: "numeric",
    month: "long",
  });

  const tabs = [
    { value: "all", label: strings.home.all, count: list.length },
    { value: "favorites", label: strings.home.favorites, count: list.filter((c) => c.favorite).length },
    ...groupList.map((g) => ({ value: g.id, label: g.name, count: list.filter((c) => c.groupIds.includes(g.id)).length })),
  ];

  return (
    <div className="relative flex-1">
      <div aria-hidden className="hero-glow pointer-events-none absolute inset-x-0 top-0 h-[320px]" />
      <div className="relative mx-auto w-full max-w-[1680px] px-8 pb-14 pt-9 max-[1099px]:px-6">
        <header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-4">
          <div className="min-w-0">
            <p className="text-[13px] font-medium text-fg-2">{dateLabel}</p>
            <h1 className="mt-1 text-[30px] font-semibold leading-[1.15] tracking-[-0.03em] text-fg">
              {strings.home.title}
            </h1>
            {list.length > 0 && (
              <p className="mt-1.5 text-sm text-fg-2">{strings.home.subtitle(online, list.length, attention)}</p>
            )}
          </div>
          <Button variant="primary" size="lg" onClick={() => openAdd(true)} className="shadow-raised">
            <Plus strokeWidth={2.4} />
            {strings.home.addCamera}
          </Button>
        </header>

        {cameras.isPending ? (
          <div className="mt-9 grid gap-5 [grid-template-columns:repeat(auto-fill,minmax(248px,1fr))]">
            {Array.from({ length: 6 }, (_, i) => (
              <CameraCardSkeleton key={i} />
            ))}
          </div>
        ) : cameras.isError ? (
          <Card className="mt-9 p-8">
            <EmptyState
              size="sm"
              title={strings.home.loadError}
              actions={
                <Button variant="outline" onClick={() => cameras.refetch()}>
                  <RotateCw />
                  {strings.common.retry}
                </Button>
              }
            />
          </Card>
        ) : list.length === 0 ? (
          <Card className="mt-9 px-8">
            <EmptyState
              art={<EmptyCamerasArt />}
              title={strings.home.emptyTitle}
              body={strings.home.emptyBody}
              actions={
                <Button variant="primary" size="lg" onClick={() => openAdd(true)}>
                  <Plus strokeWidth={2.4} />
                  {strings.home.addCamera}
                </Button>
              }
            >
              <div className="mt-6 w-full rounded-2xl bg-surface-2 p-4 text-left">
                <p className="text-xs font-semibold uppercase tracking-wider text-fg-3">
                  {strings.home.emptyChecklistTitle}
                </p>
                <ul className="mt-2.5 grid gap-2">
                  {strings.home.emptyChecklist.map((item) => (
                    <li key={item} className="flex items-start gap-2.5 text-[13px] text-fg-2">
                      <span className="mt-0.5 grid size-4 shrink-0 place-items-center rounded-full bg-brand-soft text-brand-text">
                        <Check className="size-3" strokeWidth={3} />
                      </span>
                      {item}
                    </li>
                  ))}
                </ul>
              </div>
            </EmptyState>
          </Card>
        ) : (
          <Tabs value={validGroup} onValueChange={setGroup} variant="pills" className="mt-7">
            <TabsList aria-label={strings.home.groupsLabel}>
              {tabs.map((t) => (
                <TabsTrigger key={t.value} value={t.value}>
                  {t.value === "favorites" && <Star className="size-3.5" />}
                  {t.label}
                  <span className="tabular-nums opacity-60">{t.count}</span>
                </TabsTrigger>
              ))}
            </TabsList>
            <TabsContent value={validGroup} className="mt-6" tabIndex={-1}>
              {filtered.length === 0 ? (
                <Card className="p-6">
                  <EmptyState
                    size="sm"
                    title={strings.home.groupEmptyTitle}
                    body={validGroup === "favorites" ? strings.home.favoritesEmpty : strings.home.groupEmpty}
                  />
                </Card>
              ) : (
                <LayoutGroup>
                  <div className="grid gap-5 [grid-template-columns:repeat(auto-fill,minmax(248px,1fr))]">
                    <AnimatePresence mode="popLayout" initial={false}>
                      {filtered.map((camera) => (
                        <CameraCard
                          key={camera.id}
                          camera={camera}
                          groups={groupList}
                          capturing={capturing === camera.id}
                        />
                      ))}
                    </AnimatePresence>
                  </div>
                </LayoutGroup>
              )}
            </TabsContent>
          </Tabs>
        )}
      </div>
    </div>
  );
}
