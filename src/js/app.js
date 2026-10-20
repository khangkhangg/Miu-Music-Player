import { resolveTheme } from './theme.js';
import { prereqMessage } from './prereq.js';
import { deviceCard } from './device.js';
import { folderItems, librarySummary, encodingOptions } from './folders.js';
import { songRow, visibleRange } from './songs.js';
import { nowPlaying, seekTarget } from './player.js';
import { nextRating, filterSongs, filterChips, applyRated } from './ratings.js';
import { deletePrompt, deleteResult } from './delete.js';
import { searchSongs } from './search.js';
import { sortSongs, indexEntries, sortLocale } from './sort.js';
import { presetOptions, bandLabel, dbLabel, withGain, clipText } from './eq.js';
import { rateLabel, sourceLabel, signalPath, eqActive, deviceOptions } from './output.js';
import { t, startInLanguage } from './i18n.js';
import { planSummary } from './sync.js';
import { errorState } from './errors.js';
import { wizard } from './wizard.js';
import { formatBytes } from './device.js';
import { select } from './selection.js';
import { groupAlbums, groupArtists, upNext, libraryStats, splitTitle } from './library.js';
import { formatDuration } from './songs.js';
import {
  FIELDS, editorFields, changesFor, parseFilename, autoNumber, findReplace, changeCase, previewRows,
} from './editor.js';

const ROW_HEIGHT = 46;
let songs = [];
let rawSongs = [];
let shown = []; // rawSongs after the rating filter, in display order
let activeFilter = 'all';
let query = '';
const SORT_LOCALE = sortLocale(navigator.languages);
let songsById = new Map();
let selection = { selected: new Set(), anchor: null };
let playerState = null;

const media = window.matchMedia('(prefers-color-scheme: dark)');
let preference = 'dark'; // the theme setting (F5.5); dark until the setting loads

function applyTheme() {
  document.documentElement.dataset.theme = resolveTheme(preference, media.matches);
}
applyTheme();
media.addEventListener('change', applyTheme);

const invoke = window.__TAURI__?.core?.invoke;
if (invoke) {
  const languageSetting = await startInLanguage(invoke);
  setupSettings(languageSetting);
  setupWizard();
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
    showSyncPreview();
  });
  const scroller = document.getElementById('song-scroll');
  scroller.addEventListener('scroll', () => requestAnimationFrame(renderSongs), { passive: true });
  new ResizeObserver(() => renderSongs()).observe(scroller);

  scroller.addEventListener('dblclick', e => {
    const row = e.target.closest('.song-row');
    if (row && !e.target.closest('button')) playList(shown.map(s => s.id), Number(row.dataset.index));
  });
  scroller.addEventListener('click', e => {
    const btn = e.target.closest('button[data-act]');
    if (!btn) {
      const row = e.target.closest('.song-row');
      if (!row) return;
      selection = select(selection, shown.map(x => x.id), Number(row.dataset.index), { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey });
      renderSongs();
      updateEditButton();
      return;
    }
    const song = shown[Number(btn.closest('.song-row').dataset.index)];
    if (btn.dataset.act === 'delete') confirmDelete([song]);
    else rate(song.id, btn.dataset.act);
  });
  scroller.addEventListener('contextmenu', e => {
    const row = e.target.closest('.song-row');
    if (!row) return;
    e.preventDefault();
    const id = shown[Number(row.dataset.index)].id;
    if (!selection.selected.has(id)) selection = select(selection, shown.map(x => x.id), Number(row.dataset.index), {});
    renderSongs();
    openEditor();
  });
  setupEditor();
  for (const id of ['np-like', 'np-dislike']) {
    document.getElementById(id).addEventListener('click', e => {
      if (playerState?.songId != null) rate(playerState.songId, e.currentTarget.dataset.act, true);
    });
  }
  window.__TAURI__.event.listen('player', e => showPlayer(e.payload));
  window.__TAURI__.event.listen('song-rated', e => { if (applyRated(songsById, e.payload)) refreshList(); showSyncPreview(); });
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
  document.getElementById('open-mini').addEventListener('click', () => invoke('open_mini'));
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
    renderStats();
    renderLibraryViews();
    renderUpNext();
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
  updateEditButton();
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
  document.getElementById('now-playing').classList.toggle('idle', !np);
  if (songChanged) {
    renderSongs();
    updateRateButtons();
    renderNowArt();
    renderUpNext();
    markCurrentTrack();
  }
  const song = songsById.get(event?.songId);
  document.getElementById('np-title').textContent = np ? np.title : t('player.nothing');
  document.getElementById('np-artist').textContent = np ? np.artist : '';
  document.getElementById('np-format').textContent = np && event.source ? sourceLabel(song?.format, event.source) : '';
  if (!np) {
    document.getElementById('np-fill').style.width = '0%';
    document.getElementById('np-play').classList.add('paused');
    renderOutputInfo();
    return;
  }
  document.getElementById('np-elapsed').textContent = np.elapsed;
  document.getElementById('np-duration').textContent = np.duration;
  document.getElementById('np-fill').style.width = `${np.progress * 100}%`;
  const play = document.getElementById('np-play');
  play.classList.toggle('paused', !np.playing);
  play.setAttribute('aria-label', t(np.playing ? 'player.pause' : 'player.play'));
  renderOutputInfo();
}

const THUMB_UP = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.3a2 2 0 0 0 2-1.7l1.4-9A2 2 0 0 0 19.7 9H14z"/><path d="M7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/></svg>';
const THUMB_DOWN = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10 15v4a3 3 0 0 0 3 3l4-9V2H5.7a2 2 0 0 0-2 1.7l-1.4 9A2 2 0 0 0 4.3 15H10z"/><path d="M17 2h3a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-3"/></svg>';
const TRASH = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h18M8 6V4h8v2M6 6l1 15h10l1-15"/></svg>';
const NOTE = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 18V5l11-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/></svg>';
const PLAY = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 4l14 8-14 8z"/></svg>';

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
    if (selection.selected.has(s.id)) row.classList.add('selected');
    const main = document.createElement('span');
    main.className = 'song-main';
    main.append(cell(s.title, 'song-title'), cell(s.artist, 'song-artist'));
    row.append(thumb(shown[i], 'song-thumb', 68), main, cell(s.album, 'album'), cell(s.quality, 'quality'), cell(s.duration, 'num'), rateCell(shown[i].rating, activeFilter === 'disliked'));
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
    // Tag encoding (F8.1): "Aa" opens a picker for old files with garbled titles.
    const enc = document.createElement('button');
    enc.className = 'icon-btn folder-encoding-btn';
    enc.classList.toggle('on', Boolean(item.encoding));
    enc.type = 'button';
    enc.textContent = 'Aa';
    enc.title = t('folders.encodingTitle');
    enc.setAttribute('aria-label', t('folders.encoding', { name: item.name }));
    const pick = document.createElement('select');
    pick.className = 'folder-encoding';
    pick.hidden = true;
    pick.setAttribute('aria-label', t('folders.encoding', { name: item.name }));
    pick.append(...encodingOptions(item.encoding).map(o => {
      const opt = document.createElement('option');
      opt.value = o.value;
      opt.textContent = o.label;
      opt.selected = o.selected;
      return opt;
    }));
    enc.addEventListener('click', () => { pick.hidden = !pick.hidden; if (!pick.hidden) pick.focus(); });
    pick.addEventListener('change', () => {
      pick.disabled = true;
      invoke('set_folder_encoding', { id: item.id, encoding: pick.value || null }).then(showFolders).catch(showFolderError);
    });
    li.append(name, count, enc, remove, pick);
    return li;
  }));
  const summary = librarySummary(folders);
  hasSongs = Boolean(summary);
  wizardState.folders = (folders ?? []).length;
  renderWizard();
  showView(view);
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
  lastDevices = devices;
  showSyncPreview();
  const phone = (devices ?? [])[0];
  wizardState.phone = !phone ? 'none' : phone.trusted ? 'ready' : 'untrusted';
  renderWizard();
}

// ---- Sync preview (F5.3): what a sync would change on the connected phone ----
let lastDevices = [];

function showSyncPreview() {
  const box = document.getElementById('sync-box');
  const phone = (lastDevices ?? []).find(d => d.trusted && d.udid);
  box.hidden = !phone;
  if (!phone) return;
  invoke('sync_preview', { udid: phone.udid, freeBytes: phone.info?.freeBytes ?? null }).then(p => {
    document.getElementById('sync-plan').textContent = planSummary(p.plan);
    const short = document.getElementById('sync-shortage');
    short.hidden = !p.shortage;
    if (p.shortage) short.textContent = t('sync.noSpace', { need: formatBytes(p.shortage.need), available: formatBytes(p.shortage.available) });
  }).catch(() => { box.hidden = true; });
}

function showPrereqNotice(report) {
  wizardState.driversReady = Boolean(report?.ready);
  renderWizard();
  const message = prereqMessage(report);
  const notice = document.getElementById('prereq-notice');
  notice.hidden = !message;
  if (!message) return;
  notice.querySelector('.notice-title').textContent = message.title;
  notice.querySelector('.notice-body').textContent = message.body;
  notice.querySelector('.notice-detail').textContent = message.detail;
  const action = errorState('drivers-missing').action;
  const btn = document.getElementById('prereq-action');
  btn.textContent = action.label;
  btn.onclick = () => invoke('open_store_page');
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

// ---- Tag editor (F9) ----
let editing = []; // [{ id, path, tags }] in list order
let perSong = []; // per song: field → typed value, only fields that were touched

function selectedSongs() {
  return shown.filter(x => selection.selected.has(x.id));
}

function updateEditButton() {
  const n = selectedSongs().length;
  const b = document.getElementById('edit-tags');
  b.hidden = n === 0;
  b.textContent = t('editor.editN', { n });
}

function setupEditor() {
  const $ = id => document.getElementById(id);
  $('edit-tags').addEventListener('click', openEditor);
  document.addEventListener('keydown', e => {
    if (e.key === 'F2' && selection.selected.size) { e.preventDefault(); openEditor(); }
    if (e.key === 'Escape' && !$('editor').open && document.activeElement?.id !== 'search' && selection.selected.size) {
      selection = { selected: new Set(), anchor: null };
      renderSongs();
      updateEditButton();
    }
  });
  for (const id of ['tool-find-field', 'tool-case-field']) {
    $(id).replaceChildren(...FIELDS.filter(f => !f.number).map(f => {
      const o = document.createElement('option');
      o.value = f.key;
      o.textContent = t(f.label);
      return o;
    }));
  }
  $('editor-cancel').addEventListener('click', () => $('editor').close());
  $('editor-save').addEventListener('click', saveEditor);
  $('tool-pattern-go').addEventListener('click', () => {
    let missed = 0;
    editing.forEach((song, i) => {
      const parsed = parseFilename($('tool-pattern').value, song.path);
      if (!parsed) { missed += 1; return; }
      for (const [k, v] of Object.entries(parsed)) if (FIELDS.some(f => f.key === k)) perSong[i][k] = v;
    });
    $('tool-message').textContent = missed ? t('editor.patternMissed', { n: missed }) : '';
    renderEditor();
  });
  $('tool-number').addEventListener('click', () => {
    autoNumber(editing).forEach((nums, i) => Object.assign(perSong[i], nums));
    renderEditor();
  });
  $('tool-find-go').addEventListener('click', () => {
    const key = $('tool-find-field').value;
    editing.forEach((song, i) => {
      const current = key in perSong[i] ? perSong[i][key] : song.tags[key] ?? '';
      const next = findReplace(current, $('tool-find').value, $('tool-replace').value, $('tool-case-sensitive').checked);
      if (next !== current) perSong[i][key] = next;
    });
    renderEditor();
  });
  $('tool-case-go').addEventListener('click', () => {
    const key = $('tool-case-field').value;
    editing.forEach((song, i) => {
      const current = key in perSong[i] ? perSong[i][key] : song.tags[key] ?? '';
      const next = changeCase(current, $('tool-case').value);
      if (next !== current) perSong[i][key] = next;
    });
    renderEditor();
  });
  $('toast-undo').addEventListener('click', () => {
    invoke('undo_edit').then(() => toast(t('editor.undone'), false)).catch(err => toast(String(err), false));
  });
}

async function openEditor() {
  return openEditorFor(selectedSongs());
}

async function openEditorFor(songs) {
  if (!songs.length) return;
  editing = await invoke('song_tags', { ids: songs.map(x => x.id) });
  const order = new Map(songs.map((x, i) => [x.id, i]));
  editing.sort((a, b) => order.get(a.id) - order.get(b.id));
  perSong = editing.map(() => ({}));
  const $ = id => document.getElementById(id);
  $('editor-heading').textContent = t('editor.heading', { n: editing.length });
  $('editor-tools').hidden = editing.length < 2;
  $('editor-error').textContent = '';
  $('tool-message').textContent = '';
  buildFields();
  renderEditor();
  $('editor').showModal();
}

/** Inputs for each field; typing applies the value to every song being edited. */
function buildFields() {
  const fields = editorFields(editing);
  const box = document.getElementById('editor-fields');
  box.replaceChildren(...FIELDS.flatMap(f => {
    const label = document.createElement('label');
    label.htmlFor = `edit-${f.key}`;
    label.textContent = t(f.label);
    const input = document.createElement('input');
    input.type = 'text';
    input.id = `edit-${f.key}`;
    input.dir = 'auto';
    if (f.number) input.inputMode = 'numeric';
    input.value = fields[f.key].value;
    if (fields[f.key].mixed) input.placeholder = t('editor.mixed');
    input.addEventListener('input', () => {
      input.classList.add('touched');
      perSong.forEach(edited => { edited[f.key] = input.value; });
      renderEditor();
    });
    return [label, input];
  }));
}

function renderEditor() {
  const list = document.getElementById('editor-preview');
  const rows = previewRows(editing, perSong);
  if (!rows.length) {
    const li = document.createElement('li');
    li.className = 'muted';
    li.textContent = t('editor.noChanges');
    list.replaceChildren(li);
  } else {
    const byId = new Map(editing.map(x => [x.id, x]));
    list.replaceChildren(...rows.map(r => {
      const li = document.createElement('li');
      const name = document.createElement('strong');
      name.textContent = byId.get(r.id).tags.title || byId.get(r.id).path.split(/[\\/]/).pop();
      li.append(name);
      for (const c of r.changes) {
        const from = document.createElement('span');
        from.className = 'from';
        from.textContent = c.from;
        li.append(` · ${t(FIELDS.find(f => f.key === c.field).label)}: `, from, ` → ${c.to}`);
      }
      return li;
    }));
  }
  document.getElementById('editor-save').disabled = rows.length === 0;
}

async function saveEditor() {
  const $ = id => document.getElementById(id);
  const edits = editing.map((song, i) => ({ id: song.id, changes: changesFor(perSong[i]) }))
    .filter(e => Object.keys(e.changes).length);
  $('editor-save').disabled = true;
  try {
    const report = await invoke('save_edits', { edits });
    $('editor').close();
    const failed = report.failed.length
      ? ` ${t('editor.failed', { n: report.failed.length, reasons: [...new Set(report.failed.map(([, why]) => why))].join('; ') })}`
      : '';
    toast(t('editor.saved', { n: report.saved }) + failed, report.saved > 0);
  } catch (err) {
    $('editor-error').textContent = String(err);
    $('editor-save').disabled = false;
  }
}

let toastTimer = null;
function toast(text, undoable) {
  const $ = id => document.getElementById(id);
  $('toast-text').textContent = text;
  $('toast-undo').hidden = !undoable;
  $('toast').hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { $('toast').hidden = true; }, 8000);
}

// ---- Views and settings (F5.5) ----
let view = 'songs';
let hasSongs = false;

function showView(name) {
  view = name;
  const $ = id => document.getElementById(id);
  const library = ['songs', 'albums', 'artists', 'page'].includes(name);
  $('settings-view').hidden = name !== 'settings';
  $('empty-state').hidden = !library || hasSongs;
  $('song-view').hidden = name !== 'songs' || !hasSongs;
  $('albums-view').hidden = !['albums', 'artists'].includes(name) || !hasSongs;
  $('album-page').hidden = name !== 'page' || !hasSongs;
  const navOn = name === 'songs' ? { all: 'nav-songs', liked: 'nav-liked', disliked: 'nav-disliked' }[activeFilter] ?? 'nav-songs'
    : name === 'page' ? (page?.kind === 'artist' ? 'nav-artists' : 'nav-albums') : `nav-${name}`;
  for (const id of ['nav-songs', 'nav-albums', 'nav-artists', 'nav-liked', 'nav-disliked', 'nav-settings']) $(id).classList.toggle('active', id === navOn);
  if (name === 'albums' || name === 'artists') renderTiles();
}

function showSettings(s) {
  const $ = id => document.getElementById(id);
  preference = s.theme;
  applyTheme();
  $('set-theme').value = s.theme;
  $('set-mini-minimize').checked = s.miniOnMinimize;
  $('set-mini-top').checked = s.miniOnTop;
  $('set-viz').checked = s.visualizerOn;
  $('set-viz-style').value = s.visualizerStyle;
  $('set-lossless').value = s.conversion.lossless;
  $('set-hires').value = s.conversion.hiRes;
  $('set-exclusive').checked = s.exclusiveOutput;
}

function setupSettings(languageSetting) {
  const $ = id => document.getElementById(id);
  $('language').value = languageSetting;
  $('language').addEventListener('change', () => invoke('set_ui_language', { language: $('language').value }));
  const go = (id, fn) => $(id).addEventListener('click', e => { e.preventDefault(); fn(); });
  const songsWith = filter => () => { activeFilter = filter; refreshList(); showView('songs'); };
  go('nav-songs', songsWith('all'));
  go('nav-liked', songsWith('liked'));
  go('nav-disliked', songsWith('disliked'));
  go('nav-albums', () => showView('albums'));
  go('nav-artists', () => showView('artists'));
  go('nav-settings', () => showView('settings'));
  $('album-back').addEventListener('click', () => showView(page?.kind === 'artist' ? 'artists' : 'albums'));
  $('album-play').addEventListener('click', () => { if (page) playList(page.songs.map(x => x.id), 0); });
  $('album-edit').addEventListener('click', () => {
    if (!page) return;
    selection = { selected: new Set(page.songs.map(x => x.id)), anchor: null };
    openEditorFor(page.songs);
  });
  const playTrack = li => { if (li && page) playList(page.songs.map(x => x.id), Number(li.dataset.index)); };
  $('track-list').addEventListener('click', e => { if (e.target.closest('.track-play')) playTrack(e.target.closest('.track')); });
  $('track-list').addEventListener('dblclick', e => { if (!e.target.closest('button')) playTrack(e.target.closest('.track')); });
  const reload = () => invoke('app_settings').then(showSettings);
  $('set-theme').addEventListener('change', () => invoke('set_app_setting', { name: 'theme', value: $('set-theme').value }).then(showSettings));
  const conversion = () => invoke('set_app_setting', { name: 'conversion', value: { lossless: $('set-lossless').value, hiRes: $('set-hires').value } })
    .then(showSettings).then(showSyncPreview);
  $('set-lossless').addEventListener('change', conversion);
  $('set-hires').addEventListener('change', conversion);
  $('set-mini-minimize').addEventListener('change', e => invoke('set_mini_on_minimize', { on: e.target.checked }).then(reload));
  $('set-mini-top').addEventListener('change', e => invoke('set_mini_on_top', { on: e.target.checked }).then(reload));
  $('set-viz').addEventListener('change', e => invoke('set_visualizer_on', { on: e.target.checked }).then(reload));
  $('set-viz-style').addEventListener('change', e => invoke('set_visualizer_style', { style: e.target.value }).then(reload));
  $('set-exclusive').addEventListener('change', e => invoke('set_exclusive', { on: e.target.checked }).then(renderDevices).then(reload));
  $('export-diagnostics').addEventListener('click', async () => {
    const path = await window.__TAURI__.dialog.save({ defaultPath: 'MiuMusic-diagnostics.txt', filters: [{ name: t('settings.textFiles'), extensions: ['txt'] }] }); // i18n-ignore: file name
    if (!path) return;
    const card = document.getElementById('device-card');
    const device = `${card.querySelector('.device-name').textContent} · ${card.querySelector('.device-detail').textContent}`;
    invoke('export_diagnostics', { path, device })
      .then(() => { $('diagnostics-result').textContent = t('settings.diagnosticsSaved', { path }); })
      .catch(err => { $('diagnostics-result').textContent = String(err); });
  });
  window.__TAURI__.event.listen('settings-changed', ev => showSettings(ev.payload));
  window.__TAURI__.event.listen('mini-on-top', reload);
  reload();
}

// ---- First-run wizard (F6.2) ----
const remembered = key => { try { return localStorage.getItem(key) === '1'; } catch { return false; } };
const remember = key => { try { localStorage.setItem(key, '1'); } catch { /* private storage off: ask again next time */ } };
const wizardState = {
  driversReady: true, phone: 'none', folders: 0,
  skippedPhone: remembered('wizard.skippedPhone'), dismissed: remembered('wizard.dismissed'),
};

function setupWizard() {
  const $ = id => document.getElementById(id);
  $('wizard-close').addEventListener('click', () => { wizardState.dismissed = true; remember('wizard.dismissed'); renderWizard(); });
  $('wizard-skip').addEventListener('click', () => { wizardState.skippedPhone = true; remember('wizard.skippedPhone'); renderWizard(); });
  $('wizard-drivers').addEventListener('click', () => invoke('open_store_page'));
  $('wizard-folder').addEventListener('click', pickFolder);
}

function renderWizard() {
  const w = wizard(wizardState);
  const box = document.getElementById('wizard');
  if (!box) return;
  box.hidden = !w.show;
  for (const step of w.steps) box.querySelector(`[data-step="${step.id}"]`).dataset.state = step.state;
  document.getElementById('wizard-phone-body').textContent = t(w.hint === 'trust' ? 'device.trust' : 'wizard.phoneBody');
}

// ---- Redesign: art, stats, albums and artists, album page, up next ----
let queueIds = []; // what play_songs was last given, to show what comes next
let page = null; // the album or artist page on show: { kind, title, artist, songs, coverId, year }

function playList(ids, start) {
  queueIds = ids;
  invoke('play_songs', { ids, start });
}

/** Cover thumbnail address for a song (served by the miuart: protocol, cached on disk). */
function artUrl(id, size) {
  return window.__TAURI__.core.convertFileSrc(`${id}-${size}`, 'miuart');
}

/** A box with the song's cover, or a note icon when it has none (or it fails to load). */
function thumb(song, cls, size) {
  const box = document.createElement('span');
  box.className = cls;
  const placeholder = () => { box.innerHTML = NOTE; }; // constant markup
  if (song?.hasArtwork) {
    const img = document.createElement('img');
    img.alt = '';
    img.loading = 'lazy';
    img.src = artUrl(song.id, size);
    img.addEventListener('error', placeholder, { once: true });
    box.append(img);
  } else {
    placeholder();
  }
  return box;
}

function renderStats() {
  const st = libraryStats(rawSongs);
  const fmt = new Intl.NumberFormat();
  document.getElementById('stat-songs').textContent = fmt.format(st.songs);
  document.getElementById('stat-liked').textContent = fmt.format(st.liked);
}

function renderLibraryViews() {
  if (view === 'albums' || view === 'artists') renderTiles();
  if (view === 'page' && page) {
    // Keep the page in step with edits and rescans.
    const fresh = (page.kind === 'artist' ? groupArtists(rawSongs, SORT_LOCALE).find(a => a.name === page.title)
      : groupAlbums(rawSongs, SORT_LOCALE).find(a => a.key === page.key));
    if (fresh) openPage(page.kind, fresh, false);
  }
}

/** Album or artist tiles: cover · info, then info · cover on the next row (like the mockup). */
function renderTiles() {
  const artists = view === 'artists';
  const items = artists ? groupArtists(rawSongs, SORT_LOCALE) : groupAlbums(rawSongs, SORT_LOCALE);
  document.getElementById('grid-heading').textContent = t(artists ? 'nav.artists' : 'nav.albums');
  document.getElementById('grid-summary').textContent = t(artists ? 'library.artists' : 'library.albums', { n: items.length });
  const box = document.getElementById('tiles');
  const cols = Math.max(2, Math.floor(box.clientWidth / 170 / 2) * 2) || 4;
  box.style.setProperty('--tile-cols', cols);
  const cells = [];
  items.forEach((item, i) => {
    const row = Math.floor((i * 2) / cols);
    const open = () => openPage(artists ? 'artist' : 'album', item);
    const name = artists ? item.name : item.title;
    const cover = document.createElement('button');
    cover.type = 'button';
    cover.className = 'tile tile-cover';
    cover.setAttribute('aria-label', artists ? name : `${name} — ${item.artist}`);
    cover.addEventListener('click', open);
    cover.append(thumb(item.coverId != null ? { id: item.coverId, hasArtwork: true } : null, 'tile-art', 320));
    const info = document.createElement('button');
    info.type = 'button';
    info.className = 'tile tile-info';
    info.addEventListener('click', open);
    const [l1, l2] = splitTitle(name);
    const sub = artists ? '' : item.artist;
    const stat = (label, value) => {
      const span = document.createElement('span');
      const small = document.createElement('small');
      small.className = 'stat-label';
      small.textContent = label;
      const b = document.createElement('b');
      b.textContent = value ?? '–';
      span.append(small, b);
      return span;
    };
    const stats = document.createElement('span');
    stats.className = 'tile-stats';
    stats.append(stat(t('tile.songs'), item.count), artists ? stat(t('tile.albums'), item.albums) : stat(t('tile.year'), item.year));
    info.append(cell(l1, 'tile-line1'), cell(l2, 'tile-line2'), cell(sub, 'tile-sub'), stats);
    cells.push(...(row % 2 === 0 ? [cover, info] : [info, cover]));
  });
  box.replaceChildren(...cells);
}

function openPage(kind, item, scroll = true) {
  const artist = kind === 'artist';
  page = { kind, key: item.key, title: artist ? item.name : item.title, artist: artist ? '' : item.artist, songs: item.songs, coverId: item.coverId, year: item.year };
  const $ = id => document.getElementById(id);
  const [l1, l2] = splitTitle(page.title);
  $('hero-line1').textContent = l1;
  $('hero-line2').textContent = l2;
  $('hero-artist').textContent = page.artist;
  $('hero-kicker').textContent = artist ? t('page.artist') : (page.year ? t('page.albumYear', { year: page.year }) : t('page.album'));
  $('album-back-label').textContent = t(artist ? 'nav.artists' : 'nav.albums');
  const cover = $('hero-cover');
  cover.replaceChildren(thumb(page.coverId != null ? { id: page.coverId, hasArtwork: true } : null, 'hero-art', 320));
  const bg = $('hero-bg');
  bg.hidden = page.coverId == null;
  if (page.coverId != null) bg.src = artUrl(page.coverId, 160);
  const total = page.songs.reduce((sum, x) => sum + (x.durationMs ?? 0), 0);
  const liked = page.songs.filter(x => x.rating === 1).length;
  $('album-facts').textContent = t('page.facts', { songs: t('count.songs', { n: page.songs.length }), time: formatDuration(total), liked });
  $('track-list').replaceChildren(...page.songs.map((x, i) => {
    const li = document.createElement('li');
    li.className = 'track';
    li.dataset.index = i;
    li.dataset.id = x.id;
    const play = document.createElement('button');
    play.type = 'button';
    play.className = 'track-play';
    play.setAttribute('aria-label', t('page.play', { title: x.title }));
    play.innerHTML = PLAY; // constant markup
    const name = document.createElement('span');
    name.className = 'track-name';
    name.dir = 'auto';
    const title = document.createElement('span');
    title.textContent = x.title;
    const sep = document.createElement('span');
    sep.className = 'track-sep';
    sep.textContent = ' — ';
    const who = document.createElement('span');
    who.className = 'track-artist';
    who.textContent = x.artist || t('song.unknownArtist');
    name.append(title, sep, who);
    const row = songRow(x);
    li.append(play, name, cell(row.quality, 'track-format'), rateCell(x.rating, false), cell(row.duration, 'track-time'));
    return li;
  }));
  markCurrentTrack();
  showView('page');
  if (scroll) $('album-page').scrollTop = 0;
}

function markCurrentTrack() {
  for (const li of document.querySelectorAll('#track-list .track')) li.classList.toggle('current', Number(li.dataset.id) === playerState?.songId);
}

function renderNowArt() {
  const song = songsById.get(playerState?.songId);
  const img = document.getElementById('np-art');
  const empty = document.getElementById('np-art-empty');
  const has = Boolean(song?.hasArtwork);
  img.hidden = !has;
  empty.hidden = has;
  if (has) {
    img.onerror = () => { img.hidden = true; empty.hidden = false; };
    img.src = artUrl(song.id, 640);
  }
}

function renderUpNext() {
  const list = upNext(queueIds, playerState?.songId, songsById, 4);
  document.getElementById('up-next').replaceChildren(...list.map(x => {
    const li = document.createElement('li');
    const text = document.createElement('span');
    text.className = 'text';
    text.append(cell(x.title), cell(x.artist || t('song.unknownArtist')));
    li.append(thumb(x, 'thumb', 72), text, cell(formatDuration(x.durationMs), 'time'));
    return li;
  }));
  document.querySelector('.up-next').hidden = list.length === 0;
}

// Ratings change in place on the album page too.
document.getElementById('track-list')?.addEventListener('click', e => {
  const btn = e.target.closest('button[data-act]');
  if (!btn) return;
  e.stopPropagation();
  rate(Number(btn.closest('.track').dataset.id), btn.dataset.act);
}, true);
window.addEventListener('resize', () => { if (view === 'albums' || view === 'artists') renderTiles(); });
