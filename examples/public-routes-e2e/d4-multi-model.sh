#!/usr/bin/env bash
# d4-multi-model.sh — Killer Demo #4: many agents, one wallet.
#
# Story: silan has multiple agents on the daemon — Codex, Claude
# Code (web-builder), maybe a future Gemini driver. Each agent's
# chat-base ability is its own model. silan mints ONE API key
# (one capability), uses it to drive all the models. From a
# client's perspective it looks like a multi-model gateway; from
# EasyNet's perspective it's just routing model_name to ability.
#
# What this demonstrates:
#   - one capability bearer authorises calls across all models
#     the daemon hosts (no per-model API keys, no OpenRouter-style
#     credential aggregation needed)
#   - the same model name resolution rule applies across drivers:
#     "codex" → codex.chat ability, "web-builder" → web-builder.chat
#   - silan can ask DIFFERENT agents about THE SAME thing and
#     compare answers — agent-as-evaluator pattern works trivially

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/_lib.sh"
ensure_daemon

step "1. Find which models are available"
MODELS=$(curl -s "http://127.0.0.1:$PORT/v1/models" | jq -r '.data[].id')
note "  models on this daemon:"
echo "$MODELS" | sed "s/^/    /"
COUNT=$(echo "$MODELS" | wc -l | tr -d ' ')
if [ "$COUNT" -lt 2 ]; then
    warn "only $COUNT chat-base abilities — add a second agent to see multi-model"
    note "  e.g. easynet agent add codex --type codex   (or any other type)"
fi
pause

step "2. Mint one key"
TOKEN=$(grep default_token "$HOME/.easynet/api_keys.local.toml" 2>/dev/null | cut -d'"' -f2)
if [ -z "$TOKEN" ] || ! "$EASYNET" api-key list 2>&1 | grep -q active; then
    run "$EASYNET" api-key create --label "demo-4-multi-model"
    TOKEN=$(grep default_token "$HOME/.easynet/api_keys.local.toml" | cut -d'"' -f2)
else
    ok "reusing cached key (id_prefix=$("$EASYNET" api-key list 2>/dev/null | grep active | awk '{print $1}' | head -1))"
fi
pause

step "3. Fan-out the same prompt to every model"
QUESTION="${QUESTION:-In one short sentence: what is the simplest difference between an EasyNet ability and a REST endpoint?}"
note "  question: $QUESTION"
echo
for MODEL in $MODELS; do
    printf "%s── %s%s\n" "$BOLD" "$MODEL" "$RST"
    REPLY=$(curl -s -X POST "http://127.0.0.1:$PORT/v1/chat/completions" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d "$(jq -n --arg m "$MODEL" --arg q "$QUESTION" '{
            model: $m,
            messages: [{role:"user", content:$q}]
          }')" \
        | jq -r '.choices[0].message.content // .error.message // .')
    echo "$REPLY" | fold -s -w 78 | sed "s/^/    /"
    echo
done
pause

step "4. The same key authorised every call"
note "  receipts (canonical) for the api_key resource will show:"
note "    api_key.<id> -> 01HUB.openai.chat_completions x N (one per model)"
note "       (each cites the chat-base dispatch as causal_context.Scalar)"
note "  (the receipt-walking subcommand to query this lands in v0.2)"

echo
ok "demo #4 done.  one capability, many models, full audit trail."
