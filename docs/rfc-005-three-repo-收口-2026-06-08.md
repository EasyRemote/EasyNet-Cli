# RFC-005 三仓库收口审计 — 2026-06-08

> 视角：工业教科书。范围：EasyNet-Axon / EasyNet-Cli / EasyNet(backend) 三仓库
> 当前分支上的全部未提交修改，对照
> `EasyNet-Axon/document/rfcs/005-ura-namespace-resolution-dns-plan.md`
> 与 `pr/2026-06-06-ura-namespace-resolution/` plan pack 的执行状态。

---

## 0. 总览：三仓库现在站在计划的哪一格

RFC-005 的执行契约是 `07-commit-plan.md` 的提交序列
`P0 → A1..A7 (Axon) → C1..C3 (Cli) → B1..B3 (backend) → X1 (cross-repo)`，
依赖方向是单向的：Axon 是真相源，Cli/backend 只能消费。

| 仓库 | 当前分支 | 计划相位 | 已落历史 | 工作树未提交 |
|---|---|---|---|---|
| EasyNet-Axon | `codex/rfc-005-ura-namespace-resolution` | P0 + A1–A7 主体已 commit | `f1d8419` 抽 axon-ura crate、`d2b93e6` resolver/WAL/projection、SDK owner-token 对齐、跨语言 conformance | A 相位**增量**：typed receipt failure proto、Go SDK codec/类型、跨语言 advertise owner-projection、plan-pack 文档 |
| EasyNet-Cli | `codex/rfc-005-ura-namespace-resolution-cli` | C1 大部分已 commit（`d7d3cdf` F07 daemon transport + owner projection） | F07 daemon 传输、owner projection、ability URA | C1 收尾 + C2 主体：owner projection callable_summary、catalog 写入 fence、daemon namespace.resolve、session 准入 gate、typed receipt failure |
| EasyNet(backend) | `codex/rfc-005-ura-namespace-resolution-backend` | B1–B3 + D 主体 | 已合并 SessionAuthority/DirectoryEvent/proto 再生 | **巨量**（192M+18??）：SDK 类型别名化、删除协议 fork、ability URA 交还 SDK、resolver-backed listing、typed negative、前端契约对齐、e2e 脚本 |

**结论一句话**：Axon 协议/运行时主体已落历史，三仓库工作树里是**同一波 RFC-005 收尾增量**——
Go SDK 把 invoke-remote / SessionAuthority / DelegationProof / owner-projection 的 wire 真相收归
Axon，Cli 与 backend 同步从"自己造 URA / 自己拼 wire"切换为"消费 SDK 与 resolver 答案"。
边界纪律（Axon 拥有 URA·envelope·ResolveAnswer·receipt；Cli daemon 拥有发布/解析运行时；
backend 仅消费）在抽查中**成立**——backend 已无 `NormalizeInvokeTarget`，
`InvokeRemoteUpRequest` 已是 `axonsdk.*` 类型；Cli 未在生产路径本地铸造 canonical ability URA
（grep 命中均为 `#[cfg(test)]` 断言字面量）。

---

## 1. 执行状态对照（按 plan checklist）

### Axon（04-execution-checklist「Axon protocol/domain」「Axon runtime」）
计划里这两段几乎全 `[x]`，历史 commit 已覆盖。工作树未提交部分对应的是
**A6/A7 的收尾 + Go SDK 化**，新满足项：

- ✅ `Error failure = 30` 进 `InvocationReceipt`（三份 invoke.proto **逐字节一致**，vendoring 干净），
  runtime `admission_gate.rs` 产出 typed terminal failure（`AXON_*` 大写码、stage、retryable）。
- ✅ Go SDK：`session_authority.go` / `delegation.go` / `admission_subject.go` /
  `invoke_request.go` / `invoke_remote.go` / `enums.go`，**12 个文件全部配套 `_test.go`**，
  invoke-remote codec 强制 `ability_ura`、拒绝 legacy `ability` 字段。
- ✅ `federation.go` owner-projection builders（`AbilityProjectionSummary`、`ProjectionDigest`）；
  五语言 dendrite bridge 同步把 `agent_ura → owner_ura` 并加
  `host_device_ura / projection_digest / lease_expires_unix_ms`。

### Cli（「EasyNet-Cli」段）
- ✅ C1 已落：device-owned ability URA、owner projection、catalog 作为 read-model。
- 🟡 C1 收尾（工作树）：`callable_summary` 进 projection、`ProjectionUpsertOutcome` 做
  per-owner revision/digest fence、lease `is_live_at` 过滤。
- 🟡 C2 主体（工作树）：daemon `namespace.resolve` / `namespace.proxy_resolve` 返回 Axon typed
  `ResolveAnswer`、跨 realm 走 `federation.forward_invoke`、boot 期 `session.open` 准入 gate 失败即 fail-closed。
- ⬜ C2/C3 仍 `[ ]`：resolve-before-invoke 全量替换、`ResourceRef` 每次 fs 操作复核、
  `AuthoritativeLocal` 才进权威 dispatch、fs.* 树发布。

### backend（「EasyNet backend」段 + Phase D）
- ✅ 工作树已覆盖 plan 中标 `[x]` 的 B1/B 主体：删 Envelope/DelegationProof/SessionAuthority fork
  →SDK 别名；删 invoke-remote 镜像 struct→SDK codec；删 `NormalizeInvokeTarget`→
  `DescriptorRouteSelector` 校验 resolver 描述符；`agent.list/skill.list/device.list` 走
  resolver、剥离 `device.` 前缀取 canonical public name；typed negative
  （`ResolveDirectoryFailure` + `resolve_unavailable[]`）；前端 `ability_ura` 必填 + `ResolveUnavailableBanner`。
- ⬜ Phase D 仍 `[ ]`：discovery answer 消费、owner-projection 事件仅作失效信号、
  policy-scoped read-model key、`Preview/ShadowRead` 不入共享缓存、receipt failure 码渲染全量铺开。

> 即：三仓库都把 **A·B·C 的"协议化/SDK 化/typed answer"骨架**做完了，
> 剩下的清一色是 **release-profile 权威翻转（C2 尾 + Phase E）** 与
> **read-model/discovery 治理（Phase D 尾）**——也就是 plan 里依赖 `AuthoritativeLocal` gate 的部分。

---

## 2. 收口建议（合并前必须确认）

1. **Axon 先行落历史，再动 Cli/backend 提交**。依赖方向单向；Cli/backend 的工作树编译
   依赖 Axon Go SDK 新类型与 Rust `axon_pb` 枚举（`ResolveAnswerKind`/`NegativeReason`/
   `RecordType` 等）。**先确认 Axon `--features axon-pb` + 默认双构建通过**
   （见全局记忆 axon-pb build blindspot），否则 Cli `federation_wrappers.rs` 会编译失败。
2. **共享 checkout 用 pathspec 提交**。三个工作树各自独立分支，但本机是共享 index 风格；
   一律 `git commit -- <显式路径>`，绝不无 pathspec 提交（见记忆 commit-with-pathspec）。
3. **不带 `Co-Authored-By` trailer**，作者 `Silan.Hu`（记忆 no-coauthor-trailer）。
   commit body 按 plan 带 `Refs: AXON-RFC-005` / `Frame:` / `Decisions:` / `Verification:`。
4. **plan-pack 文档单独 commit**。Axon 工作树里的 `pr/2026-06-06-.../{03,04,05,06}.md`
   是进度勾选，按 P0 纪律与代码分离成 docs commit。
5. **proto vendoring 校验**：提交 invoke.proto 后跑 `scripts/checks/sdk_proto_vendor_in_sync.sh`
   （三份已确认一致，提交后再验一次防回归）。
6. **不要把 release-profile 翻成 Authoritative**：当前 Cli/backend 仍应停在 ShadowRead/
   compatibility dispatch，plan 明确 `AuthoritativeLocal` 是 A7 gate 之后的事。任何"已切权威"
   的描述都属越界，提交信息里不要这么写。
7. **未完项显式标注**：C2/C3、Phase D 尾、Phase E 的 `[ ]` 项写进各仓库 PR body 的
   "remaining blocked gates"，对应 plan 的 X1 closeout。

---

## 3. 建议提交划分（按 feature 收口）

### EasyNet-Axon（4 代码 commit + 1 docs commit）
| # | subject | 文件组 |
|---|---|---|
| A-r1 | `feat(runtime,proto): RFC-005 typed terminal receipt failure` | 三份 invoke.proto + `admission_gate.rs` |
| A-r2 | `feat(sdk-go): SessionAuthority/DelegationProof/admission-subject/invoke domain types + invoke-remote codec` | `session_authority.go`·`delegation.go`·`admission_subject.go`·`enums.go`·`invoke_request.go`·`invoke_remote.go`(+各 test)·`ura.go`·`invocation/axiom.go` |
| A-r3 | `feat(sdk-go): owner-projection builders for federation.advertise_abilities` | `federation.go`·`federation_test.go`·`FEDERATION_INVOKE_SCHEMAS.md` |
| A-r4 | `refactor(sdk): cross-language advertise owner-projection (Java/Node/Python/Rust/Swift)` | 五语言 dendrite_bridge + FederationBody + 对应 conformance test |
| A-d1 | `docs(rfc-005): update plan-pack progress` | `pr/2026-06-06-.../{03,04,05,06}.md` |

### EasyNet-Cli（4 commit）
| # | subject | 文件组 |
|---|---|---|
| C-1 | `feat(projection): callable summary + revision/digest fence` | `owner_projection.rs`·`ability_catalog_store.rs`·`discover_ability.rs`·`agent_list_ability.rs`·`pages/{mod,publish,list_get_unpublish}.rs`·`facade/cli/pages.rs` |
| C-2 | `feat(resolve): daemon namespace.resolve/proxy_resolve typed answers` | `daemon_invocation_service.rs`·`federation_wrappers.rs` |
| C-3 | `feat(boot): fail-closed session admission gate` | `boot.rs`·`session_initiator.rs`·`facade/cli/start.rs`·`facade/cli/join.rs` |
| C-4 | `refactor(forward): forward_invoke response via Axon SDK shape` | `support/federation_invoke.rs` + docs audit md |

> 注：C-2 与 C-4 都依赖 Axon SDK 已落历史；C-3 是 F07 传输的收尾，独立性强可先提。

### EasyNet(backend)（6–7 commit）
| # | subject | 文件组 |
|---|---|---|
| B-1 | `backend: alias Envelope/DelegationProof/SessionAuthority to Axon SDK, drop forks` | `axon/{invoke_types,delegation,session_authority,enums}.go`·`daemon_grpc/invoke_remote.go` |
| B-2 | `backend: delegate ability URA construction to Axon SDK` | `axon/{urns,advertise,ability_descriptor_reader}.go` |
| B-3 | `backend: route ability/skill/device/agent listing through resolver + typed negatives` | `axon/{federation_calls,resolve_answer,namespace_resolve_answer,resolved_agents}.go`·四个 `*Logic.go`·`resolverstate/`·`pb/axon/v1/namespace.pb.go` |
| B-4 | `backend: validate canonical invoke routes from resolver descriptors` | `urautil/{route_selector,ability}.go`·`invokeAbilityLogic.go`·`invoke_request_builder.go` |
| B-5 | `backend: failure locator product facade` | `failurelocator/` |
| B-6 | `api,frontend: require ability_ura + render typed resolver negatives` | `api/easynet.api`·goctl 再生 `types.go`/`routes.go`·`Frontend/**`（abilities/devices/file-transfer/DeviceDetail 等） |
| B-7（可选） | `scripts: canonical public ability names in e2e` | `scripts/docker-e2e-*.sh` |

---

## 4. 风险与未决（写进 PR body）

- **编译耦合**：Cli/backend 未提交码依赖 Axon SDK/`axon_pb` 新符号——必须 Axon 先落、先双构建。
- **权威翻转未做**：C2 resolve-before-invoke 全量、C3 ResourceRef 复核、Phase D read-model
  治理、Phase E `AuthoritativeLocal` flip 全部仍是 `[ ]`。当前是 ShadowRead/兼容 dispatch，**不得**
  在提交里宣称已进权威路径。
- **跨语言 projection digest**：仅 Go SDK 有 `ProjectionDigest()`，其余语言待补（plan "Go first"，非阻塞）。
- **未完 e2e**：CLI `agent add + skill list`、backend Agent detail 页 e2e 仍 `[ ]`，归 X1 closeout。
