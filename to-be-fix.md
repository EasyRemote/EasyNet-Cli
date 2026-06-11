# to-be-fix.md — 跨仓技术债清单(增量维护)

> 配对文档:`to-be-fix.plan.md`(架构/状态机现状与理想态)。
> 每条:仓库 · 落点 · 类别(架构/性能/规范/重量)· 严重度(高/中/低)· 证据 · 修复方向。
> 【已核验】= 对照磁盘确认;【agent 盘点】= 需复核后再动手。编号永不复用。

---

## EasyNet-Cli

### F-001 daemon_invocation_service.rs 是 13,142 行 god-file 【已核验】
- 落点:src/services/invocation_transport/daemon_invocation_service.rs · 重量/架构 · **高**
- 证据:`wc -l` = 13,142;同文件混装:unary/stream/bidi 三个 RPC 面、session accept、
  drain、invoke_remote 路由、forward_invoke、路由解析、6000+ 行测试。
- 违反:一模块一职责;共享 checkout 下多人改同文件 = 永久合并冲突源。
- 方向:按 RPC 面 + 路由 + session-accept 拆 4–6 个模块;测试随实现走。

### F-002 transport 平面其余三文件同类超重 【已核验】
- 落点:session_initiator.rs 3,074 行 / boot.rs 2,886 行 / local_session_dispatcher.rs 2,525 行 · 重量 · 中
- 证据:boot.rs 的 `start_daemon_invocation_transport` 单函数贯穿 500+ 行;
  `spawn_session_supervisor` 8 参(clippy too_many_arguments 现行警告,boot.rs:968)。
- 方向:dial / prelude / supervisor / warmup 分模块;参数收拢为配置结构体。

### F-003 回退曲线宣称 jitter,实现无 jitter 【已修复 2026-06-11】
- 落点:session_initiator.rs · 性能/架构 · **高**
- 修复:`full_jitter(bound)` = 均匀采样 [0, bound](AWS full-jitter);确定性翻倍曲线
  保持为上界,仅 sleep 时刻随机化 → 曲线的升降/reset 语义不变,雷群被打散。
  三处文档措辞("with jitter"/"Bounded jitter")与实现同提交对齐(此前文实不符)。
  测试:full_jitter ∈ [0,bound]、零界零等待、1000 抽样 >100 distinct(验非退化);
  next_backoff 确定性测试不受影响。352 transport 测试全过。

### F-004 会话热路径双重序列化:JSON-in-protobuf 【已核验,量化待测】
- 落点:daemon_invocation_service.rs `drain_session_up_stream`(每帧 `serde_json::from_slice::<SessionDispatch>`)、
  `push_session_request_result`(`serde_json::to_vec` 包进 BinaryChunk)及全部对称点 · 性能 · **高(方向)/待量化**
- 证据:业务帧 = protobuf BinaryChunk 包 JSON;二进制 payload 在 JSON 内意味 base64 膨胀 ~33% + 每帧两次分配/解析。
- 方向:先基准(每秒帧数 × P99 延迟 × 分配),后定型 proto oneof 帧;**不测不改**。
- 诚实注:当前规模(单设备低频 dispatch)可能无感;列高是因为它是协议形状,越晚改迁移成本越高。

### F-005 lib 容忍 20 条 clippy 警告 【已核验】
- 落点:全仓(`cargo clippy --lib --features axon-pb` = 20 条) · 规范 · 中
- 证据:含 result_large_err(AxonError ≥144 字节,热 Result 路径每次返回都搬运)、
  type_complexity(session_initiator.rs:155 probe 类型、boot.rs escalation 元组)、too_many_arguments。
- 违反:工业教科书 = clippy 零警告;警告堆积让新增警告隐身(本会话差点误判 20→21)。
- 方向:分责清零;AxonError 装箱(性能顺手赚);CI 加 `-D warnings` 防回潮。

### F-006 OnceLock 全局单例 HubPublishedAbilityStore::global() 【已核验】
- 落点:src/services/hub_published_ability_store.rs:149-151 · 架构 · 中
- 证据:`static INSTANCE: OnceLock<Arc<…>>`;session prelude(session_initiator.rs join 回执处理)直接取全局。
- 违反:运行时依赖必须构造注入(本仓自家家规);隐藏依赖 + 测试隔离脆弱(跨测试共享可变状态)。
- 方向:store 随 boot 构造,经参数/字段注入两处消费点;global() 过渡期保留并标 deprecated。

### F-007 会话取消句柄 Box::leak,无优雅停机 【已核验】
- 落点:boot.rs(`Box::leak(Box::new(cancel_tx))`,session supervisor 的 cancel oneshot) · 架构 · 中
- 证据:cancel_tx 故意泄漏 → supervisor 的 cancel 分支永不可达;daemon 停机 = 进程级击杀,
  会话无 drain、上行帧可能截断(hub 侧表现为 StreamReset 而非 Eof)。
- 方向:cancel_tx 收进 daemon 关停路径(已有 SIGHUP 任务基建可挂);verify:停机时 hub 看到干净 Eof。

### F-008 设备会话生命周期为隐式状态机 【已核验,深挖见 plan §2.1】
- 落点:session_initiator.rs(dial + supervisor 控制流即状态) · 架构 · **高**
- 证据:阶段无类型表示;关闭分类靠事后指纹;2026-06-11 乒乓事故的诊断成本即此债的利息。
- 方向:plan §2.1 理想态(DeviceSessionState + CloseClass 一等化 + 转移 op_event)。

### F-009 hub 槽位无申领者指纹,乒乓类事故无源头防御 【已核验,深挖见 plan §2.2】
- 落点:presence_registry.rs PresenceSlot / daemon_invocation_service.rs accept 路径 · 架构 · 中高
- 证据:同 URA 双申领者交替顶替时,hub 无法区分「同设备换代」与「双设备打架」;
  只能靠 device 侧回退吸收(b2ba441)。
- 方向:frame0 携带 boot nonce 指纹;异指纹高频交替 → claimant_conflict 事件 + 快速 re-admit 拒绝。

### F-010 单 crate 巨石构建 【已核验(构建行为),拆分方案待设计】
- 落点:Cargo.toml(单 crate,三 crate-type,lib 测试 3,152 个;axon-pb 拉 tonic 全家) · 重量 · **高**
- 证据:链接 4–8GB,历史 OOM SIGKILL 144;任何一行改动 = 全量重链;
  共享 checkout 双引擎并行 cargo 会互锁 target/。
- 方向:workspace 化:transport / runtime / facade / persistence 分 crate;先切依赖最少的 persistence 验证收益。

### F-021 eal/interpreter.rs 3,976 行执行引擎单体 【已核验,第 3 轮】
- 落点:src/eal/interpreter.rs(wc = 3,976;含 37 个同文件单测) · 重量 · **高**
- 证据:单文件承担阶段调度、rayon 并行派发、重试控制、trace 生成、loop 执行、receipt 链、变量代换。
- 方向:按 调度/派发/重试/trace/receipt 拆 5–7 模块;测试随实现走。

### F-022 mission 运行状态 = String 字面量 + pid 文件即状态 【已核验,第 3 轮】
- 落点:src/facade/cli/mission_runs.rs:118(`pub status: String // "ok"|"error"|"partial"|"running"|"cancelled"`) · 架构/规范 · 中高
- 证据:状态无编译期约束;"running" 由运行目录 pid 文件的**存在性**表示——磁盘文件即状态机,
  进程异常退出时有 pid 残留→永久假 "running" 的风险面(残留清理路径待查证)。
- 方向:`enum MissionRunStatus` 单点序列化;liveness 改心跳时间戳;见 plan §2.6 理想态。

### F-023 EalError 是 enum 壳 + String 载荷,控制流靠错误嗅探 【已核验,第 3 轮】
- 落点:src/eal/error.rs(Validation/NotFound/Unavailable(String));src/eal/interpreter.rs:718、:874
  `lower.contains("daemon not running")` 决定 fallback 分支 · 规范 · 中高
- 违反:模块边界 typed error 家规;上游改一行错误文案 = 改变这里的控制流。
- 缓和:底子是好的(刻意不实现 `From<String>`,变体名领域化)——只差把判别信息升进类型。
- 方向:`DaemonOffline` 等一等变体;嗅探点全部改 match。

### F-024 EAL 转义责任未定约(\" 原样保留 vs wrapper 自行 unescape) 【lexer 已核验;wrapper 面 agent 盘点】
- 落点:src/eal/lexer.rs:166-198(契约注释 + 逐字保留,设计本身成立且有测试);
  缺的是下游:谁 unescape `*_json` 参数无文档、无共享 helper、无端到端转义往返测试 · 架构 · 中高
- 证据:项目记忆两次踩坑(「EAL keeps \" escapes verbatim, wrappers must unescape *_json args」);
  每个新 wrapper 都要靠记忆正确处理 = 系统性 bug 源。
- 方向:定约入 EAL 规范 + 共享 unescape helper + `\"…\"` 端到端往返测试钉住。

### F-025 【已撤销,第 4 轮】trace_id / causal_parents「双层 lowering」
- 撤销理由:复核证明是**分层非重复**——interpreter 将 `run.trace_id` 作参数下传(:600-607),
  `local_daemon_grpc.rs:559-560` 是唯一写 `envelope.trace_id` 的点;causal_parents 的
  JSON↔ReceiptRef 两表示是 domain→wire 的正常 lowering。单一源头成立,无漂移面。

### F-026 【已撤销,第 4 轮】Loop「hermetic」宣称与全局 receipt_graph 不符
- 撤销理由:RFC §3.1 的 hermetic 指**变量绑定作用域**,且已强制执行
  (interpreter.rs:1531、:1587「inner bindings do not leak」+ 编译期测试
  `hermetic_scope_outer_binding_not_visible_inside_body`,docs/plans/pr-10-loop-only.md:54);
  receipt_graph 跨迭代可见是 04559af 的**有意设计**(循环因果链依赖它)。文实相符。

### F-027 runtime/agents 平面 43,449 行;mod.rs 自身 2,918 行 【已核验,第 3 轮】
- 落点:src/runtime/agents/(real_invoke_tests.rs 3,313 行测试躺 src/;chat 2,440 / mcp_reflective 2,249 / think 2,141) · 重量 · 中高
- 方向:mod.rs 只留装配与导出;real_invoke_tests 出 src/ 进 tests/(保留 #[ignore] 契约)。

### F-028 mission 上下文经环境变量 + thread-local 传播 【已核验,第 3 轮】
- 落点:EASYNET_MISSION_ID 命中 5 文件(runtime/dispatch.rs、runtime/context.rs、facade/cli/agent.rs、
  facade/cli/mission_runs.rs、bin/real-user-smoke.rs) · 架构 · 中
- 违反:环境变量 = 进程级全局可变状态;async/rayon 混编下 thread-local 传播脆弱,测试并发互踩风险。
- 方向:显式参数 / tokio task_local 传播;env var 仅保留给子进程边界并文档化。

## EasyNet-Axon

### F-011 LocalRuntime 全局锁拓扑:admission 同步锁横跨验签 + inner 单把全局锁 【已核验,2026-06-11 第 2 轮】
- 落点:sdk/rust/src/invocation/local_runtime.rs:139(`inner: tokio::Mutex<RuntimeInner>`)、:148(`admission: std::sync::Mutex<AdmissionState>`) · 性能/架构 · **中高**
- 证据:① :142-148 文档自述 sync 锁「held only while running the four-step admission pipeline —
  a few hash-lookups and one signature verify」——即 **Ed25519 验签(~50-100µs CPU)在全进程互斥锁内**:
  admission 吞吐天花板 = 单核验签速率(~10-20k/s),并发下 "will not block runtime progress" 的自评过于乐观;
  ② `inner` 一把全局 async Mutex 在 ~25 个调用点反复抓取(:590-:1873,能力查找/core/inbox 簿记),
  执行本身不在锁内(:1873 lookup-clone-release,好),但所有 invocation 的簿记串行过同一把锁。
- 缓和因素:设计是有意为之(setter 同步化对齐多语言 SDK 人体工学),且当前规模未测出瓶颈。
- 方向:abilities 注册表读多写少 → ArcSwap/RwLock 读路径;nonce 窗口与 key resolver 分锁;
  验签移出互斥段(先取快照再验);**改前必须基准**(与 F-004 同一条纪律)。

### F-012 receipt.state 以字符串在链上往返 【已核验,2026-06-11 第 2 轮】
- 落点:sdk/rust/src/invocation/audit.rs:139(`pub state: String`)、:350(`state: impl Into<String>`) · 规范 · 中
- 证据:InvocationState 有 9 态显式 enum 且 pin 到 wire(标杆),链对象 ReceiptBody 却退化为自由字符串——
  非法状态字符串可进入收据链,验证链时才(或不)发现。
- 方向:ReceiptBody 持 enum;wire/JSON 层一次转换;canonical 字节格式不变(不破坏既有签名)。

### F-013 【已撤销,2026-06-11 第 3 轮】Voice / AbilityRegistration 隐式状态
- 撤销理由:核验推翻两半。voice.rs 的 VoiceCallState 实为显式 enum 且有 wire 往返测试
  (:282-292、:363 `voice_state_values_match_proto`),文档明言「hosts do not invent local
  call-state」;ability_registry.rs 实测 **246 行**(agent 报 8,503)且无 `changed` 标志命中。
- 编号保留作审计痕迹,不复用。

### F-014 interop_native/execution.rs 3,114 行单体(unsafe 论断撤销) 【已核验修正,第 3 轮】
- 落点:core/runtime-rs/src/interop_native/execution.rs(wc = 3,114) · 重量 · 中(降级)
- 勘误:`grep -c unsafe` = **0**——上轮 agent 的「手动内存安全断言密集/unsafe 边界」论断不成立,撤销。
- 残留问题仅文件体量;方向:按职责拆分,无安全面紧迫性。

## EasyNet(backend + Frontend)

### F-015 internal/axon 手写 fork Axon 协议层 【agent 盘点 + 实测修正,2026-06-11 第 2 轮】
- 落点:backend/internal/axon/(合计 **7,765 行**;最大:namespace_resolve_answer.go 677 /
  federation_calls.go 649 / urns.go 557 / invoke_client.go 267) · 架构 · **高(最重的跨仓债)**
- 勘误:上轮 agent 报 invoke_client.go "11,734 行" 系误读(实测 267 行);债的体量在文件数与协议面覆盖,不在单文件行数。
- 证据:8/15 关键文件 fork envelope/admission/URA;agent_uri/agent_ura 漂移;
  Axon Go SDK 已导入却基本未用;RFC-001 delta table P3 清理仍 Draft。
  佐证:docs/easynet-backend-boundary-audit-2026-06-08.md。
- 方向(替换批次,详见 plan §3.2):A 批 URA(urns.go,顺手消灭 F-016)→ B 批 envelope/admission
  (收掉 F-017 全局开关)→ C 批 invoke 客户面(对齐 delta table 两方法 Client)→ D 批 federation
  (677+649 行,最活跃,最后动,e2e answer-sheet 回归把门)。每批:冻结 fork 增量 → 替换 → 删除。

### F-016 【已撤销,第 6 轮】urns.go 术语漂移(URI/URN/URA 混用)
- 撤销理由:实测 urns.go 中 `_uri` / `_ura` / `URI` 全部 **0 命中**;`agent_uri` 全后端
  非测试 Go 文件 0 命中——2026-06-08 边界审计记录的漂移已在此后被清理。残留仅文件名
  `urns.go` 一词,随 F-015 A 批替换自然消亡,不立独立条目。

### F-017 SubjectEnforcement 全局 atomic + init() 环境读取 【已核验,第 6 轮】
- 落点:backend/internal/axon/admission.go:54-65(`var SubjectEnforcement atomic.Int32` +
  `func init()` 读 env)、:101(Load 判分支) · 架构 · 中
- 证据:进程级一次性开关;运行时不可重配;测试需手工复位。
- 方向:收进配置对象随依赖注入(随 F-015 B 批);阶段推出语义保留。

### F-018 InvokeAbilityDialog.tsx 2,926 行巨型组件(kernel.go 半边撤销) 【已核验修正,第 5 轮】
- 落点:Frontend/src/components/InvokeAbilityDialog.tsx(wc = 2,926 ✓) · 重量 · 中
- 勘误:kernel.go 实测 **938 行**(agent 以 33KB 字节数误判「巨大单体」),低于本审计 god-file 线,撤销该半边。
- 方向:Dialog 按 参数编辑/执行/结果 三分。

### F-019 【已撤销,第 6 轮】媒体会话双实现并存
- 撤销理由:DeviceMediaAccess.tsx 中 `RTCPeerConnection` / `new WebSocket` 计数 = **0**,
  其 import 走共享 `media-channel-invocation` 层——它是 store 体系的**消费者组件**
  (被 DeviceDetailPage / DeviceMediaWorkspacePage 挂载),不是并行实现。「双实现」不成立;
  1,487 行组件体量在 Dialog(F-018)之下,不另立条目。

### F-020 backend 不做 receipt 链验证且边界未文档化 【agent 盘点】
- 落点:backend/internal/receipt/(仅记录与查询) · 规范 · 低中
- 方向:边界决定(信任 Axon 已验)写进 boundary 文档;或补链验证调用。

### F-029 backend handler 层 DB 访问泄漏(5 文件 8 处) 【已核验,第 5 轮】
- 落点:handler/openai/chat_completions.go(3 处)、pages_public/serve.go(2 处)、
  terminal/wshandler.go、sse/sse_handler.go、device/verifyCredentialHandler.go(各 1 处) · 架构/规范 · 中
- 勘误:agent 报 "~19 行",实测 grep = **8 处调用点**;类别成立,计数修正。
- 违反:三层纪律;多数 handler 干净,这 5 文件是离群点。
- 方向:查询下沉 logic 层;CI 加 handler 目录禁 import ent 的 lint。

### F-030 backend goroutine recover 缺口 + 错误包装率 50/99 【已核验,第 5 轮】
- 落点:middleware/apikey.go:97-101(每请求 `go func` 异步 DB update,无 defer recover——
  goroutine panic = 进程死);logic 层 fmt.Errorf 99 处中 %w 恰 50 处 · 规范 · 中
- 方向:后台 goroutine 统一 recover 包装;%w 拉满。

### F-031 terminal 重连无退避(范围收窄) 【已核验定稿,第 6 轮】
- 落点:terminal-store.ts:112(`MAX_TERMINAL_CONNECTION_FAILURES = 3`)+ :164-167
  (`assertTerminalConnectionAllowed` 硬停,无任何退避) · 架构/性能 · 中(收窄后)
- 收窄记录:① ICE 轮询半边撤销——循环尾 :933 实有
  `await sleep(REMOTE_DESKTOP_ICE_POLL_INTERVAL_MS)`,是带间隔的有界轮询(上两轮的节选
  都停在 sleep 之前,教训:读循环必须读到循环尾);② reconnecting.go 判为**正面设计**
  (按需重拨 + `redialBackoff = 3s` 最小窗 :64-71/:144 + lastErr 错误隔离纪律 :278-285),
  非第四失守点。
- 模式债重估:跨仓退避失守实为 **2 点**(F-003 session supervisor 无 jitter、本条 terminal 硬停),
  仍值得共享 backoff util,但不再按「系统性约定缺失」定性。

### F-032 【已撤销,第 5 轮】media-channel-store 状态转移无守卫
- 撤销理由:复核推翻两半。rdCreate 在 :1733-1738 有完整重入守卫
  (`if (entry.loading || entry.session) return`,注释明确双击竞态与孤儿会话语义);
  patchEntry(:358-369)是内部展示态 helper,带 `!prev` 守卫,非公开转移面。
  agent「不检查既有 session」论断与磁盘代码直接矛盾。

### F-034 mcp_reflective_registry 三处公开 Result<_, String> 【已核验,第 7 轮】
- 落点:src/runtime/agents/mcp_reflective_registry.rs:112(`pub fn parse`)、:572、:1322 · 规范 · 低中
- 违反:模块边界 typed error 家规(同 F-023 类,规模小得多)。
- 方向:并入 F-023 的 typed-error 批次顺手收掉。

### F-035 PairingStatusExpired 死常量 【已核验,第 7 轮】
- 落点:EasyNet backend/internal/domain/constants.go:30(全后端零使用) · 规范 · 低
- 证据:过期语义实际由 `ExpiresAtGT` 时间戳谓词承担(validatePairingLogic.go:77/:134,
  设计本身达标且优于状态写入式);死常量误导读者以为 expired 是存储态。
- 方向:删除,或注释「视图态,勿写入」;随 §2.4 文档批次。

### F-033 facade/cli 面 20,858 行;agent.rs 单文件 3,556 行 【已核验,第 5 轮】
- 落点:src/facade/cli/(agent.rs 3,556;federation_wire 1,383 / start 1,267 / join 1,221 /
  auth 1,120 / agent_new_ability 1,118 各为千行级命令文件) · 重量 · 中高
- 方向:agent.rs 优先拆(子命令各自成模块);其余千行级命令文件观察,不强拆。

---

## 迭代日志

- **2026-06-11 第 1 轮**:F-001…F-020 建档(Cli transport 平面已核验 10 条;
  Axon/EasyNet 10 条 agent 盘点待复核)。
- **2026-06-11 第 2 轮**:F-011 核验实锤(验签在全进程 sync 锁内 + inner 全局锁 25 处抓取,定级中高);
  F-012 核验实锤(audit.rs:139);F-015 勘误(invoke_client.go 实测 267 行,非 11,734)并落替换批次 A→D。
- **2026-06-11 第 3 轮**:EAL/mission 面与 runtime/agents 面入册(F-021…F-028,新增 8 条,6 条已核验);
  **撤销 F-013**(voice 实为显式 enum + wire 测试;ability_registry 246 行无 changed 标志)、
  **修正 F-014**(unsafe 计数 = 0,论断撤销,仅留体量);另拒收一条假阳性(EAL retry jitter
  实测已实装 interpreter.rs:2369-2384,agent「未实装」论断不成立)。
  **流程教训:agent 盘点错误率高(累计 4 条勘误/撤销),任何 agent 条目动手修复前必须亲手复核——已是铁律,再次验证其必要性。**
  **下一轮聚焦:① 复核 F-025/F-026(EAL 双层 lowering 与 hermetic 宣称);
  ② Cli facade/persistence 平面首轮(配置/凭据/URA 解析);
  ③ EasyNet 前端 store 状态机(terminal/media)+ backend handler/logic 层首轮;
  ④ plan §3 理想修复计划完整排序初版。**
- **2026-06-11 第 4 轮**:**F-025、F-026 复核后双双撤销**(trace_id 是分层非重复;hermetic 指绑定
  作用域且已强制 + 编译期测试,receipt_graph 跨迭代可见是有意设计)——agent 论断累计 6 条被推翻。
  persistence 面审计结果:**零新债**,atomic_write 是教科书级正面样本(已记入 plan 校准节)。
  EasyNet 前后端入册 F-029…F-032(handler DB 泄漏、goroutine recover 缺口、前端退避缺失
  ——与 F-003 构成跨仓「退避纪律」模式债、media store 转移无守卫)。plan §3.6 修复排序初版落档。
  **下一轮聚焦:① 亲手复核 F-029…F-032(动手修复前提);② 覆盖剩余盲区:Cli 的
  drivers/execution/gateway 与 facade/cli 全量、Axon runtime-rs services/state、EasyNet daemon_grpc;
  ③ 复核仅存的 agent 条目 F-018/F-019/F-020;④ 向收敛推进:盲区清完后做完整性终审。**
- **2026-06-11 第 5 轮**(全程亲手核验,零 agent):F-029 计数修正后坐实(5 文件 8 处)、
  F-030 坐实(50/99 精确)、F-031 terminal 半边坐实、F-018 的 kernel.go 半边撤销(实测 938 行)、
  F-020 坐实(345 行);**F-032 撤销**(rdCreate :1733-1738 有完整重入守卫,agent 论断与代码直接矛盾,
  累计 8 条 agent 论断被推翻)。盲区清点:drivers/gateway/daemon_grpc/Axon state 面**均无 god-file 级新债**;
  新增 F-033(facade/cli 20,858 行,agent.rs 3,556)。
  **下一轮聚焦:① 读 daemon_grpc/reconnecting.go 判退避模式债第四点还是正面模板;
  ② F-019 双实现关系深读 + F-031 ICE 半边复核;③ F-016/F-017 快验收尾;
  ④ 收敛终审第一步:对照 plan §1 全景表逐 feature 标注「已审/未审」,产出完整性缺口清单。**
- **2026-06-11 第 6 轮**:待复核清零。**F-016 撤销**(urns.go 三关键词 0 命中,agent_uri 全后端
  0 命中——06-08 审计后漂移已被清理)、**F-019 撤销**(消费者组件非双实现,RTC/WS 原语 0 命中)、
  **F-031 收窄定稿**(ICE 半边撤销——循环尾 :933 实有 sleep;reconnecting.go 判正面设计;
  跨仓退避失守重估为 2 点)、F-017 坐实。agent 论断累计 **11 条**被亲核推翻。
  plan §4 覆盖全景打勾表落档,收敛路径定为两轮。
  **下一轮聚焦:① EasyNet 配对状态机深挖(最后一个未落档的状态机 feature,plan §2.4 展开);
  ② ura.rs / axiom / mcp_reflective / dendrite_bridge 质量抽查(时间盒);③ 为第 8 轮终审清场。**
- **2026-06-11 第 7 轮**:配对状态机深挖落档(plan §2.4:谓词守卫 + 时间戳过期,
  三仓隐式状态机质量之最,只差单一真源 + 文档);四个时间盒抽查——ura.rs/axiom/dendrite_bridge
  **干净**(2 unwrap/286 行;canonical_* 分型 5 unwrap;FFI 14 unsafe/2,094 行密度合理),
  mcp_reflective 录 F-034(三处 pub Result<_,String>);新增 F-035(PairingStatusExpired 死常量,
  过期实际由 ExpiresAtGT 谓词承担——设计达标)。
  **下一轮(第 8 轮,预期终轮):① 收口最后 ✗:Cli core/ability_spec + ffi 抽查、
  Axon ura-rs/proto/conformance 抽查或声明审计边界、ent schema 抽查;
  ② 完整性终审:plan §4 表无 ✗、清单全核验、§3 终稿复核;③ 达标则跳出 loop 并出收敛报告。**
- **2026-06-11 第 8 轮(终轮)**:收口抽查全过,无新债(ability_spec 2,174 / ffi 2,407 低于入册线;
  ura-rs 1,560 适中;proto 16 文件全版本化;ent schema 纪律普遍)。conformance 声明审计边界。
  plan §4 全景表无 ✗;§3.6 排序终稿(撤销条目清出,F-033/034/035 归位)。
  **终态:35 条编号 = 29 活跃(全部已核验)+ 6 撤销;状态机 feature 全部落档(§2.1–2.6);
  agent 假阳性累计 11 条全数拦截。审计收敛,loop 退出。**
  剩余开放面(声明,非缺口):Axon conformance 深审、各 god-file 拆分后的逐文件复审——
  属修复执行期工作,不属本审计范围。
