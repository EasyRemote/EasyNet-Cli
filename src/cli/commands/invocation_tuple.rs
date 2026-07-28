//! Shared CLI validation and target parsing for public invocation tuple fields.

use anyhow::{bail, Context};
use serde_json::{Map, Value};

use crate::core::ura::AbilitySelector;

#[derive(Debug, Clone)]
pub(crate) struct AbilityInvocationRef {
    selector: AbilitySelector,
    descriptor_ref: Option<String>,
}

impl AbilityInvocationRef {
    pub(crate) fn parse(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim();
        if raw.contains('@') {
            let descriptor_ref = axon_sdk::invocation::canonical_ability_descriptor_ref(raw)
                .map_err(|err| anyhow::anyhow!("parse <ability-ura>@<version>: {err}"))?;
            let ability_ura =
                crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                    &descriptor_ref,
                )
                .map_err(|err| anyhow::anyhow!("parse ability URA inside descriptor ref: {err}"))?;
            let selector = AbilitySelector::parse(&ability_ura)
                .with_context(|| "parse ability URA inside descriptor ref")?;
            return Ok(Self {
                selector,
                descriptor_ref: Some(descriptor_ref),
            });
        }

        Ok(Self {
            selector: AbilitySelector::parse(raw).with_context(|| "parse <ability-ura>")?,
            descriptor_ref: None,
        })
    }

    pub(crate) fn selector(&self) -> &AbilitySelector {
        &self.selector
    }

    pub(crate) fn is_descriptor_ref(&self) -> bool {
        self.descriptor_ref.is_some()
    }

    pub(crate) fn descriptor_ref(&self) -> Option<&str> {
        self.descriptor_ref.as_deref()
    }

    #[cfg(feature = "axon-pb")]
    pub(crate) fn remote_target_for_mode(
        &self,
        execution_target_ura: &str,
        call_mode: crate::daemon::ability::CallMode,
    ) -> anyhow::Result<
        crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget,
    > {
        match self.descriptor_ref() {
            Some(descriptor_ref) => {
                crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget::from_descriptor_ref(
                    execution_target_ura,
                    descriptor_ref,
                )
            }
            None => {
                crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget::from_ability_ura_for_mode(
                    execution_target_ura,
                    self.selector.ability_ura(),
                    call_mode,
                )
            }
        }
    }
}

pub(crate) fn required_subject<'a>(
    value: Option<&'a str>,
    surface: &str,
) -> anyhow::Result<&'a str> {
    value
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{surface} requires --subject; public ingress must carry an explicit AXIOM subject"
            )
        })
}

pub(crate) fn required_nonce_hex(value: Option<&str>, surface: &str) -> anyhow::Result<[u8; 16]> {
    parse_invocation_nonce_hex(value.ok_or_else(|| {
        anyhow::anyhow!(
            "{surface} requires --nonce-hex; public ingress must carry an explicit AXIOM nonce"
        )
    })?)
}

pub(crate) fn required_causal_context(
    causal_root: bool,
    causal_context_json: Option<&str>,
    surface: &str,
) -> anyhow::Result<axon_sdk::invocation::CausalContext> {
    match (
        causal_root,
        causal_context_json
            .map(str::trim)
            .filter(|raw| !raw.is_empty()),
    ) {
        (true, None) => Ok(axon_sdk::invocation::CausalContext::None),
        (false, Some(raw)) => parse_causal_context_json(raw),
        (true, Some(_)) => bail!(
            "{surface} accepts exactly one causal placement selector; \
             use --causal-root for root calls or --causal-context-json for non-root calls"
        ),
        (false, None) => bail!(
            "{surface} requires --causal-root or --causal-context-json; \
             public ingress must declare causal placement explicitly"
        ),
    }
}

fn parse_causal_context_json(raw: &str) -> anyhow::Result<axon_sdk::invocation::CausalContext> {
    let value: Value = serde_json::from_str(raw).context("parse --causal-context-json")?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("--causal-context-json must be a JSON object"))?;
    let form = required_string_field(object, "form")?;
    match form {
        "none" => {
            bail!("use --causal-root instead of --causal-context-json '{{\"form\":\"none\"}}'")
        }
        "scalar" => {
            require_exact_keys(object, &["form", "receipt_hash_hex", "receipt_ura"])?;
            Ok(axon_sdk::invocation::CausalContext::Scalar(
                receipt_ref_from_object(object)?,
            ))
        }
        "list" => {
            require_exact_keys(object, &["form", "prior"])?;
            let prior = object
                .get("prior")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("--causal-context-json list requires non-empty prior")
                })?;
            let mut refs = Vec::with_capacity(prior.len());
            for (index, item) in prior.iter().enumerate() {
                let item = item.as_object().ok_or_else(|| {
                    anyhow::anyhow!("--causal-context-json prior[{index}] must be an object")
                })?;
                require_exact_keys(item, &["receipt_hash_hex", "receipt_ura"])
                    .with_context(|| format!("validate --causal-context-json prior[{index}]"))?;
                refs.push(receipt_ref_from_object(item)?);
            }
            Ok(axon_sdk::invocation::CausalContext::List(refs))
        }
        "merkle" => {
            require_exact_keys(object, &["form", "proof_ura", "root_hex"])?;
            Ok(axon_sdk::invocation::CausalContext::Merkle {
                root: decode_hash_hex(object, "root_hex")?,
                proof_ura: required_string_field(object, "proof_ura")?.to_string(),
            })
        }
        other => bail!("unsupported --causal-context-json form `{other}`"),
    }
}

fn receipt_ref_from_object(
    object: &Map<String, Value>,
) -> anyhow::Result<axon_sdk::invocation::ReceiptRef> {
    Ok(axon_sdk::invocation::ReceiptRef {
        receipt_hash: decode_hash_hex(object, "receipt_hash_hex")?,
        receipt_ura: required_string_field(object, "receipt_ura")?.to_string(),
    })
}

fn required_string_field<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> anyhow::Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("--causal-context-json field `{field}` must be a non-empty string")
        })
}

fn decode_hash_hex(object: &Map<String, Value>, field: &'static str) -> anyhow::Result<[u8; 32]> {
    let raw = required_string_field(object, field)?;
    let decoded = hex::decode(raw)
        .with_context(|| format!("parse --causal-context-json field `{field}` as 32-byte hex"))?;
    let len = decoded.len();
    decoded.as_slice().try_into().map_err(|_| {
        anyhow::anyhow!(
            "--causal-context-json field `{field}` must encode exactly 32 bytes / 64 hex characters, got {len} bytes"
        )
    })
}

fn require_exact_keys(object: &Map<String, Value>, allowed: &[&str]) -> anyhow::Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!("--causal-context-json contains unsupported field `{key}`");
        }
    }
    for key in allowed {
        if !object.contains_key(*key) {
            bail!("--causal-context-json missing required field `{key}`");
        }
    }
    Ok(())
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn remote_invocation_transport_unsupported(surface: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{surface} is unsupported in this build: canonical remote invocation transport is disabled; \
         rebuild with `--features axon-pb` and retry"
    )
}

pub(crate) fn parse_invocation_nonce_hex(raw: &str) -> anyhow::Result<[u8; 16]> {
    let decoded = hex::decode(raw.trim())
        .with_context(|| "parse --nonce-hex as 16-byte hex invocation nonce")?;
    let nonce: [u8; 16] = decoded.as_slice().try_into().map_err(|_| {
        anyhow::anyhow!(
            "--nonce-hex must encode exactly 16 bytes / 32 hex characters, got {} bytes",
            decoded.len()
        )
    })?;
    if nonce == [0; 16] {
        bail!("--nonce-hex must not be the all-zero nonce");
    }
    Ok(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_invocation_nonce_hex_requires_exact_nonzero_nonce() {
        assert_eq!(
            parse_invocation_nonce_hex("01010101010101010101010101010101").unwrap(),
            [1u8; 16]
        );

        let short = parse_invocation_nonce_hex("0102").expect_err("short nonce must fail");
        assert!(format!("{short}").contains("exactly 16 bytes"));

        let zero = parse_invocation_nonce_hex("00000000000000000000000000000000")
            .expect_err("zero nonce must fail");
        assert!(format!("{zero}").contains("all-zero"));
    }

    #[test]
    fn required_causal_context_requires_exactly_one_selector() {
        assert_eq!(
            required_causal_context(true, None, "test").unwrap(),
            axon_sdk::invocation::CausalContext::None
        );

        let neither = required_causal_context(false, None, "test")
            .expect_err("causal selector must be required");
        assert!(format!("{neither}").contains("--causal-root or --causal-context-json"));

        let both = required_causal_context(true, Some(r#"{"form":"scalar"}"#), "test")
            .expect_err("dual causal selectors must fail");
        assert!(format!("{both}").contains("exactly one causal placement selector"));

        let none_json = required_causal_context(false, Some(r#"{"form":"none"}"#), "test")
            .expect_err("root JSON alias must fail");
        assert!(format!("{none_json}").contains("use --causal-root"));
    }

    #[test]
    fn required_causal_context_parses_strict_scalar_list_and_merkle() {
        let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let scalar = required_causal_context(
            false,
            Some(&format!(
                r#"{{"form":"scalar","receipt_hash_hex":"{hash}","receipt_ura":"easynet:///r/acme/resource/receipt/1"}}"#
            )),
            "test",
        )
        .expect("scalar causal context");
        assert!(matches!(
            scalar,
            axon_sdk::invocation::CausalContext::Scalar(_)
        ));

        let list = required_causal_context(
            false,
            Some(&format!(
                r#"{{"form":"list","prior":[{{"receipt_hash_hex":"{hash}","receipt_ura":"easynet:///r/acme/resource/receipt/1"}}]}}"#
            )),
            "test",
        )
        .expect("list causal context");
        let axon_sdk::invocation::CausalContext::List(prior) = list else {
            panic!("expected list causal context");
        };
        assert_eq!(prior.len(), 1);

        let merkle = required_causal_context(
            false,
            Some(&format!(
                r#"{{"form":"merkle","root_hex":"{hash}","proof_ura":"easynet:///r/acme/resource/proof/1"}}"#
            )),
            "test",
        )
        .expect("merkle causal context");
        assert!(matches!(
            merkle,
            axon_sdk::invocation::CausalContext::Merkle { .. }
        ));
    }

    #[test]
    fn required_causal_context_rejects_legacy_aliases_and_unknown_fields() {
        let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        for raw in [
            r#"{"kind":"none"}"#.to_string(),
            r#"{"form":"vector","prior":[]}"#.to_string(),
            format!(
                r#"{{"form":"scalar","receipt_hash_hex":"{hash}","receipt_ura":"easynet:///r/acme/resource/receipt/1","receipt_hash":"legacy"}}"#
            ),
            r#"{"form":"list","prior":[]}"#.to_string(),
            r#"{"form":"merkle","root_hex":"aa","proof_ura":"easynet:///r/acme/resource/proof/1"}"#
                .to_string(),
        ] {
            let error = required_causal_context(false, Some(&raw), "test")
                .expect_err("legacy or malformed causal context must fail");
            assert!(
                format!("{error}").contains("--causal-context-json")
                    || format!("{error}").contains("unsupported"),
                "unexpected error for {raw}: {error}"
            );
        }
    }

    #[test]
    fn ability_invocation_ref_parses_plain_ability_ura() {
        let parsed =
            AbilityInvocationRef::parse("easynet:///r/acme/ability/device.node.observe.health")
                .expect("plain ability URA");

        assert_eq!(
            parsed.selector().ability_ura(),
            "easynet:///r/acme/ability/device.node.observe.health"
        );
        assert!(!parsed.is_descriptor_ref());
    }

    #[test]
    fn ability_invocation_ref_preserves_explicit_descriptor_ref() {
        let descriptor_ref =
            "easynet:///r/acme/ability/device.node.observe.health@2.1.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read";
        let parsed = AbilityInvocationRef::parse(descriptor_ref).expect("descriptor ref");

        assert_eq!(
            parsed.selector().ability_ura(),
            "easynet:///r/acme/ability/device.node.observe.health"
        );
        assert_eq!(parsed.descriptor_ref(), Some(descriptor_ref));
    }
}
