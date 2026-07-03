# EasyNet Pages + LLM-API Compat — Complete Feature Documentation

**Status**: shipped on branch `easynet-page` (commits b6cd8fe, 7041400, 8912978).
**Spec**: RFC-006-B v0.6 (Pages) + RFC-006-C v0.1 (OpenAI compat).
**Audience**: silan reviewing what landed, an operator wiring it up, an engineer extending it.

This document is the operator-facing summary of two paradigms that ship as one: (1) any folder of files is a website on EasyNet; (2) any chat-base ability on EasyNet is an OpenAI-compatible LLM endpoint. Both speak external protocols (HTTP, OpenAI streaming) as **transport views over the invocation graph** — the protocol is a window onto an ability dispatch, not a parallel state plane.

---

## 1 — Mental Model

EasyNet has one primitive: `invoke(caller, callee, ability, subject, ...)`. Every external surface — an HTTP file fetch, a `POST /v1/chat/completions`, an MCP tool call — projects to that primitive. The Hub agent (`01HUB`) owns the adapter abilities that do the projection. There is no second authority; the receipt chain remains the single source of truth.

```
Browser HTTP fetch                  cursor / openai-python
  GET /index.html                     POST /v1/chat/completions stream=true
       |                                   |
       v                                   v
01HUB.pages.serve                    01HUB.openai.chat_completions
  (RFC-006-B INV-1: pure adapter)      (RFC-006-C INV-1: pure adapter)
       |                                   |
       v  forward_invoke                   v  forward_invoke
<user>.<project>.page.fetch          <agent>.chat
  (kernel sandbox; resource bytes)     (real agent dispatch — Claude Code,
                                        Codex, future LLM drivers)
```

Both adapters are abilities. Both forward through the same dispatch path every other caller takes. Neither holds conversation state, project state, or any state beyond what receipts already record.

---

## 2 — Pages: Folder → Website

### 2.1 Quick start

```bash
# 1. Pick a folder
mkdir -p ~/.easynet/web-apps/myshop
echo '<h1>hello</h1>' > ~/.easynet/web-apps/myshop/index.html

# 2. Publish (operator on Mac host, daemon running on :8787)
EASYNET_PAGES_USER=alice easynet pages create myshop \
    --folder ~/.easynet/web-apps/myshop

# 3. Open
open http://myshop.alice.pages.localhost:8787/
```

### 2.2 Project layout

```
~/.easynet/web-apps/<project_id>/
├── index.html           # GET / → /index.html (Hub convention)
├── style.css
├── app.js
├── assets/
│   └── logo.png
└── api/                 # optional dynamic backend
    ├── list_items.toml
    ├── checkout.toml
    └── add_task.toml
```

URA shape (RFC-006-B v0.6 §2): `easynet:///r/<realm>/resource/<user>.<project>/<path>`

URL shape: `https://<project>.<user>.pages.<realm>/<path>`

### 2.3 Static files

The page-fetch path runs inside a kernel-enforced read sandbox:

  - Linux: `openat2(2)` with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS` — paths cannot escape the published folder, regardless of `..` or symlinks.
  - macOS: `realpath` + prefix check + `O_NOFOLLOW` (acknowledged weaker; production targets Linux).
  - Default-deny: dotfiles (`/.git`, `/.env`, etc.) refused; extensions outside the allow-list served as `application/octet-stream` + `Content-Disposition: attachment`.

### 2.4 Dynamic backend — three manifest kinds

Every endpoint at `<project>/api/<verb>.toml` becomes `POST /api/<verb>` (and GET; OPTIONS preflight is handled). The manifest's `kind` field picks the execution mode.

#### kind = "static_json"

```toml
# api/list_items.toml
kind = "static_json"

[[response]]
id    = "mug-12oz"
name  = "Ceramic Mug"
price = 18

[[response]]
id    = "bowl-3pc"
name  = "Bowl Set"
price = 42
```

GET `/api/list_items` → returns the array verbatim. Useful for product catalogs, feature flags, anything baked into the deploy.

#### kind = "echo"

```toml
# api/checkout.toml
kind = "echo"

[extra]
order_id = "ord-demo-001"
status   = "confirmed"
eta      = "2026-05-10"
```

POST `/api/checkout` `{"items": [...], "qty": 2}` → returns the request body merged with `[extra]`'s fields. Useful for stand-in form submissions where the demo just needs to confirm receipt.

#### kind = "ability"  *(new — agent-built backends)*

```toml
# api/add_task.toml
kind = "ability"
ability_ura = "easynet:///r/easynet.run/ability/alice.web-builder.todo_add_task"
```

POST `/api/add_task` `{"title": "buy milk"}` → invokes the *real* Ability URA `easynet:///r/easynet.run/ability/alice.web-builder.todo_add_task` (which the agent deployed via `easynet ability deploy`) with the request body as args, returns whatever the ability returned. **This is the agent-driven full-stack loop**: an LLM-authored project carries its frontend, its backend ability TOML manifests, *and* the ability deploy step in one project. The api manifest then wires the HTTP route to the deployed ability.

The adapter forwards through the live registry (in-process), not through the daemon's IPC socket — invoking through your own control.sock from inside the daemon would self-deadlock.

### 2.5 CLI surface

```
easynet pages create   <project_id> --folder <path>
easynet pages list
easynet pages show     <project_id>
easynet pages delete   <project_id> --force
easynet pages url      <project_id>
```

All subcommands accept `--json` for scripting. They wrap the abilities `<user>.pages.{publish,list,get,unpublish}`; the canonical surface is the abilities, the CLI is sugar.

### 2.6 Skill seeded into every agent

Every agent added via `easynet agent add <name> --type claude-code` (or codex) now arrives with two seeded skills in its workspace:

  - `.claude/skills/easynet-pages-author/SKILL.md` — how to write a frontend, write `api/<verb>.toml` manifests (any of the three kinds), and run `easynet pages create`.
  - `.claude/skills/easynet-ability-author/SKILL.md` — how to write a real ability TOML, deploy it via `easynet ability deploy`, and verify.

A freshly-added agent that receives "build me a todo list" can therefore complete the full loop — frontend + manifests + deployed ability + deploy step + curl verification — without further briefing. Verified end-to-end against a freshly-dispatched web-builder Claude Code agent that authored TeaLab (e-commerce), ChefsTable (reservation system), and Todo (real persistent backend).

### 2.7 Receipt tier

Per RFC-006-B v0.6 INV-3 + INV-5:

  - `<user>.pages.publish` / `unpublish` — canonical receipt (state-changing).
  - `<user>.<project>.page.fetch` / `<user>.<project>.api.<verb>` — operational receipt (lossy ring buffer; aggregable rates surfaced through `observe.health`).

### 2.8 Hub-in-Docker

The hub can run in two shapes:

  - `packaging/docker/hub-pages/full/` — full daemon container. Cross-compile the linux binaries with the existing `easynet-linux-build:bookworm-arm64` image, then `docker compose build && docker compose up -d`. Volume-mount `./sites/` to publish projects from host. Browser hits `:8787` → goes straight into container daemon.
  - `packaging/docker/hub-pages/nginx-sidecar/` — nginx-only container, host daemon stays on `:8788`, container forwards `:8787` to host. Useful when disk pressure prevents a Rust toolchain build.

Both containerised shapes preserve the same INV-1 (Adapter Purity): the container is the HTTP boundary, the daemon (host or container) is the substrate. Production drops the in-daemon listener and routes only through the container; dev tolerates either.

---

## 3 — LLM-API Compat: Chat Ability → OpenAI Endpoint

### 3.1 Quick start

```bash
# 1. Mint an API key
EASYNET_PAGES_USER=alice easynet api-key create --label "cursor on my mac"
# -> token: easynet-sk-<256-bit-hex>     (only shown ONCE)
# -> cached at ~/.easynet/api_keys.local.toml mode 0600

# 2a. CLI usage
easynet llm-api "tell me a joke about EasyNet"
# -> default model = first chat-base Ability URA from /v1/models
# -> default key = the one we just minted (cached)

# 2b. From any OpenAI client (no code change!)
TOKEN=$(grep default_token ~/.easynet/api_keys.local.toml | cut -d'"' -f2)
MODEL="easynet:///r/easynet.run/ability/alice.codex.chat"
curl -sN -X POST http://127.0.0.1:8787/v1/chat/completions \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"model\":\"$MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"stream\":true}"

# 2c. From openai-python
python3 -c "
from openai import OpenAI
client = OpenAI(base_url='http://127.0.0.1:8787/v1', api_key='$TOKEN')
for chunk in client.chat.completions.create(
    model='easynet:///r/easynet.run/ability/alice.codex.chat',
    messages=[{'role':'user','content':'count from 1 to 5'}],
    stream=True,
):
    if chunk.choices[0].delta.content:
        print(chunk.choices[0].delta.content, end='', flush=True)
"
```

### 3.2 What's exposed

```
GET   /v1/models                    list chat-base abilities as OpenAI models
POST  /v1/chat/completions          unary or streaming chat completion
                                    (Authorization: Bearer easynet-sk-<id>)
OPTIONS /v1/*                       CORS preflight (open)
```

`/v1/models` projects the local ability registry's chat-base entries (any ability of the shape `<owner>.chat` where `<owner>` is a single dot-free segment; explicitly excludes `*.api.chat`, `*.page.chat`, `*.actions.chat` — those belong to the Pages reference system). Each returned OpenAI `Model.id` is the canonical agent-owned chat Ability URA, such as `easynet:///r/easynet.run/ability/alice.codex.chat`.

`/v1/chat/completions` accepts standard OpenAI body:

```json
{
  "model": "easynet:///r/easynet.run/ability/alice.codex.chat",
  "messages": [
    {"role": "system", "content": "..."},
    {"role": "user",   "content": "..."}
  ],
  "stream": true,
  "temperature": 0.7,
  ...
}
```

Returns:

  - **stream=false** → one `chat.completion` JSON object with `choices[0].message.content`.
  - **stream=true** → SSE stream: `data: {chunk}\n\n` for each chunk, then `data: [DONE]\n\n`.

The `choices` and `usage` shapes are byte-equivalent to OpenAI's. Token counts are heuristic (chars/4); replace with provider-real counts when the underlying chat ability surfaces them.

### 3.3 The four normative invariants (RFC-006-C v0.1)

  - **INV-1 Adapter Purity** — `01HUB.openai.chat_completions` is hub-rooted. No state mutation beyond a single auth-trail receipt. No caching, no rewriting. Forwards via the live registry.
  - **INV-2 Capability-URI Key** — bearer is `easynet-sk-<256-bit hex>`, addressed as `resource/api_key.<full-id>` URA. Stored as sha256(token) in `~/.easynet/api_keys.toml` mode 0600. Revocation = one ability call. No OAuth.
  - **INV-3 Filter Determinism** — same chat-ability output → same SSE chunk list, byte for byte. v0.1 fakes streaming by chunking a unary reply at 64 chars per chunk (deterministic by construction); v0.2 swaps in a real bidi without changing the invariant.
  - **INV-4 Auth Receipt Trail** — every request mints one canonical receipt against the API-key URA; dispatch sub-receipts cite it via `causal_context.Scalar`. v0.1 acknowledges this; v0.2 wires the explicit canonical-receipt mint.

### 3.4 What "chat-base" means

A chat-base ability is any RPC ability with name shape `<owner>.chat` whose input is `{prompt, system?}` and whose output carries a string at one of `reply`, `message`, or `content`. The OpenAI adapter's `flatten_messages` normalises the OpenAI `messages: [...]` shape into `{prompt, system}` (last user message → prompt, all system messages concatenated → system, multi-turn history rendered as a transcript so the agent has context).

Per silan's directive, this includes any ability that *behaves* chat-shape, not just literally-named `.chat` ones — but the v0.1 convention is name-based for simplicity. Future revisions will allow explicit interface tagging in the ability TOML so e.g. a `<agent>.discuss.start_round` ability can opt in.

### 3.5 CLI surface

```
easynet api-key create   [--label <text>] [--no-cache] [--json]
easynet api-key list                                  [--json]
easynet api-key revoke   <id_prefix>

easynet llm-api          "<prompt>"
                          [--model <ability-ura>]
                          [--key   <token>]
                          [--system <text>]
                          [--json]
```

`api-key create` returns the bearer token ONCE; raw token never enters `api_keys.toml`. The `--no-cache` flag suppresses writing `~/.easynet/api_keys.local.toml`; otherwise the freshly-minted token lands there mode 0600 and `easynet llm-api` finds it as the default.

`llm-api` defaults: `--model` resolves via `01HUB.openai.list_models` to the first chat-base Ability URA; `--key` reads `--key` arg, then `EASYNET_API_KEY` env, then the local cache file.

### 3.6 Mapping: where does my call go

```
HTTP POST /v1/chat/completions     model:"easynet:///r/easynet.run/ability/alice.codex.chat"
   | Authorization: Bearer easynet-sk-<id>
   v
01HUB.openai.chat_completions      ← adapter (INV-1 pure)
   |
   | INV-2 auth: resolve_token(<id>) -> user/<username>
   | INV-3 filter: messages[] -> {prompt, system}
   | INV-4 receipt: canonical mint
   v
codex.chat                          ← local registry key derived from the Ability URA
   |
   | dispatched into Codex agent's claude-style runner
   v
real LLM call (the actual model the agent driver uses)
   |
   v
reply text -> OpenAI chunk projection -> SSE -> client
```

The same flow works for `easynet:///r/easynet.run/ability/alice.web-builder.chat` or any other agent-owned chat Ability URA returned by `/v1/models`. From the client's perspective the model id is opaque; the daemon does the routing.

---

## 4 — Files Inventory

### 4.1 Ontology / spec

  - `docs/rfc/AXON-RFC-006-B-easynet-webapp.tex` (v0.6) — Pages reference system, four invariants, paradigm sibling RFCs.
  - `docs/rfc/AXON-RFC-006-C-openai-compat.tex` (v0.1) — OpenAI compat as agent dispatch view.
  - `EasyNet-Axon/document/concepts/ONTOLOGY_AGENT_ABILITY.md` — §3.2.1 ability ownership widening (user / agent / hub / resource).

### 4.2 Daemon-side runtime

  - `src/daemon/ability/builtins/resources/pages/{api.rs, fetch.rs, mod.rs, publish.rs, sandbox.rs, state.rs, mime.rs, list_get_unpublish.rs, identity.rs}` — Pages reference system.
  - `src/daemon/ability/builtins/governance/api_key.rs` — `<user>.api_key.{create,list,revoke}` capability mint/revoke.
  - `src/daemon/ability/builtins/integrations/openai_compat.rs` — `01HUB.openai.{chat_completions,list_models}` adapters.
  - `src/runtime/hub/{pages_listener.rs, pages_serve_ability.rs}` — HTTP boundary on axum, routes `/v1/*` and `*.*.pages.<realm>/*`.
  - `src/daemon/ability/dispatch.rs` — `chain_rpc_fallback` (resolver chaining), `list_rpc_names` (used by `list_models`), `resolve_rpc` (used by adapter).
  - `src/runtime/workspace.rs` — `write_pages_author_seed` + `write_ability_author_seed` (skill seeding into every agent workspace).
  - `src/runtime/drivers/claude_code.rs` — `--allowedTools` allowlist extension for `Bash(easynet:*)` + `Bash(curl:*)`.

### 4.3 CLI

  - `src/cli/pages.rs` — `easynet pages create/list/show/delete/url`.
  - `src/cli/api_key_cli.rs` — `easynet api-key create/list/revoke`.
  - `src/cli/llm_api.rs` — `easynet llm-api "<prompt>"`.
  - `src/cli/agent.rs` — eager workspace projection on `agent add`.

### 4.4 Skills (project-level seeds for agent workspaces)

  - `skills/easynet-pages-author/SKILL.md` — agent-facing tutorial for the Pages reference system (frontend + api manifests + deploy).
  - `skills/easynet-ability-author/SKILL.md` — pre-existing; now seeded into every workspace alongside pages-author.

### 4.5 Containers

  - `packaging/docker/hub-pages/full/{Dockerfile, docker-compose.yml, entrypoint.sh, vendor.sh, sites/}` — full-daemon container.
  - `packaging/docker/hub-pages/nginx-sidecar/{conf/nginx.conf, docker-compose.yml}` — nginx-only fallback.
  - `.dockerignore` — keeps docker-build context lean.

---

## 5 — Verification Reproducer

```bash
# 1. Build the daemon (once)
cd EasyNet-Cli
cargo build --features axon-pb --bin easynet --bin easynet-daemon

# 2. Symlink so PATH `easynet` is the dev build
ln -sf $PWD/target/debug/easynet ~/.local/bin/easynet

# 3. Start daemon
EASYNET_PAGES_PORT=8787 EASYNET_PAGES_USER=alice EASYNET_PAGES_REALM=easynet.run \
    target/debug/easynet-daemon &

# 4. Pages — agent-driven full-stack
easynet agent add web-builder --type claude-code
EASYNET_PAGES_USER=alice easynet ability invoke easynet:///r/easynet.run/ability/alice.web-builder.chat --args '{
  "prompt":"Build a todo list app: kind=ability backend, real persistent JSON storage, three abilities (list_tasks/add_task/delete_task) deployed via easynet ability deploy, frontend that calls them. Project_id=todo. EASYNET_PAGES_USER=alice. Tell me the URL when done."
}'
# -> agent writes ~/.easynet/web-apps/todo/{index.html,style.css,app.js,
#                     api/{list_tasks,add_task,delete_task}.toml,
#                     scripts/{list_tasks,add_task,delete_task}.py},
#     deploys 3 abilities, runs `easynet pages create`, replies with URL.
# -> http://todo.alice.pages.localhost:8787/   → full functional todo app.

# 5. LLM-API
EASYNET_PAGES_USER=alice easynet api-key create --label "demo"
# -> easynet-sk-<id>, cached locally

easynet llm-api "Reply with: ok"
# -> [llm-api] model=easynet:///r/easynet.run/ability/alice.codex.chat
# -> ok

MODEL="easynet:///r/easynet.run/ability/alice.codex.chat"
curl -sN -X POST http://127.0.0.1:8787/v1/chat/completions \
    -H "Authorization: Bearer $(grep default_token ~/.easynet/api_keys.local.toml | cut -d'\"' -f2)" \
    -H "Content-Type: application/json" \
    -d "{\"model\":\"$MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"stream\":true}"
# -> data: {chunk1}\n\ndata: {chunk2}\n\n...data: [DONE]\n\n

python3 -c "
from openai import OpenAI
c = OpenAI(base_url='http://127.0.0.1:8787/v1', api_key='easynet-sk-...')
for chunk in c.chat.completions.create(model='easynet:///r/easynet.run/ability/alice.codex.chat', messages=[{'role':'user','content':'count 1-5'}], stream=True):
    if chunk.choices[0].delta.content: print(chunk.choices[0].delta.content, end='', flush=True)
"
# -> 1\n2\n3\n4\n5
```

---

## 6 — Known Limitations + Future Work

  - **Streaming is fake (chunked unary)**. v0.2: real bidi when the underlying chat ability becomes a streaming primitive. Filter determinism (INV-3) holds either way.
  - **INV-4 auth receipt minting is implicit**. v0.2: explicit canonical receipt with body_hash + usage attached to the API-key URA.
  - **No `/v1/embeddings`**. Requires an `embed` ability primitive; not yet defined.
  - **No tool_call surface**. v0.2: declarative descriptor mapping so an OpenAI client passing `tools=[...]` gets the agent's tool_use frames mapped onto its descriptors.
  - **No conversation persistence**. The OpenAI protocol is stateless; EasyNet's chat ability is too. Server-side history will live in a `resource/conversation.<id>` (RFC-006-D territory).
  - **macOS sandbox is weaker than Linux**. Production targets Linux; macOS is dev only.
  - **API-key scopes are flat**. v0.2: scoped keys (e.g. "may invoke only `<owner>.<verb>` matching pattern X").
  - **No request-rate limiting**. Token-bucket per-key is post-MVP.

---

## 7 — Receipt-chain audit cheatsheet

After a chain of work the receipt store carries:

```
canonical receipts:
  <user>.pages.publish              folder published; subject = user URA
  <user>.pages.unpublish            folder withdrawn
  <user>.api_key.create             new API key minted; subject = user URA
  <user>.api_key.revoke             key revoked; subject = api_key URA
  <user>.<project>.api.<verb>       (when kind=ability and the underlying
                                     ability mutates state — its own
                                     canonical receipts will land here)

operational receipts:
  <user>.<project>.page.fetch       static byte read (per request)
  <user>.<project>.api.<verb>       (when kind=static_json or echo)
  01HUB.pages.serve                 HTTP request received (lossy)
  01HUB.openai.chat_completions     LLM-API request received (lossy in v0.1;
                                     becomes canonical with body_hash + usage in v0.2)
  <agent>.chat                      dispatch into the chat-base agent (operational
                                     because the read of "what would you say to this
                                     prompt" does not mutate canonical state, even
                                     though the agent may invoke state-changing
                                     sub-abilities along the way — those land
                                     as their own receipts further down)
```

The chain is walkable both ways (per RFC-001 §1.5 + URA v2 plan §Phase 6): from any leaf receipt up to its causal predecessor via `causal_context.Scalar`, and from any subject URA across all receipts that touched it.

---

*Closing note (silan-facing).* The two RFCs (006-B Pages, 006-C OpenAI compat) ship the same paradigm against two external protocols. Future RFC-006-D / E / F will follow the same template: file storage, conversation snapshots, mission outputs — each is a deterministic projection of the invocation graph through one more wire format. The discipline to maintain across the family is the four invariants: pure adapter, capability identity, deterministic projection, auth receipt trail.
