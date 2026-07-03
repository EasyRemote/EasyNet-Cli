# EasyNet 杀手 Demo 速查 — 中文版

**位置**: `EasyNet-Cli/examples/public-routes-e2e/`
**对应代码**: 分支 `easynet-page` (commits `b6cd8fe`, `7041400`, `8912978`, `6c5d642`)
**对应 RFC**: [RFC-006-B v0.6](../docs/rfc/AXON-RFC-006-B-easynet-webapp.tex) (Pages) + [RFC-006-C v0.1](../docs/rfc/AXON-RFC-006-C-openai-compat.tex) (LLM-API compat)
**完整功能文档**: [`docs/PAGES_AND_LLM_API.md`](../docs/PAGES_AND_LLM_API.md)

> 这份文档讲 5 个**真实可跑通**的 demo 故事。每个 demo 都是一个独立 shell 脚本，silan 在 Mac 上可以直接 `./examples/public-routes-e2e/d{1..5}-*.sh` 跑，不需要任何额外环境。每个脚本之间用 `pause` 分步——按回车继续，方便边看边讲。
>
> 加 `EASYNET_DEMO_NONINTERACTIVE=1` 可以一气跑完，用于回归测试或截屏。

---

## 0 — 一次性准备

EasyNet daemon 需要在跑：

```bash
# 1. 编译 (一次性)
cd ~/Documents/Github/EasyNet-Cli
cargo build --features axon-pb --bin easynet --bin easynet-daemon

# 2. 让 PATH 上的 easynet 指向开发版二进制
ln -sf $PWD/target/debug/easynet ~/.local/bin/easynet

# 3. 启动 daemon (后台)
EASYNET_PAGES_PORT=8787 EASYNET_PAGES_USER=alice EASYNET_PAGES_REALM=easynet.run \
    target/debug/easynet-daemon > /tmp/easynet-daemon.log 2>&1 &

# 4. 验证
lsof -iTCP:8787 -sTCP:LISTEN     # 应该看到 easynet-daemon
easynet pages list               # No published projects.
```

每个 demo 脚本都会在开头跑 `ensure_daemon`——daemon 没起就直接报错退出，不会跑半截。

---

## 一、Demo #1 — 一条命令把文件夹变网站

**脚本**: `examples/public-routes-e2e/d1-static.sh`

### 故事

silan 在 Mac 上写了几个 HTML/CSS。想给团队一个 URL 让他们浏览器打开看。**不想配 nginx，不想 push 到 GitHub Pages，不想注册 Vercel**。一条命令搞定，URL 立刻能访问，背后还有内核级沙盒兜底安全。

### 跑

```bash
./examples/public-routes-e2e/d1-static.sh
```

### 关键点

1. 写一个 `~/.easynet/web-apps/d1-snapshot/index.html` + `style.css`
2. 跑 `easynet pages create d1-snapshot --folder ...`
3. **5 秒之内**浏览器能打开 `http://d1-snapshot.alice.pages.localhost:8787/`
4. **真沙盒**：脚本会做一次 `GET /../../etc/passwd`，返回 404，因为内核 `openat2(2)` 直接拒绝越界——daemon 根本没机会读 `/etc/passwd`
5. 一个 publish = 一个 resource URA：`easynet:///r/easynet.run/resource/alice.d1-snapshot/`

### silan 看的故事

> **传统**: 静态网站 → 配 nginx + 端口 + reverse proxy + cert + DNS + ...
>
> **EasyNet**: `easynet pages create` 一行，URL 即出。背后是 `<user>.<project>.page.fetch` ability + 内核沙盒，每次请求都进 receipt 链，整链可审计。

### 期望输出

```
━━ 1. Compose a 2-file site at /Users/.../web-apps/d1-snapshot
  ✓ wrote index.html (487 bytes), style.css (316 bytes)

━━ 2. Publish (mints a resource URA, opens folder fd, registers fetch ability)
$ easynet pages create d1-snapshot --folder ...
Published.
  project_uri:  easynet:///r/easynet.run/resource/alice.d1-snapshot/
  url_root:     http://d1-snapshot.alice.pages.localhost:8787/

━━ 3. curl /index.html — real bytes from disk through sandbox
  HTTP 200  type=text/html; charset=utf-8  bytes=487

━━ 4. Try the classic path-traversal attack — kernel refuses
    attempting: GET /../../etc/passwd
  ✓ blocked at 404 — daemon's openat2 returned EXDEV before any read syscall

━━ 5. List + show + open in browser
```

---

## 二、Demo #2 — Claude Code 自己写带后端的网站

**脚本**: `examples/public-routes-e2e/d2-agent-fullstack.sh`

### 故事

silan 给 web-builder agent 一句话："给我做一个食谱网站，前端能加新食谱，刷新后还在"。**silan 一个文件都没碰**，**凉冰一个文件都没碰**。Agent 自己读它的 `easynet-pages-author` + `easynet-ability-author` skill，自己写前端、写真后端 ability、自己 deploy、自己用 curl 验证持久化。

### 跑

```bash
# 默认是食谱网站；可以通过 PROMPT="..." 覆盖
./examples/public-routes-e2e/d2-agent-fullstack.sh

# 或自定义提示
PROMPT="做一个个人 blog，文章持久化，能列出所有文章" PROJECT=blog \
    ./examples/public-routes-e2e/d2-agent-fullstack.sh
```

### 关键点

1. **agent 已经预装两个 skill**：
   - `easynet-pages-author` (前端 + api/<verb>.toml manifest 怎么写)
   - `easynet-ability-author` (写真 ability 然后 `easynet ability deploy`)

   两个 skill 在 `easynet agent add web-builder --type claude-code` 时**自动 seed** 进 agent workspace 的 `.claude/skills/`，这是 RFC-006-B v0.6 Phase 4.3 的工作。

2. **kind="ability" api manifest** 是关键。看：

   ```toml
   # ~/.easynet/web-apps/recipes/api/add_recipe.toml
   kind = "ability"
   ability = "web-builder.recipes_add_recipe"
   ```

   POST `/api/add_recipe` 进来 → Hub adapter 看 manifest → 调 `web-builder.recipes_add_recipe` 这个 ability → ability 用 `shell.run` 跑一段 python/bash 真改 `data/recipes.json`。**整条链 agent 自己写**。

3. **agent 真的 dispatch 真的 LLM 调用**：脚本会调 `easynet ability invoke web-builder.chat` → 后台起一个 `claude -p` 子进程 → 跑 1-3 分钟 → 写出全套文件并 deploy。

### silan 看的故事

> **传统全栈**: 前端 (React)、后端 (Node/Python)、数据库 (PG/SQLite)、deploy 流水线 (Docker/k8s)、auth、CORS、log... 一两周。
>
> **EasyNet**: 一句 prompt 给 agent，**3 分钟拿到 URL**。前端是文件，后端是 ability (你写的真代码)，deploy 是 `easynet pages create` 一行，全部进 EasyNet receipt 链，不动 nginx 不动 docker。
>
> 真正的 "**agent-native full-stack**" — agent 既是开发者也是 ops。

### 已验证

凉冰这一轮已经让 web-builder 真做出来：
- TeaLab (5 茶电商)
- ChefsTable (5 menu 私家厨房预约系统)
- Todo (真实持久化的 todo list, kind="ability" 真后端)

每一次都是 agent 完整自驱，silan 浏览器打开能用。

---

## 三、Demo #3 — EasyNet 即 OpenAI

**脚本**: `examples/public-routes-e2e/d3-openai-compat.sh`

### 故事

silan 用 cursor / Continue / langchain / openai-python / curl，**任何**说 OpenAI wire 的工具。**不改一行代码**，把 `base_url` 指向 EasyNet hub，`api_key` 换成 `easynet-sk-...`。EasyNet 上每个 chat-base ability **都自动是一个 model**。

### 跑

```bash
./examples/public-routes-e2e/d3-openai-compat.sh
```

### 关键点

1. **mint 一个 capability-URI 形态的 API key**
   ```bash
   easynet api-key create --label "demo"
   # → easynet-sk-<256-bit-hex>  (只显示这一次)
   # → 缓存在 ~/.easynet/api_keys.local.toml mode 0600
   ```
   这个 token = `resource/api_key.<full-id>` URA，是 EasyNet ontology 里的 capability。**没有 OAuth**，没有 token exchange，没有 scope 对象。

2. **`/v1/models`** 列 daemon 上所有 chat-base ability：
   ```json
   { "data": [
       {"id": "codex",       "ability": "codex.chat", ...},
       {"id": "web-builder", "ability": "web-builder.chat", ...}
   ]}
   ```

3. **`/v1/chat/completions`** 工作两种形态：
   - 非流式：返回标准 ChatCompletion JSON
   - 流式 (`stream:true`)：返回 SSE，每 chunk 是 ChatCompletionChunk，最后 `data: [DONE]`

4. **openai-python 零改动**：
   ```python
   from openai import OpenAI
   c = OpenAI(base_url="http://127.0.0.1:8787/v1", api_key="easynet-sk-...")
   for chunk in c.chat.completions.create(model="codex", messages=[...], stream=True):
       print(chunk.choices[0].delta.content, end="", flush=True)
   ```
   SDK 完全不知道自己在跟 EasyNet 说话——它觉得自己在跟 OpenAI 说话。

### silan 看的故事

> **传统 AI Gateway** (OpenRouter / Helicone): 把 OpenAI / Anthropic / Gemini 切换。但底层都是 LLM provider。
>
> **EasyNet**: model 不只是 LLM，**任何 EasyNet 上 publish 的 chat-base ability 都是 model**。一个用户写的 `<user>.therapist.chat` ability、一个 EAL mission 包的 chat ability、一个 multi-agent discuss 的 chat ability——**对 cursor 来讲都是一个模型 ID**。
>
> EasyNet 不是另一个 LLM provider，是**让 agent 行为图直接成为 OpenAI 兼容生态的一员**。

### 已验证

凉冰这一轮已经跑通：
- `easynet llm-api "..."` 一行 CLI 完整工作
- curl 流式 SSE 正常解析
- openai-python 真实流式输出 token

---

## 四、Demo #4 — 一个钱包多模型

**脚本**: `examples/public-routes-e2e/d4-multi-model.sh`

### 故事

silan 的 daemon 上有 codex agent、web-builder agent、未来可能还有别的。**一把 API key**，对所有 model 都有效。fan-out 同一个问题给多个 model，比较答案——agent-as-evaluator 模式不需要任何额外架构。

### 跑

```bash
./examples/public-routes-e2e/d4-multi-model.sh

# 自定义问题
QUESTION="用一句话解释 EasyNet ability 跟 REST endpoint 的区别" \
    ./examples/public-routes-e2e/d4-multi-model.sh
```

### 关键点

1. **一把 token = 一个 user URA 的全权代表**
   - mint 一次 `easynet api-key create`
   - 用这把 token 调 `/v1/chat/completions` 的任何 model 都 OK
   - 因为 token 解析到 `user/<username>` URA，admission 看的是 user 而不是 model

2. **fan-out**: 同一个 prompt 跑遍所有 model
   ```bash
   for MODEL in $MODELS; do
       curl ... -d '{"model":"'$MODEL'","messages":[...]}' | jq -r '.choices[0].message.content'
   done
   ```

3. **silan 后续可以 evaluate**：用第三个 agent (e.g. 一个 judge agent) 看哪个回答更好。完全在 EasyNet 内部，receipt 链审计完整。

### silan 看的故事

> **传统多 LLM**: OpenAI 一个 key、Anthropic 一个 key、Google 一个 key，OpenRouter 帮你聚合（再加它的一把 key），cost / quota 各管各。
>
> **EasyNet**: **一把 capability**, 调任何 model 都是它在背后授权。所有调用都进同一个 user 的 receipt 链。一个统一的 audit query 能告诉你这一周这把 key 调了多少次每个 model、用了多少 token。

---

## 五、Demo #5 — 一个 project 双 surface

**脚本**: `examples/public-routes-e2e/d5-cross-surface.sh`

### 故事

silan 想要一个 chat UI 网页：**这个网页本身**通过 Pages 部署 (RFC-006-B)，**这个网页里的 chat 功能**调 EasyNet 自己的 OpenAI-compat 接口 (RFC-006-C)。**两套 RFC 的功能在一个 demo 里同时露脸**——一个 daemon 同时 serve 静态页面 + serve OpenAI 流式回复。

### 跑

```bash
./examples/public-routes-e2e/d5-cross-surface.sh
```

### 关键点

1. 脚本写一个 `index.html` + `app.js`，前端用 fetch 直接 POST `/v1/chat/completions`
2. 把这个 site 通过 `easynet pages create ai-sandbox` 部署
3. 浏览器打开 `http://ai-sandbox.alice.pages.localhost:8787/?key=easynet-sk-...`
   - 页面读 `?key=` 存进 localStorage 然后从 URL 抹掉
   - chat 功能用 localStorage 里的 token 调本地 `/v1/chat/completions`
4. **同一个 hub listener** 既 serve 网页文件 (`pages_listener` 路由 Host) 又 serve OpenAI API (`/v1/*` 提前在 path-routing 里抓走)

### silan 看的故事

> Pages 跟 LLM-API compat 不是两个独立功能拼起来的——它们是**同一 paradigm 的两个 reference instance**：
>
> **外部协议 = invocation graph 的 transport view**。
>
> 一个 hub agent 同时持有两条 adapter ability (`01HUB.pages.serve` + `01HUB.openai.chat_completions`)，**互不干扰**，**两条都是 INV-1 pure adapter**。
>
> 未来 RFC-006-D / E / F 加新 surface (file storage / conversation snapshot / mission output) — 都是同一形态的新 reference instance, 一个 daemon 全 serve。

### 期望浏览器看到

打开页面后：
- 顶部有一个 model 下拉框 (`codex`, `web-builder`)
- 输入框打字，回车
- chat 框真实流式出 token (因为 `app.js` 解 SSE)
- 切换 model 再问一次，得到新模型的回答

---

## 六、receipt 链审计

每个 demo 跑完后，receipt 链上都留了审计 trail：

```
Demo #1 (静态网站):
  alice.pages.publish      canonical    subject = user/alice
  alice.d1-snapshot.page.fetch  operational  per-fetch (lossy)

Demo #2 (agent 全栈):
  web-builder.chat              canonical    subject = user/alice
    (causal_context cites ↑)
    └ web-builder.recipes_add_recipe   canonical
    └ web-builder.recipes_list_recipes canonical
    └ alice.pages.publish               canonical
    └ alice.recipes.api.add_recipe      canonical (kind=ability call)

Demo #3 (LLM-API):
  alice.api_key.create     canonical    subject = user/alice
  01HUB.openai.chat_completions  canonical (per request)  subject = resource/api_key.<id>
    └ codex.chat                  operational  causal_context.Scalar = ↑

Demo #5 (双 surface):
  alice.pages.publish                              canonical
  ai-sandbox.alice.page.fetch                     operational  (per page load)
  01HUB.openai.chat_completions                   canonical    (per chat send)
    └ <model>.chat                                operational
```

任何一条 leaf receipt 都能 walk back 到它的根因。这是 RFC-001 §1.5 + URA v2 plan §Phase 6 的 receipt-chain audit 在 demo 层面的体现。

---

## 七、清理

每个脚本结尾会告诉你 cleanup 命令，但通用清理：

```bash
# 撤掉所有 demo 项目
for p in d1-snapshot recipes ai-sandbox; do
    EASYNET_PAGES_USER=alice easynet pages delete $p --force 2>/dev/null
done

# 撤掉 demo API key (可选)
EASYNET_PAGES_USER=alice easynet api-key list
EASYNET_PAGES_USER=alice easynet api-key revoke <id_prefix>

# 清缓存的 token (可选)
rm ~/.easynet/api_keys.local.toml
```

---

## 八、demo 之外

| 想做的事 | 看 |
|---|---|
| 完整功能文档 (英文) | [`docs/PAGES_AND_LLM_API.md`](../docs/PAGES_AND_LLM_API.md) |
| Pages 范式 spec | [`docs/rfc/AXON-RFC-006-B-easynet-webapp.tex`](../docs/rfc/AXON-RFC-006-B-easynet-webapp.tex) |
| OpenAI compat spec | [`docs/rfc/AXON-RFC-006-C-openai-compat.tex`](../docs/rfc/AXON-RFC-006-C-openai-compat.tex) |
| Skill 教 agent 写网站 | [`skills/easynet-pages-author/SKILL.md`](../skills/easynet-pages-author/SKILL.md) |
| Skill 教 agent 写真 ability | [`skills/easynet-ability-author/SKILL.md`](../skills/easynet-ability-author/SKILL.md) |
| Hub-in-Docker 部署 | [`packaging/docker/hub-pages/full/`](../packaging/docker/hub-pages/full/) |
| 现存 e2e 测试 | [`packaging/docker/e2e/pages/pages-mvp.sh`](../packaging/docker/e2e/pages/pages-mvp.sh) |

---

*这份 demo 速查写给一类受众：第一次看 EasyNet Pages + LLM-API compat 的工程师 / 投资人 / 同事。每个 demo 都是 1-3 分钟的故事 + 真跑通的脚本。silan 想给团队 demo 这件事，按 D1 → D3 → D5 顺序走一遍最有冲击力——5 秒部署 → cursor 直接连 → 双 surface 同时 serve。*
