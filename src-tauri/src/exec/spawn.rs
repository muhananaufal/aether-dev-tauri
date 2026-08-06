//! Process machinery shared by both transports.
//!
//! `native` and `wsl` differ only in how a [`CommandSpec`] becomes an operating
//! system command line. Everything after that — spawning, timeouts, streaming,
//! compression — is identical, and lives here so it is written and tested once.

use std::io;
use std::process::Stdio;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

use super::{
    CommandOutput, CommandSpec, LogLine, LogStream, PipeIo, PipeSink, PipeSource, StreamHandle,
    StreamId,
};
use crate::error::ExecError;

/// Chunk size for piped transfers. Large enough that a multi-gigabyte dump does
/// not spend its life in syscall overhead, small enough that memory stays flat.
const CHUNK: usize = 64 * 1024;

/// Bounded queue between the blocking compression thread and the async writer.
/// Bounded, so a slow consumer back-pressures the producer instead of letting
/// an unbounded queue grow to the size of the dump.
const PIPE_QUEUE: usize = 8;

/// `CREATE_NO_WINDOW`. Without it every docker call on Windows flashes a
/// console window — dozens of them during a status refresh.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Builds a [`Command`] from an already-translated program and argument list.
pub(super) fn build(program: &str, args: &[String], spec: &CommandSpec) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args);

    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    for (key, value) in &spec.env {
        cmd.env(key, value);
    }

    // Every child must die with its handle. Without this an aborted reader task
    // leaves `docker logs -f` running until the machine reboots.
    cmd.kill_on_drop(true);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    cmd
}

/// Runs to completion and collects output.
pub(super) async fn run(
    mut cmd: Command,
    program: &str,
    timeout: Option<Duration>,
) -> Result<CommandOutput, ExecError> {
    let started = Instant::now();

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd.spawn().map_err(|source| ExecError::Spawn {
        program: program.to_owned(),
        source,
    })?;

    let output = match timeout {
        Some(limit) => tokio::time::timeout(limit, child.wait_with_output())
            .await
            // Dropping the future drops the child, and `kill_on_drop` does the
            // rest. There is no orphan left behind by a timeout.
            .map_err(|_| ExecError::Timeout {
                program: program.to_owned(),
                timeout_ms: limit.as_millis() as u64,
            })?,
        None => child.wait_with_output().await,
    }
    .map_err(|source| ExecError::Io {
        program: program.to_owned(),
        source,
    })?;

    finish(
        program,
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        started.elapsed(),
    )
}

/// Runs a long-lived command, forwarding stdout and stderr lines to `tx`.
pub(super) fn stream(
    mut cmd: Command,
    program: &str,
    id: StreamId,
    tx: mpsc::Sender<LogLine>,
) -> Result<StreamHandle, ExecError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|source| ExecError::Spawn {
        program: program.to_owned(),
        source,
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);

    let task = tokio::spawn(async move {
        // `child` is moved in deliberately: when this task ends or is aborted,
        // the child drops here and `kill_on_drop` terminates it.
        let _child: Child = child;

        let mut out = stdout.map(|s| BufReader::new(s).lines());
        let mut err = stderr.map(|s| BufReader::new(s).lines());
        let mut seq: u64 = 0;

        loop {
            let out_done = out.is_none();
            let err_done = err.is_none();
            if out_done && err_done {
                break;
            }

            tokio::select! {
                biased;

                _ = stop_rx.changed() => break,

                line = async { out.as_mut().expect("guarded above").next_line().await }, if !out_done => {
                    match line {
                        Ok(Some(text)) => {
                            seq += 1;
                            if tx.send(LogLine { stream: LogStream::Stdout, text, seq }).await.is_err() {
                                break; // receiver gone: nobody is listening any more
                            }
                        }
                        // EOF or a read error: this half is finished either way.
                        _ => out = None,
                    }
                }

                line = async { err.as_mut().expect("guarded above").next_line().await }, if !err_done => {
                    match line {
                        Ok(Some(text)) => {
                            seq += 1;
                            if tx.send(LogLine { stream: LogStream::Stderr, text, seq }).await.is_err() {
                                break;
                            }
                        }
                        _ => err = None,
                    }
                }
            }
        }
    });

    Ok(StreamHandle::new(id, stop_tx, task))
}

/// Runs with stdin and/or stdout redirected to files, (de)compressing in-process.
pub(super) async fn pipe(
    mut cmd: Command,
    program: &str,
    io_spec: &PipeIo,
    timeout: Option<Duration>,
) -> Result<CommandOutput, ExecError> {
    let started = Instant::now();

    cmd.stdin(if io_spec.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    // Always piped, sink or no sink: an undrained stdout fills its pipe buffer
    // and the child blocks forever writing into it.
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|source| ExecError::Spawn {
        program: program.to_owned(),
        source,
    })?;

    let io_err = |source: io::Error| ExecError::Io {
        program: program.to_owned(),
        source,
    };

    // Feed stdin and drain stdout concurrently. Doing them in sequence
    // deadlocks the moment either pipe buffer fills.
    let stdin_task = match (child.stdin.take(), io_spec.stdin.clone()) {
        (Some(handle), Some(source)) => Some(tokio::spawn(feed_stdin(handle, source))),
        _ => None,
    };

    let stdout_task = match (child.stdout.take(), io_spec.stdout.clone()) {
        (Some(handle), Some(sink)) => Some(tokio::spawn(drain_stdout(handle, sink))),
        (Some(mut handle), None) => Some(tokio::spawn(async move {
            let mut buf = Vec::new();
            tokio::io::copy(&mut handle, &mut buf).await.map(|_| ())
        })),
        _ => None,
    };

    let wait = async {
        let status = child.wait().await?;
        if let Some(task) = stdin_task {
            task.await.map_err(join_to_io)??;
        }
        if let Some(task) = stdout_task {
            task.await.map_err(join_to_io)??;
        }
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            use tokio::io::AsyncReadExt;
            let mut raw = Vec::new();
            handle.read_to_end(&mut raw).await?;
            stderr = String::from_utf8_lossy(&raw).into_owned();
        }
        Ok::<_, io::Error>((status, stderr))
    };

    let (status, stderr) = match timeout {
        Some(limit) => tokio::time::timeout(limit, wait)
            .await
            .map_err(|_| ExecError::Timeout {
                program: program.to_owned(),
                timeout_ms: limit.as_millis() as u64,
            })?,
        None => wait.await,
    }
    .map_err(io_err)?;

    finish(
        program,
        status.code(),
        String::new(),
        stderr,
        started.elapsed(),
    )
}

fn join_to_io(err: tokio::task::JoinError) -> io::Error {
    io::Error::other(format!("transfer task failed: {err}"))
}

/// Writes a file into the child's stdin, gunzipping on the way if asked.
async fn feed_stdin(mut stdin: ChildStdin, source: PipeSource) -> io::Result<()> {
    match source {
        PipeSource::File {
            path,
            gunzip: false,
        } => {
            let mut file = tokio::fs::File::open(&path).await?;
            tokio::io::copy(&mut file, &mut stdin).await?;
        }
        PipeSource::File { path, gunzip: true } => {
            let (tx, mut rx) = mpsc::channel::<io::Result<Vec<u8>>>(PIPE_QUEUE);

            // flate2 is synchronous, so decompression runs on the blocking pool
            // and hands chunks over rather than blocking a worker thread.
            tokio::task::spawn_blocking(move || {
                let result = (|| -> io::Result<()> {
                    use std::io::Read;
                    let file = std::fs::File::open(&path)?;
                    let mut decoder = GzDecoder::new(std::io::BufReader::new(file));
                    let mut buf = vec![0u8; CHUNK];
                    loop {
                        let read = decoder.read(&mut buf)?;
                        if read == 0 {
                            return Ok(());
                        }
                        if tx.blocking_send(Ok(buf[..read].to_vec())).is_err() {
                            return Ok(()); // consumer went away
                        }
                    }
                })();
                if let Err(err) = result {
                    let _ = tx.blocking_send(Err(err));
                }
            });

            while let Some(chunk) = rx.recv().await {
                stdin.write_all(&chunk?).await?;
            }
        }
    }

    // The child waits for EOF. Without this shutdown, `mysql` sits forever.
    stdin.shutdown().await
}

/// Writes the child's stdout to a file, gzipping on the way if asked.
async fn drain_stdout(mut stdout: ChildStdout, sink: PipeSink) -> io::Result<()> {
    match sink {
        PipeSink::File { path, gzip: false } => {
            let mut file = tokio::fs::File::create(&path).await?;
            tokio::io::copy(&mut stdout, &mut file).await?;
            file.flush().await?;
        }
        PipeSink::File { path, gzip: true } => {
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(PIPE_QUEUE);

            let writer = tokio::task::spawn_blocking(move || -> io::Result<()> {
                use std::io::Write;
                let file = std::fs::File::create(&path)?;
                let mut encoder = GzEncoder::new(file, Compression::default());
                while let Some(chunk) = rx.blocking_recv() {
                    encoder.write_all(&chunk)?;
                }
                encoder.finish()?.flush()
            });

            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; CHUNK];
            loop {
                let read = stdout.read(&mut buf).await?;
                if read == 0 {
                    break;
                }
                if tx.send(buf[..read].to_vec()).await.is_err() {
                    break;
                }
            }
            drop(tx); // closes the queue so the writer can finish the gzip trailer
            writer.await.map_err(join_to_io)??;
        }
    }
    Ok(())
}

/// Shared exit-status interpretation.
fn finish(
    program: &str,
    code: Option<i32>,
    stdout: String,
    stderr: String,
    duration: Duration,
) -> Result<CommandOutput, ExecError> {
    let Some(code) = code else {
        // Unix only: no exit code means a signal terminated the process.
        return Err(ExecError::Signalled {
            program: program.to_owned(),
        });
    };

    if code != 0 {
        return Err(ExecError::NonZeroExit {
            program: program.to_owned(),
            code,
            stderr,
        });
    }

    Ok(CommandOutput {
        code,
        stdout,
        stderr,
        duration,
    })
}
