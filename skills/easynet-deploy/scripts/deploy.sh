#!/usr/bin/env bash
# EasyNet Deploy Script
# Usage: deploy.sh [backend|frontend|all|status|logs|restart]

set -euo pipefail

SERVER="root@107.174.92.163"
EASYNET_DIR="/Users/macbook.silan.tech/Documents/GitHub/EasyNet"
REMOTE_DIR="/www/wwwroot/EasyNet.run"

deploy_backend() {
  echo "==> Building backend on server..."
  cd "$EASYNET_DIR/backend"
  tar --exclude='.git' -czf /tmp/backend-src.tar.gz .
  scp /tmp/backend-src.tar.gz "$SERVER:/tmp/"
  ssh "$SERVER" "
    rm -rf /tmp/backend-build && mkdir -p /tmp/backend-build
    cd /tmp/backend-build && tar -xzf /tmp/backend-src.tar.gz 2>/dev/null
    CGO_ENABLED=1 go build -o $REMOTE_DIR/api/easynet easynet.go
    systemctl restart easynet-api
    sleep 2
    systemctl is-active easynet-api && echo 'Backend: OK' || echo 'Backend: FAILED'
  "
}

deploy_frontend() {
  echo "==> Building frontend..."
  cd "$EASYNET_DIR/Frontend"
  VITE_APP_MODE=release npm run build 2>&1 | tail -3
  tar -czf /tmp/frontend-dist.tar.gz -C dist .
  scp /tmp/frontend-dist.tar.gz "$SERVER:/tmp/"
  ssh "$SERVER" "
    cd $REMOTE_DIR && rm -rf assets index.html
    tar -xzf /tmp/frontend-dist.tar.gz 2>/dev/null
    chown -R www:www $REMOTE_DIR/ 2>/dev/null
    echo 'Frontend: OK'
  "
}

show_status() {
  echo "==> Service Status"
  ssh "$SERVER" "systemctl status axon-runtime easynet-api --no-pager | head -20"
  echo ""
  echo "==> Health Check"
  curl -s "https://easynet.run/api/v1/health"
  echo ""
}

show_logs() {
  local service="${1:-easynet-api}"
  ssh "$SERVER" "journalctl -u $service --no-pager -n 50"
}

restart_services() {
  ssh "$SERVER" "systemctl restart axon-runtime easynet-api"
  sleep 2
  show_status
}

case "${1:-help}" in
  backend)  deploy_backend ;;
  frontend) deploy_frontend ;;
  all)
    deploy_backend
    deploy_frontend
    echo ""
    show_status
    ;;
  status)   show_status ;;
  logs)     show_logs "${2:-easynet-api}" ;;
  restart)  restart_services ;;
  *)
    echo "Usage: $0 [backend|frontend|all|status|logs|restart]"
    echo ""
    echo "  backend   - Build and deploy Go backend"
    echo "  frontend  - Build and deploy React frontend"
    echo "  all       - Deploy both"
    echo "  status    - Check service status"
    echo "  logs      - View logs (default: easynet-api, or: logs axon-runtime)"
    echo "  restart   - Restart all services"
    ;;
esac
