//! Watching the watched folders (F3.1). File events are debounced and turned into "these
//! folders changed"; the caller then rescans them (`scan::scan_folder` + `scan::reconcile`).

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::library::folders::is_inside;

/// Id of the watched folder containing `path`, if any. Folders never nest (see `folders`).
pub fn folder_for(path: &Path, folders: &[(i64, String)]) -> Option<i64> {
    let p = path.to_string_lossy();
    folders.iter().find(|(_, root)| is_inside(&p, root) || p.eq_ignore_ascii_case(root)).map(|(id, _)| *id)
}

/// Folder ids touched by `paths`, each once, in folder order.
pub fn affected_folders(paths: &[PathBuf], folders: &[(i64, String)]) -> Vec<i64> {
    let mut ids: Vec<i64> = paths.iter().filter_map(|p| folder_for(p, folders)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Recursive watch on every folder; calls `on_change(folder_ids)` once per burst of changes.
/// Dropping it stops watching.
pub struct FolderWatcher {
    _watcher: RecommendedWatcher,
}

impl FolderWatcher {
    pub fn start(
        folders: Vec<(i64, String)>,
        debounce: Duration,
        on_change: impl Fn(Vec<i64>) + Send + 'static,
    ) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if !matches!(event.kind, EventKind::Access(_)) {
                    event.paths.into_iter().for_each(|p| {
                        let _ = tx.send(p);
                    });
                }
            }
        })?;
        for (_, root) in &folders {
            watcher.watch(Path::new(root), RecursiveMode::Recursive)?;
        }
        std::thread::Builder::new()
            .name("folder-watch".into())
            .spawn(move || {
                // Ends when the watcher (and with it the sender) is dropped.
                while let Ok(first) = rx.recv() {
                    let mut burst = vec![first];
                    while let Ok(more) = rx.recv_timeout(debounce) {
                        burst.push(more);
                    }
                    let ids = affected_folders(&burst, &folders);
                    if !ids.is_empty() {
                        on_change(ids);
                    }
                }
            })
            .map_err(|e| notify::Error::io(e))?;
        Ok(Self { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> Vec<(i64, String)> {
        vec![(1, r"D:\Music".into()), (2, r"E:\Nhạc Trịnh".into()), (3, r"D:\Music2".into())]
    }

    #[test]
    fn finds_the_watched_folder_a_path_belongs_to() {
        let f = folders();
        assert_eq!(folder_for(Path::new(r"D:\Music\Jazz\a.mp3"), &f), Some(1));
        assert_eq!(folder_for(Path::new(r"d:\music\b.flac"), &f), Some(1), "case-insensitive");
        assert_eq!(folder_for(Path::new(r"D:\Music2\c.mp3"), &f), Some(3), "no prefix confusion");
        assert_eq!(folder_for(Path::new(r"E:\Nhạc Trịnh\Diễm xưa.mp3"), &f), Some(2));
        assert_eq!(folder_for(Path::new(r"C:\Other\x.mp3"), &f), None);
    }

    #[test]
    fn a_burst_of_paths_maps_to_each_affected_folder_once() {
        let paths: Vec<PathBuf> = [r"D:\Music\a.mp3", r"D:\Music\b.mp3", r"E:\Nhạc Trịnh\c.mp3", r"C:\x.mp3"].iter().map(PathBuf::from).collect();
        assert_eq!(affected_folders(&paths, &folders()), vec![1, 2]);
    }

    #[test]
    fn reports_changes_in_a_real_watched_folder() {
        let d = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let _watcher = FolderWatcher::start(vec![(7, d.path().to_string_lossy().into_owned())], Duration::from_millis(300), move |ids| {
            let _ = tx.send(ids);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(d.path().join("new song.mp3"), b"x").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).expect("no change reported"), vec![7]);
    }
}
