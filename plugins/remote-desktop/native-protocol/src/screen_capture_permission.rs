//! One-shot screen-capture permission control for the canonical media host.
//!
//! The daemon invokes this private contract so macOS attributes Screen
//! Recording preflight/prompt calls to the same packaged executable that owns
//! active ScreenCaptureKit sessions. No Invocation, subject, authority or
//! session state crosses this process boundary.

use serde::{Deserialize, Serialize};

use crate::ValidationError;

pub const SCHEMA_VERSION: u32 = 1;
pub const PROTOCOL: &str = crate::media_session::PROTOCOL;
pub const REQUEST_KIND: &str = "screen_capture_permission";
pub const RESPONSE_KIND: &str = "screen_capture_permission_result";
const MAX_BACKEND_BYTES: usize = 64;
const MAX_PATH_BYTES: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Status,
    Request,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub protocol: String,
    pub kind: String,
    pub process_generation: u64,
    pub request_id: u64,
    pub operation: Operation,
}

impl Request {
    pub fn new(process_generation: u64, request_id: u64, operation: Operation) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            kind: REQUEST_KIND.to_string(),
            process_generation,
            request_id,
            operation,
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
                "unsupported media-host screen-capture permission request",
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
    pub operation: Operation,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub backend: String,
    pub requestable: bool,
    pub previously_granted: bool,
    pub requested: bool,
    pub granted: bool,
    pub executable_path: Option<String>,
}

impl Response {
    #[allow(clippy::too_many_arguments)]
    pub fn permission_result(
        request: &Request,
        started_at_ms: u64,
        completed_at_ms: u64,
        backend: impl Into<String>,
        requestable: bool,
        previously_granted: bool,
        requested: bool,
        granted: bool,
        executable_path: Option<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            kind: RESPONSE_KIND.to_string(),
            process_generation: request.process_generation,
            request_id: request.request_id,
            operation: request.operation,
            started_at_ms,
            completed_at_ms,
            backend: backend.into(),
            requestable,
            previously_granted,
            requested,
            granted,
            executable_path,
        }
    }

    pub fn matches_request(&self, request: &Request) -> bool {
        self.schema_version == SCHEMA_VERSION
            && self.protocol == PROTOCOL
            && self.kind == RESPONSE_KIND
            && self.process_generation == request.process_generation
            && self.request_id == request.request_id
            && self.operation == request.operation
            && self.started_at_ms > 0
            && self.completed_at_ms >= self.started_at_ms
            && self.validate_result().is_ok()
    }

    fn validate_result(&self) -> Result<(), ValidationError> {
        if self.backend.is_empty() || self.backend.len() > MAX_BACKEND_BYTES {
            return Err(ValidationError::new(
                "invalid screen-capture permission backend",
            ));
        }
        if self.executable_path.as_deref().is_some_and(|path| {
            path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0')
        }) {
            return Err(ValidationError::new(
                "invalid screen-capture permission executable path",
            ));
        }
        if self.previously_granted && !self.granted {
            return Err(ValidationError::new(
                "screen-capture permission regressed inside one request",
            ));
        }
        if self.requested
            && (self.operation != Operation::Request
                || !self.requestable
                || self.previously_granted)
        {
            return Err(ValidationError::new(
                "screen-capture permission request flags disagree",
            ));
        }
        if self.operation == Operation::Status
            && (self.requested || self.previously_granted != self.granted)
        {
            return Err(ValidationError::new(
                "screen-capture status response changed permission",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::{read_frame, write_frame};

    #[test]
    fn request_response_binds_operation_and_generation() {
        let request = Request::new(3, 9, Operation::Request);
        request.validate().unwrap();
        let response = Response::permission_result(
            &request,
            10,
            11,
            "screencapturekit",
            true,
            false,
            true,
            true,
            Some("/Applications/EasyNet/easynet-remoteapp-media-host".into()),
        );
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &response).unwrap();
        let decoded: Response = read_frame(&mut Cursor::new(bytes)).unwrap().unwrap();
        assert!(decoded.matches_request(&request));
        assert!(!decoded.matches_request(&Request::new(4, 9, Operation::Request)));
        assert!(!decoded.matches_request(&Request::new(3, 9, Operation::Status)));
    }

    #[test]
    fn status_cannot_claim_that_it_prompted() {
        let request = Request::new(1, 2, Operation::Status);
        let response = Response::permission_result(
            &request,
            10,
            10,
            "screencapturekit",
            true,
            false,
            true,
            false,
            None,
        );
        assert!(!response.matches_request(&request));
    }
}
