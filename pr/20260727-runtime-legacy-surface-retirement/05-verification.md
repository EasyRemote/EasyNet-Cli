# Verification

- `cargo fmt --check`
- `cargo build --bin easynet --bin easynet-daemon --bin easynet-keyring`
- `HOME=/tmp/easynet-clean-device-home.xO07f9 ./target/debug/easynet ability list --node easynet:///r/localhost/device/abae33c6-e4e3-40ac-8af0-e21b01e054b8 --format json`
  - verified 172 catalogue rows from the local runtime route.
  - verified rows retain canonical `descriptor_ref`.
- `cargo test --lib cli::daemon_client::remote_system_ability::tests::`
- `cargo test --lib ffi::invocation::tests::runtime_descriptor_resolver_prefers_local_catalog_for_runtime_owner`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 2

- `cargo fmt --check`
- `cargo test --lib support::platform::local_invoke::tests::runtime_device_read`
- `cargo test --lib cli::commands::groups::device::tests::`
- `cargo test --lib cli::commands::discover::tests::`
- Result: prototype rejected before commit because `tools/scripts/check-canonical-runtime-convergence-v2.sh` failed the runtime-state read subject boundary for `discover.rs`.
- Reverted production source changes from the prototype.

## Iteration 3

- `cargo build --bin easynet --bin easynet-daemon --bin easynet-keyring`
- `HOME=/tmp/easynet-clean-home.Zyfl0k ./target/debug/easynet start`
  - Result: clean HOME cannot start as a paired device without credentials; this confirmed the reported product path requires identity bootstrap and cannot be reproduced from empty state alone.
- `./target/debug/easynet runtime stop || true`
- `./target/debug/easynet leave --force --yes --purge-local-state || true`
  - Result: current user local EasyNet state root removed after explicit user authorization.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "invocation.history.list meta.list_abilities descriptor_ref caller signer authority subject mismatch"`
- `npm test --prefix sdk/node`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 4

- `/Users/macbook.silan.tech/.local/bin/codegraph query "URI terminology URA canonical runtime sdk receipt canonicalizer fail open governance subject"`
- `swift test` from `sdk/swift`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 5

- `/Users/macbook.silan.tech/.local/bin/codegraph query "Java receipt canonicalizer proof facts bypass fail open runtime governance subject parity"`
- `mvn test` from `sdk/java`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo fmt --check`
- `git diff --check`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 6

- `/Users/macbook.silan.tech/.local/bin/codegraph --version`
  - verified codegraph `1.4.1`.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "LocalRuntime catalogue descriptor provider remote meta.list_abilities invocation.history.list authority subject key service"`
- Clean hub/runtime verification:
  - stopped existing daemon if present.
  - moved prior runtime state aside after user authorization.
  - generated local CA:FALSE SAN cert for localhost hub testing.
  - `./target/debug/easynet runtime start --as-hub --tenant localhost --bind 0.0.0.0:50443 --cert /Users/macbook.silan.tech/.easynet/dev-certs/hub.cert.pem --key /Users/macbook.silan.tech/.easynet/dev-certs/hub.key.pem`
- Clean federation-native device verification:
  - `HOME=/tmp/easynet-device-clean-... ./target/debug/easynet device join easynet:///r/localhost/authority --hub-ca /Users/macbook.silan.tech/.easynet/dev-certs/hub.cert.pem --hub-port 50443 --peer-hub https://127.0.0.1:50443 --boot no --yes`
  - `HOME=/tmp/easynet-device-clean-... ./target/debug/easynet runtime start`
  - `HOME=/tmp/easynet-device-clean-... ./target/debug/easynet status`
  - `HOME=/tmp/easynet-device-clean-... ./target/debug/easynet ability list --format json`
    - verified exactly one `meta.list_abilities` descriptor for the clean device.
    - verified exactly one `invocation.history.list` descriptor for the clean device.
    - verified no `browser.open_session` ability is present in this daemon catalogue.
  - `HOME=/tmp/easynet-device-clean-... ./target/debug/easynet invocation list --format json`
    - verified local history read succeeds and receipt chains are `verified=true`.
  - `HOME=/tmp/easynet-device-clean-... ./target/debug/easynet device list --format json`
    - verified failure is now a CLI boundary state: unbound federation-native device requires a user-bound runtime identity or local Authority daemon.
- `cargo fmt --check`
- `cargo test cli::commands::devices::tests::`
- `cargo build --bin easynet`

## Iteration 7

- `/Users/macbook.silan.tech/.local/bin/codegraph query "legacy compatibility fallback compat deprecated SDK runtime provider receipt descriptor URA product neutral"`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "local rpc fallback must still be bound exact realm user descriptor"`
- `cargo test --features axon-pb local_target_projection --lib`
  - 2 passed.
- `cargo test --features axon-pb matches_self_target_ura --lib`
  - 3 passed.
- `cargo fmt --check`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 8

- `/Users/macbook.silan.tech/.local/bin/codegraph query "hosted agent placement projection filter_map malformed local-agents route resolver fail closed"`
- `cargo test --features axon-pb hosted_agent_placements --lib`
  - 5 passed, including route resolver hosted placement consume/unavailable regressions.
- `cargo test --features axon-pb local_hosted_agent_placements --lib`
  - 0 matched tests; retained as a no-op filter check after the route resolver placement tests matched under `hosted_agent_placements`.
- `cargo fmt --check`
- `git diff --check`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`

## Iteration 9

- `/Users/macbook.silan.tech/.local/bin/codegraph query "AgentAggregateSnapshot registered_agent_surface_names filter_map AgentId parse malformed registry fail closed"`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "registered Agent registry key projection fail closed no filter_map AgentAggregateSnapshot"`
- `cargo test --features axon-pb local_target_projection --lib`
  - 3 passed.
- `cargo test --features axon-pb registered_agent_names --lib`
  - 1 passed.
- `cargo test --features axon-pb agent_aggregate --lib`
  - 32 passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 3 changed code files.

## Iteration 10

- `/Users/macbook.silan.tech/.local/bin/codegraph query "owner_projections default empty owner projection admission receipt authority fail closed"`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "owner projection cursor host binding validate_owner_projection_host_binding heartbeat_refresh_owner_uras_from_file filter_map return None"`
- `cargo test --features axon-pb owner_projections --lib`
  - 8 passed.
- `cargo test --features axon-pb owner_projection --lib`
  - 45 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 2 changed code files.

## Iteration 11

- `/Users/macbook.silan.tech/.local/bin/codegraph query "PluginProvider provider_registry manifest_body builtin static provider lifecycle compatibility fallback language sidecar helper"`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "PluginProviderRegistry validate_builtin_entrypoint provider manifest entrypoint binding fail closed"`
- `cargo test --features axon-pb provider_registry --lib`
  - 4 passed.
- `cargo test --features axon-pb plugins::package --lib`
  - 9 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 1 changed code file.

## Iteration 15

- `target/debug/easynet device reset --purge-local-state --force --yes`
  - removed local EasyNet state root.
- `cargo build --bin easynet`
  - passed.
- `EASYNET_E2E_REQUESTS=2 EASYNET_E2E_CONCURRENCY=1 tools/scripts/cli-hub-device-daemon-e2e.sh --skip-build --keep --out-dir target/e2e/codex-clean-repro`
  - passed; kept isolated work root `/tmp/easynet-chd.RjQzVJ`.
- Remote public invoke checks from the isolated topology:
  - `meta.list_abilities`: passed.
  - `meta.list_resources`: passed.
  - `invocation.history.list` via public remote invoke: rejected before dispatch with canonical-history-read-path guidance.
  - local canonical invocation history read: passed and returned verified receipt chains.
- `npm test --prefix sdk/node -- --test-name-pattern "runtime receipt provider|runtime ability public path|public invocation builder rejects receipt history|session history preflight"`
  - 10 passed.
- `npm test --prefix sdk/node`
  - 71 passed.
- `node --check sdk/node/index.js`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `cargo fmt --check`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 3 changed files.

## Iteration 16

- `/Users/macbook.silan.tech/.local/bin/codegraph query "unknown fallback runtime state unwrap_or default compatibility receipt history descriptor provider legacy"`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "invocation state unknown fallback RuntimeStarted PROTOCOL_MISMATCH attempt audit"`
- `cargo test --features axon-pb invocation_attempt_audit_projects_invalid_runtime_state_as_protocol_mismatch --lib`
  - passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 2 changed files.

## Iteration 17

- `/Users/macbook.silan.tech/.local/bin/codegraph query "legacy compatibility fallback compat default product ingress invocation tuple subject nonce causal_context control invoke open_bidi subscribe"`
- `/Users/macbook.silan.tech/.local/bin/codegraph query "bidi presence default canonical owner online route negative terminal lifecycle"`
- `cargo test --features axon-pb presence --lib`
  - passed.
- `cargo test --features axon-pb session_contract --lib`
  - passed.
- `cargo test --features axon-pb claimant_fingerprint --lib`
  - passed.
- `cargo test --features axon-pb handle_revoke --lib`
  - passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 6 changed files.

## Iteration 18

- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - already up to date before edits.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "descriptor_ref not found owner is not online caller signer keyring entry not found invocation history meta list abilities"`
  - isolated descriptor/signer failure surfaces and FFI native signer fork.
- `cargo test --features axon-pb session_invocation_authority --lib`
  - passed; 5 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 2 changed files.

## Iteration 19

- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - already up to date before edits.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "legacy compatibility fallback compat default product ingress invocation tuple subject nonce causal_context control invoke open_bidi subscribe descriptor signer receipt"`
  - isolated local runtime stream chunk projection as the next duplicated
    terminality/receipt assembly path.
- `cargo test --features axon-pb local_runtime_stream_progress_projection_is_running_and_nonterminal --lib`
  - passed; 1 passed.
- `cargo test --features axon-pb daemon::invocation::streams::stream_dispatcher::tests --lib`
  - passed; 2 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 1 changed file.

## Iteration 20

- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - already up to date before edits.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "legacy compatibility fallback compat default product ingress invocation tuple subject nonce causal_context control invoke open_bidi subscribe descriptor signer receipt"`
  - isolated driver invocation trace metadata as a product observability surface
    still accepting legacy/noncanonical invocation address strings.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "mission invocation gateway subject default nonce causal_context child invocation"`
  - confirmed Mission child tuple derivation is Axon-owned; test-only
    `subject()` helper is not a production defaulting path.
- `cargo test --features axon-pb invocation_trace --lib`
  - passed; 4 passed.
- `cargo test --features axon-pb stream_tool_result_backfills_easynet_invocation_identity --lib`
  - passed; 1 passed.
- `cargo test --features axon-pb current_codex_function_call_and_mcp_result_capture_easynet_identity --lib`
  - passed; 1 passed.
- `cargo test --features axon-pb current_codex_easynet_function_output_preserves_result_without_mcp_end --lib`
  - passed; 1 passed.
- `cargo test --features axon-pb daemon::execution::mission::drivers --lib`
  - passed; 14 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 3 changed files.

## Iteration 21

- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - already up to date before edits.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "PluginTemplateProfile OR PROVIDER_SIDECAR_HELPER_CAPABILITY_MATRIX OR serve_exec_plugin OR sidecar helper"`
  - isolated plugin template/helper matrix as the next product-facing surface
    with a separable generation path.
- `cargo test --features axon-pb plugin_template --lib`
  - passed; 10 passed.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 1 changed file.

## Iteration 22

- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - already up to date at baseline.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed at baseline before edits.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "mac_base64 InvokeBidiUp BidiUp frame chain signature unwrap_or_default"`
  - isolated FFI and daemon client N≥1 up-frame empty MAC paths.
- `cargo test --features axon-pb parse_bidi_up_frame_json --lib`
  - passed; 2 passed.
- `cargo test --features axon-pb invocation_bidi_close_send --lib`
  - passed; 3 passed.
- `cargo test --features axon-pb invocation_bidi_send_eof_also_half_closes_local_send --lib`
  - passed; 1 passed.
- `cargo test --features axon-pb daemon::invocation::dispatch::client --lib`
  - passed; 4 passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed after adding the FFI frame-chain MAC gate.
- `cargo fmt --check`
  - passed.
- `git diff --check`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 2 changed code files.

## Iteration 23

- `target/debug/easynet device reset --purge-local-state --force --yes`
  - passed; removed the approved local `~/.easynet` state root so stale
    credentials, keyring, descriptor/read-model, registry, and daemon
    discovery files cannot influence verification.
- `cargo build --bins`
  - passed.
- `target/debug/easynet start`
  - failed closed with `no credentials — cannot start device agent` after the
    purge, proving device start does not silently fabricate or repair identity
    state.
- `tools/scripts/cli-hub-device-daemon-e2e.sh --skip-build --requests 3 --concurrency 2 --out-dir target/e2e/cli-hub-device-daemon/codex-clean-repro`
  - passed.
  - verified local hub/device principal enrollment, device join, daemon start,
    `meta.list_abilities`-backed ability discovery, ability/skill
    publish/invoke/delete, and concurrent query paths.
- `tools/scripts/docker-media-bidi-e2e.sh --out-dir target/e2e/docker-media-bidi/codex-clean-repro`
  - passed.
  - verified Docker hub/provider/caller stream and bidi topology, provider and
    caller descriptor refs, tuple preservation, two product operations mapping
    to two unique invocation receipt chains, single terminal head receipts,
    reverse bidi input, plugin removal, and route rejection after removal.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed at baseline before clean-state product verification.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed at baseline before product verification.

## Iteration 24

- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - already up to date at baseline.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "public explicit tuple bare ability local device callee fallback daemon invocation route ingress"`
  - identified public target defaulting as the next convergence surface.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "A2A legacy node roster query agents/list target_node_legacy schema"`
  - isolated `a2a.client.send_task` accepting bare node ids despite the public
    schema naming the field `target_node_ura`.
- `cargo test --features axon-pb daemon::ability::builtins::integrations::a2a::client --lib`
  - first run exposed retired `/node/N1` test fixtures in missing-field tests.
  - second run passed; 11 passed.
- `cargo fmt --check`
  - passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - passed after adding the A2A target URA ingress gate.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  - passed.
- `tools/scripts/check-architecture-convergence.sh`
  - passed.
- `git diff --check`
  - passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - passed; synced 1 changed code file.
