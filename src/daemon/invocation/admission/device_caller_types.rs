// EasyNet Daemon — Device caller policy vocabulary
// =================================================
//
// File: src/daemon/invocation/admission/device_caller_types.rs
// Description: Always-on value types shared by the pure policy engine and the
//              Axon-enabled Device caller classifier.
//
// Device caller verification depends on Axon Invocation context and is feature
// gated. The policy vocabulary is not: default-feature builds still compile the
// pure policy engine and must be able to represent the absence of a verified
// Device purpose without depending on the verifier module.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceCallerPurpose {
    Bootstrap,
    Pairing,
    PublicationCustody,
    DeviceSelfSession,
    LifecycleSelfRevoke,
    HostedAgentRetraction,
    AbilityCatalogDiff,
}

/// Opaque proof that the public invocation classifier admitted one exact
/// Device-caller purpose. Callers may inspect or compare the purpose, but
/// cannot construct a proof from a raw enum and thereby skip classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerifiedDeviceInvocationPurpose {
    pub(in crate::daemon::invocation::admission) purpose: DeviceCallerPurpose,
    pub(in crate::daemon::invocation::admission) invocation_binding: [u8; 32],
}

impl VerifiedDeviceInvocationPurpose {
    #[must_use]
    pub(crate) const fn purpose(self) -> DeviceCallerPurpose {
        self.purpose
    }

    #[must_use]
    pub(crate) fn is(self, expected: DeviceCallerPurpose) -> bool {
        self.purpose == expected
    }

    #[must_use]
    pub(crate) const fn carries_pairing_token_scope(self) -> bool {
        matches!(
            self.purpose,
            DeviceCallerPurpose::Bootstrap | DeviceCallerPurpose::Pairing
        )
    }
}
