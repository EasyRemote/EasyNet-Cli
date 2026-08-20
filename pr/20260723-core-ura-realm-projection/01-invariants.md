# Invariants

- URA parsing remains Axon-owned through `axon_sdk::ura::parse_ura`.
- CLI modules may project parsed URA facts through `crate::core::ura`.
- Keyring/federation modules must not define their own user-URA realm parser.
- Malformed URAs and non-user URAs remain distinguishable at call sites.
- Public behavior remains fail-closed for malformed or role-mismatched URAs.
