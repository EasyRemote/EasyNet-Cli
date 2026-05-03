// EasyNet CLI — Auth Group
// =========================
//
// File: src/facade/cli/groups/auth.rs
//
// `easynet auth …` — log in to / out of an EasyNet backend, mint
// device-pairing tokens, inspect the current session.
//
// Verbs:
//   login    POST /api/v1/auth/login (or register if first time)
//   logout   Drop the locally-cached token
//   whoami   Print the email + user_id + hub URL
//   pair     POST /api/v1/devices/pairing — emits a token suitable
//            for piping into `easynet device join`
//
// silan's mental model: this is to easynet what `gh auth login` is
// to GitHub or `kubectl login` is to k8s. One command, token sits
// in `~/.easynet/auth.json`, every later HTTP-aware command picks
// it up automatically.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>

use clap::{Args, Subcommand};

use crate::facade::cli::auth;

#[derive(Debug, Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub action: AuthAction,
}

#[derive(Debug, Subcommand)]
pub enum AuthAction {
    /// Log in to an EasyNet backend (or register on first use with
    /// --register-if-missing). Saves the JWT to ~/.easynet/auth.json.
    Login(auth::LoginArgs),

    /// Clear the locally-saved token.
    Logout(auth::LogoutArgs),

    /// Print the currently logged-in user's email + hub URL.
    Whoami(auth::WhoamiArgs),

    /// Mint a fresh device-pairing token. Pipe directly into
    /// `easynet device join` to attach this host as a device:
    ///
    ///   easynet auth pair --quiet | xargs easynet device join
    Pair(auth::PairArgs),
}

pub fn dispatch(args: AuthArgs) -> anyhow::Result<()> {
    match args.action {
        AuthAction::Login(a) => auth::run_login(a),
        AuthAction::Logout(a) => auth::run_logout(a),
        AuthAction::Whoami(a) => auth::run_whoami(a),
        AuthAction::Pair(a) => auth::run_pair(a),
    }
}
