// EasyNet CLI — daemon Invocation descriptor contracts
// ====================================================
//
// File: src/daemon/ability/catalog/daemon_invocation_contracts.rs
// Description: Descriptor/control-plane projection for daemon Invocation
//              exact routes that are served outside the local ability handler
//              index.
//
// Protocol Responsibility:
// - Give every Authority-owned daemon Invocation route one governed descriptor contract.
// - Keep descriptor proof facts in the ability control plane instead of the
//   transport adapter.
//
// Implementation Approach:
// - Project `HubBaseline` rows whose surface is `DaemonInvocation`.
// - Register control-plane-only records: descriptor, authority, and native
//   implementation facts, but no local registry handler.
//
// Usage Contract:
// - `DaemonRouteRuntimeAdapter` remains the only execution installer for exact
//   unary routes.
// - This module must not parse, route, or execute Invoke requests.
//
// Architectural Position:
// - Ability catalog layer. Invocation dispatch remains under
//   `daemon::invocation::dispatch`.

use serde_json::{json, Value};

use crate::daemon::ability::conformance::{
    BaselineSurface, HubBaseline, ABILITY_FEDERATION_ADVERTISE_ABILITIES,
    ABILITY_FEDERATION_ADVERTISE_AGENT, ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_HEARTBEAT,
    ABILITY_FEDERATION_JOIN, ABILITY_FEDERATION_LIST_USER_DEVICES,
    ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES, ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE, ABILITY_FEDERATION_STATUS,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2, ABILITY_IDENTITY_LIST_USER_PUBKEYS,
    ABILITY_IDENTITY_REGISTER_PUBKEY, ABILITY_IDENTITY_REVOKE_USER_PUBKEY,
    ABILITY_NAMESPACE_PROXY_RESOLVE, ABILITY_NAMESPACE_RESOLVE, ABILITY_PRINCIPAL_ADD_KEY,
    ABILITY_PRINCIPAL_BIND_FIRST_KEY, ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
    ABILITY_PRINCIPAL_CREATE, ABILITY_PRINCIPAL_DELETE, ABILITY_PRINCIPAL_GET,
    ABILITY_PRINCIPAL_ISSUE_ENROLLMENT, ABILITY_PRINCIPAL_ISSUE_GRANT,
    ABILITY_PRINCIPAL_REACTIVATE, ABILITY_PRINCIPAL_RECOVER, ABILITY_PRINCIPAL_REVOKE_ENROLLMENT,
    ABILITY_PRINCIPAL_REVOKE_GRANT, ABILITY_PRINCIPAL_REVOKE_KEY, ABILITY_PRINCIPAL_ROTATE_KEY,
    ABILITY_PRINCIPAL_SUSPEND,
};
use crate::daemon::ability::descriptors::{AdmissionAction, ReceiptSemantics};
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, ControlPlaneImplementation, OwnerKind};
use crate::daemon::ability::manifest::AbilityManifest;
use crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonInvocationContractLayer {
    Introspection,
    Control,
    Operational,
}

pub(crate) fn register_for_owner(
    reg: &mut AxonAbilityCatalog,
    owner: &OwnerKind,
) -> anyhow::Result<()> {
    let implementation = ControlPlaneImplementation::native_daemon();
    for ability in HubBaseline::required_abilities()
        .iter()
        .copied()
        .filter(|ability| ability.surface == BaselineSurface::DaemonInvocation)
    {
        let action = admission_action_for(ability.name).ok_or_else(|| {
            anyhow::anyhow!(
                "Authority-owned daemon Invocation baseline ability {:?} has no descriptor contract action",
                ability.name
            )
        })?;
        let manifest = manifest_for(ability.name, action)?;
        reg.register_control_plane_descriptor_with_owner(
            ability.name,
            owner,
            &manifest,
            ability.call_mode,
            ReceiptSemantics::Operational,
            &implementation,
        )?;
    }
    Ok(())
}

pub(crate) fn contract_layer(name: &str) -> Option<DaemonInvocationContractLayer> {
    use DaemonInvocationContractLayer::{Control, Introspection, Operational};

    Some(match name {
        ABILITY_FEDERATION_RESOLVE
        | ABILITY_NAMESPACE_RESOLVE
        | ABILITY_NAMESPACE_PROXY_RESOLVE
        | ABILITY_FEDERATION_RESOLVE_KEY
        | ABILITY_FEDERATION_DISCOVER
        | ABILITY_FEDERATION_LIST_USER_DEVICES
        | ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES
        | ABILITY_FEDERATION_STATUS
        | ABILITY_IDENTITY_LIST_USER_PUBKEYS
        | ABILITY_PRINCIPAL_GET => Introspection,

        ABILITY_FEDERATION_HEARTBEAT | ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2 => Control,

        ABILITY_FEDERATION_JOIN
        | ABILITY_FEDERATION_ADVERTISE_AGENT
        | ABILITY_FEDERATION_ADVERTISE_ABILITIES
        | ABILITY_FEDERATION_REVOKE
        | ABILITY_IDENTITY_REGISTER_PUBKEY
        | ABILITY_IDENTITY_REVOKE_USER_PUBKEY
        | ABILITY_PRINCIPAL_CREATE
        | ABILITY_PRINCIPAL_BIND_FIRST_KEY
        | ABILITY_PRINCIPAL_ADD_KEY
        | ABILITY_PRINCIPAL_ROTATE_KEY
        | ABILITY_PRINCIPAL_REVOKE_KEY
        | ABILITY_PRINCIPAL_CONFIGURE_RECOVERY
        | ABILITY_PRINCIPAL_RECOVER
        | ABILITY_PRINCIPAL_SUSPEND
        | ABILITY_PRINCIPAL_REACTIVATE
        | ABILITY_PRINCIPAL_DELETE
        | ABILITY_PRINCIPAL_ISSUE_ENROLLMENT
        | ABILITY_PRINCIPAL_REVOKE_ENROLLMENT
        | ABILITY_PRINCIPAL_ISSUE_GRANT
        | ABILITY_PRINCIPAL_REVOKE_GRANT => Operational,

        _ => return None,
    })
}

pub(crate) fn admission_action_for(name: &str) -> Option<AdmissionAction> {
    Some(match name {
        ABILITY_FEDERATION_RESOLVE
        | ABILITY_NAMESPACE_RESOLVE
        | ABILITY_NAMESPACE_PROXY_RESOLVE
        | ABILITY_FEDERATION_RESOLVE_KEY
        | ABILITY_FEDERATION_DISCOVER
        | ABILITY_FEDERATION_LIST_USER_DEVICES
        | ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES
        | ABILITY_FEDERATION_STATUS
        | ABILITY_IDENTITY_LIST_USER_PUBKEYS
        | ABILITY_PRINCIPAL_GET => AdmissionAction::Read,

        ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2 => AdmissionAction::Stream,

        ABILITY_PRINCIPAL_CREATE
        | ABILITY_PRINCIPAL_BIND_FIRST_KEY
        | ABILITY_PRINCIPAL_ADD_KEY
        | ABILITY_PRINCIPAL_ROTATE_KEY
        | ABILITY_PRINCIPAL_CONFIGURE_RECOVERY
        | ABILITY_PRINCIPAL_RECOVER
        | ABILITY_PRINCIPAL_SUSPEND
        | ABILITY_PRINCIPAL_REACTIVATE
        | ABILITY_PRINCIPAL_ISSUE_ENROLLMENT
        | ABILITY_PRINCIPAL_ISSUE_GRANT => AdmissionAction::Invoke,

        ABILITY_FEDERATION_JOIN
        | ABILITY_FEDERATION_ADVERTISE_AGENT
        | ABILITY_FEDERATION_ADVERTISE_ABILITIES
        | ABILITY_FEDERATION_HEARTBEAT
        | ABILITY_FEDERATION_REVOKE
        | ABILITY_IDENTITY_REGISTER_PUBKEY
        | ABILITY_IDENTITY_REVOKE_USER_PUBKEY
        | ABILITY_PRINCIPAL_REVOKE_KEY
        | ABILITY_PRINCIPAL_DELETE
        | ABILITY_PRINCIPAL_REVOKE_ENROLLMENT
        | ABILITY_PRINCIPAL_REVOKE_GRANT => AdmissionAction::Manage,

        _ => return None,
    })
}

pub(crate) fn description_for(name: &str) -> Option<&'static str> {
    Some(match name {
        ABILITY_FEDERATION_JOIN => {
            "Admit a device into the realm federation and return its membership receipt."
        }
        ABILITY_FEDERATION_ADVERTISE_AGENT => {
            "Publish one hosted Agent directory row under the calling device's federation presence."
        }
        ABILITY_FEDERATION_ADVERTISE_ABILITIES => {
            "Publish governed ability descriptors for an already-advertised Agent projection."
        }
        ABILITY_FEDERATION_HEARTBEAT => {
            "Refresh federation directory leases and return the current advertised registry size."
        }
        ABILITY_FEDERATION_RESOLVE => {
            "Resolve live realm agents, devices, and advertised abilities from the federation directory."
        }
        ABILITY_NAMESPACE_RESOLVE => {
            "Resolve an owner or ability URA into a typed Axon route answer."
        }
        ABILITY_NAMESPACE_PROXY_RESOLVE => {
            "Resolve a namespace query across selected peer hubs through daemon-owned federation dialing."
        }
        ABILITY_FEDERATION_RESOLVE_KEY => {
            "Resolve the Ed25519 verification key for a trusted federation Agent URA."
        }
        ABILITY_FEDERATION_DISCOVER => {
            "Read federated directory entries visible to this hub."
        }
        ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2 => {
            "Subscribe to typed federation directory events."
        }
        ABILITY_FEDERATION_LIST_USER_DEVICES => {
            "List live devices for a user within this hub's realm directory."
        }
        ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES => {
            "Fan out user-device listing to selected peer hubs and merge their typed responses."
        }
        ABILITY_FEDERATION_REVOKE => {
            "Revoke a device or hosted Agent from federation presence and directory projections."
        }
        ABILITY_FEDERATION_STATUS => {
            "Return the daemon's canonical join/session state projection."
        }
        ABILITY_IDENTITY_REGISTER_PUBKEY => {
            "Register a trusted public key row in the daemon runtime trust anchor."
        }
        ABILITY_IDENTITY_LIST_USER_PUBKEYS => {
            "List trusted public keys currently bound to a user URA."
        }
        ABILITY_IDENTITY_REVOKE_USER_PUBKEY => {
            "Revoke one trusted public key row for a user URA."
        }
        ABILITY_PRINCIPAL_CREATE => "Create a daemon runtime principal lifecycle record.",
        ABILITY_PRINCIPAL_BIND_FIRST_KEY => {
            "Bind the first active key to a pending daemon runtime principal."
        }
        ABILITY_PRINCIPAL_ADD_KEY => "Add an active key binding to a daemon runtime principal.",
        ABILITY_PRINCIPAL_ROTATE_KEY => "Rotate one key binding on a daemon runtime principal.",
        ABILITY_PRINCIPAL_REVOKE_KEY => "Revoke one key binding from a daemon runtime principal.",
        ABILITY_PRINCIPAL_CONFIGURE_RECOVERY => {
            "Configure recovery policy for a daemon runtime principal."
        }
        ABILITY_PRINCIPAL_RECOVER => "Recover a daemon runtime principal through recovery policy.",
        ABILITY_PRINCIPAL_SUSPEND => "Suspend a daemon runtime principal.",
        ABILITY_PRINCIPAL_REACTIVATE => "Reactivate a suspended daemon runtime principal.",
        ABILITY_PRINCIPAL_DELETE => "Delete a daemon runtime principal.",
        ABILITY_PRINCIPAL_ISSUE_ENROLLMENT => {
            "Issue an enrollment capability for a daemon runtime principal."
        }
        ABILITY_PRINCIPAL_REVOKE_ENROLLMENT => {
            "Revoke an enrollment capability for a daemon runtime principal."
        }
        ABILITY_PRINCIPAL_ISSUE_GRANT => {
            "Issue an authorization grant from a daemon runtime principal."
        }
        ABILITY_PRINCIPAL_REVOKE_GRANT => {
            "Revoke an authorization grant from a daemon runtime principal."
        }
        ABILITY_PRINCIPAL_GET => "Read one daemon runtime principal lifecycle record.",
        _ => return None,
    })
}

pub(crate) fn input_schema_for(name: &str) -> Option<Value> {
    Some(match name {
        ABILITY_FEDERATION_JOIN => object_schema(
            json!({
                "membership_ura": string_prop("Device URA to admit into the realm."),
                "realm": string_prop("Realm the joining device claims."),
                "public_key_hex": string_prop("Hex-encoded 32-byte Ed25519 verifying key."),
                "principal_enrollment": {
                    "type": "object",
                    "description": "Optional product-neutral principal enrollment proof."
                }
            }),
            &["membership_ura", "realm", "public_key_hex"],
            false,
        ),
        ABILITY_FEDERATION_ADVERTISE_AGENT => object_schema(
            json!({
                "agent_ura": string_prop("Hosted Agent URA being advertised."),
                "host_node_id": string_prop("Host device or node URA that carries the Agent."),
                "public_key_hex": string_prop("Optional advertised Agent verifying key."),
                "generation": integer_prop("Monotonic directory generation for this advertisement.")
            }),
            &["agent_ura", "generation"],
            true,
        ),
        ABILITY_FEDERATION_ADVERTISE_ABILITIES => object_schema(
            json!({
                "owner_ura": string_prop("Agent or Authority URA that owns the advertised descriptors."),
                "generation": integer_prop("Monotonic generation for the descriptor projection."),
                "abilities": {
                    "type": "array",
                    "description": "AbilityDescriptor rows to publish.",
                    "items": { "type": "object" }
                }
            }),
            &["owner_ura", "abilities"],
            true,
        ),
        ABILITY_FEDERATION_HEARTBEAT => object_schema(
            json!({
                "since_abilities_revision": integer_prop("Device's last observed realm Authority-published ability revision."),
                "refresh_owner_uras": {
                    "type": "array",
                    "description": "Owner URAs whose published ability projection leases should be renewed.",
                    "items": { "type": "string" }
                }
            }),
            &["since_abilities_revision", "refresh_owner_uras"],
            false,
        ),
        ABILITY_FEDERATION_RESOLVE => object_schema(
            json!({
                "agent_ura": string_prop("Optional exact Agent or Device URA to resolve."),
                "ability": string_prop("Optional owner-local ability name to resolve."),
                "prefix": string_prop("Optional directory prefix filter."),
                "realm": string_prop("Optional realm filter.")
            }),
            &[],
            false,
        ),
        ABILITY_NAMESPACE_RESOLVE => object_schema(
            json!({
                "target_ura": string_prop("Owner, ability, or subject URA to resolve."),
                "ability": string_prop("Optional owner-local ability name."),
                "caller_ura": string_prop("Caller URA used for policy-aware resolution.")
            }),
            &[],
            false,
        ),
        ABILITY_NAMESPACE_PROXY_RESOLVE => object_schema(
            json!({
                "peer_hub_urls": {
                    "type": "array",
                    "description": "Optional peer hubs selected for proxy resolution.",
                    "items": { "type": "string" }
                },
                "query_name": string_prop("Canonical namespace query name forwarded to namespace.resolve."),
                "qtype": string_prop("Canonical ResolveType enum string."),
                "caller_ura": string_prop("Caller URA used for policy-aware resolution."),
                "subject_ura": string_prop("Subject URA used for policy-aware resolution."),
                "realm_hint": string_prop("Realm context used by peer namespace resolution."),
                "ability_name": nullable_string_prop("Explicit owner-local ability selector, or null for directory/listing queries with no separate ability selector.")
            }),
            &[
                "query_name",
                "qtype",
                "caller_ura",
                "subject_ura",
                "realm_hint",
                "ability_name",
            ],
            false,
        ),
        ABILITY_FEDERATION_RESOLVE_KEY => object_schema(
            json!({
                "agent_ura": string_prop("Agent, Device, User, or Authority URA whose verifying key is requested."),
                "presented_pubkey_b64": string_prop("Optional base64 Ed25519 public key observed on the caller envelope; used to pin multi-key principal lookup.")
            }),
            &["agent_ura"],
            false,
        ),
        ABILITY_FEDERATION_DISCOVER => object_schema(
            json!({
                "agent_ura": string_prop("Optional exact Agent or Device URA filter."),
                "origin_realm": string_prop("Optional origin realm filter.")
            }),
            &[],
            false,
        ),
        ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2 => object_schema(
            json!({
                "agent_ura": string_prop("Optional Agent or Device URA filter."),
                "since_generation": integer_prop("Optional generation cursor.")
            }),
            &[],
            false,
        ),
        ABILITY_FEDERATION_LIST_USER_DEVICES => object_schema(
            json!({
                "realm": string_prop("Realm whose live devices should be listed by the peer hub.")
            }),
            &["realm"],
            false,
        ),
        ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES => object_schema(
            json!({
                "realm": string_prop("Realm whose live devices should be listed across selected peer hubs."),
                "peer_hub_urls": {
                    "type": "array",
                    "description": "Selected trusted peer hub endpoints for daemon-owned fanout.",
                    "items": { "type": "string" }
                }
            }),
            &["realm"],
            false,
        ),
        ABILITY_FEDERATION_REVOKE => object_schema(
            json!({
                "agent_ura": string_prop("Agent or Device URA to revoke."),
                "reason": string_prop("Operator-readable revocation reason."),
                "generation": integer_prop("Expected generation for idempotent revocation.")
            }),
            &["agent_ura"],
            false,
        ),
        ABILITY_FEDERATION_STATUS => closed_empty_schema(),
        ABILITY_IDENTITY_REGISTER_PUBKEY => object_schema(
            json!({
                "principal_ura": string_prop("Principal URA whose verifying key should be trusted."),
                "public_key_b64": string_prop("Base64-encoded Ed25519 verifying key."),
                "role": string_prop("Trust role for this key row."),
                "principal_owner_ura": string_prop("Optional canonical owner User URA.")
            }),
            &["principal_ura", "public_key_b64", "role"],
            false,
        ),
        ABILITY_IDENTITY_LIST_USER_PUBKEYS => object_schema(
            json!({
                "user_ura": string_prop("User URA whose trusted keys should be listed.")
            }),
            &["user_ura"],
            false,
        ),
        ABILITY_IDENTITY_REVOKE_USER_PUBKEY => object_schema(
            json!({
                "user_ura": string_prop("User URA whose key row should be revoked."),
                "public_key_b64": string_prop("Base64-encoded Ed25519 verifying key to revoke.")
            }),
            &["user_ura", "public_key_b64"],
            false,
        ),
        ABILITY_PRINCIPAL_GET => object_schema(
            json!({
                "principal_ura": string_prop("Principal URA to read.")
            }),
            &["principal_ura"],
            false,
        ),
        name if is_principal_mutation(name) => object_schema(
            json!({
                "request": {
                    "type": "object",
                    "description": "Product-neutral principal lifecycle command envelope."
                }
            }),
            &["request"],
            true,
        ),
        _ => return None,
    })
}

fn manifest_for(ability: &str, action: AdmissionAction) -> anyhow::Result<AbilityManifest> {
    let manifest_name = ability.rsplit('.').next().unwrap_or(ability);
    let description = description_for(ability).ok_or_else(|| {
        anyhow::anyhow!("daemon Invocation ability {ability:?} is missing descriptor description")
    })?;
    let input_schema = input_schema_for(ability).ok_or_else(|| {
        anyhow::anyhow!("daemon Invocation ability {ability:?} is missing input schema")
    })?;
    AbilityManifest::new(manifest_name, description, input_schema)?
        .with_descriptor_version(DEFAULT_ABILITY_DESCRIPTOR_VERSION)?
        .with_admission_action(action.as_str())
}

fn is_principal_mutation(name: &str) -> bool {
    matches!(
        name,
        ABILITY_PRINCIPAL_CREATE
            | ABILITY_PRINCIPAL_BIND_FIRST_KEY
            | ABILITY_PRINCIPAL_ADD_KEY
            | ABILITY_PRINCIPAL_ROTATE_KEY
            | ABILITY_PRINCIPAL_REVOKE_KEY
            | ABILITY_PRINCIPAL_CONFIGURE_RECOVERY
            | ABILITY_PRINCIPAL_RECOVER
            | ABILITY_PRINCIPAL_SUSPEND
            | ABILITY_PRINCIPAL_REACTIVATE
            | ABILITY_PRINCIPAL_DELETE
            | ABILITY_PRINCIPAL_ISSUE_ENROLLMENT
            | ABILITY_PRINCIPAL_REVOKE_ENROLLMENT
            | ABILITY_PRINCIPAL_ISSUE_GRANT
            | ABILITY_PRINCIPAL_REVOKE_GRANT
    )
}

fn closed_empty_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false
    })
}

fn object_schema(properties: Value, required: &[&str], additional_properties: bool) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": additional_properties
    })
}

fn string_prop(description: &'static str) -> Value {
    json!({
        "type": "string",
        "description": description
    })
}

fn nullable_string_prop(description: &'static str) -> Value {
    json!({
        "type": ["string", "null"],
        "description": description
    })
}

fn integer_prop(description: &'static str) -> Value {
    json!({
        "type": "integer",
        "description": description
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_daemon_invocation_baseline_route_has_contract_metadata() {
        for ability in HubBaseline::required_abilities()
            .iter()
            .filter(|ability| ability.surface == BaselineSurface::DaemonInvocation)
        {
            assert!(
                admission_action_for(ability.name).is_some(),
                "{} must have an admission action",
                ability.name
            );
            assert!(
                description_for(ability.name).is_some(),
                "{} must have a descriptor description",
                ability.name
            );
            assert!(
                input_schema_for(ability.name).is_some(),
                "{} must have an input schema",
                ability.name
            );
        }
    }

    #[test]
    fn manifest_for_rejects_missing_contract_metadata() {
        let error = manifest_for("daemon.unknown", AdmissionAction::Read)
            .expect_err("daemon Invocation descriptors must be provider-backed");
        assert!(
            error.to_string().contains("missing descriptor description"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn namespace_proxy_resolve_schema_requires_explicit_resolver_tuple() {
        let schema = input_schema_for(ABILITY_NAMESPACE_PROXY_RESOLVE).expect("proxy schema");
        let required = schema["required"].as_array().expect("required array");
        for field in [
            "query_name",
            "qtype",
            "caller_ura",
            "subject_ura",
            "realm_hint",
            "ability_name",
        ] {
            assert!(
                required.iter().any(|value| value.as_str() == Some(field)),
                "namespace.proxy_resolve schema must require {field}: {schema}"
            );
        }
        let properties = schema["properties"].as_object().expect("properties");
        assert!(properties.contains_key("peer_hub_urls"));
        assert!(properties.contains_key("ability_name"));
        assert_eq!(
            properties["ability_name"]["type"],
            json!(["string", "null"])
        );
        assert!(
            !properties.contains_key("target_ura") && !properties.contains_key("peers"),
            "proxy schema must not retain retired namespace.resolve/legacy peer fields: {schema}"
        );
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn resolve_key_schema_exposes_only_base64_presented_key_pin() {
        let schema = input_schema_for(ABILITY_FEDERATION_RESOLVE_KEY).expect("resolve_key schema");
        assert_schema_requires_only(&schema, &["agent_ura"]);
        let properties = schema["properties"].as_object().expect("properties");
        assert!(properties.contains_key("agent_ura"));
        assert!(properties.contains_key("presented_pubkey_b64"));
        assert!(
            !properties.contains_key("presented_pubkey_hex"),
            "resolve_key schema must not expose retired hex presented-key pin: {schema}"
        );
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn list_user_devices_schemas_match_dispatcher_tuples_without_retired_fields() {
        let peer_schema =
            input_schema_for(ABILITY_FEDERATION_LIST_USER_DEVICES).expect("peer list schema");
        assert_schema_requires_only(&peer_schema, &["realm"]);
        let peer_properties = peer_schema["properties"].as_object().expect("properties");
        assert!(peer_properties.contains_key("realm"));
        assert!(
            !peer_properties.contains_key("user_ura")
                && !peer_properties.contains_key("peers")
                && !peer_properties.contains_key("peer_hub_urls"),
            "federation.list_user_devices schema must expose only the peer-hub realm tuple: {peer_schema}"
        );
        assert_eq!(peer_schema["additionalProperties"], false);

        let proxy_schema = input_schema_for(ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES)
            .expect("proxy list schema");
        assert_schema_requires_only(&proxy_schema, &["realm"]);
        let proxy_properties = proxy_schema["properties"].as_object().expect("properties");
        assert!(proxy_properties.contains_key("realm"));
        assert!(proxy_properties.contains_key("peer_hub_urls"));
        assert!(
            !proxy_properties.contains_key("user_ura") && !proxy_properties.contains_key("peers"),
            "federation.proxy_list_user_devices schema must not retain retired product/peer aliases: {proxy_schema}"
        );
        assert_eq!(proxy_schema["additionalProperties"], false);
    }

    fn assert_schema_requires_only(schema: &Value, expected: &[&str]) {
        let mut actual: Vec<_> = schema["required"]
            .as_array()
            .expect("required array")
            .iter()
            .map(|value| value.as_str().expect("required field"))
            .collect();
        actual.sort_unstable();
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(actual, expected, "schema required fields drifted: {schema}");
    }
}
