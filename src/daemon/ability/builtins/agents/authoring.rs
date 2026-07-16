//! File: `src/daemon/ability/builtins/agents/authoring.rs`
//! Description: Transactional authoring for hosted-Agent abilities.
//!
//! Protocol responsibility: commit executable manifests and their live
//! control-plane/runtime rows as one daemon-owned capability transaction.
//! A successful response always includes the committed publication snapshot;
//! a failed response compensates both durable files and live rows.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext, OwnerKind};
use crate::daemon::ability::manifest::{AbilityExec, AbilityManifest};
use crate::daemon::ability::CallMode;
use crate::daemon::axon_bridge::hot_agent_registrar::{block_on_hot_registrar, HotAgentRegistrar};
use crate::daemon::execution::mission::directory::{AgentDirectory, ABILITY_MANIFEST_SUFFIX};
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, AgentRegisteredAgentLoadError, AgentRegisteredWorkspaceLookupError,
};
use crate::daemon::persistence::agent_registry::{self as agents, AgentEntry};
use crate::daemon::persistence::config;

use super::lifecycle::SharedHotRegistrarCell;

pub const ABILITY_PUT_AGENT_ABILITY: &str =
    crate::daemon::ability::names::agents::AGENT_ABILITY_PUT;
pub const DESCRIPTION: &str = "Atomically persist executable hosted-Agent ability manifests and commit their live runtime publication.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoringState {
    Prepared,
    DurableApplied,
    RuntimeSynchronized,
    PublicationVerified,
    Committed,
    RollingBack,
    RolledBack,
    PartialFailure,
}

impl std::fmt::Display for AuthoringState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Prepared => "prepared",
            Self::DurableApplied => "durable_applied",
            Self::RuntimeSynchronized => "runtime_synchronized",
            Self::PublicationVerified => "publication_verified",
            Self::Committed => "committed",
            Self::RollingBack => "rolling_back",
            Self::RolledBack => "rolled_back",
            Self::PartialFailure => "partial_failure",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConflictPolicy {
    #[default]
    Reject,
    RetainSameBinding,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PutAgentAbilityArgs {
    name: String,
    manifests_toml: Vec<String>,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    conflict_policy: ConflictPolicy,
}

#[derive(Debug)]
struct PreparedManifest {
    manifest: AbilityManifest,
    body: String,
    path: PathBuf,
    prior_bytes: Option<Vec<u8>>,
}

#[derive(Debug)]
struct ExpectedPublication {
    name: String,
    version: String,
    call_mode: CallMode,
    input_schema: Value,
    output_schema: Value,
}

impl ExpectedPublication {
    fn from_manifest(manifest: &AbilityManifest) -> Self {
        let call_mode = if matches!(manifest.exec(), Some(AbilityExec::HostStream(_))) {
            CallMode::Stream
        } else {
            CallMode::Rpc
        };
        Self {
            name: manifest.name().to_string(),
            version: manifest.descriptor_version().to_string(),
            call_mode,
            input_schema: manifest.input_schema().clone(),
            output_schema: manifest.output_schema().cloned().unwrap_or(Value::Null),
        }
    }

    fn matches(&self, descriptor: &crate::daemon::ability::descriptors::AbilityDescriptor) -> bool {
        descriptor.public_name() == self.name
            && descriptor.version == self.version
            && descriptor.call_mode() == self.call_mode
            && descriptor.input_schema() == &self.input_schema
            && descriptor.output_receipt_schema() == &self.output_schema
    }
}

#[derive(Debug)]
struct DurableManifestBatch {
    abilities_dir: PathBuf,
    directory_preexisted: bool,
    writes: Vec<PreparedManifest>,
    skipped: Vec<String>,
    applied: usize,
}

impl DurableManifestBatch {
    fn prepare(
        directory: &AgentDirectory,
        manifests: Vec<AbilityManifest>,
        overwrite: bool,
        conflict_policy: ConflictPolicy,
    ) -> anyhow::Result<Self> {
        let abilities_dir = directory.abilities_dir();
        let directory_preexisted = abilities_dir.is_dir();
        let mut writes = Vec::new();
        let mut skipped = Vec::new();

        for manifest in manifests {
            let path =
                abilities_dir.join(format!("{}{}", manifest.name(), ABILITY_MANIFEST_SUFFIX));
            let body = manifest.to_toml_string()?;
            let prior_bytes = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(anyhow::anyhow!(
                        "agent.ability.put: read existing manifest {}: {error}",
                        path.display()
                    ));
                }
            };

            if let Some(prior) = prior_bytes.as_deref() {
                if prior == body.as_bytes() {
                    skipped.push(manifest.name().to_string());
                    continue;
                }
                if !overwrite {
                    let retain = matches!(conflict_policy, ConflictPolicy::RetainSameBinding)
                        && existing_binding_matches(prior, &manifest);
                    if retain {
                        skipped.push(manifest.name().to_string());
                        continue;
                    }
                    anyhow::bail!(
                        "agent.ability.put: refusing to overwrite existing ability manifest {}; pass --overwrite to replace it",
                        path.display()
                    );
                }
            }

            writes.push(PreparedManifest {
                manifest,
                body,
                path,
                prior_bytes,
            });
        }

        Ok(Self {
            abilities_dir,
            directory_preexisted,
            writes,
            skipped,
            applied: 0,
        })
    }

    fn apply(&mut self) -> anyhow::Result<()> {
        if self.writes.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.abilities_dir).map_err(|error| {
            anyhow::anyhow!(
                "agent.ability.put: create abilities directory {}: {error}",
                self.abilities_dir.display()
            )
        })?;
        for prepared in &self.writes {
            // Mark before the atomic write: a post-rename directory-sync error
            // may report failure after the new bytes became visible.
            self.applied += 1;
            config::atomic_write(&prepared.path, prepared.body.as_bytes()).map_err(|error| {
                anyhow::anyhow!(
                    "agent.ability.put: write manifest {}: {error}",
                    prepared.path.display()
                )
            })?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        for prepared in self.writes[..self.applied].iter().rev() {
            let result = match prepared.prior_bytes.as_deref() {
                Some(prior) => {
                    config::atomic_write(&prepared.path, prior).map_err(anyhow::Error::from)
                }
                None => match std::fs::remove_file(&prepared.path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                },
            };
            if let Err(error) = result {
                failures.push(format!("restore {}: {error:#}", prepared.path.display()));
            }
        }
        if !self.directory_preexisted && self.abilities_dir.exists() {
            match std::fs::remove_dir(&self.abilities_dir) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) => failures.push(format!(
                    "remove created abilities directory {}: {error}",
                    self.abilities_dir.display()
                )),
            }
        }
        failures
    }

    fn written_names(&self) -> Vec<String> {
        self.writes
            .iter()
            .map(|prepared| prepared.manifest.name().to_string())
            .collect()
    }
}

pub fn register(registry: &mut AxonAbilityCatalog, hot_registrar: Arc<SharedHotRegistrarCell>) {
    let manifest = command_manifest();
    registry.register_rpc_with_envelope_and_spec(
        ABILITY_PUT_AGENT_ABILITY,
        OwnerKind::Device,
        manifest,
        Arc::new(move |envelope: EnvelopeContext, args: Value| {
            put_agent_abilities_handler(envelope, args, &hot_registrar)
        }),
    );
}

fn put_agent_abilities_handler(
    envelope: EnvelopeContext,
    args: Value,
    hot_registrar: &SharedHotRegistrarCell,
) -> anyhow::Result<Value> {
    authorize_local_authoring(&envelope)?;
    let request: PutAgentAbilityArgs = serde_json::from_value(args)
        .map_err(|error| anyhow::anyhow!("agent.ability.put: invalid request: {error}"))?;
    let name = request.name.trim();
    agents::validate_agent_name(name)
        .map_err(|error| anyhow::anyhow!("agent.ability.put: invalid agent name: {error:#}"))?;
    if request.manifests_toml.is_empty() {
        anyhow::bail!("agent.ability.put: `manifests_toml` must contain at least one manifest");
    }

    let registrar = require_ready_registrar(hot_registrar)?;
    let _mutation_guard = authoring_lock();
    let registered = match AgentAggregateRepository::load_registered_agent(
        name,
        "agent.ability.put",
    ) {
        Ok(registered) => registered,
        Err(AgentRegisteredAgentLoadError::RegistryUnreadable { source }) => {
            return Err(anyhow::anyhow!(
                "agent.ability.put: load agent registry: {source:#}"
            ));
        }
        Err(AgentRegisteredAgentLoadError::Lookup(
            AgentRegisteredWorkspaceLookupError::Missing { .. },
        )) => {
            anyhow::bail!("agent.ability.put: agent {name:?} is not registered");
        }
        Err(AgentRegisteredAgentLoadError::Lookup(
            AgentRegisteredWorkspaceLookupError::InvalidWorkspace { .. },
        )) => {
            anyhow::bail!(
                "agent.ability.put: registered agent {name:?} has no explicit root_path; refusing to infer a mutation target"
            );
        }
    };
    let entry = registered.entry().clone();
    let root = registered.workspace().root_path().to_path_buf();
    if !root.is_dir() {
        anyhow::bail!(
            "agent.ability.put: agent {name:?} has no on-disk root at {}",
            root.display()
        );
    }
    let directory = AgentDirectory::open(&root).map_err(|error| {
        anyhow::anyhow!(
            "agent.ability.put: open Agent directory {}: {error:#}",
            root.display()
        )
    })?;
    let manifests = parse_manifests(request.manifests_toml)?;
    let expected_publication: Vec<ExpectedPublication> = manifests
        .iter()
        .map(ExpectedPublication::from_manifest)
        .collect();
    let mut batch = DurableManifestBatch::prepare(
        &directory,
        manifests,
        request.overwrite,
        request.conflict_policy,
    )?;
    let mut state = AuthoringState::Prepared;

    if let Err(error) = batch.apply() {
        return Err(authoring_failure(
            &mut state,
            &mut batch,
            None,
            name,
            &entry,
            format!("apply durable manifests: {error:#}"),
        ));
    }
    state = AuthoringState::DurableApplied;

    let sync_outcome = block_on_hot_registrar({
        let registrar = Arc::clone(&registrar);
        let name = name.to_string();
        let entry = entry.clone();
        async move {
            registrar
                .register_agent_replacing(&name, &entry, Some(&entry))
                .await
        }
    });
    let sync_outcome = match sync_outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(authoring_failure(
                &mut state,
                &mut batch,
                Some(&registrar),
                name,
                &entry,
                format!("synchronize live capability rows: {error}"),
            ));
        }
    };
    state = AuthoringState::RuntimeSynchronized;

    let publication = registrar.publication_snapshot().map_err(|error| {
        authoring_failure(
            &mut state,
            &mut batch,
            Some(&registrar),
            name,
            &entry,
            format!("capture committed publication: {error}"),
        )
    })?;
    let owner_descriptors = publication.hosted_agent_descriptors(name);
    for expected in &expected_publication {
        if !owner_descriptors
            .iter()
            .any(|descriptor| expected.matches(descriptor))
        {
            return Err(authoring_failure(
                &mut state,
                &mut batch,
                Some(&registrar),
                name,
                &entry,
                format!(
                    "live publication does not match committed ability {}.{}@{} ({})",
                    name,
                    expected.name,
                    expected.version,
                    expected.call_mode.as_str()
                ),
            ));
        }
    }
    state = AuthoringState::PublicationVerified;
    let owner_ura = owner_descriptors
        .first()
        .map(|descriptor| descriptor.owner_ura.clone())
        .ok_or_else(|| {
            authoring_failure(
                &mut state,
                &mut batch,
                Some(&registrar),
                name,
                &entry,
                "live publication omitted the hosted Agent owner".to_string(),
            )
        })?;
    let publication = owner_descriptors
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>();
    let publication = match publication {
        Ok(publication) => publication,
        Err(error) => {
            return Err(authoring_failure(
                &mut state,
                &mut batch,
                Some(&registrar),
                name,
                &entry,
                format!("encode committed publication: {error}"),
            ));
        }
    };

    state = AuthoringState::Committed;
    Ok(json!({
        "ok": true,
        "state": state.to_string(),
        "agent_name": name,
        "owner_ura": owner_ura,
        "root_path": root,
        "written": batch.written_names(),
        "skipped": batch.skipped,
        "runtime_registered": sync_outcome.registered,
        "runtime_replaced": sync_outcome.replaced,
        "runtime_removed": sync_outcome.removed,
        "publication": publication,
    }))
}

fn parse_manifests(bodies: Vec<String>) -> anyhow::Result<Vec<AbilityManifest>> {
    let mut names = BTreeSet::new();
    let mut manifests = Vec::with_capacity(bodies.len());
    for body in bodies {
        let manifest = AbilityManifest::from_toml_str(&body)
            .map_err(|error| anyhow::anyhow!("agent.ability.put: invalid manifest: {error:#}"))?;
        if manifest.exec().is_none() {
            anyhow::bail!(
                "agent.ability.put: ability {:?} has no executable binding and cannot enter the live capability catalog",
                manifest.name()
            );
        }
        if matches!(manifest.name(), "chat" | "discover" | "invoke") {
            anyhow::bail!(
                "agent.ability.put: ability name {:?} is reserved by the hosted-Agent runtime",
                manifest.name()
            );
        }
        if !names.insert(manifest.name().to_string()) {
            anyhow::bail!(
                "agent.ability.put: duplicate manifest name {:?} in one transaction",
                manifest.name()
            );
        }
        manifests.push(manifest);
    }
    Ok(manifests)
}

fn existing_binding_matches(prior: &[u8], proposed: &AbilityManifest) -> bool {
    std::str::from_utf8(prior)
        .ok()
        .and_then(|body| AbilityManifest::from_toml_str(body).ok())
        .and_then(|manifest| manifest.exec().cloned())
        == proposed.exec().cloned()
}

fn authorize_local_authoring(envelope: &EnvelopeContext) -> anyhow::Result<()> {
    let callee = crate::core::ura::parse_ura(envelope.callee()).map_err(|error| {
        anyhow::anyhow!(
            "agent.ability.put: invalid daemon callee {:?}: {error}",
            envelope.callee()
        )
    })?;
    if callee.kind != crate::core::ura::URAKind::Device {
        anyhow::bail!("agent.ability.put: authoring must target the local Device authority");
    }
    if envelope.caller() != envelope.callee()
        && envelope.caller() != crate::core::ura::LOCAL_SYSTEM_AGENT_URA
    {
        anyhow::bail!(
            "agent.ability.put: caller {:?} is not authorized to mutate local Agent abilities",
            envelope.caller()
        );
    }
    Ok(())
}

fn require_ready_registrar(
    cell: &SharedHotRegistrarCell,
) -> anyhow::Result<Arc<HotAgentRegistrar>> {
    let registrar = cell
        .get()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("agent.ability.put: hot-Agent registrar is not wired"))?;
    registrar.require_ready().map_err(|error| {
        anyhow::anyhow!("agent.ability.put: registrar precondition failed: {error}")
    })?;
    Ok(registrar)
}

fn authoring_failure(
    state: &mut AuthoringState,
    batch: &mut DurableManifestBatch,
    registrar: Option<&Arc<HotAgentRegistrar>>,
    name: &str,
    entry: &AgentEntry,
    cause: String,
) -> anyhow::Error {
    let failed_state = *state;
    *state = AuthoringState::RollingBack;
    let mut failures = batch.rollback();
    if let Some(registrar) = registrar {
        let restore = block_on_hot_registrar({
            let registrar = Arc::clone(registrar);
            let name = name.to_string();
            let entry = entry.clone();
            async move { registrar.register_agent(&name, &entry).await }
        });
        if let Err(error) = restore {
            failures.push(format!("restore live capability rows: {error}"));
        }
    }
    *state = if failures.is_empty() {
        AuthoringState::RolledBack
    } else {
        AuthoringState::PartialFailure
    };
    let rollback = if failures.is_empty() {
        "completed".to_string()
    } else {
        format!("partial({})", failures.join("; "))
    };
    anyhow::anyhow!(
        "agent.ability.put failed in state {failed_state}: {cause}; rollback={rollback}"
    )
}

fn authoring_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name", "manifests_toml"],
        "properties": {
            "name": {"type": "string", "minLength": 1},
            "manifests_toml": {
                "type": "array",
                "minItems": 1,
                "items": {"type": "string", "minLength": 1}
            },
            "overwrite": {"type": "boolean"},
            "conflict_policy": {
                "type": "string",
                "enum": ["reject", "retain_same_binding"]
            }
        },
        "additionalProperties": false
    })
}

fn command_manifest() -> AbilityManifest {
    AbilityManifest::new(
        "put",
        DESCRIPTION,
        input_schema(),
    )
    .expect("agent.ability.put command manifest is static and valid")
    .with_output_schema(json!({
        "type": "object",
        "required": ["ok", "state", "agent_name", "owner_ura", "written", "skipped", "publication"],
        "properties": {
            "ok": {"type": "boolean"},
            "state": {"type": "string"},
            "agent_name": {"type": "string"},
            "owner_ura": {"type": "string"},
            "root_path": {"type": "string"},
            "written": {"type": "array", "items": {"type": "string"}},
            "skipped": {"type": "array", "items": {"type": "string"}},
            "publication": {"type": "array"}
        },
        "additionalProperties": true
    }))
    .expect("agent.ability.put output schema is static and valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent::spec::{AgentSpec, RuntimeKind};
    use crate::daemon::ability::manifest::AbilityExec;
    use crate::daemon::execution::mission::directory::Location;

    fn shell_manifest(name: &str, command: &str) -> AbilityManifest {
        AbilityManifest::new(name, name, json!({"type": "object"}))
            .unwrap()
            .with_exec(AbilityExec::Shell(
                crate::daemon::ability::manifest::ShellExec {
                    argv: vec![command.to_string()],
                    stdout: None,
                    sandbox: None,
                },
            ))
            .unwrap()
    }

    #[test]
    fn authoring_authority_is_local_device_only() {
        let device = crate::core::ura::device_ura("authoring-test", "device-1");
        let local = EnvelopeContext::for_test_targeted_ability(
            crate::core::ura::LOCAL_SYSTEM_AGENT_URA,
            &device,
            ABILITY_PUT_AGENT_ABILITY,
            &device,
        );
        authorize_local_authoring(&local).expect("local system may author Device abilities");

        let foreign = EnvelopeContext::for_test_targeted_ability(
            crate::core::ura::agent_ura("authoring-test", "other", "agent"),
            &device,
            ABILITY_PUT_AGENT_ABILITY,
            &device,
        );
        assert!(authorize_local_authoring(&foreign)
            .expect_err("foreign Agent must not author local abilities")
            .to_string()
            .contains("not authorized"));

        let hub = crate::core::ura::hub_ura("authoring-test");
        let wrong_owner =
            EnvelopeContext::for_test_targeted_ability(&hub, &hub, ABILITY_PUT_AGENT_ABILITY, &hub);
        assert!(authorize_local_authoring(&wrong_owner)
            .expect_err("Hub target must not mutate hosted-Agent files")
            .to_string()
            .contains("local Device authority"));
    }

    #[test]
    fn parse_manifests_rejects_duplicate_names_before_persistence() {
        let manifest = shell_manifest("echo", "echo");
        let body = manifest.to_toml_string().unwrap();
        let error = parse_manifests(vec![body.clone(), body]).expect_err("duplicate must fail");
        assert!(error.to_string().contains("duplicate manifest name"));
    }

    #[test]
    fn parse_manifests_rejects_non_executable_declarations() {
        let body =
            AbilityManifest::new("declared_only", "metadata only", json!({"type": "object"}))
                .unwrap()
                .to_toml_string()
                .unwrap();
        let error = parse_manifests(vec![body]).expect_err("declaration must not publish");
        assert!(error.to_string().contains("no executable binding"));
    }

    #[test]
    fn parse_manifests_rejects_hosted_runtime_reserved_names() {
        let body = shell_manifest("chat", "echo").to_toml_string().unwrap();
        let error = parse_manifests(vec![body]).expect_err("reserved name must fail");
        assert!(error
            .to_string()
            .contains("reserved by the hosted-Agent runtime"));
    }

    #[test]
    fn durable_batch_rollback_restores_replaced_file_and_removes_created_file() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let directory = AgentDirectory::create(
            &Location::Global {
                name: "authoring-rollback".to_string(),
            },
            AgentSpec::new("authoring-rollback", RuntimeKind::ClaudeCode),
        )
        .unwrap();
        let abilities_dir = directory.abilities_dir();
        let replaced_path = abilities_dir.join(format!("echo{ABILITY_MANIFEST_SUFFIX}"));
        let created_path = abilities_dir.join(format!("summarize{ABILITY_MANIFEST_SUFFIX}"));
        let original = shell_manifest("echo", "old-command")
            .to_toml_string()
            .unwrap();
        config::atomic_write(&replaced_path, original.as_bytes()).unwrap();

        let mut batch = DurableManifestBatch::prepare(
            &directory,
            vec![
                shell_manifest("echo", "new-command"),
                shell_manifest("summarize", "summarize-command"),
            ],
            true,
            ConflictPolicy::Reject,
        )
        .unwrap();
        batch.apply().unwrap();
        assert_ne!(std::fs::read_to_string(&replaced_path).unwrap(), original);
        assert!(created_path.is_file());

        assert!(batch.rollback().is_empty());
        assert_eq!(std::fs::read_to_string(replaced_path).unwrap(), original);
        assert!(!created_path.exists());
    }

    #[test]
    fn retain_same_binding_keeps_existing_manifest_bytes() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let directory = AgentDirectory::create(
            &Location::Global {
                name: "authoring-retain".to_string(),
            },
            AgentSpec::new("authoring-retain", RuntimeKind::ClaudeCode),
        )
        .unwrap();
        let path = directory
            .abilities_dir()
            .join(format!("echo{ABILITY_MANIFEST_SUFFIX}"));
        let original = shell_manifest("echo", "same-command")
            .to_toml_string()
            .unwrap();
        config::atomic_write(&path, original.as_bytes()).unwrap();

        let proposed = AbilityManifest::new(
            "echo",
            "updated metadata must not rewrite a retained binding",
            json!({"type": "object", "properties": {"value": {"type": "string"}}}),
        )
        .unwrap()
        .with_exec(AbilityExec::Shell(
            crate::daemon::ability::manifest::ShellExec {
                argv: vec!["same-command".to_string()],
                stdout: None,
                sandbox: None,
            },
        ))
        .unwrap();
        let mut batch = DurableManifestBatch::prepare(
            &directory,
            vec![proposed],
            false,
            ConflictPolicy::RetainSameBinding,
        )
        .unwrap();

        assert_eq!(batch.skipped, vec!["echo"]);
        assert!(batch.written_names().is_empty());
        batch.apply().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }
}
