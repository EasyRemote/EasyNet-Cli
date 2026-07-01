//! EasyNet CLI - LocalRuntime descriptor-bound request factory
//! ==========================================================
//!
//! File: src/runtime/axon_bridge/local_runtime_request.rs
//! Description: Typed boundary from daemon ingress classification to
//!              Axon's public descriptor-bound request constructors.
//!
//! Protocol Responsibility:
//! - Preserve Axon's ownership of descriptor-bound admission, signature
//!   verification, nonce replay, receipt proof normalization, and launch.
//! - Prevent EasyNet-Cli from constructing or mirroring Axon runtime-internal
//!   admission state.
//!
//! Implementation Approach:
//! - `LocalRuntimeIngress::ExternalSigned` requires a caller signature and
//!   maps only to `DescriptorBoundInvocationRequest::externally_signed`.
//! - `LocalRuntimeIngress::LocalSystem` requires a system-caller envelope and
//!   maps only to `DescriptorBoundInvocationRequest::signed` with the
//!   process-local `_system.local` capability.
//!
//! Usage Contract:
//! - Daemon policy gates classify ingress before this factory is called.
//! - This factory attaches trace and dispatch keys without changing
//!   admission semantics. Receipt proof facts are owned by Axon's
//!   registration-time proof binding, not by per-request CLI injection.
//!
//! Architectural Position:
//! - EasyNet-Cli daemon/Axon bridge. Axon SDK remains the protocol source of
//!   truth; CLI owns only product policy and request plumbing.

use std::collections::HashMap;

use easynet_axon::invocation::{
    AxonError, CallMode as AxonInvocationCallMode, CallerSignature, DescriptorBoundEnvelope,
    DescriptorBoundInvocationRequest,
};

use crate::runtime::local_invocation_identity::{
    process_local_system_identity, LOCAL_SYSTEM_AGENT_URA,
};

/// Daemon classification of one descriptor-bound ingress into Axon
/// `LocalRuntime`.
#[derive(Debug)]
pub(crate) enum LocalRuntimeIngress {
    /// External caller material that still needs Axon public admission.
    ExternalSigned {
        envelope: DescriptorBoundEnvelope,
        signature: CallerSignature,
        payload: Vec<u8>,
    },
    /// Daemon-internal call. The envelope caller must be `_system.local`;
    /// the factory signs it with the daemon synthetic system key and Axon
    /// still runs the public admission path.
    LocalSystem {
        envelope: DescriptorBoundEnvelope,
        payload: Vec<u8>,
    },
}

/// Optional execution metadata that does not decide admission semantics.
#[derive(Debug, Default)]
pub(crate) struct LocalRuntimeRequestOptions {
    trace_id: String,
    request_metadata: HashMap<String, String>,
}

impl LocalRuntimeRequestOptions {
    #[must_use]
    pub(crate) fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = trace_id.into();
        self
    }

    #[must_use]
    #[cfg(feature = "axon-pb")]
    pub(crate) fn with_request_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.request_metadata = metadata;
        self
    }
}

/// Single constructor boundary for Axon descriptor-bound requests.
pub(crate) struct LocalRuntimeRequestFactory;

impl LocalRuntimeRequestFactory {
    pub(crate) fn request_for(
        mode: AxonInvocationCallMode,
        ingress: LocalRuntimeIngress,
        options: LocalRuntimeRequestOptions,
    ) -> Result<DescriptorBoundInvocationRequest, AxonError> {
        let mut request = match ingress {
            LocalRuntimeIngress::ExternalSigned {
                envelope,
                signature,
                payload,
            } => DescriptorBoundInvocationRequest::externally_signed(
                mode, envelope, signature, payload,
            ),
            LocalRuntimeIngress::LocalSystem { envelope, payload } => {
                if envelope.envelope().caller.ura != LOCAL_SYSTEM_AGENT_URA {
                    return Err(AxonError::invalid_argument(format!(
                        "local system ingress caller must be {LOCAL_SYSTEM_AGENT_URA}, got {}",
                        envelope.envelope().caller.ura
                    )));
                }
                let system_identity = process_local_system_identity();
                DescriptorBoundInvocationRequest::signed(
                    mode,
                    envelope,
                    payload,
                    system_identity.signing_key(),
                )
            }
        };

        if !options.trace_id.is_empty() {
            request = request.with_trace_id(options.trace_id);
        }
        if !options.request_metadata.is_empty() {
            request = request.with_request_metadata(options.request_metadata);
        }
        Ok(request)
    }
}
