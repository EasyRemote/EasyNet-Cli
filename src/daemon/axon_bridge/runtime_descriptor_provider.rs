//! Runtime descriptor resolution provider.
//!
//! The C ABI, Python CFFI facade, and native SDK transports all need to
//! resolve a public ability request into a descriptor-bound Ability ref. That
//! lookup is runtime business logic: provider selection, owner matching,
//! descriptor catalog materialization, and call-mode selection must not live in
//! the FFI bridge. This module is the daemon-owned provider boundary for that
//! resolution.

use serde_json::Value;

use crate::daemon::runtime_failure::RuntimeFailureFacts;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DescriptorResolutionError {
    InvalidRequest(String),
    InvalidCatalogPayload(String),
    RuntimeOwnerUnavailable(String),
    DescriptorNotFound(String),
    OwnerOffline(String),
    OwnerMismatch(String),
    CallModeUnsupported(String),
}

impl DescriptorResolutionError {
    fn invalid_request(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    fn invalid_catalog_payload(message: impl Into<String>) -> Self {
        Self::InvalidCatalogPayload(message.into())
    }

    fn runtime_owner_unavailable(_detail: impl Into<String>) -> Self {
        Self::RuntimeOwnerUnavailable(
            "CALLER_SIGNER_UNAVAILABLE: descriptor resolution requires a caller signer; \
             load or provision that identity in the local key service"
                .to_string(),
        )
    }

    fn descriptor_not_found(message: impl Into<String>) -> Self {
        Self::DescriptorNotFound(message.into())
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

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::InvalidRequest(message)
            | Self::InvalidCatalogPayload(message)
            | Self::RuntimeOwnerUnavailable(message)
            | Self::DescriptorNotFound(message)
            | Self::OwnerOffline(message)
            | Self::OwnerMismatch(message)
            | Self::CallModeUnsupported(message) => message,
        }
    }

    pub(crate) fn canonical_detail(&self) -> String {
        match self {
            Self::RuntimeOwnerUnavailable(_) | Self::OwnerOffline(_) => {
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
            Self::InvalidCatalogPayload(_) => "INVALID_CATALOG_PAYLOAD",
            Self::RuntimeOwnerUnavailable(_) => "CALLER_SIGNER_UNAVAILABLE",
            Self::OwnerOffline(_) => "DESCRIPTOR_OWNER_OFFLINE",
            Self::DescriptorNotFound(_) | Self::CallModeUnsupported(_) => "ABILITY_NOT_FOUND",
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

pub(crate) struct RuntimeDescriptorCatalog {
    entries: Vec<Value>,
    diagnostics: Vec<String>,
}

pub(crate) struct RuntimeDescriptorResolutionProvider;

impl RuntimeDescriptorResolutionProvider {
    pub(crate) fn resolve_json(
        request_json: &str,
        runtime_owner_ura: impl FnOnce() -> std::result::Result<String, String>,
    ) -> Result<Value, DescriptorResolutionError> {
        runtime_resolve_descriptor_ref_json(request_json, runtime_owner_ura)
    }

    pub(crate) fn diagnostics_catalog_json(
        runtime_owner_ura: std::result::Result<String, String>,
    ) -> Value {
        match runtime_owner_ura {
            Ok(owner_ura) => {
                let catalog = runtime_descriptor_catalog_entries(&owner_ura);
                serde_json::json!({
                    "owner_ura": owner_ura,
                    "source": "runtime_descriptor_catalog",
                    "entries": catalog.entries,
                    "diagnostics": catalog.diagnostics,
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
        descriptor_catalog_resolution_from_entries(entries, ability_ura, call_mode, source)
            .map(|resolution| resolution.into_value())
            .map_err(anyhow::Error::msg)
    }

    #[cfg(test)]
    pub(crate) fn dedupe_catalog_entries_for_test(
        entries: Vec<Value>,
    ) -> std::result::Result<Vec<Value>, String> {
        dedupe_descriptor_catalog_entries(entries)
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

fn runtime_descriptor_catalog_entries(owner_ura: &str) -> RuntimeDescriptorCatalog {
    let mut entries = Vec::new();
    let mut diagnostics = Vec::new();
    match runtime_system_descriptor_catalog_entries(owner_ura) {
        Ok(mut system_entries) => entries.append(&mut system_entries),
        Err(error) => diagnostics.push(error),
    }
    match runtime_daemon_native_agent_descriptor_catalog_entries(owner_ura) {
        Ok(mut native_agent_entries) => entries.append(&mut native_agent_entries),
        Err(error) => diagnostics.push(error),
    }
    match dedupe_descriptor_catalog_entries(entries) {
        Ok(entries) => RuntimeDescriptorCatalog {
            entries,
            diagnostics,
        },
        Err(error) => {
            diagnostics.push(error);
            RuntimeDescriptorCatalog {
                entries: Vec::new(),
                diagnostics,
            }
        }
    }
}

fn runtime_daemon_native_agent_descriptor_catalog_entries(
    owner_ura: &str,
) -> std::result::Result<Vec<Value>, String> {
    let parsed = crate::core::ura::parse_ura(owner_ura).map_err(|error| error.to_string())?;
    if parsed.kind != crate::core::ura::URAKind::Agent {
        return Ok(Vec::new());
    }
    let Some((user_id, agent_id)) = parsed.agent_ids() else {
        return Ok(Vec::new());
    };
    if agent_id != "pages" {
        return Ok(Vec::new());
    }

    let device_ura = crate::core::ura::device_ura(&parsed.realm, "runtime-descriptor-catalog");
    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
            device_ura,
        )
        .map_err(|error| error.to_string())?
        .with_declared_agent_authority_root(owner_ura)
        .map_err(|error| error.to_string())?;
    let mut catalog =
        crate::daemon::ability::dispatch::AxonAbilityCatalog::new_metadata_only_with_authority_context(
            authority_context,
        );
    crate::daemon::ability::builtins::resources::pages::register_management_ability_descriptors(
        &mut catalog,
        &parsed.realm,
        user_id,
    );

    catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .filter(|row| row.descriptor.owner_ura.as_str() == owner_ura)
        .map(|row| descriptor_catalog_entry_from_descriptor(row.descriptor))
        .collect()
}

fn runtime_resolve_descriptor_ref_json(
    request_json: &str,
    runtime_owner_ura: impl FnOnce() -> std::result::Result<String, String>,
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
        runtime_owner_ura().map_err(DescriptorResolutionError::runtime_owner_unavailable)?;
    if runtime_owner_ura == callee_ura {
        let catalog = runtime_descriptor_catalog_entries(callee_ura);
        return descriptor_resolution_or_not_found(
            descriptor_catalog_resolution_from_entries(
                &catalog.entries,
                &ability_ura,
                call_mode,
                "runtime_local_descriptor_catalog",
            )?,
            format!(
                "descriptor_ref not found in local runtime catalog for callee_ura={callee_ura:?} ability={ability_ura:?} call_mode={call_mode:?}"
            ),
        );
    }
    if provider.is_explicit() {
        let catalog = runtime_descriptor_catalog_entries(callee_ura);
        return descriptor_resolution_or_not_found(
            descriptor_catalog_resolution_from_entries(
                &catalog.entries,
                &ability_ura,
                call_mode,
                provider.source(),
            )?,
            format!(
                "descriptor_ref not found in {} for callee_ura={callee_ura:?} ability={ability_ura:?} call_mode={call_mode:?}",
                provider.source()
            ),
        );
    }
    let catalog = runtime_descriptor_catalog_entries(callee_ura);
    descriptor_resolution_or_not_found(
        descriptor_catalog_resolution_from_entries(
            &catalog.entries,
            &ability_ura,
            call_mode,
            "runtime_remote_descriptor_catalog",
        )?,
        format!(
            "descriptor_ref not found in remote runtime catalog for callee_ura={callee_ura:?} ability={ability_ura:?} call_mode={call_mode:?}"
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
    }
}

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
    } else if matches!(owner, crate::daemon::ability::dispatch::OwnerKind::Device) {
        // Device runtime publication is broader than system descriptor
        // generation: builtin plugin abilities are registered into the
        // canonical daemon registry and must resolve descriptor_refs through
        // the same AbilityDescriptor rows as ordinary device abilities.
        // `build_system_registry()` deliberately excludes plugin packages for
        // file generation; using it here splits discovery from invocation and
        // makes resource-backed plugin abilities fail before admission.
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

fn descriptor_catalog_entry_from_descriptor(
    descriptor: crate::daemon::ability::descriptors::AbilityDescriptor,
) -> std::result::Result<Value, String> {
    let name = descriptor.public_name();
    let ability_ura = descriptor.canonical_ability_ura().ok_or_else(|| {
        format!("system descriptor catalog row {name:?} missing canonical ability URA")
    })?;
    let descriptor_hash = descriptor.descriptor_hash_prefixed();
    let descriptor_hash_hex = descriptor_hash.strip_prefix("sha256:").ok_or_else(|| {
        format!(
            "system descriptor catalog row {ability_ura:?} descriptor_hash missing sha256 prefix"
        )
    })?;
    if descriptor_hash_hex.len() != 64 || hex::decode(descriptor_hash_hex).is_err() {
        return Err(format!(
            "system descriptor catalog row {ability_ura:?} descriptor_hash is not canonical hex"
        ));
    }
    let owner_ura = descriptor.owner_ura.clone();
    let version = descriptor.version.clone();
    let call_mode = descriptor.call_mode().as_str();
    let admission_action = descriptor.admission_action().as_str();
    let descriptor_ref = descriptor.descriptor_ref().map_err(|error| {
        format!(
            "system descriptor catalog row {ability_ura:?} descriptor_ref is not canonical: {error}"
        )
    })?;
    Ok(serde_json::json!({
        "name": name,
        "owner_ura": owner_ura,
        "ability_ura": ability_ura,
        "descriptor_ref": descriptor_ref,
        "version": version,
        "descriptor_hash": descriptor_hash,
        "call_mode": call_mode,
        "admission_action": admission_action,
    }))
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
}

impl CatalogResolution {
    #[cfg(test)]
    fn into_value(self) -> Option<Value> {
        match self {
            Self::Resolved(value) => Some(value),
            Self::NotFound | Self::CallModeUnsupported { .. } => None,
        }
    }
}

fn descriptor_catalog_resolution_from_entries(
    entries: &[Value],
    ability_ura: &str,
    call_mode: &str,
    source: &str,
) -> Result<CatalogResolution, DescriptorResolutionError> {
    let mut available_modes = Vec::new();
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
        let descriptor_ref =
            descriptor_catalog_required_string(entry, "descriptor_ref", ability_ura, source)?;
        let owner_ura =
            descriptor_catalog_required_string(entry, "owner_ura", ability_ura, source)?;
        let name = descriptor_catalog_required_string(entry, "name", ability_ura, source)?;
        return Ok(CatalogResolution::Resolved(serde_json::json!({
            "descriptor_ref": descriptor_ref,
            "ability_ura": ability_ura,
            "owner_ura": owner_ura,
            "name": name,
            "call_mode": call_mode,
            "source": source,
        })));
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

fn dedupe_descriptor_catalog_entries(
    entries: Vec<Value>,
) -> std::result::Result<Vec<Value>, String> {
    let mut catalog = std::collections::BTreeMap::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let owner_ura = descriptor_catalog_dedupe_required_string(&entry, "owner_ura", index)?;
        let ability_ura = descriptor_catalog_dedupe_required_string(&entry, "ability_ura", index)?;
        let call_mode = descriptor_catalog_dedupe_required_string(&entry, "call_mode", index)?;
        let descriptor_ref =
            descriptor_catalog_dedupe_required_string(&entry, "descriptor_ref", index)?;
        let key = (
            owner_ura.to_string(),
            ability_ura.to_string(),
            call_mode.to_string(),
            descriptor_ref.to_string(),
        );
        catalog.entry(key).or_insert(entry);
    }
    Ok(catalog.into_values().collect())
}

fn descriptor_catalog_dedupe_required_string<'a>(
    entry: &'a Value,
    field: &'static str,
    index: usize,
) -> std::result::Result<&'a str, String> {
    entry
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("descriptor catalog row {index} missing {field} before dedupe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_catalog_json_projects_runtime_owned_catalog() {
        let owner_ura =
            crate::core::ura::device_ura("localhost", "386b1258-3c89-494a-90a2-2321c29bf992");
        let catalog =
            RuntimeDescriptorResolutionProvider::diagnostics_catalog_json(Ok(owner_ura.clone()));

        assert_eq!(catalog["owner_ura"], owner_ura);
        assert_eq!(catalog["source"], "runtime_descriptor_catalog");
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
            "runtime provider diagnostics catalog must include device meta.list_abilities: {entries:?}"
        );
    }

    #[test]
    fn diagnostics_catalog_json_projects_runtime_owner_failure() {
        let catalog = RuntimeDescriptorResolutionProvider::diagnostics_catalog_json(Err(
            "control discovery missing daemon_identity".to_string(),
        ));

        assert!(catalog["owner_ura"].is_null());
        assert_eq!(catalog["source"], "control.json");
        assert_eq!(catalog["entries"].as_array().map(Vec::len), Some(0));
        assert_eq!(
            catalog["diagnostics"][0],
            "control discovery missing daemon_identity"
        );
    }

    #[test]
    fn runtime_descriptor_resolver_resolves_pages_project_list_for_pages_agent() {
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let pages_agent_ura = crate::core::ura::agent_ura("localhost", "descriptor-user", "pages");
        let ability_ura = "easynet:///r/localhost/ability/descriptor-user.pages.project_list";

        let resolved = RuntimeDescriptorResolutionProvider::resolve_json(
            &serde_json::json!({
                "callee_ura": pages_agent_ura.as_str(),
                "caller_ura": local_device_ura.as_str(),
                "subject_ura": pages_agent_ura.as_str(),
                "ability": "project_list",
                "call_mode": "rpc",
            })
            .to_string(),
            || Ok(local_device_ura.clone()),
        )
        .expect("Pages project_list descriptor must resolve for the Pages Agent");

        assert_eq!(resolved["ability_ura"], ability_ura);
        assert_eq!(resolved["owner_ura"], pages_agent_ura);
        assert_eq!(resolved["name"], "project_list");
        assert_eq!(resolved["call_mode"], "rpc");
        assert_eq!(resolved["source"], "runtime_remote_descriptor_catalog");
        assert!(resolved["descriptor_ref"]
            .as_str()
            .is_some_and(
                |descriptor_ref| descriptor_ref.starts_with(&format!("{ability_ura}@"))
                    && descriptor_ref.ends_with("!read")
            ));
    }

    #[test]
    fn runtime_descriptor_resolver_resolves_pages_project_list_full_ability_ura() {
        let local_device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let pages_agent_ura = crate::core::ura::agent_ura("localhost", "descriptor-user", "pages");
        let ability_ura = "easynet:///r/localhost/ability/descriptor-user.pages.project_list";

        let resolved = RuntimeDescriptorResolutionProvider::resolve_json(
            &serde_json::json!({
                "callee_ura": pages_agent_ura.as_str(),
                "caller_ura": local_device_ura.as_str(),
                "subject_ura": pages_agent_ura.as_str(),
                "ability": ability_ura,
                "call_mode": "rpc",
            })
            .to_string(),
            || Ok(local_device_ura.clone()),
        )
        .expect("Pages project_list descriptor must resolve from a canonical Ability URA");

        assert_eq!(resolved["ability_ura"], ability_ura);
        assert_eq!(resolved["owner_ura"], pages_agent_ura);
        assert_eq!(resolved["name"], "project_list");
        assert_eq!(resolved["call_mode"], "rpc");
        assert_eq!(resolved["source"], "runtime_remote_descriptor_catalog");
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn runtime_descriptor_resolver_resolves_device_remote_desktop_create_session() {
        let device_ura = crate::core::ura::device_ura("localhost", "local-runtime-node");
        let ability_ura =
            "easynet:///r/localhost/ability/device.local-runtime-node.remote_desktop.create_session";

        let resolved = RuntimeDescriptorResolutionProvider::resolve_json(
            &serde_json::json!({
                "callee_ura": device_ura.as_str(),
                "caller_ura": "easynet:///r/localhost/user/operator",
                "subject_ura": device_ura.as_str(),
                "ability": "remote_desktop.create_session",
                "call_mode": "rpc",
            })
            .to_string(),
            || Ok(device_ura.clone()),
        )
        .expect("device remote_desktop.create_session descriptor must resolve");

        assert_eq!(resolved["ability_ura"], ability_ura);
        assert_eq!(resolved["owner_ura"], device_ura);
        assert_eq!(resolved["name"], "remote_desktop.create_session");
        assert_eq!(resolved["call_mode"], "rpc");
        assert_eq!(resolved["source"], "runtime_local_descriptor_catalog");
        assert!(resolved["descriptor_ref"]
            .as_str()
            .is_some_and(
                |descriptor_ref| descriptor_ref.starts_with(&format!("{ability_ura}@"))
                    && descriptor_ref.ends_with("!manage")
            ));
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
