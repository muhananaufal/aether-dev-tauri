//! Validation for anything user-supplied that ends up in a command.
//!
//! [`CommandSpec`](super::CommandSpec) already makes shell injection
//! structurally impossible: arguments are a vector and no shell is involved. So
//! why validate at all?
//!
//! Because "not a shell injection" is not the same as "safe". A database name
//! of `--all-databases` is a perfectly ordinary argument that changes what
//! `mysqldump` does. A container name of `-v/:/host` is a valid argument that
//! mounts the root filesystem. Argument vectors stop quoting attacks; they do
//! not stop *option* injection.
//!
//! These validators are the second layer. They run before a user-supplied
//! string is allowed anywhere near a command line.

use crate::error::AppError;

/// Longest accepted identifier. MySQL allows 64 characters for a schema name,
/// which is the tightest of the engines we drive.
const MAX_IDENTIFIER: usize = 64;

/// Validates a database, index, or container name.
///
/// Accepts `[A-Za-z_][A-Za-z0-9_-]{0,63}`. Deliberately narrower than what the
/// engines themselves permit: a name that needs quoting is a name we decline to
/// handle, which is a trade the user can work around by renaming and we cannot
/// safely work around at all.
pub fn identifier(name: &str) -> Result<&str, AppError> {
    let reject = |reason: &str| Err(AppError::InvalidIdentifier(format!("`{name}` {reason}")));

    if name.is_empty() {
        return reject("is empty");
    }
    if name.len() > MAX_IDENTIFIER {
        return reject("is longer than 64 characters");
    }

    let mut chars = name.chars();
    // Leading character decides whether the string can be mistaken for a flag.
    // `-` and `--` prefixes are the whole option-injection class.
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        Some(_) => return reject("must start with a letter or underscore"),
        None => return reject("is empty"),
    }

    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return reject("may only contain letters, digits, underscore and hyphen");
        }
    }

    Ok(name)
}

/// Validates a hostname used as a `.localhost` alias in the generated Caddyfile.
///
/// The Caddyfile is configuration we write and Caddy parses. A name containing
/// whitespace or braces would not inject a shell, but it would produce a config
/// file that either fails to load or, worse, loads as something other than what
/// the user typed.
pub fn domain(name: &str) -> Result<&str, AppError> {
    let reject = |reason: &str| Err(AppError::InvalidIdentifier(format!("`{name}` {reason}")));

    if name.is_empty() {
        return reject("is empty");
    }
    if name.len() > 253 {
        return reject("is longer than 253 characters");
    }
    if name.starts_with('.') || name.ends_with('.') {
        return reject("must not start or end with a dot");
    }
    if name.contains("..") {
        return reject("contains an empty label");
    }

    for label in name.split('.') {
        if label.is_empty() {
            return reject("contains an empty label");
        }
        if label.len() > 63 {
            return reject("has a label longer than 63 characters");
        }
        if label.starts_with('-') || label.ends_with('-') {
            return reject("has a label starting or ending with a hyphen");
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return reject("may only contain letters, digits, hyphen and dot");
        }
    }

    Ok(name)
}

/// Validates a program name or path taken from configuration.
///
/// Config files are user-editable, and `settings.toml` naming an arbitrary
/// binary is a real tampering vector (STRIDE, §3.4 of the RFC). A bare name is
/// resolved against `PATH` by the OS; anything else must be an absolute path.
/// Relative paths are refused because what they resolve to depends on the
/// working directory at the moment of the call.
pub fn program(value: &str) -> Result<&str, AppError> {
    let reject = |reason: &str| {
        Err(AppError::ConfigInvalid {
            field: "program".to_owned(),
            reason: format!("`{value}` {reason}"),
        })
    };

    if value.trim().is_empty() {
        return reject("is empty");
    }
    if value.starts_with('-') {
        return reject("must not start with a hyphen");
    }
    if value.chars().any(char::is_control) {
        return reject("contains a control character");
    }

    let has_separator = value.contains('/') || value.contains('\\');
    if !has_separator {
        return Ok(value);
    }

    let path = std::path::Path::new(value);
    if !path.is_absolute() {
        return reject("must be either a bare command name or an absolute path");
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        for name in ["mydb", "my_db", "my-db", "App1", "_internal", "a"] {
            assert!(identifier(name).is_ok(), "should accept `{name}`");
        }
    }

    #[test]
    fn rejects_the_predecessor_injection_payload() {
        // Verbatim shape that worked against the PowerShell app's
        // `mysql -e "CREATE DATABASE IF NOT EXISTS $targetDb;"`.
        let payload = "mydb'; DROP DATABASE prod; --";
        assert!(identifier(payload).is_err());
    }

    #[test]
    fn rejects_option_injection() {
        // The class argument vectors do NOT protect against: a valid argument
        // that the receiving program reads as a flag.
        for hostile in ["--all-databases", "-e", "--host=evil", "-v/:/host"] {
            assert!(
                identifier(hostile).is_err(),
                "should reject flag-shaped `{hostile}`"
            );
        }
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        for hostile in ["../etc", "a/b", "a\\b", "a b", "a$b", "a`b", "a\nb"] {
            assert!(identifier(hostile).is_err(), "should reject `{hostile}`");
        }
    }

    #[test]
    fn rejects_over_length_names() {
        assert!(identifier(&"a".repeat(64)).is_ok());
        assert!(identifier(&"a".repeat(65)).is_err());
    }

    #[test]
    fn accepts_localhost_aliases() {
        for name in ["db.localhost", "kafka.localhost", "my-app.localhost", "a"] {
            assert!(domain(name).is_ok(), "should accept `{name}`");
        }
    }

    #[test]
    fn rejects_domains_that_would_corrupt_the_caddyfile() {
        for hostile in [
            "db.localhost {",
            "db localhost",
            "db..localhost",
            ".localhost",
            "localhost.",
            "-db.localhost",
            "db-.localhost",
            "db.localhost\nevil.localhost",
        ] {
            assert!(domain(hostile).is_err(), "should reject `{hostile}`");
        }
    }

    #[test]
    fn program_accepts_bare_names_and_absolute_paths() {
        assert!(program("docker").is_ok());
        assert!(program("wsl.exe").is_ok());
        #[cfg(windows)]
        assert!(program(r"C:\Program Files\Git\bin\bash.exe").is_ok());
        #[cfg(not(windows))]
        assert!(program("/usr/bin/docker").is_ok());
    }

    #[test]
    fn program_rejects_relative_paths_and_flags() {
        // A relative path resolves against whatever the cwd happens to be,
        // which is not a property a config value should have.
        assert!(program("./docker").is_err());
        assert!(program("../bin/docker").is_err());
        assert!(program("-rf").is_err());
        assert!(program("").is_err());
        assert!(program("dock\ner").is_err());
    }

    proptest::proptest! {
        /// The invariant that matters: nothing that passes `identifier` can
        /// carry a character with meaning to a shell, a flag parser, or a path
        /// resolver. Enumerating rejections misses cases; asserting over the
        /// accepted set does not.
        #[test]
        fn accepted_identifiers_are_inert(input in ".*") {
            if super::identifier(&input).is_ok() {
                let dangerous = [
                    ' ', '\t', '\n', '\r', '\0', '\'', '"', '`', '$', '\\', '/',
                    ';', '|', '&', '<', '>', '(', ')', '{', '}', '[', ']',
                    '*', '?', '!', '~', '#', '%', '^', '=', '+', ',', ':', '.',
                ];
                for c in dangerous {
                    proptest::prop_assert!(
                        !input.contains(c),
                        "accepted `{}` contains dangerous character {:?}", input, c
                    );
                }
                proptest::prop_assert!(!input.starts_with('-'));
                proptest::prop_assert!(input.is_ascii());
                proptest::prop_assert!(!input.is_empty() && input.len() <= 64);
            }
        }

        /// Same guarantee for domains, minus the dot and hyphen which are
        /// structural there.
        #[test]
        fn accepted_domains_cannot_break_caddyfile_syntax(input in ".*") {
            if super::domain(&input).is_ok() {
                for c in [' ', '\t', '\n', '\r', '{', '}', '#', '"', '\'', '\\', '/'] {
                    proptest::prop_assert!(
                        !input.contains(c),
                        "accepted `{}` contains Caddyfile metacharacter {:?}", input, c
                    );
                }
            }
        }
    }
}
