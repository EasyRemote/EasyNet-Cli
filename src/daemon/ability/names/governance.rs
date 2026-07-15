pub const ADMIN_STATUS: &str = "admin.status";
pub const OBSERVE_HEALTH: &str = "observe.health";
pub const OBSERVE_NETWORK_HEALTH: &str = "observe.network_health";
pub const RUNTIME_BOOTSTRAP_SELF_IDENTITY: &str =
    crate::daemon::ability::conformance::ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY;
pub const SYSTEM_WATCH_BOOT: &str = "system.watch_boot";

pub const CONSENT_SUBSCRIBE: &str = "consent.subscribe";
pub const CONSENT_DECIDE: &str = "consent.decide";
pub const CONSENT_LIST_PENDING: &str = "consent.list_pending";

pub const INVOCATION_HISTORY_LIST: &str = "invocation.history.list";
pub const INVOCATION_HISTORY_GET: &str = "invocation.history.get";
pub const INVOCATION_HISTORY_PATH: &str = "invocation.history.path";
pub const INVOCATION_RECORD_GET: &str = "invocation.record.get";
pub const INVOCATION_TRACE_GET: &str = "invocation.trace.get";
pub const INVOCATION_CANCEL: &str = "invocation.cancel";

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
pub const META_TEACH: &str = "meta.teach";
