//! Ordinary music players (USB mass storage, SD cards): finding them and syncing liked songs to them.

pub mod drives;
pub mod layout;
pub mod profile;
pub mod sync;
#[cfg(windows)]
pub mod service;
#[cfg(windows)]
pub mod volumes;
pub mod mtp;
