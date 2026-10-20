//! LibriVox (A.1): free public-domain audiobooks, read by volunteers. The catalogue API answers
//! JSON; each book also has an RSS feed of its chapters, which Miu reads like a podcast feed.
//! Commercial services (Audible, Storytel, Libby, …) offer no download API and sell DRM files,
//! so they are out of scope.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub id: i64,
    pub title: String,
    /// "Robert Louis Stevenson"; several authors joined with ", ".
    pub author: String,
    /// As LibriVox names it: "English", "Spanish", …
    pub language: String,
    /// Plain text (the catalogue's HTML tags removed).
    pub description: Option<String>,
    pub duration_ms: Option<u64>,
    pub chapters: u32,
    /// The chapter feed: also the book's key in the library.
    pub rss_url: String,
    pub page_url: Option<String>,
    pub cover_url: Option<String>,
}

/// Fields asked for in lists (the full answer carries every chapter and is much larger).
const FIELDS: &str = "%7Bid,title,description,language,num_sections,url_rss,url_librivox,totaltimesecs,authors,coverart_thumbnail,coverart_jpg%7D";
const API: &str = "https://librivox.org/api/feed/audiobooks?format=json&coverart=1";

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string)
}

/// Numbers come as strings ("26") or numbers (27008).
fn n(v: &Value, key: &str) -> Option<u64> {
    let x = v.get(key)?;
    x.as_u64().or_else(|| x.as_str()?.trim().parse().ok())
}

/// The catalogue's descriptions hold a little HTML (`<em>`, `<br>`, entities): keep the words.
pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn authors(v: &Value) -> String {
    let names: Vec<String> = v
        .get("authors")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let name = format!("{} {}", s(p, "first_name").unwrap_or_default(), s(p, "last_name").unwrap_or_default());
                    let name = name.trim().to_string();
                    (!name.is_empty()).then_some(name)
                })
                .collect()
        })
        .unwrap_or_default();
    names.join(", ")
}

fn book(v: &Value) -> Option<Book> {
    Some(Book {
        id: n(v, "id")? as i64,
        title: s(v, "title")?,
        author: authors(v),
        language: s(v, "language").unwrap_or_default(),
        description: s(v, "description").map(|d| plain_text(&d)).filter(|d| !d.is_empty()),
        duration_ms: n(v, "totaltimesecs").filter(|t| *t > 0).map(|t| t * 1000),
        chapters: n(v, "num_sections").unwrap_or(0) as u32,
        rss_url: s(v, "url_rss")?,
        page_url: s(v, "url_librivox"),
        cover_url: s(v, "coverart_jpg").or_else(|| s(v, "coverart_thumbnail")),
    })
}

/// Books in a catalogue answer. "Could not be found" is an empty list, not an error.
pub fn parse_books(text: &str) -> Result<Vec<Book>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("unexpected answer from LibriVox: {e}"))?;
    if let Some(books) = v.get("books").and_then(Value::as_array) {
        return Ok(books.iter().filter_map(book).collect());
    }
    match s(&v, "error") {
        Some(e) if e.to_lowercase().contains("could not be found") => Ok(Vec::new()),
        Some(e) => Err(e),
        None => Err("no books in the answer".into()),
    }
}

/// The catalogue answer at `url`. LibriVox says "not found" with HTTP 404 and a JSON body, so
/// that counts as an answer too.
pub fn get(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(40)))
        .http_status_as_error(false)
        .user_agent(concat!("MiuMusic/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut res = agent.get(url).call().map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) && status != 404 {
        return Err(format!("LibriVox answered {status}"));
    }
    res.body_mut().with_config().limit(10 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
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

/// LibriVox files titles without a leading article ("Count of Monte Cristo").
fn without_article(term: &str) -> &str {
    let t = term.trim();
    for a in ["the ", "a ", "an "] {
        if t.len() > a.len() && t[..a.len()].eq_ignore_ascii_case(a) {
            return t[a.len()..].trim_start();
        }
    }
    t
}

/// Two searches cover what people type: titles starting with the words, and authors by last name.
pub fn search_urls(term: &str, limit: u32) -> Vec<String> {
    let term = term.trim();
    if term.is_empty() {
        return Vec::new();
    }
    let limit = limit.clamp(1, 100);
    let last = term.split_whitespace().last().unwrap_or(term);
    vec![
        format!("{API}&fields={FIELDS}&limit={limit}&title=%5E{}", encode(without_article(term))),
        format!("{API}&fields={FIELDS}&limit={limit}&author={}", encode(last)),
    ]
}

/// Books catalogued or updated since `since` (seconds since 1970).
pub fn recent_url(since: i64, limit: u32) -> String {
    format!("{API}&fields={FIELDS}&limit={}&since={since}", limit.clamp(1, 500))
}

/// One book with everything.
pub fn book_url(id: i64) -> String {
    format!("{API}&fields={FIELDS}&id={id}")
}

/// Title matches first, then author matches, each book once.
pub fn merge(lists: Vec<Vec<Book>>) -> Vec<Book> {
    let mut seen = std::collections::HashSet::new();
    lists.into_iter().flatten().filter(|b| seen.insert(b.id)).collect()
}

/// Newest catalogue entries first (ids grow as books are added).
pub fn newest_first(mut books: Vec<Book>) -> Vec<Book> {
    books.sort_by(|a, b| b.id.cmp(&a.id));
    books
}

/// Books recorded in `language` ("English", "Vietnamese"; any case). Empty = all of them.
pub fn in_language(books: Vec<Book>, language: &str) -> Vec<Book> {
    let want = language.trim();
    if want.is_empty() {
        return books;
    }
    books.into_iter().filter(|b| b.language.eq_ignore_ascii_case(want)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: &str = include_str!("fixtures/search_title.json");
    const AUTHOR: &str = include_str!("fixtures/search_author.json");
    const RECENT: &str = include_str!("fixtures/recent.json");
    const NOT_FOUND: &str = include_str!("fixtures/not_found.json");

    #[test]
    fn a_catalogue_answer_becomes_books_with_their_details() {
        let books = parse_books(TITLE).unwrap();
        assert_eq!(books.len(), 3);
        let b = &books[0];
        assert_eq!((b.id, b.title.as_str(), b.author.as_str(), b.language.as_str()), (449, "Treasure Island", "Robert Louis Stevenson", "English"));
        assert_eq!(b.duration_ms, Some(27_008_000));
        assert_eq!(b.chapters, 26);
        assert_eq!(b.rss_url, "https://librivox.org/rss/449");
        assert_eq!(b.page_url.as_deref(), Some("https://librivox.org/treasure-island-by-robert-louis-stevenson/"));
        assert_eq!(b.cover_url.as_deref(), Some("https://archive.org/download/treasureisland_librivox/treasure_island_1.jpg"));
        assert!(b.description.as_deref().unwrap().starts_with("Treasure Island is an adventure novel"));
        assert_eq!(books[2].author, "Selma Lagerlöf", "names outside ASCII");
    }

    #[test]
    fn descriptions_lose_their_html() {
        let books = parse_books(AUTHOR).unwrap();
        let d = books[0].description.as_deref().unwrap();
        assert!(d.starts_with("Adventures of Huckleberry Finn (1884) by Mark Twain"), "{d}");
        assert!(!d.contains('<'));
        assert_eq!(plain_text("A <em>tale</em>&nbsp;of<br><br>two &amp; more"), "A tale of two & more");
    }

    #[test]
    fn nothing_found_is_an_empty_list_and_garbage_is_an_error() {
        assert_eq!(parse_books(NOT_FOUND).unwrap(), Vec::new());
        assert!(parse_books("<html>").is_err());
        assert!(parse_books(r#"{"error":"Database down"}"#).is_err());
    }

    #[test]
    fn search_looks_up_titles_and_authors() {
        let urls = search_urls("The Treasure Island", 20);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].ends_with("&limit=20&title=%5ETreasure%20Island"), "{}", urls[0]);
        assert!(urls[1].ends_with("&limit=20&author=Island"), "{}", urls[1]);
        assert!(search_urls("Mark Twain", 20)[1].ends_with("&author=Twain"));
        assert!(search_urls("Lagerlöf", 20)[1].ends_with("&author=Lagerl%C3%B6f"));
        assert!(search_urls("   ", 20).is_empty());
        assert!(urls[0].starts_with("https://librivox.org/api/feed/audiobooks?format=json&coverart=1&fields="));
        assert_eq!(recent_url(1_790_000_000, 200), format!("{API}&fields={FIELDS}&limit=200&since=1790000000"));
        assert!(book_url(449).ends_with("&id=449"));
    }

    #[test]
    fn title_matches_come_first_and_each_book_once() {
        let titles = parse_books(TITLE).unwrap();
        let mut authors = parse_books(AUTHOR).unwrap();
        authors.insert(0, titles[0].clone());
        let merged = merge(vec![titles, authors]);
        assert_eq!(merged.iter().map(|b| b.id).collect::<Vec<_>>(), [449, 1557, 4052, 59, 141]);
    }

    #[test]
    fn browse_lists_newest_first_and_by_language() {
        let books = newest_first(parse_books(RECENT).unwrap());
        assert_eq!(books.iter().map(|b| b.id).collect::<Vec<_>>(), [22142, 21971, 18498, 16418]);
        let spanish = in_language(books.clone(), "spanish");
        assert_eq!(spanish.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["Resurrección"]);
        assert_eq!(in_language(books.clone(), "English").len(), 2);
        assert!(in_language(books.clone(), "Vietnamese").is_empty());
        assert_eq!(in_language(books, "").len(), 4, "no language: everything");
    }
}

// Machine test: talks to LibriVox (needs the internet).
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::podcast::fetch::{fetch, Fetched};

    #[test]
    #[ignore]
    fn search_recent_and_a_chapter_feed_from_the_real_catalogue() {
        let found = merge(search_urls("treasure island", 10).iter().map(|u| parse_books(&get(u).unwrap()).unwrap()).collect());
        println!("search: {:?}", found.iter().take(3).map(|b| (&b.title, &b.author)).collect::<Vec<_>>());
        let b = found.iter().find(|b| b.id == 449).expect("Treasure Island");
        let recent = parse_books(&get(&recent_url(crate::util::time::now_unix() as i64 - 30 * 86_400, 50)).unwrap()).unwrap();
        println!("recent: {} books", recent.len());
        assert!(!recent.is_empty());
        let rss = match fetch(&b.rss_url, None, None).unwrap() {
            Fetched::Feed { body, .. } => body,
            Fetched::NotModified => panic!(),
        };
        let (_, chapters) = crate::podcast::feed::parse(&b.rss_url, &rss).unwrap();
        assert_eq!(chapters.len(), b.chapters as usize);
        assert!(chapters[0].audio_url.ends_with(".mp3"));
    }
}
