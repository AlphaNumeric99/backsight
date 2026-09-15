export type Fit = "contain" | "cover";

/** Source (s*) and destination (d*) rectangles for `drawImage`. */
export interface DrawRect {
  sx: number;
  sy: number;
  sw: number;
  sh: number;
  dx: number;
  dy: number;
  dw: number;
  dh: number;
}

/**
 * Where to draw a `srcW`×`srcH` picture on a `dstW`×`dstH` canvas: `contain` letterboxes the
 * whole picture, `cover` fills the canvas and crops the picture's overflow evenly.
 */
export function fitRect(srcW: number, srcH: number, dstW: number, dstH: number, fit: Fit): DrawRect {
  if (srcW <= 0 || srcH <= 0 || dstW <= 0 || dstH <= 0) {
    return { sx: 0, sy: 0, sw: 0, sh: 0, dx: 0, dy: 0, dw: 0, dh: 0 };
  }
  if (fit === "cover") {
    const scale = Math.max(dstW / srcW, dstH / srcH);
    const sw = dstW / scale;
    const sh = dstH / scale;
    return { sx: (srcW - sw) / 2, sy: (srcH - sh) / 2, sw, sh, dx: 0, dy: 0, dw: dstW, dh: dstH };
  }
  const scale = Math.min(dstW / srcW, dstH / srcH);
  const dw = Math.round(srcW * scale);
  const dh = Math.round(srcH * scale);
  return {
    sx: 0,
    sy: 0,
    sw: srcW,
    sh: srcH,
    dx: Math.floor((dstW - dw) / 2),
    dy: Math.floor((dstH - dh) / 2),
    dw,
    dh,
  };
}
