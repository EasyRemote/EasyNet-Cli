# Hub Daemon Status Subject Boundary Plan

Date: 2026-07-27

## Goal

Fix the clean-HOME hub/daemon-only `easynet status` path that reports the daemon as running but then warns that `observe.health` cannot run because runtime-state read subject construction requires paired device credentials.

## Reproduction

With a fresh temporary HOME and a generated localhost TLS certificate:

```text
HOME=/tmp/easynet-clean-hub-home.* ./target/debug/easynet start --as-hub --cert <cert> --key <key>
HOME=/tmp/easynet-clean-hub-home.* ./target/debug/easynet status
```

Observed warning:

```text
Local daemon is not responding to observe.health despite runtime metadata:
runtime-state read subject unavailable: no credentials found — run `easynet join <token>` first
```

## Root Abstraction Problem

`LocalRuntimeStateReadIssuer` correctly enforces paired-user, signer-custody runtime-state subjects. That issuer must not grow a daemon/default fallback.

The defect is at the product status boundary: hub/daemon-only status is using the paired-device runtime-state read path for an operational daemon health probe. Those are different use cases:

1. paired device/product runtime-state reads require user-owned resource subjects and live paired-user signer custody;
2. daemon-only hub status should use boot/status discovery or daemon-owned operational health, not a user-paired runtime-state subject.

A second boundary defect appeared after the daemon-only status path stopped
issuing user runtime-state reads: local catalogue invocation for hub mode
failed with `subject_ref_kind_unsupported:Hub`. In the canonical URA model a realm
Authority/Hub subject is an accountable agent identity at the EntityRef layer.
Rejecting it as unsupported created a product fork where device daemon subjects
could use canonical invocation, but hub daemon subjects could not.

## Boundary Invariants

1. Do not weaken `LocalRuntimeStateReadIssuer`.
2. Do not introduce daemon/default subject fallback into runtime-state read issuer.
3. `easynet status` must not report a running hub daemon as unhealthy solely because device credentials are absent.
4. Paired-device status should keep the canonical runtime-state read path.
5. Hub/daemon-only status health must remain explicit and visible as an operational probe, not hidden as a canonical user runtime-state read.
6. Realm Authority/Hub URAs project to Axon's generic Agent EntityRef kind.
7. User URAs remain unsupported as direct EntityRef subjects; user-owned state reads must continue to use explicit Resource subjects.

## Implementation Direction

1. Identify the `status` path that invokes `LocalRuntimeStateReadIssuer`.
2. Route daemon-only/hub mode through control/runtime discovery health instead of paired runtime-state read.
3. Keep paired device mode unchanged.
4. Add focused tests/gates proving hub status does not require credentials and paired runtime-state issuer still fails closed without credentials.
5. Align CLI invocation-wire EntityRef subject projection with the Axon SDK subject-ref model for Authority/Hub URAs.
6. Add architecture gate coverage so Hub/Authority subjects cannot regress to `subject_ref_kind_unsupported:Hub`, while User subjects remain rejected.

## Completed Refactoring

- Introduced an explicit `StatusRuntimeReadPolicy` state split:
  paired status uses canonical user runtime-state reads; unpaired/invalid status
  uses daemon operational readiness and does not query user-scoped federation
  directory state.
- Updated CLI invocation-wire subject projection so `URAKind::Authority`
  maps to `EntityRefKind::Agent`.
- Updated Axon Rust SDK subject projection with the same Authority -> Agent
  rule, keeping CLI and SDK on the same canonical EntityRef matrix.
- Removed the accidental hub-only subject rejection path instead of adding a
  fallback signer or fake subject.

## Acceptance Checks

- `cargo fmt --check`
- `git diff --check`
- `cargo test -q --features axon-pb runtime_read_policy`
- `cargo test -q --features axon-pb hub_subject_ref_projects_to_agent_but_user_subject_is_rejected`
- `cargo test -q --features axon-pb invocation_wire_entity_ref_kind_resolution`
- `bash tools/scripts/check-invocation-wire-entity-ref-kind-resolution-boundary.sh`
- `bash tests/scripts/test_check_invocation_wire_entity_ref_kind_resolution_boundary.sh`
- `bash tools/scripts/check-runtime-state-read-subject-boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- Clean temporary HOME hub start succeeds.
- Clean temporary HOME hub `status` does not warn about missing credentials for
  `observe.health`.
- Clean temporary HOME hub `status` reports local ability catalogue count
  without `subject_ref_kind_unsupported:Hub`.
