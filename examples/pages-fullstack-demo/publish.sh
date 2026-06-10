#!/usr/bin/env bash
# publish.sh — publish this front+back demo folder via the easynet CLI.
#
# Prereqs: a running daemon joined to a hub (`easynet status` is green).
# Usage:
#   ./publish.sh                 # publishes as project "fullstack-demo"
#   PROJECT=mydemo ./publish.sh  # custom project id
#
# Author: Silan.Hu <silan.hu@u.nus.edu>
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EASYNET="${EASYNET:-easynet}"
PROJECT="${PROJECT:-fullstack-demo}"

echo "==> publishing $HERE as project '$PROJECT'"

# Idempotent: drop a prior publish of the same id, then create.
"$EASYNET" pages delete "$PROJECT" --force >/dev/null 2>&1 || true
"$EASYNET" pages create "$PROJECT" --folder "$HERE"

echo
echo "==> URL:"
"$EASYNET" pages url "$PROJECT"
echo
echo "Open that URL in a browser. The product list comes from"
echo "GET api/products; the feedback form POSTs to api/feedback."
echo "Unpublish with: $EASYNET pages delete $PROJECT --force"
