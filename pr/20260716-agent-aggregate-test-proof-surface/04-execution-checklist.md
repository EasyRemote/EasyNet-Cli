# Execution Checklist

- [x] Inspect current worktree and avoid unrelated dirty files.
- [x] Run CodeGraph-style search for `AgentAggregateSnapshot` and
  `HostedAgentIdentityProjection`.
- [x] Confirm warned helpers/fields are test-only consumers.
- [x] Inspect convergence gate expectations before editing.
- [x] Add `#[cfg(test)]` to proof-only snapshot helpers and projection fields.
- [x] Update architecture convergence rule to pin the production repository
  resolver instead of a proof-only snapshot helper.
- [x] Run focused Agent aggregate tests.
- [x] Run compile warning check for the targeted Agent aggregate warnings.
- [x] Run architecture convergence check and script self-test.
- [x] Record verification results.
