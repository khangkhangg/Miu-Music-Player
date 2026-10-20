//! OPML (P.2): the subscription list every podcast app can export and import.

use quick_xml::events::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpmlFeed {
    pub url: String,
    pub title: Option<String>,
}

/// Every feed in an OPML document (outlines with a feed address, at any depth), first one wins.
pub fn parse(xml: &[u8]) -> Result<Vec<OpmlFeed>, String> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut is_opml = false;
    let mut feeds: Vec<OpmlFeed> = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).map_err(|e| format!("not an OPML file: {e}"))? {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name();
                let local = name.as_ref().to_ascii_lowercase();
                if local == b"opml" {
                    is_opml = true;
                } else if local == b"outline" {
                    let (mut url, mut text, mut title) = (None, None, None);
                    for a in e.attributes().flatten() {
                        let key = a.key.as_ref().to_ascii_lowercase();
                        let value = a
                            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
                            .map_err(|e| e.to_string())?
                            .trim()
                            .to_string();
                        match key.as_slice() {
                            b"xmlurl" => url = Some(value),
                            b"title" => title = Some(value),
                            b"text" => text = Some(value),
                            _ => {}
                        }
                    }
                    if let Some(url) = url.filter(|u| !u.is_empty()) {
                        if !feeds.iter().any(|f| f.url == url) {
                            feeds.push(OpmlFeed { url, title: title.or(text).filter(|t| !t.is_empty()) });
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !is_opml {
        return Err("not an OPML file".into());
    }
    Ok(feeds)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// An OPML 2.0 document listing `feeds`.
pub fn export(feeds: &[OpmlFeed]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head><title>Miu Music podcasts</title></head>\n  <body>\n");
    for f in feeds {
        match &f.title {
            Some(t) => out.push_str(&format!("    <outline type=\"rss\" text=\"{0}\" title=\"{0}\" xmlUrl=\"{1}\"/>\n", escape(t), escape(&f.url))),
            None => out.push_str(&format!("    <outline type=\"rss\" xmlUrl=\"{}\"/>\n", escape(&f.url))),
        }
    }
    out.push_str("  </body>\n</opml>\n");
    out
}

/// A feed fetched and parsed, ready to store.
pub struct Downloaded {
    pub show: crate::podcast::feed::Show,
    pub episodes: Vec<crate::podcast::feed::Episode>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub added: usize,
    pub already: usize,
    /// Feed address and why it could not be added.
    pub failed: Vec<(String, String)>,
}

/// Subscribe to every feed in an OPML list: `known` says which are subscribed already, `download`
/// fetches one (no database held), `save` stores it, `progress` hears (done, total) before each feed
/// and once at the end. One broken feed never stops the rest.
pub fn import_feeds(
    feeds: &[OpmlFeed],
    known: impl Fn(&str) -> bool,
    download: impl Fn(&str) -> Result<Downloaded, String>,
    mut save: impl FnMut(Downloaded) -> Result<(), String>,
    progress: impl Fn(usize, usize),
) -> ImportReport {
    let mut report = ImportReport::default();
    for (i, f) in feeds.iter().enumerate() {
        progress(i, feeds.len());
        if known(&f.url) {
            report.already += 1;
            continue;
        }
        match download(&f.url).and_then(&mut save) {
            Ok(()) => report.added += 1,
            Err(e) => report.failed.push((f.url.clone(), e)),
        }
    }
    progress(feeds.len(), feeds.len());
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM_ANOTHER_APP: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<opml version="2.0">
  <head><title>Podcasts</title></head>
  <body>
    <outline text="feeds">
      <outline type="rss" text="Hôm Nay Nghe Gì" title="Hôm Nay Nghe Gì" xmlUrl="https://example.vn/feed.xml" htmlUrl="https://example.vn"/>
      <outline text="Tin tức &amp; Thời sự" xmlUrl=" https://example.vn/news?x=1&amp;y=2 "/>
    </outline>
    <outline type="rss" text="Old Style" xmlurl="https://example.com/old.rss"/>
    <outline text="Just a folder"/>
    <outline type="rss" text="Duplicate" xmlUrl="https://example.vn/feed.xml"/>
  </body>
</opml>"#;

    #[test]
    fn every_feed_is_found_including_nested_ones_once_each() {
        let feeds = parse(FROM_ANOTHER_APP.as_bytes()).unwrap();
        assert_eq!(feeds, vec![
            OpmlFeed { url: "https://example.vn/feed.xml".into(), title: Some("Hôm Nay Nghe Gì".into()) },
            OpmlFeed { url: "https://example.vn/news?x=1&y=2".into(), title: Some("Tin tức & Thời sự".into()) },
            OpmlFeed { url: "https://example.com/old.rss".into(), title: Some("Old Style".into()) },
        ]);
    }

    #[test]
    fn export_reads_back_the_same_and_escapes_what_it_must() {
        let feeds = vec![
            OpmlFeed { url: "https://example.vn/news?x=1&y=2".into(), title: Some("Tin tức & <Thời sự> \"hay\"".into()) },
            OpmlFeed { url: "https://example.com/plain.rss".into(), title: None },
        ];
        let xml = export(&feeds);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<opml version=\"2.0\">"));
        assert_eq!(parse(xml.as_bytes()).unwrap(), feeds);
    }

    #[test]
    fn something_that_is_not_opml_is_refused() {
        assert!(parse(b"<rss><channel/></rss>").is_err());
        assert!(parse(b"not xml at all <<<").is_err());
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;
    use crate::podcast::feed::{Episode, Show};
    use std::cell::RefCell;

    fn feed(url: &str) -> OpmlFeed {
        OpmlFeed { url: url.into(), title: None }
    }

    fn show(url: &str) -> Show {
        Show { feed_url: url.into(), title: url.into(), author: None, description: None, artwork_url: None, link: None, language: None }
    }

    #[test]
    fn new_feeds_are_added_known_ones_skipped_and_failures_reported() {
        let feeds = vec![feed("https://a/1"), feed("https://a/known"), feed("https://a/broken"), feed("https://a/2")];
        let saved = RefCell::new(Vec::new());
        let progress = RefCell::new(Vec::new());
        let report = import_feeds(
            &feeds,
            |url| url == "https://a/known",
            |url| if url.ends_with("broken") { Err("the server answered 404".into()) } else { Ok(Downloaded { show: show(url), episodes: Vec::<Episode>::new(), etag: None, last_modified: None }) },
            |d| { saved.borrow_mut().push(d.show.feed_url); Ok(()) },
            |done, total| progress.borrow_mut().push((done, total)),
        );
        assert_eq!((report.added, report.already), (2, 1));
        assert_eq!(report.failed, vec![("https://a/broken".to_string(), "the server answered 404".to_string())]);
        assert_eq!(*saved.borrow(), vec!["https://a/1".to_string(), "https://a/2".to_string()]);
        assert_eq!(*progress.borrow(), vec![(0, 4), (1, 4), (2, 4), (3, 4), (4, 4)]);
    }
}
