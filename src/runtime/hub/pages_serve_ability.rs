// EasyNet CLI — Hub: pages.serve adapter ability
// ===============================================
//
// File: src/runtime/hub/pages_serve_ability.rs
// Description: implements the body of `01HUB.pages.serve` —
//              the pure transport adapter that forwards an HTTP
//              request into the project's `<user>.<project>.page.fetch`
//              ability via the standard local dispatch path.
//
//              The adapter is "pure" per RFC-006-B v0.6 INV-1:
//              no state mutation, no canonical receipts, no byte
//              transformation beyond mechanical HTTP framing.
//
// Conformance: RFC-006-B v0.6 INV-1 (Adapter Purity), §3.1
//              (forward via standard forward_invoke).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

use crate::runtime::agents::pages::fetch;

/// Output of an HTTP serve invocation. The Hub listener consumes
/// this and translates each field into HTTP response shape.
#[derive(Debug, Clone)]
pub struct ServedBytes {
    pub status: u16,
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub force_attachment: bool,
    pub sha256: String,
}

/// Translate an HTTP request triple `(user, project_id, path)` into
/// the project's `<user>.<project_id>.page.fetch` invocation, run
/// it through the daemon-internal handler, and return the bytes
/// + framing metadata.
///
/// In v0 we call `fetch::handle_fetch` directly because the
/// listener and the fetch handler share the same daemon process —
/// going through a full `forward_invoke` round trip with envelope
/// minting + nonce + receipt would be ceremony without any
/// adversary-distinguishable benefit at this layer. The contract
/// (Adapter Purity, INV-1) is unchanged: this function still does
/// not mutate state and emits no canonical receipts; the operational
/// receipt for the fetch is recorded by the dispatch path the same
/// way it would be for any other ability invocation.
///
/// Phase 2 promotes this to a real `forward_invoke` so the same
/// translation works from the Go backend hub against a remote
/// daemon's project.
pub fn serve_bytes(user: &str, project_id: &str, path: &str) -> ServedBytes {
    let args = json!({ "path": path });
    match fetch::handle_fetch(user, project_id, args) {
        Ok(value) => bytes_from_value(value),
        Err(err) => {
            // Map a small set of fetch errors onto sensible HTTP
            // statuses. Anything we don't recognise becomes 404 to
            // avoid leaking which kinds of failure happened — the
            // browser sees "not found" regardless of whether the
            // project exists, the path was outside the root, the
            // file was a dotfile, or the file simply did not exist.
            // (The daemon's operational receipt logs the precise
            // reason for operators.)
            let msg = err.to_string();
            let status = if msg.contains("project not published") {
                503
            } else if msg.contains("size") && msg.contains("cap") {
                502
            } else {
                404
            };
            ServedBytes {
                status,
                bytes: Vec::new(),
                content_type: "text/plain; charset=utf-8".to_string(),
                force_attachment: false,
                sha256: String::new(),
            }
        }
    }
}

fn bytes_from_value(value: Value) -> ServedBytes {
    use base64::Engine;
    let b64 = value.get("bytes_b64").and_then(Value::as_str).unwrap_or("");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap_or_default();
    let content_type = value
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_string();
    let force_attachment = value
        .get("force_attachment")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sha256 = value
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    ServedBytes {
        status: 200,
        bytes,
        content_type,
        force_attachment,
        sha256,
    }
}
