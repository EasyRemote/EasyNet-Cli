// EasyNet Daemon — Invocation Quota Meter
// =========================================
//
// File: src/daemon/invocation/quota_meter.rs
// Description: Decides which inbound unary invokes are quota-metered
//              (commit-plan-2 E5). Federation and daemon-local system
//              verbs are control-plane traffic — throttling them would
//              break liveness and key-discovery paths — so the
//              exemption rules live here, explicit and testable, not
//              scattered through the invoke shell.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::daemon::invocation::admission::list_user_pubkeys::ABILITY_IDENTITY_LIST_USER_PUBKEYS;
use crate::daemon::invocation::admission::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY;
use crate::daemon::invocation::admission::revoke_user_pubkey::ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
use crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_LIST_USER_DEVICES, ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
    ABILITY_FEDERATION_RESOLVE, ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_STATUS, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    ABILITY_NAMESPACE_PROXY_RESOLVE, ABILITY_NAMESPACE_RESOLVE,
    ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
};

fn is_quota_exempt_system_ability(function: &str) -> bool {
    matches!(
        function,
        ABILITY_FEDERATION_JOIN
            | ABILITY_FEDERATION_ADVERTISE_AGENT
            | ABILITY_FEDERATION_ADVERTISE_ABILITIES
            | ABILITY_FEDERATION_HEARTBEAT
            | ABILITY_FEDERATION_RESOLVE
            | ABILITY_NAMESPACE_RESOLVE
            | ABILITY_NAMESPACE_PROXY_RESOLVE
            | ABILITY_FEDERATION_RESOLVE_KEY
            | ABILITY_FEDERATION_DISCOVER
            | ABILITY_FEDERATION_LIST_USER_DEVICES
            | ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES
            | ABILITY_FEDERATION_STATUS
            | ABILITY_FEDERATION_REVOKE
            | ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2
            | ABILITY_IDENTITY_REGISTER_PUBKEY
            | ABILITY_IDENTITY_REVOKE_USER_PUBKEY
            | ABILITY_IDENTITY_LIST_USER_PUBKEYS
            | ABILITY_SESSION_OPEN
            | ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY
    )
}

pub(crate) fn quota_meters_function(function: &str) -> bool {
    !is_quota_exempt_system_ability(function)
}
