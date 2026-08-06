//! Data transfer objects that cross the IPC boundary.
//!
//! Every type here derives [`ts_rs::TS`], and the TypeScript definitions are
//! generated from these declarations by `cargo test`. The frontend imports the
//! generated files rather than hand-writing matching interfaces, so a field
//! renamed in Rust breaks the TypeScript build instead of failing silently at
//! runtime.
//!
//! Naming convention: Rust stays `snake_case`, the wire stays `camelCase`.

pub mod project;
pub mod service;
pub mod system;

pub use project::{GitInfo, ProjectSummary, StackKind};
pub use service::{
    DbEngine, ServiceDefinition, ServiceRegistry, ServiceState, ServiceStatus,
    REGISTRY_SCHEMA_VERSION,
};
pub use system::{EnvironmentReport, PortEntry};
