# RFC-005 架构判断：device-local authority 缺失 — 2026-06-08

> 触发：`easynet agent add anthropic` → `agent.start` 报
> `ROUTE_NEGATIVE / NODATA: owner is online but does not publish the requested ability`，
> 同时 Abilities 页 `0 total surfaces`。
> 结论：不是漏调一行，是 RFC-005 落地时把"权威模型"实现反了。

---

## 1. 一句话诊断

resolver 把 **hub 的 `AbilityCatalogStore`（rendezvous 投影缓存）当成了 device-owned ability
是否存在的唯一判据**。但 RFC-005（§10.1 storage classes、§4 ZoneId::Owner、D44、D105）明确：
**device-local daemon/runtime 才是 device-owned ability 的权威；hub projection 只是 signed、
lease-bound 的发现投影，不是真相源，也不是 route 授权。**

`agent.start` 一直注册在本地 Axon LocalRuntime registry 里（`OwnerKind::Device`），
**设备解析自己的能力本不该去问 hub 缓存**。

---

## 2. 证据链（已对照磁盘验证，非过时 Read）

### 2.1 resolver 只有一条路：查 catalog
`route_resolver.rs:384-431` — `resolve_route` 调 `handle_resolve_at(include_abilities=true)`，
其 `owner.abilities` 来源是 `AbilityCatalogStore`（`federation_wrappers.rs:523
resolved_owner_projection_values`）。catalog 里没有 `agent.start` 这条 summary → `NODATA`。

文件头自己写明了这个（错误的）设计：
- `route_resolver.rs:21` *"Negative answers are typed; no legacy catalog fallback is consulted."*
- `route_resolver.rs:19` *"A route may dispatch only when release_profile >= AuthoritativeLocal"*

### 2.2 真正的执行器用的是本地 runtime registry，不是 catalog
`dispatch_local_rpc_selected_route`（`daemon_invocation_service.rs:1481`）：
1. `resolve_local_rpc_route` → `resolve_route` → **NODATA，死在这里**（line 1465）。
2. **假如通过**，line 1501 `runtime.ability_options(&dispatch_key)` —— 真正执行靠的是
   **本地 Axon LocalRuntime registry**。`agent.start` handler 早就注册在那（registry/dispatch table）。

> 即：device-local authority（本地 registry）**已经是实际执行者**。resolver 那一步纯粹是个
> **gate**，却去查了错误的源（空的 hub catalog），把本地有的能力判成"不存在"。

### 2.3 self-target 判定 + 本地 ability 枚举 API 都已存在
- `matches_self_target_ura`（`daemon_invocation_service.rs:776`）已能判"这个 target 是不是本机"。
- 同一函数 `line 805` 已经在用 `runtime.list_abilities().await` 把本地 registry 当作自身权威枚举。

→ device-local authority 需要的两个原料（"是不是我" + "我注册了哪些 ability"）**代码里都有现成的**，
只是没接进 resolver。

### 2.4 device 发布也只是 best-effort（次要问题）
`session_initiator.rs:585-622` 在 bidi 开了之后软发一次 `federation.advertise_abilities`，
失败静默、catalog 空。`advertise_self_signed_device`（`advertise.rs:568`）只发身份不发 ability。
→ 即便"要靠 catalog"，发布也非确定性。但按 §3 结论，**设备解析自己根本不该依赖 catalog**，
这条只对"别的设备/backend 发现这台设备"有意义。

---

## 3. spec 怎么说（决定方向）

| 问题 | spec 答案 | 出处 |
|---|---|---|
| 谁是 device-owned ability 的权威？ | **device-local daemon/runtime store** | §10.1 storage classes；§4 ZoneId::Owner；D44 |
| hub projection 是什么？ | signed、lease-bound 的 rendezvous/discovery 投影，**not source of truth, not route authority** | §10.1；§4 |
| projection 空时 resolver 该怎样？ | 仍应从 device-local authority 出 `ABILITY`；`ROUTE` 取决于"device 自己有没有 runtime-local dispatch binding" | **D105** |
| `agent.start`/`skill.list` 归谁？ | device-owned（`device.<id>.<ns>.<local>`） | §1、§3.1 |
| `AuthoritativeLocal` 前提？ | WAL replay + snapshot + projection-fence + **device-local authority gates** 全过 | §10.3 state rule #3 |

D105 原文（smoking gun）：
> "The namespace directory may prove a device-owned `ABILITY` from owner projection **or
> compatibility inventory**, but it emits an executable `ROUTE` only when **the owner device
> itself has a matching runtime-local dispatch binding**."

→ 当前实现违反 D105 两处：(a) 不从 device-local inventory 证 ABILITY；
(b) 把 ROUTE 与 catalog 绑死，而非与"本地 dispatch binding"绑定。

---

## 4. 暴露的三个架构问题（按严重度）

1. **【架构】resolver 缺 device-local authority 层（D105 的 `DeviceLocalNamespaceAuthority`）。**
   resolver 只会查 catalog；对"本机 device 解析自己的 ability"应直接以本地 runtime registry 为权威，
   ABILITY 存在性 + ROUTE 可达性都从那里出，**完全绕过 catalog**。这是缺的那一层。

2. **【纪律/越界】`route_resolver.rs:462` 硬编码 `AuthoritativeLocal` 并独占 dispatch、无回退。**
   而 `AuthoritativeLocal` 的前提"device-local authority gates"在代码里根本不存在 → 越界宣称，
   且移除了 ShadowRead 安全垫，导致 fail 很硬。

3. **【可靠性】device 发布 catalog 是 best-effort、非确定性。**
   "设备在线但 0 ability"成为可复现稳态。修复后应定位它为"对外 rendezvous 投影"，
   **不再作为本机可达性前提**。

---

## 5. 关键洞察：两种"解析"被混为一谈

spec 把解析分成两个本质不同的场景，当前代码两条都走同一个 `AbilityCatalogStore` 查询：

| 场景 | 正确权威 | 当前实现 | 后果 |
|---|---|---|---|
| **设备解析自己**（agent.start 这种） | device-local runtime registry | 查 hub catalog | 连自己的能力都要先抄到 hub 缓存才能用 ← 根本错误 |
| **backend/别的设备解析这台设备**（Abilities 页） | hub projection（rendezvous） | 查 hub catalog | projection 空时应报"投影缺失/stale 可刷新"，而非"ability 不存在" |

---

## 6. 已对齐的方向决策（2026-06-08）

1. **device 解析自己 → 建 device-local authority 层（D105）**：resolver 在 `owner == 本机 device`
   时直接以本地 runtime registry（`runtime.list_abilities()` / `ability_options`）为权威，
   ABILITY+ROUTE 从本地出，绕过 catalog。这一层一旦建好，**它就是** `AuthoritativeLocal` 要求的
   "device-local authority gate"。
2. **`route_resolver.rs:462` → 降回 ShadowRead/兼容路径**：在 device-local authority gate 真正
   就位前不宣称 `AuthoritativeLocal`，保留兼容 dispatch 兜底。

> 两决策自洽：authority 层就位 = gate 满足 = 可诚实升 profile；未就位 = 诚实停 ShadowRead。

---

## 7. 改动范围预判（待你确认后再动手）

- **核心**：`route_resolver.rs` 新增 device-local authority 分支（owner==self → 查本地 registry）；
  复用已存在的 `matches_self_target_ura` / `runtime.list_abilities`。改动集中、可测。
- **profile**：移除 `route_resolver.rs:462` 硬编码，按 gate 状态产出 profile；invoke 路径
  （`daemon_invocation_service.rs:1082/1468`）的 `is_authoritative_local_or_better` gate 相应调整或保留兼容回退。
- **发布**：device 自发布改确定性 + heartbeat 补发（次优先，可后置），定位为对外 rendezvous。
- **不碰**：Axon 协议、backend、URA 语法。这是纯 Cli daemon runtime 层的修复。
