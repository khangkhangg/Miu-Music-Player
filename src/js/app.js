import { resolveTheme } from './theme.js';
import { prereqMessage } from './prereq.js';
import { deviceCard } from './device.js';
import { folderItems, librarySummary } from './folders.js';
import { songRow, visibleRange } from './songs.js';
import { nowPlaying, seekTarget } from './player.js';
import { nextRating, filterSongs, filterChips, applyRated } from './ratings.js';
import { deletePrompt, deleteResult } from './delete.js';
import { searchSongs } from './search.js';
import { sortSongs, indexEntries, sortLocale } from './sort.js';
import { presetOptions, bandLabel, dbLabel, withGain, clipText } from './eq.js';
import { rateLabel, sourceLabel, signalPath, eqActive, deviceOptions } from './output.js';
import { t, startInLanguage } from './i18n.js';

const ROW_HEIGHT = 36;
let songs = [];
let rawSongs = [];
let shown = []; // rawSongs after the rating filter, in display order
let activeFilter = 'all';
let query = '';
const SORT_LOCALE = sortLocale(navigator.languages);
let songsById = new Map();
let playerState = null;

const media = window.matchMedia('(prefers-color-scheme: dark)');
const preference = 'system'; // becomes a setting in F5.5

function applyTheme() {
  document.documentElement.dataset.theme = resolveTheme(preference, media.matches);
}
applyTheme();
media.addEventListener('change', applyTheme);

const invoke = window.__TAURI__?.core?.invoke;
if (invoke) {
  const languageSetting = await startInLanguage(invoke);
  const picker = document.getElementById('language');
  picker.value = languageSetting;
  picker.addEventListener('change', () => invoke('set_ui_language', { language: picker.value }));
  invoke('app_info').then(info => {
    document.getElementById('app-version').textContent = `${info.name} ${info.version}`;
  });
  invoke('check_prereqs').then(showPrereqNotice);
  window.__TAURI__.event.listen('devices', e => showDevices(e.payload));
  invoke('list_devices').then(showDevices);

  invoke('list_folders').then(showFolders);
  loadSongs();
  window.__TAURI__.event.listen('library-changed', e => {
    showFolders(e.payload);
    loadSongs();
  });
  const scroller = document.getElementById('song-scroll');
  scroller.addEventListener('scroll', () => requestAnimationFrame(renderSongs), { passive: true });
  new ResizeObserver(() => renderSongs()).observe(scroller);

  scroller.addEventListener('dblclick', e => {
    const row = e.target.closest('.song-row');
    if (row && !e.target.closest('button')) invoke('play_songs', { ids: shown.map(s => s.id), start: Number(row.dataset.index) });
  });
  scroller.addEventListener('click', e => {
    const btn = e.target.closest('button[data-act]');
    if (!btn) return;
    const song = shown[Number(btn.closest('.song-row').dataset.index)];
    if (btn.dataset.act === 'delete') confirmDelete([song]);
    else rate(song.id, btn.dataset.act);
  });
  for (const id of ['np-like', 'np-dislike']) {
    document.getElementById(id).addEventListener('click', e => {
      if (playerState?.songId != null) rate(playerState.songId, e.currentTarget.dataset.act, true);
    });
  }
  window.__TAURI__.event.listen('player', e => showPlayer(e.payload));
  window.__TAURI__.event.listen('song-rated', e => { if (applyRated(songsById, e.payload)) refreshList(); });
  document.getElementById('np-play').addEventListener('click', () =>
    invoke(playerState?.playing ? 'player_pause' : 'player_resume'));
  document.getElementById('np-prev').addEventListener('click', () => invoke('player_skip', { by: -1 }));
  document.getElementById('np-next').addEventListener('click', () => invoke('player_skip', { by: 1 }));
  document.getElementById('np-bar').addEventListener('click', e => {
    const rect = e.currentTarget.getBoundingClientRect();
    const song = songsById.get(playerState?.songId);
    const ms = seekTarget((e.clientX - rect.left) / rect.width, song?.durationMs);
    if (ms != null) invoke('player_seek', { ms });
  });
  document.getElementById('delete-all').addEventListener('click', () => confirmDelete(shown.filter(x => x.rating === -1)));
  setupEq();
  setupOutput();
  document.getElementById('np-volume').addEventListener('input', e =>
    invoke('player_volume', { volume: Number(e.target.value) / 100 }));
  window.__TAURI__.event.listen('tauri://drag-drop', e => e.payload.paths.forEach(addFolder));
  document.getElementById('add-folder').addEventListener('click', pickFolder);
  const search = document.getElementById('search');
  search.addEventListener('input', () => {
    query = search.value;
    document.getElementById('song-scroll').scrollTop = 0;
    refreshList();
  });
  search.addEventListener('keydown', e => {
    if (e.key === 'Escape' && search.value) { search.value = ''; search.dispatchEvent(new Event('input')); }
  });
  document.addEventListener('keydown', e => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') { e.preventDefault(); search.focus(); search.select(); }
  });
  document.getElementById('empty-add-folder').addEventListener('click', pickFolder);
}

async function pickFolder() {
  const dir = await window.__TAURI__.dialog.open({ directory: true, multiple: false });
  if (dir) addFolder(dir);
}

function addFolder(path) {
  invoke('add_folder', { path }).then(f => { showFolders(f); loadSongs(); }).catch(showFolderError);
}

function loadSongs() {
  invoke('list_songs').then(list => {
    rawSongs = sortSongs(list, SORT_LOCALE);
    songsById = new Map(list.map(s => [s.id, s]));
    refreshList();
  });
}

function refreshList() {
  shown = searchSongs(filterSongs(rawSongs, activeFilter), query);
  songs = shown.map(songRow);
  document.getElementById('song-spacer').style.height = `${songs.length * ROW_HEIGHT}px`;
  renderChips();
  renderSongs();
  updateRateButtons();
  document.getElementById('disliked-banner').hidden = !(activeFilter === 'disliked' && shown.length > 0);
  document.getElementById('duplicates-note').hidden = activeFilter !== 'duplicates';
  renderIndexBar();
}

/** Letters down the right of the list; each jumps to its first song. Hidden for grouped views. */
function renderIndexBar() {
  const bar = document.getElementById('index-bar');
  const entries = activeFilter === 'duplicates' ? [] : indexEntries(shown, SORT_LOCALE);
  bar.hidden = entries.length < 2;
  bar.replaceChildren(...entries.map(({ letter, index }) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.textContent = letter;
    b.setAttribute('aria-label', t('index.jump', { letter }));
    b.addEventListener('click', () => { document.getElementById('song-scroll').scrollTop = index * ROW_HEIGHT; });
    return b;
  }));
}

/** Ask, then move disliked songs to the Recycle Bin (F7.3). */
async function confirmDelete(list) {
  if (!list.length) return;
  const p = deletePrompt(list);
  const ok = await window.__TAURI__.dialog.ask(p.message, {
    title: p.title, kind: 'warning', okLabel: t('delete.confirm'), cancelLabel: t('delete.cancel'),
  });
  if (!ok) return;
  const report = await invoke('delete_songs', { ids: list.map(x => x.id) });
  const el = document.getElementById('delete-result');
  el.textContent = deleteResult(report);
  el.hidden = false;
  setTimeout(() => { el.hidden = true; }, 8000);
  loadSongs();
}

function renderChips() {
  const box = document.getElementById('filter-chips');
  box.replaceChildren(...filterChips(rawSongs, activeFilter).map(c => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = c.active ? 'chip active' : 'chip';
    b.setAttribute('role', 'tab');
    b.setAttribute('aria-selected', String(c.active));
    b.textContent = c.label;
    b.addEventListener('click', () => {
      activeFilter = c.id;
      document.getElementById('song-scroll').scrollTop = 0;
      refreshList();
    });
    return b;
  }));
}

/** Like/dislike a song; from the player bar, a dislike also skips to the next song. */
function rate(id, act, fromPlayer = false) {
  const song = songsById.get(id);
  if (!song) return;
  const rating = nextRating(song.rating, act);
  invoke('rate_song', { id, rating }).then(() => {
    song.rating = rating;
    refreshList();
    if (fromPlayer && rating === -1) invoke('player_skip', { by: 1 });
  });
}

function updateRateButtons() {
  const rating = songsById.get(playerState?.songId)?.rating ?? 0;
  document.getElementById('np-like').classList.toggle('on', rating === 1);
  document.getElementById('np-dislike').classList.toggle('on', rating === -1);
}

function showPlayer(event) {
  const songChanged = event?.songId !== playerState?.songId;
  playerState = event;
  const np = nowPlaying(event, songsById);
  const bar = document.getElementById('now-playing');
  bar.hidden = !np;
  if (songChanged) {
    renderSongs();
    updateRateButtons();
  }
  if (!np) return;
  document.getElementById('np-title').textContent = np.title;
  document.getElementById('np-artist').textContent = np.artist;
  document.getElementById('np-elapsed').textContent = np.elapsed;
  document.getElementById('np-duration').textContent = np.duration;
  document.getElementById('np-fill').style.width = `${np.progress * 100}%`;
  const play = document.getElementById('np-play');
  play.classList.toggle('paused', !np.playing);
  play.setAttribute('aria-label', t(np.playing ? 'player.pause' : 'player.play'));
  renderOutputInfo();
}

const THUMB_UP = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 7h3v7H2zM6 7v5.5L7.5 14H13l1.5-5.5V7H10l.8-3.2L9.5 2 6 7z"/></svg>';
const THUMB_DOWN = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 2h3v7H2zM6 9V3.5L7.5 2H13l1.5 5.5V9H10l.8 3.2L9.5 14 6 9z"/></svg>';

const TRASH = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 2h6v1h3v1.5H2V3h3zM3.5 6h9l-.8 8H4.3z"/></svg>';

function rateCell(rating, withDelete) {
  const span = document.createElement('span');
  span.className = 'rate-cell';
  if (withDelete) {
    const del = document.createElement('button');
    del.type = 'button';
    del.className = 'icon-btn rate-btn delete-btn';
    del.dataset.act = 'delete';
    del.setAttribute('aria-label', t('song.delete'));
    del.title = t('song.delete');
    del.innerHTML = TRASH; // constant markup, no user data
    span.append(del);
  }
  for (const [act, icon, label, on] of [['dislike', THUMB_DOWN, t('song.dislike'), rating === -1], ['like', THUMB_UP, t('song.like'), rating === 1]]) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = on ? 'icon-btn rate-btn on' : 'icon-btn rate-btn';
    b.dataset.act = act;
    b.setAttribute('aria-label', label);
    b.setAttribute('aria-pressed', String(on));
    b.innerHTML = icon; // constant markup, no user data
    span.append(b);
  }
  return span;
}

function cell(text, cls) {
  const span = document.createElement('span');
  span.textContent = text;
  span.dir = 'auto'; // RTL titles (Arabic, Hebrew) render right to left
  if (cls) span.className = cls;
  return span;
}

/** Render only the rows in view (plus overscan) inside the tall spacer. */
function renderSongs() {
  const scroller = document.getElementById('song-scroll');
  const spacer = document.getElementById('song-spacer');
  const { start, end } = visibleRange({
    scrollTop: scroller.scrollTop, height: scroller.clientHeight, rowHeight: ROW_HEIGHT, total: songs.length, overscan: 8,
  });
  const rows = [];
  for (let i = start; i < end; i++) {
    const s = songs[i];
    const row = document.createElement('div');
    row.className = s.id === playerState?.songId ? 'song-row current' : 'song-row';
    row.dataset.index = i;
    row.setAttribute('role', 'listitem');
    row.style.top = `${i * ROW_HEIGHT}px`;
    row.title = shown[i].path; // where the file is: tells copies apart
    row.append(cell(s.title, 'song-title'), cell(s.artist, 'muted'), cell(s.album, 'muted'), cell(s.quality, 'muted quality'), cell(s.duration, 'num muted'), rateCell(shown[i].rating, activeFilter === 'disliked'));
    rows.push(row);
  }
  spacer.replaceChildren(...rows);
}

function removeFolder(id) {
  invoke('remove_folder', { id }).then(f => { showFolders(f); loadSongs(); }).catch(showFolderError);
}

function showFolderError(message) {
  const el = document.getElementById('folder-error');
  el.textContent = t('folders.addFailed', { message });
  el.hidden = false;
  setTimeout(() => { el.hidden = true; }, 6000);
}

function showFolders(folders) {
  const list = document.getElementById('folder-list');
  list.replaceChildren(...folderItems(folders).map(item => {
    const li = document.createElement('li');
    li.className = 'folder-item';
    li.title = item.title;
    const name = document.createElement('span');
    name.className = 'folder-name';
    name.textContent = item.name;
    const count = document.createElement('span');
    count.className = 'folder-count muted';
    count.textContent = item.count;
    const remove = document.createElement('button');
    remove.className = 'icon-btn folder-remove';
    remove.type = 'button';
    remove.textContent = '×';
    remove.title = t('folders.remove');
    remove.setAttribute('aria-label', t('folders.removeNamed', { name: item.name }));
    remove.addEventListener('click', () => removeFolder(item.id));
    li.append(name, count, remove);
    return li;
  }));
  const summary = librarySummary(folders);
  document.getElementById('empty-state').hidden = Boolean(summary);
  document.getElementById('song-view').hidden = !summary;
  document.getElementById('library-summary-text').textContent = summary ?? '';
}

function showDevices(devices) {
  const card = deviceCard(devices);
  const el = document.getElementById('device-card');
  el.className = `device-card ${card.state}`;
  el.querySelector('.device-name').textContent = card.title;
  el.querySelector('.device-detail').textContent = card.detail;
  const storage = el.querySelector('.storage');
  storage.hidden = !card.storage;
  if (card.storage) {
    storage.querySelector('.storage-used').style.width = `${card.storage.usedPercent}%`;
    storage.querySelector('.storage-label').textContent = card.storage.label;
  }
}

function showPrereqNotice(report) {
  const message = prereqMessage(report);
  const notice = document.getElementById('prereq-notice');
  notice.hidden = !message;
  if (!message) return;
  notice.querySelector('.notice-title').textContent = message.title;
  notice.querySelector('.notice-body').textContent = message.body;
  notice.querySelector('.notice-detail').textContent = message.detail;
}

// ---- Equalizer panel (F7.7) ----
let eq = null;
let eqSendTimer = null;
let eqDragging = -2; // index of the slider being dragged (-1 = preamp), -2 = none

function setupEq() {
  const $ = id => document.getElementById(id);
  $('eq-open').addEventListener('click', () => togglePanel('eq'));
  $('eq-enabled').addEventListener('change', e => sendEq({ ...eq.settings, enabled: e.target.checked }, true));
  $('eq-preset').addEventListener('change', e => {
    if (e.target.value) invoke('apply_eq_preset', { name: e.target.value }).then(renderEq).catch(showEqError);
  });
  $('eq-save-open').addEventListener('click', () => { $('eq-save-row').hidden = false; $('eq-save-name').focus(); });
  $('eq-save').addEventListener('click', () => {
    invoke('save_eq_preset', { name: $('eq-save-name').value })
      .then(st => { $('eq-save-row').hidden = true; $('eq-save-name').value = ''; renderEq(st); })
      .catch(showEqError);
  });
  $('eq-delete').addEventListener('click', () => {
    if (eq?.preset) invoke('delete_eq_preset', { name: eq.preset }).then(renderEq).catch(showEqError);
  });
  window.__TAURI__.event.listen('eq-changed', e => renderEq(e.payload));
  invoke('eq_state').then(renderEq);
}

function showEqError(message) {
  const el = document.getElementById('eq-error');
  el.textContent = String(message);
  setTimeout(() => { el.textContent = ''; }, 5000);
}

/** Send settings to the engine; while dragging, at most every 50 ms. */
function sendEq(settings, now = false) {
  eq = { ...eq, settings };
  clearTimeout(eqSendTimer);
  const go = () => invoke('set_eq', { settings: eq.settings }).then(renderEq).catch(showEqError);
  if (now) go(); else eqSendTimer = setTimeout(go, 50);
}

function slider(index, value, label) {
  const wrap = document.createElement('label');
  wrap.className = index < 0 ? 'eq-band eq-preamp' : 'eq-band';
  const readout = document.createElement('span');
  readout.className = 'eq-db';
  readout.textContent = dbLabel(value);
  const input = document.createElement('input');
  input.type = 'range';
  input.min = '-12';
  input.max = '12';
  input.step = '0.5';
  input.value = String(value);
  input.setAttribute('aria-label', index < 0 ? t('eq.preamp') : t('eq.band', { label }));
  input.addEventListener('pointerdown', () => { eqDragging = index; });
  input.addEventListener('change', () => { eqDragging = -2; });
  input.addEventListener('input', () => {
    readout.textContent = dbLabel(Number(input.value));
    sendEq(withGain(eq.settings, index, Number(input.value)));
  });
  const name = document.createElement('span');
  name.className = 'eq-hz muted';
  name.textContent = label;
  wrap.append(readout, input, name);
  return wrap;
}

function renderEq(state) {
  if (!state) return;
  eq = state;
  const $ = id => document.getElementById(id);
  $('eq-enabled').checked = state.settings.enabled;
  const select = $('eq-preset');
  select.replaceChildren(...presetOptions(state).map(o => {
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.user ? t('eq.yours', { name: o.label }) : o.label;
    return opt;
  }));
  select.value = state.preset ?? '';
  $('eq-delete').hidden = !state.user.some(p => p.name === state.preset);
  if (eqDragging === -2) {
    $('eq-sliders').replaceChildren(
      slider(-1, state.settings.preampDb, t('eq.preamp')),
      ...state.bandsHz.map((hz, i) => slider(i, state.settings.gainsDb[i], bandLabel(hz))),
    );
  }
  $('eq-sliders').classList.toggle('off', !state.settings.enabled);
  const clip = clipText(state);
  $('eq-clip').hidden = !clip;
  $('eq-clip').textContent = clip ?? '';
  renderOutputInfo();
}

// ---- Output panel (F7.8) ----

/** Show one floating panel ('eq' or 'out') at a time; clicking its button again closes it. */
function togglePanel(name) {
  for (const p of ['eq', 'out']) {
    const panel = document.getElementById(`${p}-panel`);
    panel.hidden = p === name ? !panel.hidden : true;
    document.getElementById(`${p}-open`).setAttribute('aria-expanded', String(!panel.hidden));
  }
}

function setupOutput() {
  const $ = id => document.getElementById(id);
  $('out-open').addEventListener('click', () => {
    togglePanel('out');
    if (!$('out-panel').hidden) invoke('output_state').then(renderDevices); // devices may have changed
  });
  $('out-device').addEventListener('change', e => {
    $('out-device').disabled = true;
    invoke('set_output', { device: e.target.value || null })
      .then(renderDevices)
      .catch(message => {
        const el = $('out-error');
        el.textContent = String(message);
        setTimeout(() => { el.textContent = ''; }, 5000);
        invoke('output_state').then(renderDevices);
      })
      .finally(() => { $('out-device').disabled = false; });
  });
  $('out-exclusive').addEventListener('change', e => {
    $('out-exclusive').disabled = true;
    invoke('set_exclusive', { on: e.target.checked })
      .then(renderDevices)
      .catch(message => { $('out-error').textContent = String(message); invoke('output_state').then(renderDevices); })
      .finally(() => { $('out-exclusive').disabled = false; });
  });
  invoke('output_state').then(renderDevices);
}

let outputState = null;

function renderDevices(state) {
  outputState = state;
  document.getElementById('out-exclusive').checked = state.exclusive;
  document.getElementById('out-exclusive-note').hidden = !state.exclusive;
  renderOutputInfo();
  document.getElementById('out-device').replaceChildren(...deviceOptions(state).map(o => {
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    opt.selected = o.selected;
    return opt;
  }));
}

function renderOutputInfo() {
  const $ = id => document.getElementById(id);
  const ev = playerState;
  const song = songsById.get(ev?.songId);
  $('out-source').textContent = (ev?.source && sourceLabel(song?.format, ev.source)) || t('player.nothing');
  const depth = ev?.exclusive && ev.deviceBits ? ` · ${t('output.depth', { bits: ev.deviceBits })}` : '';
  $('out-device-info').textContent = ev?.deviceName && ev.deviceRate
    ? `${ev.deviceName} · ${rateLabel(ev.deviceRate)}${depth}`
    : (ev?.deviceName || t('output.noDevice'));
  const path = ev?.source && ev.deviceRate
    ? signalPath({
      sourceRate: ev.source.rate, deviceRate: ev.deviceRate, eqActive: eqActive(eq?.settings),
      exclusive: ev.exclusive, volume: ev.volume, sourceBits: ev.source.bits, deviceBits: ev.deviceBits,
    })
    : '';
  $('out-fallback').hidden = !(outputState?.exclusive && ev?.deviceRate && !ev.exclusive);
  $('out-path').textContent = path;
  $('out-path').classList.toggle('bit-perfect', path === t('output.bitPerfect'));
}
