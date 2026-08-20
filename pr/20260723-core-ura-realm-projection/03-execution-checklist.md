# Execution Checklist

- [x] Add generic and user-specific realm projection helpers to `core::ura`.
- [x] Migrate keyring token issuance and federated user resolver callers.
- [x] Remove duplicated keyring user-URA parser functions.
- [x] Remove admission/register-device generic realm parser shims.
- [x] Remove product probe realm fallback for malformed resolved device URAs.
- [x] Add gate coverage preventing keyring parser duplication and admission parser shims from returning.
- [x] Run targeted tests, SPEC gates, architecture gate, and codegraph checks.
