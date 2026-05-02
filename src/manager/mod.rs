//! Auto-download and caching of WebDriver binaries.
//!
//! Adapted from `thirtyfour`'s `manager` module. Handles version resolution
//! against upstream metadata and downloading/extracting driver archives into
//! a local cache. Xenon retains its own driver-process spawning logic — this
//! module only resolves a `(browser, version)` pair to a binary on disk.

mod browser;
mod download;
mod error;
mod resolver;
mod version;

pub use resolver::DriverResolver;
pub use version::DriverVersion;
