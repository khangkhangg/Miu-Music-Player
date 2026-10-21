import { resolveTheme } from './theme.js';
import { prereqMessage } from './prereq.js';
import { deviceCard } from './device.js';
import { folderItems, librarySummary, encodingOptions } from './folders.js';
import { songRow, visibleRange, scrollToShow } from './songs.js';
import { nowPlaying, seekTarget, nextRepeat, repeatButton, skipTarget, playButtonAction, startIndex } from './player.js';
import { nextRating, filterSongs, filterChips, applyRated } from './ratings.js';
import { deletePrompt, deleteResult } from './delete.js';
import { searchSongs } from './search.js';
import { sortSongs, indexEntries, sortLocale } from './sort.js';
import { presetOptions, bandLabel, dbLabel, withGain, clipText } from './eq.js';
import { rateLabel, sourceLabel, signalPath, eqActive, deviceOptions, secondOptions, dualStatus, delaysFor, offsetOf, offsetLabel, balanceLabel } from './output.js';
import { t, startInLanguage, translatePage } from './i18n.js';
import { playerCard, targetOptions, withPlays, PLAYS, playerStageText, playerResult, playerStopped } from './players.js';
import { planSummary, stageText, syncResult, syncStopped, showsGuide, lastSyncLabel } from './sync.js';
import { errorState } from './errors.js';
import { wizard } from './wizard.js';
import { workState } from './progress.js';
import { episodeMeta, episodeState, markSubscribed, lengthLabel } from './podcasts.js';
import { BOOK_LANGUAGES, languageName, bookMeta, bookAction, bookProgress, markAdded } from './audiobooks.js';
import { language } from './i18n.js';
import { formatBytes } from './device.js';
import { select } from './selection.js';
import { groupAlbums, groupArtists, libraryStats, splitTitle, syncRuleFor, hasSyncRule, needsArtwork } from './library.js';
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

/** Loading screen progress; at 100 it fades away. */
function boot(percent) {
  const el = document.getElementById('boot');
  if (!el || el.classList.contains('done')) return;
  document.getElementById('boot-fill').style.width = `${percent}%`;
  el.setAttribute('aria-valuenow', String(percent));
  if (percent >= 100) {
    el.classList.add('done');
    setTimeout(() => el.remove(), 400);
  }
}

const invoke = window.__TAURI__?.core?.invoke;
if (invoke) {
  boot(15);
  const languageSetting = await startInLanguage(invoke);
  boot(35);
  window.__TAURI__.event.listen('library-progress', e => showWork(e.payload));
  setupSettings(languageSetting);
  setupTaskbar();
  setupWizard();
  invoke('app_info').then(info => {
    document.getElementById('app-version').textContent = `${info.name} ${info.version}`;
  });
  invoke('check_prereqs').then(showPrereqNotice);
  window.__TAURI__.event.listen('devices', e => showDevices(e.payload));
  invoke('list_devices').then(showDevices);
  setupPlayers();

  invoke('list_folders').then(f => { showFolders(f); boot(60); });
  loadSongs().then(() => boot(100), () => boot(100));
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
      markSelection(); // in place: rebuilding the rows here would swallow a double-click
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
  document.getElementById('np-shuffle').addEventListener('click', () => invoke('player_shuffle', { on: !playerState?.shuffle }));
  document.getElementById('np-repeat').addEventListener('click', () => invoke('player_repeat', { repeat: nextRepeat(playerState?.repeat) }));
  window.__TAURI__.event.listen('player', e => showPlayer(e.payload));
  window.__TAURI__.event.listen('song-rated', e => { if (applyRated(songsById, e.payload)) refreshList(); showSyncPreview(); updateRateButtons(); });
  document.getElementById('np-play').addEventListener('click', () => {
    const action = playButtonAction(playerState, shown.length);
    if (action === 'pause' || action === 'resume') {
      invoke(`player_${action}`);
    } else if (action === 'playList') {
      // Nothing queued yet: play the list on screen (from a random song when shuffle is on).
      playList(shown.map(s => s.id), startIndex(shown.length, playerState?.shuffle));
    }
  });
  document.getElementById('np-prev').addEventListener('click', () => invoke('player_skip', { by: -1 }));
  const skip = delta => () => {
    const ep = playerState?.episodeId != null ? episodeInfo(playerState.episodeId) : null;
    invoke('player_seek', { ms: skipTarget(playerState?.positionMs ?? 0, delta, ep?.durationMs) });
  };
  document.getElementById('np-back15').addEventListener('click', skip(-15_000));
  document.getElementById('np-fwd30').addEventListener('click', skip(30_000));
  document.getElementById('np-next').addEventListener('click', () => invoke('player_skip', { by: 1 }));
  document.getElementById('np-bar').addEventListener('click', e => {
    const rect = e.currentTarget.getBoundingClientRect();
    const song = playerState?.episodeId != null ? episodeInfo(playerState.episodeId) : songsById.get(playerState?.songId);
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
  return invoke('list_songs').then(list => {
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
    updateRateButtons();
    if (fromPlayer && rating === -1) invoke('player_skip', { by: 1 });
  });
}

/** Shuffle and repeat buttons follow what the engine reports. */
function updateModeButtons() {
  const shuffle = document.getElementById('np-shuffle');
  shuffle.setAttribute('aria-pressed', String(Boolean(playerState?.shuffle)));
  const r = repeatButton(playerState?.repeat);
  const repeat = document.getElementById('np-repeat');
  repeat.setAttribute('aria-pressed', String(r.pressed));
  repeat.classList.toggle('one', r.one);
  repeat.setAttribute('aria-label', r.label);
  repeat.title = r.label;
}

function updateRateButtons() {
  updateModeButtons();
  const song = songsById.get(playerState?.songId);
  document.getElementById('np-rate').hidden = !song;
  document.getElementById('np-like').classList.toggle('on', song?.rating === 1);
  document.getElementById('np-dislike').classList.toggle('on', song?.rating === -1);
}

/** Bring the playing song into view in the song list (only when it is out of view). */
function revealPlaying() {
  if (view !== 'songs') return;
  const scroller = document.getElementById('song-scroll');
  const index = shown.findIndex(s => s.id === playerState?.songId);
  const top = scrollToShow(index, { scrollTop: scroller.scrollTop, height: scroller.clientHeight, rowHeight: ROW_HEIGHT });
  if (top != null) scroller.scrollTo({ top, behavior: 'smooth' });
}

/** Little moving bars over the playing song's cover. */
function eqBars() {
  const bars = document.createElement('span');
  bars.className = 'eq-bars';
  bars.setAttribute('aria-hidden', 'true');
  bars.append(document.createElement('i'), document.createElement('i'), document.createElement('i'));
  return bars;
}

/** A round play icon (built with DOM calls, so no markup strings). */
function playIcon() {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(ns, 'path');
  path.setAttribute('d', 'M7 4l14 8-14 8z');
  svg.append(path);
  return svg;
}

/** Play every song of a list, from the top or a random one when shuffle is on. */
function playAll(list) {
  if (!list.length) return;
  playList(list.map(s => s.id), startIndex(list.length, playerState?.shuffle));
}

function showPlayer(event) {
  if (Boolean(event?.playing) !== taskbarPlaying) {
    taskbarPlaying = Boolean(event?.playing);
    invoke('taskbar_playing', { playing: taskbarPlaying }).catch(() => {});
  }
  const songChanged = event?.songId !== playerState?.songId || event?.episodeId !== playerState?.episodeId;
  playerState = event;
  const episode = event?.episodeId != null ? episodeInfo(event.episodeId) : null;
  const np = episode ? nowPlaying({ ...event, songId: 'episode' }, new Map([['episode', episode]])) : nowPlaying(event, songsById);
  for (const id of ['np-back15', 'np-fwd30']) document.getElementById(id).hidden = !episode;
  for (const id of ['np-prev', 'np-next', 'np-shuffle', 'np-repeat']) document.getElementById(id).hidden = Boolean(episode);
  document.getElementById('now-playing').classList.toggle('idle', !np);
  const upNextChanged = (event?.upNext ?? []).join() !== (renderedUpNext ?? []).join();
  updateModeButtons();
  if (upNextChanged) renderUpNext();
  for (const row of document.querySelectorAll('.song-row.current')) row.classList.toggle('paused', !event?.playing);
  if (songChanged) {
    renderSongs();
    revealPlaying();
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

function markSelection() {
  for (const row of document.querySelectorAll('#song-spacer .song-row')) {
    row.classList.toggle('selected', selection.selected.has(shown[Number(row.dataset.index)]?.id));
  }
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
    const current = s.id === playerState?.songId;
    row.className = current ? 'song-row current' : 'song-row';
    if (current && !playerState?.playing) row.classList.add('paused');
    row.dataset.index = i;
    row.setAttribute('role', 'listitem');
    row.style.top = `${i * ROW_HEIGHT}px`;
    row.title = shown[i].path; // where the file is: tells copies apart
    if (selection.selected.has(s.id)) row.classList.add('selected');
    const main = document.createElement('span');
    main.className = 'song-main';
    main.append(cell(s.title, 'song-title'), cell(s.artist, 'song-artist'));
    const art = thumb(shown[i], 'song-thumb', 68);
    if (current) art.append(eqBars());
    row.append(art, main, cell(s.album, 'album'), cell(s.quality, 'quality'), cell(s.duration, 'num'), rateCell(shown[i].rating, activeFilter === 'disliked'));
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

// ---- Sync through iTunes (S.3) ----
let syncing = false;

function setupSync() {
  const $ = id => document.getElementById(id);
  window.__TAURI__.event.listen('sync-progress', e => {
    const s = stageText(e.payload);
    $('sync-status').hidden = false;
    $('sync-status').textContent = s.text;
  });
  $('sync-btn').addEventListener('click', () => {
    const phone = (lastDevices ?? []).find(d => d.trusted && d.udid);
    if (!phone || syncing) return;
    syncing = true;
    $('sync-btn').disabled = true;
    $('sync-btn').textContent = t('sync.syncing');
    $('sync-result').hidden = true;
    invoke('sync_now', { udid: phone.udid })
      .then(report => { const r = syncResult(report); showSyncOutcome(r.tone, null, r.lines); })
      .catch(err => { const s = syncStopped(err); showSyncOutcome('warn', s.title, s.lines); })
      .finally(() => {
        syncing = false;
        $('sync-btn').disabled = false;
        $('sync-btn').textContent = t('sync.button');
        $('sync-status').hidden = true;
        showSyncPreview();
      });
  });
}

function showSyncOutcome(tone, title, lines) {
  const box = document.getElementById('sync-result');
  box.className = `sync-result ${tone}`;
  const items = [];
  if (title) { const h = document.createElement('p'); h.className = 'sync-result-title'; h.textContent = title; items.push(h); }
  for (const line of lines) { const p = document.createElement('p'); p.textContent = line; items.push(p); }
  box.replaceChildren(...items);
  box.hidden = false;
}

function showSyncPreview() {
  for (const card of playerCards.values()) showPlayerPreview(card);
  const box = document.getElementById('sync-box');
  const phone = (lastDevices ?? []).find(d => d.trusted && d.udid);
  box.hidden = !phone;
  if (!phone) return;
  invoke('sync_preview', { udid: phone.udid, freeBytes: phone.info?.freeBytes ?? null }).then(p => {
    document.getElementById('sync-plan').textContent = planSummary(p.plan);
    document.getElementById('sync-guide').hidden = !showsGuide(p);
    const last = document.getElementById('sync-last');
    last.textContent = lastSyncLabel(p.lastRun, Math.floor(Date.now() / 1000));
    last.hidden = !p.lastRun;
    const short = document.getElementById('sync-shortage');
    short.hidden = !p.shortage;
    if (p.shortage) short.textContent = t('sync.noSpace', { need: formatBytes(p.shortage.need), available: formatBytes(p.shortage.available) });
  }).catch(() => { box.hidden = true; });
}

// ---- Music players: USB drives and memory cards ----
const playerCards = new Map(); // id → { el, player }
let playerSyncing = null; // id of the player being synced

function setupPlayers() {
  window.__TAURI__.event.listen('players', e => showPlayers(e.payload));
  window.__TAURI__.event.listen('player-sync-progress', e => {
    const card = playerCards.get(playerSyncing);
    if (!card) return;
    const status = card.el.querySelector('.player-status');
    status.hidden = false;
    status.textContent = playerStageText(e.payload).text;
  });
  document.getElementById('player-add').addEventListener('click', () => pickPlayerFolder());
  invoke('list_players').then(showPlayers).catch(() => {});
}

async function pickPlayerFolder(defaultPath) {
  const folder = await window.__TAURI__.dialog.open({ directory: true, multiple: false, defaultPath });
  if (!folder) return;
  const error = document.getElementById('player-add-error');
  error.hidden = true;
  invoke('add_player', { folder }).then(showPlayers).catch(reason => {
    error.textContent = t('player.addFailed', { reason });
    error.hidden = false;
  });
}

function showPlayers(players) {
  const list = document.getElementById('player-list');
  const seen = new Set();
  for (const p of players ?? []) {
    seen.add(p.id);
    let card = playerCards.get(p.id);
    if (!card) {
      const frag = document.getElementById('player-template').content.cloneNode(true);
      translatePage(frag);
      card = { el: frag.firstElementChild, player: p };
      wirePlayer(card);
      playerCards.set(p.id, card);
      list.append(card.el);
    }
    card.player = p;
    renderPlayer(card);
  }
  for (const [id, card] of playerCards) {
    if (!seen.has(id) && id !== playerSyncing) { card.el.remove(); playerCards.delete(id); }
  }
}

function renderPlayer(card) {
  const p = card.player;
  const c = playerCard(p);
  const el = card.el;
  el.querySelector('.device-name').textContent = c.title;
  el.querySelector('.device-detail').textContent = c.detail;
  el.querySelector('.player-suggested').hidden = !c.suggested;
  el.querySelector('.player-forget').hidden = c.suggested;
  // MTP players have no folders to browse to: Miu fills Music on the device.
  el.querySelector('.player-folder').hidden = c.mtp;
  const hint = el.querySelector('.player-hint');
  hint.hidden = !c.hint;
  hint.textContent = c.hint ?? '';
  const storage = el.querySelector('.storage');
  storage.hidden = !c.storage;
  if (c.storage) {
    storage.querySelector('.storage-used').style.width = `${c.storage.usedPercent}%`;
    storage.querySelector('.storage-label').textContent = c.storage.label;
  }
  el.querySelector('.player-target').replaceChildren(...targetOptions(p.profile.target).map(o => {
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    opt.selected = o.selected;
    return opt;
  }));
  el.querySelector('.player-plays').replaceChildren(...PLAYS.map(f => {
    const label = document.createElement('label');
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.value = f.value;
    box.checked = p.profile.plays.includes(f.value);
    label.append(box, f.label);
    return label;
  }));
  showPlayerPreview(card);
}

function savePlayer(card, profile) {
  invoke('save_player', { id: card.player.id, profile }).then(showPlayers).catch(() => renderPlayer(card));
}

function wirePlayer(card) {
  const el = card.el;
  el.querySelector('.player-target').addEventListener('change', e => savePlayer(card, { ...card.player.profile, target: e.target.value }));
  el.querySelector('.player-plays').addEventListener('change', e => savePlayer(card, withPlays(card.player.profile, e.target.value, e.target.checked)));
  el.querySelector('.player-folder').addEventListener('click', () => pickPlayerFolder(card.player.root));
  el.querySelector('.player-forget').addEventListener('click', () => invoke('forget_player', { id: card.player.id }).then(showPlayers));
  el.querySelector('.player-sync').addEventListener('click', () => {
    if (playerSyncing) return;
    playerSyncing = card.player.id;
    const button = el.querySelector('.player-sync');
    for (const b of document.querySelectorAll('.player-sync')) b.disabled = true;
    button.textContent = t('sync.syncing');
    el.querySelector('.player-result').hidden = true;
    invoke('player_sync_now', { id: card.player.id })
      .then(report => { const r = playerResult(report); showPlayerOutcome(card, r.tone, null, r.lines); })
      .catch(err => { const s = playerStopped(err); showPlayerOutcome(card, 'warn', s.title, s.lines); })
      .finally(() => {
        playerSyncing = null;
        for (const b of document.querySelectorAll('.player-sync')) b.disabled = false;
        button.textContent = t('sync.button');
        el.querySelector('.player-status').hidden = true;
        invoke('list_players').then(showPlayers);
      });
  });
}

function showPlayerOutcome(card, tone, title, lines) {
  const box = card.el.querySelector('.player-result');
  box.className = `sync-result player-result ${tone}`; // i18n-ignore: class names
  const items = [];
  if (title) { const h = document.createElement('p'); h.className = 'sync-result-title'; h.textContent = title; items.push(h); }
  for (const line of lines) { const p = document.createElement('p'); p.textContent = line; items.push(p); }
  box.replaceChildren(...items);
  box.hidden = false;
}

function showPlayerPreview(card) {
  if (playerSyncing === card.player.id) return;
  const el = card.el;
  invoke('player_sync_preview', { id: card.player.id }).then(p => {
    el.querySelector('.player-plan').textContent = planSummary(p.plan);
    const last = el.querySelector('.player-last');
    last.textContent = lastSyncLabel(p.lastRun, Math.floor(Date.now() / 1000));
    last.hidden = !p.lastRun;
    const short = el.querySelector('.player-shortage');
    short.hidden = !p.shortage;
    if (p.shortage) short.textContent = playerStopped(p.shortage).lines[0];
  }).catch(() => {});
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
  setupSecondOutput();
  invoke('output_state').then(renderDevices);
}

let outputState = null;

function renderDevices(state) {
  outputState = state;
  renderSecond(state);
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
  $('out-dual-status').textContent = ev ? dualStatus(ev) : '';
  $('out-align').setAttribute('aria-pressed', String(Boolean(ev?.aligning)));
  $('out-align').textContent = t(ev?.aligning ? 'output.alignStop' : 'output.align');
}

// ---- Second output (F7.9, experimental) ----

function setupSecondOutput() {
  const $ = id => document.getElementById(id);
  const fail = message => {
    $('out-error').textContent = String(message);
    setTimeout(() => { $('out-error').textContent = ''; }, 5000);
    invoke('output_state').then(renderDevices);
  };
  $('out-second').addEventListener('change', e => {
    $('out-second').disabled = true;
    invoke('set_second_output', { device: e.target.value || null })
      .then(renderDevices)
      .catch(fail)
      .finally(() => { $('out-second').disabled = false; });
  });
  $('out-mode').addEventListener('change', e => invoke('set_dual_mode', { mode: e.target.value }).then(renderDevices).catch(fail));
  const slider = $('out-offset');
  const show = () => { $('out-offset-val').textContent = offsetLabel(Number(slider.value)); };
  // Each change starts both outputs again together, so apply on release, and after a pause
  // when nudging (several quick presses count as one change).
  const apply = () => invoke('set_dual_delays', { delays: delaysFor(Number(slider.value)) }).catch(fail);
  let nudged = null;
  slider.addEventListener('input', show);
  slider.addEventListener('change', apply);
  for (const btn of document.querySelectorAll('[data-nudge]')) {
    btn.setAttribute('aria-label', t('output.nudge', { ms: btn.dataset.nudge }));
    btn.addEventListener('click', () => {
      slider.value = String(Number(slider.value) + Number(btn.dataset.nudge));
      show();
      clearTimeout(nudged);
      nudged = setTimeout(apply, 400);
    });
  }
  // A level change is instant (no restart), so it follows the slider while dragging.
  const balance = $('out-balance');
  let balanceAt = 0;
  let balanceLast = null;
  const sendBalance = () => invoke('set_dual_balance', { balance: Number(balance.value) }).catch(fail);
  balance.addEventListener('input', () => {
    $('out-balance-val').textContent = balanceLabel(Number(balance.value));
    clearTimeout(balanceLast);
    if (Date.now() - balanceAt > 100) { balanceAt = Date.now(); sendBalance(); }
    else balanceLast = setTimeout(sendBalance, 120);
  });
  balance.addEventListener('dblclick', () => {
    balance.value = '0';
    balance.dispatchEvent(new Event('input'));
  });
  $('out-align').addEventListener('click', () => {
    invoke('output_align', { on: !playerState?.aligning }).catch(fail);
  });
}

function renderSecond(state) {
  const $ = id => document.getElementById(id);
  $('out-second').replaceChildren(...secondOptions(state, playerState?.deviceName).map(o => {
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    opt.selected = o.selected;
    return opt;
  }));
  $('out-second').disabled = state.exclusive;
  $('out-second-exclusive').hidden = !state.exclusive;
  $('out-mode').value = state.dualMode;
  $('out-mode').disabled = !state.second || state.exclusive;
  $('out-dual').hidden = !state.second || state.exclusive;
  if (state.second) $('out-second-box').open = true;
  $('out-offset').value = String(offsetOf(state.delays));
  $('out-offset-val').textContent = offsetLabel(offsetOf(state.delays));
  $('out-balance').value = String(state.balance);
  $('out-balance-val').textContent = balanceLabel(state.balance);
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
  $('podcasts-view').hidden = name !== 'podcasts';
  $('show-page').hidden = name !== 'show';
  $('books-view').hidden = name !== 'books';
  $('book-page').hidden = name !== 'book';
  $('settings-view').hidden = name !== 'settings';
  $('empty-state').hidden = !library || hasSongs;
  $('song-view').hidden = name !== 'songs' || !hasSongs;
  $('albums-view').hidden = !['albums', 'artists'].includes(name) || !hasSongs;
  $('album-page').hidden = name !== 'page' || !hasSongs;
  const navOn = name === 'songs' ? { all: 'nav-songs', liked: 'nav-liked', disliked: 'nav-disliked' }[activeFilter] ?? 'nav-songs'
    : name === 'page' ? (page?.kind === 'artist' ? 'nav-artists' : 'nav-albums') : name === 'show' ? 'nav-podcasts'
    : name === 'books' || name === 'book' ? 'nav-audiobooks' : `nav-${name}`;
  for (const id of ['nav-songs', 'nav-albums', 'nav-artists', 'nav-podcasts', 'nav-audiobooks', 'nav-liked', 'nav-disliked', 'nav-settings']) $(id).classList.toggle('active', id === navOn);
  if (name === 'podcasts') loadPodcasts();
  if (name === 'books') loadBooks();
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
  $('set-podcast-cleanup').value = s.podcastCleanupDays == null ? 'never' : String(s.podcastCleanupDays);
}

function setupSettings(languageSetting) {
  const $ = id => document.getElementById(id);
  $('language').value = languageSetting;
  $('language').addEventListener('change', () => invoke('set_ui_language', { language: $('language').value }));
  const go = (id, fn) => $(id).addEventListener('click', e => { e.preventDefault(); fn(); });
  const songsWith = filter => () => { activeFilter = filter; refreshList(); showView('songs'); };
  go('nav-songs', songsWith('all'));
  $('nav-songs-play').addEventListener('click', () => playAll(filterSongs(rawSongs, 'all')));
  $('nav-liked-play').addEventListener('click', () => playAll(filterSongs(rawSongs, 'liked')));
  for (const b of [$('np-like'), $('np-dislike')]) {
    b.addEventListener('click', () => { if (playerState?.songId != null) rate(playerState.songId, b.dataset.act, true); });
  }
  go('nav-liked', songsWith('liked'));
  go('nav-disliked', songsWith('disliked'));
  go('nav-albums', () => showView('albums'));
  go('nav-artists', () => showView('artists'));
  go('nav-podcasts', () => showView('podcasts'));
  setupPodcasts();
  go('nav-audiobooks', () => showView('books'));
  setupBooks();
  setupSync();
  go('nav-settings', () => showView('settings'));
  $('album-back').addEventListener('click', () => showView(page?.kind === 'artist' ? 'artists' : 'albums'));
  $('album-play').addEventListener('click', () => { if (page) playList(page.songs.map(x => x.id), 0); });
  invoke('sync_rules').then(r => { syncRules = r; renderSyncRule(); }).catch(() => {});
  $('album-sync').addEventListener('click', () => {
    const rule = syncRuleFor(page);
    if (!rule) return;
    invoke('set_sync_rule', { rule, on: !hasSyncRule(syncRules, rule) })
      .then(r => { syncRules = r; renderSyncRule(); if (page) openPage(page.kind, { ...page, name: page.title }, false); });
  });
  $('album-art').addEventListener('click', () => {
    if (!needsArtwork(page)) return;
    const b = $('album-art');
    b.disabled = true;
    b.textContent = t('album.findingArt');
    invoke('find_album_art', { artist: page.artist, album: page.title })
      .then(found => { if (!found) b.textContent = t('album.noArt'); })
      .catch(() => { b.disabled = false; b.textContent = t('album.findArt'); });
  });
  let findingCovers = false;
  $('covers-find').addEventListener('click', () => {
    if (findingCovers) {
      invoke('cancel_art_fetch');
      $('covers-result').textContent = t('settings.coversStopping');
      return;
    }
    invoke('fetch_missing_art').then(n => {
      if (!n) { $('covers-result').textContent = t('settings.coversNone'); return; }
      findingCovers = true;
      $('covers-find').textContent = t('settings.coversStop');
      $('covers-result').textContent = t('settings.coversStarted', { n });
    });
  });
  window.__TAURI__.event.listen('library-progress', e => {
    if (e.payload.phase !== 'art' || e.payload.done < e.payload.total) return;
    findingCovers = false;
    $('covers-find').textContent = t('settings.coversFind');
    $('covers-result').textContent = t('settings.coversDone');
  });
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
  $('set-podcast-cleanup').addEventListener('change', e => {
    const v = e.target.value;
    invoke('set_app_setting', { name: 'podcastCleanupDays', value: v === 'never' ? null : Number(v) }).then(showSettings);
  });
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
let page = null; // the album or artist page on show: { kind, title, artist, songs, coverId, year }

function playList(ids, start) {
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
    // The cover opens the page; its round button plays the whole album or artist.
    const cover = document.createElement('div');
    cover.className = 'tile tile-cover';
    cover.setAttribute('role', 'button');
    cover.tabIndex = 0;
    cover.setAttribute('aria-label', artists ? name : `${name} — ${item.artist}`);
    cover.addEventListener('click', open);
    cover.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(); } });
    const play = document.createElement('button');
    play.type = 'button';
    play.className = 'tile-play';
    play.title = t('tile.play', { name });
    play.setAttribute('aria-label', play.title);
    play.append(playIcon());
    play.addEventListener('click', e => { e.stopPropagation(); playAll(item.songs); });
    cover.append(thumb(item.coverId != null ? { id: item.coverId, hasArtwork: true } : null, 'tile-art', 320), play);
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
  renderSyncRule();
  const findArt = $('album-art');
  findArt.hidden = !needsArtwork(page);
  findArt.disabled = false;
  findArt.textContent = t('album.findArt');
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
  const episode = playerState?.episodeId != null ? episodeInfo(playerState.episodeId) : null;
  if (episode) {
    img.hidden = !episode.artworkUrl;
    empty.hidden = Boolean(episode.artworkUrl);
    if (episode.artworkUrl) { img.referrerPolicy = 'no-referrer'; img.src = episode.artworkUrl; }
    return;
  }
  const has = Boolean(song?.hasArtwork);
  img.hidden = !has;
  empty.hidden = has;
  if (has) {
    img.onerror = () => { img.hidden = true; empty.hidden = false; };
    img.src = artUrl(song.id, 640);
  }
}

let renderedUpNext = null;

/** What plays next, in the engine's order (follows shuffle and repeat). */
function renderUpNext() {
  renderedUpNext = playerState?.upNext ?? [];
  const list = renderedUpNext.map(id => songsById.get(id)).filter(Boolean);
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

/** Thin bar while folders are checked or songs fingerprinted in the background. */
function showWork(p) {
  const w = workState(p);
  document.getElementById('work').hidden = !w.show;
  document.getElementById('work-fill').style.width = `${w.percent}%`;
  document.getElementById('work-text').textContent = w.text;
}

// ---- Podcasts (Phase 7, P.1): subscribe by feed address, shows and their episodes ----
let shows = [];
let openShow = null;

function setupPodcasts() {
  const $ = id => document.getElementById(id);
  $('podcast-add').addEventListener('submit', e => {
    e.preventDefault();
    const url = $('podcast-url').value.trim();
    if (!url) return;
    $('podcast-error').textContent = '';
    $('podcast-add-btn').disabled = true;
    invoke('podcast_subscribe', { url })
      .then(list => { $('podcast-url').value = ''; renderShows(list); })
      .catch(err => { $('podcast-error').textContent = t('podcast.failed', { reason: String(err) }); })
      .finally(() => { $('podcast-add-btn').disabled = false; });
  });
  const opmlFilter = [{ name: t('podcast.opmlFiles'), extensions: ['opml', 'xml'] }];
  $('podcast-import').addEventListener('click', async () => {
    const path = await window.__TAURI__.dialog.open({ multiple: false, filters: opmlFilter });
    if (!path) return;
    $('podcast-error').textContent = '';
    $('podcast-import').disabled = true;
    invoke('podcast_import_opml', { path })
      .then(({ report, shows: list }) => {
        renderShows(list);
        const failed = report.failed.length ? ` ${t('podcast.importFailed', { n: report.failed.length, list: report.failed.map(([url]) => url).join(', ') })}` : '';
        $('podcast-error').textContent = t('podcast.imported', { added: report.added, already: report.already }) + failed;
      })
      .catch(err => { $('podcast-error').textContent = t('podcast.failed', { reason: String(err) }); })
      .finally(() => { $('podcast-import').disabled = false; });
  });
  $('podcast-export').addEventListener('click', async () => {
    const path = await window.__TAURI__.dialog.save({ defaultPath: 'MiuMusic-podcasts.opml', filters: opmlFilter }); // i18n-ignore: file name
    if (!path) return;
    invoke('podcast_export_opml', { path })
      .then(n => { $('podcast-error').textContent = t('podcast.exported', { n, path }); })
      .catch(err => { $('podcast-error').textContent = String(err); });
  });
  $('show-back').addEventListener('click', () => showView('podcasts'));
  setupDiscover();
  const saveOptions = () => {
    if (!openShow) return;
    invoke('podcast_set_options', { id: openShow.id, autoDownload: $('show-auto').checked, keepLatest: Number($('show-keep').value) })
      .then(list => { shows = list; openShow = list.find(x => x.id === openShow.id) ?? openShow; });
  };
  $('show-auto').addEventListener('change', saveOptions);
  $('show-keep').addEventListener('change', saveOptions);
  window.__TAURI__.event.listen('podcasts-changed', e => {
    pendingDownloads.delete(e.payload);
    if (playWhenDownloaded === e.payload) {
      playWhenDownloaded = null;
      invoke('play_episode', { id: e.payload }).catch(() => {});
    }
    if (view === 'show' && openShow) loadPodcasts().then(() => openShowPage(openShow.id, false));
    else if (view === 'podcasts') loadPodcasts();
  });
  window.__TAURI__.event.listen('podcast-download-failed', e => {
    pendingDownloads.delete(e.payload.id);
    $('show-facts').textContent = t('podcast.downloadFailed', { reason: e.payload.error });
  });
  $('show-refresh').addEventListener('click', () => {
    if (!openShow) return;
    $('show-refresh').disabled = true;
    invoke('podcast_refresh', { id: openShow.id })
      .then(n => { $('show-facts').textContent = t('podcast.newEpisodes', { n }); return loadPodcasts().then(() => openShowPage(openShow.id, false)); })
      .catch(err => { $('show-facts').textContent = t('podcast.failed', { reason: String(err) }); })
      .finally(() => { $('show-refresh').disabled = false; });
  });
  $('show-unsubscribe').addEventListener('click', () => {
    if (!openShow) return;
    invoke('podcast_unsubscribe', { id: openShow.id }).then(list => { renderShows(list); showView('podcasts'); });
  });
}

function loadPodcasts() {
  return invoke('podcast_list').then(renderShows);
}

function remoteCover(url, cls) {
  const box = document.createElement('span');
  box.className = cls;
  if (url) {
    const img = document.createElement('img');
    img.className = 'remote';
    img.alt = '';
    img.loading = 'lazy';
    img.referrerPolicy = 'no-referrer';
    img.src = url;
    img.addEventListener('error', () => { box.innerHTML = NOTE; }, { once: true }); // constant markup
    box.append(img);
  } else {
    box.innerHTML = NOTE; // constant markup
  }
  return box;
}

function renderShows(list) {
  shows = list;
  const $ = id => document.getElementById(id);
  $('podcasts-summary').textContent = t('podcast.count', { n: list.length });
  $('podcast-empty').hidden = list.length > 0;
  const box = $('podcast-tiles');
  const cols = Math.max(2, Math.floor(box.clientWidth / 170 / 2) * 2) || 4;
  box.style.setProperty('--tile-cols', cols);
  const cells = [];
  list.forEach((s, i) => {
    const row = Math.floor((i * 2) / cols);
    const open = () => openShowPage(s.id);
    const cover = document.createElement('button');
    cover.type = 'button';
    cover.className = 'tile tile-cover';
    cover.setAttribute('aria-label', s.title);
    cover.addEventListener('click', open);
    cover.append(remoteCover(s.artworkUrl, 'tile-art'));
    const info = document.createElement('button');
    info.type = 'button';
    info.className = 'tile tile-info';
    info.addEventListener('click', open);
    const [l1, l2] = splitTitle(s.title);
    const stat = (label, value) => {
      const span = document.createElement('span');
      const small = document.createElement('small');
      small.className = 'stat-label';
      small.textContent = label;
      const b = document.createElement('b');
      b.textContent = value;
      span.append(small, b);
      return span;
    };
    const stats = document.createElement('span');
    stats.className = 'tile-stats';
    stats.append(stat(t('podcast.episodesLabel'), s.episodes), stat(t('podcast.newLabel'), s.unplayed));
    info.append(cell(l1, 'tile-line1'), cell(l2, 'tile-line2'), cell(s.author ?? '', 'tile-sub'), stats);
    cells.push(...(row % 2 === 0 ? [cover, info] : [info, cover]));
  });
  box.replaceChildren(...cells);
}

function openShowPage(id, scroll = true) {
  const s = shows.find(x => x.id === id);
  if (!s) return Promise.resolve();
  openShow = s;
  const $ = el => document.getElementById(el);
  const [l1, l2] = splitTitle(s.title);
  $('show-line1').textContent = l1;
  $('show-line2').textContent = l2;
  $('show-author').textContent = s.author ?? '';
  $('show-cover').replaceChildren(remoteCover(s.artworkUrl, 'hero-art'));
  const bg = $('show-bg');
  bg.hidden = !s.artworkUrl;
  if (s.artworkUrl) { bg.referrerPolicy = 'no-referrer'; bg.src = s.artworkUrl; }
  return invoke('podcast_episodes', { id }).then(eps => {
    if (scroll) $('show-facts').textContent = t('podcast.facts', { n: eps.length, unplayed: eps.filter(e => !e.played).length });
    $('show-auto').checked = Boolean(s.autoDownload);
    $('show-keep').value = String(s.keepLatest ?? 3);
    $('episode-list').replaceChildren(...eps.map(e => {
      const li = document.createElement('li');
      li.className = e.played ? 'episode played' : 'episode';
      const title = cell(e.title, 'episode-title');
      const meta = cell(episodeMeta(e, language()), 'episode-meta');
      li.append(title, episodeActions(e), meta);
      return li;
    }));
    showView('show');
    if (scroll) $('show-page').scrollTop = 0;
  });
}

// ---- Offline episodes (P.3) ----
const pendingDownloads = new Set();

function episodeActions(e) {
  const box = document.createElement('span');
  box.className = 'episode-actions';
  const button = (key, fn) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'eq-btn';
    b.textContent = t(key);
    b.addEventListener('click', fn);
    return b;
  };
  const refreshShow = () => openShow && openShowPage(openShow.id, false);
  const state = episodeState(e, pendingDownloads);
  if (state === 'download') {
    box.append(button('podcast.downloadPlay', () => {
      pendingDownloads.add(e.id);
      playWhenDownloaded = e.id;
      invoke('podcast_download', { id: e.id });
      refreshShow();
    }), button('podcast.download', () => {
      pendingDownloads.add(e.id);
      invoke('podcast_download', { id: e.id });
      refreshShow();
    }));
  } else if (state === 'downloading') {
    box.append(cell(t('podcast.downloading'), 'episode-state'));
  } else {
    const resume = !e.played && e.positionMs > 5000;
    const play = document.createElement('button');
    play.type = 'button';
    play.className = 'primary';
    play.textContent = resume ? t('podcast.resume', { at: formatDuration(e.positionMs) }) : t('podcast.play');
    play.addEventListener('click', () => invoke('play_episode', { id: e.id }));
    box.append(play, button('podcast.removeDownload', () => invoke('podcast_delete_download', { id: e.id }).then(refreshShow)));
  }
  box.append(button(e.played ? 'podcast.markUnplayed' : 'podcast.markPlayed', () => invoke('podcast_set_played', { id: e.id, played: !e.played }).then(() => loadPodcasts()).then(refreshShow)));
  return box;
}

// ---- Discover (P.4): Apple charts for Vietnam and the US, and search ----
let discoverCountry = 'vn';
let discoverTerm = '';
let discovered = [];

function setupDiscover() {
  const $ = id => document.getElementById(id);
  const tab = discover => {
    $('discover').hidden = !discover;
    $('mine').hidden = discover;
    $('tab-mine').classList.toggle('active', !discover);
    $('tab-discover').classList.toggle('active', discover);
    $('tab-mine').setAttribute('aria-selected', String(!discover));
    $('tab-discover').setAttribute('aria-selected', String(discover));
    if (discover) loadDiscover();
  };
  $('tab-mine').addEventListener('click', () => tab(false));
  $('tab-discover').addEventListener('click', () => tab(true));
  for (const b of document.querySelectorAll('[data-country]')) {
    b.addEventListener('click', () => {
      discoverCountry = b.dataset.country;
      for (const x of document.querySelectorAll('[data-country]')) x.classList.toggle('active', x === b);
      loadDiscover();
    });
  }
  $('discover-search').addEventListener('submit', e => {
    e.preventDefault();
    discoverTerm = $('discover-term').value.trim();
    loadDiscover();
  });
  $('discover-term').addEventListener('input', () => {
    if (!$('discover-term').value.trim() && discoverTerm) { discoverTerm = ''; loadDiscover(); }
  });
}

function loadDiscover() {
  const $ = id => document.getElementById(id);
  $('discover-status').textContent = t('podcast.loading');
  const call = discoverTerm
    ? invoke('podcast_search', { term: discoverTerm, country: discoverCountry })
    : invoke('podcast_chart', { country: discoverCountry });
  return call
    .then(found => {
      discovered = found;
      $('discover-status').textContent = discoverTerm
        ? t('podcast.results', { n: found.length, term: discoverTerm })
        : t(discoverCountry === 'vn' ? 'podcast.topVn' : 'podcast.topUs');
      renderDiscover();
    })
    .catch(err => { $('discover-status').textContent = t('podcast.failed', { reason: String(err) }); });
}

function renderDiscover() {
  const box = document.getElementById('discover-tiles');
  const cols = Math.max(2, Math.floor(box.clientWidth / 170 / 2) * 2) || 4;
  box.style.setProperty('--tile-cols', cols);
  const cells = [];
  markSubscribed(discovered, shows).forEach((f, i) => {
    const row = Math.floor((i * 2) / cols);
    const cover = document.createElement('div');
    cover.className = 'tile tile-cover';
    cover.append(remoteCover(f.artworkUrl, 'tile-art'));
    const info = document.createElement('div');
    info.className = 'tile tile-info';
    const [l1, l2] = splitTitle(f.title);
    info.append(cell(l1, 'tile-line1'), cell(l2, 'tile-line2'), cell(f.author, 'tile-sub'), cell(f.genre ?? '', 'tile-genre'));
    if (f.subscribed) {
      info.append(cell(t('podcast.alreadySubscribed'), 'subscribed'));
    } else {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'primary subscribe';
      b.textContent = t('podcast.subscribe');
      b.addEventListener('click', () => {
        b.disabled = true;
        invoke('podcast_subscribe_found', { appleId: f.appleId, feedUrl: f.feedUrl ?? null })
          .then(list => { shows = list; renderShows(list); renderDiscover(); })
          .catch(err => { b.disabled = false; document.getElementById('discover-status').textContent = t('podcast.failed', { reason: String(err) }); });
      });
      info.append(b);
    }
    cells.push(...(row % 2 === 0 ? [cover, info] : [info, cover]));
  });
  box.replaceChildren(...cells);
}

// ---- Podcast playback (P.5) ----
let playWhenDownloaded = null;
const episodes = new Map(); // id → { title, artist, durationMs, artworkUrl } once fetched

/** Title, show and artwork of the playing episode (fetched once, then the player view redraws). */
function episodeInfo(id) {
  if (episodes.has(id)) return episodes.get(id);
  episodes.set(id, { title: '', artist: '', durationMs: null, artworkUrl: null });
  invoke('podcast_now', { id }).then(n => {
    episodes.set(id, { title: n.title, artist: n.show, durationMs: n.durationMs, artworkUrl: n.artworkUrl });
    if (playerState?.episodeId === id) { showPlayer(playerState); renderNowArt(); }
  }).catch(() => {});
  return episodes.get(id);
}

// ---- Always sync an artist or album ----
let syncRules = [];

function renderSyncRule() {
  const button = document.getElementById('album-sync');
  const rule = syncRuleFor(page);
  button.hidden = !rule;
  const on = hasSyncRule(syncRules, rule);
  button.setAttribute('aria-pressed', String(on));
  document.getElementById('album-sync-label').textContent = t(on ? 'album.alwaysSyncOn' : 'album.alwaysSync');
}

// ---- Taskbar preview buttons (previous, play/pause, next) ----
let taskbarPlaying = false;

function setupTaskbar() {
  const labels = { prev: t('player.prev'), play: t('player.play'), pause: t('player.pause'), next: t('player.next') };
  invoke('taskbar_init', { labels }).catch(() => {});
  const target = { prev: 'np-prev', play: 'np-play', next: 'np-next' };
  window.__TAURI__.event.listen('taskbar-button', e => document.getElementById(target[e.payload])?.click());
}

// ---- Audiobooks (Phase 8): LibriVox books; chapters download and play like podcast episodes ----
let books = [];
let openBook = null;
let bookTerm = '';
let bookLanguage = '';
let foundBooks = [];
let bookFindLoaded = false;

function setupBooks() {
  const $ = id => document.getElementById(id);
  const tab = find => {
    $('book-find').hidden = !find;
    $('book-mine').hidden = find;
    $('tab-books-mine').classList.toggle('active', !find);
    $('tab-books-find').classList.toggle('active', find);
    $('tab-books-mine').setAttribute('aria-selected', String(!find));
    $('tab-books-find').setAttribute('aria-selected', String(find));
    if (find && !bookFindLoaded) loadFoundBooks();
  };
  $('tab-books-mine').addEventListener('click', () => tab(false));
  $('tab-books-find').addEventListener('click', () => tab(true));
  $('book-language').replaceChildren(...BOOK_LANGUAGES.map(l => {
    const o = document.createElement('option');
    o.value = l;
    o.textContent = languageName(l);
    return o;
  }));
  $('book-language').addEventListener('change', () => { bookLanguage = $('book-language').value; loadFoundBooks(); });
  $('book-search').addEventListener('submit', e => {
    e.preventDefault();
    bookTerm = $('book-term').value.trim();
    loadFoundBooks();
  });
  $('book-term').addEventListener('input', () => {
    if (!$('book-term').value.trim() && bookTerm) { bookTerm = ''; loadFoundBooks(); }
  });
  $('book-back').addEventListener('click', () => showView('books'));
  $('book-play').addEventListener('click', () => {
    if (!openBook) return;
    if (bookAction(openBook) === 'download') {
      invoke('audiobook_download', { id: openBook.id }).then(() => openBookPage(openBook.id, false));
      return;
    }
    invoke('play_chapters', { book: openBook.id, start: null }).catch(err => { $('book-facts').textContent = String(err); });
  });
  $('book-remove-download').addEventListener('click', () => {
    if (!openBook) return;
    invoke('audiobook_remove_download', { id: openBook.id }).then(list => { books = list; openBookPage(openBook.id, false); });
  });
  $('book-remove').addEventListener('click', () => {
    if (!openBook) return;
    invoke('audiobook_remove', { id: openBook.id }).then(list => { renderBooks(list); showView('books'); });
  });
  window.__TAURI__.event.listen('podcasts-changed', () => {
    if (view === 'book' && openBook) loadBooks().then(() => openBookPage(openBook.id, false));
    else if (view === 'books') loadBooks();
  });
  window.__TAURI__.event.listen('podcast-download-failed', e => {
    if (view === 'book') $('book-facts').textContent = t('podcast.downloadFailed', { reason: e.payload.error });
  });
}

function loadBooks() {
  return invoke('audiobook_list').then(renderBooks);
}

function renderBooks(list) {
  books = list;
  const $ = id => document.getElementById(id);
  $('books-summary').textContent = t('book.count', { n: list.length });
  $('book-empty').hidden = list.length > 0;
  const box = $('book-tiles');
  const cols = Math.max(2, Math.floor(box.clientWidth / 170 / 2) * 2) || 4;
  box.style.setProperty('--tile-cols', cols);
  const cells = [];
  list.forEach((b, i) => {
    const row = Math.floor((i * 2) / cols);
    const open = () => openBookPage(b.id);
    const cover = document.createElement('button');
    cover.type = 'button';
    cover.className = 'tile tile-cover';
    cover.setAttribute('aria-label', b.title);
    cover.addEventListener('click', open);
    cover.append(remoteCover(b.artworkUrl, 'tile-art'));
    const info = document.createElement('button');
    info.type = 'button';
    info.className = 'tile tile-info';
    info.addEventListener('click', open);
    const [l1, l2] = splitTitle(b.title);
    const stat = (label, value) => {
      const span = document.createElement('span');
      const small = document.createElement('small');
      small.className = 'stat-label';
      small.textContent = label;
      const v = document.createElement('b');
      v.textContent = value;
      span.append(small, v);
      return span;
    };
    const stats = document.createElement('span');
    stats.className = 'tile-stats';
    stats.append(stat(t('book.chaptersLabel'), b.chapters), stat(t('book.listenedLabel'), b.finished));
    info.append(cell(l1, 'tile-line1'), cell(l2, 'tile-line2'), cell(b.author ?? '', 'tile-sub'), stats);
    cells.push(...(row % 2 === 0 ? [cover, info] : [info, cover]));
  });
  box.replaceChildren(...cells);
}

function openBookPage(id, scroll = true) {
  const b = books.find(x => x.id === id);
  if (!b) return Promise.resolve();
  openBook = b;
  const $ = el => document.getElementById(el);
  const [l1, l2] = splitTitle(b.title);
  $('book-line1').textContent = l1;
  $('book-line2').textContent = l2;
  $('book-author').textContent = b.author ?? '';
  $('book-cover').replaceChildren(remoteCover(b.artworkUrl, 'hero-art'));
  const bg = $('book-bg');
  bg.hidden = !b.artworkUrl;
  if (b.artworkUrl) { bg.referrerPolicy = 'no-referrer'; bg.src = b.artworkUrl; }
  const action = bookAction(b);
  $('book-play').textContent = action === 'resume'
    ? t('book.resume', { chapter: b.resumeTitle ?? '', at: formatDuration(b.resumeMs) })
    : t(`book.${action}`);
  $('book-remove-download').hidden = !b.downloaded;
  $('book-facts').textContent = [bookProgress(b), lengthLabel(b.durationMs)].filter(Boolean).join(' · ');
  return Promise.all([invoke('audiobook_chapters', { id }), invoke('podcast_pending_downloads').catch(() => [])]).then(([chapters, pending]) => {
    for (const p of pending) pendingDownloads.add(p);
    $('chapter-list').replaceChildren(...chapters.map((c, i) => {
      const li = document.createElement('li');
      li.className = ['episode', c.played ? 'played' : '', c.id === b.resumeChapter ? 'current' : ''].filter(Boolean).join(' ');
      const title = cell(c.title || t('book.chapterNumber', { n: i + 1 }), 'episode-title');
      const meta = cell(lengthLabel(c.durationMs), 'episode-meta');
      li.append(title, chapterActions(b, c), meta);
      return li;
    }));
    showView('book');
    if (scroll) $('book-page').scrollTop = 0;
  });
}

function chapterActions(b, c) {
  const box = document.createElement('span');
  box.className = 'episode-actions';
  const button = (text, fn, cls = 'eq-btn') => {
    const el = document.createElement('button');
    el.type = 'button';
    el.className = cls;
    el.textContent = text;
    el.addEventListener('click', fn);
    return el;
  };
  const refresh = () => loadBooks().then(() => openBookPage(b.id, false));
  if (c.downloadPath) {
    const resume = !c.played && c.positionMs > 5000;
    box.append(button(resume ? t('podcast.resume', { at: formatDuration(c.positionMs) }) : t('podcast.play'),
      () => invoke('play_chapters', { book: b.id, start: c.id }), 'primary'));
  } else if (episodeState(c, pendingDownloads) === 'downloading') {
    box.append(cell(t('podcast.downloading'), 'episode-state'));
  } else {
    box.append(button(t('podcast.download'), () => { pendingDownloads.add(c.id); invoke('podcast_download', { id: c.id }); refresh(); }));
  }
  box.append(button(t(c.played ? 'podcast.markUnplayed' : 'podcast.markPlayed'),
    () => invoke('podcast_set_played', { id: c.id, played: !c.played }).then(refresh)));
  return box;
}

function loadFoundBooks() {
  const $ = id => document.getElementById(id);
  bookFindLoaded = true;
  $('book-status').textContent = t('podcast.loading');
  const call = bookTerm
    ? invoke('audiobook_search', { term: bookTerm, language: bookLanguage })
    : invoke('audiobook_recent', { language: bookLanguage });
  const asked = [bookTerm, bookLanguage].join('|');
  return call
    .then(found => {
      if (asked !== [bookTerm, bookLanguage].join('|')) return; // a newer search is on its way
      foundBooks = found;
      const lang = languageName(bookLanguage);
      $('book-status').textContent = !found.length
        ? (bookLanguage ? t('book.noneIn', { language: lang }) : t('book.none'))
        : bookTerm ? t('book.results', { n: found.length, term: bookTerm })
        : bookLanguage ? t('book.recentIn', { language: lang }) : t('book.recent');
      renderFoundBooks();
    })
    .catch(err => { $('book-status').textContent = t('book.failed', { reason: String(err) }); });
}

function renderFoundBooks() {
  const box = document.getElementById('book-found-tiles');
  const cols = Math.max(2, Math.floor(box.clientWidth / 170 / 2) * 2) || 4;
  box.style.setProperty('--tile-cols', cols);
  const cells = [];
  markAdded(foundBooks, books).forEach((f, i) => {
    const row = Math.floor((i * 2) / cols);
    const cover = document.createElement('div');
    cover.className = 'tile tile-cover';
    cover.append(remoteCover(f.coverUrl, 'tile-art'));
    const info = document.createElement('div');
    info.className = 'tile tile-info';
    info.title = f.description ?? '';
    const [l1, l2] = splitTitle(f.title);
    info.append(cell(l1, 'tile-line1'), cell(l2, 'tile-line2'), cell(bookMeta(f), 'tile-sub'));
    if (f.added) {
      info.append(cell(t('book.added'), 'subscribed'));
    } else {
      const add = document.createElement('button');
      add.type = 'button';
      add.className = 'primary subscribe';
      add.textContent = t('book.add');
      add.addEventListener('click', () => {
        add.disabled = true;
        add.textContent = t('book.adding');
        invoke('audiobook_add', { book: f })
          .then(list => { renderBooks(list); renderFoundBooks(); })
          .catch(err => {
            add.disabled = false;
            add.textContent = t('book.add');
            document.getElementById('book-status').textContent = t('book.failed', { reason: String(err) });
          });
      });
      info.append(add);
    }
    cells.push(...(row % 2 === 0 ? [cover, info] : [info, cover]));
  });
  box.replaceChildren(...cells);
}
