//! DJ controllers live (D.6): while DJ mode is open, every MIDI input is listened to (so any
//! controller can be taught with MIDI learn), plugged-in and unplugged devices are picked up,
//! messages become DJ actions through the device's mapping, and its lights follow the engine.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::actions::Deck;
use super::midi::{self, Control, Mapping, Translator};
use crate::library::service::SharedDb;
use crate::settings;

const RESCAN: Duration = Duration::from_secs(1);
const LIGHTS: Duration = Duration::from_millis(100);

/// A MIDI device as the DJ screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub name: String,
    /// The mapping in use (built-in name, or the device's own name for learnt-only), if any.
    pub mapping: Option<String>,
    pub built_in: bool,
    pub tried_on_device: bool,
    pub note: Option<String>,
    pub learnt: usize,
}

/// Shared between the DJ commands and the controller thread.
#[derive(Default)]
pub struct MidiShared {
    /// MIDI learn: the next control moved on any device is taught as this.
    pub learn: Mutex<Option<(Control, Option<Deck>)>>,
    pub devices: Mutex<Vec<Device>>,
    /// Ask the thread to rebuild mappings (after learnt controls are cleared).
    pub reload: AtomicBool,
}

struct Connected {
    _input: MidiInputConnection<()>,
    output: Option<MidiOutputConnection>,
    translator: Option<Translator>,
}

fn mapping_for(db: &SharedDb, builtin: &[Mapping], name: &str) -> (Option<Mapping>, usize) {
    let key = midi::port_key(name);
    let learnt = db.lock().map(|d| settings::dj_learnt(&d, &key)).unwrap_or_default();
    (midi::effective(midi::find(builtin, &key), &learnt, &key), learnt.len())
}

fn describe(name: &str, mapping: &Option<Mapping>, builtin: &[Mapping], learnt: usize) -> Device {
    let key = midi::port_key(name);
    Device {
        name: key.clone(),
        mapping: mapping.as_ref().map(|m| m.name.clone()),
        built_in: midi::find(builtin, &key).is_some(),
        tried_on_device: mapping.as_ref().is_some_and(|m| m.tried_on_device),
        note: mapping.as_ref().and_then(|m| m.note.clone()),
        learnt,
    }
}

/// Runs until `alive` is cleared (DJ mode closed). `apply` sends an action to the engine; the
/// screen gets `dj-controller` (browse/load) and `dj-controllers` (devices) events.
pub fn start(
    app: AppHandle,
    db: SharedDb,
    shared: Arc<MidiShared>,
    alive: Arc<AtomicBool>,
    apply: impl Fn(super::actions::Action) + Send + 'static,
    status: impl Fn() -> Option<super::engine::EngineStatus> + Send + 'static,
) {
    std::thread::spawn(move || {
        let builtin = midi::built_in();
        let (tx, rx) = mpsc::channel::<(String, Vec<u8>)>();
        let mut open: HashMap<String, Connected> = HashMap::new();
        let mut last_scan = Instant::now() - RESCAN;
        let mut last_lights = Instant::now();
        while alive.load(Ordering::Acquire) {
            if last_scan.elapsed() >= RESCAN || shared.reload.swap(false, Ordering::AcqRel) {
                last_scan = Instant::now();
                if rescan(&mut open, &tx, &db, &builtin, &shared) {
                    let list = shared.devices.lock().map(|d| d.clone()).unwrap_or_default();
                    let _ = app.emit("dj-controllers", list);
                }
            }
            while let Ok((port, msg)) = rx.recv_timeout(Duration::from_millis(10)) {
                let target = shared.learn.lock().ok().and_then(|l| *l);
                if let Some((control, deck)) = target {
                    if let Some(entry) = midi::learn(control, deck, &msg) {
                        let key = midi::port_key(&port);
                        if let Ok(d) = db.lock() {
                            let _ = settings::add_dj_learnt(&d, &key, entry);
                        }
                        if let Ok(mut l) = shared.learn.lock() {
                            *l = None;
                        }
                        log::info!("MIDI learn: {control:?} on {key}");
                        let _ = app.emit("dj-midi-learnt", serde_json::json!({ "device": key }));
                        shared.reload.store(true, Ordering::Release);
                    }
                    continue;
                }
                let Some(t) = open.get_mut(&port).and_then(|c| c.translator.as_mut()) else { continue };
                for action in t.handle(&msg) {
                    if midi::for_screen(&action) {
                        let _ = app.emit("dj-controller", &action);
                    } else {
                        apply(action);
                    }
                }
            }
            if last_lights.elapsed() >= LIGHTS {
                last_lights = Instant::now();
                if let Some(s) = status() {
                    for c in open.values_mut() {
                        if let (Some(t), Some(out)) = (c.translator.as_mut(), c.output.as_mut()) {
                            for m in t.leds(&s) {
                                let _ = out.send(&m);
                            }
                        }
                    }
                }
            }
        }
        log::info!("DJ controllers: stopped ({} devices)", open.len());
    });
}

/// Connects new devices, drops unplugged ones and rebuilds mappings; true when the list changed.
fn rescan(open: &mut HashMap<String, Connected>, tx: &mpsc::Sender<(String, Vec<u8>)>, db: &SharedDb, builtin: &[Mapping], shared: &MidiShared) -> bool {
    let Ok(probe) = MidiInput::new("Miu Music DJ") else { return false };
    let names: Vec<String> = probe.ports().iter().filter_map(|p| probe.port_name(p).ok()).collect();
    let before: Vec<Device> = shared.devices.lock().map(|d| d.clone()).unwrap_or_default();
    open.retain(|name, _| names.contains(name));
    let mut devices = Vec::new();
    for name in &names {
        let (mapping, learnt) = mapping_for(db, builtin, name);
        if !open.contains_key(name) {
            match connect(name, tx) {
                Ok(c) => {
                    log::info!("DJ controller connected: {name} ({})", mapping.as_ref().map_or("no mapping yet", |m| m.name.as_str()));
                    open.insert(name.clone(), c);
                }
                Err(e) => {
                    log::warn!("DJ controller {name}: {e}");
                    continue;
                }
            }
        }
        if let Some(c) = open.get_mut(name) {
            if c.translator.as_ref().map(|t| t.mapping()) != mapping.as_ref() {
                c.translator = mapping.clone().map(Translator::new);
            }
        }
        devices.push(describe(name, &mapping, builtin, learnt));
    }
    let changed = devices != before;
    if let Ok(mut d) = shared.devices.lock() {
        *d = devices;
    }
    changed
}

fn connect(name: &str, tx: &mpsc::Sender<(String, Vec<u8>)>) -> Result<Connected, String> {
    let input = MidiInput::new("Miu Music DJ").map_err(|e| e.to_string())?;
    let port = input.ports().into_iter().find(|p| input.port_name(p).ok().as_deref() == Some(name)).ok_or("gone")?;
    let (tx, port_name) = (tx.clone(), name.to_string());
    let conn = input
        .connect(&port, "miu-dj-in", move |_, msg, _| {
            let _ = tx.send((port_name.clone(), msg.to_vec()));
        }, ())
        .map_err(|e| e.to_string())?;
    // The lights go to the output with the same device name, when there is one.
    let output = MidiOutput::new("Miu Music DJ").ok().and_then(|out| {
        let key = midi::port_key(name);
        let p = out.ports().into_iter().find(|p| out.port_name(p).ok().is_some_and(|n| midi::port_key(&n) == key))?;
        out.connect(&p, "miu-dj-out").ok()
    });
    Ok(Connected { _input: conn, output, translator: None })
}
