//! Unofficial Rust client for TP-Link Tapo cameras.
//!
//! Talks to cameras on the local network only: the control API (device info, SD-card
//! recordings, detection events), the media stream used for live view, playback and
//! downloads, and helpers to turn that stream into playable video.
//!
//! This crate is a port of [pytapo](https://github.com/JurajNyiri/pytapo) and borrows
//! from other MIT-licensed community work; see `NOTICE` in the repository.
//!
//! **Disclaimer:** this project is not affiliated with, endorsed by, or supported by
//! TP-Link. "Tapo" and "TP-Link" are trademarks of their respective owners. It only
//! uses the cameras' local interfaces for interoperability, with credentials the user
//! owns, and comes with no warranty.

#![forbid(unsafe_code)]

mod api;
mod client;
pub mod discovery;
mod error;
pub mod media;
pub mod stream;
mod tls;
mod transport;

pub use client::{Camera, CameraConfig};
pub use error::{Error, Result, describe_code};
pub use tls::CertFingerprint;
pub use transport::PasswordHash;
