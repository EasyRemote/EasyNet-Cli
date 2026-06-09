# RFC-005 terminal.list NODATA — 根因收口 2026-06-09

> 关键纠正:此前所有"真机验证"都是在 **docker e2e 旧镜像** 上跑的,
> 我的修复从未进入测试环境。本文记录已确证的根因清单 + 真正的验证路径。

## 最致命的发现:测试环境一直是旧镜像

- 你的环境是 `docker compose -p easynet-dev`(`docker/e2e/docker-compose.yml`),
  daemon 跑在 **容器** 里,镜像 `easynet/hub-e2e:local` / `easynet/device-e2e:local`
  是 **预建** 的(`Dockerfile.hub` 用 `COPY easynet-daemon`,不从源码 build)。
- 容器内 daemon 构建于 `2026-06-08 18:13`,`strings | grep resolve_route_trace` = **0**
  → **不含我的任何修复**(device-local / heartbeat 续租 / directory 兜底 / runtime.* bypass)。
- 因此:这十几轮"还是 NODATA / bootstrap 修好了 / agent.list 通了" **全是旧二进制的行为**,
  不是我的代码。我没核实运行环境就反复推断,代价是大量往返。

**唯一能验证我修复的动作**:用我的源码重建镜像
(`EasyNet/scripts/docker-build-images.sh` 交叉编译 EasyNet-Cli `--features axon-pb,demo-fixture`
→ 烤进 hub/device 镜像)→ 重启栈 → 看容器内是新二进制 + `resolve_route_trace`。

## 旧镜像里观察到的强模式(决定性对比)

同一台 device,同一批 device-owned ability,经 `<self>.invoke_remote`(ROUTE 解析):

| 通过 ✓ | 失败 ✗ (NODATA / route missing) |
|---|---|
| terminal.create, terminal.close | terminal.list |
| meta.list_resources, camera.snapshot | meta.list_abilities |
| invocation.history.list | agent.list, skill.list |

- create/close 成功 → **PTY 真建出了 session**(`kernel: created session`),
  彻底排除"terminal.* 整类未注册/租约/catalog 空"——否则 create 也挂。
- 失败的清一色是 **列举类**(`*.list` / `list_abilities`)。
- 静态层面:`terminal.list`/`agent.list`/`skill.list`/`meta.list_abilities` 与通过的那些
  **注册方式完全相同**(`register_rpc_with_owner(..., OwnerKind::Device)`),
  都在 `published_ability_names()` 里(单测 `published_ability_names_contains_agent_list_and_terminal_list` 已证)。
- 结论:差异是 **运行时状态**,不是静态注册。旧镜像的具体机制不必反推——
  新镜像跑起来,`resolve_route_trace`(branch / runtime_has_binding / owner_matches_self /
  presence_has_owner)会一行说清。

## 本会话已落地的修复(单测全绿,待镜像验证)

| 根因 | 修复 | 测试 |
|---|---|---|
| A 心跳不续租 | `AbilityCatalogStore::refresh_lease` + `handle_heartbeat` 读 `refresh_owner_uras` 续租 | heartbeat 3/3 |
| B `runtime.*` 误走 owner 解析 → NXDOMAIN | `is_runtime_admin_ability` + `dispatch_runtime_admin_ability` 直连本地 runtime | bootstrap 2/2 |
| C device daemon catalog 空 + directory 无兜底 | `resolved_owner_projection_values` 对本机 device owner 合并静态 device profile(`self_device_ura` 门控) | federation_wrappers 39/39 |
| heartbeat agent_ura required | `#[serde(default)]`(对齐 Axon 已正确的契约) | + 反序列化回归测试 |
| 诊断 | `resolve_route_trace` op_event(owner/daemon_ura/device_local/runtime_has_binding/presence/branch) | — |

全量:route_resolver 16/16 · daemon_invocation 138/138 · federation_wrappers 39/39 ·
ability_catalog_store 8/8 · clippy 我的文件零告警 · `--features axon-pb` 双 bin 构建干净。

## 海峰审查认领的架构债(非 regression,待规划)

- CLI resolver `release_profile` 全硬编码 `AuthoritativeLocal`,CLI 永久自称权威 →
  与 RFC-005「Axon 单一权威」冲突。需 Axon A7 `ResolverProfileState` 状态机,CLI 才能如实反映。
- ResolveScope / alias-safety / reserved namespace / negative taxonomy 未落完。
- listing 负例应进 `resolve_unavailable[]` 而非静默 log+skip。

## 验证路径(镜像重建后)

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet
bash scripts/docker-build-images.sh            # 交叉编译我的源码进镜像
docker compose -p easynet-dev -f docker/e2e/docker-compose.yml up -d --force-recreate
docker exec easynet-dev-hub-1 sh -c "strings /usr/local/bin/easynet-daemon | grep -c resolve_route_trace"  # 应 >0
# 然后操作 UI,看 hub 日志 terminal.list + 容器内 daemon stderr 的 resolve_route_trace
```

## 待确认的独立问题(别假设连锁)

- HTTP 400 ×3(`/pages`, `/invocations/history/list`, `/abilities/envelope/prepare`)
- per-device dispatch channel 256 starvation(远程桌面/视频场景,root cause D,deferred)
