// Generated SVG "camera frames" for the mock: a stylised scene per camera, with a subject and a
// detection box for event thumbnails. Night-time frames get an infrared look, like real cameras.
// Returned as data: URLs, which the Tauri CSP allows for images.

import type { EventType } from "../api";
import type { SceneKind } from "./fixtures";
import { Rng } from "./prng";

const W = 320;
const H = 180;

/** Event colours baked into images (CSS variables don't reach inside data: URLs). */
const EVENT_HEX: Record<EventType, string> = {
  motion: "#ee9500",
  person: "#1faa59",
  vehicle: "#7c4dff",
  pet: "#e0457b",
  baby_cry: "#12a594",
  sound: "#0e9aa7",
  line_crossing: "#f06a1d",
  area_intrusion: "#e5482f",
  tamper: "#64748b",
  doorbell: "#0b8bd4",
  other: "#8a93a3",
};

interface Scene {
  defs: string;
  body: string;
  /** Where subjects stand: y range of the ground, and x range. */
  ground: { yMin: number; yMax: number; xMin: number; xMax: number };
  /** Scale of a subject standing at the far and near edge of the ground. */
  scale: [number, number];
}

const vGrad = (id: string, a: string, b: string) =>
  `<linearGradient id='${id}' x1='0' y1='0' x2='0' y2='1'><stop offset='0' stop-color='${a}'/><stop offset='1' stop-color='${b}'/></linearGradient>`;

function porch(): Scene {
  return {
    defs: vGrad("wall", "#d9cdbd", "#b9a791") + vGrad("floor", "#958676", "#7b6d60"),
    body:
      `<rect width='${W}' height='${H}' fill='url(#wall)'/>` +
      `<path d='M0 22H320M0 44H320M0 66H320M0 88H320M0 110H320M0 132H320' stroke='#a08f78' stroke-opacity='.35'/>` +
      `<rect x='122' y='24' width='78' height='128' rx='2' fill='#efe8dc'/>` +
      `<rect x='129' y='31' width='64' height='121' fill='#45516b'/>` +
      `<g fill='none' stroke='#37415a' stroke-width='2'><rect x='137' y='41' width='20' height='44' rx='1.5'/><rect x='165' y='41' width='20' height='44' rx='1.5'/><rect x='137' y='95' width='20' height='46' rx='1.5'/><rect x='165' y='95' width='20' height='46' rx='1.5'/></g>` +
      `<circle cx='185' cy='96' r='2.6' fill='#dcb45f'/>` +
      `<rect x='214' y='48' width='11' height='17' rx='3' fill='#2e3037'/><circle cx='219.5' cy='57' r='3.2' fill='#ffe7a8'/>` +
      `<path d='M0 150H320V180H0Z' fill='url(#floor)'/><path d='M0 150H320' stroke='#6c6053' stroke-width='2'/>` +
      `<rect x='126' y='156' width='70' height='14' rx='2' fill='#5c4a3c'/>` +
      `<path d='M62 150l6-25h26l6 25z' fill='#6b4f3a'/>` +
      `<g fill='#4f7a45'><ellipse cx='81' cy='110' rx='17' ry='15'/><ellipse cx='69' cy='119' rx='11' ry='9'/><ellipse cx='94' cy='120' rx='11' ry='9'/></g>` +
      `<path d='M254 150l5-20h22l5 20z' fill='#6b4f3a'/><g fill='#5b8650'><ellipse cx='270' cy='120' rx='14' ry='13'/></g>`,
    ground: { yMin: 146, yMax: 170, xMin: 40, xMax: 290 },
    scale: [0.95, 1.2],
  };
}

function driveway(): Scene {
  return {
    defs:
      vGrad("sky", "#9cc1e5", "#e3edf4") +
      vGrad("road", "#a7a7a2", "#86867f") +
      vGrad("lawn", "#8db06a", "#6e9450"),
    body:
      `<rect width='${W}' height='84' fill='url(#sky)'/>` +
      `<path d='M0 74Q18 50 38 64Q56 40 82 60Q100 44 122 62Q140 50 162 66V86H0Z' fill='#55714e'/>` +
      `<path d='M192 84L258 40L324 84V124H192Z' fill='#d2d6db'/><path d='M184 86L258 34L332 86' stroke='#6d5a50' stroke-width='8' fill='none'/>` +
      `<rect x='212' y='90' width='80' height='34' fill='#eceef1' stroke='#b4b9c0'/><path d='M212 98.5H292M212 107H292M212 115.5H292' stroke='#c3c8ce'/>` +
      `<rect y='84' width='${W}' height='96' fill='url(#lawn)'/>` +
      `<path d='M64 180L284 180L238 96L150 96Z' fill='url(#road)'/>` +
      `<path d='M150 96L64 180M238 96L284 180' stroke='#c9c8c1' stroke-width='2'/>` +
      `<g fill='#4a6a44'><circle cx='24' cy='118' r='14'/><circle cx='40' cy='124' r='10'/><circle cx='300' cy='150' r='12'/></g>`,
    ground: { yMin: 112, yMax: 172, xMin: 90, xMax: 262 },
    scale: [0.55, 1.15],
  };
}

function living(): Scene {
  return {
    defs:
      vGrad("wall", "#ece2d6", "#dccfbf") +
      vGrad("floor", "#b98f69", "#9d7757") +
      vGrad("glass", "#dce8f4", "#f4f7fa"),
    body:
      `<rect width='${W}' height='${H}' fill='url(#wall)'/>` +
      `<rect x='196' y='22' width='94' height='72' rx='3' fill='#f7f2e6'/><rect x='201' y='27' width='84' height='62' fill='url(#glass)'/><path d='M243 27V89M201 58H285' stroke='#f7f2e6' stroke-width='3'/>` +
      `<rect x='186' y='16' width='13' height='90' rx='2' fill='#c8a47f'/><rect x='287' y='16' width='13' height='90' rx='2' fill='#c8a47f'/>` +
      `<path d='M0 128H320V180H0Z' fill='url(#floor)'/><path d='M0 142H320M0 158H320' stroke='#8d6a4c' stroke-opacity='.45'/>` +
      `<rect x='30' y='82' width='146' height='32' rx='12' fill='#6b7fa0'/><rect x='38' y='100' width='130' height='36' rx='9' fill='#5d7191'/>` +
      `<rect x='24' y='96' width='20' height='44' rx='8' fill='#566a89'/><rect x='162' y='96' width='20' height='44' rx='8' fill='#566a89'/>` +
      `<rect x='54' y='90' width='34' height='22' rx='6' fill='#e7d7b8'/><rect x='118' y='90' width='34' height='22' rx='6' fill='#d9a07c'/>` +
      `<ellipse cx='124' cy='162' rx='92' ry='12' fill='#cbb59b'/>` +
      `<path d='M226 150V104' stroke='#3a3a3f' stroke-width='3'/><path d='M212 104h28l-6-20h-16z' fill='#f1e3c4'/><ellipse cx='226' cy='151' rx='11' ry='3' fill='#3a3a3f'/>`,
    ground: { yMin: 140, yMax: 172, xMin: 60, xMax: 300 },
    scale: [1, 1.25],
  };
}

function nursery(): Scene {
  const bars = Array.from({ length: 9 }, (_, i) => `M${106 + i * 14} 90V146`).join("");
  return {
    defs: vGrad("wall", "#ebe7f4", "#dcd5ea") + vGrad("floor", "#dccbb7", "#c9b59d"),
    body:
      `<rect width='${W}' height='${H}' fill='url(#wall)'/>` +
      `<g fill='#f1d488'><path d='M40 30l3 6 6 1-4.5 4 1 6-5.5-3-5.5 3 1-6-4.5-4 6-1z'/><path d='M272 44l2.4 4.8 4.8.8-3.6 3.2.8 4.8-4.4-2.4-4.4 2.4.8-4.8-3.6-3.2 4.8-.8z'/><circle cx='70' cy='60' r='2'/><circle cx='250' cy='24' r='2'/></g>` +
      `<path d='M0 146H320V180H0Z' fill='url(#floor)'/>` +
      `<ellipse cx='160' cy='164' rx='88' ry='10' fill='#eadbe8'/>` +
      `<path d='M160 0V34' stroke='#b5acc6' stroke-width='1.5'/><path d='M136 34H184' stroke='#b5acc6' stroke-width='2'/><circle cx='138' cy='44' r='5' fill='#f5b8c4'/><circle cx='160' cy='48' r='5' fill='#a8d5e5'/><circle cx='182' cy='44' r='5' fill='#f6dd97'/>` +
      `<rect x='92' y='122' width='136' height='22' rx='3' fill='#ffffff'/>` +
      `<path d='${bars}' stroke='#f7f3ea' stroke-width='3.2' stroke-linecap='round'/>` +
      `<rect x='92' y='84' width='136' height='64' rx='5' fill='none' stroke='#f7f3ea' stroke-width='5'/>`,
    ground: { yMin: 150, yMax: 170, xMin: 30, xMax: 290 },
    scale: [1.05, 1.2],
  };
}

function garage(): Scene {
  return {
    defs: vGrad("wall", "#bcc0c6", "#a6abb2") + vGrad("floor", "#8e9297", "#72767c"),
    body:
      `<rect width='${W}' height='${H}' fill='url(#wall)'/>` +
      `<rect x='118' y='6' width='48' height='6' rx='3' fill='#fffbe6'/>` +
      `<rect x='174' y='26' width='132' height='112' fill='#dadde2'/><path d='M174 48H306M174 70H306M174 92H306M174 114H306' stroke='#bfc4ca' stroke-width='2'/>` +
      `<path d='M0 138H320V180H0Z' fill='url(#floor)'/><ellipse cx='212' cy='160' rx='26' ry='5' fill='#5f6368' opacity='.5'/>` +
      `<path d='M16 40V138M96 40V138' stroke='#6e5a44' stroke-width='4'/><path d='M14 62H98M14 94H98M14 126H98' stroke='#7c664e' stroke-width='4'/>` +
      `<rect x='24' y='44' width='26' height='18' fill='#c99b63'/><rect x='56' y='48' width='30' height='14' fill='#7e90aa'/><rect x='28' y='76' width='22' height='18' fill='#d0a36c'/><rect x='60' y='80' width='24' height='14' fill='#b86a55'/><rect x='24' y='110' width='34' height='16' fill='#8c9aa9'/>` +
      `<path d='M0 158Q60 136 146 140L154 180H0Z' fill='#3c4656'/><path d='M22 150Q70 138 132 142' stroke='#566276' stroke-width='3' fill='none'/>`,
    ground: { yMin: 144, yMax: 168, xMin: 150, xMax: 300 },
    scale: [0.95, 1.15],
  };
}

function yard(): Scene {
  const planks = Array.from({ length: 20 }, (_, i) => `M${i * 16 + 8} 60V116`).join("");
  return {
    defs: vGrad("sky", "#a6c9e8", "#e6eff5") + vGrad("grass", "#8ab466", "#6b9449"),
    body:
      `<rect width='${W}' height='64' fill='url(#sky)'/>` +
      `<rect y='60' width='${W}' height='56' fill='#a87c57'/><path d='${planks}' stroke='#8a6444' stroke-width='1.5'/>` +
      `<rect y='116' width='${W}' height='64' fill='url(#grass)'/>` +
      `<rect x='242' y='54' width='10' height='70' fill='#6d5038'/><g fill='#4f7a45'><circle cx='247' cy='44' r='26'/><circle cx='226' cy='58' r='16'/><circle cx='268' cy='58' r='16'/></g>` +
      `<ellipse cx='96' cy='146' rx='34' ry='8' fill='#e8e2d6'/><path d='M80 150V170M112 150V170' stroke='#8d8578' stroke-width='3'/>`,
    ground: { yMin: 124, yMax: 172, xMin: 30, xMax: 300 },
    scale: [0.75, 1.2],
  };
}

function gate(): Scene {
  const bricks = Array.from({ length: 8 }, (_, r) => `M0 ${r * 14 + 14}H320`).join("");
  const bars = Array.from({ length: 7 }, (_, i) => `M${134 + i * 9} 64V146`).join("");
  return {
    defs: vGrad("wall", "#be8e71", "#a4765b") + vGrad("path", "#aba69c", "#8f8a80"),
    body:
      `<rect width='${W}' height='${H}' fill='url(#wall)'/><path d='${bricks}' stroke='#8f6650' stroke-opacity='.5'/>` +
      `<rect x='124' y='56' width='72' height='92' fill='#394049' opacity='.35'/>` +
      `<path d='M0 146H320V180H0Z' fill='url(#path)'/><path d='M40 160H120M170 168H280M90 174H150' stroke='#7f7a72' stroke-opacity='.6'/>` +
      `<rect x='128' y='60' width='64' height='88' fill='none' stroke='#2f3440' stroke-width='4'/><path d='${bars}' stroke='#2f3440' stroke-width='2.5'/>` +
      `<g fill='#4c6d44'><ellipse cx='56' cy='128' rx='60' ry='26'/><ellipse cx='268' cy='130' rx='62' ry='26'/></g>`,
    ground: { yMin: 148, yMax: 172, xMin: 40, xMax: 290 },
    scale: [0.95, 1.2],
  };
}

const SCENES: Record<SceneKind, () => Scene> = {
  porch,
  driveway,
  living,
  nursery,
  garage,
  yard,
  gate,
};

// --- Subjects -------------------------------------------------------------------------------

function person(x: number, y: number, s: number, fill = "#2b3140"): string {
  return (
    `<g transform='translate(${x.toFixed(1)} ${y.toFixed(1)}) scale(${s.toFixed(2)})' fill='${fill}'>` +
    `<circle cx='0' cy='-57' r='7'/>` +
    `<path d='M-11 -47Q0 -51 11 -47L13 -20Q0 -16 -13 -20Z'/>` +
    `<path d='M-9 -21L-8 0H-2L0 -12L2 0H8L9 -21Z'/>` +
    `<path d='M-11 -45L-16 -24M11 -45L16 -24' stroke='${fill}' stroke-width='5' stroke-linecap='round'/></g>`
  );
}

function vehicle(x: number, y: number, s: number, fill: string): string {
  return (
    `<g transform='translate(${x.toFixed(1)} ${y.toFixed(1)}) scale(${s.toFixed(2)})'>` +
    `<path d='M-50 -6Q-52 -18 -40 -20L-26 -22L-15 -34Q-9 -38 12 -38L25 -36L39 -22L47 -20Q54 -16 52 -6Z' fill='${fill}'/>` +
    `<path d='M-12 -32H8L20 -23H-22Z' fill='#cfe0ee' opacity='.85'/>` +
    `<circle cx='-28' cy='-5' r='8' fill='#1d1f24'/><circle cx='31' cy='-5' r='8' fill='#1d1f24'/>` +
    `<circle cx='-28' cy='-5' r='3.5' fill='#8b9099'/><circle cx='31' cy='-5' r='3.5' fill='#8b9099'/></g>`
  );
}

function pet(x: number, y: number, s: number, fill: string): string {
  return (
    `<g transform='translate(${x.toFixed(1)} ${y.toFixed(1)}) scale(${s.toFixed(2)})' fill='${fill}'>` +
    `<ellipse cx='0' cy='-9' rx='13' ry='10'/><circle cx='-12' cy='-22' r='7'/>` +
    `<path d='M-18 -27l2-8 4 5zM-9 -28l3-7 2 7z'/>` +
    `<path d='M12 -8Q24 -12 20 -26' stroke='${fill}' stroke-width='3.5' fill='none' stroke-linecap='round'/></g>`
  );
}

function baby(): string {
  return `<ellipse cx='168' cy='124' rx='22' ry='8' fill='#9fc3e8'/><circle cx='143' cy='121' r='7' fill='#f3d4bd'/>`;
}

function waves(x: number, y: number, color: string): string {
  return (
    `<g fill='none' stroke='${color}' stroke-width='3' stroke-linecap='round' opacity='.9'>` +
    `<path d='M${x} ${y - 8}q6 8 0 16'/><path d='M${x + 8} ${y - 14}q10 14 0 28'/><path d='M${x + 16} ${y - 20}q14 20 0 40'/></g>`
  );
}

function box(x: number, y: number, w: number, h: number, color: string): string {
  return (
    `<rect x='${x.toFixed(1)}' y='${y.toFixed(1)}' width='${w.toFixed(1)}' height='${h.toFixed(1)}' rx='3' fill='none' stroke='${color}' stroke-width='2'/>` +
    `<rect x='${x.toFixed(1)}' y='${(y - 7).toFixed(1)}' width='${Math.min(w, 30).toFixed(1)}' height='7' rx='2' fill='${color}'/>`
  );
}

const NIGHT_DEFS =
  `<filter id='ir' color-interpolation-filters='sRGB'><feColorMatrix type='matrix' values='0.28 0.52 0.1 0 0.02 0.28 0.52 0.1 0 0.04 0.28 0.52 0.1 0 0.03 0 0 0 1 0'/></filter>` +
  `<radialGradient id='irv' cx='50%' cy='48%' r='70%'><stop offset='35%' stop-color='#000' stop-opacity='0'/><stop offset='100%' stop-color='#000' stop-opacity='.72'/></radialGradient>`;
const DAY_DEFS = `<radialGradient id='vig' cx='50%' cy='46%' r='75%'><stop offset='62%' stop-color='#000' stop-opacity='0'/><stop offset='100%' stop-color='#000' stop-opacity='.32'/></radialGradient>`;
const TAMPER_DEFS = `<filter id='blur' x='-50%' y='-50%' width='200%' height='200%'><feGaussianBlur stdDeviation='14'/></filter>`;

/**
 * Percent-encodes everything that could break a consumer, including characters that
 * `encodeURIComponent` leaves alone but an unquoted CSS `url(...)` rejects: ' ( ) and spaces.
 */
function toDataUrl(svg: string): string {
  const body = encodeURIComponent(svg)
    .replace(/'/g, "%27")
    .replace(/\(/g, "%28")
    .replace(/\)/g, "%29")
    .replace(/%3D/g, "=")
    .replace(/%3A/g, ":")
    .replace(/%2F/g, "/");
  return `data:image/svg+xml,${body}`;
}

/** `content` is the scene as the lens sees it (IR-filtered at night); `overlay` is drawn on top. */
function frame(sceneDefs: string, content: string, overlay: string, night: boolean, extraDefs = ""): string {
  const defs = sceneDefs + (night ? NIGHT_DEFS : DAY_DEFS) + extraDefs;
  const shaded = night
    ? `<g filter='url(#ir)'>${content}</g><rect width='${W}' height='${H}' fill='url(#irv)'/>`
    : `${content}<rect width='${W}' height='${H}' fill='url(#vig)'/>`;
  return `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 ${W} ${H}' preserveAspectRatio='xMidYMid slice'><defs>${defs}</defs>${shaded}${overlay}</svg>`;
}

/** Is this camera-local hour dark enough for night vision? */
export function isNightHour(hour: number): boolean {
  return hour < 6 || hour >= 20;
}

/** The camera's latest frame: its scene with nobody in it. */
export function snapshotDataUrl(scene: SceneKind, night: boolean): string {
  const s = SCENES[scene]();
  let extra = "";
  if (scene === "driveway") extra = vehicle(214, 150, 1.05, "#6b7a8f");
  return toDataUrl(frame(s.defs, s.body + extra, "", night));
}

/** An event thumbnail: the scene, the subject, and a detection box in the event colour. */
export function eventThumbnailDataUrl(
  scene: SceneKind,
  type: EventType,
  night: boolean,
  seed: string,
): string {
  const rng = new Rng(seed);
  const s = SCENES[scene]();
  const { ground, scale } = s;
  const gy = rng.range(ground.yMin, ground.yMax);
  const k = scale[0] + ((gy - ground.yMin) / Math.max(1, ground.yMax - ground.yMin)) * (scale[1] - scale[0]);
  const gx = rng.range(ground.xMin, ground.xMax);
  const color = EVENT_HEX[type];
  const clothes = rng.pick(["#2b3140", "#3d4b63", "#5a3d3d", "#394a3c", "#4a4152"]);
  let subject = "";
  let overlay = "";
  let extraDefs = "";

  switch (type) {
    case "person":
    case "doorbell": {
      const kk = type === "doorbell" ? k * 1.5 : k;
      subject = person(gx, gy, kk, clothes);
      overlay = box(gx - 20 * kk, gy - 68 * kk, 40 * kk, 70 * kk, color);
      break;
    }
    case "vehicle": {
      const car = rng.pick(["#b23a3a", "#2f5d9e", "#d9dde2", "#2c2f36", "#6b7a8f"]);
      subject = vehicle(gx, gy, k, car);
      overlay = box(gx - 56 * k, gy - 44 * k, 112 * k, 46 * k, color);
      break;
    }
    case "pet": {
      const fur = rng.pick(["#3a3330", "#c07a3e", "#8c8c8c", "#e8e1d6"]);
      subject = pet(gx, gy, k * 1.1, fur);
      overlay = box(gx - 26 * k, gy - 38 * k, 50 * k, 40 * k, color);
      break;
    }
    case "baby_cry":
      subject = baby();
      overlay = waves(200, 108, "#ffffff") + box(128, 108, 70, 26, color);
      break;
    case "sound":
      overlay = waves(rng.range(60, 220), rng.range(50, 90), "#ffffff") + box(10, 10, 300, 160, color);
      break;
    case "line_crossing": {
      const y1 = ground.yMin + 4;
      const y2 = ground.yMax - 6;
      subject = person(gx, gy, k, clothes);
      overlay =
        `<path d='M24 ${y2.toFixed(0)}L296 ${y1.toFixed(0)}' stroke='${color}' stroke-width='2.5' stroke-dasharray='9 6'/>` +
        box(gx - 20 * k, gy - 68 * k, 40 * k, 70 * k, color);
      break;
    }
    case "area_intrusion": {
      const zx = ground.xMin + 10;
      const zw = (ground.xMax - ground.xMin) * 0.6;
      subject = person(zx + zw / 2, gy, k, clothes);
      overlay =
        `<path d='M${zx} ${ground.yMax + 4}L${zx + zw} ${ground.yMax + 4}L${zx + zw - 20} ${ground.yMin - 10}L${zx + 16} ${ground.yMin - 10}Z' fill='${color}' fill-opacity='.14' stroke='${color}' stroke-width='2' stroke-dasharray='6 5'/>` +
        box(zx + zw / 2 - 20 * k, gy - 68 * k, 40 * k, 70 * k, color);
      break;
    }
    case "tamper":
      extraDefs = TAMPER_DEFS;
      subject = `<ellipse cx='${rng.range(110, 210).toFixed(0)}' cy='90' rx='140' ry='90' fill='#15171b' opacity='.88' filter='url(#blur)'/>`;
      overlay = box(12, 12, 296, 156, color);
      break;
    case "motion":
    case "other": {
      const ghost = person(gx - 10 * k, gy, k, clothes).replace("<g ", "<g opacity='.25' ");
      subject =
        ghost +
        person(gx, gy, k, clothes) +
        `<path d='M${(gx - 34 * k).toFixed(0)} ${(gy - 40 * k).toFixed(0)}h14M${(gx - 38 * k).toFixed(0)} ${(gy - 28 * k).toFixed(0)}h18M${(gx - 34 * k).toFixed(0)} ${(gy - 16 * k).toFixed(0)}h12' stroke='#fff' stroke-width='2' stroke-linecap='round' opacity='.7'/>`;
      overlay = box(gx - 32 * k, gy - 68 * k, 56 * k, 70 * k, color);
      break;
    }
  }

  return toDataUrl(frame(s.defs, s.body + subject, overlay, night, extraDefs));
}
