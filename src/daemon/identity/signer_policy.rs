// EasyNet CLI - provider identity signer policy binding
// ===================================================
//
// File: src/daemon/identity/signer_policy.rs
// Description: Canonical identity-key inventory policy reference derivation.

use sha2::{Digest, Sha256};

/// Bind a provider identity signer policy to its owner, key identity, and public
/// key material. The length-delimited input prevents ambiguous concatenation.
pub(crate) fn signer_policy_ref(owner_ura: &str, key_id: &str, public_key_base64: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(owner_ura.as_bytes());
    hasher.update(b"\0");
    hasher.update(key_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(public_key_base64.as_bytes());
    let digest = hasher.finalize();
    format!(
        "provider-key-inventory:sha256:{}",
        hex::encode(&digest[..16])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_ref_binds_every_identity_key_component() {
        let baseline = signer_policy_ref("owner", "key", "public-key");

        assert_ne!(baseline, signer_policy_ref("other", "key", "public-key"));
        assert_ne!(baseline, signer_policy_ref("owner", "other", "public-key"));
        assert_ne!(baseline, signer_policy_ref("owner", "key", "other"));
    }
}
