// DJ mode screen (D.4): the booth over the app. Every control sends a DJ action to the engine;
// the engine's state is read about 20 times a second while the screen is open.

import { t, translatePage } from './i18n.js';
import {
  bpmLabel, tempoLabel, timeLeft, knobDrag, knobKey, meterHeight, fits, songMeta,
  nextText, holdProgress, filterSongsForDj, analysisText, jogTurns, keyAction, loopBeats, learnControl, moveSelection, transitionChoices, keyShiftLabel, kindChoices, transitionsSummary,
} from './dj.js';
import { drawOverview, drawZoom, sectionSpans, phraseTimes, mixZone, cueMarks, deckSpeed } from './djwave.js';

const POLL_MS = 50;
const LIST_MAX = 300;
const SIDES = ['A', 'B'];
/** Seconds of music across the zoomed waveform. */
const ZOOM_S = 6;

export function setupDj({ invoke, listen, songs, artUrl, onOpen, onClose }) {
  const $ = (id) => document.getElementById(id);
  const root = $('dj-mode');
  const party = $('dj-party-screen');
  const deckEl = (side) => root.querySelector(`.dj-deck[data-deck="${side}"]`);
  const stripEl = (side) => root.querySelector(`.dj-strip[data-deck="${side}"]`);
  const waveEl = (side) => root.querySelector(`.dj-wave[data-deck="${side}"]`);

  let timer = null;
  let state = null;
  let analyses = new Map();
  let shownIds = [undefined, undefined];
  let overviewIds = [undefined, undefined];
  let overviewData = [null, null];
  let overviewDrawn = ['', ''];
  let stateAt = 0;
  let zoomFrame = 0;
  let deckColours = [];
  const played = new Set();
  let queue = [];
  let listKey = '';
  let dragging = null; // the control being dragged, so polling does not move it back
  let lastOnAir = null;

  const act = (action) => invoke('dj_action', { action }).catch(() => {});
  const songById = (id) => songs().find((s) => s.id === id);

  // ---- Controls ----

  function setKnob(knob, value) {
    const min = Number(knob.dataset.min);
    const max = Number(knob.dataset.max);
    knob.setAttribute('aria-valuenow', String(value));
    knob.style.setProperty('--turn', `${((value - min) / (max - min)) * 270 - 135}deg`);
    knob.classList.toggle('killed', knob.dataset.ctl === 'eq' && value <= -1);
  }

  function knobAction(knob, value) {
    const deck = knob.closest('[data-deck]')?.dataset.deck;
    switch (knob.dataset.ctl) {
      case 'gain': return { type: 'gain', deck, value };
      case 'eq': return { type: 'eq', deck, band: knob.dataset.band, value };
      case 'filter': return { type: 'filter', deck, value };
      case 'master': return { type: 'masterVolume', value };
      case 'cueMix': return { type: 'cueMix', value };
      default: return { type: 'headphoneVolume', value };
    }
  }

  function turn(knob, value) {
    setKnob(knob, value);
    act(knobAction(knob, value));
  }

  for (const knob of root.querySelectorAll('.dj-knob')) {
    const range = { min: Number(knob.dataset.min), max: Number(knob.dataset.max), reset: Number(knob.dataset.reset) };
    setKnob(knob, range.reset);
    knob.addEventListener('pointerdown', (e) => {
      knob.setPointerCapture(e.pointerId);
      const start = { y: e.clientY, value: Number(knob.getAttribute('aria-valuenow')) };
      const move = (ev) => turn(knob, knobDrag(start.value, start.y - ev.clientY, range));
      const up = () => { knob.removeEventListener('pointermove', move); knob.removeEventListener('pointerup', up); };
      knob.addEventListener('pointermove', move);
      knob.addEventListener('pointerup', up);
    });
    knob.addEventListener('dblclick', () => turn(knob, range.reset));
    knob.addEventListener('keydown', (e) => {
      const v = knobKey(Number(knob.getAttribute('aria-valuenow')), e.key, { ...range, step: (range.max - range.min) / 40 });
      if (v == null) return;
      e.preventDefault();
      turn(knob, v);
    });
  }

  for (const side of SIDES) {
    const deck = deckEl(side);
    const tempo = deck.querySelector('[data-ctl="tempo"]');
    tempo.addEventListener('input', () => { dragging = tempo; act({ type: 'tempo', deck: side, value: Number(tempo.value) }); });
    tempo.addEventListener('change', () => { dragging = null; });
    tempo.addEventListener('dblclick', () => { tempo.value = '0'; act({ type: 'tempo', deck: side, value: 0 }); });
    deck.querySelector('[data-ctl="play"]').addEventListener('click', () => act({ type: 'playPause', deck: side }));
    deck.querySelector('[data-ctl="sync"]').addEventListener('click', () => act({ type: 'sync', deck: side }));
    // Key shift a semitone at a time (-6..6) and key lock, from the deck's state.
    const deckState = () => state?.decks[SIDES.indexOf(side)] ?? {};
    deck.querySelector('[data-ctl="keyUp"]').addEventListener('click', () => act({ type: 'keyShift', deck: side, semitones: Math.min(6, (deckState().keyShift ?? 0) + 1) }));
    deck.querySelector('[data-ctl="keyDown"]').addEventListener('click', () => act({ type: 'keyShift', deck: side, semitones: Math.max(-6, (deckState().keyShift ?? 0) - 1) }));
    deck.querySelector('[data-ctl="keyLock"]').addEventListener('click', () => act({ type: 'keyLock', deck: side, on: !deckState().keyLock }));
    const cue = deck.querySelector('[data-ctl="cue"]');
    cue.addEventListener('pointerdown', () => act({ type: 'cue', deck: side, pressed: true }));
    for (const ev of ['pointerup', 'pointerleave']) cue.addEventListener(ev, (e) => { if (ev === 'pointerup' || e.buttons) act({ type: 'cue', deck: side, pressed: false }); });
    for (const nudge of deck.querySelectorAll('[data-ctl="nudge"]')) {
      nudge.addEventListener('pointerdown', () => act({ type: 'nudge', deck: side, amount: Number(nudge.dataset.amount) }));
      for (const ev of ['pointerup', 'pointerleave']) nudge.addEventListener(ev, () => act({ type: 'nudge', deck: side, amount: 0 }));
    }
    // Hands-on: hot cues (right-click clears), beat loops, effects.
    for (const pad of deck.querySelectorAll('.dj-hotpad')) {
      const slot = Number(pad.dataset.slot);
      pad.addEventListener('click', () => act({ type: 'hotCue', deck: side, slot }));
      pad.addEventListener('contextmenu', (e) => { e.preventDefault(); act({ type: 'clearHotCue', deck: side, slot }); });
    }
    for (const b of deck.querySelectorAll('[data-loop]')) {
      b.addEventListener('click', () => act({ type: 'loop', deck: side, beats: b.dataset.loop === 'off' ? null : Number(b.dataset.loop) }));
    }
    for (const b of deck.querySelectorAll('[data-fx]')) {
      b.addEventListener('click', () => act({ type: 'effect', deck: side, effect: b.dataset.fx, on: b.getAttribute('aria-pressed') !== 'true' }));
    }
    // The platter: drag round it to nudge a playing song or move through a paused one.
    const platter = deck.querySelector('.dj-platter');
    platter.addEventListener('pointerdown', (e) => {
      platter.setPointerCapture(e.pointerId);
      const r = platter.getBoundingClientRect();
      const center = { x: r.left + r.width / 2, y: r.top + r.height / 2 };
      let last = { x: e.clientX, y: e.clientY };
      const move = (ev) => {
        const now = { x: ev.clientX, y: ev.clientY };
        const delta = jogTurns(last, now, center);
        last = now;
        if (delta) act({ type: 'jog', deck: side, delta });
      };
      const up = () => { platter.removeEventListener('pointermove', move); platter.removeEventListener('pointerup', up); };
      platter.addEventListener('pointermove', move);
      platter.addEventListener('pointerup', up);
    });
    const strip = stripEl(side);
    const fader = strip.querySelector('[data-ctl="volume"]');
    fader.addEventListener('input', () => act({ type: 'volume', deck: side, value: Number(fader.value) }));
    const pfl = strip.querySelector('[data-ctl="phones"]');
    pfl.addEventListener('click', () => {
      const on = pfl.getAttribute('aria-pressed') !== 'true';
      pfl.setAttribute('aria-pressed', String(on));
      act({ type: 'headphoneCue', deck: side, on });
    });
    // Click on the overview to jump there (only on a paused deck: no accidental jumps on air).
    waveEl(side).addEventListener('click', (e) => {
      const d = state?.decks[SIDES.indexOf(side)];
      if (!d?.durationS || d.playing) return;
      const r = e.currentTarget.getBoundingClientRect();
      const turns = (((e.clientX - r.left) / r.width) * d.durationS - d.positionS) / 1.8;
      act({ type: 'jog', deck: side, delta: turns });
    });
  }

  const xfader = $('dj-xfader');
  xfader.addEventListener('input', () => { dragging = xfader; act({ type: 'crossfader', value: Number(xfader.value) }); });
  xfader.addEventListener('change', () => { dragging = null; });
  xfader.addEventListener('dblclick', () => { xfader.value = '0'; act({ type: 'crossfader', value: 0 }); });

  for (const b of root.querySelectorAll('[data-pad]')) b.addEventListener('click', () => act({ type: 'pad', slot: Number(b.dataset.pad) }));

  // The DJ keyboard while the booth is open (not while typing in the search box or a menu).
  const held = new Map();
  document.addEventListener('keydown', (e) => {
    if (root.hidden || e.repeat || e.ctrlKey || e.altKey || e.metaKey || e.target.closest?.('input[type="search"], select')) return;
    const k = keyAction(e, state);
    if (!k) return;
    e.preventDefault();
    act(k.down);
    if (k.up) held.set(e.code, k.up);
  });
  document.addEventListener('keyup', (e) => {
    const up = held.get(e.code);
    if (up) { held.delete(e.code); act(up); }
  });

  const sendAuto = () => invoke('dj_set_auto', { curve: $('dj-curve').value, queue }).catch(() => {});
  $('dj-auto').addEventListener('change', (e) => act({ type: 'autoDj', on: e.target.checked }));
  $('dj-curve').addEventListener('change', sendAuto);
  // Transitions popover: the checklist Auto DJ keeps to, mix it up, and the next one by hand.
  const trPanel = $('dj-tr-panel');
  const setTrPanel = (open) => { trPanel.hidden = !open; $('dj-tr-btn').setAttribute('aria-expanded', String(open)); };
  const sendTransitions = () => {
    const allowed = [...$('dj-tr-list').querySelectorAll('input:checked')].map((c) => c.value);
    invoke('dj_set_transitions', { transitions: { allowed, mixItUp: $('dj-mix').checked } }).catch(showProblem);
  };
  $('dj-tr-btn').addEventListener('click', () => {
    setTrPanel(trPanel.hidden);
    if (trPanel.hidden) return;
    const chosen = state?.transitions ?? { allowed: [], mixItUp: false };
    $('dj-tr-list').replaceChildren(...kindChoices().map((k) => {
      const label = document.createElement('label');
      const box = document.createElement('input');
      box.type = 'checkbox';
      box.value = k.kind;
      box.checked = chosen.allowed.includes(k.kind);
      box.addEventListener('change', sendTransitions);
      const name = document.createElement('span');
      name.textContent = k.label;
      label.append(box, name);
      return label;
    }));
    $('dj-mix').checked = chosen.mixItUp;
  });
  $('dj-mix').addEventListener('change', sendTransitions);
  $('dj-tr-clear').addEventListener('click', () => {
    for (const box of $('dj-tr-list').querySelectorAll('input')) box.checked = false;
    $('dj-mix').checked = false;
    sendTransitions();
  });
  $('dj-tr-close').addEventListener('click', () => setTrPanel(false));

  $('dj-choose').addEventListener('change', (e) => {
    const v = e.target.value;
    invoke('dj_choose_transition', { transition: v === 'auto' ? null : JSON.parse(v) }).catch(showProblem);
  });
  $('dj-style').addEventListener('change', (e) => invoke('dj_set_style', { style: e.target.value }).catch(showProblem));
  $('dj-phones').addEventListener('change', (e) => invoke('dj_set_phones', { name: e.target.value || null }).catch(showProblem));
  $('dj-search').addEventListener('input', () => renderList(true));
  $('dj-fits').addEventListener('change', () => renderList(true));
  $('dj-exit').addEventListener('click', close);

  $('dj-list').addEventListener('click', (e) => {
    const b = e.target.closest('button');
    const id = Number(b?.closest('[data-id]')?.dataset.id);
    if (!b || !id) return;
    if (b.dataset.load) {
      invoke('dj_load', { deck: b.dataset.load, songId: id }).catch(showProblem);
    } else {
      // Into the party queue (the one Auto DJ plays first), next to what guests add.
      queue = queue.includes(id) ? queue : [...queue, id];
      invoke('party_dj_add', { songId: id }).catch(showProblem);
      renderList(true);
    }
  });

  // ---- DJ controllers: devices, MIDI learn, browsing from the controller ----

  const panel = $('dj-ctl-panel');
  let learning = false;
  let learnEl = null;
  const setPanel = (open) => { panel.hidden = !open; $('dj-ctl-btn').setAttribute('aria-expanded', String(open)); };
  $('dj-ctl-btn').addEventListener('click', () => { setPanel(panel.hidden); if (!panel.hidden) refreshControllers(); });
  $('dj-ctl-close').addEventListener('click', () => setPanel(false));

  function setLearning(on) {
    learning = on;
    root.classList.toggle('learning', on);
    $('dj-learn').setAttribute('aria-pressed', String(on));
    $('dj-learn-hint').textContent = t('dj.ctl.learnHint');
    learnEl?.classList.remove('learn-target');
    learnEl = null;
    if (!on) invoke('dj_midi_learn', { control: null, deck: null }).catch(() => {});
  }
  $('dj-learn').addEventListener('click', () => setLearning(!learning));

  // In learn mode a click picks the control to teach instead of using it.
  const learnPick = (e) => {
    if (!learning) return;
    const el = e.target.closest('.dj-desk [data-ctl], .dj-desk [data-slot], .dj-desk [data-loop], .dj-desk [data-fx]');
    if (!el) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.type !== 'pointerdown') return;
    const target = learnControl(el.dataset, el.closest('[data-deck]')?.dataset.deck);
    learnEl?.classList.remove('learn-target');
    if (!target) { learnEl = null; $('dj-learn-hint').textContent = t('dj.ctl.cannotLearn'); return; }
    learnEl = el;
    el.classList.add('learn-target');
    $('dj-learn-hint').textContent = t('dj.ctl.waiting');
    invoke('dj_midi_learn', target).catch(showProblem);
  };
  for (const ev of ['pointerdown', 'click', 'dblclick', 'contextmenu', 'input']) root.addEventListener(ev, learnPick, true);
  listen('dj-midi-learnt', () => {
    learnEl?.classList.remove('learn-target');
    learnEl = null;
    $('dj-learn-hint').textContent = t('dj.ctl.learntOk');
    refreshControllers();
  });

  function renderControllers(devices) {
    const ready = devices.filter((d) => d.mapping);
    $('dj-ctl-dot').classList.toggle('on', ready.length > 0);
    $('dj-ctl-label').textContent = ready.length ? ready[0].mapping : t('dj.ctl.title');
    const list = $('dj-ctl-list');
    if (!devices.length) {
      const li = document.createElement('li');
      li.className = 'muted';
      li.textContent = t('dj.ctl.none');
      list.replaceChildren(li);
      return;
    }
    list.replaceChildren(...devices.map((d) => {
      const li = document.createElement('li');
      const name = document.createElement('b');
      name.textContent = d.name;
      const state = document.createElement('span');
      state.textContent = d.mapping ? t('dj.ctl.ready', { name: d.mapping }) : t('dj.ctl.notSetUp');
      li.append(name, state);
      if (d.builtIn && !d.triedOnDevice) {
        const warn = document.createElement('small');
        warn.textContent = t('dj.ctl.untried');
        li.append(warn);
      }
      if (d.note) {
        const note = document.createElement('small');
        note.textContent = d.note;
        li.append(note);
      }
      if (d.learnt) {
        const row = document.createElement('div');
        const count = document.createElement('small');
        count.textContent = t('dj.ctl.learnt', { n: d.learnt });
        const forget = document.createElement('button');
        forget.type = 'button';
        forget.className = 'dj-top-btn';
        forget.textContent = t('dj.ctl.forget');
        forget.addEventListener('click', () => invoke('dj_midi_forget', { device: d.name }).then(refreshControllers).catch(showProblem));
        row.append(count, forget);
        li.append(row);
      }
      return li;
    }));
  }

  async function refreshControllers() {
    renderControllers(await invoke('dj_controllers').catch(() => []));
  }
  listen('dj-controllers', (e) => renderControllers(e.payload));

  // The browse knob moves a selection through the song list; load buttons load it.
  let selectedId = null;
  const rowsShown = () => [...$('dj-list').querySelectorAll('.dj-row')];
  function markSelected() {
    for (const r of rowsShown()) r.classList.toggle('sel', Number(r.dataset.id) === selectedId);
    $('dj-list').querySelector('.dj-row.sel')?.scrollIntoView({ block: 'nearest' });
  }
  listen('dj-controller', (e) => {
    const a = e.payload;
    const rows = rowsShown();
    if (a.type === 'browse') {
      const at = moveSelection(rows.findIndex((r) => Number(r.dataset.id) === selectedId), a.steps, rows.length);
      selectedId = at >= 0 ? Number(rows[at].dataset.id) : null;
      markSelected();
    } else if (a.type === 'loadSelected' && selectedId != null) {
      invoke('dj_load', { deck: a.deck, songId: selectedId }).catch(showProblem);
    }
  });

  // ---- Party screen and guest lock ----

  let locked = false;
  $('dj-party').addEventListener('click', () => { party.hidden = false; });
  $('dj-party-lock').addEventListener('click', (e) => {
    locked = !locked;
    e.currentTarget.setAttribute('aria-pressed', String(locked));
    party.classList.toggle('locked', locked);
  });
  const back = $('dj-party-back');
  let holdStart = null;
  let holdFrame = null;
  const hold = () => {
    const p = holdProgress(performance.now() - holdStart);
    back.querySelector('.dj-hold').style.width = `${p * 100}%`;
    if (p >= 1) { endHold(); party.hidden = true; locked = false; party.classList.remove('locked'); $('dj-party-lock').setAttribute('aria-pressed', 'false'); return; }
    holdFrame = requestAnimationFrame(hold);
  };
  const endHold = () => { cancelAnimationFrame(holdFrame); holdStart = null; back.querySelector('.dj-hold').style.width = '0'; };
  back.addEventListener('pointerdown', () => {
    if (!locked) { party.hidden = true; return; }
    holdStart = performance.now();
    hold();
  });
  for (const ev of ['pointerup', 'pointerleave']) back.addEventListener(ev, () => { if (holdStart != null) endHold(); });
  back.title = t('dj.unlockHold');

  // ---- State ----

  let problemTimer = null;
  function showProblem(message) {
    const el = $('dj-problem');
    el.textContent = String(message);
    el.hidden = false;
    clearTimeout(problemTimer);
    problemTimer = setTimeout(() => { el.hidden = true; }, 6000);
  }

  async function loadOverview(side, i, songId) {
    overviewIds[i] = songId;
    overviewData[i] = null;
    overviewDrawn[i] = '';
    if (songId == null) return;
    const o = await invoke('dj_overview', { deck: side }).catch(() => null);
    if (o && o.songId === songId && overviewIds[i] === songId) overviewData[i] = o;
    else overviewIds[i] = null; // not ready yet: ask again on the next poll
  }

  /** The whole-song waveform, drawn again only when something on it changed. */
  function renderOverview(side, i, d) {
    const canvas = waveEl(side).querySelector('canvas');
    const o = overviewData[i];
    const a = d.songId != null ? analyses.get(d.songId) : null;
    const mix = state.autoDj && state.onAir === side ? mixZone(state.next, d.durationS) : null;
    const marks = cueMarks(d);
    const key = JSON.stringify([d.songId, !!o, canvas.clientWidth, canvas.clientHeight, marks, mix && [Math.round(mix.from * 500), Math.round(mix.to * 500)], !!a]);
    if (key === overviewDrawn[i]) return;
    overviewDrawn[i] = key;
    drawOverview(canvas, {
      bands: o?.bands ?? [],
      sections: sectionSpans(a, d.durationS),
      phrases: phraseTimes(a, d.durationS),
      durationS: d.durationS,
      mix,
      ...marks,
    }, { label: (kind) => t(`dj.section.${kind}`), mixLabel: t('dj.mixZone') });
  }

  /** The zoomed waveform, every frame while the booth is open: positions run on between polls. */
  function renderZoom() {
    zoomFrame = 0;
    if (root.hidden || !state) return;
    if (!deckColours.length) deckColours = SIDES.map((side) => getComputedStyle(deckEl(side)).getPropertyValue('--deck').trim() || '#fff'); // i18n-ignore
    const since = (performance.now() - stateAt) / 1000;
    const lanes = state.decks.map((d, i) => {
      const o = overviewData[i];
      if (d.songId == null) return null;
      const a = analyses.get(d.songId);
      const speed = deckSpeed(d, a);
      return {
        positionS: d.positionS + (d.playing ? Math.min(since, 0.25) * speed : 0),
        speed,
        analysis: a,
        detail: o?.songId === d.songId ? o.detail : null,
        rate: o?.detailRate ?? 100,
        hotCues: d.hotCues,
        loop: d.loopS,
        colour: deckColours[i],
      };
    });
    drawZoom($('dj-zoom'), lanes, ZOOM_S);
    zoomFrame = requestAnimationFrame(renderZoom);
  }

  function onAirAnalysis() {
    const i = state?.onAir ? SIDES.indexOf(state.onAir) : state?.decks.findIndex((d) => d.playing);
    const d = state?.decks[i];
    const a = d?.songId != null ? analyses.get(d.songId) : null;
    return a ? { bpm: d.bpm ?? a.bpm, key: a.key, songId: d.songId } : null;
  }

  function renderDeck(side, i) {
    const d = state.decks[i];
    const el = deckEl(side);
    const field = (f) => el.querySelector(`[data-f="${f}"]`);
    if (shownIds[i] !== d.songId) {
      shownIds[i] = d.songId;
      const song = d.songId != null ? songById(d.songId) : null;
      field('title').textContent = song?.title ?? (d.loading ? t('dj.loading') : t('dj.empty'));
      field('artist').textContent = song?.artist ?? '';
      el.classList.toggle('empty', !song);
      for (const [img, size] of [[el.querySelector('.dj-cover img'), 120], [el.querySelector('.dj-platter img'), 240]]) { // i18n-ignore
        img.hidden = !song;
        if (song) { img.onerror = () => { img.hidden = true; }; img.src = artUrl(song.id, size); }
      }
      const a = d.songId != null ? analyses.get(d.songId) : null;

      field('energy').textContent = a ? t('dj.energy', { n: a.energy }) : '';
    }
    if (d.loading && d.songId == null) field('title').textContent = t('dj.loading');
    if (overviewIds[i] !== d.songId) loadOverview(side, i, d.songId);
    field('bpm').textContent = bpmLabel(d.bpm);
    field('time').textContent = timeLeft(d);
    field('tempo').textContent = tempoLabel(d.tempo);
    const tempo = el.querySelector('[data-ctl="tempo"]');
    if (dragging !== tempo) tempo.value = String(d.tempo);
    el.querySelector('[data-ctl="play"]').classList.toggle('on', d.playing);
    for (const pad of el.querySelectorAll('.dj-hotpad')) pad.classList.toggle('set', d.hotCues?.[Number(pad.dataset.slot)] != null);
    const beats = loopBeats(d);
    for (const b of el.querySelectorAll('[data-loop]')) b.classList.toggle('on', b.dataset.loop === 'off' ? !!d.loopS : Number(b.dataset.loop) === beats);
    el.querySelector('[data-fx="echo"]').setAttribute('aria-pressed', String(!!d.echo));
    el.querySelector('[data-fx="filterSweep"]').setAttribute('aria-pressed', String(!!d.sweep));
    // One turn per 1.8 s of music, like a record at 33 1/3: the disc shows where the song is.
    el.querySelector('.dj-platter-disc').style.transform = `rotate(${((d.positionS / 1.8) % 1) * 360}deg)`;
    field('onair').hidden = state.onAir !== side;
    const air = onAirAnalysis();
    const a = d.songId != null ? analyses.get(d.songId) : null;
    // The key heard now (shifted), with the shift beside it.
    const key = d.key ?? a?.key ?? '';
    const shift = keyShiftLabel(d.keyShift ?? 0);
    if (field('key').dataset.shown !== `${key}${shift}`) {
      field('key').dataset.shown = `${key}${shift}`;
      field('key').textContent = key;
      if (shift && key) {
        const small = document.createElement('small');
        small.textContent = shift;
        field('key').append(small);
      }
    }
    el.querySelector('[data-ctl="keyLock"]').setAttribute('aria-pressed', String(!!d.keyLock));
    field('key').classList.toggle('fits', !!air && air.songId !== d.songId && fits(air, a));
    stripEl(side).querySelector('.dj-meter div').style.height = `${meterHeight(d.level) * 100}%`;  // i18n-ignore
    const p = d.durationS ? Math.min(1, d.positionS / d.durationS) : 0;
    const wave = waveEl(side);
    wave.querySelector('.dj-wave-played').style.width = `${p * 100}%`;
    wave.querySelector('.dj-wave-head').style.left = `${p * 100}%`;
    wave.querySelector('.dj-wave-left').textContent = timeLeft(d);
    renderOverview(side, i, d);
  }

  function renderParty() {
    if (party.hidden) return;
    const i = state.onAir ? SIDES.indexOf(state.onAir) : state.decks.findIndex((d) => d.playing);
    const d = state.decks[i];
    const song = d?.songId != null ? songById(d.songId) : null;
    $('dj-party-title').textContent = song?.title ?? t('dj.nothingOnAir');
    $('dj-party-artist').textContent = song?.artist ?? '';
    const img = $('dj-party-img');
    if (song && img.dataset.id !== String(song.id)) { img.dataset.id = String(song.id); img.hidden = false; img.onerror = () => { img.hidden = true; }; img.src = artUrl(song.id, 600); }
    if (!song) img.hidden = true;
    $('dj-party-progress').style.width = `${d?.durationS ? (d.positionS / d.durationS) * 100 : 0}%`;
    const other = state.decks[1 - i];
    const next = other?.songId != null ? songById(other.songId) : null;
    $('dj-party-next').textContent = next ? t('dj.upNext', { title: [next.title, next.artist].filter(Boolean).join(' · ') }) : '';
    $('dj-party-pulse').style.opacity = String(0.15 + meterHeight(d?.level ?? 0) * 0.85);
  }

  function renderList(force = false) {
    const air = $('dj-fits').checked ? onAirAnalysis() : null;
    const key = [$('dj-search').value, air?.songId, air?.bpm?.toFixed(0), played.size, queue.join(','), analyses.size].join('|');
    if (!force && key === listKey) return;
    listKey = key;
    const fitsAir = onAirAnalysis();
    const list = filterSongsForDj(songs(), analyses, { query: $('dj-search').value, fitsWith: air, played }).slice(0, LIST_MAX);
    const rows = list.map((s) => {
      const a = analyses.get(s.id);
      const row = document.createElement('div');
      row.className = `dj-row${played.has(s.id) ? ' played' : ''}`;
      row.setAttribute('role', 'listitem');
      row.dataset.id = String(s.id);
      row.title = songMeta(a);
      const who = document.createElement('div');
      who.className = 'who';
      who.textContent = s.title ?? '';
      const by = document.createElement('small');
      by.textContent = s.artist ?? '';
      who.append(by);
      const bpm = document.createElement('div');
      bpm.className = 'num';
      bpm.textContent = a ? `${Math.round(a.bpm)} BPM` : ''; // i18n-ignore
      if (!a) bpm.title = t('dj.notAnalysed');
      const k = document.createElement('div');
      k.className = `k${fitsAir && fitsAir.songId !== s.id && fits(fitsAir, a) ? ' fits' : ''}`;
      k.textContent = a?.key ?? '';
      const energy = document.createElement('div');
      energy.className = 'num';
      energy.textContent = a ? t('dj.energy', { n: a.energy }) : t('dj.notAnalysed');
      const acts = document.createElement('div');
      acts.className = 'acts';
      for (const side of SIDES) {
        const b = document.createElement('button');
        b.type = 'button';
        b.dataset.load = side;
        b.textContent = side;
        b.title = t(side === 'A' ? 'dj.loadA' : 'dj.loadB');
        b.setAttribute('aria-label', b.title);
        acts.append(b);
      }
      const q = document.createElement('button');
      q.type = 'button';
      q.className = queue.includes(s.id) ? 'queued' : '';
      q.textContent = '+';
      q.title = queue.includes(s.id) ? t('dj.queued') : t('dj.queue');
      q.setAttribute('aria-label', q.title);
      acts.append(q);
      row.append(who, bpm, k, energy, acts);
      return row;
    });
    $('dj-list').replaceChildren(...rows);
    markSelected();
  }

  async function poll() {
    const s = await invoke('dj_state').catch(() => null);
    if (!s || root.hidden) return;
    state = s;
    stateAt = performance.now();
    if (!zoomFrame) zoomFrame = requestAnimationFrame(renderZoom);
    for (const d of s.decks) if (d.songId != null) played.add(d.songId);
    if (queue.length && queue.some((id) => played.has(id))) { queue = queue.filter((id) => !played.has(id)); sendAuto(); }
    SIDES.forEach((side, i) => renderDeck(side, i));
    if (dragging !== xfader) xfader.value = String(s.crossfader);
    $('dj-auto').checked = s.autoDj;
    // A transition picked by hand is used once: after the mix the menu is back on Auto.
    if (s.onAir !== lastOnAir) {
      if (lastOnAir != null) $('dj-choose').value = 'auto';
      lastOnAir = s.onAir;
    }
    if (document.activeElement !== $('dj-style') && s.style) $('dj-style').value = s.style;
    $('dj-tr-btn').textContent = transitionsSummary(s.transitions);
    $('dj-next').textContent = s.autoDj ? nextText(s.next) : '';
    if (s.problem) showProblem(s.problem);
    renderParty();
    renderList();
  }

  async function refreshLibrary() {
    const list = await invoke('dj_library').catch(() => []);
    analyses = new Map(list.map((x) => [x.songId, x]));
    shownIds = [undefined, undefined];
    renderList(true);
  }

  async function refreshOutputs() {
    const o = await invoke('dj_outputs').catch(() => null);
    if (!o) return;
    const sel = $('dj-phones');
    const none = new Option(t('dj.phonesNone'), '');
    sel.replaceChildren(none, ...o.devices.map((d) => new Option(d.name, d.name)));
    sel.value = o.phones ?? '';
  }

  listen('dj-analysis-progress', (e) => {
    $('dj-analysis').textContent = analysisText(e.payload);
    if (!root.hidden) refreshLibrary();
  });

  async function open() {
    try {
      state = await invoke('dj_enter');
    } catch (e) {
      showProblemOutside(e);
      return;
    }
    translatePage(root);
    root.querySelector('.dj-brand').title = t('dj.keysHelp');
    $('dj-choose').replaceChildren(...transitionChoices().map((c) => new Option(c.label, c.value)));
    root.hidden = false;
    shownIds = [undefined, undefined];
    overviewIds = [undefined, undefined];
    overviewDrawn = ['', ''];
    await Promise.all([refreshLibrary(), refreshOutputs()]);
    onOpen?.();
    refreshControllers();
    invoke('dj_analysis_progress').then((p) => { $('dj-analysis').textContent = analysisText(p); }).catch(() => {});
    clearInterval(timer);
    timer = setInterval(poll, POLL_MS);
    poll();
  }

  function showProblemOutside(e) {
    root.hidden = false;
    showProblem(t('dj.enterFailed', { message: String(e) }));
    setTimeout(() => { root.hidden = true; }, 6000);
  }

  async function close() {
    setLearning(false);
    setPanel(false);
    clearInterval(timer);
    timer = null;
    cancelAnimationFrame(zoomFrame);
    zoomFrame = 0;
    party.hidden = true;
    root.hidden = true;
    await invoke('dj_leave').catch(() => {});
    onClose?.();
  }

  return { open, close };
}
