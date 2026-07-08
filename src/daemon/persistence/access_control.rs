// EasyNet CLI — RFC-014 access-control persistence
// =================================================
//
// File: src/daemon/persistence/access_control.rs
// Description: Text-backed durable policy profile for PermissionGrant,
//              PermissionRequest, AuthorityProof, and audit records.
//
// Protocol Responsibility:
// Own daemon-local RFC-014 policy state without reading keyring secret
// material and without creating a second policy language.
//
// Implementation Approach:
// Canonical JSON Lines are the durable source of truth. The in-memory maps are
// rebuilt by replay and are never more authoritative than the journal.
//
// Usage Contract:
// Mutations must go through this store so idempotency, hash chaining,
// revocation monotonicity, and unsupported-constraint rejection stay intact.
//
// Architectural Position:
// Daemon persistence layer. Admission and governance abilities depend on this
// model; SDKs and product repositories do not write these files directly.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::invocation::admission::authority_proof::AuthorityProof;
use crate::daemon::invocation::admission::decision::{
    AccessAction, PermissionRequest, PermissionRequestStatus,
};
use crate::daemon::invocation::admission::grant_matcher::{
    PermissionConstraints, PermissionEffect, PermissionGrant, PermissionGrantLifetime,
    PermissionGrantState,
};

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};

const STORE_DIR: &str = "access-control";
const MANIFEST_FILE: &str = "policy_store.toml";
const GRANTS_FILE: &str = "grants.jsonl";
const REQUESTS_FILE: &str = "permission_requests.jsonl";
const PROOFS_FILE: &str = "authority_proofs.jsonl";
const AUDIT_FILE: &str = "audit.jsonl";
const RECORD_PROFILE: &str = "canonical-json-v0";
const STORE_FORMAT: &str = "easynet-rfc014-policy-store-v0";
const SCHEMA_VERSION: u64 = 1;
const ZERO_HASH: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessControlStoreManifest {
    pub policy_store: PolicyStoreSection,
    pub canonicalization: CanonicalizationSection,
    pub files: PolicyStoreFiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyStoreSection {
    pub format: String,
    pub schema_version: u64,
    pub owner_user_id: String,
    pub created_at: String,
    pub last_compacted_at: String,
    pub head_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalizationSection {
    pub record_profile: String,
    pub hash_algorithm: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyStoreFiles {
    pub grants: String,
    pub permission_requests: String,
    pub authority_proofs: String,
    pub audit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantAuditRecord {
    pub audit_record_id: String,
    pub grant_id: String,
    pub mutation: GrantMutation,
    pub owner_user_id: String,
    pub principal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    pub actions: Vec<AccessAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ability_ura_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_ura_pattern: Option<String>,
    pub effect: PermissionEffect,
    pub actor_ura: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub previous_grant_hash: String,
    pub new_grant_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantMutation {
    Created,
    Revoked,
    Expired,
    Edited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityBindingGrantResult {
    pub grant: PermissionGrant,
    pub idempotent_replay: bool,
    pub audit_record_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequestResolutionResult {
    pub request: PermissionRequest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_grant: Option<PermissionGrant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority_proof: Option<AuthorityProof>,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JournalRecord {
    record_kind: RecordKind,
    schema_version: u64,
    sequence: u64,
    owner_user_id: String,
    record_id: String,
    operation: RecordOperation,
    payload: Value,
    previous_record_hash: String,
    record_hash: String,
    created_at: String,
    actor_ura: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecordKind {
    PermissionGrant,
    PermissionRequest,
    AuthorityProof,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecordOperation {
    Created,
    Revoked,
    Expired,
    Edited,
    Approved,
    Denied,
    Cancelled,
    Consumed,
}

#[derive(Debug)]
pub struct AccessControlStore {
    root: PathBuf,
    manifest: AccessControlStoreManifest,
    grants: BTreeMap<String, PermissionGrant>,
    requests: BTreeMap<String, PermissionRequest>,
    proofs: BTreeMap<String, AuthorityProof>,
    consumed_proof_ids: BTreeSet<String>,
    audit: BTreeMap<String, GrantAuditRecord>,
    last_sequence: u64,
    head_hash: String,
}

impl AccessControlStore {
    pub fn open_or_create(owner_user_id: impl Into<String>) -> anyhow::Result<Self> {
        let owner_user_id = owner_user_id.into();
        Self::open_or_create_at(policy_store_dir_for_owner(&owner_user_id), owner_user_id)
    }

    pub fn open_or_create_at(
        root: impl Into<PathBuf>,
        owner_user_id: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let root = root.into();
        let owner_user_id = owner_user_id.into();
        fs::create_dir_all(&root)?;
        ensure_owner_private_dir(&root)?;

        let manifest_path = root.join(MANIFEST_FILE);
        let manifest = if manifest_path.exists() {
            let raw = fs::read_to_string(&manifest_path)?;
            toml::from_str::<AccessControlStoreManifest>(&raw)?
        } else {
            let now = now_rfc3339();
            let manifest = AccessControlStoreManifest {
                policy_store: PolicyStoreSection {
                    format: STORE_FORMAT.to_string(),
                    schema_version: SCHEMA_VERSION,
                    owner_user_id: owner_user_id.clone(),
                    created_at: now.clone(),
                    last_compacted_at: now,
                    head_hash: ZERO_HASH.to_string(),
                },
                canonicalization: CanonicalizationSection {
                    record_profile: RECORD_PROFILE.to_string(),
                    hash_algorithm: "sha256".to_string(),
                },
                files: PolicyStoreFiles {
                    grants: GRANTS_FILE.to_string(),
                    permission_requests: REQUESTS_FILE.to_string(),
                    authority_proofs: PROOFS_FILE.to_string(),
                    audit: AUDIT_FILE.to_string(),
                },
            };
            write_manifest(&manifest_path, &manifest)?;
            manifest
        };
        validate_manifest(&manifest, &owner_user_id)?;

        let mut store = Self {
            root,
            manifest,
            grants: BTreeMap::new(),
            requests: BTreeMap::new(),
            proofs: BTreeMap::new(),
            consumed_proof_ids: BTreeSet::new(),
            audit: BTreeMap::new(),
            last_sequence: 0,
            head_hash: ZERO_HASH.to_string(),
        };
        store.replay()?;
        Ok(store)
    }

    #[must_use]
    pub fn grants(&self) -> Vec<PermissionGrant> {
        self.grants.values().cloned().collect()
    }

    #[must_use]
    pub fn requests(&self) -> Vec<PermissionRequest> {
        self.requests.values().cloned().collect()
    }

    #[must_use]
    pub fn proofs(&self) -> Vec<AuthorityProof> {
        self.proofs.values().cloned().collect()
    }

    pub fn create_grant(
        &mut self,
        mut grant: PermissionGrant,
        actor_ura: &str,
    ) -> anyhow::Result<AuthorityBindingGrantResult> {
        normalize_grant_for_create(&mut grant)?;
        validate_grant(&grant)?;
        grant.state = PermissionGrantState::Active;
        let idempotency_key = grant_idempotency_key(&grant)?;
        if let Some(existing) = self.grants.values().find(|existing| {
            existing.state == PermissionGrantState::Active
                && grant_idempotency_key(existing).ok().as_deref() == Some(idempotency_key.as_str())
        }) {
            return Ok(AuthorityBindingGrantResult {
                grant: existing.clone(),
                idempotent_replay: true,
                audit_record_id: format!("audit-idempotent-{}", existing.grant_id),
            });
        }
        if self.grants.contains_key(&grant.grant_id) {
            anyhow::bail!("grant_id `{}` already exists", grant.grant_id);
        }
        let audit = audit_record(
            &grant,
            GrantMutation::Created,
            actor_ura,
            ZERO_HASH,
            grant_hash(&grant)?.as_str(),
        )?;
        self.append(
            RecordKind::PermissionGrant,
            &grant.grant_id,
            RecordOperation::Created,
            serde_json::to_value(&grant)?,
            actor_ura,
        )?;
        self.append(
            RecordKind::Audit,
            &audit.audit_record_id,
            RecordOperation::Created,
            serde_json::to_value(&audit)?,
            actor_ura,
        )?;
        self.grants.insert(grant.grant_id.clone(), grant.clone());
        self.audit
            .insert(audit.audit_record_id.clone(), audit.clone());
        Ok(AuthorityBindingGrantResult {
            grant,
            idempotent_replay: false,
            audit_record_id: audit.audit_record_id,
        })
    }

    pub fn revoke_grant(
        &mut self,
        grant_id: &str,
        owner_user_id: &str,
        actor_ura: &str,
        reason: Option<String>,
    ) -> anyhow::Result<PermissionGrant> {
        let existing = self
            .grants
            .get(grant_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("grant_id `{grant_id}` not found"))?;
        if existing.owner_user_id != owner_user_id {
            anyhow::bail!("grant `{grant_id}` does not belong to owner `{owner_user_id}`");
        }
        if existing.state == PermissionGrantState::Revoked {
            return Ok(existing);
        }
        let previous_hash = grant_hash(&existing)?;
        let mut revoked = existing;
        revoked.state = PermissionGrantState::Revoked;
        revoked.revoked_at = Some(now_rfc3339());
        revoked.updated_at = revoked.revoked_at.clone();
        revoked.reason = reason;
        let new_hash = grant_hash(&revoked)?;
        let audit = audit_record(
            &revoked,
            GrantMutation::Revoked,
            actor_ura,
            &previous_hash,
            &new_hash,
        )?;
        self.append(
            RecordKind::PermissionGrant,
            &revoked.grant_id,
            RecordOperation::Revoked,
            serde_json::to_value(&revoked)?,
            actor_ura,
        )?;
        self.append(
            RecordKind::Audit,
            &audit.audit_record_id,
            RecordOperation::Revoked,
            serde_json::to_value(&audit)?,
            actor_ura,
        )?;
        self.grants
            .insert(revoked.grant_id.clone(), revoked.clone());
        self.audit.insert(audit.audit_record_id.clone(), audit);
        Ok(revoked)
    }

    pub fn upsert_permission_request(
        &mut self,
        request: PermissionRequest,
        actor_ura: &str,
    ) -> anyhow::Result<PermissionRequest> {
        if request.status != PermissionRequestStatus::Pending {
            anyhow::bail!("policy.request.create requires a pending PermissionRequest");
        }
        validate_request_transition(None, &request)?;
        if let Some(existing) = self.requests.values().find(|existing| {
            existing.status == PermissionRequestStatus::Pending
                && permission_request_idempotency_key(existing)
                    == permission_request_idempotency_key(&request)
        }) {
            return Ok(existing.clone());
        }
        self.append(
            RecordKind::PermissionRequest,
            &request.request_id,
            RecordOperation::Created,
            serde_json::to_value(&request)?,
            actor_ura,
        )?;
        self.requests
            .insert(request.request_id.clone(), request.clone());
        Ok(request)
    }

    pub fn resolve_permission_request(
        &mut self,
        mut request: PermissionRequest,
        actor_ura: &str,
    ) -> anyhow::Result<PermissionRequest> {
        if request.status.is_terminal() {
            normalize_terminal_resolution_fields(&mut request, actor_ura);
        }
        let previous = self.requests.get(&request.request_id);
        validate_request_transition(previous, &request)?;
        self.validate_resolution_authority(&request)?;
        let operation = request_transition_operation(previous, &request)?;
        self.append(
            RecordKind::PermissionRequest,
            &request.request_id,
            operation,
            serde_json::to_value(&request)?,
            actor_ura,
        )?;
        self.requests
            .insert(request.request_id.clone(), request.clone());
        Ok(request)
    }
}

fn request_transition_operation(
    previous: Option<&PermissionRequest>,
    request: &PermissionRequest,
) -> anyhow::Result<RecordOperation> {
    match previous {
        None if request.status == PermissionRequestStatus::Pending => Ok(RecordOperation::Created),
        None => {
            anyhow::bail!(
                "initial PermissionRequest record must be pending, got `{:?}`",
                request.status
            )
        }
        Some(previous) if previous.status == PermissionRequestStatus::Pending => {
            match request.status {
                PermissionRequestStatus::Approved => Ok(RecordOperation::Approved),
                PermissionRequestStatus::Denied => Ok(RecordOperation::Denied),
                PermissionRequestStatus::Expired => Ok(RecordOperation::Expired),
                PermissionRequestStatus::Cancelled => Ok(RecordOperation::Cancelled),
                PermissionRequestStatus::Pending => {
                    anyhow::bail!("PermissionRequest cannot remain pending after initial creation")
                }
            }
        }
        Some(previous) => {
            anyhow::bail!(
                "terminal PermissionRequest cannot transition from `{:?}` to `{:?}`",
                previous.status,
                request.status
            )
        }
    }
}

impl AccessControlStore {
    pub fn resolve_permission_request_with_grant(
        &mut self,
        mut request: PermissionRequest,
        grant: PermissionGrant,
        actor_ura: &str,
    ) -> anyhow::Result<PermissionRequestResolutionResult> {
        if request.status != PermissionRequestStatus::Approved {
            anyhow::bail!("grant-backed PermissionRequest resolution requires approved status");
        }
        validate_grant_resolves_request(&grant, &request)?;
        if request.authority_proof_id.is_some() {
            anyhow::bail!(
                "grant-backed PermissionRequest resolution cannot include authority_proof_id"
            );
        }
        if request
            .created_grant_id
            .as_deref()
            .is_some_and(|grant_id| grant_id != grant.grant_id)
        {
            anyhow::bail!("PermissionRequest created_grant_id does not match created grant");
        }
        request.created_grant_id = Some(grant.grant_id.clone());
        request.authority_proof_id = None;
        normalize_terminal_resolution_fields(&mut request, actor_ura);
        let previous = self.requests.get(&request.request_id);
        validate_request_transition(previous, &request)?;

        let grant_result = self.create_grant(grant, actor_ura)?;
        let request = self.resolve_permission_request(request, actor_ura)?;
        Ok(PermissionRequestResolutionResult {
            request,
            created_grant: Some(grant_result.grant),
            authority_proof: None,
            idempotent_replay: grant_result.idempotent_replay,
        })
    }

    pub fn resolve_permission_request_with_authority_proof(
        &mut self,
        mut request: PermissionRequest,
        proof: AuthorityProof,
        actor_ura: &str,
    ) -> anyhow::Result<PermissionRequestResolutionResult> {
        if request.status != PermissionRequestStatus::Approved {
            anyhow::bail!("proof-backed PermissionRequest resolution requires approved status");
        }
        validate_proof_resolves_request(&proof, &request)?;
        if request.created_grant_id.is_some() {
            anyhow::bail!(
                "proof-backed PermissionRequest resolution cannot include created_grant_id"
            );
        }
        if request
            .authority_proof_id
            .as_deref()
            .is_some_and(|proof_id| proof_id != proof.proof_id)
        {
            anyhow::bail!("PermissionRequest authority_proof_id does not match authority proof");
        }
        request.created_grant_id = None;
        request.authority_proof_id = Some(proof.proof_id.clone());
        normalize_terminal_resolution_fields(&mut request, actor_ura);
        let previous = self.requests.get(&request.request_id);
        validate_request_transition(previous, &request)?;

        let proof = self.put_authority_proof(proof, actor_ura)?;
        let request = self.resolve_permission_request(request, actor_ura)?;
        Ok(PermissionRequestResolutionResult {
            request,
            created_grant: None,
            authority_proof: Some(proof),
            idempotent_replay: false,
        })
    }

    pub fn put_authority_proof(
        &mut self,
        proof: AuthorityProof,
        actor_ura: &str,
    ) -> anyhow::Result<AuthorityProof> {
        if self.proofs.contains_key(&proof.proof_id) {
            anyhow::bail!("authority proof `{}` already exists", proof.proof_id);
        }
        if self.consumed_proof_ids.contains(&proof.proof_id) {
            anyhow::bail!("authority proof `{}` was already consumed", proof.proof_id);
        }
        self.append(
            RecordKind::AuthorityProof,
            &proof.proof_id,
            RecordOperation::Created,
            serde_json::to_value(&proof)?,
            actor_ura,
        )?;
        self.proofs.insert(proof.proof_id.clone(), proof.clone());
        Ok(proof)
    }

    pub fn consume_authority_proof_once(
        &mut self,
        proof_id: &str,
        actor_ura: &str,
    ) -> anyhow::Result<AuthorityProof> {
        let proof = self
            .proofs
            .get(proof_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("authority proof `{proof_id}` not found"))?;
        if !is_one_time_authority_proof(&proof) {
            anyhow::bail!("authority proof `{proof_id}` is not a one-time proof");
        }
        if self.consumed_proof_ids.contains(proof_id) {
            anyhow::bail!("authority proof `{proof_id}` was already consumed");
        }
        self.append(
            RecordKind::AuthorityProof,
            proof_id,
            RecordOperation::Consumed,
            serde_json::to_value(&proof)?,
            actor_ura,
        )?;
        self.consumed_proof_ids.insert(proof_id.to_string());
        Ok(proof)
    }

    fn replay(&mut self) -> anyhow::Result<()> {
        let mut records = Vec::new();
        self.collect_records(GRANTS_FILE, &mut records)?;
        self.collect_records(REQUESTS_FILE, &mut records)?;
        self.collect_records(PROOFS_FILE, &mut records)?;
        self.collect_records(AUDIT_FILE, &mut records)?;
        records.sort_by_key(|record| record.sequence);
        let mut seen_sequences = BTreeSet::new();
        for record in records {
            if !seen_sequences.insert(record.sequence) {
                anyhow::bail!("duplicate policy journal sequence {}", record.sequence);
            }
            verify_record_hash(&record)?;
            if record.previous_record_hash != self.head_hash {
                anyhow::bail!(
                    "policy journal hash chain mismatch at sequence {}",
                    record.sequence
                );
            }
            self.last_sequence = self.last_sequence.max(record.sequence);
            self.head_hash = record.record_hash.clone();
            self.apply_record(record)?;
        }
        self.manifest.policy_store.head_hash = self.head_hash.clone();
        write_manifest(&self.root.join(MANIFEST_FILE), &self.manifest)?;
        Ok(())
    }

    fn collect_records(
        &self,
        file_name: &str,
        records: &mut Vec<JournalRecord>,
    ) -> anyhow::Result<()> {
        let path = self.root.join(file_name);
        if !path.exists() {
            return Ok(());
        }
        let file = fs::File::open(&path)?;
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record: JournalRecord = serde_json::from_str(&line)?;
            records.push(record);
        }
        Ok(())
    }

    fn apply_record(&mut self, record: JournalRecord) -> anyhow::Result<()> {
        match record.record_kind {
            RecordKind::PermissionGrant => {
                let grant: PermissionGrant = serde_json::from_value(record.payload)?;
                if self.grants.get(&grant.grant_id).is_some_and(|existing| {
                    existing.state == PermissionGrantState::Revoked
                        && grant.state == PermissionGrantState::Active
                }) {
                    anyhow::bail!("revoked grant `{}` cannot become active", grant.grant_id);
                }
                self.grants.insert(grant.grant_id.clone(), grant);
            }
            RecordKind::PermissionRequest => {
                let request: PermissionRequest = serde_json::from_value(record.payload)?;
                let previous = self.requests.get(&request.request_id);
                validate_request_transition(previous, &request)?;
                self.validate_resolution_authority(&request)?;
                self.requests.insert(request.request_id.clone(), request);
            }
            RecordKind::AuthorityProof => {
                let proof: AuthorityProof = serde_json::from_value(record.payload)?;
                match record.operation {
                    RecordOperation::Consumed => {
                        if !self.proofs.contains_key(&proof.proof_id) {
                            anyhow::bail!(
                                "authority proof `{}` consumed before creation",
                                proof.proof_id
                            );
                        }
                        self.consumed_proof_ids.insert(proof.proof_id);
                    }
                    RecordOperation::Created => {
                        if self.consumed_proof_ids.contains(&proof.proof_id) {
                            anyhow::bail!(
                                "authority proof `{}` recreated after consumption",
                                proof.proof_id
                            );
                        }
                        self.proofs.insert(proof.proof_id.clone(), proof);
                    }
                    _ => {
                        anyhow::bail!(
                            "unsupported authority proof journal operation `{:?}`",
                            record.operation
                        );
                    }
                }
            }
            RecordKind::Audit => {
                let audit: GrantAuditRecord = serde_json::from_value(record.payload)?;
                self.audit.insert(audit.audit_record_id.clone(), audit);
            }
        }
        Ok(())
    }

    fn append(
        &mut self,
        record_kind: RecordKind,
        record_id: &str,
        operation: RecordOperation,
        payload: Value,
        actor_ura: &str,
    ) -> anyhow::Result<()> {
        self.last_sequence += 1;
        let mut record = JournalRecord {
            record_kind,
            schema_version: SCHEMA_VERSION,
            sequence: self.last_sequence,
            owner_user_id: self.manifest.policy_store.owner_user_id.clone(),
            record_id: record_id.to_string(),
            operation,
            payload,
            previous_record_hash: self.head_hash.clone(),
            record_hash: String::new(),
            created_at: now_rfc3339(),
            actor_ura: actor_ura.to_string(),
        };
        record.record_hash = compute_record_hash(&record)?;
        append_jsonl(&self.root.join(file_for_kind(record_kind)), &record)?;
        self.head_hash = record.record_hash.clone();
        self.manifest.policy_store.head_hash = self.head_hash.clone();
        write_manifest(&self.root.join(MANIFEST_FILE), &self.manifest)?;
        Ok(())
    }

    fn validate_resolution_authority(&self, request: &PermissionRequest) -> anyhow::Result<()> {
        if request.status != PermissionRequestStatus::Approved {
            return Ok(());
        }
        match (
            request.created_grant_id.as_deref(),
            request.authority_proof_id.as_deref(),
        ) {
            (Some(grant_id), None) => {
                let grant = self.grants.get(grant_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "approved PermissionRequest references missing grant `{grant_id}`"
                    )
                })?;
                validate_grant_resolves_request(grant, request)?;
            }
            (None, Some(proof_id)) => {
                let proof = self.proofs.get(proof_id).ok_or_else(|| {
                    anyhow::anyhow!(
                        "approved PermissionRequest references missing authority proof `{proof_id}`"
                    )
                })?;
                validate_proof_resolves_request(proof, request)?;
            }
            (None, None) => {
                anyhow::bail!("approved PermissionRequest must create a grant or authority proof");
            }
            (Some(_), Some(_)) => {
                anyhow::bail!(
                    "approved PermissionRequest must be backed by exactly one authority source"
                );
            }
        }
        Ok(())
    }
}

pub fn default_policy_store_dir() -> PathBuf {
    state_dir().join(STORE_DIR)
}

pub fn policy_store_dir_for_owner(owner_user_id: &str) -> PathBuf {
    let safe_owner = owner_user_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    default_policy_store_dir().join(safe_owner)
}

pub fn grant_idempotency_key(grant: &PermissionGrant) -> anyhow::Result<String> {
    let value = json!({
        "owner_user_id": grant.owner_user_id,
        "principal_kind": grant.principal_kind,
        "principal_id": grant.principal_id,
        "token_id": grant.token_id,
        "callee_ura": grant.callee_ura,
        "subject_ura_pattern": grant.subject_ura_pattern,
        "ability_ura_pattern": grant.ability_ura_pattern,
        "actions": grant.actions,
        "constraints": grant.constraints,
        "effect": grant.effect,
        "lifetime": grant.lifetime,
        "expires_at": grant.expires_at,
        "created_by": grant.created_by,
    });
    Ok(sha256_value(&value))
}

fn validate_manifest(
    manifest: &AccessControlStoreManifest,
    owner_user_id: &str,
) -> anyhow::Result<()> {
    if manifest.policy_store.format != STORE_FORMAT
        || manifest.policy_store.schema_version != SCHEMA_VERSION
        || manifest.canonicalization.record_profile != RECORD_PROFILE
        || manifest.canonicalization.hash_algorithm != "sha256"
    {
        anyhow::bail!("unsupported RFC-014 policy store manifest");
    }
    if manifest.policy_store.owner_user_id != owner_user_id {
        anyhow::bail!(
            "policy store owner mismatch: manifest={} requested={}",
            manifest.policy_store.owner_user_id,
            owner_user_id
        );
    }
    Ok(())
}

fn validate_grant(grant: &PermissionGrant) -> anyhow::Result<()> {
    if grant.grant_id.trim().is_empty()
        || grant.owner_user_id.trim().is_empty()
        || grant.principal_id.trim().is_empty()
        || grant.actions.is_empty()
        || grant.created_by.trim().is_empty()
        || grant.created_at.trim().is_empty()
    {
        anyhow::bail!("PermissionGrant requires grant_id, owner_user_id, principal_id, actions, created_by, and created_at");
    }
    reject_broad_pattern(grant.ability_ura_pattern.as_deref())?;
    reject_unenforceable_constraints(grant.constraints.as_ref())?;
    DateTime::parse_from_rfc3339(&grant.created_at)
        .map_err(|_| anyhow::anyhow!("PermissionGrant created_at must be RFC3339"))?;
    if grant
        .expires_at
        .as_deref()
        .is_some_and(|raw| DateTime::parse_from_rfc3339(raw).is_err())
    {
        anyhow::bail!("PermissionGrant expires_at must be RFC3339");
    }
    if grant
        .review_required_after
        .as_deref()
        .is_some_and(|raw| DateTime::parse_from_rfc3339(raw).is_err())
    {
        anyhow::bail!("PermissionGrant review_required_after must be RFC3339");
    }
    if grant
        .last_reviewed_at
        .as_deref()
        .is_some_and(|raw| DateTime::parse_from_rfc3339(raw).is_err())
    {
        anyhow::bail!("PermissionGrant last_reviewed_at must be RFC3339");
    }
    Ok(())
}

fn normalize_grant_for_create(grant: &mut PermissionGrant) -> anyhow::Result<()> {
    if grant.state != PermissionGrantState::Active {
        grant.state = PermissionGrantState::Active;
    }
    if grant.effect == PermissionEffect::Allow
        && grant.lifetime == PermissionGrantLifetime::Permanent
        && grant.review_required_after.is_none()
        && grant.actions.iter().any(|action| {
            matches!(
                action,
                AccessAction::Stream | AccessAction::Manage | AccessAction::Grant
            )
        })
    {
        let deadline = grant
            .default_reconfirmation_deadline()
            .ok_or_else(|| anyhow::anyhow!("PermissionGrant created_at must be RFC3339"))?;
        grant.review_required_after =
            Some(deadline.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    Ok(())
}

fn reject_broad_pattern(pattern: Option<&str>) -> anyhow::Result<()> {
    let Some(pattern) = pattern.map(str::trim) else {
        return Ok(());
    };
    if matches!(
        pattern,
        "*" | "device.*" | "manage.*" | "identity.*" | "authority.*" | "policy.*"
    ) {
        anyhow::bail!("broad ability pattern `{pattern}` requires a later administrative RFC");
    }
    Ok(())
}

fn reject_unenforceable_constraints(
    constraints: Option<&PermissionConstraints>,
) -> anyhow::Result<()> {
    let Some(constraints) = constraints else {
        return Ok(());
    };
    if constraints.args_schema_filter.is_some()
        || constraints.max_duration_ms.is_some()
        || constraints.max_invocations.is_some()
        || constraints.requires_user_present.is_some()
        || !constraints.resource_types.is_empty()
    {
        anyhow::bail!("grant constraints are not yet enforceable by this daemon admission profile");
    }
    Ok(())
}

fn validate_request_transition(
    previous: Option<&PermissionRequest>,
    next: &PermissionRequest,
) -> anyhow::Result<()> {
    if next.request_id.trim().is_empty()
        || next.owner_user_id.trim().is_empty()
        || next.principal_id.trim().is_empty()
        || next.caller_ura.trim().is_empty()
        || next.callee_ura.trim().is_empty()
        || next.subject_ura.trim().is_empty()
        || next.ability_ura.trim().is_empty()
    {
        anyhow::bail!("PermissionRequest identity fields must not be empty");
    }
    if next.requested_lifetimes.is_empty() {
        anyhow::bail!("PermissionRequest requested_lifetimes must not be empty");
    }
    if next
        .nonce
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
        && next
            .canonical_hash
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        anyhow::bail!("PermissionRequest must bind nonce or canonical_hash");
    }
    validate_request_timestamps(next)?;
    request_transition_operation(previous, next)?;
    if next.status == PermissionRequestStatus::Approved
        && next.created_grant_id.is_none()
        && next.authority_proof_id.is_none()
    {
        anyhow::bail!("approved PermissionRequest must create a grant or authority proof");
    }
    if next.created_grant_id.is_some() && next.authority_proof_id.is_some() {
        anyhow::bail!("approved PermissionRequest must be backed by exactly one authority source");
    }
    if next.status != PermissionRequestStatus::Approved
        && (next.created_grant_id.is_some() || next.authority_proof_id.is_some())
    {
        anyhow::bail!("only approved PermissionRequest records may reference authority source ids");
    }
    if next.status.is_terminal() {
        if next
            .resolver_ura
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
            || next
                .resolved_at
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
        {
            anyhow::bail!("terminal PermissionRequest requires resolver_ura and resolved_at");
        }
    }
    Ok(())
}

fn validate_request_timestamps(request: &PermissionRequest) -> anyhow::Result<()> {
    let created_at = DateTime::parse_from_rfc3339(&request.created_at)
        .map_err(|_| anyhow::anyhow!("PermissionRequest created_at must be RFC3339"))?;
    let expires_at = DateTime::parse_from_rfc3339(&request.expires_at)
        .map_err(|_| anyhow::anyhow!("PermissionRequest expires_at must be RFC3339"))?;
    if expires_at <= created_at {
        anyhow::bail!("PermissionRequest expires_at must be after created_at");
    }
    if let Some(raw) = request.resolved_at.as_deref() {
        DateTime::parse_from_rfc3339(raw)
            .map_err(|_| anyhow::anyhow!("PermissionRequest resolved_at must be RFC3339"))?;
    }
    Ok(())
}

fn normalize_terminal_resolution_fields(request: &mut PermissionRequest, actor_ura: &str) {
    if request.resolver_ura.as_deref().map_or(true, str::is_empty) {
        request.resolver_ura = Some(actor_ura.to_string());
    }
    if request.resolved_at.as_deref().map_or(true, str::is_empty) {
        request.resolved_at = Some(now_rfc3339());
    }
}

fn validate_grant_resolves_request(
    grant: &PermissionGrant,
    request: &PermissionRequest,
) -> anyhow::Result<()> {
    if grant.owner_user_id != request.owner_user_id {
        anyhow::bail!("grant owner does not match PermissionRequest owner");
    }
    if grant.principal_kind != request.principal_kind
        || grant.principal_id != request.principal_id
        || grant.token_id != request.token_id
    {
        anyhow::bail!("grant principal does not match PermissionRequest principal");
    }
    if grant.effect != PermissionEffect::Allow || grant.state != PermissionGrantState::Active {
        anyhow::bail!("PermissionRequest approval grant must be active allow");
    }
    if !grant.actions.contains(&request.action) {
        anyhow::bail!("grant actions do not include PermissionRequest action");
    }
    if grant.callee_ura.as_deref() != Some(request.callee_ura.as_str())
        || grant.subject_ura_pattern.as_deref() != Some(request.subject_ura.as_str())
        || grant.ability_ura_pattern.as_deref() != Some(request.ability_ura.as_str())
    {
        anyhow::bail!("grant scope does not exactly match PermissionRequest scope");
    }
    Ok(())
}

fn validate_proof_resolves_request(
    proof: &AuthorityProof,
    request: &PermissionRequest,
) -> anyhow::Result<()> {
    if proof.permission_request_id.as_deref() != Some(request.request_id.as_str()) {
        anyhow::bail!("authority proof does not reference PermissionRequest");
    }
    if proof.owner_user_id != request.owner_user_id
        || proof.principal_kind != request.principal_kind
        || proof.principal_id != request.principal_id
        || proof.token_id != request.token_id
        || proof.callee_ura != request.callee_ura
        || proof.subject_ura != request.subject_ura
        || proof.ability_ura != request.ability_ura
        || proof.action != request.action
    {
        anyhow::bail!("authority proof binding does not match PermissionRequest");
    }
    if proof.nonce.as_deref().is_some() && proof.nonce.as_deref() != request.nonce.as_deref() {
        anyhow::bail!("authority proof nonce does not match PermissionRequest");
    }
    if proof.canonical_hash.as_deref().is_some()
        && proof.canonical_hash.as_deref() != request.canonical_hash.as_deref()
    {
        anyhow::bail!("authority proof canonical hash does not match PermissionRequest");
    }
    if !proof_binds_permission_request_invocation(proof, request) {
        anyhow::bail!("authority proof must bind PermissionRequest nonce or canonical_hash");
    }
    if DateTime::parse_from_rfc3339(&proof.expires_at).is_err() {
        anyhow::bail!("authority proof expires_at must be RFC3339");
    }
    Ok(())
}

fn proof_binds_permission_request_invocation(
    proof: &AuthorityProof,
    request: &PermissionRequest,
) -> bool {
    proof
        .nonce
        .as_deref()
        .map(str::trim)
        .filter(|nonce| !nonce.is_empty())
        .is_some_and(|nonce| Some(nonce) == request.nonce.as_deref())
        || proof
            .canonical_hash
            .as_deref()
            .map(str::trim)
            .filter(|hash| !hash.is_empty())
            .is_some_and(|hash| Some(hash) == request.canonical_hash.as_deref())
}

fn is_one_time_authority_proof(proof: &AuthorityProof) -> bool {
    proof.permission_request_id.is_some() && proof.grant_id.is_none() && proof.session_id.is_none()
}

fn permission_request_idempotency_key(request: &PermissionRequest) -> String {
    sha256_value(&json!({
        "owner_user_id": request.owner_user_id,
        "principal_id": request.principal_id,
        "token_id": request.token_id,
        "callee_ura": request.callee_ura,
        "subject_ura": request.subject_ura,
        "ability_ura": request.ability_ura,
        "action": request.action,
        "canonical_hash": request.canonical_hash,
        "nonce": request.nonce,
    }))
}

fn audit_record(
    grant: &PermissionGrant,
    mutation: GrantMutation,
    actor_ura: &str,
    previous_grant_hash: &str,
    new_grant_hash: &str,
) -> anyhow::Result<GrantAuditRecord> {
    let payload = json!({
        "grant_id": grant.grant_id,
        "mutation": mutation,
        "owner_user_id": grant.owner_user_id,
        "principal_id": grant.principal_id,
        "token_id": grant.token_id,
        "actions": grant.actions,
        "ability_ura_pattern": grant.ability_ura_pattern,
        "subject_ura_pattern": grant.subject_ura_pattern,
        "effect": grant.effect,
        "actor_ura": actor_ura,
        "previous_grant_hash": previous_grant_hash,
        "new_grant_hash": new_grant_hash,
    });
    let audit_record_id = sha256_value(&payload);
    Ok(GrantAuditRecord {
        audit_record_id,
        grant_id: grant.grant_id.clone(),
        mutation,
        owner_user_id: grant.owner_user_id.clone(),
        principal_id: grant.principal_id.clone(),
        token_id: grant.token_id.clone(),
        actions: grant.actions.clone(),
        ability_ura_pattern: grant.ability_ura_pattern.clone(),
        subject_ura_pattern: grant.subject_ura_pattern.clone(),
        effect: grant.effect,
        actor_ura: actor_ura.to_string(),
        timestamp: now_rfc3339(),
        reason: grant.reason.clone(),
        previous_grant_hash: previous_grant_hash.to_string(),
        new_grant_hash: new_grant_hash.to_string(),
    })
}

fn grant_hash(grant: &PermissionGrant) -> anyhow::Result<String> {
    Ok(sha256_value(&serde_json::to_value(grant)?))
}

fn verify_record_hash(record: &JournalRecord) -> anyhow::Result<()> {
    let expected = compute_record_hash(record)?;
    if expected != record.record_hash {
        anyhow::bail!(
            "policy journal record hash mismatch sequence={} expected={} got={}",
            record.sequence,
            expected,
            record.record_hash
        );
    }
    Ok(())
}

fn compute_record_hash(record: &JournalRecord) -> anyhow::Result<String> {
    let mut value = serde_json::to_value(record)?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("record_hash");
    }
    Ok(sha256_value(&value))
}

fn sha256_value(value: &Value) -> String {
    let bytes = crate::daemon::ability::canonical_json_bytes(value);
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn append_jsonl(path: &Path, record: &JournalRecord) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let bytes = crate::daemon::ability::canonical_json_bytes(&serde_json::to_value(record)?);
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

fn write_manifest(path: &Path, manifest: &AccessControlStoreManifest) -> anyhow::Result<()> {
    let toml = toml::to_string_pretty(manifest)?;
    atomic_write_with_permissions(path, toml.as_bytes(), WritePermissions::OwnerReadWrite)
}

fn file_for_kind(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::PermissionGrant => GRANTS_FILE,
        RecordKind::PermissionRequest => REQUESTS_FILE,
        RecordKind::AuthorityProof => PROOFS_FILE,
        RecordKind::Audit => AUDIT_FILE,
    }
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn ensure_owner_private_dir(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::invocation::admission::decision::{
        PermissionLifetime, PrincipalKind, TokenClass,
    };
    use crate::daemon::invocation::admission::grant_matcher::{
        PermissionGrantLifetime, PermissionGrantState,
    };

    fn sample_grant(id: &str) -> PermissionGrant {
        PermissionGrant {
            grant_id: id.to_string(),
            owner_user_id: "alice".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: Some("easynet:///r/test/device/dev".to_string()),
            subject_ura_pattern: Some("easynet:///r/test/device/dev".to_string()),
            ability_ura_pattern: Some("meta.describe".to_string()),
            actions: vec![AccessAction::Read],
            constraints: None,
            effect: PermissionEffect::Allow,
            lifetime: PermissionGrantLifetime::Permanent,
            state: PermissionGrantState::Active,
            expires_at: None,
            review_required_after: None,
            last_reviewed_at: None,
            last_used_at: None,
            created_by: "easynet:///r/test/user/alice".to_string(),
            created_at: "2026-07-09T00:00:00Z".to_string(),
            updated_at: None,
            revoked_at: None,
            reason: None,
        }
    }

    #[test]
    fn grant_create_is_idempotent_and_replay_rebuilds_index() {
        let _home = HomeGuard::new();
        let root = default_policy_store_dir();
        let mut store = AccessControlStore::open_or_create_at(&root, "alice").expect("store");
        let first = store
            .create_grant(sample_grant("grant-1"), "easynet:///r/test/user/alice")
            .expect("create grant");
        let second = store
            .create_grant(sample_grant("grant-2"), "easynet:///r/test/user/alice")
            .expect("idempotent grant");
        assert!(!first.idempotent_replay);
        assert!(second.idempotent_replay);
        assert_eq!(second.grant.grant_id, "grant-1");

        let reopened = AccessControlStore::open_or_create_at(root, "alice").expect("reopen");
        assert_eq!(reopened.grants().len(), 1);
        assert_eq!(reopened.grants()[0].grant_id, "grant-1");
    }

    #[test]
    fn revoke_is_monotonic() {
        let _home = HomeGuard::new();
        let root = default_policy_store_dir();
        let mut store = AccessControlStore::open_or_create_at(&root, "alice").expect("store");
        store
            .create_grant(sample_grant("grant-1"), "easynet:///r/test/user/alice")
            .expect("create grant");
        let revoked = store
            .revoke_grant(
                "grant-1",
                "alice",
                "easynet:///r/test/user/alice",
                Some("operator revoked".to_string()),
            )
            .expect("revoke grant");
        assert_eq!(revoked.state, PermissionGrantState::Revoked);

        let reopened = AccessControlStore::open_or_create_at(root, "alice").expect("reopen");
        assert_eq!(reopened.grants()[0].state, PermissionGrantState::Revoked);
    }

    #[test]
    fn permanent_stream_grant_materializes_default_review_deadline() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let mut grant = sample_grant("grant-stream");
        grant.actions = vec![AccessAction::Stream];
        grant.ability_ura_pattern = Some("terminal.create".to_string());

        let created = store
            .create_grant(grant, "easynet:///r/test/user/alice")
            .expect("create stream grant")
            .grant;

        assert_eq!(
            created.review_required_after.as_deref(),
            Some("2026-10-07T00:00:00Z")
        );
    }

    #[test]
    fn permanent_manage_or_grant_uses_thirty_day_review_deadline() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let mut grant = sample_grant("grant-manage");
        grant.actions = vec![AccessAction::Stream, AccessAction::Manage];
        grant.ability_ura_pattern = Some("device.settings".to_string());

        let created = store
            .create_grant(grant, "easynet:///r/test/user/alice")
            .expect("create manage grant")
            .grant;

        assert_eq!(
            created.review_required_after.as_deref(),
            Some("2026-08-08T00:00:00Z")
        );
    }

    #[test]
    fn grant_timestamps_must_be_rfc3339() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let mut grant = sample_grant("grant-invalid-time");
        grant.created_at = "2026-07-09 00:00:00".to_string();

        let err = store
            .create_grant(grant, "easynet:///r/test/user/alice")
            .expect_err("invalid created_at must fail");
        assert!(err
            .to_string()
            .contains("PermissionGrant created_at must be RFC3339"));
    }

    #[test]
    fn broad_patterns_are_rejected() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let mut grant = sample_grant("grant-1");
        grant.ability_ura_pattern = Some("*".to_string());
        let err = store
            .create_grant(grant, "easynet:///r/test/user/alice")
            .expect_err("broad grant must fail");
        assert!(err.to_string().contains("broad ability pattern"));
    }

    #[test]
    fn approved_request_requires_effective_grant_or_proof() {
        let request = PermissionRequest {
            request_id: "req-1".to_string(),
            owner_user_id: "alice".to_string(),
            caller_ura: "easynet:///r/test/hub".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: "easynet:///r/test/device/dev".to_string(),
            ability_ura: "terminal.create".to_string(),
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:test".to_string()),
            requested_lifetimes: vec![PermissionLifetime::Once],
            status: PermissionRequestStatus::Approved,
            created_at: "2026-07-09T00:00:00Z".to_string(),
            expires_at: "2026-07-09T00:05:00Z".to_string(),
            resolver_ura: Some("easynet:///r/test/user/alice".to_string()),
            resolved_lifetime: None,
            created_grant_id: None,
            authority_proof_id: None,
            resolved_at: Some("2026-07-09T00:01:00Z".to_string()),
            decision_reason: None,
        };
        assert!(validate_request_transition(None, &request).is_err());
        assert!(validate_request_transition(Some(&pending_request()), &request).is_err());

        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let err = store
            .upsert_permission_request(request, "easynet:///r/test/user/alice")
            .expect_err("policy.request.create must not create approved records");
        assert!(err
            .to_string()
            .contains("policy.request.create requires a pending PermissionRequest"));
    }

    #[test]
    fn approved_request_resolution_creates_effective_grant() {
        let _home = HomeGuard::new();
        let root = default_policy_store_dir();
        let mut store = AccessControlStore::open_or_create_at(&root, "alice").expect("store");
        let pending = store
            .upsert_permission_request(pending_request(), "easynet:///r/test/hub")
            .expect("create pending request");
        let mut approved = pending.clone();
        approved.status = PermissionRequestStatus::Approved;
        let grant = grant_for_request("grant-request-1", &approved);

        let result = store
            .resolve_permission_request_with_grant(approved, grant, "easynet:///r/test/user/alice")
            .expect("grant-backed approval");

        assert_eq!(result.request.status, PermissionRequestStatus::Approved);
        assert_eq!(
            result.request.created_grant_id.as_deref(),
            Some("grant-request-1")
        );
        assert!(result.request.authority_proof_id.is_none());
        assert!(result.created_grant.is_some());

        let reopened = AccessControlStore::open_or_create_at(root, "alice").expect("reopen");
        assert_eq!(
            reopened.requests()[0].status,
            PermissionRequestStatus::Approved
        );
        assert_eq!(reopened.grants()[0].grant_id, "grant-request-1");
    }

    #[test]
    fn approved_request_rejects_missing_effective_grant() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let pending = store
            .upsert_permission_request(pending_request(), "easynet:///r/test/hub")
            .expect("create pending request");
        let mut approved = pending;
        approved.status = PermissionRequestStatus::Approved;
        approved.created_grant_id = Some("missing-grant".to_string());

        let err = store
            .resolve_permission_request(approved, "easynet:///r/test/user/alice")
            .expect_err("missing approval grant must fail");
        assert!(err
            .to_string()
            .contains("approved PermissionRequest references missing grant"));
        assert_eq!(store.requests()[0].status, PermissionRequestStatus::Pending);
    }

    #[test]
    fn approved_request_rejects_scope_mismatched_created_grant() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let pending = store
            .upsert_permission_request(pending_request(), "easynet:///r/test/hub")
            .expect("create pending request");
        let mut approved = pending;
        approved.status = PermissionRequestStatus::Approved;
        let mut grant = grant_for_request("grant-request-1", &approved);
        grant.ability_ura_pattern = Some("terminal.*".to_string());

        let err = store
            .resolve_permission_request_with_grant(approved, grant, "easynet:///r/test/user/alice")
            .expect_err("grant scope mismatch must fail");
        assert!(err
            .to_string()
            .contains("grant scope does not exactly match PermissionRequest scope"));
        assert_eq!(store.requests()[0].status, PermissionRequestStatus::Pending);
        assert!(store.grants().is_empty());
    }

    #[test]
    fn approved_request_rejects_unbound_authority_proof() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let pending = store
            .upsert_permission_request(pending_request(), "easynet:///r/test/hub")
            .expect("create pending request");
        let mut approved = pending;
        approved.status = PermissionRequestStatus::Approved;
        let mut proof = proof_for_request("proof-request-1", &approved);
        proof.nonce = None;
        proof.canonical_hash = None;

        let err = store
            .resolve_permission_request_with_authority_proof(
                approved,
                proof,
                "easynet:///r/test/user/alice",
            )
            .expect_err("proof-backed approval must bind invocation identity");
        assert!(err
            .to_string()
            .contains("authority proof must bind PermissionRequest nonce or canonical_hash"));
        assert_eq!(store.requests()[0].status, PermissionRequestStatus::Pending);
        assert!(store.proofs().is_empty());
    }

    #[test]
    fn one_time_authority_proof_consumption_is_durable_and_single_use() {
        let _home = HomeGuard::new();
        let root = default_policy_store_dir();
        let mut store = AccessControlStore::open_or_create_at(&root, "alice").expect("store");
        let pending = store
            .upsert_permission_request(pending_request(), "easynet:///r/test/hub")
            .expect("create pending request");
        let mut approved = pending;
        approved.status = PermissionRequestStatus::Approved;
        let proof = proof_for_request("proof-request-1", &approved);

        store
            .resolve_permission_request_with_authority_proof(
                approved,
                proof,
                "easynet:///r/test/user/alice",
            )
            .expect("proof-backed approval");
        let consumed = store
            .consume_authority_proof_once("proof-request-1", "easynet:///r/test/device/dev")
            .expect("consume proof");
        assert_eq!(consumed.proof_id, "proof-request-1");
        let err = store
            .consume_authority_proof_once("proof-request-1", "easynet:///r/test/device/dev")
            .expect_err("one-time proof must not be reusable");
        assert!(err.to_string().contains("was already consumed"));

        let mut reopened = AccessControlStore::open_or_create_at(root, "alice").expect("reopen");
        let err = reopened
            .consume_authority_proof_once("proof-request-1", "easynet:///r/test/device/dev")
            .expect_err("consumed proof must stay consumed after replay");
        assert!(err.to_string().contains("was already consumed"));
    }

    #[test]
    fn request_creation_rejects_unbounded_prompt_shape() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");

        let mut no_lifetimes = pending_request();
        no_lifetimes.requested_lifetimes.clear();
        let err = store
            .upsert_permission_request(no_lifetimes, "easynet:///r/test/hub")
            .expect_err("prompt must expose requested lifetimes");
        assert!(err
            .to_string()
            .contains("requested_lifetimes must not be empty"));

        let mut no_invocation_binding = pending_request();
        no_invocation_binding.nonce = None;
        no_invocation_binding.canonical_hash = None;
        let err = store
            .upsert_permission_request(no_invocation_binding, "easynet:///r/test/hub")
            .expect_err("prompt must bind invocation identity");
        assert!(err
            .to_string()
            .contains("must bind nonce or canonical_hash"));

        let mut invalid_expiry = pending_request();
        invalid_expiry.expires_at = invalid_expiry.created_at.clone();
        let err = store
            .upsert_permission_request(invalid_expiry, "easynet:///r/test/hub")
            .expect_err("prompt expiry must be bounded");
        assert!(err
            .to_string()
            .contains("expires_at must be after created_at"));
    }

    #[test]
    fn request_lifecycle_allows_one_pending_to_terminal_transition() {
        let _home = HomeGuard::new();
        let mut store = AccessControlStore::open_or_create_at(default_policy_store_dir(), "alice")
            .expect("store");
        let pending = store
            .upsert_permission_request(pending_request(), "easynet:///r/test/hub")
            .expect("create pending request");

        let pending_edit = pending.clone();
        let err = store
            .resolve_permission_request(pending_edit, "easynet:///r/test/user/alice")
            .expect_err("pending edit is not a valid RFC-014 transition");
        assert!(err.to_string().contains("cannot remain pending"));

        let mut denied = pending;
        denied.status = PermissionRequestStatus::Denied;
        denied.decision_reason = Some("owner denied".to_string());
        let denied = store
            .resolve_permission_request(denied, "easynet:///r/test/user/alice")
            .expect("deny request");
        assert_eq!(denied.status, PermissionRequestStatus::Denied);
        assert_eq!(
            denied.resolver_ura.as_deref(),
            Some("easynet:///r/test/user/alice")
        );
        assert!(denied.resolved_at.is_some());

        let mut cancelled = denied;
        cancelled.status = PermissionRequestStatus::Cancelled;
        let err = store
            .resolve_permission_request(cancelled, "easynet:///r/test/user/alice")
            .expect_err("terminal request cannot transition again");
        assert!(err
            .to_string()
            .contains("terminal PermissionRequest cannot transition"));
    }

    fn pending_request() -> PermissionRequest {
        let mut request = PermissionRequest {
            request_id: "req-1".to_string(),
            owner_user_id: "alice".to_string(),
            caller_ura: "easynet:///r/test/hub".to_string(),
            principal_kind: PrincipalKind::Token,
            principal_id: "token-principal".to_string(),
            token_id: Some("token-1".to_string()),
            token_class: Some(TokenClass::HubLink),
            callee_ura: "easynet:///r/test/device/dev".to_string(),
            subject_ura: "easynet:///r/test/device/dev".to_string(),
            ability_ura: "terminal.create".to_string(),
            action: AccessAction::Stream,
            nonce: None,
            canonical_hash: Some("sha256:test".to_string()),
            requested_lifetimes: vec![PermissionLifetime::Once, PermissionLifetime::Session],
            status: PermissionRequestStatus::Pending,
            created_at: "2026-07-09T00:00:00Z".to_string(),
            expires_at: "2026-07-09T00:05:00Z".to_string(),
            resolver_ura: None,
            resolved_lifetime: None,
            created_grant_id: None,
            authority_proof_id: None,
            resolved_at: None,
            decision_reason: None,
        };
        request.status = PermissionRequestStatus::Pending;
        request
    }

    fn grant_for_request(id: &str, request: &PermissionRequest) -> PermissionGrant {
        PermissionGrant {
            grant_id: id.to_string(),
            owner_user_id: request.owner_user_id.clone(),
            principal_kind: request.principal_kind,
            principal_id: request.principal_id.clone(),
            token_id: request.token_id.clone(),
            token_class: request.token_class,
            callee_ura: Some(request.callee_ura.clone()),
            subject_ura_pattern: Some(request.subject_ura.clone()),
            ability_ura_pattern: Some(request.ability_ura.clone()),
            actions: vec![request.action],
            constraints: None,
            effect: PermissionEffect::Allow,
            lifetime: PermissionGrantLifetime::Permanent,
            state: PermissionGrantState::Active,
            expires_at: None,
            review_required_after: None,
            last_reviewed_at: None,
            last_used_at: None,
            created_by: "easynet:///r/test/user/alice".to_string(),
            created_at: "2026-07-09T00:01:00Z".to_string(),
            updated_at: None,
            revoked_at: None,
            reason: None,
        }
    }

    fn proof_for_request(id: &str, request: &PermissionRequest) -> AuthorityProof {
        AuthorityProof {
            proof_id: id.to_string(),
            grant_id: None,
            permission_request_id: Some(request.request_id.clone()),
            owner_user_id: request.owner_user_id.clone(),
            principal_kind: request.principal_kind,
            principal_id: request.principal_id.clone(),
            token_id: request.token_id.clone(),
            callee_ura: request.callee_ura.clone(),
            subject_ura: request.subject_ura.clone(),
            ability_ura: request.ability_ura.clone(),
            action: request.action,
            nonce: request.nonce.clone(),
            canonical_hash: request.canonical_hash.clone(),
            session_id: None,
            session_owner_user_id: None,
            allowed_followup_abilities: vec![],
            session_expires_at: None,
            issued_at: "2026-07-09T00:01:00Z".to_string(),
            expires_at: "2026-07-09T00:05:00Z".to_string(),
            issuer_ura: "easynet:///r/test/user/alice".to_string(),
            audience_ura: request.callee_ura.clone(),
            signature: "ed25519:test".to_string(),
        }
    }
}
