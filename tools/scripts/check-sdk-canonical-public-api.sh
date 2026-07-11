#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="$ROOT/sdk/conformance/canonical-public-api.json"
PYTHON_BIN="${PYTHON:-python3}"

"$PYTHON_BIN" - "$MANIFEST" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
if manifest.get("schema_version") != 1:
    raise SystemExit("canonical-public-api: unsupported schema version")
languages = manifest.get("languages")
if not isinstance(languages, dict) or set(languages) != {"go", "python"}:
    raise SystemExit("canonical-public-api: expected exactly go and python")
for language, symbols in languages.items():
    if not isinstance(symbols, list) or not symbols or symbols != sorted(set(symbols)):
        raise SystemExit(f"canonical-public-api: {language} symbols must be non-empty, sorted and unique")
PY

while IFS= read -r symbol; do
  (cd "$ROOT/sdk/go" && go doc ".$symbol" >/dev/null) || {
    echo "canonical-public-api: missing Go symbol $symbol" >&2
    exit 1
  }
done < <(jq -r '.languages.go[]' "$MANIFEST")

PYTHONPATH="$ROOT/sdk/python:$ROOT/../EasyNet-Axon/sdk/python${PYTHONPATH:+:$PYTHONPATH}" "$PYTHON_BIN" - "$MANIFEST" <<'PY'
import json
import sys
from pathlib import Path

import easynet_sdk

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
exports = set(getattr(easynet_sdk, "__all__", ()))
missing = [symbol for symbol in manifest["languages"]["python"] if symbol not in exports]
if missing:
    raise SystemExit("canonical-public-api: missing Python symbols: " + ", ".join(missing))
PY

echo "canonical-public-api: OK"
