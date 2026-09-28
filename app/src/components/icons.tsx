// Icons lucide doesn't have, drawn in the same 24px, 2px-stroke line style.
import type { SVGProps } from "react";
import type { MultiviewLayout } from "@/ipc";

type IconProps = SVGProps<SVGSVGElement> & { size?: number };

function base({ size = 24, ...props }: IconProps): SVGProps<SVGSVGElement> {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 2,
    strokeLinecap: "round",
    strokeLinejoin: "round",
    "aria-hidden": true,
    ...props,
  };
}

export function SdCardIcon(props: IconProps) {
  return (
    <svg {...base(props)}>
      <path d="M8 3h8.5L20 6.5V19a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7l4-4Z" />
      <path d="M9 7.5v3M12 7.5v3M15 7.5v3" />
    </svg>
  );
}

/** Tiny grid previews for the multi-view layout switcher. */
export function LayoutIcon({ layout, ...props }: IconProps & { layout: MultiviewLayout }) {
  const cells: [number, number, number, number][] = (() => {
    const grid = (n: number) => {
      const out: [number, number, number, number][] = [];
      const size = 18 / n;
      for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) out.push([3 + c * size, 3 + r * size, size, size]);
      return out;
    };
    switch (layout) {
      case "1":
        return [[3, 3, 18, 18]];
      case "2":
        return [
          [3, 6, 9, 12],
          [12, 6, 9, 12],
        ];
      case "4":
        return grid(2);
      case "1+5":
        return [
          [3, 3, 12, 12],
          [15, 3, 6, 6],
          [15, 9, 6, 6],
          [3, 15, 6, 6],
          [9, 15, 6, 6],
          [15, 15, 6, 6],
        ];
      case "9":
        return grid(3);
      case "16":
        return grid(4);
    }
  })();
  return (
    <svg {...base({ ...props, strokeWidth: 1.6 })}>
      {cells.map(([x, y, w, h], i) => (
        <rect key={i} x={x + 0.6} y={y + 0.6} width={w - 1.2} height={h - 1.2} rx={layout === "16" ? 0.6 : 1.2} />
      ))}
    </svg>
  );
}
