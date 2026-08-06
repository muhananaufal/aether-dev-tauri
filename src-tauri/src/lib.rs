//! Dev Control Center — application core.
//!
//! Layering, from the outside in:
//!
//! ```text
//! commands/   thin Tauri adapters: deserialize, call domain, serialize
//! docker/ project/ ports/ logs/    domain logic, depends only on `exec`
//! exec/       the single door to the operating system
//! ```
//!
//! Domain code never names `docker.exe` or `wsl.exe`. It talks to
//! [`exec::CommandRunner`], which is why the whole of it can be tested on a
//! runner with no Docker installed.

pub mod config;
pub mod error;
pub mod exec;
pub mod model;

pub use error::{AppError, AppResult};

/// Builds and runs the Tauri application.
///
/// # Panics
/// Panics if the Tauri runtime cannot start, which is unrecoverable — there is
/// no window to report the failure in.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to start the Tauri application");
}
