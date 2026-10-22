//! App glue for lyrics (L): the `song_lyrics` command and the HTTP client for LRCLIB.

use std::path::Path;

use tauri::State;

use crate::library::service::SharedDb;
use crate::lyrics::{find, Lyrics, Song};

/// GET `url`: the body, `None` for 404 (LRCLIB's "not found"), an error otherwise.
pub fn http_get(url: &str) -> Result<Option<String>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(10)))
        .http_status_as_error(false)
        // LRCLIB asks apps to name themselves.
        .user_agent(concat!("Miu Music ", env!("CARGO_PKG_VERSION"), " (https://github.com/khangkhangg/Miu-Music-Player)"))
        .build()
        .into();
    let mut res = agent.get(url).call().map_err(|e| e.to_string())?;
    // Busy (rate limit, overload): wait a moment and ask once more.
    if matches!(res.status().as_u16(), 429 | 503) {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        res = agent.get(url).call().map_err(|e| e.to_string())?;
    }
    match res.status().as_u16() {
        404 => Ok(None),
        200..=299 => res.body_mut().with_config().limit(2 * 1024 * 1024).read_to_string().map(Some).map_err(|e| e.to_string()),
        s => Err(format!("lyrics service answered {s}")),
    }
}

/// Lyrics for a song in the library (see `lyrics::find`), online only if the setting allows.
#[tauri::command]
pub async fn song_lyrics(id: i64, db: State<'_, SharedDb>) -> Result<Lyrics, String> {
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (row, online) = {
            let d = db.lock().map_err(|e| e.to_string())?;
            let row: (String, Option<String>, Option<String>, Option<String>, Option<i64>) = d
                .conn()
                .query_row("SELECT path, artist, title, album, duration_ms FROM songs WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .map_err(|e| e.to_string())?;
            (row, crate::settings::lyrics_online(&d))
        };
        let (path, artist, title, album, duration_ms) = row;
        let path = Path::new(&path);
        let embedded = crate::library::tags::read_lyrics(path);
        let song = Song { path, artist: artist.as_deref(), title: title.as_deref(), album: album.as_deref(), duration_ms: duration_ms.map(|d| d as u64) };
        let cache = crate::util::paths::app_data_dir_in(&crate::util::paths::appdata_root()).join("lyrics");
        Ok(find(&song, embedded.as_deref(), &cache, online, crate::util::time::now_unix(), &http_get))
    })
    .await
    .map_err(|e| e.to_string())?
}

// Machine test: asks the real LRCLIB for a song from its own documentation.
#[cfg(test)]
mod machine_tests {
    #[test]
    #[ignore]
    fn lrclib_answers_with_synced_lyrics() {
        let r = crate::lyrics::lrclib::lookup("Borislav Slavov", "I Want to Live", Some("Baldur's Gate 3 (Original Game Soundtrack)"), Some(233), &super::http_get)
            .expect("LRCLIB reachable")
            .expect("found");
        println!("{} - {}: synced {}", r.artist_name, r.track_name, r.synced_lyrics.is_some());
        assert!(r.synced_lyrics.is_some_and(|s| crate::lyrics::lrc::is_synced(&s)));
    }
}
