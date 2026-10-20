//! Fetching podcast feeds (P.1): plain HTTPS GET with a timeout and a size cap, and a conditional
//! request (ETag / Last-Modified) so an unchanged feed costs almost nothing to check.

use std::time::Duration;

use crate::library::db::Db;
use crate::podcast::{feed, store};

/// Feeds are text; anything bigger than this is not a feed we want.
const MAX_FEED_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    NotModified,
    Feed { body: Vec<u8>, etag: Option<String>, last_modified: Option<String> },
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent(concat!("MiuMusic/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// GET a feed. With the validators of the last fetch, an unchanged feed answers `NotModified`.
pub fn fetch(url: &str, etag: Option<&str>, last_modified: Option<&str>) -> Result<Fetched, String> {
    let lower = url.trim().to_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(format!("not a web address: {url}"));
    }
    let mut req = agent().get(url.trim());
    if let Some(e) = etag {
        req = req.header("If-None-Match", e);
    }
    if let Some(m) = last_modified {
        req = req.header("If-Modified-Since", m);
    }
    let mut res = req.call().map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    if status == 304 {
        return Ok(Fetched::NotModified);
    }
    if !(200..300).contains(&status) {
        return Err(format!("the server answered {status}"));
    }
    let header = |name: &str| res.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let (etag, last_modified) = (header("etag"), header("last-modified"));
    let body = res.body_mut().with_config().limit(MAX_FEED_BYTES).read_to_vec().map_err(|e| e.to_string())?;
    Ok(Fetched::Feed { body, etag, last_modified })
}

/// Fetch, parse and subscribe to the feed at `url`. Returns the show's id.
pub fn subscribe_url(db: &Db, url: &str, now: u64) -> Result<i64, String> {
    let Fetched::Feed { body, etag, last_modified } = fetch(url, None, None)? else {
        return Err("the server sent no feed".into());
    };
    let (show, episodes) = feed::parse(url.trim(), &body)?;
    let id = store::subscribe(db, &show, &episodes, now).map_err(|e| e.to_string())?;
    store::set_validators(db, id, etag.as_deref(), last_modified.as_deref()).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Check a show's feed again; returns how many new episodes arrived.
pub fn refresh_show(db: &Db, id: i64, now: u64) -> Result<usize, String> {
    let (url, etag, modified) = store::validators(db, id).map_err(|e| e.to_string())?.ok_or("no such podcast")?;
    match fetch(&url, etag.as_deref(), modified.as_deref())? {
        Fetched::NotModified => {
            store::touch(db, id, now).map_err(|e| e.to_string())?;
            Ok(0)
        }
        Fetched::Feed { body, etag, last_modified } => {
            let (show, episodes) = feed::parse(&url, &body)?;
            let added = store::refresh(db, id, &show, &episodes, now).map_err(|e| e.to_string())?;
            store::set_validators(db, id, etag.as_deref(), last_modified.as_deref()).map_err(|e| e.to_string())?;
            Ok(added)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::Db;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    const FEED: &str = r#"<?xml version="1.0"?><rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel>
<title>Hôm Nay Nghe Gì</title><itunes:author>Đài Mẫu</itunes:author>
<item><title>Tập 1</title><guid>1</guid><enclosure url="https://cdn.example.vn/1.mp3" type="audio/mpeg" length="10"/></item>
</channel></rss>"#;

    /// A one-file web server: answers 304 when the client already has ETag "v1", else the feed.
    /// Serves `requests` requests, then stops. Returns the base URL.
    fn server(requests: usize, status_for_feed: u16) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/feed.xml", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut has_etag = false;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    has_etag |= line.to_lowercase().starts_with("if-none-match: \"v1\"");
                }
                let reply = if has_etag {
                    "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\n\r\n".to_string()
                } else if status_for_feed != 200 {
                    format!("HTTP/1.1 {status_for_feed} Nope\r\nContent-Length: 0\r\n\r\n")
                } else {
                    format!("HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\n\r\n{FEED}", FEED.len())
                };
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        url
    }

    #[test]
    fn a_feed_is_fetched_with_its_validators() {
        let url = server(1, 200);
        match fetch(&url, None, None).unwrap() {
            Fetched::Feed { body, etag, .. } => {
                assert!(String::from_utf8(body).unwrap().contains("Hôm Nay Nghe Gì"));
                assert_eq!(etag.as_deref(), Some("\"v1\""));
            }
            Fetched::NotModified => panic!("expected the feed"),
        }
    }

    #[test]
    fn an_unchanged_feed_is_not_downloaded_again() {
        let url = server(1, 200);
        assert_eq!(fetch(&url, Some("\"v1\""), None).unwrap(), Fetched::NotModified);
    }

    #[test]
    fn http_errors_say_what_happened() {
        let url = server(1, 404);
        let err = fetch(&url, None, None).unwrap_err();
        assert!(err.contains("404"), "{err}");
    }

    #[test]
    fn subscribing_by_address_fetches_parses_and_stores() {
        let url = server(2, 200);
        let db = Db::open_in_memory().unwrap();
        let id = subscribe_url(&db, &url, 10).unwrap();
        let shows = crate::podcast::store::list_shows(&db).unwrap();
        assert_eq!((shows[0].id, shows[0].title.as_str(), shows[0].episodes), (id, "Hôm Nay Nghe Gì", 1));
        assert_eq!(refresh_show(&db, id, 20).unwrap(), 0, "unchanged feed: 304, nothing new");
        assert_eq!(crate::podcast::store::list_shows(&db).unwrap()[0].last_checked, Some(20));
    }

    #[test]
    fn only_web_addresses_are_accepted() {
        assert!(fetch("file:///C:/Windows/win.ini", None, None).is_err());
        assert!(fetch("not a url", None, None).is_err());
    }
}
