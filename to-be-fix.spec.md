# to-be-fix.spec.md — 跨仓技术债修复规格(v1,2026-06-12)

> 三件套之三:`to-be-fix.md`(债清单,29+3 活跃条全部已核验)· `to-be-fix.plan.md`
> (状态机现状/理想态 + 排序)· 本文件(**修复执行规格**:架构问题定性、TODO、验收门、依赖序)。
> 基线:截至 2026-06-12,已完成 F-003(full-jitter)、F-007(SessionShutdown 优雅停机)、
> F-039(retired 别名);F-005 从 20 清到 8 并装 `#![deny]` 棘轮。撤销 6 条不在本 spec 范围。

---

## §1 架构问题定性(七类)

### A1. 会话平面:隐式状态机 + 顶替无源头防御(F-008 / F-009)
**本质**:`<self>.session` 设备会话的生命周期没有类型表示——「当前处于什么阶段」散落在
dial 函数的控制流位置里;hub 侧槽位(PresenceSlot)只认 URA 不认申领者,同 URA 的
「同设备换代」与「双设备打架」不可区分。
**已付利息**:2026-06-11 的 5428 次乒乓重连事故。device 侧放大器(clean close 重置退避)
已修(b2ba441 + F-003 jitter),但**根架构未动**:hub 侧任何 drain 早退/顶替在 device 看
都是无差别 clean close,事故归因仍靠事后指纹。
**终态**(plan §2.1/2.2):`DeviceSessionState` + `CloseClass` 一等化、转移 op_event;
frame0 携带 boot-nonce 申领者指纹,异指纹高频交替 → `claimant_conflict` 事件 + 快速
re-admit 拒绝;admission receipt 携带契约版本号,契约偏斜变显式错误。

### A2. 第二 invocation 载体:SessionDispatch JSON 帧(F-004 + F-038 + F-040 同病)
**本质(2026-06-12 边界镜头升格)**:会话业务帧 = protobuf `BinaryChunk` 包 JSON,
携带 ability+args+origin claim+result——这是 **Axon Invocation 之外的第二 invocation
载体**,形状由 Cli 拥有。它的两个并发症已经兑现:backend 在 Go 里逐字节手抄这套
struct(F-040,「no translation layer」是注释原话);跨仓 fixture 抓到 `ability_ura`
字段漂移(F-038)。性能问题(base64 +33%、每帧双解析)只是表症。
**裁决依据**(runtime-boundary skill):「daemon 控制帧不得成为 Invocation 构造/签名/
receipt 绑定的第二真源」;「JSON 控制帧降级到 status/boot/lifecycle/diagnostics」。
**终态**:dispatch 帧承载 canonical Invocation/proto 形状;JSON 降级诊断面;backend
向 daemon Invocation 提交完整七元组(F-040 随之消亡)。迁移前置:caller 盘点
(Invoke/Subscribe/OpenBidi 逐调用方分类)+ 基准(量化顺带收益)。

### A3. Axon LocalRuntime 锁拓扑(F-011)
**本质**:两把全局锁。`admission: std::sync::Mutex` 按文档自述横跨 Ed25519 验签
(~50-100µs CPU)——**全进程同一时刻只能验一个签名**,admission 吞吐天花板 = 单核验签率;
`inner: tokio::Mutex<RuntimeInner>` 在 ~25 个调用点抓取,所有 invocation 的簿记串行
过同一把锁(所幸能力执行不在锁内)。
**缓和**:有意设计(setter 同步化对齐多语言 SDK),当前规模未测出瓶颈。
**终态**:注册表读多写少 → ArcSwap/RwLock;验签移出互斥段(锁内取快照、锁外验);
nonce 窗口/key resolver/ledger sink 分锁。**基准先行,同 A2 纪律**。

### A4. backend 协议真源二元化(F-015 / F-017 / F-040)
**本质(边界镜头定性升格)**:不是「backend 没怎么用 SDK」,而是**协议真源二元化**——
boundary skill Rule 1 的拒绝类。两个平面:① `internal/axon/` 7,765 行复刻
envelope/admission/URA 协议层(F-015);② `daemon_grpc/invoke_remote.go` 把产品调用
包成 daemon-internal 的 `<self>.invoke_remote`,并逐字节手抄 Cli 的 JSON 帧 struct
(F-040,skill 明令禁止的两条都踩)。fork 与漂移**没有编译期信号**,事故链已兑现过一次
(unknown field → resolve 被拒 → 静默 fallthrough → device 全量误判 REMOVED)。
**对照面**:Cli 自己是合规范本——src/ura.rs 纯门面 + 守卫脚本、AdmissionFacade 委托
SDK、FFI 七元组必填。规则不是做不到,是 backend 没接上。
**终态**:A→D 四批替换(URA → admission(收 F-017)→ invoke 客户面 → federation),
每批冻结→替换→删除,e2e answer-sheet 把门;F-040 随 A2 载体归一消亡(backend 提交
完整 Invocation,daemon 拥有路由)。**这是三仓最重的一笔债**:协议每演进一次要改两处、赌一次。

### A5. 巨石与 god-file(F-001 / F-002 / F-006 / F-010 / F-021 / F-027 / F-033)
**本质**:单 crate 三 crate-type 巨石(链接 4–8GB,历史 OOM SIGKILL,共享 checkout
双引擎 cargo 互锁);daemon_invocation_service.rs 13,142 行(三个 RPC 面 + 路由 +
accept + drain + 6,000 行测试同文件)、interpreter.rs 3,976、agent.rs 3,556、
agents/mod.rs 2,918。
**为什么是架构而非卫生**:文件即模块边界。13k 行 = 评审不可能完整、共享 checkout 永久
冲突源、任何修改全量重链。F-006(OnceLock 全局单例)评估为中型,归并 transport 拆分
同批处理(store 随 boot 构造注入,5 消费点改签名)。
**终态**:workspace 化(persistence 先切验证收益)+ 五处 god-file 按职责拆分。

### A6. mission 运行态与上下文传播(F-022 / F-028)
**本质**:mission 状态是 String 五字面量,"running" 由 **pid 文件存在性**表示——磁盘
即状态机,进程异常退出 = pid 残留 = 永久假 running;mission 上下文经
`EASYNET_MISSION_ID` 环境变量 + thread-local 传播(5 文件),async/rayon 混编下脆弱、
并发测试互踩。
**已评估**(2026-06-12):非小修——serde wire 兼容 + liveness 改心跳 + 跨仓消费面,
与 liveness 改造一并做。
**终态**(plan §2.6):`enum MissionRunStatus { Running{heartbeat}, … }` 单点序列化;
上下文显式传参/task_local,env var 只留子进程边界。

### A7. 跨仓一致性基础设施失修(F-036 / F-037 / F-038)
**本质**:跨仓契约的**执行回路断了**——conformance baseline 落后实际 29 条
(888 vs 917,落后会掩盖真实违规趋势);Federation-MVP 仓的 session_dispatch 基线缺
`ability_ura` 字段(wire 演进未重生基线,fixture 正确地报了警);pages u14 在他人
重构后破损未同步。三条共性:**守护机制存在,但没人按节拍喂它**。
**终态**:baseline 变更纳入 wire 改动的 DoD(改 wire 必重生基线 + 必更新计数);
F-036 需 CTO 决策逐条审 28 条历史违规归属,不一次性吞。

### 跨仓模式债:退避纪律(F-003 已修 / F-031 残留)
session supervisor 与前端 terminal-store 两处独立失守(后者硬停 3 次无退避)。
共享 backoff 约定(指数 + jitter + 上限)前后端各一实现;Cli 侧 full_jitter 已落,
terminal-store 接入即收口。

---

## §2 TODO(批次化,含验收门)

> 标注:〔仓〕规模(S<半天 / M≈1-2天 / L≥3天)。依赖以 → 表示。

### Phase 0 — 决策与基线(其余阶段的前置)
| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T0.1 | **F-036 决策**:逐条审 917-888=29 条 conformance 违规归属(大头 MCP keyword 743 属 P4.8d 移除待办),按真值更新 baseline 或修违规。**CTO 决策项,不一次性吞** | Cli | M(审) | check-rfc-001-baseline-lock.sh 绿,且每条增量有归属记录 |
| T0.2 | **F-037**:pages u14 修复——4aa2412 作者定:测试期望随重构更新或 owner→name 组合修回 | Cli | S | `cargo test --test pages_unit` 15/15 |
| T0.3 | **F-038**:确认 SessionDispatch::Request 增 ability_ura 是有意 wire 演进 → Federation-MVP 重生 session_dispatch.json 基线 | Fed-MVP | S | `cargo test --test session_dispatch_fixture` 绿 |
| T0.4 | **基准 harness**:会话帧路径(帧率/P99/分配)+ LocalRuntime admission 吞吐两个基准,作为 T2.1/T3.1 的 gate | Cli+Axon | M | 基准可重复运行,数字落档 |

### Phase 0.5 — DEC-F048 本体执行闸(2026-06-11 决议落地,优先级同 P0)
| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T0.5a | delegation 验证拒绝 device-owned 受托方(System Agent MUST NOT accept delegated authority) | Axon | S | 拒绝路径单测 + 错误文案引用 RFC-005 §3.1.2 |
| T0.5b | hosted-agent 注册路径断言 owner 非 `device.`(hosted user agent ≠ device-owned) | Cli | S | 注册面双形状测试(user-owned 过 / device-owned 拒) |
| T0.5c | F-047 八点按 sponsor 语义复核后施工;mcp_reflective owner_kind 点重判(疑应归 User Agent) | Cli | M | 判定表 v2 + 8 点实现 + 双形状测试 |
| T0.5d | RFC-005 §3.1.2 提交入库(现在 Axon 工作树) | Axon | S | CTO 过目后提交 |

### Phase 1 — 事故级架构(P1)
| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T1.1 | **F-008 会话状态机显式化**:DeviceSessionState + CloseClass(plan §2.1),转移集中一处,每转移发 `session_state_transition{from,to,reason}` | Cli | L | 状态覆盖测试 + 非法转移测试;现有 354 transport 测试不回归;op_event 字段齐 |
| T1.2 | **F-009 claimant 指纹**:frame0 携 boot nonce;hub 槽位存指纹;异指纹快速交替 → claimant_conflict 事件 + N ms 内 re-admit 拒绝;`OfflineReason::Displaced{by_same_claimant}` | Cli | M → T1.1 后 | 双申领者集成测试:冲突被检出且事件可见;同设备重连不受影响 |
| T1.3 | **F-031 共享退避收口**:前端 backoff util(指数+jitter+上限),terminal-store 接入替换 3 次硬停 | EasyNet FE | S | 重连间隔按曲线增长的单测;UX:上限后明确提示 |
| T1.4 | **F-024 转义定约**:契约写入 EAL 规范(payload 原样直达 wrapper;`*_json` 由 wrapper unescape)+ 共享 unescape helper + `\"…\"` 端到端往返测试 | Cli | M | 往返测试钉住;两个既有 wrapper 迁移到 helper |

### Phase 2 — 协议形状(P0 长线,T0.4 之后)
| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T2.0 | **✅ 已完成(2026-06-12 第 10-11 轮,审计期执行)**:caller 盘点结论——backend JSON 路径已退役(仅余 F-044 两注释);Cli control.sock 自标 Legacy 且有书面不变量「Nothing on control.sock dispatches product abilities」(easynet-daemon.rs:32),即 skill 迁移第 4 步(JSON 降级 boot/status/诊断)**已满足**;EAL interpreter 不触 control.sock。**载体债收窄到唯一面:gRPC bidi 内的 SessionDispatch 帧** → T2.1 范围即全部剩余工作 | — | 完成 | 见 to-be-fix.md 第 10-11 轮日志 |
| T2.1 | **F-004 载体归一**:dispatch 帧承载 canonical Invocation/proto 形状(JSON 降级诊断面);滚动升级(双读单写一版)。性能基准做对照而非 gate(边界违例本身已构成改造理由) | Cli(+Axon proto) | L → T2.0 | 基准对比落档;新旧帧互通一个版本;352+ transport 测试迁移;F-038 类漂移不再可能(单一形状源) |
| T2.1b | **F-040 收口**:backend 改为向 daemon Invocation 提交完整七元组,退役 `<self>.invoke_remote` 包装与手抄 struct(过渡:共享形状入生成代码) | EasyNet BE | M → T2.1 | invoke_remote.go 手抄 struct 删除;contract test 改打 Invocation 面 |
| T2.2a | **F-015 A 批**:urns.go(557)→ Axon Go SDK URA builders,删 fork 文件 | EasyNet BE | M | 编译零 fork 引用;URA 单测迁移;e2e 配对流程绿 |
| T2.2b | **F-015 B 批**:admission fork → SDK;**F-017 全局开关收进配置注入**(阶段推出语义保留) | EasyNet BE | M → A 批后 | admission e2e;开关运行时可重配;测试无手工复位 |
| T2.2c | **F-015 C 批**:invoke 客户面对齐 delta-table 两方法 Client 接口 | EasyNet BE | M | invoke/InvokeStream e2e |
| T2.2d | **F-015 D 批**:federation_calls(649)+ namespace_resolve_answer(677)→ SDK | EasyNet BE | L | answer-sheet 跨域 e2e 绿;internal/axon 仅余薄 glue 或删除 |

### Phase 3 — 吞吐与并发(P2,gate=基准)
| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T3.1 | **F-011 锁拓扑**:验签移出互斥段(快照后验);abilities 注册表 ArcSwap/RwLock;nonce/resolver 分锁 | Axon | L,gate=T0.4 基准证实瓶颈 | 基准前后对比;并发 admission 正确性测试(重放窗口语义不变) |

### Phase 4 — 重量与结构(P3,可与 Phase 2/3 并行蚕食)
| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T4.1 | **F-010 workspace 化**:persistence 先切验证,再 transport/runtime/facade | Cli | L | 增量链接时间与内存数字落档;CI 双特性矩阵不变 |
| T4.2 | **F-001 拆分**:daemon_invocation_service 按 unary/stream/bidi/路由/accept+drain 拆 4–6 模块,测试随行 | Cli | L | 每文件 ≤~2,000 行;测试零语义变化;git blame 可追(move-only 提交分离) |
| T4.3 | **F-002 + F-006 同批**:boot/dial/supervisor/warmup 分模块;spawn_session_supervisor 参数收拢;HubPublishedAbilityStore 注入化(5 消费点改签名,global() deprecated);local_session_dispatcher try_dispatch_via_axon 借用/owned 边界一并定 | Cli | L → T4.2 后 | too_many_arguments 清零;global() 生产路径零调用 |
| T4.4 | **F-021 拆分**:interpreter.rs → 调度/派发/重试/trace/receipt 5–7 模块 | Cli | M | 91 EAL 测试零回归 |
| T4.5 | **F-027**:agents/mod.rs 只留装配;real_invoke_tests.rs 出 src/ 进 tests/ | Cli | S | mod.rs <500 行;#[ignore] 契约保留 |
| T4.6 | **F-033**:agent.rs 按子命令拆 | Cli | M | facade::cli 259 测试零回归 |
| T4.7 | **F-018**:InvokeAbilityDialog 三分(参数编辑/执行/结果) | EasyNet FE | M | 组件测试迁移;无行为 diff |

### Phase 5 — 规范卫生(P4,合批顺手做)
| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T5.1 | **F-005 余 8**:6× result_large_err 走 SDK 侧 AxonError 瘦身(跨仓决策)或本仓 Box 化 13 调用点;2× too_many_args 随 T4.3 | Cli+Axon | M | clippy --lib 0 warning;`-D warnings` 进 CI |
| T5.2 | **F-023 + F-034 typed-error 批**:EalError 载荷结构化(DaemonOffline 一等变体),嗅探点改 match;mcp_reflective 三处 Result<_,String> | Cli | M | grep contains("daemon not running") 零命中 |
| T5.3 | **F-022 + liveness**:MissionRunStatus enum + 心跳时间戳替代 pid 文件(serde 兼容迁移) | Cli | M | 旧 run.json 可读;假 running 场景测试 |
| T5.4 | **F-028**:mission 上下文 task_local/显式传参;env var 仅子进程边界 | Cli | M → T5.3 同期 | 并发 mission 测试互不污染 |
| T5.5 | **F-012**:ReceiptBody 持 InvocationState enum,canonical 字节不变 | Axon | S | 既有签名校验测试全过 |
| T5.6 | **F-029**:8 处 handler DB 查询下沉 logic;CI lint 禁 handler import ent | EasyNet BE | S | grep 零命中 + lint 阀 |
| T5.7 | **F-030**:后台 goroutine recover 包装;%w 拉满(99/99) | EasyNet BE | S | grep 验证;panic 注入测试 |
| T5.8 | **F-020**:receipt 链验证边界写入 boundary 文档 | EasyNet BE | S | 文档 + DEC 记录 |
| T5.9 | **F-035**:死常量删除或注释「视图态」 | EasyNet BE | S | grep 零使用确认 |
| T5.10 | **F-014**:execution.rs 按职责拆分(无安全紧迫) | Axon | M | conformance 套件绿 |

### 文档批(随各阶段)
- plan §2.4:配对转移表单一真源 + 配对态×presence 态映射文档(随 T5.9)。
- A7 类 DoD 固化:「改 wire 必重生基线必更新计数」写进 conventions。

## §3 执行纪律(全批次适用)
1. **不测不改**(T2.1/T3.1 硬 gate):性能改动必须有前后基准数字。
2. **move-only 与语义变更分提交**:拆文件的 commit 不夹逻辑改动,reviewer 才可能审。
3. **文实同提交**:文档宣称与实现的修正必须在同一 commit(F-003 模式)。
4. **撤销条目不复活**:F-013/016/019/025/026/032 重提需新证据新编号。
5. **共享 checkout 纪律**:开工前 git status + mtime 检查;绿了立刻提交;pathspec 提交。
6. **每批验收即回归全套**:`cargo test --lib --features axon-pb` + 默认特性 + clippy 棘轮。

## §4 依赖序(关键路径)
```
T0.4 基准 ──► T2.1 帧定型 ──► (其后才值得做帧层微优化)
         └──► T3.1 锁拓扑
T1.1 状态机 ──► T1.2 指纹(状态机给 claimant_conflict 一个家)
T4.2 F-001 拆 ──► T4.3 F-002/F-006(boot 注入依赖拆后结构)
T2.2 A→B→C→D 严格顺序;T0.1–T0.3 立即可做,互不依赖
```
**建议起跑组合**(并行无冲突):T0.1–T0.4 + T1.3 + T1.4 + T5.6–T5.9。
