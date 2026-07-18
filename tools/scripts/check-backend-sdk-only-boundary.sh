#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/backend/api" "$tmp/backend/internal/service" "$tmp/backend/internal/runtimecontract" "$tmp/backend/internal/logic/ability" "$tmp/backend/internal/pb/axon/v1" "$tmp/backend/internal/daemon_grpc" "$tmp/backend/internal/svc" "$tmp/backend/internal/types" "$tmp/Frontend/src/lib/api" "$tmp/Frontend/src/lib/crypto"
  cat >"$tmp/backend/go.mod" <<'EOF'
module easynet-backend

require (
    easynet.run/cli/sdk/go v0.0.0
    axon.run/sdk/go v0.0.0 // indirect
)

replace axon.run/sdk/go => ../../EasyNet-Axon/sdk/go
EOF
  cat >"$tmp/backend/internal/service/allowed.go" <<'EOF'
package service

import sdk "easynet.run/cli/sdk/go"

var _ = sdk.ErrInvalidArgument
EOF
  cat >"$tmp/Frontend/src/lib/crypto/signing-material.ts" <<'EOF'
export function signOpaqueCanonicalBytes(value: Uint8Array): Uint8Array {
  return value
}
EOF
  cat >"$tmp/backend/api/easynet.api" <<'EOF'
type SignedInvokeAbilityReq {
    DescriptorRef string `json:"descriptor_ref"`
}
EOF
  cat >"$tmp/backend/internal/types/types.go" <<'EOF'
package types

type SignedInvokeAbilityReq struct {
    DescriptorRef string `json:"descriptor_ref"`
}
EOF
  cat >"$tmp/backend/internal/logic/ability/prepared_invocation_lease.go" <<'EOF'
package ability

type preparedInvocationLeaseClaims struct {
    DescriptorRef string `json:"descriptor_ref"`
}

func claimsFromResponse(response struct{ DescriptorRef string }) preparedInvocationLeaseClaims {
    return preparedInvocationLeaseClaims{DescriptorRef: response.DescriptorRef}
}

func claimsFromRequest(request struct{ DescriptorRef string }) preparedInvocationLeaseClaims {
    return preparedInvocationLeaseClaims{DescriptorRef: request.DescriptorRef}
}
EOF
  cat >"$tmp/backend/internal/logic/ability/submitSignedInvocationLogic.go" <<'EOF'
package ability

func submit(req struct{ DescriptorRef string }) {
    _ = struct{ DescriptorRef string }{DescriptorRef: req.DescriptorRef}
}
EOF
  cat >"$tmp/backend/internal/svc/sdk_bidi_carrier.go" <<'EOF'
package svc

type SDKInvocationDraftRequest struct {
    DescriptorRef string
}

func projectExplicitDescriptorRef() {
    ProjectDescriptorRef()
}

func ProjectDescriptorRef() {}
EOF
  cat >"$tmp/Frontend/src/lib/api/easynet-abilities.ts" <<'EOF'
const signed = { prepared: { descriptor_ref: "descriptor" } }
export const submission = {
  descriptor_ref: signed.prepared.descriptor_ref,
}
EOF
  "$0" "$tmp" >/dev/null
  "$0" "$tmp/backend" >/dev/null
  cat >"$tmp/backend/internal/service/forbidden.go" <<'EOF'
package service

import (
    "os/exec"

    daemon "easynet-backend/internal/daemon_grpc"
    pb "easynet-backend/internal/pb/axon/v1"
    axonsdk "axon.run/sdk/go/axon"
)

var _ = daemon.Client{}
var _ = pb.InvokeRequest{}
var _ = axonsdk.ErrInvalidArgument

func boot() {
    _ = exec.Command("easynet-daemon")
    _ = "unix:///tmp/easynet-control.sock"
}
EOF
  cat >"$tmp/backend/internal/runtimecontract/resolve_projection.go" <<'EOF'
package runtimecontract

type legacyResolveInput struct {
    QueryName string `json:"queryName"`
}

var _ = `{"answerKind":"RESOLVE_ANSWER_KIND_FINAL_ROUTE"}`

func CanonicalInvocationBytes() {}
func CanonicalBytesForRequestSignature() {}
EOF
  cat >"$tmp/backend/internal/logic/ability/proof_authority.go" <<'EOF'
package ability

import (
    "crypto/ed25519"
    "crypto/sha256"
    "encoding/hex"
)

type preparedProofMaterial interface {
    CanonicalBytesBase64() string
    CanonicalHashHex() string
}

func verifyInvocationProof(material preparedProofMaterial, canonical, publicKey, signature []byte) {
    _ = material.CanonicalBytesBase64()
    digest, _ := hex.DecodeString(material.CanonicalHashHex())
    if len(digest) == 0 {
        fallback := sha256.Sum256(canonical)
        digest = fallback[:]
    }
    _ = ed25519.Verify(publicKey, canonical, signature)
    VerifyPrepareBindToken()
    verifyTrustRegistration()
    _ = digest
}

func VerifyPrepareBindToken() {}
func verifyTrustRegistration() {}
EOF
  cat >"$tmp/Frontend/src/lib/crypto/envelope-canonical.ts" <<'EOF'
export async function canonicalInvocationBytes(): Promise<Uint8Array> {
  return new Uint8Array()
}
EOF
  cat >"$tmp/Frontend/src/lib/api/easynet-abilities.ts" <<'EOF'
export const submission = {}
EOF
  cat >"$tmp/backend/internal/pb/axon/v1/generated.go" <<'EOF'
package v1

type Invocation struct{}
EOF
  cat >"$tmp/backend/internal/daemon_grpc/client.go" <<'EOF'
package daemon_grpc

type Client struct{}
EOF
  mkdir -p "$tmp/backend/internal/axon"
  cat >"$tmp/backend/internal/axon/legacy.go" <<'EOF'
package axon
EOF
  cat >>"$tmp/backend/go.mod" <<'EOF'
require axon.run/sdk/go v0.0.0
require easynet.run/axon/sdk/go v0.41.5 // indirect
replace easynet.run/axon/sdk/go => ../../EasyNet-Axon/sdk/go
EOF
  self_test_out="$tmp/backend-sdk-only-boundary-self-test.out"
  if "$0" "$tmp/backend" >"$self_test_out" 2>&1; then
    echo "self-test expected forbidden backend fixture to fail" >&2
    exit 1
  fi
  grep -Fq "raw_axon_module_dependency" "$self_test_out"
  if [[ "$(grep -Fc "legacy_axon_module_dependency" "$self_test_out")" -lt 2 ]]; then
    echo "self-test expected legacy Axon require and replace directives to fail" >&2
    exit 1
  fi
  grep -Fq "raw_axon_import" "$self_test_out"
  grep -Fq "generated_axon_pb_import" "$self_test_out"
  grep -Fq "generated_axon_pb_package" "$self_test_out"
  grep -Fq "direct_daemon_transport_import" "$self_test_out"
  grep -Fq "direct_daemon_transport_package" "$self_test_out"
  grep -Fq "raw_daemon_socket_marker" "$self_test_out"
  grep -Fq "runtime_subprocess" "$self_test_out"
  grep -Fq "legacy_backend_axon_facade" "$self_test_out"
  grep -Fq "retired_namespace_resolve_carrier_key" "$self_test_out"
  grep -Fq "backend_canonical_invocation_facade" "$self_test_out"
  grep -Fq "local_invocation_signature_verification" "$self_test_out"
  grep -Fq "local_invocation_digest_fallback" "$self_test_out"
  grep -Fq "local_invocation_prepare_proof_verification" "$self_test_out"
  grep -Fq "local_invocation_trust_verification" "$self_test_out"
  grep -Fq "frontend_canonical_invocation_encoder" "$self_test_out"
  grep -Fq "signed_invocation_descriptor_ref_boundary" "$self_test_out"
  echo "check-backend-sdk-only-boundary self-test ok"
  exit 0
fi

BACKEND_ROOT="${1:-${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}}"

python3 - "$BACKEND_ROOT" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path


def resolve_backend_root(candidate: Path) -> Path:
    candidate = candidate.resolve()
    if (candidate / "go.mod").exists():
        return candidate
    nested = candidate / "backend"
    if (nested / "go.mod").exists():
        return nested
    return candidate


input_root = Path(sys.argv[1]).resolve()
backend = resolve_backend_root(input_root)
if not backend.exists():
    print(f"backend root does not exist: {input_root}", file=sys.stderr)
    sys.exit(2)
if not (backend / "go.mod").exists():
    print(f"backend root is missing go.mod: {input_root}", file=sys.stderr)
    sys.exit(2)
frontend = backend.parent / "Frontend"

ignored_dirs = {
    ".git",
    "node_modules",
    "vendor",
    "tmp",
    "dist",
    "build",
}

violations: list[tuple[str, int, str, str]] = []
RAW_DAEMON_SOCKET_MARKERS = (
    "control.sock",
    "daemon.sock",
    "easynet-control.sock",
    "easynet-daemon.sock",
    "unix:///tmp/easynet",
)
RUNTIME_SUBPROCESS_TARGETS = {"easynet", "easynet-daemon"}
RETIRED_NAMESPACE_RESOLVE_CARRIER_KEYS = (
    "queryName",
    "abilityName",
    "realmHint",
    "callerUra",
    "subjectUra",
    "answerKind",
    "canonicalName",
    "ownerUra",
    "abilityUra",
    "routeUra",
    "nextHop",
    "selectedRoute",
    "routeCandidates",
    "routeEvidence",
    "releaseProfile",
    "cachePolicy",
    "recordType",
    "ttlMs",
    "expiresUnixMs",
    "localDeviceAbility",
    "hostedAgentViaDevice",
    "localHubAbility",
    "dispatchName",
    "deviceUra",
    "peerHub",
    "hubUra",
    "noRoute",
    "hostedBy",
    "hostedUra",
    "hostUra",
    "targetUra",
    "executeOn",
    "retryAfterUnixMs",
    "sharedCacheable",
)
NAMESPACE_RESOLVE_CARRIER_PREFIXES = (
    "internal/axon/",
    "internal/axontest/",
    "internal/federation/",
    "internal/catalog/",
    "internal/runtimecontract/",
    "internal/svc/",
    "internal/logic/ability/",
    "internal/logic/agent/",
    "internal/logic/skill/",
)
INVOCATION_SUBMISSION_PREFIX = "internal/logic/ability/"
CANONICAL_INVOCATION_FACADE_SYMBOLS = (
    "CanonicalInvocationBytes",
    "CanonicalBytesForRequestSignature",
)
def rel(path: Path) -> str:
    return str(path.relative_to(backend))


def production_go_files(root: Path):
    for path in root.rglob("*.go"):
        parts = set(path.relative_to(root).parts)
        if parts & ignored_dirs:
            continue
        if path.name.endswith("_test.go"):
            continue
        yield path


def production_frontend_files(root: Path):
    if not root.exists():
        return
    for extension in ("*.ts", "*.tsx"):
        for path in root.rglob(extension):
            parts = set(path.relative_to(root).parts)
            if parts & ignored_dirs:
                continue
            if path.name.endswith((".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx", ".d.ts")):
                continue
            yield path


def namespace_resolve_carrier_go_files(root: Path):
    for path in root.rglob("*.go"):
        parts = set(path.relative_to(root).parts)
        if parts & ignored_dirs:
            continue
        relative = str(path.relative_to(root))
        if relative.startswith(NAMESPACE_RESOLVE_CARRIER_PREFIXES):
            yield path


def go_imports(text: str) -> list[tuple[int, str]]:
    imports: list[tuple[int, str]] = []
    lines = text.splitlines()
    in_block = False
    for index, line in enumerate(lines, start=1):
        stripped = line.strip()
        if not in_block and stripped.startswith("import "):
            rest = stripped[len("import ") :].strip()
            if rest == "(":
                in_block = True
                continue
            imports.extend((index, item) for item in re.findall(r'"([^"]+)"', rest))
            continue
        if in_block:
            if stripped == ")":
                in_block = False
                continue
            imports.extend((index, item) for item in re.findall(r'"([^"]+)"', stripped))
    return imports


def go_string_literals(text: str) -> list[tuple[int, str]]:
    strings: list[tuple[int, str]] = []
    index = 0
    line = 1
    in_block_comment = False
    while index < len(text):
        char = text[index]
        next_char = text[index + 1] if index + 1 < len(text) else ""
        if char == "\n":
            line += 1
            index += 1
            continue
        if in_block_comment:
            if char == "*" and next_char == "/":
                in_block_comment = False
                index += 2
            else:
                index += 1
            continue
        if char == "/" and next_char == "/":
            newline = text.find("\n", index + 2)
            if newline == -1:
                break
            index = newline
            continue
        if char == "/" and next_char == "*":
            in_block_comment = True
            index += 2
            continue
        if char == '"':
            start_line = line
            index += 1
            value: list[str] = []
            while index < len(text):
                char = text[index]
                if char == "\n":
                    line += 1
                if char == "\\" and index + 1 < len(text):
                    value.append(text[index + 1])
                    index += 2
                    continue
                if char == '"':
                    index += 1
                    break
                value.append(char)
                index += 1
            strings.append((start_line, "".join(value)))
            continue
        if char == "`":
            start_line = line
            index += 1
            value = []
            while index < len(text):
                char = text[index]
                if char == "`":
                    index += 1
                    break
                if char == "\n":
                    line += 1
                value.append(char)
                index += 1
            strings.append((start_line, "".join(value)))
            continue
        index += 1
    return strings


def go_code_without_comments(text: str) -> str:
    output: list[str] = []
    index = 0
    in_block_comment = False
    while index < len(text):
        char = text[index]
        next_char = text[index + 1] if index + 1 < len(text) else ""
        if in_block_comment:
            if char == "*" and next_char == "/":
                in_block_comment = False
                index += 2
            else:
                output.append("\n" if char == "\n" else " ")
                index += 1
            continue
        if char == "/" and next_char == "/":
            newline = text.find("\n", index + 2)
            if newline == -1:
                output.extend(" " for _ in text[index:])
                break
            output.extend(" " for _ in text[index:newline])
            index = newline
            continue
        if char == "/" and next_char == "*":
            output.extend("  ")
            in_block_comment = True
            index += 2
            continue
        output.append(char)
        index += 1
    return "".join(output)


def first_matching_line(text: str, pattern: str) -> int | None:
    compiled = re.compile(pattern)
    for line, source_line in enumerate(text.splitlines(), start=1):
        if compiled.search(source_line):
            return line
    return None


def require_boundary_pattern(
    root: Path,
    relative: str,
    pattern: str,
    detail: str,
) -> None:
    path = root / relative
    if not path.is_file():
        violations.append(
            (
                relative,
                1,
                "signed_invocation_descriptor_ref_boundary",
                f"required boundary file is missing: {detail}",
            )
        )
        return
    text = path.read_text(encoding="utf-8", errors="replace")
    line = first_matching_line(text, pattern)
    if line is None:
        violations.append(
            (
                relative,
                1,
                "signed_invocation_descriptor_ref_boundary",
                detail,
            )
        )


def scan_go_mod(path: Path) -> None:
    in_require_block = False
    for index, raw_line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), start=1):
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("//"):
            continue
        directive = raw_line.split("//", 1)[0]
        if re.search(r"(^|\s)easynet\.run/axon/sdk/go(?=\s|$)", directive):
            violations.append(
                (
                    "go.mod",
                    index,
                    "legacy_axon_module_dependency",
                    "easynet.run/axon/sdk/go",
                )
            )
        if in_require_block:
            if stripped == ")":
                in_require_block = False
                continue
            module = stripped.split()[0] if stripped.split() else ""
            if module.startswith("axon.run/sdk/go") and "// indirect" not in raw_line:
                violations.append(("go.mod", index, "raw_axon_module_dependency", module))
            continue
        if stripped == "require (":
            in_require_block = True
            continue
        if stripped.startswith("require "):
            fields = stripped.split()
            if len(fields) >= 2:
                module = fields[1]
                if module.startswith("axon.run/sdk/go") and "// indirect" not in raw_line:
                    violations.append(("go.mod", index, "raw_axon_module_dependency", module))


def retired_namespace_resolve_keys(literal: str) -> list[str]:
    retired: list[str] = []
    for key in RETIRED_NAMESPACE_RESOLVE_CARRIER_KEYS:
        if literal == key:
            retired.append(key)
            continue
        if re.search(r'\bjson:"' + re.escape(key) + r'(?:,|")', literal):
            retired.append(key)
            continue
        if re.search(r'"' + re.escape(key) + r'"\s*:', literal):
            retired.append(key)
    return retired


scan_go_mod(backend / "go.mod")

for source in production_go_files(backend):
    text = source.read_text(encoding="utf-8", errors="replace")
    relative = rel(source)
    normalized = "/" + relative
    imports = go_imports(text)
    string_literals = go_string_literals(text)
    code_without_comments = go_code_without_comments(text)
    if "/internal/pb/axon/v1/" in normalized:
        violations.append((relative, 1, "generated_axon_pb_package", "internal/pb/axon/v1"))
    if "/internal/daemon_grpc/" in normalized:
        violations.append((relative, 1, "direct_daemon_transport_package", "internal/daemon_grpc"))
    if "/internal/axon/" in normalized:
        violations.append((relative, 1, "legacy_backend_axon_facade", "internal/axon"))
    for line, imported in imports:
        if imported == "C":
            violations.append((relative, line, "cgo_ffi_import", imported))
        if imported.startswith("easynet.run/axon/sdk/go"):
            violations.append((relative, line, "legacy_axon_import", imported))
        if imported.startswith("axon.run/sdk/go"):
            violations.append((relative, line, "raw_axon_import", imported))
        if imported.endswith("/internal/pb/axon/v1") or "/internal/pb/axon/v1" in imported:
            violations.append((relative, line, "generated_axon_pb_import", imported))
        if imported.endswith("/internal/daemon_grpc") or "/internal/daemon_grpc" in imported:
            violations.append((relative, line, "direct_daemon_transport_import", imported))
        if imported.endswith("/internal/axon") or "/internal/axon/" in imported:
            violations.append((relative, line, "legacy_backend_axon_facade", imported))
        if "EasyRemote" in imported or "easyremote" in imported:
            violations.append((relative, line, "easyremote_runtime_dependency", imported))
    for line, literal in string_literals:
        if "libeasynet_cli" in literal or "dlopen" in literal:
            violations.append((relative, line, "raw_c_abi_marker", "libeasynet_cli/dlopen"))
        for marker in RAW_DAEMON_SOCKET_MARKERS:
            if marker in literal:
                violations.append((relative, line, "raw_daemon_socket_marker", marker))
    if "exec.Command(" in code_without_comments and any(
        literal in RUNTIME_SUBPROCESS_TARGETS for _, literal in string_literals
    ):
        violations.append((relative, 1, "runtime_subprocess", "exec.Command easynet/easynet-daemon"))
    if re.search(r"\b[A-Za-z_][A-Za-z0-9_\.]*\.Axon\b", code_without_comments):
        violations.append((relative, 1, "legacy_runtime_config_name", "Axon"))
    if relative == "cmd/seed-dev/main.go" and (
        "NewKeyFromSeed" in code_without_comments
        or "ed25519.PrivateKey" in code_without_comments
    ):
        violations.append((relative, 1, "local_key_custody", "seed-dev must use the daemon key-service SDK"))
    for symbol in CANONICAL_INVOCATION_FACADE_SYMBOLS:
        line = first_matching_line(code_without_comments, rf"\b{re.escape(symbol)}\b")
        if line is not None:
            violations.append(
                (
                    relative,
                    line,
                    "backend_canonical_invocation_facade",
                    symbol,
                )
            )
    if relative.startswith(INVOCATION_SUBMISSION_PREFIX):
        line = first_matching_line(code_without_comments, r"\bed25519\.Verify\s*\(")
        if line is not None:
            violations.append(
                (
                    relative,
                    line,
                    "local_invocation_signature_verification",
                    "caller proof must be verified by SDK/daemon",
                )
            )
        canonical_bytes_line = first_matching_line(
            code_without_comments,
            r"\.CanonicalBytesBase64\s*\(",
        )
        canonical_hash_line = first_matching_line(
            code_without_comments,
            r"\.CanonicalHashHex\s*\(",
        )
        local_sha_line = first_matching_line(
            code_without_comments,
            r"\bsha256\.(?:New|Sum256)\s*\(",
        )
        local_hash_decode_line = first_matching_line(
            code_without_comments,
            r"\bhex\.DecodeString\s*\(",
        )
        if canonical_bytes_line is not None and local_sha_line is not None:
            violations.append(
                (
                    relative,
                    local_sha_line,
                    "local_invocation_digest_fallback",
                    "canonical signing bytes must not be hashed outside the SDK",
                )
            )
        if canonical_hash_line is not None and local_hash_decode_line is not None:
            violations.append(
                (
                    relative,
                    local_hash_decode_line,
                    "local_invocation_digest_fallback",
                    "canonical commitment must not be decoded for local proof decisions",
                )
            )
        line = None
        for index, source_line in enumerate(code_without_comments.splitlines(), start=1):
            if re.search(r"\bVerifyPrepareBindToken\s*\(", source_line) and not re.search(
                r"\bfunc\s+VerifyPrepareBindToken\s*\(",
                source_line,
            ):
                line = index
                break
        if line is not None:
            violations.append(
                (
                    relative,
                    line,
                    "local_invocation_prepare_proof_verification",
                    "submit path must delegate the complete signed invocation",
                )
            )
        line = first_matching_line(
            code_without_comments,
            r"\b(?:ListUserPubkeys|verifyTrustRegistration)\b",
        )
        if line is not None:
            violations.append(
                (
                    relative,
                    line,
                    "local_invocation_trust_verification",
                    "caller-key trust is daemon admission authority",
                )
            )

for source in namespace_resolve_carrier_go_files(backend):
    text = source.read_text(encoding="utf-8", errors="replace")
    relative = rel(source)
    for line, literal in go_string_literals(text):
        for key in retired_namespace_resolve_keys(literal):
            violations.append((relative, line, "retired_namespace_resolve_carrier_key", key))

for source in production_frontend_files(frontend):
    text = source.read_text(encoding="utf-8", errors="replace")
    code_without_comments = go_code_without_comments(text)
    relative = str(source.relative_to(backend.parent))
    line = first_matching_line(
        code_without_comments,
        r"\b(?:canonicalInvocationBytes|canonicalDescriptorBoundInvocationBytes)\b",
    )
    if line is not None:
        violations.append(
            (
                relative,
                line,
                "frontend_canonical_invocation_encoder",
                "browser must sign opaque SDK-owned canonical bytes",
            )
        )

require_boundary_pattern(
    backend,
    "api/easynet.api",
    r'DescriptorRef\s+string\s+`json:"descriptor_ref"`',
    "SignedInvokeAbilityReq API must carry descriptor_ref",
)
require_boundary_pattern(
    backend,
    "internal/types/types.go",
    r'DescriptorRef\s+string\s+`json:"descriptor_ref"`',
    "generated SignedInvokeAbilityReq must carry descriptor_ref",
)
require_boundary_pattern(
    backend,
    "internal/logic/ability/prepared_invocation_lease.go",
    r"DescriptorRef:\s*response\.DescriptorRef",
    "prepared invocation lease must bind the response descriptor_ref",
)
require_boundary_pattern(
    backend,
    "internal/logic/ability/prepared_invocation_lease.go",
    r"DescriptorRef:\s*request\.DescriptorRef",
    "prepared invocation lease must verify the submitted descriptor_ref",
)
require_boundary_pattern(
    backend,
    "internal/logic/ability/submitSignedInvocationLogic.go",
    r"DescriptorRef:\s*req\.DescriptorRef",
    "signed submit must preserve the prepare-time descriptor_ref",
)
require_boundary_pattern(
    backend,
    "internal/svc/sdk_bidi_carrier.go",
    r"ProjectDescriptorRef\s*\(",
    "explicit descriptor_ref must be projected instead of reselected",
)
if frontend.exists():
    require_boundary_pattern(
        backend.parent,
        "Frontend/src/lib/api/easynet-abilities.ts",
        r"(?:const\s+descriptorRef\s*=\s*signed\.prepared\.descriptor_ref|descriptor_ref:\s*signed\.prepared\.descriptor_ref)",
        "Frontend signed submission must source descriptor_ref from the prepare response",
    )
    require_boundary_pattern(
        backend.parent,
        "Frontend/src/lib/api/easynet-abilities.ts",
        r"descriptor_ref:\s*(?:signed\.prepared\.descriptor_ref|descriptorRef)",
        "Frontend signed submission must carry the prepare-time descriptor_ref",
    )

if violations:
    print(f"backend SDK-only boundary violations in {backend}:")
    for path, line, rule, detail in sorted(violations):
        print(f"{path}:{line}: {rule}: {detail}")
    sys.exit(1)

print(f"backend SDK-only boundary ok: {backend}")
PY
