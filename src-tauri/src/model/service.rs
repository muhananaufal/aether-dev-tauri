//! The service registry and its runtime status.
//!
//! In the predecessor project these facts were hardcoded in three places that
//! had to be edited together: the service cards, the domain manager's internal
//! port map, and the log source list. They drifted. Here there is one
//! [`ServiceDefinition`] list, loaded from `services.json`, and all three
//! features read it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A database engine that supports import and export.
///
/// Services without an engine (Portainer, Caddy, Mailpit) simply omit it, and
/// the UI does not offer dump buttons for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "snake_case")]
pub enum DbEngine {
    MySql,
    Postgres,
    Mongo,
    Redis,
    Elasticsearch,
}

/// One managed service, as declared in `services.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDefinition {
    /// Compose service name, e.g. `mysql`. Used with `docker compose up`.
    pub id: String,

    /// Human label for the card, e.g. `MySQL 9.7 LTS`.
    pub label: String,

    /// Container name, e.g. `mysql-db`. Used with `docker ps` and `docker logs`.
    pub container: String,

    /// Host port probed to decide whether the service is actually serving,
    /// as opposed to merely having a running container.
    pub host_port: u16,

    /// Port inside the container, for the Caddy reverse proxy. Absent means the
    /// service is not web-reachable and gets no domain alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub internal_port: Option<u16>,

    /// Management UI, opened by the panel link on the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub panel_url: Option<String>,

    /// Suggested `.localhost` alias, pre-filled in the domain manager.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_domain: Option<String>,

    /// Enables the import/export controls when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub engine: Option<DbEngine>,

    /// Services started **and stopped** alongside this one — Elasticsearch
    /// brings up Elasticvue, Kafka brings up its UI. The predecessor expressed
    /// this as an `if/elseif` chain in the click handler.
    // Always serialized, even when empty: an omitted array and an empty array
    // would need different TypeScript types for no benefit.
    #[serde(default)]
    pub companions: Vec<String>,

    /// Services started with this one but **never stopped** with it.
    ///
    /// DbGate is shared by four databases: starting MySQL should bring the GUI
    /// up, but stopping MySQL must not take the GUI away from PostgreSQL. The
    /// predecessor got this right by accident, with `up -d $svc dbgate` on the
    /// start path and a bare `stop $svc` on the other. Making the asymmetry
    /// explicit keeps it from being "tidied up" into a bug later.
    #[serde(default)]
    pub requires: Vec<String>,

    /// Connection details shown under the label. Must never contain a
    /// credential; the UI renders secrets separately and masked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub connection_hint: Option<String>,

    /// Whether to offer this service's logs in the log source list.
    #[serde(default = "default_true")]
    pub loggable: bool,

    /// Whether this service gets its own card in the Services tab.
    ///
    /// The bundled GUI containers (DbGate, Elasticvue, Kafka UI) are `false`:
    /// they are managed through the service they accompany, but they still need
    /// registry entries so the domain manager knows their internal ports and
    /// the log picker can offer them.
    #[serde(default = "default_true")]
    pub card: bool,
}

fn default_true() -> bool {
    true
}

/// Coarse lifecycle state, derived from container presence plus a port probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    Stopped,
    /// Container is running but the port has not opened yet. Distinguishing
    /// this from `Online` is what stops the UI claiming a database is ready
    /// while it is still replaying its write-ahead log.
    Booting,
    Online,
}

/// Live status for one service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub id: String,
    pub container: String,
    pub state: ServiceState,
    /// Human-readable memory usage from `docker stats`, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub memory: Option<String>,
}

/// The whole `services.json` document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRegistry {
    /// Free-text note carried in the file for whoever opens it in an editor.
    #[serde(rename = "$comment", default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub comment: Option<String>,

    /// Bumped when the shape changes incompatibly, so an old file is reported
    /// rather than silently half-parsed.
    pub schema_version: u32,

    pub services: Vec<ServiceDefinition>,
}

/// Registry schema version this build understands.
pub const REGISTRY_SCHEMA_VERSION: u32 = 1;

impl ServiceRegistry {
    /// Checks the invariants that deserialization alone cannot express.
    ///
    /// Returns every problem found rather than the first, because a user
    /// hand-editing this file deserves the whole list in one pass.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();

        if self.schema_version != REGISTRY_SCHEMA_VERSION {
            problems.push(format!(
                "schemaVersion is {}, this build understands {}",
                self.schema_version, REGISTRY_SCHEMA_VERSION
            ));
        }

        if self.services.is_empty() {
            problems.push("registry contains no services".to_owned());
        }

        let ids: Vec<&str> = self.services.iter().map(|s| s.id.as_str()).collect();

        for (index, service) in self.services.iter().enumerate() {
            if ids.iter().filter(|id| **id == service.id).count() > 1 {
                problems.push(format!("duplicate service id `{}`", service.id));
            }

            let same_container = self
                .services
                .iter()
                .filter(|other| other.container == service.container)
                .count();
            if same_container > 1 {
                problems.push(format!("duplicate container name `{}`", service.container));
            }

            // A dangling reference would silently do nothing at runtime: the
            // user presses Start and the companion never comes up.
            for referenced in service.companions.iter().chain(service.requires.iter()) {
                if !ids.contains(&referenced.as_str()) {
                    problems.push(format!(
                        "service `{}` references unknown service `{referenced}`",
                        service.id
                    ));
                }
            }

            if service.companions.contains(&service.id) || service.requires.contains(&service.id) {
                problems.push(format!("service `{}` references itself", service.id));
            }

            if service.id.trim().is_empty() {
                problems.push(format!("service at index {index} has an empty id"));
            }
        }

        // Two services claiming one hostname would generate a Caddyfile whose
        // second block silently shadows the first.
        let domains: Vec<&str> = self
            .services
            .iter()
            .filter_map(|s| s.default_domain.as_deref())
            .collect();
        for domain in &domains {
            if domains.iter().filter(|d| *d == domain).count() > 1 {
                problems.push(format!("duplicate default domain `{domain}`"));
            }
        }

        problems.sort();
        problems.dedup();

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// Services that get a card in the Services tab.
    pub fn cards(&self) -> impl Iterator<Item = &ServiceDefinition> {
        self.services.iter().filter(|s| s.card)
    }

    pub fn find(&self, id: &str) -> Option<&ServiceDefinition> {
        self.services.iter().find(|s| s.id == id)
    }

    /// Compose service names to bring up for `id`: the service itself, its
    /// companions, and anything it requires.
    pub fn start_set(&self, id: &str) -> Vec<String> {
        let Some(service) = self.find(id) else {
            return Vec::new();
        };
        let mut set = vec![service.id.clone()];
        set.extend(service.companions.iter().cloned());
        set.extend(service.requires.iter().cloned());
        set.dedup();
        set
    }

    /// Compose service names to stop for `id`: the service and its companions,
    /// but never its requirements — those are shared with other services.
    pub fn stop_set(&self, id: &str) -> Vec<String> {
        let Some(service) = self.find(id) else {
            return Vec::new();
        };
        let mut set = vec![service.id.clone()];
        set.extend(service.companions.iter().cloned());
        set.dedup();
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry shipped inside the binary. Compiled in, so a malformed file
    /// fails the build rather than the user's first launch.
    const BUNDLED: &str = include_str!("../../resources/services.json");

    fn bundled() -> ServiceRegistry {
        serde_json::from_str(BUNDLED).expect("bundled services.json must parse")
    }

    #[test]
    fn bundled_registry_parses_and_validates() {
        let registry = bundled();
        if let Err(problems) = registry.validate() {
            panic!("bundled services.json is invalid: {problems:#?}");
        }
        assert_eq!(registry.schema_version, REGISTRY_SCHEMA_VERSION);
    }

    #[test]
    fn bundled_registry_covers_the_predecessor_service_set() {
        let registry = bundled();
        // The eleven cards the PowerShell app showed. Parity is the whole point
        // of this rewrite, so it is asserted rather than assumed.
        for id in [
            "mysql",
            "postgres",
            "mongodb",
            "redis",
            "elasticsearch",
            "mailpit",
            "minio",
            "rabbitmq",
            "kafka",
            "portainer",
            "caddy",
        ] {
            let service = registry
                .find(id)
                .unwrap_or_else(|| panic!("missing service `{id}`"));
            assert!(service.card, "`{id}` should have a card");
        }
        assert_eq!(registry.cards().count(), 11);
    }

    #[test]
    fn gui_containers_are_registered_but_cardless() {
        let registry = bundled();
        for id in ["dbgate", "elasticvue", "kafka-ui"] {
            let service = registry
                .find(id)
                .unwrap_or_else(|| panic!("missing service `{id}`"));
            assert!(!service.card, "`{id}` should not have its own card");
            // They still need an internal port so the domain manager can proxy.
            assert!(service.internal_port.is_some(), "`{id}` needs internalPort");
        }
    }

    #[test]
    fn starting_a_database_brings_up_the_shared_gui() {
        let registry = bundled();
        let set = registry.start_set("mysql");
        assert!(set.contains(&"mysql".to_owned()));
        assert!(set.contains(&"dbgate".to_owned()));
    }

    #[test]
    fn stopping_a_database_leaves_the_shared_gui_running() {
        // Stopping MySQL must not take DbGate away from PostgreSQL. This is the
        // asymmetry `requires` exists to encode.
        let registry = bundled();
        let set = registry.stop_set("mysql");
        assert_eq!(set, vec!["mysql".to_owned()]);
    }

    #[test]
    fn companions_start_and_stop_together() {
        let registry = bundled();
        assert!(registry.start_set("kafka").contains(&"kafka-ui".to_owned()));
        assert!(registry.stop_set("kafka").contains(&"kafka-ui".to_owned()));
    }

    #[test]
    fn validate_reports_dangling_references() {
        let registry = ServiceRegistry {
            comment: None,
            schema_version: REGISTRY_SCHEMA_VERSION,
            services: vec![ServiceDefinition {
                id: "mysql".into(),
                label: "MySQL".into(),
                container: "mysql-db".into(),
                host_port: 3306,
                internal_port: None,
                panel_url: None,
                default_domain: None,
                engine: None,
                companions: vec!["ghost".into()],
                requires: vec![],
                connection_hint: None,
                loggable: true,
                card: true,
            }],
        };

        let problems = registry.validate().expect_err("should be invalid");
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains("unknown service `ghost`"),
            "{problems:?}"
        );
    }

    #[test]
    fn validate_reports_duplicate_domains() {
        let make = |id: &str, container: &str| ServiceDefinition {
            id: id.into(),
            label: id.into(),
            container: container.into(),
            host_port: 80,
            internal_port: None,
            panel_url: None,
            default_domain: Some("db.localhost".into()),
            engine: None,
            companions: vec![],
            requires: vec![],
            connection_hint: None,
            loggable: true,
            card: true,
        };

        let registry = ServiceRegistry {
            comment: None,
            schema_version: REGISTRY_SCHEMA_VERSION,
            services: vec![make("a", "a-c"), make("b", "b-c")],
        };

        let problems = registry.validate().expect_err("should be invalid");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("duplicate default domain")),
            "{problems:?}"
        );
    }

    #[test]
    fn validate_rejects_a_future_schema_version() {
        let mut registry = bundled();
        registry.schema_version = REGISTRY_SCHEMA_VERSION + 1;
        let problems = registry.validate().expect_err("should be invalid");
        assert!(
            problems.iter().any(|p| p.contains("schemaVersion")),
            "{problems:?}"
        );
    }

    #[test]
    fn definition_round_trips_through_json() {
        let json = r#"{
            "id": "mysql",
            "label": "MySQL 9.7 LTS",
            "container": "mysql-db",
            "hostPort": 3306,
            "engine": "my_sql",
            "companions": [],
            "loggable": true
        }"#;

        let def: ServiceDefinition = serde_json::from_str(json).expect("should parse");
        assert_eq!(def.id, "mysql");
        assert_eq!(def.host_port, 3306);
        assert_eq!(def.engine, Some(DbEngine::MySql));
        assert_eq!(def.internal_port, None);
        assert!(def.loggable);
    }

    #[test]
    fn optional_fields_default_without_being_present() {
        let json = r#"{
            "id": "caddy",
            "label": "Caddy",
            "container": "caddy-proxy",
            "hostPort": 80
        }"#;

        let def: ServiceDefinition = serde_json::from_str(json).expect("should parse");
        assert_eq!(def.engine, None);
        assert!(def.companions.is_empty());
        // `loggable` defaults to true so a new service appears in the log
        // picker without anyone remembering to opt in.
        assert!(def.loggable);
    }

    #[test]
    fn unknown_fields_are_rejected_not_ignored() {
        // A typo in a user-edited services.json should be reported, not
        // silently dropped leaving them to wonder why nothing changed.
        let json = r#"{
            "id": "mysql",
            "label": "MySQL",
            "container": "mysql-db",
            "hostPort": 3306,
            "hostPortt": 3307
        }"#;

        let err = serde_json::from_str::<ServiceDefinition>(json)
            .expect_err("unknown field must be rejected");
        assert!(err.to_string().contains("hostPortt"), "got: {err}");
    }

    #[test]
    fn state_serializes_lowercase_for_the_ui() {
        let json = serde_json::to_string(&ServiceState::Booting).expect("serializes");
        assert_eq!(json, r#""booting""#);
    }
}
