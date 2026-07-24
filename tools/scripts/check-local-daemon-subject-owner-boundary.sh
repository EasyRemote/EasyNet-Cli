#!/usr/bin/env bash
#
# Guard local daemon subject binding ownership.

set -euo pipefail

ROOT="${CHECK_LOCAL_DAEMON_SUBJECT_OWNER_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-local-daemon-subject-owner-boundary: $*" >&2
    exit 1
}

ISSUER_RS="src/support/platform/local_invoke.rs"
TRANSPORT_RS="src/support/platform/local_daemon_grpc.rs"
IDENTITY_RS="src/daemon/identity/local_invocation.rs"
[[ -f "$ISSUER_RS" ]] || fail "missing $ISSUER_RS"
[[ -f "$TRANSPORT_RS" ]] || fail "missing $TRANSPORT_RS"
[[ -f "$IDENTITY_RS" ]] || fail "missing $IDENTITY_RS"

grep -Fq 'pub fn local_daemon_identity_subject_ura() -> anyhow::Result<String>' "$ISSUER_RS" \
    || fail "LocalDaemonSystemAbilityIssuer must own the local daemon subject helper"

grep -Fq 'crate::daemon::identity::local_invocation::local_daemon_ura()' "$ISSUER_RS" \
    || fail "LocalDaemonSystemAbilityIssuer must source subject identity from daemon::identity::local_invocation"

grep -Fq 'local_invocation_capability_unsupported_error(' "$ISSUER_RS" \
    || fail "feature-off subject resolution must use canonical unsupported capability state"

grep -Fq 'local invocation provider is unavailable; capability_state=unsupported' "$ISSUER_RS" \
    || fail "feature-off local invocation diagnostics must expose unsupported capability state"

grep -Fq 'pub(crate) fn local_daemon_ura() -> anyhow::Result<String>' "$IDENTITY_RS" \
    || fail "daemon::identity::local_invocation must own local daemon URA construction"

if rg -n 'fn local_daemon_identity_subject_ura\s*\(' "$TRANSPORT_RS"; then
    fail "local daemon gRPC transport must not expose subject-selection helpers"
fi

if rg -n 'local_daemon_grpc::local_daemon_identity_subject_ura' "$ISSUER_RS" src \
    --glob '!src/support/platform/local_daemon_grpc.rs' 2>/dev/null; then
    fail "callers must not source authority subjects from the gRPC transport module"
fi

if rg -n 'requires the `axon-pb` feature|rebuild with `cargo build --features axon-pb`' "$ISSUER_RS" 2>/dev/null; then
    fail "local subject owner must not expose retired compile-feature fallback wording"
fi

echo "check-local-daemon-subject-owner-boundary: ok"
