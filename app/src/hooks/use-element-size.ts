import { useCallback, useEffect, useState } from "react";

export interface Size {
  width: number;
  height: number;
}

/**
 * Tracks an element's size. Returns a callback ref, so it keeps working when the element is
 * mounted later or replaced.
 */
export function useElementSize<T extends HTMLElement>(): [(el: T | null) => void, Size] {
  const [el, setEl] = useState<T | null>(null);
  const [size, setSize] = useState<Size>({ width: 0, height: 0 });
  useEffect(() => {
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
  }, [el]);
  const ref = useCallback((node: T | null) => setEl(node), []);
  return [ref, size];
}
