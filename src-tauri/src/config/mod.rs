//! Application settings.
//!
//! Every environment assumption the predecessor hardcoded lives here as a
//! setting: the workspace root, the WSL distribution, the Git Bash path, the
//! editor list, the toolchain search paths. Nothing in the domain layers reads
//! an absolute path that is not ultimately one of these values.
//!
//! Settings are layered, lowest priority first:
//!
//! ```text
//! Settings::default()      compiled-in, always valid
//!   <- detection           what was actually found on this machine
//!   <- settings.toml       what the user chose
//!   <- DCC_* environment   what a script or test overrides
//! ```
//!
//! Detection sits *below* the file so an explicit choice is never overwritten
//! by a guess, and *above* the defaults so a fresh install arrives pre-filled
//! rather than as a wall of empty fields.

pub mod detect;
pub mod store;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::exec::TransportPreference;

pub use store::{config_dir, load, save, ConfigPaths};

/// Settings schema version this build understands.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub schema_version: u32,
    pub docker: DockerSettings,
    pub workspace: WorkspaceSettings,
    pub toolchain: ToolchainSettings,
    pub behavior: BehaviorSettings,
    pub editors: Vec<EditorEntry>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            docker: DockerSettings::default(),
            workspace: WorkspaceSettings::default(),
            toolchain: ToolchainSettings::default(),
            behavior: BehaviorSettings::default(),
            editors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase", default)]
pub struct DockerSettings {
    pub transport: TransportPreference,
    /// Empty means "use whichever distribution answers first".
    #[ts(optional)]
    pub wsl_distro: Option<String>,
    /// Empty means the distribution's default user.
    ///
    /// Deliberately not `root`. The predecessor ran all fifty of its calls as
    /// root; almost none of them needed to.
    #[ts(optional)]
    pub wsl_user: Option<String>,
    /// Directory holding `docker-compose.yml`. Empty means the copy bundled
    /// with the application.
    #[ts(optional)]
    pub compose_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceSettings {
    /// Directories scanned for projects. Multiple roots are supported; the
    /// predecessor was fixed to a single hardcoded `C:\Projects`.
    pub roots: Vec<PathBuf>,
    /// How many levels below a root a project may sit. Depth 2 reproduces the
    /// predecessor's `<root>/<category>/<project>` layout.
    pub scan_depth: u8,
    /// How long a completed scan is reused before the disk is walked again.
    pub cache_ttl_secs: u64,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            scan_depth: 2,
            cache_ttl_secs: 30,
        }
    }
}

/// Which shell to hand the user after a project command finishes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "lowercase")]
pub enum ShellPreference {
    /// Use zsh when it exists, otherwise bash, otherwise the platform default.
    #[default]
    Auto,
    Zsh,
    Bash,
    PowerShell,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase", default)]
pub struct ToolchainSettings {
    #[ts(optional)]
    pub git_bash: Option<PathBuf>,
    pub preferred_shell: ShellPreference,
    /// Terminal emulator to launch. Empty means the platform default.
    #[ts(optional)]
    pub terminal: Option<String>,
    /// Directories containing versioned PHP installations.
    pub php_search_paths: Vec<PathBuf>,
    #[ts(optional)]
    pub node_nvm_path: Option<PathBuf>,
    #[ts(optional)]
    pub go_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase", default)]
pub struct BehaviorSettings {
    pub refresh_interval_secs: u64,
    /// Lines of scrollback kept per log stream.
    pub log_buffer_lines: usize,
    /// Process names the "Kill Dev Servers" button targets.
    ///
    /// **Empty by default, deliberately.** The predecessor shipped
    /// `php, node, go` hardcoded, which killed every Node process on the
    /// machine — language servers, other agents, unrelated work. Opting in is
    /// the user's decision to make, with the affected PIDs shown first.
    pub kill_dev_process_names: Vec<String>,
    /// Processes the app refuses to kill regardless of what is asked.
    pub protected_process_names: Vec<String>,
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            refresh_interval_secs: 10,
            log_buffer_lines: 500,
            kill_dev_process_names: Vec::new(),
            protected_process_names: [
                // Windows
                "system",
                "smss",
                "csrss",
                "wininit",
                "winlogon",
                "services",
                "lsass",
                "svchost",
                "explorer",
                "dwm",
                "spoolsv",
                // macOS
                "launchd",
                "kernel_task",
                "windowserver",
                "loginwindow",
                // Linux
                "systemd",
                "init",
                "dbus-daemon",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        }
    }
}

/// One entry in the per-project editor button row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct EditorEntry {
    pub label: String,
    /// Bare command name or absolute path. Validated before use.
    pub program: String,
    /// Arguments. The literal `{path}` is replaced with the project directory.
    #[serde(default = "default_editor_args")]
    pub args: Vec<String>,
}

fn default_editor_args() -> Vec<String> {
    vec!["{path}".to_owned()]
}

impl EditorEntry {
    /// Expands `{path}` into the arguments for a concrete project.
    #[must_use]
    pub fn args_for(&self, path: &str) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| arg.replace("{path}", path))
            .collect()
    }
}

/// A problem found while validating settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct SettingsIssue {
    /// Dotted path, e.g. `toolchain.gitBash`, so the UI can focus the field.
    pub field: String,
    pub message: String,
    /// `true` when the app cannot function until it is fixed.
    pub blocking: bool,
}

impl Settings {
    /// Checks the settings and reports every problem found.
    ///
    /// Only three things genuinely block: the transport, the WSL distribution
    /// when WSL is pinned, and the workspace roots. Everything else at worst
    /// hides one button, and is reported without stopping anything.
    #[must_use]
    pub fn validate(&self) -> Vec<SettingsIssue> {
        let mut issues = Vec::new();

        // Non-blocking findings are collected separately so the closure that
        // records them does not hold a borrow on the list the blocking checks
        // below also write to.
        let mut warnings: Vec<SettingsIssue> = Vec::new();
        let mut warn = |field: &str, message: String| {
            warnings.push(SettingsIssue {
                field: field.to_owned(),
                message,
                blocking: false,
            });
        };

        if self.workspace.roots.is_empty() {
            issues.push(SettingsIssue {
                field: "workspace.roots".to_owned(),
                message: "No workspace root is configured, so no projects can be listed."
                    .to_owned(),
                blocking: true,
            });
        }

        for root in &self.workspace.roots {
            if !root.is_dir() {
                warn(
                    "workspace.roots",
                    format!("`{}` is not a directory.", root.display()),
                );
            }
        }

        if self.workspace.scan_depth == 0 || self.workspace.scan_depth > 4 {
            warn(
                "workspace.scanDepth",
                "Scan depth must be between 1 and 4.".to_owned(),
            );
        }

        if self.behavior.refresh_interval_secs < 2 {
            warn(
                "behavior.refreshIntervalSecs",
                "Refreshing more often than every 2 seconds keeps docker permanently busy."
                    .to_owned(),
            );
        }

        if self.behavior.log_buffer_lines == 0 {
            warn(
                "behavior.logBufferLines",
                "A zero-line buffer would discard every log line on arrival.".to_owned(),
            );
        }

        if self.docker.transport == TransportPreference::Wsl && !cfg!(windows) {
            issues.push(SettingsIssue {
                field: "docker.transport".to_owned(),
                message: "WSL is only available on Windows.".to_owned(),
                blocking: true,
            });
        }

        for (index, editor) in self.editors.iter().enumerate() {
            if let Err(err) = crate::exec::validate::program(&editor.program) {
                warn("editors", format!("Editor {index}: {err}"));
            }
            if !editor.args.iter().any(|a| a.contains("{path}")) {
                warn(
                    "editors",
                    format!(
                        "Editor `{}` has no `{{path}}` argument, so it will not open the project.",
                        editor.label
                    ),
                );
            }
        }

        for name in &self.behavior.kill_dev_process_names {
            if self
                .behavior
                .protected_process_names
                .iter()
                .any(|p| p.eq_ignore_ascii_case(name))
            {
                issues.push(SettingsIssue {
                    field: "behavior.killDevProcessNames".to_owned(),
                    message: format!("`{name}` is a protected process and will never be killed."),
                    blocking: false,
                });
            }
        }

        issues.extend(warnings);
        issues
    }

    /// Whether anything blocks normal operation.
    #[must_use]
    pub fn has_blocking_issues(&self) -> bool {
        self.validate().iter().any(|issue| issue.blocking)
    }

    /// Case-insensitive protected-process check, used before any kill.
    #[must_use]
    pub fn is_protected(&self, process_name: &str) -> bool {
        let name = process_name.trim_end_matches(".exe");
        self.behavior
            .protected_process_names
            .iter()
            .any(|p| p.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with_root() -> Settings {
        Settings {
            workspace: WorkspaceSettings {
                roots: vec![std::env::temp_dir()],
                ..WorkspaceSettings::default()
            },
            ..Settings::default()
        }
    }

    #[test]
    fn kill_list_is_empty_by_default() {
        // The predecessor shipped php/node/go hardcoded and killed unrelated
        // work. Opting in must be a decision, not a default.
        assert!(BehaviorSettings::default()
            .kill_dev_process_names
            .is_empty());
    }

    #[test]
    fn protected_list_covers_all_three_platforms() {
        let defaults = BehaviorSettings::default();
        for name in ["lsass", "launchd", "systemd"] {
            assert!(
                defaults.protected_process_names.iter().any(|p| p == name),
                "missing `{name}`"
            );
        }
    }

    #[test]
    fn protected_check_ignores_case_and_exe_suffix() {
        let settings = Settings::default();
        assert!(settings.is_protected("lsass.exe"));
        assert!(settings.is_protected("LSASS"));
        assert!(settings.is_protected("Explorer.exe"));
        assert!(!settings.is_protected("node"));
    }

    #[test]
    fn missing_workspace_root_blocks() {
        let issues = Settings::default().validate();
        let root_issue = issues
            .iter()
            .find(|i| i.field == "workspace.roots")
            .expect("should report missing roots");
        assert!(root_issue.blocking);
        assert!(Settings::default().has_blocking_issues());
    }

    #[test]
    fn a_configured_root_clears_the_blocking_issue() {
        assert!(!settings_with_root().has_blocking_issues());
    }

    #[test]
    fn a_kill_target_that_is_also_protected_is_reported() {
        let mut settings = settings_with_root();
        settings.behavior.kill_dev_process_names = vec!["Explorer".to_owned()];

        let issues = settings.validate();
        let issue = issues
            .iter()
            .find(|i| i.field == "behavior.killDevProcessNames")
            .expect("should warn about the contradiction");
        assert!(!issue.blocking);
        assert!(issue.message.contains("never be killed"));
    }

    #[test]
    fn editor_without_path_placeholder_is_reported() {
        let mut settings = settings_with_root();
        settings.editors = vec![EditorEntry {
            label: "Broken".to_owned(),
            program: "code".to_owned(),
            args: vec!["--new-window".to_owned()],
        }];

        let issues = settings.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.field == "editors" && i.message.contains("{path}")),
            "{issues:?}"
        );
    }

    #[test]
    fn editor_arguments_expand_the_path_placeholder() {
        let editor = EditorEntry {
            label: "VS Code".to_owned(),
            program: "code".to_owned(),
            args: vec!["--new-window".to_owned(), "{path}".to_owned()],
        };
        assert_eq!(
            editor.args_for("/projects/shop"),
            vec!["--new-window", "/projects/shop"]
        );
    }

    #[test]
    fn relative_editor_program_is_reported() {
        let mut settings = settings_with_root();
        settings.editors = vec![EditorEntry {
            label: "Sneaky".to_owned(),
            program: "./evil".to_owned(),
            args: vec!["{path}".to_owned()],
        }];

        assert!(settings
            .validate()
            .iter()
            .any(|i| i.field == "editors" && i.message.contains("absolute path")));
    }

    #[test]
    fn defaults_round_trip_through_toml() {
        let text = toml::to_string(&Settings::default()).expect("serializes");
        let parsed: Settings = toml::from_str(&text).expect("parses");
        assert_eq!(parsed, Settings::default());
    }
}
