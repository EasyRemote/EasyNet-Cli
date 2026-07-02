// EasyNet CLI — http.request ability (AXIOM Tier 2.5)
// =====================================================
//
// File: src/runtime/system_abilities/device_control/http.rs
// Description: The HTTP-client member of the Baseline
//              Locomotion Profile. Issues one outbound HTTP
//              request, captures status + headers + body
//              with caps, redacts auth-bearing headers from
//              every receipt the auditor may persist.
//
// Why http.request lives in baseline-locomotion-v1
// ------------------------------------------------
// AXIOM Tier 2.5 §"baseline-locomotion-v1" enumerates the
// seven abilities every host-embodied agent MUST expose:
// fs.read, fs.write, fs.list, process.exec, shell.run,
// pty.attach, http.request. The first six let the agent
// observe and act on the host filesystem and process tree;
// http.request is the only one that reaches outbound network
// without going through a user-defined `curl` (which the
// shell.run permission stage would gate). Treating it as a
// first-class ability lets receivers reason about outbound
// network access uniformly — every http.request can be
// gated, audited, capped, and logged in one place.
//
// What this ability DOES check
// ----------------------------
// 1. Schema (validated upstream by ability dispatch).
// 2. Caps: timeout_ms, body_max_bytes, redirect_max.
// 3. URL scheme allowlist: only http and https. file://,
//    data:, ftp:// reject — those would let a permission rule
//    that allows http.request leak filesystem reads or run
//    legacy stack-clobbering protocols.
// 4. Body / response size caps via stream-with-limit.
// 5. Receipt redaction: Authorization, Cookie, Set-Cookie,
//    X-API-Key, Proxy-Authorization headers are stripped
//    from the audit fields. Header NAMES still appear so
//    the operator sees an Authorization was present; the
//    VALUE never reaches the receipt.
//
// What this ability does NOT check
// --------------------------------
// * Host allowlist: a future http_constraints stage in
//   shellguard could carry it, but v1 trusts the caller's
//   policy gate (permission stage on shell.run uses the
//   same receivers; the daemon-level admission gate sits in
//   front of every ability invocation).
// * TLS pinning: ureq uses rustls with the OS root store.
//   A future ability flag could pin certs; the v1 surface
//   trusts the system trust store.
// * Proxy: respects HTTP_PROXY / HTTPS_PROXY / NO_PROXY env
//   vars via ureq's default proxy detection. v1 does not
//   force a proxy; receiver-side admission can refuse the
//   call if proxy posture is wrong.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;

use crate::runtime::ability_dispatch::OwnerKind;
/// Wire name. Pinned by AXIOM Tier 2.5.
pub const ABILITY_NAME: &str = crate::runtime::ability_names::device_control::HTTP_REQUEST;

/// Profile version echoed in every receipt.
pub const PROFILE_VERSION: &str =
    crate::runtime::ability_names::device_control::BASELINE_LOCOMOTION_PROFILE_VERSION;

/// Default timeout when the caller omits it. 30 s mirrors
/// process.exec / shell.run defaults so policy is uniform
/// across the locomotion profile.
pub const TIMEOUT_DEFAULT_MS: u64 = 30_000;
/// Hard cap. 10 minutes matches what most upstream services
/// configure as their longest connect+read budget; a longer
/// budget is almost always a sign the caller wants
/// streaming, which is a different ability surface.
pub const TIMEOUT_HARD_CAP_MS: u64 = 10 * 60 * 1000;
/// Default body-size cap on the response side. 1 MiB matches
/// process.exec / shell.run output cap so tooling that
/// budgets receipts can cap on a single number.
pub const BODY_DEFAULT_CAP: u64 = 1024 * 1024;
/// Hard cap. Same as runner's 100 MiB output cap.
pub const BODY_HARD_CAP: u64 = 100 * 1024 * 1024;

/// Maximum HTTP redirects to follow. ureq's default is 5;
/// callers can shrink (down to 0 = no follow) but cannot
/// exceed 10 (excessive chains are usually a redirect-loop).
pub const REDIRECT_HARD_CAP: u32 = 10;
pub const REDIRECT_DEFAULT: u32 = 5;

/// Header names that carry credentials. Their VALUES are
/// redacted from every receipt; only the NAMES remain so the
/// auditor can see "an Authorization was sent" without seeing
/// the bearer token. Match is case-insensitive.
const REDACTED_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
];

/// Allowed URL schemes. Anything else (file, data, ftp,
/// gopher, javascript) rejects.
const ALLOWED_SCHEMES: &[&str] = &["http", "https"];

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner("http.request", OwnerKind::Device, Arc::new(handler));
}

fn handler(args: Value) -> Result<Value> {
    let req = parse_request(&args)?;
    let outcome = run_request(&req);
    Ok(build_response(&req, outcome))
}

#[derive(Clone, Debug)]
struct HttpRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    timeout_ms: u64,
    body_max_bytes: u64,
    redirect_max: u32,
    follow_redirects: bool,
}

fn parse_request(args: &Value) -> Result<HttpRequest> {
    let url = require_string(args, "url")?.to_string();
    let parsed = parse_url_scheme(&url)?;
    if !ALLOWED_SCHEMES.contains(&parsed) {
        return Err(anyhow!(
            "http.request: scheme {parsed:?} not in allowlist (http, https)"
        ));
    }
    let method = args
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_uppercase();
    if !is_supported_method(&method) {
        return Err(anyhow!("http.request: method {method:?} not supported"));
    }
    let headers = match args.get("headers") {
        Some(Value::Object(map)) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map {
                if k.is_empty() {
                    return Err(anyhow!("http.request: header name must not be empty"));
                }
                let s = v
                    .as_str()
                    .ok_or_else(|| anyhow!("http.request: header[{k}] must be a string value"))?;
                if !is_valid_header_name(k) {
                    return Err(anyhow!(
                        "http.request: header name {k:?} contains illegal characters"
                    ));
                }
                if !is_valid_header_value(s) {
                    return Err(anyhow!(
                        "http.request: header value for {k} contains CR/LF (header injection)"
                    ));
                }
                out.push((k.clone(), s.to_string()));
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        Some(_) => return Err(anyhow!("http.request: headers must be an object")),
    };
    let body = decode_body(args)?;
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(TIMEOUT_DEFAULT_MS);
    if timeout_ms > TIMEOUT_HARD_CAP_MS {
        return Err(anyhow!(
            "http.request: timeout_ms {timeout_ms} exceeds hard cap {TIMEOUT_HARD_CAP_MS}"
        ));
    }
    let body_max_bytes = args
        .get("body_max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(BODY_DEFAULT_CAP);
    if body_max_bytes > BODY_HARD_CAP {
        return Err(anyhow!(
            "http.request: body_max_bytes {body_max_bytes} exceeds hard cap {BODY_HARD_CAP}"
        ));
    }
    let follow_redirects = args
        .get("follow_redirects")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let redirect_max = args
        .get("redirect_max")
        .and_then(Value::as_u64)
        .map(|n| n as u32)
        .unwrap_or(REDIRECT_DEFAULT);
    if redirect_max > REDIRECT_HARD_CAP {
        return Err(anyhow!(
            "http.request: redirect_max {redirect_max} exceeds hard cap {REDIRECT_HARD_CAP}"
        ));
    }
    Ok(HttpRequest {
        method,
        url,
        headers,
        body,
        timeout_ms,
        body_max_bytes,
        redirect_max,
        follow_redirects,
    })
}

fn parse_url_scheme(url: &str) -> Result<&str> {
    let idx = url
        .find("://")
        .ok_or_else(|| anyhow!("http.request: url {url:?} missing scheme://host"))?;
    let scheme = &url[..idx];
    if scheme.is_empty() {
        return Err(anyhow!("http.request: empty scheme in url {url:?}"));
    }
    Ok(scheme)
}

fn is_supported_method(m: &str) -> bool {
    matches!(
        m,
        "GET" | "HEAD" | "POST" | "PUT" | "DELETE" | "PATCH" | "OPTIONS"
    )
}

/// RFC 7230 token: alpha / digit / `!#$%&'*+-.^_`|~`. We accept
/// the practical subset; reject control chars, separators, and
/// `:`/space/tab/CR/LF which are header injection vectors.
fn is_valid_header_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    name.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '!' | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '-'
                    | '.'
                    | '^'
                    | '_'
                    | '`'
                    | '|'
                    | '~'
            )
    })
}

/// Header values may NOT contain CR / LF — those would let a
/// caller smuggle a second request line or a fake response
/// header. ureq sanitises but ours is the receiver-side defence.
fn is_valid_header_value(value: &str) -> bool {
    !value.contains('\r') && !value.contains('\n')
}

fn decode_body(args: &Value) -> Result<Option<Vec<u8>>> {
    match args.get("body") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let encoding = args
                .get("body_encoding")
                .and_then(Value::as_str)
                .unwrap_or("base64");
            match encoding {
                "base64" => BASE64_STANDARD
                    .decode(s.as_bytes())
                    .map(Some)
                    .map_err(|e| anyhow!("http.request: invalid body base64: {e}")),
                "utf8" => Ok(Some(s.as_bytes().to_vec())),
                other => Err(anyhow!(
                    "http.request: body_encoding {other:?} unknown; expected base64|utf8"
                )),
            }
        }
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, v) in items.iter().enumerate() {
                let n = v.as_u64().ok_or_else(|| {
                    anyhow!("http.request: body[{i}] must be an integer in 0..256")
                })?;
                if n > 255 {
                    return Err(anyhow!("http.request: body[{i}] = {n} out of range"));
                }
                out.push(n as u8);
            }
            Ok(Some(out))
        }
        Some(_) => Err(anyhow!(
            "http.request: body must be a string, an array of bytes, or null"
        )),
    }
}

#[derive(Debug)]
struct HttpOutcome {
    status: u16,
    response_headers: Vec<(String, String)>,
    body: Vec<u8>,
    body_truncated: bool,
    duration_ms: u64,
}

#[derive(Debug)]
enum HttpError {
    Status {
        status: u16,
        message: String,
        body: Vec<u8>,
        duration_ms: u64,
        response_headers: Vec<(String, String)>,
    },
    Transport {
        message: String,
        duration_ms: u64,
    },
}

fn run_request(req: &HttpRequest) -> Result<HttpOutcome, HttpError> {
    let start = std::time::Instant::now();
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(req.timeout_ms))
        .redirects(if req.follow_redirects {
            req.redirect_max
        } else {
            0
        })
        .build();
    let mut request = agent.request(&req.method, &req.url);
    for (k, v) in &req.headers {
        request = request.set(k, v);
    }
    let response_result = match &req.body {
        Some(bytes) => request.send_bytes(bytes),
        None => request.call(),
    };
    match response_result {
        Ok(resp) => {
            let status = resp.status();
            let response_headers = collect_response_headers(&resp);
            let (body, truncated) =
                read_capped(resp, req.body_max_bytes).map_err(|e| HttpError::Transport {
                    message: format!("body read: {e}"),
                    duration_ms: start.elapsed().as_millis() as u64,
                })?;
            Ok(HttpOutcome {
                status,
                response_headers,
                body,
                body_truncated: truncated,
                duration_ms: start.elapsed().as_millis() as u64,
            })
        }
        Err(ureq::Error::Status(status, resp)) => {
            // Non-2xx response. ureq surfaces these as Err by
            // default; we treat them as "request completed
            // with an error status" — caller still gets the
            // body and headers. Auditors want the same.
            let response_headers = collect_response_headers(&resp);
            let message = resp.status_text().to_string();
            let body = read_capped(resp, req.body_max_bytes)
                .map(|(b, _)| b)
                .unwrap_or_default();
            Err(HttpError::Status {
                status,
                message,
                body,
                duration_ms: start.elapsed().as_millis() as u64,
                response_headers,
            })
        }
        Err(ureq::Error::Transport(t)) => Err(HttpError::Transport {
            message: t.to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
        }),
    }
}

fn collect_response_headers(resp: &ureq::Response) -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(resp.headers_names().len());
    for name in resp.headers_names() {
        if let Some(v) = resp.header(&name) {
            out.push((name, v.to_string()));
        }
    }
    out
}

/// Read at most `cap` bytes from `resp`. Returns the captured
/// bytes and whether truncation occurred (read more than cap).
fn read_capped(resp: ureq::Response, cap: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut reader = resp.into_reader().take(cap + 1);
    let mut buf: Vec<u8> = Vec::with_capacity(cap.min(64 * 1024) as usize);
    reader.read_to_end(&mut buf)?;
    let truncated = buf.len() as u64 > cap;
    if truncated {
        buf.truncate(cap as usize);
    }
    Ok((buf, truncated))
}

fn build_response(req: &HttpRequest, outcome: Result<HttpOutcome, HttpError>) -> Value {
    let request_header_audit = audit_headers(&req.headers);
    match outcome {
        Ok(o) => {
            let response_header_audit = audit_headers(&o.response_headers);
            json!({
                "ok": true,
                "status": o.status,
                "response_headers": json_headers(&o.response_headers, true),
                "response_header_names_redacted": response_header_audit,
                "body": BASE64_STANDARD.encode(&o.body),
                "body_bytes": o.body.len(),
                "body_truncated": o.body_truncated,
                "duration_ms": o.duration_ms,
                "method": req.method,
                "url_host": url_host(&req.url),
                "url_sha256": url_sha256(&req.url),
                "request_header_names": header_names(&req.headers),
                "request_header_names_redacted": request_header_audit,
                "request_body_bytes": req.body.as_ref().map(Vec::len).unwrap_or(0),
                "ability_profile_version": PROFILE_VERSION,
            })
        }
        Err(HttpError::Status {
            status,
            message,
            body,
            duration_ms,
            response_headers,
        }) => {
            let response_header_audit = audit_headers(&response_headers);
            json!({
                "ok": false,
                "code": "HTTP_STATUS",
                "status": status,
                "status_message": message,
                "response_headers": json_headers(&response_headers, true),
                "response_header_names_redacted": response_header_audit,
                "body": BASE64_STANDARD.encode(&body),
                "body_bytes": body.len(),
                "duration_ms": duration_ms,
                "method": req.method,
                "url_host": url_host(&req.url),
                "url_sha256": url_sha256(&req.url),
                "request_header_names": header_names(&req.headers),
                "request_header_names_redacted": request_header_audit,
                "ability_profile_version": PROFILE_VERSION,
            })
        }
        Err(HttpError::Transport {
            message,
            duration_ms,
        }) => json!({
            "ok": false,
            "code": "HTTP_TRANSPORT",
            "error": message,
            "duration_ms": duration_ms,
            "method": req.method,
            "url_host": url_host(&req.url),
            "url_sha256": url_sha256(&req.url),
            "request_header_names": header_names(&req.headers),
            "request_header_names_redacted": request_header_audit,
            "ability_profile_version": PROFILE_VERSION,
        }),
    }
}

fn header_names(headers: &[(String, String)]) -> Vec<String> {
    headers.iter().map(|(k, _)| k.clone()).collect()
}

fn audit_headers(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter(|(k, _)| {
            REDACTED_HEADER_NAMES
                .iter()
                .any(|r| r.eq_ignore_ascii_case(k))
        })
        .map(|(k, _)| k.clone())
        .collect()
}

/// Render headers for the receipt. When `redact_values` is
/// true (always, for v1), every header whose name is in the
/// REDACTED_HEADER_NAMES set has its value replaced with the
/// literal string `[REDACTED]` so an auditor can see the
/// header name was present without seeing the bearer.
fn json_headers(headers: &[(String, String)], redact_values: bool) -> Value {
    let mut out = serde_json::Map::with_capacity(headers.len());
    for (k, v) in headers {
        let value = if redact_values && is_redacted_name(k) {
            "[REDACTED]".to_string()
        } else {
            v.clone()
        };
        out.insert(k.clone(), Value::String(value));
    }
    Value::Object(out)
}

fn is_redacted_name(name: &str) -> bool {
    REDACTED_HEADER_NAMES
        .iter()
        .any(|r| r.eq_ignore_ascii_case(name))
}

fn url_host(url: &str) -> String {
    // Substring match, no full URL parser pulled in for one
    // field. Returns "scheme://host" segment minus path/query.
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_string()
}

fn url_sha256(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    hex::encode(hasher.finalize())
}

// ── Schema + description ──────────────────────────────────────────

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["url"],
        "additionalProperties": false,
        "properties": {
            "url": { "type": "string", "minLength": 1 },
            "method": { "type": "string", "enum": ["GET","HEAD","POST","PUT","DELETE","PATCH","OPTIONS"] },
            "headers": { "type": "object", "additionalProperties": { "type": "string" } },
            "body": {
                "oneOf": [
                    { "type": "string" },
                    { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 } },
                    { "type": "null" }
                ]
            },
            "body_encoding": { "type": "string", "enum": ["base64", "utf8"] },
            "timeout_ms": { "type": "integer", "minimum": 0, "maximum": TIMEOUT_HARD_CAP_MS },
            "body_max_bytes": { "type": "integer", "minimum": 0, "maximum": BODY_HARD_CAP },
            "follow_redirects": { "type": "boolean" },
            "redirect_max": { "type": "integer", "minimum": 0, "maximum": REDIRECT_HARD_CAP }
        }
    })
}

pub fn description() -> &'static str {
    "Issue one outbound HTTP request (http/https only). \
     Captures status, response headers, and body up to body_max_bytes. \
     Auth-bearing headers (Authorization, Cookie, X-API-Key, …) are \
     REDACTED from receipts: the name appears, the value never does. \
     Part of the baseline-locomotion-v1 profile."
}

fn require_string<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("http.request: missing required string field `{field}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(pairs: &[(&str, Value)]) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Object(m)
    }

    // ─── parse_request ─────────────────────────

    #[test]
    fn parse_minimal_get_request() {
        let r = parse_request(&args(&[("url", json!("https://example.com"))])).unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.url, "https://example.com");
        assert!(r.headers.is_empty());
        assert!(r.body.is_none());
        assert_eq!(r.timeout_ms, TIMEOUT_DEFAULT_MS);
        assert!(r.follow_redirects);
    }

    #[test]
    fn method_lowercased_input_uppercased() {
        let r = parse_request(&args(&[
            ("url", json!("https://x")),
            ("method", json!("post")),
        ]))
        .unwrap();
        assert_eq!(r.method, "POST");
    }

    #[test]
    fn unsupported_method_rejects() {
        let err = parse_request(&args(&[
            ("url", json!("https://x")),
            ("method", json!("CONNECT")),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("not supported"));
    }

    #[test]
    fn file_scheme_rejects() {
        let err = parse_request(&args(&[("url", json!("file:///etc/passwd"))])).unwrap_err();
        assert!(err.to_string().contains("scheme"));
    }

    #[test]
    fn data_scheme_rejects() {
        let err = parse_request(&args(&[("url", json!("data:text/plain,hi"))])).unwrap_err();
        assert!(err.to_string().contains("scheme"));
    }

    #[test]
    fn missing_scheme_rejects() {
        let err = parse_request(&args(&[("url", json!("example.com"))])).unwrap_err();
        assert!(err.to_string().contains("missing scheme"));
    }

    #[test]
    fn header_with_crlf_rejects() {
        let err = parse_request(&args(&[
            ("url", json!("https://x")),
            ("headers", json!({"X-Custom": "a\r\nInjected: yes"})),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("CR/LF"));
    }

    #[test]
    fn header_name_with_space_rejects() {
        let err = parse_request(&args(&[
            ("url", json!("https://x")),
            ("headers", json!({"X Custom": "v"})),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("illegal characters"));
    }

    #[test]
    fn empty_header_name_rejects() {
        let err = parse_request(&args(&[
            ("url", json!("https://x")),
            ("headers", json!({"": "v"})),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn timeout_over_hard_cap_rejects() {
        let err = parse_request(&args(&[
            ("url", json!("https://x")),
            ("timeout_ms", json!(60 * 60 * 1000)),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("hard cap"));
    }

    #[test]
    fn body_max_over_hard_cap_rejects() {
        let err = parse_request(&args(&[
            ("url", json!("https://x")),
            ("body_max_bytes", json!(BODY_HARD_CAP + 1)),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("hard cap"));
    }

    #[test]
    fn redirect_over_hard_cap_rejects() {
        let err = parse_request(&args(&[
            ("url", json!("https://x")),
            ("redirect_max", json!(REDIRECT_HARD_CAP + 1)),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("hard cap"));
    }

    #[test]
    fn body_base64_decoded() {
        let r = parse_request(&args(&[
            ("url", json!("https://x")),
            ("body", json!(BASE64_STANDARD.encode("hi"))),
        ]))
        .unwrap();
        assert_eq!(r.body.unwrap(), b"hi".to_vec());
    }

    #[test]
    fn body_utf8_when_encoding_is_utf8() {
        let r = parse_request(&args(&[
            ("url", json!("https://x")),
            ("body", json!("hi")),
            ("body_encoding", json!("utf8")),
        ]))
        .unwrap();
        assert_eq!(r.body.unwrap(), b"hi".to_vec());
    }

    #[test]
    fn body_array_of_bytes() {
        let r = parse_request(&args(&[
            ("url", json!("https://x")),
            ("body", json!([1, 2, 3])),
        ]))
        .unwrap();
        assert_eq!(r.body.unwrap(), vec![1, 2, 3]);
    }

    // ─── helpers ─────────────────────────

    #[test]
    fn url_host_returns_host_only() {
        assert_eq!(
            url_host("https://example.com/foo?bar"),
            "example.com".to_string()
        );
        assert_eq!(url_host("http://localhost:8080/"), "localhost:8080");
        assert_eq!(url_host("https://x.io"), "x.io");
    }

    #[test]
    fn url_sha256_is_stable_64_hex() {
        let a = url_sha256("https://x");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, url_sha256("https://x"));
        assert_ne!(a, url_sha256("https://y"));
    }

    #[test]
    fn redacted_names_includes_authorization_case_insensitive() {
        assert!(is_redacted_name("Authorization"));
        assert!(is_redacted_name("authorization"));
        assert!(is_redacted_name("AUTHORIZATION"));
        assert!(is_redacted_name("Cookie"));
        assert!(is_redacted_name("X-API-Key"));
        assert!(!is_redacted_name("X-Custom"));
        assert!(!is_redacted_name("Content-Type"));
    }

    #[test]
    fn audit_headers_lists_only_redacted_names() {
        let h = vec![
            ("Authorization".to_string(), "Bearer x".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Cookie".to_string(), "sid=1".to_string()),
        ];
        let names = audit_headers(&h);
        assert_eq!(names, vec!["Authorization", "Cookie"]);
    }

    #[test]
    fn json_headers_redacts_value_for_redacted_names() {
        let h = vec![
            ("Authorization".to_string(), "Bearer secret".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        let v = json_headers(&h, true);
        assert_eq!(v["Authorization"], json!("[REDACTED]"));
        assert_eq!(v["Content-Type"], json!("application/json"));
    }

    #[test]
    fn input_schema_requires_url() {
        let s = input_schema();
        let req: Vec<&str> = s["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(req.contains(&"url"));
    }

    // ─── method support ──────────────────────

    #[test]
    fn supported_methods_pass_check() {
        for m in ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] {
            assert!(is_supported_method(m), "{m} should be supported");
        }
    }

    #[test]
    fn unsupported_methods_fail_check() {
        for m in ["TRACE", "CONNECT", "FOOBAR", ""] {
            assert!(!is_supported_method(m));
        }
    }

    // ─── header validators ─────────────────

    #[test]
    fn valid_header_names_pass() {
        for n in ["X-Custom", "Content-Type", "Accept", "User-Agent"] {
            assert!(is_valid_header_name(n));
        }
    }

    #[test]
    fn invalid_header_names_fail() {
        for n in ["X Custom", "Bad:Name", "", "X\r\nInjection"] {
            assert!(!is_valid_header_name(n));
        }
    }

    #[test]
    fn valid_header_values_pass() {
        assert!(is_valid_header_value("application/json"));
        assert!(is_valid_header_value("Bearer abc123"));
        assert!(is_valid_header_value(""));
    }

    #[test]
    fn invalid_header_values_fail() {
        assert!(!is_valid_header_value("a\r\nb"));
        assert!(!is_valid_header_value("a\nb"));
        assert!(!is_valid_header_value("a\rb"));
    }
}
