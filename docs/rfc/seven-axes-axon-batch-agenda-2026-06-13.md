# seven-axes Axon 批次议程件:§A6 门改线 + receipt usage + trust 投影(2026-06-13)

> 状态:**proposed**(三张决策卡待 CTO 拍板;裁决后各议题在 EasyNet-Axon 开独立分支)。
> 来源:`docs/spec/seven-axes-p0-landing-v1.md`(v1.4)在本仓收敛后剩余的全部跨仓项。
> CLI 侧接缝已全部落地并有 e2e 锁定(分支 `seven-axes-p0-landing-v1`,整理后 8 commits,
> 6 个 e2e 文件全绿);本件是把这些接缝正式递到协议侧的交接物。
> 全部 file:line 已于 2026-06-13 对两仓磁盘亲核。

---

## 议题一:`InvocationReceipt.usage` 签名字段(T3.1 — ACCOUNT↔ECONOMIC 的第一块砖)

### 现状(亲核)

| 事实 | 证据 |
|---|---|
| receipt 无任何 usage/cost 字段 | `EasyNet-Axon/core/proto/axon/v1/invoke.proto` grep 仅注释命中 |
| `types.proto` 的 `cost` 是 0.0–1.0 调度权重,非计费量 | types.proto(七轴 review §6.2 亲核) |
| CLI 的 watch/TUI 数据层只能本地数 token,显示真实性契约已标 `unsigned` | `docs/spec/seven-axes-p0-landing-v1.md` §2 T2.4 表 |
| 不签名的 usage 不可审计不可计费 —— ECONOMIC 整轴(wallet/计费)无可信底座 | review §6.2 论证 |

### 提案

`InvocationReceipt` 增加:

```proto
message InvocationUsage {
  uint64 tokens_in       = 1;
  uint64 tokens_out      = 2;
  uint64 duration_ms     = 3;
  uint32 external_calls  = 4;   // 外呼次数;金额(cost)本批不做,见决策卡①
}
// InvocationReceipt 新字段,纳入 callee_signature 签名覆盖与 self_hash 哈希链
InvocationUsage usage = <next>;
```

- **七元组纪律合规**:usage 是 **receipt 输出**,不是第八个 invoke 参数——七字段不动,
  只丰富其产物(runtime-boundary "do not add an eighth primitive parameter" 的合规路径)。
- **签名覆盖是全部意义**:emit 时填充并入签;不进签名 = 白做。

### CLI 侧已铺好的接缝

- watch/TUI 的 `unsigned` 标注位:字段落地即去标(spec T2.4 显示真实性契约)。
- 验收已成文:spec §3 W3-E2E-1 ——核心断言是**篡改 usage 字节后验签必须失败**
  (账单可信的定义);离线第三方验签;零消耗调用 usage 为零值非缺失。

### 决策卡①

1. `cost`(金额,需货币与定价源)是否随批?——**推荐:不随**(spec D4 默认),
   usage 先签,金额属 ECONOMIC 轴产品层。
2. `usage` 落 receipt 的哪个区段——state-machine fields(1..12)还是 metadata 区?
   推荐 metadata 区(不扰动终态机字段段)。

---

## 议题二:§A6 门改线 —— admission gate 调用 `policy.evaluate`

### 现状(亲核)

| 事实 | 证据 |
|---|---|
| 设计拓扑早已写明:"kernel admission gate is supposed to call policy.evaluate as an in-process sub-invocation carrying `admission_internal=true`" | `EasyNet-Cli/src/runtime/agents/policy_ability.rs` 模块头(RFC §A6) |
| daemon 侧求值器已是真的:tiny matcher(action/family-prefix/trust-below 三谓词,首条命中,空库=说出口的 baseline-allow),规则库 `policy-rules.json`,`evaluate`/`simulate` 共享单一 `decide()` | policy_ability.rs + persistence/policy_rules.rs,7 单测 + 1 e2e 全绿 |
| 今天的门绕过它:直连 PermissionService / consent broker | policy_ability.rs 模块头自述 |
| Axon resilience 仍有两条硬编码 trust 检查(install≥PRIVILEGED / admin≥ELEVATED) | `EasyNet-Axon/core/runtime-rs/src/runtime/resilience.rs:711,715` |

### 提案

1. kernel admission gate 经 §A6 调用 `policy.evaluate`(`admission_internal=true`,
   原 envelope 为 args)。kernel 热路径改动,需独立 review——这正是当初"landing the
   callable surface first"所预留的里程碑。
2. resilience.rs:711/715 的两条硬编码检查改写为同引擎**内置规则**——行为逐字节不变;
   CLI 侧回归断言已写好待接(spec §3 W3-E2E-2 ⑤)。
3. 门的每个决策**入账**(decision 记录)→ 解锁 `policy why <invocation-id>`
   (CLI 侧 deliberately-absent,拒绝"对昨天的调用重放今天的规则"——
   见 `facade/cli/policy_cli.rs` 头部)。

### CLI 侧已铺好的接缝

- `decide()` 单源保证 dry-run 与 binding 永不漂移(已有单测+e2e 双锁)。
- 信任谓词经 `trust_ability::level_rank`(pb TrustLevel 单源)消费信任目录——
  门改线后 trust 检查自动获得目录数据(见议题三)。

### 决策卡②

1. 改线时机与守门:kernel 热路径,建议独立 PR + 性能回归门(decide() 含磁盘读,
   需决策缓存——evaluate 响应已带 `expires_at`/TTL 字段,缓存语义已 pin)。
2. decision 记录落 receipt(`deny_reasons` 扩展)还是独立 decision log?
   推荐前者:receipt 是唯一账本,第二审计面违反单源纪律。

---

## 议题三:T2.3 trust 投影 —— **推荐裁决:被议题二吸收**

### 现状(亲核,含一个 D8 分叉证据)

| 事实 | 证据 |
|---|---|
| CLI 信任目录在产:`trust-levels.json`(主体=Agent URA,D8 默认)+ `identity.get_trust/set_trust` + `trust level show/set`,e2e 锁定(含 daemon 重启幸存) | trust_ability.rs / trust_levels.rs / seven_axes_w2_trust_e2e |
| RFC-001 重述表给 get_trust 的签名是 `{agent_ura}` | RFC-001 mapping(七轴 spec D8 援引) |
| Axon 执法门吃的却是 `node_trust_level` | resilience.rs:711,715 |
| CLI 全仓零代码喂 node_trust_level | grep 全空(spec §7 T2.3 行记录) |

### 提案(推荐:吸收而非搭桥)

原 T2.3 设想"daemon 维护 agent→node 投影喂给 resilience 门"。**但议题二完成后这条
投影根本不需要存在**:

- 门改线后,trust 检查 = policy 规则(`trust_below` 谓词),求值时按 **caller agent**
  直接查信任目录(`trust_ability::effective_level`)——主体天然是 agent,与 RFC-001
  重述签名一致,D8 分叉自动愈合;
- `node_trust_level` 路径随两条硬编码门一起退役——无需新建一条注定要拆的投影管道。

### 决策卡③

1. 接受"议题三被议题二吸收"?——**推荐接受**:少建一条过渡管道,D8 分叉
   (agent_ura vs node)以"门侧也归一到 agent 主体"收口。
2. 若不接受(node 级 trust 有独立存在理由,如宿主机健康降级):需先给出 node trust
   的本体定义(它不是 agent trust 的聚合是什么?)再立项——**flag 而非外推**。

---

## 验收总表(全部已在 CLI 侧成文/成码,协议侧落地即接)

| 议题 | 验收 | 所在 |
|---|---|---|
| 一 | 篡改 usage 验签必败 / 离线验签 / 零值非缺失 / 七元组形状不变 | spec §3 W3-E2E-1 |
| 二 | deny 带 rule-id / simulate==binding / C6 改写后行为逐字节不变 | policy 7 单测 + seven_axes_w3_policy_e2e + spec W3-E2E-2 ⑤ |
| 三 | (吸收后)trust 抬升跨 wire 改变 admission 结果 —— 同款断言已在 simulate 路径绿 | seven_axes_w3_policy_e2e 的 trust-interlock 断言 |
