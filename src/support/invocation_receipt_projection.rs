// EasyNet CLI — invocation receipt-anchor projection
// ===================================================
//
// File: src/support/invocation_receipt_projection.rs
// Description: One owner for projecting an Axon ledger record's
//              `receipt_chain` summary into the compact terminal-receipt
//              anchor shape the CLI threads as a causal parent.
//
// Both the daemon-invoke metadata path (support::local_daemon_grpc) and the
// discover realm-anchor resolver (facade::cli::discover) consume this exact
// projection. Keeping it in one place means the "what is the head anchor of a
// completed invocation" contract — and the completeness predicate that gates
// realm-tier causal anchoring — cannot drift between the two readers.

use serde_json::{json, Value};

/// Project a ledger record's `receipt_chain` summary into the compact
/// `{head_receipt_hash, anchor, anchor_count}` shape.
///
/// The selected `anchor` is the chain head (the anchor whose `receipt_hash`
/// equals `head_receipt_hash`), falling back to the last anchor when the head
/// hash is absent. Returns `None` when the record carries no `receipt_chain`
/// (the sink has not projected it yet), which callers treat as "not anchored
/// yet", not an error.
pub(crate) fn terminal_receipt_from_ledger_record(ledger_record: &Value) -> Option<Value> {
    let chain = ledger_record.get("receipt_chain")?;
    let head_hash = chain
        .get("head_receipt_hash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let anchors = chain
        .get("anchors")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let head_anchor = anchors
        .iter()
        .find(|anchor| {
            anchor.get("receipt_hash").and_then(Value::as_str) == Some(head_hash.as_str())
                && !head_hash.is_empty()
        })
        .or_else(|| anchors.last())
        .cloned()
        .unwrap_or(Value::Null);
    Some(json!({
        "head_receipt_hash": head_hash,
        "anchor": head_anchor,
        "anchor_count": anchors.len(),
    }))
}

/// True when the projected receipt carries a complete anchor: both a non-empty
/// `receipt_ura` and a non-empty `receipt_hash`. This is the predicate that
/// distinguishes a usable causal parent from a receipt whose chain summary is
/// still incomplete.
pub(crate) fn terminal_receipt_has_complete_anchor(receipt: &Value) -> bool {
    let Some(anchor) = receipt.get("anchor") else {
        return false;
    };
    let has_ura = anchor
        .get("receipt_ura")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_hash = anchor
        .get("receipt_hash")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    has_ura && has_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_head_anchor_by_hash() {
        let record = json!({
            "receipt_chain": {
                "head_receipt_hash": "h2",
                "anchors": [
                    {"receipt_ura": "u1", "receipt_hash": "h1"},
                    {"receipt_ura": "u2", "receipt_hash": "h2"},
                ],
            }
        });
        let projected = terminal_receipt_from_ledger_record(&record).expect("projection");
        assert_eq!(projected["head_receipt_hash"], "h2");
        assert_eq!(projected["anchor"]["receipt_ura"], "u2");
        assert_eq!(projected["anchor_count"], 2);
        assert!(terminal_receipt_has_complete_anchor(&projected));
    }

    #[test]
    fn falls_back_to_last_anchor_without_head_hash() {
        let record = json!({
            "receipt_chain": {
                "anchors": [
                    {"receipt_ura": "u1", "receipt_hash": "h1"},
                    {"receipt_ura": "u2", "receipt_hash": "h2"},
                ],
            }
        });
        let projected = terminal_receipt_from_ledger_record(&record).expect("projection");
        assert_eq!(projected["anchor"]["receipt_ura"], "u2");
    }

    #[test]
    fn missing_receipt_chain_is_none() {
        assert!(terminal_receipt_from_ledger_record(&json!({})).is_none());
    }

    #[test]
    fn incomplete_anchor_is_rejected() {
        let projected = json!({"anchor": {"receipt_ura": "u1", "receipt_hash": ""}});
        assert!(!terminal_receipt_has_complete_anchor(&projected));
        let no_anchor = json!({"head_receipt_hash": "h1"});
        assert!(!terminal_receipt_has_complete_anchor(&no_anchor));
    }
}
