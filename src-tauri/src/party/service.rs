//! Party mode in the app (P.4): one party per run of Miu; the DJ turns the Wi-Fi page (and the
//! relay) on and off, sees the queue with its votes, approves and removes.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::State;

use super::queue::{Entry, Item, Queue, Settings, HOST};
use super::relay::{self, HttpSite, RelayRunner};
use super::server::{self, LocalServer};
use super::web::Party;
use crate::library::service::SharedDb;

/// The songs guests may pick: everything not disliked.
pub fn library_for_guests(rows: &[crate::library::songs::SongRow]) -> Vec<(i64, String, String)> {
    rows.iter().filter(|r| r.rating >= 0).map(|r| (r.id, r.title.clone(), r.artist.clone().unwrap_or_default())).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostEntry {
    pub id: u64,
    #[serde(flatten)]
    pub item: Item,
    pub added_name: String,
    pub votes: usize,
}

impl From<&Entry> for HostEntry {
    fn from(e: &Entry) -> Self {
        Self { id: e.id, item: e.item.clone(), added_name: if e.added_by == HOST { String::new() } else { e.added_name.clone() }, votes: e.votes.len() }
    }
}

/// What the DJ sees: what plays, the queue in order, what waits for approval, the settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub version: u64,
    pub now: Option<HostEntry>,
    pub queue: Vec<HostEntry>,
    pub waiting: Vec<HostEntry>,
    pub settings: Settings,
}

pub fn host_view(q: &Queue) -> HostView {
    HostView {
        version: q.version(),
        now: q.now().map(HostEntry::from),
        queue: q.ordered().into_iter().map(HostEntry::from).collect(),
        waiting: q.waiting().into_iter().map(HostEntry::from).collect(),
        settings: q.settings,
    }
}

pub struct PartyHost {
    pub party: Arc<Party>,
    local: Mutex<Option<LocalServer>>,
    relay: Mutex<Option<RelayRunner>>,
    /// The video playing on the party screen (id, title), while one is.
    video: Mutex<Option<(String, String)>>,
}

pub type SharedParty = Arc<PartyHost>;

impl Default for PartyHost {
    fn default() -> Self {
        Self { party: Arc::new(Party::new(&server::new_token())), local: Mutex::new(None), relay: Mutex::new(None), video: Mutex::new(None) }
    }
}

impl PartyHost {
    pub fn video(&self) -> Option<(String, String)> {
        self.video.lock().ok().and_then(|v| v.clone())
    }

    pub fn set_video(&self, v: Option<(String, String)>) {
        if let Ok(mut g) = self.video.lock() {
            *g = v;
        }
    }

    fn refresh_library(&self, db: &SharedDb) {
        if let Some(rows) = db.lock().ok().and_then(|d| crate::library::songs::list_songs(&d).ok()) {
            self.party.set_library(library_for_guests(&rows));
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyInfo {
    pub local_on: bool,
    /// The guests' link on the Wi-Fi and its QR code (SVG), while the page is on.
    pub local_url: Option<String>,
    pub local_qr: Option<String>,
    pub relay_on: bool,
    /// The guests' link through musicplayer.vietnamcos.com and its QR code, once the room is open.
    pub relay_url: Option<String>,
    pub relay_qr: Option<String>,
    /// Why the relay is not working ("relayOff", "offline", ...), while it is not.
    pub relay_error: Option<String>,
}

fn info(host: &PartyHost) -> PartyInfo {
    let port = host.local.lock().ok().and_then(|l| l.as_ref().map(|s| s.port()));
    let url = port.and_then(|p| server::lan_ip().map(|ip| format!("http://{ip}:{p}/p/{}/", host.party.token())));
    let relay = host.relay.lock().ok().and_then(|r| r.as_ref().map(RelayRunner::status));
    PartyInfo {
        local_on: port.is_some(),
        local_qr: url.as_deref().map(server::qr_svg),
        local_url: url,
        relay_on: relay.is_some(),
        relay_qr: relay.as_ref().and_then(|r| r.link.as_deref().map(server::qr_svg)),
        relay_url: relay.as_ref().and_then(|r| r.link.clone()),
        relay_error: relay.and_then(|r| r.error),
    }
}

#[tauri::command]
pub fn party_info(host: State<'_, SharedParty>) -> PartyInfo {
    info(&host)
}

/// Turns the guests' page on the Wi-Fi on or off.
#[tauri::command]
pub fn party_set_local(on: bool, host: State<'_, SharedParty>, db: State<'_, SharedDb>) -> Result<PartyInfo, String> {
    let mut local = host.local.lock().map_err(|e| e.to_string())?;
    if on && local.is_none() {
        host.refresh_library(&db);
        *local = Some(LocalServer::start(host.party.clone(), server::FIRST_PORT)?);
    } else if !on {
        *local = None;
    }
    drop(local);
    Ok(info(&host))
}

/// Turns the party link through musicplayer.vietnamcos.com on or off (for guests not on the Wi-Fi).
#[tauri::command]
pub fn party_set_relay(on: bool, host: State<'_, SharedParty>, db: State<'_, SharedDb>) -> Result<PartyInfo, String> {
    let mut r = host.relay.lock().map_err(|e| e.to_string())?;
    if on && r.is_none() {
        host.refresh_library(&db);
        // MIU_PARTY_RELAY points Miu at another copy of the site (testing).
        let api = std::env::var("MIU_PARTY_RELAY").unwrap_or_else(|_| format!("{}api.php", relay::SITE));
        *r = Some(RelayRunner::start(host.party.clone(), Box::new(HttpSite::new(&api)), server::title_online));
    } else if !on {
        // Dropping it ends the room on the site.
        *r = None;
    }
    drop(r);
    Ok(info(&host))
}

#[tauri::command]
pub fn party_host_state(host: State<'_, SharedParty>) -> HostView {
    host_view(&host.party.queue())
}

#[tauri::command]
pub fn party_set_settings(settings: Settings, host: State<'_, SharedParty>) {
    host.party.change(|q| q.settings = settings);
}

#[tauri::command]
pub fn party_remove(id: u64, host: State<'_, SharedParty>) {
    host.party.change(|q| q.remove(id));
}

#[tauri::command]
pub fn party_approve(id: u64, host: State<'_, SharedParty>) {
    host.party.change(|q| q.approve(id));
}

/// The DJ adds a song from the library to the party queue.
#[tauri::command]
pub fn party_dj_add(song_id: i64, host: State<'_, SharedParty>, db: State<'_, SharedDb>) -> Result<(), String> {
    host.refresh_library(&db);
    let rows = db.lock().ok().and_then(|d| crate::library::songs::list_songs(&d).ok()).unwrap_or_default();
    let row = rows.iter().find(|r| r.id == song_id).ok_or("song not in the library")?;
    let item = Item::Song { song_id, title: row.title.clone(), artist: row.artist.clone().unwrap_or_default() };
    host.party.change(|q| q.add(HOST, "DJ", item)).map(|_| ()).map_err(|e| format!("{e:?}"))
}

/// The video on the party screen now, if any (the screen asks when it opens).
#[tauri::command]
pub fn party_video_now(host: State<'_, SharedParty>) -> Option<serde_json::Value> {
    host.video().map(|(id, title)| serde_json::json!({ "videoId": id, "title": title }))
}

/// The video ended (or the DJ skipped it): Auto DJ carries on with the next song.
#[tauri::command]
pub fn party_video_done(host: State<'_, SharedParty>, dj: State<'_, crate::dj::service::SharedDj>) {
    host.set_video(None);
    host.party.change(|q| q.clear_now());
    if let Some(s) = dj.lock().ok().as_ref().and_then(|g| g.as_ref()) {
        s.engine.set_hold(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, title: &str, rating: i64) -> crate::library::songs::SongRow {
        crate::library::songs::SongRow {
            id,
            title: title.into(),
            artist: Some("A".into()),
            album: None,
            duration_ms: None,
            format: "mp3".into(),
            rating,
            sample_rate: None,
            bit_depth: None,
            bitrate_kbps: None,
            path: "C:/x.mp3".into(),
            duplicate_group: None,
            album_artist: None,
            year: None,
            track_no: None,
            disc_no: None,
            has_artwork: false,
        }
    }

    #[test]
    fn guests_pick_from_everything_but_disliked_songs() {
        let lib = library_for_guests(&[row(1, "Liked", 1), row(2, "Plain", 0), row(3, "Disliked", -1)]);
        assert_eq!(lib.iter().map(|s| s.0).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(lib[0], (1, "Liked".into(), "A".into()));
    }

    #[test]
    fn the_dj_sees_the_queue_in_order_and_what_waits() {
        let mut q = Queue::default();
        q.settings.approval = true;
        let song = |id: i64| Item::Song { song_id: id, title: format!("S{id}"), artist: "A".into() };
        q.add(HOST, "DJ", song(1)).unwrap();
        q.add("ann", "Ann", song(2)).unwrap();
        let v = host_view(&q);
        assert_eq!(v.queue.len(), 1);
        assert_eq!(v.queue[0].added_name, "", "the DJ's own show no name");
        assert_eq!(v.waiting.len(), 1);
        assert_eq!(v.waiting[0].added_name, "Ann");
        assert!(v.settings.approval);
    }
}
