# Axon gRPC 收口确认清单 — 致 Axon / 架构师

**日期：** 2026-05-30 · **提出方：** CLI 团队（接口消费者视角）
**基线：** EasyNet-Axon `v0.99.4` · AXON-RFC-001 状态 = **Draft（open for review）**，2026-04-26
**约束声明：** 本文**不修改 Axon 仓库任何代码**（遵守 RFC-003 §0「禁改 Axon」）。这是一份**请 Axon / 架构师拍板的确认请求**，清理动作须走 Axon 自己的 RFC 决策 + 团队评审流程。

---

## 0. 一句话

RFC-001 把 13 个 gRPC service 收口为「单一 Invoke 原语 + ability」。**从代码事实核验：150 条 RPC 里 140 条已被 Invoke 三原语覆盖，6 条作为传输基座保留，剩 4 条仍是 RFC-001 §4 未拍板的 NEEDS-DECISION。** 其中 3 条随 `InvokeBidi` 落地已**技术性闭合**，**唯一真未闭合的覆盖缺口是 voice 事件流要的「at-least-once 有序投递 Receipt 类别」——经查证，Receipt 模型里不存在这一类。**

收口的「最后半步」不在 CLI，在 Axon：**(a) 11 个空壳 service 声明该不该删；(b) Receipt 模型补不补 delivery-class。** 这两件都只有 Axon 能做、且都需要 RFC 决策。

---

## 1. 13 个 service 的真实状态（逐文件核验）

> 数据来源：`core/proto/axon/v1/*.proto` 逐文件 `grep -c "rpc "` + RFC-001 mapping 表。

| # | Service | proto | 活 RPC | RFC-001 归类 | CLI 是否消费 |
|---|---|---|---|---|---|
| 1 | **Invocation** | invoke.proto | **3**（Invoke / InvokeStream / InvokeBidi） | **KEEP**（核心原语） | ✅ **唯一真消费** |
| 2 | **PayloadTransfer** | transfer.proto | **4**（Upload/Download/GetMeta/Delete） | **KEEP**（blob 基座） | ❌ 零消费 |
| 3 | Admin | admin.proto | 0 空壳 | DELETE → `admin.*` ability | ❌ |
| 4 | CapabilityLifecycle | capability.proto | 0 空壳 | DELETE → `capability.*` | ❌ |
| 5 | ControlPlane | control.proto | 0 空壳 | DELETE → ability（trust get/set） | ❌ |
| 6 | Federation | federation.proto | 0 空壳 | DELETE → `federation.*` | ❌（CLI 走自有模块→Invoke） |
| 7 | Identity | identity.proto | 0 空壳 | DELETE → `identity.*`（WhoAmI/agent） | ❌ |
| 8 | MissionControl | mission.proto | 0 空壳 | DELETE → ability | ❌（CLI 自带 EAL Mission IR） |
| 9 | Observe | observe.proto | 0 空壳 | DELETE → `observe.*` | ❌ |
| 10 | Policy | policy.proto | 0 空壳 | DELETE → `policy.*` | ❌ |
| 11 | StateSync | state_sync.proto | 0 空壳 | DELETE → `state.*` | ❌ |
| 12 | Stream | stream.proto | 0 空壳 | 5 DELETE + **2 NEEDS-DECISION** | ❌ |
| 13 | Voice | voice.proto | 0 空壳 | 14 DELETE + **2 NEEDS-DECISION** | ❌ |

**RFC-001 官方总账（150 条 RPC）：** DELETE **140** · KEEP **6** · NEEDS-DECISION **4**。

**CLI 消费事实：** 真正来自 Axon proto 生成码的，**只有 `Invocation` service**（`invocation_client` / `invocation_server` / bidi 消息）。`PayloadTransfer` 零消费；11 个空壳 service 的 service 与 message **CLI 全部零引用**（CLI 通过 `Invoke` 的 `function_name` + JSON `arguments` 调 ability，不依赖那些 protobuf message 形状）。
> 注意：CLI 里大量 `federation_client::*` 是 **CLI 自己的 `services::federation_client` 模块**，非 Axon proto，勿混淆。

---

## 2. 「Invoke 真的覆盖 11 个 service 了吗」——查证结论

**覆盖 10.5 / 11。** 逐条核验：

| RFC-001 §4 的 4 个 NEEDS-DECISION | 当初为何 Invoke 装不下 | `InvokeBidi` 落地后现状 |
|---|---|---|
| `stream.OpenStream`（真双向流：摄像头/遥测/传感器） | 当时 InvokeStream 仅 server-streaming（单向下行） | ✅ **已闭合**——`InvokeBidi` 是真 bidi；`BinaryChunk` 带 `sequence` + `StreamDescriptor.ordering="STRICT"`，重述为 `stream.open` ability 即可 |
| `stream.ResumeStream`（断线 bidi 续传） | 同上 | ✅ **基本闭合**——`sequence` 单调序号即续传锚点 |
| `voice.WatchCallEvents` / `WatchTransportEvents`（at-least-once + 按序 + terminal 投递保证） | Receipt 默认模型不保证按序/不重复 | ❌ **未闭合**（见 §3） |
| `voice` profile 落 daemon / hub | 部署拓扑归属未定 | ⚠️ **与原语覆盖无关**——纯部署决策，不是「Invoke 装不装得下」 |

`InvokeBidi` 能力已核验（invoke.proto）：双向 + `sequence`（按序/续传锚）+ `mac`（每帧完整性）+ `BinaryChunk`（任意二进制：媒体/PTY）+ `BidiControl`（PtyResize/PtySignal/MediaTimestamp/eof）+ 下行首帧即 admission `InvocationReceipt`。**stream 的两个缺口因此技术性闭合。**

---

## 3. 唯一真未闭合的覆盖缺口：Receipt 模型缺 delivery-class（查证实据）

RFC-001 §4 Decision 3 要求：给 Receipt 模型加一个 **at-least-once + 有序投递**的可选 Receipt class，用于 voice 事件流。**查证结果：不存在。**

| 查证点 | 结果 |
|---|---|
| SDK `ReceiptType` 枚举（`sdk/rust/src/invocation/audit.rs:26`） | 13 个值全是**生命周期状态**（Accepted/Admitted/Running/Progress/Completed/Failed…），**无任何投递保证维度** |
| `InvocationReceipt` message（invoke.proto） | 哈希链记录（prev_hash/self_hash/签名/axiom binding），**无 delivery-class / ordering / at-least-once 字段** |
| SDK 全局搜 `at-least-once / delivery-guarantee / ReceiptClass / DeliveryClass` | **零命中** |
| RFC-001 §4 Decision 3 | 仍是 **NEEDS-DECISION**，推荐解法标注「P3 设计」，**未落地** |

**易混淆点（已厘清）：** 有序投递语义**存在但在错误的层**——`StreamDescriptor.ordering="STRICT"`（invoke.proto:497，可靠/按序/`sequence==last_seen+1`，PTY 强制，v1 接收方必须拒绝非 STRICT）管的是**一次 bidi 会话内的数据帧**；Decision 3 要的是 **voice 事件作为一类可独立审计的 Receipt** 的跨投递保证。前者有，后者无。

---

## 4. 请 Axon / 架构师拍板的确认项

### 确认项 A —— 11 个空壳 service 声明的去留（RFC-001 收口正名）
- **事实：** 11 个 `service X {}` 体内 0 RPC，与 RFC-001「调用面只有一个原语」自相矛盾；它们是旧「每域一 service」架构的残留，留着会让人误以为 13-service 架构仍活（README 仍宣传「13 services」即此误解之源）。
- **问题：** 这 11 个空壳 service 声明，**删，还是显式标注「intentionally empty — restated as ability per RFC-001」保留？**
- **谁能做：** **仅 Axon**（RFC-003 §0；CLI 无权）。

### 确认项 B —— 空壳 service 的 message 类型存废（取决于 ability 传参格式）
- **岔路口：** ability 的 args/receipt 用**强类型 protobuf message**，还是**自由 JSON**？
  - 走 proto → `PolicyRule`/`MissionStep`/`GrantConsent` 等 message **必须保留**（ability 数据契约）。
  - 走 JSON → 这些 message 是旧 RPC 时代遗物，**架构上多余**。
- **CLI 侧证据：** CLI 已用 JSON args 调 ability，对这些 message **零依赖**——至少在 CLI 这个消费者眼里它们已用不上。
- **问题：** **请确认 ability 传参的官方格式**，据此决定那一批 message 的存废。
- **谁能做：** Axon RFC 决策。

### 确认项 C —— Receipt 模型是否补 delivery-class（§3 的缺口）
- **事实：** Receipt 模型无 at-least-once 有序投递类别（§3 实据），这是 voice service 收口的**最后一个真实阻塞**。
- **问题：** **是否采纳 RFC-001 §4 Decision 3 的推荐解（A）**——给 Receipt 加 at-least-once+有序的可选类别，从而把 voice 事件流收编进 `InvokeStream`/`InvokeBidi`？还是 voice 暂作 permitted exception（Decision 3 选项 B / Decision 4 选项 C）？
- **谁能做：** **仅 Axon**。

### 确认项 D —— RFC-001 §4 四个 NEEDS-DECISION 何时定稿
- OpenStream / ResumeStream（Decision 1+2，§2 已技术闭合，待正式重述为 `stream.open`/`stream.resume`）
- voice 投递保证（Decision 3，即确认项 C）
- voice profile 落 daemon/hub（Decision 4）
- **问题：** RFC-001 仍是 Draft；这 4 项定稿后，11 空壳 service 的清理才有依据。**请给出定稿目标。**

---

## 5. CLI 侧可自行收口的点（不涉 Axon，单独决策）

唯一不违反 RFC-003 §0 的 CLI 侧收口：`build.rs` 现用 `read_dir(axon/v1)` **无差别编译全部 14 个 proto** 且 `build_server+build_client` 全开，为 11 个永不调用的空壳 service 生成空 stub。
- **可选收窄：** 编译范围 → `[invoke.proto, types.proto]`（已验证 `invoke` 仅依赖 `types`，`types` 零依赖，收窄干净）。
- **取舍：** 若确认项 B 定为「ability 传参走 proto message」，未来接控制面 ability 时需把相应 proto 加回。故此项**应在确认项 B 有结论后再动**。

---

## 6. 收口路线（建议次序）

```
1. Axon 定稿 RFC-001 §4 四个 NEEDS-DECISION（确认项 D）
     └─ 含 Decision 3 = 确认项 C（Receipt delivery-class）
2. Axon 确认 ability 传参格式（确认项 B）→ 决定 message 存废
3. Axon 清理 11 空壳 service 声明（确认项 A）+ 按 2 的结论清理 message
     └─ 走 Axon RFC + 团队评审；CLI 只读，随后同步 proto 副本
4. CLI 侧据 2 的结论决定 build.rs 是否收窄（§5）
```

---

*核验命令与实据：service/RPC 计数 = `core/proto/axon/v1/*.proto` 逐文件 grep；CLI 消费面 = `src/**/*.rs` grep `pb::axon::v1::` + `build.rs:compile_axon_proto`；Receipt 模型 = `sdk/rust/src/invocation/audit.rs` + invoke.proto `InvocationReceipt`；RFC 归类 = `EasyNet-Axon/docs/rfc/AXON-RFC-001-restatement-mapping.md`（§1 总表 + §4）。本文未修改 Axon 仓库。*
