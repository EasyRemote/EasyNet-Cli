# seven-axes-p0-landing-v1 — 七轴 P0/P1 产品面落地总规格

> **日期**:2026-06-13(v1.1,自审修订) · **状态**:待 CTO 批准 → 执行
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
> ④ 新增七元组完整性断言(0.1-7 护栏 + 各 e2e);⑤ T3.2 补 ura-rs family 段解析前置;
> ⑥ T3.1 声明 usage 是 receipt 输出非第八元组参数;⑦ 新增开放决策 D8(trust 主体
> agent vs node)、D9(联邦降级退出码);⑧ receipt 引用一律按 invocation_id,不写
> receipt URA(F-042 四野形状未决,RFC-007 决策卡 pending)。
>
> **v1.2 修订记录(CLI 风格统一,2026-06-13)**:⑨ 新增 §0.2 CLI 实现风格契约
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
> 子树 watch 在那条路径成立)。W1 已落地(commits 1a5f874/dbeb5ba),W1 e2e 待补。
>
> **v1.4 修订记录(T3.2 重定界,2026-06-13 实现期)**:⑬ 审计发现
> `policy.evaluate/simulate` 已在 CLI runtime 注册为 **wire 契约已 pin 的
> v1 allow-all 桩**,且模块头明示设计拓扑为「Axon kernel 门经 §A6 调用 daemon 侧
> policy.evaluate」——求值逻辑本就归本仓,Axon 侧只剩门改线(独立里程碑)。
> 据此 T3.2 的 CLI 半边已落地:tiny matcher(action/family-prefix/trust-below
> 三谓词,首条命中,空库=说出口的 baseline-allow)+ `policy-rules.json` 规则库 +
> `easynet policy list/create/remove/simulate` 四动词。trust 谓词经
> `trust_ability::level_rank`(pb 枚举单源)消费 T2.1 的目录——PROTECT 两半在此
> 扣合。`policy why` 维持等待门改线(对昨天的调用重放今天的规则=对历史撒谎,拒做)。
> Axon 侧剩余:§A6 门改线 + T2.3 trust 投影 + T3.1 receipt usage,同批排期。

---

## §0 立意与北极星

七轴 = easy to **GET / USE / MANAGE / ORGANIZE / PROTECT / ACCOUNT / ECONOMIC**。
本规格只做其中四轴的临门一脚:

- **USE**:`discover` 收口(runtime 四级阶梯已在,缺统一门面)
- **ACCOUNT→ORGANIZE**:Invocation 因果树 TUI 一期(签名事实的唯一可视窗口)
- **PROTECT**:`trust show/set` + policy 最小求值器(两根缺失的脊梁之一半)
- **ACCOUNT↔ECONOMIC**:`InvocationReceipt.usage` 签名字段(整个 §5 经济轴的第一块砖)

### 0.1 本体护栏(任何任务不得违反,违反 = Rule 1 拒绝类)

1. **唯一 runtime 可寻址对象 = Invocation**。Mission/EAL 是脚本;重试 = re-invoke
   (经 `causal_context` 连接,types.proto:343 / invoke.proto:696);**禁止** step-level
   retry / mission 级可变状态 / checkpoint / 控制面(= 重造 Temporal)。
2. **协议真源唯一在 Axon**:usage 字段、policy 求值器、TrustLevel 枚举、InstallPolicy
   类型在 Axon 落或已在 Axon,CLI 只经 `pb::axon::v1` 消费;禁止 CLI 手抄协议形状。
3. **device 是 sponsor 不是 principal**(DEC-F048):trust 挂 agent/node 身份,
   不发明 device-owned trust,不让 device 承担 principal 问责。
4. **目录读 = 快照读**:discover/trust 读进程内快照,禁止每查询磁盘重读/全量重哈希。
5. **树是投影不是地址**:discover 的任何分组/树渲染只活在展示层和 policy scope,
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
   (reset.rs:50、join.rs:108 先例)。本规格命中:`trust level set`、
   `policy create`、`ability teach`。
6. **错误处理 `anyhow::Context`,上下文是小写短动词短语**:
   `.context("query local ability catalogue")`(ability_search.rs:143 风格)。
7. **文件头 banner 统一**:`// EasyNet CLI — <topic>` + File/Description
   (+组文件的动词表)+ `Author: Silan Hu` + Copyright(全仓一致)。
8. **实现与注册分离**:实现落 `src/facade/cli/<topic>.rs`,`groups/` 只做 clap
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
| C1 | discover runtime 半边 | ✅ **已实现**:`<self>.discover` 四级阶梯(self→device→user→public),Tier3/4 已接 `federation.resolve`,联邦失败返回类型化 `federation_not_joined`/`federation_unavailable` | `src/runtime/agents/discover_ability.rs:1-30,150,267,359` |
| C2 | discover CLI 半边 | ✅ **大半已实现**:`easynet ability search <intent>` 已有 intent 分词排序(name×3/desc×1/owner×1+全命中奖励),双路:本地 `meta.list_abilities` + 联邦 `federation.discover`,可解释排序、无 LLM | `src/facade/cli/ability_search.rs:1-21` |
| C3 | discover 缺口 | 🔴 四件:① 顶层动词(§0 A/B 拍板悬置);② search 未走 C1 四级阶梯(两套发现路径);③ 候选行无 trust 列;④ 现走 `invoke_local_ability` 便利路径,envelope caller 不回显(七元组可查性) | C1/C2 对比;`src/support/local_invoke.rs:124,160`(`with_invocation_meta` 变体已备) |
| C4 | `ability invoke` | ✅ 已收口:吃 canonical URA,本地派发 + `--node` 远程,不从裸名 mint URA | `src/facade/cli/invoke.rs:1-20` |
| C5 | trust handler(CLI 侧) | 🔴 **零命中**:CLI src/ 无 `get_trust/set_trust/TrustLevel`;RFC-001 mapping 的 `identity.get_trust/set_trust{agent_ura}` 是重述表名字,无 handler | `grep -rn` src/ 全空 |
| C6 | trust 执法门(Axon 侧) | ✅ **已在运行**:install 需 `TrustLevel::Privileged`、admin 需 `Elevated`,硬编码于 Axon resilience,吃 `node_trust_level` | `EasyNet-Axon/core/runtime-rs/src/runtime/resilience.rs:711,715-717` |
| C7 | policy 求值器 | 🔴 **零命中**:Axon core 无 `PolicyEvaluate`;review §3.8 结论(admission=C6 的 2 个硬编码检查,`PolicyRule.condition` 从未被读)仍成立 | `grep -rln` EasyNet-Axon/core/src 全空 |
| C8 | receipt usage/cost | 🔴 **未落**:`invoke.proto` 无 usage/cost 字段(仅注释命中);TUI 的 token 数只能本地数,不可审计不可计费 | `grep -n` invoke.proto |
| C9 | teach/learn handler | 🔴 **零命中**:`meta_ability.rs` 无 acquire/forget;ontology-only 双层缺口(无 handler + 无 CLI)仍成立 | `grep -n` meta_ability.rs 全空 |
| C10 | TUI 数据底座 | 🟡 **半建**:`MissionRunStatus` 类型化枚举 + heartbeat 活性 + `is_terminal()` + `is_interrupted()` 已落(F-022/T5.3);缺流式 watch 与渲染层 | `src/facade/cli/mission_runs.rs`;`docs/mission-run-status-consumer-inventory.md` |
| C11 | mission→invocation 锚点 | 🔴 **缺失**:`mission_runs.rs` 不记录/不输出 `root_invocation_id`(零命中)——watch/TUI 的入口 id 无来源,必须先补(T2.0) | `grep -n` mission_runs.rs 全空 |
| C12 | TUI 渲染依赖 | 🔴 `ratatui`/`crossterm` 未引入(Cargo.toml 仅 248 行注释提及) | `Cargo.toml:248` |
| C13 | URA builder 真源 | ✅ 齐备:`Ura::ability` :389 / `device_ability` :395 / `hub_ability` :401 + `parse_ura` round-trip;e2e fixture 有真源可依 | `EasyNet-Axon/core/ura-rs/src/lib.rs` |
| C14 | e2e 基建 | ✅ 成熟:双 daemon 真 TLS、cross-device invoke、cross-realm 目录轮询/流式等模板齐备 | `tests/cross_hub_two_daemon_real_tls_e2e.rs` 等 |
| C15 | `easynet trust` 命令组 | ⚠️ **已存在但是另一平面**:groups/trust.rs = realm trust anchor 只读面("admission 接受谁的 key、什么角色",commit-plan-2 D3/Gate D);头部明确 trust≠permission(invariant 8)且 anchor 写路径需先过 DEC。与本规格的 TrustLevel(授权属性,"接受之后信到什么程度")是两个平面,共用 noun 需 D10 裁决 | `src/facade/cli/groups/trust.rs:1-49` |

**总判断**:W1 是收口不是新建(C1+C2 焊接);W2 是补 handler + 补锚点(C11);
W3 是跨仓协议件。没有一项无地基。

---

## §2 任务分解(三波,每波可独立合入)

### W1 — discover 收口(USE 轴,预估 2–3 天,无跨仓依赖)

**T1.1 统一发现路径**
`ability search` 的候选源从「`meta.list_abilities` + `federation.discover` 双路」
切换为调用 C1 的 `<self>.discover` 四级阶梯(单一 resolver 路径)。保留现有可解释
排序算法不变。调用一律改走 `invoke_local_ability_with_invocation_meta`
(local_invoke.rs:160),使 envelope caller/subject 可回显、可断言(0.1-7)。
caller 身份 = CLI 当前操作 agent(沿用 daemon 既有 caller 解析,不在 facade 发明身份)。
- 改动面:`src/facade/cli/ability_search.rs`、`src/support/local_invoke.rs`(如需)
- 护栏:快照读(0.1-4);排序契约不变(用户可预测每行为何排在那)

**T1.2 顶层动词 `easynet discover "<intent>"`**
作为 `ability search` 的顶层别名注册(同一实现,双入口)。输出每行带
`ura / owner_kind / scope(tier) / score / trust_level`(trust 列本波恒为
`null`,渲染为 `–`,W2 点亮——界面先说真话)。
- 改动面:`src/facade/cli/mod.rs` + 新 `src/facade/cli/discover.rs`(薄壳)
- ⚠️ 依赖拍板 D1(§6)。本规格按 **A 方案(上顶层)** 编写;裁 B 则 T1.2 降级为只做 T1.1。

**T1.3 `--tree` 投影(ORGANIZE 轴顺手分)**
`discover --tree` 按 owner 前缀分组渲染候选。纯展示层:分组键取自
`parse_ura` 解析出的 owner 段(`agent_ids()` / `device_agent_ids()` 双访问器都要处理,
F-047 教训),不引入 family 一等对象,不动 URA(0.1-5)。
- 改动面:`discover.rs` 渲染分支

### W2 — trust 面 + watch/TUI 一期(PROTECT/ACCOUNT 轴,预估 1.5–2 周,等 carrier stream-arm 收口后启动)

**T2.0 mission run 锚定 trace(watch 的前置,C11;v1.3 本体论修正)**
CLI 发起的 run 没有根 Invocation——运行时身份是 **trace**(每个步骤 Invocation
的 envelope 都带 trace_id == run_id)。落地:`MissionRunMeta.trace_id` 显式字段
(在飞 meta 即携带,`#[serde(default)]` 保旧 meta 反序列化)+
`mission run --format json` 输出 `{run_id, trace_id, status, duration_ms,
steps_*, run_dir}`(与 `--trace` 互斥,stdout 归属唯一)。
- 改动面:`src/facade/cli/mission_runs.rs` + `groups/mission.rs` ✅ 已落地
- 护栏:记录的是已存在的 envelope 事实,**不**给 mission 造任何运行时状态(0.1-1);
  不伪造根 invocation——EalExec 路径下父 invocation 才是天然根

**T2.1 trust 目录 ability(CLI daemon host,语义归 Axon)**
边界裁决(runtime-boundary Rule 1/2):**TrustLevel 枚举、trust 执法语义是 Axon 真源**
(C6,resilience.rs 已在执法);CLI daemon 仅 host Hub-profile 目录 ability
`identity.get_trust` / `identity.set_trust`(同 `federation.resolve` 的 host 模式,
RFC-001 §A14),读写 hub 目录里的 trust 属性,类型经 `pb::axon::v1`,零 CLI 重定义。
七元组:`subject` = 被信任实体的 URA;`set_trust` 是一次完整 invoke,产出 receipt。
- 改动面:`src/runtime/agents/`(新 `trust_ability.rs`)+ `registry_builder.rs` 注册
- ⚠️ 依赖拍板 D8(§6):trust 主体是 `{agent_ura}`(RFC-001 重述表签名)还是
  node(C6 执法门吃 `node_trust_level`)——两处现状不一致,**flag 而非外推**。
  本规格按「主体 = agent URA,daemon 负责 agent→node 投影供执法门消费」编写,待裁。

**T2.2 `easynet trust level show <ura>` / `trust level set <ura> --level <L>`**
⚠️ 命名冲突已核查(C15):`easynet trust` 组已存在,语义是 **anchor 平面**(admission
接受谁的 key,只读)。本任务的 TrustLevel 是**授权属性平面**(接受之后,信到什么程度,
驱动 C6 执法门)。默认方案(D10):扩展既有 `groups/trust.rs` 为 `level` 子命令族,
组头部重写为两平面注释(anchor = keys / level = degree),不另造顶层 noun。
anchor 的"写路径需 DEC"告诫属 anchor 真源,不自动覆盖 level 平面;但 `level set`
仍为写操作:`-y` 门(0.2-5)+ 完整 invoke + receipt。`show` 输出 trust_level +
来源 + 最近一次变更的 `invocation_id`(0.1-8)。
- 改动面:`src/facade/cli/groups/trust.rs`(扩展)+ 新 `src/facade/cli/trust_level.rs`
  (实现,0.2-8 实现与注册分离)
- 依赖拍板 D10(§6)

**T2.3 trust 真的在管事(enforcement 验证,不是新功能)**
验证 Axon 执法门(resilience.rs:711,715)消费 T2.1 写入的值;deny 时 receipt
`deny_reasons` 含既有标识(`ADMIN_SCOPE_REQUIRES_ELEVATED_TRUST` 等,:717)。
- 改动面:预期 0–小(若 daemon 未把目录 trust 投影进 NodeDescriptor,则补投影);
  这是验收项,发现缺口按缺口立 T 编号

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
| token/cost | **未签名**(本地数的) | 渲染但标 `unsigned`,W3 后去标 |
| permission(命中规则) | **无数据**(求值器未建) | 留位显示 `–`,W3 后点亮 |

- 改动面:`src/facade/cli/`(新 `invocation_watch.rs`)、`Cargo.toml`(+ratatui)、
  复用 `mission_runs.rs` heartbeat/状态机;依赖 T2.0
- 护栏:渲染 **Invocation 因果树**,不是 workflow graph;禁止为 TUI 引入任何
  mission 级运行时状态(0.1-1);`p pause` 一期**不做**(D5)

### W3 — 跨仓协议件(预估 2–3 周,需 Axon 排期,先发 RFC 占位)

**T3.1 `InvocationReceipt.usage`(Axon 半边:`feat/receipt-usage-field`)**
proto 加 `usage { tokens_in, tokens_out, duration_ms, external_calls }`
(+预留可选 `cost`,金额/货币本期**不做**,D4);emit 时填充,纳入
`callee_signature` 签名覆盖与 `self_hash` 哈希链。
**七元组纪律声明:usage 是 receipt 输出,不是第八个元组参数**——不动
invoke 的七字段,只丰富其产物(runtime-boundary "do not add an eighth primitive
parameter" 条款的合规路径)。
- 改动面:EasyNet-Axon `invoke.proto` + receipt emit 路径;CLI 侧 `invocation show` 透传
- 护栏:字段进签名覆盖是本任务的全部意义;不进签名 = 白做

**T3.2 policy 最小求值器(Axon 半边:`feat/policy-min-evaluator`)**
**前置(同支先行)**:ura-rs 把 URA 的 family/namespace 段解析进
`ParsedURA`(review §3.7/3.8 的 (b) 步;无此则 `ability.family` 谓词无输入)。
然后:tiny matcher,只支持三类谓词 `trust_level >= <L>`、
`ability.family == "<prefix>"`(前缀匹配)、`node.label == <v>`;admission 路径上
把 C6 的 2 个硬编码检查改写为同引擎的内置规则(行为不变,实现归一);deny 产生带
rule-id 的 `deny_reasons`。CLI 暴露 `policy list/create/simulate/why` 四动词
(create 用引导式 flag,裸表达式语法本期不做,D7)。
- 改动面:EasyNet-Axon ura-rs + admission 路径;CLI 新 `policy.rs`
- 护栏:不做完整表达式语言;`policy why` 读 receipt/decision,不开第二审计面;
  family 谓词仅作 policy scope,family **永不**入路由(0.1-5)

**T3.3 teach/learn(最后,依赖 T3.2 的 visibility/consent 可执行)**
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
双 daemon(A=owner 发布 ability,B=consumer)入同一 realm。A 侧 deploy 后取得
canonical URA(记 `$URA_A`,形如
`easynet:///r/<realm>/ability/device.<device-A>.fs.read`,device-owned builder :395):
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
      "trust_level": null
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

### W2-E2E-1 `tests/seven_axes_w2_trust_e2e.rs` — trust 读写 + 真的在管事
被信任主体 URA 由测试 fixture 的 daemon 身份导出(记 `$SUBJ`,按 D8 裁决为
agent URA,形如 `easynet:///r/<realm>/agent/<user-uuid>.<agent-id>`):
```console
$ easynet trust level show "$SUBJ" --format json
{ "subject": "<$SUBJ>", "trust_level": "STANDARD", "source": "hub-directory" }

$ easynet trust level set "$SUBJ" --level elevated -y
trust level updated · invocation <invocation-id>

$ easynet trust level show "$SUBJ" --format json | jq -r .trust_level
ELEVATED
```
断言:① `set` 产生完整 invocation:`invocation show <id>` 可见
caller/callee/ability(`identity.set_trust`)/**subject = `$SUBJ`**(七元组,§3.0-2);
② **enforcement**:把 `$SUBJ` 对应 node 投影降为 UNTRUSTED 后,从该 node 发起
install → 被拒,receipt `deny_reasons` 含 Axon 既有标识
(resilience.rs:711/717 的 install-PRIVILEGED / `ADMIN_SCOPE_REQUIRES_ELEVATED_TRUST`);
③ 重启 daemon 后 trust 值幸存(快照持久,0.1-4);④ 降级路径(agent→node 投影,
T2.1 裁决)在 receipt 上可追溯。

### W2-E2E-2 `tests/seven_axes_w2_watch_e2e.rs` — watch 数据层(TUI 渲染走快照单测)
跑一个 3-phase mission,root id 从 T2.0 取得:
```console
$ TRACE=$(easynet mission run demo.eal --format json | jq -r .trace_id)
$ easynet invocation watch --trace "$TRACE" --follow --format json   # NDJSON,0.2-3 例外
{"event":"state","invocation":"<child-1>","phase":"Inventory","state":"running"}
{"event":"receipt","invocation":"<child-1>","receipt_type":"progress", ...}
{"event":"state","invocation":"<child-1>","state":"completed"}
...
{"event":"terminal","trace":"<$TRACE>","status":"ok"}
```
断言:① `trace_id` 非空、与 run_id 一致且全程稳定(T2.0 契约,在飞 meta 已携带);② 事件序列与终态
`mission show --trace` 一致(同一真源,事件流只是投影);③ kill 掉 run 进程 →
流尾出现 `{"event":"liveness","status":"interrupted","source":"local"}`
(复用 `is_interrupted()`),**而非**永远 running;④ TUI 渲染层用同一流做
snapshot 测试(单测层);⑤ usage 字段在 W3 前渲染必须带 `"signed": false`;
⑥ 子 invocation 事件均携带其 invocation_id,事件里**不出现** step 序号寻址
(0.1-1:无 step 可寻址对象)。

### W3-E2E-1 `tests/seven_axes_w3_usage_e2e.rs` — 账单可信的定义
```console
$ easynet ability invoke "$URA" --args '{...}'
$ easynet invocation show <invocation-id> --format json | jq .usage
{ "tokens_in": 1832, "tokens_out": 412, "duration_ms": 5734,
  "external_calls": 0, "signed": true }
```
断言:① `usage` 在 callee 签名覆盖内——**篡改 usage 字节后验签必须失败**(核心断言);
② `"signed"` 是 CLI 验签的**计算结果**,不是存储字段(输出文档须注明);③ 离线验证:
导出 receipt 链,第三方用公钥独立验签通过;④ 无消耗调用 usage 为零值非缺失;
⑤ 七元组不受影响:加字段前后 invoke 的七字段形状不变(usage 仅是 receipt 输出)。

### W3-E2E-2 `tests/seven_axes_w3_policy_e2e.rs` — 可解释的拒绝
```console
$ easynet policy create --effect deny --action invoke --min-trust standard -y
policy <rule-id> created

$ easynet ability invoke "$URA"        # 从 UNTRUSTED caller
error: denied by policy <rule-id>

$ easynet policy why <invocation-id> --format json
{ "decision": "deny", "rule": "<rule-id>",
  "matched": "trust_level >= STANDARD", "caller_trust": "UNTRUSTED" }

$ easynet policy simulate --caller "$CALLER_URA" --ability "$URA" --format json | jq -r .decision
deny
```
断言:① deny 带 rule-id(黑盒拒绝 = 验收失败);② simulate 与真实 admission
同函数同结论(dry-run 不是第二实现);③ 规则删除后同调用放行;④ family 谓词:
`--family-prefix <agent-owner-prefix>` 规则只命中该前缀下的 ability,且对路由
零影响(0.1-5 断言:同 URA 解析、同 dispatch 路径);⑤ C6 两个原硬编码门改写后
行为逐字节不变(回归断言)。

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
| W2 | W2-E2E-1/2 绿(含七元组断言);TUI snapshot 绿;ratatui 引入经 D3 点头;T2.3 enforcement 断言绿;T2.0 的 meta.json 向后兼容(旧 meta 无 trace_id 仍可反序列化 ✅ `pre_trace_id_meta_still_deserializes`) |
| W3 | W3-E2E-1/2/3 绿;Axon 侧 proto/ura-rs 变更过 RFC 流程;CLI 零手抄协议形状(URA guard CI 绿);C6 回归断言绿 |

---

## §5 分支与提交纪律

- **分支**:`seven-axes-p0-landing-v1`(本仓,2026-06-13 实际采用,替代原提议的
  feat/ 名);Axon 半边 `feat/receipt-usage-field`(T3.1)、
  `feat/policy-min-evaluator`(T3.2,含 ura-rs family 段前置),各发独立 RFC。
- 共享 checkout 纪律全程适用:动手前 `git status`;提交一律显式 pathspec
  `git commit -- <paths>`;同文件混入他人 hunk 时按 hunk 级核对;**不带
  Co-Authored-By**,作者 Silan.Hu。
- 1 commit = 1 逻辑变更;每个 T 编号至少一个独立 commit,message 带 T 编号。
- 与 carrier/T2.1 主线的关系:W1 可立即穿插(不触 invocation_transport);
  W2 等 stream-arm 收口;W3 的 CLI 半边等 Axon 半边合入后启动。

---

## §6 开放决策(执行前需 CTO 拍板,按阻塞面排序)

| # | 决策 | 阻塞 | 本规格默认 |
|---|---|---|---|
| D1 | review §0 的 A/B:discover/whoami/invoke 上不上顶层 | T1.2 | **A(上顶层)** |
| D2 | §6.2 跨仓提案现在就发给 Axon 排期? | T3.1 起点 | **发**(改动一天,阻塞面最大) |
| D3 | ratatui 作为仓库首个 TUI 依赖 | T2.4 | 引入(锁版本) |
| D4 | `cost` 金额字段(货币+定价源)本期做不做 | T3.1 范围 | **不做**,只做 usage |
| D5 | TUI `p pause` | T2.4 范围 | **一期砍掉**(协议无原语) |
| D6 | teach `--with-assets`(走 PayloadTransfer 传文件) | T3.3 范围 | **一期不做**,manifest-only |
| D7 | policy create 的裸表达式语法 | T3.2 范围 | 不做,引导式 flag |
| D8 | **trust 主体本体**:RFC-001 重述签名是 `{agent_ura}`,而 Axon 执法门吃 `node_trust_level`(resilience.rs:711)——主体是 agent 还是 node?两处现状不一致,需裁决 | T2.1/T2.2/W2-E2E-1 | 主体 = **agent URA**,daemon 维护 agent→node 投影供执法门消费(不发明 node URA 角色) |
| D9 | discover 联邦降级的退出码:0(优雅降级)还是非 0 | W1-E2E-2 | **0** + 类型化 envelope |
| D10 | **trust 命令面归属**(C15):既有 `trust show` 是 anchor 平面(admission keys),TrustLevel 是授权属性平面——扩展为 `trust level show/set` 子命令族,还是另立 noun?| T2.2 | **扩展既有组**:`trust level …`,组头部重写为两平面注释(anchor = 谁的 key 被接受 / level = 接受之后信到什么程度) |

---

## §7 条目→测试映射(防丢核对面)

| 任务 | e2e | 单测/快照 | 验收门 |
|---|---|---|---|
| T1.1 统一发现路径(+元组回显) | ✅ `seven_axes_w1_discover_e2e` 落地(真 UDS daemon:agent.list→ladder→双 scope→类型化降级→envelope 回显→**候选投影 + URA round-trip + 冻结分数逐位复算**)。**跨 owner user 层双 daemon e2e:判为成本已命名的 follow-up,非默认必做**——边际断言坐在三层已各有覆盖的机器上(投影:`federated_summary_candidate_requires_ability_ura` 等 2 单测含 RFC-005 拒绝;降级 envelope:本 e2e;bridge/admission 传输:cross_realm 套件),而 fixture 成本全套件最高(DendriteBridge + 真 hub + advertise 链)。如需补,挂 Axon 批次后随 hub 侧工作搭车 | 排序契约 4 测 + 投影 5 测 + JSON 契约冻结测,全绿 | W1 |
| T1.2 顶层 discover | 同上(execute() 共用实现,byte-identical 由构造保证) | — | W1 |
| T1.3 --tree 投影 | phase2 | 树分组单测 ✅ | W1 |
| T2.0 trace 锚点(v1.3 修正) | W2-E2E-2① | ✅ `pre_trace_id_meta_still_deserializes` + `in_flight_meta_carries_trace_anchor` | W2 |
| T2.1/2.2 trust 目录 ability + CLI | ✅ 实现 + e2e 双落地:`seven_axes_w2_trust_e2e` 覆盖 W2-E2E-1①(set=完整 invocation+envelope 回显)③(daemon 重启幸存)+ 线上拒绝面(菜单式坏等级、D8 非 agent 主体);共享 fixture 提取至 `tests/seven_axes_fixture/` | ✅ handler 5 测 + 持久化 2 测 + e2e 1 测,全绿 | W2 |
| T2.3 trust enforcement(Axon 门) | ⚠️ 核查发现:CLI 侧零代码喂 `node_trust_level`(grep 全空)——agent→node 投影与门的消费接线是 **Axon 侧/跨仓项**,与 T3.1 同批向 Axon 排期 | — | W2→W3 |
| T2.4 watch/TUI | ✅ 数据层 + e2e 双落地:`seven_axes_w2_watch_e2e` 覆盖账本投影→state 事件→terminal-ok(协议词表判终);fixture 接单句柄 ledger(sink 写 + history 读同一 Arc,daemon 重启共享);无 trace 的裸 unary = 单例因果集(诚实降解不拒绝);TUI 渲染层(D3)与 mission 多步流 follow-up 待补 | ✅ engine 3 测 + e2e 1 测 | W2 |
| T3.1 receipt usage | W3-E2E-1 | 验签篡改单测 | W3 |
| T3.2 policy 求值器(v1.4 重定界:CLI 半边已落地) | 🟡 W3-E2E-2 待补(simulate==binding 已单测锁定);`why` 等 §A6 门改线 | ✅ matcher 7 测(基线/越权拒绝+rule-id/trust 扣合/family 纯 scope/首条命中/dry-run 同函数/坏 envelope)+ 存储 2 测 | W3 |
| T3.3 teach/learn(v1:同设备 manifest-only,D6 默认) | ✅ 实现 + e2e 双落地:`seven_axes_w3_teach_learn_e2e` 覆盖 ①默认拒绝先行 ②学后双 URA 独立可发现(各自 exactly-one-owner)+ forget 后副本退场原件幸存 ③execution_mode 申明 sandbox_first;executor enforcement 另立里程碑 | ✅ handler 5 测 + 存储 2 测 + e2e 1 测 | W3 |
