// EasyNet CLI — Local Resources Registry (RFC-005 v3.2 §"Resource URA")
// =====================================================================
//
// File: src/daemon/persistence/resources.rs
//
// Owns ~/.easynet/resources.json — the persistent map from a stable
// `hardware_id` (CoreAudio/PulseAudio device UID, USB serial, EDID,
// camera device-path, etc.) to a canonical resource URA. Mode 0600
// because resource URAs become subjects in policy decisions, and
// stable mappings are part of the trust surface.
//
// Why this exists (INV-RESOURCE-ULID-STABLE)
// ------------------------------------------
// Per RFC-005 v3.2, every physical resource (mic, camera, display,
// speaker, etc.) gets a first-class URA of shape
// `easynet:///r/<realm>/resource/<id>` and is the `subject` of media
// invocations (mic.subscribe, camera.snapshot, …). A reboot must NOT
// re-mint the URA — otherwise every prior receipt and policy entry
// referencing that camera/mic becomes orphaned. This file is the
// persistence half of "stable URA across restarts": on first scan
// we mint, persist, and reuse on every subsequent boot.
//
// Schema
// ------
// {
//   "resources": [
//     {
//       "resource_ura":  "easynet:///r/<realm>/resource/<id>",
//       "owner_agent":   "easynet:///r/<realm>/agent/device.<device-id>.<system-agent-id>",
//       "type":          "mic" | "camera" | "display" | "application" |
//                        "window" | "speaker" | "voice" | "asr_model",
//       "binding":       "local_device" | "virtual",
//       "hardware_id":   "<platform stable id>",
//       "display_name":  "Built-in Microphone",
//       "metadata":      { ...codec hints, capabilities... },
//       "first_seen_at": "<rfc3339>"
//     }
//   ]
// }
//
// What this file is NOT
// ---------------------
// - Not a resource registry — there is no realm-level discovery
//   (deferred per plan v3.2). This is the *device-local* table only.
// - Not the live binding state. Whether a window is currently open,
//   a USB device is currently plugged in, etc., is determined at
//   handler invocation time (see INV-RESOURCE-VALIDITY: split
//   resource_not_found vs resource_unavailable). This file only
//   records "we have an entry for this hardware_id".
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};
use super::file_lock::ExclusiveFileLock;

pub(crate) const FILE_NAME: &str = "resources.json";

const RESOURCE_EPOCH_FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const RESOURCE_EPOCH_FNV_PRIME: u64 = 0x100000001b3;
const RESOURCE_EPOCH_JSON_SAFE_MAX: u64 = (1_u64 << 53) - 1;

fn write_resource_epoch(state: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *state ^= u64::from(*byte);
        *state = state.wrapping_mul(RESOURCE_EPOCH_FNV_PRIME);
    }
}

fn write_optional_resource_epoch_u64(state: &mut u64, value: Option<u64>) {
    write_resource_epoch(state, &[u8::from(value.is_some())]);
    if let Some(value) = value {
        write_resource_epoch(state, &value.to_le_bytes());
    }
}

/// Project one deterministic hash into the positive JSON integer domain shared
/// by Rust, Go, Python, and IEEE-754 Browser clients.
///
/// These values are opaque equality epochs, not cryptographic digests. Keeping
/// the canonical source within `2^53 - 1` prevents JavaScript from rounding a
/// selected target epoch before it is sent back to guarded Runtime abilities.
fn finalize_resource_epoch(state: u64) -> u64 {
    match state & RESOURCE_EPOCH_JSON_SAFE_MAX {
        0 => 1,
        epoch => epoch,
    }
}

/// Deterministic identity epoch for one committed application window set.
///
/// Resource discovery and RemoteApp session tracking must use the same
/// versioned byte representation. `DefaultHasher` is deliberately avoided:
/// its output is not a persisted cross-version contract and integer-width
/// differences between platform inventory and session types can create false
/// target rebinds.
pub(crate) fn application_window_set_epoch(
    display_id: Option<u64>,
    bundle_id: Option<&str>,
    primary_pid: Option<i64>,
    resolved_window_ids: &[u64],
) -> u64 {
    let mut resolved_window_ids = resolved_window_ids.to_vec();
    resolved_window_ids.sort_unstable();
    resolved_window_ids.dedup();

    let mut state = RESOURCE_EPOCH_FNV_OFFSET_BASIS;
    write_resource_epoch(&mut state, b"easynet.application-window-set.v1\0");
    write_optional_resource_epoch_u64(&mut state, display_id);
    match bundle_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(bundle_id) => {
            write_resource_epoch(&mut state, &[1]);
            write_resource_epoch(&mut state, &(bundle_id.len() as u64).to_le_bytes());
            write_resource_epoch(&mut state, bundle_id.as_bytes());
        }
        None => write_resource_epoch(&mut state, &[0]),
    }
    write_resource_epoch(&mut state, &[u8::from(primary_pid.is_some())]);
    if let Some(primary_pid) = primary_pid {
        write_resource_epoch(&mut state, &primary_pid.to_le_bytes());
    }
    write_resource_epoch(
        &mut state,
        &(resolved_window_ids.len() as u64).to_le_bytes(),
    );
    for window_id in &resolved_window_ids {
        write_resource_epoch(&mut state, &window_id.to_le_bytes());
    }
    finalize_resource_epoch(state)
}

/// Process-instance-aware application identity epoch for platforms where a
/// PID may be reused while an old RemoteApp binding still exists.
///
/// The v1 result is preserved when no process-instance identity is available,
/// so macOS/Windows bindings do not churn. Linux inventory supplies the
/// boot-scoped `/proc` identity and therefore uses the v2 domain.
pub(crate) fn application_window_set_epoch_with_process_instance(
    display_id: Option<u64>,
    bundle_id: Option<&str>,
    primary_pid: Option<i64>,
    process_instance_id: Option<&str>,
    resolved_window_ids: &[u64],
) -> u64 {
    let Some(process_instance_id) = process_instance_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return application_window_set_epoch(
            display_id,
            bundle_id,
            primary_pid,
            resolved_window_ids,
        );
    };
    let mut resolved_window_ids = resolved_window_ids.to_vec();
    resolved_window_ids.sort_unstable();
    resolved_window_ids.dedup();

    let mut state = RESOURCE_EPOCH_FNV_OFFSET_BASIS;
    write_resource_epoch(&mut state, b"easynet.application-window-set.v2\0");
    write_optional_resource_epoch_u64(&mut state, display_id);
    match bundle_id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(bundle_id) => {
            write_resource_epoch(&mut state, &[1]);
            write_resource_epoch(&mut state, &(bundle_id.len() as u64).to_le_bytes());
            write_resource_epoch(&mut state, bundle_id.as_bytes());
        }
        None => write_resource_epoch(&mut state, &[0]),
    }
    write_resource_epoch(&mut state, &[u8::from(primary_pid.is_some())]);
    if let Some(primary_pid) = primary_pid {
        write_resource_epoch(&mut state, &primary_pid.to_le_bytes());
    }
    write_resource_epoch(
        &mut state,
        &(process_instance_id.len() as u64).to_le_bytes(),
    );
    write_resource_epoch(&mut state, process_instance_id.as_bytes());
    write_resource_epoch(
        &mut state,
        &(resolved_window_ids.len() as u64).to_le_bytes(),
    );
    for window_id in &resolved_window_ids {
        write_resource_epoch(&mut state, &window_id.to_le_bytes());
    }
    finalize_resource_epoch(state)
}

/// Deterministic epoch for the concrete application surface composition.
///
/// Unlike `application_window_set_epoch`, input order is significant and is
/// the committed front-to-back z-order. Geometry is represented as integral
/// host pixels so CoreGraphics inventory, ScreenCaptureKit capture, recovery,
/// and target observation compare one stable contract.
pub(crate) fn application_surface_layout_epoch(
    front_to_back_surfaces: &[(u64, i64, i64, u64, u64)],
) -> u64 {
    let mut state = RESOURCE_EPOCH_FNV_OFFSET_BASIS;
    write_resource_epoch(&mut state, b"easynet.application-surface-layout.v1\0");
    write_resource_epoch(
        &mut state,
        &(front_to_back_surfaces.len() as u64).to_le_bytes(),
    );
    for (window_id, x, y, width, height) in front_to_back_surfaces {
        write_resource_epoch(&mut state, &window_id.to_le_bytes());
        write_resource_epoch(&mut state, &x.to_le_bytes());
        write_resource_epoch(&mut state, &y.to_le_bytes());
        write_resource_epoch(&mut state, &width.to_le_bytes());
        write_resource_epoch(&mut state, &height.to_le_bytes());
    }
    finalize_resource_epoch(state)
}

/// Resource type taxonomy — RFC-005 v3.2. The wire form is a
/// lowercase string, and every accepted v1 type is enumerated here so callers
/// cannot typo `"camera"` as `"cammera"` and silently misclassify.
///
/// `as_str` is the single source of truth: both the on-disk
/// JSON shape and the `meta.list_resources` ability schema's
/// `enum` list derive from `ALL` below. A new variant gets added
/// in one place; CI catches consumers that still hard-code old
/// strings (see `list_resources_ability::input_schema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Mic,
    Camera,
    Display,
    Application,
    Window,
    Speaker,
    Voice,
    AsrModel,
}

impl ResourceType {
    /// Canonical wire-form string. Same value the serde
    /// `rename_all = "snake_case"` produces; centralising via
    /// this method lets non-serde callers (TOML schema renderer,
    /// human-readable error messages) avoid hand-typing it.
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceType::Mic => "mic",
            ResourceType::Camera => "camera",
            ResourceType::Display => "display",
            ResourceType::Application => "application",
            ResourceType::Window => "window",
            ResourceType::Speaker => "speaker",
            ResourceType::Voice => "voice",
            ResourceType::AsrModel => "asr_model",
        }
    }

    /// All v1 variants, in canonical declaration order. Ability
    /// schemas, conformance tests, and admin UIs derive their
    /// type list from here so adding a new variant requires
    /// touching exactly one place.
    pub const ALL: &'static [ResourceType] = &[
        ResourceType::Mic,
        ResourceType::Camera,
        ResourceType::Display,
        ResourceType::Application,
        ResourceType::Window,
        ResourceType::Speaker,
        ResourceType::Voice,
        ResourceType::AsrModel,
    ];
}

impl std::str::FromStr for ResourceType {
    type Err = anyhow::Error;
    /// Parse the canonical wire-form string. Unknown values are
    /// rejected at the boundary; callers should not silently
    /// treat an unrecognised type as "no filter" — that would
    /// turn a typo into a permission widening on the result set.
    fn from_str(s: &str) -> anyhow::Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| anyhow::anyhow!("unknown resource type {s:?}"))
    }
}

/// `local_device` for hardware physically attached to this host;
/// `virtual` for synthesised resources (loopback mic, screen
/// recording proxy, etc.). Same single-source pattern as
/// `ResourceType` — wire form is snake_case, callers use the
/// typed enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceBinding {
    LocalDevice,
    Virtual,
}

impl ResourceBinding {
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceBinding::LocalDevice => "local_device",
            ResourceBinding::Virtual => "virtual",
        }
    }
}

/// On-disk shape of `~/.easynet/resources.json`. Field names must remain
/// stable, but only canonical current resource subjects are accepted.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourcesFile {
    /// All known resources held by this host. Order is insertion
    /// order; readers MUST NOT rely on order.
    pub resources: Vec<ResourceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceEntry {
    /// Canonical resource URA. Device-local media resources must use shape
    /// `easynet:///r/<realm>/resource/device.<device-id>/streams/<type>.<id>`.
    /// Retired single-segment local-device rows are rejected on upsert so
    /// operators clean and republish local state instead of silently rewriting
    /// subject authority.
    pub resource_ura: String,
    /// Owner Agent's URA. Device-local media resources require this to be a
    /// device-sponsored SystemAgent URA at insert/update time; pre-join
    /// local-device rows are no longer persisted.
    pub owner_agent: String,
    /// Resource type (renamed from `type` to a Rust-idiomatic
    /// `kind` so call sites avoid the `r#type` raw-identifier
    /// escape). Wire form unchanged: serde renames to `"type"`.
    #[serde(rename = "type")]
    pub kind: ResourceType,
    /// Hardware-vs-virtual binding. Wire form is the snake_case
    /// string of the variant.
    pub binding: ResourceBinding,
    /// Stable platform identifier the next boot will see. Used as
    /// the dedup key per **INV-RESOURCE-ULID-STABLE**: same
    /// `hardware_id` → same `resource_ura` across restarts.
    /// Examples:
    ///   - macOS audio:   `cpal::Device::name()` (stable across
    ///                    boots; opaque to us)
    ///   - macOS camera:  `nokhwa::CameraInfo::misc()`
    ///                    (vendor+product+serial)
    ///   - Linux PulseAudio: device's `node.name` property
    ///   - Display: EDID hash (vendor+model+serial bytes hashed)
    ///   - Application: bundle id (`com.google.Chrome`)
    ///   - Window: process_id+window_id (NOT stable across
    ///            restarts — window resources are short-lived;
    ///            we still persist so a fresh enumeration round-
    ///            trips identically within one session)
    pub hardware_id: String,
    /// Human-readable label. NOT used for routing; only for
    /// `meta.list_resources` UX. Free-form, may change between
    /// boots (e.g. user renamed display in System Preferences)
    /// without affecting `resource_ura`.
    pub display_name: String,
    /// Open-ended bag for codec hints, capabilities, max-
    /// resolution, supported sample rates, etc. Read by media
    /// handlers when negotiating args defaults.
    pub metadata: Value,
    /// RFC 3339 timestamp; useful for operator triage when an
    /// entry references hardware that's no longer present.
    pub first_seen_at: String,
}

/// Argument bundle for `upsert_resource`. A struct rather than
/// 7 positional `&str` parameters because at that count the
/// type system stops catching swap-typos (e.g. `display_name`
/// and `binding` both `&str`); a named-field call site reads as
/// the data being inserted, not as a parameter list.
#[derive(Debug, Clone)]
pub struct ResourceUpsert<'a> {
    pub realm: &'a str,
    pub owner_agent: &'a str,
    pub kind: ResourceType,
    pub binding: ResourceBinding,
    pub hardware_id: &'a str,
    pub display_name: &'a str,
    pub metadata: Value,
}

/// Resolve the on-disk path. Public so an integration test can
/// override `state_dir` via `XDG_CONFIG_HOME` without re-deriving
/// the layout.
pub fn path() -> PathBuf {
    state_dir().join(FILE_NAME)
}

/// Read the file. Returns an empty `ResourcesFile` if the file
/// does not exist (first-boot case). Returns Err only on parse
/// failure or unrecoverable I/O.
pub fn load() -> anyhow::Result<ResourcesFile> {
    let p = path();
    if !p.exists() {
        return Ok(ResourcesFile::default());
    }
    let bytes = fs::read(&p).map_err(|e| anyhow::anyhow!("read {}: {e}", p.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("parse {}: {e}", p.display()))
}

/// Atomically write the file with mode 0600.
pub fn save(file: &ResourcesFile) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("create_dir_all {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(file)?;
    atomic_write_with_permissions(&path(), json.as_bytes(), WritePermissions::OwnerReadWrite)
        .map_err(Into::into)
}

/// Execute one read-modify-write transaction against the resources table.
///
/// Callers that derive a new table from the current persisted table must use
/// this instead of open-coding `load(); mutate; save();`. Atomic file writes
/// protect readers from torn JSON, but they do not protect concurrent writers
/// from lost updates; the adjacent `<resources.json>.lock` serializes those
/// transactions across daemon/CLI processes sharing one local state directory.
pub fn update<R>(
    mutation: impl FnOnce(&mut ResourcesFile) -> anyhow::Result<(R, bool)>,
) -> anyhow::Result<R> {
    let data_path = path();
    let _guard = ExclusiveFileLock::acquire_for_data_path(&data_path)?;
    let mut file = load()?;
    let (result, should_save) = mutation(&mut file)?;
    if should_save {
        save(&file)?;
    }
    Ok(result)
}

/// Build the generic resource URA for a given realm + id.
///
/// Realm comes from credentials; id is the per-resource ULID/UUID minted on
/// first sight. Device-local media resources do not use this shape.
pub fn build_resource_ura(realm: &str, resource_id: &str) -> String {
    crate::core::ura::resource_dot_ura(realm, resource_id, "")
}

fn build_device_stream_resource_ura(
    realm: &str,
    device_id: &str,
    kind: ResourceType,
    resource_id: &str,
) -> String {
    crate::core::ura::resource_dot_ura(
        realm,
        &format!("device.{device_id}"),
        &format!("streams/{}.{}", kind.as_str(), resource_id),
    )
}

fn local_device_resource_host_device_id(realm: &str, owner_agent: &str) -> Option<String> {
    let parsed = crate::core::ura::parse_ura(owner_agent).ok()?;
    if parsed.kind != crate::core::ura::URAKind::Agent || parsed.realm != realm {
        return None;
    }
    parsed
        .device_agent_ids()
        .map(|(device_id, _system_agent_id)| device_id.to_string())
}

fn canonical_resource_ura_for_new(
    spec: &ResourceUpsert<'_>,
    resource_id: &str,
) -> anyhow::Result<String> {
    if spec.binding == ResourceBinding::LocalDevice {
        let device_id = local_device_resource_host_device_id(spec.realm, spec.owner_agent)
            .ok_or_else(|| {
            anyhow::anyhow!(
                "local-device resource {:?} requires owner_agent to be a device-sponsored SystemAgent URA in realm {:?}",
                spec.hardware_id,
                spec.realm
            )
        })?;
        return Ok(build_device_stream_resource_ura(
            spec.realm,
            &device_id,
            spec.kind,
            resource_id,
        ));
    }
    Ok(build_resource_ura(spec.realm, resource_id))
}

fn ensure_existing_resource_ura_is_canonical(
    realm: &str,
    owner_agent: &str,
    kind: ResourceType,
    entry: &ResourceEntry,
) -> anyhow::Result<()> {
    if entry.binding != ResourceBinding::LocalDevice {
        return Ok(());
    }
    let device_id = local_device_resource_host_device_id(realm, owner_agent).ok_or_else(|| {
        anyhow::anyhow!(
            "local-device resource {:?} requires owner_agent to be a device-sponsored SystemAgent URA in realm {:?}",
            entry.hardware_id,
            realm
        )
    })?;
    let parsed = crate::core::ura::parse_ura(&entry.resource_ura).map_err(|error| {
        anyhow::anyhow!(
            "local-device resource {:?} has invalid resource_ura {:?}: {error}",
            entry.hardware_id,
            entry.resource_ura
        )
    })?;
    let expected_owner = format!("device.{device_id}");
    let expected_prefix = format!("streams/{}.", kind.as_str());
    if parsed.kind == crate::core::ura::URAKind::Resource
        && parsed.resource_owner_id() == Some(expected_owner.as_str())
        && parsed
            .resource_path()
            .is_some_and(|path| path.starts_with(&expected_prefix))
    {
        return Ok(());
    }
    anyhow::bail!(
        "local-device resource {:?} uses retired subject {:?}; delete resources.json and let \
         bootstrap republish canonical device-stream resource URAs",
        entry.hardware_id,
        entry.resource_ura
    )
}

/// Look up an entry by `hardware_id`. Returns the existing URA when
/// present (caller MUST reuse it per **INV-RESOURCE-ULID-STABLE**),
/// `None` when first sighting (caller mints a fresh id).
pub fn lookup_by_hardware_id<'a>(
    file: &'a ResourcesFile,
    hardware_id: &str,
) -> Option<&'a ResourceEntry> {
    file.resources.iter().find(|e| e.hardware_id == hardware_id)
}

/// Look up an entry by its full resource URA. Used by media
/// handlers to map subject → entry at invocation time.
pub fn lookup_by_ura<'a>(file: &'a ResourcesFile, resource_ura: &str) -> Option<&'a ResourceEntry> {
    file.resources
        .iter()
        .find(|e| e.resource_ura == resource_ura)
}

/// Insert or update a resource entry keyed on `hardware_id`.
///
/// Returns the resulting `resource_ura`:
/// - if `hardware_id` already exists in the file, returns its existing stable
///   resource id after verifying the stored subject is already canonical.
///   Retired local-device URAs fail closed instead of being migrated.
///   Mutates `display_name` / `metadata` / `owner_agent` so a renamed device
///   propagates.
/// - else mints a fresh UUIDv4-based id, builds a new URA via
///   `canonical_resource_ura_for_new`, appends the entry, and returns the new
///   URA.
///
/// `kind` and `binding` on a hardware_id MUST not change between
/// upserts — those are properties of the physical resource, not
/// of this call. The helper accepts them on every upsert for the
/// fresh-insert path; on the update path it leaves the existing
/// values untouched (a renamed-but-same-hardware device must not
/// silently flip from `mic` to `camera`).
pub fn upsert_resource(
    file: &mut ResourcesFile,
    spec: ResourceUpsert<'_>,
) -> anyhow::Result<String> {
    let now = chrono::Utc::now().to_rfc3339();
    let existing_index = file
        .resources
        .iter()
        .position(|entry| entry.hardware_id == spec.hardware_id);
    apply_resource_upsert(file, spec, existing_index, &now)
}

/// Insert/update a batch of resource entries using one hardware-id index.
///
/// This preserves the same canonical URA and update semantics as
/// `upsert_resource`, but avoids the O(R*N) scan pattern that is unacceptable
/// for live remote-target inventory refreshes with many persisted rows and
/// many current windows/applications.
pub fn upsert_resources_indexed<'a>(
    file: &mut ResourcesFile,
    specs: impl IntoIterator<Item = ResourceUpsert<'a>>,
) -> anyhow::Result<Vec<String>> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut index = file
        .resources
        .iter()
        .enumerate()
        .map(|(idx, entry)| (entry.hardware_id.clone(), idx))
        .collect::<HashMap<_, _>>();
    let mut resource_uras = Vec::new();
    for spec in specs {
        let existing_index = index.get(spec.hardware_id).copied();
        let hardware_id = spec.hardware_id.to_string();
        let resource_ura = apply_resource_upsert(file, spec, existing_index, &now)?;
        if existing_index.is_none() {
            index.insert(hardware_id, file.resources.len() - 1);
        }
        resource_uras.push(resource_ura);
    }
    Ok(resource_uras)
}

fn apply_resource_upsert(
    file: &mut ResourcesFile,
    spec: ResourceUpsert<'_>,
    existing_index: Option<usize>,
    now: &str,
) -> anyhow::Result<String> {
    if let Some(index) = existing_index {
        let entry = file.resources.get(index).ok_or_else(|| {
            anyhow::anyhow!(
                "resource upsert index for hardware_id {:?} is out of bounds",
                spec.hardware_id
            )
        })?;
        ensure_existing_resource_ura_is_canonical(spec.realm, spec.owner_agent, entry.kind, entry)?;
        let entry = &mut file.resources[index];
        entry.owner_agent = spec.owner_agent.to_string();
        entry.display_name = spec.display_name.to_string();
        entry.metadata = spec.metadata;
        return Ok(entry.resource_ura.clone());
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let resource_ura = canonical_resource_ura_for_new(&spec, &id)?;
    file.resources.push(ResourceEntry {
        resource_ura: resource_ura.clone(),
        owner_agent: spec.owner_agent.to_string(),
        kind: spec.kind,
        binding: spec.binding,
        hardware_id: spec.hardware_id.to_string(),
        display_name: spec.display_name.to_string(),
        metadata: spec.metadata,
        first_seen_at: now.to_string(),
    });
    Ok(resource_ura)
}

/// Filtered view of all entries whose `kind` is in `types`. When
/// `types` is empty, returns all. Used by `meta.list_resources`
/// to honour its `args.types` filter; the handler parses the
/// caller's strings into `ResourceType` first so unknown types
/// reject at the boundary instead of silently matching nothing.
pub fn filter_by_kinds<'a>(
    file: &'a ResourcesFile,
    kinds: &[ResourceType],
) -> Vec<&'a ResourceEntry> {
    if kinds.is_empty() {
        return file.resources.iter().collect();
    }
    file.resources
        .iter()
        .filter(|e| kinds.contains(&e.kind))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty() -> ResourcesFile {
        ResourcesFile::default()
    }

    #[test]
    fn application_window_set_epoch_is_canonical_across_callers() {
        let canonical =
            application_window_set_epoch(None, Some("com.example.Editor"), Some(9001), &[10, 11]);
        assert_eq!(
            canonical,
            application_window_set_epoch(
                None,
                Some(" com.example.Editor "),
                Some(9001),
                &[11, 10, 10],
            )
        );
        assert_ne!(
            canonical,
            application_window_set_epoch(None, Some("com.example.Editor"), Some(9001), &[10, 12],)
        );
        assert!((1..=RESOURCE_EPOCH_JSON_SAFE_MAX).contains(&canonical));
    }

    #[test]
    fn process_instance_epoch_rejects_pid_reuse_without_churning_v1_callers() {
        let v1 =
            application_window_set_epoch(None, Some("com.example.Editor"), Some(9001), &[10, 11]);
        assert_eq!(
            application_window_set_epoch_with_process_instance(
                None,
                Some("com.example.Editor"),
                Some(9001),
                None,
                &[11, 10],
            ),
            v1,
        );
        let first = application_window_set_epoch_with_process_instance(
            None,
            Some("com.example.Editor"),
            Some(9001),
            Some("linux:boot-a:9001:100"),
            &[10, 11],
        );
        let reused = application_window_set_epoch_with_process_instance(
            None,
            Some("com.example.Editor"),
            Some(9001),
            Some("linux:boot-a:9001:200"),
            &[10, 11],
        );
        assert_ne!(first, reused);
        assert!((1..=(1_u64 << 53) - 1).contains(&first));
        assert!((1..=(1_u64 << 53) - 1).contains(&reused));
    }

    #[test]
    fn application_surface_layout_epoch_tracks_geometry_and_z_order() {
        let front_to_back = [(10, -100, 20, 800, 600), (11, 40, 60, 400, 300)];
        let canonical = application_surface_layout_epoch(&front_to_back);
        assert_eq!(canonical, application_surface_layout_epoch(&front_to_back));
        assert_ne!(
            canonical,
            application_surface_layout_epoch(&[(11, 40, 60, 400, 300), (10, -100, 20, 800, 600)])
        );
        assert_ne!(
            canonical,
            application_surface_layout_epoch(&[(10, -99, 20, 800, 600), (11, 40, 60, 400, 300)])
        );
        assert!((1..=RESOURCE_EPOCH_JSON_SAFE_MAX).contains(&canonical));
    }

    #[test]
    fn resource_epoch_finalizer_never_emits_zero_or_an_unsafe_json_integer() {
        assert_eq!(finalize_resource_epoch(0), 1);
        assert_eq!(
            finalize_resource_epoch(u64::MAX),
            RESOURCE_EPOCH_JSON_SAFE_MAX
        );
    }

    /// One-line spec builder for tests so each call site fits on
    /// a screen and reads as data, not as a parameter list.
    fn spec<'a>(
        kind: ResourceType,
        hardware_id: &'a str,
        display_name: &'a str,
    ) -> ResourceUpsert<'a> {
        ResourceUpsert {
            realm: "acme",
            owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
            kind,
            binding: ResourceBinding::LocalDevice,
            hardware_id,
            display_name,
            metadata: json!({}),
        }
    }

    #[test]
    fn load_missing_file_returns_default() {
        let f = ResourcesFile::default();
        assert!(f.resources.is_empty());
    }

    #[test]
    fn build_resource_ura_uses_canonical_shape() {
        assert_eq!(
            build_resource_ura("acme", "01ABC"),
            "easynet:///r/acme/resource/01ABC"
        );
    }

    #[test]
    fn upsert_inserts_when_hardware_id_absent() {
        let mut f = empty();
        let ura = upsert_resource(
            &mut f,
            ResourceUpsert {
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                metadata: json!({"sample_rates":[16000,48000]}),
                ..spec(
                    ResourceType::Mic,
                    "BuiltInMic-AAPL-0001",
                    "Built-in Microphone",
                )
            },
        );
        let ura = ura.expect("insert local-device resource");
        assert!(ura.starts_with(&crate::core::ura::realm_resource_prefix("acme")));
        let parsed = crate::core::ura::parse_ura(&ura).expect("resource URA must parse");
        assert_eq!(parsed.kind, crate::core::ura::URAKind::Resource);
        assert_eq!(parsed.realm, "acme");
        assert_eq!(
            parsed.resource_owner_id(),
            Some("device.01DEV"),
            "device-local media resources must be bound to the callee device for hub subject authorization; got {ura}"
        );
        assert!(
            parsed
                .resource_path()
                .is_some_and(|path| path.starts_with("streams/mic.")),
            "device-local mic resources must use stream subject path; got {ura}"
        );
        assert_eq!(f.resources.len(), 1);
        assert_eq!(f.resources[0].kind, ResourceType::Mic);
        assert_eq!(f.resources[0].hardware_id, "BuiltInMic-AAPL-0001");
        assert_eq!(f.resources[0].display_name, "Built-in Microphone");
        assert_eq!(
            f.resources[0].owner_agent,
            "easynet:///r/acme/agent/device.01DEV.media"
        );
    }

    #[test]
    fn upsert_rejects_retired_local_device_resource_ura_without_rewrite() {
        let mut f = ResourcesFile {
            resources: vec![ResourceEntry {
                resource_ura: "easynet:///r/acme/resource/RETIRED01".to_string(),
                owner_agent: "".to_string(),
                kind: ResourceType::Camera,
                binding: ResourceBinding::LocalDevice,
                hardware_id: "Camera-USB-12345".to_string(),
                display_name: "Old Camera".to_string(),
                metadata: json!({}),
                first_seen_at: "2026-01-01T00:00:00Z".to_string(),
            }],
        };
        let original = f.resources[0].clone();

        let error = upsert_resource(
            &mut f,
            ResourceUpsert {
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                metadata: json!({"max_fps":30}),
                ..spec(ResourceType::Camera, "Camera-USB-12345", "Renamed Camera")
            },
        )
        .expect_err("retired local-device subject must fail closed");
        assert!(
            error.to_string().contains("retired subject"),
            "unexpected error: {error}"
        );
        assert_eq!(f.resources.len(), 1);
        assert_eq!(f.resources[0], original);
    }

    #[test]
    fn upsert_returns_stable_ura_across_calls_for_same_hardware_id() {
        // INV-RESOURCE-ULID-STABLE: re-scanning the same hardware
        // MUST surface the same resource_ura. Without this, every
        // reboot orphans every prior receipt referencing that
        // resource.
        let mut f = empty();
        let ura1 = upsert_resource(
            &mut f,
            spec(ResourceType::Camera, "Camera-USB-12345", "Logitech C920"),
        )
        .expect("first upsert");
        let ura2 = upsert_resource(
            &mut f,
            ResourceUpsert {
                metadata: json!({"max_fps":30}),
                ..spec(
                    ResourceType::Camera,
                    "Camera-USB-12345",
                    "Logitech C920 (Renamed)", // display_name changed
                )
            },
        )
        .expect("second upsert");
        assert_eq!(ura1, ura2, "same hardware_id MUST yield same URA");
        assert_eq!(f.resources.len(), 1, "no duplicate entry");
        // display_name + metadata mutated; resource_ura stable.
        assert_eq!(f.resources[0].display_name, "Logitech C920 (Renamed)");
        assert_eq!(f.resources[0].metadata["max_fps"], 30);
    }

    #[test]
    fn indexed_batch_upsert_preserves_single_upsert_semantics() {
        let mut f = empty();
        let existing_ura = upsert_resource(
            &mut f,
            spec(ResourceType::Window, "window:1", "Original Window"),
        )
        .expect("seed existing window");

        let uras = upsert_resources_indexed(
            &mut f,
            vec![
                ResourceUpsert {
                    metadata: json!({"window_id": 1, "title": "Renamed"}),
                    ..spec(ResourceType::Window, "window:1", "Renamed Window")
                },
                ResourceUpsert {
                    metadata: json!({"window_id": 2, "title": "New"}),
                    ..spec(ResourceType::Window, "window:2", "New Window")
                },
            ],
        )
        .expect("indexed batch upsert");

        assert_eq!(uras.len(), 2);
        assert_eq!(uras[0], existing_ura);
        assert_ne!(uras[1], existing_ura);
        assert_eq!(f.resources.len(), 2);
        let existing = lookup_by_hardware_id(&f, "window:1").expect("existing");
        assert_eq!(existing.resource_ura, existing_ura);
        assert_eq!(existing.display_name, "Renamed Window");
        assert_eq!(existing.metadata["title"], json!("Renamed"));
        let inserted = lookup_by_hardware_id(&f, "window:2").expect("inserted");
        assert_eq!(inserted.display_name, "New Window");
        assert_eq!(inserted.metadata["title"], json!("New"));
    }

    #[test]
    fn upsert_distinguishes_distinct_hardware_ids() {
        let mut f = empty();
        let ura_front = upsert_resource(
            &mut f,
            spec(ResourceType::Camera, "Camera-Front", "Front Camera"),
        )
        .expect("front camera");
        let ura_rear = upsert_resource(
            &mut f,
            spec(ResourceType::Camera, "Camera-Rear", "Rear Camera"),
        )
        .expect("rear camera");
        assert_ne!(ura_front, ura_rear);
        assert_eq!(f.resources.len(), 2);
    }

    #[test]
    fn lookup_by_hardware_id_finds_existing_entry() {
        let mut f = empty();
        upsert_resource(
            &mut f,
            spec(ResourceType::Speaker, "Speaker-AAPL-1", "Built-in Speaker"),
        )
        .expect("seed speaker");
        let entry = lookup_by_hardware_id(&f, "Speaker-AAPL-1").expect("must find");
        assert_eq!(entry.kind, ResourceType::Speaker);
        assert!(lookup_by_hardware_id(&f, "Speaker-AAPL-2").is_none());
    }

    #[test]
    fn lookup_by_ura_finds_existing_entry() {
        let mut f = empty();
        let ura =
            upsert_resource(&mut f, spec(ResourceType::Mic, "h1", "Mic 1")).expect("seed mic");
        let entry = lookup_by_ura(&f, &ura).expect("must find by ura");
        assert_eq!(entry.hardware_id, "h1");
        assert!(lookup_by_ura(&f, "easynet:///r/acme/resource/missing").is_none());
    }

    #[test]
    fn filter_by_kinds_empty_returns_all() {
        let mut f = empty();
        upsert_resource(&mut f, spec(ResourceType::Mic, "h1", "")).expect("seed mic");
        upsert_resource(&mut f, spec(ResourceType::Camera, "h2", "")).expect("seed camera");
        let all = filter_by_kinds(&f, &[]);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn filter_by_kinds_filters_to_named_kinds() {
        let mut f = empty();
        upsert_resource(&mut f, spec(ResourceType::Mic, "h1", "")).expect("seed mic");
        upsert_resource(&mut f, spec(ResourceType::Camera, "h2", "")).expect("seed camera");
        upsert_resource(&mut f, spec(ResourceType::Speaker, "h3", "")).expect("seed speaker");
        let mics = filter_by_kinds(&f, &[ResourceType::Mic]);
        assert_eq!(mics.len(), 1);
        assert_eq!(mics[0].kind, ResourceType::Mic);
        let mics_and_speakers = filter_by_kinds(&f, &[ResourceType::Mic, ResourceType::Speaker]);
        assert_eq!(mics_and_speakers.len(), 2);
    }

    #[test]
    fn local_device_upsert_rejects_missing_device_owner() {
        let mut f = empty();
        let error = upsert_resource(
            &mut f,
            ResourceUpsert {
                owner_agent: "",
                ..spec(ResourceType::Mic, "h1", "Mic")
            },
        )
        .expect_err("local-device resources require a device owner");
        assert!(
            error
                .to_string()
                .contains("device-sponsored SystemAgent URA"),
            "unexpected error: {error}"
        );
        assert!(f.resources.is_empty());
    }

    #[test]
    fn local_device_upsert_rejects_device_ura_as_owner_agent() {
        let mut f = empty();
        let error = upsert_resource(
            &mut f,
            ResourceUpsert {
                owner_agent: "easynet:///r/acme/device/01DEV",
                ..spec(ResourceType::Mic, "h-device-owner", "Mic")
            },
        )
        .expect_err("Device is host substrate, not the owner Agent");

        assert!(
            error
                .to_string()
                .contains("device-sponsored SystemAgent URA"),
            "unexpected error: {error}"
        );
        assert!(f.resources.is_empty());
    }

    #[test]
    fn round_trip_through_json_preserves_fields() {
        // Critical wire-shape pin: the on-disk JSON uses
        // `"type": "camera"` (snake_case) — a future PR that
        // accidentally renames the serde field, drops
        // `rename = "type"`, or changes the variant casing
        // breaks the file format and silently strands every
        // operator's resources.json.
        let mut f = empty();
        upsert_resource(
            &mut f,
            ResourceUpsert {
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                metadata: json!({"max_fps":60,"resolutions":["640x480","1280x720"]}),
                ..spec(ResourceType::Camera, "h-cam-1", "Webcam")
            },
        )
        .expect("seed camera");
        let json_str = serde_json::to_string(&f).unwrap();
        assert!(
            json_str.contains("\"type\":\"camera\""),
            "wire form must use lowercase `type` key + snake_case variant; got {json_str}"
        );
        assert!(
            json_str.contains("\"binding\":\"local_device\""),
            "binding wire form must be snake_case; got {json_str}"
        );
        let parsed: ResourcesFile = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.resources.len(), 1);
        assert_eq!(parsed.resources[0].kind, ResourceType::Camera);
        assert_eq!(parsed.resources[0].binding, ResourceBinding::LocalDevice);
        assert_eq!(parsed.resources[0].metadata["max_fps"], 60);
    }

    #[test]
    fn existing_resources_file_requires_resources_field() {
        let error = serde_json::from_str::<ResourcesFile>(r#"{}"#)
            .expect_err("existing resources.json must declare resources");
        assert!(
            error.to_string().contains("missing field `resources`"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn existing_resource_entry_requires_owner_agent_display_name_and_metadata() {
        let base = serde_json::json!({
            "resource_ura": "easynet:///r/acme/resource/device.01DEV/streams/mic.01RES",
            "owner_agent": "easynet:///r/acme/agent/device.01DEV.media",
            "type": "mic",
            "binding": "local_device",
            "hardware_id": "BuiltInMic-AAPL-0001",
            "display_name": "Built-in Microphone",
            "metadata": {},
            "first_seen_at": "2026-07-23T00:00:00Z"
        });

        for (field, expected) in [
            ("owner_agent", "missing field `owner_agent`"),
            ("display_name", "missing field `display_name`"),
            ("metadata", "missing field `metadata`"),
        ] {
            let mut entry = base.clone();
            entry
                .as_object_mut()
                .expect("entry object")
                .remove(field)
                .expect("field exists in base entry");
            let file = serde_json::json!({ "resources": [entry] });
            let error = serde_json::from_value::<ResourcesFile>(file)
                .expect_err(&format!("missing {field} must fail"));
            assert!(
                error.to_string().contains(expected),
                "unexpected error for missing {field}: {error}"
            );
        }
    }

    #[test]
    fn existing_resources_file_rejects_unknown_fields() {
        let top_level_error =
            serde_json::from_str::<ResourcesFile>(r#"{"resources":[],"legacy_owner":""}"#)
                .expect_err("unknown top-level fields must fail");
        assert!(
            top_level_error
                .to_string()
                .contains("unknown field `legacy_owner`"),
            "unexpected top-level error: {top_level_error}"
        );

        let row_error = serde_json::from_value::<ResourcesFile>(serde_json::json!({
            "resources": [{
                "resource_ura": "easynet:///r/acme/resource/device.01DEV/streams/mic.01RES",
                "owner_agent": "easynet:///r/acme/agent/device.01DEV.media",
                "type": "mic",
                "binding": "local_device",
                "hardware_id": "BuiltInMic-AAPL-0001",
                "display_name": "Built-in Microphone",
                "metadata": {},
                "first_seen_at": "2026-07-23T00:00:00Z",
                "legacy_subject": "device"
            }]
        }))
        .expect_err("unknown resource-row fields must fail");
        assert!(
            row_error
                .to_string()
                .contains("unknown field `legacy_subject`"),
            "unexpected row error: {row_error}"
        );
    }

    #[test]
    fn resource_type_from_str_round_trips_every_variant() {
        // Single-source-of-truth pin for ResourceType: every variant's
        // wire string parses back to the same variant. Catches a
        // future PR that adds a variant to the enum but forgets to
        // extend `as_str` (the FromStr impl uses ALL + as_str so the
        // round trip exposes the missed branch as a panic in test).
        for &t in ResourceType::ALL {
            let parsed: ResourceType = t.as_str().parse().expect("known variant must parse");
            assert_eq!(parsed, t);
        }
        // Unknown reject path.
        assert!("totally-not-a-type".parse::<ResourceType>().is_err());
    }
}
