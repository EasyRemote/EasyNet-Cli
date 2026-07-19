// EasyNet CLI — profile lifecycle
// ===============================
//
// File: src/cli/commands/profile.rs
// Description: Product-layer Realm/account profile projection for the
//              user-facing `login`, `join`, and `profile` commands.
//
// Protocol Responsibility:
// - Keeps product account/session selection outside the canonical runtime SDK.
// - Models Profile as Realm + Account selection state, not as a Hub endpoint.
// - Stores non-secret profile projection only; auth tokens remain in the
//   existing owner-only auth session file.
//
// Implementation Approach:
// - `profiles.json` owns local profile projection and current-profile choice.
// - Bare Realm aliases resolve only from known local/built-in sources unless
//   the operator passes an explicit `--hub` override.
// - Existing auth/device join pipelines remain the underlying providers.
//
// Usage Contract:
// - `easynet profile use <name>` selects the default profile.
// - `EASYNET_PROFILE=<name>` overrides the current profile for scripts.
// - `profile remove` is local-only and does not revoke remote membership.
//
// Architectural Position:
// - CLI product façade above auth HTTP/session and device join providers.
// - No daemon admission, SDK receipt, or runtime Principal lifecycle changes.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use crate::cli::commands::auth::AuthSession;
use crate::daemon::persistence::config::{self, atomic_write_with_permissions, WritePermissions};
use crate::support::platform::output;

const OFFICIAL_REALM_ALIAS: &str = "official";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoginTarget {
    pub login_hint: Option<String>,
    pub realm: String,
}

impl LoginTarget {
    pub(crate) fn from_cli(
        target: Option<&str>,
        user: Option<&str>,
        realm: Option<&str>,
    ) -> anyhow::Result<Self> {
        let user = user.map(str::trim).filter(|value| !value.is_empty());
        let realm = realm.map(str::trim).filter(|value| !value.is_empty());

        if let Some(realm) = realm {
            return Ok(Self {
                login_hint: user.map(ToOwned::to_owned),
                realm: realm.to_string(),
            });
        }

        let target = target
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!("missing login target — use '<user>@<realm>' or '--realm <realm>'")
            })?;

        if let Some((login_hint, realm)) = target.rsplit_once('@') {
            let login_hint = login_hint.trim();
            let realm = realm.trim();
            if login_hint.is_empty() || realm.is_empty() {
                bail!("invalid login target '{target}' — expected '<user>@<realm>'");
            }
            return Ok(Self {
                login_hint: Some(login_hint.to_string()),
                realm: realm.to_string(),
            });
        }

        Ok(Self {
            login_hint: user.map(ToOwned::to_owned),
            realm: target.to_string(),
        })
    }

    pub(crate) fn profile_name_for_session(&self, session: &AuthSession) -> String {
        let user = self
            .login_hint
            .as_deref()
            .or(session.username.as_deref())
            .unwrap_or(session.email.as_str())
            .trim();
        format!("{user}@{}", self.realm)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProfileAccountSessionState {
    Authenticated,
    LoggedOut,
}

impl Default for ProfileAccountSessionState {
    fn default() -> Self {
        Self::LoggedOut
    }
}

impl ProfileAccountSessionState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Authenticated => "authenticated",
            Self::LoggedOut => "logged_out",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileEntry {
    pub profile_name: String,
    pub realm_alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_id: Option<String>,
    pub issuer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_anchor: Option<String>,
    #[serde(default)]
    pub account_session: ProfileAccountSessionState,
    #[serde(default)]
    pub device_membership: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileStore {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedRealm {
    pub realm_alias: String,
    pub realm_id: Option<String>,
    pub issuer: String,
    pub discovery_source: String,
}

pub(crate) fn profile_store_path() -> PathBuf {
    config::state_dir().join("profiles.json")
}

pub(crate) fn load_store() -> anyhow::Result<ProfileStore> {
    let path = profile_store_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProfileStore::default());
        }
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

pub(crate) fn save_store(store: &ProfileStore) -> anyhow::Result<()> {
    let dir = config::state_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let json = serde_json::to_string_pretty(store)? + "\n";
    atomic_write_with_permissions(
        &profile_store_path(),
        json.as_bytes(),
        WritePermissions::OwnerReadWrite,
    )?;
    Ok(())
}

pub(crate) fn resolve_realm(
    realm: &str,
    hub_override: Option<&str>,
) -> anyhow::Result<ResolvedRealm> {
    let realm = realm.trim();
    if realm.is_empty() {
        bail!("realm is empty");
    }
    if let Some(hub) = hub_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(ResolvedRealm {
            realm_alias: realm.to_string(),
            realm_id: None,
            issuer: hub.trim_end_matches('/').to_string(),
            discovery_source: "explicit --hub override".to_string(),
        });
    }

    let store = load_store()?;
    if let Some(profile) = store
        .profiles
        .values()
        .find(|profile| profile.realm_alias == realm || profile.profile_name == realm)
    {
        return Ok(ResolvedRealm {
            realm_alias: profile.realm_alias.clone(),
            realm_id: profile.realm_id.clone(),
            issuer: profile.issuer.clone(),
            discovery_source: "local profile".to_string(),
        });
    }

    if realm == OFFICIAL_REALM_ALIAS {
        return Ok(ResolvedRealm {
            realm_alias: OFFICIAL_REALM_ALIAS.to_string(),
            realm_id: Some("reserved:easynet:realm:official".to_string()),
            issuer: format!("https://{}", config::DEFAULT_HUB_HOST),
            discovery_source: "built-in reserved alias".to_string(),
        });
    }

    if looks_like_domain(realm) {
        return Ok(ResolvedRealm {
            realm_alias: realm.to_string(),
            realm_id: None,
            issuer: format!("https://{realm}"),
            discovery_source: "domain discovery seam".to_string(),
        });
    }

    bail!(
        "realm alias '{realm}' is not configured; use 'easynet login <user>@{realm} --hub <url>' \
         or configure an enterprise Realm directory before using bare aliases"
    )
}

fn looks_like_domain(value: &str) -> bool {
    value.contains('.') || value.starts_with("http://") || value.starts_with("https://")
}

pub(crate) fn upsert_authenticated_profile(
    target: &LoginTarget,
    realm: &ResolvedRealm,
    session: &AuthSession,
) -> anyhow::Result<ProfileEntry> {
    let mut store = load_store()?;
    let profile_name = target.profile_name_for_session(session);
    let entry = ProfileEntry {
        profile_name: profile_name.clone(),
        realm_alias: realm.realm_alias.clone(),
        realm_id: realm.realm_id.clone(),
        issuer: realm.issuer.clone(),
        login_hint: target.login_hint.clone(),
        subject: session.user_id.clone(),
        credential_ref: Some(format!(
            "local-file://{}",
            crate::cli::commands::auth::auth_session_path().display()
        )),
        trust_anchor: None,
        account_session: ProfileAccountSessionState::Authenticated,
        device_membership: device_membership_state(&realm.realm_alias),
    };
    store.profiles.insert(profile_name.clone(), entry.clone());
    store.current_profile = Some(profile_name);
    save_store(&store)?;
    Ok(entry)
}

pub(crate) fn selected_profile(explicit: Option<&str>) -> anyhow::Result<ProfileEntry> {
    let store = load_store()?;
    let selected = explicit
        .map(str::to_string)
        .or_else(|| std::env::var("EASYNET_PROFILE").ok())
        .or(store.current_profile.clone())
        .ok_or_else(|| anyhow!("no current profile — run 'easynet login <user>@<realm>' first"))?;
    store
        .profiles
        .get(&selected)
        .cloned()
        .ok_or_else(|| anyhow!("profile '{selected}' not found"))
}

pub(crate) fn ensure_auth_session_owns_profile(
    profile: &ProfileEntry,
    session: &AuthSession,
) -> anyhow::Result<()> {
    if normalize_issuer(&session.hub_url) != normalize_issuer(&profile.issuer) {
        bail!(
            "active auth session issuer {} does not match profile '{}' issuer {}; run 'easynet login {}'",
            session.hub_url,
            profile.profile_name,
            profile.issuer,
            profile.profile_name
        );
    }

    if let Some(profile_subject) = profile.subject.as_deref().and_then(non_empty) {
        let session_subject = session.user_id.as_deref().and_then(non_empty).ok_or_else(|| {
            anyhow!(
                "active auth session has no authenticated subject for profile '{}'; run 'easynet login {}'",
                profile.profile_name,
                profile.profile_name
            )
        })?;
        if session_subject != profile_subject {
            bail!(
                "active auth session subject {session_subject} does not match profile '{}' subject {profile_subject}; run 'easynet login {}'",
                profile.profile_name,
                profile.profile_name
            );
        }
        return Ok(());
    }

    let Some(profile_login_hint) = profile.login_hint.as_deref().and_then(non_empty) else {
        bail!(
            "profile '{}' has no account owner projection; run 'easynet login {}'",
            profile.profile_name,
            profile.profile_name
        );
    };
    if session_identity_candidates(session).any(|candidate| candidate == profile_login_hint) {
        return Ok(());
    }

    bail!(
        "active auth session account {} does not match profile '{}' login hint {}; run 'easynet login {}'",
        session.email,
        profile.profile_name,
        profile_login_hint,
        profile.profile_name
    );
}

fn normalize_issuer(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn session_identity_candidates(session: &AuthSession) -> impl Iterator<Item = &str> {
    std::iter::once(session.email.as_str())
        .chain(session.username.as_deref())
        .filter_map(non_empty)
}

pub(crate) fn mark_profile_session_logged_out(profile_name: &str) -> anyhow::Result<()> {
    let mut store = load_store()?;
    if let Some(profile) = store.profiles.get_mut(profile_name) {
        profile.account_session = ProfileAccountSessionState::LoggedOut;
    }
    save_store(&store)
}

pub(crate) fn mark_device_membership(profile_name: &str, state: &str) -> anyhow::Result<()> {
    let mut store = load_store()?;
    if let Some(profile) = store.profiles.get_mut(profile_name) {
        profile.device_membership = state.to_string();
        save_store(&store)?;
    }
    Ok(())
}

fn device_membership_state(realm_alias: &str) -> String {
    match config::load_credentials() {
        Ok(credentials) if credentials.realm_str() == realm_alias => "enrolled".to_string(),
        _ => "not_enrolled".to_string(),
    }
}

#[derive(Debug, Args)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub action: ProfileAction,
}

#[derive(Debug, Subcommand)]
pub enum ProfileAction {
    /// List local Realm/account profiles.
    List,
    /// Select the current profile.
    Use(ProfileUseArgs),
    /// Show one profile, or the current profile when omitted.
    Show(ProfileShowArgs),
    /// Remove a local profile projection; does not revoke remote membership.
    Remove(ProfileRemoveArgs),
}

#[derive(Debug, Args)]
pub struct ProfileUseArgs {
    pub profile: String,
}

#[derive(Debug, Args)]
pub struct ProfileShowArgs {
    pub profile: Option<String>,
}

#[derive(Debug, Args)]
pub struct ProfileRemoveArgs {
    pub profile: String,
}

pub fn run(args: ProfileArgs) -> anyhow::Result<()> {
    match args.action {
        ProfileAction::List => run_list(),
        ProfileAction::Use(args) => run_use(args),
        ProfileAction::Show(args) => run_show(args),
        ProfileAction::Remove(args) => run_remove(args),
    }
}

fn run_list() -> anyhow::Result<()> {
    let store = load_store()?;
    if store.profiles.is_empty() {
        println!("(no profiles — run 'easynet login <user>@<realm>' first)");
        return Ok(());
    }
    println!(
        "{:<3} {:<32} {:<18} {:<14} {}",
        "", "PROFILE", "REALM", "SESSION", "ISSUER"
    );
    for (name, profile) in store.profiles.iter() {
        let marker = if store.current_profile.as_deref() == Some(name.as_str()) {
            "*"
        } else {
            ""
        };
        println!(
            "{:<3} {:<32} {:<18} {:<14} {}",
            marker,
            name,
            profile.realm_alias,
            profile.account_session.as_str(),
            profile.issuer
        );
    }
    Ok(())
}

fn run_use(args: ProfileUseArgs) -> anyhow::Result<()> {
    let mut store = load_store()?;
    if !store.profiles.contains_key(&args.profile) {
        bail!("profile '{}' not found", args.profile);
    }
    store.current_profile = Some(args.profile.clone());
    save_store(&store)?;
    println!("✓ current profile: {}", args.profile);
    Ok(())
}

fn run_show(args: ProfileShowArgs) -> anyhow::Result<()> {
    let profile = selected_profile(args.profile.as_deref())?;
    output::kv_section_stdout(&[
        ("profile", profile.profile_name.as_str()),
        ("realm", profile.realm_alias.as_str()),
        ("issuer", profile.issuer.as_str()),
        ("account_session", profile.account_session.as_str()),
        ("device_membership", profile.device_membership.as_str()),
    ]);
    if let Some(realm_id) = profile.realm_id.as_deref() {
        output::kv_section_stdout(&[("realm_id", realm_id)]);
    }
    if let Some(subject) = profile.subject.as_deref() {
        output::kv_section_stdout(&[("subject", subject)]);
    }
    Ok(())
}

fn run_remove(args: ProfileRemoveArgs) -> anyhow::Result<()> {
    let mut store = load_store()?;
    if store.profiles.remove(&args.profile).is_none() {
        bail!("profile '{}' not found", args.profile);
    }
    if store.current_profile.as_deref() == Some(args.profile.as_str()) {
        store.current_profile = None;
    }
    save_store(&store)?;
    println!("✓ removed local profile {}", args.profile);
    println!("  remote account session or device membership was not revoked");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_at_realm_as_login_hint() {
        let target = LoginTarget::from_cli(Some("silan@acme"), None, None).unwrap();
        assert_eq!(target.login_hint.as_deref(), Some("silan"));
        assert_eq!(target.realm, "acme");
    }

    #[test]
    fn parses_full_user_and_realm_without_splitting_email() {
        let target =
            LoginTarget::from_cli(None, Some("silan.hu@company.com"), Some("acme")).unwrap();
        assert_eq!(target.login_hint.as_deref(), Some("silan.hu@company.com"));
        assert_eq!(target.realm, "acme");
    }
}
