// EasyNet CLI — top-level `--help` banner
// ========================================
//
// File: src/cli/banner.rs
// Description: Renders the ASCII title + creator blessing + live
//              runtime status block printed above clap's `--help`
//              when the user types `easynet`, `easynet --help`,
//              `easynet -h`, or `easynet help`. Subcommand `--help`s
//              do NOT show the banner — it would be noise.
//
// Design constraints
// ------------------
// 1. Side-effect-free: only local lifecycle/config file reads and process
//    probes through `RuntimeLifecycleService`. No network I/O — `--help` must
//    stay fast even on an offline laptop.
// 2. Restrained palette. Three roles only — `accent` (cyan, used by
//    clap too so the whole `--help` reads as one document), `dim`
//    (grey supporting text), and a *single* status colour per status
//    (green for healthy, yellow for warn, plain for unconfigured).
//    No 256-colour gradients, no colour for emphasis-by-volume.
// 3. Two-column layout. Every label is left-padded to `LABEL_WIDTH`
//    so values line up under one another regardless of label
//    length. Long values are truncated, never wrapped, so the
//    banner never grows past 4 lines of status.
// 4. Honour `NO_COLOR` / `CLICOLOR_FORCE` / TTY detection just like
//    anstream does, since we render before clap takes over stdout.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::IsTerminal;

use crate::cli::presentation::identity::{
    runtime_user_binding_display, RuntimeUserBindingDisplayState,
};
use crate::core::ura;
use crate::daemon::lifecycle::{
    RuntimeLifecycleError, RuntimeLifecycleService, RuntimeLifecycleStatus, RuntimeStatusReport,
};
use crate::daemon::persistence::config;

/// Width of the status-block label column ("Daemon:", "Hub:",
/// "Current device:"). Picked so the longest label fits with one
/// trailing space before the value column. Every shorter label
/// gets padded to match so the value column lines up vertically.
const LABEL_WIDTH: usize = 16;

/// Outer left margin for the whole banner. Empty — logo, tagline,
/// signature, and status rows all sit flush at column 0 so the
/// banner reads as a header rather than as a child of the grouped
/// command list clap renders below.
const MARGIN: &str = "";

/// Top-level "decoration" block printed before clap's `--help` text.
/// Returns a single string ready to write to stdout (no trailing
/// newline beyond what's in the banner — caller adds spacing).
///
/// Layout (top to bottom):
///   1. ASCII wordmark   — six-line block letters (single bold cyan)
///   2. Tagline line     — verbatim homepage copy
///   3. Signature        — creator credit
///   4. Status rows      — Daemon / Hub / Peers
pub fn render_top_level_banner() -> String {
    let style = ColourMode::detect();
    let mut buf = String::new();
    write_logo(&mut buf, style);
    write_tagline(&mut buf, style);
    write_runtime_status(&mut buf, style);
    buf.push('\n');
    buf
}

// ── ANSI colour control ──────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColourMode {
    On,
    Off,
}

impl ColourMode {
    /// Standard precedence: `NO_COLOR` (any value) wins absolute.
    /// Then `CLICOLOR_FORCE` forces on. Otherwise on iff stdout is
    /// a TTY. Same logic anstream uses.
    fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::Off;
        }
        if std::env::var_os("CLICOLOR_FORCE")
            .map(|v| v != "0")
            .unwrap_or(false)
        {
            return Self::On;
        }
        if std::io::stdout().is_terminal() {
            Self::On
        } else {
            Self::Off
        }
    }

    /// Wrap `text` in `style` only when colour is on. `style` is the
    /// SGR parameter list (e.g. `"1;36"` for bold cyan).
    fn paint(self, style: &str, text: &str) -> String {
        match self {
            Self::On => format!("\x1b[{style}m{text}\x1b[0m"),
            Self::Off => text.to_string(),
        }
    }
}

/// The whole banner uses three roles. Anything outside these three
/// is plain text. This keeps the palette consistent with clap's
/// own `--help` colouring (cyan headers, default body).
mod sgr {
    /// Bold cyan — primary accent. Logo, labels, anything we want
    /// the eye to lock onto. Same shade clap's own `Usage:` /
    /// `Commands:` headers use, so the banner and the help block
    /// share one accent colour.
    pub const ACCENT: &str = "1;36";
    /// Dim default — secondary text (signature, hint phrases,
    /// truncated detail). One step below body weight so it visually
    /// recedes.
    pub const DIM: &str = "2";
    /// Bold green — healthy / running / OK. Used for the daemon
    /// liveness dot and nothing else.
    pub const OK: &str = "1;32";
    /// Bold yellow — warn / inconsistent state. Used when daemon
    /// metadata exists but the process is not responding.
    pub const WARN: &str = "1;33";
}

// ── Logo ─────────────────────────────────────────────────────────────

/// Standalone logo render — six-line ASCII wordmark only, no
/// tagline / signature / status rows. Used by `easynet runtime start`
/// and `easynet device join` to brand their command output without
/// dragging in the navigation furniture that belongs above `--help`.
pub fn render_logo() -> String {
    let mut buf = String::new();
    write_logo(&mut buf, ColourMode::detect());
    buf
}

/// Six-line ASCII wordmark, painted in one shade of bold cyan.
/// silan's spec, character-for-character. The lines are 2-space
/// indented to match `MARGIN`, lining up with the rest of the banner
/// and with the grouped command rows clap renders below.
///
/// Why a single colour: a CLI banner is a navigation surface, not
/// fireworks. The earlier 256-colour gradient pulled the eye away
/// from the status rows underneath, which is where the live
/// information actually lives.
fn write_logo(buf: &mut String, style: ColourMode) {
    const LOGO_LINES: [&str; 6] = [
        "███████╗ █████╗ ███████╗██╗   ██╗███╗   ██╗███████╗████████╗",
        "██╔════╝██╔══██╗██╔════╝╚██╗ ██╔╝████╗  ██║██╔════╝╚══██╔══╝",
        "█████╗  ███████║███████╗ ╚████╔╝ ██╔██╗ ██║█████╗     ██║   ",
        "██╔══╝  ██╔══██║╚════██║  ╚██╔╝  ██║╚██╗██║██╔══╝     ██║   ",
        "███████╗██║  ██║███████║   ██║   ██║ ╚████║███████╗   ██║   ",
        "╚══════╝╚═╝  ╚═╝╚══════╝   ╚═╝   ╚═╝  ╚═══╝╚══════╝   ╚═╝   ",
    ];
    for line in LOGO_LINES {
        buf.push_str(MARGIN);
        buf.push_str(&style.paint(sgr::ACCENT, line));
        buf.push('\n');
    }
    buf.push('\n');
}

// ── Tagline ──────────────────────────────────────────────────────────

/// Wordmark + tagline + signature. Three lines, each at the shared
/// `MARGIN` indent so the whole banner reads as one block.
///
/// Source-of-truth: the tagline is the *verbatim* homepage copy
/// from <https://easynet.run> ("The Internet for Agents and
/// Devices"). It is not a phrase we make up here. If marketing
/// changes the homepage tagline, change this line — do not improvise
/// alternatives.
fn write_tagline(buf: &mut String, style: ColourMode) {
    const WORDMARK: &str = "EasyNet";
    const TAGLINE: &str = "The Internet for Agents and Devices";
    // ASCII `--` rather than U+2014 EM DASH so the banner stays
    // pure ASCII and renders identically across every locale /
    // terminal encoding. The CLI is not the place to depend on
    // Unicode-aware fonts.
    const SIGNATURE: &str = "-- Silan Hu, creator of EasyNet";

    buf.push_str(MARGIN);
    buf.push_str(&style.paint(sgr::ACCENT, WORDMARK));
    buf.push_str("  ");
    buf.push_str(&style.paint(sgr::DIM, TAGLINE));
    buf.push('\n');
    buf.push_str(MARGIN);
    buf.push_str(&style.paint(sgr::DIM, SIGNATURE));
    buf.push_str("\n\n");
}

// ── Runtime status ───────────────────────────────────────────────────

/// Live block: is the daemon up, what hub did it pair with, how many
/// federation peers are configured. Two-column layout — labels are
/// padded to `LABEL_WIDTH`, values follow. ≤ 3 lines.
fn write_runtime_status(buf: &mut String, style: ColourMode) {
    let lifecycle = RuntimeLifecycleService::new().status();
    let creds = BannerCredentialsObservation::load();

    // Row 1 — daemon liveness.
    let daemon_observation = BannerDaemonObservation::from_lifecycle_result(&lifecycle);
    write_row(
        buf,
        style,
        "Daemon:",
        &format!(
            "{} {}",
            style.paint(daemon_observation.dot_sgr, daemon_observation.dot),
            style.paint(
                daemon_observation.text_sgr,
                daemon_observation.message.as_str()
            ),
        ),
    );

    // Rows 2/3/4 — three URA rows, ordered broadest scope to
    // narrowest per RFC-001 §3.2 (hub > user > device).
    //
    //   Hub:            hub URA           — realm singleton
    //   Current user:   user URA          — owner of this device
    //   Current device: device URA        — this machine
    //
    // All three are first-class agents in the ontology, so all
    // three carry equal visual weight. The user URA is derived
    // from credentials.json's immutable user binding; when federation-native
    // credentials are intentionally device-only we render that state explicitly
    // instead of suppressing the row as a compatibility fallback.
    //
    // The transport URL (creds.hub_endpoint) is intentionally NOT
    // shown here — URA is the ontology-canonical identity for a
    // hub / user / device per RFC-001 §3.2. URLs are an
    // implementation detail no other surface uses to refer to
    // these identities.
    //
    // URA strings render in `DIM` (light grey) rather than
    // `ACCENT`. They are reference data — the eye should land on
    // labels and status dots first, URA values are there to be
    // copied / read when needed, not foregrounded.
    match &creds {
        BannerCredentialsObservation::Paired(c) => {
            let realm = c.realm_str();
            let hub_ura = ura::hub_ura(realm);
            let device_ura = ura::device_ura(realm, &c.node_id);
            write_row(buf, style, "Hub:", &style.paint(sgr::DIM, &hub_ura));
            let user_binding = runtime_user_binding_display(c);
            let user_sgr = match user_binding.state() {
                RuntimeUserBindingDisplayState::Bound | RuntimeUserBindingDisplayState::Unbound => {
                    sgr::DIM
                }
                RuntimeUserBindingDisplayState::Invalid => sgr::WARN,
            };
            write_row(
                buf,
                style,
                "Current user:",
                &style.paint(user_sgr, user_binding.value()),
            );
            write_row(
                buf,
                style,
                "Current device:",
                &style.paint(sgr::DIM, &device_ura),
            );
        }
        BannerCredentialsObservation::Unpaired => {
            write_row(
                buf,
                style,
                "Hub:",
                &style.paint(sgr::DIM, "not paired  ·  run 'easynet device join <token>'"),
            );
        }
        BannerCredentialsObservation::Invalid(error) => {
            write_row(
                buf,
                style,
                "Hub:",
                &style.paint(sgr::WARN, &format!("credentials invalid  ·  {error}")),
            );
        }
    }

    // Row 3 — peer counts. Suppressed entirely when there are no
    // peers configured: an empty row is more noise than information
    // for the common single-hub install.
    let trusted = read_trusted_hub_count();
    let federated = read_federated_peer_count();
    if trusted + federated > 0 {
        let body = format!(
            "{} trusted · {} federated  {}",
            trusted,
            federated,
            style.paint(sgr::DIM, "(see `easynet federation peers`)"),
        );
        write_row(buf, style, "Peers:", &body);
    }
}

#[derive(Debug)]
enum BannerCredentialsObservation {
    Paired(config::Credentials),
    Unpaired,
    Invalid(String),
}

impl BannerCredentialsObservation {
    fn load() -> Self {
        match config::load_credentials_optional() {
            Ok(Some(credentials)) => Self::Paired(credentials),
            Ok(None) => Self::Unpaired,
            Err(error) => Self::Invalid(error.to_string()),
        }
    }
}

struct BannerDaemonObservation {
    dot_sgr: &'static str,
    text_sgr: &'static str,
    dot: &'static str,
    message: String,
}

impl BannerDaemonObservation {
    fn from_lifecycle_result(
        lifecycle: &Result<RuntimeStatusReport, RuntimeLifecycleError>,
    ) -> Self {
        match lifecycle {
            Ok(report) => Self::from_lifecycle_status(report.status()),
            Err(error) => Self {
                dot_sgr: sgr::WARN,
                text_sgr: sgr::DIM,
                dot: "●",
                message: format!("metadata unavailable  ·  {error}"),
            },
        }
    }

    fn from_lifecycle_status(status: RuntimeLifecycleStatus) -> Self {
        match status {
            RuntimeLifecycleStatus::Running => Self {
                dot_sgr: sgr::OK,
                text_sgr: sgr::ACCENT,
                dot: "●",
                message: "running".to_string(),
            },
            RuntimeLifecycleStatus::Stopped => Self {
                dot_sgr: sgr::DIM,
                text_sgr: sgr::DIM,
                dot: "○",
                message: "not running  ·  start with 'easynet runtime start'".to_string(),
            },
            RuntimeLifecycleStatus::ProjectionPresentProcessMissing => Self {
                dot_sgr: sgr::WARN,
                text_sgr: sgr::DIM,
                dot: "●",
                message: "metadata present but process not responding".to_string(),
            },
            RuntimeLifecycleStatus::ProjectionMissingProcessRunning => Self {
                dot_sgr: sgr::WARN,
                text_sgr: sgr::DIM,
                dot: "●",
                message: "daemon facts present but runtime metadata missing".to_string(),
            },
            RuntimeLifecycleStatus::ControlOnlyInvocationDown => Self {
                dot_sgr: sgr::WARN,
                text_sgr: sgr::DIM,
                dot: "●",
                message: "control endpoint up but invocation down".to_string(),
            },
            RuntimeLifecycleStatus::IdentityMismatch => Self {
                dot_sgr: sgr::WARN,
                text_sgr: sgr::DIM,
                dot: "●",
                message: "daemon identity mismatch".to_string(),
            },
            RuntimeLifecycleStatus::StartProjectionCommitFailed => Self {
                dot_sgr: sgr::WARN,
                text_sgr: sgr::DIM,
                dot: "●",
                message: "runtime projection commit failed".to_string(),
            },
            RuntimeLifecycleStatus::StopTimedOut => Self {
                dot_sgr: sgr::WARN,
                text_sgr: sgr::DIM,
                dot: "●",
                message: "runtime stop timed out".to_string(),
            },
        }
    }
}

/// Print a `label  value` row with the label painted accent and
/// padded to `LABEL_WIDTH` so all rows line up under one another.
fn write_row(buf: &mut String, style: ColourMode, label: &str, value: &str) {
    let padded = format!("{label:<LABEL_WIDTH$}");
    buf.push_str(MARGIN);
    buf.push_str(&style.paint(sgr::ACCENT, &padded));
    buf.push(' ');
    buf.push_str(value);
    buf.push('\n');
}

/// Cheap count of `[[trusted_agent]] role="hub"` entries in
/// `realm-trust.toml`. Returns 0 on any read or parse error — banner
/// must never panic.
fn read_trusted_hub_count() -> usize {
    let Some(path) = realm_trust_path() else {
        return 0;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(doc) = raw.parse::<toml_edit::DocumentMut>() else {
        return 0;
    };
    doc.get("trusted_agent")
        .and_then(|i| i.as_array_of_tables())
        .map(|tbls| {
            tbls.iter()
                .filter(|t| t.get("role").and_then(|v| v.as_str()) == Some("hub"))
                .count()
        })
        .unwrap_or(0)
}

/// Same for `[daemon.federated_peers]` in the daemon config.
fn read_federated_peer_count() -> usize {
    let Some(path) = daemon_config_path() else {
        return 0;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(doc) = raw.parse::<toml_edit::DocumentMut>() else {
        return 0;
    };
    doc.get("daemon")
        .and_then(|i| i.as_table())
        .and_then(|t| t.get("federated_peers"))
        .and_then(|i| i.as_table())
        .map(|t| t.len())
        .unwrap_or(0)
}

fn realm_trust_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("EASYNET_REALM_TRUST_PATH") {
        return Some(p.into());
    }
    let etc = std::path::PathBuf::from("/etc/easynet/realm-trust.toml");
    if let Ok(meta) = std::fs::metadata(&etc) {
        if meta.is_file() && meta.len() > 0 {
            return Some(etc);
        }
    }
    let home = dirs::home_dir()?;
    Some(home.join(".easynet").join("realm-trust.toml"))
}

fn daemon_config_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("EASYNET_DAEMON_CONFIG_PATH") {
        return Some(p.into());
    }
    let home = dirs::home_dir()?;
    Some(home.join(".easynet").join("daemon-config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;

    /// Force colour off and render. Used by every test so the
    /// asserted output is plain ASCII-with-newlines.
    fn render_plain() -> String {
        let _home = HomeGuard::new();
        render_plain_with_current_home()
    }

    fn render_plain_with_current_home() -> String {
        unsafe {
            std::env::set_var("NO_COLOR", "1");
        }
        render_top_level_banner()
    }

    #[test]
    fn render_never_panics_in_clean_environment() {
        // The banner must produce something sensible even when the
        // user has never run easynet before — no credentials, no
        // config files, no daemon.
        let out = render_plain();
        assert!(out.contains("EasyNet"), "wordmark missing");
        assert!(out.contains("Silan Hu"), "signature missing");
        assert!(out.contains("Daemon:"), "daemon status missing");
        assert!(out.contains("Hub:"), "hub status missing");
    }

    #[test]
    fn tagline_matches_homepage_verbatim() {
        // Pin the tagline to the source-of-truth string from
        // easynet.run. If the homepage tagline changes, update both
        // this test and the constant in `write_tagline`. We do NOT
        // accept paraphrases here — the banner ships canonical copy.
        let out = render_plain();
        assert!(
            out.contains("The Internet for Agents and Devices"),
            "canonical tagline missing or paraphrased"
        );
    }

    #[test]
    fn banner_contains_no_cjk() {
        // The banner shipped a Chinese blessing line in an earlier
        // draft; this test pins the contract that we do not
        // reintroduce CJK characters. Common Unicode glyphs (`●`,
        // `·`, `…`) used by every modern CLI for status display are
        // allowed — the rule is "no CJK", not "ASCII-only". Ranges
        // covered: CJK Unified Ideographs (4E00-9FFF), Halfwidth
        // and Fullwidth Forms (FF00-FFEF), CJK Symbols and
        // Punctuation (3000-303F), Hiragana (3040-309F), Katakana
        // (30A0-30FF), Hangul (AC00-D7AF).
        let out = render_plain();
        for (i, ch) in out.char_indices() {
            let cp = ch as u32;
            let is_cjk = (0x4E00..=0x9FFF).contains(&cp)
                || (0xFF00..=0xFFEF).contains(&cp)
                || (0x3000..=0x303F).contains(&cp)
                || (0x3040..=0x309F).contains(&cp)
                || (0x30A0..=0x30FF).contains(&cp)
                || (0xAC00..=0xD7AF).contains(&cp);
            assert!(
                !is_cjk,
                "CJK character {ch:?} (U+{cp:04X}) at byte {i} in banner — \
                 the banner must stay non-CJK; tagline is the canonical \
                 English homepage line.",
            );
        }
    }

    #[test]
    fn no_color_strips_ansi() {
        let out = render_plain();
        assert!(!out.contains('\x1b'), "ANSI escape leaked despite NO_COLOR");
    }

    #[test]
    fn malformed_runtime_projection_renders_unavailable_not_stopped() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        std::fs::write(config::runtime_state_path(), "{ not json").expect("runtime projection");

        let out = render_plain_with_current_home();

        assert!(
            out.contains("metadata unavailable"),
            "banner must expose corrupt runtime projection: {out}"
        );
        assert!(
            !out.contains("not running  ·  start with 'easynet runtime start'"),
            "corrupt runtime projection must not render as stopped: {out}"
        );
    }

    #[test]
    fn malformed_credentials_render_invalid_not_unpaired() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        std::fs::write(config::state_dir().join("credentials.json"), "{ not json")
            .expect("malformed credentials");

        let out = render_plain_with_current_home();

        assert!(
            out.contains("credentials invalid"),
            "banner must expose invalid credentials: {out}"
        );
        assert!(
            !out.contains("not paired  ·  run 'easynet device join <token>'"),
            "invalid credentials must not render as unpaired: {out}"
        );
    }

    #[test]
    fn federation_native_device_only_credentials_render_explicit_unbound_user() {
        let _home = HomeGuard::new();
        let credentials = config::Credentials {
            node_id: "device-a".to_string(),
            credential_token: String::new(),
            hub_endpoint: "https://hub.example:50443".to_string(),
            realm: "localhost".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("a".repeat(64)),
        };
        config::save_credentials(&credentials).expect("save federation-native credentials");

        let out = render_plain_with_current_home();

        assert!(
            out.contains("Current user:"),
            "banner must keep the user binding row visible: {out}"
        );
        assert!(
            out.contains("not bound (federation-native device credential)"),
            "banner must render explicit unbound user state: {out}"
        );
    }
}
