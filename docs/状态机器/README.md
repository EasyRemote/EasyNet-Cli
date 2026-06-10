# EasyNet 状态机器文档索引

日期: 2026-06-09  
范围: 用户在前端拿到 pairing token 后, 执行 `easynet join`, CLI daemon 接入 Hub/Axon runtime, Backend/Frontend 显示 connected, ability resolve/invoke/read-model/failure, 以及 `runtime stop`、`self uninstall/leave` 的完整状态机器。

## 文件

| 文件 | 用途 |
| --- | --- |
| [current-code-audit-and-corrected-state-machines.md](current-code-audit-and-corrected-state-machines.md) | Canonical 审计版 Markdown。它区分当前代码事实、RFC-005 目标契约和未收敛缺口, 是后续实现依据。 |
| [current-code-audit-and-corrected-state-machines.zh-CN.tex](current-code-audit-and-corrected-state-machines.zh-CN.tex) | Canonical 审计版中文 TeX, 使用更保守的长表格与断行样式避免 PDF 重叠。 |
| [current-code-audit-and-corrected-state-machines.zh-CN.pdf](current-code-audit-and-corrected-state-machines.zh-CN.pdf) | 由 canonical TeX 编译出的 PDF。 |
| [easynet-state-machines.md](easynet-state-machines.md) | 早期 clean-target 草稿。保留作历史参考, 但不能单独作为当前实现依据。 |
| [easynet-state-machines.zh-CN.tex](easynet-state-machines.zh-CN.tex) | 早期 clean-target TeX 草稿。 |
| [easynet-state-machines.zh-CN.pdf](easynet-state-machines.zh-CN.pdf) | 早期 clean-target PDF 草稿。 |
| [build.sh](build.sh) | 可复现构建脚本, 自动查找 `xelatex` 或 `/opt/homebrew/bin/xelatex`, 编译状态机器目录下所有 TeX。 |

## 已核对来源

- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/docs/join-to-connected-state-machine-2026-06-08.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/docs/easynet-backend-boundary-audit-2026-06-08.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/document/rfcs/005-ura-namespace-resolution-dns-plan.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/pr/2026-06-06-ura-namespace-resolution/00-intent.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/pr/2026-06-06-ura-namespace-resolution/01-invariants.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/pr/2026-06-06-ura-namespace-resolution/02-architecture.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/pr/2026-06-06-ura-namespace-resolution/03-cross-repo-plan.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/pr/2026-06-06-ura-namespace-resolution/04-execution-checklist.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/pr/2026-06-06-ura-namespace-resolution/05-verification.md`
- `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/pr/2026-06-06-ura-namespace-resolution/06-decisions-log.md`
- 用户提供的 Hub 日志与 CLI 输出: `terminal.list` PresenceRegistry miss, `pages.list` agent not advertised, `invocations/history/list` missing canonical route, `join` daemon boot preflight failure, `meta.list_resources` no canonical ability route。

## 构建

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/docs/状态机器
./build.sh
```

构建要求:

- macOS 上优先使用 `/opt/homebrew/bin/xelatex`。
- TeX 使用 `fontspec`/`xeCJK`, 默认字体优先 `PingFang SC`, 不存在时回退 `Songti SC`。
- Canonical PDF 是从 `current-code-audit-and-corrected-state-machines.zh-CN.tex` 编译产生的, 不是 Markdown 打印件。

## 当前结论

这不是单一 HTTP 页面或单一 ability 的 bug, 而是 RFC-005 迁移未完全收敛造成的系统性状态机器缺口:

1. `join` 的 credential accepted 与 runtime connected 曾被产品语义混在一起。
2. Backend/Frontend 曾在多处自行推断 canonical ability route。
3. Resolver negative / hub unavailable / daemon unavailable 曾被压成空列表、普通 400、普通 500 或人类字符串。
4. Ability catalog/read model 与 invocation prepare/submit 没有完全统一到 Axon `ResolveAnswer.FinalRoute`。
5. Terminal/file/browser/download 等 producer 必须把失败统一写入 `InvocationReceipt.failure` 或 `X-EasyNet-Failure` trailer/control frame。

干净目标是: 所有用户可见状态都有稳定状态码; 所有失败都能定位到具体 transition; 所有 invoke 都必须先 resolve, 再按 resolver-selected next hop dispatch; stop/uninstall 这类 operator lifecycle 也必须和 join/invoke 一样可追踪。
