// Party mode on screen (P.5): the DJ's Guests panel (Wi-Fi page, approval, the queue), the QR
// code and queue on the party screen, and YouTube videos in YouTube's own player.

import { t } from './i18n.js';
import { videoUrl, videoEnded, listenMessage, entryLines, partyLimitOptions, relayNote, partyQr } from './party.js';

const POLL_MS = 1000;

export function setupParty({ invoke, listen }) {
  const $ = (id) => document.getElementById(id);
  const dj = $('dj-mode');
  const screen = $('dj-party-screen');
  const panel = $('dj-guests-panel');
  let info = { localOn: false };
  let view = null;
  let openedForVideo = false;

  const setPanel = (open) => { panel.hidden = !open; $('dj-guests-btn').setAttribute('aria-expanded', String(open)); };
  $('dj-guests-btn').addEventListener('click', () => setPanel(panel.hidden));
  $('dj-guests-close').addEventListener('click', () => setPanel(false));
  $('party-limit').replaceChildren(...partyLimitOptions().map((o) => new Option(o.label, String(o.value))));

  function showInfo(i) {
    info = i;
    $('party-wifi').checked = i.localOn;
    $('party-url').textContent = i.localUrl ?? (i.localOn ? '' : t('party.wifiOff'));
    $('party-relay').checked = i.relayOn;
    $('party-relay-url').textContent = relayNote(i);
    const qr = partyQr(i);
    if (qr !== shownQr) {
      $('dj-party-qr').innerHTML = qr ?? '';
      shownQr = qr;
    }
    $('dj-party-guests').hidden = !qr;
  }
  let shownQr = null;

  $('party-wifi').addEventListener('change', (e) => {
    invoke('party_set_local', { on: e.target.checked }).then(showInfo).catch((err) => {
      e.target.checked = false;
      $('party-url').textContent = String(err);
    });
  });
  $('party-relay').addEventListener('change', (e) => {
    invoke('party_set_relay', { on: e.target.checked }).then(showInfo).catch((err) => {
      e.target.checked = false;
      $('party-relay-url').textContent = String(err);
    });
  });
  const sendSettings = () => invoke('party_set_settings', {
    settings: { open: $('party-open').checked, approval: $('party-approval').checked, perGuest: Number($('party-limit').value) },
  }).catch(() => {});
  for (const id of ['party-open', 'party-approval', 'party-limit']) $(id).addEventListener('change', sendSettings);

  function entryRow(e, actions) {
    const li = document.createElement('li');
    const row = document.createElement('div');
    row.className = 'row';
    const text = document.createElement('div');
    const l = entryLines(e);
    const title = document.createElement('b');
    if (l.video) {
      const tag = document.createElement('span');
      tag.className = 'vtag';
      tag.textContent = t('party.video');
      title.append(tag);
    }
    title.append(document.createTextNode(l.title));
    const sub = document.createElement('small');
    sub.textContent = [l.sub, t('party.votes', { n: l.votes })].filter(Boolean).join(' · ');
    text.append(title, document.createElement('br'), sub);
    const acts = document.createElement('div');
    acts.className = 'acts';
    for (const [key, command] of actions) {
      const b = document.createElement('button');
      b.type = 'button';
      b.textContent = t(key);
      b.addEventListener('click', () => invoke(command, { id: e.id }).then(poll).catch(() => {}));
      acts.append(b);
    }
    row.append(text, acts);
    li.append(row);
    return li;
  }

  function render(v, autoDj) {
    view = v;
    const busy = document.activeElement;
    if (busy !== $('party-open')) $('party-open').checked = v.settings.open;
    if (busy !== $('party-approval')) $('party-approval').checked = v.settings.approval;
    if (busy !== $('party-limit')) $('party-limit').value = String(v.settings.perGuest);
    $('party-autodj').hidden = autoDj || (!v.queue.length && !v.waiting.length);
    const badge = $('dj-guests-badge');
    const waiting = v.waiting.length;
    badge.hidden = !waiting;
    badge.textContent = String(waiting);
    $('party-waiting').replaceChildren(...v.waiting.map((e) => entryRow(e, [['party.approve', 'party_approve'], ['party.remove', 'party_remove']])));
    const empty = document.createElement('li');
    empty.className = 'muted';
    empty.textContent = t('party.empty');
    $('party-queue').replaceChildren(...(v.queue.length ? v.queue.map((e) => entryRow(e, [['party.remove', 'party_remove']])) : [empty]));
    // The party screen shows the top of the queue under the QR code.
    $('dj-party-queue').replaceChildren(...v.queue.slice(0, 5).map((e) => {
      const li = document.createElement('li');
      li.textContent = e.title;
      const s = document.createElement('small');
      s.textContent = t('party.votes', { n: e.votes });
      li.append(s);
      return li;
    }));
  }

  async function poll() {
    if (dj.hidden) return;
    const [v, s] = await Promise.all([invoke('party_host_state').catch(() => null), invoke('dj_state').catch(() => null)]);
    if (v) render(v, !!s?.autoDj);
    // The relay opens its room in the background: follow its link and errors.
    if (info.relayOn) invoke('party_info').then(showInfo).catch(() => {});
  }
  setInterval(poll, POLL_MS);

  // ---- Videos: YouTube's player full-screen on the party screen ----

  const box = $('dj-party-video');
  function showVideo(videoId, title) {
    if (screen.hidden) {
      screen.hidden = false;
      openedForVideo = true;
    }
    screen.classList.add('video');
    const frame = document.createElement('iframe');
    frame.src = videoUrl(videoId, location.origin);
    frame.title = title;
    frame.allow = 'autoplay; encrypted-media; fullscreen; picture-in-picture';
    frame.allowFullscreen = true;
    // Ask the player to report its state, so the end of the video hands back to Auto DJ.
    frame.addEventListener('load', () => {
      for (const delay of [500, 1500, 3000]) setTimeout(() => frame.contentWindow?.postMessage(listenMessage(), 'https://www.youtube.com'), delay);
    });
    box.replaceChildren(frame);
    box.hidden = false;
    $('party-video-title').textContent = title;
    $('party-video-row').hidden = false;
  }

  function endVideo() {
    if (box.hidden) return;
    box.replaceChildren();
    box.hidden = true;
    screen.classList.remove('video');
    $('party-video-row').hidden = true;
    if (openedForVideo) {
      screen.hidden = true;
      openedForVideo = false;
    }
    invoke('party_video_done').catch(() => {});
  }

  window.addEventListener('message', (m) => { if (videoEnded(m)) endVideo(); });
  $('party-skip').addEventListener('click', endVideo);
  listen('party-video', (e) => showVideo(e.payload.videoId, e.payload.title));

  return {
    /** DJ mode opened: catch up with the party (a video may be playing already). */
    async opened() {
      showInfo(await invoke('party_info').catch(() => ({ localOn: false })));
      const v = await invoke('party_video_now').catch(() => null);
      if (v && box.hidden) showVideo(v.videoId, v.title);
      poll();
    },
    get info() {
      return info;
    },
    get view() {
      return view;
    },
  };
}
