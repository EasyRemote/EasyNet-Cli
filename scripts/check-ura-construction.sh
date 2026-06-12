#!/usr/bin/env bash
# check-ura-construction.sh — F-055 valve: no raw URA construction in src/
# outside the facade.
#
# The Cli's other URA guards are per-surface shape pins; none of them ban
# tree-wide `format!("easynet:///r/...")` construction, which is how the
# step-2c receipt-URA fallback minted the F-042 wild shape unnoticed
# (ledger_projection.rs:484). This is the Cli twin of the backend valve
# (EasyNet b8b2114) and the frontend AST rule (EasyNet 793972a): one
# construction ban per repo, the facade (src/ura.rs) is the whitelist.
#
# Static literals (test fixtures, prefix checks) do not trip this valve —
# only format!-interpolated construction does, same scoping as the
# frontend rule.
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
    echo "check-ura-construction: $1" >&2
    exit 1
}

# Temporary exemptions, pinned by exact content, may be added here when a
# violation has a tracked fix in flight; a stale exemption (zero matches)
# turns the valve red so it cannot outlive its reason. Both founding
# exemptions retired within hours of landing: the F-042 receipt-fallback
# mint (fixed by 6e34457) and the hub/ability route-key fixture (RULED
# round 44: a wild shape, not a spec gap — the ownership gate already
# accepts the canonical `ability/hub.<ns>.<name>` form via parse_ura;
# 8ee1c49 swapped the fixture before the ruling even landed).
EXEMPT_PATTERNS=()

hits=$(grep -rn 'format!("easynet:///r/\|format!("{URA_SCHEME}' src/ --include='*.rs' \
    | grep -v '^src/ura\.rs:' \
    | grep -v ':[0-9]*: *//' || true)

for pattern in ${EXEMPT_PATTERNS[@]+"${EXEMPT_PATTERNS[@]}"}; do
    if ! grep -rqF "$pattern" src/ --include='*.rs'; then
        fail "stale exemption (its fix landed — delete it from this script): $pattern"
    fi
    hits=$(printf '%s\n' "$hits" | grep -vF "$pattern" || true)
done

hits=$(printf '%s\n' "$hits" | grep -v '^$' || true)
if [ -n "$hits" ]; then
    echo "raw URA construction outside src/ura.rs (use the facade builders):" >&2
    printf '%s\n' "$hits" >&2
    exit 1
fi

echo "check-ura-construction: ok (${#EXEMPT_PATTERNS[@]} tracked exemption(s))"
