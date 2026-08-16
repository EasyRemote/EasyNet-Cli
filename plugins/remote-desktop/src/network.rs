// EasyNet CLI — remote desktop network candidates
// ===============================================
//
// File: plugins/remote-desktop/src/network.rs
// Description: Typed route candidate discovery for direct WebRTC endpoints.

#[cfg(unix)]
use std::net::Ipv4Addr;

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum DirectWebRtcRouteCandidateClass {
    Host,
    StunServerReflexive,
    TurnRelay,
    EasyNetRelay,
}

impl DirectWebRtcRouteCandidateClass {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host_candidate",
            Self::StunServerReflexive => "stun_srflx",
            Self::TurnRelay => "turn_relay",
            Self::EasyNetRelay => "easynet_relay",
        }
    }
}

const DIRECT_WEBRTC_ROUTE_MODEL: &[DirectWebRtcRouteCandidateClass] = &[
    DirectWebRtcRouteCandidateClass::Host,
    DirectWebRtcRouteCandidateClass::StunServerReflexive,
    DirectWebRtcRouteCandidateClass::TurnRelay,
    DirectWebRtcRouteCandidateClass::EasyNetRelay,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcRouteCandidate {
    class: DirectWebRtcRouteCandidateClass,
    endpoint: String,
}

impl DirectWebRtcRouteCandidate {
    fn host(ip: String) -> Self {
        Self {
            class: DirectWebRtcRouteCandidateClass::Host,
            endpoint: format!("{ip}:0"),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn to_value(&self) -> Value {
        json!({
            "candidate_class": self.class.as_str(),
            "endpoint": self.endpoint,
        })
    }
}

pub(in crate::daemon::plugins::remote_desktop) trait DirectWebRtcRouteCandidateProvider {
    fn provider_id(&self) -> &'static str;
    fn provider_state(&self) -> &'static str;
    fn route_candidates(&self) -> Vec<DirectWebRtcRouteCandidate>;
}

#[derive(Debug, Clone, Copy)]
pub(in crate::daemon::plugins::remote_desktop) struct LocalInterfaceRouteCandidateProvider;

impl DirectWebRtcRouteCandidateProvider for LocalInterfaceRouteCandidateProvider {
    fn provider_id(&self) -> &'static str {
        "local_interface"
    }

    fn provider_state(&self) -> &'static str {
        "host_local_only"
    }

    fn route_candidates(&self) -> Vec<DirectWebRtcRouteCandidate> {
        direct_webrtc_host_ips()
            .into_iter()
            .map(DirectWebRtcRouteCandidate::host)
            .collect()
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn direct_webrtc_route_candidate_evidence(
    provider: &impl DirectWebRtcRouteCandidateProvider,
    candidates: &[DirectWebRtcRouteCandidate],
) -> Value {
    json!({
        "provider": provider.provider_id(),
        "provider_state": provider.provider_state(),
        "route_model": DIRECT_WEBRTC_ROUTE_MODEL
            .iter()
            .map(|class| class.as_str())
            .collect::<Vec<_>>(),
        "candidate_count": candidates.len(),
        "candidates": candidates
            .iter()
            .map(DirectWebRtcRouteCandidate::to_value)
            .collect::<Vec<_>>(),
    })
}

fn direct_webrtc_host_ips() -> Vec<String> {
    let mut ips = vec!["127.0.0.1".to_string()];
    for candidate in local_interface_host_ips() {
        if !ips.iter().any(|addr| addr == &candidate) {
            ips.push(candidate);
        }
    }
    ips
}

#[cfg(unix)]
fn local_interface_host_ips() -> Vec<String> {
    let mut addrs = Vec::new();
    let mut ifaddr = std::ptr::null_mut();
    // SAFETY: `getifaddrs` initializes a linked list owned by libc. Every
    // pointer is checked before dereference and the list is freed exactly once.
    unsafe {
        if libc::getifaddrs(&mut ifaddr) != 0 {
            return addrs;
        }

        let mut cursor = ifaddr;
        while !cursor.is_null() {
            let item = &*cursor;
            let sockaddr = item.ifa_addr;
            if !sockaddr.is_null() && (*sockaddr).sa_family as i32 == libc::AF_INET {
                let inet = *(sockaddr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(inet.sin_addr.s_addr));
                if !ip.is_unspecified() && !ip.is_loopback() {
                    addrs.push(ip.to_string());
                }
            }
            cursor = item.ifa_next;
        }
        libc::freeifaddrs(ifaddr);
    }

    addrs.sort();
    addrs.dedup();
    addrs
}

#[cfg(not(unix))]
fn local_interface_host_ips() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_webrtc_route_candidates_are_typed_host_candidates() {
        let provider = LocalInterfaceRouteCandidateProvider;
        let candidates = provider.route_candidates();
        let endpoints = candidates
            .iter()
            .map(DirectWebRtcRouteCandidate::endpoint)
            .collect::<Vec<_>>();
        assert!(
            endpoints.iter().any(|endpoint| endpoint == &"127.0.0.1:0"),
            "direct WebRTC candidates must always include loopback: {endpoints:?}"
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.class == DirectWebRtcRouteCandidateClass::Host),
            "local provider must not synthesize STUN/TURN/EasyNet relay candidates: {candidates:?}"
        );
        assert!(
            !endpoints.iter().any(|endpoint| endpoint.starts_with("8.8.8.8:")),
            "direct WebRTC candidates must not include or depend on public probe targets: {endpoints:?}"
        );
    }

    #[test]
    fn route_candidate_evidence_keeps_host_only_provider_explicit() {
        let provider = LocalInterfaceRouteCandidateProvider;
        let candidates = provider.route_candidates();
        let evidence = direct_webrtc_route_candidate_evidence(&provider, &candidates);

        assert_eq!(evidence["provider"], json!("local_interface"));
        assert_eq!(evidence["provider_state"], json!("host_local_only"));
        assert_eq!(
            evidence["route_model"],
            json!([
                "host_candidate",
                "stun_srflx",
                "turn_relay",
                "easynet_relay"
            ])
        );
        assert_eq!(
            evidence["candidates"][0]["candidate_class"],
            json!("host_candidate")
        );
    }
}
