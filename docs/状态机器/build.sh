#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if command -v xelatex >/dev/null 2>&1; then
  XELATEX="$(command -v xelatex)"
elif [ -x /opt/homebrew/bin/xelatex ]; then
  XELATEX="/opt/homebrew/bin/xelatex"
else
  echo "xelatex not found. Install MacTeX/BasicTeX or expose /opt/homebrew/bin/xelatex in PATH." >&2
  exit 127
fi

cd "$ROOT"
for tex in *.tex; do
  "$XELATEX" -interaction=nonstopmode -halt-on-error "$tex"
  "$XELATEX" -interaction=nonstopmode -halt-on-error "$tex"
done
rm -f ./*.aux ./*.out ./*.toc ./*.log
