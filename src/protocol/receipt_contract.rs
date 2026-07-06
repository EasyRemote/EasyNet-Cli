// EasyNet CLI — Receipt shared contract
// ======================================
//
// File: src/protocol/receipt_contract.rs
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

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use easynet_axon::invocation::{
    canonical_receipt_bytes_with_hosted, verify_receipt_signature_with_hosted, AxonError,
    CalleeSignature, KeyResolver, ReceiptJson,
};
use ed25519_dalek::VerifyingKey;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::protocol::sdk_contract::{
    build_system_invocation, object, optional_string_field, required_string, system_descriptor_ref,
    SdkContractError,
};

const RECEIPT_PROFILE: &str = "receipt";
const ABILITY_INVOCATION_HISTORY_GET: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_GET;
const ABILITY_INVOCATION_HISTORY_LIST: &str =
    crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST;
const ABILITY_INVOCATION_TRACE_GET: &str =
    crate::daemon::ability::names::governance::INVOCATION_TRACE_GET;

pub(crate) type ReceiptError = SdkContractError;

pub(crate) fn build_fetch_invocation(request: &Value) -> Result<Value, ReceiptError> {
    let obj = object(request, "ReceiptFetchRequest")?;
    reject_unsupported_fields(obj, RECEIPT_FETCH_REQUEST_FIELDS)?;
    validate_fetch_descriptor_ref(obj)?;
    let args = fetch_args(obj)?;
    build_system_invocation(obj, RECEIPT_PROFILE, ABILITY_INVOCATION_HISTORY_GET, args)
}

pub(crate) fn build_list_history_invocation(request: &Value) -> Result<Value, ReceiptError> {
    build_history_invocation(request, ABILITY_INVOCATION_HISTORY_LIST)
}

pub(crate) fn build_get_history_invocation(request: &Value) -> Result<Value, ReceiptError> {
    build_history_invocation(request, ABILITY_INVOCATION_HISTORY_GET)
}

pub(crate) fn build_trace_invocation(request: &Value) -> Result<Value, ReceiptError> {
    build_history_invocation(request, ABILITY_INVOCATION_TRACE_GET)
}

pub(crate) fn project_receipt_summary(input: &Value) -> Result<Value, ReceiptError> {
    let projection = ReceiptProjection::from_value(input)?;
    if projection.state.is_none() {
        return Err(ReceiptError::MissingField("state"));
    }
    Ok(projection.summary_json())
}

pub(crate) fn project_receipt_verification(input: &Value) -> Result<Value, ReceiptError> {
    if let Some(verification) = project_axon_receipt_verification(input)? {
        return Ok(verification);
    }
    Ok(ReceiptProjection::from_value_lossy(input)?.verification_json())
}

pub(crate) fn project_receipt_chain_verification(input: &Value) -> Result<Value, ReceiptError> {
    if let Some(verification) = project_axon_receipt_chain_verification(input)? {
        return Ok(verification);
    }
    ReceiptChainProjection::from_value(input)?.verification_json()
}

pub(crate) fn project_causal_ref(input: &Value) -> Result<Value, ReceiptError> {
    ReceiptProjection::from_value_lossy(input)?.causal_ref_json()
}

const RECEIPT_FETCH_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "descriptor_ref",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "metadata",
    "invocation_ura",
    "request_id",
    "trace_id",
];

const RECEIPT_HISTORY_REQUEST_FIELDS: &[&str] = &[
    "caller_ura",
    "callee_ura",
    "subject_ura",
    "descriptor_version",
    "nonce_base64",
    "causal_context",
    "timeout_ms",
    "metadata",
    "arguments",
];

fn build_history_invocation(request: &Value, ability_name: &str) -> Result<Value, ReceiptError> {
    let obj = object(request, "ReceiptHistoryReadRequest")?;
    reject_unsupported_fields(obj, RECEIPT_HISTORY_REQUEST_FIELDS)?;
    let args = history_args(obj)?;
    let prepared = with_history_metadata(obj)?;
    build_system_invocation(&prepared, RECEIPT_PROFILE, ability_name, args)
}

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

fn validate_fetch_descriptor_ref(obj: &Map<String, Value>) -> Result<(), ReceiptError> {
    let descriptor_ref = required_string(obj, "descriptor_ref")?;
    let callee_ura = required_string(obj, "callee_ura")?;
    let descriptor_version = required_string(obj, "descriptor_version")?;
    let expected = system_descriptor_ref(
        callee_ura,
        ABILITY_INVOCATION_HISTORY_GET,
        descriptor_version,
    )?;
    if descriptor_ref != expected {
        return Err(ReceiptError::InvalidField(
            "descriptor_ref",
            format!(
                "must match daemon/Axon descriptor ref for {ABILITY_INVOCATION_HISTORY_GET}: {expected}"
            ),
        ));
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

fn history_args(obj: &Map<String, Value>) -> Result<Value, ReceiptError> {
    match obj.get("arguments") {
        None | Some(Value::Null) => Ok(json!({})),
        Some(Value::Object(args)) => Ok(Value::Object(args.clone())),
        Some(_) => Err(ReceiptError::InvalidField(
            "arguments",
            "must be an object".to_string(),
        )),
    }
}

fn with_history_metadata(obj: &Map<String, Value>) -> Result<Map<String, Value>, ReceiptError> {
    let mut prepared = obj.clone();
    if let Some(timeout_ms) = optional_timeout_ms(obj)? {
        let mut metadata = match prepared.remove("metadata") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(metadata)) => metadata,
            Some(_) => {
                return Err(ReceiptError::InvalidField(
                    "metadata",
                    "must be an object or null".to_string(),
                ))
            }
        };
        metadata.insert("timeout_ms".to_string(), Value::Number(timeout_ms.into()));
        prepared.insert("metadata".to_string(), Value::Object(metadata));
    }
    Ok(prepared)
}

fn optional_timeout_ms(obj: &Map<String, Value>) -> Result<Option<u64>, ReceiptError> {
    match obj.get("timeout_ms") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value.as_u64().map(Some).ok_or_else(|| {
            ReceiptError::InvalidField("timeout_ms", "must be a non-negative integer".to_string())
        }),
        Some(_) => Err(ReceiptError::InvalidField(
            "timeout_ms",
            "must be a non-negative integer".to_string(),
        )),
    }
}

fn project_axon_receipt_verification(input: &Value) -> Result<Option<Value>, ReceiptError> {
    let obj = object(input, "ReceiptVerificationRequest")?;
    let Some(receipt_value) = obj.get("receipt") else {
        return Ok(None);
    };
    let Some(keys_value) = obj.get("public_keys") else {
        return Ok(None);
    };
    let resolver = InlineReceiptKeyResolver::from_value(keys_value)?;
    let receipt_ura = optional_string(obj, "receipt_ura");
    let receipt = serde_json::from_value::<ReceiptJson>(receipt_value.clone()).map_err(|err| {
        ReceiptError::InvalidField(
            "receipt",
            format!("invalid Axon receipt bundle JSON: {err}"),
        )
    })?;
    Ok(Some(verify_axon_receipt_json(
        &receipt,
        receipt_ura,
        &resolver,
    )))
}

fn verify_axon_receipt_json(
    receipt: &ReceiptJson,
    receipt_ura: Option<String>,
    resolver: &InlineReceiptKeyResolver,
) -> Value {
    let invocation_id = Some(receipt.invocation_id.clone());
    let verified_result = verify_axon_receipt_signature(receipt, resolver);

    match verified_result {
        Ok(verified) => json!({
            "verified": true,
            "receipt_ura": receipt_ura,
            "invocation_id": invocation_id,
            "method": "axon_receipt_signature",
            "reason": "",
            "metadata": {
                "source": "axon",
                "assurance": "cryptographic",
                "verifier": "easynet_axon::invocation::verify_receipt_signature_with_hosted",
                "signature_algorithm": verified.signature_algorithm,
                "self_hash_hex": verified.self_hash_hex,
            },
        }),
        Err(err) => json!({
            "verified": false,
            "receipt_ura": receipt_ura,
            "invocation_id": invocation_id,
            "method": "axon_receipt_signature",
            "reason": err.reason,
            "metadata": {
                "source": "axon",
                "assurance": "cryptographic",
                "verifier": "easynet_axon::invocation::verify_receipt_signature_with_hosted",
                "signature_algorithm": receipt.callee_signature_alg,
            },
        }),
    }
}

fn project_axon_receipt_chain_verification(input: &Value) -> Result<Option<Value>, ReceiptError> {
    let obj = object(input, "ReceiptChainVerificationRequest")?;
    let Some(keys_value) = obj.get("public_keys") else {
        return Ok(None);
    };
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
    let resolver = InlineReceiptKeyResolver::from_value(keys_value)?;

    let mut items = Vec::with_capacity(receipts.len());
    let mut verified = true;
    let mut continuous = true;
    let mut reason = String::new();
    let mut invocation_id: Option<String> = None;
    let mut previous_hash_hex: Option<String> = None;

    for (position, receipt_value) in receipts.iter().enumerate() {
        let item = AxonReceiptChainItem::from_value(receipt_value)?;
        let receipt_ura = item.receipt_ura.clone();
        let receipt = item.receipt;
        let mut item_verified = true;
        let mut item_continuous = true;
        let mut item_reason = String::new();

        let verification = match verify_axon_receipt_signature(&receipt, &resolver) {
            Ok(verification) => verification,
            Err(err) => {
                verified = false;
                continuous = false;
                item_verified = false;
                item_continuous = false;
                item_reason = err.reason;
                if reason.is_empty() {
                    reason = format!("receipt_{position}_{}", item_reason);
                }
                AxonReceiptVerification {
                    self_hash_hex: String::new(),
                    signature_algorithm: receipt.callee_signature_alg.clone(),
                }
            }
        };

        if receipt.index != position as u64 {
            verified = false;
            continuous = false;
            item_continuous = false;
            if item_reason.is_empty() {
                item_reason = format!(
                    "receipt_index_gap:expected_{position}_got_{}",
                    receipt.index
                );
            }
            if reason.is_empty() {
                reason = item_reason.clone();
            }
        }
        match invocation_id.as_deref() {
            None => invocation_id = Some(receipt.invocation_id.clone()),
            Some(expected) if expected == receipt.invocation_id => {}
            Some(expected) => {
                verified = false;
                continuous = false;
                item_continuous = false;
                if item_reason.is_empty() {
                    item_reason = format!(
                        "receipt_invocation_id_mismatch:expected_{expected}_got_{}",
                        receipt.invocation_id
                    );
                }
                if reason.is_empty() {
                    reason = item_reason.clone();
                }
            }
        }

        let prev_hash_hex = receipt.prev_receipt_hash_hex.to_ascii_lowercase();
        let expected_prev = previous_hash_hex
            .as_deref()
            .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000");
        if prev_hash_hex != expected_prev {
            verified = false;
            continuous = false;
            item_continuous = false;
            if item_reason.is_empty() {
                item_reason = format!("receipt_prev_hash_mismatch:index_{position}");
            }
            if reason.is_empty() {
                reason = item_reason.clone();
            }
        }

        if item_verified {
            previous_hash_hex = Some(verification.self_hash_hex.clone());
        }
        items.push(json!({
            "index": position,
            "receipt_ura": receipt_ura,
            "invocation_id": receipt.invocation_id,
            "receipt_hash_hex": if verification.self_hash_hex.is_empty() {
                Value::Null
            } else {
                Value::String(verification.self_hash_hex.clone())
            },
            "prev_receipt_hash_hex": prev_hash_hex,
            "continuous": item_continuous,
            "verified": item_verified,
            "reason": item_reason,
            "metadata": {
                "source": "axon",
                "assurance": "cryptographic",
                "receipt_index": receipt.index,
                "receipt_type": receipt.receipt_type,
                "state": receipt.state,
                "signature_algorithm": verification.signature_algorithm,
            },
        }));
    }

    let root_receipt_ura = items
        .first()
        .and_then(|item| item.get("receipt_ura"))
        .cloned()
        .unwrap_or(Value::Null);
    let terminal_receipt_ura = items
        .last()
        .and_then(|item| item.get("receipt_ura"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Some(json!({
        "verified": verified,
        "continuous": continuous,
        "method": "axon_receipt_chain_signature",
        "reason": reason,
        "requires_full_receipt": false,
        "root_receipt_ura": root_receipt_ura,
        "terminal_receipt_ura": terminal_receipt_ura,
        "receipt_count": items.len(),
        "items": items,
        "metadata": {
            "source": "axon",
            "assurance": "cryptographic",
            "verifier": "easynet_axon::invocation::verify_receipt_signature_with_hosted",
            "chain_projection": "single_invocation_signature_chain",
            "dag_verified": false,
        },
    })))
}

fn verify_axon_receipt_signature(
    receipt: &ReceiptJson,
    resolver: &InlineReceiptKeyResolver,
) -> Result<AxonReceiptVerification, AxonError> {
    let (body, signature, algorithm) = receipt.to_body()?;
    let (signer_binding, host_attestation) = receipt.hosted_attestation()?;
    let hosted = ReceiptJson::hosted_attestation_ref(&signer_binding, &host_attestation);
    let signature = CalleeSignature {
        algorithm: algorithm.clone(),
        signature,
        key_id_hint: String::new(),
    };
    verify_receipt_signature_with_hosted(&body, hosted, &signature, resolver)?;
    let canonical = canonical_receipt_bytes_with_hosted(&body, hosted);
    Ok(AxonReceiptVerification {
        self_hash_hex: hex::encode(Sha256::digest(canonical)),
        signature_algorithm: algorithm,
    })
}

struct AxonReceiptVerification {
    self_hash_hex: String,
    signature_algorithm: String,
}

struct AxonReceiptChainItem {
    receipt_ura: String,
    receipt: ReceiptJson,
}

impl AxonReceiptChainItem {
    fn from_value(value: &Value) -> Result<Self, ReceiptError> {
        let obj = object(value, "AxonReceiptChainItem")?;
        let receipt_ura =
            optional_string(obj, "receipt_ura").ok_or(ReceiptError::MissingField("receipt_ura"))?;
        let receipt_value = obj.get("receipt").unwrap_or(value);
        let receipt =
            serde_json::from_value::<ReceiptJson>(receipt_value.clone()).map_err(|err| {
                ReceiptError::InvalidField(
                    "receipt",
                    format!("invalid Axon receipt bundle JSON: {err}"),
                )
            })?;
        Ok(Self {
            receipt_ura,
            receipt,
        })
    }
}

struct InlineReceiptKeyResolver {
    keys: BTreeMap<String, VerifyingKey>,
}

impl InlineReceiptKeyResolver {
    fn from_value(value: &Value) -> Result<Self, ReceiptError> {
        let obj = value.as_object().ok_or_else(|| {
            ReceiptError::InvalidField("public_keys", "must be an object".to_string())
        })?;
        if obj.is_empty() {
            return Err(ReceiptError::InvalidField(
                "public_keys",
                "must contain at least one signer URA".to_string(),
            ));
        }
        let mut keys = BTreeMap::new();
        for (agent_ura, value) in obj {
            let key_bytes = public_key_bytes(value)?;
            let key = VerifyingKey::from_bytes(&key_bytes).map_err(|err| {
                ReceiptError::InvalidField(
                    "public_keys",
                    format!("invalid Ed25519 public key for {agent_ura}: {err}"),
                )
            })?;
            keys.insert(agent_ura.clone(), key);
        }
        Ok(Self { keys })
    }
}

impl KeyResolver for InlineReceiptKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        self.keys
            .get(agent_ura)
            .copied()
            .ok_or_else(|| AxonError::invalid_argument(format!("no_pubkey_for_agent:{agent_ura}")))
    }
}

fn public_key_bytes(value: &Value) -> Result<[u8; 32], ReceiptError> {
    let raw = match value {
        Value::String(raw) => decode_public_key(raw)?,
        Value::Object(obj) => {
            if let Some(raw) = optional_string(obj, "public_key_hex") {
                decode_hex_public_key(&raw)?
            } else if let Some(raw) = optional_string(obj, "public_key_base64") {
                decode_base64_public_key(&raw)?
            } else {
                return Err(ReceiptError::InvalidField(
                    "public_keys",
                    "key object must contain public_key_hex or public_key_base64".to_string(),
                ));
            }
        }
        _ => {
            return Err(ReceiptError::InvalidField(
                "public_keys",
                "key value must be a string or object".to_string(),
            ))
        }
    };
    raw.as_slice().try_into().map_err(|_| {
        ReceiptError::InvalidField(
            "public_keys",
            format!("Ed25519 public key must be 32 bytes, got {}", raw.len()),
        )
    })
}

fn decode_public_key(raw: &str) -> Result<Vec<u8>, ReceiptError> {
    let raw = raw.trim();
    if raw.strip_prefix("hex:").is_some() || raw.len() == 64 {
        return decode_hex_public_key(raw.strip_prefix("hex:").unwrap_or(raw));
    }
    decode_base64_public_key(raw.strip_prefix("base64:").unwrap_or(raw))
}

fn decode_hex_public_key(raw: &str) -> Result<Vec<u8>, ReceiptError> {
    hex::decode(raw.trim()).map_err(|err| {
        ReceiptError::InvalidField("public_keys", format!("invalid public key hex: {err}"))
    })
}

fn decode_base64_public_key(raw: &str) -> Result<Vec<u8>, ReceiptError> {
    BASE64.decode(raw.trim()).map_err(|err| {
        ReceiptError::InvalidField("public_keys", format!("invalid public key base64: {err}"))
    })
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
            "descriptor_ref": "easynet:///r/example/ability/device.dev-a.invocation.history.get@1.0.0",
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
    fn build_fetch_invocation_requires_descriptor_ref_from_request() {
        let mut request = base_request(json!({"request_id": "req-123"}));
        request.as_object_mut().unwrap().remove("descriptor_ref");

        let err = build_fetch_invocation(&request).unwrap_err();

        assert!(err.to_string().contains("descriptor_ref"));
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
    fn project_verification_delegates_full_receipt_signature_to_axon() {
        let (receipt, public_key_hex) = signed_axon_receipt_fixture();

        let verification = project_receipt_verification(&json!({
            "receipt_ura": "easynet:///r/example/receipt/inv-axon/0",
            "receipt": receipt,
            "public_keys": {
                "easynet:///r/example/agent/alice.worker": public_key_hex
            }
        }))
        .unwrap();

        assert_eq!(verification["verified"], true);
        assert_eq!(verification["method"], "axon_receipt_signature");
        assert_eq!(verification["invocation_id"], "inv-axon");
        assert_eq!(verification["metadata"]["source"], "axon");
        assert_eq!(verification["metadata"]["assurance"], "cryptographic");
        assert!(
            verification["metadata"]["self_hash_hex"]
                .as_str()
                .unwrap()
                .len()
                == 64
        );
    }

    #[test]
    fn project_verification_rejects_tampered_axon_receipt_signature() {
        let (mut receipt, public_key_hex) = signed_axon_receipt_fixture();
        receipt["callee_signature_hex"] = json!("00".repeat(64));

        let verification = project_receipt_verification(&json!({
            "receipt": receipt,
            "public_keys": {
                "easynet:///r/example/agent/alice.worker": public_key_hex
            }
        }))
        .unwrap();

        assert_eq!(verification["verified"], false);
        assert_eq!(verification["method"], "axon_receipt_signature");
        assert!(verification["reason"]
            .as_str()
            .unwrap()
            .contains("callee_signature_invalid"));
    }

    #[test]
    fn project_chain_verification_delegates_full_receipt_chain_to_axon() {
        let (receipts, public_key_hex) = signed_axon_receipt_chain_fixture();

        let verification = project_receipt_chain_verification(&json!({
            "receipts": [
                {
                    "receipt_ura": "easynet:///r/example/receipt/inv-axon/0",
                    "receipt": receipts[0].clone()
                },
                {
                    "receipt_ura": "easynet:///r/example/receipt/inv-axon/1",
                    "receipt": receipts[1].clone()
                }
            ],
            "public_keys": {
                "easynet:///r/example/agent/alice.worker": public_key_hex
            }
        }))
        .unwrap();

        assert_eq!(verification["verified"], true);
        assert_eq!(verification["continuous"], true);
        assert_eq!(verification["method"], "axon_receipt_chain_signature");
        assert_eq!(verification["receipt_count"], 2);
        assert_eq!(verification["items"][0]["verified"], true);
        assert_eq!(verification["items"][1]["continuous"], true);
        assert_eq!(verification["metadata"]["assurance"], "cryptographic");
        assert_eq!(verification["metadata"]["dag_verified"], false);
    }

    #[test]
    fn project_chain_verification_rejects_tampered_axon_prev_hash() {
        let (mut receipts, public_key_hex) = signed_axon_receipt_chain_fixture();
        receipts[1]["prev_receipt_hash_hex"] =
            json!("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc");

        let verification = project_receipt_chain_verification(&json!({
            "receipts": [
                {
                    "receipt_ura": "easynet:///r/example/receipt/inv-axon/0",
                    "receipt": receipts[0].clone()
                },
                {
                    "receipt_ura": "easynet:///r/example/receipt/inv-axon/1",
                    "receipt": receipts[1].clone()
                }
            ],
            "public_keys": {
                "easynet:///r/example/agent/alice.worker": public_key_hex
            }
        }))
        .unwrap();

        assert_eq!(verification["verified"], false);
        assert_eq!(verification["continuous"], false);
        assert_eq!(verification["items"][1]["verified"], false);
        assert!(verification["reason"]
            .as_str()
            .unwrap()
            .contains("callee_signature_invalid"));
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

    fn signed_axon_receipt_fixture() -> (Value, String) {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        (
            signed_axon_receipt_value(&signing_key, 0, [0u8; 32], "terminal", "completed"),
            hex::encode(signing_key.verifying_key().to_bytes()),
        )
    }

    fn signed_axon_receipt_chain_fixture() -> (Vec<Value>, String) {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let first = signed_axon_receipt_value(&signing_key, 0, [0u8; 32], "accepted", "accepted");
        let first_hash = axon_receipt_hash(&first);
        let second =
            signed_axon_receipt_value(&signing_key, 1, first_hash, "terminal", "completed");
        (
            vec![first, second],
            hex::encode(signing_key.verifying_key().to_bytes()),
        )
    }

    fn axon_receipt_hash(receipt: &Value) -> [u8; 32] {
        let receipt =
            serde_json::from_value::<easynet_axon::invocation::ReceiptJson>(receipt.clone())
                .unwrap();
        let (body, _, _) = receipt.to_body().unwrap();
        let (signer_binding, host_attestation) = receipt.hosted_attestation().unwrap();
        let hosted = easynet_axon::invocation::ReceiptJson::hosted_attestation_ref(
            &signer_binding,
            &host_attestation,
        );
        let digest = Sha256::digest(canonical_receipt_bytes_with_hosted(&body, hosted));
        digest.as_slice().try_into().unwrap()
    }

    fn signed_axon_receipt_value(
        signing_key: &ed25519_dalek::SigningKey,
        index: u64,
        prev_receipt_hash: [u8; 32],
        receipt_type: &str,
        state: &str,
    ) -> Value {
        use easynet_axon::invocation::axiom::{AuthorityBinding, InvocationUsage};
        use easynet_axon::invocation::bundle::{
            AuthorityJson, CausalJson, IdentityJson, InvocationAuthorityProofJson, ReceiptJson,
        };
        use easynet_axon::invocation::{
            sign_receipt, AgentIdentity, CausalContext, EntityRef, EntityRefKind,
            InvocationAuthorityProof, ReceiptBody, ReceiptProofFacts, SubjectIdentity, UraProfile,
        };

        let profile = UraProfile::EasynetStrictV2;
        let caller = AgentIdentity::new("easynet:///r/example/agent/alice.sdk", profile);
        let callee = AgentIdentity::new("easynet:///r/example/agent/alice.worker", profile);
        let subject = SubjectIdentity::from_callee(&callee);
        let authority = AuthorityBinding::Self_ {
            principal_ura: caller.ura.clone(),
        };
        let authority_proof = InvocationAuthorityProof {
            proof_type: "self".to_string(),
            binding: Some(authority.clone()),
            proof_payload: Vec::new(),
            proof_hash: [0u8; 32],
            issuer: None,
            signature: None,
            admission_hook: "sdk-test".to_string(),
        };
        let proof_facts = ReceiptProofFacts {
            subject_ref: Some(EntityRef::new(
                EntityRefKind::Agent,
                callee.ura.clone(),
                profile,
            )),
            descriptor_version: "1.0.0".to_string(),
            schema_hash: [1u8; 32],
            impl_hash: [2u8; 32],
            runtime_env: "sdk-test".to_string(),
            authority_proof,
            input_hash: [3u8; 32],
            output_hash: [4u8; 32],
            parent_receipts: Vec::new(),
        };
        let body = ReceiptBody {
            index,
            invocation_id: "inv-axon".to_string(),
            receipt_type: receipt_type.to_string(),
            state: state.to_string(),
            timestamp_unix_ms: 1783100000123,
            prev_receipt_hash,
            payload_digest: [5u8; 32],
            reason: String::new(),
            cleanup_complete: true,
            caller_binding: caller.clone(),
            callee_binding: callee.clone(),
            subject_binding: subject.clone(),
            invocation_nonce: [9u8; 16],
            causal_binding: CausalContext::None,
            ability_binding: "easynet:///r/example/ability/alice.worker.echo@1.0.0".to_string(),
            authority_binding: authority.clone(),
            usage: InvocationUsage::default(),
            proof_facts,
        };
        let signature = sign_receipt(&signing_key, &body, "test-key");
        let receipt = ReceiptJson {
            index: body.index,
            invocation_id: body.invocation_id,
            receipt_type: body.receipt_type,
            state: body.state,
            timestamp_unix_ms: body.timestamp_unix_ms,
            prev_receipt_hash_hex: hex::encode(body.prev_receipt_hash),
            payload_sha256_hex: hex::encode(body.payload_digest),
            reason: body.reason,
            cleanup_complete: body.cleanup_complete,
            caller_binding: IdentityJson::from_agent(&body.caller_binding),
            callee_binding: IdentityJson::from_agent(&body.callee_binding),
            subject_binding: IdentityJson::from_subject(&body.subject_binding),
            invocation_nonce_hex: hex::encode(body.invocation_nonce),
            causal_binding: CausalJson::from_ctx(&body.causal_binding),
            ability_binding: body.ability_binding,
            authority_binding: AuthorityJson::from_binding(&body.authority_binding),
            callee_signature_hex: hex::encode(signature.signature),
            callee_signature_alg: signature.algorithm,
            signer_binding: None,
            host_attestation_hex: None,
            usage_tokens_in: body.usage.tokens_in,
            usage_tokens_out: body.usage.tokens_out,
            usage_duration_ms: body.usage.duration_ms,
            usage_external_calls: body.usage.external_calls,
            subject_ref: body
                .proof_facts
                .subject_ref
                .as_ref()
                .map(easynet_axon::invocation::bundle::EntityRefJson::from_entity_ref),
            descriptor_version: body.proof_facts.descriptor_version,
            schema_hash_hex: hex::encode(body.proof_facts.schema_hash),
            impl_hash_hex: hex::encode(body.proof_facts.impl_hash),
            runtime_env: body.proof_facts.runtime_env,
            authority_proof: InvocationAuthorityProofJson::from_proof(
                &body.proof_facts.authority_proof,
            ),
            input_hash_hex: hex::encode(body.proof_facts.input_hash),
            output_hash_hex: hex::encode(body.proof_facts.output_hash),
            parent_receipts: Vec::new(),
        };

        serde_json::to_value(receipt).unwrap()
    }
}
