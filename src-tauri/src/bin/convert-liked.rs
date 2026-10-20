//! Convert every liked song the iPhone cannot play as it is into the conversion cache
//! (`%LOCALAPPDATA%\MiuMusic\cache`), so the next sync finds them ready. Originals are only read.
//! Usage: `cargo run --bin convert-liked`

use std::time::Instant;

use miu_music_lib::library::convert::{action, Action};
use miu_music_lib::library::db::Db;
use miu_music_lib::library::format::Format;
use miu_music_lib::library::transcode::{ffmpeg_path, Cache, Ffmpeg};
use miu_music_lib::sync::itunes::{sendable_path, SongSource};
use miu_music_lib::util::paths;

fn main() {
    let db = Db::open(&paths::db_path_in(&paths::appdata_root())).unwrap_or_else(|e| fail(&e.to_string()));
    let policy = miu_music_lib::settings::conversion_policy(&db);
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    let ff = Ffmpeg { exe: ffmpeg_path(&exe_dir).unwrap_or_else(|| fail("ffmpeg sidecar not found next to this tool")) };
    let cache = Cache { dir: paths::local_appdata_root().join("MiuMusic").join("cache") };
    let convert = |s: &SongSource| cache.converted(&ff, &s.path, &s.audio_key, &s.tags_key, s.action);

    let songs = miu_music_lib::library::songs::list_songs(&db).unwrap_or_else(|e| fail(&e.to_string()));
    let (mut ok, mut failed) = (0, 0);
    for song in songs.into_iter().filter(|s| s.rating == 1) {
        let (id, title) = (song.id, song.title);
        let act = action(Format::from_name(&song.format), song.sample_rate, song.bit_depth, &policy);
        if matches!(act, Action::Keep | Action::Skip) {
            continue;
        }
        let started = Instant::now();
        match sendable_path(&db, id, &policy, Some(&convert)) {
            Ok(p) => {
                ok += 1;
                let size = std::fs::metadata(&p).map(|m| m.len() / 1024).unwrap_or(0);
                println!("  {:?} -> {act:?}  {size} KB  {:.1}s  {}", title, started.elapsed().as_secs_f32(), p.display());
            }
            Err(e) => {
                failed += 1;
                println!("  FAILED {:?}: {e}", title);
            }
        }
    }
    println!("{ok} converted (or already cached), {failed} failed -> {}", cache.dir.display());
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}
