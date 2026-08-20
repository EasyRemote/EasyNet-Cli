#!/usr/bin/env bash
# d3-openai-compat.sh — Killer Demo #3: EasyNet IS an OpenAI endpoint.
#
# Story: silan has cursor / Continue / Claude Desktop / langchain
# / openai-python / curl — every single tool that speaks OpenAI
# wire format. Without changing a single line of any of them,
# silan points base_url at EasyNet and api_key at an
# easynet-sk-... bearer. Every chat-base ability on EasyNet
# becomes a model. The whole external LLM tooling ecosystem
# starts working against agents EasyNet hosts.
#
# What this demonstrates:
#   - /v1/models   lists chat-base abilities as OpenAI models
#   - /v1/chat/completions  unary  + streaming SSE
#   - capability-URA auth (Bearer easynet-sk-<256-bit-id>),
#     no OAuth, mints + revoke as ability calls
#   - openai-python SDK works zero-change

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/_lib.sh"
ensure_daemon

LABEL="${LABEL:-demo-3}"

step "1. Mint an API key"
note "  this is a capability-URA: resource/api_key.<256-bit-id>"
run "$EASYNET" api-key create --label "$LABEL"
TOKEN=$(grep default_token "$HOME/.easynet/api_keys.local.toml" | cut -d'"' -f2)
ok "token cached at ~/.easynet/api_keys.local.toml (mode 0600)"
pause

step "2. List models — what's running on EasyNet right now?"
note "  GET /v1/models  (no auth needed for discovery)"
curl -s "http://127.0.0.1:$PORT/v1/models" | jq -r '.data[] | "    \(.id)  (ability=\(.ability))"'
pause

step "3. Unary chat completion"
note "  POST /v1/chat/completions  Authorization: Bearer easynet-sk-..."
curl -s -X POST "http://127.0.0.1:$PORT/v1/chat/completions" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "model": "codex",
      "messages": [
        {"role":"system", "content":"Reply in exactly one sentence."},
        {"role":"user",   "content":"What is EasyNet, in one sentence?"}
      ]
    }' | jq -r '.choices[0].message.content' | sed "s/^/    /"
pause

step "4. Streaming SSE — the real one"
note "  POST /v1/chat/completions  body: stream=true"
note "  watch the chunks arrive (each \`data: {...}\` line is one OpenAI ChatCompletionChunk):"
echo
curl -sN -X POST "http://127.0.0.1:$PORT/v1/chat/completions" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{
      "model": "codex",
      "messages": [{"role":"user","content":"Count from 1 to 8 inclusive, one number per line."}],
      "stream": true
    }' | head -20
pause

step "5. From inside CLI — easynet llm-api wraps the same path"
note "  default model = first chat-base ability; default key = cached"
run "$EASYNET" llm-api "Reply with exactly: ok"
pause

step "6. From openai-python — the SDK you already have"
if command -v python3 >/dev/null 2>&1 && python3 -c "import openai" 2>/dev/null; then
python3 <<PY
from openai import OpenAI
client = OpenAI(base_url="http://127.0.0.1:$PORT/v1", api_key="$TOKEN")

print("    [models] " + ", ".join(m.id for m in client.models.list().data))

print("    [stream] ", end="", flush=True)
for chunk in client.chat.completions.create(
    model="codex",
    messages=[{"role":"user","content":"3 short greetings, one per line"}],
    stream=True,
):
    if chunk.choices and chunk.choices[0].delta.content:
        print(chunk.choices[0].delta.content, end="", flush=True)
print()
PY
else
    warn "openai-python not installed — pip install openai to see SDK demo"
fi

echo
ok "demo #3 done.  cursor, continue, langchain — point them at $PORT and they work."
note "  base_url: http://127.0.0.1:$PORT/v1"
note "  api_key:  $TOKEN"
note "  revoke:   easynet api-key list   then   easynet api-key revoke <id_prefix>"
