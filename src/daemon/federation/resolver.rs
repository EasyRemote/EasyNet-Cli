// EasyNet CLI — Realm suffix resolver (RFC-002 §7)
// ===================================================
//
// File: src/daemon/federation/resolver.rs
//
// Maps realm strings (the values stored in the credentials file
// under `realm`) to:
//
//   - admission mode: Local-fast vs Federated
//   - hub endpoint(s) to dial for federation calls
//
// Suffix rules:
//   *.localhost     → Local mode, no hub
//   *.easynet       → Federated, hub list from rendezvous config
//   *.<other-tld>   → Federated, hub from DNS TXT (or static config)
//   anything else   → Local mode by default (preserves pre-RFC-002 behaviour)
//
// `UraScope` (Prv|Org) remains as informational metadata for callers
// that want a hint at the federation posture, but does NOT influence
// URI shape — v4.1.5 §A.URA-7 has one canonical form per role
// (`easynet:///r/<realm>/<role>/<tail>`); the legacy
// `r/{prv,org}/reg/agent.<id>?tenant_id=<t>` shapes are dead.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionMode {
    /// No KeyResolver installed; signatures not crypto-verified.
    /// RFC-001 §5.2.
    LocalFast,
    /// KeyResolver installed; envelopes must be signed and verifiable.
    Federated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UraScope {
    /// Realm-private federation posture (informational; v4.1.5 URAs
    /// do not embed `?tenant_id=` — tenant binding rides envelope).
    Prv,
    /// Organisation-scoped public-ish; bound-tenant mode applies.
    Org,
}

impl UraScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            UraScope::Prv => "prv",
            UraScope::Org => "org",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RealmResolution {
    pub mode: AdmissionMode,
    pub scope: UraScope,
    /// Hub endpoints (URLs) to dial for federation.* calls. Empty
    /// when `mode == LocalFast`. Populated by suffix-specific
    /// configuration; the resolver returns the *resolved* list, not
    /// raw input. Multiple endpoints permit failover.
    pub hub_endpoints: Vec<String>,
    /// Echo of the realm used for traceability in receipts.
    pub realm: String,
}

/// Static configuration for resolver behaviour. In production this is
/// loaded from `~/.config/easynet/rendezvous.json`; tests inject a
/// `ResolverConfig` directly.
#[derive(Debug, Clone, Default)]
pub struct ResolverConfig {
    /// Endpoints to use for any realm ending in `.easynet`. Empty by
    /// default — operators opt in by adding endpoints.
    pub easynet_rendezvous: Vec<String>,
    /// Per-domain static hub overrides. Maps full realm (or
    /// suffix `.<host>`) to hub endpoints. Used for `<host>.com`-style
    /// realms when DNS TXT lookup is not configured (or in tests).
    pub static_hubs: std::collections::HashMap<String, Vec<String>>,
}

impl ResolverConfig {
    pub fn from_env_and_file() -> Self {
        let mut cfg = ResolverConfig::default();
        if let Ok(list) = std::env::var("EASYNET_RENDEZVOUS") {
            cfg.easynet_rendezvous = list
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(path) = dirs::config_dir().map(|d| d.join("easynet").join("rendezvous.json")) {
            if path.exists() {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(parsed) = serde_json::from_slice::<ResolverConfig>(&bytes) {
                        if !parsed.easynet_rendezvous.is_empty() {
                            cfg.easynet_rendezvous = parsed.easynet_rendezvous;
                        }
                        for (k, v) in parsed.static_hubs {
                            cfg.static_hubs.insert(k, v);
                        }
                    }
                }
            }
        }
        cfg
    }
}

impl serde::Serialize for ResolverConfig {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("ResolverConfig", 2)?;
        st.serialize_field("easynet_rendezvous", &self.easynet_rendezvous)?;
        st.serialize_field("static_hubs", &self.static_hubs)?;
        st.end()
    }
}

impl<'de> serde::Deserialize<'de> for ResolverConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Helper {
            #[serde(default)]
            easynet_rendezvous: Vec<String>,
            #[serde(default)]
            static_hubs: std::collections::HashMap<String, Vec<String>>,
        }
        let h = Helper::deserialize(d)?;
        Ok(ResolverConfig {
            easynet_rendezvous: h.easynet_rendezvous,
            static_hubs: h.static_hubs,
        })
    }
}

/// Resolve a realm into mode + scope + endpoints.
pub fn resolve(realm: &str, cfg: &ResolverConfig) -> RealmResolution {
    let lower = realm.to_ascii_lowercase();

    if let Some(endpoints) = cfg.static_hubs.get(&lower) {
        return RealmResolution {
            mode: AdmissionMode::Federated,
            scope: UraScope::Org,
            hub_endpoints: endpoints.clone(),
            realm: realm.to_string(),
        };
    }

    if lower.ends_with(".localhost") || lower == "localhost" {
        return RealmResolution {
            mode: AdmissionMode::LocalFast,
            scope: UraScope::Prv,
            hub_endpoints: Vec::new(),
            realm: realm.to_string(),
        };
    }

    if lower.ends_with(".easynet") {
        return RealmResolution {
            mode: AdmissionMode::Federated,
            scope: UraScope::Org,
            hub_endpoints: cfg.easynet_rendezvous.clone(),
            realm: realm.to_string(),
        };
    }

    // FQDN: defer DNS TXT lookup to a future phase; for now treat as
    // Federated with no static endpoints — the daemon must have a
    // static_hubs entry configured, otherwise federation is a no-op
    // until the operator adds one.
    if lower.contains('.') {
        return RealmResolution {
            mode: AdmissionMode::Federated,
            scope: UraScope::Org,
            hub_endpoints: Vec::new(),
            realm: realm.to_string(),
        };
    }

    // Bare token (legacy `tenant-test`, `acme`, etc.). Backward-compat:
    // treat as Local-fast under prv scope.
    RealmResolution {
        mode: AdmissionMode::LocalFast,
        scope: UraScope::Prv,
        hub_endpoints: Vec::new(),
        realm: realm.to_string(),
    }
}

/// Build a v4.1.5 standard device URA. Shape is identical regardless
/// of `resolution.scope` (prv/org) — v4.1.5 §A.URA-7 has only one
/// device URI form: `easynet:///r/<realm>/device/<node-id>`. The
/// realm rides in the URA; tenant binding rides envelope, not URI,
/// so the legacy `?tenant_id=<t>` query is gone. The `scope` field
/// remains on `RealmResolution` as informational metadata for the
/// federation layer's hub-discovery decision but does NOT appear in
/// any URI.
pub fn canonical_device_ura(node_id: &str, resolution: &RealmResolution) -> String {
    crate::core::ura::device_ura(&resolution.realm, node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ResolverConfig {
        ResolverConfig {
            easynet_rendezvous: vec!["http://rendezvous.easynet:7700".into()],
            static_hubs: [(
                "acme.com".to_string(),
                vec!["http://acme-hub.acme.com:7700".into()],
            )]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn localhost_suffix_is_local_fast_prv() {
        let r = resolve("silan.localhost", &cfg());
        assert_eq!(r.mode, AdmissionMode::LocalFast);
        assert_eq!(r.scope, UraScope::Prv);
        assert!(r.hub_endpoints.is_empty());
    }

    #[test]
    fn easynet_suffix_is_federated_with_rendezvous() {
        let r = resolve("silan.easynet", &cfg());
        assert_eq!(r.mode, AdmissionMode::Federated);
        assert_eq!(r.scope, UraScope::Org);
        assert_eq!(r.hub_endpoints.len(), 1);
        assert!(r.hub_endpoints[0].contains("rendezvous"));
    }

    #[test]
    fn static_hubs_take_precedence() {
        let r = resolve("acme.com", &cfg());
        assert_eq!(r.mode, AdmissionMode::Federated);
        assert_eq!(r.scope, UraScope::Org);
        assert_eq!(r.hub_endpoints[0], "http://acme-hub.acme.com:7700");
    }

    #[test]
    fn unknown_fqdn_is_federated_but_endpointless() {
        let r = resolve("alice.xyz", &cfg());
        assert_eq!(r.mode, AdmissionMode::Federated);
        assert!(r.hub_endpoints.is_empty());
    }

    #[test]
    fn bare_token_falls_back_to_local() {
        let r = resolve("tenant-test", &cfg());
        assert_eq!(r.mode, AdmissionMode::LocalFast);
        assert_eq!(r.scope, UraScope::Prv);
    }

    #[test]
    fn case_insensitive_suffix_match() {
        let r = resolve("Silan.LOCALHOST", &cfg());
        assert_eq!(r.mode, AdmissionMode::LocalFast);
    }

    #[test]
    fn canonical_device_uri_is_v4_1_5_regardless_of_scope() {
        // v4.1.5 §A.URA-7: one device shape, no scope-dependent forms,
        // no `?tenant_id=` query. The legacy
        // `r/{prv,org}/reg/agent.<id>?tenant_id=<t>` shapes are dead.
        let local = resolve("silan.localhost", &cfg());
        let net = resolve("silan.easynet", &cfg());
        assert_eq!(
            canonical_device_ura("01HABC", &local),
            "easynet:///r/silan.localhost/device/01HABC"
        );
        assert_eq!(
            canonical_device_ura("01HABC", &net),
            "easynet:///r/silan.easynet/device/01HABC"
        );
    }
}
