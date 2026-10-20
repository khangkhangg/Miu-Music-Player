//! Discover (P.4): Apple's public podcast charts (top shows per country) and search, plus the
//! lookup that turns a chart entry's Apple id into its feed address.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub apple_id: i64,
    pub title: String,
    pub author: String,
    pub artwork_url: Option<String>,
    pub genre: Option<String>,
    /// Known for search results; chart entries need a lookup first.
    pub feed_url: Option<String>,
}

fn json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("unexpected answer from Apple: {e}"))
}

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string)
}

/// Apple artwork addresses end in "<w>x<h>bb.jpg"; ask for 600 px instead of 100.
fn larger(url: Option<String>) -> Option<String> {
    url.map(|u| u.replace("/100x100bb.", "/600x600bb."))
}

/// Top shows from the marketing-tools chart feed.
pub fn parse_chart(text: &str) -> Result<Vec<Found>, String> {
    let v = json(text)?;
    let results = v.pointer("/feed/results").and_then(Value::as_array).ok_or("no chart in the answer")?;
    Ok(results
        .iter()
        .filter_map(|r| {
            Some(Found {
                apple_id: s(r, "id")?.parse().ok()?,
                title: s(r, "name")?,
                author: s(r, "artistName").unwrap_or_default(),
                artwork_url: larger(s(r, "artworkUrl100")),
                genre: r.get("genres").and_then(Value::as_array).and_then(|g| g.first()).and_then(|g| s(g, "name")),
                feed_url: None,
            })
        })
        .collect())
}

/// Search results that have a feed (some podcasts on Apple do not publish one).
pub fn parse_search(text: &str) -> Result<Vec<Found>, String> {
    let v = json(text)?;
    let results = v.get("results").and_then(Value::as_array).ok_or("no results in the answer")?;
    Ok(results
        .iter()
        .filter_map(|r| {
            Some(Found {
                apple_id: r.get("collectionId").and_then(Value::as_i64)?,
                title: s(r, "collectionName")?,
                author: s(r, "artistName").unwrap_or_default(),
                artwork_url: s(r, "artworkUrl600").or_else(|| larger(s(r, "artworkUrl100"))),
                genre: s(r, "primaryGenreName"),
                feed_url: Some(s(r, "feedUrl")?),
            })
        })
        .collect())
}

/// The feed address from a lookup answer.
pub fn parse_lookup(text: &str) -> Option<String> {
    let v = json(text).ok()?;
    v.get("results")?.as_array()?.iter().find_map(|r| s(r, "feedUrl"))
}

fn country(code: &str) -> Result<String, String> {
    let c = code.trim().to_ascii_lowercase();
    if c.len() == 2 && c.bytes().all(|b| b.is_ascii_lowercase()) { Ok(c) } else { Err(format!("not a country code: {code}")) }
}

/// Percent-encode a query value (RFC 3986 unreserved characters stay).
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn chart_url(country_code: &str, count: u32) -> Result<String, String> {
    Ok(format!("https://rss.marketingtools.apple.com/api/v2/{}/podcasts/top/{}/podcasts.json", country(country_code)?, count.clamp(1, 100)))
}

pub fn search_url(term: &str, country_code: &str) -> Result<String, String> {
    Ok(format!("https://itunes.apple.com/search?media=podcast&entity=podcast&limit=40&country={}&term={}", country(country_code)?, encode(term.trim())))
}

pub fn lookup_url(apple_id: i64) -> String {
    format!("https://itunes.apple.com/lookup?entity=podcast&id={apple_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = r#"{"feed":{"title":"Top Shows","results":[
      {"artistName":"Tri Kỷ Cảm Xúc","id":"1355572154","name":"Tri Kỷ Cảm Xúc","kind":"podcasts",
       "artworkUrl100":"https://is1-ssl.mzstatic.com/image/thumb/Podcasts/ab/cd/100x100bb.jpg",
       "genres":[{"genreId":"1324","name":"Society & Culture","url":"https://itunes.apple.com/genre/id1324"}],
       "url":"https://podcasts.apple.com/vn/podcast/id1355572154"},
      {"artistName":"Thuần","id":"1571496739","name":"Thuần Podcast","kind":"podcasts","artworkUrl100":"https://x/100x100bb.jpg","genres":[],"url":"u"}
    ]}}"#;

    const SEARCH: &str = r#"{"resultCount":2,"results":[
      {"wrapperType":"track","kind":"podcast","collectionId":1582689579,"collectionName":"NHẠC TÌNH HOA NIÊN","artistName":"uwaaliz.ne",
       "feedUrl":"https://feeds.acast.com/public/shows/613e193ae8d2fd00138096b8","artworkUrl600":"https://x/600x600bb.jpg","primaryGenreName":"Music"},
      {"wrapperType":"track","kind":"podcast","collectionId":42,"collectionName":"No feed","artistName":"A"}
    ]}"#;

    #[test]
    fn chart_entries_become_shows_with_large_artwork_and_their_category() {
        let shows = parse_chart(CHART).unwrap();
        assert_eq!(shows.len(), 2);
        let s = &shows[0];
        assert_eq!((s.apple_id, s.title.as_str(), s.author.as_str()), (1_355_572_154, "Tri Kỷ Cảm Xúc", "Tri Kỷ Cảm Xúc"));
        assert_eq!(s.artwork_url.as_deref(), Some("https://is1-ssl.mzstatic.com/image/thumb/Podcasts/ab/cd/600x600bb.jpg"));
        assert_eq!(s.genre.as_deref(), Some("Society & Culture"));
        assert_eq!(s.feed_url, None, "charts do not carry the feed address");
        assert_eq!(shows[1].genre, None);
    }

    #[test]
    fn search_results_carry_their_feed_and_ones_without_are_dropped() {
        let shows = parse_search(SEARCH).unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].feed_url.as_deref(), Some("https://feeds.acast.com/public/shows/613e193ae8d2fd00138096b8"));
        assert_eq!(shows[0].genre.as_deref(), Some("Music"));
        assert_eq!(shows[0].apple_id, 1_582_689_579);
    }

    #[test]
    fn a_lookup_gives_the_feed_address() {
        assert_eq!(parse_lookup(SEARCH).as_deref(), Some("https://feeds.acast.com/public/shows/613e193ae8d2fd00138096b8"));
        assert_eq!(parse_lookup(r#"{"resultCount":0,"results":[]}"#), None);
    }

    #[test]
    fn addresses_for_vietnam_and_the_us() {
        assert_eq!(chart_url("vn", 50).unwrap(), "https://rss.marketingtools.apple.com/api/v2/vn/podcasts/top/50/podcasts.json");
        assert_eq!(chart_url("US", 25).unwrap(), "https://rss.marketingtools.apple.com/api/v2/us/podcasts/top/25/podcasts.json");
        assert!(chart_url("../x", 5).is_err(), "only two-letter country codes");
        assert_eq!(search_url("nhạc trịnh & co", "vn").unwrap(),
            "https://itunes.apple.com/search?media=podcast&entity=podcast&limit=40&country=vn&term=nh%E1%BA%A1c%20tr%E1%BB%8Bnh%20%26%20co");
        assert_eq!(lookup_url(1_355_572_154), "https://itunes.apple.com/lookup?entity=podcast&id=1355572154");
    }

    #[test]
    fn not_json_is_an_error() {
        assert!(parse_chart("<html>").is_err());
        assert!(parse_search("nope").is_err());
    }
}

// Machine test: talks to Apple (needs the internet).
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::podcast::fetch::{fetch, Fetched};

    fn get(url: &str) -> String {
        match fetch(url, None, None).unwrap() {
            Fetched::Feed { body, .. } => String::from_utf8(body).unwrap(),
            Fetched::NotModified => panic!("no body"),
        }
    }

    #[test]
    #[ignore]
    fn vietnam_and_us_charts_lead_to_real_feeds() {
        for cc in ["vn", "us"] {
            let chart = parse_chart(&get(&chart_url(cc, 10).unwrap())).unwrap();
            println!("{cc}: {:?}", chart.iter().take(3).map(|s| &s.title).collect::<Vec<_>>());
            assert!(chart.len() >= 5, "{cc} chart");
            let feed = parse_lookup(&get(&lookup_url(chart[0].apple_id))).expect("first chart entry has a feed");
            assert!(feed.starts_with("http"), "{feed}");
        }
        let found = parse_search(&get(&search_url("nhạc trịnh", "vn").unwrap())).unwrap();
        println!("search: {:?}", found.iter().take(3).map(|s| &s.title).collect::<Vec<_>>());
        assert!(!found.is_empty());
    }
}
