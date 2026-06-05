# JSON Control Caller Inventory

**Document status:** Step 6 implementation evidence for demoting JSON control to boot/status only.
**Baseline:** current working tree on `codex/f07-hub-device-unify-2026-06-05`.
**Updated:** 2026-06-05 after daemon SDK, complete Invocation FFI, public control-schema shrink, and runtime-dispatch adapter cleanup.

## 1. Purpose

This document proves that EasyNet-Cli no longer uses the public JSON control socket as a product ability transport.

`control.sock` is now scoped to:

- daemon boot/status subscription via `system.watch_boot`;
- cancellation of retained boot/status subscriptions;
- protocol diagnostics for malformed frames and unknown control subscriptions.

Product calls use:

- daemon Invocation over `daemon.sock` for CLI, FFI, and backend-facing product paths;
- the separate runtime-dispatch socket only when Axon owns an incoming Invocation and delegates to a daemon-local registered tool.

## 2. Current public control schema

`src/services/control/frames.rs` now defines only:

- `IncomingFrame::Subscribe { subscription_id, ability, args }`
- `IncomingFrame::Cancel { subscription_id }`
- `OutgoingFrame::Frame { subscription_id, frame }`
- `OutgoingFrame::Terminal { subscription_id, reason }`
- `OutgoingFrame::Error { subscription_id, code, message }`

Removed from active public control schema:

- `Invoke`
- `OpenBidi`
- `SendBidi`
- `CloseBidi`
- `Result`
- `RecvBidi`
- `TerminalBidi`
- `ErrorBidi`

Retired raw discriminators such as `{"type":"invoke"}` are rejected by serde as `protocol` errors; they are not routed through a compatibility handler.

## 3. Production caller inventory

| Category | Caller | Transport | Product ability caller | Status |
|---|---|---|---|---|
| FFI unary | `easynet_invocation_invoke` in `src/ffi/invocation.rs` | daemon Invocation over `daemon.sock` | Yes | Migrated; requires complete Invocation JSON with `subject_ura`. |
| FFI stream | `easynet_invocation_stream_open/cancel` in `src/ffi/invocation.rs` | daemon `InvokeStream` over `daemon.sock` | Yes | Migrated; `scripts/ffi-smoke.sh` proves callback frame delivery through a real daemon. Cancellation drops local gRPC stream handle. |
| FFI bidi | `easynet_invocation_bidi_open/send/close/cancel` in `src/ffi/invocation.rs` | daemon `InvokeBidi` over `daemon.sock` | Yes | Migrated; `scripts/ffi-smoke.sh` proves frame-0 admission callback, typed send, business binary data, and terminal cleanup receipt through a real local-bidi ability. |
| FFI legacy ability+args | `easynet_ability_*` in `src/ffi/ability.rs` | None | No | Retired hard-fail stubs; no JSON control construction. |
| CLI plugin | `src/facade/cli/groups/plugin.rs` | `DaemonClient::invoke(DaemonInvocation)` | Yes | Migrated; supplies subject from paired device credentials or loopback CLI device URA. |
| CLI boot watcher | `src/facade/cli/start_boot_watcher.rs` | JSON control `Subscribe` | No | Retained lifecycle/status caller for `system.watch_boot`. |
| Backend | N/A in this repo | N/A | No | Confirmed absent from EasyNet-Cli. |

Production JSON-control product callers remaining: **0**.

## 4. Daemon internal surfaces

| Surface | Module | Responsibility | Product JSON control? |
|---|---|---|---|
| Public control socket | `src/services/control/server.rs` | Serve `system.watch_boot`, handle `Cancel`, return `not_found` for unknown control subscriptions, recover from malformed frames. | No. |
| Control frame schema | `src/services/control/frames.rs` | Boot/status-only frame model and codec tests. | No. |
| Runtime dispatch responder | `src/services/control/runtime_dispatch.rs` | Separate newline-delimited socket consumed by Axon runtime-local-tool dispatch. | No; separate internal protocol. |
| Runtime dispatch adapter | `src/services/control/runtime_dispatch_adapter.rs` | Convert runtime-dispatch requests into daemon-hosted `LocalRuntime` calls. | No; constructs no `IncomingFrame`/`OutgoingFrame`. |
| Daemon Invocation SDK | `src/daemon.rs` | Lifecycle, endpoint discovery, unary invoke, stream invoke. | Product transport over daemon Invocation. |

## 5. Test inventory

| Test surface | Current pin |
|---|---|
| `src/services/control/frames.rs` | `Subscribe`/`Cancel` round trips, `Frame`/`Terminal` output, retired raw `invoke` parse rejection. |
| `src/services/control/server.rs` | Boot watch delivery, cancellation, malformed-frame recovery, unknown control subscription `not_found`. |
| `src/services/control/runtime_dispatch.rs` | Runtime-dispatch request parsing, response shape, stream output, and socket handling. |
| `src/services/control/runtime_dispatch_adapter.rs` | Adapter dispatch to live local runtime and node hint parsing. |
| `src/ffi/client.rs` | Retained boot/status round trip against a real control server. |
| `scripts/check-daemon-invocation-migration.sh` | Mechanical guard that rejects restored product control frames, direct `DaemonInvocation` construction outside `src/daemon.rs`, and reintroduced CLI-owned `Invocation`/`canonical_bytes`/`invocation_id_of` semantics. |

Legacy product-frame compatibility tests have been removed from active control modules.

## 6. Demotion gate

- [x] G1 — Daemon SDK implemented: `src/daemon.rs`.
- [x] G2 — Complete Invocation FFI implemented for current external unary, stream, and bidi surfaces; bidi terminal E2E is proven through a real daemon local-bidi ability.
- [x] G3 — CLI plugin caller migrated to daemon Invocation.
- [x] G4 — Legacy unary ability+args FFI retired.
- [x] G5 — Legacy stream ability+args FFI retired; complete Invocation stream added.
- [x] G6 — Legacy JSON-control cancel not revived; stream cancel is local daemon stream cancellation.
- [x] G7 — FFI client harness exercises retained `system.watch_boot`, not product `Invoke`.
- [x] G8 — Receipt-chain completeness proof over all migrated product callers. Current product callers either parse complete Invocation JSON (`src/ffi/invocation.rs`) or use `DaemonInvocation::builder(caller, callee, ability, subject)` (`src/facade/cli/groups/plugin.rs`); direct `DaemonInvocation` construction is now mechanically rejected outside `src/daemon.rs`.
- [x] G9 — Product-caller sweep returns empty for JSON-control product frame constructors.
- [x] G10 — Boot/status control path preserved.
- [x] G11 — Compat tests reconciled with removed product frame variants.
- [x] G12 — Baseline re-confirmed against `main` (`6f463c6826dd3ff28d44db91263504d3fbc26023`). `main` still contains `src/services/control/ability_proxy.rs`, product control frame variants, and CLI-owned `Invocation`/`CausalContext`/`canonical_bytes`/`invocation_id_of`; the current tree removes those active paths and passes `scripts/check-daemon-invocation-migration.sh`.

## 7. Remaining work

Step 6 and Step 7 are complete in active EasyNet-Cli code. `RuntimeInvocation` is now a daemon-local adapter over Axon canonical bytes, not a second canonical Invocation model.

Cross-repo backend authority work is now closed end-to-end for the current
hub-mediated path: EasyNet/backend emits backend-signed
`x-easynet-session-authority` metadata for backend sessions and reserves
`x-easynet-delegation` for true user-signed delegation. This repository's
daemon admission consumer parses and verifies the appropriate authority
metadata before authorizing user-subject calls.
