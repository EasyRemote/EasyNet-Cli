// EasyNet CLI — Runtime-dispatch Ability Adapter
// ==============================================
//
// File: src/daemon/control/runtime_dispatch_adapter.rs
// Description: Daemon-internal adapter from the runtime-dispatch UDS
//              protocol to the daemon-hosted Axon `LocalRuntime`.
//
// Boundary
// --------
// This module is NOT a public `control.sock` ability surface. Public
// product calls use daemon `Invocation` over `daemon.sock`; the JSON control
// socket is boot/status only. This adapter exists because Axon local-tool
// dispatch still needs a compact newline-delimited bridge into the daemon's
// embedded `LocalRuntime`, while preserving the already-admitted signed call.
//
// Invariants
// ----------
// 1. No `IncomingFrame` / `OutgoingFrame` JSON-control product frame is
//    constructed or interpreted here.
// 2. Invocation canonicalization, admission, receipts, and protocol
//    stream/bidi semantics remain Axon-owned. This adapter re-enters
//    Axon's externally-signed descriptor-bound ingress with the exact
//    admitted envelope; it never reconstructs a daemon-local Invocation.
// 3. The adapter holds only the runtime and resolver it needs. Kernel,
//    receipt-header, subscription, and bidi-session state are not part
//    of runtime-dispatch ownership.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use easynet_axon::invocation::{LocalRuntime, StreamingInvocationHandle};
use easynet_axon::pb::axon::v1 as pb;

use crate::daemon::axon_bridge::dispatch_shim::{
    dispatch_rpc_external_signed, external_signed_from_wire_parts, open_stream_external_signed,
};
use crate::daemon::invocation::dispatch::local_runtime_invoker::decode_json_payload;
use crate::support::async_bridge::{run_blocking, NoRuntimeFallback};

/// Opaque invocation material received from the Axon runtime-local bridge.
///
/// The bridge owns parsing its JSON carrier; this type makes the complete
/// signed tuple an atomic adapter input, so callers cannot accidentally pass
/// only a subject or a routing hint and cause the adapter to mint local state.
#[derive(Debug)]
pub(crate) struct RuntimeDispatchEnvelope {
    pub(crate) envelope: pb::Envelope,
    pub(crate) descriptor_ref: String,
    pub(crate) metadata: std::collections::HashMap<String, String>,
}

/// Daemon-internal runtime-dispatch adapter.
///
/// It is deliberately small: runtime-dispatch speaks a separate
/// newline-delimited JSON protocol, so routing through the retired
/// control-frame schema would reintroduce the product JSON surface
/// Step 6 is removing.
#[derive(Clone)]
pub struct RuntimeDispatchAdapter {
    local_runtime: Arc<LocalRuntime>,
}

impl RuntimeDispatchAdapter {
    /// Construct an adapter over the daemon's already-built
    /// `LocalRuntime`.
    ///
    /// Production daemon boot should use this constructor so the
    /// runtime-dispatch path observes the exact same handlers as the
    /// daemon Invocation transport.
    pub fn new_with_runtime(local_runtime: Arc<LocalRuntime>) -> Self {
        Self { local_runtime }
    }

    /// Execute one runtime-dispatch RPC request.
    ///
    /// The caller provides the already-parsed tool name and JSON
    /// arguments. The returned value is the raw ability result used by
    /// `runtime_dispatch.rs` to build its newline-delimited response.
    pub(crate) fn execute_runtime_dispatch(
        &self,
        context: RuntimeDispatchEnvelope,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let payload =
            serde_json::to_vec(&args).map_err(|err| format!("encode JSON payload: {err}"))?;
        let wire = external_signed_from_wire_parts(
            context.envelope,
            context.descriptor_ref,
            payload,
            context.metadata,
        )
        .map_err(|err| err.to_string())?;
        let outcome = run_blocking(
            dispatch_rpc_external_signed(&self.local_runtime, wire),
            NoRuntimeFallback::BuildCurrentThreadTokio,
        );
        match outcome.error {
            Some(error) => Err(error.to_string()),
            None => decode_json_payload(&outcome.payload_bytes),
        }
    }

    /// Execute one runtime-dispatch stream request.
    ///
    /// The live Axon streaming handle is returned to
    /// `runtime_dispatch.rs`, which owns wire-level backpressure and
    /// line framing for this internal protocol.
    pub(crate) fn execute_runtime_dispatch_stream(
        &self,
        context: RuntimeDispatchEnvelope,
        args: serde_json::Value,
    ) -> Result<StreamingInvocationHandle, String> {
        let payload =
            serde_json::to_vec(&args).map_err(|err| format!("encode JSON payload: {err}"))?;
        let wire = external_signed_from_wire_parts(
            context.envelope,
            context.descriptor_ref,
            payload,
            context.metadata,
        )
        .map_err(|err| err.to_string())?;
        run_blocking(
            open_stream_external_signed(&self.local_runtime, wire),
            NoRuntimeFallback::BuildCurrentThreadTokio,
        )
        .map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_has_no_local_routing_state() {
        let adapter = RuntimeDispatchAdapter::new_with_runtime(LocalRuntime::new());
        assert_eq!(
            std::mem::size_of_val(&adapter),
            std::mem::size_of::<Arc<LocalRuntime>>()
        );
    }
}
