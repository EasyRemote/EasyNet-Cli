# Workspace 化 crate 依赖图设计(T4.1 设计件,2026-06-12)

> spec T4.1 前置:「crate 依赖图先画后切——op_event! 宏、config 类型、error 类型等
> 横切面的归属 crate 先定,否则切到一半发现环依赖回退重来」。
> 方法:Explore agent 全量边扫描 + 三个命门事实亲手核验(含一个 agent 漏报的生产环)。
> 验收:依赖图经评审无环(本文档 §3 的预搬完成后)。

## 1. 现状:模块矩阵(12 顶层模块,~377k 行)

生产依赖边(已剔除 13 个 `#[cfg(test)]` test_support 假边):

```
ura(319 行)、core(8.7k)、daemon(3.0k)     → 无内部依赖(天然叶子)
persistence(10.0k) → runtime(仅 runtime::domain::TenantId 一条类型边)【亲核】
support(18.3k)     → core, persistence, registry, services(federation_invoke.rs 一条)【亲核】
eal(14.1k)         → core, registry, runtime, support
registry(3.5k)     → core, persistence, runtime
services(77.3k)    → core, persistence, registry, runtime, support
runtime(177.6k)    → core, eal, persistence, registry, services, support, ura
facade(38.3k)      → 几乎全部(顶层装配)
ffi(8.8k)          → daemon, services
```

## 2. 环清单(核验后定性)

| 环 | 性质 | 解法 |
|---|---|---|
| runtime ↔ services | **真生产环,agent 漏报、亲核坐实**:runtime/advertise.rs:503 与 runtime/agents/meta_ability.rs:373/:399 经 `hub_published_ability_store::global()` / `ability_health::snapshot()` 反向伸进 services | **环的根因 = F-006 的 OnceLock 全局单例**(全局函数让 runtime 绕过注入直达 services;ability_health 新特性沿同模式又加了一条)。T4.3 注入化(store/health 句柄随 boot 构造传入)即斩环——**T4.3 从工程卫生升格为切 crate 的硬前置** |
| persistence ↔ facade / registry ↔ facade / runtime ↔ facade | 13 文件全为 `#[cfg(test)]` 引 test_support::HomeGuard(抽验 persistence/config.rs ✓) | 预搬 §3-c:HomeGuard 家族独立成 dev-dependency crate,假边整体消亡 |
| support ↔ services | support/federation_invoke.rs 引 ProtoEnvelope(单文件错位) | 预搬 §3-b:该文件本属 transport,搬回 services |
| persistence → runtime | 单条类型边(TenantId) | 预搬 §3-a:domain 类型下沉 |

**结论:生产图在三个小预搬后无环**——分层是真实存在的,只是被单例、错位文件和测试助手糊住了。

## 3. 预搬批(全部 S 级,workspace 切割前完成)

- **a. `easynet-domain` 萃取**:src/runtime/domain(TenantId/NodeId/ScheduleId 等纯类型)
  + core 的 agent_id/agent_spec → 新叶子 crate。persistence→runtime 边消亡。
- **b. federation_invoke.rs 归位**:src/support → src/services/invocation_transport。
  support→services 边消亡。
- **c. `easynet-test-support` dev crate**:HomeGuard 家族出 facade。13 条 cfg(test)
  假边消亡;各 crate 以 dev-dependency 引用(dev 依赖不参与发布图,允许向上引用)。
- **d. T4.3 注入化**(已在 spec,M 级,此处确认其新角色):6 个 global() 消费点 +
  ability_health 快照面改注入 → runtime→services 边消亡。**这是唯一的 M 级前置。**

## 4. 目标 crate 图(预搬后,无环)

```
tier 0  easynet-domain      (纯类型,proto-free)
        easynet-ura         (Axon ura 门面,proto-free)
        easynet-daemon-ipc  (daemon/ 协议+客户端)
tier 1  easynet-persistence (→ domain)                 ← 首切验证件
        easynet-support     (op_event/operator_log 等 → domain, persistence)
tier 2  easynet-transport   (services/ → 以上全部)      ← pb 重心(25 文件)
tier 3  easynet-runtime     (runtime/ + eal + registry + plugins → 以上)
tier 4  easynet-facade      (facade/ + ffi + 三个 bin → 全部)
```

### 横切面归属
- **op_event!**:`#[macro_export]` 留 easynet-support;跨 crate 消费改
  `use easynet_support::op_event`(283 调用点,机械替换;runtime/services 占 270)。
- **error 类型**:核验确认**无统一错误类型**——各模块自有 typed error,天然随模块
  入各自 crate,零横切冲突(这是过去 typed-error 纪律的红利)。
- **axon-pb 特性**:pb 依赖集中 transport(25 文件)/facade(17)/ffi(2);
  **domain/persistence/core/registry/plugins 零 pb** → tier 0-1 crate 永不拉
  tonic/prost,增量构建收益最大化(F-010 的 OOM 解药首先在这里兑现)。
  eal(1 文件)/daemon(1)/runtime(4)/support(4)的零星 pb 引用在切割时逐文件
  评估:能下沉 transport 的下沉,否则特性门控留在各 crate。

## 5. 切割序与验收(每刀必量)

| 刀 | 内容 | 验收 |
|---|---|---|
| 0 | 预搬 a/b/c(+d 已排 T4.3) | 全测试零回归;生产图 cargo-deps 无环证明落档 |
| 1 | persistence 首切 | **增量链接时间/内存 before-after 数字落档**(F-010 验收);CI 双特性矩阵不变 |
| 2 | domain/ura/daemon-ipc/support 叶子批 | 同上累积测量 |
| 3 | transport 切(等 T4.3) | pb 编译只发生在 tier 2+;改 persistence 一行不再重链 tonic |
| 4 | runtime/facade 终切 | 全仓 god-file 拆分(T4.2/T4.4/T4.6)与本刀互不阻塞,先后皆可 |

## 6. 风险与开放点(评审)

1. runtime(177.6k)单 crate 仍然巨大——tier 3 内部是否再分(agents/kernel/federation)
   留待 T4.2/T4.4 拆完后按编译热点二次评估,不在本批强切。
2. 13 条 test_support 假边的 cfg(test) 定性系 agent 报告 + 1 例抽验;执行刀 0-c 时
   逐文件复核(铁律:agent 论断动手前亲核)。
3. bench/(另一会话在建)与 tests/ 的归属:随 facade(集成面)走,刀 4 时定。
