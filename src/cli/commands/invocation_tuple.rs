//! Shared CLI validation and target parsing for public invocation tuple fields.

use anyhow::{bail, Context};

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

pub(crate) fn require_causal_root(value: bool, surface: &str) -> anyhow::Result<()> {
    if !value {
        bail!(
            "{surface} requires --causal-root for root calls; \
             public ingress must declare causal placement explicitly"
        );
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
