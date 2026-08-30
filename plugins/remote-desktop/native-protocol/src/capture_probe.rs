//! One-shot exact target verification and bounded diagnostic capture contract.
//!
//! Diagnostic JPEG uses bounded base64 because it is one preview frame;
//! active RemoteApp media never uses this control representation.

use serde::{Deserialize, Serialize};

use crate::media_session::{CaptureProof, FailureReason, NativeTargetPlan};
use crate::ValidationError;

pub const PROTOCOL: &str = "remoteapp_media_host_capture_probe_v1";
pub const REQUEST_KIND: &str = "capture_probe";
pub const RESPONSE_KIND: &str = "capture_probe_result";
pub const MAX_DIAGNOSTIC_JPEG_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_DIAGNOSTIC_DIMENSION: u32 = u16::MAX as u32;
const MAX_DETAIL_BYTES: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    VerifyTarget,
    DiagnosticJpeg { width: u32, height: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub protocol: String,
    pub kind: String,
    pub process_generation: u64,
    pub request_id: u64,
    pub target: NativeTargetPlan,
    pub operation: Operation,
}

impl Request {
    pub fn new(
        process_generation: u64,
        request_id: u64,
        target: NativeTargetPlan,
        operation: Operation,
    ) -> Self {
        Self {
            schema_version: crate::media_session::SCHEMA_VERSION,
            protocol: PROTOCOL.into(),
            kind: REQUEST_KIND.into(),
            process_generation,
            request_id,
            target,
            operation,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != crate::media_session::SCHEMA_VERSION
            || self.protocol != PROTOCOL
            || self.kind != REQUEST_KIND
            || self.process_generation == 0
            || self.request_id == 0
        {
            return Err(ValidationError::new(
                "unsupported capture-probe request envelope",
            ));
        }
        self.target.validate()?;
        if let Operation::DiagnosticJpeg { width, height } = self.operation {
            let pixels = u64::from(width).checked_mul(u64::from(height));
            if width == 0
                || height == 0
                || width > MAX_DIAGNOSTIC_DIMENSION
                || height > MAX_DIAGNOSTIC_DIMENSION
                || pixels.is_none_or(|pixels| pixels > 33_177_600)
            {
                return Err(ValidationError::new(
                    "invalid capture-probe diagnostic dimensions",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Verified {
        capture_proof: CaptureProof,
    },
    DiagnosticJpeg {
        capture_proof: CaptureProof,
        width: u32,
        height: u32,
        jpeg_base64: String,
    },
    Failed {
        reason: FailureReason,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub schema_version: u32,
    pub protocol: String,
    pub kind: String,
    pub process_generation: u64,
    pub request_id: u64,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub outcome: Outcome,
}

impl Response {
    pub fn new(
        request: &Request,
        started_at_ms: u64,
        completed_at_ms: u64,
        outcome: Outcome,
    ) -> Self {
        Self {
            schema_version: crate::media_session::SCHEMA_VERSION,
            protocol: PROTOCOL.into(),
            kind: RESPONSE_KIND.into(),
            process_generation: request.process_generation,
            request_id: request.request_id,
            started_at_ms,
            completed_at_ms,
            outcome,
        }
    }

    pub fn validate_for(&self, request: &Request) -> Result<(), ValidationError> {
        request.validate()?;
        if self.schema_version != crate::media_session::SCHEMA_VERSION
            || self.protocol != PROTOCOL
            || self.kind != RESPONSE_KIND
            || self.process_generation != request.process_generation
            || self.request_id != request.request_id
            || self.started_at_ms == 0
            || self.completed_at_ms < self.started_at_ms
        {
            return Err(ValidationError::new(
                "capture-probe response correlation mismatch",
            ));
        }
        match (&request.operation, &self.outcome) {
            (Operation::VerifyTarget, Outcome::Verified { capture_proof }) => {
                capture_proof.validate_for(&request.target)
            }
            (
                Operation::DiagnosticJpeg { width, height },
                Outcome::DiagnosticJpeg {
                    capture_proof,
                    width: actual_width,
                    height: actual_height,
                    jpeg_base64,
                },
            ) => {
                capture_proof.validate_for(&request.target)?;
                if actual_width != width
                    || actual_height != height
                    || jpeg_base64.is_empty()
                    || jpeg_base64.len() > encoded_len(MAX_DIAGNOSTIC_JPEG_BYTES)
                    || !jpeg_base64.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
                    })
                {
                    return Err(ValidationError::new(
                        "invalid capture-probe diagnostic payload",
                    ));
                }
                Ok(())
            }
            (_, Outcome::Failed { detail, .. })
                if !detail.is_empty() && detail.len() <= MAX_DETAIL_BYTES =>
            {
                Ok(())
            }
            _ => Err(ValidationError::new(
                "capture-probe response does not match requested operation",
            )),
        }
    }
}

const fn encoded_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_session::{CaptureBackend, TargetKind};

    fn target() -> NativeTargetPlan {
        NativeTargetPlan {
            kind: TargetKind::Display,
            display_id: Some(1),
            window_id: None,
            pid: None,
            process_instance_id: None,
            app_identity: None,
            bundle_id: None,
            application: None,
        }
    }

    #[test]
    fn response_is_bound_to_exact_request_and_target() {
        let request = Request::new(7, 9, target(), Operation::VerifyTarget);
        let response = Response::new(
            &request,
            10,
            11,
            Outcome::Verified {
                capture_proof: CaptureProof {
                    backend: CaptureBackend::ScreenCaptureKit,
                    observed_target: target(),
                    native_width: 1280,
                    native_height: 720,
                    verified_at_ms: 10,
                },
            },
        );
        response.validate_for(&request).unwrap();
        let another = Request::new(7, 10, target(), Operation::VerifyTarget);
        assert!(response.validate_for(&another).is_err());
    }

    #[test]
    fn diagnostic_dimensions_fit_the_jpeg_encoder_contract() {
        let valid = Request::new(
            7,
            9,
            target(),
            Operation::DiagnosticJpeg {
                width: MAX_DIAGNOSTIC_DIMENSION,
                height: 1,
            },
        );
        valid.validate().unwrap();

        let too_wide = Request::new(
            7,
            9,
            target(),
            Operation::DiagnosticJpeg {
                width: MAX_DIAGNOSTIC_DIMENSION + 1,
                height: 1,
            },
        );
        assert!(too_wide.validate().is_err());
    }
}
