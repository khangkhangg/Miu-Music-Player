//! Episode downloads (P.3): to `<file>.part` first, resumed with an HTTP Range request after an
//! interruption, and renamed into place only when complete.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where an unfinished download is kept.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

/// File extension for an episode: from the address if it has an audio one, else from the type.
pub fn extension_for(url: &str, mime: Option<&str>) -> &'static str {
    const KNOWN: [&str; 7] = ["mp3", "m4a", "aac", "mp4", "ogg", "opus", "wav"];
    let path = url.split(['?', '#']).next().unwrap_or("");
    if let Some(ext) = path.rsplit('/').next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()) {
        if let Some(k) = KNOWN.iter().find(|k| **k == ext) {
            return k;
        }
    }
    match mime.unwrap_or("").to_ascii_lowercase().as_str() {
        "audio/x-m4a" | "audio/m4a" | "audio/mp4" | "video/mp4" => "m4a",
        "audio/aac" | "audio/aacp" => "aac",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => "mp3",
    }
}

/// Download `url` to `dest`, resuming a `.part` file when the server allows it. `progress` hears
/// (bytes so far, total if known). Returns the file's size.
pub fn download(url: &str, dest: &Path, progress: impl Fn(u64, Option<u64>)) -> Result<u64, String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = part_path(dest);
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .user_agent(concat!("MiuMusic/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut req = agent.get(url);
    if have > 0 {
        req = req.header("Range", format!("bytes={have}-"));
    }
    let res = req.call().map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let resumed = status == 206 && have > 0;
    if !(200..300).contains(&status) {
        return Err(format!("the server answered {status}"));
    }
    let start = if resumed { have } else { 0 };
    let total = res.body().content_length().map(|n| n + start);
    let mut file = if resumed {
        std::fs::OpenOptions::new().append(true).open(&part)
    } else {
        std::fs::File::create(&part)
    }
    .map_err(|e| e.to_string())?;
    let mut reader = res.into_body().into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = start;
    let mut last_report = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if done - last_report >= 256 * 1024 {
            progress(done, total);
            last_report = done;
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    if total.is_some_and(|t| done < t) {
        return Err(format!("the download stopped at {done} bytes"));
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())?;
    progress(done, total.or(Some(done)));
    Ok(done)
}

/// `download`, tried up to `tries` times when the server fails or the connection drops (a book's
/// many chapters often meet a passing error); "not found" and other 4xx answers are final.
pub fn download_retrying(url: &str, dest: &Path, tries: u32, pause: Duration, progress: impl Fn(u64, Option<u64>)) -> Result<u64, String> {
    let mut attempt = 1;
    loop {
        match download(url, dest, &progress) {
            Err(e) if attempt < tries && !e.starts_with("the server answered 4") => {
                log::info!("download of {url} failed ({e}), trying again");
                std::thread::sleep(pause * attempt);
                attempt += 1;
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn audio() -> Vec<u8> {
        (0..100_000u32).map(|i| (i % 251) as u8).collect()
    }

    /// Serves `audio()`; honours `Range: bytes=N-` when `ranges` is on; cuts the first response
    /// after `cut_after` bytes. Records each request's Range header.
    fn server(requests: usize, ranges: bool, cut_after: Option<usize>) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/ep.mp3", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().take(requests).enumerate() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                        range = Some(v.trim().trim_end_matches('-').to_string());
                    }
                }
                seen2.lock().unwrap().push(range.clone());
                let body = audio();
                let (status, from) = match (ranges, range.and_then(|r| r.parse::<usize>().ok())) {
                    (true, Some(from)) => ("206 Partial Content", from),
                    _ => ("200 OK", 0),
                };
                let part = &body[from..];
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\n\r\n", part.len());
                stream.write_all(head.as_bytes()).unwrap();
                let send = match (n, cut_after) {
                    (0, Some(c)) => &part[..c],
                    _ => part,
                };
                let _ = stream.write_all(send);
            }
        });
        (url, seen)
    }

    #[test]
    fn an_episode_downloads_and_only_appears_when_complete() {
        let d = tempfile::tempdir().unwrap();
        let (url, _) = server(1, true, None);
        let dest = d.path().join("shows").join("1").join("7.mp3");
        let progress = Mutex::new(Vec::new());
        let n = download(&url, &dest, |done, total| progress.lock().unwrap().push((done, total))).unwrap();
        assert_eq!(n, 100_000);
        assert_eq!(std::fs::read(&dest).unwrap(), audio());
        assert!(!part_path(&dest).exists(), "no leftover .part");
        let p = progress.lock().unwrap();
        assert_eq!(p.last(), Some(&(100_000, Some(100_000))));
    }

    #[test]
    fn an_interrupted_download_resumes_where_it_stopped() {
        let d = tempfile::tempdir().unwrap();
        let (url, seen) = server(2, true, Some(40_000));
        let dest = d.path().join("7.mp3");
        assert!(download(&url, &dest, |_, _| {}).is_err(), "connection cut");
        assert!(!dest.exists());
        assert_eq!(std::fs::metadata(part_path(&dest)).unwrap().len(), 40_000, "what arrived is kept");
        download(&url, &dest, |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), audio());
        assert_eq!(*seen.lock().unwrap(), vec![None, Some("40000".to_string())], "second request asked for the rest");
    }

    #[test]
    fn a_server_without_ranges_starts_over_cleanly() {
        let d = tempfile::tempdir().unwrap();
        let (url, _) = server(1, false, None);
        let dest = d.path().join("7.mp3");
        std::fs::write(part_path(&dest), vec![9u8; 500]).unwrap(); // stale partial file
        download(&url, &dest, |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), audio());
    }

    /// Answers `first` to the first request, then serves `audio()`. Counts requests.
    fn flaky(first: u16, requests: usize) -> (String, Arc<Mutex<usize>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/ch.mp3", listener.local_addr().unwrap());
        let count = Arc::new(Mutex::new(0));
        let c = count.clone();
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().take(requests).enumerate() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                *c.lock().unwrap() += 1;
                if n == 0 {
                    let _ = stream.write_all(format!("HTTP/1.1 {first} Oops\r\nContent-Length: 0\r\n\r\n").as_bytes());
                } else {
                    let body = audio();
                    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes());
                    let _ = stream.write_all(&body);
                }
            }
        });
        (url, count)
    }

    #[test]
    fn a_passing_server_error_is_tried_again() {
        let d = tempfile::tempdir().unwrap();
        let (url, count) = flaky(500, 2);
        let dest = d.path().join("2.mp3");
        download_retrying(&url, &dest, 3, Duration::ZERO, |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), audio());
        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[test]
    fn a_missing_file_is_not_tried_again() {
        let d = tempfile::tempdir().unwrap();
        let (url, count) = flaky(404, 2);
        assert!(download_retrying(&url, &d.path().join("2.mp3"), 3, Duration::ZERO, |_, _| {}).is_err());
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[test]
    fn file_names_keep_the_audio_extension() {
        assert_eq!(extension_for("https://cdn.example.vn/ep12.mp3?token=1", Some("audio/mpeg")), "mp3");
        assert_eq!(extension_for("https://cdn.example.vn/stream", Some("audio/x-m4a")), "m4a");
        assert_eq!(extension_for("https://cdn.example.vn/stream", Some("audio/aac")), "aac");
        assert_eq!(extension_for("https://cdn.example.vn/x.php", None), "mp3", "unknown: mp3, the common case");
    }
}
