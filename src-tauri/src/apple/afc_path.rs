//! Paths on the iPhone's media partition as seen through AFC (F1.5). Every path we send to the
//! device goes through `device_path`, so nothing can escape the media root.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("device paths must start with /")]
    NotAbsolute,
    #[error("device paths may not contain ..")]
    ParentDir,
    #[error("device paths may not contain \\ or NUL")]
    BadChar,
}

/// Clean, absolute, root-confined form of `path` (`//` and `.` removed, no trailing slash).
pub fn device_path(path: &str) -> Result<String, PathError> {
    if path.contains(['\\', '\0']) {
        return Err(PathError::BadChar);
    }
    if !path.starts_with('/') {
        return Err(PathError::NotAbsolute);
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(PathError::ParentDir),
            p => parts.push(p),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

pub fn join(dir: &str, name: &str) -> Result<String, PathError> {
    device_path(&format!("{dir}/{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_clean_absolute_paths() {
        assert_eq!(device_path("/iTunes_Control/iTunes/MediaLibrary.sqlitedb").unwrap(), "/iTunes_Control/iTunes/MediaLibrary.sqlitedb");
        assert_eq!(device_path("/").unwrap(), "/");
    }

    #[test]
    fn tidies_redundant_parts() {
        assert_eq!(device_path("/iTunes_Control//Music/./F00/").unwrap(), "/iTunes_Control/Music/F00");
    }

    #[test]
    fn keeps_unicode_names() {
        assert_eq!(device_path("/MiuMusicTest/Mùa thu.mp3").unwrap(), "/MiuMusicTest/Mùa thu.mp3");
    }

    #[test]
    fn rejects_relative_paths() {
        assert_eq!(device_path("iTunes_Control"), Err(PathError::NotAbsolute));
        assert_eq!(device_path(""), Err(PathError::NotAbsolute));
    }

    #[test]
    fn rejects_climbing_out_of_the_media_root() {
        assert_eq!(device_path("/../private/var"), Err(PathError::ParentDir));
        assert_eq!(device_path("/iTunes_Control/../../etc"), Err(PathError::ParentDir));
    }

    #[test]
    fn rejects_windows_separators_and_nul() {
        assert_eq!(device_path("/iTunes_Control\\iTunes"), Err(PathError::BadChar));
        assert_eq!(device_path("/a\0b"), Err(PathError::BadChar));
    }

    #[test]
    fn joins_child_names() {
        assert_eq!(join("/iTunes_Control/Music", "F00").unwrap(), "/iTunes_Control/Music/F00");
        assert_eq!(join("/", "iTunes_Control").unwrap(), "/iTunes_Control");
        assert_eq!(join("/a", "../b"), Err(PathError::ParentDir));
    }
}
