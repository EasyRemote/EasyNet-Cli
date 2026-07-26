# Discover callable fact cutover

## Goal

Remove the CLI discover read-model fallback that treats a missing `callable`
fact as an implicit non-callable candidate. Candidate rows must carry explicit
typed callability before ranking or rendering.

## Root abstraction problem

`DiscoverCandidateRow` parsed `callable` as `Option<bool>` and
`Candidate::from_ladder_row` repaired missing values with `false` plus a
diagnostic. That keeps an incomplete runtime catalogue/read-model row alive as a
displayable candidate. The product then sees "not callable" instead of the real
invariant failure: the candidate row lacked a required lifecycle fact.

## Invariants

1. Every discover candidate row must carry a boolean `callable` field.
2. Missing or non-boolean `callable` fails closed before ranking.
3. Unminted rows remain explicit non-callable rows only when they declare
   `callable: false`.
4. Minted rows still require a canonical `qualified_name`.
5. The public report shape remains unchanged for valid rows.

## Boundary proof

The runtime ladder owns candidate facts; the CLI projection only validates and
ranks those facts. Defaulting missing callability in the CLI reverses ownership
and creates a second lifecycle authority. Moving the invariant into the typed
row parser keeps discovery display logic pure and makes corrupt read-model state
observable as a deterministic failure.

## Verification plan

- Targeted `discover` candidate projection tests.
- SPEC v2 gate and self-test.
- Architecture convergence gate.
- `cargo fmt --check`.
- `git diff --check`.
