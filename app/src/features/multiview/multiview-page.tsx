import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEventHandler,
  type PointerEventHandler,
} from "react";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import { SortableContext, rectSortingStrategy, sortableKeyboardCoordinates, useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { AnimatePresence, motion } from "motion/react";
import { ChevronLeft, ChevronRight, LayoutGrid, Maximize, Minimize, Plus } from "lucide-react";
import type { Camera, MultiviewLayout } from "@/ipc";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { transitions } from "@/lib/motion";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Segmented } from "@/components/ui/segmented";
import { EmptyState } from "@/components/ui/empty-state";
import { Card } from "@/components/ui/misc";
import { LayoutIcon } from "@/components/icons";
import { useElementSize } from "@/hooks/use-element-size";
import { useCameras } from "@/queries/cameras";
import { useSettings, useUpdateSettings } from "@/queries/settings";
import { useUiStore } from "@/state/ui";
import { useFullscreen } from "@/features/camera/video-stage";
import { LAYOUTS, LAYOUT_ORDER, fitGrid, formatPage, moveInOrder, normalizeOrder, pageCount, pageSlice, tileQuality } from "./layouts";
import { Tile } from "./tile";

const GAP = 3;

export function MultiviewPage() {
  const cameras = useCameras();
  const settings = useSettings();
  const update = useUpdateSettings();
  const page = useUiStore((s) => s.multiviewPage);
  const setPage = useUiStore((s) => s.setMultiviewPage);
  const openAdd = useUiStore((s) => s.setAddCameraOpen);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [audioId, setAudioId] = useState<string | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const areaRef = useRef<HTMLDivElement>(null);
  const size = useElementSize(areaRef);
  const [fullscreen, toggleFullscreen] = useFullscreen(areaRef);

  useEffect(() => {
    document.title = `${strings.multiview.title} — ${strings.app.name}`;
  }, []);

  const list = cameras.data ?? [];
  const byId = useMemo(() => new Map(list.map((c) => [c.id, c])), [list]);
  const layout: MultiviewLayout = settings.data?.multiviewLayout ?? "4";
  const spec = LAYOUTS[layout];
  const order = useMemo(() => normalizeOrder(settings.data?.multiviewOrder ?? [], list), [settings.data?.multiviewOrder, list]);
  const pages = pageCount(order.length, spec);
  const current = Math.min(page, pages - 1);
  const ids = focusId ? [focusId] : pageSlice(order, spec, current);
  const shownSpec = focusId ? LAYOUTS["1"] : spec;
  const grid = fitGrid(size, shownSpec, GAP);

  useEffect(() => {
    if (page !== current) setPage(current);
  }, [page, current, setPage]);

  // Esc leaves focus mode.
  useEffect(() => {
    if (!focusId) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.fullscreenElement) setFocusId(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [focusId]);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const onDragStart = (e: DragStartEvent) => setDraggingId(String(e.active.id));
  const onDragEnd = (e: DragEndEvent) => {
    setDraggingId(null);
    if (!e.over || e.active.id === e.over.id) return;
    update.mutate({ multiviewOrder: moveInOrder(order, String(e.active.id), String(e.over.id)) });
  };

  const setLayout = (next: MultiviewLayout) => {
    setFocusId(null);
    update.mutate({ multiviewLayout: next });
  };

  const toggleMute = useCallback((id: string) => setAudioId((a) => (a === id ? null : id)), []);
  const toggleFocus = useCallback((id: string) => setFocusId((f) => (f === id ? null : id)), []);

  const nameOf = (id: string | number) => byId.get(String(id))?.name ?? String(id);
  const positionOf = (id: string | number) => order.indexOf(String(id)) + 1;

  const empty = cameras.isSuccess && list.length === 0;

  return (
    <div className="flex h-full flex-1 flex-col px-6 pb-6 pt-6">
      <header className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-3">
        <div className="min-w-0 flex-1">
          <h1 className="text-[24px] font-semibold leading-tight tracking-[-0.025em] text-fg">{strings.multiview.title}</h1>
          <p className="mt-0.5 text-[13px] text-fg-3">{strings.multiview.reorderHint}</p>
        </div>
        <Segmented
          aria-label={strings.multiview.layout}
          value={layout}
          onValueChange={setLayout}
          items={LAYOUT_ORDER.map((l) => ({
            value: l,
            label: strings.multiview.layouts[l],
            icon: <LayoutIcon layout={l} size={18} />,
            iconOnly: true,
          }))}
        />
        <div
          className={cn(
            "inline-flex h-10 items-center gap-0.5 rounded-full border border-card-border bg-surface p-1 shadow-xs",
            (pages <= 1 || focusId) && "opacity-50",
          )}
        >
          <IconButton
            label={strings.multiview.prevPage}
            size="icon-sm"
            disabled={current === 0 || !!focusId}
            onClick={() => setPage(current - 1)}
          >
            <ChevronLeft />
          </IconButton>
          <span
            className="min-w-[58px] text-center text-[13px] font-semibold tabular-nums text-fg"
            aria-label={strings.multiview.page(current + 1, pages)}
          >
            {formatPage(current + 1)}/{formatPage(pages)}
          </span>
          <IconButton
            label={strings.multiview.nextPage}
            size="icon-sm"
            disabled={current >= pages - 1 || !!focusId}
            onClick={() => setPage(current + 1)}
          >
            <ChevronRight />
          </IconButton>
        </div>
        <IconButton
          label={fullscreen ? strings.live.exitFullscreen : strings.live.fullscreen}
          variant="outline"
          onClick={toggleFullscreen}
          disabled={empty}
        >
          {fullscreen ? <Minimize /> : <Maximize />}
        </IconButton>
      </header>

      {empty ? (
        <Card className="grid flex-1 place-items-center">
          <EmptyState
            art={
              <div className="grid size-16 place-items-center rounded-2xl bg-brand-soft text-brand-text">
                <LayoutGrid className="size-8" strokeWidth={1.6} />
              </div>
            }
            title={strings.multiview.noCamerasTitle}
            body={strings.multiview.noCamerasBody}
            actions={
              <Button variant="primary" onClick={() => openAdd(true)}>
                <Plus />
                {strings.multiview.addCamera}
              </Button>
            }
          />
        </Card>
      ) : (
        <div
          ref={areaRef}
          data-theme="dark"
          className="theme-scope relative grid min-h-[320px] flex-1 place-items-center overflow-hidden rounded-card bg-[#030405]"
        >
          <AnimatePresence mode="popLayout" initial={false}>
            {focusId && (
              <motion.div
                key="focus-bar"
                initial={{ opacity: 0, y: -6 }}
                animate={{ opacity: 1, y: 0, transition: transitions.base }}
                exit={{ opacity: 0, transition: transitions.fast }}
                className="absolute left-1/2 top-3 z-40 -translate-x-1/2"
              >
                <Button size="sm" variant="overlay" onClick={() => setFocusId(null)}>
                  <Minimize className="size-4" />
                  {strings.multiview.exitFocus}
                  <kbd className="ml-1 rounded bg-white/15 px-1.5 text-[10.5px]">{strings.keys.esc}</kbd>
                </Button>
              </motion.div>
            )}
          </AnimatePresence>

          {grid.width > 0 && (
            <DndContext
              sensors={sensors}
              collisionDetection={closestCenter}
              onDragStart={onDragStart}
              onDragEnd={onDragEnd}
              onDragCancel={() => setDraggingId(null)}
              accessibility={{
                screenReaderInstructions: { draggable: strings.multiview.dnd.instructions },
                announcements: {
                  onDragStart: ({ active }) => strings.multiview.dnd.pickUp(nameOf(active.id)),
                  onDragOver: ({ active, over }) =>
                    over ? strings.multiview.dnd.over(nameOf(active.id), positionOf(over.id)) : undefined,
                  onDragEnd: ({ active, over }) =>
                    over ? strings.multiview.dnd.drop(nameOf(active.id), positionOf(over.id)) : undefined,
                  onDragCancel: ({ active }) => strings.multiview.dnd.cancel(nameOf(active.id)),
                },
              }}
            >
              <SortableContext items={ids} strategy={rectSortingStrategy}>
                <motion.div
                  key={`${focusId ?? layout}-${current}`}
                  initial={{ opacity: 0, scale: 0.985 }}
                  animate={{ opacity: 1, scale: 1, transition: transitions.slow }}
                  className="grid"
                  style={{
                    width: grid.width,
                    height: grid.height,
                    gap: GAP,
                    gridTemplateColumns: `repeat(${shownSpec.cols}, minmax(0, 1fr))`,
                    gridTemplateRows: `repeat(${shownSpec.rows}, minmax(0, 1fr))`,
                  }}
                >
                  {Array.from({ length: shownSpec.tiles }, (_, i) => {
                    const id = ids[i];
                    const camera = id ? byId.get(id) : undefined;
                    const feature = shownSpec.feature === i;
                    const span = feature ? { gridColumn: "span 2", gridRow: "span 2" } : undefined;
                    if (!camera) {
                      return (
                        <EmptySlot key={`empty-${i}`} style={span} compact={shownSpec.tiles >= 9} onAdd={() => openAdd(true)} />
                      );
                    }
                    return (
                      <SortableTile
                        key={camera.id}
                        camera={camera}
                        quality={focusId ? "hd" : tileQuality(shownSpec, i)}
                        muted={audioId !== camera.id}
                        onToggleMute={toggleMute}
                        focused={focusId === camera.id}
                        onToggleFocus={toggleFocus}
                        compact={!feature && shownSpec.tiles >= 9}
                        style={span}
                        dragDisabled={Boolean(focusId)}
                        isDraggingThis={draggingId === camera.id}
                      />
                    );
                  })}
                </motion.div>
              </SortableContext>
            </DndContext>
          )}
        </div>
      )}
    </div>
  );
}

function SortableTile({
  camera,
  quality,
  muted,
  onToggleMute,
  focused,
  onToggleFocus,
  compact,
  style,
  dragDisabled,
  isDraggingThis,
}: {
  camera: Camera;
  quality: "hd" | "sd";
  muted: boolean;
  onToggleMute: (id: string) => void;
  focused: boolean;
  onToggleFocus: (id: string) => void;
  compact: boolean;
  style?: CSSProperties;
  dragDisabled: boolean;
  isDraggingThis: boolean;
}) {
  const { attributes, listeners, setNodeRef, setActivatorNodeRef, transform, transition } = useSortable({
    id: camera.id,
    disabled: dragDisabled,
  });
  const onPointerDown = listeners?.onPointerDown as PointerEventHandler | undefined;
  const onKeyDown = listeners?.onKeyDown as KeyboardEventHandler | undefined;
  return (
    <Tile
      nodeRef={setNodeRef}
      handleRef={setActivatorNodeRef}
      camera={camera}
      quality={quality}
      muted={muted}
      onToggleMute={() => onToggleMute(camera.id)}
      focused={focused}
      onToggleFocus={() => onToggleFocus(camera.id)}
      compact={compact}
      dragging={isDraggingThis}
      onPointerDown={onPointerDown}
      handleProps={{ ...attributes, onKeyDown }}
      style={{ ...style, transform: CSS.Translate.toString(transform), transition }}
    />
  );
}

function EmptySlot({ style, compact, onAdd }: { style?: CSSProperties; compact: boolean; onAdd: () => void }) {
  return (
    <div style={style} className="grid place-items-center rounded-[6px] border border-dashed border-white/10 bg-white/[0.02]">
      <button
        type="button"
        onClick={onAdd}
        className="flex flex-col items-center gap-1.5 rounded-xl p-3 text-white/35 transition-colors hover:text-white/70"
      >
        <span className={cn("grid place-items-center rounded-full border border-white/15", compact ? "size-7" : "size-9")}>
          <Plus className={compact ? "size-3.5" : "size-4"} />
        </span>
        {!compact && <span className="text-xs font-medium">{strings.multiview.emptySlot}</span>}
      </button>
    </div>
  );
}
