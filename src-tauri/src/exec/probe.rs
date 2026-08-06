//! Environment detection.
//!
//! Decides which transport this machine can use, and what it is able to do.
//! Runs once at startup and again whenever the user changes the relevant
//! settings.
//!
//! The guiding rule is **fail open, report clearly**: a machine with no Docker
//! still opens the app, with an [`EnvironmentReport`] explaining what is
//! missing. Refusing to launch would leave the user with no way to reach the
//! settings screen that fixes the problem.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{
    native::NativeRunner, wsl::parse_distro_list, wsl::WslRunner, Capabilities, CommandRunner,
    CommandSpec, TransportInfo,
};
use crate::model::EnvironmentReport;

/// Probing must not stall startup. A wedged docker daemon should cost a couple
/// of seconds, not a hung window.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Which transport the user wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "lowercase")]
pub enum TransportPreference {
    /// Try native first, then WSL. The default.
    #[default]
    Auto,
    Native,
    Wsl,
}

/// Inputs to [`probe`], normally taken from settings.
#[derive(Debug, Clone, Default)]
pub struct ProbeOptions {
    pub preferred: TransportPreference,
    /// Explicit distribution. `None` means try each registered one in order.
    pub wsl_distro: Option<String>,
    /// Explicit user. `None` means the distribution's default — deliberately
    /// not root, unlike the predecessor which used root for all 50 calls.
    pub wsl_user: Option<String>,
}

/// Outcome of probing.
pub struct Probed {
    /// `None` when nothing usable was found.
    pub runner: Option<Arc<dyn CommandRunner>>,
    pub report: EnvironmentReport,
}

fn probe_spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec::new(program)
        .args(args.iter().copied())
        .timeout(Some(PROBE_TIMEOUT))
}

/// Asks the daemon for its version. Succeeds only if docker exists **and** the
/// daemon is reachable — a CLI with no daemon behind it is not a usable
/// transport, and reporting it as one would produce confusing failures later.
pub async fn server_version(runner: &dyn CommandRunner) -> Option<String> {
    let spec = probe_spec("docker", &["version", "--format", "{{.Server.Version}}"]);
    let output = runner.run(&spec).await.ok()?;
    let version = output.stdout.trim().to_owned();
    (!version.is_empty()).then_some(version)
}

/// Determines what the runner can do. Separated from [`probe`] so it can be
/// tested against a mock without a container runtime present.
pub async fn detect_capabilities(runner: &dyn CommandRunner) -> Capabilities {
    let compose_v2 = runner
        .run(&probe_spec("docker", &["compose", "version"]))
        .await
        .is_ok();

    let container_stats = runner
        .run(&probe_spec(
            "docker",
            &["stats", "--no-stream", "--format", "{{.Name}}"],
        ))
        .await
        .is_ok();

    // Dropping the page cache needs a writable /proc entry, which in practice
    // means root inside the distribution. Anything else is a Docker Desktop VM
    // we do not administer.
    let page_cache_drop = matches!(runner.transport(), super::Transport::Wsl { .. })
        && runner
            .run(&probe_spec("test", &["-w", "/proc/sys/vm/drop_caches"]))
            .await
            .is_ok();

    Capabilities {
        page_cache_drop,
        compose_v2,
        container_stats,
    }
}

/// Lists registered WSL distributions. Empty on non-Windows hosts.
pub async fn list_wsl_distros() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }

    // Probing uses a native runner because `wsl.exe --list` is a Windows
    // command, not something to run inside a distribution.
    let probe_runner = NativeRunner::new(Capabilities::default());
    let spec = probe_spec("wsl.exe", &["--list", "--quiet"]);

    match probe_runner.run(&spec).await {
        Ok(output) => parse_distro_list(&output.stdout),
        Err(_) => Vec::new(),
    }
}

/// Detects the best available transport.
pub async fn probe(options: &ProbeOptions) -> Probed {
    let mut warnings = Vec::new();

    let try_native = matches!(
        options.preferred,
        TransportPreference::Auto | TransportPreference::Native
    );
    let try_wsl = matches!(
        options.preferred,
        TransportPreference::Auto | TransportPreference::Wsl
    );

    if try_native {
        let runner = NativeRunner::new(Capabilities::default());
        if let Some(version) = server_version(&runner).await {
            let capabilities = detect_capabilities(&runner).await;
            if !capabilities.compose_v2 {
                warnings.push(
                    "Found docker, but not the Compose v2 plugin. Install it, or \
                     `docker compose` commands will fail."
                        .to_owned(),
                );
            }
            return Probed {
                runner: Some(Arc::new(NativeRunner::new(capabilities))),
                report: EnvironmentReport {
                    transport: Some(TransportInfo::Native {
                        docker_version: version,
                    }),
                    capabilities,
                    compose_dir: None,
                    warnings,
                },
            };
        }
        if options.preferred == TransportPreference::Native {
            warnings.push(
                "No reachable docker daemon on this machine's PATH. Is Docker Desktop running?"
                    .to_owned(),
            );
            return Probed {
                runner: None,
                report: EnvironmentReport::unavailable(warnings),
            };
        }
    }

    if try_wsl {
        let candidates = match &options.wsl_distro {
            Some(distro) => vec![distro.clone()],
            None => list_wsl_distros().await,
        };

        if candidates.is_empty() {
            warnings.push("No WSL distributions are registered on this machine.".to_owned());
        }

        for distro in candidates {
            let runner = WslRunner::new(
                distro.clone(),
                options.wsl_user.clone(),
                Capabilities::default(),
            );

            let Some(version) = server_version(&runner).await else {
                continue;
            };

            let capabilities = detect_capabilities(&runner).await;
            if !capabilities.compose_v2 {
                warnings.push(format!(
                    "Found docker in `{distro}`, but not the Compose v2 plugin."
                ));
            }

            return Probed {
                runner: Some(Arc::new(WslRunner::new(
                    distro.clone(),
                    options.wsl_user.clone(),
                    capabilities,
                ))),
                report: EnvironmentReport {
                    transport: Some(TransportInfo::Wsl {
                        distro,
                        user: options.wsl_user.clone(),
                        docker_version: version,
                    }),
                    capabilities,
                    compose_dir: None,
                    warnings,
                },
            };
        }
    }

    warnings.push(
        "No container runtime was found. Install Docker Desktop, or Docker Engine \
         inside a WSL distribution, then re-check from Settings."
            .to_owned(),
    );

    Probed {
        runner: None,
        report: EnvironmentReport::unavailable(warnings),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ExecError;
    use crate::exec::{CommandOutput, MockCommandRunner, Transport};

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            code: 0,
            stdout: stdout.to_owned(),
            stderr: String::new(),
            duration: Duration::from_millis(1),
        }
    }

    fn fail() -> ExecError {
        ExecError::NonZeroExit {
            program: "docker".to_owned(),
            code: 1,
            stderr: "not found".to_owned(),
        }
    }

    #[tokio::test]
    async fn server_version_reads_the_daemon_version() {
        let mut mock = MockCommandRunner::new();
        mock.expect_run().times(1).returning(|_| Ok(ok("28.1.0\n")));

        assert_eq!(server_version(&mock).await, Some("28.1.0".to_owned()));
    }

    #[tokio::test]
    async fn server_version_is_none_when_the_daemon_is_unreachable() {
        // A docker CLI with no daemon behind it must not count as usable.
        let mut mock = MockCommandRunner::new();
        mock.expect_run().times(1).returning(|_| Err(fail()));

        assert_eq!(server_version(&mock).await, None);
    }

    #[tokio::test]
    async fn server_version_rejects_empty_output() {
        let mut mock = MockCommandRunner::new();
        mock.expect_run().times(1).returning(|_| Ok(ok("   \n")));

        assert_eq!(server_version(&mock).await, None);
    }

    #[tokio::test]
    async fn capabilities_reflect_what_actually_answered() {
        let mut mock = MockCommandRunner::new();
        mock.expect_transport().returning(|| Transport::Native);
        mock.expect_run()
            .returning(|spec| match spec.args.first().map(String::as_str) {
                Some("compose") => Ok(ok("Docker Compose version v2.30.0")),
                _ => Err(fail()),
            });

        let caps = detect_capabilities(&mock).await;
        assert!(caps.compose_v2);
        assert!(!caps.container_stats);
        // Native transport can never drop a Linux page cache, regardless of
        // what the probe command would have returned.
        assert!(!caps.page_cache_drop);
    }

    #[tokio::test]
    async fn page_cache_drop_requires_wsl_and_a_writable_proc_entry() {
        let mut mock = MockCommandRunner::new();
        mock.expect_transport().returning(|| Transport::Wsl {
            distro: "Ubuntu".to_owned(),
            user: Some("root".to_owned()),
        });
        mock.expect_run().returning(|_| Ok(ok("")));

        let caps = detect_capabilities(&mock).await;
        assert!(caps.page_cache_drop);
    }

    #[tokio::test]
    async fn page_cache_drop_is_false_when_proc_is_not_writable() {
        let mut mock = MockCommandRunner::new();
        mock.expect_transport().returning(|| Transport::Wsl {
            distro: "Ubuntu".to_owned(),
            user: None,
        });
        mock.expect_run().returning(|spec| {
            if spec.program == "test" {
                Err(fail())
            } else {
                Ok(ok("ok"))
            }
        });

        let caps = detect_capabilities(&mock).await;
        assert!(!caps.page_cache_drop);
        assert!(caps.compose_v2);
    }

    #[tokio::test]
    async fn explicit_native_preference_does_not_fall_back_to_wsl() {
        // If the user pinned Native in Settings, silently using WSL instead
        // would make the setting a lie.
        let options = ProbeOptions {
            preferred: TransportPreference::Native,
            ..ProbeOptions::default()
        };
        let probed = probe(&options).await;

        if probed.runner.is_none() {
            assert!(!probed.report.warnings.is_empty());
            assert!(probed.report.transport.is_none());
        } else {
            assert!(matches!(
                probed.report.transport,
                Some(TransportInfo::Native { .. })
            ));
        }
    }

    #[test]
    fn auto_is_the_default_preference() {
        assert_eq!(TransportPreference::default(), TransportPreference::Auto);
    }
}
