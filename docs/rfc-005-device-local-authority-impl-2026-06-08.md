# RFC-005 device-local authority — 实现记录 2026-06-08

> 修复 `agent.start` → `ROUTE_NEGATIVE/NODATA` 与 Abilities 页 `0 surfaces`。
> 面向终态、不兼容旧方案。架构判断见
> `rfc-005-device-local-authority-architecture-2026-06-08.md`。

## 根因(一句话)

resolver 把 hub 的 `AbilityCatalogStore`(rendezvous 投影缓存)当成 device-owned
ability 是否存在的唯一判据。RFC-005 §10.1/§4/D44/D105:device-local runtime 才是权威,
hub projection 只是签名的发现投影。`agent.start` 一直注册在本地 runtime,设备解析自己
本不该问 hub 缓存。

## 实现(纯 CLI daemon runtime 层,不碰 Axon/backend/URA 语法)

### 1. `DeviceLocalAuthority` 抽象(OOP 接缝)
`route_resolver.rs`:
- `trait DeviceLocalAuthority` — 同步谓词:`(device_ura, public_name) → Option<DeviceLocalAbility>`。
- `LocalRuntimeAuthoritySnapshot` — 生产实现,`capture(runtime).await` 把 `LocalRuntime::list_abilities()`
  快照成 dispatch-key 集合;成员判定用 `local_dispatch_ability_key`(与真实执行器同一映射,绝不漂移)。
- 注入式依赖:`DaemonRouteResolver::with_device_local_authority(device_ura, Box<dyn ..>)`,
  按值持有(self-contained,规避借用跨栈帧)。

### 2. resolver 分流(D105)
`resolve_route`:
- owner == 本机 device → `resolve_route_from_device_local`:从本地 runtime 证 ABILITY+ROUTE,
  **完全绕过 catalog**,`release_profile = AuthoritativeLocal`,`next_hop = LocalDeviceAbility`。
  不在 runtime → 典型 `NODATA`;device 不在 presence → `NOROUTE`。
- 其他 owner → `resolve_route_from_projection`(原投影路径),但 profile 不再硬编码。

### 3. profile 模型(经一次过度修正后的正解)
**第一版我把投影路径默认设成 `ShadowRead`(只有执行宿主==本机才 AuthoritativeLocal),这是错的。**
真机暴露:backend 经 **hub daemon** 走 `runtime.invoke_remote` 解析 `device.f9904e66.terminal.list`,
hub 的 `daemon_ura=.../hub` ≠ 设备 URA → 走投影 → 被我标成 ShadowRead → `ROUTE_PROFILE_BLOCKED`。
但 hub 对"这条 device-owned ability 在哪台设备、转发过去"**本就是权威**——这不是 shadow read。

**正解**:投影路径在 placement gate(离线→NOROUTE)之后**一律 `AuthoritativeLocal`**。
"选择把在线 owner 的 ability 派到哪/转发给哪台设备"正是单 hub resolver 的权威职责。
跨 realm 在建路由前已被 `Delegation/PeerHub` 拦截,不会走到这里。`ShadowRead` 不再由本实现产出。

device-local 分支保留,价值收窄为唯一关键点:**设备解析自己的 ability 时无需 catalog 行**
(就是 `agent.start` 空 catalog 那个生产 bug)。两条路径现都产 AuthoritativeLocal,语义自洽:
- 设备解析自己 → device-local(免 catalog)→ AuthoritativeLocal
- hub/他节点解析在线 owner → 投影 → AuthoritativeLocal(权威的路由/转发决定)
- 离线 → NOROUTE;跨 realm → Delegation

### 4. 调用点
`daemon_invocation_service.rs`:`daemon_route_resolver()` 改 `async`,内部一次性快照本地 runtime
并注入 authority;9 处调用点 + 3 个 helper fn(`resolve_forward_invoke_route`/
`resolve_cross_realm_forward_delegation`/`dispatch_namespace_resolve`)相应 `async`/`.await`。

## 测试(全绿)

`route_resolver.rs` 16/16 通过,新增/改写:
- `device_owns_control_ability_via_local_authority_without_any_projection` —
  **正是这次 bug**:catalog 空,`agent.start` 仍解析为 FinalRoute/AuthoritativeLocal。
- `device_ability_not_registered_in_runtime_resolves_nodata` — 未注册 → NODATA。
- `device_offline_resolves_noroute_even_with_local_authority` — 离线 → NOROUTE。
- `hosted_agent_implemented_on_this_device_is_authoritative_local` — 本机托管 agent → AuthoritativeLocal。
- `projection_only_route_for_other_owner_is_shadow_read` — 他人投影 → ShadowRead。
- 改写两处旧测试,使其注入 device-local authority(匹配生产),并删除随之死掉的
  `publish_device_profile` helper。

## 验证命令(注意 axon-pb 双构建)
- `cargo test --features axon-pb --lib route_resolver::` ✅ 16/16
- `cargo test --features axon-pb --lib daemon_invocation`(运行中)
- 默认无 feature 构建会把 `invocation_transport` 整个 cfg 掉,**必须带 `--features axon-pb`**。

## 拓扑要点(真机调试得到)
本地是**两个 daemon**:device daemon(`~/.easynet`,mode=device,`device/f9904e66`)+
hub daemon(`EasyNet/.dev-hub-home/.easynet`,mode=hub,听 50443)。backend(caller=`.../hub`)
经 hub 走 `runtime.invoke_remote` 转发到 device。所以 `terminal.list` 的 resolve **发生在 hub**,
hub 的 daemon_ura 是 hub 不是 device —— 这正是第一版 ShadowRead 误判的来源。修正后 hub 投影路径
产 AuthoritativeLocal、正常转发给 device。

## 验证状态
- ✅ `cargo test --features axon-pb --lib route_resolver::` 16/16
- ✅ `cargo test --features axon-pb --lib daemon_invocation` 138/138
  (4 个旧测试按新模型更新:device 自有 ability 的 runtime-miss 现在是 resolve 期 NODATA,
   不再是 executor 期 NotFound——更早更精确)
- ✅ `cargo clippy --features axon-pb --lib` 我的文件零告警
- ✅ `cargo build --features axon-pb --bin easynet --bin easynet-daemon` 干净
- 新二进制:`target/debug/easynet-daemon`(00:43)

## 后端日志暴露的两个独立真因(已修)

后端日志把问题收敛为 2 类根因,均已修复:

### 根因 A:heartbeat 从不续租 → device ability 整批 NODATA(治本)
`handle_heartbeat` 是空壳:`_request` 带下划线丢弃 `refresh_owner_uras`,只读 presence,
**从不碰 catalog 租约**。租约一过期,`catalog.get_at()` 过滤掉所有 device-owned ability →
`terminal.list/create`、`agent.list`、`skill.list` 全 `NODATA: owner is online but does not publish`。
完美解释"刚注册稳定、一段时间后失效"。
- 修:`AbilityCatalogStore::refresh_lease`(只延 `lease_expires_unix_ms`,不碰 summaries/revision/
  digest,符合 spec "heartbeat must not mutate projection contents or digest";过期未驱逐的行可被复活);
  `HeartbeatRequest.refresh_owner_uras` 字段;`handle_heartbeat` 接 catalog+now 续租;
  `dispatch_federation_heartbeat` 传 `&self.ability_catalog`。device 侧 `heartbeat()` 本就发
  `refresh_owner_uras`,链路闭合。TTL 用 `owner_projection::lease_expiry_from_now`(与发布同源,不漂移)。
- 测试:`handle_heartbeat_renews_owner_projection_lease`(精确复现 过期→心跳→复活,且 revision/digest 不变)
  + skip-unknown-owner。

### 根因 B:runtime.bootstrap_self_identity NXDOMAIN(hub 身份注册失败)
backend(身份=hub)在 device daemon 上调 `runtime.bootstrap_self_identity`,daemon unary catch-all
把它丢进 owner-resolve,owner=hub 不在 device presence → `NXDOMAIN owner is not online`。
`runtime.*` 是节点内部 admin 握手(像 `legacy self alias.*`),不该走 owner 解析。
- 修:`is_runtime_admin_ability`(`runtime.` 前缀)+ `dispatch_runtime_admin_ability`:直接在 LocalRuntime
  按 ability 名 dispatch,绕过 owner-presence resolve,由 SDK admin surface 自己做权限校验。
- 测试:success 测试去掉 `publish_test_route` 仍通过(证明绕过 owner 解析);no-admin → `NotFound: not installed`。
- 非致命(backend 注释 "SHOULD log and continue"),但消掉了刺眼 ERROR 且修对了语义。

## 全量验证(本轮新增后)
- route_resolver 16/16 · daemon_invocation 138/138 · heartbeat 3/3 · ability_catalog_store 8/8
- clippy 我的文件零告警(owner_projection.rs:407 是既有告警,非本次改动)
- `cargo build --features axon-pb --bin easynet --bin easynet-daemon` 干净;新二进制 01:17

## 剩余(需你的环境)
- [ ] **重启 device + hub 两个 daemon** 用新二进制,跑 `easynet agent add anthropic` 端到端确认。
      hub daemon 也必须换新二进制(ShadowRead 误判就在 hub 侧)。
- [ ] device 自发布 catalog 仍 best-effort(对外 rendezvous 用),非本机可达性前提 —— 已不阻塞。
