# EasyNet 后端边界审计报告 — Hub/Device 生命周期 feature 的仓库归属

**日期:** 2026-06-08
**审计人:** Claude (Opus 4.8) + 15-file workflow(23 agents,对抗核实)
**触发:** CTO 质疑 —— "EasyNet 仓作为 Hub 的功能实际上是 CLI 和 Axon 实现的,按此推理 EasyNet 仓在这次更新计划里边界没想充分"
**裁决基准:** `easynet-runtime-boundary` skill(Axon 拥有协议契约;CLI daemon 拥有 device/Hub 进程与 ability;后端拥有产品 DB/HTTP,经 daemon.sock 提交完整 Invocation)

---

## 0. 结论(TL;DR)

**质疑成立。** EasyNet 后端 `backend/internal/axon/`(15 个文件,~4000 行 Go)里,**8 个文件是协议 fork**(高 drift),在 Go 里重新实现了本应由 Axon 拥有的契约:envelope canonicalization、admission、URA grammar、delegation、session-authority、enum 名表。

最关键的事实:

1. **Axon 已经发布了 Go SDK**(`EasyNet-Axon/sdk/go/easynet/`:`ura.go`、`signed_invoke_request.go`、`invocation/`),且**后端 go.mod 已经 import 它**(`easynet.run/axon/sdk/go v0.41.5` + 本地 `replace`)。
2. 但**只有 `urns.go` 真正用了它**(39 处引用);其余 14 个文件**零委托**,各自 fork。
3. 这就是 `agent_uri` vs `agent_ura` 漂移的根:后端那份 fork 与 Axon SDK 的字段名/shape 已经不一致,而且**没有任何编译期信号**——Axon 改契约,后端 Go fork 静默漂移,直到运行时报 `unknown field` / `SUBJECT_KEY_UNREGISTERED` / device REMOVED。
4. **反向也成立:迁移断层是双向的。** 对抗核实发现,有 3 个 fork(`admission` 的 subject-kind 谓词、`session_authority`、`enums` 的 name 表)**目前 Axon Go SDK 还没 export 对应符号**——所以后端不是"懒得用 SDK",而是**SDK 还没把这些契约提供出来**。计划要求后端委托,但上游没备齐。

**分类分布:** protocol-fork=8 · thin-wrapper=2 · product-logic=4 · retired-shim=1 | drift: high=8 · medium=3 · low=3 · none=1

---

## 1. 生命周期 Feature × 仓库归属总表

> 列含义:**现状** = 今天代码实际怎么实现;**应归属** = 按边界规则该谁实现;**怎么实现** = clean-target 形态;**drift** = Axon 演进时后端会不会静默坏。

| # | 生命周期 Feature | 后端现状文件 | 现状(怎么实现的) | 按理应由谁实现 | clean-target 怎么实现 | drift |
|---|---|---|---|---|---|---|
| F1 | **Identity / 信任注册**(backend self-identity → axon-runtime trusted_keys) | `bootstrap_self_identity.go` | 后端构造 `runtime.bootstrap_self_identity` Invocation,经 daemon.sock 发 | **后端**(合法 transport-binding)— ability 由 **CLI daemon** 实现(`runtime.bootstrap_self_identity` 已存在) | 保留后端薄封装;envelope/签名应换成 Axon SDK `SignedInvokeRequest`+`SigningConfig`(见 §3 nuance) | high* |
| F2 | **Admission / subject 准入**(subject URA kind ∈ {user,device,agent,resource}) | `admission.go` | 后端 Go 硬编码 §5.2 subject-kind 谓词 + env 开关 | **Axon**(谓词)+ 后端(rollout 开关) | Axon SDK 导出 subject-kind 谓词;后端只留 `EASYNET_ADMISSION_SUBJECT_ENFORCE` soak 开关 | **high** |
| F3 | **Federation 目录广播**(advertise_agent/abilities) | `advertise.go` | 后端 fork envelope+causal+signing-authority+payload | **Axon SDK**(已有 `FederationAdvertiseAgent/Abilities`) | 删 fork,调 SDK `DendriteBridge.FederationAdvertise*` | **high** |
| F4 | **Namespace/Federation 解析**(device state / agent list 的 directory resolve) | `federation_calls.go` | 后端曾 fork envelope+session-authority,调 `federation.resolve`;当前本地 read model 已切到 daemon `namespace.resolve` | **mixed**:wire/answer→Axon SDK/proto;ability→**CLI daemon**(`namespace.resolve`) | 后端只构造产品查询参数并消费 typed `ResolveAnswer`;不直连 Axon runtime | **high** |
| F5 | **Delegation 证明**(用户授权链) | `delegation.go` | 后端 fork `DelegationProof` shape + 规范字节 | **Axon SDK** | 换 SDK delegation 类型 | **high** |
| F6 | **Session Authority**(backend 签名的会话授权) | `session_authority.go` | 后端 fork SessionAuthority canonical payload | **Axon**(目前 SDK 未 export) | Axon SDK 先实现 `SessionAuthority` Go 类型,再换 | **high** |
| F7 | **Envelope 类型**(7-tuple invoke 的 Go 表示) | `invoke_types.go` | 后端 fork `Envelope/AgentRef/SubjectRef/CausalContext/InvokeRequest` | **Axon SDK**(已有 `SignedInvokeRequest`/`SignedCausalContext`/四个 Causal 构造子) | 删 fork,统一用 SDK | **high** |
| F8 | **URA 语法**(DeviceURA/HubURA/AgentURA/解析) | `urns.go` | **部分已委托** SDK(39 引用)+ 残留本地 builder | **Axon SDK**(残留部分) | 收口残留 builder/namespace 校验到 SDK | medium |
| F9 | **Enum 名表**(NodeState/TrustLevel/VoiceTransport) | `enums.go` | 后端手维护 ordinal→name 映射 | **Axon**(目前 SDK 未生成) | Axon SDK 从 proto 生成导出,后端删本地表 | **high** |
| F10 | **Invoke 传输接口**(窄 InvokeClient) | `invoke_client.go` | 仅声明 Invoke/Stream/Close 接口 | **后端**(合法) | 保留 | low |
| F11 | **descriptor 投影**(federation.resolve 结果→后端类型) | `ability_descriptor_reader.go`, `node_mapper.go`, `resolved_agents.go` | 后端解析 receipt JSON → 产品类型 | **后端**(产品投影) | 保留;字段名跟 SDK proto 对齐 | medium |
| F12 | **device 产品状态**(state mapper / DB) | `device_state.go`(logic 包) | `federationStatusToDeviceState`: active→ONLINE,else→REMOVED | **后端**(产品语义) | 保留;但需修静默 fallthrough(见 §4) | — |
| F13 | **legacy 已退休面**(ExecCommand 等) | `retired.go` | 显式占位待迁移 | **后端**(过渡) | 按计划逐个补 ability 后删 | none |

\* F1 drift 标 high 是 workflow 初判;对抗核实**推翻**为 low(它是合法 transport-binding,不是 fork)——详见 §3。

---

## 2. 核心判断:边界哪里对、哪里错

### ✅ 对的部分(质疑需要精确定性,不能一棍子打死)

- **传输层正确**:后端经 `~/.easynet/daemon.sock`(easynet-daemon)发 Invocation,**不直连 axon-runtime**。符合 skill 的 Backend Usage 范式。
- **调用形态正确**:`namespace.resolve` / `federation.resolve` / `runtime.bootstrap_self_identity` 是普通 daemon ability,后端经 daemon.sock 调它们是对的。
- **ability 归属正确**:这些 hub/device 能力**确实由 CLI daemon 实现**(已验证:`namespace.resolve/federation.resolve/advertise_agent/advertise_abilities/forward_invoke/heartbeat/resolve_key` + `runtime.bootstrap_self_identity/register_local_tool` 全在 CLI runtime/daemon dispatch 面)。

### ❌ 错的部分(质疑的实质)

**后端在 Go 里 fork 了一份 Axon 协议实现的中间层**:envelope 构造、admission 谓词、URA 语法、delegation、session-authority、enum 表。这违反 skill Rule 1("URA parsing, canonicalization, signatures, admission, receipt verification, federation wire invariants → 必须在 Axon")和明确禁令("Reject designs that duplicate protocol contracts in CLI/backend when Axon should own them")。

**这次更新计划没把后端这片协议 fork 列入迁移**,所以:
- 后端持续与 Axon/CLI 协议契约漂移(`agent_uri`/`agent_ura` 只是第一个暴露);
- 每次 Axon 演进 envelope/URA/admission,后端 Go fork 要手动追平,追不上就出现"看似 bug、实则契约漂移"的症状(REMOVED、UNREGISTERED、unknown field)。

---

## 3. 对抗核实揭示的双向断层(关键 nuance)

workflow 对每个 protocol-fork 去 Axon Go SDK 核实"替代符号是否真存在"。结果**不是单纯'后端该用 SDK'**:

| 文件 | 初判 | 核实结果 | 真相 |
|---|---|---|---|
| `bootstrap_self_identity.go` | protocol-fork | **推翻** | 是合法 transport-binding(typed input → daemon_grpc protobuf),**保留**。SDK 符号存在但不覆盖此 call site。 |
| `admission.go` | fork/Axon | 站得住,**但** `sdk_exists=False` | subject-kind 谓词 **Axon SDK 还没实现**。`ValidateEnvelope` 只查 subject 非空,无 kind 校验。**得先在 Axon 加**。 |
| `session_authority.go` | fork/Axon | 站得住,**但** `sdk_exists=False` | Go SDK **无** `SessionAuthority` 类型(只有 proto + Rust)。**得先在 Axon Go SDK 实现**。 |
| `enums.go` | fork/Axon | 站得住,**但** `sdk_exists=False` | Go SDK **没从 proto 生成** NodeState/TrustLevel name 表。**得先在 Axon 生成导出**。 |
| `advertise.go` / `invoke_types.go` / `delegation.go` / `federation_calls.go` | fork | 站得住,`sdk_exists=True` | SDK 已有替代符号,**后端可直接迁**。 |

**断层是双向的:** 一部分是"后端没用已有的 SDK"(F3/F5/F7/F4),另一部分是"Axon SDK 还没把契约 export 出来"(F2/F6/F9)。**这正说明计划在三仓协同上没拉通**——既没安排后端迁移,也没安排 Axon Go SDK 补齐缺口。

---

## 4. 与 REMOVED 症状的关联 + 一处诚实纠正

**纠正(重要):** 我先前基于一次 **stale 源码读取**,误判 REMOVED 根因是 CLI 侧 `StoredDeviceIdentity` 用 `#[serde(deny_unknown_fields)]`。复核磁盘真实状态后:**当前源码已无此 bug**(已是 `RejectedTenantId` 方案,能正常解析你的 credentials.json,提交 `d7d3cdf`)。你跑的是 `/usr/local/bin/easynet-daemon`(`01:55` 安装的旧二进制),REMOVED 来自旧二进制 + 下列后端侧成因,非当前 CLI 源码。我未改动你的运行环境。

REMOVED 的**真实成因链**(与本审计直接相关):
1. **后端 `device_state.go`**:`item.State` 初值 `"REMOVED"`,`ResolveAgents` 失败时 `if axonErr == nil` **完全静默**(无 log、不改 state)→ 保持 REMOVED。这是产品侧的 fail-silent bug。
2. **后端 self-identity fork 漂移**:日志 `backend_identity_trust_upsert_failed: unknown field 'agent_uri', expected 'agent_ura'`(`~/.easynet-hub/localhost/identity.json`)→ backend self-identity 没注册 → directory resolve 被拒(`SUBJECT_KEY_UNREGISTERED`)→ `ResolveAgents` 报错 → 走静默 fallthrough → REMOVED。**这就是 F1/F8 协议 fork 漂移的直接后果。**

---

## 5. 更新计划 + Commit 安排

### 阶段 0 — 止血(已完成 / 你自己更新二进制)
- CLI 侧 `StoredDeviceIdentity` 修复已在 `d7d3cdf`(HEAD)。你更新 `/usr/local/bin/easynet-daemon` 到当前构建即可让设备恢复(假设 §4.2 后端侧也修)。

### 阶段 1 — 后端产品侧 fail-silent 修复(EasyNet 仓,独立可立即做)
- **C1**: `getDeviceLogic.go` — `ResolveAgents` 报错时 `logx.Errorf` 而非静默;REMOVED 区分"确认离线" vs "解析失败"两种语义。
- **C2**: `device_state.go` — `federationStatusToDeviceState` 默认分支加注释 + 让 caller 能区分 unknown-status 与 resolve-failure。

### 阶段 2 — 修 self-identity 契约漂移(止 REMOVED 第二成因,EasyNet 仓)
- **C3**: 后端 self-identity 文件读写统一用 `agent_ura`(消除 `agent_uri`),与 Axon/CLI 的 `~/.easynet-hub/<realm>/identity.json` schema 对齐。这是 F1/F8 漂移的具体修复点。

### 阶段 3 — Axon Go SDK 补齐缺口(EasyNet-Axon 仓,解锁后端迁移)
- **C4**: Axon Go SDK 导出 subject-kind 谓词(F2)。
- **C5**: Axon Go SDK 实现 `SessionAuthority` Go 类型 + canonical payload(F6)。
- **C6**: Axon Go SDK 从 `axon.v1` proto 生成 NodeState/TrustLevel/VoiceTransport name 表(F9)。

### 阶段 4 — 后端协议 fork → SDK 委托(EasyNet 仓,SDK 已备齐的先做)
- **C7**: `invoke_types.go` → Axon SDK `SignedInvokeRequest`/`SignedCausalContext`(F7,SDK 已有)。
- **C8**: `advertise.go` → SDK `FederationAdvertiseAgent/Abilities`(F3,SDK 已有)。
- **C9**: `delegation.go` → SDK delegation 类型(F5,SDK 已有)。
- **C10**: `federation_calls.go` envelope 部分 → SDK(F4,SDK 已有);保留业务参数构造。
- **C11**: `admission.go` → SDK subject-kind 谓词(F2,**依赖 C4**);保留 env soak 开关。
- **C12**: `session_authority.go` → SDK(F6,**依赖 C5**)。
- **C13**: `enums.go` → SDK 生成表(F9,**依赖 C6**);删本地表。
- **C14**: `urns.go` 残留 builder/namespace 校验收口到 SDK(F8)。

### 阶段 5 — 收尾
- **C15**: 删 `retired.go` 中已补 ability 的项;更新跨仓迁移 ledger。

**Commit 纪律(按你的 memory 约定):**
- 1 commit = 1 逻辑变更;显式 pathspec(`git commit -- <paths>`,共享 index);无 `Co-Authored-By`;作者 Silan.Hu。
- 跨仓 commit 不混在一个 PR;Axon(C4-C6)→ 后端(C7-C14)有依赖序。
- 阶段 1/2(止血)可独立先合;阶段 3 是阶段 4 部分项的前置。

### 依赖序图
```
止血:  C1,C2 (后端产品侧)  ──┐
       C3   (后端identity漂移)─┤→ 设备恢复 ONLINE
                              │
Axon:  C4,C5,C6 ──────────────┼──→ 解锁 ─→ C11,C12,C13
后端:  C7,C8,C9,C10,C14 ──────┘(SDK已备齐，可并行)──→ C15
```

### 2026-06-08 实施进度补记

本审计报告触发后的实现已经推进到 backend/Axon SDK/CLI daemon 的主要协议 fork 收口阶段。已验证完成的事实包括:

- Axon Go SDK 已补齐 backend 迁移依赖: subject-kind predicate、SessionAuthority、DelegationProof、invoke-remote frame codec、federation payload builders、resource namespace/URA projection helpers。
- EasyNet backend 已把 admission、SessionAuthority、DelegationProof、invoke envelope/types、federation resolve/revoke/advertise payload、invoke-remote frame-0、resource namespace/display projection迁到 Axon Go SDK 或 SDK facade。
- EasyNet backend 已从 `EasyNet-Axon/core/proto` 生成 Axon namespace Go 绑定(`namespace.pb.go` / `namespace_grpc.pb.go`),`ResolveDirectoryFailure` 内部使用 `axon.v1.NegativeReason` 包装,API 出口只投影短 reason 字符串,不再维护本地 ordinal 镜像。
- EasyNet backend 已新增 namespace `ResolveAnswer` 产品投影: `answer_kind`、`next_hop`、`selected_route`、`route_evidence`、`records`、`release_profile`、`authority`、`cache_policy` 和 typed `negative` 均由 generated proto 读取;directory facade 收到 namespace negative 时进入同一 `ResolveDirectoryFailure` lane,positive route answer 不会被误当成空 directory。
- EasyNet backend 已移除从 host placement 反推 device-owned ability identity 的路径: `NormalizeInvokeTarget` 不再做 ownership inference, `DeviceOwnedAbilityURAForTarget` 已删除。
- `agent.list` 和 `skill.list` 已改为消费 `namespace.resolve` 返回的 typed `ResolveAnswer.records`: backend 读取 ABILITY record 的 `owner_ura` / `ability_ura`, 只把 ID record 的 device id 当 placement hint。
- EasyNet backend 已新增 `ResolveDirectoryAnswer` typed facade: 本地 `namespace.resolve`、peer fanout `namespace.proxy_resolve` 的正向目录结果和 typed negative/failure 结果分流; `NXDOMAIN` / `NODATA` / `NOROUTE` / `UNAUTHORIZED` / `STALE` 等原因码不再需要从人类 `reason` 字符串里解析。旧 `ResolveAgents` 仍只是 unwrap facade。
- EasyNet backend `aggregate.*` ability catalog read model 已改为消费 `ResolveAgentsAnswer`,并通过 `errors.As(*axon.ResolveDirectoryFailure)` 保留 typed negative 给后续 HTTP/Frontend 渲染层。
- EasyNet `/api/v1/abilities` 已扩展 `resolve_unavailable[]`: primary `namespace.resolve` 返回 typed negative 时不再继续 dispatch `meta.list_abilities`,也不把 negative 压成普通 5xx 字符串;响应保留 `source/reason/query_name/message/code/stage/retryable` 给前端渲染。
- EasyNet `/api/v1/agents` 和 `/api/v1/skills/installed` 已扩展/接入 `resolve_unavailable[]`: resolver negative、hub rejection、transport failure 不再伪装成健康空列表或直接 5xx;前端 Agents/Skills 页面会显示 typed resolver 状态。
- EasyNet `/api/v1/devices`、`/api/v1/devices/:id` 和 `/api/v1/devices/:id/abilities` 已扩展/接入 `resolve_unavailable[]`: device live-state resolve 或 host ability resolve 返回 typed negative/transport failure 时,HTTP 响应保留已有 DB/read-model rows,同时显式返回 `source/reason/query_name/message`。
- EasyNet `/api/v1/devices/:id/sessions` 已扩展/接入 `resolve_unavailable[]`: `terminal.list` 失败时不再 5xx,而是返回用户自己的 terminal ownership ledger fallback rows(状态 `UNKNOWN`)加 typed unavailable,不泄露同设备其它用户 session。
- EasyNet `/api/v1/pages` 已扩展/接入 `resolve_unavailable[]`: `pages.list` 的 daemon/pages host unavailable 不再伪装成健康空列表,而是返回空 project read model 加 typed unavailable;Frontend Pages 页面会显示该 resolver/runtime state。
- EasyNet public pages/files HTTP adapter 已补齐 latest-only `AbilityURA` 构造:后端统一通过 `axon.HostedAbilityInvokeRequest` 派生 callee-owned canonical Ability URA;`/web/<user>/<project>/...` 和 OpenAI files surface 使用实际 host device 作为 runtime callee,用户 project/blob/API key 只作为 Subject/Resource 表达,不再虚构 backend-owned pages/files agent。
- `scripts/docker-e2e-public-routes.sh` 已更新为当前结构化 daemon log 判定(`kind=bidi_opened` / `kind=advertise_agent_prelude_done`),并新增 `/api/v1/pages` product-path 断言;OpenAI chat model 缺席时只跳过 chat round-trip,不再阻断 public pages/files 回归验证。
- EasyNet file transfer HTTP entry 已接结构化 `failure` locator: request/open/unary/pre-stream 失败返回 `{error,failure}` JSON,`failure` 明确 `source/code/layer/message/retryable`;download body 已提交后的 mid-stream receipt/transport failure 通过 `X-EasyNet-Failure` trailer(base64url JSON `FailureLocator`)表达,不会污染 `application/octet-stream` body。
- EasyNet terminal WebSocket 已接结构化 `terminal_error` 控制帧: backend 从 transport error 或 Axon `InvocationReceipt` 生成 `failure` locator,Frontend 直接渲染 `source code: message`,不再解析 ANSI/人类 `reason` 字符串来判断 attach failure。
- Frontend 能力目录 API、Agents API、Skills API、Devices API 以及 Abilities/Agents/Skills/Devices/DeviceDetail 页面已消费 `resolve_unavailable[]`,可直接展示 `NOROUTE` / `UNAUTHORIZED` / `STALE` 等 typed resolver 状态;DeviceDetail Access tab 已显示 session resolver state。
- Axon canonical `InvocationReceipt` 已新增 `failure: axon.v1.Error`;Axon runtime 签名 terminal receipt 和 EasyNet-Cli daemon 本地 bidi terminal receipt 在 Failed/TimedOut/Cancelled 时填充 typed `code/message/stage/retryable`,Completed/admission receipt 不携带 failure。EasyNet backend `failurelocator` 优先消费该 typed failure,只把 `reason` 当人类 message fallback。
- EasyNet-Cli daemon 侧 `federation.advertise_abilities` clean target 是 latest-only owner projection: `owner_ura + host_device_ura + projection_* + ability_summaries`;旧 `agent_ura + abilities` 不再作为一等兼容入口保留。
- EasyNet-Cli `AbilityCatalogStore` 已降级为 resolver projection read model:写入只接受 per-owner monotonic revision/digest fence,同 revision digest conflict 不覆盖,stale revision 不覆盖;读取 `federation.resolve(include_abilities=true)` 时按 `lease_expires_unix_ms` 过滤过期 projection。

本轮已跑过的 backend 验证:

- `go test -count=1 ./internal/utils/urautil ./internal/axon ./internal/logic/agent ./internal/logic/skill ./internal/logic/ability ./internal/logic/device`
- `go test -count=1 ./internal/axon ./internal/aggregator`
- `go test -count=1 ./internal/axon ./internal/aggregator ./internal/logic/ability`
- `go test -count=1 ./internal/axon ./internal/aggregator ./internal/logic/ability ./internal/logic/agent ./internal/logic/skill ./internal/logic/device ./internal/utils/urautil`
- `go test -count=1 ./internal/axon ./internal/aggregator ./internal/logic/ability ./internal/logic/agent ./internal/logic/skill ./internal/logic/device ./internal/utils/urautil ./internal/federation`
- `make proto`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm test -- --run src/pages/easynet/AgentsPage.test.tsx src/pages/easynet/SkillsPage.test.tsx src/lib/api/easynet-agents.test.ts src/lib/api/easynet-skills.test.ts src/lib/api/easynet-abilities.test.ts`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm run build`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm test -- --run src/lib/api/easynet-abilities.test.ts src/pages/easynet/AbilitiesPage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/logic/device ./internal/logic/agent ./internal/logic/skill ./internal/logic/ability ./internal/axon ./internal/resolverstate`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm test -- --run src/lib/api/easynet-devices.test.ts src/pages/easynet/DevicesPage.test.tsx src/pages/easynet/DeviceDetailPage.test.tsx src/pages/easynet/AgentsPage.test.tsx src/pages/easynet/SkillsPage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm run build`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/logic/device ./internal/types ./internal/resolverstate`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm test -- --run src/lib/api/easynet-devices.test.ts src/pages/easynet/DeviceDetailPage.test.tsx src/pages/easynet/DevicesPage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm run build`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/failurelocator ./internal/handler/file ./internal/handler/terminal`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/failurelocator ./internal/handler/file`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm test -- --run src/store/terminal-store.test.ts src/pages/easynet/TerminalPage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm run build`
- `! rg -n "DeviceOwnedAbilityURAForTarget|device\\.agent\\.list|device\\.skill\\.list" internal/logic/agent internal/logic/skill internal/utils/urautil internal/axon`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/failurelocator ./internal/handler/file ./internal/handler/terminal`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && make proto`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/logic/page`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/logic/page ./internal/types ./internal/resolverstate`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/handler/openai ./internal/handler/pages_public ./internal/axon`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm test -- --run src/lib/api/easynet-pages.test.ts src/pages/easynet/PagesPage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/axon ./internal/daemon_grpc ./internal/handler/file`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend && go test -count=1 ./internal/logic/device ./internal/logic/agent ./internal/logic/skill ./internal/logic/ability ./internal/aggregator ./internal/resolverstate ./internal/types`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && node ./node_modules/vitest/vitest.mjs run src/pages/easynet/AbilityDetailPage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && node ./node_modules/vitest/vitest.mjs run src/pages/easynet/BrowserHomePage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && node ./node_modules/vitest/vitest.mjs run src/pages/easynet/AgentDetailPage.test.tsx`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend && npm run build`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet && ./scripts/docker-build-images.sh`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet && HUB_A_HTTP_PORT=19096 HUB_A_TLS_PORT=51559 ./scripts/docker-e2e-public-routes.sh`

本轮已跑过的 CLI 验证:

- `cargo test -q --features axon-pb ability_catalog_store`
- `cargo test -q --features axon-pb federation_wrappers`
- `cargo test -q --features axon-pb owner_projection`
- `cargo test -q --features axon-pb discover_ability`
- `cargo test -q --features axon-pb decode_forward_invoke_response`
- `cargo test -q --features axon-pb forward_invoke_routes_through_escalation_when_handle_attached`
- `cargo test -q --features axon-pb end_to_end_device_escalation_resolves_via_hub_session_request`
- `cargo test --features axon-pb map_local_bidi_handler_file_transfer`
- `cargo test --features axon-pb -- --list | rg "map_local_bidi_handler_file_transfer"`

本轮已跑过的 Axon runtime/proto 验证:

- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon && scripts/proto/sync_axon_v1.sh --write && scripts/proto/sync_axon_v1.sh --check`
- `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon && cargo test --manifest-path core/runtime-rs/Cargo.toml services::invocation::admission_gate::tests::terminal_receipt_failure_is_typed_for_non_completed_states`

追加修复结果:

- CLI `federation.forward_invoke` helper 已改为用 Axon SDK
  `ForwardInvokeResponse` 类型解析返回体;`result_bytes` 按 SDK base64
  字符串解码,不再按旧 JSON byte-array fork 读取。
- CLI 已删除 `delivery=accepted` 展示兜底;空 payload 现在是真实空结果
  (`null`),非 JSON payload 走 hex 降级显示。
- `EasyNet/scripts/docker-e2e-join-invoke.sh` 和
  `EasyNet/scripts/docker-e2e-cross-hub.sh` 已删除通过 daemon log 确认
  accepted 的旧验证路径;脚本必须从 CLI 输出直接解出 sentinel payload。
- 已重跑 `HUB_HTTP_PORT=18187 HUB_TLS_PORT=52449
  ./scripts/docker-e2e-join-invoke.sh`:same-hub `fs.read` 直接返回
  `docker-e2e-1780862791268815000-a44365e9-bed2-4a88-a37e-5b96597ba561`。
- 已重跑 `HUB_A_HTTP_PORT=19181 HUB_B_HTTP_PORT=29181
  HUB_A_TLS_PORT=53444 HUB_B_TLS_PORT=63444
  ./scripts/docker-e2e-cross-hub.sh`:cross-hub `fs.read` 一次 session
  retry 后直接返回
  `cross-hub-1780862855137821000-f2ba0b61-a925-4d73-b3c5-90faa9b4a5c5`。
- Backend `agent.list` / `skill.list` 最新实现不再推导 owner route:
  本地 realm 通过 daemon `namespace.resolve` 读取 Axon typed
  `ResolveAnswer.records`,peer fanout 通过 daemon-owned
  `namespace.proxy_resolve` 扇出到 peer daemon 的 `namespace.resolve`,
  再只从 typed resolver records 中读取 `owner_ura` / `ability_ura`
  和 `HOSTED_BY` placement。
- Backend `AbilityDescriptor` reader 已支持 namespace-safe
  `AbilityProjectionSummary` 字段(`namespace` + `local_name`),因此
  resolver summary 和完整 descriptor 统一进入同一个 route selector。
- CLI `federation.resolve(include_abilities=true)` 对 live device presence
  补齐 device-owned `agent.list` / `skill.list` route summaries;这些
  summaries 由 device profile descriptor 投影得到,不是 backend fallback。
- CLI owner projection `AbilityProjectionSummary` 已新增受控
  `callable_summary`:只发布 `public_name`、description、class、hints
  和输入字段名/类型/required 位,供 CLI/MCP/plugin/script operator
  展示 child callable;不发布 raw `AbilityDescriptor`、完整 JSON Schema、
  manifest source 或本机 host path。
- CLI filesystem callable summaries 已固定为 ResourceRef-first:投影只显示
  `resource_ref` / `max_bytes` / `encoding` 等可调用字段摘要,不发布 raw
  host filesystem tree;`fs.read/write/stat/list`、`fs.edit`、`fs.transfer`
  每次调用仍通过 `runtime/resources/filesystem.rs` 重校验 namespace、
  revision、expiry、capability、owner、virtual root、path traversal 与本地
  root 映射后才触碰文件系统。
- CLI `federation.resolve(include_abilities=true)` 在 catalog 已接入时
  严格消费 leased owner projection:projection 过期后返回空 summaries,
  不再回退到本地 device profile 静态 descriptor,避免绕过租约读模型。
- CLI `agent.list` 输出 hosted agent 的 canonical `ura`,backend 不再把
  peer 短名 agent 误归属到本地 user。
- CLI `runtime.invoke_remote` 已修复跨 realm target:本地 daemon 接到
  hub/backend 的 invoke_remote 后,若 target realm 不是本地 realm,会构造
  canonical `federation.forward_invoke` 并由 CLI daemon 的 peer hub dialer
  转发;后端不直连 peer hub,也不在本地 PresenceRegistry 查 peer 设备。
- Backend `/api/v1/calls` 已改为 latest-only daemon-local voice 查询:
  `voice.*` 是 CLI daemon 本地 call signalling ability,当前 call registry
  属于 hub daemon 进程内状态,不是 device-hosted ability,也不是跨 hub 复制
  read model。后端经 daemon.sock 发完整 Axon Invocation,callee 是本 realm
  `hub` URA,subject 是 backend synthetic system user URA,并携带 backend-signed
  SessionAuthority;CLI daemon 自己决定 raw local runtime 还是远端 session.open,
  backend 不再按 ability 名分类。
- Backend file transfer HTTP entry 已删除旧 unary fork 路径,统一通过
  `streambridge.OpenSignedBidiSession` 打开 backend-signed bidi session;
  请求参数使用 `resource_ref`,由 CLI filesystem ResourceRef namespace 在
  runtime 侧重校验 root/revision/expiry/capability/path traversal,后端不再
  拼 host path 或伪造 device 文件系统能力。
- Frontend `AbilityDetailPage` 已消费 ability catalog 的
  `resolve_unavailable[]`:当 resolver/catalog 不可用导致详情页无法定位
  tool route 时,页面显示 typed resolver state,不再把 resolver failure
  误呈现成单纯 stale link / `Ability not found`。
- Frontend `BrowserHomePage` 运行 agent chat 任务时已优先渲染 stream
  terminal 的结构化 `terminal_error.code/message`:例如
  `ROUTE_STALE: selected owner projection expired`,不再被普通
  `result.message` 覆盖成无来源的失败文本。
- Frontend `AgentDetailPage` 已消费 owner-scoped ability catalog 的
  `resolve_unavailable[]`:agent 详情页在 agent 存在但 owner ability
  projection/route resolver 不可用时显示 typed resolver state,不再把
  resolver failure 静默压成空的 "No abilities activated" 状态。
- 已重跑 `KEEP_DOCKER_E2E=1 HUB_A_HTTP_PORT=19186
  HUB_B_HTTP_PORT=29186 HUB_A_TLS_PORT=53449 HUB_B_TLS_PORT=63449
  ./scripts/docker-e2e-cross-hub.sh`:hub-a `/api/v1/agents` 投影出
  hub-b hosted agent `cross-e2e-agent-1780865581`;hub-a
  `/api/v1/skills/installed` 投影出 hub-b skill
  `cross-e2e-skill-1780865581`;cross-hub `fs.read` decoded bytes 直接匹配
  `cross-hub-1780865581540418000-bc94ea56-6f5e-48e5-8900-f0e01ba9b0fc`。
- 已重跑 `KEEP_DOCKER_E2E=1 SKIP_DOCKER_BUILD=1
  HUB_A_HTTP_PORT=19097 HUB_A_TLS_PORT=51560 HUB_B_HTTP_PORT=29097
  HUB_B_TLS_PORT=61560 ./scripts/docker-e2e-deep.sh`:A-I 全矩阵
  L1/L2/L3 通过,`failed_functions=0`,证据目录
  `/tmp/easynet-docker-e2e-deep-9C7AdH`。本次覆盖
  `/api/v1/pages`、`/api/v1/devices/:id/sessions`、public pages/files、
  file transfer、PTY terminal、cross-hub ability invoke、call signalling、
  SSE event stream、receipt/policy。
- CLI daemon 已新增 latest-only `namespace.proxy_resolve` typed peer
  fanout surface: backend 只提供 peer hub URL 集合和 namespace query,
  daemon 负责 trust-anchor 过滤、peer hub dial、peer envelope signing,
  并把 peer `namespace.resolve` 的 Axon `ResolveAnswer.records` 合并为
  typed `ResolveAnswer`。旧 `federation.proxy_resolve` daemon 入口已删除,
  避免后续产品路径重新消费 legacy directory rows。
- CLI daemon `namespace.resolve` directory answer 已为 hosted agent 输出
  `RECORD_TYPE_HOSTED_BY`;Backend namespace projection 消费该 record,
  因此 peer agent / skill / ability catalog 路由不再从 ID row 或 host
  字符串推断 placement。
- 追加验证已通过:
  `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend &&
  go test -count=1 ./internal/axon ./internal/federation
  ./internal/logic/agent ./internal/logic/device ./internal/logic/ability
  ./internal/logic/skill ./internal/aggregator`;
  `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli &&
  cargo test --features axon-pb namespace_resolve_directory_includes_hosted_by_for_hosted_agents`;
  `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli &&
  cargo test --features axon-pb invoke_dispatches_namespace_proxy_resolve_to_typed_peer_surface`;
  `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli &&
  cargo test --features axon-pb ability_name_constants_match_spec_section_4`;
  `cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli &&
  cargo test --features axon-pb quota_meters_user_abilities_but_exempts_control_plane`。

本轮闭合后的剩余边界任务:

- CLI daemon 产品路径已暴露 `namespace.resolve` 和 `namespace.proxy_resolve` 普通 daemon ability,并返回 Axon `ResolveAnswer` proto-JSON 投影;Backend HTTP read model 已通过 daemon `Invocation` 消费 `ResolveAgentsAnswer`,不再以 `federation.resolve` directory facade 作为一等本地 read model,也不再通过 `federation.proxy_resolve` legacy rows 做 peer fanout。后端仍不直连 peer hub 或 Axon runtime namespace RPC。
- 已继续逐页审查 HTTP/Frontend resolver state:Abilities、Agents、Skills、Devices、DeviceDetail、Sessions、Pages 及详情页不再把 daemon/resolver/hub 不可用压成健康空列表或普通 5xx。本轮补齐 daemon offline / peer endpoint missing 的 typed `resolve_unavailable[]` 路径,保留已有 DB/read-model rows。
- Browser file-transfer download client 已补 `X-EasyNet-Failure` trailer 解码:下载 body 读完后检查 failure trailer,抛出携带 `FailureLocator` 与 partial blob 的结构化错误;成功路径读取 `X-Sha256` trailer。
- Axon receipt typed failure schema、Axon runtime signed terminal receipt producer、CLI daemon local bidi terminal receipt producer 已核对并落地:Failed/TimedOut/Cancelled 填 `InvocationReceipt.failure`,Completed/admission receipt 不填。剩余是后续新增 terminal receipt producer 必须接入同一规则,并继续把 RFC-005 更细 failure code(`ABILITY_NOT_FOUND`、`ROUTE_STALE`、`RESOURCE_SCOPE_DENIED` 等)从 resolver evidence 原生投进 receipt,避免退回默认 `INVOCATION_FAILED`。

---

## 6. 一句话给 CTO

你的判断对:**REMOVED 不是孤立 bug,而是后端 read model 曾 fork 协议形状、CLI runtime 承担产品解析面、Axon typed answer 没被统一消费导致的边界债。** 本轮已把产品解析路径收口到 daemon `namespace.resolve` / `namespace.proxy_resolve` + Axon `ResolveAnswer`,HTTP/Frontend 不再把 resolver/hub failure 伪装成健康空列表,文件下载和 terminal receipt 也有 typed failure 载体。剩余治理只应是后续新增 producer 遵守 `InvocationReceipt.failure` 规则,以及按 RFC-005 逐步细化 failure code,不再回到旧 facade。
