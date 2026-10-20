//! Podcast feeds (P.1): an RSS document becomes a show and its episodes. Apple's `itunes:` tags
//! are preferred where present (author, artwork, duration), with plain RSS as the fallback.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Show {
    pub feed_url: String,
    pub title: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub artwork_url: Option<String>,
    pub link: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    /// Stable id within the show: the feed's guid, else the audio address.
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    /// Seconds since 1970.
    pub published: Option<i64>,
    pub duration_ms: Option<u64>,
    pub audio_url: String,
    pub audio_type: Option<String>,
    /// From the enclosure; `None` when the feed says 0 or nothing.
    pub audio_bytes: Option<u64>,
    pub season: Option<u32>,
    pub number: Option<u32>,
}

fn text(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// `itunes:duration` as milliseconds: "H:MM:SS", "MM:SS" or plain (possibly fractional) seconds.
pub fn duration_ms(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.iter().any(|p| p.is_empty()) || parts.len() > 3 {
        return None;
    }
    let mut seconds = 0f64;
    for p in &parts {
        seconds = seconds * 60.0 + p.parse::<f64>().ok().filter(|v| *v >= 0.0)?;
    }
    Some((seconds * 1000.0).round() as u64)
}

fn published(s: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(s?.trim()).ok().map(|d| d.timestamp())
}

/// Parse a podcast feed fetched from `feed_url`. Items without audio are left out.
pub fn parse(feed_url: &str, xml: &[u8]) -> Result<(Show, Vec<Episode>), String> {
    let ch = rss::Channel::read_from(xml).map_err(|e| format!("not a podcast feed: {e}"))?;
    let it = ch.itunes_ext();
    let show = Show {
        feed_url: feed_url.to_string(),
        title: text(Some(ch.title())).unwrap_or_else(|| feed_url.to_string()),
        author: text(it.and_then(|i| i.author())),
        description: text(Some(ch.description())).or_else(|| text(it.and_then(|i| i.summary()))),
        artwork_url: text(it.and_then(|i| i.image())).or_else(|| text(ch.image().map(|i| i.url()))),
        link: text(Some(ch.link())),
        language: text(ch.language()),
    };
    let episodes = ch
        .items()
        .iter()
        .filter_map(|item| {
            let enc = item.enclosure()?;
            let audio_url = text(Some(enc.url()))?;
            let ix = item.itunes_ext();
            Some(Episode {
                guid: text(item.guid().map(|g| g.value())).unwrap_or_else(|| audio_url.clone()),
                title: text(item.title()).unwrap_or_else(|| audio_url.clone()),
                description: text(item.description()).or_else(|| text(ix.and_then(|i| i.summary()))),
                published: published(item.pub_date()),
                duration_ms: ix.and_then(|i| i.duration()).and_then(duration_ms),
                audio_type: text(Some(enc.mime_type())),
                audio_bytes: enc.length().trim().parse::<u64>().ok().filter(|n| *n > 0),
                season: ix.and_then(|i| i.season()).and_then(|s| s.trim().parse().ok()),
                number: ix.and_then(|i| i.episode()).and_then(|s| s.trim().parse().ok()),
                audio_url,
            })
        })
        .collect();
    Ok((show, episodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
  <channel>
    <title>Hôm Nay Nghe Gì</title>
    <link>https://example.vn/podcast</link>
    <description>Câu chuyện âm nhạc Việt mỗi tuần.</description>
    <language>vi</language>
    <itunes:author>Đài Phát Thanh Mẫu</itunes:author>
    <itunes:image href="https://example.vn/art.jpg"/>
    <item>
      <title>Tập 12: Trịnh Công Sơn và những bài ca không năm tháng</title>
      <guid isPermaLink="false">ep-12</guid>
      <pubDate>Mon, 19 Oct 2026 08:00:00 +0700</pubDate>
      <enclosure url="https://cdn.example.vn/ep12.mp3" length="48211245" type="audio/mpeg"/>
      <itunes:duration>01:02:03</itunes:duration>
      <itunes:episode>12</itunes:episode>
      <itunes:season>2</itunes:season>
      <description>Nghe lại Diễm xưa, Hạ trắng.</description>
    </item>
    <item>
      <title>Tập 11: Mùa thu cho em</title>
      <guid>ep-11</guid>
      <pubDate>Mon, 12 Oct 2026 08:00:00 +0700</pubDate>
      <enclosure url="https://cdn.example.vn/ep11.m4a" length="0" type="audio/x-m4a"/>
      <itunes:duration>2730</itunes:duration>
    </item>
    <item>
      <title>Thông báo (không có âm thanh)</title>
      <guid>notice</guid>
    </item>
  </channel>
</rss>"#;

    const SPARSE: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <title>Bare Show</title>
  <image><url>https://example.com/rss-image.png</url><title>Bare</title><link>https://example.com</link></image>
  <item>
    <title>Only an enclosure</title>
    <enclosure url="https://example.com/a.mp3" type="audio/mpeg" length="12"/>
    <itunes:duration xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">45:30</itunes:duration>
  </item>
</channel></rss>"#;

    #[test]
    fn a_show_comes_with_its_details() {
        let (show, _) = parse("https://example.vn/feed.xml", VN.as_bytes()).unwrap();
        assert_eq!(show.feed_url, "https://example.vn/feed.xml");
        assert_eq!(show.title, "Hôm Nay Nghe Gì");
        assert_eq!(show.author.as_deref(), Some("Đài Phát Thanh Mẫu"));
        assert_eq!(show.artwork_url.as_deref(), Some("https://example.vn/art.jpg"));
        assert_eq!(show.link.as_deref(), Some("https://example.vn/podcast"));
        assert_eq!(show.language.as_deref(), Some("vi"));
        assert_eq!(show.description.as_deref(), Some("Câu chuyện âm nhạc Việt mỗi tuần."));
    }

    #[test]
    fn episodes_carry_audio_dates_and_durations_and_items_without_audio_are_skipped() {
        let (_, eps) = parse("u", VN.as_bytes()).unwrap();
        assert_eq!(eps.len(), 2, "the notice without audio is not an episode");
        let e = &eps[0];
        assert_eq!(e.guid, "ep-12");
        assert_eq!(e.title, "Tập 12: Trịnh Công Sơn và những bài ca không năm tháng");
        assert_eq!(e.audio_url, "https://cdn.example.vn/ep12.mp3");
        assert_eq!(e.audio_type.as_deref(), Some("audio/mpeg"));
        assert_eq!(e.audio_bytes, Some(48_211_245));
        assert_eq!(e.duration_ms, Some(3_723_000));
        assert_eq!(e.published, Some(1_792_371_600), "2026-10-19 01:00 UTC");
        assert_eq!((e.season, e.number), (Some(2), Some(12)));
        assert_eq!(eps[1].duration_ms, Some(2_730_000), "plain seconds");
        assert_eq!(eps[1].audio_bytes, None, "length 0 means unknown");
    }

    #[test]
    fn sparse_feeds_fall_back_sensibly() {
        let (show, eps) = parse("https://example.com/f", SPARSE.as_bytes()).unwrap();
        assert_eq!(show.artwork_url.as_deref(), Some("https://example.com/rss-image.png"), "plain RSS image");
        assert_eq!(show.author, None);
        assert_eq!(eps[0].guid, "https://example.com/a.mp3", "no guid: the audio address identifies it");
        assert_eq!(eps[0].duration_ms, Some(2_730_000), "MM:SS");
        assert_eq!(eps[0].published, None);
    }

    #[test]
    fn durations_in_every_common_shape() {
        assert_eq!(duration_ms("1:02:03"), Some(3_723_000));
        assert_eq!(duration_ms("62:03"), Some(3_723_000));
        assert_eq!(duration_ms("3723"), Some(3_723_000));
        assert_eq!(duration_ms(" 90.5 "), Some(90_500));
        assert_eq!(duration_ms("soon"), None);
        assert_eq!(duration_ms(""), None);
    }

    #[test]
    fn not_a_feed_is_an_error() {
        assert!(parse("u", b"<html><body>Not found</body></html>").is_err());
    }
}
