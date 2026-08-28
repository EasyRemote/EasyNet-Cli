//! Capability mode of the private RemoteApp media-host protocol.
//!
//! This schema is intentionally separate from target observation and from the
//! future binary session-media data plane. It carries one request and one
//! bounded capability result; daemon-owned TTL, generation and admission state
//! never cross this process boundary.

use serde::{Deserialize, Serialize};

use crate::ValidationError;

pub const SCHEMA_VERSION: u32 = 1;
pub const PROTOCOL: &str = crate::media_session::PROTOCOL;
pub const REQUEST_KIND: &str = "probe_capabilities";
pub const RESPONSE_KIND: &str = "capabilities";
const MAX_DIAGNOSTIC_BYTES: usize = 2_048;
const MAX_REASON_BYTES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub protocol: String,
    pub kind: String,
    pub process_generation: u64,
    pub request_id: u64,
}

impl Request {
    pub fn probe_capabilities(process_generation: u64, request_id: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            kind: REQUEST_KIND.to_string(),
            process_generation,
            request_id,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION
            || self.protocol != PROTOCOL
            || self.kind != REQUEST_KIND
            || self.process_generation == 0
            || self.request_id == 0
        {
            return Err(ValidationError::new(
                "unsupported media-host capability request envelope",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub schema_version: u32,
    pub protocol: String,
    pub kind: String,
    pub process_generation: u64,
    pub request_id: u64,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub capability: HostAudioCapability,
}

impl Response {
    pub fn capabilities(
        request: &Request,
        started_at_ms: u64,
        completed_at_ms: u64,
        capability: HostAudioCapability,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            kind: RESPONSE_KIND.to_string(),
            process_generation: request.process_generation,
            request_id: request.request_id,
            started_at_ms,
            completed_at_ms,
            capability,
        }
    }

    pub fn matches_request(&self, process_generation: u64, request_id: u64) -> bool {
        self.schema_version == SCHEMA_VERSION
            && self.protocol == PROTOCOL
            && self.kind == RESPONSE_KIND
            && self.process_generation == process_generation
            && self.request_id == request_id
            && self.started_at_ms > 0
            && self.completed_at_ms >= self.started_at_ms
            && self.capability.validate().is_ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostAudioCapability {
    pub compiled_supported: bool,
    pub runtime_reachable: bool,
    pub runtime_blocker: Option<String>,
    pub system_loopback: SourceReadiness,
    pub process_tree_loopback: SourceReadiness,
    pub diagnostic_detail: Option<String>,
}

impl HostAudioCapability {
    pub fn new(
        compiled_supported: bool,
        runtime_reachable: bool,
        runtime_blocker: Option<impl Into<String>>,
        system_loopback: SourceReadiness,
        process_tree_loopback: SourceReadiness,
        diagnostic_detail: Option<String>,
    ) -> Self {
        Self {
            compiled_supported,
            runtime_reachable,
            runtime_blocker: runtime_blocker.map(Into::into),
            system_loopback,
            process_tree_loopback,
            diagnostic_detail,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_string(&self.runtime_blocker, MAX_REASON_BYTES, "runtime blocker")?;
        validate_optional_string(
            &self.diagnostic_detail,
            MAX_DIAGNOSTIC_BYTES,
            "diagnostic detail",
        )?;
        self.system_loopback.validate()?;
        self.process_tree_loopback.validate()?;
        if !self.compiled_supported && self.runtime_reachable {
            return Err(ValidationError::new(
                "uncompiled host audio cannot be runtime reachable",
            ));
        }
        if self.runtime_reachable == self.runtime_blocker.is_some() {
            return Err(ValidationError::new(
                "runtime reachability and blocker disagree",
            ));
        }
        if !self.runtime_reachable
            && (self.system_loopback.ready || self.process_tree_loopback.ready)
        {
            return Err(ValidationError::new(
                "an unreachable runtime cannot expose a ready source",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReadiness {
    pub ready: bool,
    pub blocker: Option<String>,
}

impl SourceReadiness {
    pub fn ready() -> Self {
        Self {
            ready: true,
            blocker: None,
        }
    }

    pub fn blocked(reason: impl Into<String>) -> Self {
        Self {
            ready: false,
            blocker: Some(reason.into()),
        }
    }

    fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_string(&self.blocker, MAX_REASON_BYTES, "source blocker")?;
        if self.ready == self.blocker.is_some() {
            return Err(ValidationError::new(
                "source readiness and blocker disagree",
            ));
        }
        Ok(())
    }
}

fn validate_optional_string(
    value: &Option<String>,
    max_bytes: usize,
    field: &str,
) -> Result<(), ValidationError> {
    if value
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.len() > max_bytes)
    {
        return Err(ValidationError::new(format!(
            "media-host capability {field} is empty or oversized"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::{read_frame, write_frame};

    #[test]
    fn exact_request_response_round_trip_validates() {
        let request = Request::probe_capabilities(3, 7);
        request.validate().unwrap();
        let response = Response::capabilities(
            &request,
            10,
            11,
            HostAudioCapability::new(
                true,
                true,
                None::<String>,
                SourceReadiness::ready(),
                SourceReadiness::blocked("process_loopback_unavailable"),
                None,
            ),
        );
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &response).unwrap();
        let decoded: Response = read_frame(&mut Cursor::new(bytes)).unwrap().unwrap();
        assert!(decoded.matches_request(3, 7));
    }

    #[test]
    fn inconsistent_capability_is_rejected() {
        let invalid = HostAudioCapability::new(
            false,
            true,
            None::<String>,
            SourceReadiness::ready(),
            SourceReadiness::blocked("blocked"),
            None,
        );
        assert!(invalid.validate().is_err());
    }
}
