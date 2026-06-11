// EasyNet CLI — Join-to-connected state machine
// =============================================
//
// File: src/runtime/join_connection_state.rs
// Description: Canonical product state snapshot for the lifecycle that starts
//              at a browser pairing token and ends when the Hub admits this
//              device into PresenceRegistry.
//
// Protocol Responsibility:
// - Owns stable state codes, transition IDs, and failure codes for the local
//   EasyNet product runtime. This is not an Axon protocol enum; Axon still owns
//   invocation, admission, receipt, stream, and bidi semantics.
//
// Implementation Approach:
// - Keep the public wire shape as one small value object
//   (`JoinConnectionSnapshot`) and derive every CLI/operator view from it.
// - Persist the latest snapshot under ~/.easynet/connection-state.json so a
//   failed daemon start remains inspectable by `doctor` after the process exits.
//
// Usage Contract:
// - A caller that observes a failed transition must record a snapshot before
//   returning the error.
// - `ONLINE` is recorded only after device-mode daemon boot has completed its
//   initial `<self>.session` admission gate.
//
// Architectural Position:
// - Runtime product state object. CLI facade commands render and update it;
//   backend/frontend mirror the same contract in their own DTOs.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use crate::persistence::config::{self, Credentials, WritePermissions};
use crate::runtime::failure_codes::FailureCodeClassifier;
use crate::ura;

const SNAPSHOT_FILE: &str = "connection-state.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JoinConnectionState {
    PairingNone,
    PairingTokenPending,
    PairingTokenPreflighted,
    PairingTokenExpired,
    PairingTokenConsumed,
    DeviceValidatedJoining,
    CredentialsSaved,
    LocalTrustWired,
    RuntimeStarting,
    HubCredentialVerified,
    HubSessionEndpointReachable,
    DaemonBooting,
    SelfSessionAdmissionPending,
    ConnectedOnline,
    ConnectedSuspect,
    ConnectedDraining,
    DisconnectedRemoved,
    ConnectionUnknown,
    Failed,
}

impl JoinConnectionState {
    pub fn code(self) -> &'static str {
        match self {
            Self::PairingNone => "J000",
            Self::PairingTokenPending => "J100",
            Self::PairingTokenPreflighted => "J100",
            Self::PairingTokenExpired => "F510",
            Self::PairingTokenConsumed => "J100",
            Self::DeviceValidatedJoining => "J200",
            Self::CredentialsSaved => "J300",
            Self::LocalTrustWired => "J300",
            Self::RuntimeStarting => "J400",
            Self::HubCredentialVerified => "J300",
            Self::HubSessionEndpointReachable => "J300",
            Self::DaemonBooting => "J400",
            Self::SelfSessionAdmissionPending => "J500",
            Self::ConnectedOnline => "J800",
            Self::ConnectedSuspect => "J800",
            Self::ConnectedDraining => "J800",
            Self::DisconnectedRemoved => "F530",
            Self::ConnectionUnknown => "F530",
            Self::Failed => "F000",
        }
    }

    pub fn as_wire(self) -> &'static str {
        match self {
            Self::PairingNone => "PAIRING_NONE",
            Self::PairingTokenPending => "AUTH_TOKEN_READY",
            Self::PairingTokenPreflighted => "AUTH_TOKEN_READY",
            Self::PairingTokenExpired => "JOIN_REJECTED",
            Self::PairingTokenConsumed => "JOIN_REQUESTED",
            Self::DeviceValidatedJoining => "PAIRING_ACCEPTED",
            Self::CredentialsSaved => "HUB_PREFLIGHT",
            Self::LocalTrustWired => "HUB_PREFLIGHT",
            Self::RuntimeStarting => "DAEMON_BOOT",
            Self::HubCredentialVerified => "HUB_PREFLIGHT",
            Self::HubSessionEndpointReachable => "HUB_PREFLIGHT",
            Self::DaemonBooting => "DAEMON_BOOT",
            Self::SelfSessionAdmissionPending => "SESSION_CONNECTING",
            Self::ConnectedOnline => "FRONTEND_CONNECTED",
            Self::ConnectedSuspect => "DEGRADED",
            Self::ConnectedDraining => "OFFLINE",
            Self::DisconnectedRemoved => "OFFLINE",
            Self::ConnectionUnknown => "OFFLINE",
            Self::Failed => "FAILED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JoinTransition {
    CreatePairing,
    PreflightToken,
    ValidateToken,
    SaveCredentials,
    WireLocalTrust,
    VerifyCredential,
    ConnectSessionEndpoint,
    BootDaemon,
    OpenSelfSession,
    AdmitPresence,
    RefetchReadModel,
    RemovePresence,
}

impl JoinTransition {
    pub fn id(self) -> &'static str {
        match self {
            Self::CreatePairing => "T01_CREATE_PAIRING",
            Self::PreflightToken => "T02_PREFLIGHT_TOKEN",
            Self::ValidateToken => "T03_VALIDATE_TOKEN",
            Self::SaveCredentials => "T04_SAVE_CREDENTIALS",
            Self::WireLocalTrust => "T05_WIRE_LOCAL_TRUST",
            Self::VerifyCredential => "T06_VERIFY_CREDENTIAL",
            Self::ConnectSessionEndpoint => "T07_CONNECT_SESSION_ENDPOINT",
            Self::BootDaemon => "T08_BOOT_DAEMON",
            Self::OpenSelfSession => "T09_OPEN_SELF_SESSION",
            Self::AdmitPresence => "T10_ADMIT_PRESENCE",
            Self::RefetchReadModel => "T11_REFETCH_READ_MODEL",
            Self::RemovePresence => "T12_REMOVE_PRESENCE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JoinFailureCode {
    JoinFailedPreflight,
    JoinFailedValidate,
    StartFailedCredentialVerify,
    StartFailedSessionEndpoint,
    StartFailedSelfSessionAdmission,
    StartFailedBootStage,
    ResolveUnavailable,
}

impl JoinFailureCode {
    pub fn state_code(self) -> &'static str {
        match self {
            Self::JoinFailedPreflight => "F500",
            Self::JoinFailedValidate => "F510",
            Self::StartFailedCredentialVerify => "F520",
            Self::StartFailedSessionEndpoint => "F530",
            Self::StartFailedSelfSessionAdmission => "F550",
            Self::StartFailedBootStage => "F550",
            Self::ResolveUnavailable => "F560",
        }
    }

    pub fn as_wire(self) -> &'static str {
        match self {
            Self::JoinFailedPreflight => "JOIN_FAILED_PREFLIGHT",
            Self::JoinFailedValidate => "JOIN_FAILED_VALIDATE",
            Self::StartFailedCredentialVerify => "START_FAILED_CREDENTIAL_VERIFY",
            Self::StartFailedSessionEndpoint => "HUB_UNREACHABLE",
            Self::StartFailedSelfSessionAdmission => "DAEMON_BOOT_FAILED",
            Self::StartFailedBootStage => "DAEMON_BOOT_FAILED",
            Self::ResolveUnavailable => "RESOLVE_UNAVAILABLE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinConnectionFailure {
    pub code: String,
    pub message: String,
    pub stage: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinConnectionSnapshot {
    pub state: String,
    pub state_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupted_transition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<JoinConnectionFailure>,
    pub realm: String,
    pub node_id: String,
    pub device_ura: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub_endpoint: Option<String>,
    pub source: String,
    pub observed_at_unix_ms: i64,
}

/// The fields of a failed `JoinConnectionSnapshot`, grouped so the
/// constructor takes one value instead of eight positional arguments
/// (the realm/node_id/message/source strings are owned here, decoded
/// once from the caller's `impl Into<String>`).
pub struct JoinFailureParts {
    pub failure_code: JoinFailureCode,
    pub transition: JoinTransition,
    pub realm: String,
    pub node_id: String,
    pub hub_endpoint: Option<String>,
    pub message: String,
    pub retryable: bool,
    pub source: String,
}

impl JoinConnectionSnapshot {
    pub fn from_parts(
        state: JoinConnectionState,
        transition: Option<JoinTransition>,
        realm: impl Into<String>,
        node_id: impl Into<String>,
        hub_endpoint: Option<String>,
        source: impl Into<String>,
    ) -> Self {
        let realm = realm.into();
        let node_id = node_id.into();
        let device_ura = if realm.is_empty() || node_id.is_empty() {
            String::new()
        } else {
            ura::device_ura(&realm, &node_id)
        };
        Self {
            state: state.as_wire().to_string(),
            state_code: state.code().to_string(),
            transition_id: transition.map(|t| t.id().to_string()),
            interrupted_transition: None,
            failure: None,
            realm,
            node_id,
            device_ura,
            hub_endpoint,
            source: source.into(),
            observed_at_unix_ms: Utc::now().timestamp_millis(),
        }
    }

    pub fn from_credentials(
        state: JoinConnectionState,
        transition: Option<JoinTransition>,
        creds: &Credentials,
        source: impl Into<String>,
    ) -> Self {
        Self {
            state: state.as_wire().to_string(),
            state_code: state.code().to_string(),
            transition_id: transition.map(|t| t.id().to_string()),
            interrupted_transition: None,
            failure: None,
            realm: creds.realm.clone(),
            node_id: creds.node_id.clone(),
            device_ura: ura::device_ura(creds.realm_str(), &creds.node_id),
            hub_endpoint: Some(creds.hub_endpoint.clone()),
            source: source.into(),
            observed_at_unix_ms: Utc::now().timestamp_millis(),
        }
    }

    pub fn failed_from_credentials(
        failure_code: JoinFailureCode,
        transition: JoinTransition,
        creds: &Credentials,
        message: impl Into<String>,
        retryable: bool,
        source: impl Into<String>,
    ) -> Self {
        let message = message.into();
        let detail_code = failure_detail_code(&message, failure_code);
        Self {
            state: failure_code.as_wire().to_string(),
            state_code: failure_code.state_code().to_string(),
            transition_id: Some(transition.id().to_string()),
            interrupted_transition: Some(transition.id().to_string()),
            failure: Some(JoinConnectionFailure {
                code: detail_code,
                message,
                stage: transition.id().to_string(),
                retryable,
            }),
            realm: creds.realm.clone(),
            node_id: creds.node_id.clone(),
            device_ura: ura::device_ura(creds.realm_str(), &creds.node_id),
            hub_endpoint: Some(creds.hub_endpoint.clone()),
            source: source.into(),
            observed_at_unix_ms: Utc::now().timestamp_millis(),
        }
    }

    pub fn failed_from_parts(parts: JoinFailureParts) -> Self {
        let JoinFailureParts {
            failure_code,
            transition,
            realm,
            node_id,
            hub_endpoint,
            message,
            retryable,
            source,
        } = parts;
        let detail_code = failure_detail_code(&message, failure_code);
        let device_ura = if realm.is_empty() || node_id.is_empty() {
            String::new()
        } else {
            ura::device_ura(&realm, &node_id)
        };
        Self {
            state: failure_code.as_wire().to_string(),
            state_code: failure_code.state_code().to_string(),
            transition_id: Some(transition.id().to_string()),
            interrupted_transition: Some(transition.id().to_string()),
            failure: Some(JoinConnectionFailure {
                code: detail_code,
                message,
                stage: transition.id().to_string(),
                retryable,
            }),
            realm,
            node_id,
            device_ura,
            hub_endpoint,
            source,
            observed_at_unix_ms: Utc::now().timestamp_millis(),
        }
    }

    pub fn unpaired() -> Self {
        Self {
            state: JoinConnectionState::PairingNone.as_wire().to_string(),
            state_code: JoinConnectionState::PairingNone.code().to_string(),
            transition_id: None,
            interrupted_transition: None,
            failure: None,
            realm: String::new(),
            node_id: String::new(),
            device_ura: String::new(),
            hub_endpoint: None,
            source: "cli".to_string(),
            observed_at_unix_ms: Utc::now().timestamp_millis(),
        }
    }
}

impl fmt::Display for JoinConnectionSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} [{}]", self.state, self.state_code)?;
        if let Some(t) = &self.interrupted_transition {
            write!(f, " interrupted_transition={t}")?;
        } else if let Some(t) = &self.transition_id {
            write!(f, " transition={t}")?;
        }
        if let Some(failure) = &self.failure {
            write!(f, " reason={}", failure.code)?;
        }
        Ok(())
    }
}

pub fn snapshot_path() -> PathBuf {
    config::state_dir().join(SNAPSHOT_FILE)
}

pub fn save_snapshot(snapshot: &JoinConnectionSnapshot) -> anyhow::Result<()> {
    let dir = config::state_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(snapshot)?;
    config::atomic_write_with_permissions(&snapshot_path(), &json, WritePermissions::OwnerReadWrite)
}

pub fn load_snapshot() -> anyhow::Result<JoinConnectionSnapshot> {
    let data = std::fs::read_to_string(snapshot_path())?;
    Ok(serde_json::from_str(&data)?)
}

pub fn record_snapshot(snapshot: JoinConnectionSnapshot) {
    if let Err(err) = save_snapshot(&snapshot) {
        eprintln!("[easynet warn] could not persist connection-state snapshot: {err}");
    }
}

pub fn latest_snapshot() -> JoinConnectionSnapshot {
    if let Ok(snapshot) = load_snapshot() {
        return snapshot;
    }
    match config::load_credentials() {
        Ok(creds) => JoinConnectionSnapshot::from_credentials(
            JoinConnectionState::CredentialsSaved,
            None,
            &creds,
            "cli",
        ),
        Err(_) => JoinConnectionSnapshot::unpaired(),
    }
}

pub fn classify_boot_failure(message: &str) -> (JoinFailureCode, JoinTransition, bool) {
    let lower = message.to_ascii_lowercase();
    if message.contains("CALLER_SIGNATURE_INVALID")
        || lower.contains("<self>.session")
        || lower.contains("self session")
        || lower.contains("session admission")
    {
        (
            JoinFailureCode::StartFailedSelfSessionAdmission,
            JoinTransition::OpenSelfSession,
            false,
        )
    } else {
        (
            JoinFailureCode::StartFailedBootStage,
            JoinTransition::BootDaemon,
            true,
        )
    }
}

fn failure_detail_code(message: &str, fallback: JoinFailureCode) -> String {
    FailureCodeClassifier::classify_or(message, fallback.as_wire())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;

    fn creds() -> Credentials {
        Credentials {
            node_id: "node-1".into(),
            credential_token: "secret".into(),
            hub_endpoint: "https://127.0.0.1:50443".into(),
            realm: "localhost".into(),
            deploy_signature: String::new(),
            hub_api_base: Some("http://127.0.0.1:8080".into()),
            username: Some("alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
        }
    }

    #[test]
    fn success_snapshot_carries_stable_code_and_transition() {
        let snapshot = JoinConnectionSnapshot::from_credentials(
            JoinConnectionState::ConnectedOnline,
            Some(JoinTransition::AdmitPresence),
            &creds(),
            "test",
        );
        assert_eq!(snapshot.state, "FRONTEND_CONNECTED");
        assert_eq!(snapshot.state_code, "J800");
        assert_eq!(
            snapshot.transition_id.as_deref(),
            Some("T10_ADMIT_PRESENCE")
        );
        assert_eq!(snapshot.device_ura, "easynet:///r/localhost/device/node-1");
    }

    #[test]
    fn failed_snapshot_names_interrupted_transition() {
        let snapshot = JoinConnectionSnapshot::failed_from_credentials(
            JoinFailureCode::StartFailedSelfSessionAdmission,
            JoinTransition::OpenSelfSession,
            &creds(),
            "CALLER_SIGNATURE_INVALID",
            false,
            "test",
        );
        assert_eq!(snapshot.state_code, "F550");
        assert_eq!(
            snapshot.interrupted_transition.as_deref(),
            Some("T09_OPEN_SELF_SESSION")
        );
        assert_eq!(
            snapshot.failure.as_ref().map(|f| f.code.as_str()),
            Some("CALLER_SIGNATURE_INVALID")
        );
    }

    #[test]
    fn failed_snapshot_extracts_presence_registry_reason() {
        let snapshot = JoinConnectionSnapshot::failed_from_credentials(
            JoinFailureCode::StartFailedSelfSessionAdmission,
            JoinTransition::OpenSelfSession,
            &creds(),
            "target device is not in PresenceRegistry; the owning daemon is offline",
            false,
            "test",
        );
        assert_eq!(snapshot.state_code, "F550");
        assert_eq!(
            snapshot.failure.as_ref().map(|f| f.code.as_str()),
            Some("TARGET_NOT_IN_PRESENCE_REGISTRY")
        );
    }

    #[test]
    fn snapshot_round_trip_is_queryable_after_process_failure() {
        let _home = HomeGuard::new();
        let snapshot = JoinConnectionSnapshot::from_credentials(
            JoinConnectionState::LocalTrustWired,
            Some(JoinTransition::WireLocalTrust),
            &creds(),
            "test",
        );
        save_snapshot(&snapshot).expect("save snapshot");
        let loaded = load_snapshot().expect("load snapshot");
        assert_eq!(loaded.state_code, "J300");
        assert_eq!(
            loaded.transition_id.as_deref(),
            Some("T05_WIRE_LOCAL_TRUST")
        );
    }

    #[test]
    fn boot_failure_classifier_pins_self_session_admission() {
        let (code, transition, retryable) =
            classify_boot_failure("CALLER_SIGNATURE_INVALID: rejected <self>.session");
        assert_eq!(code, JoinFailureCode::StartFailedSelfSessionAdmission);
        assert_eq!(transition, JoinTransition::OpenSelfSession);
        assert!(!retryable);
    }

    #[test]
    fn daemon_boot_failure_preserves_dendrite_bridge_missing_code() {
        let snapshot = JoinConnectionSnapshot::failed_from_credentials(
            JoinFailureCode::StartFailedBootStage,
            JoinTransition::BootDaemon,
            &creds(),
            "bridge: dendrite bridge library not found; set EASYNET_DENDRITE_BRIDGE_LIB",
            true,
            "test",
        );
        assert_eq!(snapshot.state_code, "F550");
        assert_eq!(
            snapshot.interrupted_transition.as_deref(),
            Some("T08_BOOT_DAEMON")
        );
        assert_eq!(
            snapshot.failure.as_ref().map(|f| f.code.as_str()),
            Some("DENDRITE_BRIDGE_LIBRARY_NOT_FOUND")
        );
    }
}
