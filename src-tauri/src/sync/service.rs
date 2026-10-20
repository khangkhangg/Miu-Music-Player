//! App glue for the sync plan (F5.3): a preview of what a sync would change on a phone.

use serde::Serialize;
use tauri::State;

use crate::library::service::SharedDb;
use crate::sync::plan::{check_space, plan, Plan};

/// Room left on the phone after a sync, so iOS keeps working comfortably.
const MARGIN_BYTES: u64 = 200_000_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortage {
    need: u64,
    available: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreview {
    plan: Plan,
    /// Set when the plan does not fit in `free_bytes`.
    shortage: Option<Shortage>,
}

#[tauri::command]
pub fn sync_preview(udid: String, free_bytes: Option<u64>, db: State<'_, SharedDb>) -> Result<SyncPreview, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    let p = plan(&d, &udid, &crate::settings::conversion_policy(&d)).map_err(|e| e.to_string())?;
    let shortage = free_bytes.and_then(|free| check_space(&p, free, MARGIN_BYTES).err()).map(|s| Shortage { need: s.need, available: s.available });
    Ok(SyncPreview { plan: p, shortage })
}
