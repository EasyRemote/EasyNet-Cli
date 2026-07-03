# to-be-fix.plan.md — Feature 架构 / 状态机现状与理想态

> 跨仓审计计划文档(EasyNet-Cli / EasyNet-Axon / EasyNet)。
> 与 `to-be-fix.md`(问题清单)配对:本文件回答「feature 的架构与状态机长什么样、理想态是什么」,
> 清单文件回答「具体哪里不达标」。每轮迭代增量补充,迭代日志见文末。
> 标注约定:【已核验】= 本人对照磁盘确认;【agent 盘点】= Explore 扫描结论,引用前需复核。
>
> **⚠ 状态时效(2026-06-12 注)**:本文件的现状描述与 §3.6 排序为**定稿期存档**
> (2026-06-11),不随修复滚动更新——**live 状态一律以 `to-be-fix.spec.md` §3 行为准**。
> 存档后已闭的大项(速览,防误读):P1 的 F-003/F-031(退避两点)、F-008 主体(T1.1 状态
> 机)已闭;P2 的 F-011 全五步闭;P3 的 F-018(Dialog 四刀)、F-001/F-033/F-027 拆分
> 在制或已闭;P4 大半已闭(F-005 双仓 result_large_err 归零、F-012/F-017/F-022/F-029/
> F-030 等)。排序逻辑本身(P0 协议形状优先、测后改、合批)仍有效。

---

## §1 跨仓 Feature 全景(索引)

### EasyNet-Axon(Rust 协议内核 + SDK)【agent 盘点,结构与本人既有认知一致】
| Feature | 落点 | 状态机性质 |
|---|---|---|
| Invocation envelope 签名/验签(AXIOM-7) | sdk/rust/src/invocation/axiom.rs | 无状态 |
| Admission gate 四步门禁 | sdk/rust/src/invocation/admission.rs | 管道(无状态机) |
| LocalRuntime dispatch | sdk/rust/src/invocation/local_runtime.rs (2476 行) | InvocationState 驱动 |
| **InvocationState 生命周期** | handle.rs(9 态显式 enum,pinned proto wire 号) | **显式 — 全宇宙的标杆** |
| Receipt 因果链 + Ledger | audit.rs / ledger.rs | 链验证;receipt.state 为字符串往返 |
| Nonce 重放检测 | admission.rs NonceReplayStore | 滑动窗口 |
| 流式背压 / CallMode | call_mode.rs(3 态显式) | 显式 |
| Voice session | sdk/rust/src/voice.rs | 显式 enum + wire 测试(F-013 复核撤销初判「隐式」) |
| Ability 注册 | ability_registry.rs | 无状态机债(实测 246 行,无 changed 标志;F-013 复核撤销) |
| C 互操作执行 | core/runtime-rs/interop_native/execution.rs (3114 行) | — |

> 形状补记(2026-06-12):已批准双新形状——`agent/device.<device-id>.<agent-id>`
> (device-sponsored System Agent,DEC-F048;normative 文本 RFC-005 §3.1.2,Axon 52bb764a)
> 与 hosted-agent 设备寻址(35efe641)。消费纪律见 ura-discipline skill 形状表 + 清单 F-047。

### EasyNet-Cli(Rust daemon + CLI)【已核验,本会话深度上下文】
| Feature | 落点 | 状态机性质 |
|---|---|---|
| **`session.open` 设备会话**(本轮深挖,见 §2.1) | session_initiator.rs / boot.rs | **隐式**(控制流即状态) |
| **hub 侧会话槽位 / presence** (§2.2) | presence_registry.rs / daemon_invocation_service.rs | 半显式(slot+session_id) |
| `runtime.invoke_remote` 关联派发 | daemon_invocation_service.rs + PendingDispatchMap | 关联表(call_id) |
| federation.forward_invoke(含 --node CLI) | federation_wrappers.rs / support/federation_invoke.rs | 转发管道 |
| Session escalation(device→hub 反向请求) | session_escalation.rs | 关联表 + outbox 快照 |
| Device/User trust sync(on-miss 取键) | device_trust_sync.rs(单飞+负缓存) | 半显式(负缓存表) |
| Origin-caller claim(跨设备 caller 保真) | origin_caller.rs | 无状态验证 |
| Ability/plugin 运行时、MCP 反射注册 | runtime/agents/* | 已盘点(第 3/7 轮:F-027/F-034) |
| EAL mission 脚本 / 语义算子 | core/ability_spec.rs、eal 相关 | 已盘点(§2.6;半隐式,F-022/F-028) |
| 联邦目录 presence 平面(F-049 揭示) | session_initiator.rs 心跳循环(fc8df1b)/ Axon federation.rs sweeper | 时间戳衰减(15s 无心跳即降级) |

### EasyNet(Go backend + React 前端)【agent 盘点】
| Feature | 落点 | 状态机性质 |
|---|---|---|
| 设备注册/配对 | backend/internal/logic/device/*Pairing*.go | **隐式**(status 字段散落多个 logic 文件) |
| 设备生命周期 | device_state.go(ONLINE/JOINING/SUSPECT/DRAINING/REMOVED/UNKNOWN) | 字符串常量(半显式) |
| Axon 协议层 fork | backend/internal/axon/(实测 7,765 行;invoke_client.go 实测 267 行,初报 11,734 系误读,F-015 勘误) | — 架构债重点 |
| Hub PTY 会话 | internal/runtime/kernel.go(938 行,F-018 勘误)+ pty_driver.go | 隐式 |
| 前端终端/媒体会话 | terminal-store.ts / media-channel-store.ts (2095 行) | TS union(半显式) |
| 能力调用 UI | InvokeAbilityDialog.tsx (2926 行) | 隐式 |

---

## §2 状态机深挖(本轮:EasyNet-Cli invocation transport 平面)【已核验】

### §2.1 设备会话生命周期(`session.open`)

**现状(隐式,状态 = 控制流位置):**

```
[run_session_supervisor 循环]
  Sleep(backoff) → dial_and_run_session_with_idle_timeout:
    credential warmup (spawn_blocking, REST)        ← 软失败
    → Endpoint connect (10s 超时, TLS)
    → federation.join prelude (unary, 软失败)
    → advertise/owner-projection prelude            ← OwnerProjectionFailed 硬失败
    → invoke_bidi 开流 + frame0 EnvelopeOpen
    → 「admitted」(probe.admitted(); 即 RPC 被接受)
    → 帧循环 { 下行帧 / 心跳 / 15s idle 看门狗 }
    → 终态三类: clean EOF (Ok(SessionCloseStats))
              | SessionError (12+ 变体)
              | IdleTimeout
  → 回退决策: backoff_after_clean_close / next_backoff 翻倍, 30s 封顶
```

- 状态没有类型表示;「当前处于什么阶段」只能从日志 kind 推断
  (bidi_opened / initial_admission_observed / bidi_closed_cleanly / bidi_error_reconnecting)。
- 关闭分类是事后指纹(b2ba441 加的 uptime/frames_received),不是一等状态。
- 文档宣称 jitter,代码无 jitter → 文实不符(F-003)。**✅ 已修 2026-06-11**:full_jitter
  落地、文实同提交对齐;理想态中「回退曲线 jitter」一项已兑现,其余(状态类型化)仍归 T1.1。

**理想态:**

```rust
enum DeviceSessionState {
    Idle,
    Dialing { attempt: u32 },
    Preluding(PreludeStep),                       // join / advertise / projection
    Live { since: Instant, frames: u64 },
    Backoff { attempt: u32, until: Instant, last_close: CloseClass },
}
enum CloseClass {                                  // 一等公民,不是日志指纹
    Healthy,                                       // uptime ≥ 健康阈值
    DisplacedSuspect,                              // 亚秒 + 仅 admission 帧
    NoAdmissionReceipt,                            // 0 帧(旧 hub)
    ContractSkew,                                  // ~心跳周期即被关
    Errored(SessionErrorKind),
}
```
- 转移函数集中一处,每次转移发一条 `session_state_transition{from,to,reason}` op_event;
  报警/SLO 直接消费状态转移流,不再 grep 散落 kind。
- 回退曲线:decorrelated/full jitter(兑现文档承诺),雷群消除(hub 重启时全舰队不同步重连)。
- frame-0 admission receipt 携带 **契约版本号 + hub session_id + displaced_prior**:
  契约偏斜变成显式错误(而非 clean close),displacement 在 device 侧可直接观测。

### §2.2 hub 侧会话槽位(PresenceRegistry)

**现状(半显式):** `PresenceSlot { session_id: u64, sender }`,顶替 = insert 覆盖
(Offline→Online 事件序),移除身份感知(remove_if_session,有回归测试)。
状态实际只有 {Vacant, Live};「同 URA 第二申领者」无指纹区分,
乒乓顶替类事故(2026-06-11,5428 循环)只能事后从 device 指纹推断。

**理想态:**
- slot 增加 **claimant fingerprint**(设备每次进程启动的 boot nonce,frame0 携带):
  同设备重连(同 fingerprint)= 正常换代;异 fingerprint 高频交替 = 双申领者,
  hub 直接发 `claimant_conflict` op_event 并可拒绝快速 re-admit(< N ms)——
  把整类乒乓事故消灭在源头,而非靠回退曲线吸收。
- 顶替序列(Offline→Online)保持;但 displaced 原因细分
  `OfflineReason::Displaced { by_same_claimant: bool }`。

### §2.3 Axon InvocationState(对照标杆)

9 态显式 enum、`#[repr(i32)]` pin 到 wire、terminal_states 常量、事件流驱动。
**这是全宇宙状态机的家规;§2.1/§2.2 与前端 store 应向它看齐**
(Voice/AbilityRegistration 经 F-013 复核已达标,从看齐批次移除)。
缺口:receipt.state 在 audit.rs 以字符串往返(from_wire_str)——链上对象应直接用 enum。

### §2.4 EasyNet 设备配对状态机(2026-06-11 第 7 轮深挖)【已核验】

**现状(半显式,实现质量为三仓隐式状态机之最):**
```
存储态(domain/constants.go:27-30,ent 字段验证器把门 devicepairing.go:57-63):
  pending ──validate(守卫 StatusEQ(pending) ∧ ExpiresAtGT(now), :133-136)──► validated
  pending/validated ──revoke(removeDevice:114;validate 冲突分支 :171-175
                       守卫 StatusEQ(validated))──► revoked
非存储态:expired = ExpiresAtGT 谓词计算(5min TTL;create 时清理过期 pending :69-74
  + 未过期配额;无 sweep、无写者时钟问题——达标设计)
```
- **达标点**:每个转移写点自带 WHERE 谓词(乐观并发、数据库层原子);schema 验证器拒绝
  非法字面量;token 单次使用 + SHA-256 哈希存储。
- **缺口**:① 转移表散落 4 个 logic 文件,无单一真源;② `PairingStatusExpired` 是死常量
  (全后端零使用,误导读者以为是存储态,F-035);③ 配对 4 态与 device_state.go 的 6 态
  展示空间(ONLINE/JOINING/SUSPECT/…)是两层状态,映射关系无文档。

**理想态:** domain 包内 `PairingTransition` 表(from → to → guard 谓词构造器),写点引用表;
删除死常量(或注释「视图态,勿写入」);device_state.go 头部落档 配对态 × presence 态 映射表。
与 §2.1 不同,这里不需要重写——补「单一真源 + 文档」即达教科书线。

---

## §3 理想修复计划(骨架,逐轮充实)

1. **状态机显式化批次**(§2.1→§2.2→前端 store;Voice/AbilityRegistration 经 F-013
   复核已达标,移除):每个先落转移表 + op_event,再迁移控制流。验收:状态覆盖测试 + 非法转移测试。
2. **协议边界归一**:EasyNet backend internal/axon fork(实测 7,765 行)→ Axon Go SDK
   (RFC-001 delta table P3)。替换批次【2026-06-11 第 2 轮定稿】:
   - **A 批 URA**:urns.go(557)+ uri_test.go——纯函数、SDK 面最稳,顺手消灭 URI/URN/URA 漂移(F-016);
   - **B 批 envelope/admission**:admission.go + interop 测试——收掉 SubjectEnforcement 全局开关(F-017);
   - **C 批 invoke 客户面**:invoke_client.go(267)+ advertise.go + ability_descriptor_reader.go——
     对齐 delta table 的两方法 Client 接口;
   - **D 批 federation**:federation_calls.go(649)+ namespace_resolve_answer.go(677)+ resolve_answer.go +
     delegation——最活跃面最后动,answer-sheet e2e 回归把门。
   每批纪律:冻结 fork 增量 → 按文件替换 → 删除 fork 文件;批间不混。
3. **热路径序列化整改**:SessionDispatch JSON-in-protobuf → 定型 proto 帧
   (先基准测量,见清单 F-004,不测不改)。
4. **god-file 拆分**:daemon_invocation_service.rs(13,142 行)按 RPC 面切分;
   kernel.go、InvokeAbilityDialog.tsx 同类。验收:文件 ≤ 千行级、单一职责、测试不动语义。
5. **工程卫生**:clippy 清零(分配责任批次)、全局单例收编为注入依赖;
   优雅停机 **✅ 已完成**(F-007 SessionShutdown,2026-06-11)。

### §3.6 修复排序终稿(2026-06-11 第 8 轮定稿;初版见第 4 轮,已按第 5–7 轮核验结果修订)

| 级 | 内容 | 条目 | 理由 |
|---|---|---|---|
| P0 | 协议形状类(越拖越贵) | F-004(帧格式,**先基准后改**)、F-015(fork 替换 A→D 批) | 迁移成本随部署面增长 |
| P1 | 事故级架构 | F-008/F-009(会话状态机显式化 + claimant 指纹)、F-003+F-031(退避两点,共享 backoff util)、F-024(转义定约) | 已付过事故利息的债 |
| P2 | 吞吐天花板(**测后改**) | F-011(LocalRuntime 锁拓扑) | 有意设计,需基准证伪后动 |
| P3 | 工程重量(可并行蚕食) | F-001/F-002/F-021/F-027/F-033(god-file 五处)、F-010(workspace 化)、F-018(Dialog 三分) | 机械工作,按文件分批 |
| P4 | 规范卫生(顺手批) | F-005(clippy 清零+CI 阀)/F-006/F-007/F-012/F-020/F-022/F-023+F-034(typed error 合批)/F-028/F-029/F-030/F-035;F-017 随 B 批 | 单条小,合批做 |

**执行纪律**(适用全部批次):每改必有验收(基准/测试/lint 阀);文实不符类(F-003)文档与实现同提交;
撤销条目(F-013/016/019/025/026/032)不复活——重提需新证据新编号。

**达标项校准记录**(审计公平性,不是债):
- Cli persistence:`atomic_write_with_permissions`(config.rs:68-154)教科书级——
  暂存文件 + chmod-先于-rename 关权限竞态 + sync_all 防掉电 + 文档完整;
- 前端:零跨 store 直接调用;Remote Desktop 资源清理全路径覆盖(closeAttach :826-869);
- terminal-store 状态转移全部走带守卫 action(F-032 的修复模板);
- backend:context.Background() 15 处均在合理的后台路径,请求链 ctx 传播无丢失。

---

### §2.5 Axon LocalRuntime 并发模型(2026-06-11 第 2 轮补)【已核验】

**现状:** 两把全局锁——`admission: std::sync::Mutex<AdmissionState>`(local_runtime.rs:148,
**验签在锁内**,全进程一次只验一个签名)+ `inner: tokio::Mutex<RuntimeInner>`(:139,
~25 个调用点抓取;能力执行不在锁内,簿记在)。
**理想态:** 读多写少分层——abilities 注册表走 ArcSwap/RwLock 读路径;nonce 滑窗、key resolver、
ledger sink 各自细粒度锁;验签移出互斥段(锁内取快照,锁外验);admission 成为纯函数管道
(输入:envelope + anchor 快照 + nonce 窗口句柄),与多语言 SDK 同步 setter 人体工学不冲突。

### §2.6 EAL Mission 运行生命周期(2026-06-11 第 3 轮)【关键点已核验】

**现状(五阶段管道,状态半隐式):**

```
mission run → [1] 编译 parse→plan→MissionIr(隐式 fallback 编译期拒绝 ✓)
            → [2] 持久化 run-dir + pid 文件
            → [3] 上下文 MissionContextGuard(env var EASYNET_MISSION_ID + thread-local)
            → [4] 执行 interpreter::execute_with_endpoint
                  · trace_id = mission_id 一次发放,随 RunContext 线程化
                  · 阶段内 rayon 并行;loop 串行,receipt_graph 全局累积
                  · 重试:指数退避 + 确定性 jitter(:2369-2384,已实装 ✓)
            → [5] meta.json { status: String } / trace.json(CappedTraceBuffer 500 条 ✓)
```
状态表示:`status: String` 五字面量;"running" = pid 文件存在性(磁盘即状态机,F-022)。

**值得记录的达标项**(校准用,不是债):单一入口不变量(run_mission_inproc,违者 release blocker)、
no-implicit-agent-fallback 三联回归测试、IrTarget 类型化(运行时无字符串 is_agent 判断)、
EalError 拒绝 From<String>、trace 缓冲有界。**EAL 面的工程纪律明显高于 transport 面平均水平。**

**理想态:**
```rust
enum MissionRunStatus { Running { heartbeat: Timestamp }, Ok, Partial, Error, Cancelled }
```
- 状态单点序列化(meta.json 原子写);liveness = 心跳时间戳,pid 文件废除;
- mission 上下文显式传参 / tokio task_local(env var 只留子进程边界,F-028);
- trace_id 单一编码点(envelope 层,F-025);
- loop 迭代作用域与 RFC「hermetic」对齐:per-iteration receipt scope + 显式导出(F-026);
- 转义契约入规范 + 共享 unescape helper + 端到端往返测试(F-024)。

## §4 覆盖全景打勾与收敛路径(2026-06-11 第 6 轮)

| 平面 | 体量审 | 质量/状态机审 |
|---|---|---|
| Cli transport / EAL / persistence | ✓ | ✓ 深(§2.1/2.2/2.6;persistence 正面) |
| Cli runtime/agents、facade/cli | ✓(F-027/F-033) | ✗ 质量层未抽查 |
| Cli drivers/gateway | ✓(无债) | —(体量小,不再深审) |
| Cli ura.rs | ✓(286 行) | ✓ 抽查干净(2 unwrap,第 7 轮) |
| Cli mcp_reflective 质量层 | ✓ | ✓ 抽查 → F-034(第 7 轮) |
| Cli core/ability_spec / ffi | ✓(2,174 / 2,407) | ✓ 抽查(均低于 god-file 线;ffi 的 result_large_err 已在 F-005;第 8 轮) |
| Axon local_runtime/admission/handle/voice/registry | ✓ | ✓(锁拓扑 §2.5;handle 标杆;voice/registry 正面) |
| Axon axiom 签名实现 | ✓ | ✓ 抽查干净(canonical_* 分型,全文件 5 unwrap,第 7 轮) |
| Axon dendrite_bridge | ✓(2,094) | ✓ 抽查(FFI 14 unsafe/2,094 行,密度合理,第 7 轮) |
| Axon ura-rs / proto 卫生 | ✓(1,560;16 proto) | ✓ 抽查(proto 全量 package axon.v1 版本化;第 8 轮) |
| Axon conformance | — | **声明审计边界**:只读仓,套件存在(capability 4,011 + voice 2,999 行),深审不在本次范围 |
| EasyNet handler/logic/middleware/daemon_grpc | ✓ | ✓ 抽查(reconnecting.go 正面) |
| EasyNet frontend stores | ✓ | ✓ 深 |
| EasyNet 配对状态机 | ✓ | ✓ 深(§2.4,第 7 轮;质量为隐式状态机之最) |
| EasyNet ent schema | ✓(全部 ≤136 行) | ✓ 抽查(索引/验证器纪律普遍,devicepairing 为范本;第 8 轮) |
| EasyNet Dialog 内部 | — | 并入 F-018 拆分批次,不另审(声明) |

**收敛路径(两轮)**:第 7 轮——配对状态机深挖(§2.4 展开,理想态落档)+
ura.rs/axiom/mcp_reflective/dendrite_bridge 质量抽查(各时间盒一眼,有债则录,无债则在本表打勾);
第 8 轮——完整性终审(本表无 ✗ 残留)+ §3 理想修复计划终稿复核 → 达成即跳出 loop。

**校准补录(第 6 轮)**:daemon_grpc/reconnecting.go 为正面设计(按需重拨 + 3s 最小窗 +
lastErr 错误隔离);ICE 轮询实有间隔休眠(:933),有界轮询达标。

## 迭代日志

- **2026-06-11 第 1 轮**:建档。Cli transport 平面状态机深挖(§2.1–2.2 已核验);
  Axon/EasyNet 全景索引(agent 盘点)。下一轮:核验 Axon LocalRuntime admission
  同步锁问题 + receipt 字符串状态;EasyNet backend fork 边界细读。
- **2026-06-11 第 2 轮**:§2.5 LocalRuntime 并发模型落档(已核验);§3.2 fork 替换批次 A→D 定稿;
  循环切为每 10 分钟 cron(任务 49a5230c)。下一轮:Cli runtime/agents 平面 + EAL mission 面首轮盘点,
  核验 F-013/F-014。
- **2026-06-11 第 3 轮**:§2.6 EAL mission 生命周期落档(关键点已核验,含达标项校准);
  F-013 撤销、F-014 修正、EAL jitter 假阳性拒收;F-021…F-028 入清单。
  下一轮:复核 F-025/F-026;Cli facade/persistence 面;EasyNet 前端 store 状态机 + backend handler 层;
  §3 完整排序初版。
- **2026-06-11 第 4 轮**:F-025/F-026 撤销(分层非重复;hermetic 已强制);persistence 面零新债
  (atomic_write 正面样本);F-029…F-032 入册;§3.6 修复排序初版 + 达标项校准节落档。
  下一轮:复核 F-029…F-032 与 F-018/F-019/F-020;扫剩余盲区(Cli drivers/gateway、
  Axon runtime-rs services/state、EasyNet daemon_grpc);开始收敛终审。
- **2026-06-11 第 5 轮**:复核批完成(F-029/030/031t/018d/020 坐实或修正;F-032 撤销);
  盲区清点完成——drivers/gateway(≤903 行)、daemon_grpc(≤594 行)、Axon state(≈千行级)
  均无 god-file 级新债;F-033(facade/cli 面)入册。达标项补两条:rdCreate 重入守卫注释、
  daemon_grpc 体量纪律。下一轮:reconnecting.go 退避判定、F-019/F-031i/F-016/F-017 收尾、
  全景表完整性打勾(收敛终审第一步)。
- **2026-06-11 第 6 轮**:待复核清零(F-016/F-019 撤销、F-031 收窄定稿、F-017 坐实);
  §4 全景打勾表 + 两轮收敛路径落档;校准补录 reconnecting.go 与 ICE 间隔轮询。
  下一轮:配对状态机深挖(§2.4 展开)+ 残余平面质量抽查(时间盒)。
- **2026-06-11 第 7 轮**:§2.4 配对状态机深挖落档(最后一个状态机 feature 闭环);
  ura.rs/axiom/dendrite_bridge 抽查干净打勾,F-034/F-035 入册。
  第 8 轮为预期终轮:收口残余 ✗ → 完整性终审 → 跳出 loop。
- **2026-06-11 第 8 轮(终轮)**:§4 表收口无 ✗(conformance/Dialog 内部声明边界);
  §3.6 排序终稿。**审计收敛:状态机全落档、清单 29 活跃条全核验、修复计划全排序。loop 退出。**
- **2026-06-12 边界镜头 + 第 9 轮(loop 重启)**:新维度「所有权边界」依 runtime-boundary/
  ura-discipline 两 skill 铺满:F-040(backend 包 daemon-internal ability + 手抄帧)、
  F-004 升格载体债、F-041(守卫不对称)、F-042(receipt URA 第 4 野生形状/AXIOM 22.2 反例);
  正面:Cli ura 门面+9 守卫、FFI 七元组必填、Frontend 构造集中化。
  下一轮:T2.0 caller 盘点(审计执行)、ability.json/control.sock 所有权、URAChip 渲染纪律。
- **2026-06-12 第 10 轮**:T2.0 首批——backend JSON 控制路径**已退役**(好消息,余 2 注释
  F-044);Cli control.sock 自标 Legacy,8 内部文件待 op 级分类(重点 interpreter.rs);
  F-043(URAChip 失守 6+)。下一轮:T2.0 收尾 + ability.json 所有权 + 收敛重估。
- **2026-06-12 第 11 轮(第二次收敛)**:T2.0 关闭全绿(interpreter 澄清;control.sock
  降级不变量已书面存在;载体债收窄至 SessionDispatch 唯一面);ability manifest 合规正面。
  **五维(质量/重量/规范/状态机/边界)全扫毕,无未审残留,loop 退出。**
- **2026-06-12 第 12 轮(增量模式)**:§1 形状索引注意——Axon 已批准 device-owned agent/ability
  双新形状(64190a6b/35efe641,`device.` 保留 owner token,`device_agent_ids()` 分立访问器),
  本文件 §1 与 ura-discipline skill 的形状表**均待补**(skill 更新需 CTO 授权)。
  F-047 入册(Cli 管理面 8 消费点对新变体隐式 bail)。CI 棘轮 + tests 工作流落地(正面)。
- **第 13-15 轮(实际日期 2026-06-11;「06-12」系跨夜误标,勘误见清单文件)**:
  F-047 判定 8/8、None 嫌疑清除、e2e 通过、skill 形状表经授权更新;
  三仓零新提交 + 连续两轮零新债 → **第三次收敛,loop 退出**。
  增量审计的再启动条件:新提交批量落地后由 CTO 重启 loop,或并入常规 review 流程。
- **2026-06-12 同步会话**:对照磁盘勘正本文件——§1 四处陈旧论断回写(Voice/AbilityRegistration
  随 F-013 撤销、invoke_client.go 267/kernel.go 938 勘误数字、Cli 两行「未盘点」已盘);
  §1 补双新形状注记 + 联邦目录 presence 平面行(F-049);§2.1 标注 F-003 已修;
  §2.3/§3 批次随 F-013 撤销与 F-007 完成收口。spec 升 **v2**(44 活跃条全映射 +
  §0 立意终局 + §6 防丢核对表),执行清单以 spec v2 为准,本文件继续承担「现状/理想态」。
