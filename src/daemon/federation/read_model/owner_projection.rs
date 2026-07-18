// EasyNet CLI — Federation Owner Projection Read Model
// ====================================================
//
// File: src/daemon/federation/read_model/owner_projection.rs
// Description: Converts daemon-local ability descriptors into the compact
//              owner projection shape consumed by the federation resolver
//              read model. Persistence stores only the last publication
//              cursor; this module owns the daemon read-model projection.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::daemon::ability::descriptors::{
    AbilityDescriptor, CallMode, ReceiptSemantics, Visibility,
};
use crate::daemon::persistence::owner_projections::{
    self, OwnerProjectionCursor, OwnerProjectionCursorFile, OwnerProjectionCursorLifecycle,
};

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
    pub call_mode: CallMode,
    pub receipt_semantics: ReceiptSemantics,
    pub input_fields: Vec<AbilityInputFieldSummary>,
    pub flags: AbilityCallableFlags,
    /// Governed descriptor variants grouped under this canonical Ability URA.
    /// The scalar callable fields above remain the deterministic primary view;
    /// this geometry is the lossless transport/version proof for every mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mode_geometry: Vec<AbilityCallModeGeometry>,
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
pub(crate) struct AbilityCallModeGeometry {
    pub call_mode: CallMode,
    pub descriptor_version: String,
    pub descriptor_revision: String,
    pub admission_action: String,
    pub schema_hash: String,
    pub policy_ref: String,
    pub policy_hash: String,
    pub description: String,
    pub receipt_semantics: ReceiptSemantics,
    pub input_fields: Vec<AbilityInputFieldSummary>,
    pub flags: AbilityCallableFlags,
    pub tags: Vec<String>,
}

/// Compact owner-projection row published via `federation.advertise_abilities`
/// and read back via `federation.resolve`.
///
/// REGISTRY/INDEX METADATA — intentionally outside the Axon wire contract
/// (refactor SPEC §15.1 item 2; owner ruling 2026-06-29).
///
/// The first ten fields mirror the Axon `AbilityProjectionSummary` proto
/// (`EasyNet-Axon/core/proto/axon/v1/namespace.proto`). The eleventh field,
/// `callable_summary` (`public_name` / `description` / `call_mode` /
/// `receipt_semantics` / input fields / flags / `mode_geometry`), is daemon
/// registry/index metadata deliberately NOT part of the Axon proto. The mode
/// geometry binds the descriptor variants represented by one canonical
/// Ability publication row; the remaining fields are presentation metadata.
///
/// DECISION (do not relitigate): `callable_summary` is NOT promoted into the
/// proto. The Axon wire contract carries only the canonical *execution*
/// contract — invocation, authority, receipt, causal context. Discovery
/// registry/index extension is not a second Axon execution contract. Binding
/// it into the proto would couple daemon discovery projection to wire
/// compatibility, which RFC-005 §4.2 forbids when it says the projection
/// summary "MUST NOT carry implementation-private fields".
///
/// It is load-bearing today and travels as a tolerated serde extension, NOT a
/// proto field:
///   * EMITTED: `advertise.rs` serialises `ability_summaries` (this struct,
///     `callable_summary` included) via `serde_json::to_value(&args)` onto the
///     `federation.advertise_abilities` invocation envelope.
///   * CONSUMED CROSS-PROCESS: a peer daemon's discover ladder reads
///     `summary.callable_summary.description` in
///     `daemon::ability::builtins::agents::discover` after a `federation.resolve`
///     round-trip — written by daemon A, read by daemon B.
///   * TOLERATED BY NON-Rust DISCOVERY CONSUMERS: it rides as `serde_json` with
///     `#[serde(default)]`; a consumer using `DiscardUnknown` may still read the
///     proto projection, but that lossy view cannot be re-admitted as an owner
///     publication because governed mode geometry is then absent.
///
/// Guarded by `callable_summary_survives_projection_wire_roundtrip`. Do NOT
/// add this (or any other discovery-presentation field) to the Axon proto;
/// keep presentation metadata in this registry/index layer.
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
    /// Proto-unacknowledged daemon extension — see the struct-level
    /// CONTRACT-DRIFT INVARIANT above before touching this field.
    #[serde(default)]
    pub callable_summary: AbilityCallableSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OwnerProjectionPublication {
    pub owner_ura: String,
    pub host_device_ura: String,
    /// Monotonic incarnation allocated by the durable owner cursor. A retired
    /// owner that is registered again receives a strictly larger generation.
    pub generation: u64,
    pub projection_revision: u64,
    pub projection_digest: String,
    pub lease_expires_unix_ms: i64,
    /// Transport fencing for purge-only projection delivery. These fields are
    /// intentionally outside `projection_digest`: takeover advances the
    /// delivery fence without changing the journaled projection content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purge_delivery: Option<PurgeProjectionDelivery>,
    #[serde(default)]
    pub ability_summaries: Vec<AbilityProjectionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PurgeProjectionDelivery {
    pub protocol_version: u32,
    pub transaction_id: String,
    pub generation: u64,
    pub authority_ura: String,
    pub delivery_fence: u64,
}

impl OwnerProjectionPublication {
    pub(crate) fn canonical_digest(&self) -> String {
        projection_digest(
            &self.owner_ura,
            &self.host_device_ura,
            self.generation,
            self.projection_revision,
            self.lease_expires_unix_ms,
            &self.ability_summaries,
        )
    }

    /// Validate a received complete-set projection before it reaches the
    /// directory read model. One row represents one canonical Ability URA;
    /// its mode geometry preserves every governed descriptor variant. The
    /// digest binds every routing and descriptor field, while the owner checks
    /// prevent internally consistent payloads from smuggling abilities for a
    /// different principal.
    pub(crate) fn validate_integrity(&self) -> Result<(), String> {
        if self.owner_ura.trim() != self.owner_ura || self.owner_ura.is_empty() {
            return Err("owner_ura must be non-empty and trimmed".to_string());
        }
        if self.host_device_ura.trim() != self.host_device_ura || self.host_device_ura.is_empty() {
            return Err("host_device_ura must be non-empty and trimmed".to_string());
        }
        if self.projection_revision == 0 {
            return Err("projection_revision must be greater than zero".to_string());
        }
        if self.generation == 0 {
            return Err("generation must be greater than zero".to_string());
        }
        if let Some(delivery) = &self.purge_delivery {
            if delivery.protocol_version
                != crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION
                || delivery.transaction_id.len() != 32
                || delivery.generation != self.generation
                || delivery.authority_ura != self.host_device_ura
                || delivery.delivery_fence == 0
            {
                return Err("purge delivery metadata is contradictory".to_string());
            }
        }

        let mut seen_ability_uras = BTreeSet::new();
        for summary in &self.ability_summaries {
            if summary.owner_ura != self.owner_ura {
                return Err(format!(
                    "ability summary owner_ura `{}` does not match projection owner `{}`",
                    summary.owner_ura, self.owner_ura
                ));
            }
            let selector = crate::core::ura::AbilitySelector::parse(&summary.ability_ura)
                .map_err(|err| format!("invalid ability_ura `{}`: {err}", summary.ability_ura))?;
            if selector.owner_ura() != self.owner_ura {
                return Err(format!(
                    "ability_ura `{}` belongs to `{}`, not projection owner `{}`",
                    summary.ability_ura,
                    selector.owner_ura(),
                    self.owner_ura
                ));
            }
            if !seen_ability_uras.insert(summary.ability_ura.as_str()) {
                return Err(format!(
                    "projection contains duplicate ability_ura `{}`",
                    summary.ability_ura
                ));
            }
            validate_mode_geometry(summary)?;
        }

        let expected_digest = self.canonical_digest();
        if self.projection_digest != expected_digest {
            return Err(format!(
                "projection_digest mismatch: expected `{expected_digest}`"
            ));
        }
        Ok(())
    }
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
    owner_projections::update(|file| {
        let prepared = prepare_at(owner_ura, host_device_ura, descriptors, file, now_unix_ms())
            .map_err(anyhow::Error::msg)?;
        file.upsert(prepared.cursor);
        Ok(prepared.publication)
    })
    .map_err(|e| format!("update owner projection cursor failed: {e}"))
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
    owner_projections::update(|file| {
        if file.active_cursor_for(owner_ura).is_none() {
            return Ok(None);
        }
        let prepared = prepare_at(owner_ura, host_device_ura, &[], file, now_unix_ms())
            .map_err(anyhow::Error::msg)?;
        let publication = prepared.publication;
        file.upsert(prepared.cursor);
        file.retire(owner_ura);
        Ok(Some(publication))
    })
    .map_err(|e| format!("update owner projection removal cursor failed: {e}"))
}

/// Persist an empty complete-set cursor without retiring it. Destructive
/// lifecycle transactions journal the returned publication, publish it after
/// their local commit, and only then call [`retire_removal_cursor`]. Keeping
/// the tombstone cursor durable closes the crash window in which a newer
/// revision was allocated but lost before Hub publication.
pub(crate) fn prepare_journaled_removal(
    owner_ura: &str,
) -> Result<Option<OwnerProjectionPublication>, String> {
    owner_projections::update(|file| {
        let Some(active) = file.active_cursor_for(owner_ura) else {
            return Ok(None);
        };
        let host_device_ura = active.host_device_ura.clone();
        let prepared = prepare_at(owner_ura, &host_device_ura, &[], file, now_unix_ms())
            .map_err(anyhow::Error::msg)?;
        let publication = prepared.publication;
        file.upsert(prepared.cursor);
        Ok(Some(publication))
    })
    .map_err(|e| format!("update owner projection tombstone cursor failed: {e}"))
}

pub(crate) fn publication_required(owner_ura: &str) -> Result<bool, String> {
    owner_projections::load()
        .map(|file| file.active_cursor_for(owner_ura).is_some())
        .map_err(|e| format!("load owner projection cursor failed: {e}"))
}

/// Remove only the exact tombstone cursor already recorded in a committed
/// purge journal. A newer or divergent cursor means ownership changed and is
/// rejected instead of being erased by stale recovery.
pub(crate) fn retire_removal_cursor(
    owner_ura: &str,
    projection_revision: u64,
    projection_digest: &str,
) -> Result<(), String> {
    owner_projections::update(|file| {
        let Some(cursor) = file.cursor_for(owner_ura) else {
            return Ok(());
        };
        if cursor.projection_revision != projection_revision
            || cursor.projection_digest != projection_digest
        {
            return Err(anyhow::anyhow!(
                "refuse to retire changed owner projection cursor for `{owner_ura}`: journal=({projection_revision}, {projection_digest}), current=({}, {})",
                cursor.projection_revision,
                cursor.projection_digest
            ));
        }
        file.retire(owner_ura);
        Ok(())
    })
    .map_err(|e| format!("retire owner projection tombstone cursor failed: {e}"))
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

    let previous = cursors.cursor_for(owner_ura);
    let generation = match previous {
        Some(cursor) if cursor.lifecycle == OwnerProjectionCursorLifecycle::Retired => cursor
            .generation
            .checked_add(1)
            .ok_or_else(|| "owner projection generation exhausted".to_string())?,
        Some(cursor) => cursor.generation,
        None => 1,
    };
    let summaries = summaries_from_descriptors(owner_ura, descriptors)?;
    let fingerprint = content_fingerprint(owner_ura, host_device_ura, generation, &summaries);
    let same_content_previous = previous.filter(|cursor| {
        cursor.lifecycle == OwnerProjectionCursorLifecycle::Active
            && cursor.content_fingerprint == fingerprint
    });
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
                generation,
                revision,
                lease_expires_unix_ms,
                &summaries,
            );
            (revision, digest, lease_expires_unix_ms)
        } else {
            let revision = match previous {
                Some(cursor) => cursor
                    .projection_revision
                    .checked_add(1)
                    .ok_or_else(|| "owner projection revision exhausted".to_string())?,
                None => 1,
            };
            // C4: lease cancelled (ISS-002) — see same-content branch above.
            let lease_expires_unix_ms = 0;
            let digest = projection_digest(
                owner_ura,
                host_device_ura,
                generation,
                revision,
                lease_expires_unix_ms,
                &summaries,
            );
            (revision, digest, lease_expires_unix_ms)
        };
    let updated_at = format_unix_ms(now_ms);

    let publication = OwnerProjectionPublication {
        owner_ura: owner_ura.to_string(),
        host_device_ura: host_device_ura.to_string(),
        generation,
        projection_revision,
        projection_digest: projection_digest.clone(),
        lease_expires_unix_ms,
        purge_delivery: None,
        ability_summaries: summaries,
    };
    publication.validate_integrity()?;

    Ok(PreparedProjection {
        publication,
        cursor: OwnerProjectionCursor {
            owner_ura: owner_ura.to_string(),
            host_device_ura: host_device_ura.to_string(),
            generation,
            lifecycle: OwnerProjectionCursorLifecycle::Active,
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
            if cursor.lifecycle != OwnerProjectionCursorLifecycle::Active {
                return None;
            }
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
    let mut tags = vec![
        format!("mode:{}", descriptor.call_mode().as_str()),
        format!("receipt:{}", descriptor.receipt_semantics().as_str()),
    ];
    if let Some(transition) = descriptor.receipt_semantics().transition() {
        tags.push(format!("transition:{}", transition.transition_id()));
        tags.push(format!(
            "transition_class:{}",
            transition.transition_class().as_str()
        ));
    }
    if !descriptor.source.trim().is_empty() {
        tags.push(format!("source:{}", bounded_tag_value(&descriptor.source)));
    }
    tags.sort();
    tags.dedup();
    let callable_summary = callable_summary_from_descriptor(descriptor, &public_name);
    let mode_geometry = AbilityCallModeGeometry {
        call_mode: descriptor.call_mode(),
        descriptor_version: descriptor.version.clone(),
        descriptor_revision: descriptor_revision.clone(),
        admission_action: descriptor.admission_action().as_str().to_string(),
        schema_hash: descriptor.schema_hash_prefixed(),
        policy_ref: visibility_policy_ref(descriptor.visibility).to_string(),
        policy_hash: descriptor.access_policy_hash_prefixed(),
        description: callable_summary.description.clone(),
        receipt_semantics: callable_summary.receipt_semantics.clone(),
        input_fields: callable_summary.input_fields.clone(),
        flags: callable_summary.flags.clone(),
        tags: tags.clone(),
    };

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
        callable_summary: AbilityCallableSummary {
            mode_geometry: vec![mode_geometry],
            ..callable_summary
        },
    })
}

fn summaries_from_descriptors(
    owner_ura: &str,
    descriptors: &[AbilityDescriptor],
) -> Result<Vec<AbilityProjectionSummary>, String> {
    let mut summaries_by_ability = BTreeMap::<String, Vec<AbilityProjectionSummary>>::new();
    for descriptor in descriptors {
        let summary = summary_from_descriptor(descriptor)?;
        if summary.owner_ura != owner_ura {
            return Err(format!(
                "descriptor owner `{}` does not match projection owner `{owner_ura}`",
                summary.owner_ura
            ));
        }
        summaries_by_ability
            .entry(summary.ability_ura.clone())
            .or_default()
            .push(summary);
    }

    summaries_by_ability
        .into_values()
        .map(merge_ability_summaries)
        .collect()
}

/// Project governed daemon descriptors into the canonical federation summary
/// JSON shape. Product fixtures and outbound adapters use this boundary instead
/// of copying descriptor hashing, mode geometry, or tag aggregation rules.
pub fn canonical_summary_values_from_descriptors(
    owner_ura: &str,
    descriptors: &[AbilityDescriptor],
) -> Result<Vec<Value>, String> {
    summaries_from_descriptors(owner_ura, descriptors).map(|summaries| {
        summaries
            .iter()
            .map(canonical_summary_json)
            .collect::<Vec<_>>()
    })
}

/// Compute the canonical owner-projection digest from admitted summary JSON.
///
/// Parsing through `AbilityProjectionSummary` rejects incomplete or drifted
/// summary shapes before hashing, keeping the digest authority in this module.
pub fn canonical_projection_digest_from_values(
    owner_ura: &str,
    host_device_ura: &str,
    generation: u64,
    projection_revision: u64,
    lease_expires_unix_ms: i64,
    summaries: &[Value],
) -> Result<String, String> {
    let summaries = summaries
        .iter()
        .map(|value| {
            let summary: AbilityProjectionSummary = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid ability projection summary: {error}"))?;
            if summary.owner_ura != owner_ura {
                return Err(format!(
                    "ability projection owner `{}` does not match `{owner_ura}`",
                    summary.owner_ura
                ));
            }
            validate_mode_geometry(&summary)?;
            Ok(summary)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(projection_digest(
        owner_ura,
        host_device_ura,
        generation,
        projection_revision,
        lease_expires_unix_ms,
        &summaries,
    ))
}

fn merge_ability_summaries(
    mut summaries: Vec<AbilityProjectionSummary>,
) -> Result<AbilityProjectionSummary, String> {
    summaries.sort_by(|left, right| {
        let left = left
            .callable_summary
            .mode_geometry
            .first()
            .expect("descriptor projection has one mode geometry");
        let right = right
            .callable_summary
            .mode_geometry
            .first()
            .expect("descriptor projection has one mode geometry");
        compare_mode_geometry(left, right)
    });

    let mut base = summaries
        .first()
        .cloned()
        .ok_or_else(|| "cannot merge an empty ability projection group".to_string())?;
    for summary in &summaries {
        if summary.owner_ura != base.owner_ura
            || summary.namespace != base.namespace
            || summary.local_name != base.local_name
            || summary.schema_ref != base.schema_ref
            || summary.route_summary_ref != base.route_summary_ref
            || summary.callable_summary.public_name != base.callable_summary.public_name
        {
            return Err(format!(
                "ability `{}` has conflicting identity or routing projections",
                base.ability_ura
            ));
        }
    }

    let geometry = canonicalize_mode_geometry(
        summaries
            .iter()
            .flat_map(|summary| summary.callable_summary.mode_geometry.iter().cloned())
            .collect(),
    )?;
    let primary = geometry
        .first()
        .expect("merged descriptor projection has mode geometry");
    let tags = geometry
        .iter()
        .flat_map(|variant| variant.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    base.descriptor_revision = aggregate_descriptor_revision(&base.ability_ura, &geometry);
    base.schema_hash = Some(aggregate_schema_hash(&base.ability_ura, &geometry));
    base.policy_ref = primary.policy_ref.clone();
    base.tags = tags;
    base.callable_summary = AbilityCallableSummary {
        public_name: base.callable_summary.public_name,
        description: primary.description.clone(),
        call_mode: primary.call_mode,
        receipt_semantics: primary.receipt_semantics.clone(),
        input_fields: primary.input_fields.clone(),
        flags: aggregate_callable_flags(&geometry),
        mode_geometry: geometry,
    };
    Ok(base)
}

fn canonicalize_mode_geometry(
    mut geometry: Vec<AbilityCallModeGeometry>,
) -> Result<Vec<AbilityCallModeGeometry>, String> {
    geometry.sort_by(compare_mode_geometry);
    let mut canonical = Vec::<AbilityCallModeGeometry>::with_capacity(geometry.len());
    for variant in geometry {
        if let Some(previous) = canonical.last() {
            if previous.call_mode == variant.call_mode
                && previous.descriptor_version == variant.descriptor_version
            {
                if previous != &variant {
                    return Err(format!(
                        "conflicting {} descriptor version `{}` in one ability publication",
                        variant.call_mode.as_str(),
                        variant.descriptor_version
                    ));
                }
                continue;
            }
        }
        canonical.push(variant);
    }
    Ok(canonical)
}

fn compare_mode_geometry(
    left: &AbilityCallModeGeometry,
    right: &AbilityCallModeGeometry,
) -> std::cmp::Ordering {
    left.call_mode
        .cmp(&right.call_mode)
        .then_with(|| left.descriptor_version.cmp(&right.descriptor_version))
        .then_with(|| left.descriptor_revision.cmp(&right.descriptor_revision))
        .then_with(|| left.admission_action.cmp(&right.admission_action))
}

fn aggregate_callable_flags(geometry: &[AbilityCallModeGeometry]) -> AbilityCallableFlags {
    let call_modes = geometry
        .iter()
        .map(|variant| variant.call_mode)
        .collect::<BTreeSet<_>>();
    AbilityCallableFlags {
        read_only: geometry.iter().all(|variant| variant.flags.read_only),
        destructive: geometry.iter().any(|variant| variant.flags.destructive),
        idempotent: geometry.iter().all(|variant| variant.flags.idempotent),
        streaming_only: call_modes.len() == 1 && call_modes.contains(&CallMode::Stream),
        bidi_only: call_modes.len() == 1 && call_modes.contains(&CallMode::Bidi),
    }
}

fn aggregate_descriptor_revision(
    ability_ura: &str,
    geometry: &[AbilityCallModeGeometry],
) -> String {
    if let [only] = geometry {
        return only.descriptor_revision.clone();
    }
    prefixed_hash_value(&json!({
        "ability_ura": ability_ura,
        "mode_geometry": geometry,
    }))
}

fn aggregate_schema_hash(ability_ura: &str, geometry: &[AbilityCallModeGeometry]) -> String {
    if let [only] = geometry {
        return only.schema_hash.clone();
    }
    let schemas = geometry
        .iter()
        .map(|variant| {
            json!({
                "call_mode": variant.call_mode,
                "descriptor_version": variant.descriptor_version,
                "admission_action": variant.admission_action,
                "schema_hash": variant.schema_hash,
            })
        })
        .collect::<Vec<_>>();
    prefixed_hash_value(&json!({
        "ability_ura": ability_ura,
        "schemas": schemas,
    }))
}

fn validate_mode_geometry(summary: &AbilityProjectionSummary) -> Result<(), String> {
    let incoming = &summary.callable_summary.mode_geometry;
    if incoming.is_empty() {
        return Err(format!(
            "ability `{}` must publish governed mode geometry",
            summary.ability_ura
        ));
    }

    for variant in incoming {
        if !crate::daemon::ability::descriptors::is_valid_descriptor_version(
            &variant.descriptor_version,
        ) {
            return Err(format!(
                "ability `{}` has invalid {} descriptor version `{}`",
                summary.ability_ura,
                variant.call_mode.as_str(),
                variant.descriptor_version
            ));
        }
        for (field, value) in [
            ("descriptor_revision", variant.descriptor_revision.as_str()),
            ("schema_hash", variant.schema_hash.as_str()),
            ("policy_hash", variant.policy_hash.as_str()),
        ] {
            if !is_prefixed_sha256(value) {
                return Err(format!(
                    "ability `{}` {} {} must be a sha256 proof",
                    summary.ability_ura,
                    variant.call_mode.as_str(),
                    field
                ));
            }
        }
        if !is_valid_admission_action(&variant.admission_action) {
            return Err(format!(
                "ability `{}` {} admission_action must be one of read/invoke/stream/manage/grant",
                summary.ability_ura,
                variant.call_mode.as_str()
            ));
        }
        if variant.policy_ref.trim() != variant.policy_ref || variant.policy_ref.is_empty() {
            return Err(format!(
                "ability `{}` {} policy_ref must be non-empty and trimmed",
                summary.ability_ura,
                variant.call_mode.as_str()
            ));
        }
        let mode_tag = format!("mode:{}", variant.call_mode.as_str());
        let mut canonical_tags = variant.tags.clone();
        canonical_tags.sort();
        canonical_tags.dedup();
        if canonical_tags != variant.tags || !variant.tags.contains(&mode_tag) {
            return Err(format!(
                "ability `{}` {} geometry has non-canonical mode tags",
                summary.ability_ura,
                variant.call_mode.as_str()
            ));
        }
        if variant.flags.streaming_only != (variant.call_mode == CallMode::Stream)
            || variant.flags.bidi_only != (variant.call_mode == CallMode::Bidi)
        {
            return Err(format!(
                "ability `{}` {} geometry has contradictory transport flags",
                summary.ability_ura,
                variant.call_mode.as_str()
            ));
        }
    }

    let geometry = canonicalize_mode_geometry(incoming.clone())?;
    if geometry.len() != incoming.len() || &geometry != incoming {
        return Err(format!(
            "ability `{}` mode geometry must be sorted and unique by call_mode/version",
            summary.ability_ura
        ));
    }
    let primary = geometry
        .first()
        .expect("non-empty geometry has a primary descriptor");
    if primary.policy_ref != summary.policy_ref {
        return Err(format!(
            "ability `{}` primary mode policy_ref does not match the projection",
            summary.ability_ura
        ));
    }
    let public_name = summary_public_name(summary).ok_or_else(|| {
        format!(
            "ability `{}` cannot derive a public name",
            summary.ability_ura
        )
    })?;
    if summary.callable_summary.public_name != public_name
        || summary.callable_summary.description != primary.description
        || summary.callable_summary.call_mode != primary.call_mode
        || summary.callable_summary.receipt_semantics != primary.receipt_semantics
        || summary.callable_summary.input_fields != primary.input_fields
        || summary.callable_summary.flags != aggregate_callable_flags(&geometry)
    {
        return Err(format!(
            "ability `{}` scalar callable summary does not match its mode geometry",
            summary.ability_ura
        ));
    }
    let expected_descriptor_revision =
        aggregate_descriptor_revision(&summary.ability_ura, &geometry);
    let expected_schema_hash = aggregate_schema_hash(&summary.ability_ura, &geometry);
    if summary.descriptor_revision != expected_descriptor_revision
        || summary.schema_hash.as_deref() != Some(expected_schema_hash.as_str())
    {
        return Err(format!(
            "ability `{}` aggregate descriptor/schema proof does not match its mode geometry",
            summary.ability_ura
        ));
    }
    let expected_tags = geometry
        .iter()
        .flat_map(|variant| variant.tags.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if summary.tags != expected_tags {
        return Err(format!(
            "ability `{}` aggregate tags do not match its mode geometry",
            summary.ability_ura
        ));
    }
    Ok(())
}

pub(crate) fn descriptor_ref_for_summary_call_mode(
    summary: &AbilityProjectionSummary,
    call_mode: CallMode,
) -> anyhow::Result<String> {
    let public_name = summary_public_name(summary).ok_or_else(|| {
        anyhow::anyhow!(
            "ability projection `{}` cannot derive a public ability name",
            summary.ability_ura
        )
    })?;
    let variant = summary
        .callable_summary
        .mode_geometry
        .iter()
        .find(|variant| variant.call_mode == call_mode)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability projection `{}` has no {} descriptor geometry",
                summary.ability_ura,
                call_mode.as_str()
            )
        })?;
    let descriptor_hash = prefixed_sha256_bytes(&variant.descriptor_revision)
        .map_err(|err| anyhow::anyhow!("ability projection `{}` {err}", summary.ability_ura))?;
    let descriptor_binding =
        crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
            &variant.descriptor_version,
            descriptor_hash,
            &variant.admission_action,
        )
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
        &summary.owner_ura,
        &public_name,
        &descriptor_binding,
    )
    .map_err(|err| anyhow::anyhow!("{err}"))
}

fn is_prefixed_sha256(value: &str) -> bool {
    prefixed_sha256_bytes(value).is_ok()
}

fn prefixed_sha256_bytes(value: &str) -> Result<[u8; 32], String> {
    let digest = value
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("sha256 proof `{value}` must use sha256:<hex> form"))?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "sha256 proof `{value}` must contain exactly 32 bytes of hex"
        ));
    }
    let decoded = hex::decode(digest).map_err(|err| format!("sha256 proof `{value}`: {err}"))?;
    decoded.try_into().map_err(|bytes: Vec<u8>| {
        format!("sha256 proof `{value}` decoded to {} bytes", bytes.len())
    })
}

fn is_valid_admission_action(value: &str) -> bool {
    matches!(value, "read" | "invoke" | "stream" | "manage" | "grant")
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
        call_mode: descriptor.call_mode(),
        receipt_semantics: descriptor.receipt_semantics().clone(),
        input_fields: input_field_summaries(&descriptor.schema_summary.input),
        flags: AbilityCallableFlags {
            read_only: descriptor.hints.read_only,
            destructive: descriptor.hints.destructive,
            idempotent: descriptor.hints.idempotent,
            streaming_only: descriptor.call_mode() == CallMode::Stream,
            bidi_only: descriptor.call_mode() == CallMode::Bidi,
        },
        mode_geometry: Vec::new(),
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
    generation: u64,
    summaries: &[AbilityProjectionSummary],
) -> String {
    hash_value_hex(&canonical_projection_json(
        owner_ura,
        host_device_ura,
        generation,
        0,
        0,
        summaries,
    ))
}

fn projection_digest(
    owner_ura: &str,
    host_device_ura: &str,
    generation: u64,
    projection_revision: u64,
    lease_expires_unix_ms: i64,
    summaries: &[AbilityProjectionSummary],
) -> String {
    hash_value_hex(&canonical_projection_json(
        owner_ura,
        host_device_ura,
        generation,
        projection_revision,
        lease_expires_unix_ms,
        summaries,
    ))
}

fn canonical_projection_json(
    owner_ura: &str,
    host_device_ura: &str,
    generation: u64,
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
        "generation": generation,
        "projection_revision": projection_revision,
        "lease_expires_unix_ms": lease_expires_unix_ms,
        "abilities": ability_values,
    })
}

fn canonical_summary_json(summary: &AbilityProjectionSummary) -> Value {
    let mut callable_summary = summary.callable_summary.clone();
    callable_summary
        .mode_geometry
        .sort_by(compare_mode_geometry);
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
        "callable_summary": callable_summary,
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
    let parsed = crate::core::ura::parse_ura(subject_ura)
        .map_err(|e| format!("invalid subject_ura {subject_ura:?}: {e}"))?;
    match parsed.kind {
        crate::core::ura::URAKind::Agent => Ok(AgentSkillSubjectProjection {
            agent_ura: subject_ura.to_string(),
            skill_name: None,
        }),
        crate::core::ura::URAKind::Resource => {
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
                agent_ura: crate::core::ura::agent_ura(&parsed.realm, &user_id, &agent_id),
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
    let parsed = crate::core::ura::parse_ura(agent_ura).ok()?;
    if parsed.kind != crate::core::ura::URAKind::Agent {
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
    Some(crate::core::ura::resource_dot_ura(
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

fn prefixed_hash_value(value: &Value) -> String {
    format!("sha256:{}", hash_value_hex(value))
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

    const CURSOR_WRITER_CHILD_ENV: &str = "EASYNET_OWNER_CURSOR_WRITER_CHILD";
    const CURSOR_WRITER_OWNER_ENV: &str = "EASYNET_OWNER_CURSOR_WRITER_OWNER";
    use crate::daemon::ability::descriptors::AbilityDescriptor;

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
        AbilityDescriptor::new(
            name,
            owner,
            Visibility::Public,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
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
        assert_eq!(summary.callable_summary.call_mode, CallMode::Rpc);
        assert_eq!(
            summary.callable_summary.receipt_semantics,
            ReceiptSemantics::Operational
        );
        assert_eq!(prepared.publication.projection_revision, 1);
        // C4: lease cancelled (ISS-002) — projections publish lease=0.
        assert_eq!(prepared.publication.lease_expires_unix_ms, 0);
    }

    #[test]
    fn canonical_ability_publication_merges_rpc_stream_and_bidi_geometry() {
        let owner = "easynet:///r/acme/agent/alice.bot";
        let rpc = descriptor("chat", owner)
            .with_version("1.0.0")
            .unwrap()
            .with_call_mode(CallMode::Rpc);
        let stream = descriptor("chat", owner)
            .with_version("1.1.0")
            .unwrap()
            .with_call_mode(CallMode::Stream);
        let bidi = descriptor("chat", owner)
            .with_version("2.0.0")
            .unwrap()
            .with_call_mode(CallMode::Bidi);
        let descriptors = vec![stream.clone(), rpc.clone(), bidi.clone(), rpc.clone()];

        let prepared = prepare_at(
            owner,
            "easynet:///r/acme/device/dev-1",
            &descriptors,
            &OwnerProjectionCursorFile::default(),
            1_000,
        )
        .expect("multi-mode ability projection");

        assert_eq!(prepared.publication.ability_summaries.len(), 1);
        let summary = &prepared.publication.ability_summaries[0];
        assert_eq!(
            summary.ability_ura,
            "easynet:///r/acme/ability/alice.bot.chat"
        );
        assert_eq!(
            summary
                .callable_summary
                .mode_geometry
                .iter()
                .map(|variant| (variant.call_mode, variant.descriptor_version.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (CallMode::Rpc, "1.0.0"),
                (CallMode::Stream, "1.1.0"),
                (CallMode::Bidi, "2.0.0"),
            ]
        );
        for descriptor in [&rpc, &stream, &bidi] {
            let variant = summary
                .callable_summary
                .mode_geometry
                .iter()
                .find(|variant| variant.call_mode == descriptor.call_mode())
                .expect("mode geometry retained");
            assert_eq!(variant.descriptor_version, descriptor.version);
            assert_eq!(
                variant.descriptor_revision,
                descriptor.descriptor_hash_prefixed()
            );
            assert_eq!(variant.schema_hash, descriptor.schema_hash_prefixed());
            assert_eq!(
                variant.policy_hash,
                descriptor.access_policy_hash_prefixed()
            );
        }
        assert_eq!(summary.callable_summary.call_mode, CallMode::Rpc);
        assert!(!summary.callable_summary.flags.streaming_only);
        assert!(!summary.callable_summary.flags.bidi_only);
        assert!(summary.tags.contains(&"mode:rpc".to_string()));
        assert!(summary.tags.contains(&"mode:stream".to_string()));
        assert!(summary.tags.contains(&"mode:bidi".to_string()));
        prepared
            .publication
            .validate_integrity()
            .expect("receiver accepts canonical multi-mode ability unit");
    }

    #[test]
    fn multi_mode_publication_is_independent_of_descriptor_input_order() {
        let owner = "easynet:///r/acme/agent/alice.bot";
        let host = "easynet:///r/acme/device/dev-1";
        let rpc = descriptor("chat", owner).with_call_mode(CallMode::Rpc);
        let stream = descriptor("chat", owner).with_call_mode(CallMode::Stream);

        let first = prepare_at(
            owner,
            host,
            &[rpc.clone(), stream.clone()],
            &OwnerProjectionCursorFile::default(),
            1_000,
        )
        .expect("first order");
        let second = prepare_at(
            owner,
            host,
            &[stream, rpc],
            &OwnerProjectionCursorFile::default(),
            2_000,
        )
        .expect("reverse order");

        assert_eq!(first.publication, second.publication);
        assert_eq!(
            first.cursor.content_fingerprint,
            second.cursor.content_fingerprint
        );
    }

    #[test]
    fn conflicting_same_mode_and_version_is_rejected_before_publication() {
        let owner = "easynet:///r/acme/agent/alice.bot";
        let first = descriptor("chat", owner).with_call_mode(CallMode::Rpc);
        let conflicting = descriptor("chat", owner)
            .with_call_mode(CallMode::Rpc)
            .with_input_schema(json!({
                "type": "object",
                "properties": {"message": {"type": "string"}}
            }));

        let error = prepare_at(
            owner,
            "easynet:///r/acme/device/dev-1",
            &[first, conflicting],
            &OwnerProjectionCursorFile::default(),
            1_000,
        )
        .expect_err("same mode/version cannot carry divergent proofs");

        assert!(
            error.contains("conflicting rpc descriptor version `1.0.0`"),
            "{error}"
        );
    }

    #[test]
    fn integrity_rejects_rehashed_mode_geometry_tampering() {
        let owner = "easynet:///r/acme/agent/alice.bot";
        let mut publication = prepare_at(
            owner,
            "easynet:///r/acme/device/dev-1",
            &[
                descriptor("chat", owner).with_call_mode(CallMode::Rpc),
                descriptor("chat", owner).with_call_mode(CallMode::Stream),
            ],
            &OwnerProjectionCursorFile::default(),
            1_000,
        )
        .expect("projection")
        .publication;
        publication.ability_summaries[0]
            .callable_summary
            .mode_geometry[1]
            .descriptor_revision = format!("sha256:{}", "0".repeat(64));
        publication.projection_digest = publication.canonical_digest();

        let error = publication
            .validate_integrity()
            .expect_err("top-level descriptor proof must bind mode geometry");
        assert!(
            error.contains("aggregate descriptor/schema proof"),
            "{error}"
        );
    }

    #[test]
    fn integrity_rejects_descriptor_rows_without_mode_geometry() {
        let owner = "easynet:///r/acme/agent/alice.bot";
        let mut publication = prepare_at(
            owner,
            "easynet:///r/acme/device/dev-1",
            &[descriptor("chat", owner)],
            &OwnerProjectionCursorFile::default(),
            1_000,
        )
        .expect("projection")
        .publication;
        publication.ability_summaries[0]
            .callable_summary
            .mode_geometry
            .clear();
        publication.projection_digest = publication.canonical_digest();

        let error = publication
            .validate_integrity()
            .expect_err("descriptor rows require canonical mode geometry");
        assert!(
            error.contains("must publish governed mode geometry"),
            "{error}"
        );
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
    fn journaled_removal_retains_exact_cursor_until_compare_and_retire() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let owner = "easynet:///r/acme/agent/alice.worker";
        let host = "easynet:///r/acme/device/dev-1";
        prepare_and_persist(owner, host, &[descriptor("worker.chat", owner)]).unwrap();

        let tombstone = prepare_journaled_removal(owner)
            .unwrap()
            .expect("existing projection produces tombstone");
        let persisted = owner_projections::load().unwrap();
        let cursor = persisted
            .cursor_for(owner)
            .expect("tombstone cursor retained");
        assert_eq!(cursor.projection_revision, tombstone.projection_revision);
        assert_eq!(cursor.projection_digest, tombstone.projection_digest);
        assert!(tombstone.ability_summaries.is_empty());

        assert!(
            retire_removal_cursor(owner, tombstone.projection_revision, "different-digest")
                .is_err()
        );
        assert!(owner_projections::load()
            .unwrap()
            .cursor_for(owner)
            .is_some());

        retire_removal_cursor(
            owner,
            tombstone.projection_revision,
            &tombstone.projection_digest,
        )
        .unwrap();
        let retired = owner_projections::load().unwrap();
        let cursor = retired
            .cursor_for(owner)
            .expect("retirement preserves revision high-water state");
        assert_eq!(cursor.lifecycle, OwnerProjectionCursorLifecycle::Retired);
        assert_eq!(cursor.projection_revision, tombstone.projection_revision);
        assert!(retired.active_cursor_for(owner).is_none());

        let recreated = prepare_and_persist(owner, host, &[descriptor("worker.chat", owner)])
            .expect("same URA may be recreated after purge");
        let active = owner_projections::load().unwrap();
        let recreated_cursor = active.active_cursor_for(owner).unwrap();
        assert!(recreated.projection_revision > tombstone.projection_revision);
        assert!(recreated_cursor.generation > cursor.generation);
    }

    #[test]
    fn owner_cursor_writer_child_process() {
        if std::env::var_os(CURSOR_WRITER_CHILD_ENV).is_none() {
            return;
        }
        let owner = std::env::var(CURSOR_WRITER_OWNER_ENV).unwrap();
        let host = "easynet:///r/acme/device/dev-1";
        let local_owner = owner.rsplit('.').next().unwrap();
        let ability = format!("{local_owner}.chat");
        prepare_and_persist(&owner, host, &[descriptor(&ability, &owner)])
            .expect("child writes owner projection under process lock");
    }

    #[test]
    fn owner_cursor_transactions_preserve_concurrent_process_writers() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut children = Vec::new();
        for index in 0..12 {
            let owner = crate::core::ura::agent_ura("acme", "user", &format!("worker-{index}"));
            children.push(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .arg("--exact")
                    .arg("daemon::federation::read_model::owner_projection::tests::owner_cursor_writer_child_process")
                    .arg("--nocapture")
                    .env(CURSOR_WRITER_CHILD_ENV, "1")
                    .env(CURSOR_WRITER_OWNER_ENV, owner)
                    .env("HOME", crate::daemon::persistence::config::home_dir())
                    .spawn()
                    .expect("spawn owner cursor writer"),
            );
        }
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
        let file = owner_projections::load().unwrap();
        assert_eq!(file.projections.len(), 12);
        assert!(file
            .projections
            .iter()
            .all(|cursor| cursor.lifecycle == OwnerProjectionCursorLifecycle::Active));
    }

    #[test]
    fn filesystem_summary_publishes_callable_fields_without_host_paths() {
        let owner = "easynet:///r/acme/device/01DEV";
        let descriptor = descriptor("fs.read", owner)
            .with_description(
                crate::daemon::ability::builtins::device_control::files::description_read(),
            )
            .with_input_schema(
                crate::daemon::ability::builtins::device_control::files::input_schema_read(),
            );
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

    /// Guards the SPEC §15.1-2 contract-drift invariant documented on
    /// `AbilityProjectionSummary`: `callable_summary` is wire-carried (it
    /// must survive the serde JSON round-trip the federation envelope uses),
    /// AND a discovery payload that omits it — as a proto-only consumer may
    /// emit — must still decode via `#[serde(default)]`. Such a lossy row is
    /// deliberately rejected by `OwnerProjectionPublication::validate_integrity`
    /// and remains usable only by read-only discovery projection.
    #[test]
    fn callable_summary_survives_projection_wire_roundtrip() {
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
            callable_summary: AbilityCallableSummary {
                public_name: "fs.read".into(),
                description: "read a file".into(),
                call_mode: CallMode::Rpc,
                receipt_semantics: ReceiptSemantics::Operational,
                ..AbilityCallableSummary::default()
            },
        };

        // 1. Wire-carried: a full serialize -> deserialize preserves the
        //    daemon extension across the federation envelope shape.
        let wire = serde_json::to_value(&summary).expect("summary serializes");
        assert_eq!(
            wire.get("callable_summary")
                .and_then(|cs| cs.get("description"))
                .and_then(Value::as_str),
            Some("read a file"),
            "callable_summary must be present on the projection wire shape"
        );
        assert_eq!(wire["callable_summary"]["call_mode"], "rpc");
        assert_eq!(
            wire["callable_summary"]["receipt_semantics"]["kind"],
            "operational"
        );
        assert!(wire["callable_summary"].get("ability_class").is_none());
        let decoded: AbilityProjectionSummary =
            serde_json::from_value(wire).expect("summary round-trips");
        assert_eq!(decoded, summary);

        // 2. A proto-shaped discovery row still decodes for read-only listing.
        //    Owner publication admission separately requires mode geometry.
        let proto_shaped = json!({
            "ability_ura": "easynet:///r/acme/ability/device.01DEV.fs.read",
            "owner_ura": "easynet:///r/acme/device/01DEV",
            "namespace": "fs",
            "local_name": "read",
            "descriptor_revision": "sha256:descriptor",
            "schema_ref": null,
            "schema_hash": null,
            "policy_ref": "visibility:PUBLIC",
            "route_summary_ref": null,
            "tags": [],
        });
        let without_extension: AbilityProjectionSummary =
            serde_json::from_value(proto_shaped).expect("proto-shaped payload decodes");
        assert_eq!(
            without_extension.callable_summary,
            AbilityCallableSummary::default(),
            "omitted callable_summary must default, not fail to parse"
        );
    }

    fn cursor(
        owner_ura: &str,
        host_device_ura: &str,
        projection_revision: u64,
    ) -> OwnerProjectionCursor {
        OwnerProjectionCursor {
            owner_ura: owner_ura.into(),
            host_device_ura: host_device_ura.into(),
            generation: 1,
            lifecycle: OwnerProjectionCursorLifecycle::Active,
            projection_revision,
            projection_digest: format!("digest-{owner_ura}-{projection_revision}"),
            content_fingerprint: format!("fingerprint-{owner_ura}-{projection_revision}"),
            lease_expires_unix_ms: 61_000,
            updated_at: "1970-01-01T00:00:01.000Z".into(),
        }
    }
}
