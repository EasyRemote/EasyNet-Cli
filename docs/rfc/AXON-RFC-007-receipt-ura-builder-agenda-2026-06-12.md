# RFC-007 议程件:receipt body URA builder + device-agent 资源 owner 形状(T5.11② / F-042②,2026-06-12)

> 状态:**proposed**(两张决策卡待 CTO 拍板)。本文是 RFC-007(invocation-unity)
> 的种子议程——RFC-007/008 目前无草案文件,本件为首个可引用条目。
> 关系:RFC-006 定义 canonical/operational receipt 语义(不动);RFC-008(ability
> home 目录)未起草,本件议题一与其物理投影解耦(见裁决理由)。
> 全部形状已对照 Axon `core/ura-rs/src/lib.rs` 磁盘亲核(2026-06-12)。

---

## 议题一:receipt body URA 正式 builder

### 现状(四形状盘点,T5.11① 后)

| 形状 | 来源 | 现状 |
|---|---|---|
| `resource/<owner>/invocation/<id>/history` | `Ura::invocation_history_resource`(ura-rs,唯一生产 builder) | ✅ 在产,但指向 history ledger 投影,**非 receipt body** |
| `resource/<owner>.invocations/<id>` | ledger.rs 测试约定(borrowed) | T5.11① 后全部带 borrowed 标注;Hub 的 `invocation_record_owner_id` 映射也产 `hub.invocations` 后缀 |
| `<host-kind>/<host-id>/invocation/<id>` | DeviceDetailPage.test.tsx fixture | 仅前端 fixture,无 builder |
| `invocation/<id>/receipt/<n>` | 非法(`invocation/` 非 role) | T5.11① 已清零 + grep 闸 |

### 候选与裁决

- **(A) 推荐:`resource/<owner>/invocation/<invocation-id>/receipt`**
  —— history builder 的同族尾段。新 builder:
  ```rust
  // ura-rs,与 invocation_history_resource 并排
  pub fn invocation_receipt_resource(realm: &str, owner_id: &str, invocation_id: &str) -> Self {
      Self::resource_dot(realm, owner_id, &format!("invocation/{invocation_id}/receipt"))
  }
  // 高层入口与 invocation_record_ura 同构:owner 段经 invocation_record_owner_id 派生
  pub fn invocation_receipt_ura(owner_ura: &str, invocation_id: &str) -> Option<String>
  ```
  理由:① receipt 身份 = owner + invocation_id,**与存储位置解耦**(RFC-008 的
  runs/<run-id>/receipt.json 是物理投影,寻址不应耦合目录布局——run 工作区可 GC/迁移,
  receipt 地址必须不变);② 与唯一在产 builder 同族,owner 派生逻辑
  (`invocation_record_owner_id`)零新增;③ 今日语法即可 round-trip `parse_ura`,
  无解析器改动。
- (B) 否:把 `.invocations` 测试约定转正——magic dotted 后缀无 namespace 治理,
  且与「invocations 集合条目」混淆 receipt body 本体;borrowed 标注的存在本身
  就是它非 canonical 的证据。
- (C) 否(现在):ability-home 锚定(`<ability-home>/runs/<run-id>/receipt.json`)
  ——被 RFC-008 未批阻塞,且把身份耦合进存储布局;留作 RFC-008 的物理投影条目,
  与 (A) 互为投影(同一事实,两个地址面,receipt.json 内自带 cross-ref)。

**开放点(默认从简)**:operational receipt 的 `<n>` 索引(死形状遗留概念)——
RFC-006 的 canonical receipt 每 transition 恰一份,`/receipt` 尾段指它;
operational 索引**暂不立形状**,等到出现真实消费者再入议程(不做投机性灵活度)。

### ③ 执行路径(builder 落地后,原 T5.11③)

1. 全仓 `receipt_ura` 字段反序列化处过 `parse_ura`(Rust)/`parseURA`(TS)——
   非法形状解析期拒绝,不再是裸字符串;
2. borrowed 标注的 ledger.rs/test_support 形状迁移到 (A),borrowed 注释退役;
3. T5.11① 的 grep 闸升级:从「拒非法 role」升为「receipt_ura 只认 (A) 形状」。

---

## 议题二:device-agent 的资源 owner 段

### 现状(比 F-047 入册时更尖锐,行号亲核)

ura-rs 三个 owner 派生点全部走 `agent_ids()`,而 device-agent 只有
`device_agent_ids()`(`agent_ids()` 返回 None)——即 **device-sponsored System
Agent 今天静默地不能拥有协议资源、不能立案 invocation record**:

| 派生点 | 行为 |
|---|---|
| `protocol_resource_owner_id_from_ura`(:1050)| Agent 臂 `agent_ids()?` → device-agent 得 None,不能为 payload 等协议资源 owner |
| `invocation_record_owner_id`(:1119)| 同上 → device-agent 的调用无 history/receipt 归档地址 |
| `agent_principal_id_from_ura`(:1040)| 同上 → principal 解析对 device-agent 失败 |

silent-None 符合「flag 而非 extrapolate」纪律,但形状缺口使 System Agent 的
receipt 链在源头断裂——与议题一同批闭合最经济。

### 候选与裁决

- **(A) 推荐:owner 段 = `agent.device.<device-id>.<agent-id>`**
  —— 既有 kind-tagged 文法的机械延伸(`user.<id>` / `device.<id>` /
  `agent.<user>.<agent>` / `hub`),`device.` 中缀与已批 agent URA 文法
  (`agent/device.<device-id>.<agent-id>`,ura-rs :383)逐字对应。
  三个派生点的 Agent 臂改为:`agent_ids()` 与 `device_agent_ids()` 二选一命中。
  **§3.1.2 警句随行**:device 是 sponsor 非 principal——该 owner 段的问责
  仍经配对 principal 解析;`agent_principal_id_from_ura` 对 device-agent
  返回什么(配对 principal?还是 None + 显式错误?)是决策卡 2 的子问题。
- (B) 否:device 锚定(`resource/<device-id>/agent/<agent-id>/...`)——把 agent
  资源记到 device 名下,违反「Agent owns Ability/资源随 owner」读法,且与
  DEC-F048 的 sponsor 语义相抵。

---

## 镜像与验收(实施 TODO 的门,builder 批准后开工)

| 面 | 动作 |
|---|---|
| Axon ura-rs | 两 builder + Agent 臂双访问器;round-trip 测试(parse → kind/owner/path 取回) |
| Axon client-sdk / sdk wrappers | 门面转发(与 invocation_history_resource 同模式) |
| Axon Go SDK | 对等函数 + 与 Rust 同测试向量 |
| Frontend parseURA | 镜像纪律:语法无新增(均为既有 resource 文法),仅补 fixture |
| Cli/backend | 消费点替换 borrowed 形状;②③ 联动收口 |

验收:两 builder 入 SDK 且 9 处(7 borrowed + 2 前端 fixture)迁移零残留;
device-agent 调用可立案(invocation_record_ura 非 None);③ 解析期拒绝测试。

---

## 决策卡(CTO)

1. **receipt body 形状**:批 (A) `…/invocation/<id>/receipt`?(B/C 已论否,
   推荐即默认,签字即开工——实施挂 Axon 仓,S–M 级)
2. **device-agent owner 段**:批 (A) `agent.device.<device-id>.<agent-id>`?
   子问题:`agent_principal_id_from_ura(device-agent)` 返回配对 principal
   (需查 pairing 面)还是显式 None(调用方自行解析)?**默认建议:显式 None +
   文档指向 pairing 解析**——principal 查询是 daemon 策略面,不该塞进纯文法库
   (boundary Rule 1 的反向应用:Axon 不懂 EasyNet 配对)。
