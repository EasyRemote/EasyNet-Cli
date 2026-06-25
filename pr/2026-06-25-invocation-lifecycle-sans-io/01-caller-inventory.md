# S-1 — Caller Inventory（删除安全依据）

对照磁盘亲核（分支 `seven-axes-p0-landing-v1`，2026-06-25）。本表是 S4/S5 删除四个 dispatcher
的安全依据：每个对外表面有谁消费、S4 切点在哪、删除后无悬空引用。

## 待删 dispatcher 的对外表面与消费方

| dispatcher | 对外类型 | 工厂方法 | 被谁消费 |
|---|---|---|---|
| `unary_dispatcher.rs` (2282) | `UnaryDispatcher` `pub(crate)` :186；`is_runtime_admin_ability` | `daemon_invocation_service.rs:386 unary_dispatcher()` | `daemon_invocation_service.rs`（arm `invoke`）+ `bidi_dispatcher.rs`（内部复用 unary） |
| `bidi_dispatcher.rs` (3369) | `BidiDispatcher`/`BidiDispatcherDeps` `pub(crate)` :93/103；`validate_and_extract_bidi_frame0` | `daemon_invocation_service.rs:400 bidi_dispatcher()` | `daemon_invocation_service.rs`（arm `invoke_bidi`）+ `ledger_projection.rs` |
| `stream_dispatcher.rs` (472) | `StreamDispatcher` `pub(crate)` :45 | `daemon_invocation_service.rs:376 stream_dispatcher()` | `daemon_invocation_service.rs`（arm `invoke_stream`） |
| `local_session_dispatcher.rs` (3704) | `LocalAxonSessionDispatcher` `pub` :55（+ `with_*` 装配器 :640/731/744/761） | `LocalAxonSessionDispatcher::new()` :626 | `boot.rs`（装配） |

## tonic service arm 切点（`daemon_invocation_service.rs`）

| arm | fn | 行 | 现调 | S4 步 |
|---|---|---|---|---|
| unary | `invoke` | 720 → unary@758 | `self.unary_dispatcher()` | S4-unary |
| stream | `invoke_stream` | 853 → stream@861 | `self.stream_dispatcher()` | S4-stream |
| bidi | `invoke_bidi` | 882 → bidi@915 | `self.bidi_dispatcher()` | S4-bidi |

工厂方法定义：`stream_dispatcher()`:376 / `unary_dispatcher()`:386 / `bidi_dispatcher()`:400。

## 依赖顺序约束（决定 S4 子步顺序）

- **bidi 依赖 unary**：`daemon_invocation_service.rs:408` `unary: self.unary_dispatcher()`，
  `BidiDispatcherDeps` 内嵌 unary。⇒ **S4-unary 必须先于 S4-bidi**（RFC-008 落地序已正确）。
- `local_session` 经 `boot.rs` 独立装配，与 3 个 tonic arm 解耦 ⇒ S4-session 可最后切。

## 切换策略（每个 S4 子步）

1. 新增 `transport_impls/<geometry>.rs`（实现 Transport，Proto↔TransportFrame 转换）。
2. 把对应工厂方法 + arm 改为 `lifecycle_driver` + 该 impl。
3. **同 commit** 删除旧 dispatcher 文件 + `daemon_invocation_service.rs` 里对它的 `use` 与工厂方法。
4. 六命令验证；对应 `daemon_invocation_service_tests/<geometry>.rs` 全绿。

## 删除后需同步清理的悬空引用（S5）

| 引用源 | 处置 |
|---|---|
| `daemon_invocation_service.rs:127-153` 的 3 个 `use ..._dispatcher::*` | 随各 S4 子步删除对应行 |
| `ledger_projection.rs`（消费 bidi） | S4-bidi 时改走 `ProjectReceipt`；`build_unary_ledger_record`/`ledger_record_from_remote_receipt` 删除（被单一 ProjectReceipt 取代） |
| `bidi_dispatcher.rs` 内部 unary 复用 | S4-bidi 时 driver 直接用 unary impl 的等价 Action 路径 |

## 不动清单（删除面之外）

- `federation_wrappers.rs`：`handle_join`:266 / `handle_advertise_agent`:356 / `handle_advertise_abilities`:399 /
  `handle_resolve`:521 / `handle_resolve_at`:542 / `handle_resolve_key`:776 —— 纯 handler library，
  作为 ability handler 被 driver 消费，不并入核心、不 dedup。
- `route_resolver.rs` 的 `DaemonRouteResolver` live lookup —— 留 CLI，执行 `ResolveRoute` action。
- `PresenceRegistry` / session up-down sender / pending maps / plugin·ability registration —— 留 CLI。
