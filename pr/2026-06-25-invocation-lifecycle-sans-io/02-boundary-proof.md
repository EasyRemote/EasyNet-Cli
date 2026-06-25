# S-1 — 边界证明（Axon 拥有什么 / CLI 留什么）

证明 CTO 修正版边界线是对的：Axon 拥有**协议语义**（与 I/O、运行时、daemon 存储无关的纯演进规则），
CLI 拥有**本地事实**（live lookup）。判据：一段逻辑若 Go backend 复用同一套时**逐字相同**，它是协议语义 → Axon；
若它的答案依赖本进程的 live 状态（presence 槽位、session sender、plugin 注册表），它是 daemon 事实 → CLI。

## 下沉 Axon（协议语义，Go 可逐字复用）

| 逻辑 | 现位置 | 为何是语义 |
|---|---|---|
| 状态转移表（Admitting→…→Settled） | 散在 4 dispatcher 控制流 | 转移规则与传输几何无关；Go hub 用同一张表 |
| 终态分类（Completed/Failed/TimedOut/Cancelled） | unary:1524 等各自手写 | "超时算 TimedOut 不算 Failed" 是协议约定，非本地事实 |
| receipt 投影规则（何时投、投哪种） | `ledger_projection.rs` + 各 dispatcher | "terminal 必投、admission 失败投失败 receipt" 是协议规则 |
| stream/bidi/unary 语义差异（一发收口 vs 流块 vs 双向） | 三 dispatcher | 几何语义是协议定义，由 `DispatchChunk`/`DispatchTerminal` 事件区分 |
| 纯路由类型 `RouteQuery/ResolvedRoute/RouteProfile/DispatchTarget` | CLI-local `route_resolver.rs:146` | 类型形状是协议契约（值不是） |

## 留 CLI（daemon 事实，依赖 live 进程状态）

| 逻辑 | 位置 | 为何是事实 |
|---|---|---|
| `DaemonRouteResolver` live lookup | `route_resolver.rs` | 答案 = 此刻哪个 agent 在线、placement 在哪 —— 进程 live 状态 |
| `PresenceRegistry` / pending maps | transport | 谁此刻连着 —— 纯 runtime 事实 |
| session up/down sender、`SessionUpSender` | local_session | tokio mpsc，I/O primitive，不是语义 |
| plugin/ability registration | runtime | 本进程装了哪些 ability |
| tonic/proto ↔ TransportFrame 转换 | 各 transport_impl | wire 编码细节，Axon core 只见 wire-agnostic frame |
| federation wrapper handlers | `federation_wrappers.rs` | ability handler，非 lifecycle |

## 接缝：Action ⇄ Event 的执行委托

core 发 `Action`（语义意图），CLI driver 执行（用 live 事实），回喂 `Event`（结果）：

| Action（Axon 发） | CLI driver 执行（用 daemon 事实） | 回喂 Event |
|---|---|---|
| `RunAdmission` | 调 `run_descriptor_bound_admission` + NonceReplayStore | `AdmissionGranted/Denied` |
| `ResolveRoute` | `DaemonRouteResolver` live lookup | `RouteResolved/RouteRejected` |
| `DispatchTo` | 按 opaque key 路由到 local runtime / presence / plugin | `DispatchAccepted` |
| `ProjectReceipt` | 按 I4 规则取 receipt（不铸造）投 ledger | （无，原子） |
| `SendFrame` | Transport::send_frame（geometry 特定） | （无） |

**证明闭合**：core 永不直接触碰 presence/session/plugin —— 它只发 opaque-key 的 Action，由 driver 用 live 事实兑现。
故 core 真 sans-IO，Go 复用同一 core + 自己的 driver 即可。
