pub const ADMIN_STATUS: &str = "admin.status";
pub const OBSERVE_HEALTH: &str = "observe.health";
pub const OBSERVE_NETWORK_HEALTH: &str = "observe.network_health";

/// Device-sponsored SystemAgent id for local health, network posture, and
/// operator status probes. The Device remains the observed host/subject, while
/// this SystemAgent is the public callee for ordinary health reads.
pub const RUNTIME_HEALTH_SYSTEM_AGENT_ID: &str = "runtime-health";

/// Device-sponsored SystemAgent id for local RFC-014 runtime governance
/// abilities. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.runtime-governance`.
pub const RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID: &str = "runtime-governance";

/// Device-sponsored SystemAgent id for local runtime read-model introspection:
/// meta.describe, meta.list_abilities, and meta.list_resources. The canonical
/// callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.runtime-introspection`.
pub const RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID: &str = "runtime-introspection";

/// Device-sponsored SystemAgent id for local descriptor-transfer lifecycle:
/// meta.teach, meta.acquire, and meta.forget. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.descriptor-transfer`.
pub const DESCRIPTOR_TRANSFER_SYSTEM_AGENT_ID: &str = "descriptor-transfer";

/// Device-sponsored SystemAgent id for local API-key lifecycle abilities used
/// by device-hosted compatibility adapters. The canonical callee shape is
/// `easynet:///r/<realm>/agent/device.<device-id>.api-key-management`.
pub const API_KEY_MANAGEMENT_SYSTEM_AGENT_ID: &str = "api-key-management";

/// Device-sponsored SystemAgent id for local keyring administration and
/// managed-signing inventory. The Device hosts the vault/key service; this
/// SystemAgent owns the public AbilityDescriptor rows.
pub const KEYRING_MANAGEMENT_SYSTEM_AGENT_ID: &str = "keyring-management";

pub const KEYRING_CREATE: &str = "device.keyring.create";
pub const KEYRING_LIST: &str = "device.keyring.list";
pub const KEYRING_GET_PUBLIC: &str = "device.keyring.get_public";
pub const KEYRING_ROTATE: &str = "device.keyring.rotate";
pub const KEYRING_REVOKE: &str = "device.keyring.revoke";
pub const KEYRING_EXPIRE_SET: &str = "device.keyring.expire_set";
pub const KEYRING_BIND_SUBJECT: &str = "device.keyring.bind_subject";
pub const KEYRING_PEER_ADD: &str = "device.keyring.peer_add";
pub const KEYRING_PEER_LIST: &str = "device.keyring.peer_list";
pub const KEYRING_FEDERATE_USER_IDENTITY_TOKEN: &str =
    "device.keyring.federate_user_identity_token";

pub const KEYRING_ABILITIES: [&str; 10] = [
    KEYRING_CREATE,
    KEYRING_LIST,
    KEYRING_GET_PUBLIC,
    KEYRING_ROTATE,
    KEYRING_REVOKE,
    KEYRING_EXPIRE_SET,
    KEYRING_BIND_SUBJECT,
    KEYRING_PEER_ADD,
    KEYRING_PEER_LIST,
    KEYRING_FEDERATE_USER_IDENTITY_TOKEN,
];

/// Device-sponsored SystemAgent id for the daemon-native human-consent broker.
/// Consent is a governed runtime service, not a synthetic User-owned Agent.
pub const CONSENT_SYSTEM_AGENT_ID: &str = "consent-management";

pub const RUNTIME_BOOTSTRAP_SELF_IDENTITY: &str =
    crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY;
pub const SYSTEM_WATCH_BOOT: &str = "system.watch_boot";

pub const CONSENT_SUBSCRIBE: &str = "consent.subscribe";
pub const CONSENT_DECIDE: &str = "consent.decide";
pub const CONSENT_LIST_PENDING: &str = "consent.list_pending";

pub const INVOCATION_HISTORY_LIST: &str =
    crate::daemon::ability::receipt_routes_gen::INVOCATION_HISTORY_LIST;
pub const INVOCATION_HISTORY_GET: &str =
    crate::daemon::ability::receipt_routes_gen::INVOCATION_HISTORY_GET;
pub const INVOCATION_HISTORY_PATH: &str = "invocation.history.path";
pub const INVOCATION_RECORD_GET: &str = "invocation.record.get";
pub const INVOCATION_TRACE_GET: &str =
    crate::daemon::ability::receipt_routes_gen::INVOCATION_TRACE_GET;
pub const INVOCATION_CANCEL: &str = "invocation.cancel";

/// Canonical receipt-history/governance read-model ability family.
///
/// These abilities are runtime governance reads. They are callable through the
/// canonical history read issuer, not as product/public remote actions owned by
/// a target device. Keeping the predicate with the governance ability names
/// prevents CLI wrappers, SDK/FFI direct ingress, and daemon dispatch from
/// carrying separate allow/deny lists.
pub(crate) fn is_invocation_history_read(ability: &str) -> bool {
    crate::daemon::ability::runtime_governance_routes_gen::descriptor_provider(ability)
        == Some(crate::daemon::ability::runtime_governance_routes_gen::RECEIPT_HISTORY_PROVIDER)
}

/// Canonical runtime catalogue read ability family.
///
/// Remote catalogue reads have a distinct tuple policy from product actions:
/// the subject is the runtime owner being read. The predicate lives next to the
/// governance ability names so remote issuers and direct dispatch admission do
/// not maintain parallel string lists.
pub(crate) fn is_runtime_catalogue_read(ability: &str) -> bool {
    crate::daemon::ability::runtime_governance_routes_gen::descriptor_provider(ability)
        == Some(crate::daemon::ability::runtime_governance_routes_gen::ABILITY_DESCRIPTOR_PROVIDER)
}

pub const AUTHORITY_BINDING_GRANT: &str =
    crate::daemon::ability::access_control_routes_gen::AUTHORITY_BINDING_GRANT;
pub const AUTHORITY_BINDING_REVOKE: &str =
    crate::daemon::ability::access_control_routes_gen::AUTHORITY_BINDING_REVOKE;
pub const AUTHORITY_BINDING_LIST: &str =
    crate::daemon::ability::access_control_routes_gen::AUTHORITY_BINDING_LIST;
pub const AUTHORITY_BINDING_CHECK: &str =
    crate::daemon::ability::access_control_routes_gen::AUTHORITY_BINDING_CHECK;

pub const POLICY_REQUEST_CREATE: &str =
    crate::daemon::ability::access_control_routes_gen::POLICY_REQUEST_CREATE;
pub const POLICY_REQUEST_RESOLVE: &str =
    crate::daemon::ability::access_control_routes_gen::POLICY_REQUEST_RESOLVE;
pub const POLICY_REQUEST_LIST: &str =
    crate::daemon::ability::access_control_routes_gen::POLICY_REQUEST_LIST;

pub const ADMISSION_EXPLAIN: &str =
    crate::daemon::ability::access_control_routes_gen::ADMISSION_EXPLAIN;

pub const PRINCIPAL_CREATE: &str = crate::daemon::ability::conformance::ABILITY_PRINCIPAL_CREATE;
pub const PRINCIPAL_BIND_FIRST_KEY: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_BIND_FIRST_KEY;
pub const PRINCIPAL_ADD_KEY: &str = crate::daemon::ability::conformance::ABILITY_PRINCIPAL_ADD_KEY;
pub const PRINCIPAL_ROTATE_KEY: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_ROTATE_KEY;
pub const PRINCIPAL_REVOKE_KEY: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_REVOKE_KEY;
pub const PRINCIPAL_CONFIGURE_RECOVERY: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_CONFIGURE_RECOVERY;
pub const PRINCIPAL_RECOVER: &str = crate::daemon::ability::conformance::ABILITY_PRINCIPAL_RECOVER;
pub const PRINCIPAL_SUSPEND: &str = crate::daemon::ability::conformance::ABILITY_PRINCIPAL_SUSPEND;
pub const PRINCIPAL_REACTIVATE: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_REACTIVATE;
pub const PRINCIPAL_DELETE: &str = crate::daemon::ability::conformance::ABILITY_PRINCIPAL_DELETE;
pub const PRINCIPAL_ISSUE_ENROLLMENT: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_ISSUE_ENROLLMENT;
pub const PRINCIPAL_REVOKE_ENROLLMENT: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_REVOKE_ENROLLMENT;
pub const PRINCIPAL_ISSUE_GRANT: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_ISSUE_GRANT;
pub const PRINCIPAL_REVOKE_GRANT: &str =
    crate::daemon::ability::conformance::ABILITY_PRINCIPAL_REVOKE_GRANT;
pub const PRINCIPAL_GET: &str = crate::daemon::ability::conformance::ABILITY_PRINCIPAL_GET;

pub const META_ACQUIRE: &str = "meta.acquire";
pub const META_DESCRIBE: &str = "meta.describe";
pub const META_FORGET: &str = "meta.forget";
pub const META_LIST_ABILITIES: &str = "meta.list_abilities";
pub const META_LIST_RESOURCES: &str = crate::daemon::ability::names::resources::META_LIST_RESOURCES;
pub const META_TEACH: &str = "meta.teach";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_runtime_governance_routes_are_exact_not_prefix_based() {
        assert!(is_invocation_history_read("invocation.record.get"));
        assert!(is_invocation_history_read("invocation.history.list"));
        assert!(!is_invocation_history_read("invocation.history.delete"));
        assert!(is_runtime_catalogue_read("meta.list_abilities"));
        assert!(is_runtime_catalogue_read("meta.list_resources"));
        assert!(!is_runtime_catalogue_read("meta.list_everything"));
    }
}
