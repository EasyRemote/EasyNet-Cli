# JSON control caller inventory

Date: 2026-06-04; updated 2026-06-05 for CLI plugin Invocation migration.

Scope: EasyNet-Cli only. This inventory covers the length-prefixed JSON
control socket frame types `Invoke`, `Subscribe`, `OpenBidi`, `SendBidi`, and
`CloseBidi`. `Cancel` is not a target frame for this commit, but it is noted
where Subscribe compatibility depends on it.

The current JSON control schema is `IncomingFrame` in
`src/services/control/frames.rs`. The schema is serde-tagged on `type` with
snake_case names (`invoke`, `subscribe`, `open_bidi`, `send_bidi`,
`close_bidi`) and is still documented as the JSON mirror of the future proto
control-plane oneof (`src/services/control/frames.rs:12`,
`src/services/control/frames.rs:21`, `src/services/control/frames.rs:37`).

## Inventory method

Primary search:

```text
rg -n "IncomingFrame::(Invoke|Subscribe|OpenBidi|SendBidi|CloseBidi)" src tests -g '!target'
```

Control transport search:

```text
rg -n "serde_json::to_(vec|string)|serde_json::from_(slice|str)|LengthDelimitedCodec|Framed" \
  src/services/control src/ffi src/facade/cli/start_boot_watcher.rs src/facade/cli/groups/plugin.rs
```

Backend exclusion search:

```text
rg -n "IncomingFrame::(Invoke|Subscribe|OpenBidi|SendBidi|CloseBidi)" \
  src/services/invocation_transport src/runtime src/support src/facade -g '!target'
```

The backend exclusion search now returns only the facade boot watcher JSON-control
caller: `src/facade/cli/start_boot_watcher.rs:155`. No
`src/services/invocation_transport`, `src/runtime`, `src/support`, or CLI
plugin path constructs a target JSON control product-ability frame.

## Production caller inventory

| Category | Caller | Frames | Product ability caller | Evidence | Classification |
|---|---|---:|---|---|---|
| language binding | C ABI `easynet_ability_invoke` | `Invoke` | Yes. This is the exported product ability RPC entry for non-Rust bindings. | `src/ffi/mod.rs:5` says the C ABI is consumed by Go, Python, Node, Swift, and Java bindings; `src/ffi/ability.rs:5` names `easynet_ability_invoke`; the function constructs `IncomingFrame::Invoke` at `src/ffi/ability.rs:155`; it sends through `IpcClient::round_trip` at `src/ffi/ability.rs:189`; `round_trip` serializes the frame and decodes one `OutgoingFrame` at `src/ffi/client.rs:216`. | Product ability caller over legacy JSON control. |
| language binding | C ABI `easynet_ability_subscribe` | `Subscribe` | Yes. This is the exported product ability streaming entry for non-Rust bindings. | `src/ffi/ability.rs:5` names `easynet_ability_subscribe`; the reader task constructs `IncomingFrame::Subscribe` at `src/ffi/ability.rs:461`; it serializes and sends at `src/ffi/ability.rs:467`; the stream loop decodes `Frame`/`Terminal`/`Error` at `src/ffi/ability.rs:551`. | Product ability caller over legacy JSON control. |
| boot/status path | `easynet start` boot watcher | `Subscribe` | No. This is lifecycle/status only. | `src/facade/cli/start_boot_watcher.rs:5` describes waiting for daemon boot and translating boot events to UI; it constructs `IncomingFrame::Subscribe` at `src/facade/cli/start_boot_watcher.rs:155`; the ability is `WATCH_BOOT_ABILITY` (`system.watch_boot`) from `src/services/control/server.rs:64`; it sends at `src/facade/cli/start_boot_watcher.rs:161`. | Boot/status lifecycle caller. |

## Target frames with no production JSON caller

| Category | Frames | Product ability caller | Evidence | Classification |
|---|---:|---|---|---|
| language binding | `OpenBidi`, `SendBidi`, `CloseBidi` | No production caller in the current C ABI. | The only production FFI constructors are `Invoke` at `src/ffi/ability.rs:155` and `Subscribe` at `src/ffi/ability.rs:461`; `rg` finds no `IncomingFrame::OpenBidi`, `IncomingFrame::SendBidi`, or `IncomingFrame::CloseBidi` under `src/ffi`. | No language-binding JSON bidi caller exists today. |
| CLI command | `Invoke`, `OpenBidi`, `SendBidi`, `CloseBidi` | No production caller. | The only facade constructor is the boot `Subscribe` at `src/facade/cli/start_boot_watcher.rs:155`; plugin status/reload now uses daemon Invocation instead of JSON control. | No CLI JSON product-ability or bidi caller exists today. |
| backend | `Invoke`, `Subscribe`, `OpenBidi`, `SendBidi`, `CloseBidi` | No backend JSON-control caller in EasyNet-Cli. | Backend-like local ability calls route through Axon gRPC `daemon.sock`: `src/support/local_invoke.rs:5`, `src/support/local_invoke.rs:57`, and `src/support/local_daemon_grpc.rs:338`; the Axon server side is separately served on `daemon.sock`, distinct from `control.sock`, at `src/services/control/server.rs:23`. | Backend category is empty for JSON control callers; backend/product ability traffic is already on Axon Invocation gRPC where applicable. |

## Daemon internal receiver and compatibility paths

These are not callers. They are the daemon-side paths that still accept and
translate target JSON frames. They must remain while any caller above can still
reach `control.sock`.

| Category | Path | Frames | Evidence | Product or lifecycle scope |
|---|---|---:|---|---|
| daemon internal | `control.sock` accept loop and JSON codec | All target frames after decode | `src/services/control/server.rs:14` binds `~/.easynet/control.sock`; `src/services/control/server.rs:18` states each task uses `LengthDelimitedCodec`; `src/services/control/server.rs:23` says this is distinct from `daemon.sock`; `src/services/control/server.rs:280` deserializes bytes into `IncomingFrame`; `src/services/control/server.rs:295` hands the decoded frame to `handle_request`. | Compatibility ingress for both product FFI callers and lifecycle callers. |
| daemon internal | boot short-circuit for `system.watch_boot` | `Subscribe` | `src/services/control/server.rs:64` reserves `system.watch_boot`; `src/services/control/server.rs:330` detects `IncomingFrame::Subscribe`; `src/services/control/server.rs:336` matches `WATCH_BOOT_ABILITY`; `src/services/control/server.rs:388` spawns the boot forwarder. | Boot/status only. |
| daemon internal | booting-state rejection | `Invoke`, `Subscribe`, `OpenBidi`, `SendBidi`, `CloseBidi` | `src/services/control/server.rs:354` builds the booting error; `src/services/control/server.rs:357` handles `Invoke`; `src/services/control/server.rs:363` handles `Subscribe`; `src/services/control/server.rs:377` handles the bidi trio. | Compatibility diagnostics while daemon starts. |
| daemon internal | `AbilityProxy::handle_async` | All target frames | `src/services/control/ability_proxy.rs:25` states every `Invoke`/`Subscribe`/`OpenBidi` becomes an `InvocationPlan`; `src/services/control/ability_proxy.rs:259` handles `Invoke`; `src/services/control/ability_proxy.rs:340` handles `Subscribe`; `src/services/control/ability_proxy.rs:373` handles `OpenBidi`; `src/services/control/ability_proxy.rs:382` handles `SendBidi`; `src/services/control/ability_proxy.rs:448` handles `CloseBidi`. | Product ability dispatch for `Invoke`/`Subscribe`; compatibility receiver for bidi JSON frames. |
| daemon internal | bidi session registry and forwarder | `OpenBidi`, `SendBidi`, `CloseBidi` | The per-connection bidi registry is described at `src/services/control/ability_proxy.rs:75`; `OpenBidi` installs a row at `src/services/control/ability_proxy.rs:614`; `SendBidi` looks up the row and forwards JSON payload bytes at `src/services/control/ability_proxy.rs:394`; `CloseBidi` removes the row at `src/services/control/ability_proxy.rs:460`; the forwarder emits exactly one `TerminalBidi` at `src/services/control/ability_proxy.rs:1002`. | Compatibility receiver. No production JSON caller currently opens these sessions. |

## Test inventory

| Category | Test surface | Frames | Evidence | Purpose |
|---|---|---:|---|---|
| test | Wire-shape round trips | `Invoke`, `Subscribe`, `OpenBidi`, `SendBidi`, `CloseBidi` | `src/services/control/frames.rs:208` constructs `Invoke`; `src/services/control/frames.rs:231` constructs `Subscribe`; `src/services/control/frames.rs:307` constructs `OpenBidi`; `src/services/control/frames.rs:345` constructs `SendBidi`; `src/services/control/frames.rs:353` constructs `CloseBidi`. | Pins JSON discriminator and field names. |
| test | FFI client integration | `Invoke` | `src/ffi/client.rs:326` sends `IncomingFrame::Invoke` through `round_trip`. | Pins language-binding client codec path. |
| test | Server boot and wire tests | `Invoke`, `Subscribe`, `OpenBidi`, `SendBidi`, `CloseBidi` | Booting `Invoke`: `src/services/control/server.rs:571`; boot watch `Subscribe`: `src/services/control/server.rs:604`; E2E `Invoke`: `src/services/control/server.rs:678`; E2E bidi sequence: `src/services/control/server.rs:800`, `src/services/control/server.rs:811`, and `src/services/control/server.rs:820`; malformed recovery `Invoke`: `src/services/control/server.rs:928`. | Pins daemon JSON socket compatibility and recovery behavior. |
| test | Ability proxy unit tests | `Invoke`, `Subscribe`, `OpenBidi`, `SendBidi`, `CloseBidi` | `Invoke` constructors include `src/services/control/ability_proxy.rs:1251`, `src/services/control/ability_proxy.rs:1273`, `src/services/control/ability_proxy.rs:1387`, `src/services/control/ability_proxy.rs:1437`, and `src/services/control/ability_proxy.rs:2032`; `Subscribe` is at `src/services/control/ability_proxy.rs:1325`; bidi constructors include `src/services/control/ability_proxy.rs:1618`, `src/services/control/ability_proxy.rs:1633`, `src/services/control/ability_proxy.rs:1646`, `src/services/control/ability_proxy.rs:1769`, `src/services/control/ability_proxy.rs:1799`, `src/services/control/ability_proxy.rs:1851`, `src/services/control/ability_proxy.rs:1875`, `src/services/control/ability_proxy.rs:1927`, and `src/services/control/ability_proxy.rs:1962`. | Pins daemon internal translation and compatibility invariants. |
| test | FFI ABI validation | Indirect `Invoke`/`Subscribe` entry validation | `src/ffi/ability.rs:654` tests `easynet_ability_invoke` validation; `src/ffi/ability.rs:715` tests `easynet_ability_subscribe` callback validation; these validation tests do not reach the JSON wire because test sessions have no IPC client (`src/ffi/ability.rs:646`). | ABI safety tests, not wire callers. |

## Current product ability caller status

Product ability callers still on JSON control:

- `easynet_ability_invoke` for non-Rust client bindings.
- `easynet_ability_subscribe` for non-Rust client bindings.

Product ability callers already off JSON control:

- CLI ability invocation uses Axon Invocation gRPC on `daemon.sock`, not
  JSON control. `src/facade/cli/invoke.rs:43` states the rewrite routes through
  the daemon's Axon Invocation gRPC surface; `src/facade/cli/invoke.rs:189`
  calls `invoke_local_ability_with_subject`; `src/support/local_invoke.rs:57`
  declares this helper the canonical CLI entry; `src/support/local_daemon_grpc.rs:338`
  invokes daemon-hosted abilities through Axon gRPC.
- CLI plugin status/reload uses `DaemonClient::invoke(DaemonInvocation)` on
  `daemon.sock`; `src/facade/cli/groups/plugin.rs:223` builds the Invocation and
  `src/facade/cli/groups/plugin.rs:245` derives the subject from paired device
  credentials, falling back to a local loopback device URA when unpaired.

Lifecycle, diagnostics, or boot/status callers still on JSON control:

- `easynet start` boot progress watcher via `system.watch_boot`.

## Demotion checklist

Before demoting JSON control from product ability transport:

- [ ] Move `easynet_ability_invoke` internals from `IpcClient::round_trip` to
  Axon Invocation gRPC while preserving the exported C ABI signature,
  `EASYNET_ABI_VERSION` behavior, last-error semantics, and `OutgoingFrame`
  result/error mapping currently implemented at `src/ffi/ability.rs:202`.
- [ ] Move `easynet_ability_subscribe` internals from JSON `Subscribe` to Axon
  `InvokeStream` while preserving callback delivery, the dedicated dispatcher
  thread rationale at `src/ffi/ability.rs:470`, and idempotent cancellation
  semantics at `src/ffi/ability.rs:599`.
- [x] Move plugin lifecycle/status commands off `control.sock` and through the
  canonical CLI ability helper or another Axon Invocation path; preserve the
  daemon-down fallback behavior in `src/facade/cli/groups/plugin.rs:201`.
- [ ] Replace `system.watch_boot` JSON `Subscribe` with a boot/status transport
  that works before the full dispatcher is ready, or keep this JSON path as a
  boot-only compatibility endpoint. The current daemon intentionally accepts
  this subscription while `proxy` is still absent (`src/services/control/server.rs:330`).
- [ ] Decide whether any external binding needs bidi. The daemon receiver
  supports JSON `OpenBidi`/`SendBidi`/`CloseBidi`, but this inventory finds no
  production JSON caller for the bidi trio.
- [ ] Update `control.json` capability flags after caller migration. The v1
  discovery file advertises `ability_invoke` and `ability_subscribe` capability
  flags at `src/services/control/discovery.rs:123`; downrev clients may gate
  behavior on those strings.

Compatibility paths that cannot be deleted until the checklist is closed:

- `IncomingFrame` JSON serde shape in `src/services/control/frames.rs:37`, plus
  the wire-shape tests for all target frames.
- `control.sock` accept/decode path in `src/services/control/server.rs:235` and
  `src/services/control/server.rs:280`.
- `system.watch_boot` special-case in `src/services/control/server.rs:330`.
- Booting-state typed errors for all target frames in
  `src/services/control/server.rs:354`.
- `AbilityProxy::handle_async` target-frame translation in
  `src/services/control/ability_proxy.rs:251`.
- `IpcClient::connect` and `IpcClient::round_trip` while exported FFI callers
  still depend on them (`src/ffi/client.rs:113`, `src/ffi/client.rs:216`).
- Exported C ABI symbols in `src/ffi/mod.rs` and `src/ffi/ability.rs`; internal
  transport can change, but callers still link those symbols.
