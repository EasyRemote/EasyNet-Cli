// EasyNet CLI — remote desktop network candidates
// ===============================================
//
// File: plugins/remote-desktop/src/network.rs
// Description: Typed route candidate discovery for direct WebRTC endpoints.

#[cfg(unix)]
use std::net::Ipv4Addr;

use anyhow::{bail, Context};
use serde_json::{json, Value};

const ENV_STUN_URLS: &str = "EASYNET_REMOTE_DESKTOP_STUN_URLS";
const ENV_TURN_URLS: &str = "EASYNET_REMOTE_DESKTOP_TURN_URLS";
const ENV_TURN_USERNAME: &str = "EASYNET_REMOTE_DESKTOP_TURN_USERNAME";
const ENV_TURN_CREDENTIAL: &str = "EASYNET_REMOTE_DESKTOP_TURN_CREDENTIAL";
const ENV_EASYNET_RELAY_URLS: &str = "EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_URLS";
const ENV_EASYNET_RELAY_USERNAME: &str = "EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_USERNAME";
const ENV_EASYNET_RELAY_CREDENTIAL: &str = "EASYNET_REMOTE_DESKTOP_EASYNET_RELAY_CREDENTIAL";

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

    fn configured(class: DirectWebRtcRouteCandidateClass, endpoint: String) -> Self {
        Self { class, endpoint }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(in crate::daemon::plugins::remote_desktop) fn local_bind_endpoint(&self) -> Option<&str> {
        (self.class == DirectWebRtcRouteCandidateClass::Host).then_some(self.endpoint())
    }

    fn to_value(&self) -> Value {
        json!({
            "candidate_class": self.class.as_str(),
            "endpoint": self.endpoint,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcIceServerConfig {
    class: DirectWebRtcRouteCandidateClass,
    urls: Vec<String>,
    username: String,
    credential: String,
}

impl DirectWebRtcIceServerConfig {
    fn new(
        class: DirectWebRtcRouteCandidateClass,
        urls: Vec<String>,
        username: String,
        credential: String,
    ) -> anyhow::Result<Self> {
        if urls.is_empty() {
            bail!("direct WebRTC ICE route for {} has no URLs", class.as_str());
        }
        match class {
            DirectWebRtcRouteCandidateClass::StunServerReflexive => {
                for url in &urls {
                    require_ice_url_scheme(url, &[IceUrlScheme::Stun, IceUrlScheme::Stuns])?;
                }
                if !username.is_empty() || !credential.is_empty() {
                    bail!("STUN route config must not carry relay credentials");
                }
            }
            DirectWebRtcRouteCandidateClass::TurnRelay
            | DirectWebRtcRouteCandidateClass::EasyNetRelay => {
                for url in &urls {
                    require_ice_url_scheme(url, &[IceUrlScheme::Turn, IceUrlScheme::Turns])?;
                }
                if username.is_empty() || credential.is_empty() {
                    bail!(
                        "{} route config requires both username and credential",
                        class.as_str()
                    );
                }
            }
            DirectWebRtcRouteCandidateClass::Host => {
                bail!("host route candidates are local bind endpoints, not ICE server config");
            }
        }
        Ok(Self {
            class,
            urls,
            username,
            credential,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn urls(&self) -> &[String] {
        &self.urls
    }

    pub(in crate::daemon::plugins::remote_desktop) fn username(&self) -> &str {
        &self.username
    }

    pub(in crate::daemon::plugins::remote_desktop) fn credential(&self) -> &str {
        &self.credential
    }

    fn route_candidates(&self) -> Vec<DirectWebRtcRouteCandidate> {
        self.urls
            .iter()
            .cloned()
            .map(|url| DirectWebRtcRouteCandidate::configured(self.class, url))
            .collect()
    }

    fn to_evidence_value(&self) -> Value {
        json!({
            "candidate_class": self.class.as_str(),
            "urls": self.urls,
            "credential_configured": !self.username.is_empty() || !self.credential.is_empty(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcRouteConfig {
    ice_servers: Vec<DirectWebRtcIceServerConfig>,
}

impl DirectWebRtcRouteConfig {
    pub(in crate::daemon::plugins::remote_desktop) fn from_env() -> anyhow::Result<Self> {
        let mut config = Self::default();
        config.add_stun_urls(read_url_list_env(ENV_STUN_URLS)?)?;
        config.add_turn_urls(
            read_url_list_env(ENV_TURN_URLS)?,
            read_optional_env(ENV_TURN_USERNAME)?,
            read_optional_env(ENV_TURN_CREDENTIAL)?,
        )?;
        config.add_easynet_relay_urls(
            read_url_list_env(ENV_EASYNET_RELAY_URLS)?,
            read_optional_env(ENV_EASYNET_RELAY_USERNAME)?,
            read_optional_env(ENV_EASYNET_RELAY_CREDENTIAL)?,
        )?;
        Ok(config)
    }

    fn add_stun_urls(&mut self, urls: Vec<String>) -> anyhow::Result<()> {
        if urls.is_empty() {
            return Ok(());
        }
        self.ice_servers.push(DirectWebRtcIceServerConfig::new(
            DirectWebRtcRouteCandidateClass::StunServerReflexive,
            urls,
            String::new(),
            String::new(),
        )?);
        Ok(())
    }

    fn add_turn_urls(
        &mut self,
        urls: Vec<String>,
        username: Option<String>,
        credential: Option<String>,
    ) -> anyhow::Result<()> {
        if urls.is_empty() {
            return Ok(());
        }
        self.ice_servers.push(DirectWebRtcIceServerConfig::new(
            DirectWebRtcRouteCandidateClass::TurnRelay,
            urls,
            username.unwrap_or_default(),
            credential.unwrap_or_default(),
        )?);
        Ok(())
    }

    fn add_easynet_relay_urls(
        &mut self,
        urls: Vec<String>,
        username: Option<String>,
        credential: Option<String>,
    ) -> anyhow::Result<()> {
        if urls.is_empty() {
            return Ok(());
        }
        self.ice_servers.push(DirectWebRtcIceServerConfig::new(
            DirectWebRtcRouteCandidateClass::EasyNetRelay,
            urls,
            username.unwrap_or_default(),
            credential.unwrap_or_default(),
        )?);
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.ice_servers.is_empty()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn ice_servers(
        &self,
    ) -> &[DirectWebRtcIceServerConfig] {
        &self.ice_servers
    }

    fn configured_candidates(&self) -> Vec<DirectWebRtcRouteCandidate> {
        self.ice_servers
            .iter()
            .flat_map(DirectWebRtcIceServerConfig::route_candidates)
            .collect()
    }

    fn evidence_value(&self) -> Value {
        let stun_server_count = self
            .ice_servers
            .iter()
            .filter(|server| server.class == DirectWebRtcRouteCandidateClass::StunServerReflexive)
            .count();
        let turn_server_count = self
            .ice_servers
            .iter()
            .filter(|server| server.class == DirectWebRtcRouteCandidateClass::TurnRelay)
            .count();
        let easynet_relay_count = self
            .ice_servers
            .iter()
            .filter(|server| server.class == DirectWebRtcRouteCandidateClass::EasyNetRelay)
            .count();
        json!({
            "ice_server_count": self.ice_servers.len(),
            "stun_server_count": stun_server_count,
            "turn_server_count": turn_server_count,
            "easynet_relay_count": easynet_relay_count,
            "ice_servers": self
                .ice_servers
                .iter()
                .map(DirectWebRtcIceServerConfig::to_evidence_value)
                .collect::<Vec<_>>(),
        })
    }
}

pub(in crate::daemon::plugins::remote_desktop) trait DirectWebRtcRouteCandidateProvider {
    fn provider_id(&self) -> &'static str;
    fn provider_state(&self) -> &'static str;
    fn route_candidates(&self) -> Vec<DirectWebRtcRouteCandidate>;
    fn ice_servers(&self) -> Vec<DirectWebRtcIceServerConfig>;
    fn route_config_evidence(&self) -> Value;
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

    fn ice_servers(&self) -> Vec<DirectWebRtcIceServerConfig> {
        Vec::new()
    }

    fn route_config_evidence(&self) -> Value {
        DirectWebRtcRouteConfig::default().evidence_value()
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct ConfiguredDirectWebRtcRouteProvider {
    local: LocalInterfaceRouteCandidateProvider,
    route_config: DirectWebRtcRouteConfig,
}

impl ConfiguredDirectWebRtcRouteProvider {
    pub(in crate::daemon::plugins::remote_desktop) fn from_env() -> anyhow::Result<Self> {
        Ok(Self::new(DirectWebRtcRouteConfig::from_env()?))
    }

    fn new(route_config: DirectWebRtcRouteConfig) -> Self {
        Self {
            local: LocalInterfaceRouteCandidateProvider,
            route_config,
        }
    }
}

impl DirectWebRtcRouteCandidateProvider for ConfiguredDirectWebRtcRouteProvider {
    fn provider_id(&self) -> &'static str {
        "configured_direct_webrtc_route"
    }

    fn provider_state(&self) -> &'static str {
        if self.route_config.is_empty() {
            "host_local_only"
        } else {
            "configured_ice_routes"
        }
    }

    fn route_candidates(&self) -> Vec<DirectWebRtcRouteCandidate> {
        let mut candidates = self.local.route_candidates();
        candidates.extend(self.route_config.configured_candidates());
        candidates
    }

    fn ice_servers(&self) -> Vec<DirectWebRtcIceServerConfig> {
        self.route_config.ice_servers().to_vec()
    }

    fn route_config_evidence(&self) -> Value {
        self.route_config.evidence_value()
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
        "route_config": provider.route_config_evidence(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IceUrlScheme {
    Stun,
    Stuns,
    Turn,
    Turns,
}

impl IceUrlScheme {
    fn prefix(self) -> &'static str {
        match self {
            Self::Stun => "stun:",
            Self::Stuns => "stuns:",
            Self::Turn => "turn:",
            Self::Turns => "turns:",
        }
    }
}

fn require_ice_url_scheme(url: &str, allowed: &[IceUrlScheme]) -> anyhow::Result<()> {
    if allowed
        .iter()
        .any(|scheme| url.starts_with(scheme.prefix()))
    {
        return Ok(());
    }
    bail!("unsupported direct WebRTC ICE route URL scheme for {url}");
}

fn read_url_list_env(name: &str) -> anyhow::Result<Vec<String>> {
    let Some(raw) = read_optional_env(name)? else {
        return Ok(Vec::new());
    };
    let values = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    Ok(values)
}

fn read_optional_env(name: &str) -> anyhow::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value.trim().to_string()).filter(|value| !value.is_empty())),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(anyhow::anyhow!("{name} must be valid Unicode"))
                .with_context(|| format!("read direct WebRTC route config {name}"))
        }
    }
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
            .filter_map(DirectWebRtcRouteCandidate::local_bind_endpoint)
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
        assert_eq!(evidence["route_config"]["ice_server_count"], json!(0));
    }

    #[test]
    fn configured_route_provider_projects_ice_servers_without_credentials_in_evidence() {
        let mut config = DirectWebRtcRouteConfig::default();
        config
            .add_stun_urls(vec!["stun:stun.example.test:3478".to_string()])
            .expect("stun route");
        config
            .add_turn_urls(
                vec!["turn:turn.example.test:3478?transport=udp".to_string()],
                Some("turn-user".to_string()),
                Some("turn-secret".to_string()),
            )
            .expect("turn route");
        config
            .add_easynet_relay_urls(
                vec!["turns:relay.easynet.test:5349?transport=tcp".to_string()],
                Some("relay-user".to_string()),
                Some("relay-secret".to_string()),
            )
            .expect("easynet relay route");
        let provider = ConfiguredDirectWebRtcRouteProvider::new(config);
        let candidates = provider.route_candidates();
        let evidence = direct_webrtc_route_candidate_evidence(&provider, &candidates);

        assert_eq!(provider.provider_state(), "configured_ice_routes");
        assert_eq!(provider.ice_servers().len(), 3);
        assert_eq!(evidence["route_config"]["ice_server_count"], json!(3));
        assert_eq!(evidence["route_config"]["stun_server_count"], json!(1));
        assert_eq!(evidence["route_config"]["turn_server_count"], json!(1));
        assert_eq!(evidence["route_config"]["easynet_relay_count"], json!(1));
        assert_eq!(
            evidence["route_config"]["ice_servers"][1]["credential_configured"],
            json!(true)
        );
        assert!(
            !evidence.to_string().contains("turn-secret"),
            "TURN credentials must not leak into public route evidence: {evidence}"
        );
        assert!(
            !evidence.to_string().contains("relay-secret"),
            "EasyNet relay credentials must not leak into public route evidence: {evidence}"
        );
    }

    #[test]
    fn configured_ice_routes_do_not_become_local_udp_bind_endpoints() {
        let mut config = DirectWebRtcRouteConfig::default();
        config
            .add_stun_urls(vec!["stun:stun.example.test:3478".to_string()])
            .expect("stun route");
        config
            .add_turn_urls(
                vec!["turn:turn.example.test:3478?transport=udp".to_string()],
                Some("turn-user".to_string()),
                Some("turn-secret".to_string()),
            )
            .expect("turn route");
        let provider = ConfiguredDirectWebRtcRouteProvider::new(config);

        let local_bind_endpoints = provider
            .route_candidates()
            .iter()
            .filter_map(DirectWebRtcRouteCandidate::local_bind_endpoint)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        assert!(
            local_bind_endpoints
                .iter()
                .all(|endpoint| !endpoint.starts_with("stun:")
                    && !endpoint.starts_with("turn:")
                    && !endpoint.starts_with("turns:")),
            "ICE server URLs must not be passed to with_udp_addrs: {local_bind_endpoints:?}"
        );
    }

    #[test]
    fn turn_route_config_requires_credentials() {
        let mut config = DirectWebRtcRouteConfig::default();
        let error = config
            .add_turn_urls(
                vec!["turn:turn.example.test:3478?transport=udp".to_string()],
                None,
                Some("turn-secret".to_string()),
            )
            .expect_err("TURN route without username must fail closed");

        assert!(
            error
                .to_string()
                .contains("requires both username and credential"),
            "unexpected error: {error}"
        );
    }
}
