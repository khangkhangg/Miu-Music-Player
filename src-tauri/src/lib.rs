mod apple;
mod util;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Miu Music",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[tauri::command]
fn check_prereqs() -> apple::locate::PrereqReport {
    let report = apple::locate::PrereqReport::from(apple::locate::locate_on_system());
    if report.ready {
        log::info!("Apple DLLs found: {:?}", report.paths);
    } else {
        log::warn!("Apple DLLs missing: {:?}", report.missing);
    }
    report
}

pub fn run() {
    let logs = util::paths::logs_dir_in(&util::paths::appdata_root());
    if let Err(e) = util::log::init(&logs, log::LevelFilter::Info) {
        eprintln!("logging disabled: {e}");
    }
    log::info!("Miu Music {} started", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info, check_prereqs])
        .run(tauri::generate_context!())
        .expect("error while running Miu Music");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_reports_product_name_and_crate_version() {
        let info = app_info();
        assert_eq!(info.name, "Miu Music");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn app_info_matches_tauri_config() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let info = app_info();
        assert_eq!(conf["productName"], info.name);
        assert_eq!(conf["version"], info.version);
    }
}
