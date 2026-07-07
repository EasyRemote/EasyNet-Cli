// EasyNet CLI — SDK protocol projections
// ======================================
//
// File: src/protocol/mod.rs
// Description: Language-neutral daemon SDK JSON/DTO contract boundary.
//
// Protocol Responsibility
// -----------------------
// Own Axon-derived JSON/schema projections and typed daemon protocol DTO
// contracts consumed by Rust, C ABI, Go, Python, and future SDK facades.
//
// Implementation Approach
// -----------------------
// Keep shared carrier validation in `sdk_contract` and profile-specific DTO
// logic in focused modules. Daemon process modules may execute or serve these
// contracts, but they are not the semantic owner of the wire projection.
//
// Usage Contract
// --------------
// FFI and SDK facade code import protocol modules from this boundary. Product
// code must not bypass these helpers with hand-built carrier JSON.
//
// Architectural Position
// ----------------------
// This module is between daemon process internals and language bindings. Axon
// remains the source of protocol truth for canonical Invocation, DescriptorRef,
// URA, stream/bidi, and receipt semantics.

pub mod admin_gateway_contract;
pub mod agent_record_contract;
pub mod companion_contract;
pub mod compatibility_contract;
pub mod directory_contract;
pub mod events_contract;
pub mod host_stream_contract;
pub mod identity_contract;
pub mod mission_contract;
pub mod publication_contract;
pub mod receipt_contract;
pub mod runtime_stream_contract;
pub mod sdk_contract;
pub mod surface_contract;
pub mod wrapper_contract;
