// EasyNet CLI — Receipt shared contract
// ======================================
//
// File: src/daemon/receipt_contract.rs
// Description: Shared daemon SDK contract for Receipt fetch carriers and
//              conservative receipt DTO projections.
//
// Protocol Responsibility
// -----------------------
// Own the EasyNet-Cli SDK Receipt profile carrier/projection semantics without
// changing Axon receipt verification rules. This module builds complete
// Invocation carriers for daemon-owned receipt read models and projects
// receipt-like JSON into stable SDK DTOs.
//
// Implementation Approach
// -----------------------
// Reuse the shared daemon SDK carrier builder for `invocation.history.get`.
// Keep receipt verification conservative: summary-shaped data never becomes a
// cryptographic proof, and causal refs require explicit receipt URA/hash facts.
//
// Usage Contract
// --------------
// Carrier construction requires explicit Invocation tuple fields and exactly
// one fetch selector. Projection accepts object-shaped receipt facts and
// rejects missing state for summaries or missing hash anchors for causal refs.
//
// Architectural Position
// ----------------------
// EasyNet-Cli daemon SDK Receipt profile. Runtime Core remains the only
// submit path for returned carriers; Axon remains the verifier authority for
// full cryptographic receipt verification.

use serde_json::{json, Map, Value};

use crate::daemon::sdk_contract::{
    build_system_invocation, object, optional_string_field, SdkContractError,
};

const RECEIPT_PROFILE: &str = "receipt";
const ABILITY_INVOCATION_HISTORY_GET: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_GET;

pub(crate) type ReceiptError = SdkContractError;

pub(crate) fn build_fetch_invocation(request: &Value) -> Result<Value, ReceiptError> {
    let obj = object(request, "ReceiptFetchRequest")?;
    reject_unsupported_fields(obj, RECEIPT_FETCH_REQUEST_FIELDS)?;
    let args = fetch_args(obj)?;
    build_system_invocation(obj, RECEIPT_PROFILE, ABILITY_INVOCATION_HISTORY_GET, args)
}

pub(crate) fn project_receipt_summary(input: &Value) -> Result<Value, ReceiptError> {
    let projection = ReceiptProjection::from_value(input)?;
    if projection.state.is_none() {
        return Err(ReceiptError::MissingField("state"));
    }
    Ok(projection.summary_json())
}

pub(crate) fn project_receipt_verification(input: &Value) -> Result<Value, ReceiptError> {
    Ok(ReceiptProjection::from_value_lossy(input)?.verification_json())
}

pub(crate) fn project_receipt_chain_verification(input: &Value) -> Result<Value, ReceiptError> {
    ReceiptChainProjection::from_value(input)?.verification_json()
}

pub(crate) fn project_causal_ref(input: &Value) -> Result<Value, ReceiptError> {
    ReceiptProjection::from_value_lossy(input)?.causal_ref_json()
}

const RECEIPT_FETCH_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
    "invocation_ura",
    "request_id",
    "trace_id",
];

fn reject_unsupported_fields(
    obj: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), ReceiptError> {
    for key in obj.keys() {
        if !allowed.iter().any(|allowed| allowed == key) {
            return Err(ReceiptError::InvalidField(
                "request",
                format!("unsupported field `{key}`"),
            ));
        }
    }
    Ok(())
}

fn fetch_args(obj: &Map<String, Value>) -> Result<Value, ReceiptError> {
    let selectors = [
        (
            "invocation_ura",
            "ura",
            optional_string_field(obj, "invocation_ura")?,
        ),
        (
            "request_id",
            "request_id",
            optional_string_field(obj, "request_id")?,
        ),
        (
            "trace_id",
            "trace_id",
            optional_string_field(obj, "trace_id")?,
        ),
    ];
    let selected = selectors
        .iter()
        .filter_map(|(public, daemon, value)| value.as_ref().map(|value| (*public, *daemon, value)))
        .collect::<Vec<_>>();
    match selected.as_slice() {
        [] => Err(ReceiptError::MissingField("invocation_ura")),
        [(_, daemon, value)] => Ok(json!({
            "key": {
                *daemon: value,
            }
        })),
        _ => Err(ReceiptError::InvalidField(
            "request",
            "must include exactly one of invocation_ura, request_id, or trace_id".to_string(),
        )),
    }
}

#[derive(Debug, Clone)]
struct ReceiptProjection {
    receipt_ura: Option<String>,
    invocation_id: Option<String>,
    state: Option<String>,
    verified_input: bool,
    output: Value,
    error: Value,
    causal_ref: Option<String>,
    receipt_hash_hex: Option<String>,
    prev_receipt_hash_hex: Option<String>,
    metadata: Map<String, Value>,
}

impl ReceiptProjection {
    fn from_value(input: &Value) -> Result<Self, ReceiptError> {
        Ok(Self::from_object_lossy(object(input, "Receipt")?.clone()))
    }

    fn from_value_lossy(input: &Value) -> Result<Self, ReceiptError> {
        Ok(Self::from_object_lossy(object(input, "Receipt")?.clone()))
    }

    fn from_object_lossy(obj: Map<String, Value>) -> Self {
        let receipt_ura = optional_string(&obj, "receipt_ura");
        let invocation_id = optional_string(&obj, "invocation_id");
        let state =
            optional_string(&obj, "state").or_else(|| optional_string(&obj, "terminal_state"));
        let verified_input = obj
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let output = obj
            .get("output")
            .or_else(|| obj.get("output_json"))
            .or_else(|| obj.get("payload_json"))
            .or_else(|| obj.get("result_json"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let error = obj.get("error").cloned().unwrap_or(Value::Null);
        let causal_ref = optional_string(&obj, "causal_ref");
        let receipt_hash_hex = receipt_hash_hex(&obj);
        let prev_receipt_hash_hex =
            receipt_hash_value_hex(&obj, &["prev_receipt_hash_hex", "parent_receipt_hash_hex"]);
        let metadata = receipt_metadata(&obj, verified_input, receipt_hash_hex.as_deref());
        Self {
            receipt_ura,
            invocation_id,
            state,
            verified_input,
            output,
            error,
            causal_ref,
            receipt_hash_hex,
            prev_receipt_hash_hex,
            metadata,
        }
    }

    fn summary_json(&self) -> Value {
        json!({
            "receipt_ura": self.receipt_ura,
            "invocation_id": self.invocation_id,
            "state": self.state.as_deref().unwrap_or("unknown"),
            "verified": false,
            "output": self.output,
            "error": self.error,
            "causal_ref": self.causal_ref,
            "metadata": self.metadata,
        })
    }

    fn verification_json(&self) -> Value {
        json!({
            "verified": false,
            "method": "summary_projection",
            "reason": "Receipt profile projection does not perform Axon cryptographic receipt verification",
            "requires_full_receipt": true,
            "receipt_ura": self.receipt_ura,
            "invocation_id": self.invocation_id,
            "state": self.state,
            "metadata": {
                "has_receipt_hash": self.receipt_hash_hex.is_some(),
                "verified_input_downgraded": self.verified_input,
            },
        })
    }

    fn causal_ref_json(&self) -> Result<Value, ReceiptError> {
        let receipt_ura = self
            .receipt_ura
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(ReceiptError::MissingField("receipt_ura"))?;
        let receipt_hash_hex = self.receipt_hash_hex.as_deref().ok_or_else(|| {
            ReceiptError::InvalidField(
                "receipt_hash",
                "missing receipt hash field `self_hash_hex`, `receipt_hash_hex`, or `receipt_hash`"
                    .to_string(),
            )
        })?;
        validate_hash_hex(receipt_hash_hex)?;
        Ok(json!({
            "receipt_ura": receipt_ura,
            "receipt_hash_hex": receipt_hash_hex,
            "verified": false,
            "causal_context": {
                "form": "scalar",
                "receipt_ura": receipt_ura,
                "receipt_hash_hex": receipt_hash_hex,
            },
        }))
    }
}

#[derive(Debug)]
struct ReceiptChainProjection {
    items: Vec<ReceiptProjection>,
}

impl ReceiptChainProjection {
    fn from_value(input: &Value) -> Result<Self, ReceiptError> {
        let obj = object(input, "ReceiptChainVerificationRequest")?;
        reject_unsupported_fields(obj, &["receipts", "metadata"])?;
        let receipts = obj
            .get("receipts")
            .and_then(Value::as_array)
            .ok_or(ReceiptError::MissingField("receipts"))?;
        if receipts.is_empty() {
            return Err(ReceiptError::InvalidField(
                "receipts",
                "must include at least one receipt".to_string(),
            ));
        }

        let mut seen_hashes = std::collections::BTreeSet::new();
        let mut items = Vec::with_capacity(receipts.len());
        for receipt in receipts {
            let projection = ReceiptProjection::from_value_lossy(receipt)?;
            let hash = projection.receipt_hash_hex.as_deref().ok_or_else(|| {
                ReceiptError::InvalidField(
                    "receipt_hash",
                    "each chain receipt must include self_hash_hex, receipt_hash_hex, or receipt_hash"
                        .to_string(),
                )
            })?;
            validate_hash_hex(hash)?;
            if !seen_hashes.insert(hash.to_string()) {
                return Err(ReceiptError::InvalidField(
                    "receipts",
                    "duplicate receipt hash in chain".to_string(),
                ));
            }
            items.push(projection);
        }

        Ok(Self { items })
    }

    fn verification_json(&self) -> Result<Value, ReceiptError> {
        let mut projected_items = Vec::with_capacity(self.items.len());
        let mut continuous = true;
        let mut previous_hash: Option<&str> = None;

        for (index, item) in self.items.iter().enumerate() {
            let receipt_ura = item
                .receipt_ura
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or(ReceiptError::MissingField("receipt_ura"))?;
            let receipt_hash_hex = item
                .receipt_hash_hex
                .as_deref()
                .ok_or(ReceiptError::MissingField("receipt_hash"))?;
            if let Some(parent_hash) = item.prev_receipt_hash_hex.as_deref() {
                validate_hash_hex(parent_hash)?;
            }
            let item_continuous =
                match (index, previous_hash, item.prev_receipt_hash_hex.as_deref()) {
                    (0, _, _) => true,
                    (_, Some(previous), Some(parent)) => parent == previous,
                    _ => false,
                };
            if !item_continuous {
                continuous = false;
            }
            projected_items.push(json!({
                "index": index,
                "receipt_ura": receipt_ura,
                "invocation_id": item.invocation_id,
                "receipt_hash_hex": receipt_hash_hex,
                "prev_receipt_hash_hex": item.prev_receipt_hash_hex.as_deref(),
                "continuous": item_continuous,
                "metadata": item.metadata.clone(),
            }));
            previous_hash = Some(receipt_hash_hex);
        }

        let root_receipt_ura = self
            .items
            .first()
            .and_then(|item| item.receipt_ura.as_deref());
        let terminal_receipt_ura = self
            .items
            .last()
            .and_then(|item| item.receipt_ura.as_deref());
        Ok(json!({
            "verified": false,
            "continuous": continuous,
            "method": "daemon_receipt_chain_continuity",
            "reason": "Receipt chain continuity was projected by daemon receipt facts; Axon cryptographic verification requires full receipt authority",
            "requires_full_receipt": true,
            "root_receipt_ura": root_receipt_ura,
            "terminal_receipt_ura": terminal_receipt_ura,
            "receipt_count": self.items.len(),
            "items": projected_items,
            "metadata": {
                "chain_projection": "hash_continuity",
            },
        }))
    }
}

fn optional_string(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn receipt_hash_hex(obj: &Map<String, Value>) -> Option<String> {
    receipt_hash_value_hex(obj, &["self_hash_hex", "receipt_hash_hex", "receipt_hash"])
}

fn receipt_hash_value_hex(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(raw) = optional_string(obj, key) else {
            continue;
        };
        let raw = raw.strip_prefix("sha256:").unwrap_or(&raw);
        let normalized = raw.trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }
    None
}

fn validate_hash_hex(raw: &str) -> Result<(), ReceiptError> {
    let decoded = hex::decode(raw).map_err(|_| {
        ReceiptError::InvalidField(
            "receipt_hash",
            "receipt hash must decode to exactly 32 bytes".to_string(),
        )
    })?;
    if decoded.len() != 32 {
        return Err(ReceiptError::InvalidField(
            "receipt_hash",
            "receipt hash must decode to exactly 32 bytes".to_string(),
        ));
    }
    Ok(())
}

fn receipt_metadata(
    obj: &Map<String, Value>,
    verified_input: bool,
    receipt_hash_hex: Option<&str>,
) -> Map<String, Value> {
    let mut metadata = obj
        .get("metadata")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for key in [
        "index",
        "receipt_type",
        "timestamp_unix_ms",
        "prev_receipt_hash_hex",
        "payload_content_type",
        "cleanup_complete",
        "reason",
        "child_invocation_id",
    ] {
        if let Some(value) = obj.get(key) {
            metadata.insert(key.to_string(), value.clone());
        }
    }
    if let Some(hash) = receipt_hash_hex {
        metadata.insert(
            "receipt_hash_hex".to_string(),
            Value::String(hash.to_string()),
        );
    }
    if verified_input {
        metadata.insert(
            "verification_claim_downgraded".to_string(),
            Value::Bool(true),
        );
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request(extra: Value) -> Value {
        let mut request = json!({
            "caller_ura": "easynet:///r/example/agent/alice.sdk",
            "callee_ura": "easynet:///r/example/device/dev-a",
            "subject_ura": "easynet:///r/example/device/dev-a",
            "descriptor_version": "1.0.0",
            "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
            "causal_context": {"form": "none"},
            "metadata": {"request_id": "receipt-fetch-1"}
        });
        let Value::Object(extra) = extra else {
            return request;
        };
        let obj = request.as_object_mut().unwrap();
        for (key, value) in extra {
            obj.insert(key, value);
        }
        request
    }

    #[test]
    fn build_fetch_invocation_targets_invocation_history_get() {
        let request = base_request(json!({"request_id": "req-123"}));

        let invocation = build_fetch_invocation(&request).unwrap();

        assert_eq!(
            invocation["metadata"]["system_ability"],
            ABILITY_INVOCATION_HISTORY_GET
        );
        assert_eq!(
            invocation["args"],
            json!({"key": {"request_id": "req-123"}})
        );
        assert_eq!(
            invocation["descriptor_ref"],
            "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0"
        );
    }

    #[test]
    fn build_fetch_invocation_rejects_ambiguous_selector() {
        let request = base_request(json!({
            "request_id": "req-123",
            "trace_id": "trace-1"
        }));

        let err = build_fetch_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn project_summary_downgrades_verification_claim() {
        let summary = project_receipt_summary(&json!({
            "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
            "invocation_id": "inv-1",
            "state": "completed",
            "verified": true,
            "output": {"ok": true},
            "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "metadata": {"source": "test"}
        }))
        .unwrap();

        assert_eq!(summary["state"], "completed");
        assert_eq!(summary["verified"], false);
        assert_eq!(summary["output"]["ok"], true);
        assert_eq!(summary["metadata"]["source"], "test");
        assert_eq!(summary["metadata"]["verification_claim_downgraded"], true);
    }

    #[test]
    fn project_summary_rejects_missing_state() {
        let err = project_receipt_summary(&json!({"invocation_id": "inv-1"})).unwrap_err();

        assert!(err.to_string().contains("state"));
    }

    #[test]
    fn project_verification_is_conservative_for_summary_input() {
        let verification = project_receipt_verification(&json!({
            "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
            "invocation_id": "inv-1",
            "state": "completed",
            "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .unwrap();

        assert_eq!(verification["verified"], false);
        assert_eq!(verification["method"], "summary_projection");
        assert_eq!(verification["metadata"]["has_receipt_hash"], true);
    }

    #[test]
    fn project_chain_verification_reports_continuity_without_crypto_upgrade() {
        let verification = project_receipt_chain_verification(&json!({
            "receipts": [
                {
                    "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                    "invocation_id": "inv-1",
                    "state": "completed",
                    "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                },
                {
                    "receipt_ura": "easynet:///r/acme/resource/invocations/inv-2/receipt/1",
                    "invocation_id": "inv-2",
                    "state": "completed",
                    "self_hash_hex": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "prev_receipt_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            ]
        }))
        .unwrap();

        assert_eq!(verification["verified"], false);
        assert_eq!(verification["continuous"], true);
        assert_eq!(verification["method"], "daemon_receipt_chain_continuity");
        assert_eq!(verification["receipt_count"], 2);
        assert_eq!(verification["items"][1]["continuous"], true);
    }

    #[test]
    fn project_chain_verification_marks_broken_parent_hash() {
        let verification = project_receipt_chain_verification(&json!({
            "receipts": [
                {
                    "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                    "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                },
                {
                    "receipt_ura": "easynet:///r/acme/resource/invocations/inv-2/receipt/1",
                    "self_hash_hex": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "prev_receipt_hash_hex": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                }
            ]
        }))
        .unwrap();

        assert_eq!(verification["continuous"], false);
        assert_eq!(verification["items"][1]["continuous"], false);
    }

    #[test]
    fn project_chain_verification_rejects_duplicate_hash() {
        let err = project_receipt_chain_verification(&json!({
            "receipts": [
                {
                    "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
                    "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                },
                {
                    "receipt_ura": "easynet:///r/acme/resource/invocations/inv-2/receipt/1",
                    "self_hash_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            ]
        }))
        .unwrap_err();

        assert!(err.to_string().contains("duplicate receipt hash"));
    }

    #[test]
    fn causal_ref_requires_explicit_hash_pair() {
        let err = project_causal_ref(&json!({
            "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
            "state": "completed"
        }))
        .unwrap_err();

        assert!(err.to_string().contains("receipt hash"));
    }

    #[test]
    fn causal_ref_builds_scalar_context_from_hash_pair() {
        let causal = project_causal_ref(&json!({
            "receipt_ura": "easynet:///r/acme/resource/invocations/inv-1/receipt/1",
            "state": "completed",
            "receipt_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }))
        .unwrap();

        assert_eq!(
            causal["causal_context"]["receipt_hash_hex"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(causal["verified"], false);
    }
}
