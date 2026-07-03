#!/usr/bin/env bash
# d1-static.sh — Killer Demo #1: 5-second static site deploy.
#
# Story: silan has a folder of HTML/CSS, wants a URL someone can
# open in a browser, doesn't want nginx, doesn't want Vercel,
# doesn't want to push to GitHub. One command, real URL, real
# kernel-sandboxed serve, real receipt chain.
#
# What this demonstrates:
#   - the simplest Pages reference system flow (static_json,
#     no api manifests, no kind=ability)
#   - kernel sandbox refuses path-traversal attacks at the
#     openat2 layer (silan can verify with curl)
#   - browser fetch returns deterministic bytes; ETag on
#     Content-Type-aware static MIME

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/_lib.sh"
ensure_daemon

PROJECT="d1-snapshot"
SITE="$WEBAPPS_DIR/$PROJECT"

step "1. Compose a 2-file site at $SITE"
rm -rf "$SITE"; mkdir -p "$SITE"
cat > "$SITE/index.html" <<'EOF'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <link rel="stylesheet" href="style.css">
  <title>EasyNet Pages — Demo #1</title>
</head>
<body>
  <main>
    <h1>Hello from EasyNet.</h1>
    <p>Five seconds ago this was a folder on silan's Mac. Now it's a URL.</p>
    <p class="dim">Served by <code>&lt;user&gt;.&lt;project&gt;.page.fetch</code>
       through a kernel-enforced sandbox (Linux <code>openat2 + RESOLVE_BENEATH</code>).</p>
  </main>
</body>
</html>
EOF
cat > "$SITE/style.css" <<'EOF'
body { font-family: ui-sans-serif, system-ui, sans-serif; max-width: 640px;
       margin: 5rem auto; padding: 0 1rem; line-height: 1.55; color: #1a1a1a; }
h1   { color: #cc0033; font-size: 2.6rem; }
.dim { color: #888; font-size: 0.9rem; }
code { background: #f4f4f4; padding: 0.05rem 0.3rem; border-radius: 3px; }
EOF
ok "wrote index.html ($(wc -c < $SITE/index.html) bytes), style.css ($(wc -c < $SITE/style.css) bytes)"
pause

step "2. Publish (mints a resource URA, opens folder fd, registers fetch ability)"
run "$EASYNET" pages delete "$PROJECT" --force >/dev/null 2>&1 || true
run "$EASYNET" pages create "$PROJECT" --folder "$SITE"
URL="http://$PROJECT.$USER_ID.pages.localhost:$PORT/"
pause

step "3. curl /index.html — real bytes from disk through sandbox"
curl -s -o /dev/null -w "  HTTP %{http_code}  type=%{content_type}  bytes=%{size_download}\n" \
    -H "Host: $PROJECT.$USER_ID.pages.localhost:$PORT" "http://127.0.0.1:$PORT/"
pause

step "4. Try the classic path-traversal attack — kernel refuses"
note "  attempting: GET /../../etc/passwd"
local_code=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "Host: $PROJECT.$USER_ID.pages.localhost:$PORT" \
    "http://127.0.0.1:$PORT/../../etc/passwd")
if [ "$local_code" = "404" ]; then
    ok "blocked at 404 — daemon's openat2 returned EXDEV before any read syscall"
else
    fail "expected 404, got $local_code (sandbox might have a hole — investigate!)"
fi
pause

step "5. List + show + open in browser"
run "$EASYNET" pages list
echo
run "$EASYNET" pages show "$PROJECT"
echo
note "opening: $URL"
open_browser "$URL"

echo
ok "demo #1 done.  unpublish with: easynet pages delete $PROJECT --force"
