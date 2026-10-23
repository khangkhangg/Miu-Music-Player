// Miu Party guest page: the queue (most votes first), voting, adding songs from the DJ's library
// or YouTube links. Served by Miu on the Wi-Fi at /p/<token>/ (API at ./api/) and by the relay site
// at /party/?r=ROOM (API at ./api.php).
(() => {
  'use strict';

  const STRINGS = {
    en: {
      nowPlaying: 'Now playing', nothing: 'Waiting for the first song', queue: 'Queue', add: 'Add a song',
      empty: 'Nothing queued yet. Add a song!', searchLabel: 'Search the DJ’s songs', linkLabel: 'Or paste a link',
      addLink: 'Add', linkHint: 'YouTube links play as videos on the party screen (great for karaoke). Spotify and Apple Music links are looked up in the DJ’s songs.',
      welcome: 'Welcome to the party!', yourName: 'Your name', join: 'Join', video: 'Video', by: 'added by {name}',
      added: 'Added!', voted: 'Voted', closed: 'The DJ has closed the queue for now.', limit: 'You have enough songs waiting. Vote for others!',
      justPlayed: 'That one just played.', notInLibrary: 'That song is not in the DJ’s library. Try a YouTube link.',
      unknownLink: 'Paste a YouTube, Spotify or Apple Music link.', offline: 'Lost the party, reconnecting…',
      waiting: '{n} of your songs waiting for the DJ', approval: 'The DJ approves new songs.', closedNote: 'The queue is closed right now.',
      vote: 'Vote', sent: 'Sent to the DJ!', relayOff: 'The party link is off right now. Ask the DJ.', roomGone: 'This party has ended.',
      slowDown: 'Too many tries. Wait a moment.', busy: 'The DJ is busy. Try again in a moment.', relayDown: 'The party link is not working. Try again soon.',
    },
    vi: {
      nowPlaying: 'Đang phát', nothing: 'Chờ bài đầu tiên', queue: 'Danh sách chờ', add: 'Thêm bài',
      empty: 'Chưa có bài nào. Thêm một bài đi!', searchLabel: 'Tìm trong bài của DJ', linkLabel: 'Hoặc dán đường dẫn',
      addLink: 'Thêm', linkHint: 'Đường dẫn YouTube phát dạng video trên màn hình tiệc (hát karaoke rất hợp). Đường dẫn Spotify và Apple Music được tìm trong bài của DJ.',
      welcome: 'Chào mừng đến bữa tiệc!', yourName: 'Tên của bạn', join: 'Vào', video: 'Video', by: '{name} đã thêm',
      added: 'Đã thêm!', voted: 'Đã bình chọn', closed: 'DJ đang tạm đóng danh sách.', limit: 'Bạn đã có đủ bài đang chờ. Hãy bình chọn bài khác!',
      justPlayed: 'Bài này vừa phát xong.', notInLibrary: 'Bài này không có trong thư viện của DJ. Thử đường dẫn YouTube.',
      unknownLink: 'Dán đường dẫn YouTube, Spotify hoặc Apple Music.', offline: 'Mất kết nối, đang kết nối lại…',
      waiting: '{n} bài của bạn đang chờ DJ duyệt', approval: 'DJ duyệt bài mới.', closedNote: 'Danh sách đang đóng.',
      vote: 'Bình chọn', sent: 'Đã gửi cho DJ!', relayOff: 'Đường dẫn tiệc đang tắt. Hỏi DJ nhé.', roomGone: 'Bữa tiệc đã kết thúc.',
      slowDown: 'Thử nhiều quá. Chờ một chút nhé.', busy: 'DJ đang bận. Thử lại sau giây lát.', relayDown: 'Đường dẫn tiệc đang lỗi. Thử lại sau nhé.',
    },
  };
  const lang = (navigator.language || 'en').toLowerCase().startsWith('vi') ? 'vi' : 'en';
  const tr = (key, vars = {}) => (STRINGS[lang][key] || STRINGS.en[key] || key).replace(/\{(\w+)\}/g, (_, k) => vars[k] ?? '');
  const $ = (id) => document.getElementById(id);
  // On the Wi-Fi the API sits next to the page (./api/<op>); through the relay site the page link
  // carries the room (?r=CODE) and every call goes to api.php with it.
  const room = new URLSearchParams(location.search).get('r');
  const api = (op, params = {}) => {
    const u = room ? new URL('api.php', location.href) : new URL(`api/${op}`, location.href.split('?')[0].replace(/[^/]*$/, ''));
    if (room) { u.searchParams.set('r', room); u.searchParams.set('op', op); }
    for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== '') u.searchParams.set(k, v);
    return u.toString();
  };

  document.documentElement.lang = lang;
  for (const el of document.querySelectorAll('[data-t]')) el.textContent = tr(el.dataset.t);

  // Who this phone is: a random id kept on the phone, and the name they gave.
  const store = (k, v) => { try { if (v === undefined) return localStorage.getItem(k); localStorage.setItem(k, v); } catch { /* private mode */ } return v; };
  let guest = store('miu-guest');
  if (!guest) guest = store('miu-guest', (crypto.randomUUID ? crypto.randomUUID() : String(Math.random()).slice(2)).replace(/-/g, '').slice(0, 24));
  let name = store('miu-guest-name') || '';
  const showName = () => { $('who').textContent = name || '?'; };
  const askName = () => { $('name').value = name; $('name-dialog').showModal(); };
  $('name-form').addEventListener('submit', () => { name = $('name').value.trim().slice(0, 40); store('miu-guest-name', name); showName(); });
  $('who').addEventListener('click', askName);
  showName();
  if (!name) askName();

  let toastTimer;
  const toast = (text) => {
    const el = $('toast');
    el.textContent = text;
    el.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { el.hidden = true; }, 3500);
  };

  const post = async (op, body) => {
    const r = await fetch(api(op), { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ guest, name, ...body }) });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.error || 'error');
    return data;
  };

  // Tabs
  const tab = (which) => {
    for (const t of ['queue', 'add']) {
      $(`tab-${t}`).setAttribute('aria-selected', String(t === which));
      $(`panel-${t}`).hidden = t !== which;
    }
    if (which === 'add') $('search').focus();
  };
  $('tab-queue').addEventListener('click', () => tab('queue'));
  $('tab-add').addEventListener('click', () => tab('add'));

  const heart = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 21s-7-4.5-9.5-9A5.5 5.5 0 0 1 12 6a5.5 5.5 0 0 1 9.5 6c-2.5 4.5-9.5 9-9.5 9z"/></svg>';

  const line = (e) => {
    const text = document.createElement('div');
    const t = document.createElement('div');
    t.className = 't';
    if (e.kind === 'video') {
      const tag = document.createElement('span');
      tag.className = 'tag';
      tag.textContent = tr('video');
      t.append(tag);
    }
    t.append(document.createTextNode(e.title));
    const s = document.createElement('div');
    s.className = 's';
    s.textContent = [e.artist, e.addedName ? tr('by', { name: e.addedName }) : ''].filter(Boolean).join(' · ');
    text.append(t, s);
    return text;
  };

  const render = (state) => {
    $('now-title').textContent = state.now ? state.now.title : tr('nothing');
    $('now-artist').textContent = state.now ? (state.now.artist || '') : '';
    const notes = [];
    if (!state.open) notes.push(tr('closedNote'));
    else if (state.approval) notes.push(tr('approval'));
    if (state.waitingMine) notes.push(tr('waiting', { n: state.waitingMine }));
    if (!state.queue.length) notes.push(tr('empty'));
    $('queue-note').textContent = notes.join(' ');
    $('queue').replaceChildren(...state.queue.map((e) => {
      const li = document.createElement('li');
      if (e.mine) li.className = 'mine';
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'vote';
      b.setAttribute('aria-pressed', String(e.voted));
      b.setAttribute('aria-label', tr('vote'));
      b.innerHTML = heart;
      b.append(document.createTextNode(String(e.votes)));
      b.addEventListener('click', () => post('vote', { id: e.id, on: !e.voted }).catch((err) => toast(tr(err.message))));
      li.append(line(e), b);
      return li;
    }));
  };

  // The queue: ask again as soon as it changes (the server holds the request until then).
  let version = -1;
  const follow = async () => {
    for (;;) {
      try {
        const r = await fetch(api('state', { guest, since: version >= 0 ? version : '' }), { cache: 'no-store' });
        const state = await r.json();
        if (!r.ok) throw new Error(state.error || 'error');
        version = state.version;
        render(state);
        // Through the relay the DJ's answer to an add comes later: refusals show here.
        for (const n of state.notices ?? []) if (n.error) toast(tr(n.error));
        // The relay answers at once (no waiting request): ask again in a few seconds.
        if (room) await new Promise((ok) => setTimeout(ok, 3000));
      } catch (err) {
        toast(tr(['relayOff', 'roomGone', 'slowDown', 'relayDown'].includes(err.message) ? err.message : 'offline'));
        await new Promise((ok) => setTimeout(ok, 3000));
      }
    }
  };
  follow();

  // Search the DJ's library.
  let searchTimer;
  $('search').addEventListener('input', () => {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(async () => {
      const q = $('search').value.trim();
      const list = q ? await fetch(api('search', { q })).then((r) => r.json()).catch(() => []) : [];
      $('results').replaceChildren(...list.map((s) => {
        const li = document.createElement('li');
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'add';
        b.textContent = '+';
        b.setAttribute('aria-label', tr('add'));
        b.addEventListener('click', () => post('add', { songId: s.songId }).then((d) => { toast(tr(d.pending ? 'sent' : 'added')); tab('queue'); }).catch((err) => toast(tr(err.message))));
        li.append(line(s), b);
        return li;
      }));
    }, 250);
  });

  // Paste a link.
  $('link-add').addEventListener('click', () => {
    const url = $('link').value.trim();
    if (!url) return;
    post('add', { url }).then((d) => { $('link').value = ''; toast(tr(d.pending ? 'sent' : 'added')); tab('queue'); }).catch((err) => toast(tr(err.message)));
  });
})();
