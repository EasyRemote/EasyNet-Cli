# CLI ⇄ Axon 接口审查（产品经理视角）

**日期：** 2026-05-30 · **审查范围：** EasyNet-Cli `v1.33.4` ⇄ EasyNet-Axon `v0.99.4`
**视角：** 不是"CLI 应该有哪些命令"（那是 [`cli-command-review-2026-05-30.md`](./cli-command-review-2026-05-30.md) 的命题），
而是"**两个仓库之间这条接口**：哪些是冗余的、哪些是缺失的"。

---

## 0. 接口长什么样（先对齐事实）

CLI 通过 **两条独立的通道** 消费 Axon，二者在磁盘上是两个不同的路径：

| 通道 | 是什么 | 路径 | 默认开启？ |
|---|---|---|---|
| **A. Rust SDK** | 纯 Rust 类型 + 客户端 API（`easynet-axon` crate） | `../EasyNet-Axon/sdk/rust` | ✅ 始终链接 |
| **B. Proto / gRPC** | tonic 生成的 `pb::axon::v1`（`Invocation` 服务） | `../EasyNet-Axon/core/runtime-rs/client-sdk/proto` | ❌ 仅在 `axon-pb` feature 下编译 |

**关键约束（来自 `Cargo.toml` / `build.rs` 注释）：**
> *"RFC-003 spec §0 forbids modifying axon repo"* —— CLI 只能**读** Axon 的 proto，不能改。
> 这意味着：任何接口冗余/缺失，CLI 侧都无权单方面修复 wire 契约，必须走 Axon RFC。这是整条接口的政治现实。

Axon 自我定位是 **Capability Control Plane**：六大保证（Addressability / Decidability / Explainability /
Auditability / Resumability / Boundedness），核心动词 **discover · invoke · receipt · bill**。
CLI 今天牢牢吃住了 **invoke** 和 **receipt**；接口的冗余与缺失，几乎全部落在另外两个动词和"控制面"上。

---

## 1. 冗余清单（接口上重复 / 已死 / 名实不符的部分）

> 冗余的代价不是"多占空间"，而是**让消费方（CLI）在两个真相之间猜**，以及**让新同事把死接口当活接口去对接**。
> 下面按"对产品的伤害程度"从高到低排。

### R1 — 【高】Proto 里 11 个"空壳服务"，但 README 还在宣传"13 gRPC services" 🔴

RFC-001 把 150 个历史 RPC 删掉 140 个、重述为 ability。今天 `.proto` 里真正有 RPC 的只有 **2 个服务**
（`Invocation` 3 个方法 + `PayloadTransfer` 4 个方法）。其余 11 个（`Admin / CapabilityLifecycle /
ControlPlane / Federation / Identity / MissionControl / Observe / Policy / StateSync / Stream / Voice`）
全部是 `// intentionally empty`——**只剩消息类型，没有方法**。

但 Axon `README.md` 的徽章和分层图仍写着"Proto 13 Services / 13 gRPC services"。

- **冗余表现**：接口的"自我描述"和"接口实体"对不上。一个新对接者读 README 会去找 `Federation.JoinFederation`，
  结果 proto 里那个 service 是空的。
- **对 CLI 的实际影响**：[`cli-command-review`](./cli-command-review-2026-05-30.md) 里大量被标成 `proto`（"今天就有"）
  的 backing，其实指向的是**空壳服务里的消息类型**，不是可调用的 RPC。即 §4 trust、§4.5 policy/permission、
  §8 federation actions 的 "proto-backed" ——**消息在、方法不在**。这是把"数据契约存在"误读成"可调用接口存在"。
- **PM 结论**：**这是接口文档债，不是代码债。** Axon 必须更新 README 的服务计数，并明确"能力通过 ability 名调用，
  不通过这些 service"。否则 CLI roadmap 的工作量估算会系统性偏低（以为是"暴露 RPC"，实际是"先确认 ability 命名空间是否落地"）。

### R2 — 【高】Proto 残留的 "vestigial" 消息类型（FanOut / Async / Batch / Query / Watch / Cancel / Replay）🟡

`invoke.proto` 里这些消息**对应的 RPC 已删**（被 RFC-001 折叠进单一 `Invoke`），但消息壳子还留着，
作为 ability 的参数/回执形状复用。

- **冗余表现**：同一个文件里，有的消息背后有方法，有的没有，肉眼无法区分。
- **对 CLI 的影响**：CLI 的 `wire_conv.rs` / `dispatch_shim.rs` 在做 `From<pb::*>` 转换时，要靠人脑记住"哪些是活的"。
- **PM 结论**：低优先级，但 Axon 应在 proto 注释里给这些消息打 `// RPC removed by RFC-001, retained as ability arg shape`，
  把"接口考古"成本降下来。

### R3 — 【中】Proto 在两个路径下各存一份（CLI 编译的不是"主"路径）🟡

- 主 proto：`core/proto/axon/v1/*.proto`
- CLI 编译用：`core/runtime-rs/client-sdk/proto/axon/v1/*.proto`（**逐字节副本**）
- 还有第三份：`dist/protocol-pack/1.0.0/proto/...`（打包子集）

CLI 的 `build.rs` 故意选 `runtime-rs/client-sdk` 那份，注释说"和 EasyNet-Federation-MVP/common/build.rs
用同一路径，保证两个 crate 编译的是 byte-identical 的 proto"。

- **冗余表现**：同一份 wire 契约三处副本，靠"约定"保持一致，没有 CI 校验它们真的一致。
- **风险**：哪天有人只改了 `core/proto` 那份、忘了同步副本，CLI 会编译出**过时的 wire 类型**而毫无报错——
  这是最危险的一类冗余（沉默地分叉）。
- **PM 结论**：要么 Axon 把副本改成构建时生成/软链，要么加一个 CI diff 守卫。**这是接口"防腐层"的缺口。**

### R4 — 【中】CLI 侧本地重实现的接口概念，目前与 Axon 并存 🟡

CLI 正在做"从自带实现迁移到直接消费 Axon"的过程（`src/runtime/axon_bridge/mod.rs` 记录了迁移）。
当下仍并存的本地副本：

| CLI 本地类型 | 镜像的 Axon 概念 | 当前状态 / 是否真冗余 |
|---|---|---|
| `runtime::invocation::{Invocation, Receipt, CausalContext, Ura}` | Axon AXIOM Invocation/Receipt | **半冗余**：是 KernelApi 边界的结构记录；签名字段 `caller/callee_signature` 永远是 `None`（"v1 信任仍走 Axon mTLS"）。注释说 v1 是"记录系统不是计算系统"。 |
| `HostedAgentReceiptHeader` | Axon Receipt（RFC-001 §A12 托管 agent 模型） | **暂不冗余**：托管 agent 无私钥，CLI 侧只是搬运 `host_attestation`，签名/验签仍委托 Axon。等后端签名落地后可收敛。 |
| `eal::ir::MissionStep` / Mission IR | Axon `MissionControl v1`（已是空壳服务） | **故意分叉，不是冗余**：CLI 注释明说这是"对 MissionControl v1 的关键改进"，加了 `input_refs`/`output_binding` 数据流，等 Axon 出 MissionControl v2 proto 再对齐。 |

- **PM 结论**：这一类大部分是**受控的过渡态**，不是该立刻删的死代码。真正要盯的是：**谁来宣布迁移完成、何时删本地副本**。
  没有这个"收尾负责人"，过渡态会永久化，变成长期冗余。已删除的并行实现（`AxonAbilityCatalog` 旧 dispatch、并行
  `AdmissionFacade`、`SharedReceiptStore`）证明这条收敛路径是可行的——但需要明确的收尾节奏。

### R5 — 【低】`proto-gen` 与 `axon-pb` 两个 feature 容易被混淆 🟢

`proto-gen` 编译的是 **CLI 自己的** `schemas/*.proto`，`axon-pb` 编译的是 **Axon 的** proto。命名上没有体现归属。

- **PM 结论**：纯命名问题。建议把 `proto-gen` 改名 `cli-proto-gen`（或在帮助文本里点明），消除歧义。低优先级。

---

## 2. 缺失清单（接口上该有、却没有跨过边界的部分）

> 缺失分两种，**产品上必须分清**，因为它们的"修复成本"差一个数量级：
> - **(a) 暴露缺口**：Axon 侧契约已存在，CLI 没接 → CLI 自己能补，**便宜**。
> - **(b) 协议缺口**：Axon 侧契约根本不存在（连消息都没有）→ 必须先做 Axon RFC，**贵、跨仓库**。

### M1 — 【P0 · 协议缺口 (b)】控制面动词没有"可调用接口"，只有消息壳 🔴

这是整条接口最大的缺失，也是最容易被误判的一处。

CLI 的命令审查把 **trust（§4）/ policy+permission（§4.5）/ federation actions（§8）** 都标成 "proto-backed = 今天就有"。
但结合 R1 的事实：**那些 service 是空壳，方法不存在**。所以真实情况是：

| 控制面动词 | Axon 现状 | 缺口性质 |
|---|---|---|
| `trust show/set`（GetNodeTrust/SetNodeTrust） | 消息类型在 `types.proto`，但 `ControlPlane` service 为空 | **协议缺口**——需要 RFC 决定它走 ability 还是补 RPC |
| `policy create/simulate/why`（Policy service） | 消息在，`Policy` service 为空 | **协议缺口** |
| `permission grant/revoke`（GrantConsent） | 消息在，`CapabilityLifecycle` service 为空 | **协议缺口** |
| `federation join/nodes`（ListFederatedNodes） | 消息在，`Federation` service 为空 | **协议缺口** |

- **PM 结论（最重要的一条）**：命令审查里"这是 exposure work, not protocol work"的判断，对**消息**成立、对**调用接口**不成立。
  RFC-001 的逻辑是"这些都重述为 ability"——所以**真正缺失的，是 Axon 把这些控制面能力以 ability 名义正式发布的那一层**
  （ability 命名空间：`identity.* / policy.* / capability.* / federation.*`，RFC-001 §5 列了，但**落地状态未知**）。
  在 CLI 动工前，**必须先向 Axon 确认这些 ability 命名空间是否已实现可调用**，否则 P0 工作量会被严重低估。

### M2 — 【P0 · 协议缺口 (b)】经济动词在接口上完全不存在 🔴

`Agreement / Wallet / Payment / Escrow` —— proto 里**连消息都没有**。这是纯粹的协议空白，
不是暴露问题。README 把 receipt 卖成"billing / liability 的基础"，但接口上没有任何东西把 receipt 变成钱。

- **PM 结论**：这是**跨仓库依赖**，CLI 无法单独推进。必须先确认 Axon roadmap 上有 EasyNet-Ledger + Agreement primitive
  及其目标版本，再谈 CLI 的 `wallet`/`agreement`。在那之前，receipt 只是取证工具，不是市场。

### M3 — 【P1 · 协议缺口 (b)】URA `namespace`（family）段：设计了、却没穿过接口 🔴

**证据级发现**（详见命令审查 §3.7）。ability URA 是三段点分：`<owner>.<namespace>.<ability-id>`，
但 Axon SDK 的 `ParsedURA`（`sdk/rust/src/ura.rs:117`）**只校验三段存在、然后把中间段丢掉**——
没有 `family`/`namespace` 字段，整条尾巴塌缩进 `ability_id: String`。

- **缺口性质**：这是**接口契约自身的内部缺失**——格式要求有这一维，实现没有保留它。
- **对 CLI 的影响**：CLI 想做 `discover --tree`（按 family 折叠）和 `policy` 按 family 授权，**无字段可 key**。
  hub 侧的 `hub.federation.resolve` 等已经事实上在用 `federation` 当 family，但用户侧 ability 拿不到这个维度。
- **PM 结论**：这是 **Axon SDK 侧的缺口**（要改 `ParsedURA`，且要六语言 SDK 对齐），不是 CLI 能补的。
  补上后，命令审查 §3.6 的 discovery-fold 和 §4.5 的 policy-scope "免费掉出来"。**性价比极高的一处协议补全。**

### M4 — 【P1 · 暴露缺口 (a)】Agent 作为"网络公民"的解析/注册接口 🟡

`RegisterAgent / ResolveAgent / MigrateAgent` ——产品定位 agent 是"有身份、记忆、关系、声誉、经济能力的一等公民"，
但接口上今天只有 node 能 resolve，agent 不能。

- **缺口性质**：需确认 Axon 是否把它作为 `identity.*` ability 发布（同 M1 的不确定性）。若已发布则是暴露缺口，若未发布则是协议缺口。
- **PM 结论**：**先问 Axon：agent-identity 是 v1.1 承诺还是更晚？** 这个答案决定它是 (a) 还是 (b)。

### M5 — 【P2 · 暴露缺口 (a)】可观测性接口没接 🟢

`GetNetworkHealth / GetSLOStatus / GetBurnRate / WatchEvents`（`Observe` service）——同样是空壳 service +
消息类型。CLI 今天只有本地 `doctor`，没有网络级 `health/slo/burn-rate`。

- **PM 结论**：与 M1 同源（依赖 Axon 把 `observe.*` 以 ability 发布）。运维向，missions 跑无人值守时才痛，P2。

### M6 — 【P2 · 接口防腐缺口】没有"接口一致性"的自动校验 🟡

承接 R3：CLI ⇄ Axon 之间没有任何自动机制保证——
- 三份 proto 副本一致；
- CLI 的 `ed25519` 派生方式和 SDK 一致（`Cargo.toml:151` 注释警告："不一致则 `verify_easynet_subject_key_binding`
  会拒掉每一个 Invoke"）；
- CLI 编译的 `pb` 类型与 Axon 当前 proto 同步。

- **PM 结论**：这是**接口缺一个"契约测试 / CI 守卫"**。今天全靠 `Cargo.toml` 里的人工注释维系。
  建议加一个跨仓库的 proto-diff + 一个端到端的"签名互认"冒烟测试。**这是把"约定"升级成"保证"的关键一笔。**

---

## 3. 一页纸总结（给决策用）

### 接口健康度判断
- **invoke / receipt 通道：健康**。SDK 的 `LocalRuntime` / `InvocationLedger` / admission gate / `KeyResolver` /
  `DendriteBridge` / MCP server 都是**真消费、非重实现**，迁移方向正确。
- **discover / control-plane / bill 通道：名存实亡**。proto 留了消息、删了方法，README 还在宣传旧服务数——
  这是冗余（R1）和缺失（M1/M2）同时发生的根因。

### 冗余 · 按修复成本排序
| # | 冗余 | 性质 | 谁来修 | 成本 |
|---|---|---|---|---|
| R1 | README 宣传"13 services"，实际 2 个活服务 | 文档债 | **Axon** | 低（改文档+说清 ability 调用模型） |
| R3 | proto 三份副本无 CI 守卫 | 防腐缺口 | Axon + CLI | 中（加 diff 守卫） |
| R4 | CLI 本地 Invocation/Receipt 副本（过渡态） | 受控过渡 | **CLI**（需收尾负责人） | 中 |
| R2 | proto 残留 vestigial 消息 | 考古成本 | Axon | 低（加注释） |
| R5 | `proto-gen` vs `axon-pb` 命名歧义 | 命名 | CLI | 低 |

### 缺失 · 按"是否阻塞在 Axon"排序
| # | 缺失 | (a)暴露 / (b)协议 | 阻塞点 | 优先级 |
|---|---|---|---|---|
| M1 | 控制面（trust/policy/permission/federation）可调用接口 | **(b) 协议** | Axon 是否已把 `identity/policy/capability/federation.*` 以 ability 发布？**先确认** | P0 |
| M2 | 经济动词（wallet/agreement） | **(b) 协议** | Axon roadmap 是否有 Ledger+Agreement？ | P0（但跨仓库依赖） |
| M3 | URA `namespace`/family 段未解析保留 | **(b) 协议（SDK 侧）** | 改 `ParsedURA` + 六语言对齐 | P1（性价比最高） |
| M4 | Agent resolve/register/migrate | (a)/(b) 待定 | 同 M1 的发布问题 | P1 |
| M5 | 可观测性（health/slo/burn-rate） | (b) 协议 | 同 M1 | P2 |
| M6 | 接口一致性 CI 守卫 | 防腐 | CLI + Axon | P2 |

### 给团队的三个必答问题（动工前）
1. **【最关键】** RFC-001 把 140 个 RPC 重述为 ability —— 这些 ability 命名空间（`identity/policy/capability/
   federation/observe.*`）**今天在 Axon 真的可调用了吗，还是只在 RFC 里？** 这一个答案决定了命令审查里一大批
   "P0 exposure work" 究竟是便宜的暴露活、还是昂贵的协议活。
2. **经济层**：EasyNet-Ledger + Agreement primitive 是否已在 Axon roadmap 且有目标版本？（gate 住 M2 全部）
3. **接口收尾**：R4 的 CLI 本地副本，**谁负责宣布迁移完成并删除**？给个节点，否则过渡态永久化。

---

*事实来源：Axon SDK `sdk/rust/src/`、proto `core/proto/axon/v1/*.proto`、RFC-001
`docs/rfc/AXON-RFC-001-restatement-mapping.md`；CLI `Cargo.toml` / `build.rs` 边界注释、
`src/runtime/axon_bridge/`、`src/services/axon_serve/`。命令级 gap 见
[`cli-command-review-2026-05-30.md`](./cli-command-review-2026-05-30.md)，本文与其互补——
那篇问"CLI 该有哪些命令"，本文问"两仓库这条接口冗余/缺失在哪"。*
