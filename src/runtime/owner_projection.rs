// EasyNet CLI — Owner Projection Publication (AXON-RFC-005 Phase C)
// =================================================================
//
// Converts the CLI's local ability descriptors into the compact
// owner projection shape consumed by Axon's resolver read model.
// This module owns protocol semantics; persistence only stores the
// last publication cursor.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::persistence::owner_projections::{
    self, OwnerProjectionCursor, OwnerProjectionCursorFile,
};
use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};

pub(crate) const OWNER_PROJECTION_HEARTBEAT_REFRESH_LIMIT: usize = 64;
#[cfg(feature = "axon-pb")]
const OWNER_PROJECTION_LEASE_TTL_MS: i64 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct AbilityCallableFlags {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub streaming_only: bool,
    pub bidi_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AbilityInputFieldSummary {
    pub name: String,
    pub required: bool,
    pub value_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct AbilityCallableSummary {
    pub public_name: String,
    pub description: String,
    pub ability_class: String,
    pub input_fields: Vec<AbilityInputFieldSummary>,
    pub flags: AbilityCallableFlags,
}

impl AbilityCallableSummary {
    #[cfg(test)]
    pub(crate) fn minimal(public_name: impl Into<String>) -> Self {
        let public_name = public_name.into();
        Self {
            description: public_name.clone(),
            public_name,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AbilityProjectionSummary {
    pub ability_ura: String,
    pub owner_ura: String,
    pub namespace: String,
    pub local_name: String,
    pub descriptor_revision: String,
    pub schema_ref: Option<String>,
    pub schema_hash: Option<String>,
    pub policy_ref: String,
    pub route_summary_ref: Option<String>,
    pub tags: Vec<String>,
    #[serde(default)]
    pub callable_summary: AbilityCallableSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OwnerProjectionPublication {
    pub owner_ura: String,
    pub host_device_ura: String,
    pub projection_revision: u64,
    pub projection_digest: String,
    pub lease_expires_unix_ms: i64,
    pub ability_summaries: Vec<AbilityProjectionSummary>,
}

#[derive(Debug, Clone)]
struct PreparedProjection {
    publication: OwnerProjectionPublication,
    cursor: OwnerProjectionCursor,
}

pub(crate) fn prepare_and_persist(
    owner_ura: &str,
    host_device_ura: &str,
    descriptors: &[AbilityDescriptor],
) -> Result<OwnerProjectionPublication, String> {
    let mut file = owner_projections::load()
        .map_err(|e| format!("load owner projection cursor failed: {e}"))?;
    let prepared = prepare_at(
        owner_ura,
        host_device_ura,
        descriptors,
        &file,
        now_unix_ms(),
    )?;
    file.upsert(prepared.cursor);
    owner_projections::save(&file)
        .map_err(|e| format!("save owner projection cursor failed: {e}"))?;
    Ok(prepared.publication)
}

/// Build a tombstone publication for `owner_ura` (empty ability set,
/// revision bumped strictly past the prior cursor so the hub's D26
/// fence accepts it) and drop the local cursor so the owner leaves the
/// heartbeat refresh batch. Used on `agent.stop`: advertising the empty
/// complete-set makes the hub remove every prior projected ability
/// (complete-set REPLACE → `removed = old − ∅ = all`). Returns `None`
/// when no prior cursor existed (nothing to tombstone). ISS-002.
pub(crate) fn prepare_removal_and_persist(
    owner_ura: &str,
    host_device_ura: &str,
) -> Result<Option<OwnerProjectionPublication>, String> {
    let mut file = owner_projections::load()
        .map_err(|e| format!("load owner projection cursor failed: {e}"))?;
    if file.cursor_for(owner_ura).is_none() {
        return Ok(None);
    }
    // Empty descriptor set → empty summaries; prepare_at bumps the
    // revision past the prior cursor (new-content branch) so the hub
    // accepts the tombstone as strictly-newer.
    let prepared = prepare_at(owner_ura, host_device_ura, &[], &file, now_unix_ms())?;
    let publication = prepared.publication;
    file.remove(owner_ura);
    owner_projections::save(&file)
        .map_err(|e| format!("save owner projection cursor failed: {e}"))?;
    Ok(Some(publication))
}

pub(crate) fn heartbeat_refresh_owner_uras() -> Result<Vec<String>, String> {
    let file = owner_projections::load()
        .map_err(|e| format!("load owner projection cursor failed: {e}"))?;
    Ok(heartbeat_refresh_owner_uras_from_file(&file))
}

fn prepare_at(
    owner_ura: &str,
    host_device_ura: &str,
    descriptors: &[AbilityDescriptor],
    cursors: &OwnerProjectionCursorFile,
    now_ms: i64,
) -> Result<PreparedProjection, String> {
    let owner_ura = owner_ura.trim();
    let host_device_ura = host_device_ura.trim();
    if owner_ura.is_empty() {
        return Err("owner_ura must not be empty".into());
    }
    if host_device_ura.is_empty() {
        return Err("host_device_ura must not be empty".into());
    }

    let mut summaries = descriptors
        .iter()
        .map(summary_from_descriptor)
        .collect::<Result<Vec<_>, _>>()?;
    summaries.sort_by(|a, b| {
        serialize_value(&canonical_summary_json(a))
            .cmp(&serialize_value(&canonical_summary_json(b)))
    });
    summaries.dedup_by(|a, b| canonical_summary_json(a) == canonical_summary_json(b));

    let fingerprint = content_fingerprint(owner_ura, host_device_ura, &summaries);
    let previous = cursors.cursor_for(owner_ura);
    let same_content_previous = previous.filter(|cursor| cursor.content_fingerprint == fingerprint);
    let (projection_revision, projection_digest, lease_expires_unix_ms) =
        if let Some(cursor) = same_content_previous {
            let revision = cursor.projection_revision.max(1);
            // C4: lease cancelled (ISS-002) — publish lease=0 so the hub
            // treats the projection as non-expiring (Axon expire_leases
            // guards on lease > 0). TTL-as-existence is replaced by
            // event-driven advertise on agent.start.
            let lease_expires_unix_ms = 0;
            let digest = projection_digest(
                owner_ura,
                host_device_ura,
                revision,
                lease_expires_unix_ms,
                &summaries,
            );
            (revision, digest, lease_expires_unix_ms)
        } else {
            let revision = match previous {
                Some(cursor) => cursor.projection_revision.saturating_add(1).max(1),
                None => 1,
            };
            // C4: lease cancelled (ISS-002) — see same-content branch above.
            let lease_expires_unix_ms = 0;
            let digest = projection_digest(
                owner_ura,
                host_device_ura,
                revision,
                lease_expires_unix_ms,
                &summaries,
            );
            (revision, digest, lease_expires_unix_ms)
        };
    let updated_at = format_unix_ms(now_ms);

    Ok(PreparedProjection {
        publication: OwnerProjectionPublication {
            owner_ura: owner_ura.to_string(),
            host_device_ura: host_device_ura.to_string(),
            projection_revision,
            projection_digest: projection_digest.clone(),
            lease_expires_unix_ms,
            ability_summaries: summaries,
        },
        cursor: OwnerProjectionCursor {
            owner_ura: owner_ura.to_string(),
            host_device_ura: host_device_ura.to_string(),
            projection_revision,
            projection_digest,
            content_fingerprint: fingerprint,
            lease_expires_unix_ms,
            updated_at,
        },
    })
}

fn heartbeat_refresh_owner_uras_from_file(file: &OwnerProjectionCursorFile) -> Vec<String> {
    let owners = file
        .projections
        .iter()
        .filter_map(|cursor| {
            let owner_ura = cursor.owner_ura.trim();
            if owner_ura.is_empty() || cursor.host_device_ura.trim().is_empty() {
                return None;
            }
            Some(owner_ura.to_string())
        })
        .collect::<BTreeSet<_>>();
    owners
        .into_iter()
        .take(OWNER_PROJECTION_HEARTBEAT_REFRESH_LIMIT)
        .collect()
}

/// Lease expiry `OWNER_PROJECTION_LEASE_TTL_MS` after `now_ms`. Shared by
/// projection publication and `federation.heartbeat` lease refresh so both
/// renew to the same TTL and the lease window cannot drift between them.
#[cfg(feature = "axon-pb")]
pub(crate) fn lease_expiry_from_now(now_ms: i64) -> i64 {
    now_ms.saturating_add(OWNER_PROJECTION_LEASE_TTL_MS)
}

fn now_unix_ms() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn format_unix_ms(now_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn summary_from_descriptor(
    descriptor: &AbilityDescriptor,
) -> Result<AbilityProjectionSummary, String> {
    let ability_ura = descriptor.canonical_ability_ura().ok_or_else(|| {
        format!(
            "cannot derive ability URA for descriptor {}",
            descriptor.name
        )
    })?;
    let public_name = descriptor.public_name();
    let (namespace, local_name) = split_public_name(&public_name);
    let descriptor_revision = descriptor.descriptor_hash_prefixed();
    let schema_hash = Some(descriptor.schema_hash_prefixed());
    let mut tags = vec![format!("class:{}", descriptor.ability_class().as_str())];
    if !descriptor.source.trim().is_empty() {
        tags.push(format!("source:{}", bounded_tag_value(&descriptor.source)));
    }
    tags.sort();
    tags.dedup();

    Ok(AbilityProjectionSummary {
        ability_ura: ability_ura.clone(),
        owner_ura: descriptor.owner_ura.clone(),
        namespace,
        local_name,
        descriptor_revision,
        schema_ref: None,
        schema_hash,
        policy_ref: visibility_policy_ref(descriptor.visibility).to_string(),
        route_summary_ref: Some(format!("route-ref::{ability_ura}")),
        tags,
        callable_summary: callable_summary_from_descriptor(descriptor, &public_name),
    })
}

fn callable_summary_from_descriptor(
    descriptor: &AbilityDescriptor,
    public_name: &str,
) -> AbilityCallableSummary {
    AbilityCallableSummary {
        public_name: public_name.to_string(),
        description: if descriptor.description.trim().is_empty() {
            public_name.to_string()
        } else {
            descriptor.description.trim().to_string()
        },
        ability_class: descriptor.ability_class().as_str().to_string(),
        input_fields: input_field_summaries(&descriptor.schema_summary.input),
        flags: AbilityCallableFlags {
            read_only: descriptor.hints.read_only,
            destructive: descriptor.hints.destructive,
            idempotent: descriptor.hints.idempotent,
            streaming_only: descriptor.hints.streaming_only,
            bidi_only: descriptor.hints.bidi_only,
        },
    }
}

fn input_field_summaries(schema: &Value) -> Vec<AbilityInputFieldSummary> {
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    properties
        .iter()
        .map(|(name, property)| AbilityInputFieldSummary {
            name: name.to_string(),
            required: required.contains(name),
            value_type: property_value_type(property),
        })
        .collect()
}

fn property_value_type(property: &Value) -> String {
    if let Some(ty) = property.get("type").and_then(Value::as_str) {
        return ty.to_string();
    }
    for aggregate in ["oneOf", "anyOf", "allOf"] {
        if property.get(aggregate).is_some() {
            return aggregate.to_string();
        }
    }
    if property.get("enum").is_some() {
        return "enum".to_string();
    }
    if property.get("const").is_some() {
        return "const".to_string();
    }
    "unknown".to_string()
}

fn split_public_name(public_name: &str) -> (String, String) {
    public_name
        .split_once('.')
        .map(|(namespace, local_name)| (namespace.to_string(), local_name.to_string()))
        .unwrap_or_else(|| (String::new(), public_name.to_string()))
}

fn visibility_policy_ref(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "visibility:PUBLIC",
        Visibility::Scoped => "visibility:SCOPED",
        Visibility::Private => "visibility:PRIVATE",
    }
}

fn bounded_tag_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 96 {
        return trimmed.to_string();
    }
    let prefix = trimmed.chars().take(64).collect::<String>();
    format!("{}:{}", prefix, hash_bytes_hex(trimmed.as_bytes()))
}

fn content_fingerprint(
    owner_ura: &str,
    host_device_ura: &str,
    summaries: &[AbilityProjectionSummary],
) -> String {
    hash_value_hex(&canonical_projection_json(
        owner_ura,
        host_device_ura,
        0,
        0,
        summaries,
    ))
}

fn projection_digest(
    owner_ura: &str,
    host_device_ura: &str,
    projection_revision: u64,
    lease_expires_unix_ms: i64,
    summaries: &[AbilityProjectionSummary],
) -> String {
    hash_value_hex(&canonical_projection_json(
        owner_ura,
        host_device_ura,
        projection_revision,
        lease_expires_unix_ms,
        summaries,
    ))
}

fn canonical_projection_json(
    owner_ura: &str,
    host_device_ura: &str,
    projection_revision: u64,
    lease_expires_unix_ms: i64,
    summaries: &[AbilityProjectionSummary],
) -> Value {
    let mut ability_values = summaries
        .iter()
        .map(canonical_summary_json)
        .collect::<Vec<_>>();
    ability_values.sort_by_key(serialize_value);
    ability_values.dedup_by(|a, b| serialize_value(a) == serialize_value(b));
    json!({
        "owner_ura": owner_ura,
        "host_device_ura": host_device_ura,
        "projection_revision": projection_revision,
        "lease_expires_unix_ms": lease_expires_unix_ms,
        "abilities": ability_values,
    })
}

fn canonical_summary_json(summary: &AbilityProjectionSummary) -> Value {
    json!({
        "ability_ura": summary.ability_ura,
        "owner_ura": summary.owner_ura,
        "namespace": summary.namespace,
        "local_name": summary.local_name,
        "descriptor_revision": summary.descriptor_revision,
        "schema_ref": summary.schema_ref,
        "schema_hash": summary.schema_hash,
        "policy_ref": summary.policy_ref,
        "route_summary_ref": summary.route_summary_ref,
        "tags": summary.tags,
        "callable_summary": summary.callable_summary,
    })
}

pub(crate) fn summary_from_value(value: &Value) -> Option<AbilityProjectionSummary> {
    serde_json::from_value(value.clone()).ok()
}

#[cfg(test)]
pub(crate) fn summary_public_name_from_value(value: &Value) -> Option<String> {
    let summary = summary_from_value(value)?;
    summary_public_name(&summary)
}

pub(crate) fn summary_public_name(summary: &AbilityProjectionSummary) -> Option<String> {
    let local_name = summary.local_name.trim();
    if local_name.is_empty() {
        return None;
    }
    let namespace = summary.namespace.trim();
    if namespace.is_empty() {
        Some(local_name.to_string())
    } else {
        Some(format!("{namespace}.{local_name}"))
    }
}

/// Projection of a skill subject URA onto its owning Agent.
///
/// Invariants:
/// 1. `agent_ura` is always a canonical Agent URA derived from the subject.
/// 2. `skill_name` is present only when the subject names one concrete skill
///    package resource (`skill/<skill-name>`).
/// 3. This type does not prove local hosting. Callers that need a local agent
///    name must still resolve `agent_ura` through `local-agents.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSkillSubjectProjection {
    pub agent_ura: String,
    pub skill_name: Option<String>,
}

/// Interpret an owner Agent URA or an agent-owned skill Resource URA.
///
/// This is the single projection API for skill inventory/install callers. It
/// accepts:
/// - `easynet:///r/<realm>/agent/<user>.<agent>`
/// - `easynet:///r/<realm>/resource/agent.<user>.<agent>/skill/<skill-name>`
///
/// It intentionally rejects other resource paths because those are different
/// owner projections, not aliases for skill inventory scope.
pub(crate) fn project_agent_skill_subject(
    subject_ura: &str,
) -> Result<AgentSkillSubjectProjection, String> {
    let parsed = crate::ura::parse_ura(subject_ura)
        .map_err(|e| format!("invalid subject_ura {subject_ura:?}: {e}"))?;
    match parsed.kind {
        crate::ura::URAKind::Agent => Ok(AgentSkillSubjectProjection {
            agent_ura: subject_ura.to_string(),
            skill_name: None,
        }),
        crate::ura::URAKind::Resource => {
            let owner_id = parsed
                .resource_owner_id()
                .ok_or_else(|| "subject_ura resource owner missing".to_string())?;
            let (user_id, agent_id) = resource_owner_agent_parts(owner_id).ok_or_else(|| {
                format!(
                    "subject_ura resource owner must be agent.<user>.<agent>, got {:?}",
                    owner_id
                )
            })?;
            let resource_path = parsed.resource_path().unwrap_or_default();
            let skill_name = parsed
                .resource_path()
                .unwrap_or_default()
                .strip_prefix("skill/")
                .and_then(|name| {
                    (!name.is_empty() && !name.contains('/')).then_some(name.to_string())
                })
                .ok_or_else(|| {
                    format!(
                        "subject_ura resource path must be skill/<skill-name>, got {:?}",
                        resource_path
                    )
                })?;
            Ok(AgentSkillSubjectProjection {
                agent_ura: crate::ura::agent_ura(&parsed.realm, &user_id, &agent_id),
                skill_name: Some(skill_name),
            })
        }
        other => Err(format!(
            "subject_ura must be an Agent URA or skill Resource URA, got {other:?}"
        )),
    }
}

/// Build the canonical Resource URA for a skill package owned by an Agent.
///
/// Returns `None` when `agent_ura` is not an Agent URA, and for
/// device-sponsored System Agents (see below). The caller decides
/// whether that should be an omitted optional field or a hard input error.
pub(crate) fn skill_resource_ura(agent_ura: &str, skill_name: &str) -> Option<String> {
    let parsed = crate::ura::parse_ura(agent_ura).ok()?;
    if parsed.kind != crate::ura::URAKind::Agent {
        return None;
    }
    // DEC-F048 / RFC gap: the resource_dot owner segment for a
    // device-sponsored System Agent (`agent.device.<id>.<agent>`?) is
    // NOT yet defined. Deliberately None rather than an invented
    // shape — ura-discipline: flag, don't extrapolate; the gap is on
    // the RFC-007/008 agenda (F-047 verdict).
    if parsed.device_agent_ids().is_some() {
        return None;
    }
    let (user_id, agent_id) = parsed.agent_ids()?;
    Some(crate::ura::resource_dot_ura(
        &parsed.realm,
        &format!("agent.{user_id}.{agent_id}"),
        &format!("skill/{skill_name}"),
    ))
}

fn resource_owner_agent_parts(owner: &str) -> Option<(String, String)> {
    let tail = owner.strip_prefix("agent.")?;
    let (user_id, agent_id) = tail.split_once('.')?;
    if user_id.is_empty() || agent_id.is_empty() || agent_id.contains('.') {
        return None;
    }
    Some((user_id.to_string(), agent_id.to_string()))
}

fn hash_value_hex(value: &Value) -> String {
    hash_bytes_hex(&serde_json::to_vec(value).expect("serde_json::Value serialization cannot fail"))
}

fn hash_bytes_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn serialize_value(value: &Value) -> String {
    serde_json::to_string(value).expect("serde_json::Value serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_descriptor::AbilityDescriptor;

    #[test]
    fn skill_resource_ura_dual_shape() {
        // User-owned agent: canonical resource_dot shape.
        assert_eq!(
            skill_resource_ura("easynet:///r/localhost/agent/dev.claude", "alive-video"),
            Some("easynet:///r/localhost/resource/agent.dev.claude/skill/alive-video".to_string())
        );
        // Device-sponsored System Agent: declared None — its
        // resource_dot owner shape is an open RFC-007/008 gap
        // (DEC-F048; F-047 verdict), never an invented form.
        assert_eq!(
            skill_resource_ura(
                "easynet:///r/localhost/agent/device.dev-1.terminal",
                "alive-video"
            ),
            None
        );
    }

    fn descriptor(name: &str, owner: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(name, owner, Visibility::Public)
            .unwrap()
            .with_source("kernel:built-in")
    }

    #[test]
    fn builds_device_owned_summary_with_public_device_namespace() {
        let owner = "easynet:///r/acme/device/01DEV";
        let descriptors = vec![descriptor("fs.read", owner)];
        let file = OwnerProjectionCursorFile::default();
        let prepared =
            prepare_at(owner, owner, &descriptors, &file, 1_000).expect("prepare projection");
        let summary = &prepared.publication.ability_summaries[0];

        assert_eq!(
            summary.ability_ura,
            "easynet:///r/acme/ability/device.01DEV.fs.read"
        );
        assert_eq!(summary.namespace, "fs");
        assert_eq!(summary.local_name, "read");
        assert_eq!(summary.policy_ref, "visibility:PUBLIC");
        assert!(summary.descriptor_revision.starts_with("sha256:"));
        assert!(summary.schema_hash.as_ref().unwrap().starts_with("sha256:"));
        assert_eq!(summary.callable_summary.public_name, "fs.read");
        assert_eq!(summary.callable_summary.description, "fs.read");
        assert_eq!(summary.callable_summary.ability_class, "query");
        assert_eq!(prepared.publication.projection_revision, 1);
        // C4: lease cancelled (ISS-002) — projections publish lease=0.
        assert_eq!(prepared.publication.lease_expires_unix_ms, 0);
    }

    #[test]
    fn empty_set_tombstone_bumps_revision_past_prior_and_clears_summaries() {
        // ISS-002 stop side: re-advertising an empty complete-set must
        // bump the revision strictly past the prior projection (so the
        // hub's D26 fence accepts the tombstone) and carry zero
        // summaries (so complete-set REPLACE removes every prior ability).
        let owner = "easynet:///r/acme/agent/alice.claude";
        let host = "easynet:///r/acme/device/01DEV";
        let descriptors = vec![descriptor("chat", owner)];
        let mut file = OwnerProjectionCursorFile::default();
        let first = prepare_at(owner, host, &descriptors, &file, 1_000).expect("first publish");
        file.upsert(first.cursor);
        assert_eq!(first.publication.projection_revision, 1);

        let tombstone = prepare_at(owner, host, &[], &file, 2_000).expect("tombstone publish");
        assert!(
            tombstone.publication.projection_revision > first.publication.projection_revision,
            "tombstone revision must be strictly newer for the hub fence to accept it"
        );
        assert!(
            tombstone.publication.ability_summaries.is_empty(),
            "tombstone must carry an empty complete-set so the hub removes all prior abilities"
        );
        assert_eq!(tombstone.publication.lease_expires_unix_ms, 0);
    }

    #[test]
    fn filesystem_summary_publishes_callable_fields_without_host_paths() {
        let owner = "easynet:///r/acme/device/01DEV";
        let descriptor = descriptor("fs.read", owner)
            .with_description(crate::runtime::agents::fs_ability::description_read())
            .with_input_schema(crate::runtime::agents::fs_ability::input_schema_read());
        let summary = summary_from_descriptor(&descriptor).expect("summary");

        let fields = summary
            .callable_summary
            .input_fields
            .iter()
            .map(|field| {
                (
                    field.name.as_str(),
                    field.required,
                    field.value_type.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            fields.contains(&("resource_ref", true, "object")),
            "filesystem callable summary must advertise ResourceRef input: {fields:?}"
        );
        assert!(fields.contains(&("max_bytes", false, "integer")));
        assert!(fields.contains(&("encoding", false, "string")));

        let wire = serde_json::to_string(&summary).expect("summary serializes");
        assert!(
            !wire.contains("/Users/")
                && !wire.contains("/private/")
                && !wire.contains("\"properties\""),
            "projection summary must not publish raw host paths or full schema: {wire}"
        );
    }

    #[test]
    fn summary_public_name_joins_namespace_and_local_name() {
        let summary = AbilityProjectionSummary {
            ability_ura: "easynet:///r/acme/ability/device.01DEV.fs.read".into(),
            owner_ura: "easynet:///r/acme/device/01DEV".into(),
            namespace: "fs".into(),
            local_name: "read".into(),
            descriptor_revision: "sha256:descriptor".into(),
            schema_ref: None,
            schema_hash: None,
            policy_ref: "visibility:PUBLIC".into(),
            route_summary_ref: None,
            tags: Vec::new(),
            callable_summary: AbilityCallableSummary::minimal("fs.read"),
        };
        let value = serde_json::to_value(&summary).expect("summary serializes");

        assert_eq!(summary_public_name(&summary).as_deref(), Some("fs.read"));
        assert_eq!(
            summary_public_name_from_value(&value).as_deref(),
            Some("fs.read")
        );
    }

    #[test]
    fn summary_public_name_allows_empty_namespace_agent_ability() {
        let summary = AbilityProjectionSummary {
            ability_ura: "easynet:///r/acme/ability/alice.bot.chat".into(),
            owner_ura: "easynet:///r/acme/agent/alice.bot".into(),
            namespace: String::new(),
            local_name: "chat".into(),
            descriptor_revision: "sha256:descriptor".into(),
            schema_ref: None,
            schema_hash: None,
            policy_ref: "visibility:SCOPED".into(),
            route_summary_ref: None,
            tags: Vec::new(),
            callable_summary: AbilityCallableSummary::minimal("chat"),
        };

        assert_eq!(summary_public_name(&summary).as_deref(), Some("chat"));
    }

    #[test]
    fn project_agent_skill_subject_accepts_owner_ura() {
        let subject = "easynet:///r/acme/agent/alice.claude";
        let projection = project_agent_skill_subject(subject).expect("agent subject");

        assert_eq!(projection.agent_ura, subject);
        assert_eq!(projection.skill_name, None);
    }

    #[test]
    fn project_agent_skill_subject_maps_skill_resource_to_owner_agent() {
        let projection = project_agent_skill_subject(
            "easynet:///r/acme/resource/agent.alice.claude/skill/inspectable",
        )
        .expect("skill resource subject");

        assert_eq!(projection.agent_ura, "easynet:///r/acme/agent/alice.claude");
        assert_eq!(projection.skill_name.as_deref(), Some("inspectable"));
    }

    #[test]
    fn project_agent_skill_subject_rejects_non_skill_resource() {
        let err = project_agent_skill_subject(
            "easynet:///r/acme/resource/agent.alice.claude/memory/inspectable",
        )
        .unwrap_err();

        assert!(err.contains("skill/<skill-name>"), "{err}");
    }

    #[test]
    fn skill_resource_ura_builds_agent_owned_resource_ura() {
        assert_eq!(
            skill_resource_ura("easynet:///r/acme/agent/alice.claude", "inspectable").as_deref(),
            Some("easynet:///r/acme/resource/agent.alice.claude/skill/inspectable")
        );
        assert_eq!(
            skill_resource_ura("easynet:///r/acme/device/01DEV", "inspectable"),
            None
        );
    }

    #[test]
    fn unchanged_content_reuses_revision_with_cancelled_lease() {
        let owner = "easynet:///r/acme/device/01DEV";
        let descriptors = vec![descriptor("fs.read", owner)];
        let first = prepare_at(
            owner,
            owner,
            &descriptors,
            &OwnerProjectionCursorFile::default(),
            1_000,
        )
        .expect("first");
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(first.cursor);

        let second = prepare_at(owner, owner, &descriptors, &file, 30_000).expect("second");
        assert_eq!(second.publication.projection_revision, 1);
        // C4: lease cancelled (ISS-002) — with lease=0 the digest no longer
        // drifts with the clock, so a re-publish of unchanged content keeps
        // the same revision, the same digest, and lease=0.
        assert_eq!(
            second.publication.projection_digest,
            file.cursor_for(owner).unwrap().projection_digest
        );
        assert_eq!(second.publication.lease_expires_unix_ms, 0);
        assert_eq!(file.cursor_for(owner).unwrap().lease_expires_unix_ms, 0);
    }

    #[test]
    fn changed_content_bumps_revision() {
        let owner = "easynet:///r/acme/device/01DEV";
        let first_descriptors = vec![descriptor("fs.read", owner)];
        let first = prepare_at(
            owner,
            owner,
            &first_descriptors,
            &OwnerProjectionCursorFile::default(),
            1_000,
        )
        .expect("first");
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(first.cursor);

        let second_descriptors = vec![
            descriptor("fs.read", owner),
            descriptor("skill.list", owner),
        ];
        let second = prepare_at(owner, owner, &second_descriptors, &file, 30_000).expect("second");
        assert_eq!(second.publication.projection_revision, 2);
        assert_ne!(
            second.publication.projection_digest,
            file.cursor_for(owner).unwrap().projection_digest
        );
        // C4: lease cancelled (ISS-002) — projections publish lease=0.
        assert_eq!(second.publication.lease_expires_unix_ms, 0);
    }

    #[test]
    fn heartbeat_refresh_owner_uras_are_deduped_stable_and_bounded() {
        let mut file = OwnerProjectionCursorFile::default();
        file.upsert(cursor("z", "host", 1));
        file.upsert(cursor("", "host", 1));
        file.upsert(cursor("a", "", 1));
        file.upsert(cursor("m", "host", 1));
        file.upsert(cursor("z", "host", 2));
        for idx in 0..70 {
            file.upsert(cursor(&format!("owner-{idx:02}"), "host", 1));
        }

        let owners = heartbeat_refresh_owner_uras_from_file(&file);

        assert_eq!(owners.len(), OWNER_PROJECTION_HEARTBEAT_REFRESH_LIMIT);
        assert_eq!(owners[0], "m");
        assert!(owners.iter().all(|owner| !owner.trim().is_empty()));
        assert!(owners.windows(2).all(|pair| pair[0] < pair[1]));
    }

    fn cursor(
        owner_ura: &str,
        host_device_ura: &str,
        projection_revision: u64,
    ) -> OwnerProjectionCursor {
        OwnerProjectionCursor {
            owner_ura: owner_ura.into(),
            host_device_ura: host_device_ura.into(),
            projection_revision,
            projection_digest: format!("digest-{owner_ura}-{projection_revision}"),
            content_fingerprint: format!("fingerprint-{owner_ura}-{projection_revision}"),
            lease_expires_unix_ms: 61_000,
            updated_at: "1970-01-01T00:00:01.000Z".into(),
        }
    }
}
