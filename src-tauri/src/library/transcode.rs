//! Converting songs for the iPhone with the bundled ffmpeg sidecar (F3.5, F6.1).
//! Converted copies live in a cache (`%LOCALAPPDATA%\MiuMusic\cache`) keyed by the audio and the
//! conversion, so a song is encoded once; a tag edit only rewrites the copy's tags. PC playback
//! always uses the original file.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::library::convert::Action;

/// Encodes audio and copies tags; the real one runs ffmpeg and lofty, tests use a fake.
pub trait Transcoder {
    fn encode(&self, src: &Path, dst: &Path, action: Action) -> Result<(), String>;
    /// Copy tags and artwork from `src` into the converted file `dst`; `cover` (an image file) is
    /// added when `src` has no picture of its own.
    fn retag(&self, src: &Path, dst: &Path, cover: Option<&Path>) -> Result<(), String>;
}

/// ffmpeg arguments for one conversion, or `None` when the file goes as it is.
pub fn ffmpeg_args(src: &Path, dst: &Path, action: Action) -> Option<Vec<OsString>> {
    let codec: Vec<&str> = match action {
        Action::Keep | Action::Skip => return None,
        Action::Alac { bits: 16, .. } => vec!["-c:a", "alac", "-sample_fmt", "s16p"],
        Action::Alac { .. } => vec!["-c:a", "alac", "-sample_fmt", "s32p", "-bits_per_raw_sample", "24"],
        Action::Aac { .. } => vec!["-c:a", "aac"],
    };
    let (rate, kbps) = match action {
        Action::Alac { rate, .. } => (rate, None),
        Action::Aac { kbps, rate } => (rate, Some(kbps)),
        _ => unreachable!(),
    };
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i"].iter().map(OsString::from).collect();
    args.push(src.as_os_str().to_owned());
    args.extend(["-map", "0:a:0", "-vn", "-map_metadata", "0"].iter().map(OsString::from));
    args.extend(codec.iter().map(OsString::from));
    if let Some(k) = kbps {
        args.extend([OsString::from("-b:a"), OsString::from(format!("{k}k"))]);
    }
    args.extend([OsString::from("-ar"), OsString::from(rate.to_string()), OsString::from("-f"), OsString::from("ipod")]);
    args.push(dst.as_os_str().to_owned());
    Some(args)
}

const SIDECAR: &str = "ffmpeg.exe";
const DEV_SIDECAR: &str = "ffmpeg-x86_64-pc-windows-msvc.exe";

/// Where the ffmpeg sidecar is: next to the app (installed, and `tauri dev` copies it there),
/// or in `src-tauri/binaries` when run from `target/<profile>`.
pub fn ffmpeg_path(exe_dir: &Path) -> Option<PathBuf> {
    let next_to_app = exe_dir.join(SIDECAR);
    let dev = exe_dir.parent().and_then(Path::parent).map(|src_tauri| src_tauri.join("binaries").join(DEV_SIDECAR));
    std::iter::once(next_to_app).chain(dev).find(|p| p.is_file())
}

pub struct Cache {
    pub dir: PathBuf,
}

impl Cache {
    /// Delete the least recently used copies until the cache holds at most `max_bytes`.
    /// Returns the bytes freed. A trimmed song is simply converted again when it is next needed.
    pub fn trim(&self, max_bytes: u64) -> Result<u64, String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Ok(0) };
        let mut copies: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "m4a") && !p.to_string_lossy().ends_with(".partial.m4a"))
            .filter_map(|p| {
                let m = std::fs::metadata(&p).ok()?;
                Some((m.modified().ok()?, m.len(), p))
            })
            .collect();
        copies.sort();
        let mut total: u64 = copies.iter().map(|c| c.1).sum();
        let mut freed = 0;
        for (_, len, path) in copies {
            if total <= max_bytes {
                break;
            }
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(path.with_extension("tags"));
            total -= len;
            freed += len;
        }
        Ok(freed)
    }

    /// The converted copy of `src`. `audio_key` identifies the audio (the song's audio hash),
    /// `tags_key` its current tags: a new `tags_key` re-tags the copy without re-encoding.
    pub fn converted(&self, t: &dyn Transcoder, src: &Path, audio_key: &str, tags_key: &str, action: Action, cover: Option<&Path>) -> Result<PathBuf, String> {
        let io = |e: std::io::Error| e.to_string();
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let name = crate::library::fingerprint::fnv64(format!("{audio_key}\u{1f}{action:?}").as_bytes());
        let file = self.dir.join(format!("{name}.m4a"));
        let tags_file = self.dir.join(format!("{name}.tags"));
        if !file.is_file() {
            let partial = self.dir.join(format!("{name}.partial.m4a"));
            let encoded = t.encode(src, &partial, action).and_then(|_| t.retag(src, &partial, cover));
            if let Err(e) = encoded {
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
            std::fs::rename(&partial, &file).map_err(io)?;
        } else if std::fs::read_to_string(&tags_file).ok().as_deref() == Some(tags_key) {
            // Recently used copies are the last to be trimmed.
            let _ = std::fs::File::options().write(true).open(&file).and_then(|f| f.set_modified(std::time::SystemTime::now()));
            return Ok(file);
        } else {
            t.retag(src, &file, cover)?;
        }
        std::fs::write(&tags_file, tags_key).map_err(io)?;
        Ok(file)
    }
}

/// The real converter: the ffmpeg sidecar encodes, lofty copies tags and artwork.
pub struct Ffmpeg {
    pub exe: PathBuf,
}

impl Transcoder for Ffmpeg {
    fn encode(&self, src: &Path, dst: &Path, action: Action) -> Result<(), String> {
        let args = ffmpeg_args(src, dst, action).ok_or("nothing to convert")?;
        let mut cmd = std::process::Command::new(&self.exe);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output().map_err(|e| format!("cannot run ffmpeg: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("ffmpeg: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed")))
    }

    fn retag(&self, src: &Path, dst: &Path, cover: Option<&Path>) -> Result<(), String> {
        use lofty::config::WriteOptions;
        use lofty::file::TaggedFileExt;
        use lofty::picture::{MimeType, Picture, PictureType};
        use lofty::tag::{Tag, TagExt, TagType};
        let file = lofty::read_from_path(src).map_err(|e| e.to_string())?;
        let mut tag = file.primary_tag().or_else(|| file.first_tag()).cloned().unwrap_or_else(|| Tag::new(TagType::Mp4Ilst));
        tag.re_map(TagType::Mp4Ilst);
        if tag.picture_count() == 0 {
            if let Some(jpg) = cover.and_then(|c| std::fs::read(c).ok()) {
                tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(MimeType::Jpeg), None, jpg));
            }
        }
        tag.save_to_path(dst, WriteOptions::default()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn strs(args: &[OsString]) -> Vec<String> {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn has(args: &[String], pair: [&str; 2]) -> bool {
        args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1])
    }

    #[test]
    fn alac_arguments_keep_bit_depth_and_set_the_rate() {
        let a = strs(&ffmpeg_args(Path::new(r"E:\Nhạc\Diễm xưa.flac"), Path::new(r"C:\c\x.m4a"), Action::Alac { bits: 16, rate: 44_100 }).unwrap());
        for pair in [["-i", r"E:\Nhạc\Diễm xưa.flac"], ["-c:a", "alac"], ["-sample_fmt", "s16p"], ["-ar", "44100"], ["-map_metadata", "0"], ["-f", "ipod"]] {
            assert!(has(&a, pair), "{pair:?} in {a:?}");
        }
        assert!(a.contains(&"-vn".to_string()), "artwork is copied by retag, not by ffmpeg");
        assert_eq!(a.last().unwrap(), r"C:\c\x.m4a");
        let hi = strs(&ffmpeg_args(Path::new("a.flac"), Path::new("x.m4a"), Action::Alac { bits: 24, rate: 48_000 }).unwrap());
        assert!(has(&hi, ["-sample_fmt", "s32p"]) && has(&hi, ["-bits_per_raw_sample", "24"]) && has(&hi, ["-ar", "48000"]), "{hi:?}");
    }

    #[test]
    fn aac_arguments_set_bitrate_and_rate() {
        let a = strs(&ffmpeg_args(Path::new("a.ogg"), Path::new("x.m4a"), Action::Aac { kbps: 256, rate: 48_000 }).unwrap());
        assert!(has(&a, ["-c:a", "aac"]) && has(&a, ["-b:a", "256k"]) && has(&a, ["-ar", "48000"]) && has(&a, ["-f", "ipod"]), "{a:?}");
    }

    #[test]
    fn files_that_go_as_they_are_need_no_ffmpeg() {
        assert!(ffmpeg_args(Path::new("a.mp3"), Path::new("x"), Action::Keep).is_none());
        assert!(ffmpeg_args(Path::new("a.xyz"), Path::new("x"), Action::Skip).is_none());
    }

    #[test]
    fn the_sidecar_is_found_next_to_the_app_or_in_binaries_during_development() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("Miu Music");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("ffmpeg.exe"), b"x").unwrap();
        assert_eq!(ffmpeg_path(&installed), Some(installed.join("ffmpeg.exe")));

        let src_tauri = root.path().join("src-tauri");
        let exe_dir = src_tauri.join("target").join("debug");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(src_tauri.join("binaries")).unwrap();
        let dev = src_tauri.join("binaries").join("ffmpeg-x86_64-pc-windows-msvc.exe");
        std::fs::write(&dev, b"x").unwrap();
        assert_eq!(ffmpeg_path(&exe_dir), Some(dev));

        assert_eq!(ffmpeg_path(&root.path().join("nowhere")), None);
    }

    #[derive(Default)]
    struct Fake {
        encodes: RefCell<usize>,
        retags: RefCell<usize>,
        fail: bool,
    }

    impl Transcoder for Fake {
        fn encode(&self, src: &Path, dst: &Path, action: Action) -> Result<(), String> {
            *self.encodes.borrow_mut() += 1;
            std::fs::write(dst, format!("{} as {action:?}", src.display())).unwrap();
            if self.fail { Err("ffmpeg: Invalid data found when processing input".into()) } else { Ok(()) }
        }
        fn retag(&self, _src: &Path, _dst: &Path, _cover: Option<&Path>) -> Result<(), String> {
            *self.retags.borrow_mut() += 1;
            Ok(())
        }
    }

    const ALAC: Action = Action::Alac { bits: 16, rate: 44_100 };

    #[test]
    fn the_same_audio_is_encoded_once() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache { dir: dir.path().join("cache") };
        let t = Fake::default();
        let a = cache.converted(&t, Path::new("a.flac"), "audio1", "tags1", ALAC, None).unwrap();
        let b = cache.converted(&t, Path::new("a.flac"), "audio1", "tags1", ALAC, None).unwrap();
        assert_eq!(a, b);
        assert!(a.is_file() && a.extension().unwrap() == "m4a");
        assert_eq!((*t.encodes.borrow(), *t.retags.borrow()), (1, 1));
    }

    #[test]
    fn a_tag_change_retags_without_encoding_again() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache { dir: dir.path().to_path_buf() };
        let t = Fake::default();
        cache.converted(&t, Path::new("a.flac"), "audio1", "tags1", ALAC, None).unwrap();
        cache.converted(&t, Path::new("a.flac"), "audio1", "tags2", ALAC, None).unwrap();
        assert_eq!((*t.encodes.borrow(), *t.retags.borrow()), (1, 2));
    }

    #[test]
    fn new_audio_or_a_new_conversion_encodes_again() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache { dir: dir.path().to_path_buf() };
        let t = Fake::default();
        let a = cache.converted(&t, Path::new("a.flac"), "audio1", "tags1", ALAC, None).unwrap();
        let b = cache.converted(&t, Path::new("a.flac"), "audio2", "tags1", ALAC, None).unwrap();
        let c = cache.converted(&t, Path::new("a.flac"), "audio1", "tags1", Action::Alac { bits: 24, rate: 48_000 }, None).unwrap();
        assert_eq!(*t.encodes.borrow(), 3);
        assert!(a != b && a != c && b != c);
    }

    fn age(path: &Path, secs_ago: u64) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn trimming_removes_the_least_recently_used_copies_down_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache { dir: dir.path().to_path_buf() };
        let t = Fake::default();
        let old = cache.converted(&t, Path::new("a.flac"), "old", "t", ALAC, None).unwrap();
        let mid = cache.converted(&t, Path::new("b.flac"), "mid", "t", ALAC, None).unwrap();
        let new = cache.converted(&t, Path::new("c.flac"), "new", "t", ALAC, None).unwrap();
        age(&old, 300);
        age(&mid, 200);
        age(&new, 100);
        let each = std::fs::metadata(&new).unwrap().len();
        let freed = cache.trim(each * 2).unwrap();
        assert_eq!(freed, std::fs::metadata(&mid).unwrap().len());
        assert!(!old.exists() && !dir.path().join(format!("{}.tags", old.file_stem().unwrap().to_string_lossy())).exists());
        assert!(mid.exists() && new.exists(), "oldest goes first");
    }

    #[test]
    fn a_cache_hit_counts_as_recent_use() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache { dir: dir.path().to_path_buf() };
        let t = Fake::default();
        let a = cache.converted(&t, Path::new("a.flac"), "a", "t", ALAC, None).unwrap();
        let b = cache.converted(&t, Path::new("b.flac"), "b", "t", ALAC, None).unwrap();
        age(&a, 300);
        age(&b, 100);
        cache.converted(&t, Path::new("a.flac"), "a", "t", ALAC, None).unwrap();
        cache.trim(std::fs::metadata(&a).unwrap().len()).unwrap();
        assert!(a.exists() && !b.exists(), "a was just used");
    }

    #[test]
    fn a_failed_encode_leaves_nothing_in_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache { dir: dir.path().to_path_buf() };
        let t = Fake { fail: true, ..Default::default() };
        let err = cache.converted(&t, Path::new("a.flac"), "audio1", "tags1", ALAC, None).unwrap_err();
        assert!(err.contains("Invalid data"), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "no partial or finished file left");
    }

    /// Needs the ffmpeg sidecar in src-tauri/binaries (not in git; see the F6.1 checklist entry).
    /// `cargo test --lib transcode -- --ignored`
    #[test]
    #[ignore]
    fn flac_becomes_alac_with_its_tags_and_artwork() {
        use lofty::file::{AudioFile, TaggedFileExt};
        use lofty::picture::{MimeType, Picture, PictureType};
        use lofty::tag::{Accessor, TagExt};
        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries").join(DEV_SIDECAR);
        assert!(exe.is_file(), "ffmpeg sidecar in src-tauri/binaries");
        let ff = Ffmpeg { exe };
        let dir = tempfile::tempdir().unwrap();
        // A 24-bit / 96 kHz FLAC with Vietnamese tags and a cover, made from a generated WAV.
        let wav = crate::audio::fixtures::wav(dir.path(), "src.wav", 96_000, 2, 24, 96_000);
        let flac = dir.path().join("Diễm xưa.flac");
        let ok = std::process::Command::new(&ff.exe).args(["-hide_banner", "-loglevel", "error", "-i"]).arg(&wav).arg(&flac).status().unwrap();
        assert!(ok.success());
        let mut f = lofty::read_from_path(&flac).unwrap();
        let tag = f.primary_tag_mut().unwrap();
        tag.set_title("Diễm xưa".into());
        tag.set_artist("Khánh Ly".into());
        tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(MimeType::Png), None, vec![0x89, b'P', b'N', b'G', 1, 2, 3]));
        tag.save_to_path(&flac, lofty::config::WriteOptions::default()).unwrap();

        let cache = Cache { dir: dir.path().join("cache") };
        let out = cache.converted(&ff, &flac, "audio", "tags", Action::Alac { bits: 24, rate: 48_000 }, None).unwrap();
        let m4a = lofty::read_from_path(&out).unwrap();
        assert_eq!(m4a.file_type(), lofty::file::FileType::Mp4);
        let props = m4a.properties();
        assert_eq!((props.sample_rate(), props.bit_depth(), props.channels()), (Some(48_000), Some(24), Some(2)));
        let tag = m4a.primary_tag().unwrap();
        assert_eq!((tag.title().as_deref(), tag.artist().as_deref()), (Some("Diễm xưa"), Some("Khánh Ly")));
        assert_eq!(tag.picture_count(), 1);
        let mp4 = lofty::mp4::Mp4File::read_from(&mut std::fs::File::open(&out).unwrap(), Default::default()).unwrap();
        assert_eq!(mp4.properties().codec(), &lofty::mp4::Mp4Codec::ALAC);
    }
}
