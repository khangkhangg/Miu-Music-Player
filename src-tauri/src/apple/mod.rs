//! Everything that touches Apple's DLLs lives here (CLAUDE.md rule 2).
#[cfg(windows)]
pub mod cf;
#[cfg(windows)]
pub mod device;
pub mod exports;
#[cfg(windows)]
pub mod loader;
pub mod locate;
pub mod stage;
pub mod value;
