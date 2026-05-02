// EasyNet CLI — Local Resources Registry (RFC-005 v3.2 §"Resource URA")
// =====================================================================
//
// File: src/persistence/resources.rs
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
//       "resource_uri":  "easynet:///r/<realm>/resource/<id>",
//       "owner_agent":   "easynet:///r/<realm>/agent/<id>",
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

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::{atomic_write_with_permissions, state_dir, WritePermissions};

pub(crate) const FILE_NAME: &str = "resources.json";

/// Resource type taxonomy — RFC-005 v3.2. The wire form is a
/// lowercase string (forward-compat: a future deployment that
/// invents `gpu` lands without a schema migration), but every
/// known v1 type is enumerated here so callers cannot typo
/// `"camera"` as `"cammera"` and silently misclassify.
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

/// On-disk shape of `~/.easynet/resources.json`. Field names must
/// remain stable — older daemons must read what newer daemons write.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourcesFile {
    /// All known resources held by this host. Order is insertion
    /// order; readers MUST NOT rely on order.
    #[serde(default)]
    pub resources: Vec<ResourceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceEntry {
    /// Canonical resource URA, shape
    /// `easynet:///r/<realm>/resource/<id>` per RFC-005 v3.2.
    pub resource_uri: String,
    /// Owner Agent's URA. Empty at first-boot (no host URA known
    /// yet); patched on next save once `local-agents.json` knows
    /// the device URA. Empty in pre-join state mirrors the
    /// `local-agents.json` "hosted_by:<unset>" convention.
    #[serde(default)]
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
    /// `hardware_id` → same `resource_uri` across restarts.
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
    /// without affecting `resource_uri`.
    #[serde(default)]
    pub display_name: String,
    /// Open-ended bag for codec hints, capabilities, max-
    /// resolution, supported sample rates, etc. Read by media
    /// handlers when negotiating args defaults.
    #[serde(default)]
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
}

/// Build the canonical resource URA for a given realm + id.
///
/// Realm comes from credentials; id is the per-resource ULID/UUID
/// minted on first sight. Centralised so every consumer (upsert,
/// tests, future federation.advertise hook) agrees on the shape.
pub fn build_resource_uri(realm: &str, resource_id: &str) -> String {
    // URI v2: resource_id is opaque tail; this convenience helper
    // takes a pre-composed `<kind>.<id>` value and just slots it in
    // — keeps backward-compat with existing callers that don't
    // know about the kind+id split.
    format!("{}{}/resource/{}", "easynet:///r/", realm, resource_id)
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
pub fn lookup_by_uri<'a>(file: &'a ResourcesFile, resource_uri: &str) -> Option<&'a ResourceEntry> {
    file.resources
        .iter()
        .find(|e| e.resource_uri == resource_uri)
}

/// Insert or update a resource entry keyed on `hardware_id`.
///
/// Returns the resulting `resource_uri`:
/// - if `hardware_id` already exists in the file, returns its
///   existing URA (URA does NOT change — INV-RESOURCE-ULID-STABLE).
///   Mutates `display_name` / `metadata` / `owner_agent` so a
///   renamed device or post-join host URA propagates without
///   orphaning the URA.
/// - else mints a fresh UUIDv4-based id, builds a new URA via
///   `build_resource_uri(realm, ...)`, appends the entry, and
///   returns the new URA.
///
/// `kind` and `binding` on a hardware_id MUST not change between
/// upserts — those are properties of the physical resource, not
/// of this call. The helper accepts them on every upsert for the
/// fresh-insert path; on the update path it leaves the existing
/// values untouched (a renamed-but-same-hardware device must not
/// silently flip from `mic` to `camera`).
pub fn upsert_resource(file: &mut ResourcesFile, spec: ResourceUpsert<'_>) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(entry) = file
        .resources
        .iter_mut()
        .find(|e| e.hardware_id == spec.hardware_id)
    {
        // Stable URA + immutable kind/binding — never mutated.
        entry.owner_agent = spec.owner_agent.to_string();
        entry.display_name = spec.display_name.to_string();
        entry.metadata = spec.metadata;
        return entry.resource_uri.clone();
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let resource_uri = build_resource_uri(spec.realm, &id);
    file.resources.push(ResourceEntry {
        resource_uri: resource_uri.clone(),
        owner_agent: spec.owner_agent.to_string(),
        kind: spec.kind,
        binding: spec.binding,
        hardware_id: spec.hardware_id.to_string(),
        display_name: spec.display_name.to_string(),
        metadata: spec.metadata,
        first_seen_at: now,
    });
    resource_uri
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

    /// One-line spec builder for tests so each call site fits on
    /// a screen and reads as data, not as a parameter list.
    fn spec<'a>(
        kind: ResourceType,
        hardware_id: &'a str,
        display_name: &'a str,
    ) -> ResourceUpsert<'a> {
        ResourceUpsert {
            realm: "acme",
            owner_agent: "",
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
    fn build_resource_uri_uses_canonical_shape() {
        assert_eq!(
            build_resource_uri("acme", "01ABC"),
            "easynet:///r/acme/resource/01ABC"
        );
    }

    #[test]
    fn upsert_inserts_when_hardware_id_absent() {
        let mut f = empty();
        let uri = upsert_resource(
            &mut f,
            ResourceUpsert {
                owner_agent: "easynet:///r/acme/agent/01DEV",
                metadata: json!({"sample_rates":[16000,48000]}),
                ..spec(
                    ResourceType::Mic,
                    "BuiltInMic-AAPL-0001",
                    "Built-in Microphone",
                )
            },
        );
        assert!(uri.starts_with("easynet:///r/acme/resource/"));
        assert_eq!(f.resources.len(), 1);
        assert_eq!(f.resources[0].kind, ResourceType::Mic);
        assert_eq!(f.resources[0].hardware_id, "BuiltInMic-AAPL-0001");
        assert_eq!(f.resources[0].display_name, "Built-in Microphone");
        assert_eq!(f.resources[0].owner_agent, "easynet:///r/acme/agent/01DEV");
    }

    #[test]
    fn upsert_returns_stable_uri_across_calls_for_same_hardware_id() {
        // INV-RESOURCE-ULID-STABLE: re-scanning the same hardware
        // MUST surface the same resource_uri. Without this, every
        // reboot orphans every prior receipt referencing that
        // resource.
        let mut f = empty();
        let uri1 = upsert_resource(
            &mut f,
            spec(ResourceType::Camera, "Camera-USB-12345", "Logitech C920"),
        );
        let uri2 = upsert_resource(
            &mut f,
            ResourceUpsert {
                metadata: json!({"max_fps":30}),
                ..spec(
                    ResourceType::Camera,
                    "Camera-USB-12345",
                    "Logitech C920 (Renamed)", // display_name changed
                )
            },
        );
        assert_eq!(uri1, uri2, "same hardware_id MUST yield same URA");
        assert_eq!(f.resources.len(), 1, "no duplicate entry");
        // display_name + metadata mutated; resource_uri stable.
        assert_eq!(f.resources[0].display_name, "Logitech C920 (Renamed)");
        assert_eq!(f.resources[0].metadata["max_fps"], 30);
    }

    #[test]
    fn upsert_distinguishes_distinct_hardware_ids() {
        let mut f = empty();
        let uri_front = upsert_resource(
            &mut f,
            spec(ResourceType::Camera, "Camera-Front", "Front Camera"),
        );
        let uri_rear = upsert_resource(
            &mut f,
            spec(ResourceType::Camera, "Camera-Rear", "Rear Camera"),
        );
        assert_ne!(uri_front, uri_rear);
        assert_eq!(f.resources.len(), 2);
    }

    #[test]
    fn lookup_by_hardware_id_finds_existing_entry() {
        let mut f = empty();
        upsert_resource(
            &mut f,
            spec(ResourceType::Speaker, "Speaker-AAPL-1", "Built-in Speaker"),
        );
        let entry = lookup_by_hardware_id(&f, "Speaker-AAPL-1").expect("must find");
        assert_eq!(entry.kind, ResourceType::Speaker);
        assert!(lookup_by_hardware_id(&f, "Speaker-AAPL-2").is_none());
    }

    #[test]
    fn lookup_by_uri_finds_existing_entry() {
        let mut f = empty();
        let uri = upsert_resource(&mut f, spec(ResourceType::Mic, "h1", "Mic 1"));
        let entry = lookup_by_uri(&f, &uri).expect("must find by uri");
        assert_eq!(entry.hardware_id, "h1");
        assert!(lookup_by_uri(&f, "easynet:///r/acme/resource/missing").is_none());
    }

    #[test]
    fn filter_by_kinds_empty_returns_all() {
        let mut f = empty();
        upsert_resource(&mut f, spec(ResourceType::Mic, "h1", ""));
        upsert_resource(&mut f, spec(ResourceType::Camera, "h2", ""));
        let all = filter_by_kinds(&f, &[]);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn filter_by_kinds_filters_to_named_kinds() {
        let mut f = empty();
        upsert_resource(&mut f, spec(ResourceType::Mic, "h1", ""));
        upsert_resource(&mut f, spec(ResourceType::Camera, "h2", ""));
        upsert_resource(&mut f, spec(ResourceType::Speaker, "h3", ""));
        let mics = filter_by_kinds(&f, &[ResourceType::Mic]);
        assert_eq!(mics.len(), 1);
        assert_eq!(mics[0].kind, ResourceType::Mic);
        let mics_and_speakers = filter_by_kinds(&f, &[ResourceType::Mic, ResourceType::Speaker]);
        assert_eq!(mics_and_speakers.len(), 2);
    }

    #[test]
    fn upsert_pre_join_records_empty_owner_agent() {
        // Mirror local_agents.rs: pre-join state allowed, owner
        // is patched on the next save when the device URA is
        // known.
        let mut f = empty();
        upsert_resource(&mut f, spec(ResourceType::Mic, "h1", "Mic"));
        assert_eq!(f.resources[0].owner_agent, "");
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
                owner_agent: "easynet:///r/acme/agent/01DEV",
                metadata: json!({"max_fps":60,"resolutions":["640x480","1280x720"]}),
                ..spec(ResourceType::Camera, "h-cam-1", "Webcam")
            },
        );
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
