# seven-axes Axon batch agenda (2026-06-13) -- SUPERSEDED

**Status:** superseded by the seven-axes v1.9/v2.0 decision recorded in
`docs/spec/seven-axes-p0-landing-v1.md` and by the control-plane model/status
documents.

This file is kept only as a dated decision record. Do not use its original
`policy.evaluate` or `trust-level` sections as implementation guidance.

## Superseding Decision

The seven-axes P0 product surface no longer carries standalone `policy` or
`trust-level` commands, abilities, stores, or e2e contracts.

The current P0 ownership is:

- `USE`: ability discovery and invocation.
- `ACCOUNT / ORGANIZE`: invocation trace, watch, and ledger projection.
- `PROTECT`: trust anchor and signed admission boundary.
- `ECONOMIC`: signed usage on receipts.
- `GET / TRANSFER`: owner-initiated ability teach/learn.

Access/permission work must land as a unified ability access/permission model.
It must not reintroduce an independent policy-rule product face or a separate
trust-level directory.

## What Changed Since This Agenda

1. Signed usage is no longer an agenda proposal. It is wired through Axon
   receipts and projected through the CLI ledger/watch path.
2. The policy matcher, policy rule store, `policy.evaluate`,
   `policy.simulate`, and related CLI/e2e surface were intentionally removed.
3. The trust-level store, `identity.get_trust/set_trust`, `trust level show/set`,
   and related e2e surface were intentionally removed.
4. Trust remains a trust-anchor/admission concept: whose signatures this daemon
   accepts. It does not grant ability permissions.
5. Descriptor/runtime proof facts now ride Axon receipt proof slots through the
   LocalRuntime proof-binding path. The remaining product work is query and
   audit visibility, not receipt-shape design.

## Active Follow-up Guidance

Future Axon/CLI follow-up work should use these boundaries:

- Keep usage as signed receipt output, not an eighth invocation parameter.
- Keep descriptor version, schema hash, and implementation hash in Axon's
  signed receipt proof facts.
- Expose proof facts through query/watch/audit surfaces only as projections of
  signed receipts.
- Define future permissions under ability access/permission. Do not route
  admission through a revived standalone `policy.evaluate` ability.
- Keep trust anchor semantics separate from authorization semantics.

## Historical Note

The original version of this agenda predated the P0 correction that removed the
standalone policy/trust-level surfaces. Its old sections referenced deleted
files such as `policy_ability.rs`, `policy_rules.rs`, `trust_ability.rs`, and
`trust_levels.rs`. Those references are now historical evidence only.
