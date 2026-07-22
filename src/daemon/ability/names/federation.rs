pub const NODE_LIST: &str = "node.list";
pub const NODE_DESCRIBE: &str = "node.describe";
pub const NODE_REMOVE: &str = "node.remove";

pub const ABILITY_DEPLOY: &str = "ability.deploy";
pub const ABILITY_UNINSTALL: &str = "ability.uninstall";
pub const ABILITY_PUBLISH: &str = "ability.publish";
pub const ABILITY_UNPUBLISH: &str = "ability.unpublish";

pub const JOIN: &str = crate::daemon::ability::conformance::ABILITY_FEDERATION_JOIN;
pub const ADVERTISE_AGENT: &str =
    crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_AGENT;
pub const ADVERTISE_ABILITIES: &str =
    crate::daemon::ability::conformance::ABILITY_FEDERATION_ADVERTISE_ABILITIES;
pub const HEARTBEAT: &str = crate::daemon::ability::conformance::ABILITY_FEDERATION_HEARTBEAT;
pub const RESOLVE: &str = crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE;
pub const DISCOVER: &str = crate::daemon::ability::conformance::ABILITY_FEDERATION_DISCOVER;
pub const STATUS: &str = crate::daemon::ability::conformance::ABILITY_FEDERATION_STATUS;
pub const NAMESPACE_RESOLVE: &str = crate::daemon::ability::conformance::ABILITY_NAMESPACE_RESOLVE;
pub const NAMESPACE_PROXY_RESOLVE: &str =
    crate::daemon::ability::conformance::ABILITY_NAMESPACE_PROXY_RESOLVE;
pub const RESOLVE_KEY: &str = crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY;
pub const SUBSCRIBE_DIRECTORY_V2: &str =
    crate::daemon::ability::conformance::ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2;
pub const LIST_USER_DEVICES: &str =
    crate::daemon::ability::conformance::ABILITY_FEDERATION_LIST_USER_DEVICES;
pub const PROXY_LIST_USER_DEVICES: &str =
    crate::daemon::ability::conformance::ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES;
pub const REVOKE: &str = crate::daemon::ability::runtime_admin_routes_gen::FEDERATION_REVOKE;

pub const IDENTITY_LIST_USER_PUBKEYS: &str = "identity.list_user_pubkeys";
pub const IDENTITY_REGISTER_PUBKEY: &str = "identity.register_pubkey";
pub const IDENTITY_REVOKE_USER_PUBKEY: &str = "identity.revoke_user_pubkey";
