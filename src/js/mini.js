// Mini player window (F7.4). Logic lives in the tested modules; this is DOM glue.
import { resolveTheme } from './theme.js';
import { nowPlaying, seekTarget } from './player.js';
import { nextRating, applyRated } from './ratings.js';
import { fromBytes, geometry } from './visualizer.js';
import { t, startInLanguage } from './i18n.js';

const media = window.matchMedia('(prefers-color-scheme: dark)');
let preference = 'dark'; // the theme setting (F5.5); dark until the setting loads
const applyTheme = () => { document.documentElement.dataset.theme = resolveTheme(preference, media.matches); };
applyTheme();
media.addEventListener('change', applyTheme);

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = id => document.getElementById(id);
await startInLanguage(invoke);
const useSettings = s => { preference = s.theme; applyTheme(); };
invoke('app_settings').then(useSettings);
listen('settings-changed', e => useSettings(e.payload));

let songsById = new Map();
let state = null;

function loadSongs() {
  invoke('list_songs').then(list => {
    songsById = new Map(list.map(s => [s.id, s]));
    show(state);
  });
}

let artFor = null;

/** The playing song's cover (from the miuart: protocol), or the note icon. */
function showArt(song) {
  if (artFor === (song?.id ?? null)) return;
  artFor = song?.id ?? null;
  const img = $('mini-art');
  const has = Boolean(song?.hasArtwork);
  img.hidden = !has;
  $('mini-art-empty').toggleAttribute('hidden', has);
  if (has) {
    img.onerror = () => { img.hidden = true; $('mini-art-empty').removeAttribute('hidden'); };
    img.src = window.__TAURI__.core.convertFileSrc(`${song.id}-240`, 'miuart');
  }
}

function show(event) {
  state = event;
  showArt(songsById.get(event?.songId));
  const np = nowPlaying(event, songsById);
  $('mini-title').textContent = np ? np.title : t('player.nothing');
  $('mini-artist').textContent = np ? np.artist : t('mini.hint');
  $('mini-fill').style.width = `${(np?.progress ?? 0) * 100}%`;
  $('mini-play').classList.toggle('paused', !np?.playing);
  $('mini-play').setAttribute('aria-label', t(np?.playing ? 'player.pause' : 'player.play'));
  const rating = songsById.get(event?.songId)?.rating ?? 0;
  $('mini-like').classList.toggle('on', rating === 1);
  $('mini-dislike').classList.toggle('on', rating === -1);
}

function rate(act) {
  const song = songsById.get(state?.songId);
  if (!song) return;
  const rating = nextRating(song.rating, act);
  invoke('rate_song', { id: song.id, rating }).then(() => {
    // Thumbs-down in the mini player means "not this one": skip right away.
    if (rating === -1) invoke('player_skip', { by: 1 });
  });
}

listen('player', e => show(e.payload));
listen('library-changed', loadSongs);
listen('song-rated', e => { if (applyRated(songsById, e.payload)) show(state); });

$('mini-play').addEventListener('click', () => invoke(state?.playing ? 'player_pause' : 'player_resume'));
$('mini-prev').addEventListener('click', () => invoke('player_skip', { by: -1 }));
$('mini-next').addEventListener('click', () => invoke('player_skip', { by: 1 }));
$('mini-like').addEventListener('click', () => rate('like'));
$('mini-dislike').addEventListener('click', () => rate('dislike'));
$('mini-bar').addEventListener('click', e => {
  const rect = e.currentTarget.getBoundingClientRect();
  const ms = seekTarget((e.clientX - rect.left) / rect.width, songsById.get(state?.songId)?.durationMs);
  if (ms != null) invoke('player_seek', { ms });
});

// Visualizer (F7.6): draw the latest frame; click the strip to cycle styles, last one turns it off.
const STYLES = ['bars', 'mirrored', 'line'];
let viz = { on: true, style: 'bars' };
let levels = new Array(28).fill(0);
let drawPending = false;

function draw() {
  drawPending = false;
  const canvas = $('mini-viz');
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  if (canvas.width !== Math.round(w * dpr)) { canvas.width = Math.round(w * dpr); canvas.height = Math.round(h * dpr); }
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const color = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#3b6fd8';
  const g = geometry(viz.style, levels, w, h, 2);
  ctx.fillStyle = color;
  ctx.strokeStyle = color;
  if (g.rects) {
    for (const r of g.rects) ctx.fillRect(r.x, r.y, r.w, r.h);
  } else {
    ctx.lineWidth = 2;
    ctx.beginPath();
    g.points.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)));
    ctx.stroke();
  }
}

function requestDraw() {
  if (!drawPending) { drawPending = true; requestAnimationFrame(draw); }
}

function showViz(settings) {
  viz = settings;
  $('mini-viz').hidden = !viz.on;
  $('viz-off').hidden = viz.on;
  requestDraw();
}

listen('spectrum', e => { levels = fromBytes(e.payload); requestDraw(); });
listen('visualizer-settings', e => showViz(e.payload));
listen('player', e => { if (!e.payload?.playing) { levels = levels.map(() => 0); requestDraw(); } });
invoke('visualizer_settings').then(showViz);
$('mini-viz').addEventListener('click', () => {
  const next = STYLES.indexOf(viz.style) + 1;
  if (next >= STYLES.length) invoke('set_visualizer_on', { on: false });
  else invoke('set_visualizer_style', { style: STYLES[next] });
});
$('viz-off').addEventListener('click', () => {
  invoke('set_visualizer_style', { style: 'bars' }).then(() => invoke('set_visualizer_on', { on: true }));
});

loadSongs();
