//! Shared CLI validation for public invocation tuple fields.

use anyhow::{bail, Context};

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
}
