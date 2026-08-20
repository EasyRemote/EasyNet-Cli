# Execution checklist

- [x] Identify pairing validate response as the next root boundary.
- [x] Make pairing response DTO deny unknown fields.
- [x] Remove `Default` from pairing DTOs that carry required facts.
- [x] Replace test `..Default::default()` construction with a complete fixture.
- [x] Add retired alias regression test at pairing response ingress.
- [x] Extend SPEC v2 gate.
- [ ] Run targeted join/pairing tests and gates.
