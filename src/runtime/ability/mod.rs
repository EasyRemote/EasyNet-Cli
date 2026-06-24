// EasyNet CLI - Ability control-plane model
// =========================================
//
// File: src/runtime/ability/mod.rs
// Description: Daemon-local split between governed ability descriptors,
//              authority bindings, and executable implementation bindings.
//
// Protocol Responsibility:
//   Axon owns canonical Invocation/Receipt wire semantics. This module owns
//   the EasyNet daemon's projection of local ability registration into those
//   protocol facts without making plugins or DeviceAgent profiles the owner
//   of the governed interface.
//
// Architectural Position:
//   `ability_dispatch` remains the compatibility facade for existing
//   register(&mut catalog) call sites. It writes through to the registries
//   here so interface, governance, and execution binding no longer collapse
//   into one handler map.

pub mod authority;
pub mod descriptor;
pub mod error;
pub mod impl_binding;
pub mod registry;

pub use authority::{
    AuthorityBindingKind, AuthorityBindingRecord, AuthorityBindingRegistry, AuthorityPredicate,
    AuthorityScope, HostedAgentAuthority, HostedAgentDelegationClaims,
    HostedAgentDelegationContext, HOSTED_AGENT_DELEGATION_METADATA_KEY,
};
pub use descriptor::{
    canonical_json_bytes, AbilityDescriptorKey, AbilityDescriptorRecord, AbilityDescriptorRegistry,
    AbilityDescriptorVersion, CallMode, DescriptorHash, SchemaHash,
    DEFAULT_ABILITY_DESCRIPTOR_VERSION,
};
pub use error::AbilityControlPlaneError;
pub use impl_binding::{AbilityImplBinding, AbilityImplRegistry, AbilityImplSource, RuntimeEnv};
pub use registry::{
    AbilityControlPlaneAuthorityModeLookupError, AbilityControlPlaneLookupError,
    AbilityControlPlaneLookupMatch, AbilityControlPlaneRecord, AbilityControlPlaneRegistry,
};
