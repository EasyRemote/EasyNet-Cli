# mission run.json 消费面盘点(T5.3 前置设计件,2026-06-12)

> spec T5.3 前置:「run.json 消费面盘点——跨仓 grep,消费点清单落档后才许改 wire」。
> 方法:三仓全量 grep(run.json / MissionRunMeta / mission_runs / 五个 status 字面量)。

## 结论:wire 风险塌缩

1. **跨仓消费为零**【实测】:EasyNet backend(Go)、Frontend(TS)、scripts/、tests/
   对 run.json 与 mission status 字面量全部零命中。审计期担心的「跨仓读 mission run
   的消费面」不存在——T5.3 的 serde 兼容约束塌缩为单条:**磁盘上既有 run 目录的
   历史 meta 仍可读**(且按总指令「不兼容旧方案」,此条也可降级为:旧 run 读不出
   status 时显示 unknown,不阻塞)。
2. **Cli 侧单一所有权**:facade/cli/mission_runs.rs 拥有全部:
   - 结构定义 :118(`pub status: String // "ok"|"error"|"partial"|"running"|"cancelled"`);
   - 写点 :245(cancelled)、:515(partial 投影)、创建期初值;
   - **pid 活性判定 :174**(`path.join("pid").exists()` —— F-022 的「磁盘文件即状态机」本体);
   - pid 写 :50 / 清 :74(注释自述 presence == in-flight)。
   其余命中(eal/ir、parser、dispatch.rs:642 的 exit_status、mission_ability.rs 文档行)
   均非 run.json status 的读写方。
3. **enum 化的真实工作量**:`MissionRunStatus{Running, Ok, Partial, Error, Cancelled}`
   + `#[serde(rename_all = "lowercase")]` 即可原位读旧值(旧字面量恰为小写)——
   serde 迁移近乎免费。**T5.3 的实际难点只剩 liveness 半边**:
   - pid 文件 → 心跳时间戳(运行循环周期 touch);
   - 层向注意:心跳写手必须由 run 持有者(mission 运行循环)驱动,不可让 eal
     反向依赖 facade——建议 MissionRunDir 暴露 `heartbeat()` 方法,以句柄形式
     传入运行闭包(与 MissionContextGuard 同途),eal 不知道文件布局;
   - 读侧:心跳超龄(> 3× 周期)→ 投影 `Error{interrupted}`,假 running 消亡。

## 对 spec 的修正建议
T5.3 规模可从 M 降至 S+(单文件 + 心跳句柄穿线);「旧 run.json 可读」验收保留
(lowercase rename 免费满足),「跨仓消费面无遗漏」验收由本盘点闭合。
