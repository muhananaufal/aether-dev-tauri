//! Loading, saving and migrating settings.

use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Serialized, Toml};
use figment::Figment;
use rand::Rng;

use super::{detect, Settings, SCHEMA_VERSION};
use crate::error::{AppError, AppResult};

/// Where the application keeps its files on this platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub settings_file: PathBuf,
    /// User override for the bundled service registry. Absent by default.
    pub services_file: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
}

/// Resolves the per-platform directories.
///
/// Fails only when the OS reports no home directory at all, which happens in
/// some service accounts and sandboxes.
pub fn config_dir() -> AppResult<ConfigPaths> {
    let dirs = directories::ProjectDirs::from("dev", "controlcenter", "DevControlCenter")
        .ok_or_else(|| AppError::ConfigInvalid {
            field: "configDir".to_owned(),
            reason: "the operating system reported no home directory".to_owned(),
        })?;

    let config = dirs.config_dir().to_path_buf();
    let data = dirs.data_dir().to_path_buf();

    Ok(ConfigPaths {
        settings_file: config.join("settings.toml"),
        services_file: config.join("services.json"),
        log_dir: data.join("logs"),
        config_dir: config,
        data_dir: data,
    })
}

/// Loads settings, layering defaults, detection, the file and the environment.
///
/// A missing file is normal — it means a fresh install, and the result is the
/// detected settings. A malformed file is not: the error names the field and
/// the reason, and the caller shows it rather than silently starting from
/// defaults and losing the user's configuration.
pub fn load(settings_file: &Path) -> AppResult<Settings> {
    let stored = read_and_migrate(settings_file)?;

    let mut figment = Figment::from(Serialized::defaults(Settings::default()))
        .merge(Serialized::defaults(detect::detect()));

    if let Some(text) = stored {
        figment = figment.merge(Toml::string(&text));
    }

    // `DCC_BEHAVIOR__REFRESH_INTERVAL_SECS=5` reaches
    // `behavior.refresh_interval_secs`. Used by tests and scripts; not a
    // documented user-facing surface.
    figment = figment.merge(Env::prefixed("DCC_").split("__"));

    figment.extract().map_err(|err| {
        let field = err
            .path
            .first()
            .cloned()
            .unwrap_or_else(|| "settings".to_owned());
        AppError::ConfigInvalid {
            field,
            reason: err.to_string(),
        }
    })
}

/// Reads the settings file and brings it up to the current schema version.
///
/// Returns `None` when the file does not exist.
fn read_and_migrate(settings_file: &Path) -> AppResult<Option<String>> {
    if !settings_file.is_file() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(settings_file).map_err(|err| AppError::ConfigInvalid {
        field: "settings".to_owned(),
        reason: format!("could not read {}: {err}", settings_file.display()),
    })?;

    let value: toml::Value = toml::from_str(&text).map_err(|err| AppError::ConfigInvalid {
        field: "settings".to_owned(),
        reason: format!("{} is not valid TOML: {err}", settings_file.display()),
    })?;

    let version = value
        .get("schemaVersion")
        .or_else(|| value.get("schema_version"))
        .and_then(toml::Value::as_integer)
        .unwrap_or(i64::from(SCHEMA_VERSION)) as u32;

    if version > SCHEMA_VERSION {
        // Downgrading the app must not silently rewrite a newer file into
        // something the newer version can no longer read.
        return Err(AppError::ConfigInvalid {
            field: "schemaVersion".to_owned(),
            reason: format!(
                "settings were written by a newer version (schema {version}, this build \
                 understands {SCHEMA_VERSION}). Update the app, or move the file aside."
            ),
        });
    }

    if version < SCHEMA_VERSION {
        // Keep the original before rewriting: a migration bug must not be the
        // last thing that ever touched the user's configuration.
        let backup = settings_file.with_extension("toml.bak");
        let _ = std::fs::copy(settings_file, &backup);
        let migrated = migrate(value, version)?;
        let text = toml::to_string_pretty(&migrated).map_err(|err| AppError::ConfigInvalid {
            field: "settings".to_owned(),
            reason: format!("migration produced invalid TOML: {err}"),
        })?;
        let _ = std::fs::write(settings_file, &text);
        return Ok(Some(text));
    }

    Ok(Some(text))
}

/// Applies migrations from `from` up to [`SCHEMA_VERSION`].
///
/// Each step is a pure function on the parsed document, so it can be tested
/// without touching disk.
pub fn migrate(mut value: toml::Value, from: u32) -> AppResult<toml::Value> {
    // Schema 1 is the first version, so no steps exist yet. When schema 2
    // arrives this becomes a chain of pure functions, each taking the document
    // from one version to the next:
    //
    //     if from < 2 { migrate_v1_to_v2(&mut value)?; }
    //     if from < 3 { migrate_v2_to_v3(&mut value)?; }
    //
    // Until then, anything claiming an older schema is a file this build has
    // never written and cannot interpret. Refusing beats guessing.
    if from < SCHEMA_VERSION {
        return Err(AppError::ConfigInvalid {
            field: "schemaVersion".to_owned(),
            reason: format!(
                "settings declare schema {from}, and no migration to {SCHEMA_VERSION} exists. \
                 Move the file aside to start from defaults."
            ),
        });
    }

    if let Some(table) = value.as_table_mut() {
        table.insert(
            "schemaVersion".to_owned(),
            toml::Value::Integer(i64::from(SCHEMA_VERSION)),
        );
    }

    Ok(value)
}

/// Writes settings, creating the directory if needed.
pub fn save(settings_file: &Path, settings: &Settings) -> AppResult<()> {
    if let Some(parent) = settings_file.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AppError::ConfigInvalid {
            field: "configDir".to_owned(),
            reason: format!("could not create {}: {err}", parent.display()),
        })?;
    }

    let text = toml::to_string_pretty(settings).map_err(|err| AppError::ConfigInvalid {
        field: "settings".to_owned(),
        reason: format!("could not serialize settings: {err}"),
    })?;

    std::fs::write(settings_file, text).map_err(|err| AppError::ConfigInvalid {
        field: "settings".to_owned(),
        reason: format!("could not write {}: {err}", settings_file.display()),
    })
}

/// Characters used for generated credentials.
///
/// Alphanumeric only, deliberately. Punctuation in a password is fine until it
/// reaches a connection string, a YAML file and a shell in the same afternoon.
const CREDENTIAL_ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Generates a random credential.
#[must_use]
pub fn generate_credential(length: usize) -> String {
    let mut rng = rand::rng();
    (0..length)
        .map(|_| {
            let index = rng.random_range(0..CREDENTIAL_ALPHABET.len());
            CREDENTIAL_ALPHABET[index] as char
        })
        .collect()
}

/// Builds the contents of the stack `.env` file with fresh credentials.
///
/// The predecessor shipped `secret_mysql_password` as the working default in
/// twenty-one places and committed the resulting `.env`. Every install here
/// gets its own credentials, and the file is gitignored.
#[must_use]
pub fn render_env_file() -> String {
    let mysql_root = generate_credential(24);
    let mysql_user_password = generate_credential(24);
    let postgres = generate_credential(24);
    let mongo = generate_credential(24);
    let redis = generate_credential(24);
    let minio = generate_credential(24);
    let rabbit = generate_credential(24);

    format!(
        "# Credentials for the local development stack.\n\
         # Generated on first run; never commit this file.\n\
         # Delete it and restart the app to roll every credential.\n\
         \n\
         MYSQL_ROOT_PASSWORD={mysql_root}\n\
         MYSQL_DATABASE=dev_db\n\
         MYSQL_USER=dev_user\n\
         MYSQL_PASSWORD={mysql_user_password}\n\
         \n\
         POSTGRES_USER=postgres\n\
         POSTGRES_PASSWORD={postgres}\n\
         POSTGRES_DB=postgres\n\
         \n\
         MONGO_INITDB_ROOT_USERNAME=mongo_admin\n\
         MONGO_INITDB_ROOT_PASSWORD={mongo}\n\
         \n\
         REDIS_PASSWORD={redis}\n\
         \n\
         MINIO_ROOT_USER=minio_admin\n\
         MINIO_ROOT_PASSWORD={minio}\n\
         \n\
         RABBITMQ_DEFAULT_USER=rabbit_admin\n\
         RABBITMQ_DEFAULT_PASS={rabbit}\n\
         \n\
         DBGATE_PORT=19000\n"
    )
}

/// Writes the stack `.env` if it does not already exist.
///
/// Returns `true` when a file was created. Never overwrites: doing so would
/// rotate credentials out from under running containers whose volumes still
/// hold data initialised with the old ones.
pub fn ensure_env_file(env_file: &Path) -> AppResult<bool> {
    if env_file.exists() {
        return Ok(false);
    }

    if let Some(parent) = env_file.parent() {
        std::fs::create_dir_all(parent).map_err(|err| AppError::ConfigInvalid {
            field: "composeDir".to_owned(),
            reason: format!("could not create {}: {err}", parent.display()),
        })?;
    }

    std::fs::write(env_file, render_env_file()).map_err(|err| AppError::ConfigInvalid {
        field: "composeDir".to_owned(),
        reason: format!("could not write {}: {err}", env_file.display()),
    })?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::TransportPreference;

    fn temp_settings() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("settings.toml");
        (dir, file)
    }

    #[test]
    fn missing_file_loads_detected_settings_rather_than_failing() {
        let (_guard, file) = temp_settings();
        let settings = load(&file).expect("a fresh install must load");
        assert_eq!(settings.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn stored_values_win_over_detection() {
        let (_guard, file) = temp_settings();
        std::fs::write(
            &file,
            "schemaVersion = 1\n[behavior]\nrefreshIntervalSecs = 42\n",
        )
        .expect("write");

        let settings = load(&file).expect("should load");
        assert_eq!(settings.behavior.refresh_interval_secs, 42);
    }

    #[test]
    fn malformed_toml_names_the_file_instead_of_silently_defaulting() {
        // Silently falling back to defaults would look like the user's
        // configuration had been erased.
        let (_guard, file) = temp_settings();
        std::fs::write(&file, "this is not = = toml").expect("write");

        let err = load(&file).expect_err("should refuse");
        match err {
            AppError::ConfigInvalid { reason, .. } => {
                assert!(reason.contains("not valid TOML"), "{reason}")
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    #[test]
    fn a_newer_schema_is_refused_not_overwritten() {
        let (_guard, file) = temp_settings();
        std::fs::write(&file, "schemaVersion = 99\n").expect("write");

        let err = load(&file).expect_err("should refuse");
        match err {
            AppError::ConfigInvalid { field, reason } => {
                assert_eq!(field, "schemaVersion");
                assert!(reason.contains("newer version"), "{reason}");
            }
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }

        // And the file must be untouched, so downgrading is recoverable.
        let after = std::fs::read_to_string(&file).expect("still readable");
        assert!(after.contains("99"));
    }

    #[test]
    fn an_older_schema_without_a_migration_is_refused_and_backed_up() {
        let (_guard, file) = temp_settings();
        std::fs::write(&file, "schemaVersion = 0\n[behavior]\nlogBufferLines = 7\n")
            .expect("write");

        let err = load(&file).expect_err("should refuse");
        assert!(matches!(err, AppError::ConfigInvalid { .. }), "{err:?}");

        // The user's file must survive a refused migration.
        let backup = file.with_extension("toml.bak");
        assert!(
            backup.is_file(),
            "a backup must be written before any rewrite"
        );
        assert!(std::fs::read_to_string(&backup)
            .expect("read backup")
            .contains("logBufferLines = 7"));
    }

    #[test]
    fn save_then_load_round_trips() {
        let (_guard, file) = temp_settings();
        let mut settings = Settings::default();
        settings.docker.transport = TransportPreference::Wsl;
        settings.docker.wsl_distro = Some("Debian".to_owned());
        settings.workspace.roots = vec![PathBuf::from("/tmp/projects")];
        settings.behavior.kill_dev_process_names = vec!["node".to_owned()];

        save(&file, &settings).expect("should save");
        let loaded = load(&file).expect("should load");

        assert_eq!(loaded.docker.transport, TransportPreference::Wsl);
        assert_eq!(loaded.docker.wsl_distro, Some("Debian".to_owned()));
        assert_eq!(loaded.workspace.roots, vec![PathBuf::from("/tmp/projects")]);
        assert_eq!(loaded.behavior.kill_dev_process_names, vec!["node"]);
    }

    #[test]
    fn save_creates_the_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let nested = dir.path().join("a").join("b").join("settings.toml");
        save(&nested, &Settings::default()).expect("should create parents");
        assert!(nested.is_file());
    }

    #[test]
    fn generated_credentials_are_long_random_and_alphanumeric() {
        let a = generate_credential(24);
        let b = generate_credential(24);

        assert_eq!(a.len(), 24);
        assert_ne!(a, b, "two credentials must not be identical");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn env_file_contains_no_default_passwords() {
        let text = render_env_file();
        // The exact strings the predecessor shipped as working defaults.
        for leaked in [
            "secret_mysql_password",
            "secret_postgres_password",
            "secret_mongo_password",
            "secret_redis_password",
            "secret_minio_password",
            "secret_rabbit_password",
        ] {
            assert!(!text.contains(leaked), "still ships `{leaked}`");
        }
    }

    #[test]
    fn env_file_defines_every_key_the_compose_file_needs() {
        let text = render_env_file();
        for key in [
            "MYSQL_ROOT_PASSWORD",
            "MYSQL_DATABASE",
            "MYSQL_USER",
            "MYSQL_PASSWORD",
            "POSTGRES_USER",
            "POSTGRES_PASSWORD",
            "POSTGRES_DB",
            "MONGO_INITDB_ROOT_USERNAME",
            "MONGO_INITDB_ROOT_PASSWORD",
            "REDIS_PASSWORD",
            "MINIO_ROOT_USER",
            "MINIO_ROOT_PASSWORD",
            "RABBITMQ_DEFAULT_USER",
            "RABBITMQ_DEFAULT_PASS",
            "DBGATE_PORT",
        ] {
            assert!(text.contains(&format!("{key}=")), "missing `{key}`");
        }
    }

    #[test]
    fn two_generated_env_files_differ() {
        assert_ne!(render_env_file(), render_env_file());
    }

    #[test]
    fn ensure_env_file_never_overwrites_an_existing_one() {
        // Rotating credentials under a running container whose volume was
        // initialised with the old ones locks the user out of their data.
        let dir = tempfile::tempdir().expect("temp dir");
        let env_file = dir.path().join(".env");

        assert!(ensure_env_file(&env_file).expect("first call creates"));
        let first = std::fs::read_to_string(&env_file).expect("read");

        assert!(!ensure_env_file(&env_file).expect("second call is a no-op"));
        assert_eq!(std::fs::read_to_string(&env_file).expect("read"), first);
    }
}
