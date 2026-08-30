#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVER="$ROOT/tools/scripts/remoteapp-stun-binding-server.py"
OUT_DIR="$(mktemp -d)"
SERVER_PID=""

cleanup() {
  local exit_code=$?
  if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
    kill -TERM "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$OUT_DIR"
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

python3 -m py_compile "$SERVER"
"$SERVER" \
  --listen-host 127.0.0.1 \
  --listen-port 0 \
  --event-log "$OUT_DIR/events.jsonl" \
  --ready-file "$OUT_DIR/ready.json" &
SERVER_PID=$!

for _ in {1..100}; do
  [[ -s "$OUT_DIR/ready.json" ]] && break
  kill -0 "$SERVER_PID" >/dev/null 2>&1 || {
    echo "test_remoteapp_stun_binding_server: server exited before readiness" >&2
    exit 1
  }
  sleep 0.02
done
[[ -s "$OUT_DIR/ready.json" ]] || {
  echo "test_remoteapp_stun_binding_server: readiness file was not written" >&2
  exit 1
}

python3 - "$OUT_DIR/ready.json" <<'PY'
import ipaddress
import json
import os
import socket
import struct
import sys

ready = json.load(open(sys.argv[1], encoding="utf-8"))
assert ready["schema"] == "easynet.remoteapp.stun-binding-ready.v1"
port = ready["listen_port"]
cookie = 0x2112A442
transaction = os.urandom(12)
request = struct.pack("!HHI", 0x0001, 0, cookie) + transaction

with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
    client.settimeout(1)
    client.connect(("127.0.0.1", port))
    client.send(request)
    response = client.recv(1024)
    message_type, length, response_cookie = struct.unpack("!HHI", response[:8])
    assert message_type == 0x0101
    assert response_cookie == cookie
    assert response[8:20] == transaction
    assert len(response) == 20 + length
    attribute_type, attribute_length = struct.unpack("!HH", response[20:24])
    assert attribute_type == 0x0020
    assert attribute_length == 8
    reserved, family, xor_port = struct.unpack("!BBH", response[24:28])
    assert reserved == 0 and family == 1
    observed_port = xor_port ^ (cookie >> 16)
    observed_ip = ipaddress.IPv4Address(struct.unpack("!I", response[28:32])[0] ^ cookie)
    local_ip, local_port = client.getsockname()
    assert str(observed_ip) == local_ip
    assert observed_port == local_port

    client.settimeout(0.1)
    client.send(b"not-a-stun-message")
    try:
        client.recv(1024)
    except socket.timeout:
        pass
    else:
        raise AssertionError("malformed STUN datagram received a response")
PY

kill -TERM "$SERVER_PID"
wait "$SERVER_PID"
SERVER_PID=""

python3 - "$OUT_DIR/events.jsonl" <<'PY'
import json
import pathlib
import sys

rows = [json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines()]
events = [row["event"] for row in rows]
assert events[0] == "stun_server_ready"
assert events.count("stun_binding_succeeded") == 1
assert "stun_request_rejected" in events
assert events[-1] == "stun_server_stopped"
for row in rows:
    assert set(row).isdisjoint({"ip", "address", "candidate", "transaction_id"})
PY

echo "test_remoteapp_stun_binding_server: ok"
