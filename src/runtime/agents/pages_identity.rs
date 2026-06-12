// EasyNet CLI — registry build identity: the <user> slot
// ====================================================
//
// Split from agents/mod.rs (F-027 / T4.5); body is move-only.

/// Identity + configuration the registry build needs to mint
/// user-rooted ability families (pages, files, api_key). The
/// `<user>` slot is sourced explicitly so the registry build is
/// pure — no global env-var reads, no thread-locals, no implicit
/// dependence on credentials.json. Production callers
/// (`bin/easynet-daemon.rs`, supervisor reboots) read
/// EASYNET_PAGES_USER + credentials.json once at boot and pass
/// the resolved value here. Tests that exercise the user-rooted
/// surface pass a fixed username; unpaired-daemon tests pass
/// `None` and the family stays unregistered.
#[derive(Debug, Clone, Default)]
pub struct PagesIdentity {
    /// Username segment for `<user>.api_key.*` / `<user>.pages.*` /
    /// `<user>.files.*`. `None` means "this daemon isn't paired
    /// yet" — the user-rooted families are skipped (no
    /// `self.api_key.*` placeholder leak).
    pub user: Option<String>,
    /// Realm the user-rooted handlers stamp into URAs. Defaults
    /// to `crate::ura::REALM_EASYNET`.
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
    pub fn from_env() -> Self {
        let user = std::env::var("EASYNET_PAGES_USER")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| {
                crate::persistence::config::load_credentials()
                    .ok()
                    .and_then(|c| c.username)
                    .filter(|v| !v.is_empty())
            });
        Self {
            user,
            // Follow the daemon's actual realm before the public
            // fallback: EASYNET_PAGES_REALM override → credentials
            // realm → (None here; REALM_EASYNET applied at register).
            // Without the credentials step a daemon joined to a
            // non-default realm (e.g. `localhost`) would mint pages
            // URLs under `easynet.run`, which don't resolve to it.
            realm: std::env::var("EASYNET_PAGES_REALM")
                .ok()
                .filter(|v| !v.is_empty())
                .or_else(|| {
                    crate::persistence::config::load_credentials()
                        .ok()
                        .map(|c| c.realm)
                        .filter(|v| !v.is_empty())
                }),
            listener_port: std::env::var("EASYNET_PAGES_PORT")
                .ok()
                .and_then(|s| s.parse::<u16>().ok()),
        }
    }
}
