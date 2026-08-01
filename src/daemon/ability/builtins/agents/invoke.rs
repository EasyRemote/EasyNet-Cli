//! File: `src/daemon/ability/builtins/agents/invoke.rs`
//! Description: Strict business-argument contract for agent-owned invoke input.
//!
//! Protocol responsibility: keep the agent invoke argument shape restricted to
//! business data only. Invocation metadata, caller identity, timeout,
//! idempotency, and causal facts are canonical runtime-envelope facts and must
//! not enter this parser through sidecar fields.
//!
//! Implementation approach: parse exactly `{ability_ura, args}` and fail closed
//! on every other key, including underscore-prefixed fields that older adapter
//! shapes used as sidecar metadata.
//!
//! Architectural position: daemon ability boundary. This module does not
//! register a separate dispatch route; discovered tools are invoked through the
//! descriptor-bound runtime path that issues signed child Invocations.

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
struct InvokeArgs {
    ability_ura: String,
    args: Value,
}

impl InvokeArgs {
    fn parse(raw: &Value) -> anyhow::Result<Self> {
        let obj = raw
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("invalid_args: invoke args must be a JSON object"))?;

        let raw_ability_ura = obj
            .get("ability_ura")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("invalid_args: `ability_ura` (string) is required"))?
            .to_string();
        if raw_ability_ura.is_empty() {
            anyhow::bail!("invalid_args: `ability_ura` must not be empty");
        }
        crate::core::ura::AbilitySelector::parse(&raw_ability_ura)
            .map_err(|error| anyhow::anyhow!("invalid_args: {error}"))?;

        let args = match obj.get("args") {
            None | Some(Value::Null) => json!({}),
            Some(value) if value.is_object() => value.clone(),
            Some(other) => {
                anyhow::bail!("invalid_args: `args` must be a JSON object; got {other}")
            }
        };

        // There is no underscore sidecar exception here. Request identity,
        // caller identity, timeout, idempotency, and causal facts are separate:
        // metadata are canonical invocation-envelope facts owned by the
        // SDK/runtime boundary, not hidden fields inside this business parser.
        const KNOWN: &[&str] = &["ability_ura", "args"];
        for key in obj.keys() {
            if KNOWN.contains(&key.as_str()) {
                continue;
            }
            anyhow::bail!("invalid_args: unknown field {key:?}; known: {:?}", KNOWN);
        }

        Ok(Self {
            ability_ura: raw_ability_ura,
            args,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_ability_ura() -> String {
        crate::core::ura::owner_ability_ura("easynet:///r/test/agent/local.codex", "fs.read")
            .expect("test ability URA")
    }

    #[test]
    fn parse_accepts_canonical_business_args() {
        let parsed = InvokeArgs::parse(&json!({
            "ability_ura": valid_ability_ura(),
            "args": {"path": "/tmp/a.txt"}
        }))
        .expect("canonical args");

        assert_eq!(parsed.args["path"], "/tmp/a.txt");
    }

    #[test]
    fn parse_rejects_underscore_prefixed_sidecar_fields() {
        let error = InvokeArgs::parse(&json!({
            "ability_ura": valid_ability_ura(),
            "_request_id": "req-1"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn parse_rejects_unread_underscore_fields() {
        let error = InvokeArgs::parse(&json!({
            "ability_ura": valid_ability_ura(),
            "_ignored": "value"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("_ignored"));
    }

    #[test]
    fn parse_rejects_non_string_sidecar_values() {
        let error = InvokeArgs::parse(&json!({
            "ability_ura": valid_ability_ura(),
            "_caller_ura": 42
        }))
        .unwrap_err();

        assert!(error.to_string().contains("_caller_ura"));
    }
}
