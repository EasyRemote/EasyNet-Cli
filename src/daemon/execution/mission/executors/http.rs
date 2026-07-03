// EasyNet CLI — HTTP Ability Executor
// =====================================
//
// File: src/daemon/execution/mission/executors/http.rs
// Description: Implementation backing `[exec] kind = "http"` in an
//              ability manifest. Issues one HTTP request with
//              templated URL / headers / body, returns a structured
//              JSON envelope.
//
// Why a dedicated executor instead of going through shell + curl
// --------------------------------------------------------------
// `[exec] kind = "shell" + curl` works, but it pulls in a process
// spawn (~50 ms) plus shellguard validation, and the manifest author
// has to remember curl flags (`--silent --fail --max-time …`) for
// every call. The HTTP executor is in-process via ureq: no process
// spawn, no shell escaping, and the safety knobs are the executor's
// concern, not the manifest's.
//
// Substitution model
// ------------------
// URL: `{{ name }}` placeholders are URL-encoded automatically. A
// `{{ city }}` arg of `"São Paulo"` becomes `S%C3%A3o%20Paulo` so a
// non-ASCII or whitespace value never breaks the URL.
//
// Headers + body: same `{{ name }}` substitution but values are NOT
// URL-encoded. Header CR/LF injection is rejected by the underlying
// http client.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::core::ability::spec::HttpExec;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// Default per-call timeout. Same rationale as shell_executor's
/// constant: protect against an upstream hang without forcing every
/// manifest author to think about bounds.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Cap on captured response bytes. A misconfigured ability that
/// fetches a 10 GiB CDN asset would otherwise pin daemon memory;
/// truncating with a loud error is the minimum-surprise behaviour.
const MAX_RESPONSE_BYTES: usize = 1_048_576; // 1 MiB

/// Run the HTTP ability. Returns a JSON envelope
/// `{"result": <decoded body>, "status": N, "headers": {...},
/// "fulfilled_by": "http", "elapsed_ms": N}`. Errors come back as
/// `Err(anyhow)`; the dispatcher surfaces them as typed error
/// frames.
pub fn run_http_exec(
    spec: &HttpExec,
    args: &Value,
    timeout: Option<Duration>,
) -> anyhow::Result<Value> {
    let bindings = match args {
        Value::Object(map) => Some(map),
        Value::Null => None,
        other => anyhow::bail!(
            "http executor: args must be a JSON object (got {})",
            short_kind(other)
        ),
    };

    let url = render_url(&spec.url, bindings)?;
    let method = spec.method.to_ascii_uppercase();

    let started = Instant::now();
    let timeout = timeout.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS));

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .timeout_write(timeout)
        .build();

    let mut req = agent.request(&method, &url);
    if let Some(headers) = &spec.headers {
        for (name, value_template) in headers {
            let rendered = render_one(value_template, bindings, false)?;
            req = req.set(name, &rendered);
        }
    }

    let body_rendered = match &spec.body {
        Some(b) => Some(render_one(b, bindings, false)?),
        None => None,
    };

    let resp = match body_rendered.as_deref() {
        Some(b) => req.send_string(b),
        None => req.call(),
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;

    let resp = match resp {
        Ok(r) => r,
        // A non-2xx surfaces as `Status` from ureq — but for an
        // ability author that wants `--fail`-like semantics this is
        // exactly what they expect: 404 / 500 IS an error, not
        // "result with status=404". A future variant `accept_status`
        // could opt back into success-on-non-2xx; v1 doesn't need it.
        Err(ureq::Error::Status(status, r)) => {
            let body = read_capped(r.into_reader())?;
            anyhow::bail!(
                "http executor: {method} {url} returned status {status}: {}",
                body_preview(&body)
            );
        }
        Err(ureq::Error::Transport(t)) => {
            anyhow::bail!("http executor: transport error for {method} {url}: {t}");
        }
    };

    let status = resp.status();
    let headers: serde_json::Map<String, Value> = resp
        .headers_names()
        .into_iter()
        .filter_map(|name| {
            resp.header(&name)
                .map(|v| (name.clone(), Value::String(v.to_string())))
        })
        .collect();
    let body = read_capped(resp.into_reader())?;
    let decoded = decode_body(&body, spec.response.as_deref())?;

    Ok(json!({
        "result": decoded,
        "fulfilled_by": "http",
        "status": status,
        "headers": Value::Object(headers),
        "elapsed_ms": elapsed_ms,
    }))
}

fn read_capped(mut r: impl std::io::Read) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    loop {
        match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_RESPONSE_BYTES {
                    anyhow::bail!(
                        "http executor: response exceeded {} bytes (cap). \
                         An ability returning streaming or large content needs \
                         a different executor.",
                        MAX_RESPONSE_BYTES
                    );
                }
            }
            Err(e) => anyhow::bail!("http executor: read body failed: {e}"),
        }
    }
    Ok(buf)
}

fn body_preview(bytes: &[u8]) -> String {
    let take = bytes.len().min(200);
    String::from_utf8_lossy(&bytes[..take]).into_owned()
}

fn decode_body(bytes: &[u8], mode: Option<&str>) -> anyhow::Result<Value> {
    let mode = mode.unwrap_or("text_trim");
    match mode {
        "text_trim" => {
            let s = std::str::from_utf8(bytes).map_err(|e| {
                anyhow::anyhow!(
                    "http executor: response is not valid UTF-8 (configure \
                     `response = \"base64\"` once that mode lands if the \
                     ability returns binary): {e}"
                )
            })?;
            Ok(Value::String(s.trim_end().to_string()))
        }
        other => anyhow::bail!("http executor: response decoder {other:?} is not implemented"),
    }
}

/// Render the URL with placeholders URL-encoded. Splits on `{{ }}`
/// the same way the shell executor splits argv elements. The literal
/// portions of the URL pass through verbatim — author is responsible
/// for keeping them well-formed; only the *substituted* parts are
/// auto-encoded.
fn render_url(
    template: &str,
    bindings: Option<&serde_json::Map<String, Value>>,
) -> anyhow::Result<String> {
    render_one(template, bindings, true)
}

fn render_one(
    template: &str,
    bindings: Option<&serde_json::Map<String, Value>>,
    url_encode: bool,
) -> anyhow::Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut cursor = 0usize;
    let bytes = template.as_bytes();
    while cursor < bytes.len() {
        if cursor + 1 < bytes.len() && bytes[cursor] == b'{' && bytes[cursor + 1] == b'{' {
            let rest = &template[cursor + 2..];
            let end = rest.find("}}").ok_or_else(|| {
                anyhow::anyhow!(
                    "http executor: template {:?} has an unclosed `{{{{`",
                    template
                )
            })?;
            let key = rest[..end].trim();
            if key.is_empty() {
                anyhow::bail!(
                    "http executor: template {:?} contains an empty `{{{{ }}}}` placeholder",
                    template
                );
            }
            let bindings = bindings.ok_or_else(|| {
                anyhow::anyhow!(
                    "http executor: template {:?} references arg `{}` but call \
                     passed no args",
                    template,
                    key
                )
            })?;
            let val = bindings.get(key).ok_or_else(|| {
                anyhow::anyhow!(
                    "http executor: template {:?} references arg `{}` which \
                     is not present (provided keys: {:?})",
                    template,
                    key,
                    bindings.keys().collect::<Vec<_>>()
                )
            })?;
            let raw = stringify_arg(val);
            if url_encode {
                out.push_str(&urlencoding::encode(&raw));
            } else {
                out.push_str(&raw);
            }
            cursor += 2 + end + 2;
        } else {
            let ch_end = next_char_boundary(template, cursor);
            out.push_str(&template[cursor..ch_end]);
            cursor = ch_end;
        }
    }
    Ok(out)
}

fn next_char_boundary(s: &str, from: usize) -> usize {
    let mut i = from + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

fn stringify_arg(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn short_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_url_url_encodes_substituted_values() {
        let mut bindings = serde_json::Map::new();
        bindings.insert("city".into(), Value::String("São Paulo".into()));
        let url = render_url("https://wttr.in/{{ city }}?format=4", Some(&bindings)).unwrap();
        assert!(url.contains("S%C3%A3o%20Paulo"), "got {url}");
    }

    #[test]
    fn render_url_passes_literal_parts_through_verbatim() {
        // The literal `?format=%l` would itself be percent-encoded if
        // we URL-encoded the whole template — only substituted values
        // are encoded, so the format string survives.
        let mut bindings = serde_json::Map::new();
        bindings.insert("loc".into(), Value::String("Beijing".into()));
        let url = render_url(
            "https://wttr.in/{{ loc }}?format=%l:+%C+%t",
            Some(&bindings),
        )
        .unwrap();
        assert_eq!(url, "https://wttr.in/Beijing?format=%l:+%C+%t");
    }

    #[test]
    fn render_one_for_headers_does_not_url_encode() {
        // Header values are passed verbatim. `Authorization: Bearer
        // foo bar` is a real value (token "foo bar"); URL-encoding
        // would corrupt it.
        let mut bindings = serde_json::Map::new();
        bindings.insert("token".into(), Value::String("abc def".into()));
        let v = render_one("Bearer {{ token }}", Some(&bindings), false).unwrap();
        assert_eq!(v, "Bearer abc def");
    }

    #[test]
    fn missing_arg_in_template_is_an_error() {
        let bindings = serde_json::Map::new();
        let err = render_one("{{ city }}", Some(&bindings), false).unwrap_err();
        assert!(format!("{err}").contains("city"));
    }
}
