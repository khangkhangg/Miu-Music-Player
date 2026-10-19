//! File access to a phone's media partition, as the sync code sees it. Implemented by
//! `apple::afc::Afc` for real phones and by in-memory fakes in tests (the Apple boundary).

pub trait DeviceFiles {
    /// Names in a directory (no `.`/`..`).
    fn list(&self, dir: &str) -> Result<Vec<String>, String>;
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    /// Create or replace a file.
    fn write(&self, path: &str, data: &[u8]) -> Result<(), String>;
    fn remove(&self, path: &str) -> Result<(), String>;
}
