use std::path::{Path, PathBuf};

const APP_DIR: &str = "MiuMusic";

pub fn app_data_dir_in(appdata: &Path) -> PathBuf {
    appdata.join(APP_DIR)
}

pub fn logs_dir_in(appdata: &Path) -> PathBuf {
    app_data_dir_in(appdata).join("logs")
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
    fn logs_dir_is_inside_app_data_dir() {
        let base = Path::new(r"C:\Users\x\AppData\Roaming");
        assert_eq!(logs_dir_in(base), base.join("MiuMusic").join("logs"));
    }
}
