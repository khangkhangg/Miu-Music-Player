//! The party through musicplayer.vietnamcos.com (P.6), for guests who cannot reach this PC on the
//! Wi-Fi. The site only passes messages: Miu opens a room, pushes the queue when it changes,
//! fetches what guests asked for (adds, votes), handles them exactly like the Wi-Fi page does and
//! sends back the answers. Guests' phones poll the site every few seconds.
//!
//! Protocol (all JSON, `api.php?op=..&r=ROOM`, the host sends its secret in `X-Party-Secret`):
//!   host:  create → {room, secret} · push {state, library?} · actions?after=N → {actions}
//!          · results {results: {id: {status, error?}}} · close
//!   guest: state?guest=G · add · vote · search?q=     (see relay/api.php)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::web::{self, Party, Request};

/// The site's party folder; guests open `?r=ROOM` there.
pub const SITE: &str = "https://musicplayer.vietnamcos.com/party/";
/// How often Miu asks the site for guests' actions.
const EVERY: Duration = Duration::from_secs(2);

/// The relay site, one call at a time (`HttpSite`, or a fake in tests).
pub trait Site {
    /// Calls `op`; an answer that is not 2xx comes back as its `error` code.
    fn call(&self, op: &str, room: Option<&str>, secret: Option<&str>, query: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String>;
}

/// Miu's side of one relayed party.
#[derive(Default)]
pub struct RelayHost {
    /// (room, secret) once the site has opened one.
    room: Option<(String, String)>,
    pushed: Option<u64>,
    library_sent: Option<u64>,
    after: u64,
}

impl RelayHost {
    /// The guests' link, once the room is open.
    pub fn link(&self) -> Option<String> {
        self.room.as_ref().map(|(r, _)| format!("{SITE}?r={r}"))
    }

    /// One round: open the room if needed, handle guests' actions, push the queue if it changed.
    pub fn step(&mut self, p: &Party, site: &dyn Site, title_of: impl Fn(&str) -> Option<String>) -> Result<(), String> {
        let r = self.round(p, site, &title_of);
        if r.as_ref().err().map(String::as_str) == Some("roomGone") {
            // The room ran out or the admin ended it: open a new one next time (a new link).
            *self = RelayHost::default();
        }
        r
    }

    fn round(&mut self, p: &Party, site: &dyn Site, title_of: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
        if self.room.is_none() {
            let v = site.call("create", None, None, "", None)?;
            let (Some(room), Some(secret)) = (v["room"].as_str(), v["secret"].as_str()) else { return Err("badAnswer".into()) };
            self.room = Some((room.to_string(), secret.to_string()));
        }
        let (room, secret) = self.room.clone().unwrap_or_default();
        let call = |op: &str, query: &str, body| site.call(op, Some(&room), Some(&secret), query, body);

        let actions = call("actions", &format!("after={}", self.after), None)?;
        let mut results = serde_json::Map::new();
        for a in actions["actions"].as_array().into_iter().flatten() {
            let Some(id) = a["id"].as_u64() else { continue };
            self.after = self.after.max(id);
            let answer = match a["op"].as_str() {
                Some(op @ ("add" | "vote")) => {
                    let body = a["body"].to_string();
                    let res = web::handle(p, &Request { method: "POST", path: &format!("/api/{op}"), query: "", body: body.as_bytes() }, title_of);
                    let error = serde_json::from_slice::<serde_json::Value>(&res.body).ok().and_then(|v| v["error"].as_str().map(str::to_string));
                    serde_json::json!({ "status": res.status, "error": error })
                }
                _ => serde_json::json!({ "status": 400, "error": "badRequest" }),
            };
            results.insert(id.to_string(), answer);
        }
        if !results.is_empty() {
            call("results", "", Some(serde_json::json!({ "results": results })))?;
        }

        let state = p.queue().relay_view();
        let (lib_gen, library) = p.library();
        let send_library = self.library_sent != Some(lib_gen);
        if self.pushed != Some(state.version) || send_library {
            let mut body = serde_json::json!({ "state": state });
            if send_library {
                body["library"] = library.iter().map(|(id, t, a)| serde_json::json!({ "songId": id, "title": t, "artist": a })).collect();
            }
            call("push", "", Some(body))?;
            self.pushed = Some(state.version);
            self.library_sent = Some(lib_gen);
        }
        Ok(())
    }

    /// Ends the room on the site (guests see that the party has ended).
    pub fn close(&mut self, site: &dyn Site) {
        if let Some((room, secret)) = self.room.take() {
            let _ = site.call("close", Some(&room), Some(&secret), "", None);
        }
    }
}

/// The real site over HTTPS.
pub struct HttpSite {
    pub api: String,
    agent: ureq::Agent,
}

impl HttpSite {
    pub fn new(api: &str) -> Self {
        let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).http_status_as_error(false).build().into();
        Self { api: api.to_string(), agent }
    }
}

impl Site for HttpSite {
    fn call(&self, op: &str, room: Option<&str>, secret: Option<&str>, query: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
        let mut url = format!("{}?op={op}", self.api);
        if let Some(r) = room {
            url.push_str(&format!("&r={}", super::server::escape(r)));
        }
        if !query.is_empty() {
            url.push('&');
            url.push_str(query);
        }
        let res = match body {
            Some(b) => {
                let mut req = self.agent.post(&url).header("content-type", "application/json");
                if let Some(s) = secret {
                    req = req.header("x-party-secret", s);
                }
                req.send(b.to_string())
            }
            None => {
                let mut req = self.agent.post(&url);
                if let Some(s) = secret {
                    req = req.header("x-party-secret", s);
                }
                req.send_empty()
            }
        };
        let mut res = res.map_err(|_| "offline".to_string())?;
        let status = res.status().as_u16();
        let text = res.body_mut().read_to_string().unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        if (200..300).contains(&status) {
            Ok(v)
        } else {
            Err(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("http{status}")))
        }
    }
}

/// What the DJ's screen shows about the relay.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatus {
    pub link: Option<String>,
    pub error: Option<String>,
}

/// The relay running in the background while it is on.
pub struct RelayRunner {
    stop: Arc<AtomicBool>,
    pub status: Arc<Mutex<RelayStatus>>,
}

impl RelayRunner {
    pub fn start(party: Arc<Party>, site: Box<dyn Site + Send>, title_of: fn(&str) -> Option<String>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(RelayStatus::default()));
        let (s, st) = (stop.clone(), status.clone());
        std::thread::spawn(move || {
            let mut host = RelayHost::default();
            while !s.load(Ordering::Relaxed) {
                let r = host.step(&party, site.as_ref(), title_of);
                if let Ok(mut g) = st.lock() {
                    *g = RelayStatus { link: host.link(), error: r.err() };
                }
                // Sleep in short naps so turning it off is quick.
                for _ in 0..(EVERY.as_millis() / 100) {
                    if s.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            host.close(site.as_ref());
        });
        Self { stop, status }
    }

    pub fn status(&self) -> RelayStatus {
        self.status.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for RelayRunner {
    /// Stops it without waiting: the thread ends the room on the site on its way out.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::queue::{Item, HOST};
    use crate::party::web::Party;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// The relay site in memory, as relay/api.php behaves.
    #[derive(Default)]
    struct FakeSite {
        rooms: RefCell<HashMap<String, serde_json::Value>>,
        actions: RefCell<Vec<serde_json::Value>>,
        results: RefCell<serde_json::Map<String, serde_json::Value>>,
        pushes: RefCell<usize>,
        off: RefCell<bool>,
    }

    impl Site for FakeSite {
        fn call(&self, op: &str, room: Option<&str>, _secret: Option<&str>, query: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
            if *self.off.borrow() {
                return Err("relayOff".into());
            }
            match op {
                "create" => {
                    self.rooms.borrow_mut().insert("ABC123".into(), serde_json::Value::Null);
                    Ok(serde_json::json!({ "room": "ABC123", "secret": "s3cret" }))
                }
                "push" => {
                    *self.pushes.borrow_mut() += 1;
                    self.rooms.borrow_mut().insert(room.unwrap().into(), body.unwrap());
                    Ok(serde_json::json!({ "ok": true }))
                }
                "actions" => {
                    let after: u64 = query.strip_prefix("after=").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let list: Vec<_> = self.actions.borrow().iter().filter(|a| a["id"].as_u64().unwrap() > after).cloned().collect();
                    Ok(serde_json::json!({ "actions": list }))
                }
                "results" => {
                    for (k, v) in body.unwrap()["results"].as_object().unwrap() {
                        self.results.borrow_mut().insert(k.clone(), v.clone());
                    }
                    Ok(serde_json::json!({ "ok": true }))
                }
                "close" => Ok(serde_json::json!({ "ok": true })),
                _ => Err(format!("unknown op {op}")),
            }
        }
    }

    fn party() -> Party {
        let p = Party::new("tok");
        p.set_library(vec![(1, "Love Yourself".into(), "Justin Bieber".into()), (2, "Faded".into(), "Alan Walker".into())]);
        p
    }

    #[test]
    fn opens_a_room_and_pushes_the_queue_with_the_library() {
        let (site, p) = (FakeSite::default(), party());
        let mut host = RelayHost::default();
        host.step(&p, &site, |_| None).unwrap();
        assert_eq!(host.link(), Some("https://musicplayer.vietnamcos.com/party/?r=ABC123".to_string()));
        let pushed = site.rooms.borrow()["ABC123"].clone();
        assert_eq!(pushed["library"].as_array().unwrap().len(), 2, "the first push carries the library");
        assert_eq!(pushed["state"]["queue"].as_array().unwrap().len(), 0);
        host.step(&p, &site, |_| None).unwrap();
        assert_eq!(*site.pushes.borrow(), 1, "nothing changed: nothing pushed");
        p.change(|q| q.add(HOST, "DJ", Item::Song { song_id: 2, title: "Faded".into(), artist: "Alan Walker".into() })).unwrap();
        host.step(&p, &site, |_| None).unwrap();
        assert_eq!(*site.pushes.borrow(), 2);
        let pushed = site.rooms.borrow()["ABC123"].clone();
        assert!(pushed.get("library").is_none(), "the library only once");
        assert_eq!(pushed["state"]["queue"][0]["title"], "Faded");
    }

    #[test]
    fn guests_actions_are_handled_like_on_the_wifi_and_answered() {
        let (site, p) = (FakeSite::default(), party());
        p.queue().settings.per_guest = 1;
        let mut host = RelayHost::default();
        host.step(&p, &site, |_| None).unwrap();
        site.actions.borrow_mut().extend([
            serde_json::json!({ "id": 1, "op": "add", "body": { "guest": "g1", "name": "Ann", "songId": 1 } }),
            serde_json::json!({ "id": 2, "op": "add", "body": { "guest": "g1", "name": "Ann", "songId": 2 } }),
            serde_json::json!({ "id": 3, "op": "add", "body": { "guest": "g2", "name": "Bob", "url": "https://youtu.be/dQw4w9WgXcQ" } }),
            serde_json::json!({ "id": 4, "op": "vote", "body": { "guest": "g3", "id": 1, "on": true } }),
            serde_json::json!({ "id": 5, "op": "steal", "body": {} }),
        ]);
        host.step(&p, &site, |_| Some("Karaoke - Song".into())).unwrap();
        let r = site.results.borrow();
        assert_eq!(r["1"]["status"], 200);
        assert_eq!(r["2"]["error"], "limit", "the guest hears why");
        assert_eq!(r["3"]["status"], 200);
        assert_eq!(r["5"]["status"], 400, "unknown requests are refused");
        drop(r);
        let q = p.queue();
        assert_eq!(q.ordered().len(), 2);
        assert_eq!(q.ordered()[0].votes.len(), 2, "the vote counted");
        drop(q);
        let pushed = site.rooms.borrow()["ABC123"].clone();
        let entry = &pushed["state"]["queue"][0];
        assert_eq!(entry["voters"].as_array().unwrap().len(), 2, "the site works out each guest\x27s view from these");
        assert_eq!(entry["addedBy"], "g1");
        // The same actions are not handled twice.
        host.step(&p, &site, |_| None).unwrap();
        assert_eq!(p.queue().ordered()[0].votes.len(), 2);
    }

    #[test]
    fn when_the_site_is_off_it_says_so() {
        let (site, p) = (FakeSite::default(), party());
        *site.off.borrow_mut() = true;
        let mut host = RelayHost::default();
        assert_eq!(host.step(&p, &site, |_| None), Err("relayOff".into()));
        assert_eq!(host.link(), None);
    }
}
