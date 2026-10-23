// DJ waveforms: the whole song coloured by sound (bass blue, mids amber, highs white) with its
// sections, phrases, hot cues, loop and the coming mix. The engine sends band levels
// (`dj_overview`); drawing is plain canvas.

const BASS = [59, 111, 224];
const MIDS = [224, 169, 59];
const HIGHS = [242, 242, 242];
const SILENCE = [90, 90, 100];
const PHRASE_BARS = 8;

export const SECTION_COLOURS = {
  intro: '#4a4a58',
  main: '#5e6a86',
  build: '#b0813a',
  drop: '#d64f6b',
  breakdown: '#7d5bd6',
  outro: '#4a4a58',
};
export const CUE_COLOURS = ['#e0453b', '#3bb36b', '#e0a93b', '#3b8fe0'];
const MIX = 'rgba(43, 179, 168, 0.28)';
const MIX_EDGE = '#2bb3a8';

const round = (v, places = 6) => Math.round(v * 10 ** places) / 10 ** places;

/** The colour of [bass, mids, highs] levels: each band's colour by its share of the power, so the
 * band that dominates shows clearly (shares of the level alone blend most music into one beige). */
export function bandColour([l, m, h]) {
  const [pl, pm, ph] = [l * l, m * m, h * h];
  const sum = pl + pm + ph;
  if (sum <= 0) return `rgb(${SILENCE.join(',')})`;
  const c = [0, 1, 2].map((k) => Math.round((BASS[k] * pl + MIDS[k] * pm + HIGHS[k] * ph) / sum));
  return `rgb(${c.join(',')})`;
}

/** How tall the waveform is for these levels, 0..1. */
export function bandHeight([l, m, h]) {
  return Math.min(1, Math.sqrt(l * l + m * m + h * h) / 255);
}

/** The song's sections as parts of it (0..1). */
export function sectionSpans(analysis, durationS) {
  const sections = analysis?.structure?.sections ?? [];
  if (!durationS) return [];
  const f = (s) => round(Math.min(1, Math.max(0, s / durationS)));
  return sections.map((s) => ({ kind: s.kind, from: f(s.startS), to: f(s.endS) })).filter((s) => s.to > s.from);
}

/** Where each phrase (8 bars) starts, seconds, across the whole song. */
export function phraseTimes(analysis, durationS) {
  const start = analysis?.structure?.phraseStartS;
  if (!analysis?.bpm || start == null || !durationS) return [];
  const len = (PHRASE_BARS * 4 * 60) / analysis.bpm;
  let t = start - Math.floor(start / len + 1e-9) * len;
  const out = [];
  for (; t < durationS; t += len) out.push(round(t));
  return out;
}

/** Where Auto DJ's coming mix sits on the song on air (0..1). */
export function mixZone(next, durationS) {
  if (!next || !durationS) return null;
  return { from: round(Math.min(1, next.atS / durationS)), to: round(Math.min(1, (next.atS + next.lengthS) / durationS)) };
}

/** Hot cues and the loop as places in the song (0..1). */
export function cueMarks(d) {
  if (!d?.durationS) return { cues: [], loop: null };
  const f = (s) => round(s / d.durationS);
  const cues = (d.hotCues ?? []).flatMap((c, slot) => (c == null ? [] : [{ slot, at: f(c) }]));
  return { cues, loop: d.loopS ? { from: f(d.loopS[0]), to: f(d.loopS[1]) } : null };
}

function fit(canvas) {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
  }
  const g = canvas.getContext('2d');
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, w, h);
  return { g, w, h };
}

const STRIP = 12;

/**
 * The whole song: section strip on top (labelled when there is room), phrase lines, the
 * waveform coloured by sound, the loop, the coming mix and the hot cues.
 */
export function drawOverview(canvas, { bands, sections, phrases, durationS, mix, cues, loop }, { label, mixLabel }) {
  const { g, w, h } = fit(canvas);
  g.font = '10px system-ui, sans-serif'; // i18n-ignore
  g.textBaseline = 'middle';
  for (const s of sections) {
    const x = s.from * w;
    const sw = (s.to - s.from) * w;
    g.fillStyle = SECTION_COLOURS[s.kind] ?? SECTION_COLOURS.main;
    g.fillRect(x + 0.5, 0, Math.max(1, sw - 1), STRIP - 2);
    const text = label(s.kind);
    if (text && g.measureText(text).width + 8 < sw) {
      g.fillStyle = '#fff';
      g.fillText(text, x + 4, (STRIP - 2) / 2 + 0.5);
    }
  }
  const top = sections.length ? STRIP : 0;
  const waveH = h - top;
  if (mix) {
    g.fillStyle = MIX;
    g.fillRect(mix.from * w, top, Math.max(2, (mix.to - mix.from) * w), waveH);
  }
  if (loop) {
    g.fillStyle = 'rgba(255, 255, 255, 0.14)';
    g.fillRect(loop.from * w, top, Math.max(2, (loop.to - loop.from) * w), waveH);
  }
  g.fillStyle = 'rgba(255, 255, 255, 0.13)';
  for (const p of phrases) g.fillRect(Math.round((p / durationS) * w), top, 1, waveH);
  const mid = top + waveH / 2;
  const n = bands?.length ?? 0;
  const bw = w / Math.max(1, n);
  for (let i = 0; i < n; i++) {
    const a = bandHeight(bands[i]) * (waveH / 2 - 1);
    if (a < 0.3) continue;
    g.fillStyle = bandColour(bands[i]);
    g.fillRect(i * bw, mid - a, bw + 0.6, a * 2);
  }
  if (mix) {
    g.fillStyle = MIX_EDGE;
    g.fillRect(mix.from * w, top, 2, waveH);
    g.fillStyle = '#fff';
    if (mixLabel && (1 - mix.from) * w > g.measureText(mixLabel).width + 8) {
      g.textBaseline = 'bottom';
      g.fillText(mixLabel, mix.from * w + 4, h - 1);
    }
  }
  g.textBaseline = 'middle';
  for (const c of cues) {
    const x = c.at * w;
    g.fillStyle = CUE_COLOURS[c.slot % CUE_COLOURS.length];
    g.fillRect(x, top, 2, waveH);
    g.fillRect(x, top, 11, 11);
    g.fillStyle = '#111';
    g.fillText(String(c.slot + 1), x + 3, top + 6);
  }
}
