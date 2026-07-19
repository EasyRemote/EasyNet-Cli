// EasyNet CLI — product login façade
// ==================================
//
// File: src/cli/commands/login.rs
// Description: Top-level `easynet login` / `easynet logout` product UX.
//
// Protocol Responsibility:
// - Authenticates a user account against a Realm/Hub auth provider.
// - Creates or updates a local Profile projection for that Realm/account.
// - Keeps account login outside daemon runtime abilities and the canonical SDK.
//
// Implementation Approach:
// - Parse `<login-hint>@<realm>` as shorthand; full `--user --realm` remains
//   available for email-like user identifiers and SSO seams.
// - Resolve Realm to an issuer through local/built-in/domain seams.
// - Delegate HTTP auth to the existing `auth` command provider.
//
// Usage Contract:
// - `login` establishes account session only; it does not enroll this device.
// - `logout` clears account session only; it does not remove device membership.
//
// Architectural Position:
// - CLI product layer above auth HTTP and profile projection.
// - No runtime ability, receipt, or daemon admission ownership.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>

use anyhow::anyhow;
use clap::Args;

use crate::cli::commands::{auth, profile};

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Login target shorthand: '<login-hint>@<realm>' or '<realm>'.
    pub target: Option<String>,

    /// Explicit login hint. Use when the account id itself contains '@'.
    #[arg(long)]
    pub user: Option<String>,

    /// Explicit Realm alias/domain.
    #[arg(long)]
    pub realm: Option<String>,

    /// Explicit Hub/Auth endpoint override.
    #[arg(long)]
    pub hub: Option<String>,

    /// Password. If omitted, prompt interactively for the current HTTP auth provider.
    #[arg(long)]
    pub password: Option<String>,

    /// Register the user first if the backend supports it and login fails.
    #[arg(long)]
    pub register_if_missing: bool,

    /// Nickname to use when --register-if-missing creates a new account.
    #[arg(long)]
    pub nickname: Option<String>,
}

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// Profile to log out. Defaults to EASYNET_PROFILE/current profile.
    #[arg(long)]
    pub profile: Option<String>,
}

pub(crate) struct LoginOutcome {
    pub session: auth::AuthSession,
    pub profile: profile::ProfileEntry,
}

pub fn run_login(args: LoginArgs) -> anyhow::Result<()> {
    let outcome = login_and_select_profile(args)?;
    render_login_outcome(&outcome);
    Ok(())
}

pub(crate) fn login_and_select_profile(args: LoginArgs) -> anyhow::Result<LoginOutcome> {
    let target = resolve_login_target(&args)?;
    let realm = profile::resolve_realm(&target.realm, args.hub.as_deref())?;
    let login_hint = target.login_hint.as_deref().ok_or_else(|| {
        anyhow!(
            "this auth provider requires a login hint today — use 'easynet login <user>@{}' \
             or '--user <id> --realm {}'",
            target.realm,
            target.realm
        )
    })?;

    println!("Resolving realm \"{}\"...", target.realm);
    println!("  source: {}", realm.discovery_source);
    println!("  issuer: {}", realm.issuer);
    if let Some(realm_id) = realm.realm_id.as_deref() {
        println!("  realm_id: {realm_id}");
    }

    let session = auth::login_and_save(auth::LoginArgs {
        email: login_hint.to_string(),
        password: args.password,
        hub: realm.issuer.clone(),
        register_if_missing: args.register_if_missing,
        nickname: args.nickname,
    })?;
    let entry = profile::upsert_authenticated_profile(&target, &realm, &session)?;

    Ok(LoginOutcome {
        session,
        profile: entry,
    })
}

pub(crate) fn render_login_outcome(outcome: &LoginOutcome) {
    auth::render_login_success(&outcome.session);
    println!("  profile: {}", outcome.profile.profile_name);
    println!();
    println!("Next:");
    println!("  easynet join");
    println!("  easynet status");
}

pub fn run_logout(args: LogoutArgs) -> anyhow::Result<()> {
    let selected = match profile::selected_profile(args.profile.as_deref()) {
        Ok(profile) => Some(profile),
        Err(error) if args.profile.is_some() => return Err(error),
        Err(_) => None,
    };
    if let Some(selected) = selected.as_ref() {
        if let Some(session) = auth::load_session()? {
            profile::ensure_auth_session_owns_profile(selected, &session)?;
        }
    }
    auth::clear_session()?;
    if let Some(selected) = selected {
        profile::mark_profile_session_logged_out(&selected.profile_name)?;
    }
    println!("✓ logged out");
    println!("  account session cleared; device membership was not removed");
    println!("  run 'easynet leave' to remove this device from the Realm");
    Ok(())
}

pub(crate) fn resolve_login_target(args: &LoginArgs) -> anyhow::Result<profile::LoginTarget> {
    if args.target.is_none() && args.user.is_none() && args.realm.is_none() {
        let selected = profile::selected_profile(None)?;
        return profile::LoginTarget::from_cli(
            Some(selected.realm_alias.as_str()),
            selected.login_hint.as_deref(),
            Some(selected.realm_alias.as_str()),
        );
    }
    profile::LoginTarget::from_cli(
        args.target.as_deref(),
        args.user.as_deref(),
        args.realm.as_deref(),
    )
}
