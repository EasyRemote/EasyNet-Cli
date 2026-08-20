//! EasyNet product resolver wire vocabulary.
//!
//! Axon carries these values as opaque Invocation JSON. Their lifecycle and
//! interpretation belong to the EasyNet daemon's federation/route policy, so
//! they must not live in the canonical runtime SDK or its generated schemas.

macro_rules! resolver_wire_enum {
    (
        $(#[$meta:meta])*
        $visibility:vis enum $name:ident {
            $($variant:ident = $number:literal => $wire:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(i32)]
        $visibility enum $name {
            $($variant = $number),+
        }

        impl $name {
            // A closed wire vocabulary intentionally retains both directions
            // even when one product path currently consumes only one of them.
            #[allow(dead_code)]
            #[must_use]
            $visibility const fn as_str_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            #[allow(dead_code)]
            #[must_use]
            $visibility fn from_str_name(value: &str) -> Option<Self> {
                match value {
                    $($wire => Some(Self::$variant)),+,
                    _ => None,
                }
            }
        }

        impl TryFrom<i32> for $name {
            type Error = ();

            fn try_from(value: i32) -> Result<Self, Self::Error> {
                match value {
                    $($number => Ok(Self::$variant)),+,
                    _ => Err(()),
                }
            }
        }
    };
}

resolver_wire_enum! {
    pub(crate) enum UraKind {
        Unspecified = 0 => "URA_KIND_UNSPECIFIED",
        Hub = 1 => "URA_KIND_HUB",
        Device = 2 => "URA_KIND_DEVICE",
        User = 3 => "URA_KIND_USER",
        Agent = 4 => "URA_KIND_AGENT",
        Ability = 5 => "URA_KIND_ABILITY",
        Resource = 6 => "URA_KIND_RESOURCE",
    }
}

resolver_wire_enum! {
    pub(crate) enum RecordType {
        Unspecified = 0 => "RECORD_TYPE_UNSPECIFIED",
        Id = 1 => "RECORD_TYPE_ID",
        Alias = 2 => "RECORD_TYPE_ALIAS",
        Delegate = 3 => "RECORD_TYPE_DELEGATE",
        HostedBy = 4 => "RECORD_TYPE_HOSTED_BY",
        Ability = 5 => "RECORD_TYPE_ABILITY",
        Route = 6 => "RECORD_TYPE_ROUTE",
        Service = 7 => "RECORD_TYPE_SERVICE",
        Key = 8 => "RECORD_TYPE_KEY",
        Policy = 9 => "RECORD_TYPE_POLICY",
        Negative = 10 => "RECORD_TYPE_NEGATIVE",
    }
}

resolver_wire_enum! {
    pub(crate) enum NegativeReason {
        Unspecified = 0 => "NEGATIVE_REASON_UNSPECIFIED",
        Nxdomain = 1 => "NEGATIVE_REASON_NXDOMAIN",
        Nodata = 2 => "NEGATIVE_REASON_NODATA",
        Noroute = 3 => "NEGATIVE_REASON_NOROUTE",
        Stale = 4 => "NEGATIVE_REASON_STALE",
        Unauthorized = 5 => "NEGATIVE_REASON_UNAUTHORIZED",
        Throttled = 6 => "NEGATIVE_REASON_THROTTLED",
        Overloaded = 7 => "NEGATIVE_REASON_OVERLOADED",
        Refused = 8 => "NEGATIVE_REASON_REFUSED",
        Loop = 9 => "NEGATIVE_REASON_LOOP",
    }
}

resolver_wire_enum! {
    pub(crate) enum ResolveType {
        Unspecified = 0 => "RESOLVE_TYPE_UNSPECIFIED",
        CanonicalIdentity = 1 => "RESOLVE_TYPE_CANONICAL_IDENTITY",
        Owner = 2 => "RESOLVE_TYPE_OWNER",
        Ability = 3 => "RESOLVE_TYPE_ABILITY",
        Route = 4 => "RESOLVE_TYPE_ROUTE",
        Key = 5 => "RESOLVE_TYPE_KEY",
        Service = 6 => "RESOLVE_TYPE_SERVICE",
        DirectoryListing = 7 => "RESOLVE_TYPE_DIRECTORY_LISTING",
    }
}

resolver_wire_enum! {
    pub(crate) enum ResolveAnswerKind {
        Unspecified = 0 => "RESOLVE_ANSWER_KIND_UNSPECIFIED",
        NonDispatchable = 1 => "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        Delegation = 2 => "RESOLVE_ANSWER_KIND_DELEGATION",
        FinalRoute = 3 => "RESOLVE_ANSWER_KIND_FINAL_ROUTE",
        Negative = 4 => "RESOLVE_ANSWER_KIND_NEGATIVE",
    }
}

resolver_wire_enum! {
    pub(crate) enum ResolverReleaseProfile {
        Unspecified = 0 => "RESOLVER_RELEASE_PROFILE_UNSPECIFIED",
        Preview = 1 => "RESOLVER_RELEASE_PROFILE_PREVIEW",
        ShadowRead = 2 => "RESOLVER_RELEASE_PROFILE_SHADOW_READ",
        AuthoritativeLocal = 3 => "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL",
        Production = 4 => "RESOLVER_RELEASE_PROFILE_PRODUCTION",
    }
}

resolver_wire_enum! {
    pub(crate) enum RouteHealth {
        Unspecified = 0 => "ROUTE_HEALTH_UNSPECIFIED",
        Healthy = 1 => "ROUTE_HEALTH_HEALTHY",
        Degraded = 2 => "ROUTE_HEALTH_DEGRADED",
        Unhealthy = 3 => "ROUTE_HEALTH_UNHEALTHY",
        Unknown = 4 => "ROUTE_HEALTH_UNKNOWN",
    }
}

resolver_wire_enum! {
    pub(crate) enum RouteReason {
        Unspecified = 0 => "ROUTE_REASON_UNSPECIFIED",
        LocalHub = 1 => "ROUTE_REASON_LOCAL_HUB",
        LocalDevice = 2 => "ROUTE_REASON_LOCAL_DEVICE",
        HostedAgent = 3 => "ROUTE_REASON_HOSTED_AGENT",
        PeerDelegation = 4 => "ROUTE_REASON_PEER_DELEGATION",
        WeightedSelection = 5 => "ROUTE_REASON_WEIGHTED_SELECTION",
    }
}

resolver_wire_enum! {
    pub(crate) enum GateResult {
        Unspecified = 0 => "GATE_RESULT_UNSPECIFIED",
        Pass = 1 => "GATE_RESULT_PASS",
        Fail = 2 => "GATE_RESULT_FAIL",
        NotApplicable = 3 => "GATE_RESULT_NOT_APPLICABLE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_wire_vocabulary_is_stable_and_fail_closed() {
        assert_eq!(
            ResolveType::from_str_name("RESOLVE_TYPE_ROUTE"),
            Some(ResolveType::Route)
        );
        assert_eq!(ResolveType::try_from(4), Ok(ResolveType::Route));
        assert_eq!(
            NegativeReason::Noroute.as_str_name(),
            "NEGATIVE_REASON_NOROUTE"
        );
        assert_eq!(
            ResolverReleaseProfile::AuthoritativeLocal.as_str_name(),
            "RESOLVER_RELEASE_PROFILE_AUTHORITATIVE_LOCAL"
        );
        assert_eq!(ResolveType::from_str_name("ROUTE"), None);
        assert_eq!(ResolveType::try_from(99), Err(()));
    }
}
