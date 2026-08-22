# Decisions and Evidence

## Decision

Add a focused `JoinConnectionSnapshot` hydration step in
`join_connection_state::latest_snapshot()`. This keeps the state-machine module
as the owner of connection-state projection semantics and avoids teaching the
RemoteApp product-flow harness how to read credentials directly.

## Required evidence

- Unit test for same-device snapshot hydration from credentials.
- Unit test proving different-device snapshots are not hydrated.
- `cargo test` for the `join_connection_state` tests.
- Hub API readiness preflight `--run` should now reach the configured Hub API
  endpoint when credentials provide it, and fail on actual reachability rather
  than missing endpoint context if the Hub API is down.
- Failed Hub API health probes should write a standard failed preflight report,
  preserving the health URL and connection-refused/error detail.
- Existing RemoteApp frontend product-flow and closure checkers must stay green.

## Current live evidence

- `target/e2e/hub-api-readiness/20260823-hydrated-health-report-21626/report.md`
- `target/e2e/frontend-remoteapp-product-flow/20260823-hydrated-health-report-21627/report.md`

The runtime-status report now exposes `hub_api_endpoint=http://localhost:8080`.
The Hub API readiness preflight reaches the canonical health URL and fails on
the actual environment state: `http://localhost:8080/api/v1/health` returns
connection refused while Docker is reachable. This is stronger evidence than
the previous missing-endpoint failure, but RemoteApp product-flow still stops
before frontend, host capture, media, or input evidence.
