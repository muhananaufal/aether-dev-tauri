//! Error types and the boundary between them.
//!
//! Two layers, deliberately separate:
//!
//! * [`ExecError`] — what the operating system did. Carries raw detail and is
//!   never handed to the webview.
//! * [`AppError`] — what the user needs to know. Serializes across IPC with a
//!   machine-readable `code` so the frontend branches on a discriminant rather
//!   than pattern-matching English prose.
//!
//! The conversion between them logs the raw error and returns a sanitized one.
//! That is the only place raw system detail is allowed to stop.

use serde::Serialize;
use ts_rs::TS;

pub type AppResult<T> = Result<T, AppError>;

/// Transport-layer failure. Internal to the Rust side.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("failed to spawn `{program}`")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`{program}` exited with status {code}")]
    NonZeroExit {
        program: String,
        code: i32,
        stderr: String,
    },

    #[error("`{program}` did not finish within {timeout_ms}ms")]
    Timeout { program: String, timeout_ms: u64 },

    #[error("`{program}` was terminated by a signal")]
    Signalled { program: String },

    #[error("io error while talking to `{program}`")]
    Io {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("no usable container runtime was found")]
    NoRuntime,

    #[error("wsl distribution `{0}` is not registered")]
    WslDistroMissing(String),
}

/// Error as seen by the frontend.
///
/// Serialized adjacently tagged, so TypeScript receives
/// `{ code: "COMMAND_FAILED", detail: { … } }` and can switch on `code`.
#[derive(Debug, thiserror::Error, Serialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(tag = "code", content = "detail", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppError {
    /// Neither a native docker binary nor a WSL distribution with docker was
    /// found. The UI should route the user to Settings rather than retrying.
    #[error("no container runtime detected")]
    DockerUnavailable,

    #[error("WSL distribution `{0}` is not registered")]
    WslDistroMissing(String),

    /// A user-supplied name failed validation before it was allowed anywhere
    /// near a command line.
    #[error("invalid name: {0}")]
    InvalidIdentifier(String),

    #[error("command failed with exit code {code}")]
    CommandFailed { code: i32, stderr_excerpt: String },

    #[error("configuration field `{field}` is invalid: {reason}")]
    ConfigInvalid { field: String, reason: String },

    /// The active transport cannot do this. Dropping the Linux page cache, for
    /// example, is meaningless when docker runs under Docker Desktop.
    #[error("not supported by the active transport")]
    UnsupportedOnTransport,

    /// A destructive operation of the same kind is already running. Guards
    /// against two windows importing into the same database at once.
    #[error("another `{operation}` is already in progress")]
    Busy { operation: String },

    /// Something unexpected. The real cause is in the log, not in this message.
    #[error("internal error")]
    Internal,
}

impl From<ExecError> for AppError {
    fn from(err: ExecError) -> Self {
        // Log the real thing exactly once, here, then hand back the sanitized
        // version. Without this the obfuscation below would destroy the only
        // copy of the diagnostic.
        tracing::error!(error = ?err, "transport error");

        match err {
            ExecError::NoRuntime => AppError::DockerUnavailable,
            ExecError::WslDistroMissing(d) => AppError::WslDistroMissing(d),
            ExecError::NonZeroExit { code, stderr, .. } => AppError::CommandFailed {
                code,
                stderr_excerpt: excerpt(&scrub_secrets(&stderr)),
            },
            ExecError::Timeout {
                program,
                timeout_ms,
            } => AppError::CommandFailed {
                code: -1,
                stderr_excerpt: format!("`{program}` timed out after {timeout_ms}ms"),
            },
            ExecError::Spawn { .. } | ExecError::Io { .. } | ExecError::Signalled { .. } => {
                AppError::Internal
            }
        }
    }
}

/// Maximum stderr text forwarded to the UI. A runaway process can emit
/// megabytes; the webview does not need them and IPC should not carry them.
const EXCERPT_LIMIT: usize = 2048;

fn excerpt(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= EXCERPT_LIMIT {
        return trimmed.to_owned();
    }
    // Slice on a character boundary; stderr is not guaranteed to be ASCII.
    let cut = trimmed
        .char_indices()
        .take_while(|(i, _)| *i <= EXCERPT_LIMIT)
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("{}\n… (truncated)", &trimmed[..cut])
}

/// Removes credential-shaped text before it reaches a log sink or the UI.
///
/// This is a backstop, not the primary defence: commands are built from
/// argument vectors and secrets are passed via environment or stdin, so they
/// should not appear in stderr at all. Batch 3 extends this with the concrete
/// values loaded from `.env` so that even an unexpected echo is caught.
pub fn scrub_secrets(text: &str) -> String {
    const MASK: &str = "***";
    let mut out = String::with_capacity(text.len());

    for line in text.lines() {
        let mut scrubbed = line.to_owned();

        // `KEY=value` where the key looks sensitive.
        if let Some(eq) = scrubbed.find('=') {
            let key_upper = scrubbed[..eq].to_ascii_uppercase();
            if ["PASSWORD", "PASSWD", "SECRET", "TOKEN", "APIKEY", "API_KEY"]
                .iter()
                .any(|needle| key_upper.contains(needle))
            {
                scrubbed.truncate(eq + 1);
                scrubbed.push_str(MASK);
            }
        }

        // MySQL's `-psecret` form, which has no separator to key off.
        if let Some(pos) = scrubbed.find("-p") {
            let rest = &scrubbed[pos + 2..];
            let end = rest.find(char::is_whitespace).map_or(rest.len(), |i| i);
            if end > 0 {
                scrubbed.replace_range(pos + 2..pos + 2 + end, MASK);
            }
        }

        out.push_str(&scrubbed);
        out.push('\n');
    }

    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_masks_key_value_secrets() {
        let input = "MYSQL_ROOT_PASSWORD=hunter2\nMYSQL_USER=dev";
        let out = scrub_secrets(input);
        assert_eq!(out, "MYSQL_ROOT_PASSWORD=***\nMYSQL_USER=dev");
    }

    #[test]
    fn scrub_masks_mysql_inline_password_flag() {
        let out = scrub_secrets("mysqldump -u root -phunter2 mydb");
        assert_eq!(out, "mysqldump -u root -p*** mydb");
        assert!(!out.contains("hunter2"));
    }

    #[test]
    fn scrub_leaves_ordinary_text_untouched() {
        let input = "ERROR 1045: access denied for user 'root'@'localhost'";
        assert_eq!(scrub_secrets(input), input);
    }

    #[test]
    fn excerpt_truncates_and_marks() {
        let long = "x".repeat(EXCERPT_LIMIT + 500);
        let out = excerpt(&long);
        assert!(out.len() < long.len());
        assert!(out.ends_with("… (truncated)"));
    }

    #[test]
    fn excerpt_keeps_short_text_verbatim() {
        assert_eq!(excerpt("  boom  "), "boom");
    }

    #[test]
    fn exec_error_never_leaks_spawn_detail_to_frontend() {
        let err = ExecError::Spawn {
            program: "/secret/path/docker".into(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        };
        let app: AppError = err.into();
        let json = serde_json::to_string(&app).expect("AppError must serialize");
        assert_eq!(json, r#"{"code":"INTERNAL"}"#);
        assert!(!json.contains("/secret/path"));
    }

    #[test]
    fn command_failure_serializes_with_machine_code() {
        let err = ExecError::NonZeroExit {
            program: "docker".into(),
            code: 125,
            stderr: "MYSQL_PASSWORD=hunter2\ncontainer not found".into(),
        };
        let app: AppError = err.into();
        let json = serde_json::to_string(&app).expect("AppError must serialize");
        assert!(json.contains(r#""code":"COMMAND_FAILED""#));
        assert!(json.contains("125"));
        assert!(json.contains("container not found"));
        assert!(!json.contains("hunter2"));
    }
}
