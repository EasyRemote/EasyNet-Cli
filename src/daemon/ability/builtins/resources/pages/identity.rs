// EasyNet CLI — Pages resource identity: the <user> slot
// ======================================================
//
// Resolved once at daemon boot and fed explicitly into registry assembly.

/// Identity + configuration the registry build needs to mint
/// user-rooted ability families (pages, files, api_key). The
/// `<user>` slot is sourced explicitly so the registry build is
/// pure — no global env-var reads, no thread-locals, no implicit
/// dependence on credentials.json. Production callers
/// (`bin/easynet-daemon.rs`, supervisor reboots) read
/// EASYNET_PAGES_USER + credentials.json once at boot and pass
/// the resolved value here. Missing credentials is the unpaired
/// daemon state; malformed credentials is unavailable boot
/// identity state and fails before registry assembly. Tests that
/// exercise the user-rooted surface pass a fixed username;
/// unpaired-daemon tests pass `None` and the family stays
/// unregistered.
#[derive(Debug, Clone, Default)]
pub struct PagesIdentity {
    /// Username segment for `<user>.api_key.*` / `<user>.pages.*` /
    /// `<user>.files.*`. `None` means "this daemon isn't paired
    /// yet" — the user-rooted families are skipped (no
    /// `self.api_key.*` placeholder leak).
    pub user: Option<String>,
    /// Realm the user-rooted handlers stamp into URAs. `None` is valid only
    /// when `user` is also `None` (unpaired daemon). Paired user-rooted
    /// surfaces must carry an explicit realm; callers must not fabricate the
    /// product default at registration time.
    pub realm: Option<String>,
    /// HTTP listener port for the pages server. `None` falls back
    /// to the historical 8787.
    pub listener_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagesUserRootIdentity {
    pub user: String,
    pub realm: String,
}

impl PagesIdentity {
    pub fn user_root_identity(&self) -> anyhow::Result<Option<PagesUserRootIdentity>> {
        let user = self
            .user
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let realm = self
            .realm
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        match (user, realm) {
            (None, _) => Ok(None),
            (Some(_), None) => {
                anyhow::bail!("PagesIdentity user-root identity requires an explicit realm")
            }
            (Some(user), Some(realm)) => Ok(Some(PagesUserRootIdentity {
                user: user.to_string(),
                realm: realm.to_string(),
            })),
        }
    }

    /// Resolve from the boot-time env vars. Read ONCE at process
    /// startup; downstream callers receive the resolved struct.
    /// Tests should not call this — they construct
    /// `PagesIdentity` directly so the registry shape is
    /// deterministic regardless of process env state.
    pub fn try_from_env() -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials_optional()?;
        Ok(Self {
            user: pages_user_from_env_or_credentials(credentials.as_ref())?,
            // Follow the daemon's actual realm before the public
            // default: EASYNET_PAGES_REALM override → credentials
            // realm → None for unpaired state.
            // Without the credentials step a daemon joined to a
            // non-default realm (e.g. `localhost`) would mint pages
            // URLs under `easynet.run`, which don't resolve to it.
            realm: pages_realm_from_env_or_credentials(credentials.as_ref()),
            listener_port: pages_listener_port_from_env()?,
        })
    }
}

pub(crate) fn pages_user_from_env_or_credentials(
    credentials: Option<&crate::daemon::persistence::config::Credentials>,
) -> anyhow::Result<Option<String>> {
    if let Some(user) = non_blank_env("EASYNET_PAGES_USER") {
        return Ok(Some(user));
    }
    let Some(credentials) = credentials else {
        return Ok(None);
    };
    if credentials.username.is_none() && credentials.join_receipt_hash().is_some() {
        return Ok(None);
    }
    credentials
        .username_slug()
        .map(|username| Some(username.to_string()))
}

fn pages_realm_from_env_or_credentials(
    credentials: Option<&crate::daemon::persistence::config::Credentials>,
) -> Option<String> {
    non_blank_env("EASYNET_PAGES_REALM").or_else(|| {
        credentials
            .map(crate::daemon::persistence::config::Credentials::realm_str)
            .map(str::trim)
            .filter(|realm| !realm.is_empty())
            .map(str::to_string)
    })
}

fn non_blank_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn pages_listener_port_from_env() -> anyhow::Result<Option<u16>> {
    let raw = match std::env::var("EASYNET_PAGES_PORT") {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("EASYNET_PAGES_PORT must be valid UTF-8")
        }
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let port = trimmed
        .parse::<u16>()
        .map_err(|err| anyhow::anyhow!("EASYNET_PAGES_PORT must be a valid TCP port: {err}"))?;
    if port == 0 {
        anyhow::bail!("EASYNET_PAGES_PORT must be greater than 0");
    }
    Ok(Some(port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::persistence::config::{self, Credentials};

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }

        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.as_ref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn credentials(username: &str, realm: &str) -> Credentials {
        Credentials {
            node_id: "device-1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "axon://hub.example:7700".to_string(),
            realm: realm.to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some(username.to_string()),
            user_id: Some("user-alice".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    fn federation_native_credentials(username: Option<&str>) -> Credentials {
        Credentials {
            node_id: "device-1".to_string(),
            credential_token: String::new(),
            hub_endpoint: "https://hub.example:50443".to_string(),
            realm: "localhost".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: username.map(str::to_string),
            user_id: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("a".repeat(64)),
        }
    }

    #[test]
    fn pages_identity_missing_credentials_is_unpaired_state() {
        let _home = HomeGuard::new();
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let _port = EnvGuard::remove("EASYNET_PAGES_PORT");

        let identity = PagesIdentity::try_from_env().expect("missing credentials is unpaired");

        assert_eq!(identity.user, None);
        assert_eq!(identity.realm, None);
        assert_eq!(identity.listener_port, None);
    }

    #[test]
    fn pages_identity_trims_env_user_and_realm_overrides() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::set("EASYNET_PAGES_USER", " alice ");
        let _realm = EnvGuard::set("EASYNET_PAGES_REALM", " localhost ");
        let _port = EnvGuard::remove("EASYNET_PAGES_PORT");

        let identity = PagesIdentity::try_from_env().expect("env identity");

        assert_eq!(identity.user.as_deref(), Some("alice"));
        assert_eq!(identity.realm.as_deref(), Some("localhost"));
    }

    #[test]
    fn pages_identity_reads_credentials_when_present() {
        let _home = HomeGuard::new();
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let _port = EnvGuard::remove("EASYNET_PAGES_PORT");
        config::save_credentials(&credentials("alice", "localhost")).expect("save credentials");

        let identity = PagesIdentity::try_from_env().expect("credentials identity");

        assert_eq!(identity.user.as_deref(), Some("alice"));
        assert_eq!(identity.realm.as_deref(), Some("localhost"));
    }

    #[test]
    fn pages_identity_projects_federation_native_credentials_as_device_only() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::remove("EASYNET_PAGES_USER");
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let _port = EnvGuard::remove("EASYNET_PAGES_PORT");
        config::save_credentials(&federation_native_credentials(None))
            .expect("save federation-native credentials");

        let identity = PagesIdentity::try_from_env().expect("device-only identity");

        assert_eq!(identity.user, None);
        assert_eq!(identity.realm.as_deref(), Some("localhost"));
        assert!(
            identity
                .user_root_identity()
                .expect("device-only projection")
                .is_none(),
            "device-only credentials must not register user-rooted Pages surfaces"
        );
    }

    #[test]
    fn pages_identity_rejects_blank_credential_username_instead_of_defaulting() {
        let _home = HomeGuard::new();
        let _user = EnvGuard::remove("EASYNET_PAGES_USER");
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let _port = EnvGuard::remove("EASYNET_PAGES_PORT");
        config::save_credentials(&federation_native_credentials(Some("   ")))
            .expect("save blank federation-native username");

        let error = PagesIdentity::try_from_env()
            .expect_err("blank credential username must fail boot identity projection");

        assert!(
            error.to_string().contains("missing username"),
            "error should surface the invalid username fact: {error}"
        );
    }

    #[test]
    fn pages_identity_user_root_projection_requires_realm() {
        let identity = PagesIdentity {
            user: Some("alice".to_string()),
            realm: None,
            listener_port: None,
        };

        let error = identity
            .user_root_identity()
            .expect_err("paired user-root identity must not invent a default realm");

        assert!(
            error.to_string().contains("explicit realm"),
            "error should require explicit realm: {error}"
        );
    }

    #[test]
    fn pages_identity_user_root_projection_accepts_complete_identity() {
        let identity = PagesIdentity {
            user: Some("alice".to_string()),
            realm: Some("localhost".to_string()),
            listener_port: None,
        };

        let projected = identity
            .user_root_identity()
            .expect("complete identity")
            .expect("paired identity");

        assert_eq!(projected.user, "alice");
        assert_eq!(projected.realm, "localhost");
    }

    #[test]
    fn pages_identity_rejects_malformed_credentials_instead_of_defaulting() {
        let _home = HomeGuard::new();
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let _port = EnvGuard::remove("EASYNET_PAGES_PORT");
        std::fs::create_dir_all(config::state_dir()).expect("create state dir");
        std::fs::write(config::state_dir().join("credentials.json"), b"{")
            .expect("write malformed credentials");

        let error = PagesIdentity::try_from_env().expect_err("malformed credentials fail closed");

        assert!(
            error.to_string().contains("parse credentials"),
            "error should surface credential parse failure: {error}"
        );
    }

    #[test]
    fn pages_identity_rejects_invalid_port_instead_of_defaulting() {
        let _home = HomeGuard::new();
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let _port = EnvGuard::set("EASYNET_PAGES_PORT", "not-a-port");

        let error = PagesIdentity::try_from_env().expect_err("invalid port fails closed");

        assert!(
            error.to_string().contains("EASYNET_PAGES_PORT"),
            "error should name invalid port env: {error}"
        );
    }

    #[test]
    fn pages_identity_rejects_zero_port_instead_of_defaulting() {
        let _home = HomeGuard::new();
        let _realm = EnvGuard::remove("EASYNET_PAGES_REALM");
        let _port = EnvGuard::set("EASYNET_PAGES_PORT", "0");

        let error = PagesIdentity::try_from_env().expect_err("zero port fails closed");

        assert!(
            error.to_string().contains("greater than 0"),
            "error should reject zero port: {error}"
        );
    }
}
