//! The single door to the operating system.
//!
//! Every external command in this application goes through [`CommandRunner`].
//! Domain code never names `docker.exe`, `wsl.exe`, or a shell. That has two
//! consequences worth stating plainly, because they are the reason this layer
//! exists at all:
//!
//! 1. **Testability.** Domain logic is exercised against `MockCommandRunner`,
//!    so continuous integration is meaningful on a runner with no Docker and no
//!    WSL installed.
//! 2. **Injection resistance.** [`CommandSpec`] carries an argument *vector*,
//!    never a command *string*. There is no shell to inject into. The
//!    predecessor project interpolated a free-text database name straight into
//!    `bash -c`; that shape is unrepresentable here.
//!
//! Implementations live in `native.rs` and `wsl.rs` (Batch 2).

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::ExecError;

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// How commands reach a container runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// A `docker` binary on this machine's own PATH — Docker Desktop on
    /// Windows and macOS, or Docker Engine on Linux.
    Native,

    /// A `docker` binary inside a WSL distribution, reached via `wsl.exe`.
    Wsl {
        distro: String,
        /// `None` means the distribution's default user. Root is requested
        /// only for the few operations that genuinely require it.
        user: Option<String>,
    },
}

/// Serializable view of [`Transport`] for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransportInfo {
    Native {
        docker_version: String,
    },
    Wsl {
        distro: String,
        user: Option<String>,
        docker_version: String,
    },
}

/// What the active transport can actually do.
///
/// Checked before a feature is offered, so the UI hides what would fail rather
/// than presenting a button that errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Dropping the Linux page cache. Only meaningful when docker runs inside
    /// a WSL distribution we can reach as root.
    pub page_cache_drop: bool,
    /// `docker compose` (the v2 plugin) is present. Compose v1 is not
    /// supported; see the accepted RFC.
    pub compose_v2: bool,
    /// `docker stats` is available for per-container memory reporting.
    pub container_stats: bool,
}

// ---------------------------------------------------------------------------
// Command description
// ---------------------------------------------------------------------------

/// A command to run, described as data.
///
/// `args` is a vector precisely so that no caller can smuggle shell
/// metacharacters into a command line. Do not add a `shell: bool` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Extra environment for the child. Secrets belong here or on stdin —
    /// never in `args`, which is visible in the host process list.
    pub env: Vec<(String, String)>,
    pub timeout: Option<Duration>,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            timeout: Some(Duration::from_secs(30)),
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Overrides the default 30s timeout. `None` means wait indefinitely, which
    /// is only correct for streams owned by a [`StreamHandle`].
    #[must_use]
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Result of a command that ran to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

impl CommandOutput {
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == 0
    }

    /// Non-empty stdout lines, trimmed. The shape most docker `--format`
    /// queries want.
    #[must_use]
    pub fn stdout_lines(&self) -> Vec<&str> {
        self.stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// One line from a live stream, on its way to the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
    /// Monotonic per stream. Lets the UI detect gaps if it ever drops behind.
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
pub struct StreamId(pub u64);

/// Ownership handle for a running stream.
///
/// Dropping or stopping this **must** kill the underlying child process.
/// Implementations are required to spawn with `kill_on_drop(true)`: aborting
/// the reader task alone leaves an orphaned `docker logs -f` running forever,
/// which is exactly the leak this handle exists to prevent.
#[derive(Debug)]
pub struct StreamHandle {
    id: StreamId,
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl StreamHandle {
    pub fn new(
        id: StreamId,
        stop: tokio::sync::watch::Sender<bool>,
        task: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self { id, stop, task }
    }

    #[must_use]
    pub fn id(&self) -> StreamId {
        self.id
    }

    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Signals the reader to wind down, then aborts it. The child dies with the
    /// task because of `kill_on_drop`.
    pub fn stop(self) {
        let _ = self.stop.send(true);
        self.task.abort();
    }
}

// ---------------------------------------------------------------------------
// Piped IO
// ---------------------------------------------------------------------------

/// Where a child's stdin comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeSource {
    /// Read a file, optionally gunzipping it **in this process**.
    ///
    /// Decompression happens in Rust rather than by piping through `gzip` in a
    /// shell. That is what allows the dump paths to avoid `bash -c` entirely.
    File { path: PathBuf, gunzip: bool },
}

/// Where a child's stdout goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeSink {
    /// Write to a file, optionally gzipping it in this process.
    File { path: PathBuf, gzip: bool },
}

/// Redirection for [`CommandRunner::pipe`]. Either side may be absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipeIo {
    pub stdin: Option<PipeSource>,
    pub stdout: Option<PipeSink>,
}

// ---------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------

/// Runs commands somewhere. The only abstraction over the operating system.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CommandRunner: Send + Sync + std::fmt::Debug {
    /// Runs to completion and collects output.
    ///
    /// A non-zero exit is returned as [`ExecError::NonZeroExit`], not as an
    /// `Ok` with a bad code — callers that genuinely tolerate failure should
    /// match on that variant explicitly rather than forgetting to check.
    async fn run(&self, spec: &CommandSpec) -> Result<CommandOutput, ExecError>;

    /// Runs a long-lived command, forwarding output lines to `tx`.
    ///
    /// `tx` is expected to be bounded. When the consumer falls behind, the
    /// send back-pressures the reader rather than growing without limit.
    async fn stream(
        &self,
        spec: &CommandSpec,
        tx: tokio::sync::mpsc::Sender<LogLine>,
    ) -> Result<StreamHandle, ExecError>;

    /// Runs with stdin and/or stdout redirected to files, compressing or
    /// decompressing in-process as requested.
    async fn pipe(&self, spec: &CommandSpec, io: &PipeIo) -> Result<CommandOutput, ExecError>;

    /// Which transport this runner uses.
    fn transport(&self) -> Transport;

    /// What this runner can do. Probed once at construction, not per call.
    fn capabilities(&self) -> Capabilities;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_builder_collects_args_in_order() {
        let spec = CommandSpec::new("docker")
            .arg("compose")
            .args(["up", "-d"])
            .arg("mysql");

        assert_eq!(spec.program, "docker");
        assert_eq!(spec.args, vec!["compose", "up", "-d", "mysql"]);
    }

    #[test]
    fn spec_defaults_to_a_finite_timeout() {
        // An unbounded default would let a wedged docker call hang a UI action
        // forever with no way for the user to tell what happened.
        assert_eq!(
            CommandSpec::new("docker").timeout,
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn shell_metacharacters_stay_inside_a_single_argument() {
        // The injection payload that worked against the predecessor project.
        let hostile = "mydb'; DROP DATABASE prod; --";
        let spec = CommandSpec::new("docker").arg("exec").arg(hostile);

        assert_eq!(spec.args.len(), 2);
        assert_eq!(spec.args[1], hostile);
        // No field on the spec can splice this into a command line: there is
        // no shell string anywhere in the type.
        assert_eq!(spec.program, "docker");
    }

    #[test]
    fn stdout_lines_trims_and_drops_blanks() {
        let out = CommandOutput {
            code: 0,
            stdout: "mysql-db\n\n  postgres-db  \n".into(),
            stderr: String::new(),
            duration: Duration::from_millis(1),
        };
        assert_eq!(out.stdout_lines(), vec!["mysql-db", "postgres-db"]);
        assert!(out.success());
    }

    #[test]
    fn capabilities_default_to_nothing_available() {
        // Fail closed: a feature is offered only after it is proven present.
        let caps = Capabilities::default();
        assert!(!caps.page_cache_drop);
        assert!(!caps.compose_v2);
        assert!(!caps.container_stats);
    }
}
