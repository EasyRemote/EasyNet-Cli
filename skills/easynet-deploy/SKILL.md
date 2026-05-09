---
name: easynet-deploy
description: Build, deploy, and manage the EasyNet platform itself — backend (Go) + frontend (React) + Axon Hub on the production server, plus local dev workflows. Use when the user asks to deploy / restart / build EasyNet services, push changes to device.easynet.run, debug the production server, or run local backend/frontend dev — OR when YOU notice the user is editing platform code and probably needs to redeploy after.
allowed-tools: [Bash, Read]
---

# EasyNet Deploy

Build, deploy, and run the EasyNet platform across local dev + production server. This skill is about deploying **EasyNet itself** (`backend/` + `Frontend/` + `Axon Hub`), not about deploying user-authored abilities (that's `easynet-author`).

## When This Skill Activates

### User-prompted triggers

- The user says "deploy backend / frontend / EasyNet to device.easynet.run"
- The user says "restart axon-runtime / easynet-api on the prod server"
- The user says "build EasyNet locally" / "run dev server"
- The user mentions the production server `107.174.92.163` or domain `device.easynet.run`
- The user asks how the prod server is configured (systemd units, Nginx, BaoTa)
- The user has a backend / frontend code change and needs to push it

### Self-prompted triggers

- The user has just finished editing `backend/` Go files or `Frontend/` React code without explicit deploy intent — confirm whether they want to deploy.
- The user references "the deployed version" or "what's running on the server" — verify by curling `https://easynet.run/api/v1/health` rather than guessing.
- The user is debugging a behavior gap between local and prod — load the systemd / Nginx context first; the gap is usually env vars or service state.

## Process

### 1. Identify the deploy scope

```
backend only    → rebuild Go binary on the prod server (CGO + Linux .so)
frontend only   → npm build locally, scp the dist
both            → use the bundled deploy.sh helper
service ops     → systemctl restart / status / logs
```

### 2. Use the bundled deploy script when possible

```bash
${CLAUDE_SKILL_DIR}/scripts/deploy.sh all          # backend + frontend
${CLAUDE_SKILL_DIR}/scripts/deploy.sh backend      # backend only
${CLAUDE_SKILL_DIR}/scripts/deploy.sh frontend     # frontend only
```

For one-off operations, use the explicit commands below.

### 3. Health check after deploy

```bash
curl -sf https://easynet.run/api/v1/health
ssh root@107.174.92.163 "systemctl is-active axon-runtime easynet-api"
```

A 200 from the health endpoint plus `active` from both services is the green light. If either fails, jump to the logs.

## Repository layout

| Path | Purpose |
|---|---|
| `backend/` | Go (go-zero + Ent ORM) |
| `Frontend/` | React + Vite + TypeScript + Tailwind |
| `../EasyNet-Axon` (sibling repo) | Axon Hub source |
| `device.easynet.run/axon/sdk/go@v0.27.14` | Go module published from EasyNet-Axon (Dendrite bridge) |

Repo: `https://github.com/EasyRemote/EasyNet.git`

## Production server

| | |
|---|---|
| Host | `107.174.92.163` |
| SSH | `ssh root@107.174.92.163` (key-based, no password) |
| Domain | `device.easynet.run` (Let's Encrypt, BaoTa-managed) |
| Panel | BaoTa at `https://107.174.92.163:8888` |
| OS | Ubuntu 24.04 x86_64 |

### File layout

```
/www/wwwroot/EasyNet.run/
├── index.html               # Frontend (Vite output)
├── assets/                  # Frontend bundles
├── platform/                # OS logos
└── api/
    ├── easynet              # Backend binary (CGO, links libaxon_dendrite_bridge.so)
    ├── axon-runtime         # Axon Hub binary
    ├── etc/easynet-api.yaml # Backend config (prod)
    └── native/libaxon_dendrite_bridge.so
```

### Systemd services

| Service | Unit | Port | Purpose |
|---|---|---|---|
| Axon Hub | `axon-runtime.service` | `:50051` (public) | Device registration, gRPC dispatch |
| Backend API | `easynet-api.service` | `:8080` (localhost; Nginx proxies `/api/`) | Go API |
| PostgreSQL | `postgresql.service` | `:5432` | DB `easynet`, user `postgres:postgres` |

### Nginx config

`/www/server/panel/vhost/nginx/easynet.run.conf`. Key behaviours:

- `/api/` → reverse proxy to `127.0.0.1:8080` (with WebSocket upgrade for terminal endpoints)
- `/` → SPA fallback (`try_files $uri /index.html`)
- `/axon/sdk/go` → Go-module import-path redirect

### Service env vars (in `easynet-api.service` unit)

```
POSTGRES_DSN=postgres://postgres:postgres@localhost:5432/easynet?sslmode=disable
EASYNET_ACCESS_SECRET=<rotate-me>
AXON_PUBLIC_ENDPOINT=axon://easynet.run:50051
EASYNET_DENDRITE_BRIDGE_LIB=/www/wwwroot/EasyNet.run/api/native/libaxon_dendrite_bridge.so
LD_LIBRARY_PATH=/www/wwwroot/EasyNet.run/api/native
```

## Build & deploy commands

### Backend (compile ON the server — needs CGO + Linux `.so`)

```bash
# Local: package source
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend
tar --exclude='.git' -czf /tmp/backend-src.tar.gz .

# Upload + build on server
scp /tmp/backend-src.tar.gz root@107.174.92.163:/tmp/
ssh root@107.174.92.163 "
  rm -rf /tmp/backend-build && mkdir -p /tmp/backend-build
  cd /tmp/backend-build && tar -xzf /tmp/backend-src.tar.gz 2>/dev/null
  CGO_ENABLED=1 go build -o /www/wwwroot/EasyNet.run/api/easynet device.easynet.go
  systemctl restart easynet-api
"
```

### Frontend

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend
VITE_APP_MODE=release npm run build

tar -czf /tmp/frontend-dist.tar.gz -C dist .
scp /tmp/frontend-dist.tar.gz root@107.174.92.163:/tmp/
ssh root@107.174.92.163 "
  cd /www/wwwroot/EasyNet.run
  rm -rf assets index.html
  tar -xzf /tmp/frontend-dist.tar.gz 2>/dev/null
  chown -R www:www /www/wwwroot/EasyNet.run/ 2>/dev/null
"
```

### Service ops

```bash
# Status
ssh root@107.174.92.163 "systemctl status axon-runtime easynet-api --no-pager"

# Restart
ssh root@107.174.92.163 "systemctl restart axon-runtime easynet-api"

# Logs
ssh root@107.174.92.163 "journalctl -u easynet-api  --no-pager -n 80"
ssh root@107.174.92.163 "journalctl -u axon-runtime --no-pager -n 80"

# Health
curl -sf https://easynet.run/api/v1/health
```

## Local development

### Backend

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend
go run device.easynet.go -f etc/easynet-api.yaml
# :8080, talks to local Axon at :50051
```

### Frontend

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/Frontend
npm run dev
# :5173, proxies /api/* to :8080
```

### Local Axon runtime federating to production Hub

```bash
AXON_BIND=127.0.0.1:50052 \
AXON_ENFORCE_MTLS=false \
AXON_HUB=axon://107.174.92.163:50051 \
AXON_FEDERATION_TENANT=easynet-platform \
AXON_FEDERATION_LABEL=$(hostname) \
/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/target/release/axon-runtime
```

### Ent code generation (after schema changes)

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend
go generate ./ent
```

## Native artifacts (Dendrite bridge + Axon runtime)

Published as GitHub Release assets on EasyNet-Axon. Available platforms:
`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`.

```bash
cd /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon
gh release download v0.27.14 \
  --pattern "dendrite-bridge-x86_64-unknown-linux-gnu.tar.gz" \
  --pattern "runtime-x86_64-unknown-linux-gnu.tar.gz" \
  --dir /tmp/axon-linux
```

## Key API endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/auth/register` | – | Register user |
| POST | `/api/v1/auth/login` | – | Login (returns JWT) |
| POST | `/api/v1/devices/pairing` | JWT | Create pairing session |
| POST | `/api/v1/devices/pairing/:token/validate` | – | Device registers via token |
| GET | `/api/v1/devices` | JWT | List devices |
| GET | `/api/v1/devices/:id` | JWT | Get device detail |
| GET | `/api/v1/devices/:id/abilities` | JWT | List device abilities |
| POST | `/api/v1/devices/:id/exec` | JWT | Execute command |
| DELETE | `/api/v1/devices/:id` | JWT | Remove device |
| GET | `/api/v1/health` | – | Health check |

## Examples

### Example: deploy a backend change

**Input:**
```
User: I just fixed the device-pairing race condition, push it to prod.
```

**Process:**
1. Verify the fix is committed locally (`git status` clean).
2. Run `${CLAUDE_SKILL_DIR}/scripts/deploy.sh backend`.
3. Health check: `curl -sf https://easynet.run/api/v1/health`.
4. Confirm: `ssh root@107.174.92.163 "systemctl is-active easynet-api"` returns `active`.

### Example: self-prompted check

**Input:**
```
User has been editing backend/internal/handler/devices.go for 30 min and now says "looks good".
```

**Process:**
1. Self-trigger: edits to platform code without explicit deploy → ask the user.
2. "Want me to deploy this to device.easynet.run? Or is this for local-dev testing only?"
3. Wait for confirmation before running deploy.

## Architecture notes

- Backend talks to Axon Hub via `localhost:50051` (Dendrite bridge, dlopen at runtime — that's why CGO is required at build time).
- Pairing returns `PublicEndpoint` (`axon://easynet.run:50051`) so remote devices can connect from anywhere.
- Device metadata (OS, arch) lives in PostgreSQL `device_pairing` table, merged with Axon's live state at read time.
- Remote command execution falls back to A2A (`SendA2ATask`) when local `ExecCommand` returns NODE_NOT_FOUND.
- Frontend is a React SPA; API calls use relative paths (`/api/v1/…`) so Nginx reverse-proxies cleanly.

## Notes

- **Don't** edit the prod server file system directly; always go through the deploy commands so the source of truth is the git repo.
- Backend rebuild is **server-side** because it needs `libaxon_dendrite_bridge.so` for the matching Linux x86_64. A macOS-built binary won't run.
- Frontend rebuild is **local** because Vite's dist is platform-agnostic.
- BaoTa controls SSL renewal and the Nginx vhost. If SSL expires, log into BaoTa rather than editing certs by hand.
