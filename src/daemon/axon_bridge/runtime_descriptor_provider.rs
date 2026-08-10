//! EasyNet CLI - Runtime descriptor resolution provider
//! ====================================================
//!
//! File: src/daemon/axon_bridge/runtime_descriptor_provider.rs
//! Description: Resolves public runtime ability requests against the daemon's
//!              committed descriptor control plane.
//!
//! Protocol Responsibility:
//! - Preserve exact callee, Ability URA, call-mode, version, hash, and action
//!   binding when selecting a public descriptor ref.
//! - Fail closed on owner mismatch, malformed catalog rows, unsupported modes,
//!   and unavailable descriptor authority.
//!
//! Implementation Approach:
//! - Generic public abilities are read through an explicit
//!   `RuntimeDescriptorCatalogReader` backed by daemon `meta.list_abilities`.
//! - Built-in governance descriptors use the same reader; the attached daemon's
//!   bare local-system route is the non-recursive bootstrap ingress.
//!
//! Usage Contract:
//! - Callers must attach a catalog reader to the exact runtime session endpoint.
//! - Catalog readers return the daemon response unchanged; this module validates
//!   every row before selecting the requested mode.
//!
//! Architectural Position:
//! - EasyNet-Cli daemon/Axon bridge. The daemon committed control plane owns
//!   product descriptor truth; FFI and language facades only transport it.

use serde_json::Value;

use crate::daemon::ability::{
    insert_catalog_descriptor, AbilityCatalogQuery, AbilityCatalogRow, CatalogDescriptorKey,
};
use crate::daemon::runtime_failure::RuntimeFailureFacts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DescriptorResolutionError {
    InvalidRequest(String),
    InvalidCatalogPayload(String),
    RuntimeAttachmentUnavailable(String),
    CatalogUnavailable(String),
    DescriptorNotFound(String),
    OwnerOffline(String),
    OwnerMismatch(String),
    CallModeUnsupported(String),
    DescriptorVersionAmbiguous(String),
}

impl DescriptorResolutionError {
    fn invalid_request(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    fn invalid_catalog_payload(message: impl Into<String>) -> Self {
        Self::InvalidCatalogPayload(message.into())
    }

    pub(crate) fn runtime_attachment_unavailable(detail: impl Into<String>) -> Self {
        Self::RuntimeAttachmentUnavailable(format!(
            "RUNTIME_OFFLINE: descriptor resolution requires attached daemon identity: {}",
            detail.into()
        ))
    }

    fn descriptor_not_found(message: impl Into<String>) -> Self {
        Self::DescriptorNotFound(message.into())
    }

    pub(crate) fn catalog_unavailable(message: impl Into<String>) -> Self {
        Self::CatalogUnavailable(message.into())
    }

    pub(crate) fn owner_offline(message: impl Into<String>) -> Self {
        Self::OwnerOffline(message.into())
    }

    fn owner_mismatch(message: impl Into<String>) -> Self {
        Self::OwnerMismatch(message.into())
    }

    fn call_mode_unsupported(message: impl Into<String>) -> Self {
        Self::CallModeUnsupported(message.into())
    }

    fn descriptor_version_ambiguous(message: impl Into<String>) -> Self {
        Self::DescriptorVersionAmbiguous(message.into())
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::InvalidRequest(message)
            | Self::InvalidCatalogPayload(message)
            | Self::RuntimeAttachmentUnavailable(message)
            | Self::CatalogUnavailable(message)
            | Self::DescriptorNotFound(message)
            | Self::OwnerOffline(message)
            | Self::OwnerMismatch(message)
            | Self::CallModeUnsupported(message)
            | Self::DescriptorVersionAmbiguous(message) => message,
        }
    }

    pub(crate) fn canonical_detail(&self) -> String {
        match self {
            Self::RuntimeAttachmentUnavailable(_) | Self::OwnerOffline(_) => {
                self.runtime_failure_facts().canonical_detail()
            }
            _ => self.message().trim().to_string(),
        }
    }

    fn runtime_failure_facts(&self) -> RuntimeFailureFacts<'_> {
        RuntimeFailureFacts::new(self.runtime_failure_code(), self.message())
    }

    fn runtime_failure_code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) | Self::OwnerMismatch(_) => "INVALID_ARGUMENT",
            Self::InvalidCatalogPayload(_) => "INVALID_ARGUMENT",
            Self::RuntimeAttachmentUnavailable(_) => "RUNTIME_OFFLINE",
            Self::CatalogUnavailable(_) => "PROVIDER_UNAVAILABLE",
            Self::OwnerOffline(_) => "DESCRIPTOR_OWNER_OFFLINE",
            Self::DescriptorNotFound(_) => "DESCRIPTOR_NOT_FOUND",
            Self::CallModeUnsupported(_) => "DESCRIPTOR_MODE_UNSUPPORTED",
            Self::DescriptorVersionAmbiguous(_) => "VERSION_MISMATCH",
        }
    }
}

impl std::fmt::Display for DescriptorResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl From<crate::daemon::invocation::routing::route_resolver::ResolveRouteFailure>
    for DescriptorResolutionError
{
    fn from(
        error: crate::daemon::invocation::routing::route_resolver::ResolveRouteFailure,
    ) -> Self {
        if error.is_owner_offline() {
            Self::owner_offline(format!(
                "descriptor owner is offline for {}: {}",
                error.query_name, error.detail
            ))
        } else {
            Self::descriptor_not_found(format!(
                "descriptor route resolution failed for {}: {}",
                error.query_name, error.detail
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeDescriptorProviderKind {
    Generic,
    AbilityDescriptor,
    ReceiptHistory,
}

impl RuntimeDescriptorProviderKind {
    fn parse(raw: Option<&str>) -> Result<Self, DescriptorResolutionError> {
        let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::Generic);
        };
        match raw {
            "ability_descriptor" => Ok(Self::AbilityDescriptor),
            "receipt_history" => Ok(Self::ReceiptHistory),
            other => Err(DescriptorResolutionError::invalid_request(format!(
                "descriptor_ref request provider {other:?} is not supported"
            ))),
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::Generic => "runtime_descriptor_catalog",
            Self::AbilityDescriptor => "runtime_ability_descriptor_provider",
            Self::ReceiptHistory => "runtime_receipt_provider",
        }
    }

    fn require_ability(self, ability: &str) -> Result<(), DescriptorResolutionError> {
        match self {
            Self::Generic
                if crate::daemon::ability::names::governance::is_runtime_catalogue_read(
                    ability,
                ) =>
            {
                Err(DescriptorResolutionError::invalid_request(format!(
                    "descriptor_ref generic provider cannot resolve runtime catalogue read ability {ability:?}; use provider \"ability_descriptor\""
                )))
            }
            Self::Generic
                if crate::daemon::ability::names::governance::is_invocation_history_read(
                    ability,
                ) =>
            {
                Err(DescriptorResolutionError::invalid_request(format!(
                    "descriptor_ref generic provider cannot resolve receipt history read ability {ability:?}; use provider \"receipt_history\""
                )))
            }
            Self::Generic => Ok(()),
            Self::AbilityDescriptor
                if crate::daemon::ability::names::governance::is_runtime_catalogue_read(
                    ability,
                ) =>
            {
                Ok(())
            }
            Self::ReceiptHistory
                if crate::daemon::ability::names::governance::is_invocation_history_read(
                    ability,
                ) =>
            {
                Ok(())
            }
            Self::AbilityDescriptor => Err(DescriptorResolutionError::invalid_request(format!(
                "descriptor_ref provider ability_descriptor cannot resolve non-catalogue ability {ability:?}"
            ))),
            Self::ReceiptHistory => Err(DescriptorResolutionError::invalid_request(format!(
                "descriptor_ref provider receipt_history cannot resolve non-receipt ability {ability:?}"
            ))),
        }
    }

    fn validate_request_subject(
        self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), DescriptorResolutionError> {
        match self {
            Self::AbilityDescriptor => validate_ability_descriptor_catalogue_subject(object),
            Self::ReceiptHistory => validate_receipt_history_descriptor_subject(object),
            Self::Generic => Ok(()),
        }
    }

    fn is_explicit(self) -> bool {
        !matches!(self, Self::Generic)
    }
}

pub(crate) trait RuntimeDescriptorCatalogReader {
    fn read_catalog(
        &self,
        runtime_owner_ura: &str,
        query: &AbilityCatalogQuery,
    ) -> Result<Value, DescriptorResolutionError>;
}

pub(crate) struct RuntimeDescriptorResolutionProvider;

impl RuntimeDescriptorResolutionProvider {
    pub(crate) fn resolve_json(
        request_json: &str,
        runtime_owner_ura: impl FnOnce() -> std::result::Result<String, String>,
        catalog_reader: &dyn RuntimeDescriptorCatalogReader,
    ) -> Result<Value, DescriptorResolutionError> {
        runtime_resolve_descriptor_ref_json(request_json, runtime_owner_ura, catalog_reader)
    }

    pub(crate) fn diagnostics_catalog_json(
        runtime_owner_ura: std::result::Result<String, String>,
        catalog_reader: &dyn RuntimeDescriptorCatalogReader,
    ) -> Value {
        match runtime_owner_ura {
            Ok(owner_ura) => {
                let catalog = runtime_live_descriptor_catalog_entries(
                    catalog_reader,
                    &owner_ura,
                    &AbilityCatalogQuery::all_realm(),
                );
                let (entries, diagnostics) = match catalog {
                    Ok(entries) => (entries, Vec::new()),
                    Err(error) => (Vec::new(), vec![error.canonical_detail()]),
                };
                serde_json::json!({
                    "owner_ura": owner_ura,
                    "source": "runtime_committed_descriptor_catalog",
                    "entries": entries,
                    "diagnostics": diagnostics,
                })
            }
            Err(error) => serde_json::json!({
                "owner_ura": null,
                "source": "control.json",
                "entries": [],
                "diagnostics": [error],
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn resolve_catalog_entries_for_test(
        entries: &[Value],
        ability_ura: &str,
        call_mode: &str,
        source: &str,
    ) -> anyhow::Result<Option<Value>> {
        descriptor_catalog_resolution_from_entries(entries, ability_ura, call_mode, None, source)
            .map(|resolution| resolution.into_value())
            .map_err(anyhow::Error::msg)
    }

    #[cfg(test)]
    pub(crate) fn system_catalog_entries_for_test(
        owner_ura: &str,
    ) -> std::result::Result<Vec<Value>, String> {
        runtime_system_descriptor_catalog_entries(owner_ura)
    }
}

fn descriptor_request_required_text<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    missing_message: &'static str,
) -> Result<&'a str, DescriptorResolutionError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DescriptorResolutionError::invalid_request(missing_message))
}

fn validate_ability_descriptor_catalogue_subject(
    object: &serde_json::Map<String, Value>,
) -> Result<(), DescriptorResolutionError> {
    let callee_ura = descriptor_request_required_text(
        object,
        "callee_ura",
        "descriptor_ref provider ability_descriptor requires callee_ura",
    )?;
    let subject_ura = descriptor_request_required_text(
        object,
        "subject_ura",
        "descriptor_ref provider ability_descriptor requires subject_ura",
    )?;
    crate::core::identity::RuntimeGovernanceReadSubject::parse_for_callee(subject_ura, callee_ura)
        .map(|_| ())
        .map_err(ability_descriptor_catalogue_subject_error)
}

fn ability_descriptor_catalogue_subject_error(
    error: crate::core::identity::RuntimeGovernanceReadSubjectError,
) -> DescriptorResolutionError {
    use crate::core::identity::RuntimeGovernanceReadSubjectError;

    match error {
        RuntimeGovernanceReadSubjectError::Empty => DescriptorResolutionError::invalid_request(
            "descriptor_ref provider ability_descriptor requires subject_ura",
        ),
        RuntimeGovernanceReadSubjectError::AllZeroPrincipalPlaceholder => {
            DescriptorResolutionError::invalid_request(
                "descriptor_ref provider ability_descriptor subject_ura must not be all-zero",
            )
        }
        RuntimeGovernanceReadSubjectError::InvalidSyntax(error) => {
            DescriptorResolutionError::invalid_request(format!(
                "descriptor_ref provider ability_descriptor subject_ura must be canonical: {error}"
            ))
        }
        RuntimeGovernanceReadSubjectError::InvalidCallee(error) => {
            DescriptorResolutionError::invalid_request(format!(
                "descriptor_ref provider ability_descriptor callee_ura must be canonical: {error}"
            ))
        }
        RuntimeGovernanceReadSubjectError::NotRuntimeGovernanceRead => {
            DescriptorResolutionError::invalid_request(
                "descriptor_ref provider ability_descriptor subject_ura must be a user-owned runtime-state read subject or the callee runtime-owner subject",
            )
        }
    }
}

fn validate_receipt_history_descriptor_subject(
    object: &serde_json::Map<String, Value>,
) -> Result<(), DescriptorResolutionError> {
    let callee_ura = descriptor_request_required_text(
        object,
        "callee_ura",
        "descriptor_ref provider receipt_history requires callee_ura",
    )?;
    let subject_ura = descriptor_request_required_text(
        object,
        "subject_ura",
        "descriptor_ref provider receipt_history requires subject_ura",
    )?;
    crate::core::identity::RuntimeGovernanceReadSubject::parse_for_callee(subject_ura, callee_ura)
        .map(|_| ())
        .map_err(receipt_history_descriptor_subject_error)
}

fn receipt_history_descriptor_subject_error(
    error: crate::core::identity::RuntimeGovernanceReadSubjectError,
) -> DescriptorResolutionError {
    use crate::core::identity::RuntimeGovernanceReadSubjectError;

    match error {
        RuntimeGovernanceReadSubjectError::Empty => DescriptorResolutionError::invalid_request(
            "descriptor_ref provider receipt_history requires subject_ura",
        ),
        RuntimeGovernanceReadSubjectError::AllZeroPrincipalPlaceholder => {
            DescriptorResolutionError::invalid_request(
                "descriptor_ref provider receipt_history subject_ura must not be all-zero",
            )
        }
        RuntimeGovernanceReadSubjectError::InvalidSyntax(error) => {
            DescriptorResolutionError::invalid_request(format!(
                "descriptor_ref provider receipt_history subject_ura must be canonical: {error}"
            ))
        }
        RuntimeGovernanceReadSubjectError::InvalidCallee(error) => {
            DescriptorResolutionError::invalid_request(format!(
                "descriptor_ref provider receipt_history callee_ura must be canonical: {error}"
            ))
        }
        RuntimeGovernanceReadSubjectError::NotRuntimeGovernanceRead => {
            DescriptorResolutionError::invalid_request(
                "descriptor_ref provider receipt_history subject_ura must be a user-owned runtime-state read subject or the callee runtime-owner subject",
            )
        }
    }
}

fn runtime_live_descriptor_catalog_entries(
    catalog_reader: &dyn RuntimeDescriptorCatalogReader,
    runtime_owner_ura: &str,
    query: &AbilityCatalogQuery,
) -> Result<Vec<Value>, DescriptorResolutionError> {
    let payload = catalog_reader.read_catalog(runtime_owner_ura, query)?;
    let entries = payload
        .get("abilities")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            DescriptorResolutionError::invalid_catalog_payload(
                "committed runtime descriptor catalog response missing abilities array",
            )
        })?;
    let mut canonical = std::collections::BTreeMap::<
        CatalogDescriptorKey,
        crate::daemon::ability::AbilityDescriptor,
    >::new();
    for (index, entry) in entries.iter().enumerate() {
        let row = AbilityCatalogRow::parse(entry, index, "committed runtime descriptor")
            .map_err(DescriptorResolutionError::invalid_catalog_payload)?;
        let key = row.key();
        if query
            .owner_ura()
            .is_some_and(|expected| expected != key.owner_ura())
        {
            return Err(DescriptorResolutionError::invalid_catalog_payload(format!(
                "committed runtime descriptor catalog row #{index} owner_ura {:?} does not match requested owner {:?}",
                key.owner_ura(),
                query.owner_ura()
            )));
        }
        if query
            .ability_ura()
            .is_some_and(|expected| expected != key.ability_ura())
        {
            return Err(DescriptorResolutionError::invalid_catalog_payload(format!(
                "committed runtime descriptor catalog row #{index} ability_ura {:?} does not match requested ability {:?}",
                key.ability_ura(),
                query.ability_ura()
            )));
        }
        if query
            .descriptor_version()
            .is_some_and(|expected| expected != key.descriptor_version())
        {
            return Err(DescriptorResolutionError::invalid_catalog_payload(format!(
                "committed runtime descriptor catalog row #{index} version {:?} does not match requested descriptor_version {:?}",
                key.descriptor_version(),
                query.descriptor_version()
            )));
        }
        insert_catalog_descriptor(
            &mut canonical,
            row.descriptor().clone(),
            "committed runtime descriptor catalog",
        )
        .map_err(DescriptorResolutionError::invalid_catalog_payload)?;
    }
    canonical
        .into_values()
        .map(|descriptor| {
            AbilityCatalogRow::from_descriptor(descriptor)
                .map(AbilityCatalogRow::into_value)
                .map_err(DescriptorResolutionError::invalid_catalog_payload)
        })
        .collect()
}

fn runtime_resolve_descriptor_ref_json(
    request_json: &str,
    runtime_owner_ura: impl FnOnce() -> std::result::Result<String, String>,
    catalog_reader: &dyn RuntimeDescriptorCatalogReader,
) -> Result<Value, DescriptorResolutionError> {
    let request: Value = serde_json::from_str(request_json).map_err(|error| {
        DescriptorResolutionError::invalid_request(format!(
            "decode descriptor_ref request: {error}"
        ))
    })?;
    let object = request.as_object().ok_or_else(|| {
        DescriptorResolutionError::invalid_request("descriptor_ref request must be a JSON object")
    })?;
    let callee_ura = object
        .get("callee_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            DescriptorResolutionError::invalid_request("descriptor_ref request missing callee_ura")
        })?;
    let ability = object
        .get("ability")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            DescriptorResolutionError::invalid_request("descriptor_ref request missing ability")
        })?;
    let call_mode = object
        .get("call_mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            DescriptorResolutionError::invalid_request("descriptor_ref request missing call_mode")
        })?;
    let descriptor_version = object
        .get("descriptor_version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(version) = descriptor_version {
        crate::daemon::ability::AbilityDescriptorVersion::new(version.to_string()).map_err(
            |error| {
                DescriptorResolutionError::invalid_request(format!(
                    "descriptor_ref request has invalid descriptor_version {version:?}: {error}"
                ))
            },
        )?;
    }
    let provider =
        RuntimeDescriptorProviderKind::parse(object.get("provider").and_then(Value::as_str))?;
    let ability_ura =
        crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(callee_ura, ability)
            .map_err(|error| descriptor_ability_error(callee_ura, ability, error))?;
    let public_ability = crate::core::ura::AbilitySelector::parse(&ability_ura)
        .map(|selector| selector.public_name().to_string())
        .map_err(|error| {
            DescriptorResolutionError::invalid_request(format!(
                "resolve descriptor_ref public ability for ability_ura={ability_ura:?}: {error}"
            ))
        })?;
    provider.require_ability(&public_ability)?;
    provider.validate_request_subject(object)?;
    let runtime_owner_ura =
        runtime_owner_ura().map_err(DescriptorResolutionError::runtime_attachment_unavailable)?;
    let query = AbilityCatalogQuery::exact(callee_ura, &ability_ura, descriptor_version);
    let catalog =
        runtime_live_descriptor_catalog_entries(catalog_reader, &runtime_owner_ura, &query)?;
    let source = if provider.is_explicit() {
        provider.source()
    } else {
        "runtime_committed_descriptor_catalog"
    };
    descriptor_resolution_or_not_found(
        descriptor_catalog_resolution_from_entries(
            &catalog,
            &ability_ura,
            call_mode,
            descriptor_version,
            source,
        )?,
        format!(
            "descriptor_ref not found in committed runtime catalog for callee_ura={callee_ura:?} ability={ability_ura:?} descriptor_version={descriptor_version:?} call_mode={call_mode:?}"
        ),
    )
}

fn descriptor_ability_error(
    callee_ura: &str,
    ability: &str,
    error: axon_sdk::invocation::AxonError,
) -> DescriptorResolutionError {
    let message = error.to_string();
    if message.contains("does not match callee") {
        return DescriptorResolutionError::owner_mismatch(message);
    }
    DescriptorResolutionError::invalid_request(format!(
        "resolve descriptor_ref ability for callee_ura={callee_ura:?} ability={ability:?}: {message}"
    ))
}

fn descriptor_resolution_or_not_found(
    resolution: CatalogResolution,
    not_found: String,
) -> Result<Value, DescriptorResolutionError> {
    match resolution {
        CatalogResolution::Resolved(value) => Ok(value),
        CatalogResolution::NotFound => {
            Err(DescriptorResolutionError::descriptor_not_found(not_found))
        }
        CatalogResolution::CallModeUnsupported {
            ability_ura,
            call_mode,
            available_modes,
            source,
        } => Err(DescriptorResolutionError::call_mode_unsupported(format!(
            "descriptor_ref call_mode {call_mode:?} is not supported for ability {ability_ura:?} in {source}; available_call_modes={available_modes:?}"
        ))),
        CatalogResolution::VersionAmbiguous {
            ability_ura,
            call_mode,
            available_versions,
            source,
        } => Err(DescriptorResolutionError::descriptor_version_ambiguous(format!(
            "descriptor_ref version is ambiguous for ability {ability_ura:?} call_mode={call_mode:?} in {source}; available_descriptor_versions={available_versions:?}; pass descriptor_version"
        ))),
    }
}

#[cfg(test)]
fn runtime_system_descriptor_catalog_entries(
    owner_ura: &str,
) -> std::result::Result<Vec<Value>, String> {
    let owner = crate::daemon::axon_bridge::descriptor_ref::catalog_owner_kind_for_wire(owner_ura)
        .map_err(|error| error.to_string())?;
    let catalog = if matches!(
        owner,
        crate::daemon::ability::dispatch::OwnerKind::RealmAuthority
    ) {
        crate::daemon::ability::catalog::build_system_registry_for_authority_owner(owner_ura)?
    } else if owner == crate::daemon::ability::dispatch::OwnerKind::plugin_management_system() {
        // Builtin plugin descriptors are contributed by the canonical plugin
        // registry and owned by the plugin-management SystemAgent. Loading the
        // full registry here keeps descriptor-ref tests aligned with the
        // runtime catalog without projecting plugin abilities back onto the
        // Device execution host.
        crate::daemon::ability::catalog::build_registry()
    } else {
        crate::daemon::ability::catalog::build_system_registry()
    };
    let mut entries = Vec::new();
    for row in catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .filter(|row| row.owner == owner)
    {
        let descriptor = row
            .descriptor
            .rebind_owner_ura(owner_ura)
            .map_err(|error| format!("system descriptor catalog rebind failed: {error}"))?;
        entries.push(descriptor_catalog_entry_from_descriptor(descriptor)?);
    }
    Ok(entries)
}

#[cfg(test)]
fn descriptor_catalog_entry_from_descriptor(
    descriptor: crate::daemon::ability::descriptors::AbilityDescriptor,
) -> std::result::Result<Value, String> {
    AbilityCatalogRow::from_descriptor(descriptor).map(AbilityCatalogRow::into_value)
}

#[derive(Debug)]
enum CatalogResolution {
    Resolved(Value),
    NotFound,
    CallModeUnsupported {
        ability_ura: String,
        call_mode: String,
        available_modes: Vec<String>,
        source: String,
    },
    VersionAmbiguous {
        ability_ura: String,
        call_mode: String,
        available_versions: Vec<String>,
        source: String,
    },
}

impl CatalogResolution {
    #[cfg(test)]
    fn into_value(self) -> Option<Value> {
        match self {
            Self::Resolved(value) => Some(value),
            Self::NotFound | Self::CallModeUnsupported { .. } | Self::VersionAmbiguous { .. } => {
                None
            }
        }
    }
}

fn descriptor_catalog_resolution_from_entries(
    entries: &[Value],
    ability_ura: &str,
    call_mode: &str,
    descriptor_version: Option<&str>,
    source: &str,
) -> Result<CatalogResolution, DescriptorResolutionError> {
    let mut available_modes = Vec::new();
    let mut matching = Vec::new();
    for entry in entries {
        let entry_ability = entry
            .get("ability_ura")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if entry_ability != Some(ability_ura) {
            continue;
        }
        let entry_call_mode = entry
            .get("call_mode")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                DescriptorResolutionError::invalid_catalog_payload(format!(
                    "descriptor catalog row for ability {ability_ura:?} from {source} missing call_mode"
                ))
            })?;
        available_modes.push(entry_call_mode.to_string());
        if entry_call_mode != call_mode {
            continue;
        }
        let entry_version =
            descriptor_catalog_required_string(entry, "version", ability_ura, source)?;
        if descriptor_version.is_some_and(|expected| expected != entry_version) {
            continue;
        }
        let descriptor_ref =
            descriptor_catalog_required_string(entry, "descriptor_ref", ability_ura, source)?;
        let owner_ura =
            descriptor_catalog_required_string(entry, "owner_ura", ability_ura, source)?;
        let name = descriptor_catalog_required_string(entry, "name", ability_ura, source)?;
        matching.push((
            entry_version.to_string(),
            serde_json::json!({
                "descriptor_ref": descriptor_ref,
                "ability_ura": ability_ura,
                "owner_ura": owner_ura,
                "name": name,
                "descriptor_version": entry_version,
                "call_mode": call_mode,
                "source": source,
            }),
        ));
    }
    if matching.len() == 1 {
        return Ok(CatalogResolution::Resolved(
            matching.pop().expect("one matching descriptor").1,
        ));
    }
    if matching.len() > 1 {
        let mut available_versions = matching
            .into_iter()
            .map(|(version, _)| version)
            .collect::<Vec<_>>();
        available_versions.sort();
        available_versions.dedup();
        return Ok(CatalogResolution::VersionAmbiguous {
            ability_ura: ability_ura.to_string(),
            call_mode: call_mode.to_string(),
            available_versions,
            source: source.to_string(),
        });
    }
    if available_modes.is_empty() {
        Ok(CatalogResolution::NotFound)
    } else {
        available_modes.sort();
        available_modes.dedup();
        Ok(CatalogResolution::CallModeUnsupported {
            ability_ura: ability_ura.to_string(),
            call_mode: call_mode.to_string(),
            available_modes,
            source: source.to_string(),
        })
    }
}

fn descriptor_catalog_required_string<'a>(
    entry: &'a Value,
    field: &'static str,
    ability_ura: &str,
    source: &str,
) -> Result<&'a str, DescriptorResolutionError> {
    entry
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            DescriptorResolutionError::invalid_catalog_payload(format!(
                "descriptor catalog row for ability {ability_ura:?} from {source} missing {field}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CommittedCatalogReader {
        entries: Vec<Value>,
    }

    impl CommittedCatalogReader {
        fn new(entries: Vec<Value>) -> Self {
            Self { entries }
        }
    }

    impl RuntimeDescriptorCatalogReader for CommittedCatalogReader {
        fn read_catalog(
            &self,
            _runtime_owner_ura: &str,
            query: &AbilityCatalogQuery,
        ) -> Result<Value, DescriptorResolutionError> {
            let abilities = self
                .entries
                .iter()
                .filter(|entry| {
                    query.owner_ura().is_none_or(|owner_ura| {
                        entry.get("owner_ura").and_then(Value::as_str) == Some(owner_ura)
                    }) && query.ability_ura().is_none_or(|ability_ura| {
                        entry.get("ability_ura").and_then(Value::as_str) == Some(ability_ura)
                    }) && query.descriptor_version().is_none_or(|version| {
                        entry.get("version").and_then(Value::as_str) == Some(version)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            Ok(serde_json::json!({ "abilities": abilities }))
        }
    }

    struct UnfilteredCatalogReader {
        entries: Vec<Value>,
    }

    impl RuntimeDescriptorCatalogReader for UnfilteredCatalogReader {
        fn read_catalog(
            &self,
            _runtime_owner_ura: &str,
            _query: &AbilityCatalogQuery,
        ) -> Result<Value, DescriptorResolutionError> {
            Ok(serde_json::json!({ "abilities": self.entries }))
        }
    }

    fn hosted_agent_descriptor_entry(
        owner_ura: &str,
        call_mode: crate::daemon::ability::descriptors::CallMode,
        action: crate::daemon::ability::descriptors::AdmissionAction,
    ) -> Value {
        let descriptor = crate::daemon::ability::descriptors::AbilityDescriptor::new(
            "chat",
            owner_ura,
            crate::daemon::ability::descriptors::Visibility::Public,
            action,
        )
        .expect("hosted-agent descriptor")
        .with_call_mode(call_mode);
        descriptor_catalog_entry_from_descriptor(descriptor).expect("catalog entry")
    }

    #[test]
    fn diagnostics_catalog_json_projects_runtime_owned_catalog() {
        let owner_ura = crate::core::ura::device_agent_ura(
            "localhost",
            "386b1258-3c89-494a-90a2-2321c29bf992",
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        );
        let entries = runtime_system_descriptor_catalog_entries(&owner_ura)
            .expect("runtime system descriptors");
        let reader = CommittedCatalogReader::new(entries);
        let catalog = RuntimeDescriptorResolutionProvider::diagnostics_catalog_json(
            Ok(owner_ura.clone()),
            &reader,
        );

        assert_eq!(catalog["owner_ura"], owner_ura);
        assert_eq!(catalog["source"], "runtime_committed_descriptor_catalog");
        assert!(catalog["diagnostics"].as_array().is_some_and(Vec::is_empty));
        let entries = catalog["entries"].as_array().expect("catalog entries");
        assert!(
            entries.iter().any(|entry| {
                entry["owner_ura"] == owner_ura
                    && entry["name"]
                        == crate::daemon::ability::names::governance::META_LIST_ABILITIES
                    && entry["call_mode"] == "rpc"
                    && entry["descriptor_ref"]
                        .as_str()
                        .is_some_and(|descriptor_ref| descriptor_ref.ends_with("!read"))
            }),
            "runtime provider diagnostics catalog must include runtime-introspection meta.list_abilities: {entries:?}"
        );
    }

    #[test]
    fn diagnostics_catalog_json_projects_runtime_owner_failure() {
        let reader = CommittedCatalogReader::new(Vec::new());
        let catalog = RuntimeDescriptorResolutionProvider::diagnostics_catalog_json(
            Err("control discovery missing daemon_identity".to_string()),
            &reader,
        );

        assert!(catalog["owner_ura"].is_null());
        assert_eq!(catalog["source"], "control.json");
        assert_eq!(catalog["entries"].as_array().map(Vec::len), Some(0));
        assert_eq!(
            catalog["diagnostics"][0],
            "control discovery missing daemon_identity"
        );
    }

    #[test]
    fn runtime_descriptor_resolver_resolves_hosted_agent_rpc_and_stream_from_committed_catalog() {
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let agent_ura = crate::core::ura::agent_ura(
            "localhost",
            "2ce7a746-fb6c-45dc-9aff-d494296acf48",
            "codex-smoke",
        );
        let ability_ura = format!(
            "easynet:///r/localhost/ability/2ce7a746-fb6c-45dc-9aff-d494296acf48.codex-smoke.chat"
        );
        let reader = CommittedCatalogReader::new(vec![
            hosted_agent_descriptor_entry(
                &agent_ura,
                crate::daemon::ability::descriptors::CallMode::Rpc,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            ),
            hosted_agent_descriptor_entry(
                &agent_ura,
                crate::daemon::ability::descriptors::CallMode::Stream,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            ),
        ]);

        for call_mode in ["rpc", "stream"] {
            let resolved = RuntimeDescriptorResolutionProvider::resolve_json(
                &serde_json::json!({
                    "callee_ura": agent_ura,
                    "caller_ura": local_device_ura,
                    "subject_ura": agent_ura,
                    "ability": "chat",
                    "call_mode": call_mode,
                })
                .to_string(),
                || Ok(local_device_ura.clone()),
                &reader,
            )
            .unwrap_or_else(|error| panic!("hosted-agent {call_mode} descriptor: {error}"));

            assert_eq!(resolved["ability_ura"], ability_ura);
            assert_eq!(resolved["owner_ura"], agent_ura);
            assert_eq!(resolved["name"], "chat");
            assert_eq!(resolved["call_mode"], call_mode);
            assert_eq!(resolved["source"], "runtime_committed_descriptor_catalog");
            assert!(resolved["descriptor_ref"]
                .as_str()
                .is_some_and(|descriptor_ref| descriptor_ref
                    .starts_with(&format!("{ability_ura}@"))
                    && descriptor_ref.ends_with("!invoke")));
        }
    }

    #[test]
    fn runtime_descriptor_resolver_rejects_catalog_rows_from_another_owner() {
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let requested_agent_ura =
            crate::core::ura::agent_ura("localhost", "requested-agent", "codex-smoke");
        let stale_agent_ura =
            crate::core::ura::agent_ura("localhost", "stale-agent", "codex-smoke");
        let reader = UnfilteredCatalogReader {
            entries: vec![hosted_agent_descriptor_entry(
                &stale_agent_ura,
                crate::daemon::ability::descriptors::CallMode::Rpc,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            )],
        };

        let error = RuntimeDescriptorResolutionProvider::resolve_json(
            &serde_json::json!({
                "callee_ura": requested_agent_ura,
                "caller_ura": local_device_ura,
                "subject_ura": requested_agent_ura,
                "ability": "chat",
                "call_mode": "rpc",
            })
            .to_string(),
            || Ok(local_device_ura),
            &reader,
        )
        .expect_err("a committed catalog row from another owner must fail closed");

        assert!(matches!(
            error,
            DescriptorResolutionError::InvalidCatalogPayload(_)
        ));
        assert!(error.message().contains("does not match requested owner"));
    }

    #[test]
    fn runtime_descriptor_resolver_reports_unsupported_hosted_agent_call_mode() {
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let agent_ura = crate::core::ura::agent_ura("localhost", "mode-agent", "codex-smoke");
        let reader = CommittedCatalogReader::new(vec![hosted_agent_descriptor_entry(
            &agent_ura,
            crate::daemon::ability::descriptors::CallMode::Rpc,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )]);

        let error = RuntimeDescriptorResolutionProvider::resolve_json(
            &serde_json::json!({
                "callee_ura": agent_ura,
                "caller_ura": local_device_ura,
                "subject_ura": agent_ura,
                "ability": "chat",
                "call_mode": "stream",
            })
            .to_string(),
            || Ok(local_device_ura),
            &reader,
        )
        .expect_err("the resolver must not substitute a different call mode");

        assert!(matches!(
            error,
            DescriptorResolutionError::CallModeUnsupported(_)
        ));
        assert!(error.message().contains("available_call_modes=[\"rpc\"]"));
    }

    #[test]
    fn runtime_descriptor_resolver_requires_version_when_catalog_has_multiple_versions() {
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let agent_ura = crate::core::ura::agent_ura("localhost", "version-agent", "codex-smoke");
        let v1 = crate::daemon::ability::descriptors::AbilityDescriptor::new(
            "chat",
            &agent_ura,
            crate::daemon::ability::descriptors::Visibility::Public,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("v1 descriptor");
        let v2 = v1.clone().with_version("2.0.0").expect("v2 descriptor");
        let reader = CommittedCatalogReader::new(vec![
            descriptor_catalog_entry_from_descriptor(v1).expect("v1 row"),
            descriptor_catalog_entry_from_descriptor(v2).expect("v2 row"),
        ]);
        let request = |version: Option<&str>| {
            let mut request = serde_json::json!({
                "callee_ura": agent_ura,
                "caller_ura": local_device_ura,
                "subject_ura": agent_ura,
                "ability": "chat",
                "call_mode": "rpc",
            });
            if let Some(version) = version {
                request["descriptor_version"] = Value::String(version.to_string());
            }
            request.to_string()
        };

        let error = RuntimeDescriptorResolutionProvider::resolve_json(
            &request(None),
            || Ok(local_device_ura.clone()),
            &reader,
        )
        .expect_err("versionless selection must fail when multiple versions are visible");
        assert!(matches!(
            error,
            DescriptorResolutionError::DescriptorVersionAmbiguous(_)
        ));
        assert!(error.message().contains("[\"1.0.0\", \"2.0.0\"]"));

        let resolved = RuntimeDescriptorResolutionProvider::resolve_json(
            &request(Some("2.0.0")),
            || Ok(local_device_ura.clone()),
            &reader,
        )
        .expect("explicit descriptor version must select exactly one row");
        assert_eq!(resolved["descriptor_version"], "2.0.0");
    }

    #[test]
    fn runtime_descriptor_resolver_rejects_descriptor_hash_drift() {
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let agent_ura = crate::core::ura::agent_ura("localhost", "hash-agent", "codex-smoke");
        let mut entry = hosted_agent_descriptor_entry(
            &agent_ura,
            crate::daemon::ability::descriptors::CallMode::Rpc,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        );
        entry["descriptor_hash"] = Value::String(format!("sha256:{}", "00".repeat(32)));
        let reader = UnfilteredCatalogReader {
            entries: vec![entry],
        };

        let error = RuntimeDescriptorResolutionProvider::resolve_json(
            &serde_json::json!({
                "callee_ura": agent_ura,
                "caller_ura": local_device_ura,
                "subject_ura": agent_ura,
                "ability": "chat",
                "call_mode": "rpc",
            })
            .to_string(),
            || Ok(local_device_ura),
            &reader,
        )
        .expect_err("descriptor hash drift must fail closed");

        assert!(matches!(
            error,
            DescriptorResolutionError::InvalidCatalogPayload(_)
        ));
        assert!(error.message().contains("wire descriptor_hash"));
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn runtime_descriptor_resolver_resolves_every_remote_desktop_system_agent_descriptor() {
        let device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let plugin_management_ura = crate::core::ura::device_agent_ura(
            "localhost",
            "local-runtime-node",
            crate::daemon::ability::names::integrations::PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID,
        );
        let reader = CommittedCatalogReader::new(
            runtime_system_descriptor_catalog_entries(&plugin_management_ura)
                .expect("plugin-management descriptor catalog"),
        );
        for (ability, call_mode, action) in [
            ("remote_desktop.add_ice_candidate", "rpc", "manage"),
            ("remote_desktop.attach", "bidi", "stream"),
            ("remote_desktop.create_session", "rpc", "manage"),
            ("remote_desktop.end_session", "rpc", "manage"),
            ("remote_desktop.grant_consent", "rpc", "manage"),
            ("remote_desktop.permission_status", "rpc", "read"),
            ("remote_desktop.refresh_lease", "rpc", "manage"),
            ("remote_desktop.request_permission", "rpc", "manage"),
            ("remote_desktop.set_description", "rpc", "manage"),
            ("remote_desktop.show_session", "rpc", "read"),
            ("remote_desktop.watch_events", "stream", "stream"),
        ] {
            let ability_ura = crate::core::ura::owner_ability_ura(&plugin_management_ura, ability)
                .expect("plugin-management remote_desktop ability URA");
            let resolved = RuntimeDescriptorResolutionProvider::resolve_json(
                &serde_json::json!({
                    "callee_ura": plugin_management_ura.as_str(),
                    "caller_ura": "easynet:///r/localhost/user/operator",
                    "subject_ura": "easynet:///r/localhost/resource/remote-desktop-session",
                    "ability": ability,
                    "call_mode": call_mode,
                })
                .to_string(),
                || Ok(device_ura.clone()),
                &reader,
            )
            .unwrap_or_else(|error| {
                panic!("plugin-management {ability} descriptor must resolve: {error}")
            });

            assert_eq!(resolved["ability_ura"], ability_ura, "{ability}");
            assert_eq!(resolved["owner_ura"], plugin_management_ura, "{ability}");
            assert_eq!(resolved["name"], ability, "{ability}");
            assert_eq!(resolved["call_mode"], call_mode, "{ability}");
            assert_eq!(
                resolved["source"], "runtime_committed_descriptor_catalog",
                "{ability}"
            );
            assert!(
                resolved["descriptor_ref"]
                    .as_str()
                    .is_some_and(|descriptor_ref| {
                        descriptor_ref.starts_with(&format!("{ability_ura}@"))
                            && descriptor_ref.ends_with(&format!("!{action}"))
                    }),
                "{ability} must resolve to its exact action-bound descriptor_ref"
            );
        }
    }

    #[test]
    fn route_negative_owner_offline_maps_to_descriptor_owner_offline() {
        let route_failure =
            crate::daemon::invocation::routing::route_resolver::ResolveRouteFailure::owner_offline(
                "easynet:///r/localhost/ability/device.dev-a.meta.list_abilities",
                crate::daemon::federation::resolver_contract::NegativeReason::Nxdomain,
            );

        let descriptor_error = DescriptorResolutionError::from(route_failure);

        assert!(matches!(
            descriptor_error,
            DescriptorResolutionError::OwnerOffline(_)
        ));
        assert_eq!(
            descriptor_error.canonical_detail(),
            "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online"
        );
    }

    #[test]
    fn route_negative_generic_maps_to_descriptor_not_found() {
        let route_failure =
            crate::daemon::invocation::routing::route_resolver::ResolveRouteFailure::new(
                "easynet:///r/localhost/ability/device.dev-a.custom.missing",
                crate::daemon::federation::resolver_contract::NegativeReason::Nxdomain,
                "ability label is not published",
            );

        let descriptor_error = DescriptorResolutionError::from(route_failure);

        assert!(matches!(
            descriptor_error,
            DescriptorResolutionError::DescriptorNotFound(_)
        ));
        assert!(!descriptor_error
            .canonical_detail()
            .contains("DESCRIPTOR_OWNER_OFFLINE"));
    }
}
