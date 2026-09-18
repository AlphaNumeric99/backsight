import { useLayoutEffect, useState, type RefObject } from "react";

export interface Size {
  width: number;
  height: number;
}

/** Tracks an element's content-box size. */
export function useElementSize<T extends HTMLElement>(ref: RefObject<T | null>): Size {
  const [size, setSize] = useState<Size>({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => {
      const rect = el.getBoundingClientRect();
      setSize((prev) =>
        Math.abs(prev.width - rect.width) < 0.5 && Math.abs(prev.height - rect.height) < 0.5
          ? prev
          : { width: rect.width, height: rect.height },
      );
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return size;
}
