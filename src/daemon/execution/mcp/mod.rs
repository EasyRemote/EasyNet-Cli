// EasyNet CLI — McpClientService (C-M9b)
// =======================================
//
// File: src/daemon/execution/mcp_client/mod.rs
//
// Sub-service that owns every outbound MCP server connection the
// daemon spawns. Mirrors PtyService's process-wide-Arc shape: one
// service handle, lazily-instantiated child connections, indexed
// by server name.
//
// Wire protocol
// -------------
// MCP stdio transport: JSON-RPC 2.0 messages sent over the child's
// stdin/stdout. Historical SDKs used one JSON object per line;
// newer transports may use `Content-Length` frames. McpServerSpec
// carries `stdio_framing` so operators can match the upstream.
// The two methods we use are:
//
//   tools/list  → returns { tools: [{ name, description, inputSchema }] }
//   tools/call  → returns { content: [...], isError: bool }
//
// We send `initialize` once on first connection (per the MCP
// spec's handshake) and then drive the two RPCs above. v1 ignores
// the `notifications/*` channel — we don't need progress / cancel
// for the first cut.
//
// Concurrency model
// -----------------
// One background **listener task** per stdio server connection.
// The listener owns the child's stdout and runs a permanent loop
// that decodes one MCP frame at a time and dispatches it:
//
//   * **Response** (`id` field present) → look up the matching
//     `oneshot::Sender` in `pending_responses` and deliver. This
//     unblocks the `McpConnection::rpc` caller that registered it
//     before writing the request.
//   * **Notification** (no `id` field) → broadcast to every entry
//     in `notification_sinks`. The hot-reload path attaches a
//     `RegistryRefreshSink` here so `notifications/tools/list_changed`
//     can trigger a `refresh_server` without piggybacking on a live
//     RPC. The pre-listener architecture only received notifications
//     mid-call, so an idle daemon never noticed an upstream catalogue
//     change — exactly the gap hot-reload exists to close.
//
// Writes still serialise behind a `Mutex<ChildStdin>` because the
// JSON-RPC wire format requires whole-frame writes to be atomic.
// Reads no longer borrow `&mut self`: every caller registers an
// `oneshot` and `await`s the listener instead. That means multiple
// pipelined RPCs against the same server can be in flight at once
// (the listener resolves them in any order by id).
//
// On child crash the listener drains every `pending_responses` entry
// with an error before exiting, so no RPC caller is left awaiting
// forever. Subsequent `rpc` calls fail fast because the listener
// `JoinHandle` has finished and `pending_responses` is empty.
//
// Why not use an external mcp-client crate
// ----------------------------------------
// MCP is two RPC methods over stdio JSON-RPC 2.0. Pulling a crate
// would import schemas, capability negotiation, transport
// abstraction layers we don't use. Hand-rolling the minimum keeps
// the dependency surface clean and the wire shape pinned to the
// MCP spec we actually exercise.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Config row for one upstream MCP server. Mirrors the shape of
/// `~/.claude/mcp_servers.json` so an operator who already runs
/// MCP can drop their existing config in. Fields:
///
///   * `name`        — short identifier the operator picks; ability
///                     callers reference it by this string.
///   * `command`     — executable to spawn for stdio transport
///                     (e.g. `"node"`, `"python"`, `"npx"`). Empty
///                     string is valid only when `transport == "http"`.
///   * `args`        — argv tail (stdio only).
///   * `env`         — extra env vars merged with the daemon's env
///                     (stdio only).
///   * `stdio_framing` — `"line"` (default, compatible with MCP-Bench
///                     SDK 1.x servers) or `"content-length"`.
///   * `transport`   — `"stdio"` (default) or `"http"`.
///   * `url`         — base URL for the HTTP transport
///                     (e.g. `"http://127.0.0.1:3001"`). Required
///                     when `transport == "http"`.
///   * `endpoint`    — HTTP path for the MCP endpoint; defaults to
///                     `"/mcp"` per the streamable-HTTP spec.
///   * `name_prefix` — applied to every tool name when this server's
///                     tools are reflectively registered as abilities.
///                     Required when an operator runs ≥2 upstreams
///                     whose tool catalogues could collide. Empty
///                     prefix (default) means no rewriting.
///   * `aliases`     — per-tool rename map (`upstream_name` →
///                     `local_name`). Applied AFTER `name_prefix`
///                     bypass: an alias wins over the prefix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerSpec {
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_stdio_framing")]
    pub stdio_framing: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name_prefix: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub aliases: HashMap<String, String>,
    /// TLS configuration for `https://` URLs. Default (empty) uses
    /// the Mozilla CA bundle bundled via `webpki-roots`. Operators
    /// can override the trust roots (private CA), pin a server name
    /// for SNI when it differs from the URL host, or — for closed
    /// test environments only — skip verification entirely.
    /// Plain `http://` URLs ignore this field.
    #[serde(default, skip_serializing_if = "TlsSpec::is_empty")]
    pub tls: TlsSpec,
    /// Authentication credentials presented on every outgoing request
    /// (both POST and the GET listener). None means no Authorization /
    /// auth headers are sent. The variants cover the three real cases
    /// MCP server operators actually use; production deployments that
    /// need anything more elaborate (OAuth refresh flows etc.) belong
    /// behind a sidecar that converts to one of these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthSpec>,
}

/// TLS configuration for an HTTPS MCP upstream. Empty default means
/// "use Mozilla roots, verify normally, derive SNI from URL".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TlsSpec {
    /// Optional path to a PEM file containing one or more CA
    /// certificates. When set, these are loaded *in addition to*
    /// the default Mozilla roots (not instead of) — operators with
    /// a private CA do not lose ability to reach public MCP
    /// servers from the same daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_bundle: Option<std::path::PathBuf>,
    /// Override the SNI / certificate hostname used during the TLS
    /// handshake. Defaults to the host component of `url`. Needed
    /// when the URL host is a bare IP, a CNAME, or otherwise does
    /// not match the cert's SAN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    /// **DANGER**: disables certificate verification entirely. Allowed
    /// for closed test environments only. The streamable HTTP client
    /// logs a warning to stderr at every connection when this is set,
    /// so it's hard to forget. Not honored unless the daemon was
    /// started with the `EASYNET_ALLOW_INSECURE_TLS=1` env var; this
    /// double-gate prevents an attacker who can write the config file
    /// from silently downgrading TLS.
    #[serde(default, skip_serializing_if = "is_false")]
    pub insecure_skip_verify: bool,
}

impl TlsSpec {
    /// Whether this spec has any non-default fields. Used by
    /// `skip_serializing_if` so an operator who didn't set any TLS
    /// fields doesn't get a `tls = {}` line in their dumped config.
    fn is_empty(&self) -> bool {
        self.ca_bundle.is_none() && self.server_name.is_none() && !self.insecure_skip_verify
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Auth credential variants for HTTP/HTTPS MCP upstreams.
///
/// Tag-based serde representation (`{"type": "bearer", "token": "..."}`)
/// keeps the TOML / JSON readable and lets new variants land without
/// breaking older configs — unknown variants fail at config load with
/// a typed error rather than silently parsing as a different shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthSpec {
    /// Static bearer token. Appears as `Authorization: Bearer <token>`.
    ///
    /// **Discouraged.** A non-empty `token` lands in the config file
    /// verbatim and is therefore included in every backup, every
    /// shell-history capture of `cat mcp_clients.json`, every
    /// support-bundle scrape. `validate()` emits a
    /// `kind=auth_plaintext_bearer level=warn` op-event each time it
    /// sees this variant with a non-empty token so operators get a
    /// loud reminder at boot. Prefer [`AuthSpec::BearerEnv`], which
    /// sources the token from an env var at request time and keeps
    /// the secret out of every config-file lifecycle.
    Bearer { token: String },
    /// Bearer token sourced from an environment variable at request
    /// time. The env var name is config (non-secret); the token
    /// itself never lands in config files. Missing env var fails the
    /// request with a typed error.
    BearerEnv { env: String },
    /// Arbitrary header map. Used by upstreams that key on a custom
    /// header (`X-Api-Key`, `MCP-Tenant-Id`, …). Multiple entries are
    /// applied in declared order; collisions with the standard
    /// `Authorization` header are allowed (operator's responsibility).
    Headers { headers: HashMap<String, String> },
}

fn default_transport() -> String {
    "stdio".to_string()
}

fn default_stdio_framing() -> String {
    "line".to_string()
}

fn default_endpoint() -> String {
    "/mcp".to_string()
}

impl Default for McpServerSpec {
    /// Cheap default used by existing call sites and tests that
    /// construct stdio specs in-line. Yields a stdio-transport spec
    /// with empty name/command — callers MUST overwrite both fields
    /// before use; `validate()` will refuse the empty default.
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            stdio_framing: default_stdio_framing(),
            transport: default_transport(),
            url: None,
            endpoint: default_endpoint(),
            name_prefix: String::new(),
            aliases: HashMap::new(),
            tls: TlsSpec::default(),
            auth: None,
        }
    }
}

impl McpServerSpec {
    /// Compute the local ability name for an upstream tool name.
    ///
    /// Resolution order:
    ///   1. If the operator declared an explicit alias for this tool
    ///      in `aliases`, use that verbatim. Aliases are the highest-
    ///      precedence rename — they let an operator pick a clean
    ///      local name even for an upstream whose default name would
    ///      collide.
    ///   2. Otherwise, prepend `name_prefix` (which may be empty, in
    ///      which case the upstream name passes through unchanged).
    ///
    /// The output is the bare local ability tail; the caller (the
    /// reflective registry) is what turns this into a full URA.
    pub fn apply_local_name(&self, upstream_tool: &str) -> String {
        if let Some(alias) = self.aliases.get(upstream_tool) {
            return alias.clone();
        }
        if self.name_prefix.is_empty() {
            upstream_tool.to_string()
        } else {
            format!("{}{}", self.name_prefix, upstream_tool)
        }
    }

    /// Validate the spec at config-load time so an operator's typo
    /// surfaces at boot, not at the first call. Returns Ok(()) when
    /// the spec is internally consistent for its declared transport.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.is_empty() {
            anyhow::bail!("MCP server spec missing `name`");
        }
        match self.transport.as_str() {
            "stdio" => {
                if self.command.is_empty() {
                    anyhow::bail!(
                        "MCP server `{}`: stdio transport requires non-empty `command`",
                        self.name
                    );
                }
                match self.stdio_framing.as_str() {
                    "line" | "content-length" => {}
                    other => anyhow::bail!(
                        "MCP server `{}`: unknown stdio_framing `{}` (expected line | content-length)",
                        self.name,
                        other
                    ),
                }
            }
            "http" => {
                let Some(url) = self.url.as_deref() else {
                    anyhow::bail!("MCP server `{}`: http transport requires `url`", self.name);
                };
                let lowered = url.to_ascii_lowercase();
                let is_https = lowered.starts_with("https://");
                if self.tls.insecure_skip_verify && !is_https {
                    // No point asking for TLS shortcuts on a plain HTTP URL —
                    // surface this as a typo at boot, not silently ignore.
                    anyhow::bail!(
                        "MCP server `{}`: tls.insecure_skip_verify set on an http:// URL",
                        self.name
                    );
                }
                if !is_https && self.tls.ca_bundle.is_some() {
                    anyhow::bail!(
                        "MCP server `{}`: tls.ca_bundle set on an http:// URL",
                        self.name
                    );
                }
                match &self.auth {
                    Some(AuthSpec::BearerEnv { env }) if env.is_empty() => {
                        anyhow::bail!(
                            "MCP server `{}`: auth.bearer_env requires non-empty `env`",
                            self.name
                        );
                    }
                    Some(AuthSpec::Bearer { token }) if !token.is_empty() => {
                        // Loud reminder at boot. We do not bail —
                        // there are legitimate uses (closed test
                        // setups, CI fixtures) and operator habit
                        // forms slowly. Warn every boot so the
                        // warning cannot be silenced by adding it to
                        // a file the operator only looks at once.
                        let server_name = self.name.as_str();
                        crate::op_event!(
                            component = mcp_http_client,
                            kind = auth_plaintext_bearer,
                            level = "warn",
                            server = server_name,
                            message = "AuthSpec::Bearer { token } persists the secret in mcp_clients.json; \
                                       prefer AuthSpec::BearerEnv to keep the token out of config files",
                        );
                    }
                    Some(AuthSpec::Headers { headers }) if !headers.is_empty() => {
                        // Boot-time validation of every header pair.
                        // hyper's request builder would otherwise
                        // reject a malformed name / CRLF-injected
                        // value at first-call time with a generic
                        // `hyper::http::Error`; surfacing it here
                        // turns the operator's typo into a loud
                        // refusal at config load, with the server
                        // name and the offending header attached.
                        // (`HeaderName::from_bytes` is the same path
                        // `Request::header(name, _)` walks; matching
                        // it here is the exact invariant the runtime
                        // path will enforce, only earlier.)
                        for (name, value) in headers {
                            hyper::header::HeaderName::from_bytes(name.as_bytes())
                                .map_err(|e| anyhow::anyhow!(
                                    "MCP server `{}`: auth.headers entry `{}` is not a valid HTTP header name ({e})",
                                    self.name,
                                    name,
                                ))?;
                            hyper::header::HeaderValue::from_str(value)
                                .map_err(|e| anyhow::anyhow!(
                                    "MCP server `{}`: auth.headers value for `{}` is not a valid HTTP header value ({e}); \
                                     ASCII-printable only — embedded CR/LF/NUL are refused to block header injection",
                                    self.name,
                                    name,
                                ))?;
                        }
                        // Same risk surface as plaintext Bearer.
                        // `Headers` is the right shape for upstreams
                        // that key on `X-Api-Key` / `MCP-Tenant-Id`,
                        // but a value embedded here is just as
                        // persisted as a plaintext bearer token.
                        let server_name = self.name.as_str();
                        crate::op_event!(
                            component = mcp_http_client,
                            kind = auth_plaintext_headers,
                            level = "warn",
                            server = server_name,
                            header_count = headers.len(),
                            message = "AuthSpec::Headers persists every header value in mcp_clients.json; \
                                       audit each entry to confirm none are secrets",
                        );
                    }
                    _ => {}
                }
            }
            other => anyhow::bail!(
                "MCP server `{}`: unknown transport `{}` (expected stdio | http)",
                self.name,
                other
            ),
        }
        Ok(())
    }
}

/// Top-level config file shape. The `servers` array is the single
/// source of truth for which upstreams are reachable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpClientsFile {
    #[serde(default)]
    pub servers: Vec<McpServerSpec>,
}

impl McpClientsFile {
    /// Look up a server spec by name; returns the first match.
    pub fn get(&self, name: &str) -> Option<&McpServerSpec> {
        self.servers.iter().find(|s| s.name == name)
    }
}

/// Map of in-flight request ids → caller-owned oneshot senders. The
/// listener task fills the sender when the matching response frame
/// arrives. On listener exit (child crash, EOF) every remaining entry
/// is drained with an error so callers don't hang forever.
type PendingResponses = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// Per-server registry of long-lived notification observers. Distinct
/// from the per-RPC `&mut dyn NotificationSink` parameter the
/// pre-listener architecture used: entries here see notifications
/// **at any time**, including while the daemon is idle. The hot-reload
/// sink lives here.
///
/// Keyed by a u64 handle so a transient subscriber (e.g.
/// `rpc_with_progress` registering itself for the duration of one
/// call) can `unregister` precisely without disturbing the long-lived
/// sinks.
type NotificationSinks = Arc<Mutex<HashMap<u64, Box<dyn NotificationSink + Send>>>>;

/// One live outbound MCP connection.
///
/// **Listener model.** A background task owns the child's stdout and
/// dispatches frames into `pending_responses` (for id-keyed responses)
/// or `notification_sinks` (for id-less notifications). RPC callers
/// register an `oneshot::Sender` in `pending_responses` *before*
/// writing the request, then await its receiver — so the listener
/// can deliver responses in any order. Writes still serialise behind
/// `stdin` because JSON-RPC frames must be written atomically.
struct McpConnection {
    /// Serialised write half. JSON-RPC frames are written
    /// header+body and must not interleave across concurrent
    /// callers, so we lock per write.
    stdin: Mutex<ChildStdin>,
    /// Monotonic request-id counter. Atomic so multiple in-flight
    /// `rpc` calls can each grab a fresh id without contending on
    /// the stdin lock just to read+increment.
    next_id: AtomicU64,
    /// Carried for diagnostic context only — actual framing is set
    /// up at spawn time and lives inside the listener task.
    stdio_framing: String,
    /// id → caller oneshot. Shared with the listener task; both
    /// sides take a tokio mutex to insert / remove.
    pending_responses: PendingResponses,
    /// Long-lived observers for notification frames. Shared with the
    /// listener task; the daemon registers the hot-reload sink here
    /// at boot.
    notification_sinks: NotificationSinks,
    /// Listener task handle. Dropping `McpConnection` aborts it so
    /// the child stdio fds are released. Kept private — callers
    /// drive the connection via `rpc` / `register_sink`.
    listener: tokio::task::JoinHandle<()>,
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        // Listener owns the child's stdout half; if we don't abort,
        // it could keep the fd open after the connection is gone.
        self.listener.abort();
    }
}

/// One MCP notification observed mid-call. Currently only the
/// notifications/progress family is parsed; other notification
/// methods are surfaced with their full method name + params so
/// future hooks can attach (notifications/tools/list_changed,
/// notifications/cancelled, ...).
#[derive(Debug, Clone)]
pub struct ObservedNotification {
    pub method: String,
    pub params: Value,
}

impl ObservedNotification {
    /// Helper: extract the progress payload when this notification
    /// is `notifications/progress`. Returns `None` otherwise.
    pub fn as_progress(&self) -> Option<ProgressFrame> {
        if self.method != "notifications/progress" {
            return None;
        }
        let p = self.params.as_object()?;
        let token = p.get("progressToken").cloned()?;
        let progress = p.get("progress").and_then(Value::as_f64)?;
        let total = p.get("total").and_then(Value::as_f64);
        let message = p
            .get("message")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        Some(ProgressFrame {
            token,
            progress,
            total,
            message,
        })
    }
}

/// Parsed shape of one `notifications/progress` frame, per
/// MCP spec 2025-06-18 §"Progress".
#[derive(Debug, Clone)]
pub struct ProgressFrame {
    pub token: Value,
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

/// Optional sink for in-flight notifications. The caller of an
/// `rpc_with_progress` call passes one; the connection routes every
/// non-response frame to it (per MCP spec, servers MAY interleave
/// `notifications/progress`, `notifications/tools/list_changed`, etc.
/// before the eventual response). The default `rpc()` discards
/// notifications, preserving existing call sites unchanged.
pub trait NotificationSink: Send {
    fn observe(&mut self, note: ObservedNotification);
}

impl McpConnection {
    /// Send a JSON-RPC request and await its response.
    ///
    /// Allocates a fresh id, registers a oneshot in `pending_responses`,
    /// writes the request, then awaits the listener's delivery of the
    /// matching response. Notifications observed between request and
    /// response are routed to the long-lived sinks in
    /// `notification_sinks` — the per-call sink parameter that the
    /// pre-listener architecture used is gone, because notifications
    /// no longer live "in the rpc loop" (they may arrive at any time,
    /// including before the request was even written).
    async fn rpc(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        // Register BEFORE writing — if the response arrives faster
        // than we can write+await, we still catch it.
        {
            let mut pending = self.pending_responses.lock().await;
            pending.insert(id, tx);
        }
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let write_result = {
            let mut stdin = self.stdin.lock().await;
            write_stdio_message(&mut stdin, &self.stdio_framing, &req).await
        };
        if let Err(e) = write_result {
            // Pull our entry back out so a future response (won't come
            // — child saw partial write) doesn't pile up entries.
            self.pending_responses.lock().await.remove(&id);
            return Err(e);
        }

        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(reason)) => anyhow::bail!("MCP server returned JSON-RPC error: {reason}"),
            // Listener exited (child crashed, EOF, abort). It drains
            // every pending entry with `Err` before exiting; getting
            // here means the channel was simply dropped without a
            // value, which can happen if `listener.abort()` cancelled
            // mid-iteration. Surface a consistent error.
            Err(_) => anyhow::bail!(
                "MCP listener task ended before response to `{method}` arrived; \
                 upstream connection lost"
            ),
        }
    }

    /// Subscribe a long-lived observer to this connection's
    /// notification stream. Used by `RegistryRefreshSink` to react to
    /// `notifications/tools/list_changed` even when no RPC is in
    /// flight, and by `rpc_with_progress` to forward progress frames
    /// for the duration of one call. Returns a handle the caller
    /// uses to `unregister_sink` when done — long-lived registrations
    /// (the hot-reload sink) simply keep the handle alive for the
    /// daemon's lifetime.
    async fn register_sink(&self, sink: Box<dyn NotificationSink + Send>) -> u64 {
        // Use a fresh range so handles never collide with request ids.
        // next_id is monotone so we just borrow its counter.
        let handle = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut sinks = self.notification_sinks.lock().await;
        sinks.insert(handle, sink);
        handle
    }

    /// Drop a previously-registered sink. No-op if the handle was
    /// already removed (idempotent — safe to call twice from a
    /// `Drop` guard).
    async fn unregister_sink(&self, handle: u64) {
        let mut sinks = self.notification_sinks.lock().await;
        sinks.remove(&handle);
    }
}

/// One row in the per-process server registry. Wraps the
/// connection (None when not yet established) and the spec the
/// operator declared.
struct McpServerRow {
    spec: McpServerSpec,
    /// stdio connection. `None` until the first call lazily spawns
    /// the child. Subsequent calls reuse the connection. A future
    /// health-check could clear this entry on stdio failure to
    /// trigger re-spawn; v1 surfaces the failure to the caller.
    /// Only populated when `spec.transport == "stdio"`.
    conn: Option<Arc<McpConnection>>,
    /// Streamable HTTP connection. `None` until the first call
    /// performs the initialize handshake. Carries the session id +
    /// negotiated protocol version, both of which the spec REQUIRES
    /// on every subsequent request. Only populated when
    /// `spec.transport == "http"`.
    http_conn: Option<http::HttpConnection>,
}

/// Process-wide outbound MCP client registry. Cloneable handle.
#[derive(Clone)]
pub struct McpClientService {
    inner: Arc<Mutex<McpClientServiceInner>>,
}

struct McpClientServiceInner {
    servers: HashMap<String, McpServerRow>,
}

impl Default for McpClientService {
    fn default() -> Self {
        Self::new()
    }
}

impl McpClientService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(McpClientServiceInner {
                servers: HashMap::new(),
            })),
        }
    }

    /// Canonical operator config path for outbound MCP clients.
    /// Shared by daemon boot, the MCP executor, and CLI authoring
    /// commands so every surface reads the same server catalogue.
    pub fn default_config_path() -> PathBuf {
        std::env::var("EASYNET_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(".easynet")
            })
            .join("mcp_clients.json")
    }

    /// Drop every cached upstream connection (stdio child handles,
    /// listener tasks, HTTP clients). The next call per server
    /// reconnects lazily on the CALLER's runtime.
    ///
    /// Why this exists: boot-time eager reflection drives
    /// `reflect_all` on a temporary helper runtime
    /// (`mcp_reflective_registry::run_eager_blocking`). Connections
    /// born there die with that runtime — a serve-time reuse touches
    /// a shut-down tokio context ("A Tokio 1.x context was found,
    /// but it is being shutdown"). Reflection callers reset the
    /// cache before their helper runtime drops.
    pub async fn reset_connections(&self) {
        let mut g = self.inner.lock().await;
        for row in g.servers.values_mut() {
            row.conn = None;
            row.http_conn = None;
        }
    }

    /// Construct from an in-memory file (test path or operator-
    /// supplied snapshot). Production callers prefer `from_path`.
    pub fn from_file(file: McpClientsFile) -> Self {
        let svc = Self::new();
        let mut g = svc
            .inner
            .try_lock()
            .expect("fresh service has no contention");
        for spec in file.servers {
            g.servers.insert(
                spec.name.clone(),
                McpServerRow {
                    spec,
                    conn: None,
                    http_conn: None,
                },
            );
        }
        drop(g);
        svc
    }

    /// Read a config file from disk. Missing file → empty service
    /// (no upstream MCP servers configured); parse error
    /// propagates so the operator sees their typo at boot rather
    /// than as silent "no servers."
    pub fn from_path(path: &PathBuf) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::new());
        }
        let bytes =
            std::fs::read(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        let file: McpClientsFile = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
        for spec in &file.servers {
            spec.validate()
                .map_err(|e| anyhow::anyhow!("invalid spec in {}: {e}", path.display()))?;
        }
        Ok(Self::from_file(file))
    }

    /// Returns the spec for an upstream server, if configured.
    /// Used by the reflective registry to apply name_prefix / aliases
    /// without poking through the lock-guarded inner map directly.
    pub async fn spec(&self, name: &str) -> Option<McpServerSpec> {
        let g = self.inner.lock().await;
        g.servers.get(name).map(|row| row.spec.clone())
    }

    /// List the names of every configured upstream server. Cheap
    /// (no I/O); the names come straight off the static config.
    pub async fn server_names(&self) -> Vec<String> {
        let g = self.inner.lock().await;
        let mut names: Vec<String> = g.servers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Lazily ensure a connection to `name` is alive, then send a
    /// JSON-RPC request. Returns the JSON-RPC `result` field.
    ///
    /// Transport dispatch:
    ///   * `transport == "stdio"` (default): spawn child on first
    ///     call, send initialize, then send the requested method;
    ///     subsequent calls reuse the live stdio connection.
    ///   * `transport == "http"`: send initialize over HTTP POST to
    ///     <url><endpoint> on first call (captures session id +
    ///     protocol version), then send each subsequent method as a
    ///     fresh POST. Per MCP 2025-06-18 streamable HTTP §"Sending
    ///     Messages", every JSON-RPC message is a new POST; the
    ///     session is identified by the `Mcp-Session-Id` header.
    pub async fn rpc(&self, name: &str, method: &str, params: Value) -> anyhow::Result<Value> {
        match self.rpc_once(name, method, params.clone()).await {
            Err(err) if is_dead_runtime_context(&err) => {
                // Self-healing reconnect: a cached connection whose
                // owning runtime has shut down (born on a temporary
                // bridge runtime — boot reflection, hot refresh, any
                // future sync→async bridge) surfaces tokio's
                // "context ... being shutdown" from its dead child
                // pipes or listener task. The CALLER's runtime is
                // alive (we are polling on it) — drop the dead
                // connection and reconnect here, once. This heals
                // every creation path instead of patching each
                // bridge call site.
                crate::op_event!(
                    component = mcp_client,
                    kind = dead_runtime_connection_recycled,
                    server = name,
                );
                self.invalidate_connection(name).await;
                self.rpc_once(name, method, params).await
            }
            other => other,
        }
    }

    async fn invalidate_connection(&self, name: &str) {
        let mut g = self.inner.lock().await;
        if let Some(row) = g.servers.get_mut(name) {
            row.conn = None;
            row.http_conn = None;
        }
    }

    async fn rpc_once(&self, name: &str, method: &str, params: Value) -> anyhow::Result<Value> {
        let mut g = self.inner.lock().await;
        let row = g.servers.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!("no upstream MCP server configured with name `{name}`")
        })?;
        match row.spec.transport.as_str() {
            "stdio" => {
                if row.conn.is_none() {
                    let conn = spawn_and_initialize(&row.spec).await?;
                    row.conn = Some(Arc::new(conn));
                }
                // Listener model: clone the Arc handle, release the
                // service-wide mutex, then await the response. Holding
                // `g` for the entire await would serialise every
                // server's traffic behind one lock — exactly what the
                // listener architecture exists to avoid.
                let conn = row.conn.as_ref().expect("conn just set").clone();
                drop(g);
                conn.rpc(method, params).await
            }
            "http" => {
                if row.http_conn.is_none() {
                    let conn = http::HttpConnection::initialize(&row.spec).await?;
                    row.http_conn = Some(conn);
                }
                let conn = row.http_conn.as_mut().expect("http_conn just set");
                conn.rpc(method, params).await
            }
            other => anyhow::bail!(
                "MCP server `{name}` has unknown transport `{other}` — \
                 only stdio and http are supported by McpClientService"
            ),
        }
    }

    /// Like `rpc`, but routes any `notifications/*` frames the
    /// upstream interleaves through `sink` BEFORE the eventual
    /// response. Use this when invoking an upstream tool that
    /// supports `notifications/progress` (MCP spec 2025-06-18
    /// §"Progress") and the caller wants to surface progress to
    /// its own consumer.
    ///
    /// HTTP transport routes intervening `notifications/*` frames
    /// out of the SSE response stream into `sink` before returning
    /// the terminal JSON-RPC response; see [`http::HttpConnection::
    /// rpc_with_sink`]. Stdio transport mirrors the same contract
    /// via [`McpConnection::rpc_with_sink`].
    pub async fn rpc_with_progress(
        &self,
        name: &str,
        method: &str,
        params: Value,
        sink: &mut dyn NotificationSink,
    ) -> anyhow::Result<Value> {
        let mut g = self.inner.lock().await;
        let row = g.servers.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!("no upstream MCP server configured with name `{name}`")
        })?;
        match row.spec.transport.as_str() {
            "stdio" => {
                if row.conn.is_none() {
                    let conn = spawn_and_initialize(&row.spec).await?;
                    row.conn = Some(Arc::new(conn));
                }
                let conn = row.conn.as_ref().expect("conn just set").clone();
                drop(g);
                // The listener model demands long-lived sinks
                // (`Box<dyn NotificationSink + Send>`). The caller
                // hands us a borrowed `&mut dyn` — we bridge the gap
                // with an unbounded mpsc: register a forwarding sink
                // on the connection, then `select!` between the
                // notification stream and the RPC response. The sink
                // is unregistered the moment the RPC resolves (or
                // errors out) so the forwarder doesn't outlive the
                // caller's borrow.
                let (note_tx, mut note_rx) = mpsc::unbounded_channel::<ObservedNotification>();
                struct ForwardSink(mpsc::UnboundedSender<ObservedNotification>);
                impl NotificationSink for ForwardSink {
                    fn observe(&mut self, note: ObservedNotification) {
                        // Caller may have already dropped (the rpc
                        // resolved first). Drop quietly.
                        let _ = self.0.send(note);
                    }
                }
                let handle = conn.register_sink(Box::new(ForwardSink(note_tx))).await;
                let rpc_fut = conn.rpc(method, params);
                tokio::pin!(rpc_fut);
                let result = loop {
                    tokio::select! {
                        // Bias toward draining notifications first
                        // when both are ready so progress frames
                        // observed concurrently with the response
                        // don't get silently swallowed.
                        biased;
                        Some(note) = note_rx.recv() => {
                            sink.observe(note);
                        }
                        r = &mut rpc_fut => break r,
                    }
                };
                // Drain any remaining notifications the listener
                // queued just before the response — these arrived
                // while the rpc was being delivered to the oneshot
                // and we want the caller's sink to see them too.
                while let Ok(note) = note_rx.try_recv() {
                    sink.observe(note);
                }
                conn.unregister_sink(handle).await;
                result
            }
            "http" => {
                // SSE-aware: the streamable HTTP client decodes any
                // intervening `notifications/*` frames out of the
                // SSE stream and routes them through `sink` before
                // returning the terminal response. Servers that
                // return plain `application/json` (no notifications)
                // still work — the sink is simply never invoked.
                if row.http_conn.is_none() {
                    let conn = http::HttpConnection::initialize(&row.spec).await?;
                    row.http_conn = Some(conn);
                }
                let conn = row.http_conn.as_mut().expect("http_conn just set");
                conn.rpc_with_sink(method, params, sink).await
            }
            other => anyhow::bail!(
                "MCP server `{name}` has unknown transport `{other}` — \
                 only stdio and http are supported by McpClientService"
            ),
        }
    }

    /// Register a long-lived notification observer on the named
    /// stdio server. Used by `RegistryRefreshSink` to react to
    /// `notifications/tools/list_changed` outside of any in-flight
    /// RPC. Returns `None` if the server is not configured or its
    /// transport is not stdio (HTTP notifications don't flow through
    /// this surface — they come back inside the SSE response to the
    /// originating RPC). Lazily spawns the child if needed, mirroring
    /// the `rpc` lazy-initialise contract.
    pub async fn register_notification_sink(
        &self,
        name: &str,
        sink: Box<dyn NotificationSink + Send>,
    ) -> anyhow::Result<u64> {
        let mut g = self.inner.lock().await;
        let row = g.servers.get_mut(name).ok_or_else(|| {
            anyhow::anyhow!("no upstream MCP server configured with name `{name}`")
        })?;
        if row.spec.transport != "stdio" {
            anyhow::bail!(
                "register_notification_sink: `{name}` is `{}` transport; \
                 only stdio supports long-lived sinks in this build",
                row.spec.transport
            );
        }
        if row.conn.is_none() {
            let conn = spawn_and_initialize(&row.spec).await?;
            row.conn = Some(Arc::new(conn));
        }
        let conn = row.conn.as_ref().expect("conn just set").clone();
        drop(g);
        Ok(conn.register_sink(sink).await)
    }
}

/// Streamable HTTP transport for outbound MCP. Per MCP 2025-06-18
/// §"Streamable HTTP". Lives in its own submodule so the stdio
/// implementation above stays readable.
///
/// Public surface is [`http::HttpConnection`]; internal helpers
/// (TLS connector, auth header injection, SSE parsers, hyper IO
/// shim) are `pub(super)` inside that module.
pub mod http;

impl std::fmt::Debug for McpClientService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClientService").finish_non_exhaustive()
    }
}

/// tokio's stable diagnostic for touching a resource whose owning
/// runtime is gone (`tokio::util::error::RUNTIME_SHUTTING_DOWN_ERROR`,
/// surfaced through child pipes / dropped listener tasks of a
/// connection born on a since-dropped bridge runtime). String-matched
/// because tokio types it as a plain io::Error.
fn is_dead_runtime_context(err: &anyhow::Error) -> bool {
    format!("{err:#}").contains("but it is being shutdown")
}

/// Spawn the configured child and send the MCP `initialize`
/// handshake. Returns the live connection ready for `tools/*`
/// calls.
async fn spawn_and_initialize(spec: &McpServerSpec) -> anyhow::Result<McpConnection> {
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args)
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn `{}`: {e}", spec.command))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("MCP child has no stdin pipe"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("MCP child has no stdout pipe"))?;
    // Detach: we own the child's stdio but not its lifetime.
    // Dropping `child` here lets it run until close (the OS
    // reaper claims it on exit).
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    let pending_responses: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
    let notification_sinks: NotificationSinks = Arc::new(Mutex::new(HashMap::new()));

    // Hand stdout to the listener task — it owns the read side from
    // here on. Clones of pending/sinks let the connection (the writer
    // side) talk to the listener via shared state rather than a
    // channel; both directions need atomic access to those maps.
    let listener_pending = pending_responses.clone();
    let listener_sinks = notification_sinks.clone();
    let listener_framing = spec.stdio_framing.clone();
    let listener_server_name = spec.name.clone();
    let listener = tokio::spawn(async move {
        run_listener(
            BufReader::new(stdout),
            listener_framing,
            listener_server_name,
            listener_pending,
            listener_sinks,
        )
        .await;
    });

    let conn = McpConnection {
        stdin: Mutex::new(stdin),
        next_id: AtomicU64::new(1),
        stdio_framing: spec.stdio_framing.clone(),
        pending_responses,
        notification_sinks,
        listener,
    };
    // MCP `initialize` handshake. We claim a minimal client
    // capability surface (no sampling, no roots) — enough to drive
    // tools/list and tools/call.
    let _ = conn
        .rpc(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "easynet-daemon",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )
        .await?;
    // Per spec the client MUST send the `notifications/initialized`
    // notification after the initialize round-trip. We send it as a
    // best-effort bare notification (no `id` field, no response).
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    {
        let mut stdin_guard = conn.stdin.lock().await;
        write_stdio_message(&mut stdin_guard, &conn.stdio_framing, &notif).await?;
    }
    Ok(conn)
}

/// Background reader. Owns the child's stdout half for the lifetime
/// of the connection. Decodes one MCP frame per iteration and routes
/// it: id-keyed responses go to the matching oneshot in
/// `pending_responses`; id-less notifications fan out to every entry
/// in `notification_sinks`. Exits on read error (child stdout EOF /
/// pipe broken / decoder failure), draining all outstanding pending
/// entries with an error so callers don't hang.
async fn run_listener(
    mut stdout: BufReader<ChildStdout>,
    framing: String,
    server_name: String,
    pending_responses: PendingResponses,
    notification_sinks: NotificationSinks,
) {
    loop {
        let frame = read_stdio_message(&mut stdout, &framing, "<listener>").await;
        let v = match frame {
            Ok(v) => v,
            Err(e) => {
                // Drain every outstanding caller with the same error
                // so nobody awaits forever.
                let reason = format!("MCP listener for `{server_name}` exiting: {e}");
                let mut pending = pending_responses.lock().await;
                for (_, tx) in pending.drain() {
                    let _ = tx.send(Err(reason.clone()));
                }
                return;
            }
        };

        let resp_id = v.get("id").and_then(Value::as_u64);
        if let Some(id) = resp_id {
            // Response (success or JSON-RPC error). Pull caller out
            // of the table and deliver.
            let tx = {
                let mut pending = pending_responses.lock().await;
                pending.remove(&id)
            };
            if let Some(tx) = tx {
                let payload = if let Some(err) = v.get("error") {
                    Err(err.to_string())
                } else {
                    Ok(v.get("result").cloned().unwrap_or(Value::Null))
                };
                // Receiver may have been dropped (caller gave up /
                // timed out). Best-effort send.
                let _ = tx.send(payload);
            }
            // Orphan response (no caller registered) — silently drop.
            // This happens after `rpc` already wrote+awaited and got
            // a duplicate or out-of-order frame from a buggy upstream.
            continue;
        }

        // Notification — broadcast to every long-lived sink.
        let note_method = v
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let note_params = v.get("params").cloned().unwrap_or(Value::Null);
        let note = ObservedNotification {
            method: note_method,
            params: note_params,
        };
        let mut sinks = notification_sinks.lock().await;
        for sink in sinks.values_mut() {
            sink.observe(note.clone());
        }
    }
}

async fn write_stdio_message(
    stdin: &mut ChildStdin,
    framing: &str,
    value: &Value,
) -> anyhow::Result<()> {
    if framing == "line" {
        let line = format!("{}\n", serde_json::to_string(value)?);
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        return Ok(());
    }

    let body = serde_json::to_vec(value)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stdin.write_all(header.as_bytes()).await?;
    stdin.write_all(&body).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_stdio_message(
    stdout: &mut BufReader<ChildStdout>,
    framing: &str,
    method: &str,
) -> anyhow::Result<Value> {
    if framing == "line" {
        return read_stdio_line(stdout, method).await;
    }
    read_mcp_frame(stdout, method).await
}

async fn read_stdio_line(
    stdout: &mut BufReader<ChildStdout>,
    method: &str,
) -> anyhow::Result<Value> {
    loop {
        let mut buf = String::new();
        let n = stdout.read_line(&mut buf).await?;
        if n == 0 {
            anyhow::bail!("MCP server closed stdout before responding to `{method}`");
        }
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        return Ok(v);
    }
}

async fn read_mcp_frame(
    stdout: &mut BufReader<ChildStdout>,
    method: &str,
) -> anyhow::Result<Value> {
    loop {
        let mut content_length = None;

        loop {
            let mut line = String::new();
            let n = stdout.read_line(&mut line).await?;
            if n == 0 {
                anyhow::bail!("MCP server closed stdout before responding to `{method}`");
            }

            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                if let Some(len) = content_length {
                    let mut body = vec![0_u8; len];
                    stdout.read_exact(&mut body).await?;
                    return serde_json::from_slice(&body)
                        .map_err(|e| anyhow::anyhow!("MCP response was not valid JSON: {e}"));
                }
                break;
            }

            let Some((name, value)) = trimmed.split_once(':') else {
                // Some broken servers log on stdout before the first
                // frame. Ignore the line and keep looking for a
                // Content-Length header rather than treating it as a
                // hard protocol failure.
                continue;
            };
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(value.trim().parse::<usize>().map_err(|e| {
                    anyhow::anyhow!("invalid MCP Content-Length `{}`: {e}", value.trim())
                })?);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_service_has_no_servers() {
        let svc = McpClientService::new();
        let names = futures::executor::block_on(svc.server_names());
        assert!(names.is_empty());
    }

    #[test]
    fn from_file_indexes_servers_by_name() {
        let file = McpClientsFile {
            servers: vec![
                McpServerSpec {
                    name: "alpha".into(),
                    command: "echo".into(),
                    args: vec!["alpha".into()],
                    ..Default::default()
                },
                McpServerSpec {
                    name: "beta".into(),
                    command: "echo".into(),
                    ..Default::default()
                },
            ],
        };
        let svc = McpClientService::from_file(file);
        let names = futures::executor::block_on(svc.server_names());
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn from_path_missing_file_yields_empty_service() {
        let p = PathBuf::from("/tmp/eznt-mcp-clients-doesnotexist-test.json");
        let svc = McpClientService::from_path(&p).expect("missing file is OK");
        let names = futures::executor::block_on(svc.server_names());
        assert!(names.is_empty());
    }

    #[test]
    fn from_path_malformed_json_propagates_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mcp.json");
        std::fs::write(&p, b"not json at all").unwrap();
        let err = McpClientService::from_path(&p).unwrap_err();
        assert!(format!("{err}").contains("parse"));
    }

    #[test]
    fn mcp_clients_file_serde_roundtrip() {
        // Pin the on-disk schema so a refactor that renamed a field
        // would trip this and force an explicit migration story.
        let file = McpClientsFile {
            servers: vec![McpServerSpec {
                name: "context7".into(),
                command: "npx".into(),
                args: vec!["-y".into(), "@upstash/context7-mcp".into()],
                env: HashMap::from([("API_KEY".into(), "x".into())]),
                ..Default::default()
            }],
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: McpClientsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.servers.len(), 1);
        assert_eq!(back.servers[0].name, "context7");
        assert_eq!(back.servers[0].args, vec!["-y", "@upstash/context7-mcp"]);
        // New fields default sensibly so pre-existing config files
        // continue to deserialise unchanged.
        assert_eq!(back.servers[0].transport, "stdio");
        assert_eq!(back.servers[0].stdio_framing, "line");
        assert_eq!(back.servers[0].endpoint, "/mcp");
        assert!(back.servers[0].url.is_none());
        assert!(back.servers[0].name_prefix.is_empty());
        assert!(back.servers[0].aliases.is_empty());
    }

    #[test]
    fn apply_local_name_passes_through_when_no_prefix_or_alias() {
        let spec = McpServerSpec {
            name: "weather".into(),
            command: "python".into(),
            ..Default::default()
        };
        assert_eq!(spec.apply_local_name("get_forecast"), "get_forecast");
    }

    #[test]
    fn apply_local_name_prepends_prefix() {
        let spec = McpServerSpec {
            name: "context7".into(),
            command: "node".into(),
            name_prefix: "ctx7.".into(),
            ..Default::default()
        };
        assert_eq!(spec.apply_local_name("search_docs"), "ctx7.search_docs");
    }

    #[test]
    fn apply_local_name_alias_beats_prefix() {
        // Aliases are the highest-precedence rename — they let an
        // operator pin a clean local name regardless of the prefix.
        let spec = McpServerSpec {
            name: "context7".into(),
            command: "node".into(),
            name_prefix: "ctx7.".into(),
            aliases: HashMap::from([("search_docs".into(), "docs.search".into())]),
            ..Default::default()
        };
        assert_eq!(spec.apply_local_name("search_docs"), "docs.search");
        // Non-aliased tools still get the prefix.
        assert_eq!(spec.apply_local_name("get_version"), "ctx7.get_version");
    }

    #[test]
    fn validate_stdio_requires_command() {
        let mut spec = McpServerSpec {
            name: "s1".into(),
            ..Default::default()
        };
        let err = spec.validate().unwrap_err();
        assert!(format!("{err}").contains("requires non-empty `command`"));
        spec.command = "python".into();
        spec.validate().unwrap();
    }

    #[test]
    fn validate_http_requires_url() {
        let mut spec = McpServerSpec {
            name: "gmaps".into(),
            transport: "http".into(),
            ..Default::default()
        };
        let err = spec.validate().unwrap_err();
        assert!(format!("{err}").contains("requires `url`"));
        spec.url = Some("http://127.0.0.1:3001".into());
        spec.validate().unwrap();
    }

    #[test]
    fn validate_rejects_unknown_transport() {
        let spec = McpServerSpec {
            name: "x".into(),
            transport: "carrier_pigeon".into(),
            ..Default::default()
        };
        let err = spec.validate().unwrap_err();
        assert!(format!("{err}").contains("unknown transport"));
    }

    #[test]
    fn validate_accepts_plaintext_bearer_but_does_not_bail() {
        // Plaintext Bearer is discouraged (see the doc-comment on
        // `AuthSpec::Bearer`) but we deliberately do NOT bail —
        // closed test setups and CI fixtures legitimately use it.
        // The audit-trail warning emitted by `validate` is on
        // stderr; this test pins the contract that the validation
        // outcome remains `Ok(())`.
        let spec = McpServerSpec {
            name: "with-plaintext-bearer".into(),
            transport: "http".into(),
            url: Some("https://example.com".into()),
            auth: Some(AuthSpec::Bearer {
                token: "shh-secret".into(),
            }),
            ..Default::default()
        };
        spec.validate()
            .expect("plaintext bearer is discouraged but not a hard error");
    }

    #[test]
    fn validate_accepts_empty_bearer_without_warning_path() {
        // The warning is gated on `!token.is_empty()` so an empty
        // string (e.g. operator commented out the secret to disable
        // auth) does not produce a misleading audit-trail entry.
        // We can't capture stderr cheaply, but we can pin that this
        // branch is not the bail branch.
        let spec = McpServerSpec {
            name: "empty-bearer".into(),
            transport: "http".into(),
            url: Some("https://example.com".into()),
            auth: Some(AuthSpec::Bearer {
                token: String::new(),
            }),
            ..Default::default()
        };
        spec.validate().expect("empty token validates");
    }

    #[test]
    fn validate_accepts_plaintext_headers_with_warning_emission_path() {
        // Same contract as the plaintext-bearer test: we exercise
        // the warning branch and pin that validation still passes.
        let mut headers = std::collections::HashMap::new();
        headers.insert("X-Api-Key".to_string(), "k-123".to_string());
        let spec = McpServerSpec {
            name: "with-headers".into(),
            transport: "http".into(),
            url: Some("https://example.com".into()),
            auth: Some(AuthSpec::Headers { headers }),
            ..Default::default()
        };
        spec.validate()
            .expect("plaintext headers are discouraged but not a hard error");
    }

    #[test]
    fn validate_rejects_auth_headers_with_invalid_name_at_config_load() {
        // **Boot-time validation pin**. A header name with embedded
        // whitespace (or any byte hyper's `HeaderName::from_bytes`
        // refuses) must fail at `validate()` — not silently at first
        // request. The operator typo surfaces with the server name
        // attached so they can locate it in mcp_clients.json.
        let mut headers = std::collections::HashMap::new();
        headers.insert("Bad Header Name".to_string(), "v".to_string());
        let spec = McpServerSpec {
            name: "bad-header-name".into(),
            transport: "http".into(),
            url: Some("https://example.com".into()),
            auth: Some(AuthSpec::Headers { headers }),
            ..Default::default()
        };
        let err = spec
            .validate()
            .expect_err("invalid header name must bail at config load");
        let msg = format!("{err:#}");
        assert!(msg.contains("bad-header-name"), "got: {msg}");
        assert!(msg.contains("not a valid HTTP header name"), "got: {msg}");
    }

    #[test]
    fn validate_rejects_auth_headers_with_crlf_injected_value_at_config_load() {
        // **Header-injection pin**. An attacker who can write the
        // config file could try to smuggle a second header (or
        // request body) by stuffing `\r\n` into a value. hyper's
        // `HeaderValue::from_str` refuses these bytes, so threading
        // the validation through it at boot blocks the class.
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "X-Api-Key".to_string(),
            "ok-prefix\r\nX-Injected: smuggled".to_string(),
        );
        let spec = McpServerSpec {
            name: "crlf-injected".into(),
            transport: "http".into(),
            url: Some("https://example.com".into()),
            auth: Some(AuthSpec::Headers { headers }),
            ..Default::default()
        };
        let err = spec
            .validate()
            .expect_err("CRLF-injected header value must bail at config load");
        let msg = format!("{err:#}");
        assert!(msg.contains("crlf-injected"), "got: {msg}");
        assert!(msg.contains("X-Api-Key"), "got: {msg}");
        assert!(msg.contains("not a valid HTTP header value"), "got: {msg}");
    }

    #[test]
    fn validate_accepts_bearer_env_without_warning_path() {
        // BearerEnv is the recommended shape — no plaintext warning
        // should fire. (We can't capture stderr here, but `validate`
        // must still pass for the non-empty env name.)
        let spec = McpServerSpec {
            name: "with-bearer-env".into(),
            transport: "http".into(),
            url: Some("https://example.com".into()),
            auth: Some(AuthSpec::BearerEnv {
                env: "MY_TOKEN".into(),
            }),
            ..Default::default()
        };
        spec.validate().expect("BearerEnv is the recommended path");
    }

    #[test]
    fn http_spec_serde_roundtrip_preserves_url_and_endpoint() {
        let file = McpClientsFile {
            servers: vec![McpServerSpec {
                name: "Google Maps".into(),
                command: String::new(),
                transport: "http".into(),
                url: Some("http://127.0.0.1:3001".into()),
                endpoint: "/mcp".into(),
                ..Default::default()
            }],
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: McpClientsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.servers[0].transport, "http");
        assert_eq!(
            back.servers[0].url.as_deref(),
            Some("http://127.0.0.1:3001")
        );
        assert_eq!(back.servers[0].endpoint, "/mcp");
    }

    #[tokio::test]
    async fn rpc_unknown_server_errors_clearly() {
        let svc = McpClientService::new();
        let err = svc
            .rpc("nonexistent", "tools/list", json!({}))
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("nonexistent"),
            "error must name the missing server; got {msg:?}"
        );
    }

    /// Round-trip through a real MCP-style stdio echo server.
    /// Uses a tiny inline shell script so the test stays
    /// dependency-free. The script reads JSON-RPC requests from
    /// stdin and replies with `{result: {echoed: <params>}}` on
    /// the same id.
    #[tokio::test]
    #[cfg(unix)]
    async fn rpc_round_trips_through_a_real_subprocess() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mcp_echo.sh");
        std::fs::write(
            &script,
            // POSIX sh + jq isn't universal; use Python which is
            // present on every macOS + Linux dev box.
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode().strip()
        if not line:
            break
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(resp):
    body = json.dumps(resp).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    # Echo back the params as the result, keyed to the request id.
    resp = {"jsonrpc": "2.0", "id": req.get("id"), "result": {"echoed": req.get("params")}}
    write_msg(resp)
'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let file = McpClientsFile {
            servers: vec![McpServerSpec {
                name: "echo".into(),
                command: script.to_string_lossy().to_string(),
                stdio_framing: "content-length".into(),
                ..Default::default()
            }],
        };
        let svc = McpClientService::from_file(file);

        // First call: spawns child, runs initialize handshake,
        // then the actual tools/list request. The echo server
        // returns the params as result; we just confirm the
        // round-trip works.
        let resp = svc
            .rpc("echo", "tools/list", json!({"probe": "first-call"}))
            .await
            .expect("first rpc should succeed");
        assert_eq!(resp["echoed"]["probe"], "first-call");

        // Second call: reuses the live connection; id is now 3
        // (1=initialize, 2=tools/list, 3=this).
        let resp2 = svc
            .rpc("echo", "tools/call", json!({"probe": "second-call"}))
            .await
            .expect("second rpc should succeed");
        assert_eq!(resp2["echoed"]["probe"], "second-call");
    }

    /// B3 — verify rpc_with_progress routes interleaved
    /// notifications/progress frames through the supplied sink
    /// while still returning the final response.
    ///
    /// The Python upstream below emits ONE progress notification
    /// before the response for any `tools/call`; for `initialize`
    /// + `tools/list` it just replies normally. That isolates the
    /// "notifications interleave before the matching response"
    /// behaviour we want to test.
    #[tokio::test]
    #[cfg(unix)]
    async fn rpc_with_progress_routes_interleaved_progress_to_sink() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("progress_mcp.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {}
    while True:
        raw = sys.stdin.buffer.readline()
        if not raw:
            return None
        line = raw.decode().strip()
        if not line:
            break
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(msg):
    body = json.dumps(msg).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    rid = req.get("id")
    method = req.get("method")
    # JSON-RPC 2.0: requests with no id are notifications and MUST
    # NOT receive a response. notifications/initialized lands here.
    if rid is None:
        continue
    if method == "tools/call":
        # Emit a progress notification BEFORE the matching response.
        write_msg({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {"progressToken": "tok-7", "progress": 0.5, "total": 1.0, "message": "halfway"}
        })
        write_msg({"jsonrpc": "2.0", "id": rid, "result": {"echoed": req.get("params")}})
    else:
        write_msg({"jsonrpc": "2.0", "id": rid, "result": {}})
'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let svc = McpClientService::from_file(McpClientsFile {
            servers: vec![McpServerSpec {
                name: "echo".into(),
                command: script.to_string_lossy().to_string(),
                stdio_framing: "content-length".into(),
                ..Default::default()
            }],
        });

        struct CollectSink {
            seen: Vec<ObservedNotification>,
        }
        impl NotificationSink for CollectSink {
            fn observe(&mut self, n: ObservedNotification) {
                self.seen.push(n);
            }
        }
        let mut sink = CollectSink { seen: Vec::new() };
        let resp = svc
            .rpc_with_progress("echo", "tools/call", json!({"k": "v"}), &mut sink)
            .await
            .expect("rpc_with_progress must round-trip");
        // Response shape unchanged from rpc().
        assert_eq!(resp["echoed"]["k"], "v");
        // The progress notification reached the sink.
        assert_eq!(
            sink.seen.len(),
            1,
            "expected exactly 1 progress notification, got {:?}",
            sink.seen
        );
        let p = sink.seen[0]
            .as_progress()
            .expect("notifications/progress must parse via as_progress()");
        assert_eq!(p.token, json!("tok-7"));
        assert_eq!(p.progress, 0.5);
        assert_eq!(p.total, Some(1.0));
        assert_eq!(p.message.as_deref(), Some("halfway"));
    }

    /// Service-level end-to-end SSE progress contract. Mirrors the
    /// stdio test above but exercises the HTTP transport. Spins up
    /// an in-process MCP-style HTTP server that emits two
    /// notifications/progress frames in an SSE stream before the
    /// terminal `tools/call` response; `rpc_with_progress` must
    /// route both frames to the sink and return the response
    /// verbatim.
    ///
    /// Why this lives at the service level rather than the `http`
    /// submodule: the lower layer's tests pin
    /// `http::HttpConnection::rpc_with_sink` in isolation; this one
    /// pins the end-to-end seam — `McpClientService::rpc_with_progress`
    /// dispatching to the HTTP branch and threading the sink
    /// through — which is the API every caller (reflective
    /// registry, MCP-bench round-1, bridge progress projection)
    /// actually uses.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn service_rpc_with_progress_routes_http_sse_notifications() {
        use http_body_util::BodyExt;
        use http_body_util::Full as RespFull;
        use hyper::body::Bytes;
        use hyper::body::Incoming;
        use hyper::server::conn::http1::Builder as ServerBuilder;
        use hyper::service::service_fn;
        use hyper::{Request, Response, StatusCode};
        use std::convert::Infallible;
        use std::net::SocketAddr;
        use tokio::net::TcpListener;

        // Inline minimal hyper IO adapter — identical in shape to
        // `http::hyper_io::HyperTokioIo`. Inlined here so this
        // module's test doesn't have to reach across a `pub(super)`
        // boundary into the `http` submodule.
        struct ServerIo<T>(T);
        impl<T: tokio::io::AsyncRead + Unpin> hyper::rt::Read for ServerIo<T> {
            fn poll_read(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                mut buf: hyper::rt::ReadBufCursor<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                let n = unsafe {
                    let mut tbuf = tokio::io::ReadBuf::uninit(buf.as_mut());
                    match std::pin::Pin::new(&mut self.0).poll_read(cx, &mut tbuf) {
                        std::task::Poll::Ready(Ok(())) => tbuf.filled().len(),
                        other => return other,
                    }
                };
                unsafe { buf.advance(n) };
                std::task::Poll::Ready(Ok(()))
            }
        }
        impl<T: tokio::io::AsyncWrite + Unpin> hyper::rt::Write for ServerIo<T> {
            fn poll_write(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &[u8],
            ) -> std::task::Poll<std::io::Result<usize>> {
                std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
            }
            fn poll_flush(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::pin::Pin::new(&mut self.0).poll_flush(cx)
            }
            fn poll_shutdown(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
            }
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                tokio::spawn(async move {
                    let io = ServerIo(stream);
                    let svc = service_fn(move |req: Request<Incoming>| async move {
                        let body = req
                            .into_body()
                            .collect()
                            .await
                            .map(|c| c.to_bytes())
                            .unwrap_or_default();
                        let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
                        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
                        match method {
                            "initialize" => Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .header("content-type", "application/json")
                                    .body(RespFull::new(Bytes::from(
                                        serde_json::to_vec(&json!({
                                            "jsonrpc":"2.0",
                                            "id":id,
                                            "result":{
                                                "protocolVersion":"2025-06-18",
                                                "capabilities":{},
                                                "serverInfo":{"name":"t","version":"0"}
                                            }
                                        }))
                                        .unwrap(),
                                    )))
                                    .unwrap(),
                            ),
                            "notifications/initialized" => Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::ACCEPTED)
                                    .body(RespFull::new(Bytes::new()))
                                    .unwrap(),
                            ),
                            "tools/call" => {
                                let id_str = serde_json::to_string(&id).unwrap();
                                let sse = format!(
                                    "data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{\"progressToken\":\"http-tok\",\"progress\":0.2,\"total\":1.0,\"message\":\"first\"}}}}\n\n\
                                     data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{\"progressToken\":\"http-tok\",\"progress\":0.9,\"total\":1.0,\"message\":\"almost\"}}}}\n\n\
                                     data: {{\"jsonrpc\":\"2.0\",\"id\":{id_str},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"finished\"}}],\"isError\":false}}}}\n\n"
                                );
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(StatusCode::OK)
                                        .header("content-type", "text/event-stream")
                                        .body(RespFull::new(Bytes::from(sse)))
                                        .unwrap(),
                                )
                            }
                            _ => Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::NOT_FOUND)
                                    .body(RespFull::new(Bytes::new()))
                                    .unwrap(),
                            ),
                        }
                    });
                    let _ = ServerBuilder::new().serve_connection(io, svc).await;
                });
            }
        });

        let svc = McpClientService::from_file(McpClientsFile {
            servers: vec![McpServerSpec {
                name: "http-progress".into(),
                command: String::new(),
                transport: "http".into(),
                url: Some(url),
                endpoint: "/mcp".into(),
                ..Default::default()
            }],
        });

        struct CollectSink {
            seen: Vec<ObservedNotification>,
        }
        impl NotificationSink for CollectSink {
            fn observe(&mut self, n: ObservedNotification) {
                self.seen.push(n);
            }
        }
        let mut sink = CollectSink { seen: Vec::new() };
        let resp = svc
            .rpc_with_progress(
                "http-progress",
                "tools/call",
                json!({"name":"any","arguments":{}}),
                &mut sink,
            )
            .await
            .expect("HTTP rpc_with_progress must round-trip");
        // Terminal response: the MCP tools/call shape, surfaced
        // verbatim.
        assert_eq!(resp["content"][0]["text"], "finished");
        assert_eq!(resp["isError"], false);
        // Both progress frames reached the sink in order.
        assert_eq!(
            sink.seen.len(),
            2,
            "expected 2 progress notes over HTTP SSE, got {:?}",
            sink.seen
        );
        let p0 = sink.seen[0]
            .as_progress()
            .expect("first SSE notification must parse as progress");
        assert_eq!(p0.token, json!("http-tok"));
        assert_eq!(p0.progress, 0.2);
        assert_eq!(p0.message.as_deref(), Some("first"));
        let p1 = sink.seen[1].as_progress().expect("second must parse");
        assert_eq!(p1.progress, 0.9);
        assert_eq!(p1.message.as_deref(), Some("almost"));
    }
}
