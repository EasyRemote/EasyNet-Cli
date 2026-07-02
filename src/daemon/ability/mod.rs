// EasyNet CLI - daemon ability services
// ======================================
//
// File: src/daemon/ability/mod.rs
// Description: Daemon-owned ability control-plane models, catalog
//              projection, wire lookup, built-in handlers, and
//              operator-facing ability metadata.
//
// Axon owns canonical Invocation/Receipt wire semantics. This module owns
// EasyNet-Cli's daemon-local projection of governed descriptors, authority
// bindings, implementation bindings, and the services that publish or enrich
// ability surfaces.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod authority;
pub mod builtins;
pub mod catalog;
pub mod conformance;
mod control_plane;
mod control_plane_error;
pub mod descriptors;
pub mod health;
pub mod impl_bindings;
pub mod names;
pub mod wire;

pub(crate) use authority::public_route_ability_from_descriptor_ref;
pub use authority::{
    AuthorityBindingKind, AuthorityBindingRecord, AuthorityBindingRegistry, AuthorityPredicate,
    AuthorityScope, HostedAgentAuthority, HostedAgentDelegationClaims,
    HostedAgentDelegationContext, HostedAgentDelegationEnvelopeBinding,
    HostedAgentDelegationRequest, HOSTED_AGENT_DELEGATION_METADATA_KEY,
    HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY,
};
pub use control_plane::{
    AbilityControlPlaneAuthorityModeLookupError, AbilityControlPlaneLookupError,
    AbilityControlPlaneLookupMatch, AbilityControlPlaneRecord, AbilityControlPlaneRegistration,
    AbilityControlPlaneRegistry,
};
pub use control_plane_error::AbilityControlPlaneError;
pub use descriptors::{
    canonical_json_bytes, AbilityControlPlaneKey, AbilityDescriptorKey, AbilityDescriptorRecord,
    AbilityDescriptorRegistry, AbilityDescriptorVersion, CallMode, DescriptorHash, SchemaHash,
    DEFAULT_ABILITY_DESCRIPTOR_VERSION,
};
pub use impl_bindings::{AbilityImplBinding, AbilityImplRegistry, AbilityImplSource, RuntimeEnv};
