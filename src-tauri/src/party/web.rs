//! The guests' page and its little API (P.3): served by Miu on the Wi-Fi (and the same page by
//! the relay site). Requests are handled here as plain data so the whole API is tested without a
//! network; `server` only moves them in and out of the HTTP server.
//!
//!   GET  ./            the page          GET  ./api/state?guest=ID&since=V   queue (waits for a change)
//!   POST ./api/add     {guest,name,songId|url}   POST ./api/vote {guest,id,on}
//!   GET  ./api/search?q=words                     songs in the DJ's library

use std::sync::{Condvar, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::links::{self, Link};
use super::queue::{Item, Queue, Refusal, HOST};
use crate::library::artfetch::fold;

/// The guest page and its files (the relay site serves the same ones).
pub const PAGE_HTML: &str = include_str!("page/guest.html");
pub const PAGE_JS: &str = include_str!("page/guest.js");
pub const PAGE_CSS: &str = include_str!("page/guest.css");

/// How long a guest's page waits for the queue to change before asking again.
const LONG_POLL: Duration = Duration::from_secs(20);
const SEARCH_MAX: usize = 30;

/// One party: the queue the DJ's Miu holds, and the library guests can pick from.
pub struct Party {
    token: String,
    queue: Mutex<Queue>,
    changed: Condvar,
    library: RwLock<Vec<(i64, String, String)>>,
    wait: Mutex<Duration>,
}

pub struct Request<'a> {
    pub method: &'a str,
    /// The path after the party's own prefix ("/", "/api/state", ...).
    pub path: &'a str,
    pub query: &'a str,
    pub body: &'a [u8],
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn json(status: u16, v: &impl serde::Serialize) -> Self {
        Response { status, content_type: "application/json; charset=utf-8", body: serde_json::to_vec(v).unwrap_or_default() }
    }

    fn error(status: u16, code: &str) -> Self {
        Self::json(status, &serde_json::json!({ "error": code }))
    }

    fn file(content_type: &'static str, body: &str) -> Self {
        Response { status: 200, content_type, body: body.as_bytes().to_vec() }
    }
}

impl Party {
    pub fn new(token: &str) -> Self {
        Self { token: token.to_string(), queue: Mutex::new(Queue::default()), changed: Condvar::new(), library: RwLock::new(Vec::new()), wait: Mutex::new(LONG_POLL) }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// The songs guests may pick: (id, title, artist).
    pub fn set_library(&self, songs: Vec<(i64, String, String)>) {
        if let Ok(mut l) = self.library.write() {
            *l = songs;
        }
    }

    pub fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the queue and wakes every guest page waiting for news.
    pub fn change<T>(&self, f: impl FnOnce(&mut Queue) -> T) -> T {
        let r = f(&mut self.queue());
        self.changed.notify_all();
        r
    }

    #[cfg(test)]
    pub fn set_wait(&self, d: Duration) {
        *self.wait.lock().unwrap() = d;
    }

    fn library_song(&self, id: i64) -> Option<Item> {
        let lib = self.library.read().ok()?;
        lib.iter().find(|(i, ..)| *i == id).map(|(i, t, a)| Item::Song { song_id: *i, title: t.clone(), artist: a.clone() })
    }

    fn library_match(&self, title: &str) -> Option<Item> {
        let id = links::find_in_library(title, None, &self.library.read().ok()?)?;
        self.library_song(id)
    }

    fn search(&self, q: &str) -> Vec<serde_json::Value> {
        let words: Vec<String> = fold(q).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let Ok(lib) = self.library.read() else { return Vec::new() };
        lib.iter()
            .filter(|(_, t, a)| {
                let hay = fold(&format!("{t} {a}"));
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .take(SEARCH_MAX)
            .map(|(id, t, a)| serde_json::json!({ "songId": id, "title": t, "artist": a }))
            .collect()
    }

    /// The guest's view, waiting (up to the long-poll time) for a version newer than `since`.
    fn state(&self, guest: &str, since: Option<u64>) -> serde_json::Value {
        let wait = *self.wait.lock().unwrap_or_else(|e| e.into_inner());
        let deadline = Instant::now() + wait;
        let mut q = self.queue();
        while let Some(v) = since {
            if q.version() > v {
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            q = self.changed.wait_timeout(q, left).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        }
        serde_json::to_value(q.view(guest)).unwrap_or_default()
    }
}

/// `%XX` and `+` decoded.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| decode(v)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddBody {
    guest: String,
    #[serde(default)]
    name: String,
    song_id: Option<i64>,
    url: Option<String>,
}

#[derive(Deserialize)]
struct VoteBody {
    guest: String,
    id: u64,
    on: bool,
}

fn valid_guest(g: &str) -> bool {
    !g.is_empty() && g.len() <= 64 && g != HOST
}

/// Answers one request. `title_of(url)` looks a link's title up online (YouTube, Spotify).
pub fn handle(p: &Party, req: &Request, title_of: &dyn Fn(&str) -> Option<String>) -> Response {
    match (req.method, req.path) {
        ("GET", "/" | "") => Response::file("text/html; charset=utf-8", PAGE_HTML),
        ("GET", "/guest.js") => Response::file("text/javascript; charset=utf-8", PAGE_JS),
        ("GET", "/guest.css") => Response::file("text/css; charset=utf-8", PAGE_CSS),
        ("GET", "/api/state") => {
            let guest = param(req.query, "guest").unwrap_or_default();
            let since = param(req.query, "since").and_then(|v| v.parse().ok());
            Response::json(200, &p.state(&guest, since))
        }
        ("GET", "/api/search") => Response::json(200, &p.search(&param(req.query, "q").unwrap_or_default())),
        ("POST", "/api/add") => {
            let Ok(b) = serde_json::from_slice::<AddBody>(req.body) else { return Response::error(400, "badRequest") };
            if !valid_guest(&b.guest) {
                return Response::error(400, "badRequest");
            }
            let name: String = b.name.trim().chars().take(40).collect();
            let item = match (b.song_id, b.url.as_deref()) {
                (Some(id), _) => p.library_song(id),
                (None, Some(url)) => match links::parse(url) {
                    Some(Link::YouTube { video_id }) => {
                        // Videos keep their whole title (karaoke versions say so in it).
                        let title = title_of(&format!("https://www.youtube.com/watch?v={video_id}")).map(|t| t.trim().to_string()).unwrap_or_else(|| video_id.clone());
                        Some(Item::Video { video_id, title })
                    }
                    Some(Link::Spotify { url }) => title_of(&url).and_then(|t| p.library_match(&t)),
                    Some(Link::Apple { title }) => p.library_match(&title),
                    None => return Response::error(422, "unknownLink"),
                },
                (None, None) => return Response::error(400, "badRequest"),
            };
            let Some(item) = item else { return Response::error(422, "notInLibrary") };
            match p.change(|q| q.add(&b.guest, &name, item)) {
                Ok(id) => Response::json(200, &serde_json::json!({ "id": id })),
                Err(r) => Response::error(
                    409,
                    match r {
                        Refusal::Closed => "closed",
                        Refusal::Limit => "limit",
                        Refusal::JustPlayed => "justPlayed",
                    },
                ),
            }
        }
        ("POST", "/api/vote") => {
            let Ok(b) = serde_json::from_slice::<VoteBody>(req.body) else { return Response::error(400, "badRequest") };
            if !valid_guest(&b.guest) {
                return Response::error(400, "badRequest");
            }
            p.change(|q| q.vote(&b.guest, b.id, b.on));
            Response::json(200, &serde_json::json!({ "ok": true }))
        }
        _ => Response::error(404, "notFound"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::queue::Item;

    fn party() -> Party {
        let p = Party::new("tok123");
        p.set_library(vec![
            (1, "Love Yourself".into(), "Justin Bieber".into()),
            (2, "Đi Về Nhà".into(), "Đen, JustaTee".into()),
            (3, "Faded".into(), "Alan Walker".into()),
        ]);
        p
    }

    fn get(p: &Party, path: &str, query: &str) -> Response {
        handle(p, &Request { method: "GET", path, query, body: b"" }, &|_| None)
    }

    fn post(p: &Party, path: &str, body: serde_json::Value) -> Response {
        let titles = |url: &str| -> Option<String> {
            if url.contains("youtube") || url.contains("youtu.be") {
                Some("Rick Astley - Never Gonna Give You Up (Official Video)".into())
            } else if url.contains("spotify") {
                Some("Love Yourself".into())
            } else {
                None
            }
        };
        let body = body.to_string();
        handle(p, &Request { method: "POST", path, query: "", body: body.as_bytes() }, &titles)
    }

    fn json(r: &Response) -> serde_json::Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    #[test]
    fn the_page_and_its_files() {
        let p = party();
        let page = get(&p, "/", "");
        assert_eq!(page.status, 200);
        assert!(page.content_type.starts_with("text/html"));
        assert!(String::from_utf8_lossy(&page.body).contains("<html"));
        assert_eq!(get(&p, "/guest.js", "").status, 200);
        assert_eq!(get(&p, "/nope", "").status, 404);
    }

    #[test]
    fn guests_add_songs_and_videos_and_vote() {
        let p = party();
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "songId": 2 }));
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g2", "name": "Bob", "url": "https://youtu.be/dQw4w9WgXcQ" }));
        assert_eq!(r.status, 200);
        let state = json(&get(&p, "/api/state", "guest=g2"));
        let q = state["queue"].as_array().unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(q[0]["title"], "Đi Về Nhà");
        assert_eq!(q[1]["kind"], "video");
        assert_eq!(q[1]["title"], "Rick Astley - Never Gonna Give You Up (Official Video)", "a video keeps its whole title");
        assert_eq!(q[1]["mine"], true);
        let id = q[1]["id"].as_u64().unwrap();
        assert_eq!(post(&p, "/api/vote", serde_json::json!({ "guest": "g3", "id": id, "on": true })).status, 200);
        let state = json(&get(&p, "/api/state", "guest=g3"));
        assert_eq!(state["queue"][0]["kind"], "video", "two votes move the video up");
        assert_eq!(state["queue"][0]["voted"], true);
    }

    #[test]
    fn spotify_and_apple_links_find_the_song_in_the_library() {
        let p = party();
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "url": "https://open.spotify.com/track/abc" }));
        assert_eq!(r.status, 200);
        assert!(matches!(p.queue().ordered()[0].item, Item::Song { song_id: 1, .. }));
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "url": "https://music.apple.com/us/song/faded/123" }));
        assert_eq!(r.status, 200);
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "url": "https://music.apple.com/us/song/unknown-song/9" }));
        assert_eq!(r.status, 422);
        assert_eq!(json(&r)["error"], "notInLibrary");
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "url": "https://example.com/x" }));
        assert_eq!(json(&r)["error"], "unknownLink");
    }

    #[test]
    fn refusals_say_why() {
        let p = party();
        p.queue().settings.per_guest = 1;
        post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "songId": 1 }));
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "songId": 3 }));
        assert_eq!(r.status, 409);
        assert_eq!(json(&r)["error"], "limit");
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "songId": 99 }));
        assert_eq!(json(&r)["error"], "notInLibrary");
        assert_eq!(post(&p, "/api/add", serde_json::json!({ "nonsense": true })).status, 400);
        let r = post(&p, "/api/add", serde_json::json!({ "guest": "host", "name": "Ann", "songId": 3 }));
        assert_eq!(r.status, 400, "guests cannot pretend to be the DJ");
    }

    #[test]
    fn search_finds_library_songs_by_any_word() {
        let p = party();
        let r = json(&get(&p, "/api/search", "q=di%20ve"));
        assert_eq!(r.as_array().unwrap().len(), 1);
        assert_eq!(r[0]["songId"], 2);
        let r = json(&get(&p, "/api/search", "q=alan"));
        assert_eq!(r[0]["title"], "Faded");
        assert!(json(&get(&p, "/api/search", "q=")).as_array().unwrap().is_empty());
    }

    #[test]
    fn state_answers_at_once_when_something_changed_since() {
        let p = party();
        post(&p, "/api/add", serde_json::json!({ "guest": "g1", "name": "Ann", "songId": 1 }));
        let v = json(&get(&p, "/api/state", "guest=g1"))["version"].as_u64().unwrap();
        let start = std::time::Instant::now();
        let r = json(&get(&p, "/api/state", &format!("guest=g1&since={}", v - 1)));
        assert_eq!(r["version"].as_u64().unwrap(), v);
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
        // Nothing new: it waits (briefly in tests) and answers with the same version.
        p.set_wait(std::time::Duration::from_millis(50));
        let start = std::time::Instant::now();
        let r = json(&get(&p, "/api/state", &format!("guest=g1&since={v}")));
        assert!(start.elapsed() >= std::time::Duration::from_millis(45));
        assert_eq!(r["version"].as_u64().unwrap(), v);
    }
}
