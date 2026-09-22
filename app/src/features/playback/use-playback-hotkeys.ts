import { useEffect, useRef } from "react";
import { keyToAction, type TimelineAction } from "@/features/timeline/math";

/** Widgets that own their arrow keys (and Space), so shortcuts stay out of their way. */
const COMPOSITE_WIDGETS =
  "[role=dialog],[role=alertdialog],[role=menu],[role=listbox],[role=grid],[role=tablist],[role=radiogroup],[role=toolbar],[role=slider],[data-radix-collection-item],[data-hotkeys-ignore]";

export function shouldIgnoreHotkey(target: EventTarget | null, action: TimelineAction): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return true;
  // The playback timeline is a slider, but these shortcuts are exactly its keys.
  if (target.closest("[data-playback-timeline]")) return false;
  if (target.closest(COMPOSITE_WIDGETS)) return true;
  // Space activates focused buttons and links natively.
  if (action.type === "toggle-play" && target.closest("button,a,[role=button]")) return true;
  return false;
}

/** Playback shortcuts on the whole page while the Playback tab is open. */
export function usePlaybackHotkeys(enabled: boolean, onAction: (action: TimelineAction) => void) {
  const handler = useRef(onAction);
  useEffect(() => {
    handler.current = onAction;
  });
  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const action = keyToAction(e);
      if (!action || shouldIgnoreHotkey(e.target, action)) return;
      if (action.type === "toggle-play" && e.repeat) return;
      e.preventDefault();
      handler.current(action);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled]);
}
