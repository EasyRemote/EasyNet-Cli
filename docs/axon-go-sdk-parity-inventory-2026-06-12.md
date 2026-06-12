# Axon Go SDK ↔ backend fork parity 盘点(T2.2 设计件,2026-06-12)

> spec T2.2 前置:「Go SDK parity 盘点——fork 面 API ↔ SDK 现有面逐函数映射;
> 缺口先补 SDK,禁止半截替换」。盘点方法:Explore agent 全量枚举 + 关键论断亲手核验
> (urns.go 委托性、invoke_types 别名性、resolver answer 独立性——三项全部坐实)。

## 一、核心结论:F-015 的定性已经过时,执行图景大幅收窄

清单 F-015 与记忆「Axon Go SDK exists & imported but unused」基于 2026-06-08 边界审计。
RFC-005 namespace-resolution 跨仓 PR(Axon cd0a51b5 / EasyNet 2c1118d)落地后,实况是:

- backend 已活跃使用 **46+ SDK 导出**(go.mod `easynet.run/axon/sdk/go v0.41.5`,replace 指向兄弟仓);
- 20 个 fork 文件中 **9 个已是 SDK 薄委托**——其中 urns.go(557 行,A 批主目标)实测
  48 处 `sdk.` 委托、**0 处本地 URA 拼接**;invoke_types.go 是纯 `type X = axonsdk.X` 别名面。

**即:原 A 批(URA)与 B 批(admission/delegation/session_authority)的协议真源二元化已经消亡。**
T2.2a/b 剩余工作 = 验收级清理(确认薄委托层无漂移余地),不再是替换工程。

## 二、逐文件判定(20 文件,亲核标注)

### SDK-covered(薄委托,真源已唯一)— 9 文件
| 文件 | 行 | 判定依据 |
|---|---|---|
| urns.go | 557 | 【亲核】48 处 sdk. 委托,0 本地拼接;文件已是 facade |
| invoke_types.go | 58 | 【亲核】纯类型别名(`type Envelope = axonsdk.Envelope` 等) |
| admission.go | 121 | ValidateSubjectURA 来自 SDK |
| delegation.go | 92 | DelegationProof/Raw 系 SDK re-export |
| session_authority.go | 69 | SDK Sign/Verify/UnmarshalRaw 委托 |
| advertise.go | 268 | 包装 SDK FederationAdvertise* payload 构造器 |
| enums.go | 40 | SDK EnumNameLookup 回调模式 |
| invoke_request_builder.go | 39 | 以 SDK 类型构造 InvokeRequest |
| noop.go | 51 | 实现 SDK Client 接口的测试桩 |

### Partially covered(真实剩余债;处置 = 先补 SDK 再替换)— 7 文件
| 文件 | 行 | SDK 缺口 | 处置 |
|---|---|---|---|
| namespace_resolve_answer.go | 677 | 【亲核】0 处 sdk. —— RFC-005 resolver answer 解码链全在 backend | **缺口最大**:resolver answer codec 应入 SDK(answer 形状是协议形状,Rule 1);Rust 侧已有,Go SDK 补对等面 |
| resolve_answer.go | 318 | 负向 answer/failure lane 解码同上 | 随 answer codec 同批入 SDK |
| resolve_route.go | 246 | 路由选择 gate(FINAL_ROUTE/release profile)| 拆分:answer 解码归 SDK;**dispatch 策略留 backend**(产品策略非协议) |
| ability_descriptor_reader.go | ~~260~~ | ~~SDK 有 descriptor builder 无 reader~~ **✅ 已收口(2026-06-12)**:SDK 落 ReadAdvertisedAbilityDescriptor + JoinAbilityPublicName(与 split 同档互逆,write→wire→read 回环测试钉死;Axon b58d124c);backend 瘦身为纯产品投影,零 wire key 知识(EasyNet 2edbc56,−71 行;4/4 行为钉死测试不变) | 完成 |
| federation_calls.go | 649 | advertise payload 已用 SDK;resolve/proxy_resolve 包装层自有 | answer codec 入 SDK 后此文件瘦身为调用编排 |
| invoke_client.go | 267 | SDK Client 之上的 backend 抽象(LivenessProbe/RemoteInvoker/OriginCaller) | 评审:抽象属产品层可留;~~OriginCaller 形状须钉到 SDK 类型~~ **✅ 已钉(2026-06-12)**:SDK 落 NewOriginCallerClaim 单一编码+校验边界(Axon fe6060e7);backend 三表示收一(originCallerWire 副本、手工 b64、legacy metadata 双写全退役,EasyNet 7a0feed,−80 行)。**随手账**:Cli origin_caller.rs 的 from_metadata 遗留回退面自此全网死代码(唯一写者已撤),transport 释放后删 |
| client.go / node_mapper.go | 240+130 | A2A roster/NodeInfo 投影为 backend UI 形状 | 产品投影可留;A2A label v2 解析若是协议形状则上移 |

### No counterpart(非协议 fork,移出 F-015 范围)— 4 文件
bootstrap_self_identity.go(runtime admin RPC)· resolved_agents.go(纯合并工具)·
retired.go(P5 退役标记)· voice.go(占位)。这些是 backend-internal,不构成第二真源。

## 三、对 spec 的修正建议(待 CTO 确认后改 spec)

1. **T2.2a/b 改判「验收级收尾」**:守卫脚本(F-041 搭车项)落地 + 薄委托层抽查,无替换工程。
2. **T2.2c/d 重定义为「resolver answer codec 入 SDK」批**:namespace_resolve_answer +
   resolve_answer 的解码链(995 行)是剩余债主体;先在 Axon Go SDK 落 answer codec
   (与 Rust 侧同源生成或同测试向量钉死),backend 再切换。ability_descriptor_reader
   同车。e2e answer-sheet 回归把门不变。
3. **F-015 体量修正**:「7,765 行 fork」→ 真实剩余 ≈ 2,500 行 partially-covered,
   其中约 1,000 行(answer codec 家族)是协议形状必须上移,其余经评审可能合法留在产品层。
4. 模式判定:backend 的演进方向证明 Rule 1 是可执行的——RFC-005 PR 把 URA/envelope
   真源自然收敛到了 SDK;剩余债集中且有清晰处置路径。

## 三·五、answer codec 深挖(第二轮 + 第三轮自我勘误,2026-06-12)

> 第三轮勘误:第二轮把 backend 的 16 个 `*FromProto` 投影函数误读成「手写 JSON
> 解码器」,把 probe 函数的双 casing 探测键误读成「容忍性 Postel 病」——基于 grep
> 碎片构造叙事而未读实现体,与本盘点自立的「agent 论断必亲核」同罪。以下为
> 通读实现后的定稿。

1. **wire 真源已是 proto 且消费半边已达目标形**【第三轮亲核】:
   `DecodeNamespaceResolveAnswerJSON` 实现 = `protojson.Unmarshal{DiscardUnknown}
   → pb.ResolveAnswer → NamespaceResolveAnswerFromProto`(8 行);16 个内部函数
   全部是 proto→产品视图投影(合法且有测试);`answer_kind`/`answerKind` 双键
   仅出现在 maybeDecode 的「这是不是 answer」嗅探 probe 里,真正解析由 protojson
   按 proto3 JSON 规范处理。**消费半边无债。**
2. **真实剩余债 = 生产半边单边**【第二轮亲核,仍成立】:Cli route_resolver.rs 用
   `serde_json::json!` + `as_str_name()` 手拼 protojson-canonical 形(camelCase 键
   + 全名枚举,14 顶键与 proto 字段 1:1)。它能工作,但形状由手对齐——producer 漂移
   不会有编译期信号(F-038 同病,但只剩这一侧)。
3. **修复(收窄后)**:
   - 生产半边:route_resolver 构造真 `pb::ResolveAnswer` 经 pbjson 序列化
     (前置:Axon pb 启用 pbjson 生成——build 基建项);
   - 钉合:Fed-MVP schema_compat 落 answer 向量,生产侧 CI 吃(消费侧 protojson
     无漂移面,可选同吃作回归带)。

## 三·六、T2.2a 验收抽查完成(2026-06-12 第四轮,9/9 亲核)

| 文件 | 判定依据 |
|---|---|
| urns.go | 48 处 sdk. 委托,0 本地拼接(第一轮) |
| invoke_types.go | 纯 `type X = axonsdk.X` 别名(第一轮) |
| admission.go | :28 `axonsdk.ValidateSubjectURA` 显式委托;余为 SubjectViolation 包装+日志钩子(产品层) |
| delegation.go | Sign 在 SDK 别名类型上(:88 `proof.Sign`);本地仅产品策略校验(issuer==subject 等)+key-size 前置;ed25519 import 仅取 `PrivateKeySize` 常量 |
| session_authority.go | 同上(:65 `authority.Sign`,SDK 别名) |
| advertise.go | 8 处 SDK payload 构造器引用 |
| enums.go | SDK EnumNameLookup 回调模式 |
| invoke_request_builder.go | 经别名类型 + urns 门面(`PublishedAbilityURA`)构造,零本地形状 |
| noop.go | SDK Client 接口测试桩 |

**结论:零平行协议实现。** 配合 F-041 双阀落地(backend/Frontend
check-ura-construction.sh,三态验证,入 conformance CI),**T2.2a 整体完成**——
A 批从「替换工程」到「验收收尾」的改判被执行兑现。

## 四、风险与验证门

- ~~「SDK-covered」九文件的抽查纪律~~ **已完成**(§三·六,9/9)。
- answer codec 入 SDK 时必须与 Rust 实现共享测试向量(Fed-MVP schema_compat 模式),
  否则只是把第二真源从 backend 挪到 Go SDK。
