#!/usr/bin/env bash
#
# Guard CrossHubDialer against boot-time trust-anchor snapshot mode.

set -euo pipefail

ROOT="${CHECK_CROSS_HUB_TRUST_SOURCE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-cross-hub-trust-source-boundary: $*" >&2
    exit 1
}

DIALER_RS="src/daemon/federation/client/cross_hub_dial.rs"
BOOT_RS="src/daemon/boot/invocation/mod.rs"

[[ -f "$DIALER_RS" ]] || fail "missing $DIALER_RS"
[[ -f "$BOOT_RS" ]] || fail "missing $BOOT_RS"

grep -q 'trust_anchor: SharedTrustAnchor' "$DIALER_RS" \
    || fail "CrossHubDialer must store the live SharedTrustAnchor cell"

grep -q 'pub fn with_trust_anchor_cell(cell: SharedTrustAnchor) -> Self' "$DIALER_RS" \
    || fail "CrossHubDialer construction must require SharedTrustAnchor"

grep -q 'let trust_snapshot = self.trust_anchor.snapshot();' "$DIALER_RS" \
    || fail "CrossHubDialer must snapshot the live trust cell per dial"

grep -q 'self.trust_anchor.cert_anchor_generation()' "$DIALER_RS" \
    || fail "CrossHubDialer channel cache generation must come from SharedTrustAnchor"

grep -q 'CrossHubDialer::with_trust_anchor_cell' "$BOOT_RS" \
    || fail "daemon boot must wire CrossHubDialer with SharedTrustAnchor"

bad="$(
    grep -nE 'enum TrustSource|TrustSource::|pub fn new\(trust_anchor: Arc<RealmTrustAnchor>\)|Self::from_trust_source|Snapshot\(Arc<RealmTrustAnchor>\)|boot-time snapshot|snapshot constructor|snapshot-flavour' \
        "$DIALER_RS" "$BOOT_RS" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "cross-hub dialer still exposes a retired snapshot trust-source path:
$bad"
fi

echo "check-cross-hub-trust-source-boundary: ok"
