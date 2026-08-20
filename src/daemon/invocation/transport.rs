// EasyNet CLI — canonical Invocation transport policy
// ===================================================
//
// File: src/daemon/invocation/transport.rs
// Description: Owns symmetric gRPC envelope limits and the canonical
//              Invocation client constructor used by every daemon route.
//
// A transport limit is not an ability payload policy. Abilities still own
// pagination and chunking; this module only guarantees that every generated
// Invocation client uses the same bounded envelope contract as the server.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[cfg(feature = "axon-pb")]
use axon_sdk::pb::axon::v1::invocation_client::InvocationClient;
#[cfg(feature = "axon-pb")]
use tonic::transport::Channel;

/// Symmetric gRPC envelope cap for every Invocation server and client.
///
/// tonic defaults to 4 MiB, which is too small for bounded runtime envelopes
/// that include self-contained signed receipts. 64 MiB remains a transport
/// safety limit; list/read-model abilities must impose substantially smaller
/// application-level byte budgets.
pub(crate) const MAX_INVOCATION_GRPC_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// Construct an Invocation client with the canonical symmetric envelope cap.
#[cfg(feature = "axon-pb")]
pub(crate) fn invocation_client(channel: Channel) -> InvocationClient<Channel> {
    InvocationClient::new(channel)
        .max_decoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
}
