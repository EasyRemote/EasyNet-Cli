# Architecture

## Boundary

`src/cli/commands/pairing_contract.rs` owns the EasyNet backend pairing REST
response shape. It is not part of the canonical runtime SDK and must not
introduce generic runtime concepts.

## Layering

- Product REST DTO: `PairingCredentialEnvelope`.
- Join domain validation: `validate_pairing_response`.
- Persistence projection: `credentials_from_pairing_contract`.
- Runtime SDK: unaffected.

The DTO rejects unknown carriers before domain validation. Domain validation
then checks semantic completeness such as non-empty paired user facts.
