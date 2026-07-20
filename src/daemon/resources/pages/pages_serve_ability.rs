// EasyNet CLI — Hub: pages.serve adapter ability
// ===============================================
//
// File: src/daemon/hub/pages_serve_ability.rs
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
//              (forward via standard canonical_invoke).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::ability::builtins::resources::pages::fetch;

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
/// going through a full `canonical_invoke` round trip with envelope
/// minting + nonce + receipt would be ceremony without any
/// adversary-distinguishable benefit at this layer. The contract
/// (Adapter Purity, INV-1) is unchanged: this function still does
/// not mutate state and emits no canonical receipts; the operational
/// receipt for the fetch is recorded by the dispatch path the same
/// way it would be for any other ability invocation.
///
/// Phase 2 promotes this to a real `canonical_invoke` so the same
/// translation works from the Go backend hub against a remote
/// daemon's project.
pub fn serve_bytes(user: &str, project_id: &str, path: &str) -> ServedBytes {
    let args = json!({ "path": path });
    match fetch::handle_fetch(user, project_id, args) {
        Ok(value) => bytes_from_value(value).unwrap_or_else(|err| ServedBytes {
            status: 502,
            bytes: Vec::new(),
            content_type: "text/plain; charset=utf-8".to_string(),
            force_attachment: false,
            sha256: format!("invalid fetch projection: {err}"),
        }),
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

fn bytes_from_value(value: Value) -> anyhow::Result<ServedBytes> {
    use base64::Engine;
    let b64 = required_non_empty_string(&value, "bytes_b64")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|error| anyhow::anyhow!("bytes_b64 is not valid base64: {error}"))?;
    let content_type = required_non_empty_string(&value, "content_type")?.to_string();
    let force_attachment = value
        .get("force_attachment")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("force_attachment must be a boolean"))?;
    let sha256 = required_non_empty_string(&value, "sha256")?.to_string();
    let actual_sha256 = hex_sha256(&bytes);
    if sha256 != actual_sha256 {
        anyhow::bail!(
            "sha256 does not match decoded bytes: expected {sha256}, actual {actual_sha256}"
        );
    }
    Ok(ServedBytes {
        status: 200,
        bytes,
        content_type,
        force_attachment,
        sha256,
    })
}

fn required_non_empty_string<'a>(value: &'a Value, field: &str) -> anyhow::Result<&'a str> {
    let field_value = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{field} must be a string"))?
        .trim();
    if field_value.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    Ok(field_value)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use serde_json::json;

    use super::*;

    #[test]
    fn bytes_from_value_decodes_schema_bound_fetch_projection() {
        let bytes = b"hello pages";
        let served = bytes_from_value(json!({
            "bytes_b64": B64.encode(bytes),
            "content_type": "text/plain; charset=utf-8",
            "force_attachment": false,
            "sha256": hex_sha256(bytes),
        }))
        .expect("valid fetch projection");

        assert_eq!(served.status, 200);
        assert_eq!(served.bytes, bytes);
        assert_eq!(served.content_type, "text/plain; charset=utf-8");
        assert!(!served.force_attachment);
        assert_eq!(served.sha256, hex_sha256(bytes));
    }

    #[test]
    fn bytes_from_value_rejects_missing_bytes_b64() {
        let err = bytes_from_value(json!({
            "content_type": "text/plain",
            "force_attachment": false,
            "sha256": hex_sha256(b""),
        }))
        .expect_err("missing bytes_b64 must not become an empty 200 response");

        assert!(
            err.to_string().contains("bytes_b64 must be a string"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn bytes_from_value_rejects_invalid_base64() {
        let err = bytes_from_value(json!({
            "bytes_b64": "not base64",
            "content_type": "text/plain",
            "force_attachment": false,
            "sha256": hex_sha256(b""),
        }))
        .expect_err("invalid bytes_b64 must fail closed");

        assert!(
            err.to_string().contains("bytes_b64 is not valid base64"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn bytes_from_value_rejects_sha_mismatch() {
        let err = bytes_from_value(json!({
            "bytes_b64": B64.encode(b"actual"),
            "content_type": "text/plain",
            "force_attachment": false,
            "sha256": hex_sha256(b"different"),
        }))
        .expect_err("sha mismatch must fail closed");

        assert!(
            err.to_string()
                .contains("sha256 does not match decoded bytes"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn bytes_from_value_requires_force_attachment_boolean() {
        let err = bytes_from_value(json!({
            "bytes_b64": B64.encode(b"hello"),
            "content_type": "text/plain",
            "force_attachment": "false",
            "sha256": hex_sha256(b"hello"),
        }))
        .expect_err("force_attachment type must be schema-bound");

        assert!(
            err.to_string()
                .contains("force_attachment must be a boolean"),
            "unexpected error: {err}"
        );
    }
}
