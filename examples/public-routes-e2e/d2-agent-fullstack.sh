#!/usr/bin/env bash
# d2-agent-fullstack.sh — Killer Demo #2: agent writes the whole stack.
#
# Story: silan tells a Claude Code agent (not silan, not 凉冰)
# "build me X". The agent reads its seeded easynet-pages-author
# + easynet-ability-author skills, writes a frontend, writes a
# real persistent backend (deployed as EasyNet abilities), wires
# the api manifests with kind="ability", deploys via easynet
# pages create, and replies with the URL. silan opens browser,
# clicks around, data persists.
#
# What this demonstrates:
#   - skill seeding into agent workspace (easynet agent add ...
#     auto-installs the two skills)
#   - kind="ability" api manifests routing real backend behavior
#   - the full-stack loop is REPRODUCIBLE — not a one-off; the
#     agent does it for any reasonable prompt
#   - true agent autonomy: no凉冰-side hand-holding, no human
#     touching files, no human running the deploy step
#
# Cost: this dispatches a real Claude Code session, which costs
# tokens. A typical demo run is ~30k input / ~10k output tokens.

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/_lib.sh"
ensure_daemon

PROMPT_DEFAULT="Build a small recipes web app. Frontend lists recipes from \
a real backend; user can submit a new recipe and it persists across page \
reloads. Use the easynet-pages-author skill (.claude/skills/easynet-pages-author/) \
for the frontend layout and api/<verb>.toml manifest shape. Use the \
easynet-ability-author skill (.claude/skills/easynet-ability-author/) to write \
real backend abilities — list_recipes, add_recipe — that read/write a JSON \
file under ~/.easynet/web-apps/recipes/data/. The api/<verb>.toml manifests \
MUST use kind=\"ability\" pointing at the deployed abilities. Project_id = \
recipes. EASYNET_PAGES_USER=alice. After deploy, curl POST /api/add_recipe \
with a real recipe and curl /api/list_recipes to confirm persistence. \
Bash(easynet:*) and Bash(curl:*) are pre-approved. Tell me the URL when done."

PROMPT="${PROMPT:-$PROMPT_DEFAULT}"
AGENT="${AGENT:-web-builder}"
PROJECT="${PROJECT:-recipes}"

step "1. Confirm $AGENT agent is registered"
if ! "$EASYNET" agent list 2>&1 | grep -q "^  $AGENT "; then
    note "  agent '$AGENT' not found — registering now"
    run "$EASYNET" agent add "$AGENT" --type claude-code --label "网站搭建 — full-stack"
else
    ok "$AGENT is registered"
fi

step "2. Confirm seeded skills exist in agent's workspace"
WORKSPACE="$HOME/.easynet/workspaces/$AGENT"
SKILLS_DIR="$WORKSPACE/.claude/skills"
[ -d "$SKILLS_DIR/easynet-pages-author" ] && ok "easynet-pages-author skill seeded"     || warn "missing pages-author skill"
[ -d "$SKILLS_DIR/easynet-ability-author" ] && ok "easynet-ability-author skill seeded" || warn "missing ability-author skill"
[ -d "$SKILLS_DIR/easynet-collaborate" ] && ok "easynet-collaborate skill seeded"       || warn "missing collaborate skill"
pause

step "3. Clean slate — drop any prior $PROJECT publish"
run "$EASYNET" pages delete "$PROJECT" --force >/dev/null 2>&1 || true
rm -rf "$WEBAPPS_DIR/$PROJECT"
ok "$WEBAPPS_DIR/$PROJECT cleared"
pause

step "4. Dispatch the agent — this is a real LLM call (~30k tokens, 1-3 min)"
note "  prompt:"
echo "$PROMPT" | fold -s -w 70 | sed "s/^/    /"
echo
note "  invoking $AGENT.chat ..."
TS=$(date +%s)
DISPATCH_OUT=$(mktemp -t easynet-d2.XXXXXX.json)
trap 'rm -f "$DISPATCH_OUT"' EXIT
EASYNET_PAGES_USER="$USER_ID" "$EASYNET" ability invoke "$AGENT.chat" \
    --args "$(jq -n --arg p "$PROMPT" '{prompt:$p}')" > "$DISPATCH_OUT" 2>&1
RC=$?
ELAPSED=$(( $(date +%s) - TS ))
if [ "$RC" = 0 ]; then
    ok "dispatch finished in ${ELAPSED}s"
else
    fail "dispatch exited $RC"
    tail -30 "$DISPATCH_OUT" >&2
    exit 1
fi
pause

step "5. What did the agent build?"
ls -la "$WEBAPPS_DIR/$PROJECT" 2>&1 | head -10 | sed "s/^/    /"
echo
note "  api manifests:"
for f in "$WEBAPPS_DIR/$PROJECT"/api/*.toml; do
    [ -f "$f" ] || continue
    echo "    $(basename $f):"
    head -3 "$f" | sed "s/^/      /"
done
pause

step "6. Verify $PROJECT is published"
if "$EASYNET" pages list 2>&1 | grep -q "$PROJECT"; then
    ok "$PROJECT published"
    URL="http://$PROJECT.$USER_ID.pages.localhost:$PORT/"
else
    warn "agent didn't run easynet pages create — try yourself:"
    note "  $EASYNET pages create $PROJECT --folder $WEBAPPS_DIR/$PROJECT"
    URL=""
fi
pause

if [ -n "$URL" ]; then
    step "7. Open $URL in the browser"
    note "$URL"
    open_browser "$URL"
fi

echo
ok "demo #2 done.  agent built + deployed a working app, no凉冰 hand-holding."
note "  reply file: $DISPATCH_OUT"
note "  receipts:    look for canonical receipts on $AGENT.chat (ability_id), then walk down via causal_context."
