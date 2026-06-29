# EasyNet Wrapper -> CLI Runtime Refactor SPEC

Status: Draft for review
Date: 2026-06-28
Owner: EasyNet runtime architecture review

Review mode: architecture/spec correction first. This document is not an
implementation authorization by itself. An agent or engineer reading this SPEC
must first produce:

1. a gap report against current code,
2. an explicit decision list for every open design branch,
3. a proposed patch sequence,
4. a verification plan.

Code implementation starts only after the owner explicitly approves the patch
sequence or separately asks to implement. This rule exists to prevent
well-intentioned partial patches from hardening an under-reviewed architecture.

## 1. 背景与结论

本次重构的方向不是把 EasyNet 仓库继续建设成一个独立 Hub 系统，而是把 EasyNet 仓库定位为 EasyNet-Cli runtime(`easynet` CLI + `easynet-daemon`)的产品包装层。

目标是让 CLI 能够完整承担 Device 与 Hub 的基础运行时能力，包括身份、信任、Join、联邦发现、命名空间解析、能力注册、能力调用转发、会话、回执、流式/双向会话，以及设备侧基础能力。EasyNet 仓库则负责更好的注册用户、账号体系、产品 API、Join Token 生成与传播、前端体验和 HTTP/WS/SSE 包装。

最终方向:

```text
Browser / Frontend
  -> EasyNet backend product API
  -> local easynet-daemon runtime
  -> CLI-owned Hub and Device abilities
  -> Axon-owned neutral protocol contracts
```

不是:

```text
Browser / Frontend
  -> EasyNet backend as an independent Hub runtime
```

## 2. 目标

1. EasyNet-Cli 成为 Device 与 Hub 基础能力的唯一运行时承载层。
2. EasyNet backend 从“运行 Hub 能力”收敛为“包装 CLI/daemon 的产品层”。
3. 用户注册、Join Token、传播与兑换流程由 EasyNet 提供更好的产品入口，但实际 Join、Trust、Advertise、Resolve、Invoke 由 CLI daemon 执行。
4. Axon 保持为中立协议与 wire contract 层，不掺入 EasyNet 产品策略。
5. 所有公开能力调用路径保留完整 Invocation 语义，不允许退化成 Go/Rust 内部 DTO RPC。
6. 前端可以通过 EasyNet 获得更好的 API/UX，但不能绕过 CLI daemon 的 runtime 边界。

## 3. 非目标

1. 不把 EasyNet backend 做成第二套 Hub runtime。
2. 不把 EasyNet 的账号系统、产品 DB、注册体验下沉到 CLI。
3. 不把 EasyNet 的产品策略、用户增长逻辑、页面 API 放入 Axon。
4. 不把 `federation.forward_invoke` 直接暴露给浏览器作为产品 API。
5. 不用一个 method 对应一个 ability 的固定 ABI 取代通用 Invocation 模型。
6. 不在本 SPEC 中估算工时。
7. 不允许在没有 owner approval 的情况下把本 SPEC 直接当作代码修改任务执行。
8. 不允许用“兼容旧架构”作为默认答案。任何 compatibility wrapper 必须有 owner、到期条件、观测指标和删除门槛。

## 4. 分层归属

### 4.1 Axon

Axon 负责跨语言、跨 runtime 的中立协议契约:

1. URA、Invocation tuple、canonicalization、signature、admission primitives。
2. Receipt、stream、bidi 的协议语义。
3. Namespace resolve、federation resolve、directory entry、list user devices、resolve key 的中立 wire shape。
4. Join credential envelope 的中立 wire shape。
5. Ability descriptor / descriptor projection 的中立 shape，如果该 shape 需要跨 Go/Rust/前端边界复用。
6. 最小运行时参考实现、协议测试、SDK 类型。

Axon 不负责:

1. EasyNet 用户账号 DB。
2. Join Token 存储、传播、投放策略。
3. Daemon 插件策略。
4. 产品权限页面、用户增长、团队空间、邀请 UX。
5. EasyNet backend 的 HTTP DTO。

### 4.2 EasyNet-Cli / easynet-daemon

EasyNet-Cli 是运行时本体。它负责:

1. Daemon 生命周期。
2. Device mode、Hub mode、Both mode。
3. Identity、trust、keyring、public key registration。
4. Federation join、advertise、heartbeat、discover、directory subscription、revoke。
5. Namespace resolve、proxy resolve、route selection。
6. Ability registry、descriptor registry、local runtime dispatch。
7. Remote invoke、forward invoke、session dispatch。
8. Receipt、stream、bidi 的 runtime lifecycle。
9. 设备侧能力，包括文件、进程、PTY、Agent、MCP、A2A、EAL Mission、Browser、Remote Desktop、Media 等。
10. Hub 侧基础能力，包括设备目录、用户设备列表、跨 realm resolve、key resolve、catalog projection。

CLI daemon 是唯一允许决定 invocation locality 的层。这 5 种 locality 必须由单一 `RouteResolver` 解析为一个 `SelectedRoute` 枚举,而不是散落在多个 `dispatch_*` 函数分支里(当前 `route_resolver.rs` + `unary_dispatcher.rs` 即是散落分支,见第 5 节不变量 13):

1. 本地设备能力(LocalDevice)。
2. 同 realm 远端设备能力(SameRealmDevice)。
3. Hub-owned ability(HubOwned)。
4. Cross-realm peer(CrossRealmPeer)。
5. Proxy / relay path(Proxy)。

规则:`daemon_invocation_service` 等上层不得内联 locality `match`;它们只消费 `RouteResolver` 的输出。新增 locality 是新增 `SelectedRoute` variant + resolver 分支,不是上层再加一条 if。

### 4.3 EasyNet backend

EasyNet backend 是产品包装层。它负责:

1. 注册、登录、JWT/session、邮箱、WebAuthn、产品用户记录。
2. 用户、设备、peer hub 的产品 DB。
3. Join Token mint、distribution、redeem preflight、revoke、audit、rate limit。
4. 前端 HTTP API、WS、SSE。
5. 对 CLI daemon Invocation API 的包装与 DTO projection。
6. 前端页面需要的聚合展示，但聚合数据必须来自 daemon ability 或产品 DB，不能由 backend 自己成为 runtime。
7. Daemon offline/reconnect 状态管理与产品提示。

EasyNet backend 不负责:

1. 本地执行能力实现。
2. 自己维护一套 Hub ability registry。
3. 自己决定 namespace route。
4. 自己实现 federation forwarding。
5. 自己生成 runtime receipt 语义。
6. 自己维护与 Axon 不一致的 wire contract。

## 5. 核心不变量

1. 所有公开 ability invocation 必须保留完整七元组:

```text
caller
callee
ability
subject
nonce
causal_context
args
```

2. EasyNet backend 可以选择产品输入、校验 JWT、做 DTO projection，但不能重新实现 resolver、admission、execution。
3. CLI daemon 负责 locality、route、session、stream/bidi lifecycle。
4. Backend DB row 不能替代 Axon/daemon 的 canonical invocation、receipt、directory state。
5. Go/Rust/前端共享的 wire shape 必须来自 Axon 或由 Axon 生成，不允许长期手写镜像。
6. Device/Hub mode 默认使用 `easynet-daemon`。裸 `axon-runtime` 只作为协议参考、测试、最小 runtime，不作为产品运行入口。
7. `aggregate.list_abilities_catalog` 不进入 canonical daemon baseline。前端能力目录必须迁移到 `meta.list_abilities` + daemon catalog projection。若短期保留 alias，必须是显式 transitional wrapper，默认不实现。
8. `federation.forward_invoke` 是 daemon/runtime 能力，不是浏览器产品 API。
9. 所有 stream/bidi 路径必须有明确终止状态与错误映射。
10. 所有 baseline 能力必须有 typed conformance model，不能只存在于注释、Markdown 列表、测试里的散字符串。
11. 所有路由/dispatch fallback 必须能证明不会隐藏 missing handler、wrong call mode、wrong owner、wrong surface。不能用 fallback 让测试“看起来通过”。
12. Ability registry 只能有一个权威存储。`src/runtime/ability_dispatch.rs::AxonAbilityCatalog` 当前同时持有九张以 ability name 为键的 legacy `BTreeMap`(`rpc`/`stream`/`bidi` × args-only/`_with_env`，加 `owner`/`authority_scope`/`manifests`，并在 `DynamicCatalogue` 重复一份)，又另持 `control_plane: RwLock<AbilityControlPlaneRegistry>`；其自身注释承认 `owner`/`manifests` 是 "compatibility side tables"。这是三套并行真相，是本项目最大的非收敛债，必须在本次重构中收口为单一存储，详见第 9.1.A 节。新增 ability 不允许再写第二张并行表。
13. Invocation locality 解析只能有一个 owner。当前具体 owner 是 `src/services/invocation_transport/route_resolver.rs::DaemonRouteResolver`，输出 `SelectedInvokeRoute` / `DelegatedInvokeRoute` typed selection；如果未来抽成 `src/runtime/resolver` 或 closed enum wrapper，必须原子迁移，不能保留两套 resolver。所有 unary/stream/bidi dispatch path 必须消费同一 selected route，新增第 6 种 locality 只能扩展 typed selection model，不能在 dispatcher 里另开 ad hoc 分支。详见第 4.2 与第 7.4 节。
14. Runtime terminal-state 只能有一个 canonical 词汇表。Axon SDK 的 `InvocationState`(含 `is_terminal()`)是 source of truth；CLI 侧的 `TerminalState`(Succeeded/Failed/Cancelled)必须是它的显式投影，不允许两套终态词汇各自独立演化。详见第 7.4 节。
15. Runtime trust 是一个聚合，不是三条互不协调的写。`identity.register_pubkey` / `identity.list_user_pubkeys` / `identity.revoke_user_pubkey` 必须是同一 `RuntimeTrust` 聚合的唯一 mutators；revoke 必须使 presence 失效并驱动 `JoinConnectionState::DisconnectedRemoved`。详见第 6.1 与第 7.4 节。
16. EasyNet backend 到 local daemon 的传输通道必须有显式信任边界(见第 4.3 与第 6.6 节)。不允许把“本机即可信”当作隐式前提，尤其是 `identity.register_pubkey` 对 backend/hub/user role 的特权 self-signed trust 写入。

## 6. 目标架构流程

### 6.1 用户注册与 runtime trust

```text
Browser
  -> EasyNet /register
  -> EasyNet account DB + session/JWT
  -> EasyNet calls daemon only when a browser/user key must enter runtime trust
  -> daemon identity.register_pubkey
```

规则:

1. 用户注册是 EasyNet 产品能力。
2. Runtime trust 写入是 daemon 能力。
3. EasyNet 只能通过 daemon 的 identity ability 修改 runtime trust。
4. EasyNet 用户 ID 到 Axon caller/callee/subject 的映射已有 canonical 实现:`backend/internal/axon/urns.go:107 func UserURA(realm, userID string)`(委托 `axonsdk.UserURA`)。本次重构不引入第二套映射;所有产品入口必须经此函数,禁止手写字符串拼接。**未决 bug(必须在本重构中钉死)**:`urns.go` 文档说参数是 `users.id`(UUID),但 `prepareEnvelopeLogic.go` 实际传 `username`。必须确定哪一个是 canonical subject 锚点并统一,否则同一用户在 register 与 invoke 两条路径上得到不同 URA。
5. Runtime trust 必须建模为一个 `RuntimeTrust` 聚合，以 subject URA 为键，拥有 {admitted keys, rotation_epoch, revocation set}(rotation_epoch 已存在于 `ResolveKeyReceipt`，聚合应复用而非新造)。admission 是对该聚合的纯决策；`identity.register_pubkey` / `identity.revoke_user_pubkey` 是其唯一 mutators。不变量: revocation 单调、rotation 递增 epoch、revoke 使 presence 失效并驱动 `JoinConnectionState::DisconnectedRemoved`。

### 6.2 Join Token 生成、传播与兑换

```text
Browser/API
  -> EasyNet join-token endpoint
  -> EasyNet mints one-time token / credential envelope
  -> token is copied, linked, QR encoded, or sent through product channels

CLI
  -> easynet join <token>
  -> EasyNet redeem/preflight
  -> CLI writes credential, daemon config, realm trust, federated_peers
  -> daemon federation.join membership acknowledgement
  -> daemon session.open establishes presence / reachability
  -> daemon federation.advertise_agent publishes hosted-agent linkage
  -> daemon federation.advertise_abilities publishes owner projection read model
```

规则:

1. Join Token 的产品分发由 EasyNet 做。
2. Token 必须支持一次性、过期、撤销、审计与限流。
3. Token redeem 后得到的是 CLI/daemon 可消费的 credential envelope。
4. Credential envelope 的跨语言 shape 应归 Axon。
5. EasyNet backend 不能代替 CLI daemon 完成 runtime join。
6. `federation.join` 不写 presence，也不证明设备在线；它只确认 membership intent / receipt。directory presence 必须来自 daemon-owned `session.open` stream。
7. `federation.advertise_agent` 只发布 hosted-agent linkage；`federation.advertise_abilities` 只更新 owner projection read model。二者都不能成为 ability implementation registry。

### 6.3 Ability 调用

```text
Browser
  -> EasyNet prepare/submit invoke endpoint
  -> EasyNet validates JWT and product permission
  -> EasyNet builds or verifies full Invocation
  -> EasyNet submits Invocation to local daemon
  -> daemon namespace.resolve
  -> daemon LocalRuntime / runtime.invoke_remote / federation.forward_invoke
  -> daemon returns receipt / stream / bidi lifecycle
  -> EasyNet projects response DTO to frontend
```

规则:

1. EasyNet endpoint 是 wrapper，不是 ability executor。
2. 每个 wrapper 必须能说清楚它调用了哪个 daemon ability。
3. Product permission 只能限制用户能不能发起调用，不能替代 runtime admission。
4. Runtime receipt 的终态由 daemon 产生。
5. 对 signed public calls，nonce 与 causal_context 必须保留。

### 6.4 实时目录与事件

```text
daemon federation.subscribe_directory_v2
  -> EasyNet backend event bridge
  -> EasyNet SSE/WS frontend channel
```

规则:

1. EasyNet 可以 fan out SSE/WS。
2. EasyNet 不应从产品 DB 推断 runtime presence。
3. Directory event shape 如跨语言复用，应归 Axon。

### 6.5 文件、终端、浏览器、远程桌面

```text
Browser HTTP/WS
  -> EasyNet framing/auth wrapper
  -> daemon fs / terminal / browser / remote_desktop ability
  -> daemon owns target routing and stream/bidi session lifecycle
```

规则:

1. EasyNet 可以提供前端友好的 upload/download/WS framing。
2. 文件读写、PTY、remote desktop、browser control 的能力执行属于 CLI daemon。
3. 所有长连接必须有 daemon session id、terminal closure、error receipt。

### 6.6 Backend 到 daemon 的信任边界

整套架构的安全性建立在“backend 是 wrapper，daemon 拥有 trust”之上,但这一跳本身必须被认证,否则边界是空的。当前现状(必须在本重构中定调,不能继续隐式):

1. backend → daemon 走本地 UDS,仅靠文件权限(`0600`),`listeners.rs` 自己注释说这是 "a hardening detail, not a correctness one"。
2. `daemon_grpc/client.go` 使用 `insecure.NewCredentials()`,无任何 peer 认证;代码中无 `SO_PEERCRED`。
3. `identity.register_pubkey` 对 role ∈ {backend, hub, user} 是特权 self-signed trust 写入,且在 `admission_facade.rs` 被豁免 delegation-proof。

规则:

1. 必须显式定义 backend↔daemon 信任边界:同一 uid 的本地 UDS,并在 daemon 侧用 `SO_PEERCRED`(或等价 OS 机制)校验对端 uid,而不是只靠目录权限。
2. 特权 identity 写入(backend/hub/user role)必须有 daemon 侧的 caller gate,与普通 device-local ability 区分开。
3. 错误必须区分 product auth error 与 runtime admission error(见第 13.5 节);二者不能塌缩为同一 401/403。
4. 远程(非本机)backend 接入 daemon 不在本 SPEC 默认范围；若未来需要，必须升级为 mTLS 并单列 RFC，不能复用本机 insecure 通道。

## 7. CLI 必须承载的基础能力

### 7.1 Hub / Federation / Control baseline

以下能力应由 EasyNet-Cli daemon 承载，在 Hub mode 或 Both mode 可用:

1. `federation.join`
2. `federation.advertise_agent`
3. `federation.advertise_abilities`
4. `federation.heartbeat`
5. `federation.resolve`
6. `namespace.resolve`
7. `namespace.proxy_resolve`
8. `federation.forward_invoke`
9. `session.open`
10. `runtime.invoke_remote`
11. `federation.resolve_key`
12. `federation.discover`
13. `federation.subscribe_directory`
14. `federation.subscribe_directory_v2`
15. `federation.list_user_devices`
16. `federation.proxy_list_user_devices`
17. `federation.revoke`
18. `identity.register_pubkey`
19. `identity.list_user_pubkeys`
20. `identity.revoke_user_pubkey`
21. `runtime.bootstrap_self_identity`
22. `meta.list_abilities`
23. `federation.status`

特别说明:

1. `aggregate.list_abilities_catalog` 不属于 Hub baseline，不应由 EasyNet Go backend 作为 runtime aggregator 实现，也不应作为 daemon canonical API 重新引入。
2. `federation.status` 结论: 实现为 daemon read-only ability，读取 daemon federation/init 状态机的观测投影。不得另建并行状态源。
3. `meta.list_abilities` 应支持必要的 realm/catalog projection，使前端无需依赖 backend-local aggregator。
4. `federation.heartbeat` 目前是迁移期 daemon wrapper，允许返回 typed no-op / lease-refresh success；它不能重新成为 authoritative liveness source。authoritative liveness 必须来自 `session.open` / presence stream。
5. `runtime.bootstrap_self_identity` 是 runtime-admin handshake，不应由 CLI wrapper 假 ack；如果 Axon LocalRuntime 没有安装对应实现，应显式失败。
6. `session.open` 是 daemon-owned long-lived bidi carrier，是 presence/reachability 与 remote dispatch 的权威来源；不能被 `federation.heartbeat` 或 advertise wrapper 替代。
7. `runtime.invoke_remote` 是 daemon-owned per-call remote dispatch，必须通过 live `session.open` 与 `PendingDispatchMap` 完成；它不是 EasyNet backend 的本地 wrapper implementation。
8. `runtime.*` 是 runtime-admin / daemon-runtime namespace 规则，不等于所有 `runtime.*` 都进入 Hub baseline。runtime-admin ability 的 wire/schema owner 是 Axon；CLI baseline 只表达 daemon 必须安装并可达。dispatcher 可以按前缀绕过 owner-presence resolution，但 conformance 必须单独验证已承诺项是否真的安装在 Axon `LocalRuntime` 或 daemon bidi surface。

### 7.2 Device runtime baseline

以下能力组应由 EasyNet-Cli daemon 作为设备侧基础能力承载。本节是产品能力族概览；规范性的 concrete rows 必须落在 `src/runtime/ability/conformance.rs` 的 `DeviceBaseline`，并标注 call mode、surface、domain、feature gate。

Health / Admin / Meta:

1. `observe.health`
2. `observe.network_health`
3. `admin.status`
4. `meta.describe`
5. `meta.list_abilities`
6. `meta.list_resources`

Node / Ability lifecycle:

1. `node.list`
2. `node.describe`
3. `node.remove`
4. `ability.deploy`
5. `ability.uninstall`
6. `ability.publish`
7. `ability.unpublish`

Baseline locomotion:

1. `fs.read`
2. `fs.write`
3. `fs.stat`
4. `fs.list`
5. `fs.edit`
6. `process.exec`
7. `shell.run`
8. `http.request`

File transfer:

1. `fs.transfer`

PTY / Terminal:

1. `terminal.create`
2. `terminal.list`
3. `terminal.attach`
4. `terminal.input`
5. `terminal.read`
6. `terminal.resize`
7. `terminal.close`

Session / Consent:

1. `session.list`
2. `session.attach`
3. `consent.subscribe`
4. `consent.decide`
5. `consent.list_pending`

Agent / Chat / History:

1. `agent.list`
2. `agent.start`
3. `agent.stop`
4. `agent.refresh`
5. `chat.history.list`
6. `chat.history.get`

Skills / MCP / A2A:

1. `skill.*`
2. `mcp.*`
3. `a2a.*`

Orchestration:

1. `mission.*`
2. `discuss.*`
3. `loop.*`
4. `schedule.*`

Context / Resources / Media:

1. `context.*`
2. `camera.*`
3. `screen.*`
4. `mic.subscribe`
5. `speaker.publish`
6. `voice.*`

Browser / Remote Desktop:

1. `browser.*`
2. `remote_desktop.*`

OpenAI compatibility:

1. `openai.chat_completions`
2. `openai.list_models`

特别说明:

1. OpenAI compatibility 默认是 device-owned compatibility ability。
2. 如果未来需要 Hub-owned OpenAI gateway，应显式命名为 `hub.openai.*` 或其他清晰前缀，不能与 device compatibility ability 混淆。
3. `skill.*`、`mcp.*`、`a2a.*`、`mission.*`、`context.*`、`browser.*`、`remote_desktop.*` 等通配写法只允许出现在 SPEC 叙述层。实现与测试必须展开成具体 ability rows，并标注 call mode、surface、domain、feature gate。
4. `remote_desktop.*` 如果受 Cargo feature 或平台依赖控制，baseline conformance 必须显式表达 feature-gated rows，不能让默认 product build 与 specialist build 混用同一断言。

### 7.3 Baseline conformance model

第 7.1 与第 7.2 的列表不能直接被实现为散落数组。必须先建立 typed conformance model，然后让 transport、registry、测试消费同一份模型。

最小模型:

1. `BaselineAbility`
   - `name`
   - `call_mode`: `Rpc | Stream | Bidi`
   - `surface`: `DaemonInvocation | LocalRegistry | AxonRuntimeAdmin`
   - `domain`: `HubFederation | HubNamespace | HubIdentity | HubRuntimeAdmin | HubIntrospection | Device...`
2. `HubBaseline`
   - 返回 Hub mode / Both mode 必须满足的 typed rows。
   - 区分 daemon Invocation surface、Axon runtime-admin handshake、local introspection row。
3. `DeviceBaseline`
   - 返回 Device mode / Both mode 必须满足的 typed rows。
   - 支持 feature-gated rows，例如 `remote_desktop.*`。
4. `RegistryConformance`
   - 验证 LocalRegistry rows 是否真实存在且 call mode 正确。
5. `DaemonInvocationSurface`
   - 验证 daemon Invocation rows 是否由 unary/stream/bidi service 显式路由。
   - route set 必须从 dispatcher/exported route table 派生，测试不得手写第二份 route 数组后再拿它与 baseline 对比。
6. `BaselineConformanceReport`
   - 输出缺失 ability、期望 call mode、surface、domain。
7. `RuntimeAdminConformance`
   - 验证 `AxonRuntimeAdmin` rows 是否安装在 Axon `LocalRuntime`。
   - 缺失时必须返回 missing-handler / not-found 语义，不能由 CLI wrapper 伪造成功。

规则:

1. 新增 baseline ability 必须先加 typed row，再接 handler/route。
2. transport 可以 re-export ability 常量，但不能拥有独立 canonical string table。
3. tests 不能自己维护第二份 baseline list，也不能维护第二份 route list；测试只能消费 canonical conformance model 与生产 route introspection。
4. conformance failure 是设计失败，不是测试小问题；禁止用 fallback handler 消除 failure。
5. `aggregate.list_abilities_catalog` 必须有负向测试，确保它不会被误加为 canonical baseline。

### 7.4 状态机与终止语义要求

本 SPEC 涉及的 runtime path 必须被写成状态机或等价的 typed lifecycle，不允许靠 scattered boolean / string state 组合。

必须建模的状态机:

1. Federation init / status
   - 状态必须以现有 `FederationInitOutcome` 为 canonical model: pre-set `boot_in_progress`，以及 `disabled`、`installed`、`already_installed`、`failed`。
   - `federation.status` 只能读取该状态机投影。
   - 不得另建 `ready/degraded` 等并行命名；如果产品需要更高层状态，只能从 `FederationInitOutcome::code()` projection 派生。
2. Join token lifecycle
   - `minted -> redeemed | expired | revoked`
   - redeem 必须幂等可查询，重复 redeem 不得产生第二个 runtime join。
3. Daemon mode lifecycle
   - `stopped -> starting -> running(device|hub|both) -> draining -> stopped | failed`
   - mode 切换不得留下旧 listener / stale trust / stale route cache。
4. Invocation lifecycle
   - admission、route selection、local/remote dispatch、terminal receipt 必须单终止。
   - terminal-state 词汇表只有一个 canonical source:Axon SDK `InvocationState`(以 `is_terminal()` 判定终态)。CLI 侧 `TerminalState`(`src/runtime/invocation.rs`,Succeeded/Failed/Cancelled)必须实现为该 enum 的显式投影函数(落一个单一 `TerminalState::from(InvocationState)` 映射),并在测试中断言两侧不漂移;不允许两套终态词汇各自演化。
   - route selection 必须经第 4.2 节单一 route resolver 的 typed selection,dispatcher 不得内联 locality 判定。
   - 每个长连接路径必须恰好合成一帧 terminal;source 在 terminal 前关闭必须合成 `Cancelled`/`Failed`,不能静默挂起。
   - stream/bidi 必须有 terminal closure、timeout、peer disconnect、cancel、error receipt 映射。
5. Remote route / session lifecycle
   - 必须以 `DeviceSessionPhase` / `PreludeStep` 这类 typed state 表达 `Idle -> Dialing -> Preluding(join|owner_projection|advertise) -> Live -> Backoff`。
   - clean close 必须通过 `CloseClass` 或等价 closed enum 分类，例如 healthy、displaced、no admission receipt、contract skew。
   - session open、application heartbeat、offline、pending dispatch cancellation 必须 bounded。
6. Pending dispatch lifecycle
   - `PendingDispatchMap` / stream variant 必须 cleanup-on-drop。
   - target offline 必须触发 `cancel_for` 或等价机制，不能等 HTTP/gRPC 外层 timeout。
   - 每个 pending call id 只能完成一次；late complete 必须 no-op 且可观测。
7. Directory stream lifecycle
   - `subscribe_directory` / `subscribe_directory_v2` 必须在所有 senders drop 后 bounded close。
   - `subscribe_directory_v2` 必须有 idle heartbeat，防止 subscriber 把健康空闲误判为 dead stream。
8. `runtime.invoke_remote` down-stream lifecycle
   - 必须产生 zero-or-more chunk + exactly-one terminal result，或在 source close before terminal 时返回 aborted。
   - terminal error 必须携带 typed failure projection，不能只返回人类字符串。

验收规则:

1. 每个状态机必须有 owner module。
2. 每个状态机必须有 terminal states。
3. 每个 async wait 必须有 deadline 或 cancellation source。
4. 每个 replay/idempotency decision 必须可查询。
5. 每个 failure code 必须区分 product auth、runtime admission、route not found、handler missing、transport failure。
6. 每个 stream/bidi 测试必须证明 close path，不允许只测 happy path 首帧。
7. 每个 heartbeat 不得成为新的 authoritative presence source；heartbeat 只能维持 lease、keepalive 或健康观测。

## 8. EasyNet wrapper 功能范围

### 8.1 Auth / User registration

EasyNet 负责:

1. Email/password/WebAuthn/OAuth 等产品登录方式。
2. JWT/session issuance。
3. Product user profile。
4. Team/org/invite 关系。
5. 用户级产品权限。

EasyNet 通过 daemon 处理:

1. `identity.register_pubkey`
2. `identity.list_user_pubkeys`
3. `identity.revoke_user_pubkey`
4. runtime trust bootstrap。

禁止:

1. Backend 直接写 runtime trust store。
2. Backend 用产品 user table 替代 runtime identity registry。

### 8.2 Join Token

EasyNet 负责:

1. `POST /join-tokens` 创建 token。
2. `GET /join-tokens/{id}` 展示 token 状态。
3. `POST /join-tokens/{id}/revoke` 撤销 token。
4. `POST /join-tokens/redeem` 兑换 token。
5. Token audit、rate limit、expiry、single-use enforcement。
6. QR/link/copy/分享渠道。

CLI 负责:

1. `easynet join <token>`。
2. redeem preflight 后写入 credentials/config。
3. 更新 daemon trust 与 `federated_peers`。
4. 启动或通知 daemon 执行 `federation.join` membership acknowledgement。
5. 建立 `session.open` presence stream。
6. advertise agent 与 abilities projection。

Axon 负责:

1. credential envelope 的中立 shape。
2. join target、realm、hub endpoint、public key material、expiry、signature 的协议字段定义。

### 8.3 Device / Agent / Ability 页面

EasyNet 负责:

1. 页面级查询参数、筛选、排序。
2. DTO projection。
3. 产品权限过滤。
4. 前端缓存与 loading/error 状态。

EasyNet 调用 daemon:

1. `namespace.resolve`
2. `meta.list_abilities`
3. `federation.discover`
4. `federation.list_user_devices`
5. `federation.proxy_list_user_devices`
6. `federation.subscribe_directory_v2`

禁止:

1. Backend 自己维护 authoritative ability catalog。
2. Backend 从 DB 推断设备是否在线。
3. Backend 实现 independent directory resolver。

### 8.4 Ability Invoke API

EasyNet 负责:

1. Product-level permission。
2. Browser-friendly request/response DTO。
3. Signed invocation prepare/submit。
4. JWT caller 映射。
5. Response projection。

EasyNet 调用 daemon:

1. Generic Invocation submit。
2. InvokeStream。
3. InvokeBidi。

禁止:

1. Wrapper endpoint 直接调用 Go local implementation 来伪装 ability。
2. 缺失 `subject`、`nonce`、`causal_context` 的 public invoke。
3. 用 endpoint path 隐式替代 `ability` 字段。

### 8.5 文件、终端、浏览器、远程桌面

EasyNet 负责:

1. HTTP upload/download 包装。
2. WebSocket framing。
3. Browser auth/session。
4. 前端状态映射。

Daemon 负责:

1. `fs.*`
2. `terminal.*`
3. `browser.*`
4. `remote_desktop.*`
5. Session lifecycle。
6. Terminal closure / stream close。

### 8.6 Admin / Status

EasyNet 负责:

1. 展示 daemon health。
2. 展示 backend-to-daemon connection 状态。
3. 展示 product API 状态。

Daemon 负责:

1. `admin.status`
2. `observe.health`
3. `observe.network_health`
4. Hub/device mode status。

## 9. 仓库更新范围

### 9.1 EasyNet-Cli

必须更新:

1. 建立 `runtime::ability::conformance` typed baseline model 与 conformance 检查。
2. 确保 Hub mode / Both mode 注册第 7.1 节所有 Hub baseline abilities。
3. 确保 Device mode 注册第 7.2 节所有 Device baseline ability groups。
4. 补齐 `namespace.resolve`、`namespace.proxy_resolve`、`federation.resolve_key`、`list_user_devices`、`subscribe_directory_v2` 的统一 daemon invocation surface。
5. 明确 `aggregate.list_abilities_catalog`:
   - canonical path 是删除并迁移前端到 `meta.list_abilities` + catalog projection。
   - 若保留 transitional alias，必须单独写 migration note、owner、expiry、negative canonical-baseline test。
6. 明确 `federation.status`:
   - 实现 read-only daemon ability。
   - 读取 `runtime::federation_init::FederationStatusProbe` / `FederationInitOutcome` 投影。
   - 不允许新建并行状态源。
7. 给 Hub mode 与 Device mode 加 conformance tests；Hub daemon invocation surface 的 routes 必须从生产 dispatcher/exported route table 派生，不能在测试中手写第二份数组。
8. 对 stream/bidi 能力补 terminal closure 测试。
9. 文档中统一表述 `easynet-daemon` 是 runtime entrypoint。
10. 每个新增 route 必须说明属于 `DaemonInvocation`、`LocalRegistry` 还是 `AxonRuntimeAdmin` surface。
11. 每个 fallback 必须接受 review；默认删除不必要 fallback。
12. `federation.heartbeat` 的保留理由必须写成 transitional wrapper：它可以续约或兼容旧 caller，但不得成为 presence/liveness 的 source of truth。
13. `runtime.*` 前缀绕过 owner-presence resolution 是 dispatcher policy；baseline row 与 LocalRuntime 安装检查仍必须逐项显式化。

#### 9.1.A Ability registry 收敛规则

当前代码事实: `src/runtime/ability_dispatch.rs::AxonAbilityCatalog` 同时维护 static handler maps、`owner` / `authority_scope` / `manifests` compatibility side tables、`control_plane: RwLock<AbilityControlPlaneRegistry>`，并在 `DynamicCatalogue` 再镜像一套 dynamic maps。这个形态不能继续靠补丁叠加；必须收敛成一个领域对象与一个事务边界。

目标模型:

1. `AbilityControlPlaneRegistry` / `AbilityControlPlaneRecord` 是 descriptor、owner、authority scope、call mode、implementation source、manifest/version 的 canonical model。
2. `AxonAbilityCatalog` 中的 handler maps 只能作为 execution index，不再承载 metadata truth；读 owner、manifest、authority、projection 时必须经 control-plane read API。
3. Static registration 与 dynamic/hot registration 必须走同一类 transaction：control-plane record、LocalRuntime binding、handler index 三者要么一起 commit，要么一起 rollback。
4. `owner`、`authority_scope`、`manifests` side tables 必须删除，或降级为由 control-plane 派生的 private cache；cache miss 不得 fallback 到旧表制造成功。
5. `DynamicCatalogue` 不能再复制完整 catalogue shape；它只能表达 dynamic lifecycle/index overlay，或把 dynamic row 写回同一 control-plane registry 并附生命周期标记。
6. `meta.list_abilities`、catalog projection、route resolver、plugin hot register/unregister、MCP reflective registry 必须消费同一 control-plane read API。
7. 多 call mode 的同名 ability 必须通过 `(authority, ability, call_mode)` 这类 typed key 表达；不得靠字符串拼接或多张 map 的并集推断。

验收测试:

1. Static ability registration 产生 exactly one control-plane row per `(authority, ability, call_mode)`，并能证明 handler index 指向同一 row。
2. Dynamic/hot registration 成功时 control-plane、LocalRuntime binding、handler index 同步可见；失败时三者均不可见。
3. Hot unregister 移除 dynamic handler index 与 control-plane row，不留下 `meta.list_abilities` phantom entry。
4. `meta.list_abilities` / catalog projection 不读取 legacy side table。
5. 故意破坏 control-plane record 时，注册或 dispatch 必须 fail fast；不得从 handler map fallback 成功。

### 9.2 EasyNet

必须更新:

1. 将 backend 运行时定位改为 Product Wrapper。
2. 移除或废弃 `backend/internal/aggregator` 作为 ability host 的语义；当前代码中 `aggregator.go` 与 `conformance_test.go` 仍强制 Frontend catalog 走 Aggregator，这是与本 SPEC 冲突的现状债务。
3. 移除 backend-profile self-advertise loop，或降级为 transitional daemon bootstrap。当前 `aggregator.Register` / `MaintainRegistration` 会 advertise backend Agent 与 `aggregate.*` descriptors，必须被删除、反转测试，或显式标注 transitional owner/expiry。
4. 将 `internal/axon/*` 调用收敛为 daemon Invocation facade / generated contract mapping，不再承载 raw runtime semantics。当前 `internal/axon/bootstrap_self_identity.go` 等注释仍写 `axon-runtime`，必须迁移为 daemon Invocation 语义。
5. 为 Join Token 建立 first-class API:
   - mint
   - list/status
   - revoke
   - redeem/preflight
   - audit
6. 前端所有 Hub/Device/Ability 页面改为通过 EasyNet wrapper 调 daemon。
7. 所有文档/注释从 “backend hub ability” 改为 “backend product wrapper around CLI daemon”。
8. Backend wrapper 每个 handler 必须能映射到一个 daemon ability 或一个 product-only action。
9. 删除 backend-local resolver / ability catalog authoritative 逻辑。
10. 保留 `backend/internal/daemon_grpc` 作为当前 daemon client 具体实现是允许的；禁止在未重命名/迁移完成前并行创建第二套 `daemonclient` 包。逻辑边界是 daemon client，具体包名可以先是 `daemon_grpc`。
11. `backend/internal/runtime` 可以保留为产品会话 kernel / driver adapter / read model，但所有实际能力执行必须经 daemon Invocation；不得成为本地 ability runtime、resolver 或 transport owner。
12. `ServiceContext.Axon` / `AxonBidi` / `AxonReconnect` 这类旧字段名如果暂时保留，必须在注释中说明它们是 daemon Invocation client facade；新代码不得把字段名当作 raw axon-runtime 所有权依据。

### 9.3 EasyNet-Axon

必须更新或确认:

1. Join credential envelope。
2. Namespace resolve answer。
3. Federation resolve answer。
4. DirectoryEntry。
5. `list_user_devices` response shape。
6. `resolve_key` response shape。
7. Ability descriptor projection，如果跨语言复用。
8. Invocation/Receipt/Stream/Bidi conformance tests。

禁止放入 Axon:

1. EasyNet account DB shape。
2. Product Join Token table shape。
3. Frontend DTO。
4. Product permission policy。
5. Daemon plugin policy。

### 9.4 预期更新后的项目结构

本次结构调整应优先保持现有仓库大形状稳定，通过新增清晰子域和迁移责任边界完成重构。不要做一次性大搬家，避免把架构纠偏变成无关路径重命名。

#### 9.4.1 EasyNet-Cli 目标结构

EasyNet-Cli 是 runtime 本体。预期结构应围绕 `daemon -> runtime -> services -> facade/ffi` 展开:

```text
EasyNet-Cli/
  docs/
    spec/
      easynet-wrapper-cli-runtime-refactor-spec-2026-06-28.md
    architecture/
      daemon-runtime-boundary.md
      hub-device-mode.md
    runbooks/
      join-token-cli-flow.md

  schemas/
    descriptor/
    receipt/
    hub-baseline/
      abilities.yaml
    device-baseline/
      abilities.yaml

  src/
    bin/
      easynet.rs
      easynet-daemon.rs

    daemon/
      config.rs
      lifecycle.rs
      mode.rs
      control_plane.rs
      invocation_endpoint.rs
      health.rs
      join.rs

    services/
      control/
      federation_client/
      invocation_transport/
        daemon_invocation_service.rs
        federation_wrappers.rs
        stream.rs
        bidi.rs
        boot/
        session_initiator/

    runtime/
      ability/
        descriptor.rs
        registry.rs
        catalog_projection.rs
        conformance.rs
      hub/
        directory.rs
        federation.rs
        namespace.rs
        identity.rs
        key_resolution.rs
        catalog.rs
      resolver/
        namespace.rs
        route.rs
        proxy.rs
      agents/
      execution/
      resources/
      keyring/
      federation_init/
      plugin_host/
      axon_bridge/

    eal/
    plugins/
      builtin/
      remote_desktop/

    facade/
      cli/
        groups/
        join.rs
        hub.rs
        device.rs
      mcp/

    ffi/
      daemon_client.rs
      invocation.rs
      lifecycle.rs

  tests/
    conformance/
      hub_baseline.rs
      device_baseline.rs
      join_token_cli.rs
      stream_bidi_closure.rs
    e2e/
      wrapper_product_flow.rs
```

关键结构规则:

1. `src/daemon` 只负责 daemon 生命周期、mode、control/invocation endpoint、join bootstrap，不放具体业务能力实现。
2. `src/runtime/hub` 承载 Hub baseline ability 的 daemon-owned 实现，不承载 canonical baseline list。
3. `src/runtime/ability` 承载 descriptor、registry、catalog projection、baseline conformance。
4. `src/services/invocation_transport/route_resolver.rs` 是当前 namespace/proxy route owner；若迁移到 `src/runtime/resolver`，必须删除旧 owner 并更新所有 call sites，EasyNet backend 不再有 authoritative resolver。
5. `src/services/invocation_transport` 承载 unary/stream/bidi 的 daemon transport 与 wrapper，不承载产品 API。
6. `src/facade/cli` 提供 CLI 命令，包括 `easynet join <token>`、hub/device status、ability catalog 查询。
7. `src/ffi` / `libeasynet_cli` 只暴露 daemon lifecycle 与 generic Invocation，不新增 one-method-per-ability ABI。
8. `tests/conformance` 固化 Hub baseline、Device baseline 与 stream/bidi terminal closure。

现有目录与目标结构的对应关系:

1. `src/runtime/hub` 继续作为 Hub runtime 收口点，但需要补全 baseline ability。
2. `src/services/invocation_transport` 继续作为 daemon invocation transport 收口点，但需要确保完整 Invocation 七元组。
3. `src/services/invocation_transport/route_resolver.rs` 继续作为当前 concrete route resolver；不得为了目标目录图再创建第二个 `src/runtime/resolver`。
4. `src/runtime/agents/profiles` 继续承载 device/profile projection，但不应变成 backend profile source。
5. `src/facade/cli/groups` 可以保留现有命令组织，新 join/hub/device 命令可按现有风格接入。
6. `abilities/system` 与 `plugins/builtin` 作为 ability implementation/resource plane，不能替代 `AbilityDescriptor` registry。
7. `src/runtime/ability/conformance.rs` 是 baseline contract 的 canonical source；Hub、Device、transport、registry 测试只能消费它，不能复制列表。

#### 9.4.2 EasyNet 目标结构

EasyNet 是产品包装层。预期结构应把 backend 分成 product、join、daemon client、wrapper、handler，而不是继续让 `internal/aggregator` 或 `internal/axon` 承载 runtime。

当前代码事实: `backend/internal/daemon_grpc` 已经是 daemon Invocation client 的具体实现，并包含 AXIOM 7-tuple 到 proto 的 mapping；迁移时应把它作为 daemon client boundary 收敛或原子重命名，不能再复制一层平行 client。`backend/internal/aggregator`、`backend/internal/axon`、`backend/internal/runtime` 仍存在，必须按下列语义分别收敛。

```text
EasyNet/
  backend/
    api/
      easynet.api

    ent/
      schema/
        user.go
        user_peer_hub.go
        device_pairing.go
        join_token.go
        join_token_audit.go

    internal/
      product/
        auth/
        users/
        organizations/
        permissions/

      join/
        token.go
        mint.go
        redeem.go
        revoke.go
        audit.go
        envelope.go

      daemonclient/
        client.go
        lifecycle.go
        health.go
        invocation.go
        stream.go
        bidi.go
        directory.go

      wrapper/
        abilities/
        agents/
        devices/
        files/
        terminal/
        browser/
        remote_desktop/
        realtime/
        admin/
        openai/

      handler/
        auth/
        join/
        device/
        ability/
        invocation/
        file/
        terminal/
        streambridge/
        sse/
        system/
        user/

      events/
        daemon_bridge.go
        sse_fanout.go

      axoncontract/
        generated/
        mapping.go

      config/
      middleware/
      store/
      svc/

    docs/
      backend-as-cli-wrapper.md
      join-token-product-flow.md

  Frontend/
    src/
      lib/
        api/
          easynet-auth.ts
          easynet-join-tokens.ts
          easynet-daemon-status.ts
          easynet-devices.ts
          easynet-abilities.ts
          easynet-invocation.ts
          easynet-realtime.ts
          easynet-files.ts
          remote-desktop-session.ts
      pages/
        easynet/
          RegisterPage.tsx
          JoinTokenPage.tsx
          DevicesPage.tsx
          AbilitiesPage.tsx
          AbilityDetailPage.tsx
          TerminalPage.tsx
          WebBrowserPage.tsx
          RemoteDesktopPage.tsx
      store/
        easynet-auth-store.ts
        daemon-connection-store.ts
        terminal-store.ts
        browser-session-store.ts
```

关键结构规则:

1. `backend/internal/product` 只负责产品账号、权限、组织、用户态资源。
2. `backend/internal/join` 只负责 Join Token 的产品生命周期，不执行 runtime join。
3. `backend/internal/daemonclient` 是 EasyNet backend 与本地 `easynet-daemon` 的唯一 runtime 接口；当前 `backend/internal/daemon_grpc` 可作为该边界的具体包名/实现，直到有原子重命名计划。
4. `backend/internal/wrapper` 是 handler 与 daemonclient 之间的产品 wrapper 层，所有 wrapper 必须能映射到 daemon ability。
5. `backend/internal/handler` 只做 HTTP/WS/SSE 边界，不放 runtime 执行逻辑。
6. `backend/internal/events` 可以从 daemon directory subscription fan out 到 SSE/WS，但不能自建 runtime presence。
7. `backend/internal/axoncontract` 只放 generated contract / mapping，不放 raw Axon runtime client 语义。
8. Frontend 只调用 EasyNet product API，不直接决定 daemon route。

需要迁移或废弃的现有目录语义:

1. `backend/internal/aggregator` 默认删除。只有在前端迁移窗口被 owner 明确批准时，才允许临时降级为调用 `daemonclient` / `daemon_grpc` 的 compatibility wrapper；该 wrapper 必须有 owner、expiry、telemetry 和 deletion criteria。现有“Frontend MUST go through Aggregator”的注释、`aggregator.Register` / `MaintainRegistration`、以及 aggregator conformance test 必须删除或反转为禁止新增 aggregate 依赖。
2. `backend/internal/axon` 需要收敛为 contract/mapping、Invocation request builder、daemon invocation helper，不能继续承载 raw runtime 或把目标写成 axon-runtime。
3. `backend/internal/federation` 如果保留，只能是产品 peer hub 配置与 wrapper，不是 federation runtime。
4. `backend/internal/resolverstate` 如果保留，只能是 UI/cache/read model，不是 authoritative resolver。
5. `backend/internal/runtime` 如果保留，只能表达产品会话 kernel、PTY/browser/remote-desktop driver adapter、runtime health/read model；driver 可以调用 daemon Invocation，但不能本地执行 ability implementation，也不能拥有 namespace resolver。
6. `backend/internal/registry` 如果保留，只能是 product registry/read model，不是 ability descriptor source of truth。

#### 9.4.3 EasyNet-Axon 目标结构

EasyNet-Axon 继续作为协议与 SDK contract 仓库。预期结构保持 `core/proto`、`runtime-rs`、`ura-rs`、`packaging` 主线。

```text
EasyNet-Axon/
  core/
    proto/
      axon/
        v1/
          invocation.proto
          receipt.proto
          stream.proto
          federation.proto
          namespace.proto
          directory.proto
          join.proto
          ability_descriptor.proto

    runtime-rs/
      src/
        invocation/
        receipt/
        stream/
        federation/
        namespace/
        admission/
      client-sdk/
      dendrite-bridge/

    ura-rs/
      src/

  packaging/
    protocol-pack/
      conformance-vectors/
        invocation/
        receipt/
        namespace/
        join/
        directory/
    sdk-pack/
    release/

  docs/
    rfc/
    design/

  document/
    concepts/
    rfcs/
```

关键结构规则:

1. `join.proto` 只定义 credential envelope，不定义 EasyNet product token table。
2. `namespace.proto` 定义 resolve answer，不定义 EasyNet UI DTO。
3. `directory.proto` 定义 DirectoryEntry / list user devices shape，不定义 frontend list item。
4. `ability_descriptor.proto` 定义跨语言 descriptor projection，不定义 EasyNet 页面字段。
5. `runtime-rs` 可以提供 reference runtime 和 SDK primitives，但不启动 `easynet-daemon`。
6. `packaging/protocol-pack/conformance-vectors` 提供 Go/Rust/TS 可共享测试向量。

#### 9.4.4 三仓库依赖方向

允许的依赖方向:

```text
EasyNet backend
  -> EasyNet-Cli daemon endpoint / libeasynet_cli client
  -> EasyNet-Axon generated contracts

EasyNet-Cli
  -> EasyNet-Axon runtime primitives and generated contracts

EasyNet-Axon
  -> no dependency on EasyNet or EasyNet-Cli product code
```

禁止的依赖方向:

```text
EasyNet-Axon
  -> EasyNet product DB / CLI daemon plugin policy

EasyNet-Cli runtime
  -> EasyNet backend product handlers

EasyNet backend
  -> raw ability implementation / authoritative resolver / independent Hub runtime
```

#### 9.4.5 最终可读心智模型

更新后的项目结构应让工程师一眼看出:

1. 要改协议字段，去 EasyNet-Axon。
2. 要改 Device/Hub runtime 能力，去 EasyNet-Cli。
3. 要改用户注册、Join Token 页面、邀请传播、产品 API，去 EasyNet。
4. 要改浏览器页面展示，去 EasyNet `Frontend`。
5. 要改 daemon invocation transport，去 EasyNet-Cli `src/services/invocation_transport`。
6. 要改 backend 到 daemon 的产品包装，去 EasyNet `backend/internal/wrapper` + daemon client boundary；当前具体实现是 `backend/internal/daemon_grpc`，若要改名为 `daemonclient` 必须原子迁移。
7. 要改 ability descriptor registry，去 EasyNet-Cli `src/runtime/ability`，不是 EasyNet backend。

## 10. 迁移策略

### 10.1 第一阶段: 固定边界

1. 在文档与代码注释中统一 runtime 边界。
2. EasyNet backend handler 标注为 wrapper 或 product-only。
3. 找出所有 backend-local ability execution / aggregation / resolver 代码路径。
4. 列出所有前端依赖的 legacy backend product endpoints / wrapper endpoints。
5. 将每个 API 映射到 daemon ability、Axon contract 或 product-only action。

### 10.2 第二阶段: CLI daemon 补齐 Hub baseline

1. 补齐缺失 Hub baseline abilities，并先补 typed conformance row。
2. 为 Hub mode 建立 conformance test。
3. 为 Device mode 建立 conformance test。
4. 统一 catalog projection。
5. 将 `federation.status` 固化为 read-only daemon ability，并补 state projection 与 conformance test。
6. 将 `aggregate.list_abilities_catalog` 从 canonical baseline 移除；如需 transitional alias，必须单独写 migration owner、expiry、telemetry 和 deletion criteria。

### 10.3 第三阶段: EasyNet wrapper 改造

1. Join Token API first-class 化。
2. Backend 连接 local daemon。
3. Ability invoke endpoints 改为 generic Invocation wrapper。
4. Device/Agent/Ability 页面改为 daemon-backed data。
5. SSE/WS 改为 daemon event bridge。
6. 文件/终端/remote desktop/browser 改为 daemon stream/bidi wrapper。

### 10.4 第四阶段: 删除旧 runtime 假象

1. 删除 backend-local aggregator runtime 语义。
2. 删除 backend self-profile advertise。
3. 删除 stale hub ability docs。
4. 删除 Go/Rust 手写重复 wire DTO，改用 Axon contract。
5. 加入 CI/conformance 防止回归。

## 11. 兼容约束

1. 可以暂时保留旧 HTTP endpoints，但实现必须变成 daemon wrapper。
2. 兼容层必须有 deprecation 注释与迁移目标。
3. 不允许新增 backend-local ability implementation。
4. 不允许新增 backend-local authoritative ability catalog。
5. 不允许为了兼容而丢弃 Invocation 七元组。
6. 不允许把 `aggregate.list_abilities_catalog` 变成新的 canonical API。
7. 不允许把 `federation.forward_invoke` 暴露为 browser-facing endpoint。
8. 兼容 endpoint 的响应 DTO 可以不等于 Axon wire shape，但内部调用必须保留 canonical invocation。
9. Backend-profile `aggregate.*` registration/self-advertise loop 默认删除；若短期保留，只能作为 daemon-backed transitional alias，并必须带 owner、expiry、telemetry、negative canonical-baseline test 与删除条件。
10. 旧字段名如 `ServiceContext.Axon` / `AxonBidi` / `AxonReconnect` 不构成架构所有权；保留时必须在注释中声明它们是 daemon Invocation client facade。

## 12. 验收标准

### 12.1 End-to-end product path

完整流程必须成立:

```text
Register user
  -> Create join token
  -> CLI redeem token
  -> CLI receives membership acknowledgement
  -> daemon session.open establishes presence
  -> daemon advertise_agent publishes hosted-agent linkage
  -> daemon advertise_abilities publishes owner projection read model
  -> frontend sees device directory event
  -> frontend lists device abilities
  -> frontend invokes a device ability through EasyNet wrapper
  -> daemon returns receipt
  -> revoke removes presence/trust
```

### 12.2 Runtime ownership

1. Hub mode 和 Device mode 均由 `easynet-daemon` 承载。
2. EasyNet backend 没有本地执行 ability implementation 的路径。
3. EasyNet backend 没有 authoritative namespace resolver。
4. EasyNet backend 没有 authoritative ability catalog。
5. 每个 wrapper handler 都能指向 daemon ability。

### 12.3 Invocation correctness

1. 所有 public invoke 使用完整 Invocation 七元组。
2. Signed invoke 必须包含 nonce。
3. `subject` 必须显式或由可审计规则生成。
4. `causal_context` 不允许静默丢弃。
5. Receipt terminal state deterministic。
6. Stream/bidi 有 terminal closure。
7. `runtime.invoke_remote` produces zero-or-more chunks plus exactly one terminal result, or aborts on pre-terminal close。
8. `session.open` phase transitions are typed and legal-edge checked。
9. Pending dispatch entries are removed on reply, caller cancellation, or target offline cancellation。
10. Directory streams close when producers drop and emit heartbeat on healthy idle v2 streams。

### 12.4 Join correctness

1. Token single-use。
2. Token expiring。
3. Token revocable。
4. Redeem 后写入 CLI credential/config。
5. Daemon 执行 `federation.join` membership acknowledgement。
6. Daemon 通过 `session.open` 建立 presence。
7. Daemon 通过 `federation.advertise_agent` 发布 hosted-agent linkage；通过 `federation.advertise_abilities` 发布 owner projection read model。
8. Backend 不代替 daemon 成为 joined runtime。

### 12.5 Contract correctness

1. 跨 Go/Rust/前端复用 shape 有 Axon single source。
2. `resolve_key` 编码有 single source。
3. DirectoryEntry 有 single source。
4. Join credential envelope 有 single source。
5. Ability descriptor projection 如跨语言复用，有 single source。

## 13. Review Checklist

### 13.1 Boundary Checklist

- [ ] 这是 runtime behavior 吗？如果是，必须在 EasyNet-Cli。
- [ ] 这是 account/product DB 吗？如果是，必须在 EasyNet。
- [ ] 这是 cross-language wire shape 吗？如果是，必须在 Axon。
- [ ] Backend 是否执行或模拟了 ability？如果是，拒绝。
- [ ] CLI daemon 是否拥有 locality / resolution / session？如果不是，拒绝。
- [ ] Product permission 是否被误用为 runtime admission？如果是，拒绝。
- [ ] Backend DB state 是否被误用为 runtime directory state？如果是，拒绝。

### 13.2 Invocation Checklist

- [ ] `caller` present。
- [ ] `callee` present。
- [ ] `ability` present。
- [ ] `subject` explicit or validated。
- [ ] `nonce` present for signed public calls。
- [ ] `causal_context` explicit。
- [ ] `args` descriptor-valid。
- [ ] Runtime admission path explicit。
- [ ] Receipt terminal state deterministic。
- [ ] Stream/bidi has terminal closure。

### 13.3 Join Token Checklist

- [ ] Token minted by EasyNet product API。
- [ ] Token single-use。
- [ ] Token expiring。
- [ ] Token revocable。
- [ ] Token redeem audited。
- [ ] Credential envelope shape owned by Axon contract。
- [ ] CLI redeem writes credentials。
- [ ] CLI redeem writes daemon config。
- [ ] CLI redeem updates realm trust / federated peers。
- [ ] Daemon performs `federation.join` membership acknowledgement without writing presence。
- [ ] Daemon establishes presence through `session.open`。
- [ ] Daemon performs `federation.advertise_agent` for hosted-agent linkage。
- [ ] Daemon performs `federation.advertise_abilities` for owner projection read model。
- [ ] Backend does not join on behalf as runtime。

### 13.4 Ability Ownership Checklist

- [ ] Device baseline abilities are registered in CLI daemon。
- [ ] Hub baseline abilities are registered in CLI daemon。
- [ ] Baseline abilities are represented by typed rows with `name`, `call_mode`, `surface`, and `domain`。
- [ ] Daemon Invocation surface passes Hub baseline conformance。
- [ ] Local registry passes Device baseline conformance。
- [ ] `openai.*` compatibility abilities are classified as device-owned unless explicit `hub.openai.*` exists。
- [ ] `aggregate.list_abilities_catalog` is not canonical daemon baseline。
- [ ] Any transitional `aggregate.list_abilities_catalog` alias has owner, expiry, telemetry, and deletion criteria。
- [ ] `federation.status` is daemon read-only ability backed by federation/init state projection。
- [ ] `meta.list_abilities` can serve frontend catalog needs。
- [ ] Ability metadata reads from `AbilityControlPlaneRegistry`; handler maps are execution indexes only。
- [ ] Static and dynamic ability registration share one commit/rollback path。
- [ ] `federation.subscribe_directory_v2` is used for frontend realtime directory。

### 13.5 EasyNet Wrapper Checklist

- [ ] HTTP handler has no runtime execution logic。
- [ ] Handler calls daemon ability through Invocation or stream/bidi wrapper。
- [ ] DB query only selects product scope。
- [ ] DB query does not route runtime execution。
- [ ] Response is DTO projection only。
- [ ] Daemon offline behavior is explicit。
- [ ] Daemon reconnect behavior is explicit。
- [ ] Wrapper errors distinguish product auth error from runtime admission error。

### 13.6 Axon Contract Checklist

- [ ] No long-lived Go/Rust hand-mirrored shape without Axon source。
- [ ] Join credential envelope has neutral contract。
- [ ] Namespace resolve answer has neutral contract。
- [ ] DirectoryEntry has neutral contract。
- [ ] `list_user_devices` shape has neutral contract。
- [ ] `resolve_key` shape has neutral contract。
- [ ] Ability descriptor projection has neutral contract if cross-language。
- [ ] Invocation/Receipt/Stream/Bidi conformance tests exist。

### 13.7 Test Checklist

- [ ] User registration + runtime pubkey registration E2E。
- [ ] Join token mint E2E。
- [ ] Join token redeem E2E。
- [ ] `easynet join <token>` E2E。
- [ ] `federation.join` E2E。
- [ ] `federation.advertise_agent` E2E。
- [ ] `federation.advertise_abilities` E2E。
- [ ] `session.open` establishes presence and offline event on close。
- [ ] `runtime.invoke_remote` returns terminal result or aborted pre-terminal close。
- [ ] Pending dispatch cancellation on target offline。
- [ ] `subscribe_directory_v2` emits idle heartbeat and closes when producers drop。
- [ ] `namespace.resolve` E2E。
- [ ] `meta.list_abilities` catalog E2E。
- [ ] Signed invoke E2E。
- [ ] File ability wrapper E2E。
- [ ] Terminal stream/bidi E2E。
- [ ] Browser/remote desktop stream/bidi E2E。
- [ ] `federation.subscribe_directory_v2` -> SSE E2E。
- [ ] Revoke/removal E2E。
- [ ] Daemon offline/reconnect E2E。

### 13.8 Project Structure Checklist

- [ ] EasyNet-Cli `src/daemon` only owns daemon lifecycle/mode/endpoints/bootstrap。
- [ ] EasyNet-Cli `src/runtime/hub` owns Hub baseline ability implementations。
- [ ] EasyNet-Cli `src/runtime/ability` owns descriptor registry, catalog projection, and baseline conformance。
- [ ] EasyNet-Cli has exactly one namespace/proxy route owner: current `src/services/invocation_transport/route_resolver.rs` or an atomic replacement, never both。
- [ ] EasyNet-Cli `src/services/invocation_transport` owns unary/stream/bidi transport with full Invocation。
- [ ] EasyNet-Cli `src/ffi` exposes generic daemon lifecycle/invocation, not one method per ability。
- [ ] EasyNet backend has a single daemon client runtime access layer (`daemonclient` logical boundary; current concrete package may be `daemon_grpc`)。
- [ ] EasyNet backend wrapper handlers do not import or execute ability implementations。
- [ ] EasyNet backend `join` module owns token lifecycle but not runtime join。
- [ ] EasyNet backend `aggregator` is removed or explicitly marked transitional daemon-backed compatibility with owner, expiry, telemetry, and deletion criteria。
- [ ] EasyNet backend `runtime` package, if retained, is product session kernel / driver adapter only and calls daemon Invocation for execution。
- [ ] EasyNet backend `axoncontract` contains generated contract/mapping only。
- [ ] EasyNet backend `internal/axon` comments and package contracts no longer claim raw axon-runtime ownership for product paths。
- [ ] EasyNet frontend API clients call EasyNet product APIs, not daemon routes directly。
- [ ] EasyNet-Axon owns shared protocol/proto/conformance vectors only。
- [ ] No dependency from EasyNet-Axon to EasyNet product code or CLI plugin policy。
- [ ] No dependency from EasyNet-Cli runtime to EasyNet backend handlers。

## 14. 已决事项与开放问题

### 14.1 已决事项

1. `aggregate.list_abilities_catalog` 不进入 canonical daemon baseline。默认迁移到 `meta.list_abilities` + daemon catalog projection。
2. `federation.status` 成为 daemon read-only ability，读取 `FederationStatusProbe` / `FederationInitOutcome` 投影。
3. Baseline contract canonical source 放在 EasyNet-Cli `src/runtime/ability/conformance.rs`，不是 `src/runtime/hub` 或 transport wrapper。
4. 通配 ability group 必须在实现前展开成 typed rows。
5. EasyNet backend 当前 `daemon_grpc` 是 daemon client 边界的具体实现；迁移不得并行制造第二套 runtime client。
6. EasyNet backend `runtime` 可以作为产品会话 kernel / driver adapter 保留，但不能成为 ability runtime。
7. `federation.join` 只表达 membership acknowledgement，不写 presence；presence 只能由 daemon-owned `session.open` 建立。
8. `federation.advertise_agent` 与 `federation.advertise_abilities` 是 projection/read-model 发布，不是 ability implementation registry。
9. EasyNet-Cli 当前 route owner 是 `DaemonRouteResolver`；任何目录迁移必须替换它，不能复制它。
10. EasyNet backend `aggregator.Register` / `MaintainRegistration` 的 backend-profile self-advertise loop 默认删除；若临时保留，必须作为有 owner/expiry/telemetry/deletion criteria 的 transitional alias。
11. EasyNet 对外应称为 Product API wrapping CLI daemon；不得用 “Hub API” 表达 backend 自己拥有 runtime/hub ability。
12. 前端 legacy endpoint 路径默认不保留；若产品发布需要短期兼容，必须按第 11 节 compatibility wrapper 规则显式写 owner、expiry、telemetry 与删除条件。

### 14.2 需要 review 的开放问题

1. Join credential envelope 的第一版字段是否包括:
   - realm id
   - hub endpoint
   - peer id
   - device id suggestion
   - user id binding
   - public key material
   - expiry
   - token id
   - signature
2. `hub.openai.*` 是否进入本次 refactor，还是推迟到 Hub-owned gateway 设计？
3. Join Token redeem 是否需要 product-level grace period，还是严格 one-time immediate invalidation？
4. Browser/Remote Desktop stream 是否复用 daemon stream/bidi receipt model，还是需要 product-side session projection table？若需要 table，必须证明它不是 canonical runtime state。

## 15. 本地证据引用

当前代码里与本 SPEC 直接相关的文件:

1. `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/src/services/invocation_transport/federation_wrappers.rs`
2. `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/src/services/invocation_transport/daemon_invocation_service.rs`
3. `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/src/runtime/agents/profiles/device.rs`
4. `/Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend/internal/daemon_grpc/invoke_remote.go`
5. `/Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend/internal/aggregator/aggregator.go`

这些引用的作用是 review 边界与迁移方向，不表示全部改动只限于这些文件。

### 15.1 本重构必须钉死的已知 latent bug

以下三处不是 review 引用，而是本重构必须修掉的具体缺陷，已逐一在代码中核实(2026-06-28):

1. `src/services/invocation_transport/federation_invoke.rs:319` — `causal_context_bytes: Vec::new()`：`federation.forward_invoke` 外层跳没有透传 causal context,违反第 5 节不变量 1(七元组完整)。必须把 inbound invocation 的 causal context 接到 forward hop。
2. `src/runtime/owner_projection.rs:74` — `AbilityProjectionSummary` 手写第 11 个字段 `callable_summary`,携带 proto(`namespace.proto`)不承认的 impl-private 数据。这是当前唯一真实的 Axon wire 手写镜像 fork;必须 reconcile 进 Axon descriptor projection contract,或证明它纯属 daemon-local 不跨边界。
3. `backend/internal/axon/urns.go` 文档说 `UserURA` 参数是 `users.id`(UUID),但 `prepareEnvelopeLogic.go` 传 `username`(见第 6.1 节规则 4)。必须钉死 canonical subject 锚点并统一两条路径。
