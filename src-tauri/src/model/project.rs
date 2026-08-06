//! Workspace project descriptions.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Detected project stack.
///
/// Ordering matters where markers overlap: a Laravel project also has a
/// `composer.json`, and a CodeIgniter 4 project also has `composer.json`, so
/// detection checks the most specific marker first. The detector lives in
/// `project::stack` (Batch 5); this enum is only the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "snake_case")]
pub enum StackKind {
    Laravel,
    CodeIgniter3,
    CodeIgniter4,
    Rust,
    Go,
    Django,
    FastApi,
    Python,
    NextJs,
    Vite,
    Node,
    DockerStack,
    Generic,
}

/// Git state of a project, read without shelling out where possible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    /// `None` when the directory is not a repository. Distinct from a repo
    /// sitting on a detached HEAD, which reports `detached:<short sha>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    pub modified: u32,
    pub untracked: u32,
}

impl GitInfo {
    #[must_use]
    pub fn none() -> Self {
        Self {
            branch: None,
            modified: 0,
            untracked: 0,
        }
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.modified > 0 || self.untracked > 0
    }
}

/// One project card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub name: String,
    /// Parent folder under the workspace root, used as the filter pill.
    pub category: String,
    /// Directory the user picked, as displayed.
    pub path: String,
    /// Directory commands actually run in. Differs from `path` when the real
    /// project sits one level down, e.g. a repo with `api/` and `web/`.
    pub working_path: String,
    pub stack: StackKind,
    /// Framework version, e.g. `v11.9.2`. Absent when it cannot be determined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub framework_version: Option<String>,
    /// Resolved language runtime for this project, e.g. `PHP 8.2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runtime_version: Option<String>,
    /// Command the Run button executes, e.g. `php artisan serve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dev_command: Option<String>,
    /// Port checked to decide whether the dev server is already up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_port: Option<u16>,
    pub git: GitInfo,
    pub port_open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_info_none_is_clean_and_branchless() {
        let git = GitInfo::none();
        assert!(!git.is_dirty());
        assert_eq!(git.branch, None);
    }

    #[test]
    fn untracked_files_alone_count_as_dirty() {
        // A repo with only new files is not clean; reporting it as clean would
        // let a user close a window over unsaved work.
        let git = GitInfo {
            branch: Some("main".into()),
            modified: 0,
            untracked: 3,
        };
        assert!(git.is_dirty());
    }

    #[test]
    fn summary_serializes_camel_case_for_the_ui() {
        let summary = ProjectSummary {
            name: "shop".into(),
            category: "client".into(),
            path: "/projects/client/shop".into(),
            working_path: "/projects/client/shop/api".into(),
            stack: StackKind::Laravel,
            framework_version: Some("v11.9.2".into()),
            runtime_version: None,
            dev_command: Some("php artisan serve".into()),
            default_port: Some(8000),
            git: GitInfo::none(),
            port_open: false,
        };

        let json = serde_json::to_string(&summary).expect("serializes");
        assert!(json.contains(r#""workingPath""#));
        assert!(json.contains(r#""stack":"laravel""#));
        // Absent optionals are omitted rather than sent as null, so the
        // TypeScript side sees `undefined` consistently.
        assert!(!json.contains("runtimeVersion"));
    }
}
