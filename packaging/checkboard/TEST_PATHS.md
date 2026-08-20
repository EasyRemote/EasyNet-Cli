# E2E Test Path Descriptions

This file explains what each E2E path is intended to prove and where its logs
land when run through `packaging/checkboard/run-checkboard.sh`.

## EasyNet-Cli Rust E2E Tests

All Rust E2E tests run through `cargo test --test <name> -- --nocapture`.

| Path | Purpose |
|---|---|
| `tests/resolve_before_invoke_e2e.rs` | Verifies that ability resolution happens before invocation and that descriptor admission metadata is present before dispatch. |
| `tests/hub_ura_tls_join_cli_e2e.rs` | Exercises CLI-only Hub URA join over TLS without relying on the browser pairing path. |
| `tests/principal_lifecycle_cli_e2e.rs` | Covers principal lifecycle enrollment and CLI join flows. |
| `tests/principal_lifecycle_daemon_e2e.rs` | Covers daemon-side principal lifecycle operations and state transitions. |
| `tests/cross_realm_user_binding_e2e.rs` | Verifies cross-realm user/device binding behavior and ownership checks. |
| `tests/cross_realm_signed_admission_e2e.rs` | Verifies signed admission across realms and rejects invalid authority proofs. |
| `tests/cross_realm_directory_poll_e2e.rs` | Exercises cross-realm directory polling and local projection convergence. |
| `tests/cross_realm_directory_streaming_e2e.rs` | Exercises directory streaming behavior across realms. |
| `tests/mcp_hot_reload_e2e.rs` | Verifies MCP bridge hot reload and dynamic tool reflection. |
| `tests/mcp_bench_round1_e2e.rs` | Measures MCP bridge round-one latency and load behavior. |
| `tests/seven_axes_w1_discover_e2e.rs` | Covers seven-axes discovery behavior for ability ranking. |
| `tests/seven_axes_w2_watch_e2e.rs` | Covers seven-axes watch/subscription behavior. |
| `tests/seven_axes_w3_teach_learn_e2e.rs` | Covers teach/learn descriptor transfer behavior. |
| `tests/seven_axes_w3_usage_e2e.rs` | Covers usage accounting and related seven-axes behavior. |

## EasyNet-Cli Shell E2E Scripts

| Path | Purpose |
|---|---|
| `tools/scripts/frontend-daemon-cli-e2e.sh` | Replays frontend-visible daemon-bound CLI queries, including ability, skill, and agent listing latency under concurrency. |
| `tools/scripts/docker-three-node-cli-real-user-e2e.sh` | Starts one Hub and two devices in Docker, joins both devices to one user through CLI, creates custom agent/ability/skill state, and records frontend-equivalent auth queries. |
| `tools/scripts/docker-three-node-cli-real-user-e2e.sh --requests 48 --concurrency 16` | Repeats the one-Hub/two-device flow with higher-concurrency frontend-equivalent auth load so latency and projection behavior are captured under pressure. |
| `tools/scripts/docker-three-node-cli-real-user-e2e.sh --strict-frontend-projection` | Strict architecture probe. It fails when custom local daemon state does not project to frontend-equivalent auth devices, auth agents, and auth abilities. |
| `tools/scripts/backend-live-http-daemon-e2e.sh` | Exercises backend HTTP paths against a live daemon. |
| `tools/scripts/backend-live-principal-e2e.sh` | Exercises backend principal lifecycle paths against a live setup. |
| `tools/scripts/runtime-events-live-daemon-e2e.sh` | Verifies runtime event capture from a live daemon. |
| `tools/scripts/runtime-events-cross-repo-e2e.sh` | Verifies runtime events across EasyNet-Cli and sibling repositories. |
| `tools/scripts/standalone-hub-principal-lifecycle-e2e.sh` | Runs standalone Hub principal lifecycle checks. |
| `tools/scripts/standalone-hub-recovery-e2e.sh` | Verifies standalone Hub recovery behavior. |
| `tools/scripts/go-sdk-live-smoke.sh` | Runs Go SDK live smoke checks against a running runtime. |
| `tools/scripts/python-sdk-live-smoke.sh` | Runs Python SDK live smoke checks against a running runtime. |

## Packaging Release E2E

| Path | Purpose |
|---|---|
| `packaging/release/e2e-release-flow.sh` | Exercises release packaging flow end to end. |
| `packaging/release/e2e-release-install.sh` | Verifies installation behavior from the release artifact. |

## Sibling EasyNet Docker Harnesses

These paths live in `../EasyNet` and are marked `external` in the manifest.

| Path | Purpose |
|---|---|
| `../EasyNet/scripts/docker-e2e-join-invoke.sh` | Brings up Hub and two devices, joins devices, and verifies a cross-device invoke smoke. |
| `../EasyNet/scripts/docker-e2e-default-abilities.sh` | Exercises default device abilities across the Docker two-device topology. |
| `../EasyNet/scripts/docker-e2e-cli-survey.sh` | Broad CLI command and ability survey against the Docker topology. |
| `../EasyNet/scripts/docker-e2e-latency.sh` | Measures latency-sensitive paths in Docker. |
| `../EasyNet/scripts/docker-e2e-lease-renewal.sh` | Verifies device lease renewal behavior. |
| `../EasyNet/scripts/docker-e2e-cross-hub.sh` | Exercises cross-Hub routing and descriptor projection behavior. |
| `../EasyNet/scripts/docker-e2e-deep.sh` | Deep cross-surface Docker integration harness. |
| `../EasyNet/scripts/docker-e2e-public-routes.sh` | Exercises public route behavior through Docker. |
| `../EasyNet/scripts/cli-e2e-full.sh` | Full CLI E2E harness in the EasyNet repository. |
| `../EasyNet/scripts/dev-host-e2e.sh` | Host-side development E2E for local backend and CLI pairing. |

## Current Three-Node Finding

The current `docker-three-node-cli-real-user-e2e.sh` run proved that
daemon-bound custom agent, ability, and skill creation works locally. It also
captured a Hub/backend projection gap: `easynet auth devices --json` sees both
devices but keeps them in `UNKNOWN`, while `auth abilities` and `auth agents`
return empty lists. The daemon logs point at Hub trust bootstrap:

```text
federation.resolve_key: agent_ura `easynet:///r/easynet.run/user/...` not in this hub's trust set
```

The checkboard keeps this as a real test signal rather than hiding it behind a
fallback.

The strict projection row intentionally turns this signal into a failing
architecture gate until the Hub/frontend projection path is converged with the
local daemon source of truth.
