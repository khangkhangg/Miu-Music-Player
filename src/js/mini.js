// Mini player window (F7.4). Logic lives in the tested modules; this is DOM glue.
import { resolveTheme } from './theme.js';
import { nowPlaying, seekTarget, pinButton } from './player.js';
import { nextRating, applyRated } from './ratings.js';

const media = window.matchMedia('(prefers-color-scheme: dark)');
const applyTheme = () => { document.documentElement.dataset.theme = resolveTheme('system', media.matches); };
applyTheme();
media.addEventListener('change', applyTheme);

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = id => document.getElementById(id);

let songsById = new Map();
let state = null;
let onTop = true;

function showPin(on) {
  onTop = on;
  const p = pinButton(on);
  const b = $('mini-pin');
  b.classList.toggle('on', p.pressed);
  b.setAttribute('aria-pressed', String(p.pressed));
  b.setAttribute('aria-label', p.label);
  b.title = p.label;
}

function loadSongs() {
  invoke('list_songs').then(list => {
    songsById = new Map(list.map(s => [s.id, s]));
    show(state);
  });
}

function show(event) {
  state = event;
  const np = nowPlaying(event, songsById);
  $('mini-title').textContent = np ? np.title : 'Nothing playing';
  $('mini-artist').textContent = np ? np.artist : 'Double-click a song in Miu Music';
  $('mini-fill').style.width = `${(np?.progress ?? 0) * 100}%`;
  $('mini-play').classList.toggle('paused', !np?.playing);
  $('mini-play').setAttribute('aria-label', np?.playing ? 'Pause' : 'Play');
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
$('mini-pin').addEventListener('click', () => invoke('set_mini_on_top', { on: !onTop }));
listen('mini-on-top', e => showPin(e.payload));
invoke('mini_on_top').then(showPin);
$('mini-open').addEventListener('click', () => invoke('open_main'));
$('mini-bar').addEventListener('click', e => {
  const rect = e.currentTarget.getBoundingClientRect();
  const ms = seekTarget((e.clientX - rect.left) / rect.width, songsById.get(state?.songId)?.durationMs);
  if (ms != null) invoke('player_seek', { ms });
});

loadSongs();
