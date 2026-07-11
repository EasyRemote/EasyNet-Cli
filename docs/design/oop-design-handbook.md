# EasyNet 平台 OOP 设计技术手册

**版本**：v1（2026-06-25 历史快照）
**面向读者**：CTO 凉冰 / Silan、EasyNet 工程团队（莫浩、海峰、晓雯及新入职工程师）
**性质**：非规范性历史手册。当前边界只以 [`../ARCHITECTURE_STATE.md`](../ARCHITECTURE_STATE.md) 为准；本文件中的类型名、行号和设计评价不得作为当前实现依据。
**仓库**：`EasyNet-Cli`（下游消费方）、`EasyNet-Axon`（本体 / 协议宿主，相对路径 `../EasyNet-Axon`）

---

## 0. 摘要与代码量基线

### 0.1 代码量基线（已测量）

| 仓库 | 生产 Rust LOC | 测试 LOC | 测试占比 | 角色 |
|---|---|---|---|---|
| EasyNet-Cli | ~155.8k | ~107k | ~41%（test / (prod+test)） | 下游消费方：daemon、能力、EAL、CLI facade |
| EasyNet-Axon | ~84k | ~54k | ~39% | 本体宿主：Envelope / URA / Invocation / Receipt 的唯一定义源 |

测试占比近 40%，对于一个仍在演进、跨仓库、带密码学受签语义（签名、回执链、admission）的系统，这个比例是健康的；它是本手册敢于把"非 lossy 错误转换""typestate 不可表示非法状态"等论断当作事实陈述的底气来源——这些不变量都有 `#[test]` 守护（如 `error_codes_are_stable_and_unique`、`axon_error_stays_within_result_budget`、`proof_boundary_does_not_use_plain_encoder`）。

### 0.2 本体定义在一处

`Envelope / URA / Invocation / Receipt` 这套本体 **只在 Axon proto + Rust SDK builder 中定义一次**，CLI 通过 facade / 投影（projection）消费，**从不 fork 线缆语法（wire grammar）**。这是第 8 章"跨仓接缝"的核心论断，也是理解整个平台分层的钥匙：**Axon 定义并规范化（define & canonicalize），CLI 富化并派发（enrich & dispatch）。**

### 0.3 五条主干（five-trunk domain spine）

平台的领域骨架是五条主干，按依赖顺序排列：

```
Identity  →  Admission  →  Policy  →  Receipt  →  Discovery
（身份）     （准入）      （策略）    （回执）    （发现）
```

- **Identity**：`Ura` / `ParsedURA` / `URAKind`（`ura-rs/src/lib.rs`）—— 类型化身份分解
- **Admission**：`run_admission` + `NonceReplayStore`（`admission.rs`）—— 语法 + 重放 + 签名校验
- **Policy**：`NamespacePolicyGate` / `ProcessLiveness` / `AuthorityBinding` —— 准入资格评估
- **Receipt**：`InvocationReceipt` + `AxiomBinding`（`audit.rs`）—— 哈希链审计
- **Discovery**：`NamespaceResolver` 四件套 + `DiscoverFederationResolver` —— 命名空间解析

注意：`trust` 是**属性（attribute）**，不是主干。它分布在 admission 的模式（Local-fast / Trusted / Federated）和 `AgentRecord.trust_level` 上，而不是独立一根支柱。

### 0.4 本手册覆盖的内容

| 章 | 主题 | 主要仓库 |
|---|---|---|
| 1 | 设计哲学：Rust 即 OOP | 两仓 |
| 2 | 本体层（Axon SDK）：URA + Envelope/Receipt | Axon |
| 3 | 运行时层：Axon server + CLI 三注册表控制面 | 两仓 |
| 4 | 能力层：agent/ability trait + 实现 | CLI |
| 5 | 传输层：dispatcher / session 对象模型（诚实记录其 pre-refactor 状态） | CLI |
| 6 | 编排与语言层：EAL 编译器 + CLI facade | CLI |
| 7 | 横切设计：错误模型、trait 分类、状态机清单、所有权约定、newtype 纪律、异步对象模型 | 两仓 |
| 8 | 跨仓接缝：facade / re-export 机制 | 两仓 |
| 9 | 设计评价：优点、坏味道、终局方向 | 两仓 |

---

## 1. 设计哲学 —— Rust 即 OOP

这是 Rust 代码库。"OOP"在这里不是 Java 式的类继承层级，而是 Rust 表达对象导向的七种惯用法。本手册始终以这七种惯用法为透镜来描述代码。

### 1.1 七种 OOP 表达手段

| OOP 概念 | Rust 表达 | 本代码库的代表 |
|---|---|---|
| 类 + 方法 | `struct` + `impl` 块 | `AxonRuntime` / `AxonState`、`AxonAbilityCatalog`、`DaemonInvocationService` |
| 接口 / 多态 | `trait` + `dyn`（trait 对象）或泛型 | `SessionProvider`、`StepDispatcher`、`DiscoverFederationResolver`、`KeyResolver` |
| 不可表示的非法状态 | typestate（编译期 gated enum） | `UnaryInvokeAdmissionState`、`AbilityControlPlaneRegistrationStage`、`TerminalProofState` |
| 强类型标识 | newtype（私有内层基元） | `Ura(String)`、`SchemaHash([u8;32])`、`AbilityDescriptorVersion(String)` |
| 析构 / 资源清理 | `Drop` / RAII | `NodeIdLock`、`ExclusiveFileLock`、`CalleeSigner`（`ZeroizeOnDrop`）、`HeartbeatPump` |
| 和类型（sum type） | `enum` | `CausalContext`、`AuthorityBinding`、`OwnerKind`、`AbilityImplSource`、`InvocationState` |
| 共享所有权 | `Arc<T>` + 内部可变性（`DashMap`/`RwLock`/atomics） | `Arc<AxonState>`、`Arc<InvocationCore>`、`Arc<AxonAbilityCatalog>` |

### 1.2 一条贯穿全平台的设计公理：parse, don't validate

整个代码库奉行 **"解析而非校验后传字符串"**：基元（`String` / `[u8;32]` / `i32`）穿越模块边界时，**只能裹在一个拥有自己校验逻辑的 newtype 里**。构造即校验门（construction is the validation gate），内层基元一律私有。

最纯粹的例子是 `Ura(String)`（`ura-rs/src/lib.rs:361`）：内层 `String` 私有，只能经 `.as_str()`（借用）或 `.into_string()`（消费）取出。这条公理在第 2、3、7 章反复出现。

### 1.3 traits 抽象"接缝"而非"数据形状"

一条贯穿全平台的分层规则（第 7 章详述）：**trait 只抽象一个接缝（seam）—— 传输后端、密钥源、时间源、策略门，从不抽象数据形状。** 数据形状一律用 newtype / struct / enum 表达。

因此你**不会**在这个代码库里找到一个庞大的 `Transport` 或 `Ability` 上帝 trait 层级。整个协议表面就是一个 tonic `Invocation` trait（三个方法 `invoke` / `invoke_stream` / `invoke_bidi`）；federation、voice、mission、capability **都是这一个 trait 下的能力名（ability NAME），不是新的 service trait**。这是理解整个平台"协议面被压平成单一 gRPC service"的关键。

---

## 2. 本体层（Axon SDK）—— 唯一真理源

本章描述 `EasyNet-Axon/sdk/rust` 与 `core/ura-rs`：URA / identity 本体，以及 Envelope / Admission / Receipt 对象模型。这是所有受签字节的规范定义处。

### 2.1 URA / Identity 本体

URA（EasyNet 资源地址）是身份主干。语法规范**只定义一次**，落在 `parse_ura` + `parse_ability_tail` 两个函数里；所有 builder（`Ura::agent` / `::device` / `::ability` …）和所有访问器（`ParsedURA::agent_ids()` …）都路由经过这两个函数——builder 与 parser 之间没有漂移空间。

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `Ura` | newtype | 已校验 URA 的 OOP 边界；内层 `String` 私有，仅存完整 `easynet:///r/...` 规范地址 | `core/ura-rs/src/lib.rs:361` |
| `ParsedURA` | struct | URA 的类型化分解：realm + kind + role-specific body；body 私有，只能经访问器查询 | `core/ura-rs/src/lib.rs:155` |
| `URAKind` | enum | 角色分类和类型：Hub / Device / User / Agent / Ability / Resource / Unknown | `core/ura-rs/src/lib.rs:66` |
| `ParsedURABody` | enum（私有） | 角色专属解析数据；判别式与 `URAKind` 对齐；外部**不能** pattern-match | `core/ura-rs/src/lib.rs:162` |
| `AbilityOwner` | enum | owner-token-first 分类（RFC-005 §3.1.1）：Hub（bare）/ Agent（user_id+agent_id）/ Device（device_id） | `core/ura-rs/src/lib.rs:121` |
| `ParsedAbility` | struct | ability tail 分解：owner-token.namespace.local-name | `core/ura-rs/src/lib.rs:135` |
| `CanonicalAbilityPath` | struct | 校验过的规范 ability URA 部件，用于路由 / 能力编码 | `core/ura-rs/src/lib.rs:147` |
| `ResourceNamespace` | enum | 资源命名空间：Fs / Process / Pty / Shell / Http | `core/ura-rs/src/lib.rs:92` |
| `ParseError` | enum | 类型化解析错误（`UserBadShape` vs `DeviceBadShape` 等高特异性变体） | `core/ura-rs/src/lib.rs:265` |

**关键设计：sealed enum + 查询方法。** `ParsedURABody` 私有，`URAKind` + body 判别式封印在 `ParsedURA` 内。调用方**不能**直接 match body，只能调类型化访问器（`.agent_ids()` → `Option<(&str,&str)>`、`.resource_path()` …）。对一个 User URA 调 `.agent_ids()` 返回 `None`，**永不 panic**。这是单一真理源的强制手段。

**owner-token-first（RFC-005）：** `AbilityOwner` 按 ability-tail 前缀判别（`hub.` → Hub、`device.` → Device、bare → Agent），**绝不靠字符串形状推断**。`hub` / `device` 作为 bare 首段被保留并对 agent 拒绝。这是 RFC-001 那个"前缀推断 owner"P0 回归的根治。

**Identity 层 trait（都是 std trait 落在 newtype 上）：** `Display`（`:470`）、`AsRef<str>`（`:476`，零拷贝借用）、`TryFrom<&str>`（`:482`，fallible parse）、`From<Ura> for String`（`:490`，owned 提取）、`std::error::Error`（`ParseError`，`:351`）。**这里没有大 trait——"trait"其实就是那个单一 parse 函数。**

### 2.2 Node-ID 锁与进程存活（Identity 主干的防御性子系统）

`client-sdk/src/domain/identity/node_id/` 是身份层最防御性的部分，体现 RAII 与状态机两种 OOP 手段。

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `LockOwnerInfo` | struct | 持久锁文件元数据：owner_token / pid / hostname / host_fingerprint / pid_start_marker / created_unix_ms；text `key=value` 编解码 | `node_id/types.rs:40` |
| `NodeIdLock` | struct | RAII 守卫，`Drop` 时删锁文件，防 panic/早返回导致锁泄漏 | `node_id/types.rs:29` |
| `ProcessLiveness` | enum | Alive / Dead / Unknown；探测不确定时保守落到 Unknown | `node_id/types.rs:50` |
| `NodeIdLockMetrics` | struct | u64 计数器：attempted / succeeded / skipped(alive) / skipped(unverified) / force_reclaimed | `client-sdk/src/model/mod.rs:1` |

**两阶段 reclaim（状态机）：** `Unacquired → Locked`（`fs::create_new` 原子获取）→ `Reclaimed`（pre/post 策略检查 + 原子 rename）→ `Removed`（post 通过）或 `Restored`（post 失败回滚）。**复合见证（composite witness）= host-fingerprint + pid + start-marker** 三维证明，防止误回收一个合法重启的进程。`ProcessLiveness::Unknown` 默认 ⇒ 不回收，除非 `force` + host 匹配。

### 2.3 Invocation / Admission / Receipt 对象模型（axiom）

这是平台的密码学心脏，落在 `sdk/rust/src/invocation/`。

#### 2.3.1 Envelope 与签名（axiom.rs）

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `InvocationEnvelope` | struct | axiom 7 元组线缆形：caller / callee / subject / ability / args_digest / invocation_nonce / causal_context；经 `from_wire_parts()` 重组 | `axiom.rs:402` |
| `DescriptorBoundEnvelope` | struct（typestate） | 类型收紧的 envelope：subject 派生为 `EntityRef`，ability 校验为 `ability_ura@version`；构造时 `validate()` | `axiom.rs:478` |
| `CallerSignature` | struct | caller 侧对规范字节的签名（RFC-001 §4.1）：algorithm / signature / key_id_hint | `axiom.rs:383` |
| `CalleeSignature` | struct | callee 侧对回执规范字节的签名（§4.3）；**不被 self_hash 覆盖**（签名不能覆盖自己） | `axiom.rs:391` |
| `CausalContext` | enum | 因果谱系（§3.3）：None（root）/ Scalar（单父）/ List（fan-in）/ Merkle（大 fan-in，hash） | `axiom.rs:245` |
| `AgentIdentity` | struct | agent 复合身份：URA + profile（EasynetStrictV2 / WebSafeV2） | `axiom.rs:60` |
| `SubjectIdentity` | struct | subject 复合身份；与 `AgentIdentity` **刻意区分类型**以强制 INV-7（subject 可与 callee 不同域） | `axiom.rs:78` |
| `EntityRef` | struct | 从 `SubjectIdentity` 派生的规范化证明形，带 `EntityRefKind` 标签 | `axiom.rs:135` |
| `AuthorityBinding` | enum | "谁有权"：Self_ / Delegated / Capability / Policy / Session / Bootstrap；哈希进受签区（不可伪造） | `axiom.rs:272` |
| `ReceiptProofFacts` | struct | descriptor/version/schema/impl/runtime 证明事实；零哈希是受签的"not bound"事实 | `axiom.rs:343` |
| `UraProfile` | enum | Axis B：EasynetStrictV2（RFC-001 默认）/ WebSafeV2（HTTP 工具用） | `axiom.rs:31` |

**关键设计：零哈希是事实，不是缺失。** `schema_hash` / `impl_hash` 为零时，是**受签的"未绑定"事实**，不是被剥离的缺失字段。验证方必须用 `.schema_bound()` / `.impl_bound()` 谓词判断，**不能直接零值检查**。这条贯穿整个 receipt 验证逻辑。

#### 2.3.2 Receipt 与 AxiomBinding（audit.rs）

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `InvocationReceipt` | struct | 哈希链审计的一条记录：index / invocation_id / receipt_type / state / timestamp / prev_receipt_hash / self_hash / payload / axiom_binding / usage / proof_facts | `audit.rs:140` |
| `AxiomBinding` | struct | §3.6 每条回执必带的一等绑定：caller / callee / subject / nonce / causal / payload_digest / callee_signature / signer_binding（§A12 hosted agent）/ host_attestation / ability_binding / authority_binding | `audit.rs:79` |

**Hosted Agent §A12：** `signer_binding`（`Option<AgentIdentity>`）+ `host_attestation`（`Vec<u8>`）编码 self-signed vs hosted 之分。校验强制：`Self_` ⇒ signer == callee；`Hosted` ⇒ signer == host 且 attestation 非空。

#### 2.3.3 Admission（admission.rs）

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `NonceReplayStore` | struct | §5.2 滑动窗口重放检测；128 桶时间轮，O(1) 摊还 check-and-record；去重键 `(caller.ura, ability, invocation_nonce)` | `admission.rs:404` |
| `KeyResolver` | trait | 抽象公钥解析，供 Federated/Trusted admission；`FileKeyResolver` 实现；Local-fast 模式跳过 | `axiom.rs`（~1250+） |

**三阶段 admission split（性能关键）：** （1）无状态 `verify_phase`（envelope + 结构检查 + 可选 crypto verify）**跑在 replay-store 锁之外**；（2）`run_admission` 有序组合：`validate_envelope → validate_signature_structure → verify_signature（仅 Federated/Trusted）→ check_and_record`；（3）畸形 envelope/签名在**触碰 replay store 之前**就被拒。Ed25519 verify（~20µs）跑在 nonce mutex 之外，单核 verify 速率不会拖死整个 fleet 吞吐（T3.1 锁拓扑决策）。

#### 2.3.4 InvocationState / Core / Handle（handle.rs）

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `InvocationState` | enum `#[repr(i32)]` | 9 态生命周期：Unspecified(0)→Accepted(1)→Admitted(2)→Dispatched(3)→Running(4)→{Completed(5)\|Failed(6)\|TimedOut(7)\|Cancelled(8)} | `handle.rs:27` |
| `InvocationCore` | struct | 单次 invocation 的共享核心；`Arc<InvocationCore>` 被 Handle + AbilityContext + stream 订阅者共享；receipts `Vec`（Mutex 内）是**唯一规范日志** | `handle.rs:282` |
| `InvocationHandle` | struct | caller 侧句柄；状态查询 / 事件流 / 取消；包 `Arc<InvocationCore>` | `handle.rs`（varies） |
| `EmitExtras` | struct | `emit_with()` 的每次 emit 元数据：event_type / payload / content_type / reason / cleanup_complete / child_invocation_id / usage | `handle.rs:256` |
| `BackpressurePolicy` | enum | Unbounded（仅从 receipts 重放）/ Block(capacity, max_wait) | `backpressure.rs` |

**F-012 类型安全（已验证磁盘）：** `InvocationState` `#[repr(i32)]` 钉死线缆布局。`TryFrom<i32>` / `TryFrom<&str>` 拒绝越界值——**非法状态字符串在 parse 处失败，永不进入**。`as_str()` ↔ `from_str` 是精确互逆（已在 `handle.rs:38-52` 验证）。事件视图是从 receipts 链按需投影的 projection，**没有并行事件日志**（省内存）。

#### 2.3.5 两个 AxonError —— 一个 crate 内的陷阱

**这是 Axon 错误模型最重要的一条事实，新工程师必读。** 一个 crate 里住着两个都叫 `AxonError` 的类型，服务于不同边界：

| 名 | 种类 | 边界 | 文件:行 |
|---|---|---|---|
| `invocation::error::AxonError` | **struct** | 线缆/回执分类法（7 类模型，serde-transparent，进受签回执 payload） | `sdk/rust/src/invocation/error.rs:239`（已验证磁盘） |
| crate-root `AxonError` | **enum**（`thiserror`） | SDK-facade，按生命周期阶段排序，re-export 为 `easynet_axon::AxonError` | `sdk/rust/src/error.rs:47`（已验证磁盘） |

- struct 形（receipt 分类法）：`AxonErrorKind`（7 个 gRPC 对齐类：Cancelled / DeadlineExceeded / Unavailable / InvalidArgument / ResourceExhausted / PermissionDenied / Internal）× `ErrorCode`（~48 个 SCREAMING_SNAKE 语义码，如 `CALLER_NONCE_REPLAYED`、`AUTHORITY_SCOPE_VIOLATION`、`ABILITY_FORBIDDEN`）× `ErrorStage`（9 个流水线位置）× `SecurityClass`（8 个域）。**reason 字符串（`REASON_NONCE_REPLAY`）是稳定 API**，audit/metrics 流水线 grep 它——**永不重命名**。未知 proto code 映射到 `Internal` + `"unknown_error_code"`（静默透传 = 一致性违规，见文件头）。
- enum 形（CLI 看见的）：`Validation` / `SymbolNotFound` / `Bridge` / `DeadlineExceeded` / `NotInstalled` / `NotActivated` / `Invocation` / `Stream` / `PolicyDenied` / `PartialSuccess` / `Json` / `Io` / `Mcp`，按 Validation → Bridge → lifecycle → execution → transport → batch 阶段排序。

记住：**CLI 内部转换的是 enum 形，不是 struct 形。** 详见第 7、8 章。

---

## 3. 运行时层

本章描述两个运行时对象模型：Axon server 端 invocation kernel（`EasyNet-Axon/core/runtime-rs`），以及 CLI 的能力控制面（三注册表）。

### 3.1 Axon Runtime server 对象模型

整个 server 是一个 invocation-centric kernel：每次能力执行、每个 mission step、每次 voice 交互**都走 Invoke RPC**（RFC-001 P1.2）。这把协议面压平成一个 `InvocationService` + 两个观测服务（`Namespace.Resolve`、`PayloadTransfer`）。

#### 3.1.1 顶层句柄与状态

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `AxonRuntime` | struct | 轻量可 clone 句柄，包 `Arc<AxonState>`；gRPC service impl 入口 | `src/runtime/mod.rs:120-124` |
| `AxonState` | struct | 进程生命周期内存权威存储：nodes / capabilities / invocations / missions / identity / federation / streaming / voice / policy / admission / audit | `src/state/mod.rs:623-723` |
| `RuntimeConfig` | struct | 进程配置：role / wal_enabled / max_concurrent_invokes / idempotency_index_capacity / federation / audit | `src/state/config.rs` |
| `RuntimeRole` | enum | Full / Hub / Member / Orchestrator | `src/state/config.rs` |

#### 3.1.2 域状态子结构（`AxonState` 的字段，按主干切分）

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `NodeTopologyState` | nodes/installs DashMap + 二级索引（install_index_by_node_cap / by_tenant_fn）+ circuits / rate_limits | `src/state/mod.rs:397-418` |
| `ExecutionState` | invocations DashMap + request_to_invocation 索引 + `invoke_slots` 信号量 + terminal_notify | `src/state/mod.rs:505-515` |
| `IdempotencyState` | scoped 索引 + inflight + fingerprints + 有界淘汰 + 全局/per-tenant 指标 | `src/state/mod.rs:363-378` |
| `AdmissionState` | `NonceReplayStore` + 可选 `KeyResolver` override + `CalleeSigner` + proof_bindings | `src/state/mod.rs:552-570` |
| `CalleeSigner` | server 回执签名身份：`AgentIdentity` + Ed25519 `SigningKey`（**ZeroizeOnDrop**） | `src/state/mod.rs:578-592` |
| `IdentityState` | tokens / tenants / node_keys / bootstrap_node_guards / session_keys | `src/state/mod.rs:531-542` |
| `PolicyGovernanceState` | policies / decisions / overrides / eval_cache / per-tenant 指标 | `src/state/mod.rs:380-393` |
| `FederationState` | member 记录 / federated nodes / cross-runtime invocations / 目录事件 / shard 路由 | `src/state/mod.rs:270-356` |
| `StreamingState` | streams + payloads DashMap | `src/state/mod.rs:595-598` |
| `EventChannels` | 进程级 broadcast sink：membership / capability / invocation / mission / events / stream / voice / state_changes | `src/state/mod.rs:600-621` |
| `OwnerProjectionStore` | RFC-005 A5/A7 已接受能力投影；owner-keyed + per-owner 单调 revision fence | `src/state/owner_projection.rs` |
| `BoundedEvictionQueue<K>` | LRU 淘汰追踪（`VecDeque` + `Mutex`）；O(1) push / O(n) remove | `src/state/mod.rs:59-106` |

#### 3.1.3 账本记录类型

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `InvocationRecord` | 执行账本：lifecycle status / 调度理由 / timing / payload / receipts | `src/state/invocation.rs:30-54` |
| `MissionRecord` | 多步工作流编排：step status / timeline / admission telemetry / deadline | `src/state/mission.rs:110-143` |
| `AgentRecord` | RFC-001 §1.2 RealmDirectory 行：canonical URA / 公钥 / 签名权 / status / resolver abilities | `src/state/agents.rs:107-146` |
| `AgentsCatalog` | 权威 realm 成员索引：primary by_ura + 二级 by_tenant / by_host_node | `src/state/agents.rs:213-222` |
| `FederationMemberRecord` | hub 侧成员运行时身份 | `src/runtime/federation.rs:36-58` |
| `FederatedInvocationRecord` | 跨 shard invocation 追踪 | `src/runtime/federation.rs:121-149` |

#### 3.1.4 Session kernel（AXON-RFC-002）—— kernel↔backend 接缝

这是 server 端最干净的 trait 接缝，把 kernel（`BidiStreamHandle` 生命周期、HMAC 链）与资源后端（PTY / LLM / MCP）解耦。

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `SessionProvider` | **trait**（object-safe，Send+Sync+'static） | `kind()` / `create(session_id, args, content_type)` / `attach(session_id, handle)` | `src/services/invocation/session_provider.rs:137-203` |
| `BidiStreamHandle` | struct（**!Clone**） | provider-facing bidi 管道：up/down chunk / control / shutdown token / failure_reason | `session_provider.rs:447-492` |
| `BidiStreamWriter` | struct（**可 clone**） | split 后的写侧，供 down 方向 fan-out | `session_provider.rs:700-705` |
| `BidiStreamReader` | struct（**单 owner**） | split 后的读侧 | `session_provider.rs:754-759` |
| `SessionRegistry` | struct | 进程级 session kernel；构造时 seal | `src/services/invocation/session_registry.rs` |

`SessionProvider` **object-safe 是有意为之**（无泛型、无关联类型），这样 `Arc<dyn SessionProvider>` 异构注册表才成立。`BidiStreamHandle::split()` → `(可 clone 的 Writer, 单 owner 的 Reader)` 是 fan-out 模式的核心。

#### 3.1.5 终态化（terminal）

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `TerminalOutcome` | 核心终态化输入：identity / routing / timing / state / result / error / audit detail | `src/services/invocation/terminal.rs:17-40` |
| `InvokeTerminal` | 完整 gRPC response 构造器：outcome / header / backpressure / circuit / rate_limit / scheduling_score / policy_decision_id / receipt_context | `src/services/invocation/terminal.rs:53-62` |

#### 3.1.6 Server 端 trait 接缝总览

| trait | 角色 | 文件 |
|---|---|---|
| `TimeProvider` | `now_unix_ms()`；`SystemTimeProvider` 实现，可注入测试 | `src/runtime/mod.rs:107-117` |
| `SessionProvider` | kernel/backend split | `session_provider.rs:137-203` |
| `ShardResolver` | 抽象 shard 成员 + 路由 | `src/services/invocation/shard_resolver.rs` |
| `InvocationRelay` | 只转发完整 `InvokeRequest` 的跨 shard 接缝 | `src/services/invocation/invocation_relay.rs` |
| `NamespaceRouteResolver` | RFC-005 §6 解析算法 | `src/services/namespace/resolver.rs` |
| `NamespaceDirectory` | fact-store 边界 + v1 适配 | `src/services/namespace/directory.rs` |
| `NamespaceDiscoveryResolver` | RFC-005 discovery 解析 | `src/services/namespace/discovery.rs` |
| `NamespacePolicyGate` | 可行性 + 可注入策略门（`AllowAllPolicyGate` 默认） | `src/services/namespace/gates.rs` |
| `AliasStore` | §4.3 loop-safe rewrite store | `src/services/namespace/alias.rs` |

**最干净的跨仓 trait 复用：** `AxonRuntime::run_admission_gate`（`state/mod.rs:557` 附近）是个薄适配器，把进程本地的 `RealmDirectory` 当作 `KeyResolver` 插进 `easynet_run_axon_client::admission::run_admission`——**admission 逻辑单源于 Axon，server 和任何 client 共享同一原语。**

### 3.2 CLI 能力控制面 —— 三注册表模型

CLI 持有比 Axon 更丰富的控制面元数据，按三个关注点切成三张表。落在 `src/daemon/ability/`。

#### 3.2.1 三张表 + 统一键

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `AbilityDescriptor` | aggregate | 第一表：完整受治理接口、权限、schema、transport 与 receipt semantics 的唯一聚合 | `src/daemon/ability/descriptors/surface.rs` |
| `AbilityDescriptorRegistry` | struct | 第一表存储；`BTreeMap` 确定性迭代 | `src/daemon/ability/descriptors/mod.rs` |
| `AuthorityBinding` | struct | 第二表：治理谓词（governs_advertise / governs_invoke）+ policy binding + scope | `src/daemon/ability/authority/mod.rs` |
| `AuthorityBindingRegistry` | struct | 第二表存储 | `authority.rs:597-619` |
| `AbilityImplBinding` | struct | 第三表：可执行事实（runtime_env / impl_source / impl_hash / content_hash） | `src/daemon/ability/impl_binding.rs:98-219` |
| `AbilityImplRegistry` | struct | 第三表存储 | `impl_binding.rs:221-243` |
| `AbilityControlPlaneKey` | struct（**复合 newtype**） | (authority_root, ability, descriptor_version, call_mode) —— **统一三表的唯一规范键** | `descriptor.rs:130-217` |
| `AbilityControlPlaneRecord` | struct | (key, descriptor, authority, impl) 原子聚合，dispatch 层作为整体读 | `src/daemon/ability/control_plane.rs` |
| `AbilityControlPlaneRegistry` | struct | 聚合 facade，持三表；注册经物化状态机写入，查询统一聚合 | `src/daemon/ability/control_plane.rs` |

**不变量（已验证磁盘）：** 每个键要么在三表全在，要么全不在（`assert_record_keys_match` 强制）。这是三注册表模型的灵魂。

#### 3.2.2 newtype 家族

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `AbilityDescriptorVersion` | newtype(String)，构造时校验语法（如 `1.0.0`） | `descriptor.rs:20-54` |
| `SchemaHash` | newtype([u8;32])，governed schema 摘要 SHA256 | `descriptor.rs:252-263` |
| `DescriptorHash` | newtype([u8;32])，绑定 (ability_ura, name, version, call_mode, schema_hash) | `descriptor.rs:265-276` |
| `CallMode` | enum：Rpc / Stream / Bidi；`.axon_call_mode()` 投影到 Axon `AbilityCallMode` | `descriptor.rs:226-250` |
| `RuntimeEnv` | struct：执行环境不透明标签（如 `easynet-cli/0.23.1;rust-native`），绑入 impl_hash | `impl_binding.rs:19-68` |
| `AbilityImplSource` | enum：NativeDaemon / BuiltinPlugin / SidecarPlugin / DeclarativePlugin / DeviceDeploy / Eal / Mcp / Test | `impl_binding.rs:70-96` |
| `OwnerKind` | enum：Device / Hub / Agent(id) / User(id)；**RFC-001 结构性根治**（M0 前 owner 从名字前缀推断，M0 起注册时声明） | `src/daemon/ability/dispatch.rs:974`（已验证磁盘） |
| `AbilityControlPlaneError` | enum（45+ 变体），所有公共构造器无 panic，边界一律返回 Result | `src/daemon/ability/error.rs:8-100` |

#### 3.2.3 三阶段提交 typestate（已验证磁盘 `registry.rs:138`）

```rust
enum AbilityControlPlaneRegistrationStage { Planned, Materialized, Committed }
```

`Planned --materialize()--> Materialized --commit()--> Committed`。物化阶段构造三行 + 校验键一致性；commit 原子写三表。**回滚机制：** mutate 前抓 `records_for_authority_mode()` 快照，失败时 `restore_authority_mode_records()` 恢复。`debug_assert_eq!` 守护状态合法性，测试期捕获误用。

**哈希始终是 Axon 权威：** `AbilityDescriptor` 的 schema/descriptor hash 方法调用 Axon `axiom::ability_schema_hash()` / `ability_descriptor_hash()`；CLI 不维护第二个 hash record。

---

## 4. 能力层

本章描述每个能力实现的共享抽象，以及 chat / teach / think / discover / mcp 等具体能力。当前实现落在 `src/daemon/ability/builtins/`、`src/daemon/ability/catalog/`、`src/runtime/executors/` 与 `src/daemon/ability/dispatch.rs`；旧 `runtime::agents` 兼容 facade 已退休。

### 4.1 中央派发枢纽

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `AxonAbilityCatalog` | struct | 所有已注册能力的中央注册表 & 派发枢纽；桥接 CLI daemon 与 Axon `LocalRuntime`；六张同构 handler map（RPC/Stream/Bidi × with/without envelope） | `src/daemon/ability/dispatch.rs:1247` |
| `DynamicCatalogue` | struct | post-boot 热重载侧表，与静态 map 分离；`RwLock` 守护；查询先静态后动态 | `ability_dispatch.rs:1337` |
| `EnvelopeContext` | struct | AXIOM 7 元组到产品 handler 的投影；构造时校验所有字段，**无半初始化态** | `ability_dispatch.rs:91`（亦见 `90-277`） |
| `AbilityAuthorityContext` | struct | 进程本地权威根：device / hub authority_root + source | `ability_dispatch.rs:1035` |

### 4.2 共享抽象 —— handler newtype 家族

每个能力最终都被表达成一个 handler 闭包，裹在一个 `Arc<dyn Fn>` 类型别名里。**这就是"每个能力实现的共享抽象"**——不是一个大 `Ability` trait，而是六种同构 handler 形状（已验证磁盘 `ability_dispatch.rs:60`）：

| 别名 | 形状 | 用途 | 文件:行 |
|---|---|---|---|
| `LocalRpcHandler` | `Arc<dyn Fn(Value) -> anyhow::Result<Value>>` | 一元 RPC（spawn_blocking 内执行） | `ability_dispatch.rs:60` |
| `LocalRpcHandlerWithEnvelope` | `Arc<dyn Fn(EnvelopeContext, Value) -> Result>` | 收 AXIOM 7 元组的 RPC（媒体 / subject-aware） | `ability_dispatch.rs:376` |
| `LocalStreamHandler` | `Arc<dyn Fn(Value) -> Result<StreamSource>>` | server-stream | `ability_dispatch.rs:445` |
| `LocalBidiHandler` | `Arc<dyn Fn(Value) -> Result<BidiSource>>` | bidi，handler 自己 spawn 拥有 session 生命周期的循环 | `ability_dispatch.rs:544` |

配套返回类型：

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `StreamSource` | enum | Snapshot(Vec) / Live(broadcast::Rx) / SnapshotThenLive；handler 经 `From<T>` 转换 | `ability_dispatch.rs:403` |
| `BidiSource` | struct | 一个开放 bidi session 的两端：to_client(`mpsc::Sender`) / from_client(`mpsc::Receiver`) | `ability_dispatch.rs:497` |

**安全并发由构造保证：** handler 闭包**按值捕获**（`String` / `Arc` clone），**绝不 `Rc<RefCell>`**。唯一的共享可变性走类型化内部可变层（DashMap/RwLock/atomics），永不在 handler 内部。

### 4.3 各能力实现的 trait 接缝

能力层的 trait **只用于扩展接缝与测试 seam**，不用于派发本身：

| trait | 角色 | 实现 | 文件:行 |
|---|---|---|---|
| `ContextLoader` | 可插拔 chat 上下文贡献者：`name()` + `load(agent, session) -> Option<String>` | UserProfileLoader / ScheduleLoader / MemoryLoader | `src/daemon/ability/builtins/agents/chat.rs:132` |
| `DiscoverFederationResolver` | discover ladder 与 realm directory 之间的依赖边界 | Bridge / Deferred / LocalDirectory 三实现 | `src/daemon/ability/builtins/agents/discover.rs:110` |
| `TeachClock` | teach grant 事务的确定性时间 seam | —— | `src/daemon/ability/builtins/governance/teach.rs:59` |
| `DeviceOpsClock` | device op 事务的 boot-timestamp seam | —— | `src/daemon/ability/builtins/device_control/ability_management/ops.rs:72` |
| `AcquiringArtifactTxn` | 两阶段 descriptor 暂存 commit/rollback | manifest provisioning | `src/persistence/teach_grants.rs:266` |

**ContextLoader 是正确的扩展点：** 未来 loader（memory、project folders）实现它即可，无需改 `chat_ability.rs`。Registry 把 `Arc<Vec<Arc<dyn ContextLoader>>>` 传进每个 chat handler。

### 4.4 manifest-first 派发哲学

若能力的 manifest 有 `[exec]` 绑定 → 直接跑执行器（shell / http / eal / mcp），**无 LLM 往返**。只有 descriptor-only（无 `[exec]`）能力才路由进 chat。这条让"能力"既可以是确定性可执行单元，也可以是 LLM 中介行为，统一在同一派发面。

### 4.5 注册构建与热注册

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `RegistryBuildConfig` | 传给 fallible `build_registry_with_services_result()` 的不可变配置 | `src/daemon/ability/catalog/build.rs` |
| `BuiltAbilityRegistry` | 构建输出：(catalog Arc, plugin_runtime_manager Arc, device_registrar_cell OnceLock) | `src/daemon/ability/catalog/build.rs:155` |
| `HotAgentRegistrar` | post-boot 把 hosted-agent handler 集物化进 LocalRuntime + catalog | `src/daemon/axon_bridge/hot_agent_registrar.rs:160` |

**late-binding 解决 bootstrap 鸡生蛋：** `local_registry_handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>>`——handler 闭合在 `OnceLock` 而非 `Arc` 本身，使 handler 注册可以先于 catalog 被 `Arc::new` 包裹。`HotAgentRegistrar` 是 phase-5c"内存里注册 agent"与 phase-6"持久化 descriptor 元数据"之间的桥；把它放在 `axon_bridge/` 强调 **LocalRuntime 才是可用性的真理源，AxonAbilityCatalog 是元数据 + 派发**。

---

## 5. 传输层 —— dispatcher / session 对象模型（诚实记录其今日状态）

> **本章诚实声明：** 传输层是 pre-refactor 状态。生命周期逻辑在三种几何形态（unary / stream / bidi）间**重复**；`InvocationLifecycle` sink 收敛重构**仍在飞行中**。本章描述今日代码，不是终局。这正是第 9 章"坏味道"的最大一条。

落在 `src/daemon/invocation/`。

### 5.1 根服务与依赖平面

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `DaemonInvocationService` | struct | 根 tonic gRPC service handler；三个 RPC 面（Invoke/Stream/Bidi）的单一所有权入口；持六个 builder 注入的依赖平面 | `daemon_invocation_service.rs:187-206` |
| `DirectoryPlane` | struct | 只读 presence / agents / abilities / federated 目录快照 | `deps.rs:47-71` |
| `FederationDial` | struct | 跨 realm federation client + peer map cell + hub 签名种子 | `deps.rs:76-94` |
| `SessionPlane` | struct | device↔hub 关联 map（PendingDispatchMap / PendingStreamDispatchMap）+ escalation 句柄 | `deps.rs:99-117` |
| `IdentityPlane` | struct | register-pubkey handler context + daemon realm | `deps.rs:131-142` |
| `RuntimePlane` | struct | LocalRuntime（唯一进程内执行面）+ invocation ledger + ability wire registry | `deps.rs:147-160` |

**依赖平面模式（设计优点）：** 每个平面聚一个域；`DaemonInvocationService` 持全部六个（Arc 包）；dispatcher **提取所需子集**而非接整个 service。这实现了 per-RPC 粒度的细粒度依赖注入。

### 5.2 三个几何派发器（重复的根源）

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `UnaryDispatcher` | 按 function_name 路由 Invoke；持 federation wrappers / namespace.resolve / identity abilities / LocalRuntime arm | `unary_dispatcher.rs:186-194` |
| `StreamDispatcher` | 路由 InvokeStream；持 federation.subscribe_directory v1/v2 + LocalRuntime stream arm | `stream_dispatcher.rs:44-48` |
| `BidiDispatcher` | 路由 InvokeBidi；持 `runtime.invoke_remote` / `session.open` / LocalRuntime bidi arm | `bidi_dispatcher.rs:92-100` |
| `TargetGate` | 共享 resolve-first RFC-005 路由门；持 route_resolver / admission / planes；dispatch 前守 locality | `target_gate.rs:39-46` |

### 5.3 线缆类型与路由结果

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `ProtoEnvelope` | struct | proto Envelope 的类型化包装；构造 builder（caller_only / loopback / targeted）；强制 URA 语法 / nonce / 签名生成 | `invocation_wire.rs:62-229` |
| `SelectedInvokeRoute` | struct | namespace.resolve 的结果：callee_ura / execution_host_ura / dispatch_name / ability_ura / route profile | `route_resolver.rs:137-242` |
| `DelegatedInvokeRoute` | struct | 跨 realm 委派答案：peer hubs / NextHop / realm | `route_resolver.rs:244-317` |
| `DaemonRouteResolver` | struct | per-call namespace.resolve 引擎 | `route_resolver.rs:371-479` |
| `SessionDispatch` | enum | `session.open` 帧线缆封套：Dispatch / Result / BidiInput | `invoke_remote_initiator.rs:251-347` |
| `InvokeRemoteUp` / `InvokeRemoteDown` | enum | device 侧请求 / hub 侧 down 形 | `invoke_remote_initiator.rs:165-218` |
| `SessionRequestError` | enum | `session.open` 结构化错误：UnreachableRoute / NoCapacity / Timeout | `invoke_remote_initiator.rs:367-426` |

### 5.4 传输层 trait

| trait | 角色 | 文件:行 |
|---|---|---|
| `Invocation`（tonic） | 来自 easynet-axon 的 gRPC service trait；三方法；`DaemonInvocationService` 实现 | `daemon_invocation_service.rs:709-919` |
| `LocalRuntimeAuthority` | 本地运行时能力元数据查询接口 | `route_resolver.rs:59-135` |
| `SessionFrameDispatcher` | `session.open` 入站帧的可插拔 handler | `session_initiator.rs:292-322` |
| `FederationClient` | 跨 hub dial 接口（外部 trait，gRPC shim 实现于 `federation_client.rs`） | —— |

### 5.5 传输层状态机（生命周期逻辑分散的实证）

- **Session Liveness（spec §3）：** Absent → Online → Offline。**channel 成员资格 = liveness，无 heartbeat**（bidi stream 本身就是心跳）。displacement：新 device 认领同一 URA 时驱逐旧 sender，发 Offline(prior)+Online(new)。`bidi_dispatcher.rs:1279-1331`。
- **Dispatch Contract Negotiation（DEC-F004 v0→v1）：** Legacy_v0(JSON) ↔ v1_Carrier(proto)；frame-0 协商 `min(device, HUB=1)`。`bidi_dispatcher.rs:1419-1429`，`HUB_DISPATCH_CONTRACT_VERSION=1`。
- **Remote Bidi Session：** Open → Active → Closed。`local_session_dispatcher.rs:67`。
- **Local Bidi Frame Mapping：** BinaryChunk→stdin / Control(Eof)→EOF / Control(PtyResize)→resize。`bidi_dispatcher.rs:626-835`。
- **Escalation：** Local（presence 在）vs Escalation（无本地 presence，经 `session.open` forward 到 hub）。`daemon_invocation_service.rs:526-534`。

**这五个状态机分散在三个几何派发器里、各自实现自己的生命周期——这就是"lifecycle logic duplicated across geometries"的字面证据。** 终局是把它们收敛进一个 `InvocationLifecycle` sink（见第 9 章）。

### 5.6 异步通道在传输层的角色

- `mpsc::channel<Result<DispatchFrame, Status>>`（capacity=256）：hub→device。`try_send` disposition 编码 liveness：`Full`=慢设备（重试）、`Closed`=离线（移除）。
- `oneshot`：unary await-reply 跨 bidi session（`PRESENCE_DISPATCH_REPLY_TIMEOUT` 60s）；presence-offline watcher 在驱逐时令等待中的 oneshot 失败。
- `Weak<Arc<PresenceRegistry>>`：broadcast pump（`stream_dispatcher.rs:90`）不延长 registry 生命周期，daemon 关闭时优雅终止。

---

## 6. 编排与语言层 —— EAL 编译器 + CLI facade

本章把 EAL 编译器当作一本教科书级编译器 OOP 设计来描述：lexer → parser → AST → planner → IR → interpreter。落在 `src/eal/`。

### 6.1 编译器流水线（教科书五阶段）

```
源码 → Lexer → Vec<Token> → Parser → EalProgram(AST)
     → Planner（合并 analyzer+compiler）→ MissionIr
     → Interpreter（经 StepDispatcher trait 对象执行）→ ExecutionReport
```

每个阶段向前转换 owned/borrowed 类型，无循环依赖。

### 6.2 AST 层

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `EalProgram` | struct | 解析后根节点，包 `MissionDecl` | `src/eal/ast.rs:21` |
| `MissionDecl` | struct | 顶层 mission：name + 有序 statement 向量 | `ast.rs:27` |
| `Statement` | enum | LetCall / Call / Loop | `ast.rs:33` |
| `CallExpr` | struct | 单次能力调用：function_name / target_node / **target_kind** / arguments / options | `ast.rs:77` |
| `TargetKind` | enum（**typestate**） | 区分 member-call(Agent) vs 传统 call(Device)；parse 时设定，运行时**绝不重分类** | `ast.rs:90` |
| `LoopBlock` | struct | RFC-010 块语句：name / max_iters[1..32] / body / verify | `ast.rs:58` |
| `FieldValue` | enum | 参数值：String / Int / Float / Bool / VarRef / Object（VarRef 捕获数据依赖如 `photo.output`） | `ast.rs:110` |
| `FailurePolicy` | enum | Abort / Skip / Retry / Continue（默认） | `ast.rs:157` |
| `Token` | enum | lexer 输出 | `src/eal/lexer.rs:28` |
| `Lexer` | struct | 上下文无关 tokenizer | `lexer.rs:68` |

**本体编码进类型（关键设计）：** `TargetKind` + `IrTarget` 一起**阻止隐式 agent 回退**（docs/AGENT_IDENTITY.md 不变量 2：无 `is_agent` 字符串检查）。member-call → Agent，传统 call → Device，在 parse 时定，IR 直接 lower，**运行时按 enum 变体匹配，永不字符串重分类**。

### 6.3 IR 层

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `IrTarget` | enum | 派发目标：Agent(AgentId) / Device(node_id) | `src/eal/ir.rs:53` |
| `IrCall` | struct | 扁平 IR 调用：step_id / ability / IrTarget / static_arguments / input_refs / output_binding | `ir.rs:85` |
| `IrStep` | enum（`#[serde(untagged)]`） | Loop(块) / Call(扁平)；**untagged 保留 pre-PR-10 Call-only 线缆形** | `ir.rs:141` |
| `IrLoop` | struct | RFC-010 loop IR：kind / name / max_iters / body / verify / result_binding | `ir.rs:208` |
| `MissionIr` | struct | 可序列化 IR：name / steps / phases / constraints | `ir.rs:296` |
| `PhaseRange` | struct | phase 边界 `[start, end)`；phase 间顺序、phase 内独立步并行 | `ir.rs:305` |
| `IrConstraints` | struct | max_parallel / deadline_seconds / `default_max_calls()`=256（RFC §4.1 planner 期界） | `ir.rs:311` |
| `IrLoopTag` | newtype | `#[serde(untagged)]` 的单例判别子，序列化成字面 `"loop"`——**无 tag 字段即可结构性消歧** | `ir.rs`（IrLoop 内） |

### 6.4 Interpreter 层

| 类型 | 种类 | 角色 | 文件:行 |
|---|---|---|---|
| `StepDispatcher` | **trait** | 步执行抽象层：`dispatch()` 收 IrTarget+ability+args+timeout+causal_parents，返回 `StepDispatchOutcome`；`clone_for_thread()` 支持并行 | `src/eal/interpreter/mod.rs:103` |
| `StepDispatchOutcome` | struct | 派发结果：value(JSON) + invocation(可选 7 元组回执) | `interpreter/mod.rs:142` |
| `AgentAwareDispatcher` | struct | 生产实现，持 `Arc<AgentRegistry>`；按 IrTarget 变体路由 | `src/eal/interpreter/dispatch.rs:123` |
| `RunContext` | struct | per-mission 执行上下文：tenant / trace_id（= mission run id，盖在每个 Axon envelope 上） | `interpreter/mod.rs:98` |
| `PhaseRunState` | struct | 可变 phase 执行上下文，经 `&mut` 在 phase walk 中借出 | `interpreter/phases.rs:46` |
| `PhasePartition` | enum | Calls(连续运行) / Loop(块)；`split_phase_steps()` 分区并保源序 | `phases.rs:83` |
| `ExecutionTrace` | struct | mission 执行审计日志：mission_id / steps(head/tail capping) / outcome / receipt graph / ability graph | `interpreter/trace.rs:148` |
| `CappedTraceBuffer` | struct | 有界 trace buffer：保 head(前 500) + tail(后 500)，丢中间并计数 | `trace.rs:94` |
| `EalError` | enum | StepDispatcher 结构化错误（见 6.5） | `src/eal/error.rs:53` |

**并行语义：** phase 间顺序，phase 内独立步经 `rayon::scope` work-stealing 并行（若 dispatcher 可 clone）。每个 worker 拿自己的 `dispatcher.clone_for_thread()`，结果经 `crossbeam::SegQueue`（lock-free）收集。`DispatchContext`（mission trace_id）在 `rayon::scope` 入口捕获一次、每 worker 重装（F-028，防并发 mission 身份冲突）。

### 6.5 EalError —— 与 MCP 共享词汇但独立演进（已验证磁盘）

```rust
pub enum EalError { Validation, NotFound, Unavailable, DeadlineExceeded, Internal }  // src/eal/error.rs:53
```

`error_code()`（`error.rs:77`，已验证磁盘）返回稳定机读串：`"validation"` / `"not_found"` / `"unavailable"` / `"deadline_exceeded"` / `"internal"`，**刻意镜像 `McpError`**，使 MCP 线缆契约与内部解释器错误共享词汇却独立演进。`error_codes_are_stable_and_unique` 测试守护。

**转换接缝（load-bearing，已验证磁盘 `error.rs:159`）：** `EalError::from_axon_error(easynet_axon::AxonError)` 是从 SDK-facade enum 到解释器分类法的**已审、非 lossy** 映射：

```
A::Validation | A::PolicyDenied        → Validation
A::NotInstalled | A::NotActivated      → NotFound
A::DeadlineExceeded                    → DeadlineExceeded
A::Bridge | A::Stream | A::Invocation | A::Mcp | A::Io → Unavailable
A::SymbolNotFound | A::Json | A::PartialSuccess → Internal
```

**团队刻意不实现 `From<String> for EalError`**（`error.rs:204+` 注释，已验证磁盘）。早期草案有 `impl From<String>{ Unavailable(s) }`，让每个未注解 `?` 静默归类为 `Unavailable`——而约一半真实错误并非瞬态。这条注释原文称之为"a *deception* at pre-release scale"。纪律：**每个跨错误边界都是显式、已审的 match，绝无 stringly default。**

### 6.6 Mission 运行持久化

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `MissionRunStore` | mission-run 持久化 facade，锚定 `~/.easynet/missions/runs/` | `src/cli/mission_runs.rs:51` |
| `MissionRunDir` | 打开的运行目录句柄；持 `PathBuf` + 可选 `HeartbeatPump`；`Drop` 停心跳 | `mission_runs.rs:142` |

**F-022 liveness：** 用 heartbeat 文件 mtime 新鲜度替代 pid-file 存在性（< 15s = 存活）。`MissionRunStatus`：Running → {Completed | Failed | Aborted}。

### 6.7 CLI facade（薄）

| 类型 | 角色 | 文件:行 |
|---|---|---|
| `App` | 顶层 clap Parser；noun-first 命令组：agent / ability / device / mission / runtime / mcp + 横切 doctor / completion | `src/cli/mod.rs:198` |

facade 极薄：`mission` 子命令 → `mission_runs.rs::run_mission_inproc()` → `eal::planner::compile` + `eal::interpreter::execute_*`。

---

## 7. 横切设计

### 7.1 错误模型 —— 全平台无单一全局错误类型

平台**没有**单一全局错误类型，这是有意为之。错误按子系统建模，刻意区分 **wire/receipt 错误**（丰富分类法、稳定字符串）与 **internal/SDK-facade 错误**（thiserror enum，边缘用 anyhow）。三个层：

1. **WIRE 分类法（Axon 权威）：** `invocation::error::AxonError` **struct**（`error.rs:239`）—— 7 类模型（见 §2.3.5）。serde-transparent，round-trip 进受签回执。
2. **SDK-FACADE（Axon crate-root）：** `error::AxonError` **enum**（`error.rs:47`），`thiserror`，按生命周期排序，re-export 为 `easynet_axon::AxonError` + `AxonResult`。**陷阱：一个 crate 两个 `AxonError`。**
3. **CLI-INTERNAL：** 各子系统 thiserror/enum，**刻意不复用** wire 分类法。`EalError`（5 变体，§6.5）、`AbilityControlPlaneError`（45+ 变体，构造器无 panic）、ShellGuard 的 fail-closed sum type（`SecurityVerdict::Reject(SecurityViolation)`、`ParseForSecurityResult`——是 fail-closed 和类型，不是 Result）。

**约定（一句话）：** *anyhow inward, typed outward at any boundary another party observes.* handler 闭包与 boot/IO 路径用 `anyhow::Result`（`LocalRpcHandler = Arc<dyn Fn(Value)->anyhow::Result<Value>>`）；每个治理/协议/安全边界用 typed enum。gRPC 边界经集中式 `status_from_axon_invoke_error` helper 产 tonic `Status`，防 per-arm 漂移。

### 7.2 trait 分类（抽象骨架）

骨架**薄且角色专一**，无上帝 trait。trait 聚成五个功能带，object-safety 是自觉的门槛：kernel-registry trait object-safe（异构 `Arc<dyn T>` 注册表）；compiler/dispatch trait 用 `clone_for_thread` 而非泛型以保 rayon send-ability。

| 带 | 主题 | 代表 trait | 仓库 |
|---|---|---|---|
| 1 | Identity/Canonicalization | std trait on newtype + 单一 parse 函数（**非 trait**） | Axon ura-rs |
| 2 | Admission/Key Resolution | `KeyResolver`（server 经 `run_admission_gate` 复用同一原语） | Axon + server |
| 3 | Dispatch/Execution（真骨架） | `AbilityRegistry` / `SessionProvider` / `ShardResolver` / `InvocationRelay` / `LocalRuntimeAuthority` / `SessionFrameDispatcher` / `StepDispatcher` | 两仓 |
| 4 | Namespace/Discovery（RFC-005） | `NamespaceRouteResolver` / `NamespaceDirectory` / `NamespaceDiscoveryResolver` / `NamespacePolicyGate` / `AliasStore` / `DiscoverFederationResolver` | server + CLI |
| 5 | Extension/Test seam | `ContextLoader` / `TimeProvider` / `TeachClock` / `DeviceOpsClock` / `AcquiringArtifactTxn` / `BuiltinPluginBinding`（函数指针，无 vtable） | 两仓 |

**分层规则：** trait 抽象一个**接缝**（传输后端、密钥源、时间、策略），**绝不抽象数据形状**。gRPC `Invocation` trait（3 方法）是两仓都实现的**唯一协议表面**——其余一切都是这个 trait 下的能力名，不是新 service trait。

### 7.3 全状态机清单

两种风味：（A）wire/repr enum，i32 布局由 proto 钉死、`TryFrom` 边界校验（F-012，非法状态 parse 处拒，永不进入）；（B）编译期 typestate enum，gate 方法可用性。

**核心 invocation 生命周期（定义两次并保持同步）：**
- Axon SDK `InvocationState` 9 态 `#[repr(i32)]`（`handle.rs:27`）：Unspecified→Accepted→Admitted→Dispatched→Running→{Completed|Failed|TimedOut|Cancelled}；`is_terminal()` 谓词。
- Axon Runtime `InvocationState` 7 态（`src/common.rs`，proto_enum!）：Accepted→Dispatched→Running→{Completed|Failed|TimedOut}（server 投影时丢 Admitted/Cancelled）。
- CLI 经 invocation_watch 投影消费两者。

**拓扑/生命周期（Axon runtime，`src/common.rs`）：**
- `NodeState`：Probation→{Healthy|Suspect|Quarantined|Draining}（sweep 驱动）
- `CapabilityState`：Installed→{Activated|Deactivated|RolledBack|Revoked}
- `MissionState`：Planned→Dispatching→Running→{Partial|Completed|Failed|Cancelled|Aborted} + `StepState`：Pending→Running→{Completed|Failed|Cancelled}
- `CircuitState`：Closed→Open→HalfOpen→Closed
- `KeyState`：Active→{Rotated|Revoked}
- `AgentStatus`（`state/agents.rs`）：Active | Revoked{reason} | Suspended{reason}
- `FederatedInvocationState`：Accepted→[Started]→{Completed|Failed|TimedOut}

**TYPESTATE（编译期 gated，两仓）：**
- Axon：`UnaryInvokeAdmissionState`（Unsigned→Verified→Admitted）；`TerminalProofState`（NotRequired|Emitted|ConstructionFailed）；`DescriptorBoundEnvelope`（构造时 validate()）；`IdempotencyOwnerState`（Replayable|Expired|InflightPendingPublication|Orphaned）
- CLI：`AbilityControlPlaneRegistrationStage`（Planned→Materialized→Committed，快照-恢复回滚，`registry.rs:138`）；`TargetKind`/`IrTarget`（Agent|Device，parse 时定，编码"无隐式 agent 回退"不变量）；EAL `MissionOutcome`（Completed|Partial|Aborted）/ `StepOutcome`（Completed|Failed|Skipped|Internal）/ `VerifyDone`（True|False|Malformed）；`DescriptorImportState`（Acquiring→{Active|rollback}，Active→Forgetting→{removed|restored}，持久 tombstone，`teach_grants.rs:251`）；`MissionRunStatus`（Running→{Completed|Failed|Aborted}，heartbeat-mtime liveness，F-022）

**LIVENESS/RECLAIM（Axon client-sdk，最防御）：** Node-ID Lock Lifecycle（Unacquired→Locked→{Reclaimed→Removed|Restored}，两阶段 pre/post 策略）；`ProcessLiveness`（Alive|Dead|Unknown，保守默认 Unknown ⇒ 不回收除非 force+host 匹配）。

**TRANSPORT/SESSION（CLI invocation_transport，§5.5）：** Session Liveness（Absent→Online→Offline，channel 成员即 liveness）；Dispatch Contract Negotiation（Legacy_v0↔v1_Carrier）；Remote Bidi（Open→Active→Closed）；Escalation（Local|Escalation）。Server transport：`SessionStatus`（Created→Attached→Closed）；`ResolverProfileState`（fail-closed 8 门状态机，capped at AuthoritativeLocal）。

**跨所有状态机的模式：** 终态显式 + 谓词检查；转移钉成回执事件（审计链是黄金日志）；typestate 变体使非法转移**不可表示**而非运行时检查。

### 7.4 所有权 / Arc / lifetime 约定 —— 所有权梯度

一条清晰的、键于生命周期与争用的所有权梯度，两仓一致：

1. **Identity/Value 层 = owned-or-borrowed，零 Arc。** URA/identity 类型（`Ura(String)`、`ParsedURA`、`ReceiptRef`、所有 CLI ability newtype）小、Clone、要么克隆要么经访问器借用（`.as_str()` 零拷贝）。ura-rs 注释原文："identity types are small and either cloned or borrowed; no Arc or Rc in the URA layer."
2. **Invocation Core = `Arc<…>` 共享，Mutex 守的规范日志。** `Arc<InvocationCore>` 被 Handle + AbilityContext + 订阅者共享；receipts `Vec`（Mutex 内）是唯一规范日志，所有事件/状态视图按需投影（无并行事件日志）。Weak clone 允许优雅关闭不成生命周期环。
3. **Process State = `Arc<AxonState>` + 内部可变性，锁拓扑 per-map 调优（T3.1/F-011）。** DashMap（lock-free 热路径：invocations/nodes/installs/agents/missions）；RwLock（读多共享：abilities registry / admission_key_resolver）；Mutex（串行化工作：nonce check_and_record **only**——Ed25519 verify 跑在 mutex 外）；broadcast::Sender（非阻塞 fan-out）；AtomicUsize+compare_exchange（member_count）；AtomicU64（指标）；OnceLock（boot-set-never-mutated 单例：shard_resolver / hub_forward_dispatcher）。
4. **CLI Catalog = boot 后不可变，Arc 包裹，仅热重载用 RwLock。** `AxonAbilityCatalog` 单线程构建后 Arc 包；静态 handler map post-Arc lock-free；post-boot 变更隔离到 `RwLock<DynamicCatalogue>` + `Mutex<()>` 事务临界区。`Arc<OnceLock<Arc<AxonAbilityCatalog>>>` late-binding 解 registry↔runtime bootstrap 鸡生蛋。
5. **RAII/Drop 无处不在（凡有资源或秘密）：** `NodeIdLock`（锁文件清理）、`ExclusiveFileLock`/`SharedFileLock`（advisory-lock 释放）、`HeartbeatPump`（停后台线程）、`MissionRunDir`、`CalleeSigner`（Ed25519 key 上 ZeroizeOnDrop）、`BidiStreamHandle` channel-close ⇒ 终态回执。

**ACTOR vs SHARED-STATE：** 主体是**共享状态 + 细粒度内部可变性**，**非** actor-per-entity。actor-ish 例外只有 bidi session 模型（`BidiStreamHandle` !Clone 单 owner，`.split()` 成可 clone Writer + 单 owner Reader）和 `tokio::spawn` 自己 session-生命周期循环的 handler 闭包。**deferred-emission 纪律：** 事件在 DashMap RefMut 下收集、**守卫 drop 后**再 flush（`DeferredMembershipEvent`/`DeferredAudit`），避免跨 await 持 shard 锁——这是一条被标记为坏味道的隐式排序契约。**两仓核心路径无 unsafe。**

### 7.5 newtype 纪律

newtype 在**每个**裸 `String`/`[u8;32]`/`i32` 可能混淆的边界包裹身份。纪律是"parse, don't validate-then-pass-string"：构造即校验门，内层基元私有。

- **IDENTITY（ura-rs）：** `Ura(String)` 内层私有，仅 `.as_str()`/`.into_string()`。双模构造：total builder（`Ura::agent`/`::device`/…，无校验产合法串）+ fallible `Ura::parse`/`TryFrom`（返 `Result<_,ParseError>`）。`ParsedURA` 分解进**私有** `ParsedURABody` enum，封在类型化查询方法后——调用方**不能** match body。
- **HASH/VERSION（CLI 控制面）：** `SchemaHash([u8;32])`、`DescriptorHash([u8;32])`、`AbilityDescriptorVersion(String, 校验语法)`，带 `.hex()`/`.prefixed_hex()`("sha256:…")——防把 schema_hash 当 impl_hash 传。`AbilityControlPlaneKey` 是统一三表的复合 newtype。
- **WIRE ENUM：** `proto_enum!` 宏（`src/common.rs`）包 i32 proto enum，给 round-trip + `TryFrom` 越界拒绝。CLI `CallMode`(Rpc/Stream/Bidi) 镜像 Axon `AbilityCallMode`，带 `.axon_call_mode()` 投影。
- **OWNER/SOURCE 和类型（替代字符串嗅探）：** `OwnerKind`（Device|Hub|Agent(id)|User(id)，RFC-001 结构性根治）；`AbilityImplSource`；`ResourceType::ALL` 单源 const list 驱动 schema gen + FromStr。
- **SERDE-TRANSPARENT newtype（线缆兼容）：** `IrLoopTag` 单例判别子序列化成字面 `"loop"`，使 `#[serde(untagged)] IrStep` 无 tag 字段（保 pre-PR-10 Call-only 线缆形）。

**THE RULE：** 基元穿模块边界**只能**裹在拥有自己校验的 newtype 里；少数残留 stringly-typed 面（`BTreeMap<String,Handler>` ability 键、错误 context `BTreeMap<String,String>`）**被显式标记为已知债务**，post-v1 deferred。

### 7.6 异步对象模型

tokio 基础，围绕**一个 gRPC service trait** + receipt-投影事件流 + 刻意的 sync/async 桥构建。非 actor-framework；手卷 channel + 显式所有权。

**SERVICE 面：** 整个协议就是 tonic `Invocation` trait（3 异步方法）。`DaemonInvocationService`（CLI）与 `AxonRuntime`（server）各实现一次；federation/voice/mission/capability 是其下能力名，非新异步服务。只有两个旁路服务（Namespace.Resolve、PayloadTransfer）。`AxonRuntime`/`DaemonInvocationService` 是 cheap-clone Arc 句柄，per-RPC spawn。

**CHANNEL 类型与角色：**
- `broadcast::Sender`：所有可观测事件的非阻塞 fan-out；`StreamSource` enum（Snapshot|Live|SnapshotThenLive）一个 handler 给"重放历史 + 订阅未来"。
- `mpsc::channel`：hub→device 派发（capacity 256）、bidi up/down chunk；`try_send` disposition 编码 liveness。
- `oneshot`：unary await-reply 跨 bidi session。
- `tokio::sync::Notify` + `Arc<Mutex<>>`：InvocationCore watcher 唤醒。
- `Box<Pin<dyn Stream+Send>>`（BoxedDownStream）：tonic 边界 per-RPC 响应流装箱。

**BIDI 模型：** `BidiStreamHandle` !Clone（per-attach 单 owner），`.split()` → 可 clone Writer + 单 owner Reader。`SessionProvider` 实现 spawn 自己拥有 session 生命周期的 pump 任务；channel close 触发终态回执。

**BACKPRESSURE：** `BackpressurePolicy`（Unbounded 从 receipts 重放 | Block(capacity,max_wait) 有界 mpsc）构造时钉死。server `invoke_slots` Semaphore 界定并发，带 queue-timeout admission。

**SYNC/ASYNC 桥（CLI，load-bearing）：** EAL 在 `tokio::task::spawn_blocking` 内跑同步 handler；mission PHASE 经 `rayon::scope` work-stealing + crossbeam SegQueue 收集，每 worker 拿 `dispatcher.clone_for_thread()`。`run_blocking`/`try_run_blocking_in_tokio`（`src/support/async_bridge.rs`）配 `NoRuntimeFallback` enum（UseFuturesExecutor | BuildCurrentThreadTokio）是**唯一获认可的 sync←async 配方**——future+output 必须 Send，无 lifetime 跨界。

**并发安全不变量：** handler 闭包**按值捕获**（String/Arc clone），永不 `Rc<RefCell>`——安全并发由构造保证；唯一共享可变性走类型化内部可变层。

---

## 8. 跨仓接缝 —— CLI 如何投影 Axon SDK 类型

接缝是**单向 facade**：EasyNet-Axon **拥有**本体（Envelope/URA/Invocation/Receipt 在 Axon proto + Rust SDK builder 定义一次）；EasyNet-Cli 是下游消费方，**投影** Axon 类型进产品 facade，**绝不 fork 线缆语法**。这**不是重复**——下面是已验证机制。

### 8.1 proto 所有权门（Cargo feature）

CLI 的 `axon-pb` feature（默认开）门控 `easynet-axon/grpc`。Cargo 注释原文："The SDK owns Axon proto codegen; CLI must not compile ../EasyNet-Axon/.../proto directly." CLI 只经 `easynet_axon::pb::axon::v1::*`（如 `InvokeRequest`、`CausalContext`）触达 proto 类型——**消费生成代码，从不重新生成**。

> **盲点（MEMORY 记载）：** 默认 build 是 proto-free 的；声称"build clean"前**必须** with/without `axon-pb` 都 build + clippy。这是一个已记录的 blindspot。

### 8.2 re-export / import 面

CLI 从 `easynet_axon::invocation::{LocalRuntime, CausalContext, ReceiptRef, CallerSignature, CallMode, AgentIdentity, SubjectIdentity, UraProfile, InvocationState, InvocationLedger, AbilityChangeEvent, axiom::{AuthorityBinding, CanonicalAbilityDescriptor, InvocationUsage}, audit::{HostedAgentReceiptHeader, SigningModel}, persistence::PersistentLog}` 以及 crate-root `easynet_axon::{AxonError, AxonResult}` import 一切线缆形类型。~40+ import site 散布于 `runtime/`（ability_dispatch、kernel、dispatch_receipt）、`daemon/axon_bridge/`、`daemon/invocation/`、`daemon/`、`ffi/`。

### 8.3 投影方法（CLI 类型 → Axon 类型，重算规范哈希）

facade 的心脏：CLI 的唯一 `AbilityDescriptor` 聚合调用 Axon canonical hash 原语；`CallMode::axon_call_mode()` 只在 Axon 边界完成 enum 投影，不存在第二个 descriptor record。

CLI 持更丰富的三注册表控制面元数据（descriptor/authority/impl，Axon 没有），但**每个被签或被路由的字节都经 Axon 函数规范化**。

### 8.4 错误接缝（已审、非 lossy）

CLI **从不内部复用** Axon wire 错误类型；它在边界经 `EalError::from_axon_error(easynet_axon::AxonError)`（`error.rs:159`，已验证磁盘）转换，per-variant 显式 match（§6.5），并**刻意省略 `From<String>`** 禁止静默误分类。tonic Status 映射集中（`status_from_axon_invoke_error`）。

### 8.5 运行时接缝

`axon_bridge/` 模块是结构边界：CLI 的 `AxonAbilityCatalog` 把注册**立即写穿**到 `easynet_axon::invocation::LocalRuntime`（LocalRuntime 是可用性真理源；catalog 是元数据 + 派发）。`HotAgentRegistrar` 在 post-boot 把 handler 集物化进 LocalRuntime + catalog。server 端 `AxonRuntime::run_admission_gate` 复用 SDK 暴露的**同一** `easynet_run_axon_client::admission::run_admission` 原语，把本地 `RealmDirectory` 当 `KeyResolver` 插入——admission 逻辑单源于 Axon。

### 8.6 净结论

跨仓契约 = **"Axon 定义并规范化；CLI 富化并派发。"** facade 由三点强制：(a) proto feature 门，(b) 投影方法把每个哈希/签名委托给 Axon 函数，(c) 显式已审的 error/Status 转换。**CLI 没有任何地方重实现 envelope/receipt 规范字节。** 当下风险（审计 MEMORY 记载）是 **Go backend fork 协议层**（`backend/internal/axon` 8/15 文件在 Go 里 fork envelope/admission/URA）；而 **Rust CLI↔Axon 接缝是干净的**。

---

## 9. 设计评价

### 9.1 优点（教科书级干净处）

1. **本体单源 + 五主干清晰。** Envelope/URA/Invocation/Receipt 在 Axon 定义一次，CLI 投影。Identity→Admission→Policy→Receipt→Discovery 在代码里能逐根定位到具体模块。
2. **URA 设计是单一真理源。** 语法定义在 `parse_ura` + `parse_ability_tail` 两函数，所有 builder/accessor 路由经过，builder 与 parser 零漂移。`ParsedURA` 查询方法防 kind 误用（对 User URA 调 `.agent_ids()` 返 None，永不 panic）。
3. **三注册表分离关注点（教科书因式分解）。** descriptor（接口契约）/ authority（治理谓词）/ impl（执行绑定）无任一表混关注点；`AbilityControlPlaneKey` 统一，`assert_record_keys_match` 强制"全在或全不在"。
4. **typestate 把不变量编进类型。** `UnaryInvokeAdmissionState`、`TerminalProofState`、`AbilityControlPlaneRegistrationStage`——不可能在 proof 解析前 emit terminal，不可能跳过 materialize 直接 commit。F-012 让非法 invocation 状态在 parse 处失败。
5. **错误模型纪律严明。** 非 lossy 转换接缝（`from_axon_error`）、刻意省略 `From<String>`、集中式 Status 映射——"anyhow inward, typed outward"是真正贯彻的，不是口号。
6. **admission 锁拓扑为吞吐而设。** Ed25519 verify 跑在 nonce mutex 外；128 桶时间轮 O(1) 摊还重放检测；零哈希是受签事实而非缺失。
7. **EAL 是教科书编译器。** 五阶段流水线、typestate 编码本体不变量（无隐式 agent 回退）、有界 forensic trace、确定性 jitter 退避可复现测试。
8. **RAII 无处不在。** 锁、心跳、签名密钥、运行目录都靠 Drop 清理；`CalleeSigner` ZeroizeOnDrop 处理密钥。
9. **跨仓 trait 复用最干净处。** server 与任何 client 共享 `run_admission` 原语，`run_admission_gate` 仅是薄适配器。

### 9.2 坏味道（不干净处）

1. **传输层未收敛（最大一条）。** §5 已诚实声明：生命周期逻辑在 unary/stream/bidi 三几何形态间**重复**；五个传输状态机分散在三个派发器里各自实现。`DaemonInvocationService` 是 god-struct，跨 8-commit phase 中介全部三个 RPC 面。**`InvocationLifecycle` sink 收敛仍在飞行中。** 终局是把生命周期逻辑抽进单一 sink，把三派发器降为薄壳（dispatch shell）。
2. **`axon-took-it` flag proxy。** `unary_dispatcher` 返 `(response, bool)` 让 service 知道跳过 `record_unary_invocation()`——flag 是"LedgerSink 已见此行"的代理；理想是 ledger 成为订阅者（移进 Axon）。
3. **handler map 重复（6 静态 + 6 动态 = 12）。** 一 trait-对象-per-mode 设计能减半代码，但闭包 trait 对象强制同构形状。终局：sealed enum of handler types + 单 map。
4. **string-keyed 注册表。** ability 名是 `BTreeMap<String,Handler>` 键；admin-facing 错误仍引字符串。理想是符号键（`AbilityId` enum）。已显式标记 post-v1 deferred。
5. **deferred-emission 隐式排序契约。** DashMap RefMut 下收集、守卫 drop 后 flush 跨多 map 时需小心排序；被标记为坏味道。
6. **错误 context map 无类型无版本。** `BTreeMap<String,String>` 按字符串键插半结构化数据；重命名键会静默破坏消费者。
7. **OnceLock 双 set 警告。** `registry_builder` 在 boot→op_event 调 `set()` 两次；生产不可能但 ops 见警告。
8. **teach grants 泛型复杂度。** `AcquireStagedGrant<T>` 的 T 重复了 `AcquiringArtifactTxn` 已指定的东西；`DescriptorImportState` 的 `mark_active`/`mark_forgetting` 是 Vec 内结构的私有可变方法（field-mutation anti-pattern）。
9. **Go backend fork 协议层（跨仓最大风险）。** 不在本手册主体（Rust）范围，但记录在案：`backend/internal/axon` 曾在 Go 里 fork envelope/admission/URA，造成 Agent URA 字段漂移并丢失 device 语义。当前边界只允许 canonical URA；不得再引入第二套定位命名。

### 9.3 终局方向

1. **传输层收敛到 `InvocationLifecycle` sink。** 把 unary/stream/bidi 的重复生命周期逻辑抽进单一 sink；三派发器降为薄壳，把"提取所需平面"模式抬到 tonic 边界。ledger 成为该 sink 的订阅者，消灭 `axon-took-it` flag。
2. **handler 形状统一。** sealed enum of handler types + 单 map，替代 12 张同构 map。
3. **符号键替代 string 键。** `AbilityId` enum 替代 `BTreeMap<String,Handler>`。
4. **联邦 admission 用控制面哈希。** descriptor_hash + invoke_policy_hash + impl_hash 已是本地状态的确定性证明；未来 PR 用它们做联邦 admission（远端无需信任本地 daemon 即可验 descriptor 完整性）。
5. **Go backend 回归 Axon Go SDK。** 消除协议 fork，让 backend 像 Rust CLI 一样消费而非 fork 本体（审计：`docs/easynet-backend-boundary-audit-2026-06-08.md`）。
6. **五主干补全。** Receipt 主干的 `proof_facts` 仍缺 Axon SDK 开 `ProofFactsResolver`；Discovery 主干的 resolver profile 仍 capped at AuthoritativeLocal（A7 写路径已发但无周期性 snapshot 任务）。

**一句话终局判断：** 本体层（Axon SDK）和能力控制面（三注册表）已是教科书级、可作为新人 onboarding 的标杆；**传输层是唯一显著未收敛的子系统，也是下一阶段工程投入应当聚焦之处。** 库已经很漂亮——但"漂亮的库"不是终点，终点是那条贯穿 Identity→Admission→Policy→Receipt→Discovery 的、在每一层都被类型强制的主干。传输层是这条主干上最后一段还在用 runtime 检查而非类型不变量表达自己的路。

---

*本手册描述磁盘真实代码，每处类型断言可经 `grep <类型名> <文件>` 验证。发现与代码不符之处，以代码为准并提交勘误。*
