#!/usr/bin/env bash
# Real-process smoke for the plugin-private RemoteApp native host.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BINARY="${EASYNET_REMOTEAPP_NATIVE_HOST_BIN:-$ROOT/target/debug/easynet-remoteapp-native-host}"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  grep -Fq 'sample_target_inventory' "$0"
  grep -Fq 'EASYNET_REMOTEAPP_PARENT_LIVENESS_FD' "$0"
  grep -Fq '4 * 1024 * 1024' "$0"
  grep -Fq 'parent-liveness' "$0"
  echo "host-remoteapp-native-host-e2e self-test ok"
  exit 0
fi

[[ -x "$BINARY" ]] || {
  echo "host-remoteapp-native-host-e2e: native host is not executable: $BINARY" >&2
  echo "build it with: tools/scripts/build-daemon-process-set.sh" >&2
  exit 1
}

python3 - "$BINARY" <<'PY'
import json
import os
import struct
import subprocess
import sys
import time

binary = sys.argv[1]
max_frame = 4 * 1024 * 1024


def spawn():
    read_fd, write_fd = os.pipe()
    process = subprocess.Popen(
        [binary],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={"EASYNET_REMOTEAPP_PARENT_LIVENESS_FD": str(read_fd)},
        pass_fds=(read_fd,),
    )
    os.close(read_fd)
    return process, write_fd


def read_exact(stream, length):
    chunks = bytearray()
    while len(chunks) < length:
        chunk = stream.read(length - len(chunks))
        if not chunk:
            raise AssertionError(f"unexpected EOF after {len(chunks)} of {length} bytes")
        chunks.extend(chunk)
    return bytes(chunks)


# A real helper process must return one exact, bounded, generation-bound sample.
process, liveness = spawn()
request = json.dumps(
    {
        "schema_version": 1,
        "protocol": "remoteapp_native_host_v1",
        "kind": "sample_target_inventory",
        "process_generation": 73,
        "request_id": 91,
    },
    separators=(",", ":"),
).encode()
process.stdin.write(struct.pack(">I", len(request)) + request)
process.stdin.flush()
length = struct.unpack(">I", read_exact(process.stdout, 4))[0]
assert 0 < length <= max_frame, length
response = json.loads(read_exact(process.stdout, length))
assert response["schema_version"] == 1
assert response["protocol"] == "remoteapp_native_host_v1"
assert response["kind"] == "target_inventory_sample"
assert response["process_generation"] == 73
assert response["request_id"] == 91
assert response["completed_at_ms"] >= response["started_at_ms"]
assert isinstance(response["observation"], dict)
process.stdin.close()
os.close(liveness)
process.wait(timeout=3)

# Closing only the parent-liveness pipe must terminate an otherwise idle helper;
# leaving stdin open ensures EOF on the request channel is not the cause.
orphan, orphan_liveness = spawn()
os.close(orphan_liveness)
deadline = time.monotonic() + 3
while orphan.poll() is None and time.monotonic() < deadline:
    time.sleep(0.01)
assert orphan.poll() is not None, "parent-liveness watchdog did not terminate helper"
orphan.stdin.close()

# An oversized frame length is rejected before body allocation.
malformed, malformed_liveness = spawn()
malformed.stdin.write(struct.pack(">I", max_frame + 1))
malformed.stdin.flush()
malformed.wait(timeout=3)
os.close(malformed_liveness)
assert malformed.returncode != 0, "oversized frame unexpectedly succeeded"

print(
    json.dumps(
        {
            "status": "passed",
            "sample_state": response["observation"].get("state"),
            "response_bytes": length,
            "parent_liveness": "proved",
            "oversized_frame": "rejected",
        },
        sort_keys=True,
    )
)
PY
