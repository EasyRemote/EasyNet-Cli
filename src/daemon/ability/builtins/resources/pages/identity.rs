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
    /// Realm the user-rooted handlers stamp into URAs. Defaults
    /// to `crate::core::ura::REALM_EASYNET`.
    pub realm: Option<String>,
    /// HTTP listener port for the pages server. `None` falls back
    /// to the historical 8787.
    pub listener_port: Option<u16>,
}

impl PagesIdentity {
    /// Resolve from the boot-time env vars. Read ONCE at process
    /// startup; downstream callers receive the resolved struct.
    /// Tests should not call this — they construct
    /// `PagesIdentity` directly so the registry shape is
    /// deterministic regardless of process env state.
    pub fn try_from_env() -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials_optional()?;
        let user = std::env::var("EASYNET_PAGES_USER")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| credentials.as_ref().and_then(|c| c.username.clone()))
            .filter(|v| !v.is_empty());
        Ok(Self {
            user,
            // Follow the daemon's actual realm before the public
            // default: EASYNET_PAGES_REALM override → credentials
            // realm → (None here; REALM_EASYNET applied at register).
            // Without the credentials step a daemon joined to a
            // non-default realm (e.g. `localhost`) would mint pages
            // URLs under `easynet.run`, which don't resolve to it.
            realm: std::env::var("EASYNET_PAGES_REALM")
                .ok()
                .filter(|v| !v.is_empty())
                .or_else(|| credentials.as_ref().map(|c| c.realm.clone()))
                .filter(|v| !v.is_empty()),
            listener_port: pages_listener_port_from_env()?,
        })
    }
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
