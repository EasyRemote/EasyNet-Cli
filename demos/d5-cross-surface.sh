#!/usr/bin/env bash
# d5-cross-surface.sh — Killer Demo #5: pages + LLM-API in one chain.
#
# Story: a single project on EasyNet exposes BOTH a static
# website AND an OpenAI-compatible model. The same EasyNet
# user owns the project's frontend, the project's api/<verb>
# manifests, AND the chat-base ability that's exposed as a
# model. The receipt store records each step; one bearer
# capability + the correct project_id reaches everything.
#
# What this demonstrates:
#   - Pages and LLM-API are not two separate features bolted
#     together; they're two views over the SAME ability graph.
#   - The two RFCs (006-B Pages, 006-C OpenAI compat) share the
#     same paradigm (external protocol = transport view over
#     invocation graph). This demo wires both onto one project.
#
# What we'll build:
#   project_id = "ai-sandbox"
#   frontend  : a single-page chat UI (HTML/CSS/JS that talks
#               to OpenAI-shape /v1/chat/completions)
#   backend   : there isn't one — the page just calls /v1/...
#               directly with the operator's bearer
#
# The killer thing: this whole site's BACKEND is "every chat-base
# ability on the daemon, served as OpenAI". Add a new agent →
# the chat UI sees a new model in the dropdown, no redeploy.

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/_lib.sh"
ensure_daemon

PROJECT="ai-sandbox"
SITE="$WEBAPPS_DIR/$PROJECT"

step "1. Mint (or reuse) a key"
TOKEN=$(grep default_token "$HOME/.easynet/api_keys.local.toml" 2>/dev/null | cut -d'"' -f2)
if [ -z "$TOKEN" ]; then
    run "$EASYNET" api-key create --label "demo-5-sandbox"
    TOKEN=$(grep default_token "$HOME/.easynet/api_keys.local.toml" | cut -d'"' -f2)
else
    ok "reusing cached key"
fi
pause

step "2. Compose the chat UI (uses /v1 from the SAME hub that serves the page)"
rm -rf "$SITE"; mkdir -p "$SITE"

cat > "$SITE/index.html" <<'EOF'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <link rel="stylesheet" href="style.css">
  <title>EasyNet AI Sandbox</title>
</head>
<body>
  <header>
    <h1>EasyNet AI Sandbox</h1>
    <p class="dim">A chat UI talking to <code>/v1/chat/completions</code> on this same daemon.
       Whatever chat-base abilities the daemon registers show up in the model picker.</p>
  </header>
  <main>
    <div id="controls">
      <label>Model: <select id="model"></select></label>
      <button id="refresh">↻ Reload models</button>
    </div>
    <div id="conversation"></div>
    <form id="composer">
      <textarea id="input" placeholder="Ask anything..." rows="3"></textarea>
      <button id="send" type="submit">Send</button>
    </form>
  </main>
  <script src="app.js"></script>
</body>
</html>
EOF

cat > "$SITE/style.css" <<'EOF'
:root { --accent: #2f6b4f; --bg: #faf7f2; --card: #fff; --line: #e3ddd1;
        --muted: #6b7a70; --ink: #1f2a23; }
* { box-sizing: border-box; }
body { margin: 0; font-family: ui-sans-serif, system-ui, sans-serif; background: var(--bg);
       color: var(--ink); line-height: 1.55; }
header { background: var(--card); border-bottom: 1px solid var(--line); padding: 1.5rem 2rem; }
header h1 { margin: 0; color: var(--accent); }
header .dim { color: var(--muted); font-size: 0.9rem; margin: 0.4rem 0 0; }
header code { background: var(--bg); padding: 0.05rem 0.35rem; border-radius: 3px; }
main { max-width: 880px; margin: 1.5rem auto; padding: 0 1.5rem; }
#controls { display: flex; gap: 1rem; align-items: center; margin-bottom: 1rem;
            font-size: 0.9rem; color: var(--muted); }
#controls select { padding: 0.3rem 0.5rem; }
#controls button { padding: 0.3rem 0.7rem; cursor: pointer; }
#conversation { display: flex; flex-direction: column; gap: 0.75rem;
                min-height: 300px; padding: 1rem; background: var(--card);
                border: 1px solid var(--line); border-radius: 8px; }
.msg { padding: 0.5rem 0.75rem; border-radius: 6px; max-width: 85%; }
.msg.user      { align-self: flex-end; background: #d8f0e3; }
.msg.assistant { align-self: flex-start; background: #f4f4f4; white-space: pre-wrap; }
#composer { display: flex; gap: 0.5rem; margin-top: 1rem; }
#composer textarea { flex: 1; padding: 0.6rem; border: 1px solid var(--line);
                     border-radius: 6px; font: inherit; resize: vertical; }
#composer button { padding: 0.6rem 1.2rem; background: var(--accent); color: white;
                   border: none; border-radius: 6px; cursor: pointer; }
EOF

cat > "$SITE/app.js" <<'EOF'
// Talk to /v1/chat/completions on the same origin.
// The Bearer token is read from a query param at first load:
//   open the URL with ?key=easynet-sk-... once; we store it in localStorage.
const params = new URLSearchParams(location.search);
if (params.get('key')) {
  localStorage.setItem('easynet-key', params.get('key'));
  history.replaceState({}, '', location.pathname);
}
const TOKEN = localStorage.getItem('easynet-key') || '';

const modelSel = document.getElementById('model');
const refreshBtn = document.getElementById('refresh');
const convo = document.getElementById('conversation');
const input = document.getElementById('input');
const form  = document.getElementById('composer');

async function loadModels() {
  modelSel.innerHTML = '';
  const r = await fetch('/v1/models');
  const j = await r.json();
  for (const m of j.data || []) {
    const opt = document.createElement('option');
    opt.value = m.id; opt.textContent = m.id;
    modelSel.appendChild(opt);
  }
}
refreshBtn.addEventListener('click', loadModels);
loadModels();

const history = [];

function addMsg(role, text) {
  const el = document.createElement('div');
  el.className = 'msg ' + role;
  el.textContent = text;
  convo.appendChild(el);
  convo.scrollTop = convo.scrollHeight;
  return el;
}

form.addEventListener('submit', async (e) => {
  e.preventDefault();
  const text = input.value.trim();
  if (!text) return;
  input.value = '';
  history.push({ role: 'user', content: text });
  addMsg('user', text);

  const out = addMsg('assistant', '');
  const r = await fetch('/v1/chat/completions', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': 'Bearer ' + TOKEN,
    },
    body: JSON.stringify({
      model: modelSel.value,
      messages: history,
      stream: true,
    }),
  });
  if (!r.ok) {
    out.textContent = `Error: HTTP ${r.status}. Pass ?key=easynet-sk-... in the URL once to authorise.`;
    return;
  }
  // Read SSE stream
  const reader = r.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    let i;
    while ((i = buf.indexOf('\n\n')) !== -1) {
      const evt = buf.slice(0, i); buf = buf.slice(i + 2);
      if (!evt.startsWith('data: ')) continue;
      const payload = evt.slice(6).trim();
      if (payload === '[DONE]') return;
      try {
        const j = JSON.parse(payload);
        const delta = j.choices?.[0]?.delta?.content;
        if (delta) out.textContent += delta;
      } catch {}
    }
  }
  history.push({ role: 'assistant', content: out.textContent });
});
EOF

ok "wrote $(ls $SITE | wc -l | tr -d ' ') files"
pause

step "3. Publish (the page lives on the SAME hub that exposes /v1)"
run "$EASYNET" pages delete "$PROJECT" --force >/dev/null 2>&1 || true
run "$EASYNET" pages create "$PROJECT" --folder "$SITE"
URL_BASE="http://$PROJECT.$USER_ID.pages.localhost:$PORT/"

step "4. Open the chat UI with the bearer baked into the URL"
URL_KEYED="${URL_BASE}?key=$TOKEN"
note "$URL_KEYED"
note "  (the page lifts ?key=... into localStorage, then strips it from the URL)"
open_browser "$URL_KEYED"
pause

step "5. So what's actually happening behind the scenes?"
note "  Browser   →   $PROJECT.$USER_ID.pages.localhost:$PORT/index.html"
note "                 ↑ pages_listener routes by Host"
note "  Browser   →   /v1/chat/completions"
note "                 ↑ pages_listener routes /v1/* to OpenAI adapter,"
note "                   IGNORING the Host (realm-level endpoint)"
note "  Adapter   →   forward_invoke <model_name>.chat"
note "  Receipt   :   one canonical auth receipt (api_key URA),"
note "                one chat-base operational receipt (causal_context cites it)"

echo
ok "demo #5 done.  one project, two surfaces, one paradigm."
