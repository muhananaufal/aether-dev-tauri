//! Transport for a `docker` binary on this machine's own PATH.
//!
//! Docker Desktop on Windows and macOS, Docker Engine on Linux. The simplest
//! of the two transports: a [`CommandSpec`] is already what the OS wants.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::{
    spawn, Capabilities, CommandOutput, CommandRunner, CommandSpec, LogLine, PipeIo, StreamHandle,
    StreamId, Transport,
};
use crate::error::ExecError;

#[derive(Debug)]
pub struct NativeRunner {
    capabilities: Capabilities,
    next_stream_id: AtomicU64,
}

impl NativeRunner {
    #[must_use]
    pub fn new(capabilities: Capabilities) -> Self {
        Self {
            capabilities,
            next_stream_id: AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> StreamId {
        StreamId(self.next_stream_id.fetch_add(1, Ordering::Relaxed))
    }
}

#[async_trait]
impl CommandRunner for NativeRunner {
    async fn run(&self, spec: &CommandSpec) -> Result<CommandOutput, ExecError> {
        let cmd = spawn::build(&spec.program, &spec.args, spec);
        spawn::run(cmd, &spec.program, spec.timeout).await
    }

    async fn stream(
        &self,
        spec: &CommandSpec,
        tx: mpsc::Sender<LogLine>,
    ) -> Result<StreamHandle, ExecError> {
        let cmd = spawn::build(&spec.program, &spec.args, spec);
        spawn::stream(cmd, &spec.program, self.next_id(), tx)
    }

    async fn pipe(&self, spec: &CommandSpec, io: &PipeIo) -> Result<CommandOutput, ExecError> {
        let cmd = spawn::build(&spec.program, &spec.args, spec);
        spawn::pipe(cmd, &spec.program, io, spec.timeout).await
    }

    fn transport(&self) -> Transport {
        Transport::Native
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn runner() -> NativeRunner {
        NativeRunner::new(Capabilities::default())
    }

    #[test]
    fn reports_native_transport() {
        assert_eq!(runner().transport(), Transport::Native);
    }

    #[test]
    fn stream_ids_are_unique_and_monotonic() {
        let runner = runner();
        let first = runner.next_id();
        let second = runner.next_id();
        assert!(second.0 > first.0);
    }

    // These exercise the real process machinery against binaries that exist
    // everywhere, so they run on a bare CI machine with no Docker.
    #[cfg(windows)]
    const ECHO: (&str, &[&str]) = ("cmd", &["/c", "echo", "hello"]);
    #[cfg(not(windows))]
    const ECHO: (&str, &[&str]) = ("echo", &["hello"]);

    #[tokio::test]
    async fn runs_a_real_command_and_captures_stdout() {
        let spec = CommandSpec::new(ECHO.0).args(ECHO.1.iter().copied());
        let out = runner().run(&spec).await.expect("echo should succeed");
        assert!(out.success());
        assert!(out.stdout.contains("hello"), "got: {:?}", out.stdout);
    }

    #[tokio::test]
    async fn missing_program_is_a_spawn_error_not_a_panic() {
        let spec = CommandSpec::new("definitely-not-a-real-binary-xyz");
        let err = runner().run(&spec).await.expect_err("should fail");
        assert!(matches!(err, ExecError::Spawn { .. }), "got: {err:?}");
    }

    #[tokio::test]
    async fn nonzero_exit_is_surfaced_as_an_error() {
        #[cfg(windows)]
        let spec = CommandSpec::new("cmd").args(["/c", "exit", "3"]);
        #[cfg(not(windows))]
        let spec = CommandSpec::new("sh").args(["-c", "exit 3"]);

        let err = runner().run(&spec).await.expect_err("should fail");
        match err {
            ExecError::NonZeroExit { code, .. } => assert_eq!(code, 3),
            other => panic!("expected NonZeroExit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_hung_command_times_out_rather_than_hanging_the_caller() {
        #[cfg(windows)]
        let spec = CommandSpec::new("cmd")
            .args(["/c", "ping", "-n", "30", "127.0.0.1"])
            .timeout(Some(Duration::from_millis(300)));
        #[cfg(not(windows))]
        let spec = CommandSpec::new("sleep")
            .arg("30")
            .timeout(Some(Duration::from_millis(300)));

        let err = runner().run(&spec).await.expect_err("should time out");
        assert!(matches!(err, ExecError::Timeout { .. }), "got: {err:?}");
    }

    #[tokio::test]
    async fn streaming_delivers_lines_then_ends_at_eof() {
        #[cfg(windows)]
        let spec = CommandSpec::new("cmd").args(["/c", "echo", "one"]);
        #[cfg(not(windows))]
        let spec = CommandSpec::new("sh").args(["-c", "echo one"]);

        let (tx, mut rx) = mpsc::channel(16);
        let handle = runner().stream(&spec, tx).await.expect("should start");

        let line = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("should not time out")
            .expect("should receive a line");
        assert!(line.text.contains("one"), "got: {:?}", line.text);
        assert_eq!(line.seq, 1);

        handle.stop();
    }
}
