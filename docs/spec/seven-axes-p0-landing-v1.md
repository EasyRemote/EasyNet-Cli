# seven-axes-p0-landing-v1 — 七轴 P0/P1 产品面落地总规格

> **日期**:2026-06-13(v2.0,policy/trust-level 产品面撤销修订) · **状态**:本仓 W1/W2/W3 已落地,Axon reference gate/trust 投影原语已落地;本仓 standalone `policy` 与 `trust level` product surface 已移除,统一 access-control 另立设计
> **分支**:`seven-axes-p0-landing-v1`(EasyNet-Cli,已采用;Axon 半边见 §2 W3 各任务)
> **权威依据**:`docs/cli-command-review-2026-05-30.md`(产品宪法,25 节)·
> `to-be-fix.spec.md` §0.1 本体不变量 · **easynet-runtime-boundary skill(边界裁决)**·
> **easynet-ura-discipline skill(URA/本体形状)**· AXON-RFC-001 v4.1.x · RFC-005 §3.1.2 /
> DEC-F048 · `EasyNet-Axon/document/concepts/CONCEPT_MODEL.md:59-68` ·
> `ONTOLOGY_AGENT_ABILITY.md:148,220` · `EasyNet-Axon/core/ura-rs/src/lib.rs`(URA 真源)
> **范围**:把产品评审定下的 P0/P1 七轴缺口落成可验收的 CLI 能力,分三波(W1/W2/W3)。
> **不在范围**:wallet/agreement(被 §6.2 字段阻塞,另立 RFC);voice/stream/state_sync/admin
> 四域(review §10 挂起,等 out-of-scope 裁决)。
>
> **v1.1 修订记录(对 v1.0 的自审,2026-06-13)**:① 清除 3 处发明的 URA 形状,改为
> builder 引用 + 占位符,新增测试作者规则(§3.0);② trust 边界归属重写(执法门在 Axon
> resilience.rs:711-717,CLI 只 host 目录 ability);③ 新增 T2.0(mission run 输出
> root_invocation_id——原 W2-E2E-2 的 root-id 无来源,核查 mission_runs.rs 零命中);
> ④ 新增七元组完整性断言(0.1-7 护栏 + 各 e2e);⑤ T3.2 当时曾补 ura-rs family 段解析前置;
> ⑥ T3.1 声明 usage 是 receipt 输出非第八元组参数;⑦ 当时新增决策 D8(trust 主体
> agent vs node)、D9(联邦降级退出码);⑧ receipt 引用一律按 invocation_id,不写
> receipt URA(F-042 四野形状未决,RFC-007 决策卡 pending)。
>
> **v1.2 修订记录(CLI 风格统一,2026-06-13;已被 v2.0 修正 trust-level 口径)**:⑨ 新增 §0.2 CLI 实现风格契约
> (10 条,每条带现行代码先例);⑩ 发现既有 `easynet trust` 组语义冲突(anchor 平面
> vs TrustLevel 平面,C15)→ T2.2 重写为 `trust level` 子命令族 + 新增 D10;
> ⑪ 全文 `--json` 统一为 house style `--format json`(ability_search.rs:45 /
> groups/trust.rs:49 先例),watch 流式 NDJSON 在 §0.2-3 标注为唯一例外。
>
> **v1.3 修订记录(T2.0 本体论修正,2026-06-13 实现期)**:⑫ 实现核查发现
> CLI 发起的 mission run **不存在根 Invocation 对象**——运行时身份是
> trace(`MissionContextGuard::enter(run_id)` 把 run_id 作为 trace_id 打在每个
> 步骤 envelope 上)。伪造根 invocation = 从后门造 mission 级运行时实体,违反
> §0.1-1。T2.0 改为 **trace 锚定**:`MissionRunMeta.trace_id` 显式落盘
> (在飞 meta 即携带,watch 可中途挂上)+ `mission run --format json` 输出
> `{run_id, trace_id, status, …}`。watch 入口相应为
> `invocation watch --trace <id>`(全 run)或 `invocation watch <invocation-id>`
> (因果子树——当 mission 作为 EalExec ability 实现运行时,父 invocation 是天然根,
> 子树 watch 在那条路径成立)。W1 已落地(commits 1a5f874/dbeb5ba),W1 e2e 已补齐。
>
> **v1.4 修订记录(T3.2 重定界,2026-06-13 实现期,已被 v1.9 撤销本仓半边)**:
> 当时曾把 `policy.evaluate/simulate`、tiny matcher、`policy-rules.json` 和
> `easynet policy …` 作为 CLI daemon product gate 推进。v1.9 复核 PMF 与权限
> 架构后判定:该分支会抢占未来统一 ability access/permission control 的位置,
> 且错误解释可由通用 admission/dispatch error + ledger 承担。本仓半边已移除;
> Axon reference runtime 的内部 trust gate rule table 保留为相邻仓实现细节。
>
> **v1.5 修订记录(T3.1 收口核查,2026-06-13 实现期)**:⑭ 核查相邻
> `EasyNet-Axon` 当前工作树,`InvocationReceipt.usage` 已进入 proto field 31,
> runtime/client-sdk/easynet-verify/Rust SDK/多语言 SDK 签名体;`usage_tail_is_signed_material`
> 已证明 usage tail 是 signed material。本仓 W3 usage e2e 证明 receipt→ledger→watch
> consumer 链闭合。因此 T3.1 从未收口项移除;`invocation show/watch` 已展示
> `usage_signed_material=true` 与 `usage_verified_by_cli=false`(诚实标注:协议签名覆盖已在
> Axon,CLI 不做离线验签)。v1.5 当时 Axon §A6 gate 改线 + trust 投影尚未收口;
> v1.7 已完成 reference runtime 原语。
>
> **v1.6 修订记录(T3.2 live gate/why 收口,2026-06-13 实现期,已被 v1.9 撤销)**:
> 当时曾让 daemon `AdmissionFacade` 调用 `PolicyRuleEngine`,并把 live deny 以
> `POLICY_DENIED` 写入 ledger policy context。v1.9 已删除这条 product gate:
> admission 只负责身份、trust-anchor、签名、replay、delegation、quota 等既有门;
> ability 调用权限回到统一 access-control/permission 体系设计。
>
> **v1.7 修订记录(Axon reference C6/T2.3 收口,2026-06-13 实现期)**:⑯
> 相邻 `EasyNet-Axon/core/runtime-rs` 已新增 `runtime::policy` 内置规则表,
> 把 C6 install/admin trust gate 从 `resilience.rs` 内联 if 改为
> `evaluate_builtin_runtime_policy(RuntimePolicyFacts)`;deny reason
> `TRUST_LEVEL_TOO_LOW` / `ADMIN_SCOPE_REQUIRES_ELEVATED_TRUST` 逐字节保持。
> 同模块新增 `project_agent_trust_to_host_node(agent_ura,new_trust,reason)`,
> 以 `AgentRecord.host_node_id → NodeDescriptor.trust_level` 完成 T2.3
> agent→node policy input 投影,并清 policy eval cache + 发 membership event。
> 已有 Axon 单测覆盖 rule table、C6 reason 回归、projection 后 install gate
> 放行;v1.7 当时把产品路径复核列为下一步,v1.8 已判定当前 P0 产品路径
> 不需要 CLI daemon 调用该 Axon reference 原语。
>
> **v1.8 修订记录(产品路径闭环判定,2026-06-13 实现期)**:⑰ 复核
> `easynet runtime start` / daemon boot / runtime factory / trust handler 后确认:
> 当前产品形态启动 `easynet-daemon`(device/hub 同形),并在 daemon 内构造
> `easynet_axon::invocation::LocalRuntime`;v1.9 已撤销 W3 live policy gate,
> v2.0 已删除本仓 `identity.get_trust/set_trust` 与 trust directory 产品面。`EasyNet-Axon/core/runtime-rs::AxonRuntime` 的
> `project_agent_trust_to_host_node` 是 raw/reference runtime 部署形态的原语,
> 不是当前 CLI daemon 必须主动调用的 API。故 v1.7 所列"产品路径复核"
> 收口为 **不适用于当前 P0 产品路径**;若未来重新启用 raw axon-runtime 作为
> 产品执行进程,另立部署连接器任务,不回压本规格 P0。
>
> **v1.9 修订记录(policy 产品面撤销,2026-06-13 实现期)**:⑱ 删除本仓
> standalone `policy` 分支:`policy.evaluate/simulate` ability、`easynet policy`
> CLI、`policy-rules.json` store、daemon live policy hook、ledger `policy_*`
> context、`policy-default` hosted profile 与 W3 policy e2e。理由:PMF 价值实际是
> ability access/permission,不是独立 policy 产品;大系统权限启动应统一规范
> ability policy/access control,当前小 matcher 会污染未来主线。错误解释继续由
> 具体 admission/dispatch/ability access error message 与 ledger 通用 error record
> 承担。
>
> **v2.0 修订记录(trust-level 产品面撤销,2026-06-13 实现期)**:⑲ 删除本仓
> `trust level show/set` CLI、`identity.get_trust/set_trust` hosted ability、
> `trust-levels.json` store、discover `trust_level` 候选列与 W2 trust e2e。理由:
> `trust anchor` 被 `AdmissionFacade` 实际消费,必须保留;`trust level` 在 policy
> 产品面删除后没有本仓消费边,只剩目录事实/展示字段,会误导为权限控制。未来如需
> ability access/permission,应在统一权限模型里定义可消费的能力、主体、scope 与
> ledger error 合约,不得从该目录面复活。

---

## §0 立意与北极星

七轴 = easy to **GET / USE / MANAGE / ORGANIZE / PROTECT / ACCOUNT / ECONOMIC**。
本规格只做其中四轴的临门一脚:

- **USE**:`discover` 收口(runtime 四级阶梯已在,缺统一门面)
- **ACCOUNT→ORGANIZE**:Invocation 因果树 TUI 一期(签名事实的唯一可视窗口)
- **PROTECT**:`trust show`(realm trust anchor) + 既有 ability access/permission 底座(standalone policy/trust-level 已撤销)
- **ACCOUNT↔ECONOMIC**:`InvocationReceipt.usage` 签名字段(整个 §5 经济轴的第一块砖)

### 0.1 本体护栏(任何任务不得违反,违反 = Rule 1 拒绝类)

1. **唯一 runtime 可寻址对象 = Invocation**。Mission/EAL 是脚本;重试 = re-invoke
   (经 `causal_context` 连接,types.proto:343 / invoke.proto:696);**禁止** step-level
   retry / mission 级可变状态 / checkpoint / 控制面(= 重造 Temporal)。
2. **协议真源唯一在 Axon;产品 access-control 另立统一设计**:usage 字段、协议 trust 枚举、
   InstallPolicy 类型在 Axon 落或已在 Axon,CLI 只经 `pb::axon::v1`
   消费;禁止 CLI 手抄协议形状。EasyNet-Cli daemon 不再拥有 standalone
   policy matcher;ability 调用权限应归入统一 ability access/permission control。
3. **device 是 sponsor 不是 principal**(DEC-F048):trust 挂 agent/node 身份,
   不发明 device-owned trust,不让 device 承担 principal 问责。
4. **目录读 = 快照读**:discover/trust 读进程内快照,禁止每查询磁盘重读/全量重哈希。
5. **树是投影不是地址**:discover 的任何分组/树渲染只活在展示层和 access grouping,
   **永不**进 URA 与路由。
6. **owner 主动性**:teach/learn 默认关(`allow_transferred_code=false`),
   能力是 owner 授予的,永远不是 consumer 拉取的(无 `pull`)。
7. **七元组完整性**:每个新增动词在公开边界上必须保持
   `invoke(caller, callee, ability, subject, nonce, causal_context, args) → receipt`
   七字段完整可查。subject/nonce/causal_context 被隐藏、静默默认、钉空 =
   receipt 链正确性 bug(不是工学问题)。本规格各 e2e 含元组断言。
8. **URA 纪律**:本文及测试中出现的所有 URA 仅为**形状示意**,以
   `EasyNet-Axon/core/ura-rs/src/lib.rs` 的 builder 为唯一真源
   (`Ura::ability` :389 / `Ura::device_ability` :395 / `Ura::hub_ability` :401 /
   `Ura::device_agent` :383);测试 fixture 一律由 daemon 输出或 builder 生成,
   **禁止手写 URA 字面量**;断言走 `parse_ura` round-trip,不做字符串外推。
   receipt 引用一律按 `invocation_id`(receipt body URA 形状未决,F-042/RFC-007)。

### 0.2 CLI 实现风格契约(新命令一律遵守;每条 = 现行代码先例,不是新发明)

1. **名词组下挂动词,不造平行入口**:动词进既有 `groups/<noun>.rs`
   (trust/invocation/mission 组已存在);仅 discover 经 D1 裁决才得顶层入口。
   组文件头部维护动词路由表注释(`verb → cli::<module>`,groups/ability.rs:11-19)。
2. **故意不做的动词写进组头部**:维护 "Verbs DELIBERATELY ABSENT" 段,记录被
   本体论否决的动词及理由(groups/ability.rs 的 `update` 先例)。本规格新增义务:
   ability 组记 `pull`(§2.5 裁决:能力是 teach 的,不是 pull 的);
   invocation/mission 组记 `retry-step`/`resume-step`/`patch-step`(0.1-1)。
3. **输出格式 = `--format <table|json>`**:
   `#[arg(long, value_enum, default_value_t = OutputFormat::Table)]`
   (ability_search.rs:45、groups/trust.rs:49)。**禁止新增裸 `--json` flag**。
   唯一例外:`watch --follow --format json` 输出 NDJSON 事件流(流无 table 形态)。
4. **人类输出走 `support::output` 助手**:success/warn/error/info/step/detail/
   kv_section + `table()`(comfy_table UTF8_FULL_CONDENSED,output.rs:29-118);
   禁止手写 println! 表格;强调色统一 `console::style`。
5. **破坏性/写操作挂 `-y/--yes` 确认门**:`#[arg(long, short = 'y')]`
   (reset.rs:50、join.rs:108 先例)。本规格命中:`ability teach`。
6. **错误处理 `anyhow::Context`,上下文是小写短动词短语**:
   `.context("query local ability catalogue")`(ability_search.rs:143 风格)。
7. **文件头 banner 统一**:`// EasyNet CLI — <topic>` + File/Description
   (+组文件的动词表)+ `Author: Silan Hu` + Copyright(全仓一致)。
8. **实现与注册分离**:实现落 `src/cli/<topic>.rs`,`groups/` 只做 clap
   分发;薄壳别名(discover → ability_search)共用同一实现函数,**禁止复制逻辑**
   (W1 验收门的 byte-identical 断言由此保证)。
9. **JSON 输出形状稳定可断言**:e2e 断言过的字段名即冻结;演进只加字段不改名
   (C11 的 meta.json 向后兼容教训同源)。
10. **排序/分组可解释**:ranked/tree 输出的排序键必须出现在 `--format json` 里
    (score 字段先例,ability_search.rs 排序契约)——用户能预测每行为何在那。

---

## §1 现状核查表(evidence-grade,2026-06-13 对 HEAD 核查)

| # | 项 | 核查结论 | 证据 |
|---|---|---|---|
| C1 | discover runtime 半边 | ✅ **已实现**:`<agent>.discover` 四级阶梯(self→device→user→public),Tier3/4 已接 `federation.resolve`,联邦失败返回类型化 `federation_not_joined`/`federation_unavailable` | `src/runtime/agents/discover_ability.rs:1-30,150,267,359` |
| C2 | discover CLI 半边 | ✅ 已收口:`easynet discover <intent>` 与 `ability search` 共享 `discover.rs` 单实现,走 `<agent>.discover` ladder,本地排序仍为 name×3/desc×1/owner×1+全命中奖励;JSON 携带 invocation envelope 回显 | `src/cli/discover.rs`;`tests/seven_axes_w1_discover_e2e.rs` |
| C3 | discover 缺口收口 | ✅ 已收口:cross-owner user-tier 通过 daemon 本地 hub read model 覆盖,测试经公开 `federation.advertise_agent`/`advertise_abilities` 写目录,再由 `<agent>.discover(scope=user)` 读回;顶层动词/envelope 回显已落;v2.0 已删除未消费的 trust 列 | `src/runtime/agents/discover_ability.rs`;`src/services/invocation_transport/daemon_invocation_service.rs`;`tests/seven_axes_w1_discover_e2e.rs` |
| C4 | `ability invoke` | ✅ 已收口:吃 canonical URA,本地派发 + `--node` 远程,不从裸名 mint URA | `src/cli/invoke.rs:1-20` |
| C5 | trust level handler(CLI 侧) | 🧹 **已从本仓 P0 移除**:`identity.get_trust/set_trust`、`trust-levels.json` 与 `easynet trust level` 不再存在。保留的是被 admission 消费的 realm trust anchor | `src/cli/groups/trust.rs`;`src/services/invocation_transport/admission_facade.rs` |
| C6 | trust 执法门(Axon 侧) | ✅ **已归一为规则表**:install 需 `TrustLevel::Privileged`、admin 需 `Elevated`,由 Axon `runtime::policy` 内置规则表求值,`resilience.rs` 只负责 cache/override/metrics;deny reason 保持旧字节 | `EasyNet-Axon/core/runtime-rs/src/runtime/policy.rs`;`EasyNet-Axon/core/runtime-rs/src/runtime/resilience.rs`;`EasyNet-Axon/core/runtime-rs/src/tests/resilience_unit.rs` |
| C7 | standalone policy 求值器 | 🧹 **已从本仓 P0 移除**:不保留 `policy.evaluate/simulate`、`easynet policy`、`policy-rules.json`、daemon live matcher 或 ledger `policy_*` context。权限产品语义改回统一 ability access/permission control;Axon reference runtime 的 C6 内置 rule table 仍是相邻仓内部 trust gate 实现 | `src/runtime/agents/invoke_ability.rs`;`src/daemon/execution/permission/mod.rs`;`EasyNet-Axon/core/runtime-rs/src/runtime/policy.rs` |
| C8 | receipt usage/cost | ✅ **已收口**:本仓 terminal receipt 的 usage 逐字进入 ledger row,watch terminal 事件聚合展示,W3 usage e2e 已绿;相邻 Axon 工作树已把 `InvocationUsage` 放入 signed receipt tail,`usage_tail_is_signed_material` 已绿;cost 明确不做(D4) | `src/services/invocation_transport/ledger_projection.rs`;`src/cli/invocation_watch.rs`;`tests/seven_axes_w3_usage_e2e.rs`;`EasyNet-Axon/sdk/rust/src/invocation/axiom.rs` |
| C9 | teach/learn handler | ✅ 已落地:`meta.teach` / `meta.acquire`(CLI `ability learn`) / `meta.forget` 走 owner 主动授权,learn 后新 URA 独立归属 learner,forget 只删 learned copy | `src/runtime/agents/teach_ability.rs`;`src/cli/teach.rs`;`tests/seven_axes_w3_teach_learn_e2e.rs` |
| C10 | TUI 数据底座 | ✅ 已落地:`invocation watch` 从 ledger/trace 投影 state + terminal + local liveness;TUI 由同一 snapshot engine 渲染 | `src/cli/invocation_watch.rs`;`tests/seven_axes_w2_watch_e2e.rs` |
| C11 | mission→invocation 锚点 | ✅ 已落地:`MissionRunMeta.trace_id == run_id`,mission runner 将同一 trace_id 注入每个 daemon-lowered child Invocation;三步 mission watch e2e 验证 ledger trace 可回读 | `src/cli/mission_runs.rs`;`src/eal/interpreter/mod.rs`;`tests/seven_axes_w2_watch_e2e.rs` |
| C12 | TUI 渲染依赖 | ✅ 已引入:`ratatui` snapshot renderer(无 crossterm 运行时事件循环;一期 one-shot/follow 重绘即可) | `Cargo.toml`;`src/cli/invocation_watch.rs` |
| C13 | URA builder 真源 | ✅ 齐备:`Ura::ability` :389 / `device_ability` :395 / `hub_ability` :401 + `parse_ura` round-trip;e2e fixture 有真源可依 | `EasyNet-Axon/core/ura-rs/src/lib.rs` |
| C14 | e2e 基建 | ✅ 成熟:双 daemon 真 TLS、cross-device invoke、cross-realm 目录轮询/流式等模板齐备 | `tests/cross_hub_two_daemon_real_tls_e2e.rs` 等 |
| C15 | `easynet trust` 命令组 | ✅ **收口为单一 anchor 面**:`groups/trust.rs` 只读展示 realm trust anchor("admission 接受谁的 key、什么角色",commit-plan-2 D3/Gate D)。`trust level show/set` 已删除,避免把未消费目录误写成权限控制 | `src/cli/groups/trust.rs` |

**总判断**:W1/W2/W3 均已按本规格收口;当前表格保留的是实现证据索引,
不是待施工清单。

---

## §2 任务分解(三波,每波可独立合入)

### W1 — discover 收口(USE 轴,已落地)

**T1.1 统一发现路径**
`ability search` 的候选源从「`meta.list_abilities` + `federation.discover` 双路」
切换为调用 C1 的 `<agent>.discover` 四级阶梯(单一 resolver 路径)。保留现有可解释
排序算法不变。调用一律改走 `invoke_local_ability_with_invocation_meta`
(local_invoke.rs:160),使 envelope caller/subject 可回显、可断言(0.1-7)。
caller 身份 = CLI 当前操作 agent(沿用 daemon 既有 caller 解析,不在 facade 发明身份)。
- 改动面:`src/cli/ability_search.rs`、`src/support/local_invoke.rs`(如需)
- 护栏:快照读(0.1-4);排序契约不变(用户可预测每行为何排在那)

**T1.2 顶层动词 `easynet discover "<intent>"`**
作为 `ability search` 的顶层别名注册(同一实现,双入口)。输出每行带
`ura / owner_kind / scope(tier) / score / description`。v2.0 已删除未被权限消费的
`trust_level` 候选列;discover 不承担权限解释。
- 改动面:`src/cli/mod.rs` + 新 `src/cli/discover.rs`(薄壳)
- D1 已按 A 方案落地:`discover` 上顶层,并与 `ability search` 共用同一实现。

**T1.3 `--tree` 投影(ORGANIZE 轴顺手分)**
`discover --tree` 按 owner 前缀分组渲染候选。纯展示层:分组键取自
`parse_ura` 解析出的 owner 段(`agent_ids()` / `device_agent_ids()` 双访问器都要处理,
F-047 教训),不引入 family 一等对象,不动 URA(0.1-5)。
- 改动面:`discover.rs` 渲染分支

### W2 — trust 面 + watch/TUI 一期(PROTECT/ACCOUNT 轴,已落地)

**T2.0 mission run 锚定 trace(watch 的前置,C11;v1.3 本体论修正)**
CLI 发起的 run 没有根 Invocation——运行时身份是 **trace**(每个步骤 Invocation
的 envelope 都带 trace_id == run_id)。落地:`MissionRunMeta.trace_id` 显式字段
(在飞 meta 即携带,`#[serde(default)]` 保旧 meta 反序列化)+
`mission run --format json` 输出 `{run_id, trace_id, status, duration_ms,
steps_*, run_dir}`(与 `--trace` 互斥,stdout 归属唯一)。
- 改动面:`src/cli/mission_runs.rs` + `groups/mission.rs` ✅ 已落地
- 护栏:记录的是已存在的 envelope 事实,**不**给 mission 造任何运行时状态(0.1-1);
  不伪造根 invocation——EalExec 路径下父 invocation 才是天然根

**T2.1/T2.2 trust-level 目录 ability + CLI(已撤销)**
v2.0 删除 `identity.get_trust/set_trust`、`trust-levels.json` 与
`easynet trust level show/set`。当前本仓只保留被 admission 实际消费的
realm trust anchor(`easynet trust show`)。如果未来需要"接受之后能做什么"的产品
能力,必须进入统一 ability access/permission 设计,并明确消费者、主体、scope、
拒绝原因与 ledger 合约。

**T2.3 trust 真的在管事(enforcement 验证,不是新功能)**
验证两条消费边界各自闭合:
1. CLI 产品面:只保留 trust anchor admission gate;P0 不再维护 trust-level directory,
   也不再用 standalone matcher 把它接成横切 admission policy。
2. Axon reference 面:`runtime::policy` 规则表替代 C6 硬编码门,
   `project_agent_trust_to_host_node` 把 AgentRecord 的 trust 更新投影到
   `NodeDescriptor.trust_level`,projection 后 install gate 放行。
- 改动面:已完成;当前 P0 不再要求 CLI daemon 主动调用 reference
  `AxonRuntime` 投影 API。

**T2.4 `invocation watch <invocation-id> | --trace <trace-id>` `[--follow] [--format json|--tui]` 一期**
- 数据层:NDJSON 事件流(每事件 = 子 invocation 状态变迁/receipt 到达),TUI 与
  `--format json`(0.2-3 的流式例外形态)共享同一事件流(先 json 后渲染,e2e 可测)。事件流是 **receipt/派发状态的
  投影,不是第二真源**(orchestration emits Invocations; it does not redefine them)。
- 渲染层:引入 `ratatui`(仓库首个 TUI 依赖,D3)。三栏 master-detail:
  Phases(EAL IR 编译期分区 = 意图)/ 子 invocation 列表(= 事实)/ 单条 receipt 展开。
  **Planned graph ≠ actual DAG**:计费/审计字段只读 receipt,永不读 EAL 意图。
- **显示真实性契约**(每字段标注数据级别,界面如实渲染):

| TUI 元素 | 数据级别 | 一期处理 |
|---|---|---|
| phase 树 | EAL IR(编译期意图) | ✅ 渲染,仅作分组 |
| 子 invocation 行 / 状态点 / caller→callee→subject / 工具流水 / Outcome | 签名事实(receipt 链) | ✅ 渲染 |
| 活性/崩溃(`is_interrupted`) | 本地 heartbeat | ✅ 渲染,标 `local` |
| usage counters / cost | usage counters = **协议签名材料**(Axon receipt signed tail);cost 按 D4 deferred | 渲染 `usage_signed_material=true`;CLI 离线验签标 `usage_verified_by_cli=false` |
| permission(命中规则) | **无数据**(求值器未建) | 留位显示 `–`,W3 后点亮 |

- 改动面:`src/cli/`(新 `invocation_watch.rs`)、`Cargo.toml`(+ratatui)、
  复用 `mission_runs.rs` heartbeat/状态机;依赖 T2.0
- 护栏:渲染 **Invocation 因果树**,不是 workflow graph;禁止为 TUI 引入任何
  mission 级运行时状态(0.1-1);`p pause` 一期**不做**(D5)

### W3 — 跨仓协议件(已收口)

**T3.1 `InvocationReceipt.usage`(已收口)**
本仓 consumer wire-ride 子集已完成:daemon terminal receipt 携带的 `usage`
被 `LedgerSink` 原样投进 ledger row,`invocation watch` 从同一 row 聚合 terminal
事件,无 token handler 时计数器保持零值非缺失。Axon 半边也已完成当前工作树实现:
`usage { tokens_in, tokens_out, duration_ms, external_calls }`
(+预留可选 `cost`,金额/货币本期**不做**,D4)进入 proto field 31,emit 时填充,
并纳入 `callee_signature` 签名覆盖与 `self_hash` 哈希链。
**七元组纪律声明:usage 是 receipt 输出,不是第八个元组参数**——不动
invoke 的七字段,只丰富其产物(runtime-boundary "do not add an eighth primitive
parameter" 条款的合规路径)。
- 改动面:EasyNet-Axon `invoke.proto` + receipt emit 路径;CLI 侧已完成
  receipt→ledger→watch 透传,并在 `invocation show/watch` 输出 usage 签名材料标记;
  离线签名验真仍归 `easynet-verify`/Axon verify 工具链,不在 CLI P0 内重造
- 护栏:字段进签名覆盖是本任务的全部意义;当前由 Axon
  `usage_tail_is_signed_material` 固定

**T3.2 standalone policy 最小求值器(本仓撤销;Axon reference 半边保留)**
本仓撤销 daemon-owned matcher 与 operator 面:`policy list/create/remove/simulate`
四动词、`policy-rules.json`、`policy.evaluate/simulate` handler、daemon
admission path hook、ledger `policy_*` context。原因:PMF 价值实际是 ability
access/permission,不是独立 policy 产品;在大系统中权限启动应统一规范 ability 的
access policy、consent/permission broker、trust anchor 与 ledger error
解释。P0 不引入会污染未来统一权限主线的小 matcher。

Axon reference 半边仍保持 C6/T2.3 当前收口:runtime 内置规则表替代 C6 硬编码
trust gate(行为不变,deny reason 逐字节保持),并提供 agent→host-node trust
projection 原语。该 `runtime::policy` 是 Axon reference runtime 的内部 gate
实现名,不等同于本仓产品面 `easynet policy`。
- 改动面:本仓删除 `src/cli/policy_cli.rs` /
  `src/runtime/agents/policy_ability.rs` / `src/runtime/agents/policy_engine.rs` /
  `src/persistence/policy_rules.rs` / `tests/seven_axes_w3_policy_e2e.rs`,并移除
  `AdmissionFacade` 的 live matcher hook。
- 护栏:未来权限统一设计不得从 standalone `policy` 复活;应以 ability access
  contract 为入口,错误解释走具体 access/permission/admission error message +
  ledger 通用 error record。

**T3.3 teach/learn(已收口,同设备 manifest-only,D6 默认)**
`meta.acquire`/`meta.forget` handler + `ability teach --to` / `learn --from` /
`forget`;InstallPolicy 三闸门(`allow_transferred_code` 默认 false、
`require_consent`、`execution_mode=sandbox_first` 默认),**类型一律
`pb::axon::v1`,CLI 不得重定义**。learn 后 learner 持有自己 URA 下的新 ability
(由 `Ura::ability` 以 learner 的 user/agent id 生成),原 ability owner 不变。
assets 走 `PayloadTransfer`(仅存的非 Invoke service)——但 `--with-assets`
一期不做(D6),manifest-only。
- 改动面:`meta_ability.rs` + 新 CLI 动词
- 护栏:0.1-6(owner 主动性);0.1-8(URA 由 builder 生成)

---

## §3 CLI 端到端测试规格(验收即命令输出,落 `tests/` 现有模板)

### §3.0 测试作者规则(适用全部 e2e,违者打回)

1. **fixture 不手写 URA**:测试中的 URA 一律取自 daemon 输出(deploy/publish 的
   返回值)或 ura-rs builder;期望值断言走 `parse_ura` round-trip + 成分比对
   (realm/owner/ability-id),不做整串字符串比对(0.1-8)。
2. **七元组断言**:每个新动词至少一条测试断言其 invocation 的
   caller/callee/ability/subject 四字段非空且形状正确(经
   `invoke_local_ability_with_invocation_meta` 的 envelope 回显或 receipt 查询)。
3. **receipt 按 invocation_id 引用**,不出现 receipt URA 字面量(F-042 未决)。
4. 双 daemon 场景复用 `cross_hub_two_daemon_real_tls_e2e.rs` fixture 模式;
   命名 `tests/seven_axes_w<N>_<topic>_e2e.rs`。
5. 下文 JSON 中的 `easynet:///r/...` 均为**形状示意**(按 C13 builder 形状书写),
   实测断言按规则 1 执行。

### W1-E2E-1 `tests/seven_axes_w1_discover_e2e.rs` — 跨 owner 发现闭环
真 UDS daemon 作为本地 hub read model:测试经公开
`federation.advertise_agent` / `federation.advertise_abilities` 写入一个 hosted
agent 的远端 ability 投影,再通过普通 `easynet discover` 路径触发
`<agent>.discover(scope=user)` 读回 canonical URA(记 `$URA_A`,形如
`easynet:///r/<realm>/ability/agent.<owner>.<ability>`,agent-owned builder):
```console
$ easynet discover "read a file" --format json
{
  "query": "read a file",
  "tiers_searched": ["self", "device", "user"],
  "candidates": [
    {
      "ura": "<$URA_A>",
      "owner_kind": "device",
      "scope": "user",
      "score": 8,
      "description": "read a remote file from another owner"
    }
  ]
}
```
断言:① 候选含 `$URA_A` 且 `scope` 层级正确;② 每个候选 `ura` 经 `parse_ura`
round-trip 成功,owner 段经 `agent_ids()`/`device_agent_ids()` 正确分派(F-047);
③ 排序分数可复算(name×3/desc×1/owner×1 契约);④ `--tree` 输出按 owner 段分组,
**不出现**任何输入集合之外的 URA;⑤ 七元组:本次 discover 调用的 envelope 回显
caller 非空且为合法 agent URA(§3.0-2)。

### W1-E2E-2 同文件 — 联邦降级是类型化的
B 不入 realm 时:
```console
$ easynet discover "anything" --format json
{ "tiers_searched": ["self","device"],
  "federation": { "status": "federation_not_joined" }, "candidates": [ ... ] }
```
断言:类型化 envelope 而非裸错;本地两级照常返回;退出码按 D9 裁决执行
(本规格默认 0 = 优雅降级)。

### W2-E2E-1 trust-level 产品面(已撤销)
`tests/seven_axes_w2_trust_e2e.rs` 已删除。P0 不再提供 `trust level show/set`
或 `identity.get_trust/set_trust`;trust anchor 的实际消费由 admission 路径覆盖,
权限/ability access 另立统一设计。

### W2-E2E-2 `tests/seven_axes_w2_watch_e2e.rs` — watch 数据层(TUI 渲染走快照单测)
跑一个 3-phase mission,trace id 从 T2.0 取得:
```console
$ TRACE=$(easynet mission run demo.eal --format json | jq -r .trace_id)
$ easynet invocation watch --trace "$TRACE" --follow --format json   # NDJSON,0.2-3 例外
{"event":"state","invocation":"<child-1>","ability":"testbot.echo","state":"COMPLETED"}
{"event":"state","invocation":"<child-2>","ability":"testbot.echo","state":"COMPLETED"}
{"event":"state","invocation":"<child-3>","ability":"testbot.echo","state":"COMPLETED"}
...
{"event":"terminal","trace":"<$TRACE>","status":"ok"}
```
断言:① `trace_id` 非空、与 run_id 一致且全程稳定(T2.0 契约,在飞 meta 已携带);② 事件序列与终态
`mission show --trace` 一致(同一真源,事件流只是投影);③ dead heartbeat 的 running
meta → 流尾出现 `{"event":"liveness","status":"interrupted","source":"local"}`,
**而非**永远 running;④ TUI 渲染层用同一流做 snapshot 测试(单测层);
⑤ 子 invocation 事件均携带其 invocation_id,事件里**不出现** step 序号寻址
(0.1-1:无 step 可寻址对象)。usage 现在按 signed receipt→ledger→watch
同链路展示;签名覆盖由 Axon T3.1 固定。

### W3-E2E-1 `tests/seven_axes_w3_usage_e2e.rs` — 账单可信的定义
```console
$ easynet ability invoke "$URA" --args '{...}'
$ easynet invocation show <invocation-id> --format json | jq .usage
{ "tokens_in": 1832, "tokens_out": 412, "duration_ms": 5734,
  "external_calls": 0, "signed": true }
```
当前本仓断言:① 真实 invocation 的 terminal receipt 携带 `usage`;② ledger row
逐字接住 usage;③ watch terminal 事件聚合展示;④ 无 token handler 时 usage
计数器为零值非缺失;⑤ 七元组不受影响:usage 仅是 receipt 输出。Axon 协议层断言:
`usage` 在 callee 签名覆盖内,篡改 usage 字节后验签必须失败
(由 Axon `usage_tail_is_signed_material` 固定签名数学)。

### W3-E2E-2 — standalone policy e2e 已删除
本仓不再维护 `tests/seven_axes_w3_policy_e2e.rs`。可解释拒绝不再通过
`easynet policy why` 或 `POLICY_DENIED` 专属 context 验收;后续统一权限设计应在
ability access/permission path 上定义自己的 e2e,并把具体拒绝原因放入普通
`LedgerErrorRecord.message/context`。

### W3-E2E-3 `tests/seven_axes_w3_teach_learn_e2e.rs` — owner 主动性三连
A 的 ability URA 记 `$URA_A`;learn 成功后 B 的新 URA 记 `$URA_B`
(由 daemon 以 learner 身份经 `Ura::ability` 生成,形如
`easynet:///r/<realm>/ability/<userB-uuid>.<agentB-id>.<ability-id>`):
```console
$ easynet ability learn "$URA_A" --from <A>     # A 未 teach
error: not teachable (allow_transferred_code=false)

$ # A 侧:
$ easynet ability teach "$URA_A" --to <B> -y
$ # B 侧:
$ easynet ability learn "$URA_A" --from <A>
learned · new ura: <$URA_B>

$ easynet ability show "$URA_A" --format json | jq -r .owner   # 原 owner 不变
<A>
```
断言:① 默认拒绝是第一条测试(最重要);② `$URA_B` 经 `parse_ura` round-trip,
owner 成分 = B,且 `$URA_A` owner 不变(双 URA 各自归账,两条独立 receipt 链);
③ learned 能力首次执行落 `sandbox_first`(InstallPolicy 经 `pb::axon::v1` 读取);
④ 七元组:teach 的 subject = `$URA_A`,learn 是 `invoke(self, self, meta.acquire, …)`
反身形(ontology:220),receipt 上可见。

---

## §4 验收门(每波退出条件 = 对应 e2e 全绿 + 纪律检查)

| 波 | 退出条件 |
|---|---|
| W1 | W1-E2E-1/2 绿;`ability search` 与 `discover` 输出 byte-identical(同一实现);URA guard CI 绿;`cargo clippy` 含 `--features axon-pb` 双跑干净 |
| W2 | W2-E2E-1/2 绿(含七元组断言);TUI snapshot 绿;ratatui 引入经 D3 点头;T2.0 的 meta.json 向后兼容(旧 meta 无 trace_id 仍可反序列化 ✅ `pre_trace_id_meta_still_deserializes`);T2.3 Axon reference projection 原语已落地,不再阻塞 CLI W2 退出 |
| W3 | 本仓 W3-E2E-1/3 绿;W3-E2E-2 standalone policy 已删除;CLI 零手抄协议形状(URA guard CI 绿);usage Axon signed-tail 已验;Axon reference C6 rule-table 与 T2.3 projection 单测已验;当前 P0 产品路径闭环,无剩余实现项 |

---

## §5 分支与提交纪律

- **分支**:`seven-axes-p0-landing-v1`(本仓,2026-06-13 实际采用,替代原提议的
  feat/ 名);Axon usage 半边已落(T3.1),Axon reference C6 rule-table 与 T2.3
  projection 原语已落;本仓 standalone policy product gate 已撤销。
- 共享 checkout 纪律全程适用:动手前 `git status`;提交一律显式 pathspec
  `git commit -- <paths>`;同文件混入他人 hunk 时按 hunk 级核对;**不带
  Co-Authored-By**,作者 Silan.Hu。
- 1 commit = 1 逻辑变更;每个 T 编号至少一个独立 commit,message 带 T 编号。
- 与 carrier/T2.1 主线的关系:W1/W2/W3 CLI 半边已在本分支收口;
  Axon reference C6 gate 改线与 T2.3 projection 原语已落地。产品路径复核确认
  当前 P0 不需要 CLI daemon 主动调用 reference `AxonRuntime` 投影 API;本仓也不
  保留 standalone policy matcher。

---

## §6 决策收口表(执行期口径)

| # | 决策 | 阻塞 | 本规格默认 |
|---|---|---|---|
| D1 | review §0 的 A/B:discover/whoami/invoke 上不上顶层 | T1.2 | **已落地:A(上顶层)** |
| D2 | §6.2 跨仓提案现在就发给 Axon 排期? | T3.2/T2.3 Axon gate 改线 | **已执行**:reference runtime 原语已落;当前 CLI daemon 产品路径不需要主动调用 raw/reference `AxonRuntime` 投影 API |
| D3 | ratatui 作为仓库首个 TUI 依赖 | T2.4 | 引入(锁版本) |
| D4 | `cost` 金额字段(货币+定价源)本期做不做 | T3.1 范围 | **不做**,只做 usage |
| D5 | TUI `p pause` | T2.4 范围 | **一期砍掉**(协议无原语) |
| D6 | teach `--with-assets`(走 PayloadTransfer 传文件) | T3.3 范围 | **一期不做**,manifest-only |
| D7 | standalone `policy create` 的裸表达式语法 | T3.2 范围 | **不做且已删除该产品面**;未来走统一 ability access/permission control |
| D8 | **trust 主体本体**:RFC-001 重述签名是 `{agent_ura}`,而 Axon 执法门吃 `node_trust_level`(resilience.rs:711)——主体是 agent 还是 node?两处现状不一致,需裁决 | T2.1/T2.2/W2-E2E-1 | **CLI 产品面撤销该目录事实**;当前本仓只保留 admission trust anchor。Axon reference 面仍可由 `project_agent_trust_to_host_node` 投影到 node trust |
| D9 | discover 联邦降级的退出码:0(优雅降级)还是非 0 | W1-E2E-2 | **0** + 类型化 envelope |
| D10 | **trust 命令面归属**(C15):既有 `trust show` 是 anchor 平面(admission keys),TrustLevel 是授权属性平面——扩展为 `trust level show/set` 子命令族,还是另立 noun?| T2.2 | **v2.0 已改判:不暴露 trust-level 产品面**。`trust` 组只保留 anchor show;权限能力另立统一 ability access/permission |

---

## §7 条目→测试映射(防丢核对面)

| 任务 | e2e | 单测/快照 | 验收门 |
|---|---|---|---|
| T1.1 统一发现路径(+元组回显) | ✅ `seven_axes_w1_discover_e2e` 落地(真 UDS daemon:agent.list→ladder→双 scope→类型化降级→envelope 回显→**候选投影 + URA round-trip + 冻结分数逐位复算 + cross-owner user-tier 经公开 federation advertise/read-model 闭环**)。discover handler 注入 daemon-local federation resolver,不在 handler 内反拨 UDS,生产 boot 与 e2e fixture 共享 `AdvertisedAgentStore`/`AbilityCatalogStore` | 排序契约 4 测 + 投影 5 测 + JSON 契约冻结测,全绿 | W1 |
| T1.2 顶层 discover | 同上(execute() 共用实现,byte-identical 由构造保证) | — | W1 |
| T1.3 --tree 投影 | ✅ 已落地 | ✅ `tree_groups_by_owner_and_preserves_score_order_within_groups` | W1 |
| T2.0 trace 锚点(v1.3 修正) | W2-E2E-2① | ✅ `pre_trace_id_meta_still_deserializes` + `in_flight_meta_carries_trace_anchor` | W2 |
| T2.1/2.2 trust-level 目录 ability + CLI | 🧹 v2.0 已删除:`seven_axes_w2_trust_e2e`、`identity.get_trust/set_trust`、`trust-levels.json`、`trust level show/set`、discover trust 列。保留 `trust show` anchor admission 面 | 本仓编译/检索确认无 trust-level product 面 | W2 |
| T2.3 trust enforcement(Axon 门) | ✅ Axon reference 原语已落地:`project_agent_trust_to_host_node` 通过 `AgentRecord.host_node_id` 更新 `NodeDescriptor.trust_level`,清 policy eval cache 并发 membership event;单测证明 projection 后 C6 install gate 放行。当前 CLI daemon 产品路径不再有 standalone admission policy gate,也不需要主动调用 raw/reference `AxonRuntime` 投影 API | ✅ Axon projection 单测 | W2→W3 |
| T2.4 watch/TUI | ✅ 数据层 + e2e 双落地:`seven_axes_w2_watch_e2e` 覆盖账本投影→state 事件→terminal-ok(协议词表判终)+三步 EAL mission→daemon child Invocations→ledger trace→watch trace 同源+dead heartbeat → `liveness/interrupted/local`;fixture 接单句柄 ledger(sink 写 + history 读同一 Arc,daemon 重启共享);无 trace 的裸 unary = 单例因果集(诚实降解不拒绝);`--format tui` 已引入 ratatui snapshot renderer(三栏 Phases/Invocations/Receipt,同一 watch engine,不暴露 step 地址) | ✅ engine 4 测(含 TUI snapshot) + interpreter trace 契约测 + e2e 1 测 | W2 |
| T3.1 receipt usage | ✅ `seven_axes_w3_usage_e2e` 落地:terminal receipt usage→ledger row→watch terminal 一路透传;零 counters 非缺失;Axon `usage_tail_is_signed_material` 已绿,证明 usage tail 是签名材料;`invocation show/watch` 显示 signed-material/CLI-unverified 双标记 | ✅ Axon signed-tail 单测 + 本仓 watch/ledger/show 聚合测 | W3 |
| T3.2 standalone policy 求值器 | 🧹 本仓已删除:不再有 `seven_axes_w3_policy_e2e`、`PolicyRuleEngine`、`policy.evaluate/simulate`、`easynet policy` 或 `policy-rules.json`;Axon reference runtime 内置规则表仍替代 C6 硬编码 gate | ✅ Axon rule-table/projection 单测;本仓编译/检索确认无 standalone policy product 面 | W3 |
| T3.3 teach/learn(v1:同设备 manifest-only,D6 默认) | ✅ 实现 + e2e 双落地:`seven_axes_w3_teach_learn_e2e` 覆盖 ①默认拒绝先行 ②学后双 URA 独立可发现(各自 exactly-one-owner)+ forget 后副本退场原件幸存 ③execution_mode 申明 sandbox_first;executor enforcement 另立里程碑 | ✅ handler 5 测 + 存储 2 测 + e2e 1 测 | W3 |
