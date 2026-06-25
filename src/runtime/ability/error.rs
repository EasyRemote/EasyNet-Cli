// EasyNet CLI - Ability control-plane errors
// ==========================================
//
// File: src/runtime/ability/error.rs
// Description: Typed boundary errors for daemon-local ability descriptor,
//              authority, and implementation facts.

/// Errors raised while constructing daemon-local ability control-plane facts.
///
/// These errors deliberately describe operator-fixable domain defects instead
/// of panicking inside constructors. Static daemon registrations should never
/// hit them; deployed manifests and future federation/catalog inputs can.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AbilityControlPlaneError {
    /// Descriptor version was supplied as an empty or whitespace-only string.
    #[error("ability descriptor version must be non-empty")]
    EmptyDescriptorVersion,
    /// Descriptor version was not a numeric dot-separated protocol version.
    #[error("ability descriptor version has invalid format: {version:?}")]
    InvalidDescriptorVersion { version: String },
    /// A manifest-declared interface version disagreed with an explicit
    /// control-plane registration version.
    #[error("ability manifest descriptor_version {manifest_version:?} does not match registration descriptor_version {registration_version:?}")]
    DescriptorVersionMismatch {
        manifest_version: String,
        registration_version: String,
    },
    /// Descriptor name was supplied as an empty or whitespace-only string.
    #[error("ability descriptor name must be non-empty")]
    EmptyDescriptorName,
    /// Descriptor name contains a character or segment shape that cannot be
    /// represented as a canonical ability name.
    #[error("ability descriptor name has invalid format: {name:?}")]
    InvalidDescriptorName { name: String },
    /// Canonical descriptor Ability URA was empty.
    #[error("ability descriptor URA must be non-empty")]
    EmptyDescriptorAbilityUra,
    /// Canonical descriptor Ability URA was not a valid Ability URA.
    #[error("ability descriptor URA has invalid format: {ability_ura:?}")]
    InvalidDescriptorAbilityUra { ability_ura: String },
    /// Descriptor Ability URA could not be derived from the authority root.
    #[error("ability descriptor URA cannot be derived from authority root {authority_root:?} and ability {ability:?}")]
    DescriptorAbilityUraDerivationFailed {
        authority_root: String,
        ability: String,
    },
    /// Authority owner projection lacked the local owner-plane label.
    #[error("authority owner projection must be non-empty")]
    EmptyAuthorityOwnerProjection,
    /// Authority owner projection was not one of the canonical owner-plane
    /// markers (`device`, `hub`, `agent:<id>`, `user:<id>`, `plugin:<id>`).
    #[error("authority owner projection has invalid format: {projection:?}")]
    InvalidAuthorityOwnerProjection { projection: String },
    /// Authority root lacked the URA or local marker backing the binding.
    #[error("authority root must be non-empty")]
    EmptyAuthorityRoot,
    /// Authority root carried leading or trailing whitespace, or interior
    /// control characters, so it would not round-trip as a stable key.
    #[error("authority root has invalid format: {authority_root:?}")]
    InvalidAuthorityRoot { authority_root: String },
    /// Authority binding was created without an ability name.
    #[error("authority ability must be non-empty")]
    EmptyAuthorityAbility,
    /// Authority binding was created with an invalid ability name.
    #[error("authority ability has invalid format: {ability:?}")]
    InvalidAuthorityAbility { ability: String },
    /// Authority binding was created without the descriptor version it governs.
    #[error("authority descriptor_version must be non-empty")]
    EmptyAuthorityDescriptorVersion,
    /// Authority binding was created with an invalid descriptor version.
    #[error("authority descriptor_version has invalid format: {version:?}")]
    InvalidAuthorityDescriptorVersion { version: String },
    /// Implementation binding was created without a runtime environment label.
    #[error("runtime env label must be non-empty")]
    EmptyRuntimeEnv,
    /// Implementation binding was created without an ability name.
    #[error("implementation ability must be non-empty")]
    EmptyImplementationAbility,
    /// Implementation binding was created with an invalid ability name.
    #[error("implementation ability has invalid format: {ability:?}")]
    InvalidImplementationAbility { ability: String },
    /// Implementation binding was created without the descriptor version it satisfies.
    #[error("implementation descriptor_version must be non-empty")]
    EmptyImplementationDescriptorVersion,
    /// Implementation binding was created with an invalid descriptor version.
    #[error("implementation descriptor_version has invalid format: {version:?}")]
    InvalidImplementationDescriptorVersion { version: String },
    /// Implementation binding carried a content hash outside the canonical
    /// `sha256:<64 lowercase hex>` form.
    #[error("implementation content_hash must be sha256:<64 lowercase hex>: {hash:?}")]
    InvalidImplementationContentHash { hash: String },
}
