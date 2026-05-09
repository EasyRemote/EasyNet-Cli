# 公网路由端到端演示 (Phase 9, v4.1.5 URA)

## 这个 demo 证明什么

silan Mac 上跑的一个 daemon, 通过 easynet.run 公网 hub, 暴露成两条公网可调用的 surface:

1. **OpenAI 兼容 API** — `POST https://easynet.run/v1/chat/completions` + `GET /v1/models`. 任何 LLM client (Cursor / Continue / openai SDK) 直接当 OpenAI endpoint 用. 鉴权 = `Authorization: Bearer easynet-sk-<key>`. 这条路是 RFC-006-C v0.1 落地.

2. **Pages 静态站点** — `GET https://easynet.run/web/<username>/<project>/<path>`. silan 在本地 `easynet pages create my-site --folder ./build` 之后, 全世界都能直接访问 `https://easynet.run/web/silan/my-site/index.html`. 这条路是 RFC-006-B v0.6 落地, 通过 Linux `openat2 RESOLVE_BENEATH` 内核沙箱保证 path traversal 不可能.

## 上一句话看不懂的人, 直接跑这个

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet
bash scripts/demo-public-routes.sh
```

通了你会看到:

```
━━ 6. GET /v1/models
  ✓ GET /v1/models returned data array (count=1)
      - probe-agent

━━ 7. POST /v1/chat/completions (unary)
  ✓ unary chat completion replied
      ok

━━ 8. POST /v1/chat/completions (streaming) — routing reachability
  ✓ streaming: full round-trip — 3 chunks + [DONE] sentinel

━━ 9. Publish a folder on caller-a + GET /web/<u>/<p>/
    Published.
      project_uri:  easynet:///r/easynet.run/resource/<u>.probe/
      url_root:     http://probe.<u>.pages.localhost:8787/
  ✓ GET /web/<u>/probe/ → 200
      <!doctype html><h1>hello from probe</h1>

━━ 10. Summary
All checks PASSED
```

## 这个 demo 实际在做什么 — 拆给你看

### Step 1-3: 起 docker + register user + pair device

```
postgres-a + hub-a (Go backend + Rust hub daemon) + caller-a (Rust device daemon)
↓
curl http://127.0.0.1:19080/api/v1/auth/register  → user account
curl http://127.0.0.1:19080/api/v1/devices/pairing → pairing token
docker exec caller-a easynet device join $TOKEN  → caller-a 拿到 node_id
```

到这一步 caller-a 是 user 的"已配对设备", hub 上 trust anchor 包含 caller-a 的公钥.

### Step 4: 把 hub 的 TLS CA pin 到 caller-a + 重载 hub trust

```
caller-a 的 realm-trust.toml 加上 tls_ca_pem_path
hub-a 重启 → 从盘上重新读 trust (它不 hot-reload)
caller-a 重连 hub-a 的 bidi session
```

### Step 5: `easynet agent add probe-agent --type claude-code`

注册一个名叫 `probe-agent` 的 chat-base agent, 类型是 claude-code. daemon 现在 host `probe-agent.chat` ability.

### Step 6: `easynet runtime start` + 关键 prelude

daemon 启动 + 跟 hub 建 bidi session 时, **session prelude** 做三件事:

1. `federation.join` — "我加入这个 realm"
2. `federation.advertise_abilities` — "我 host 这些 ability" (115 个)
3. **`federation.advertise_agent` × N** — "我 host 这些 agent" (Phase 9 新增)

第三步关键: device 把 `agent/<username>.probe-agent` + `agent/<username>.pages` 等 hosted-by-self 关系 publish 给 hub 的 `AdvertisedAgentStore`. 之后 hub 收到 agent URA 的 callee 就能查这个 store 反向找到 host device.

### Step 7-9: 实际跑三条公网路

#### 7-8. OpenAI 兼容路径

```
curl -X POST http://127.0.0.1:19080/v1/chat/completions \
    -H "Authorization: Bearer $RAW_KEY" \
    -d '{"model":"probe-agent","messages":[{"role":"user","content":"reply with: ok"}]}'
```

backend Go handler `chat_completions.go`:

```go
calleeURA := axon.AgentURI(realm, username, modelStr)
// = "easynet:///r/hub-a.local/agent/<username>.probe-agent"
```

backend → daemon-grpc → hub daemon's `<self>.invoke_remote` → 
hub `lookup_target_with_agent_fallback(agent/<u>.probe-agent)` 查 AdvertisedAgentStore 找到 host_uri = `<caller-a device URI>` → 通过 PresenceRegistry 取到 caller-a 的 bidi sender → push dispatch frame → caller-a daemon's `matches_self_target_uri` 接受 (本地 host 这个 agent) → 派发到 `probe-agent.chat` → reply 返回原路.

streaming case: 同样路由, daemon 内 `01HUB.openai.chat_completions` adapter 把 chat ability 的 unary reply 切 chunks 在 64 字符边界, 发回 backend, backend 写 `text/event-stream`.

#### 9. Pages 路径

```
docker exec caller-a EASYNET_PAGES_USER=$USERNAME easynet pages create probe --folder /srv/easynet/web-apps/probe
curl http://127.0.0.1:19080/web/$USERNAME/probe/
```

`easynet pages create` 在 daemon 里:
- 打开 folder fd (Linux `openat2(RESOLVE_BENEATH | NO_SYMLINKS | NO_MAGICLINKS)`)
- 把 `(user, project)` 写进 `PUBLISHED_PROJECTS`
- 注册 `<user>.probe.page.fetch` ability

backend Go handler `pages_public/serve.go`:

```go
calleeURA := axon.AgentURI(realm, username, "pages")
// = "easynet:///r/hub-a.local/agent/<username>.pages"
ability  := fmt.Sprintf("%s.%s.page.fetch", username, project)
subjectURI := axon.PagesResourceURI(realm, username, project, rest)
// = "easynet:///r/hub-a.local/resource/<username>.probe/index.html"
```

caller-a daemon 在 prelude 时已经把 `agent/<u>.pages` advertise 给了 hub. hub forward → daemon 沙箱 `open_beneath` 读 bytes → b64 编码返回 → backend decode 后 wire 上送回 client, 设 `Content-Type: text/html` + `ETag: "sha256-..."`.

## 这个 demo 在哪些层面证明了 Phase 9 真的对

1. **wire 全标准 v4.1.5**: callee = `easynet:///r/<realm>/agent/<u>.<a>` (不是旧的 `agent/<bare-uuid>` 也不是 `r/prv/...`); subject = `resource/<...>` 用 `axon.ApiKeyResourceURI` / `axon.PagesResourceURI` helpers; ability = `<owner>.<agent>.<verb>` 三段 dot-tail.

2. **AdvertisedAgentStore round-trip 真的发生**: 没这个 store 的 host_uri 反查, agent URA callee → `target_offline`. demo 里 `/v1/chat/completions` 和 `/web/<u>/<p>/` 都通过 agent URA, 说明 advertise prelude 的 wire 调用真的到了 hub, 真的被 upsert 了.

3. **kernel 沙箱真的保护**: `RESOLVE_BENEATH` 拦截任何 `..` 跳出. 直接 `curl http://localhost:19080/web/$USERNAME/probe/../../etc/passwd` 会 404 (file not found beneath project root), 不会 leak.

4. **bearer auth 通过 Subject + Delegation 而非 token forward**: backend 用 ApiKey ent 表 resolve bearer → 拿 user_id + username → 构造 `subject = api_key URA` + sign envelope. daemon side 不重新校验 bearer, 直接信任 envelope. RFC-006-C v0.1 §INV-2 的 capability-URI 模型.

## 怎么改成在 silan Mac 上裸跑 (不走 docker)

把 `scripts/demo-public-routes.sh` 的 `HUB_HTTP="http://127.0.0.1:19080"` 改成 `https://easynet.run`, 把 `caller-a` 改成 silan 的本机 daemon, 把 `easynet device join` 用真实 pairing token 跑一次, 之后所有 curl 命令都对真实公网生效.

## 涉及的 commits (Phase 8 + 9)

- EasyNet `05119cd` — `/v1/chat/completions` + `/v1/models` + `/web/<u>/<p>/` 三个 handler
- EasyNet-Cli `caf92ea` — daemon `matches_self_target_uri` 接受 agent URA
- EasyNet-Cli `326c1a0` — 杀光 `r/prv/{hub,reg}` legacy URI shape
- EasyNet `008582a` — backend 删 dead `AbilityResourceURI` + 加 `ApiKeyResourceURI` / `PagesResourceURI` helpers
- EasyNet-Cli `fb91b98` — daemon session prelude `federation.advertise_agent` × N
- EasyNet `f9ee1c0` — backend wire callee = `agent/<username>.<agent>`

跨两 repo, 6 个 commit. 全部 v4.1.5 严格符合, 没有 legacy URI 形态在 wire 上出现.
