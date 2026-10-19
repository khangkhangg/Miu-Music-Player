use std::path::{Path, PathBuf};

const APP_DIR: &str = "MiuMusic";

pub fn app_data_dir_in(appdata: &Path) -> PathBuf {
    appdata.join(APP_DIR)
}

pub fn logs_dir_in(appdata: &Path) -> PathBuf {
    app_data_dir_in(appdata).join("logs")
}

pub fn db_path_in(appdata: &Path) -> PathBuf {
    app_data_dir_in(appdata).join("miumusic.db")
}

/// Private copies of Apple's DLLs: `%LOCALAPPDATA%\MiuMusic\apple` (machine-local, not roaming).
pub fn apple_dlls_dir_in(local_appdata: &Path) -> PathBuf {
    local_appdata.join(APP_DIR).join("apple")
}

/// `%LOCALAPPDATA%`, falling back to the temp dir if unset.
pub fn local_appdata_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// `%APPDATA%\MiuMusic`, falling back to the temp dir if APPDATA is unset.
pub fn appdata_root() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn app_data_dir_is_miumusic_under_appdata() {
        let base = Path::new(r"C:\Users\x\AppData\Roaming");
        assert_eq!(app_data_dir_in(base), base.join("MiuMusic"));
    }

    #[test]
    fn apple_dll_copies_live_in_local_app_data() {
        let local = Path::new(r"C:\Users\x\AppData\Local");
        assert_eq!(apple_dlls_dir_in(local), local.join("MiuMusic").join("apple"));
    }

    #[test]
    fn database_file_is_in_app_data_dir() {
        let base = Path::new(r"C:\Users\x\AppData\Roaming");
        assert_eq!(db_path_in(base), base.join("MiuMusic").join("miumusic.db"));
    }

    #[test]
    fn logs_dir_is_inside_app_data_dir() {
        let base = Path::new(r"C:\Users\x\AppData\Roaming");
        assert_eq!(logs_dir_in(base), base.join("MiuMusic").join("logs"));
    }
}
