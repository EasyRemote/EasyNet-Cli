# Hosted Agent Prelude Generation Convergence

## Root Fork

`session_initiator/prelude.rs` published hosted-agent identity with a hard-coded
`generation: 1`, then published the owner's ability projection through
`owner_projection::prepare_and_persist`. The ability projection cursor already
allocates a monotonic generation when a retired owner URA is recreated, but the
identity advertisement did not consume that state. After same-URA recreation,
Hub identity and ability rows could therefore disagree about incarnation.

## CodeGraph Evidence

- `send_advertise_agent_prelude` serialized `generation: 1` for every
  `federation.advertise_agent`.
- `advertise_hosted_agent_abilities` called
  `owner_projection::prepare_and_persist`, which is the durable owner cursor and
  already advances generation after `OwnerProjectionCursorLifecycle::Retired`.
- `federation_wrappers::handle_revoke` removes rows by generation and preserves
  newer incarnations during stale revoke replay, so mismatched identity
  generation is protocol-significant rather than cosmetic.
- The hot-agent lifecycle path already uses the owner projection publication
  generation for `advertise_hosted_agent`; the session prelude was the duplicate
  publication path.

## Invariant

For a hosted Agent owner URA, identity advertisement and ability projection
publication in the same session prelude must use one durable owner cursor
generation. The generation source is `owner_projection::prepare_and_persist`.

## Design

Introduce a typed hosted-agent prelude publication plan:

- Select committed descriptors for the hosted owner.
- Persist the owner projection cursor before identity advertisement.
- Publish `federation.advertise_agent` with the plan generation.
- Publish `federation.advertise_abilities` from the already prepared
  publication.

This removes the duplicate generation source without changing public request or
response schemas.

## Verification Plan

- Focused tests for session prelude generation consistency.
- Focused owner projection test proving same-URA recreation increments
  generation.
- Architecture convergence script.
- `git diff --check`.

## Verification Results

- `cargo test --locked --lib hosted_agent_prelude_plan_uses_retired_owner_cursor_generation -- --nocapture`
- `cargo test --locked --lib session_prelude_publishes_hosted_llm_agent_ability_projection -- --nocapture`
- `cargo test --locked --lib journaled_removal_retains_exact_cursor_until_compare_and_retire -- --nocapture`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `rustfmt --check --edition 2021 src/daemon/invocation/bidi/session_initiator/prelude.rs src/daemon/invocation/bidi/session_initiator.rs`
- `git diff --check -- src/daemon/invocation/bidi/session_initiator/prelude.rs src/daemon/invocation/bidi/session_initiator.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-hosted-agent-prelude-generation/proof.md`
