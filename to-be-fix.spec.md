# to-be-fix.spec.md — 跨仓技术债修复总规格(v2,2026-06-12)

> 三件套之三:`to-be-fix.md`(债清单,50 编号 = 44 活跃 + 6 撤销,全部已核验)·
> `to-be-fix.plan.md`(状态机现状/理想态 + 排序)· 本文件(**修复执行总规格**)。
> v2 相对 v1 的变化:① 覆盖补全——v1 漏编的 F-041…F-046、F-049/F-050 残留全部入批;
> ② 基线刷新(F-049 主修 fc8df1b、F-050 主修 c03df45 + backend 半边 0716861、T0.5d=52bb764a);
> ③ 新增 §0 立意与终局、§6 条目→TODO 完整映射表(防丢核对面)。
> 权威依据:easynet-runtime-boundary skill(边界裁决)· easynet-ura-discipline skill(本体形状)·
> AXIOM.tex · RFC-001 v4.1.x / RFC-005 §3.1.2 / RFC-006 · DEC-F048 · DEC-009。

---

## §0 立意与终局(全部修复的北极星)

### 0.1 本体不变量(任何批次不得违反)

```text
Agent owns Ability.            Invocation calls Ability.
Axon governs Invocation.       CLI daemon hosts EasyNet device/Hub abilities.
Skill 支撑 Ability 实现(资源面,不可寻址)。  EasyNet backend/browser 是产品面。
```

1. **七元组公理**:每次 Agent 通信 =
   `invoke(caller, callee, ability, subject, nonce, causal_context, args) → receipt`。
   公开边界上元组不完整 = receipt 链正确性 bug,不是 API 工学问题。
2. **协议真源唯一**:协议形状(envelope/admission/URA/receipt/wire)只在 Axon。
   Cli/backend/Frontend 是消费者;任何手抄/fork/字符串拼接 = Rule 1 拒绝类。
3. **URA 只由 Axon 提供**:不许应用层发明形状;每个 URA 必须能 round-trip
   `parse_ura`/`parseURA`;无 builder 的形状 = RFC 缺口,**flag 而非 extrapolate**。
4. **device 是 sponsor 不是 principal**(DEC-F048,RFC-005 §3.1.2 normative):
   device → sponsors System Agent → agent owns ability → receipt 记 agent →
   问责经配对 principal 解析。hosted user agent ≠ device-owned agent;
   System Agent 不迁移、不承载用户身份、不受 delegated authority。
5. **Invocation 是唯一 runtime-addressable 载体**:Mission/EAL 是脚本(实现策略),
   步骤产生子 Invocation;重试 = 重新 invoke,**没有步级重试**(不重造 Temporal)。
   daemon 控制帧不得成为 Invocation 构造/签名/receipt 绑定的第二真源。
6. **状态机家规**:Axon InvocationState(9 态显式 enum,`#[repr(i32)]` pin wire,
   terminal_states 常量)是全宇宙标杆;新状态面一律显式 enum + 转移单点 + op_event。
7. **目录读 = 快照读**:目录类接口(meta.list_abilities/discover/skill.list)读进程内
   快照,写路径主动 publish 刷新(c03df45 模式);禁止每查询全量重建/重哈希/磁盘重读。

### 0.2 终局架构(全部批次落完后的世界)

- **单 invocation 载体**:gRPC bidi dispatch 帧承载 canonical Invocation/proto;
  SessionDispatch JSON 帧消亡;JSON 仅剩 status/boot/lifecycle/diagnostics。
  跨仓 fixture(F-038 类)漂移在结构上不再可能——因为只有一个形状源。
- **backend 零协议 fork**:`internal/axon/` 7,765 行被 Axon Go SDK 替换殆尽;
  backend 向 daemon 提交完整七元组 Invocation,daemon 拥有 callee 本地性解析/转发;
  `<self>.invoke_remote` 包装与手抄 struct 退役。
- **会话/槽位状态机显式化**:DeviceSessionState + CloseClass 一等公民,转移单点发
  op_event;hub 槽位带 claimant 指纹,乒乓类事故在源头被识别为 claimant_conflict。
- **三仓守卫对称**:URA 裸构造防回潮阀 Cli 9 脚本 + backend 1 + Frontend 1;
  URAChip 渲染纪律有 eslint 阀;conformance baseline 纳入 wire 改动 DoD。
- **Cli workspace 化**:persistence/transport/runtime/facade 分 crate;
  无 >3k 行 god-file;clippy `-D warnings` 全量进 CI(棘轮已锁的 5 类之外全清)。
- **typed error 全面化**:EalError 判别升进类型,错误嗅探(`contains("daemon not
  running")`)灭绝;mcp_reflective 等公开面零 `Result<_, String>`。
- **本体缺口闭合**:receipt body URA 正式 builder(RFC-007/008)、device-agent 的
  resource_dot owner 形状、签名密钥访问契约(DEC-F046)全部有 ratified 形状。

### 0.3 工业教科书 Definition of Done(逐仓绿色定义)

| 仓 | 绿色定义 |
|---|---|
| EasyNet-Cli | `cargo test --lib --features axon-pb` + 默认特性全过;全测试 target 编译;clippy 双特性 0 warning(`-D warnings` 入 CI);9+2 守卫脚本过;baseline-lock 绿;help-drift/边界脚本过 |
| EasyNet-Axon | sdk + core 测试全过;conformance 套件绿;proto 全版本化;RFC 正文与语法同提交对齐 |
| EasyNet BE | go test ./...;handler 禁 import ent lint;e2e answer-sheet/配对/invoke 流全绿;零 fork 引用 |
| EasyNet FE | 组件测试过;URAChip eslint 阀;parseURA 与 Axon 语法同步(镜像纪律) |
| 跨仓 | Federation-MVP fixture 绿;「改 wire 必重生基线必更新计数」写入 conventions 并被 CI 执行 |

---

## §1 基线(已完成,2026-06-12 对照磁盘核验)

| 项 | 完成证据 |
|---|---|
| F-003 退避 full-jitter | session_initiator(2026-06-11);文实同提交 |
| F-007 SessionShutdown 优雅停机 | boot.rs / easynet-daemon.rs(2026-06-11) |
| F-039 retired 顶层别名删除 | a1a4aea + f1c1f29 |
| F-049 **主修**:会话级 federation.heartbeat | **fc8df1b**(5s 心跳循环,AbortOnDrop,21/21 定向测试;残留 3 项见 T1.5) |
| F-050 **主修**:builtin 索引 + default_state 缓存 | c03df45(3.41s → 0.15s);backend fan-out 半边 **0716861**(并发 + 短 TTL 缓存);残留见 T5.12 |
| F-005 部分:clippy 20 → **8**(实跑核验:6 result_large_err + 2 too_many_args) | `#![deny]` 棘轮锁 5 类(lib.rs:49);clippy-ratchet.yml 咬合 CI |
| T2.0 载体归一 caller 盘点 | 第 10-11 轮;载体债收窄到 SessionDispatch 帧唯一面 |
| T0.5d RFC-005 §3.1.2 入库 | Axon **52bb764a**(DEC-F048 C 案 normative 文本) |
| T5.7 F-030 recover 全覆盖 | **4960a99**(8 站点按形状分置 GoSafe / 每 tick RunSafe / fanout 捕获重抛 + 注入测试;%w 复核 0 残留——50/99 系审计期分母误计) |
| §0.1-5 执行:RFC-001 mapping 步级动词删除 | Axon **db296efb**(retry_step/skip_step/rollback{to_step} → deleted-not-restated + normative note,引 CONCEPT_MODEL.md:59-68;common.rs 转移矩阵注释同步) |
| T0.5a delegation 本体闸 | Axon **785765ca**(Step 0 拒 device-owned caller/subject,AXON_DELEGATION_DEVICE_AGENT_FORBIDDEN,引 §3.1.2;587/587 绿) |
| T0.5b 注册闸 | Cli **de04b7b**(registrar 拒 `device.` owner token + op_event + 双形状测试) |
| 撤销 6 条(F-013/016/019/025/026/032) | 不在本 spec 范围;重提需新证据新编号 |

---

## §2 架构问题定性(九类 + 模式债)

### A1. 会话平面:隐式状态机 + 顶替无源头防御(F-008 / F-009)
**本质**:`<self>.session` 生命周期无类型表示(状态 = dial 函数控制流位置);hub 槽位
只认 URA 不认申领者,「同设备换代」与「双设备打架」不可区分。
**已付利息**:2026-06-11 的 5428 次乒乓重连事故;F-049 心跳断裂的诊断过程再次付费
(阶段不可观测,只能 grep 日志 kind)。device 侧放大器已修(b2ba441 + F-003),根架构未动。
**终态**(plan §2.1/2.2):`DeviceSessionState` + `CloseClass` 一等化、转移 op_event;
frame0 携 boot-nonce 指纹 → `claimant_conflict`;admission receipt 携契约版本号。
→ T1.1 / T1.2

### A2. 第二 invocation 载体:SessionDispatch JSON 帧(F-004 + F-038 + F-040 + F-044)
**本质**:会话业务帧 = protobuf BinaryChunk 包 JSON,携带 ability+args+origin claim+result
——Axon Invocation 之外的第二载体,形状由 Cli 拥有。并发症已兑现两次:backend 逐字节
手抄 struct(F-040,「no translation layer」是注释原话);跨仓 fixture 抓到 ability_ura
漂移(F-038)。性能(base64 +33%、每帧双解析)只是表症。
**裁决**(boundary skill):「daemon 控制帧不得成为第二真源」「JSON 降级到
status/boot/lifecycle/diagnostics」。T2.0 盘点已确认范围收窄到此唯一面。
**终态**:dispatch 帧承载 canonical Invocation/proto;backend 提交完整七元组,F-040/F-044
随之消亡。boundary skill 迁移序 1-3 步已满足(hub/device 已统一 daemon、盘点完成),
本批是第 4 步本体。→ T2.1 / T2.1b

### A3. Axon LocalRuntime 锁拓扑(F-011 + 附注)
**本质**:`admission: std::sync::Mutex` 横跨 Ed25519 验签(全进程同时只验一个签名,
吞吐天花板 = 单核验签率);`inner: tokio::Mutex` 25+ 调用点簿记串行;新成员
`ability_names_with_prefix`(:1063-1073)在全局锁内 O(registry) 扫描+排序+克隆。
**缓和**:有意设计,当前规模未测出瓶颈。**终态**:注册表 ArcSwap/RwLock、验签出锁
(快照后验)、nonce/resolver/ledger 分锁、prefix 扫描出锁。**基准先行**。→ T3.1

### A4. backend 协议真源二元化(F-015 / F-017 / F-040 / F-041)
**本质**:`internal/axon/` 7,765 行复刻 envelope/admission/URA(F-015);invoke_remote.go
手抄帧(F-040,归 A2 收口);SubjectEnforcement 全局 atomic + init() 读 env(F-017);
守卫不对称——Cli 9 个防裸构造脚本,backend/Frontend 零阀(F-041,当前无活跃违例但
合规靠约定)。fork 与漂移无编译期信号,事故链兑现过一次(unknown field → resolve 拒
→ 静默 fallthrough → 设备全量误判 REMOVED)。
**对照面**:Cli 是合规范本(src/ura.rs 纯门面 + 守卫、AdmissionFacade 委托 SDK、FFI
七元组必填)。**终态**:A→D 四批替换,每批冻结→替换→删除;守卫脚本随 A 批移植两仓。
→ T2.2a–d

### A5. 巨石与 god-file(F-001 / F-002 / F-006 / F-010 / F-021 / F-027 / F-033)
**本质**:单 crate 三 crate-type(链接 4–8GB,历史 OOM SIGKILL,共享 checkout 双引擎
互锁);daemon_invocation_service.rs 13,142 / interpreter.rs 3,976 / agent.rs 3,556 /
agents/mod.rs 2,918(session_initiator 3,238 / boot 2,958,2026-06-12 刷新)。文件即
模块边界:13k 行 = 评审不可能完整 + 永久冲突源 + 全量重链。F-006(OnceLock 单例)
评估为中型,归并 transport 拆分批(注:F-049 心跳修复新增一个 `global()` 消费点
session_initiator::send_federation_heartbeat,注入化时一并收,共 6 点)。
**终态**:workspace 化 + 五处 god-file 拆分。→ T4.1–T4.6

### A6. mission 运行态与上下文传播(F-022 / F-028)
**本质**:状态 = String 五字面量,"running" = pid 文件存在性(磁盘即状态机,异常退出
= 永久假 running);上下文经 `EASYNET_MISSION_ID` env var + thread-local(5 文件),
async/rayon 混编脆弱。已评估非小修(serde wire 兼容 + liveness + 跨仓消费面)。
**终态**(plan §2.6):`enum MissionRunStatus { Running{heartbeat}, … }` 单点序列化;
显式传参/task_local。**守护性约束**:不引入步级重试/断点续跑(§0.1-5)。→ T5.3 / T5.4

### A7. 跨仓一致性基础设施失修(F-036 / F-037 / F-038)
**本质**:契约执行回路断了——conformance baseline 落后 29 条(888 vs 917,实跑复核
仍 FAIL);Fed-MVP session_dispatch 基线缺 ability_ura(fixture 正确报警,实跑复核仍缺);
pages u14 他人重构后破损未同步(实跑复核仍 14/15 FAILED)。共性:守护机制存在,
没人按节拍喂。**终态**:「改 wire 必重生基线必更新计数」入 DoD。→ T0.1–T0.3 + 文档批

### A8. URA 本体闭合(F-042 / F-045 / F-046 / F-047;DEC-F048 执行闸)
**本质**:本体演进快于收口——① receipt body URA 第 4 野生形状(非法顶层 role
`invocation/` 钉进测试夹具,raw string 不 round-trip,AXIOM 22.2 反例)且无正式 builder
(F-042);② device-owned agent 语法已批,Cli 管理面 8 消费点行为未声明(F-047,判定表
8/8 已落);③ device-agent 的 resource_dot owner 形状未定义(F-047 两点 + skill 缺口节
同源);④ FFI 缺 per-call timeout 透传(F-045);⑤ SDK 取签面无契约(F-046,keyring 归
daemon)。DEC-F048 已决但两道执行闸(delegation 拒绝、注册断言)未实施。
**终态**:判定表施工完毕 + 执行闸落地 + RFC-007/008 收口 receipt/owner 形状 +
DEC-F046 拍板签名路线。→ T0.5a–c / T0.6 / T2.3 / T5.11 / T5.1(F-045 搭车)

### A9. presence 目录平面与目录读纪律残留(F-049 残留 / F-050 残留)
**本质**:F-049 主修后,双平面架构残留三项:legacy `_EASYNET_HB_ENDPOINT` sidecar 是
对 TLS hub 必死的死代码;backend device_state.go 把「目录无条目」default 映射 REMOVED
(墓碑语义误导);hub 无 bidi 活跃兜底刷新。F-050 主修后,discover/skill.list 仍直读
磁盘(`load_default()` 散布调用),违反「目录读 = 快照读」。
**终态**:sidecar 退役、REMOVED 只留给操作员撤销、目录读全部走快照。→ T1.5 / T5.12

### 模式债(跨条目共性)
| 模式 | 条目 | 收口 |
|---|---|---|
| 退避纪律 | F-003 ✅ / F-031(terminal-store 硬停 3 次) | T1.3 共享 backoff util |
| typed error | F-023(EalError 嗅探)/ F-034(3× pub Result<_,String>) | T5.2 合批 |
| 字符串冒充状态 | F-012(receipt.state)/ F-022(mission status) | T5.5 / T5.3 |
| Go 工程卫生 | F-029(handler DB 8 处)/ F-030(recover 缺口 + %w 50/99)/ F-035(死常量)/ F-020(链验证边界未文档) | T5.6–T5.9 |
| 约定已立无阀拦截 | F-041(URA 守卫)/ F-043(URAChip)/ F-036(baseline) | T2.2a 搭车 / T4.7 搭车 / 文档批 DoD |

---

## §3 执行清单(批次化;每条:落点 → 做法 → 验收)

> 标注:〔仓〕规模(S<半天 / M≈1-2天 / L≥3天)。→ 表依赖。
> 通用前置:开工 `git status` + mtime 检查(共享 checkout);pathspec 提交;无 Co-Authored-By。

### Phase 0 — 决策与基线(其余阶段的前置)

| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T0.1 | **🟡 归属表已产出(e9d6d92,2026-06-12),仅余 CTO 签字**:漂移现值 927(快照法对 757a917 / HEAD 4e1536b 逐 pattern 差分,docs/rfc/AXON-RFC-001-baseline-delta-attribution-2026-06-12.md)——+39 = plugin_host 新 MCP 面 +27(**唯一真裁决点**:豁免域 vs 随 P4.8d 清)+ 必要修复文案 +9 + 拆分重分布净 ±4/−8 + F07 AgentType +16(P4.4 决议内)− 已修违规 1(基线应随降)。推荐基线重钉 927 带归属注记。裁决卡 #4 已刷新 | Cli | 签字即闭 | check-rfc-001-baseline-lock.sh 绿,每条增量有归属记录(表 ✓) |
| T0.2 | **✅ 已完成(155b6b4,2026-06-12)** F-037:判定为测试期望过时——u14 写于 owner 限定名时代(9d4fe17),779d295 统一管理面为「裸名 + OwnerKind 旁表」约定(同 admin.status/a2a.*),期望未随迁。测试改裸名 | Cli | 完成 | pages_unit 15/15 ✓ |
| T0.3 | **✅ 已完成(Fed-MVP 705a0e3,2026-06-12)** F-038:确认 `ability` → `ability_ura` 是有意 wire 演进(28d3822,struct 文档自述 canonical Ability URA);基线单行外科替换,URA 用 ratified device-owned 形状(镜像 in-crate 夹具 dev-X.fs.read)。T2.1 落地后此基线随 canonical 帧再重生 | Fed-MVP | 完成 | session_dispatch_fixture 2/2 ✓ |
| T0.4 | **🟡 ② 已完成(Axon 71ed82c6,2026-06-12;验收复核过)**:admission 锁拓扑基准三场景——ed25519_verify_floor / full/single / verify_in_lock vs verify_outside_lock(threads=N),F-011 坐实(锁内验签=单核验签率天花板),**T3.1 gate 满足**;数字 + 机器基线落 sdk/rust/benches/BASELINE.md(2026-06-12 节,~1.76M admissions/s 单线程上限等)。**① 会话帧路径仍开放**(落点 transport,被 T1.1 在制占用) | Cli+Axon | ② 完成 | ② ✓;① 待 transport 释放 |
| T0.6 | **DEC-F046 签名密钥访问契约**:两路线拍板——(a) **签名服务面**:facade 交 canonical bytes,daemon 持钥签名,私钥不出 daemon(审计倾向,最小暴露);(b) 导出契约:keyring 派生只读种子给本机 SDK。boundary 裁决已否决 facade 直读 keyring.enc。过渡期「调用方自备种子」(caller_signature 透传)今天可用。**决议必须回答签名预言机三问**(路线 (a) 即「替任意本地调用方签名的 oracle」,等于本机进程可借设备身份):① 谁可请求签名(本机界?配对身份?进程鉴别?);② 作用域(per-ability/per-subject 限定,还是任意 canonical bytes?);③ 每次签名发放是否记审计日志(receipt 化?) | DEC | S(决策) | DEC-F046 入 Develop-Plan/Cooperation/decisions/,**含三问答案**;实施 = T2.3 |
| T0.7 | **✅ 第 16–18 轮队列全清(2026-06-12)**:第 16 轮五面零新债(07860b3);第 17 轮清 D7/T5.4/chat-Cli 半(一新债 F-051 同轮修复);第 18 轮清 step-1 帧形状 + chat/lifelong 收尾——五面零新债(28245ab4/90841fed+f9d7419b/70beb54c/c54c59c/cebccda5,全部磁盘亲核,详见 to-be-fix.md 第 18 轮),一新债 **F-052**(lifelong 前端半边零测试,挂前端线搭车)。**常设纪律**:增量审计随新提交波重启,审计欠账即新债 | 审计 | 第 18 轮 ✓ | 队列清空;F-052 已映射 §6 |

### Phase 0.5 — DEC-F048 本体执行闸(全部完成 ✅:T0.5a/b/c/d)

| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T0.5a | **✅ 已完成(Axon 785765ca,2026-06-12)** delegation 闸:run_delegation_gate Step 0(任何密码学工作之前)拒绝 caller_ura/subject_ura 为 device-owned agent 的 proof,错误码 AXON_DELEGATION_DEVICE_AGENT_FORBIDDEN,文案引 §3.1.2 双规范;检测走 ParsedURA::device_agent_ids() 非字符串嗅探;issuer 侧留给 Step-2 trust_role 硬化批(闸内注释已标) | Axon | 完成 | 两条拒绝路径单测 + 587/587 全绿 ✓ |
| T0.5b | **✅ 已完成(Cli de04b7b,2026-06-12)** 注册闸:hot_agent_registrar::register_agent 入口 name_claims_reserved_device_owner 拒绝 + rejected_reserved_owner outcome + op_event,文案引 §3.1.2;双形状测试(device.x/裸 device 拒,常规名过) | Cli | 完成 | ✓ |
| T0.5c | **✅ 已完成(Cli 4487344,2026-06-12;验收复核通过)** F-047 八点闭合:判定表 v2 即 commit body——支持×2(ura.rs 双点,双访问器 fallback);重判 support→显式拒绝×3(agent.stop:T0.5b 使 device-owned 不可注册故生命周期引用是类别错误;**mcp_reflective:sponsor 复核结论 = MCP 描述符承载用户配置工具,归 User Agent**;ability_publish:发布是 hosted 用户面);判 None 不发明×2(owner_projection/invocation_history 的 resource_dot owner 缺口挂 RFC-007/008);现状即正确补注×1(bootstrap None→false)。9 处文案引 §3.1.2;双形状测试随附。复跑并入 transport 拆分后全量验证 | Cli | 完成 | 验收复核 ✓(循环会话) |

### Phase 1 — 事故级架构(P1)

| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T1.1 | **F-008 会话状态机显式化**:`DeviceSessionState{Idle, Dialing{attempt}, Preluding(PreludeStep), Live{since,frames}, Backoff{attempt,until,last_close}}` + `CloseClass{Healthy, DisplacedSuspect, NoAdmissionReceipt, ContractSkew, Errored}`(plan §2.1);转移函数集中一处,每转移发 `session_state_transition{from,to,reason}` op_event;frame0 admission receipt 携契约版本号 + hub session_id + displaced_prior。**注意**:F-049 心跳循环(fc8df1b)随会话生死,状态机化时心跳归属 Live 态的伴生任务,显式建模 | Cli | L | 状态覆盖测试 + 非法转移测试;现有 354 transport 测试不回归;op_event 字段齐 |
| T1.2 | **F-009 claimant 指纹**:frame0 携 boot nonce;PresenceSlot 存指纹;异指纹 < N ms 交替 → `claimant_conflict` op_event + 快速 re-admit 拒绝;`OfflineReason::Displaced{by_same_claimant}` | Cli | M → T1.1 | 双申领者集成测试:冲突检出且事件可见;同设备重连不受影响 |
| T1.3 | **✅ 已完成(EasyNet c54c59c,2026-06-12)** F-031:src/lib/backoff.ts 共享 util(full-jitter,250ms×2^n 封顶 30s,对齐 Cli F-003;random 可注入);terminal-store 永久硬停换为时间门控窗口(成功清零、窗口外放行、文案"next attempt allowed in ~Ns");曲线/jitter 边界单测 + store 测试改钉新语义(注入 state 保证确定性);36/36 + tsc 干净 | EasyNet FE | 完成 | ✓ |
| T1.4 | **✅ 已完成(Cli f206026,2026-06-12;axon-pb 全量 3183/3183 验证过)** F-024:契约单一剥离点落 src/eal/string_escape.rs(规范注释 + unescape_string_literal——只剥作者层 `\"`/`\\`,其余转义原样属载荷);mod.rs 管线文档载规范行;`\"…\"` 端到端往返测试钉住。**刻意不在 planner 自动应用**(存量自行 unescape 的 wrapper 会双重解码)。wrapper 迁移半边复核为仓内空集(审计"两个 wrapper"系盘点级;"新 wrapper 有据可依"由规范行+helper 满足) | Cli | 完成 | ✓ |
| T1.5 | **F-049 残留:①② ✅ 已完成(2026-06-12)**。① sidecar 退役(Cli fbbad85,−978 行:heartbeat.rs + 隐藏命令 + boot 门控全删,边界契约测试换 Auth 锚点;grep 三关键词零命中,bins 双特性编译,facade::cli 240/240,script_checks 27/27);② device-state 映射(EasyNet e1c332b):`suspended`(15s 清扫降级,事故本体)→ SUSPECT,`revoked` 显式 → REMOVED,未知词汇/无条目 → UNKNOWN——REMOVED 只留操作员撤销,UNKNOWN 留在 ratified 7 态词汇内不发明 OFFLINE;③(可选,M)hub bidi 活跃兜底刷新目录——设计随 T1.1 一并定,**仍开放** | Cli+BE | ①②完成(+M) | ③ 若做:断 device 心跳仅留 bidi,目录不降级 |

### Phase 2 — 协议形状(P0 长线;T2.0 ✅)

> F-015 各批(T2.2a–d)共同前置:**Go SDK parity 盘点**——fork 面 API ↔ SDK 现有面
> 逐函数映射(Go SDK「已导入基本未用」,parity 从未被验证);缺口**先补 SDK**(Axon 仓,
> 带测试)再替换。**禁止半截替换**——一半 SDK 一半 fork 比纯 fork 更糟(两个真源变三个)。

| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T2.1-pre | **dispatch 帧 mini-RFC**(设计件,T2.1 施工硬前置):① proto schema——SessionDispatch::{Request,Result} 字段 ↔ canonical Invocation 七元组逐字段映射,origin-caller claim 与 receipt 回程归位;② 版本协商——契约版本号载于 frame0 admission receipt,**与 T1.1 同一设计(frame0 只动一次)**;③ JSON 残留面清单(status/boot/lifecycle/diagnostics 哪些帧留下);④ 滚动升级序(双读单写一版,新旧 hub×device 四象限行为表) | Axon+Cli | M → T2.0✅ | mini-RFC 经 CTO 评审;字段映射表零「待定」格;四象限表完整 |
| T2.1 | **F-004 载体归一**:按 T2.1-pre 设计施工(proto 定义落 Axon,Cli 消费——boundary Rule 1);JSON 降级诊断面。性能基准(T0.4)做对照而非 gate——边界违例本身已构成改造理由 | Cli(+Axon proto) | L → T2.1-pre, T0.4 | 基准对比落档;新旧帧互通一个版本;352+ transport 测试迁移;Fed-MVP 基线随新帧重生(T0.3 终态);「单一形状源」= F-038 类漂移结构性不可能 |
| T2.1b | **F-040 收口(🟢 execution-ready)**:backend 改为向 daemon Invocation 面提交完整七元组;退役 `<self>.invoke_remote` 包装与手抄 struct。**施工准备件已落**(docs/t2.1b-backend-cutover-prep-2026-06-12.md,2026-06-12):消费面 9+6 文件盘点齐——真切口仅 daemon_grpc 一层(handler 经 routing client 隔离零感知);降级层 remote_routing 路由矩阵随 callee 本地性解析归 daemon 而退役;五步序列单批可完。**搭车 F-044 勘正**:陈旧 cliipc 注释实为 **3 处**(client.go:12 / servicecontext.go:179 / mapping.go:348 新发现) | EasyNet BE | M → T2.1 | invoke_remote.go(546)删除;contract test 改打 Invocation 面;grep cliipc 零命中(3 处) |
| T2.2a | **✅ 已完成(2026-06-12;「验收收尾」改判被兑现)**:urns.go 实已 SDK 门面(48 委托/0 拼接);9/9 SDK-covered 验收抽查通过(盘点 §三·六,Cli 7665536),密码学旗标双双排除。**F-041 搭车落地(EasyNet b8b2114)**:backend+Frontend 双阀,三态验证,入 conformance CI | EasyNet BE+FE | 完成 | ✓ 阀注入即红验证过 |
| T2.2b | **✅ 已完成(2026-06-12)**:admission fork 验明薄委托;**F-017 收口(EasyNet 8bc917d)**:SubjectGate 注入对象替双全局+init() 暗读——boot 显式读 env、SetEnforcement 运行时翻转、sink 构造期入结构化日志;5 生产点+streambridge 必填字段+urautil 显式收参;测试自建 gate 零复位+翻转钉死;32/32 | EasyNet BE | 完成 | ✓ 运行时可重配 ✓ 测试无复位 |
| T2.2c | **🟡 F-015 C 批,2/3 收口(2026-06-12)**:① descriptor 读面上移——SDK 读写同档互逆(Axon b58d124c + EasyNet 2edbc56,−71 行 wire 知识);② OriginCaller 钉 SDK 类型——NewOriginCallerClaim 单一编码+校验边界(Axon fe6060e7),backend 三表示收一、legacy metadata 双写退役(EasyNet 7a0feed,−80 行;Cli origin_caller.rs from_metadata 回退面成死代码,transport 释放后删)。**余**:answer codec 批(盘点 §三-2 重定义待 CTO 签;Rust 产者半边待 pbjson 基建批) | EasyNet BE | M → 盘点签字 | descriptor 读写同源 ✓;claim 单编码边界 ✓;余项随 §三-2 裁决 |
| T2.2d | **F-015 D 批 federation**:federation_calls.go(649)+ namespace_resolve_answer.go(677)+ resolve_answer + delegation → SDK。最活跃面最后动 | EasyNet BE | L → T2.2c | answer-sheet 跨域 e2e 绿;internal/axon 仅余薄 glue 或删除 |
| T2.3 | **F-046 实施**(按 T0.6 决议;若 (a) 路线):daemon 暴露签名服务面(ability 或 FFI 入口)——输入 canonical bytes,输出签名,私钥不出 daemon;FFI `caller_signature` 透传保留为「自备种子」模式;facade SDK `sign=True` 接通 | Cli | M → T0.6 | 签名面单测(canonical bytes → 可验签名);keyring.enc 零 facade 读取(守卫);Python/JS facade 端到端签名调用过 |

### Phase 3 — 吞吐与并发(P2;gate = 基准)

| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T3.1 | **✅ 全部完成(⑤=Axon 70beb54c,2026-06-12)** F-011 锁拓扑——①验签出锁 ✅(随 71ed82c6 bench 批,admit 注释载 ~4.5× 收益);②prefix 扫描出锁 ✅(锁内快照、锁外排序,注释引 T3.1 纪律);③ledger sink 分锁 ✅(设计期即独立 Mutex);④nonce/resolver 分锁 ✅(**fa08831f**,2026-06-12:AdmissionState 解体——nonce Mutex 每次准入仅取一次,resolver 改 RwLock 共享读;重放窗口语义不变,sdk 439/439 绿,bench quick 与基线同量级;零新依赖,单读多槽位 RwLock 即足)。⑤注册表读路径 ✅(70beb54c):abilities 表移出 inner 至独立 sync RwLock,22 触点逐点审计(全部单表原子对,broadcast 锁外,守卫不跨 await);前后基准落档——8 线程 +194%(9.35→27.8M/s 平台期消灭)、单线程 +20%、64 任务超订阅 −37% 如实记录(纯查找乒乓 profile,真实派发路径不具备;ArcSwap 留作饱和读升级路径);440/440 绿 | Axon | ⑤剩余,L | ①-④ ✓;⑤ 待施工 |

### Phase 4 — 重量与结构(P3;可与 Phase 2/3 并行蚕食)

| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T4.1 | **F-010 workspace 化**:persistence 先切验证收益,再 transport/runtime/facade。**前置设计件(M)**:crate 依赖图先画后切——op_event! 宏、config 类型、error 类型等横切面的归属 crate 先定,否则切到一半发现环依赖回退重来 | Cli | L(+M 设计) | 依赖图经评审无环;增量链接时间/内存数字落档;CI 双特性矩阵不变 |
| T4.2 | **F-001 拆分**:daemon_invocation_service.rs(13,142)按 unary/stream/bidi 三 RPC 面 + 路由 + accept/drain 拆 4–6 模块;6,000 行同文件测试随实现走 | Cli | L | 每文件 ≤~2,000 行;测试零语义变化;move-only 提交与逻辑提交分离(git blame 可追) |
| T4.3 | **F-002 + F-006 同批**:boot/dial/supervisor/warmup 分模块;spawn_session_supervisor 8 参收拢配置结构体;HubPublishedAbilityStore 注入化——store 随 boot 构造存入 service(照 AdvertisedAgentStore),**6 个** global() 消费点改签名(advertise.rs:503、meta_ability.rs:371/:769、session_initiator.rs:1485 + fc8df1b 心跳点);local_session_dispatcher try_dispatch_via_axon 借用 vs owned 边界一并定(7 参全借用 + tokio::spawn 跨 async,见清单 F-005 评估) | Cli | L → T4.2 | too_many_arguments 清零;global() 生产路径零调用(deprecated → 删除) |
| T4.4 | **F-021 拆分**:interpreter.rs(3,976)→ 调度/派发/重试/trace/receipt 5–7 模块。**守护性约束**:拆分不改执行语义——mission 仍是脚本,无步级重试(§0.1-5) | Cli | M | 91 EAL 测试零回归 |
| T4.5 | **🟡 mod.rs 半边 ✅(524818d,2026-06-12)**:2,918 → mod.rs 260(装配+再导出,公开路径不变)+ pages_identity/registry_builder(~990)/catalog_metadata(~900)/assembly_tests(~750,22/22 绿);move-only,仅拆分强制的 import/pub(super) 调整,双特性 check 干净。**real_invoke 半边按证据重判待裁决**:该文件 124 处 crate 内部引用且已在 #[cfg(test)] 后(零 lib 重量)——"出 src/ 进 tests/"会强迫 pub-for-tests 可见性泄漏,原验收"文件不存在"系盘点期误判动机(以为它进 lib 产物)。建议改判:留 src/ 现状即正确,或仅删验收第二条。**CTO 拍板** | Cli | mod.rs 半边完成 | mod.rs 260<500 ✓;real_invoke 待重裁 |
| T4.6 | **✅ 已完成(Cli e57990f + a22042f,2026-06-12)** F-033:agent.rs(3,556)→ agent/ 八文件(mod=参数+派发 474、lifecycle 610、send 613、mcp 541、inspect 179、publish 160、history 141、tests 1088);move-only,仅拆分强制的 import/pub(super)/两处搁浅节文档迁移;agent 套件 64/64 双跑绿(format 前后);验证经独立 CARGO_TARGET_DIR 绕开共享构建锁(对方 5 个套件排队)。千行级观察名单(federation_wire/start/join/auth/agent_new_ability)维持不强拆 | Cli | 完成 | ✓ 64/64 ×2 |
| T4.7 | **✅ 已完成(EasyNet 2d78e71 + f0c3300 + 605e14b + f72ea1a + 4a48df6,2026-06-12)** F-043 阀:本地 flat-config 插件规则(显式 AST visitor 非 esquery 串),7 处换 URAChip + 2 处理由豁免,阀双向验证。F-018 拆分四刀(move-only):Cut1 history.tsx(388)+panel.ts(27);Cut2 output.tsx(447,输出/回执面);Cut3 api.tsx(530,API 工作区+OpenAITestResult 探针类型);Cut4 workspaces.tsx(622,context-rail 模型+卫星簇——**闭簇裁定**:info→overview/contract、demo→input/guide 为簇内边,拆两文件会强迫私有接线跨文件导出,故一文件 5 出口、面板内件全 file-private)。Dialog 3,114→1,119(shell+detail+workbench 编排);每刀 tsc 收敛+52/52+eslint 0 错,pathspec 提交 | EasyNet FE | 完成 | ✓ 1,119<1,500;52/52 ×4 |

### Phase 5 — 规范卫生(P4;合批顺手做)

| # | 内容 | 仓 | 规模 | 验收 |
|---|---|---|---|---|
| T5.1 | **🟡 F-045 ✅(4930a67 + 夹具 ff8b009) + F-005 本仓半边 ✅(58d08ed,2026-06-12)**:result_large_err 在 Cli lib 归零——dispatch_shim 三签名 + json_value_to_payload/payload_to_json_value(审计后新增)Box 化,消费者仅错误路径拆箱(*err / map_err(\|e\| *e));decode_pubkey 例外走 justified #[allow](SDK trait KeyResolver::resolve 钉死错误类型,Box 即拆是凑数——该处归 SDK AxonError 瘦身)。注:bidi *err 与 daemon/invocation session_ext 两 hunk 被并行会话 520f1a3 吸收入库(共享索引,内容无损,归属混入)。**SDK 半边 ✅(Axon 3a4d4cdf,2026-06-12)**:AxonError 144→112 字节——诊断尾字段(context/cause_chain,两仓零按值消费点)Box 化,auto-deref 全访问点零改动、serde wire 透明;Axon 36 处 result_large_err 归零于源头;尺寸 pin 测试(<128,「新字段装箱而非提阈值」)+ wire 透明回环双钉。**仍开放**:2× too_many_args(随 T4.3);**静窗随手账**:decode_pubkey 的 justified #[allow] 已失效可删(类型已入预算)+ lib.rs 棘轮 result_large_err warn→deny(需热树释放后实跑确认) | Cli+Axon | 双半边完成 | result_large_err 两仓 0 ✓;too_many_args 随 T4.3 |
| T5.2 | **✅ 已完成(F-023 f0ce6f0 + F-034 8504e1a + 夹具修 ff8b009,2026-06-12;全量 axon-pb 3189/3189)** F-023:判别升到铸造点——LocalInvokeFailure{DaemonOffline,AbilityUnregistered}(thiserror,anyhow 链 downcast);classify_invoke_error 落 local_invoke 边界,旧子串表只在此作过渡回退(daemon 状态码无类型面=RFC 缺口),"全 crate 唯一许可嗅探点";四处生产消费改 match。**判别落 local_invoke 而非 EalError 加变体**:EAL 非唯一消费者,边界判别一次服务全部。验收 grep:生产面零命中,仅剩 mcp.rs:1137 文案钉死测试(消息契约非控制流)。F-034::572/:1322 已被先前重构消解;:112 改 UnknownReflectionMode 数据承载错误。F-045 测试夹具随 ff8b009 升 canonical URA(parse-only 旧夹具过不了 builder checked_ura) | Cli | 完成 | ✓ 3189/3189 |
| T5.3 | **F-022**:`enum MissionRunStatus{Running{heartbeat},Ok,Partial,Error,Cancelled}` 单点序列化;liveness = 心跳时间戳,pid 文件废除(serde wire 兼容迁移:旧 run.json 字符串可读)。**前置(S)**:run.json 消费面盘点——跨仓 grep(backend/前端/脚本是否直读 run.json 或 status 字符串),消费点清单落档后才许改 wire | Cli | M | 消费面清单无遗漏;旧 run.json 可读;假 running 场景测试(进程亡 → 状态可判定) |
| T5.4 | **F-028**:mission 上下文显式传参 / tokio task_local;`EASYNET_MISSION_ID` env var 仅保留子进程边界并文档化(5 文件:dispatch.rs/context.rs/agent.rs/mission_runs.rs/real-user-smoke.rs) | Cli | M,与 T5.3 同期 | 并发 mission 测试互不污染 |
| T5.5 | **✅ 已完成(Axon 28245ab4,2026-06-12)** F-012:InvocationReceipt.state 升 9 态 enum,new_receipt 收 enum(生产者本就持 enum,删一次转换);typed→wire 转换收敛到恰两个边界点(canonical_serialise + signing_body)以 as_str() 字节同形——**既有签名/链校验测试零改动全过(439→440)即 canonical 不变性实证**;TryFrom<&str> 落解析期拒绝(as_str 精确逆,大小写严格,bogus 拒绝,往返测试与 i32 pin 并排) | Axon | 完成 | ✓ 440/440;编译期+解析期双拒绝 |
| T5.6 | **✅ 已完成(EasyNet e6e7a56,2026-06-12)** F-029:8 处(实查另有 list_models.go 第 4 调用点 + firstValidatedDevice 跨包逐字重复)全部下沉 logic 层(pairing_queries/username_lookup/resolve_bearer 三新文件);check-handler-layer.sh 阀入 conformance workflow。build 清洁,16 测试包 ok | EasyNet BE | 完成 | 阀绿 ✓ |
| T5.7 | **✅ 已完成(EasyNet 4960a99,2026-06-12)** F-030:8 站点按形状分置——fire-and-forget×3 GoSafe;清扫循环×3 外层 GoSafe+每 tick RunSafe(单 panic 不杀循环);ws bidi 泵 GoSafe;fanout.Map 捕获 worker panic(值+栈)Wait 后调用方重抛(经 recover 中间件变 500)+ 注入测试钉住。%w 半边复核:宽模式 grep 0 残留(50/99 系审计期把无底层 err 的新建错误计入分母),无需施工 | EasyNet BE | 完成 | build/vet/触达包测试绿 ✓ |
| T5.8 | **🟡 DEC 已草拟待 CTO 签(Develop-Plan 8efba76 + EasyNet 6b08d7d,2026-06-12)** F-020:复核坐实两个新事实——backend receipt 行是有损投影(无 self_hash/nonce/bindings,canonical hash 不可重算)、Phase-9 sidecar 摄入叙事已死(实际摄入 = invokeAbilityLogic.go:365)。DEC-F020 推荐 B 案(非权威读模型,验证权威留 Axon)现在 + C 案(摄入重验,经 Go SDK)挂 T2.2c/d;store.go 包契约 + appendReceipt 注释已落边界声明。**待办:CTO 签 DEC;C 案落地前 UI 不得把行措辞为"已验证回执"(前端文案批核查)** | EasyNet BE | DEC 待签 | 文档 ✓ + DEC(proposed) |
| T5.9 | **✅ 已完成(EasyNet 84e2092,2026-06-12)** F-035:死常量删除(grep 零命中);配对转移表落 constants.go(写点 WHERE 守卫的单一真源);配对态×presence 态两层映射落 device_state.go 头部 | EasyNet BE | 完成 | ✓ |
| T5.10 | **✅ 已完成(Axon bc1fce71,2026-06-12)** F-014:execution.rs(3,114)→ execution/{mod 75,dispatch 1112,camera 840,uds_tools 572,tests 557}——dispatch 管线/相机捕获/UDS 本地工具一职责一文件,测试原样随迁;公开面不变(interop_native 再导出原三项);move-only,仅文件拆分强制的 import/可见性/super 路径调整;587/587 与拆前同数,clippy 零新增(注:无独立 conformance crate,587 即 runtime 全量) | Axon | 完成 | ✓ |
| T5.11 | **F-042 receipt URA 收口:① ✅ 已完成(2026-06-12,Cli dbf7615 + EasyNet 458c60a)**——非法 `invocation/` 顶层 role 实为 **7 处/4 文件**(审计记录 4 处,test_support.rs/real_invoke_tests.rs 另有 3),全部改 borrowed ledger 形状 + 标注,grep 闸零命中,RD 82/82;Frontend demo 补标注 + Dialog 测试的错形 owner 段(`resource/invocations/`)规范化,vitest 17/17。**② ✅ 议程件已草拟(2026-06-12)**:docs/rfc/AXON-RFC-007-receipt-ura-builder-agenda-2026-06-12.md——两议题各一推荐形状(receipt body = `…/invocation/<id>/receipt` history 同族;device-agent owner = `agent.device.<id>.<id>` kind-tagged 延伸)+ 两决策卡;**新事实**:ura-rs 三个 owner 派生点(:1040/:1050/:1119)Agent 臂全走 agent_ids(),device-agent 静默 None——System Agent receipt 链源头断裂,与议题一同批闭合最经济。**③ 长线仍开放**:receipt_ura 反序列化处过 parse_ura(builder 落地后,路径已写进议程件) | Cli+FE+RFC | ② 待 CTO 签两卡 | ② 可引用 ✓;③ 非法形状解析期拒绝 |
| T5.12 | **✅ 已完成(Cli 5935018,2026-06-12;axon-pb 全量 3183/3183 验证过)** F-050 残留:复核发现 skill/discover 面已零直读(spec 清单系审计期快照);真实残留 4 点全收口——ability_wire::load_default_profile 改读快照、PluginRuntimeManager::new() 经快照(boot 一次盘读暖缓存)、default_state() 升 pub(crate) 并把规则注释在 getter、resolver_seed 判冷注释。现存直读仅剩 getter miss 路径 + register/reload 写路径(故意重读后 publish,合规)。延迟口径:热面早已在快照上,沿用 c03df45 落档数字,无新增热路径需测 | Cli | 完成 | ✓ |

### 文档与制度批(随各阶段,不阻塞)
- **A7 DoD 固化**:「改 wire 必重生基线(Fed-MVP)必更新计数(RFC-001 baseline)」写进
  team-work/conventions/,并由 CI(tests.yml 已有 baseline-lock)执行。
- plan §2.4 配对状态机文档批(随 T5.9)。
- ura-discipline skill 演进点:RFC-007/008 落地(T5.11)、DEC-F046 落地(T2.3)、
  resource_dot owner 形状落地(T0.5c RFC 注记的解)时,skill 缺口节同步更新(需 CTO 授权)。
- 三件套维护纪律:状态变更(已修/已撤销)必须带提交哈希;行数证据标注核验日期。

---

## §4 执行纪律(全批次适用)

1. **不测不改**(T3.1 硬 gate;T2.1 基准做对照):性能改动必须有前后基准数字。
2. **move-only 与语义变更分提交**:拆文件 commit 不夹逻辑改动。
3. **文实同提交**:文档宣称与实现的修正在同一 commit(F-003 模式);本体正文与语法
   同提交对齐(F-048 教训)。
4. **撤销条目不复活**:F-013/016/019/025/026/032 重提需新证据新编号。
5. **共享 checkout 纪律**:开工 git status + mtime;绿了立刻提交;pathspec 提交;
   无 Co-Authored-By。
6. **每批验收即回归全套**:`cargo test --lib --features axon-pb` + 默认特性 +
   clippy 棘轮 + 守卫脚本。**axon-pb 盲点纪律**:默认构建 proto-free,宣称 build clean
   前必须双特性。
7. **URA 写前工作流**(ura-discipline):写任何 `easynet:///r/` 字面量前 grep Axon SDK
   builder;不能为每段辩护到 builder/RFC 行 = 形状错误;无 builder = flag 不发明。
8. **边界审查问句**(boundary skill checklist):每个新 API 先答「哪层拥有语义契约?」
   「七元组在公开边界是否完整?」「是否把 skill 当协议可调用?」。
9. **agent 盘点不直接施工**:任何 agent/Explore 结论动手前亲手复核(本审计 11 条
   假阳性全靠此拦截);**对照磁盘,不信陈旧 Read**。
10. **基线变更要有归属**:conformance/fixture 基线每次更新,增量逐条记录归属
    (T0.1 模式),禁止一次性吞。

## §5 依赖序与起跑组合

```
T2.1-pre 帧 mini-RFC ──► T2.1 帧定型 ──► T2.1b F-040 收口(backend 七元组)
  (frame0 契约版本号与 T1.1 共设计——frame0 只动一次)
T0.4 基准 ──► T2.1 对照 / T3.1 gate(锁拓扑)
SDK parity 盘点 ──► T2.2 A→B→C→D 严格顺序;F-041 守卫随 A 批
T0.5b 注册闸 ───► T0.5c F-047 八点(闸先立,面后修)
T0.6 DEC-F046(含预言机三问)──► T2.3 签名服务面
T1.1 状态机 ────► T1.2 指纹(claimant_conflict 需要状态机的家)
T4.1 依赖图设计 ─► T4.1 切分;T4.2 F-001 拆 ──► T4.3 F-002/F-006
T5.3 消费面盘点 ─► T5.3 改 wire
T0.1–T0.3 / T0.7 / T1.3 / T1.4 / T1.5①② / T5.6–T5.9 / T5.11① 互不依赖,立即可做
```

**建议起跑组合**(并行无冲突):
- **决策线**(CTO):T0.1(29 条归属)+ T0.6(DEC-F046 + 预言机三问)+ T0.7(增量审计重启)
- **小修线**(半天级,即刻见绿):T0.2 + T0.3 + T1.5①② + T5.11① + T5.9 + T5.6
- **工程线**:T0.4 基准 + T0.5a/b 两道闸 → T0.5c
- **设计件线**(可与工程线并行):T2.1-pre 帧 mini-RFC + T2.2 SDK parity 盘点 + T4.1 crate 依赖图
- **前端线**:T1.3 + T4.7(含 F-043)

**设计件清单**(决策完备、设计待产的四件,产出物都是文档,先评审后施工):
T2.1-pre(帧 mini-RFC)· T2.2 parity 盘点(逐函数映射表)· T4.1 crate 依赖图 ·
T5.3 run.json 消费面清单。其余 TODO 按行内描述即可直接施工。

## §6 条目 → TODO 完整映射(防丢核对表;44 活跃条,每条恰有一个主归宿)

| 条目 | 主归宿 | 搭车/备注 |
|---|---|---|
| F-001 | T4.2 | |
| F-002 | T4.3 | |
| F-003 ✅ | 基线 | 文实同提交范本 |
| F-004 | T2.1 | A2 核心 |
| F-005 | T5.1 | 2× too_many_args 随 T4.3 |
| F-006 | T4.3 | global() 6 消费点(含 fc8df1b 新增心跳点) |
| F-007 ✅ | 基线 | |
| F-008 | T1.1 | |
| F-009 | T1.2 | |
| F-010 | T4.1 | |
| F-011 | T3.1 | 含 prefix 扫描附注 |
| F-012 | T5.5 | |
| F-014 ✅ | T5.10(bc1fce71) | |
| F-015 | T2.2a–d | |
| F-017 | T2.2b | |
| F-018 | T4.7 | |
| F-020 | T5.8 | |
| F-021 | T4.4 | |
| F-022 | T5.3 | |
| F-023 | T5.2 | |
| F-024 | T1.4 | |
| F-027 | T4.5 | |
| F-028 | T5.4 | |
| F-029 | T5.6 | |
| F-030 ✅ | T5.7(4960a99) | %w 半边复核 0 残留 |
| F-031 | T1.3 | |
| F-033 | T4.6 | |
| F-034 | T5.2 | |
| F-035 | T5.9 | |
| F-036 | T0.1 | + 文档批 DoD |
| F-037 | T0.2 | |
| F-038 | T0.3 | 终态由 T2.1 结构性消灭 |
| F-039 ✅ | 基线 | a1a4aea + f1c1f29 |
| F-040 | T2.1b | |
| F-041 | T2.2a 搭车 | backend+FE 守卫脚本 |
| F-042 | T5.11 | ②③ 系 RFC-007/008 议程 |
| F-043 | T4.7 搭车 | + eslint 阀 |
| F-044 | T2.1b 搭车 | 注释清理 |
| F-045 | T5.1 搭车 | FFI timeout 透传 |
| F-046 | T0.6 → T2.3 | DEC 先行 |
| F-047 ✅ | T0.5c(4487344) | 判定表 v2 = commit body;验收复核过 |
| F-048 ✅决议 | T0.5a/b 执行闸 | §3.1.2 已入库 52bb764a |
| F-049 ✅主修 | T1.5 残留 | fc8df1b |
| F-050 ✅主修 | T5.12 残留 | c03df45 + 0716861 |
| F-051 ✅ | T0.7 第 17 轮(同轮修复) | D7 stats 三测试入册 |
| F-052 | T0.7 第 18 轮新债 → 前端线搭车 | lifelong 前端半边零测试;AskToDoPage 在制占用,释放后补绑定三测 |
| F-053 ✅主修 | 第 19 轮(同轮修复,Axon c1a03e8f) | §7.1 wire 形入 normative;残留按需(SDK 长 decode 时采全 schema) |

(撤销 6 条 F-013/016/019/025/026/032 不在表内;F-016 残迹随 T2.2a 文件删除自然消亡。)
