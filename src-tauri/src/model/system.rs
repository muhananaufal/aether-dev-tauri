//! Host system facts: listening ports and the detected environment.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::exec::{Capabilities, TransportInfo};

/// One listening TCP socket and the process behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct PortEntry {
    pub port: u16,
    pub pid: u32,
    pub process: String,
    pub local_address: String,
    /// System-critical process the app refuses to kill. The guard is enforced
    /// in Rust, not by hiding the button — a webview must not be the only thing
    /// standing between a user and terminating `lsass.exe`.
    pub protected: bool,
}

/// What was found on this machine at startup.
///
/// Every field is deliberately optional or defaulted: the app opens and stays
/// usable on a machine with no Docker at all, routing the user to Settings
/// instead of failing to launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/bindings/")]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentReport {
    /// `None` means no container runtime was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transport: Option<TransportInfo>,
    pub capabilities: Capabilities,
    /// Directory holding `docker-compose.yml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub compose_dir: Option<String>,
    /// Non-fatal findings worth surfacing, e.g. "found docker but not the
    /// compose v2 plugin". Shown as a banner, never as a modal.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl EnvironmentReport {
    /// The report produced when nothing usable was found.
    #[must_use]
    pub fn unavailable(warnings: Vec<String>) -> Self {
        Self {
            transport: None,
            capabilities: Capabilities::default(),
            compose_dir: None,
            warnings,
        }
    }

    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.transport.is_some() && self.capabilities.compose_v2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_report_is_not_usable() {
        let report = EnvironmentReport::unavailable(vec!["docker not found".into()]);
        assert!(!report.is_usable());
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn docker_without_compose_v2_is_not_usable() {
        // Compose v1 is out of scope per the accepted RFC. Finding a docker
        // binary is not enough to call the environment ready.
        let report = EnvironmentReport {
            transport: Some(TransportInfo::Native {
                docker_version: "28.1.0".into(),
            }),
            capabilities: Capabilities {
                compose_v2: false,
                ..Capabilities::default()
            },
            compose_dir: Some("/opt/dcc".into()),
            warnings: vec![],
        };
        assert!(!report.is_usable());
    }

    #[test]
    fn transport_info_tags_its_variant_for_the_ui() {
        let info = TransportInfo::Wsl {
            distro: "Ubuntu".into(),
            user: None,
            docker_version: "28.1.0".into(),
        };
        let json = serde_json::to_string(&info).expect("serializes");
        assert!(json.contains(r#""kind":"wsl""#));
        assert!(json.contains(r#""distro":"Ubuntu""#));
    }

    #[test]
    fn protected_flag_travels_with_the_port_entry() {
        let entry = PortEntry {
            port: 445,
            pid: 4,
            process: "System".into(),
            local_address: "0.0.0.0".into(),
            protected: true,
        };
        let json = serde_json::to_string(&entry).expect("serializes");
        assert!(json.contains(r#""protected":true"#));
        assert!(json.contains(r#""localAddress""#));
    }
}
