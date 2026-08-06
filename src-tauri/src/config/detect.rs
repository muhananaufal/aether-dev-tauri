//! Toolchain and workspace detection.
//!
//! This is what makes a settings screen usable rather than a wall of empty
//! fields nobody knows how to fill. Nobody memorises where nvm keeps its
//! versions; the app finds it and the user corrects it if the guess is wrong.
//!
//! Detection never overrides a stored choice — see the layering note in the
//! module above. It only supplies starting values.
//!
//! Everything that can be pure is pure: candidate paths and version parsing are
//! separated from the filesystem checks so they can be tested without staging
//! a PHP installation on the test machine.

use std::path::{Path, PathBuf};

use super::{EditorEntry, Settings, ShellPreference, ToolchainSettings, WorkspaceSettings};

/// Extracts `major.minor` from a PHP installation directory name.
///
/// Handles the shapes actually seen in the wild: `php-8.2.10-Win32-vs16-x64`
/// (windows.php.net), `php8.2` (Laragon, Homebrew), `8.2.10` (asdf).
#[must_use]
pub fn parse_php_version(dir_name: &str) -> Option<String> {
    let digits: String = dir_name
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let mut parts = digits.split('.').filter(|p| !p.is_empty());
    let major = parts.next()?;
    let minor = parts.next().unwrap_or("0");

    // A bare "8" is ambiguous between 8.0 and "the eighth thing"; require that
    // the major looks like a PHP major version to avoid matching stray numbers.
    if !matches!(major, "5" | "7" | "8" | "9") {
        return None;
    }

    Some(format!("{major}.{minor}"))
}

/// Extracts a Node version from an nvm directory name, e.g. `v20.11.0`.
#[must_use]
pub fn parse_node_version(dir_name: &str) -> Option<String> {
    let trimmed = dir_name.trim_start_matches('v');
    let mut parts = trimmed.split('.');
    let major = parts.next()?;
    if major.is_empty() || !major.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Splits a `PATH` value into directories.
#[must_use]
pub fn split_path_var(value: &str) -> Vec<PathBuf> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    value
        .split(separator)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Whether `program` is resolvable on `PATH`.
#[must_use]
pub fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_owned())
            .split(';')
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };

    for dir in split_path_var(&path.to_string_lossy()) {
        for extension in &extensions {
            if dir.join(format!("{program}{extension}")).is_file() {
                return true;
            }
        }
    }
    false
}

fn home() -> Option<PathBuf> {
    directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

/// Directories that commonly hold versioned PHP installations.
///
/// Returned whether or not they exist; the caller filters. Keeping the list
/// pure makes it reviewable and testable.
#[must_use]
pub fn php_candidate_dirs() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\laragon\bin\php"));
        candidates.push(PathBuf::from(r"C:\ProgramData\laragon\bin\php"));
        candidates.push(PathBuf::from(r"C:\tools\php"));
        candidates.push(PathBuf::from(r"C:\php"));
        if let Some(local) = env_path("LOCALAPPDATA") {
            candidates.push(local.join("Herd").join("bin"));
        }
    } else {
        // Homebrew keeps kegs per version; asdf and Herd use a home directory.
        candidates.push(PathBuf::from("/opt/homebrew/opt"));
        candidates.push(PathBuf::from("/usr/local/opt"));
        candidates.push(PathBuf::from("/usr/lib/php"));
        if let Some(home) = home() {
            candidates.push(home.join(".asdf/installs/php"));
            candidates.push(home.join("Library/Application Support/Herd/bin"));
        }
    }

    candidates
}

/// PHP search paths that actually exist on this machine.
#[must_use]
pub fn detect_php_search_paths() -> Vec<PathBuf> {
    php_candidate_dirs()
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect()
}

/// A PHP installation found under a search path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpInstall {
    /// `major.minor`, e.g. `8.2`.
    pub version: String,
    /// Directory containing the executable.
    pub bin_dir: PathBuf,
}

/// Scans one directory level below each search path for PHP installations.
#[must_use]
pub fn detect_php_installs(search_paths: &[PathBuf]) -> Vec<PhpInstall> {
    let executable = if cfg!(windows) { "php.exe" } else { "php" };
    let mut found: Vec<PhpInstall> = Vec::new();

    for base in search_paths {
        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(version) = parse_php_version(&name) else {
                continue;
            };

            // Homebrew puts the binary in a `bin` subdirectory; Windows builds
            // put it beside the directory root.
            let bin_dir = if dir.join("bin").join(executable).is_file() {
                dir.join("bin")
            } else if dir.join(executable).is_file() {
                dir.clone()
            } else {
                continue;
            };

            if !found.iter().any(|i| i.version == version) {
                found.push(PhpInstall { version, bin_dir });
            }
        }
    }

    found.sort_by(|a, b| a.version.cmp(&b.version));
    found
}

/// Locates the nvm installation directory.
#[must_use]
pub fn detect_nvm_dir() -> Option<PathBuf> {
    if let Some(explicit) = env_path("NVM_HOME").or_else(|| env_path("NVM_DIR")) {
        if explicit.is_dir() {
            return Some(explicit);
        }
    }

    let mut candidates = Vec::new();
    if cfg!(windows) {
        if let Some(local) = env_path("LOCALAPPDATA") {
            candidates.push(local.join("nvm"));
        }
        if let Some(roaming) = env_path("APPDATA") {
            candidates.push(roaming.join("nvm"));
        }
    }
    if let Some(home) = home() {
        candidates.push(home.join(".nvm/versions/node"));
        candidates.push(home.join(".nvm"));
        candidates.push(home.join(".asdf/installs/nodejs"));
    }

    candidates.into_iter().find(|dir| dir.is_dir())
}

/// Locates a Go SDK root.
#[must_use]
pub fn detect_go_root() -> Option<PathBuf> {
    if let Some(explicit) = env_path("GOROOT") {
        if explicit.is_dir() {
            return Some(explicit);
        }
    }

    let executable = if cfg!(windows) { "go.exe" } else { "go" };
    let mut candidates = Vec::new();
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Program Files\Go"));
        candidates.push(PathBuf::from(r"C:\Go"));
    } else {
        candidates.push(PathBuf::from("/usr/local/go"));
        candidates.push(PathBuf::from("/opt/homebrew/opt/go/libexec"));
        candidates.push(PathBuf::from("/usr/lib/go"));
    }

    candidates
        .into_iter()
        .find(|dir| dir.join("bin").join(executable).is_file())
}

/// Locates Git Bash, which backs the isolated project shell on Windows.
#[must_use]
pub fn detect_git_bash() -> Option<PathBuf> {
    if !cfg!(windows) {
        // Elsewhere the system shell is already POSIX; no bridge is needed.
        return ["/bin/bash", "/usr/bin/bash", "/opt/homebrew/bin/bash"]
            .iter()
            .map(PathBuf::from)
            .find(|path| path.is_file());
    }

    let mut candidates = vec![
        PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
        PathBuf::from(r"C:\Program Files (x86)\Git\bin\bash.exe"),
    ];
    if let Some(local) = env_path("LOCALAPPDATA") {
        candidates.push(local.join(r"Programs\Git\bin\bash.exe"));
    }

    candidates.into_iter().find(|path| path.is_file())
}

/// Picks a terminal emulator.
#[must_use]
pub fn detect_terminal() -> Option<String> {
    let candidates: &[&str] = if cfg!(windows) {
        &["wt.exe"]
    } else if cfg!(target_os = "macos") {
        &[]
    } else {
        &[
            "wezterm",
            "alacritty",
            "kitty",
            "gnome-terminal",
            "konsole",
            "xterm",
        ]
    };

    candidates
        .iter()
        .find(|program| on_path(program))
        .map(|program| (*program).to_owned())
}

/// Editors found on `PATH`, in the order they should appear as buttons.
#[must_use]
pub fn detect_editors() -> Vec<EditorEntry> {
    // Label, launcher command. Ordered by how likely a user is to want it as
    // the primary button.
    const KNOWN: &[(&str, &str)] = &[
        ("VS Code", "code"),
        ("Cursor", "cursor"),
        ("Windsurf", "windsurf"),
        ("Zed", "zed"),
        ("Sublime", "subl"),
        ("IntelliJ", "idea"),
        ("Neovim", "nvim"),
    ];

    KNOWN
        .iter()
        .filter(|(_, program)| on_path(program))
        .map(|(label, program)| EditorEntry {
            label: (*label).to_owned(),
            program: (*program).to_owned(),
            args: vec!["{path}".to_owned()],
        })
        .collect()
}

/// Plausible workspace roots. Only directories that exist are returned.
#[must_use]
pub fn detect_workspace_roots() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // The predecessor's fixed location, so an existing user is recognised.
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Projects"));
    }
    if let Some(home) = home() {
        for name in [
            "Projects",
            "projects",
            "Code",
            "code",
            "dev",
            "Developer",
            "src",
            "repos",
        ] {
            candidates.push(home.join(name));
        }
    }

    let mut roots: Vec<PathBuf> = candidates.into_iter().filter(|dir| dir.is_dir()).collect();
    roots.dedup();
    roots.truncate(1); // Offer one starting point; the user adds more.
    roots
}

/// Whether a directory looks like the compose stack directory.
#[must_use]
pub fn is_compose_dir(dir: &Path) -> bool {
    dir.join("docker-compose.yml").is_file() || dir.join("compose.yaml").is_file()
}

/// Builds a [`Settings`] from what this machine actually has.
///
/// Used as the detection layer beneath `settings.toml`, and by the "re-detect"
/// button in the settings screen.
#[must_use]
pub fn detect() -> Settings {
    Settings {
        workspace: WorkspaceSettings {
            roots: detect_workspace_roots(),
            ..WorkspaceSettings::default()
        },
        toolchain: ToolchainSettings {
            git_bash: detect_git_bash(),
            preferred_shell: ShellPreference::Auto,
            terminal: detect_terminal(),
            php_search_paths: detect_php_search_paths(),
            node_nvm_path: detect_nvm_dir(),
            go_root: detect_go_root(),
        },
        editors: detect_editors(),
        ..Settings::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_php_versions_from_real_directory_shapes() {
        assert_eq!(
            parse_php_version("php-8.2.10-Win32-vs16-x64"),
            Some("8.2".into())
        );
        assert_eq!(parse_php_version("php8.2"), Some("8.2".into()));
        assert_eq!(parse_php_version("8.3.1"), Some("8.3".into()));
        assert_eq!(parse_php_version("php@8.1"), Some("8.1".into()));
        assert_eq!(parse_php_version("php-7.4.33"), Some("7.4".into()));
    }

    #[test]
    fn ignores_directories_that_are_not_php_versions() {
        assert_eq!(parse_php_version("composer"), None);
        assert_eq!(parse_php_version("ext"), None);
        // A stray number that is not a plausible PHP major must not match, or
        // every `bin`-adjacent folder becomes a fake PHP installation.
        assert_eq!(parse_php_version("node-22.1.0"), None);
        assert_eq!(parse_php_version("v1.2.3"), None);
    }

    #[test]
    fn parses_node_versions_with_and_without_the_v_prefix() {
        assert_eq!(parse_node_version("v20.11.0"), Some("20.11.0".into()));
        assert_eq!(parse_node_version("22.1.0"), Some("22.1.0".into()));
        assert_eq!(parse_node_version("lts"), None);
        assert_eq!(parse_node_version(""), None);
    }

    #[test]
    fn splits_path_with_the_platform_separator() {
        #[cfg(windows)]
        {
            let dirs = split_path_var(r"C:\bin;;D:\tools\bin");
            assert_eq!(
                dirs,
                vec![PathBuf::from(r"C:\bin"), PathBuf::from(r"D:\tools\bin")]
            );
        }
        #[cfg(not(windows))]
        {
            let dirs = split_path_var("/usr/bin::/usr/local/bin");
            assert_eq!(
                dirs,
                vec![PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")]
            );
        }
    }

    #[test]
    fn php_candidates_are_offered_before_they_are_filtered() {
        // The pure list must not depend on this machine having PHP installed,
        // otherwise the function is untestable anywhere but a dev laptop.
        assert!(!php_candidate_dirs().is_empty());
    }

    #[test]
    fn detected_php_installs_are_deduplicated_and_sorted() {
        let temp = tempfile::tempdir().expect("temp dir");
        let base = temp.path();
        let executable = if cfg!(windows) { "php.exe" } else { "php" };

        for name in ["php-8.2.10-Win32", "php8.1", "not-php"] {
            let dir = base.join(name);
            std::fs::create_dir_all(&dir).expect("create");
            std::fs::write(dir.join(executable), b"").expect("write");
        }

        let installs = detect_php_installs(&[base.to_path_buf()]);
        let versions: Vec<&str> = installs.iter().map(|i| i.version.as_str()).collect();
        assert_eq!(versions, vec!["8.1", "8.2"]);
    }

    #[test]
    fn php_directory_without_an_executable_is_skipped() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(temp.path().join("php8.2")).expect("create");

        assert!(detect_php_installs(&[temp.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn compose_dir_recognises_both_filenames() {
        let temp = tempfile::tempdir().expect("temp dir");
        assert!(!is_compose_dir(temp.path()));

        std::fs::write(temp.path().join("compose.yaml"), b"").expect("write");
        assert!(is_compose_dir(temp.path()));
    }

    #[test]
    fn detected_editors_all_carry_the_path_placeholder() {
        // Whatever is installed on the machine running this, every entry must
        // be launchable — an editor button that opens nothing is worse than no
        // button at all.
        for editor in detect_editors() {
            assert!(
                editor.args.iter().any(|a| a.contains("{path}")),
                "`{}` has no {{path}}",
                editor.label
            );
        }
    }

    #[test]
    fn detect_produces_settings_that_are_at_worst_incomplete() {
        // Detection must never produce settings that fail their own validation
        // for a reason other than "nothing was found yet".
        let detected = detect();
        for issue in detected.validate() {
            assert_eq!(
                issue.field, "workspace.roots",
                "unexpected issue from detection: {issue:?}"
            );
        }
    }
}
