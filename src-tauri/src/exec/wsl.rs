//! Transport for a `docker` binary inside a WSL distribution.
//!
//! Translation is `wsl.exe -d <distro> [--user <user>] -- <program> <args…>`.
//!
//! The `--` matters: it terminates `wsl.exe`'s own option parsing, so a program
//! or argument that happens to start with a hyphen is passed through instead of
//! being eaten. Everything after it is delivered as an argument vector — there
//! is no `bash -c`, and therefore nothing to quote or escape.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{
    spawn, Capabilities, CommandOutput, CommandRunner, CommandSpec, LogLine, PipeIo, StreamHandle,
    StreamId, Transport,
};
use crate::error::ExecError;

const WSL: &str = "wsl.exe";

#[derive(Debug)]
pub struct WslRunner {
    distro: String,
    user: Option<String>,
    capabilities: Capabilities,
    next_stream_id: AtomicU64,
}

impl WslRunner {
    #[must_use]
    pub fn new(
        distro: impl Into<String>,
        user: Option<String>,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            distro: distro.into(),
            user,
            capabilities,
            next_stream_id: AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> StreamId {
        StreamId(self.next_stream_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Builds the `wsl.exe` argument vector for a spec.
    fn translate(&self, spec: &CommandSpec) -> Vec<String> {
        let mut args = vec!["-d".to_owned(), self.distro.clone()];

        if let Some(user) = &self.user {
            args.push("--user".to_owned());
            args.push(user.clone());
        }

        args.push("--".to_owned());
        args.push(spec.program.clone());
        args.extend(spec.args.iter().cloned());
        args
    }

    /// Prepares the command, including the environment bridge.
    fn command(&self, spec: &CommandSpec) -> tokio::process::Command {
        let args = self.translate(spec);

        // `spec.env` names Windows-side variables as far as `wsl.exe` is
        // concerned. WSL forwards nothing by default: without WSLENV listing
        // them, the Linux process sees none of these and a password passed by
        // environment silently becomes empty. Discovering that at runtime looks
        // like an authentication failure, not a plumbing bug.
        let mut cmd = spawn::build(WSL, &args, spec);
        if !spec.env.is_empty() {
            let names: Vec<&str> = spec.env.iter().map(|(key, _)| key.as_str()).collect();
            cmd.env("WSLENV", names.join(":"));
        }
        cmd
    }
}

#[async_trait]
impl CommandRunner for WslRunner {
    async fn run(&self, spec: &CommandSpec) -> Result<CommandOutput, ExecError> {
        spawn::run(self.command(spec), &spec.program, spec.timeout).await
    }

    async fn stream(
        &self,
        spec: &CommandSpec,
        tx: mpsc::Sender<LogLine>,
    ) -> Result<StreamHandle, ExecError> {
        spawn::stream(self.command(spec), &spec.program, self.next_id(), tx)
    }

    async fn pipe(&self, spec: &CommandSpec, io: &PipeIo) -> Result<CommandOutput, ExecError> {
        spawn::pipe(self.command(spec), &spec.program, io, spec.timeout).await
    }

    fn transport(&self) -> Transport {
        Transport::Wsl {
            distro: self.distro.clone(),
            user: self.user.clone(),
        }
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
}

/// Parses `wsl.exe --list --quiet` output.
///
/// The output is UTF-16LE, which the caller has already lossily decoded. Stray
/// NUL bytes survive that decode, so they are stripped here rather than
/// becoming part of a distribution name that then fails to match.
#[must_use]
pub fn parse_distro_list(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| line.replace('\u{0}', "").trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(user: Option<&str>) -> WslRunner {
        WslRunner::new(
            "Ubuntu",
            user.map(ToOwned::to_owned),
            Capabilities::default(),
        )
    }

    #[test]
    fn translates_to_a_distro_scoped_argument_vector() {
        let spec = CommandSpec::new("docker").args(["ps", "--format", "{{.Names}}"]);
        let args = runner(None).translate(&spec);

        assert_eq!(
            args,
            vec![
                "-d",
                "Ubuntu",
                "--",
                "docker",
                "ps",
                "--format",
                "{{.Names}}"
            ]
        );
    }

    #[test]
    fn includes_the_user_flag_only_when_one_is_configured() {
        let spec = CommandSpec::new("docker").arg("ps");

        assert!(!runner(None).translate(&spec).contains(&"--user".to_owned()));

        let elevated = runner(Some("root")).translate(&spec);
        let position = elevated
            .iter()
            .position(|a| a == "--user")
            .expect("present");
        assert_eq!(elevated[position + 1], "root");
    }

    #[test]
    fn separator_precedes_the_command_so_hyphens_are_not_eaten() {
        // Without `--`, wsl.exe would try to interpret `--version` as its own.
        let spec = CommandSpec::new("docker").arg("--version");
        let args = runner(None).translate(&spec);

        let separator = args
            .iter()
            .position(|a| a == "--")
            .expect("separator present");
        let program = args
            .iter()
            .position(|a| a == "docker")
            .expect("program present");
        assert!(separator < program);
    }

    #[test]
    fn hostile_argument_survives_as_exactly_one_element() {
        // The payload that broke the predecessor. Here it is one argv entry;
        // there is no command string for it to escape from.
        let hostile = "mydb'; DROP DATABASE prod; --";
        let spec = CommandSpec::new("mysql").arg(hostile);
        let args = runner(None).translate(&spec);

        assert_eq!(args.iter().filter(|a| *a == hostile).count(), 1);
        assert_eq!(args.last().map(String::as_str), Some(hostile));
    }

    #[test]
    fn transport_reports_distro_and_user() {
        assert_eq!(
            runner(Some("root")).transport(),
            Transport::Wsl {
                distro: "Ubuntu".to_owned(),
                user: Some("root".to_owned()),
            }
        );
    }

    #[test]
    fn distro_list_strips_utf16_nul_padding() {
        // Verbatim shape of `wsl -l -q` decoded lossily from UTF-16LE.
        let raw =
            "U\u{0}b\u{0}u\u{0}n\u{0}t\u{0}u\u{0}\r\n\u{0}D\u{0}e\u{0}b\u{0}i\u{0}a\u{0}n\u{0}\r\n";
        assert_eq!(parse_distro_list(raw), vec!["Ubuntu", "Debian"]);
    }

    #[test]
    fn distro_list_ignores_blank_lines() {
        assert_eq!(
            parse_distro_list("Ubuntu\n\n  \nDebian\n"),
            vec!["Ubuntu", "Debian"]
        );
    }
}
