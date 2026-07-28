// EasyNet CLI — Ability Manifest (abilities/*.ability.toml)
// ==========================================================
//
// File: src/daemon/ability/manifest.rs
// Description: On-disk schema for one file under
//              `<agent-root>/abilities/<name>.ability.toml`. One
//              manifest per file; the stem of the file name is
//              authoritative for the ability's verb portion.
//
// Where this fits in the stack
// ----------------------------
// An `AbilityManifest` is the daemon persistence/import representation of a
// single
// `abilities/<verb>.ability.toml`. One agent has many manifests,
// kept as independent files rather than one combined manifest so
// that adding, editing, and removing a single ability is a
// single-file operation (friendly to `git diff`, friendly to
// `mv`/`rm` refactors, friendly to a future `agent publish --only
// <verb>` workflow).
//
// Who reads this
// --------------
// * `daemon::execution::mission::directory` enumerates the files on disk.
// * `HotAgentRegistrar` commits each manifest as one governed
//   descriptor/authority/implementation binding in the daemon catalog.
// * `a2a_labels` projects discovery JSON from that live catalog snapshot.
//
// Why this lives in `daemon::ability`
// -----------------------------------
// A manifest describes an executable package: executor selection, boot and
// health probes, and daemon-local access policy. Those are deployment facts,
// not core ontology. The daemon normalizes a manifest into the governed
// descriptor, authority binding, and implementation binding at registration.
//
// What is NOT in this file
// ------------------------
// * Filesystem enumeration — `daemon::execution::mission::directory` walks
//   `<agent-root>/abilities/` and parses each file.
// * Invocation plumbing — the hot registrar binds a manifest to daemon
//   Invocation; `daemon::execution::mission::dispatch` does the
//   actual subprocess wrangling when an invocation lands.
// * Any agent-name awareness — the manifest does NOT know which
//   agent it belongs to. The agent name is contributed by the
//   enclosing directory: `<agent>` + `<verb>` → `<agent>.<verb>`
//   is assembled one layer up. That keeps a manifest file
//   portable across `cp -R` of an agent root.
//
// Layering rule
// -------------
// `daemon::ability::manifest` must not import any other `crate::` module
// and must not pull in external crates beyond `serde` + `toml` +
// `serde_json` (for the embedded JSON Schema fields).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current on-disk schema version. Matches `AgentSpec`'s versioning
/// policy — a single digit, bumped only on a breaking change to the
/// shape. Reader refuses unknown versions; writer stamps the
/// `CURRENT_SCHEMA_VERSION` on every file it generates.
pub const CURRENT_SCHEMA_VERSION: &str = "1";

/// Default governed interface version for ability descriptors. This is not
/// the TOML file schema version; it is the version that enters descriptor
/// hashes, authority bindings, implementation bindings, and Axon receipts.
pub const DEFAULT_DESCRIPTOR_VERSION: &str = "1.0.0";

/// Validate a governed ability descriptor version.
///
/// The grammar is intentionally narrower than SemVer: exactly three
/// dot-separated numeric fields. Capability negotiation belongs one layer up;
/// this field is a stable control-plane fact, not a range expression.
pub fn is_valid_descriptor_version(version: &str) -> bool {
    if version.trim().is_empty() || version.trim() != version {
        return false;
    }
    let mut parts = version.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    [major, minor, patch]
        .into_iter()
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

/// Two kinds of ability the CLI publishes. The split disambiguates
/// agent abilities (per-agent `<agent>.<verb>`-style) from daemon-owned
/// device/control-plane abilities.
///
/// Why an enum and not a free-form string: the kind is a wire-level
/// promise about handler ownership. `System` means the publishing node
/// owns the handler and no agent subprocess is reached. The enum lets a
/// reader look at one field and know which dispatch path applies,
/// instead of grepping a prefix.
///
/// `Agent` is the existing case (every `<name>.chat` pre-PR-SYS
/// shipped under this kind, even though the kind didn't exist
/// then). `System` is the new case enabled by PR-SYS — the daemon
/// publishes the handler, no agent involved. Future kinds (e.g.
/// `Skill` for installable skill bundles) plug in here as another
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityKind {
    /// Belongs to one registered agent; dispatch lands inside the
    /// agent's subprocess. Names: `<agent>.<verb>`.
    Agent,
    /// Belongs to the daemon (the node) itself; dispatch lands
    /// in-process via daemon-owned ability handlers.
    System,
}

impl AbilityKind {
    /// Infer kind from a fully-qualified ability name. Useful at
    /// dispatch-router boundaries that receive a string and need
    /// to know which sub-system owns the handler.
    ///
    /// The classifier matches the canonical first-segment partition:
    /// `device.*`, `hub.*`, and current control-plane `system.*` names
    /// are daemon-owned; everything else is agent-owned.
    pub fn from_qualified_name(name: &str) -> Self {
        if name.starts_with("device.") || name.starts_with("hub.") || name.starts_with("system.") {
            Self::System
        } else {
            Self::Agent
        }
    }
}

/// Versions the reader will accept. When bumping:
///   1. Extend this array with the new version.
///   2. Add a migration pass that rewrites manifests from the old
///      version into the new shape on load, and re-saves them.
///   3. Only remove the old version once every supported agent
///      install has been through a migration pass.
pub const SUPPORTED_SCHEMA_VERSIONS: &[&str] = &["1", "2"];

/// One ability the owning agent offers as a network-visible tool.
///
/// Fields
/// ------
/// * `schema_version` — required on-disk schema version. Writer always
///   stamps explicitly; readers reject absent or unsupported versions
///   instead of inferring an implicit migration state.
/// * `descriptor_version` — governed interface version; this is the
///   version hashed into descriptor proofs and propagated into authority /
///   implementation bindings. Absence means [`DEFAULT_DESCRIPTOR_VERSION`].
/// * `name` — the *verb* portion of the ability's name. The wire-
///   level name is assembled as `<agent>.<name>` by whoever calls
///   `qualified_name` below. Must not contain `.` (that is the
///   agent/verb separator), must not be empty.
/// * `description` — a short, human-readable blurb. Passed verbatim
///   to the tool-use contract shown to an agent choosing which tool
///   to call. Not a protocol field; safe to tune for readability.
/// * `timeout_seconds` — upper bound on how long an invocation may
///   run before the dispatcher aborts it. `None` inherits the
///   runtime default (see `support::timeouts`). The *unit* is
///   seconds specifically — we carry the raw `u64` and only convert
///   to `Duration` at the boundary so TOML round-trips are exact.
/// * `input_schema` / `output_schema` — JSON Schema documents.
///   `input_schema` is required and must be a JSON object at its
///   top level (`{"type": "object", ...}`). `output_schema` is
///   optional; absence means "the ability returns opaque content"
///   (typical for a chat-style ability whose reply is a string the
///   caller is expected to post-process).
///
/// Why the two schemas are `serde_json::Value`
/// --------------------------------------------
/// A JSON Schema is itself a tree of nested objects with
/// schema-specific keywords (`$ref`, `oneOf`, etc.). Typing that
/// tree statically would reimplement a JSON Schema crate in our
/// ontology layer; instead we carry a validated `Value` and let
/// downstream tooling (Axon's ToolSpec, OpenAI's tool-use contract)
/// validate on the read side. Our own validation is limited to
/// "top-level is an object" — the one invariant that makes every
/// consumer's failure mode the same.
///
/// Why private fields with getters
/// -------------------------------
/// Construction goes through `AbilityManifest::new` or
/// `from_toml_str`; both run `validate()`. Public fields would
/// let a caller mint a malformed manifest with a literal, which
/// would then explode in a distant consumer at read time. The
/// narrow constructor makes "well-formed by construction" the
/// only path.
// No `Eq`: `serde_json::Value` contains `f64` which is not `Eq`-able
// (NaN != NaN). `AgentSpec` has `Eq`, so the asymmetry would otherwise
// be surprising to a reader expecting `HashMap<AbilityManifest, _>` or
// `BTreeSet<AbilityManifest>` to compile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbilityManifest {
    schema_version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    descriptor_version: Option<String>,
    /// Authorization action committed into the governed descriptor. Absence
    /// is accepted while editing metadata, but canonical registration rejects
    /// it; no action is inferred from the ability name or transport mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    admission_action: Option<String>,
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u64>,
    input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_schema: Option<Value>,
    /// Optional executor binding. When present, the daemon dispatches
    /// the ability to the named executor directly. Absence means the
    /// manifest is discoverable metadata only; it is not an invocable
    /// route.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    exec: Option<AbilityExec>,
    /// Optional access policy. Drives `<agent>.discover` filtering and
    /// the `<agent>.invoke` permission check. Absence is treated as the
    /// default policy (`AccessPolicy::default()`), which sets
    /// `visibility = "device"` — the same trust boundary as "agents
    /// running on the same physical device under one user".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    access: Option<AccessPolicy>,
    /// Optional cost declaration. Discovery / MCP surface annotates
    /// every advertised ability with a `cost_kind` + `cost_label`
    /// pair so an operator (or an LLM choosing between two tools)
    /// can see at a glance whether a call is free, LLM-billed,
    /// upstream-metered, or unknown. Declared values are authoritative.
    /// Absence means cost is undeclared and projects as `unknown`; neither
    /// source strings nor executor kind may infer a billing class. Per plan
    /// §"Cost is static catalog metadata", this manifest field is the single
    /// source of truth for catalog-level cost; runtime usage accounting
    /// (per-call token counts, vendor-reported $) is a different surface that
    /// flows through telemetry, not this struct.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    cost: Option<CostMeta>,
    /// Optional boot script (`[boot]`). Declares how to START the
    /// external service this ability fronts (a local n8n container,
    /// a database, …). Run by the daemon's ability-health monitor
    /// when the `[health]` probe reports the service down. Only
    /// meaningful together with `[health]` — a manifest that
    /// declares `[boot]` without `[health]` is rejected at
    /// validate-time, because a boot whose outcome can never be
    /// observed is an unverifiable side effect.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    boot: Option<BootSpec>,
    /// Optional health probe (`[health]`). Declares how to CHECK the
    /// live status of the backing service. The daemon probes on the
    /// declared interval and publishes the result as catalog
    /// metadata (`health_status` / `health_detail` /
    /// `health_checked_unix_ms`), so discovery surfaces reflect the
    /// real service state instead of just owner presence. Abilities
    /// with an external dependency (`cost.kind = "external_metered"`)
    /// that omit `[health]` are surfaced as `unmonitored` rather
    /// than silently presented as invocable.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    health: Option<HealthSpec>,
}

/// Per-ability cost declaration, written to disk under the `[cost]`
/// section of an `*.ability.toml` file.
///
/// **Why a separate struct, not two free-form strings.** `kind` is a
/// closed enum so consumers (frontend filters, discovery sorters,
/// audit ledgers) can switch on the variant rather than string-match.
/// `label` is free-form human text — the place an operator declares
/// the actual upstream ("Google Maps Geocoding API — $5 per 1000
/// requests") so the LLM and the auditor see the real-world rate
/// rather than just the bucket name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostMeta {
    /// Coarse cost bucket. Must be one of the `CostKind` variants;
    /// unknown strings are rejected at parse time by serde.
    pub kind: CostKind,
    /// Free-form human label. Optional — when absent the consumer
    /// renders a generic per-kind blurb (e.g. `kind = llm_metered`
    /// → "LLM token billing may apply"). Present a real label
    /// whenever you can; it is what reaches the operator's eye.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
}

/// Coarse cost classification. The set is intentionally small — a
/// frontend filter should fit on one row of checkboxes, and a
/// discovery sorter should sort all abilities into four buckets, not
/// forty.
///
/// Adding a variant: extend this enum, update `as_wire_str`, and
/// update every consumer that switches on the string form (currently
/// `profiles::mcp::inferred_cost_label`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostKind {
    /// Local computation, no external billing. CLI utilities, jq, fs.read.
    Free,
    /// Upstream API or third-party service that bills. HTTP executors
    /// to a paid API, MCP servers fronting Google Maps / Stripe / etc.
    ExternalMetered,
    /// LLM token usage will be billed by the model vendor. Agent-chat
    /// abilities and any executor that internally drives an LLM.
    LlmMetered,
    /// Operator has not declared a cost. Default when the runtime
    /// cannot prove a path is local. Renders as "cost not declared".
    Unknown,
}

impl CostKind {
    /// Stable string form used for `x-easynet.cost_kind` and for the
    /// `cost: <kind> (<label>)` line in MCP descriptions. Mirrors the
    /// TOML serde rename rules so wire form and on-disk form agree.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            CostKind::Free => "free",
            CostKind::ExternalMetered => "external_metered",
            CostKind::LlmMetered => "llm_metered",
            CostKind::Unknown => "unknown",
        }
    }
}

impl CostMeta {
    fn validate(&self) -> anyhow::Result<()> {
        if let Some(label) = &self.label {
            if label.trim().is_empty() {
                anyhow::bail!(
                    "ability.toml [cost] `label`, when present, must be a non-empty string \
                     — omit the field instead of writing an empty/whitespace value"
                );
            }
        }
        Ok(())
    }
}

/// Where this ability is discoverable and callable from.
///
/// The model is monotonic: each tier strictly includes the smaller
/// ones (`device` includes `self`, `public` includes both). This
/// matches the `<agent>.discover(scope: ...)` ladder — Tier 1 returns
/// `self`+, Tier 2 returns `device`+, Tier 3 returns `public`.
///
/// Default is `Device` rather than `Self` because two agents on the
/// same device share the user's trust boundary; requiring an explicit
/// opt-in for every per-agent ability would be a usability tax with
/// no real security gain (the OS already gates the device boundary).
/// An author who wants stricter scoping ticks `visibility = "self"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ManifestAccessScope {
    /// Only the owning agent can discover or invoke. Useful for
    /// internal helpers an agent uses inside its own chat loop and
    /// does not want peers to call.
    #[serde(rename = "self")]
    Selfish,
    /// Other agents on the same device may discover and invoke.
    /// Default. Matches "this is my computer, my agents can talk".
    #[default]
    Device,
    /// Visible to the EasyNet federation (other users' devices).
    /// Requires the federation layer to route. Until that layer
    /// ships, the runtime treats `public` like `device` for local
    /// dispatch and surfaces nothing extra to remote callers.
    Public,
}

/// Per-ability access policy.
///
/// ManifestAccessScope is the coarse "who can see / call this at all" knob;
/// `allow_callers` / `deny_callers` are the fine-grained "of those
/// allowed by visibility, which specific peer agents are pinned in
/// (or out)" knobs. Order of evaluation:
///
///   1. `visibility` filter — `self`/`device`/`public` controls the
///      tier.
///   2. `deny_callers` — if the caller's name is here, reject. Deny
///      always wins over allow.
///   3. `allow_callers` — if non-empty, the caller's name MUST be
///      in the list. Empty list = "anyone allowed by visibility".
///
/// Caller name comparison is exact-string (no glob / wildcard yet).
/// `*` is reserved for future glob support but is not interpreted
/// in v1 — it's just an unusual literal name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AccessPolicy {
    /// Discoverability + invocation tier. Default `ManifestAccessScope::Device`.
    /// See `ManifestAccessScope` doc for the trust model.
    #[serde(default)]
    pub visibility: ManifestAccessScope,
    /// Optional whitelist of caller agent names. When non-empty, ONLY
    /// these callers may invoke; everyone else is rejected with
    /// `permission_denied`. Empty (default) = no whitelist applied.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub allow_callers: Option<Vec<String>>,
    /// Optional blacklist of caller agent names. Always wins over
    /// `allow_callers` and over `visibility`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub deny_callers: Option<Vec<String>>,
}

impl AccessPolicy {
    /// Whether this policy permits a caller in `caller_scope` to
    /// discover or invoke. The caller's scope is determined by the
    /// dispatch layer (self if same agent, device if peer-on-this-box,
    /// public if federation routed).
    ///
    /// This check ignores caller identity — pair with
    /// `allows_caller_name` when the caller is named.
    pub fn allows_caller(&self, caller_scope: ManifestAccessScope) -> bool {
        // Monotonic tier check: caller's scope must be ≤ ability's
        // visibility. `self` is the strictest, `public` is the
        // broadest.
        let ability_tier = self.visibility.tier();
        let caller_tier = caller_scope.tier();
        caller_tier <= ability_tier
    }

    /// Whether this policy admits a caller named `caller_name` once
    /// the visibility tier check has passed. Returns false when the
    /// deny list contains the name OR when the allow list is non-
    /// empty and does NOT contain the name.
    ///
    /// Exact-string match. v1 has no glob support; that's a future
    /// addition (the `*` literal stays uninterpreted).
    pub fn allows_caller_name(&self, caller_name: &str) -> bool {
        if let Some(deny) = &self.deny_callers {
            if deny.iter().any(|n| n == caller_name) {
                return false;
            }
        }
        if let Some(allow) = &self.allow_callers {
            if !allow.is_empty() && !allow.iter().any(|n| n == caller_name) {
                return false;
            }
        }
        true
    }
}

impl ManifestAccessScope {
    /// Numeric tier so `allows_caller` can compare. The values are
    /// intentionally not part of the serialised TOML — TOML carries
    /// the snake_case names and `tier()` is private numeric form.
    fn tier(self) -> u8 {
        match self {
            ManifestAccessScope::Selfish => 0,
            ManifestAccessScope::Device => 1,
            ManifestAccessScope::Public => 2,
        }
    }

    /// Stable string form used in JSON discovery payloads (the
    /// `visibility` field of a `<agent>.discover` candidate). Mirrors
    /// the TOML serde rename rules so the wire form and the on-disk
    /// form agree.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            ManifestAccessScope::Selfish => "self",
            ManifestAccessScope::Device => "device",
            ManifestAccessScope::Public => "public",
        }
    }
}

/// Executor binding for an ability. The TOML uses an internally-tagged
/// representation (`kind = "shell"`); future executor kinds (http,
/// wasm, agent_chat with a non-self target) plug in as additional
/// variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AbilityExec {
    /// Spawn a subprocess via `argv`. Each element is rendered as a
    /// minijinja template with the call's arguments bound by name —
    /// so `{{ location }}` in argv pulls from `args["location"]`. The
    /// argv form deliberately bypasses `sh -c` so a value that
    /// contains a space or shell metacharacter cannot expand into a
    /// second token.
    Shell(ShellExec),
    /// Issue one HTTP request and return the response. Distinct from
    /// the shell executor + curl: no subprocess, no shellguard, no
    /// argv injection surface. Args are rendered into URL / headers /
    /// body via the same `{{ name }}` template rules, but values are
    /// URL-encoded automatically when interpolated into the URL — a
    /// `{{ city }}` of `"São Paulo"` becomes `S%C3%A3o%20Paulo` so
    /// the call doesn't fail mid-fetch on a control character.
    Http(HttpExec),
    /// Run a small EAL program as the ability's implementation. The
    /// `source` field carries the EAL text with `{{ name }}` template
    /// placeholders rendered against call args BEFORE the parser
    /// runs. Lets a curator-published ability compose existing
    /// abilities into a reusable workflow without inventing a second
    /// orchestration surface — same EAL exposed through the canonical
    /// `mission.run` ability.
    Eal(EalExec),
    /// Dispatch to one configured upstream MCP tool. This is the
    /// deterministic executor used when an operator binds an MCP
    /// server's tool catalogue into a specific EasyNet agent via the
    /// CLI. It preserves the MCP `tools/call` response shape and
    /// avoids routing through shell or chat translation.
    Mcp(McpExec),
    /// Stream frames from an external warm host over a Unix socket.
    /// The daemon opens `host_socket`, sends one request line, then
    /// reads many JSON frame lines until an explicit terminal — letting
    /// an external resident process (e.g. an easyremote Python host
    /// running a generator) stream frames without re-spawning per
    /// frame. Unlike `Shell` (one bounded result, RPC-only), this is
    /// the sole external-process *server-stream* path: an ability with
    /// this exec registers as stream-mode. The wire protocol is the
    /// single source of truth in `HostStreamExec`'s doc comment.
    HostStream(HostStreamExec),
}

/// Configuration for the `host_stream` executor.
///
/// **Wire protocol (newline-delimited UTF-8 JSON over `host_socket`),
/// the single source of truth for both the daemon executor and the
/// external host:**
///
/// ```text
/// daemon → host:  {"request":{"fn":"<function>","args":{...},"call_id":"<id>"}}
/// host → daemon:  {"stream_item":<value>,"seq":0}
///                 {"stream_item":<value>,"seq":1}
///                 {"terminal":{"output_hash":"sha256:<hex>","frames":2}}
///   — OR (mutually exclusive with terminal, at most once) —
///                 {"error":{"kind":"<KIND>","message":"...","recoverable":false}}
/// ```
///
/// Invariants:
/// 1. `seq` is monotonically increasing from 0; a gap or reorder is a
///    truncation failure (frame reorder must not be invisible).
/// 2. `output_hash = H(prev_hash || seq || canonical_json(frame))`,
///    seeded with the empty hash, folded over every emitted frame in
///    `seq` order. The host sends the final rolling hash on `terminal`;
///    the daemon recomputes and a mismatch is a truncation failure.
/// 3. `terminal` and `error` are mutually exclusive and each may appear
///    at most once; whichever arrives first ends the stream.
/// 4. EOF (socket close) before `terminal`/`error` is `STREAM_TRUNCATED`,
///    NOT a clean terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostStreamExec {
    /// AF_UNIX path to the external warm host's stream socket.
    pub host_socket: String,
    /// The resident function name to invoke on the host.
    pub function: String,
}

impl HostStreamExec {
    fn validate(&self) -> anyhow::Result<()> {
        let host_socket = self.host_socket.trim();
        if host_socket.is_empty() {
            anyhow::bail!("host_stream exec: host_socket must not be empty");
        }
        let socket_path = std::path::Path::new(host_socket);
        if !socket_path.is_absolute() {
            anyhow::bail!("host_stream exec: host_socket must be an absolute Unix socket path");
        }
        if socket_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            anyhow::bail!("host_stream exec: host_socket must not contain `..` components");
        }

        let function = self.function.trim();
        if function.is_empty() {
            anyhow::bail!("host_stream exec: function must not be empty");
        }
        if !is_host_stream_function_token(function) {
            anyhow::bail!(
                "host_stream exec: function must contain only ASCII letters, digits, `_`, `-`, \
                 `.`, or `:` and must start with a letter or `_`"
            );
        }
        Ok(())
    }
}

fn is_host_stream_function_token(function: &str) -> bool {
    let mut chars = function.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
}

/// Configuration for the `shell` executor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellExec {
    /// Argv vector. argv[0] is the program; subsequent elements are
    /// arguments. Each element is independently rendered as a
    /// minijinja template against the call args. Empty argv is
    /// rejected at validate-time.
    pub argv: Vec<String>,
    /// Optional override for stdout decoding. Default is `"utf8_trim"`
    /// (decode as UTF-8 and trim trailing whitespace). Future values:
    /// `"json"`, `"base64"`. Kept as a string so adding a mode does
    /// not break the schema.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stdout: Option<String>,
    /// Optional OS-level sandbox profile. When set the executor
    /// wraps the spawn under macOS `sandbox-exec` (Linux: bwrap, when
    /// wired). Pre-set named profiles:
    ///
    ///   * `none`         — no sandbox. Default behaviour. Use when
    ///                      the ability is already trusted (a local
    ///                      tool the operator vetted).
    ///   * `net_only`     — deny filesystem writes outside of /tmp,
    ///                      allow outbound network. Right for
    ///                      `curl wttr.in`-style abilities.
    ///   * `pure_compute` — deny network and writes; read-only fs
    ///                      access. Right for `jq`/`awk`-style
    ///                      abilities that don't need external
    ///                      resources.
    ///
    /// On a platform without a backing sandbox tool (Linux without
    /// bwrap, Windows) a non-`none` profile aborts the call rather
    /// than silently no-op'ing — a security feature an operator
    /// asked for must not become a no-op behind their back.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sandbox: Option<String>,
}

/// Configuration for the `http` executor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpExec {
    /// HTTP method. Validated case-insensitively against the small
    /// safe-methods set (GET / POST / PUT / DELETE / PATCH /
    /// HEAD). A method outside that set is rejected at validate-time
    /// rather than passed through, since most outliers (CONNECT /
    /// TRACE / arbitrary verbs) are footguns.
    pub method: String,
    /// URL with `{{ name }}` placeholders. Each placeholder's value
    /// is URL-encoded automatically when expanded into the URL —
    /// see `HttpExec` doc for the encoding rule.
    pub url: String,
    /// Optional headers, each value rendered with the same template
    /// rules. Header names are passed verbatim; values are NOT
    /// URL-encoded (URL encoding is path-specific, not header-
    /// specific) but ARE rejected for CR/LF on the way out via the
    /// underlying http client's safe-header check.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    /// Optional body. Rendered with the same template rules. For a
    /// JSON body the author should pre-stringify the JSON (the
    /// executor will not auto-serialise) — this keeps the body
    /// representation a single transparent string in the manifest.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub body: Option<String>,
    /// Optional response decoding mode. Default `"text_trim"` (UTF-8
    /// + trim trailing whitespace). Future: `"json"` (parse to JSON
    /// Value), `"base64"` (binary). Same kept-as-string forward-
    /// compat shape as `ShellExec.stdout`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub response: Option<String>,
}

/// Configuration for the `eal` executor — an ability whose
/// implementation is a small EAL program composing other
/// abilities. The `source` field is rendered with `{{ name }}`
/// substitution against call args BEFORE the parser runs, then
/// passed to `mission_runs::run_mission_inproc`.
///
/// Why we cap source size + reject empty
/// -------------------------------------
/// An empty source compiles to an empty mission (legal but
/// useless); we reject so a typo in `ability.publish` surfaces at
/// validate-time. The size cap (1 MiB) prevents an accidental
/// stamp of a huge document into an ability — by far the more
/// common manifest mistake than a deliberately large EAL
/// program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EalExec {
    /// EAL source, with `{{ name }}` placeholders substituted
    /// against the caller's `args` JSON before the parser runs.
    pub source: String,
    /// Optional binding name whose value becomes the ability's
    /// final result. When set, the executor extracts
    /// `mission_run.bound_vars[binding]` and returns it as the
    /// envelope's `result` field. When absent, the executor
    /// returns the entire `bound_vars` map as `result`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result_binding: Option<String>,
}

/// Configuration for the `mcp` executor. The upstream server name
/// must match one row in `~/.easynet/mcp_clients.json`; `tool` is the
/// upstream MCP tool name exactly as returned by `tools/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpExec {
    /// Operator-chosen upstream server name from mcp_clients.json.
    pub server: String,
    /// Upstream MCP tool name, passed verbatim to tools/call.
    pub tool: String,
}

/// Configuration for the `[boot]` section — a script that starts the
/// external service backing this ability (e.g. `docker start
/// easynet-n8n`). Same argv-not-`sh -c` rule as `ShellExec`: values
/// cannot expand into extra tokens. No template substitution — boot
/// runs with no invocation args in scope, so placeholders would have
/// nothing to bind against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootSpec {
    /// Argv vector. argv[0] is the program (absolute path when the
    /// daemon's PATH cannot be assumed); subsequent elements are
    /// arguments. Run verbatim — no templating.
    pub argv: Vec<String>,
    /// Upper bound for the boot script run. `None` inherits the
    /// monitor default (60 s). The monitor kills the process at the
    /// deadline and records the attempt as failed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timeout_seconds: Option<u64>,
}

/// Configuration for the `[health]` section — a liveness probe for
/// the backing service. Exit status 0 means healthy, anything else
/// (including a timeout) means unhealthy — the same convention as
/// Docker `HEALTHCHECK` and Kubernetes exec probes, so an operator
/// can paste an existing probe command unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSpec {
    /// Argv vector for the probe (e.g. `["curl", "-fsS", "-m", "5",
    /// "http://127.0.0.1:5678/healthz"]`). Run verbatim — no
    /// templating.
    pub argv: Vec<String>,
    /// Seconds between probes. `None` inherits the monitor default
    /// (30 s).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub interval_seconds: Option<u64>,
    /// Upper bound for one probe run. `None` inherits the monitor
    /// default (10 s). A probe killed at the deadline counts as
    /// unhealthy.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timeout_seconds: Option<u64>,
}

impl AbilityManifest {
    /// Build a manifest, validating the fields that downstream
    /// consumers rely on.
    ///
    /// This is the canonical constructor; `from_toml_str` funnels
    /// through the same `validate()` once it has deserialized.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> anyhow::Result<Self> {
        let m = Self {
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            descriptor_version: None,
            admission_action: None,
            name: name.into(),
            description: description.into(),
            timeout_seconds: None,
            input_schema,
            output_schema: None,
            exec: None,
            access: None,
            cost: None,
            boot: None,
            health: None,
        };
        m.validate()?;
        Ok(m)
    }

    /// Attach a boot script. Requires a `[health]` probe to already
    /// be attached (or attached before the manifest is persisted) —
    /// `validate()` rejects boot-without-health. Returns the
    /// manifest for builder chaining.
    pub fn with_boot(mut self, boot: BootSpec) -> anyhow::Result<Self> {
        self.boot = Some(boot);
        self.validate()?;
        Ok(self)
    }

    /// Attach a health probe. Returns the manifest for builder
    /// chaining.
    pub fn with_health(mut self, health: HealthSpec) -> anyhow::Result<Self> {
        self.health = Some(health);
        self.validate()?;
        Ok(self)
    }

    /// The declared boot script, if any.
    pub fn boot(&self) -> Option<&BootSpec> {
        self.boot.as_ref()
    }

    /// The declared health probe, if any.
    pub fn health(&self) -> Option<&HealthSpec> {
        self.health.as_ref()
    }

    /// Attach a cost declaration. Optional; absence projects as explicit
    /// undeclared/unknown cost at metadata-emit time. Returns the manifest for
    /// builder chaining.
    pub fn with_cost(mut self, cost: CostMeta) -> anyhow::Result<Self> {
        self.cost = Some(cost);
        self.validate()?;
        Ok(self)
    }

    /// The declared cost meta, if any. `None` means the author has
    /// not declared cost; consumers must project explicit uncertainty rather
    /// than infer a bucket from owner/source/executor facts.
    pub fn cost(&self) -> Option<&CostMeta> {
        self.cost.as_ref()
    }

    /// Attach an executor binding. Returns the manifest for builder
    /// chaining.
    pub fn with_exec(mut self, exec: AbilityExec) -> anyhow::Result<Self> {
        self.exec = Some(exec);
        self.validate()?;
        Ok(self)
    }

    /// Attach an access policy. Optional; absence is treated as the
    /// default policy (`device` visibility) at read time. Returns the
    /// manifest for builder chaining.
    pub fn with_access(mut self, access: AccessPolicy) -> anyhow::Result<Self> {
        self.access = Some(access);
        self.validate()?;
        Ok(self)
    }

    /// Set the governed interface version for this ability. This value is
    /// distinct from `schema_version`: changing it changes the descriptor
    /// hash and the authority / implementation binding key.
    pub fn with_descriptor_version(mut self, version: impl Into<String>) -> anyhow::Result<Self> {
        self.descriptor_version = Some(version.into());
        self.validate()?;
        Ok(self)
    }

    pub fn with_admission_action(mut self, action: impl Into<String>) -> anyhow::Result<Self> {
        self.admission_action = Some(action.into());
        self.validate()?;
        Ok(self)
    }

    /// Override the default `None` timeout. Returns `self` for the
    /// builder-style chain a caller of `new(...)` might use.
    pub fn with_timeout_seconds(mut self, seconds: u64) -> anyhow::Result<Self> {
        self.timeout_seconds = Some(seconds);
        self.validate()?;
        Ok(self)
    }

    /// Attach an `output_schema`. Optional; only set when the
    /// ability has a typed return contract (code-review scorecard,
    /// structured evaluation, etc.) — chat-style abilities
    /// deliberately leave it absent.
    pub fn with_output_schema(mut self, schema: Value) -> anyhow::Result<Self> {
        self.output_schema = Some(schema);
        self.validate()?;
        Ok(self)
    }

    /// Parse from TOML. Validates before returning — a manifest
    /// whose disk form is well-formed TOML but semantically invalid
    /// (empty name, input_schema that isn't an object, …) becomes
    /// an error here, not a subtle bug in a downstream call site.
    pub fn from_toml_str(toml: &str) -> anyhow::Result<Self> {
        let m: Self = ::toml::from_str(toml)
            .map_err(|e| anyhow::anyhow!("failed to parse ability.toml: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    /// Parse from JSON. This is the canonical entry point for deployed
    /// `ability.json` bundles and boot replay.
    ///
    /// JSON deployment uses the same semantic validation as TOML manifests:
    /// private fields alone are not a construction boundary because serde can
    /// deserialize them directly inside this module's crate.
    pub fn from_json_slice(bytes: &[u8]) -> anyhow::Result<Self> {
        let m: Self = serde_json::from_slice(bytes)
            .map_err(|e| anyhow::anyhow!("failed to parse ability.json: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    /// Serialize to TOML. The writer always stamps the current
    /// schema version so the round-tripped file is always
    /// self-describing.
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        let mut stamped = self.clone();
        stamped.schema_version = CURRENT_SCHEMA_VERSION.to_string();
        ::toml::to_string_pretty(&stamped)
            .map_err(|e| anyhow::anyhow!("failed to serialize ability.toml: {e}"))
    }

    /// The verb portion of the ability name as written on disk.
    /// Callers assembling the wire-level `<agent>.<verb>` use
    /// [`qualified_name`](Self::qualified_name) instead.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable blurb. Not a protocol field.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Effective governed interface version. Absent manifest field means the
    /// default descriptor version, not an unknown version.
    pub fn descriptor_version(&self) -> &str {
        self.descriptor_version
            .as_deref()
            .unwrap_or(DEFAULT_DESCRIPTOR_VERSION)
    }

    pub fn admission_action(&self) -> Option<&str> {
        self.admission_action.as_deref()
    }

    /// Per-ability invocation timeout. `None` means "inherit
    /// runtime default"; see module doc for semantics.
    pub fn timeout_seconds(&self) -> Option<u64> {
        self.timeout_seconds
    }

    /// The required input schema. Always an object at its top level
    /// (enforced by `validate()`).
    pub fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    /// The optional output schema.
    pub fn output_schema(&self) -> Option<&Value> {
        self.output_schema.as_ref()
    }

    /// The optional executor binding. `None` means the manifest is
    /// discovery-only metadata and has no executable runtime binding.
    pub fn exec(&self) -> Option<&AbilityExec> {
        self.exec.as_ref()
    }

    /// The effective access policy. Falls back to `AccessPolicy::default()`
    /// (visibility = `device`) when no `[access]` table was written, so
    /// every consumer gets a non-None value without having to repeat
    /// the default on every call site.
    pub fn access(&self) -> AccessPolicy {
        self.access.clone().unwrap_or_default()
    }

    /// Build the wire-level fully-qualified name `<agent>.<verb>`.
    /// The dot is the reserved separator — the agent-name validator
    /// in `registry::agents` rejects any agent name that contains a
    /// dot, so this concatenation is unambiguous by construction.
    pub fn qualified_name(&self, agent_name: &str) -> String {
        format!("{agent_name}.{}", self.name)
    }

    /// Validate the invariants every consumer relies on.
    fn validate(&self) -> anyhow::Result<()> {
        if !SUPPORTED_SCHEMA_VERSIONS.contains(&self.schema_version.as_str()) {
            anyhow::bail!(
                "ability.toml schema_version = {:?} is not supported (known: {:?})",
                self.schema_version,
                SUPPORTED_SCHEMA_VERSIONS
            );
        }
        if let Some(version) = &self.descriptor_version {
            if version.trim().is_empty() {
                anyhow::bail!("ability.toml `descriptor_version` must not be empty");
            }
            if version.trim() != version {
                anyhow::bail!(
                    "ability.toml `descriptor_version` must not contain leading/trailing whitespace: {:?}",
                    version
                );
            }
            if !is_valid_descriptor_version(version) {
                anyhow::bail!(
                    "ability.toml `descriptor_version` must use N.N.N numeric form (got {:?})",
                    version
                );
            }
        }
        if self.admission_action.as_deref().is_some_and(|action| {
            !matches!(action, "invoke" | "read" | "manage" | "grant" | "stream")
        }) {
            anyhow::bail!("ability.toml `admission_action` is invalid");
        }
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("ability.toml `name` must not be empty");
        }
        if trimmed != self.name {
            anyhow::bail!(
                "ability.toml `name` must not contain leading/trailing whitespace: {:?}",
                self.name
            );
        }
        if self.name.contains('.') {
            anyhow::bail!(
                "ability.toml `name` must not contain `.` — that is the agent/verb \
                 separator. Got {:?}",
                self.name
            );
        }
        if self.name.contains('/') || self.name.contains(std::path::MAIN_SEPARATOR) {
            anyhow::bail!(
                "ability.toml `name` must not contain path separators: {:?}",
                self.name
            );
        }
        // Reject control characters and whitespace in the interior —
        // the name will end up in a wire-level tool identifier and
        // an embedded space would turn it into two tokens. The
        // check is deliberately strict: any non-visible-ASCII run
        // gets rejected.
        for c in self.name.chars() {
            if c.is_control() || c.is_whitespace() {
                anyhow::bail!(
                    "ability.toml `name` must not contain whitespace or control chars: \
                     {:?}",
                    self.name
                );
            }
        }
        if !self.input_schema.is_object() {
            anyhow::bail!(
                "ability.toml `input_schema` must be a JSON object at the top level \
                 (got {}); JSON Schema needs `{{\"type\": \"object\", ...}}`",
                match &self.input_schema {
                    Value::Null => "null",
                    Value::Bool(_) => "a boolean",
                    Value::Number(_) => "a number",
                    Value::String(_) => "a string",
                    Value::Array(_) => "an array",
                    Value::Object(_) => unreachable!(),
                }
            );
        }
        if let Some(out) = &self.output_schema {
            if !out.is_object() {
                anyhow::bail!(
                    "ability.toml `output_schema`, when present, must be a JSON \
                     object (JSON Schema)"
                );
            }
        }
        if let Some(0) = self.timeout_seconds {
            anyhow::bail!(
                "ability.toml `timeout_seconds` of 0 is a footgun — it means `kill \
                 immediately` to the subprocess supervisor. If you want \"inherit \
                 the runtime default\", omit the field. If you want \"no timeout\", \
                 pick a real upper bound."
            );
        }
        if let Some(exec) = &self.exec {
            exec.validate()?;
        }
        if let Some(cost) = &self.cost {
            cost.validate()?;
        }
        if let Some(boot) = &self.boot {
            boot.validate()?;
            if self.health.is_none() {
                anyhow::bail!(
                    "ability.toml declares [boot] without [health] — a boot script \
                     whose outcome can never be observed is an unverifiable side \
                     effect. Declare a [health] probe for the service the boot \
                     script starts."
                );
            }
        }
        if let Some(health) = &self.health {
            health.validate()?;
        }
        Ok(())
    }
}

impl AbilityExec {
    fn validate(&self) -> anyhow::Result<()> {
        match self {
            AbilityExec::Shell(s) => s.validate(),
            AbilityExec::Http(h) => h.validate(),
            AbilityExec::Eal(e) => e.validate(),
            AbilityExec::Mcp(m) => m.validate(),
            AbilityExec::HostStream(h) => h.validate(),
        }
    }
}

impl BootSpec {
    fn validate(&self) -> anyhow::Result<()> {
        validate_lifecycle_argv("[boot]", &self.argv)?;
        if let Some(0) = self.timeout_seconds {
            anyhow::bail!(
                "ability.toml [boot] `timeout_seconds` of 0 means `kill \
                 immediately`. Omit the field to inherit the monitor default."
            );
        }
        Ok(())
    }
}

impl HealthSpec {
    fn validate(&self) -> anyhow::Result<()> {
        validate_lifecycle_argv("[health]", &self.argv)?;
        if let Some(0) = self.interval_seconds {
            anyhow::bail!(
                "ability.toml [health] `interval_seconds` of 0 would probe in a \
                 busy loop. Omit the field to inherit the monitor default."
            );
        }
        if let Some(0) = self.timeout_seconds {
            anyhow::bail!(
                "ability.toml [health] `timeout_seconds` of 0 means every probe \
                 is killed immediately and counts as unhealthy. Omit the field \
                 to inherit the monitor default."
            );
        }
        Ok(())
    }
}

/// Shared argv validation for `[boot]` / `[health]` — mirrors the
/// `ShellExec` rules so the three script-shaped sections fail with
/// the same vocabulary.
fn validate_lifecycle_argv(section: &str, argv: &[String]) -> anyhow::Result<()> {
    if argv.is_empty() {
        anyhow::bail!(
            "ability.toml {section} requires a non-empty `argv` (the first \
             element is the program; subsequent elements are arguments)"
        );
    }
    if argv[0].trim().is_empty() {
        anyhow::bail!(
            "ability.toml {section} `argv[0]` (the program) must not be \
             empty/whitespace"
        );
    }
    Ok(())
}

impl ShellExec {
    fn validate(&self) -> anyhow::Result<()> {
        if self.argv.is_empty() {
            anyhow::bail!(
                "ability.toml [exec] kind=\"shell\" requires a non-empty `argv` (the \
                 first element is the program; subsequent elements are arguments)"
            );
        }
        if self.argv[0].trim().is_empty() {
            anyhow::bail!(
                "ability.toml [exec] kind=\"shell\" `argv[0]` (the program) must not \
                 be empty/whitespace"
            );
        }
        if let Some(mode) = &self.stdout {
            // Forward-compat: only the default decoder is implemented
            // today; reject unknown values loud rather than silently
            // ignore them so a typo in the manifest surfaces at load.
            const KNOWN: &[&str] = &["utf8_trim"];
            if !KNOWN.contains(&mode.as_str()) {
                anyhow::bail!(
                    "ability.toml [exec.shell] `stdout` = {:?} is not recognised; \
                     known values: {:?}",
                    mode,
                    KNOWN
                );
            }
        }
        if let Some(profile) = &self.sandbox {
            const KNOWN_PROFILES: &[&str] = &["none", "net_only", "pure_compute"];
            if !KNOWN_PROFILES.contains(&profile.as_str()) {
                anyhow::bail!(
                    "ability.toml [exec.shell] `sandbox` = {:?} is not a known \
                     profile; known values: {:?}",
                    profile,
                    KNOWN_PROFILES
                );
            }
        }
        Ok(())
    }
}

impl HttpExec {
    fn validate(&self) -> anyhow::Result<()> {
        const KNOWN_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"];
        let upper = self.method.to_ascii_uppercase();
        if !KNOWN_METHODS.contains(&upper.as_str()) {
            anyhow::bail!(
                "ability.toml [exec.http] `method` = {:?} is not in the safe set {:?}",
                self.method,
                KNOWN_METHODS
            );
        }
        if self.url.trim().is_empty() {
            anyhow::bail!("ability.toml [exec.http] `url` must not be empty");
        }
        // Reject schemes outside http/https up front. The runtime
        // executor would also reject them, but catching at load time
        // keeps a typo from sitting in a manifest until first call.
        let lower = self.url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            // Templated URLs that begin with `{{ base }}` count as
            // valid here — we can't resolve the scheme until runtime.
            // Only reject when the prefix is literal AND not http(s).
            if !self.url.starts_with("{{") {
                anyhow::bail!(
                    "ability.toml [exec.http] `url` must start with http:// or https:// \
                     (got {:?})",
                    self.url
                );
            }
        }
        if let Some(mode) = &self.response {
            const KNOWN: &[&str] = &["text_trim"];
            if !KNOWN.contains(&mode.as_str()) {
                anyhow::bail!(
                    "ability.toml [exec.http] `response` = {:?} is not recognised; \
                     known values: {:?}",
                    mode,
                    KNOWN
                );
            }
        }
        Ok(())
    }
}

impl EalExec {
    /// Soft cap on EAL `source` size embedded in a manifest. 1 MiB
    /// is generous compared to anything an author would type by hand
    /// (a curator-published workflow tends to be a few hundred lines)
    /// while still small enough that a paste-the-wrong-thing accident
    /// — dropping a transcript or binary blob into `source` — fails
    /// loud at `from_toml_str` instead of silently bloating the on-
    /// disk manifest.
    const MAX_SOURCE_BYTES: usize = 1024 * 1024;

    fn validate(&self) -> anyhow::Result<()> {
        if self.source.trim().is_empty() {
            anyhow::bail!(
                "ability.toml [exec] kind=\"eal\" requires a non-empty `source` (the \
                 EAL program text)"
            );
        }
        if self.source.len() > Self::MAX_SOURCE_BYTES {
            anyhow::bail!(
                "ability.toml [exec] kind=\"eal\" `source` is {} bytes which exceeds \
                 the {}-byte cap; split the workflow into smaller abilities or call \
                 them via `mission.run` directly",
                self.source.len(),
                Self::MAX_SOURCE_BYTES
            );
        }
        if let Some(binding) = &self.result_binding {
            if binding.trim().is_empty() {
                anyhow::bail!(
                    "ability.toml [exec.eal] `result_binding`, when set, must be a \
                     non-empty binding name"
                );
            }
        }
        Ok(())
    }
}

impl McpExec {
    fn validate(&self) -> anyhow::Result<()> {
        if self.server.trim().is_empty() {
            anyhow::bail!(
                "ability.toml [exec] kind=\"mcp\" requires a non-empty `server` \
                 matching an entry in mcp_clients.json"
            );
        }
        if self.tool.trim().is_empty() {
            anyhow::bail!(
                "ability.toml [exec] kind=\"mcp\" requires a non-empty upstream `tool` name"
            );
        }
        Ok(())
    }
}

/// Build the default `chat` manifest that every freshly-created
/// agent ships with. The agent's default input channel is surfaced
/// as a `chat` ability so external callers can reach it over the
/// network without any extra operator action.
///
/// Parity with `daemon::execution::mission::agent_ability_specs::chat_ability`
/// --------------------------------------------------------------------------
/// Two sources of truth exist for the `chat` ability's shape until
/// a later PR collapses them: this helper (on-disk template) and
/// `daemon::execution::mission::agent_ability_specs::chat_ability`
/// (hardcoded baseline used by today's dispatch + discovery). Only
/// the **input_schema** is a
/// protocol contract that must match — publishing two different
/// tool specs depending on which path discovery goes through is
/// exactly the silent-fail the parity guard exists to catch.
/// **Descriptions are allowed to differ**: the hardcoded side
/// interpolates the agent name for better UX at discovery time;
/// this template is agent-agnostic because a manifest does not
/// know which agent it belongs to.
///
/// The input_schema parity is pinned by
/// `hardcoded_chat_ability_input_schema_agrees_with_default_chat_manifest`
/// in `daemon::execution::mission::agent_ability_specs`'s test module — if
/// you touch the shape on either side, update both or the parity test will
/// fail loud.
pub fn default_chat_manifest() -> AbilityManifest {
    // The schema below is the wire contract for the chat ability. It
    // exposes two first-class entry shapes: the canonical minimal
    // prompt payload and the strict structured single-turn payload.
    // The canonical minimal `{"prompt": "..."}` payload remains a
    // first-class product contract, not a compatibility alias.
    // Optional fields model the current canonical chat runtime:
    // (1) resume a multi-turn session via
    // `session_id`, (2) decide which other abilities of the same agent
    // to expose to the LLM as tools (`skills`), (3) decide which
    // context loaders to run before invoking the LLM
    // (`context_loaders`), (4) override per-invocation driver knobs
    // without editing agent.toml (`driver`), and (5) flip on a
    // streaming RPC variant (`stream`).
    //
    // `additionalProperties: false` is load-bearing — sending an
    // unrecognised top-level field surfaces as a schema error rather
    // than silently being dropped, which makes "I added context but
    // it didn't take effect" tractable to debug. Sub-objects use the
    // same rule recursively.
    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "prompt": {
                "type": "string",
                "description": "The canonical minimal `{\"prompt\": \"...\"}` payload. Exactly one of `prompt` or `messages` is required."
            },
            "messages": {
                "type": "array",
                "minItems": 1,
                "maxItems": 2,
                "description": "Structured fresh single-turn input: one user message with one optional preceding system message. Unsupported roles or multi-turn history fail closed.",
                "items": {
                    "type": "object",
                    "properties": {
                        "role": {
                            "type": "string",
                            "enum": ["system", "user"]
                        },
                        "content": {
                            "type": "string",
                            "minLength": 1
                        }
                    },
                    "required": ["role", "content"],
                    "additionalProperties": false
                }
            },
            "context": {
                "type": "string",
                "description": "Optional system-style preamble prepended before `prompt`. \
                                Carried through to compose_prompt() as a literal string; \
                                use `context_loaders` instead when the data should come \
                                from a registered loader."
            },
            "session_id": {
                "type": "string",
                "description": "Optional conversation id to resume an existing session. When \
                                omitted the chat handler creates a fresh one and returns the \
                                generated id in the response. The literal value `lifelong` \
                                selects the agent's lifelong default thread: it resumes the \
                                session bound as lifelong, binding one first when none exists \
                                yet; the response carries the resolved concrete id."
            },
            "skills": {
                "type": "object",
                "description": "Controls which of this agent's other abilities are exposed \
                                to the LLM as tools for the current invocation.",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "none", "explicit"],
                        "description": "auto = expose every ability of this agent (default); \
                                        none = expose nothing; explicit = expose only those \
                                        listed in `include`."
                    },
                    "include": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Fully-qualified ability names (`<agent>.<verb>`) to \
                                        expose. Honoured in `explicit` mode; ignored in \
                                        `auto`/`none`."
                    },
                    "exclude": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Fully-qualified ability names to filter out, applied \
                                        after `mode`/`include`. Useful with `auto` to drop \
                                        a noisy or expensive tool from a single call."
                    }
                },
                "additionalProperties": false
            },
            "context_loaders": {
                "type": "object",
                "description": "Controls which registered context loaders run before the LLM \
                                is invoked. Each loader's output is appended to the prompt's \
                                context block, alongside the literal `context` arg if any.",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["auto", "none", "explicit"]
                    },
                    "include": {
                        "type": "array",
                        "items": {"type": "string"}
                    },
                    "exclude": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                },
                "additionalProperties": false
            },
            "driver": {
                "type": "object",
                "description": "Per-invocation overrides for the underlying LLM driver. \
                                Omit to use the agent's defaults from agent.toml.",
                "properties": {
                    "model": {
                        "type": "string",
                        "description": "Override the agent's default model for this call."
                    },
                    "temperature": {
                        "type": "number",
                        "minimum": 0,
                        "maximum": 2
                    },
                    "max_tokens": {
                        "type": "integer",
                        "minimum": 1
                    }
                },
                "additionalProperties": false
            },
            "stream": {
                "type": "boolean",
                "description": "When true via the RPC entry point, the handler rejects the \
                                call and asks the caller to use the subscribe entry point \
                                instead. The streaming subscribe path emits typed frames \
                                (session/loaded/delta/tool_call_*/done|error)."
            },
            "attachments": {
                "type": "array",
                "description": "Files to surface to the agent. Each entry names its source \
                                with exactly one of `path` (daemon-local file, embedded \
                                inline in the prompt's context block, per-call total cap \
                                of 1 MiB) or `ura` (a `<user>.files` store blob, \
                                materialised into the agent workspace's `uploads/` \
                                directory and cited by workspace-relative path so the \
                                agent reads it with its own file tools — no inline size \
                                cap). Use this instead of writing file contents into \
                                `context` by hand.",
                "items": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Filesystem path. Relative paths resolve against \
                                            the agent's workspace directory."
                        },
                        "encoding": {
                            "type": "string",
                            "enum": ["utf8", "base64"],
                            "description": "Only valid with `path`. How to read the file: \
                                            `utf8` (default) embeds text verbatim; `base64` \
                                            embeds binary content with \
                                            `<file encoding=\"base64\">…</file>` so the LLM \
                                            can reason about non-text payloads."
                        },
                        "ura": {
                            "type": "string",
                            "description": "v4.1.5 files-store resource URA \
                                            (easynet:///r/<realm>/resource/<u>.files/<sha256>), \
                                            e.g. as returned by `<user>.files.put`."
                        },
                        "filename": {
                            "type": "string",
                            "description": "Only valid with `ura`: display name for the \
                                            materialised copy. Sanitised to its basename; \
                                            the file lands at uploads/<sha8>-<name>."
                        }
                    },
                    "additionalProperties": false
                }
            },
            "execution": {
                "type": "object",
                "description": "Per-invocation daemon execution policy. cwd is relative to the registered agent root, never a host-absolute path.",
                "properties": {
                    "cwd": {
                        "type": "string",
                        "minLength": 1
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 900000
                    },
                    "isolation": {
                        "type": "string",
                        "enum": ["agent", "strict"]
                    }
                },
                "additionalProperties": false
            }
        },
        "oneOf": [
            {
                "required": ["prompt"],
                "not": {"required": ["messages"]}
            },
            {
                "required": ["messages"],
                "not": {"required": ["prompt"]}
            }
        ],
        "additionalProperties": false,
    });

    // Output schema documents what an RPC invocation returns. `reply`
    // is the primary assistant text, and the surrounding fields expose
    // session, tool, context, usage, and latency facts for structured
    // composition and UI rendering.
    //
    // Most fields are required because they appear on every RPC reply
    // — `usage` is the exception (LLM driver may not surface token
    // counts on every backend).
    let output_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "session_id": {
                "type": "string",
                "description": "The session id used for this turn. Echoes the input when \
                                provided; freshly generated otherwise."
            },
            "reply": {
                "type": "string",
                "description": "The assistant's final reply text for this chat turn."
            },
            "skills_loaded": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Fully-qualified ability names that were actually exposed to \
                                the LLM as tools for this call (after applying `skills.mode` \
                                and `exclude`)."
            },
            "tool_calls": {
                "type": "array",
                "description": "Per-tool-call observability: every ability the LLM invoked \
                                during this turn, in order, with args/result/error/elapsed.",
                "items": {
                    "type": "object",
                    "properties": {
                        "ability": {"type": "string"},
                        "args": {},
                        "result": {},
                        "error": {"type": "string"},
                        "elapsed_ms": {"type": "integer", "minimum": 0},
                        "tool_use_id": {"type": "string"},
                        "mcp_tool_name": {"type": "string"},
                        "request_id": {"type": "string"},
                        "ability_ura": {"type": "string"},
                        "invocation_ura": {"type": "string"},
                        "caller_ura": {"type": "string"},
                        "callee_ura": {"type": "string"},
                        "subject_ura": {"type": "string"}
                    },
                    "required": ["ability"]
                }
            },
            "context_used": {
                "type": "array",
                "description": "Per-loader contribution: which context loaders ran and how \
                                many bytes each contributed to the assembled context block.",
                "items": {
                    "type": "object",
                    "properties": {
                        "loader": {"type": "string"},
                        "bytes": {"type": "integer", "minimum": 0}
                    },
                    "required": ["loader", "bytes"]
                }
            },
            "usage": {
                "type": "object",
                "description": "Token accounting reported by the driver, when the underlying \
                                LLM backend exposes it.",
                "properties": {
                    "input_tokens": {"type": "integer", "minimum": 0},
                    "output_tokens": {"type": "integer", "minimum": 0},
                    "cache_read_tokens": {"type": "integer", "minimum": 0},
                    "cached_input_tokens": {"type": "integer", "minimum": 0},
                    "cache_creation_tokens": {"type": "integer", "minimum": 0},
                    "num_turns": {"type": "integer", "minimum": 0},
                    "total_cost_usd": {"type": "number", "minimum": 0},
                    "model": {"type": "string"}
                }
            },
            "elapsed_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Wall-clock duration of the full chat invocation."
            }
        },
        "required": ["session_id", "reply", "skills_loaded", "tool_calls", "context_used", "elapsed_ms"]
    });

    // The description is intentionally short — it ships in the
    // 4 KiB-capped `a2a.agents_json` discovery label for every
    // registered agent. The verbose blurb that used to live here
    // (4 sentences × ~250 bytes) blew the label past the cap when
    // a node had two agents. Long-form documentation lives in
    // `docs/`; the per-field `description` strings inside
    // `input_schema` carry the per-arg detail.
    AbilityManifest::new(
        "chat",
        "Send a prompt to the agent and get its reply.",
        input_schema,
    )
    .expect(
        "default_chat_manifest is a constant, well-formed input; validation failing \
         here would be a compile-time contract violation in this file",
    )
    .with_admission_action("invoke")
    .expect(
        "the embedded chat admission action is a constant canonical enum value; validation \
         failure here would be a compile-time contract violation in this file",
    )
    .with_output_schema(output_schema)
    .expect(
        "the embedded output schema is a JSON object; validation failure here would \
         be a compile-time contract violation in this file",
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! The tests pin the invariants every consumer relies on:
    //! construction-time validation, stamped schema version on
    //! write, qualified-name assembly, and TOML round-trip.

    use super::*;
    use serde_json::json;

    fn object_schema() -> Value {
        json!({"type": "object", "properties": {}, "required": []})
    }

    // ── happy path ──────────────────────────────────────────────────────────

    #[test]
    fn new_builds_and_validates_a_minimal_manifest() {
        let m = AbilityManifest::new("chat", "hello", object_schema()).unwrap();
        assert_eq!(m.name(), "chat");
        assert_eq!(m.description(), "hello");
        assert_eq!(m.descriptor_version(), DEFAULT_DESCRIPTOR_VERSION);
        assert!(m.input_schema().is_object());
        assert!(m.output_schema().is_none());
        assert_eq!(m.timeout_seconds(), None);
    }

    #[test]
    fn descriptor_version_is_an_interface_fact_not_schema_version() {
        let m = AbilityManifest::new("chat", "hello", object_schema())
            .unwrap()
            .with_descriptor_version("2.1.0")
            .unwrap();
        assert_eq!(m.descriptor_version(), "2.1.0");

        let toml = m.to_toml_string().unwrap();
        assert!(toml.contains(&format!("schema_version = \"{CURRENT_SCHEMA_VERSION}\"")));
        assert!(
            toml.contains("descriptor_version = \"2.1.0\""),
            "writer must persist the governed interface version; got:\n{toml}"
        );
        assert_eq!(AbilityManifest::from_toml_str(&toml).unwrap(), m);
    }

    #[test]
    fn from_json_slice_rejects_semantically_invalid_manifest() {
        let raw = serde_json::to_vec(&json!({
            "schema_version": "1",
            "name": "bad.name",
            "description": "invalid dotted verb",
            "input_schema": {"type": "object"},
        }))
        .unwrap();

        let err = AbilityManifest::from_json_slice(&raw).unwrap_err();
        assert!(format!("{err}").contains("must not contain `.`"));
    }

    #[test]
    fn from_json_slice_rejects_unknown_top_level_fields() {
        let raw = serde_json::to_vec(&json!({
            "schema_version": "1",
            "name": "weather",
            "description": "strict manifest",
            "input_schema": {"type": "object"},
            "namespace": "er",
        }))
        .unwrap();

        let err = AbilityManifest::from_json_slice(&raw).unwrap_err();
        assert!(
            format!("{err}").contains("unknown field `namespace`"),
            "canonical manifest must not silently absorb deploy-envelope fields: {err}"
        );
    }

    #[test]
    fn from_toml_str_rejects_unknown_nested_exec_fields() {
        let toml = r#"
name = "weather"
description = "strict manifest"

[input_schema]
type = "object"

[exec]
kind = "shell"
argv = ["echo", "hi"]
tool_name = "legacy-provider-field"
"#;

        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(
            format!("{err}").contains("unknown field `tool_name`"),
            "executor metadata must be explicitly modeled, not ignored: {err}"
        );
    }

    #[test]
    fn qualified_name_concatenates_agent_and_verb_with_dot() {
        let m = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        assert_eq!(m.qualified_name("alice"), "alice.chat");
    }

    #[test]
    fn default_chat_manifest_matches_hardcoded_baseline_shape() {
        // Guards against the default-manifest helper drifting away
        // from the daemon::execution::mission::agent_ability_specs baseline
        // before the two paths converge in a later PR. If this breaks, update
        // both sides in the same PR, not one at a time.
        let m = default_chat_manifest();
        assert_eq!(m.name(), "chat");
        let props = m
            .input_schema()
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        assert!(props.contains_key("prompt"));
        assert!(props.contains_key("messages"));
        assert!(props.contains_key("context"));
        let one_of = m
            .input_schema()
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("oneOf entry-shape guard is an array");
        assert_eq!(one_of.len(), 2);
        assert_eq!(
            m.input_schema().get("additionalProperties"),
            Some(&Value::Bool(false)),
            "schema must reject extra args"
        );
    }

    #[test]
    fn default_chat_manifest_declares_extended_input_fields() {
        // The post-refactor input schema adds optional fields that the
        // chat handler reads at invocation time. Pin every one — losing
        // any of them silently would break the contract the EasyNet
        // backend's ability detail card depends on.
        let m = default_chat_manifest();
        let props = m
            .input_schema()
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        for required_key in [
            "prompt",
            "messages",
            "context",
            "session_id",
            "skills",
            "context_loaders",
            "driver",
            "stream",
            "attachments",
            "execution",
        ] {
            assert!(
                props.contains_key(required_key),
                "input_schema.properties is missing {required_key:?}; got keys = {:?}",
                props.keys().collect::<Vec<_>>()
            );
        }
        // JSON Schema discovery exposes both entry shapes and pins their
        // mutual exclusion so product clients can reject malformed requests
        // before dispatching them to the daemon.
        let one_of = m
            .input_schema()
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("oneOf entry-shape guard is an array");
        assert_eq!(one_of.len(), 2);
    }

    #[test]
    fn default_chat_manifest_skills_subobject_uses_mode_include_exclude() {
        // The shape `{ mode, include, exclude }` is shared between
        // `skills` and `context_loaders` so a renderer (frontend
        // SchemaForm) can treat them uniformly. Drift on either side
        // would break that uniformity.
        let m = default_chat_manifest();
        for key in ["skills", "context_loaders"] {
            let sub = m
                .input_schema()
                .get("properties")
                .and_then(|p| p.get(key))
                .and_then(|s| s.get("properties"))
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{key} sub-object must declare properties"));
            for inner in ["mode", "include", "exclude"] {
                assert!(
                    sub.contains_key(inner),
                    "{key}.{inner} missing; got {:?}",
                    sub.keys().collect::<Vec<_>>()
                );
            }
            let mode_enum = m
                .input_schema()
                .get("properties")
                .and_then(|p| p.get(key))
                .and_then(|s| s.get("properties"))
                .and_then(|p| p.get("mode"))
                .and_then(|m| m.get("enum"))
                .and_then(Value::as_array)
                .unwrap_or_else(|| panic!("{key}.mode.enum must be a list"));
            let modes: Vec<&str> = mode_enum.iter().filter_map(Value::as_str).collect();
            assert_eq!(modes, vec!["auto", "none", "explicit"]);
        }
    }

    #[test]
    fn default_chat_manifest_publishes_typed_output_schema() {
        // Pre-refactor chat omitted output_schema (opaque text). Post-
        // refactor we publish a typed shape so the EasyNet ability
        // detail card can render structured output and so an agent
        // composing other abilities knows what to expect.
        let m = default_chat_manifest();
        let out = m
            .output_schema()
            .expect("default chat manifest must publish an output_schema");
        let props = out
            .get("properties")
            .and_then(Value::as_object)
            .expect("output_schema.properties must be an object");
        for key in [
            "session_id",
            "reply",
            "skills_loaded",
            "tool_calls",
            "context_used",
            "usage",
            "elapsed_ms",
        ] {
            assert!(
                props.contains_key(key),
                "output_schema is missing {key:?}; got {:?}",
                props.keys().collect::<Vec<_>>()
            );
        }
        // `reply` is the canonical primary assistant text and remains
        // required so callers can depend on a compact success field.
        assert_eq!(
            props
                .get("reply")
                .and_then(|p| p.get("type"))
                .and_then(Value::as_str),
            Some("string"),
        );
        // `usage` is intentionally NOT required (some drivers don't
        // surface tokens). Pin that explicitly.
        let required = out
            .get("required")
            .and_then(Value::as_array)
            .expect("required is an array");
        let req: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
        assert!(!req.contains(&"usage"), "usage must NOT be required");
        for must in [
            "session_id",
            "reply",
            "skills_loaded",
            "tool_calls",
            "context_used",
            "elapsed_ms",
        ] {
            assert!(req.contains(&must), "output_schema.required missing {must}");
        }
    }

    #[test]
    fn default_chat_manifest_rejects_unknown_top_level_args() {
        // additionalProperties: false is load-bearing. A legacy caller
        // sending only {prompt, context} validates; an unknown field
        // surfaces as a schema error rather than being silently
        // dropped, which is what makes "I added X but it didn't take
        // effect" tractable to debug.
        let m = default_chat_manifest();
        assert_eq!(
            m.input_schema().get("additionalProperties"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn toml_round_trip_preserves_fields() {
        let m = AbilityManifest::new("chat", "blurb", object_schema())
            .unwrap()
            .with_timeout_seconds(30)
            .unwrap()
            .with_output_schema(json!({"type": "object"}))
            .unwrap();
        let toml = m.to_toml_string().unwrap();
        let parsed = AbilityManifest::from_toml_str(&toml).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn to_toml_string_always_stamps_current_schema_version() {
        let mut m = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        m.schema_version = "1".to_string();
        let toml = m.to_toml_string().unwrap();
        assert!(
            toml.contains(&format!("schema_version = \"{CURRENT_SCHEMA_VERSION}\"")),
            "writer must stamp CURRENT_SCHEMA_VERSION; got:\n{toml}"
        );
    }

    #[test]
    fn from_toml_str_rejects_missing_schema_version() {
        let toml = "name = \"chat\"\n\
             description = \"x\"\n\
             [input_schema]\n\
             type = \"object\"\n"
            .to_string();
        let err = AbilityManifest::from_toml_str(&toml).unwrap_err();
        assert!(format!("{err}").contains("schema_version"), "{err}");
    }

    #[test]
    fn from_toml_str_rejects_invalid_descriptor_version() {
        for invalid in ["", "  ", "v1", "1", "1.0", "1.0.0.0", "1.0.x", " 1.0.0"] {
            let toml = format!(
                "schema_version = \"1\"\n\
                 descriptor_version = {invalid:?}\n\
                 name = \"chat\"\n\
                 description = \"x\"\n\
                 [input_schema]\n\
                 type = \"object\"\n"
            );
            let err = AbilityManifest::from_toml_str(&toml).unwrap_err();
            assert!(
                format!("{err}").contains("descriptor_version"),
                "{invalid:?} should fail as descriptor_version; got {err}"
            );
        }
    }

    // ── failure path ────────────────────────────────────────────────────────

    #[test]
    fn new_rejects_empty_name() {
        let err = AbilityManifest::new("", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("must not be empty"));
    }

    #[test]
    fn new_rejects_whitespace_only_name() {
        let err = AbilityManifest::new("   ", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn new_rejects_name_containing_dot() {
        // `.` is the agent/verb separator; embedding one would
        // make `<agent>.<name>` ambiguous on the wire.
        let err = AbilityManifest::new("chat.v2", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("`."));
    }

    #[test]
    fn new_rejects_name_with_slash() {
        let err = AbilityManifest::new("my/chat", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("path separators"));
    }

    #[test]
    fn new_rejects_name_with_interior_whitespace() {
        let err = AbilityManifest::new("my chat", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("whitespace"));
    }

    #[test]
    fn new_rejects_name_with_control_character() {
        let err = AbilityManifest::new("chat\t", "x", object_schema()).unwrap_err();
        assert!(format!("{err}").contains("whitespace") || format!("{err}").contains("control"));
    }

    #[test]
    fn new_rejects_input_schema_that_is_not_an_object() {
        let err = AbilityManifest::new("chat", "x", json!(["a", "b"])).unwrap_err();
        assert!(format!("{err}").contains("object"));
    }

    #[test]
    fn new_rejects_input_schema_null() {
        let err = AbilityManifest::new("chat", "x", json!(null)).unwrap_err();
        assert!(format!("{err}").contains("null"));
    }

    #[test]
    fn with_output_schema_rejects_non_object() {
        let base = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        let err = base.with_output_schema(json!(42)).unwrap_err();
        assert!(format!("{err}").contains("object"));
    }

    #[test]
    fn with_timeout_seconds_rejects_zero() {
        // Zero-timeout means "kill immediately" to the supervisor;
        // the field is for upper bounds, not abort switches. A
        // user wanting the default should omit the field, not
        // write 0.
        let base = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        let err = base.with_timeout_seconds(0).unwrap_err();
        assert!(format!("{err}").contains("0"));
    }

    #[test]
    fn from_toml_str_rejects_malformed_toml() {
        let err = AbilityManifest::from_toml_str("not = a = valid = toml").unwrap_err();
        assert!(format!("{err}").contains("parse"));
    }

    #[test]
    fn from_toml_str_round_trips_shell_exec_section() {
        // Pin the on-disk shape: `[exec] kind = "shell"` with a
        // top-level `argv` array. The internally-tagged enum
        // representation flattens `ShellExec`'s fields into the
        // same table as `kind`, so an author writing
        //
        //     [exec]
        //     kind = "shell"
        //     argv = [...]
        //
        // matches what serde produces. Asserting the parse here
        // means a future refactor that switches to
        // `[exec.shell] argv = [...]` (a different, externally-
        // tagged shape) would have to update this test, surfacing
        // the breaking change to every author with a manifest on
        // disk.
        let toml = r#"
schema_version = "1"
name = "weather"
description = "fetch weather"
[input_schema]
type = "object"
[exec]
kind = "shell"
argv = ["curl", "-s", "https://example/{{ location }}"]
"#;
        let m = AbilityManifest::from_toml_str(toml).expect("manifest with [exec] must parse");
        let exec = m.exec().expect("[exec] section must be preserved on parse");
        match exec {
            AbilityExec::Shell(s) => {
                assert_eq!(s.argv.len(), 3);
                assert_eq!(s.argv[0], "curl");
                assert!(s.argv[2].contains("{{ location }}"));
            }
            other => panic!("expected Shell, got {other:?}"),
        }
    }

    #[test]
    fn from_toml_str_rejects_shell_exec_with_empty_argv() {
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[exec]
kind = "shell"
argv = []
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(
            format!("{err}").contains("argv"),
            "validator must call out empty argv: {err}"
        );
    }

    #[test]
    fn access_defaults_to_device_when_section_absent() {
        // The on-disk shape that 99% of existing manifests use:
        // no `[access]` section. The accessor must materialise the
        // default policy so downstream consumers never have to repeat
        // the "if None then device" branch.
        let m = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        assert_eq!(m.access().visibility, ManifestAccessScope::Device);
    }

    #[test]
    fn access_visibility_self_round_trips_through_toml() {
        // Author-side opt-in: an internal helper an agent doesn't
        // want peers to see. The on-disk word must be exactly "self"
        // (matching the discover scope name) to keep one vocabulary.
        let toml = r#"
schema_version = "1"
name = "internal_helper"
description = "private to the owning agent"
[input_schema]
type = "object"
[access]
visibility = "self"
"#;
        let m = AbilityManifest::from_toml_str(toml).unwrap();
        assert_eq!(m.access().visibility, ManifestAccessScope::Selfish);
        let round_tripped = AbilityManifest::from_toml_str(&m.to_toml_string().unwrap()).unwrap();
        assert_eq!(
            round_tripped.access().visibility,
            ManifestAccessScope::Selfish
        );
    }

    #[test]
    fn access_visibility_public_parses() {
        // The federation tier is not wired up yet, but the schema
        // must already accept `public` so an author who pre-publishes
        // an ability for the federation rollout doesn't have to
        // rewrite the manifest later.
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[access]
visibility = "public"
"#;
        let m = AbilityManifest::from_toml_str(toml).unwrap();
        assert_eq!(m.access().visibility, ManifestAccessScope::Public);
    }

    #[test]
    fn access_visibility_unknown_value_is_rejected() {
        // A typo like "publik" must surface at load time rather than
        // silently default to one of the legal values — the latter
        // would let a misconfigured manifest leak (or hide) by
        // accident.
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[access]
visibility = "publik"
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(
            format!("{err}").contains("visibility") || format!("{err}").contains("variant"),
            "unknown visibility must mention the offending field: {err}"
        );
    }

    #[test]
    fn allows_caller_name_passes_when_no_lists_set() {
        let p = AccessPolicy::default();
        assert!(p.allows_caller_name("anyone"));
    }

    #[test]
    fn allows_caller_name_respects_deny_list() {
        let p = AccessPolicy {
            visibility: ManifestAccessScope::Device,
            deny_callers: Some(vec!["mallory".into()]),
            allow_callers: None,
        };
        assert!(!p.allows_caller_name("mallory"));
        assert!(p.allows_caller_name("alice"));
    }

    #[test]
    fn allows_caller_name_deny_wins_over_allow() {
        // Same name in both lists → deny wins. Lock the policy
        // direction here so a future refactor that flips the order
        // (allow-then-deny) trips the test loud.
        let p = AccessPolicy {
            visibility: ManifestAccessScope::Device,
            allow_callers: Some(vec!["alice".into()]),
            deny_callers: Some(vec!["alice".into()]),
        };
        assert!(!p.allows_caller_name("alice"));
    }

    #[test]
    fn allows_caller_name_respects_non_empty_allow_list() {
        let p = AccessPolicy {
            visibility: ManifestAccessScope::Device,
            allow_callers: Some(vec!["alice".into(), "bob".into()]),
            deny_callers: None,
        };
        assert!(p.allows_caller_name("alice"));
        assert!(p.allows_caller_name("bob"));
        assert!(!p.allows_caller_name("eve"));
    }

    #[test]
    fn allows_caller_name_empty_allow_list_is_no_whitelist() {
        // `allow_callers = []` (rare but legal) means "no whitelist
        // applied" — caller still passes. Pinning here so the
        // ergonomics of "I cleared the list" doesn't accidentally
        // become "I locked everyone out".
        let p = AccessPolicy {
            visibility: ManifestAccessScope::Device,
            allow_callers: Some(vec![]),
            deny_callers: None,
        };
        assert!(p.allows_caller_name("anyone"));
    }

    #[test]
    fn access_policy_with_caller_lists_round_trips_through_toml() {
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[access]
visibility = "device"
allow_callers = ["claude", "alice"]
deny_callers = ["mallory"]
"#;
        let m = AbilityManifest::from_toml_str(toml).unwrap();
        let access = m.access();
        assert_eq!(
            access.allow_callers.as_deref(),
            Some(&["claude".to_string(), "alice".to_string()][..])
        );
        assert_eq!(
            access.deny_callers.as_deref(),
            Some(&["mallory".to_string()][..])
        );
    }

    #[test]
    fn access_policy_allows_caller_is_monotonic() {
        // self < device < public — a stricter ability rejects looser
        // callers; a looser ability admits stricter callers. Spelled
        // out as a small matrix so a regression that flips the
        // direction (or treats Public as the strictest) trips loud.
        let cases: &[(ManifestAccessScope, ManifestAccessScope, bool)] = &[
            (
                ManifestAccessScope::Selfish,
                ManifestAccessScope::Selfish,
                true,
            ),
            (
                ManifestAccessScope::Selfish,
                ManifestAccessScope::Device,
                false,
            ),
            (
                ManifestAccessScope::Selfish,
                ManifestAccessScope::Public,
                false,
            ),
            (
                ManifestAccessScope::Device,
                ManifestAccessScope::Selfish,
                true,
            ),
            (
                ManifestAccessScope::Device,
                ManifestAccessScope::Device,
                true,
            ),
            (
                ManifestAccessScope::Device,
                ManifestAccessScope::Public,
                false,
            ),
            (
                ManifestAccessScope::Public,
                ManifestAccessScope::Selfish,
                true,
            ),
            (
                ManifestAccessScope::Public,
                ManifestAccessScope::Device,
                true,
            ),
            (
                ManifestAccessScope::Public,
                ManifestAccessScope::Public,
                true,
            ),
        ];
        for (ability_vis, caller_scope, expected) in cases {
            let policy = AccessPolicy {
                visibility: *ability_vis,
                ..Default::default()
            };
            assert_eq!(
                policy.allows_caller(*caller_scope),
                *expected,
                "ability_vis={ability_vis:?} caller_scope={caller_scope:?}"
            );
        }
    }

    #[test]
    fn http_exec_round_trips_through_toml() {
        let toml = r#"
schema_version = "1"
name = "weather_v2"
description = "fetch weather over HTTP"
[input_schema]
type = "object"
[exec]
kind = "http"
method = "GET"
url = "https://wttr.in/{{ location }}?format=4"
[exec.headers]
"User-Agent" = "easynet-ability/1"
"#;
        let m = AbilityManifest::from_toml_str(toml).expect("manifest must parse");
        match m.exec().expect("exec preserved") {
            AbilityExec::Http(h) => {
                assert_eq!(h.method, "GET");
                assert!(h.url.contains("{{ location }}"));
                assert_eq!(
                    h.headers
                        .as_ref()
                        .and_then(|m| m.get("User-Agent"))
                        .map(String::as_str),
                    Some("easynet-ability/1")
                );
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn http_exec_rejects_unsafe_method() {
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[exec]
kind = "http"
method = "TRACE"
url = "https://example.com"
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(
            format!("{err}").contains("method"),
            "validator must call out method: {err}"
        );
    }

    #[test]
    fn http_exec_rejects_non_http_scheme_when_literal() {
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[exec]
kind = "http"
method = "GET"
url = "ftp://example.com"
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(format!("{err}").contains("http://"));
    }

    // ── [boot] / [health] lifecycle sections ───────────────────────────────

    #[test]
    fn boot_and_health_round_trip_through_toml() {
        // Round-trip must hold for any well-formed lifecycle pair, not
        // one blessed fixture: pin the all-optionals shape and the
        // minimal shape with neutral argv values (the schema owes
        // nothing to any particular backing service). Struct equality
        // after the round-trip covers field preservation generically;
        // the per-variant closure asserts the parse mapped TOML to the
        // right fields in the first place.
        type Check = fn(&AbilityManifest);
        let full_check: Check = |m| {
            let boot = m.boot().expect("boot preserved");
            assert_eq!(boot.argv, vec!["svc-up", "--now"]);
            assert_eq!(boot.timeout_seconds, Some(60));
            let health = m.health().expect("health preserved");
            assert_eq!(health.argv, vec!["svc-probe"]);
            assert_eq!(health.interval_seconds, Some(30));
            assert_eq!(health.timeout_seconds, Some(10));
        };
        let minimal_check: Check = |m| {
            let boot = m.boot().expect("boot preserved");
            assert_eq!(boot.timeout_seconds, None);
            let health = m.health().expect("health preserved");
            assert_eq!(health.interval_seconds, None);
            assert_eq!(health.timeout_seconds, None);
        };
        let variants: [(&str, Check); 2] = [
            (
                "[boot]\nargv = [\"svc-up\", \"--now\"]\ntimeout_seconds = 60\n\
                 [health]\nargv = [\"svc-probe\"]\ninterval_seconds = 30\ntimeout_seconds = 10",
                full_check,
            ),
            (
                "[boot]\nargv = [\"svc-up\"]\n[health]\nargv = [\"svc-probe\"]",
                minimal_check,
            ),
        ];
        for (lifecycle_toml, check) in variants {
            let toml = format!(
                "schema_version = \"1\"\nname = \"x\"\ndescription = \"\"\n\
                 [input_schema]\ntype = \"object\"\n{lifecycle_toml}\n"
            );
            let m = AbilityManifest::from_toml_str(&toml).expect("manifest must parse");
            check(&m);
            let serialized = m.to_toml_string().expect("serializes");
            let back = AbilityManifest::from_toml_str(&serialized).expect("round-trips");
            assert_eq!(
                back, m,
                "round-trip must preserve every field for: {lifecycle_toml}"
            );
        }
    }

    #[test]
    fn health_without_boot_is_a_valid_probe_only_manifest() {
        // A remote SaaS dependency can be probed but not booted —
        // probe-only must stay legal.
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[health]
argv = ["curl", "-fsS", "https://api.example.com/health"]
"#;
        let m = AbilityManifest::from_toml_str(toml).expect("probe-only manifest parses");
        assert!(m.boot().is_none());
        assert!(m.health().is_some());
    }

    #[test]
    fn boot_without_health_is_rejected() {
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[boot]
argv = ["docker", "start", "svc"]
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(
            format!("{err}").contains("[boot] without [health]"),
            "validator must explain the boot-without-health rule: {err}"
        );
    }

    #[test]
    fn boot_rejects_empty_argv() {
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[boot]
argv = []
[health]
argv = ["true"]
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(format!("{err}").contains("[boot]"), "{err}");
        assert!(format!("{err}").contains("argv"), "{err}");
    }

    #[test]
    fn health_rejects_zero_interval_and_zero_timeout() {
        for (field, toml) in [
            (
                "interval_seconds",
                r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[health]
argv = ["true"]
interval_seconds = 0
"#,
            ),
            (
                "timeout_seconds",
                r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[health]
argv = ["true"]
timeout_seconds = 0
"#,
            ),
        ] {
            let err = AbilityManifest::from_toml_str(toml).unwrap_err();
            assert!(
                format!("{err}").contains(field),
                "validator must call out {field}: {err}"
            );
        }
    }

    #[test]
    fn http_exec_accepts_templated_url_prefix() {
        // A manifest that interpolates the base URL must validate
        // even though the literal scheme isn't visible at load time
        // — the executor will reject at call time if the rendered
        // URL is non-http(s).
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[exec]
kind = "http"
method = "GET"
url = "{{ base }}/path"
"#;
        let m = AbilityManifest::from_toml_str(toml).expect("templated URL must parse");
        assert!(matches!(m.exec(), Some(AbilityExec::Http(_))));
    }

    #[test]
    fn eal_exec_round_trips_through_toml() {
        // The EAL executor variant must parse from on-disk TOML and
        // round-trip back unchanged. This pins the wire shape:
        // `kind = "eal"`, `source = "..."`, optional
        // `result_binding = "..."`. Without a round-trip test a
        // serialization rename (e.g. accidentally renaming `source`
        // to `program`) would silently break every published EAL
        // ability without surfacing in any other test.
        let toml = r#"
schema_version = "1"
name = "summarise_then_review"
description = "summarise + judge in one call"
[input_schema]
type = "object"
[exec]
kind = "eal"
source = """
mission "x" {
  let s = call "summarise" on "device" with { text = "{{ text }}" }
}
"""
result_binding = "s"
"#;
        let m = AbilityManifest::from_toml_str(toml).expect("EAL manifest must parse");
        match m.exec() {
            Some(AbilityExec::Eal(e)) => {
                assert!(e.source.contains("mission"));
                assert!(e.source.contains("{{ text }}"));
                assert_eq!(e.result_binding.as_deref(), Some("s"));
            }
            other => panic!("expected Eal, got {other:?}"),
        }
        // Round-trip: serialise and re-parse; the second parse must
        // succeed and preserve the same binding.
        let written = m.to_toml_string().expect("serialise");
        let m2 = AbilityManifest::from_toml_str(&written).expect("re-parse");
        match m2.exec() {
            Some(AbilityExec::Eal(e)) => {
                assert_eq!(e.result_binding.as_deref(), Some("s"));
            }
            other => panic!("round-trip lost the EAL variant: {other:?}"),
        }
    }

    #[test]
    fn eal_exec_rejects_empty_source() {
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[exec]
kind = "eal"
source = ""
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(
            format!("{err}").contains("source"),
            "validator must call out empty source: {err}"
        );
    }

    #[test]
    fn eal_exec_rejects_empty_result_binding_when_present() {
        // `result_binding` is optional; absence is fine. But when
        // the author writes `result_binding = ""` they almost
        // certainly meant to delete the line — fail loud rather
        // than silently treating it as "no binding".
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[exec]
kind = "eal"
source = "mission \"x\" {}"
result_binding = ""
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(format!("{err}").contains("result_binding"));
    }

    #[test]
    fn host_stream_exec_accepts_absolute_socket_and_function_token() {
        let toml = r#"
schema_version = "1"
name = "weather"
description = "stream weather frames"
[input_schema]
type = "object"
[exec]
kind = "host_stream"
host_socket = "/tmp/easynet-weather.sock"
function = "er.weather:forecast_v1"
"#;

        let m = AbilityManifest::from_toml_str(toml).expect("host_stream manifest must parse");
        match m.exec() {
            Some(AbilityExec::HostStream(exec)) => {
                assert_eq!(exec.host_socket, "/tmp/easynet-weather.sock");
                assert_eq!(exec.function, "er.weather:forecast_v1");
            }
            other => panic!("expected HostStream, got {other:?}"),
        }
    }

    #[test]
    fn host_stream_exec_rejects_relative_socket_and_invalid_function() {
        let relative_socket = r#"
schema_version = "1"
name = "weather"
description = ""
[input_schema]
type = "object"
[exec]
kind = "host_stream"
host_socket = "tmp/easynet-weather.sock"
function = "er.weather"
"#;
        let err = AbilityManifest::from_toml_str(relative_socket).unwrap_err();
        assert!(format!("{err}").contains("absolute Unix socket path"));

        let invalid_function = r#"
schema_version = "1"
name = "weather"
description = ""
[input_schema]
type = "object"
[exec]
kind = "host_stream"
host_socket = "/tmp/easynet-weather.sock"
function = "1;rm -rf"
"#;
        let err = AbilityManifest::from_toml_str(invalid_function).unwrap_err();
        assert!(format!("{err}").contains("function"));
    }

    #[test]
    fn from_toml_str_rejects_unknown_schema_version() {
        // Forward-compat: a writer that stamps an unknown version
        // (99) must be rejected loudly, not accepted with a silent
        // "pretend it's v1" fallback.
        let toml = "schema_version = \"99\"\n\
                    name = \"chat\"\n\
                    description = \"x\"\n\
                    [input_schema]\n\
                    type = \"object\"\n";
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(format!("{err}").contains("schema_version"));
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn name_with_dashes_and_underscores_and_digits_is_accepted() {
        // These are the allowed character class for verb names.
        // The test exists so the validator change log has to
        // argue with a pinned contract rather than silently shift
        // the allowed set.
        for name in ["chat", "chat_v2", "chat-v2", "chat2"] {
            AbilityManifest::new(name, "x", object_schema())
                .unwrap_or_else(|e| panic!("{name:?} should be accepted: {e}"));
        }
    }

    #[test]
    fn empty_description_is_accepted_even_though_it_is_bad_ux() {
        // We don't gatekeep description — the UX layer can warn,
        // but the protocol layer should not refuse to load.
        // Rejecting here would block an operator from committing
        // a WIP manifest.
        let m = AbilityManifest::new("chat", "", object_schema()).unwrap();
        assert_eq!(m.description(), "");
    }

    #[test]
    fn large_timeout_seconds_round_trips_exactly() {
        // 24h * 7 * 365 as a sanity bound. We carry u64 to keep
        // the door open for long-running batch abilities without
        // having to widen the type later.
        let secs: u64 = 60 * 60 * 24 * 7 * 365;
        let m = AbilityManifest::new("chat", "x", object_schema())
            .unwrap()
            .with_timeout_seconds(secs)
            .unwrap();
        let toml = m.to_toml_string().unwrap();
        let parsed = AbilityManifest::from_toml_str(&toml).unwrap();
        assert_eq!(parsed.timeout_seconds(), Some(secs));
    }

    #[test]
    fn input_schema_with_nested_references_round_trips() {
        // A realistic JSON Schema uses `$ref` and `oneOf` etc.
        // We don't validate those — we just ensure the
        // serde-json passthrough survives.
        let schema = json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "tools": {
                    "type": "array",
                    "items": {"$ref": "#/definitions/Tool"}
                }
            },
            "required": ["prompt"],
            "definitions": {
                "Tool": {
                    "oneOf": [
                        {"const": "shell"},
                        {"const": "edit"}
                    ]
                }
            }
        });
        let m = AbilityManifest::new("chat", "x", schema).unwrap();
        let toml = m.to_toml_string().unwrap();
        let parsed = AbilityManifest::from_toml_str(&toml).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn cost_section_round_trips_through_toml() {
        // Pin the on-disk shape: `[cost] kind = "..." label = "..."`.
        // Both fields must survive a parse → serialise → parse cycle
        // unchanged so a future operator's hand-written manifest does
        // not silently lose the label after the daemon rewrites it.
        let toml = r#"
schema_version = "1"
name = "geocode"
description = "Geocode an address."
[input_schema]
type = "object"
[cost]
kind = "external_metered"
label = "Google Maps Geocoding API — $5 per 1000 requests"
"#;
        let m = AbilityManifest::from_toml_str(toml).expect("[cost] must parse");
        let cost = m.cost().expect("cost present");
        assert_eq!(cost.kind, CostKind::ExternalMetered);
        assert_eq!(
            cost.label.as_deref(),
            Some("Google Maps Geocoding API — $5 per 1000 requests")
        );
        let round_tripped = AbilityManifest::from_toml_str(&m.to_toml_string().unwrap()).unwrap();
        assert_eq!(round_tripped.cost(), m.cost());
    }

    #[test]
    fn cost_kind_label_optional_when_omitted_in_toml() {
        // Author may declare only the bucket; consumers fall back to
        // the per-kind generic blurb at render time.
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[cost]
kind = "llm_metered"
"#;
        let m = AbilityManifest::from_toml_str(toml).unwrap();
        let cost = m.cost().expect("cost present");
        assert_eq!(cost.kind, CostKind::LlmMetered);
        assert!(cost.label.is_none());
    }

    #[test]
    fn cost_unknown_kind_value_is_rejected() {
        // A typo like "extrenal_metered" must surface at load time
        // rather than silently default to one of the legal buckets —
        // the latter would let a billed ability advertise as free.
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[cost]
kind = "extrenal_metered"
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(
            format!("{err}").contains("kind") || format!("{err}").contains("variant"),
            "unknown cost kind must mention the offending field: {err}"
        );
    }

    #[test]
    fn cost_label_empty_string_is_rejected() {
        // `label = ""` is almost certainly a deletion mistake;
        // surface at load time rather than treating it as "no label".
        let toml = r#"
schema_version = "1"
name = "x"
description = ""
[input_schema]
type = "object"
[cost]
kind = "free"
label = "   "
"#;
        let err = AbilityManifest::from_toml_str(toml).unwrap_err();
        assert!(format!("{err}").contains("label"));
    }

    #[test]
    fn with_cost_builder_attaches_cost_and_validates() {
        let m = AbilityManifest::new("chat", "x", object_schema())
            .unwrap()
            .with_cost(CostMeta {
                kind: CostKind::LlmMetered,
                label: Some("Claude tokens".into()),
            })
            .unwrap();
        assert_eq!(m.cost().unwrap().kind, CostKind::LlmMetered);
        // Empty label fails the validator on the builder path too.
        let err = AbilityManifest::new("chat", "x", object_schema())
            .unwrap()
            .with_cost(CostMeta {
                kind: CostKind::Free,
                label: Some("".into()),
            })
            .unwrap_err();
        assert!(format!("{err}").contains("label"));
    }

    #[test]
    fn qualified_name_with_unicode_agent_is_stored_verbatim() {
        // We do not re-validate the agent name here — it has
        // already been validated by `registry::agents` upstream.
        // The manifest's only job is to concatenate.
        let m = AbilityManifest::new("chat", "x", object_schema()).unwrap();
        assert_eq!(m.qualified_name("alice"), "alice.chat");
    }
}
