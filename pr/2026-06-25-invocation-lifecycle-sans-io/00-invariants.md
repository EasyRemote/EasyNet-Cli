# S-1 — Invariants（F-051 / AXON-RFC-008 v2）

实现期间任何 commit 不得违反的不变量。每条都对应 RFC-008 §5 的一条验收必测。

## I1. sans-IO 核心纯净
`InvocationLifecycle` 核心（`EasyNet-Axon/sdk/rust/src/invocation/lifecycle.rs`）依赖图中
**不得出现** `tokio` / `tonic` / `mpsc` / 任何 daemon store 类型。核心表面仅 `on_event(Event)->Vec<Action>`
+ `state()`。验证：`cargo test -p easynet-axon invocation::lifecycle` 不需 tokio runtime 即可跑。

## I2. 终态不可混淆
`TerminalKind` 的 `Completed` / `Failed{code}` / `TimedOut` / `Cancelled` 四变体互斥。
- `DeadlineExceeded` 事件 → `Settled(TimedOut)`，**绝不** `Settled(Failed)`。
- `CallerCancelled` 事件 → `Settled(Cancelled)`，**绝不** `Settled(Failed)`。

## I3. receipt 投影是 terminal 原子动作
`ProjectReceipt` 只在进入 `Settled(_)` 的同一次 `on_event` 内发出，序列为 `[ProjectReceipt, SendFrame(terminal)]`。
不存在长期 `Projecting` 状态。terminal frame 发送失败 **不回滚 receipt**（invocation 已终态，失败仅为 transport diagnostic）。

## I4. CLI 不铸造 canonical receipt
- local Axon runtime 执行的 call → receipt 由 Axon `LocalRuntime` receipt chain 产生。
- remote carrier-v1 callee-signed receipt → CLI 仅投本地 hub ledger，**不重签**。
- admission/route 失败（未进 callee execution）→ 产 runtime/admission 失败 receipt 或 ledger diagnostic，
  **不伪造 callee execution receipt**。

## I5. Axon 不解释 daemon 事实
Axon 的 `RouteQuery/ResolvedRoute/RouteProfile/DispatchTarget` 是纯类型。`DispatchTarget` 中
daemon 专属字段（local session / presence / plugin 路由）以 **opaque key** 承载，Axon core 不解释其含义。
`DaemonRouteResolver` 留 CLI。

## I6. busy ≠ offline
unary（及全 geometry）：`try_send` Full → `DispatchFailed{retryable:true, resource_exhausted}`（**不移除 presence**）；
Closed → offline。当前只在 `unary_dispatcher.rs:1524` 区落实，收敛后由**单处** transition 表达。

## I7. 删除即彻底
旧 dispatcher 删除不弃用：无 fallback 路径、无 compat 垫片、无弃用别名、无死代码残留。
`federation_wrappers.rs` **不在删除面**（CTO 确认：非 lifecycle 复制体，字节大小巧合 95394 但 MD5 异、零重叠 fn）。

## I8. 每步可编译
S-1…S7 每步独立可编译、1 commit = 1 逻辑变更，每步过 RFC-008 §5 六命令：
`cargo build` / `cargo build --features axon-pb` / `cargo clippy --all-targets -- -D warnings` /
`cargo clippy --all-targets --features axon-pb -- -D warnings` / `cargo test` / `cargo test --features axon-pb`。

## I9. 依赖 A2 先行
本工作依赖 carrier-unification（A2 / F-004）保证的帧七元组完整 + `DispatchResult.receipt`。
若 A2 未落到位，`InvocationReceived` 无法解码干净 envelope —— 此为前置 gate，非本 RFC 范围。
