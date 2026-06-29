# Dispatch 帧载体归一 mini-RFC(T2.1-pre,v1,2026-06-12)

> spec T2.1 的施工硬前置(设计件四号)。状态:**已批准**——§6 三个开放问题经
> DEC-F004 全数裁决(A/A/A:hub 不入链、ReverseDispatch 保 16B nonce、
> 失败复用 axon.v1.Error;凉冰按 CTO 授权默认路径自决,CTO 可推翻)。
> schema gate 开放;施工见 §5(进行中,2d+)。
> 解决三个被钉死的债:F-004(第二 invocation 载体)、F-038(跨仓形状漂移)、
> F-040(backend 手抄帧);并为 T1.1(frame0 契约版本)与 T1.2(claimant 指纹)
> 提供 frame0 的一次性改动设计——**frame0 只动一次**。

## 0. 核心观察(为什么归一是「回家」而不是「迁移」)

`session.open` 的 bidi 协议**本来就有 canonical 载体**:

- `InvokeBidiUp` frame 0 = `EnvelopeOpen`,携带完整 AXIOM 七元组 `Envelope`
  (caller/callee/subject/nonce/causal_context/args_digest/ability)+ canonical args;
- `InvokeBidiDown` frame 0 = `InvocationReceipt`——「receipt IS the accept」。

SessionDispatch JSON 帧(Dispatch/BidiOpen/BidiInput/Result/Request/RequestResult)
寄生在 `BinaryChunk` 的字节载荷里,是绕开这套机制的第二载体。其并发症全部由此而来:

1. **七元组不完整**(boundary skill:receipt 链正确性 bug):Dispatch 帧只带
   ability/args/可选 callee+subject——**没有 nonce、没有 causal_context、没有
   args_digest 绑定**。跨设备调用在 hub 跳处脱离了 receipt 链。
2. **origin_caller claim 是补丁**:因为帧里没有真 Envelope,真实用户身份只能
   作为旁路 JSON claim 附带,target 设备再做一套专用验证——这套验证逻辑
   (origin_caller.rs)是 Envelope.caller_signature 验证的平行实现。
3. **Result 帧不带 receipt**:返回的是裸 bytes + Option<String> 错误——调用方
   拿不到 callee 签名的执行回执。

归一 = 让 dispatch 帧承载 `Envelope` + 让结果帧承载 `InvocationReceipt`。
不是发明新协议,是停止绕开已有协议。

## 1. proto schema(①,落 Axon 仓——boundary Rule 1)

新增 `axon.v1` 消息,作为 `InvokeBidiUp/Down` oneof 的新 payload 变体
(同层于 EnvelopeOpen/BinaryChunk/BidiControl;字段号续用 oneof 空位):

```proto
// Hub → device:在 target 本地运行一次完整 Invocation。
message DispatchCall {
  uint64   call_id  = 1;   // 会话内路由关联(非协议身份,不入签名)
  Envelope envelope = 2;   // 七元组,原 caller 的 envelope 原样转发
  bytes    args     = 3;   // canonical 编码,SHA-256 == envelope.args_digest
  ContentEnvelope content = 4;
  map<string,string> metadata = 5;   // 非 axiom 透传(§4.1.2 禁入签名字节)
  bool     open_bidi = 6;  // true = 原 BidiOpen 语义(长生命周期本地 bidi)
}

// Device → hub:结果 + 回执。
message DispatchResult {
  uint64            call_id  = 1;
  bytes             payload  = 2;
  bool              terminal = 3;
  InvocationReceipt receipt  = 4;  // terminal 帧必填:callee 签名的执行回执
  Error             failure  = 5;  // 类型化(替代 Option<String>+SessionFailure 双轨)
}

// Device → hub 反向(原 Request/RequestResult):同形,call_id 改 16 字节 nonce。
message ReverseDispatchCall   { bytes call_id = 1; Envelope envelope = 2; ... }
message ReverseDispatchResult { bytes call_id = 1; ... 同 DispatchResult ... }

// BidiInput 增量帧:保持 BinaryChunk(本就是裸字节通道,无归一必要)。
```

### 字段映射表(JSON → proto,零「待定」)

| SessionDispatch(JSON) | 归宿 | 备注 |
|---|---|---|
| Dispatch.call_id | DispatchCall.call_id | 不变 |
| Dispatch.ability | ~~envelope.ability~~ **request.function_name**(勘误 2:Envelope 不携带 ability——split-wire 惯例;帧改携完整 InvokeRequest) | canonical 名;owner-local 投影在 target 侧用现有 helper |
| Dispatch.callee_ura(可选) | envelope.callee(必填) | 可选→必填:七元组完整性 |
| Dispatch.subject_ura(可选) | envelope.subject(必填) | 同上;无明确 subject 时 = callee(AXIOM 允许) |
| Dispatch.args + args_content_envelope | args + content | bytes 直载,**base64 膨胀消亡** |
| Dispatch.metadata | metadata | 不变 |
| **Dispatch.origin_caller(claim 旁路)** | **envelope.caller + envelope.caller_signature** | 真实用户即 caller,浏览器签名即 caller_signature;origin_caller.rs 平行验证退役,统一走 admission verify |
| (缺失) nonce / causal_context / args_digest | envelope.* | 原 caller envelope 原样转发——hub 不铸造、不改写(改写会毁签名) |
| BidiOpen.* | DispatchCall{open_bidi=true} | 变体收敛 |
| BidiInput.{call_id,payload,eof} | BinaryChunk(stream 语义) | 本就是字节通道 |
| Result.{payload,terminal} | DispatchResult.{payload,terminal} | 不变 |
| Result.error + failure(双轨) | DispatchResult.failure(单轨类型化) | Option<String> 退役 |
| Result.request_id | (删除) | call_id 已是关联键;字段系历史冗余 |
| **(缺失)执行回执** | **DispatchResult.receipt** | receipt 链在 hub 跳处闭合——本 RFC 的最大正收益 |
| Request/RequestResult | ReverseDispatch* | 16 字节 nonce call_id 保持 |

## 2. frame0 一次性改动(②,T1.1 + T1.2 + T2.1 共车)

**Up(EnvelopeOpen 新增 field):**
```proto
message SessionOpenExt {
  uint32 contract_version   = 1;  // 本 RFC = 1;0/缺失 = 遗留 JSON 设备
  bytes  claimant_boot_nonce = 2; // T1.2:每进程启动随机 16B,槽位申领者指纹
}
// EnvelopeOpen { ...; SessionOpenExt session_ext = 8; }
// 勘误(DEC-F004 落地核对):草案曾写 field 7,但 content_envelope 已占 7;
// Axon 90841fed 落地为 field 8。
```

**Down(admission receipt 的 payload 扩展,InvocationReceipt.payload JSON 内):**
```json
{ "session_contract": { "version": 1, "dispatch_encoding": "proto",
    "hub_session_id": "...", "displaced_prior": true|false } }
```
- T1.1 收口:契约偏斜从「无差别 clean close」变为显式可观测
  (`version` 不匹配 → 设备端 ContractSkew 关闭类有了直接证据);
- T1.2 上车:hub 槽位存 `claimant_boot_nonce`,异指纹快速交替 →
  `claimant_conflict` op_event + 快速 re-admit 拒绝;`displaced_prior`
  让 displacement 在 device 侧第一帧即可见(CloseClass::DisplacedSuspect
  从指纹推断升级为协议事实)。

## 3. JSON 残留面清单(③)

| 面 | 处置 |
|---|---|
| `session.open` 业务帧(本 RFC 范围) | proto 化,JSON 形状删除 |
| control.sock(boot/status/诊断) | 保持 JSON——已有书面不变量「Nothing on control.sock dispatches product abilities」(easynet-daemon.rs),非 invocation 载体 |
| 会话 keepalive/控制 | 已是 BidiControl proto,无变化 |
| 跨仓 fixture(Fed-MVP session_dispatch.json) | 最后一次随新帧重生为 proto schema_compat 向量,JSON 基线退役 |

## 4. 滚动升级四象限(④,双读单写一个版本)

读侧:双方都同时接受 proto 变体与 BinaryChunk-JSON(一个版本期)。写侧按协商:

| | 新 hub | 旧 hub |
|---|---|---|
| **新 device** | frame0 ext(v1)↔ receipt session_contract(v1):**双向 proto**;JSON 写路径不触发 | device 发 ext,旧 hub 不识别(proto 未知字段安全忽略);receipt 无 session_contract → **device 写 JSON**,读两者 |
| **旧 device** | 无 ext → hub 按 v0 处理:**hub 写 JSON**,读两者 | 现状(全 JSON) |

一个发布周期后:**写读两侧同时删除 JSON 形状**(干净切割——窗口是发布机制,
不是长期兼容层;符合「不兼容旧方案」的总指令)。删除项清单:SessionDispatch
enum 全体、origin_caller.rs 平行验证、backend invoke_remote.go 手抄 struct
(F-040/F-044 随之消亡——backend 改提交完整 InvokeRequest,T2.1b)。

## 5. 施工序(T2.1 的步骤分解)

1. Axon:proto 消息 + oneof 变体 + SessionOpenExt(纯增量,旧端忽略未知字段);
2. Cli hub 侧:读双轨(proto 优先);frame0 协商落 PresenceSlot(含 T1.2 指纹存储);
3. Cli device 侧:读双轨 + 按协商写 proto;DispatchResult 开始携带本地执行 receipt;
4. backend(T2.1b):invoke_remote 包装退役,直接 InvokeRequest;
5. 一个版本后:JSON 写读删除 + Fed-MVP 基线切 proto 向量 + F-038 类漂移结构性免疫。

验收(spec T2.1 行不变):基准对比落档(T0.4 帧基准做 before/after)、
新旧帧互通一个版本(四象限各一条集成测试)、357+ transport 测试迁移、
receipt 链端到端测试(跨设备调用的 receipt 可验签、causal 链可追)。

## 6. 评审点(已裁决——DEC-F004,2026-06-12)

1. **hub 不入链(A)**:`DispatchCall.envelope` 原样转发不重签;转发事实记 hub
   本地账本(与 DEC-F020 非权威读模型一致)。签名保真是七元组保真的底线。
2. **ReverseDispatch 保持 16B nonce(A)**:跨 hub 边界无碰撞面;u64 计数器的
   重连重置语义复杂度不值 8 字节。
3. **失败复用 axon.v1.Error + reason 枚举(A)**:单一错误本体——本 RFC 正在消灭
   双轨,不再造第二错误本体。
