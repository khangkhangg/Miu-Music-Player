//! Everything that touches Apple's DLLs lives here (CLAUDE.md rule 2).
#[cfg(windows)]
pub mod cf;
pub mod exports;
#[cfg(windows)]
pub mod loader;
pub mod locate;
pub mod stage;
