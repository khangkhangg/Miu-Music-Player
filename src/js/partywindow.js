// The party screen in its own window (P.5): drag it to the TV, full screen with F11 or a
// double-click. It follows the DJ by itself: the song on air and the next one, the guests' QR code
// and queue, and YouTube videos in YouTube's own player (the end of a video hands back to Auto DJ).

import { t, startInLanguage, translatePage } from './i18n.js';
import { videoUrl, videoEnded, listenMessage, partyQr, nowAndNext, beatLight, dropCountdown } from './party.js';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const POLL_MS = 250;
const GUESTS_MS = 1500;

const songs = new Map();
let songsAt = 0;
const songById = (id) => songs.get(id);
const artUrl = (id, size) => window.__TAURI__.core.convertFileSrc(`${id}-${size}`, 'miuart');

async function loadSongs() {
  songsAt = Date.now();
  const list = await invoke('list_songs').catch(() => []);
  songs.clear();
  for (const s of list) songs.set(s.id, s);
}

/** A song the screen has not heard of (added since it opened): read the library again, at most every 10 s. */
function knowAll(state) {
  const missing = (state?.decks ?? []).some((d) => d.songId != null && !songs.has(d.songId));
  if (missing && Date.now() - songsAt > 10000) loadSongs();
}

function renderNow(state) {
  const { now, next, progress, level } = nowAndNext(state, songById);
  $('party-title').textContent = now?.title ?? t('dj.nothingOnAir');
  $('party-artist').textContent = now?.artist ?? '';
  const img = $('party-img');
  if (now && img.dataset.id !== String(now.id)) {
    img.dataset.id = String(now.id);
    img.hidden = false;
    img.onerror = () => { img.hidden = true; };
    img.src = artUrl(now.id, 600);
  }
  if (!now) { img.hidden = true; img.dataset.id = ''; }
  $('party-progress').style.width = `${progress * 100}%`;
  $('party-next').textContent = next ? t('dj.upNext', { title: [next.title, next.artist].filter(Boolean).join(' · ') }) : '';
  $('party-pulse').style.opacity = String(0.15 + Math.min(1, level * 1.6) * 0.85);
}

let shownQr = null;
function renderGuests(info, view) {
  const qr = partyQr(info);
  if (qr !== shownQr) {
    $('party-qr').innerHTML = qr ?? '';
    shownQr = qr;
  }
  $('party-guests').hidden = !qr;
  $('party-queue').replaceChildren(...(view?.queue ?? []).slice(0, 5).map((e) => {
    const li = document.createElement('li');
    li.textContent = e.title;
    li.dir = 'auto';
    const s = document.createElement('small');
    s.textContent = t('party.votes', { n: e.votes });
    li.append(s);
    return li;
  }));
}

// ---- Videos ----

const box = $('party-video');
let videoId = null;

function showVideo(id, title) {
  if (videoId === id) return;
  videoId = id;
  $('party').classList.add('video');
  const frame = document.createElement('iframe');
  frame.src = videoUrl(id, location.origin);
  frame.title = title;
  frame.allow = 'autoplay; encrypted-media; fullscreen; picture-in-picture';
  frame.allowFullscreen = true;
  // Ask the player to report its state, so the end of the video hands back to Auto DJ.
  frame.addEventListener('load', () => {
    for (const delay of [500, 1500, 3000]) setTimeout(() => frame.contentWindow?.postMessage(listenMessage(), 'https://www.youtube.com'), delay);
  });
  box.replaceChildren(frame);
  box.hidden = false;
}

function hideVideo() {
  if (videoId == null) return;
  videoId = null;
  box.replaceChildren();
  box.hidden = true;
  $('party').classList.remove('video');
}

window.addEventListener('message', (m) => {
  if (videoId != null && videoEnded(m)) {
    hideVideo();
    invoke('party_video_done').catch(() => {});
  }
});
listen('party-video', (e) => showVideo(e.payload.videoId, e.payload.title));

// ---- Following the DJ ----

let lastState = null;
let stateAt = 0;

async function poll() {
  const state = await invoke('dj_state').catch(() => null);
  lastState = state;
  stateAt = performance.now();
  knowAll(state);
  renderNow(state);
}

// ---- Light show and drop countdown, every frame from the last beat the engine reported ----

const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
const COLOURS = ['rgba(242, 193, 46, 0.55)', 'rgba(255, 122, 69, 0.5)', 'rgba(226, 51, 122, 0.5)', 'rgba(57, 198, 192, 0.5)'];
let shownCount = '';

function frame() {
  const show = lastState?.show ?? null;
  const since = performance.now() - stateAt;
  const light = beatLight(show, since);
  const lights = $('party-lights');
  if (light && lastState?.decks?.some((d) => d.playing)) {
    const bar = Math.floor((show.beatPos + since / 1000 / show.beatS) / 4);
    lights.style.setProperty('--light', COLOURS[((bar % 4) + 4) % 4]);
    lights.style.opacity = String((calm ? 0.15 : light.bar ? 0.9 : 0.55) * light.pulse);
  } else {
    lights.style.opacity = '0';
  }
  const count = dropCountdown(show, since);
  const key = count ? (count.drop ? 'drop' : String(count.bars)) : '';
  if (key !== shownCount) {
    shownCount = key;
    const box = $('party-count');
    box.hidden = !count || videoId != null;
    box.classList.toggle('drop', !!count?.drop);
    $('party-count-n').textContent = count?.drop ? t('party.dropNow') : key;
    // Restart the pop on each new number.
    const b = $('party-count-n');
    b.style.animation = 'none';
    void b.offsetWidth;
    b.style.animation = '';
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

async function pollGuests() {
  const [info, view, video] = await Promise.all([
    invoke('party_info').catch(() => ({})),
    invoke('party_host_state').catch(() => null),
    invoke('party_video_now').catch(() => null),
  ]);
  renderGuests(info, view);
  // The DJ skipped the video, or it started before this window opened.
  if (video) showVideo(video.videoId, video.title);
  else hideVideo();
}

const fullScreen = () => invoke('party_screen_fullscreen').catch(() => {});
$('party-full').addEventListener('click', fullScreen);
document.addEventListener('dblclick', (e) => { if (!e.target.closest('button')) fullScreen(); });
document.addEventListener('keydown', (e) => {
  if (e.key === 'F11') { e.preventDefault(); fullScreen(); }
});

// The pointer and the full-screen button hide when the mouse rests (guests see only the show).
let idle;
const wake = () => {
  document.body.classList.remove('idle');
  clearTimeout(idle);
  idle = setTimeout(() => document.body.classList.add('idle'), 2500);
};
document.addEventListener('pointermove', wake);

(async () => {
  await startInLanguage(invoke);
  translatePage(document);
  await loadSongs();
  wake();
  poll();
  pollGuests();
  setInterval(poll, POLL_MS);
  setInterval(pollGuests, GUESTS_MS);
})();
