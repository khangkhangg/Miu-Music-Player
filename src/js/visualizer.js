// Visualizer drawing (F7.6): turn band levels into shapes for a canvas. Styles: bars, mirrored, line.

export function fromBytes(bytes) {
  return Array.from(bytes, b => b / 255);
}

/**
 * Shapes for `levels` (0..1) in a `width`×`height` area with `gap` px between bands.
 * Returns `{ rects }` for bars / mirrored, `{ points }` for line. Unknown styles draw bars.
 */
export function geometry(style, levels, width, height, gap = 2) {
  const n = levels.length;
  const bw = (width - gap * (n - 1)) / n;
  if (style === 'line') {
    return { points: levels.map((l, i) => ({ x: i * (bw + gap) + bw / 2, y: height - l * height })) };
  }
  const mirrored = style === 'mirrored';
  return {
    rects: levels.map((l, i) => {
      const h = Math.max(1, l * height); // a hairline even when silent
      return { x: i * (bw + gap), y: mirrored ? (height - h) / 2 : height - h, w: bw, h };
    }),
  };
}
