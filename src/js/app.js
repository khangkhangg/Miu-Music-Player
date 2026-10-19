import { resolveTheme } from './theme.js';
import { prereqMessage } from './prereq.js';
import { deviceCard } from './device.js';
import { folderItems, librarySummary } from './folders.js';
import { songRow, visibleRange } from './songs.js';

const ROW_HEIGHT = 36;
let songs = [];

const media = window.matchMedia('(prefers-color-scheme: dark)');
const preference = 'system'; // becomes a setting in F5.5

function applyTheme() {
  document.documentElement.dataset.theme = resolveTheme(preference, media.matches);
}
applyTheme();
media.addEventListener('change', applyTheme);

const invoke = window.__TAURI__?.core?.invoke;
if (invoke) {
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
  window.__TAURI__.event.listen('tauri://drag-drop', e => e.payload.paths.forEach(addFolder));
  document.getElementById('add-folder').addEventListener('click', pickFolder);
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
    songs = list.map(songRow);
    document.getElementById('song-spacer').style.height = `${songs.length * ROW_HEIGHT}px`;
    renderSongs();
  });
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
    row.className = 'song-row';
    row.setAttribute('role', 'listitem');
    row.style.top = `${i * ROW_HEIGHT}px`;
    row.append(cell(s.title, 'song-title'), cell(s.artist, 'muted'), cell(s.album, 'muted'), cell(s.duration, 'num muted'));
    rows.push(row);
  }
  spacer.replaceChildren(...rows);
}

function removeFolder(id) {
  invoke('remove_folder', { id }).then(f => { showFolders(f); loadSongs(); }).catch(showFolderError);
}

function showFolderError(message) {
  const el = document.getElementById('folder-error');
  el.textContent = `Couldn't add that folder: ${message}`;
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
    remove.title = 'Stop watching (songs stay on disk)';
    remove.setAttribute('aria-label', `Stop watching ${item.name}`);
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
