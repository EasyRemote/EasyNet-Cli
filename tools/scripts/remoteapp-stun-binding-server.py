#!/usr/bin/env python3
"""Bounded RFC 5389 STUN Binding fixture for RemoteApp network E2E.

This fixture intentionally implements only unauthenticated Binding requests.
It runs on the provider host so a Browser inside an independently reachable VM
observes one real VM-NAT server-reflexive mapping without Docker Desktop adding
a second, non-routable NAT boundary.

The event log contains no socket addresses, candidate strings, credentials, or
transaction identifiers. It is evidence that an independent STUN endpoint saw
and answered a valid Binding request; selected-pair truth remains in Browser
RTCStats and the canonical network projector.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import ipaddress
import json
import os
from pathlib import Path
import signal
import socket
import struct
import sys
import tempfile
import time
from typing import Any


MAGIC_COOKIE = 0x2112A442
BINDING_REQUEST = 0x0001
BINDING_SUCCESS = 0x0101
XOR_MAPPED_ADDRESS = 0x0020
MESSAGE_HEADER_BYTES = 20
MAX_DATAGRAM_BYTES = 4096


class BindingServer:
    """One bounded UDP listener with explicit startup and terminal lifecycle."""

    def __init__(
        self,
        listen_host: str,
        listen_port: int,
        event_log: Path,
        ready_file: Path | None,
        max_bindings: int,
    ) -> None:
        self._listen_host = listen_host
        self._listen_port = listen_port
        self._event_log = event_log
        self._ready_file = ready_file
        self._max_bindings = max_bindings
        self._stopping = False
        self._binding_count = 0

    def stop(self, _signum: int, _frame: Any) -> None:
        self._stopping = True

    def run(self) -> None:
        family = socket.AF_INET6 if ":" in self._listen_host else socket.AF_INET
        with socket.socket(family, socket.SOCK_DGRAM) as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind((self._listen_host, self._listen_port))
            listener.settimeout(0.25)
            bound = listener.getsockname()
            self._write_ready(int(bound[1]))
            self._record("stun_server_ready", listen_family=self._family_name(family))

            while not self._stopping:
                if self._max_bindings and self._binding_count >= self._max_bindings:
                    break
                try:
                    body, peer = listener.recvfrom(MAX_DATAGRAM_BYTES + 1)
                except socket.timeout:
                    continue
                if len(body) > MAX_DATAGRAM_BYTES:
                    self._record("stun_request_rejected", reason="datagram_too_large")
                    continue
                response = self._binding_response(body, peer)
                if response is None:
                    continue
                listener.sendto(response, peer)
                self._binding_count += 1
                self._record(
                    "stun_binding_succeeded",
                    binding_sequence=self._binding_count,
                    peer_family=self._peer_family(peer),
                )

            self._record("stun_server_stopped", binding_count=self._binding_count)

    def _binding_response(self, body: bytes, peer: tuple[Any, ...]) -> bytes | None:
        if len(body) < MESSAGE_HEADER_BYTES:
            self._record("stun_request_rejected", reason="header_truncated")
            return None
        message_type, declared_length, cookie = struct.unpack("!HHI", body[:8])
        if message_type != BINDING_REQUEST:
            self._record("stun_request_rejected", reason="not_binding_request")
            return None
        if cookie != MAGIC_COOKIE:
            self._record("stun_request_rejected", reason="invalid_magic_cookie")
            return None
        if declared_length % 4 != 0 or MESSAGE_HEADER_BYTES + declared_length != len(body):
            self._record("stun_request_rejected", reason="invalid_message_length")
            return None

        transaction_id = body[8:20]
        try:
            ip = ipaddress.ip_address(str(peer[0]))
            port = int(peer[1])
        except (ValueError, TypeError, IndexError):
            self._record("stun_request_rejected", reason="invalid_peer_address")
            return None

        xor_port = port ^ (MAGIC_COOKIE >> 16)
        if isinstance(ip, ipaddress.IPv4Address):
            family = 0x01
            xor_ip = int(ip) ^ MAGIC_COOKIE
            address = struct.pack("!I", xor_ip)
        else:
            family = 0x02
            mask = struct.pack("!I", MAGIC_COOKIE) + transaction_id
            address = bytes(left ^ right for left, right in zip(ip.packed, mask, strict=True))
        value = struct.pack("!BBH", 0, family, xor_port) + address
        attribute = struct.pack("!HH", XOR_MAPPED_ADDRESS, len(value)) + value
        return (
            struct.pack("!HHI", BINDING_SUCCESS, len(attribute), MAGIC_COOKIE)
            + transaction_id
            + attribute
        )

    def _write_ready(self, bound_port: int) -> None:
        if self._ready_file is None:
            return
        self._ready_file.parent.mkdir(parents=True, exist_ok=True)
        self._write_json_atomic(
            self._ready_file,
            {
                "schema": "easynet.remoteapp.stun-binding-ready.v1",
                "listen_family": self._family_name(
                    socket.AF_INET6 if ":" in self._listen_host else socket.AF_INET
                ),
                "listen_port": bound_port,
            },
        )

    def _record(self, event: str, **fields: Any) -> None:
        now = datetime.now(timezone.utc)
        row = {
            "schema": "easynet.remoteapp.stun-binding-event.v1",
            "event": event,
            "observed_at": now.isoformat(timespec="milliseconds").replace("+00:00", "Z"),
            "observed_at_ms": time.time_ns() // 1_000_000,
            **fields,
        }
        self._event_log.parent.mkdir(parents=True, exist_ok=True)
        with self._event_log.open("a", encoding="utf-8") as output:
            output.write(json.dumps(row, separators=(",", ":"), sort_keys=True) + "\n")
            output.flush()
            os.fsync(output.fileno())

    @staticmethod
    def _write_json_atomic(path: Path, value: dict[str, Any]) -> None:
        descriptor, temporary = tempfile.mkstemp(
            prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
        )
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as output:
                json.dump(value, output, separators=(",", ":"), sort_keys=True)
                output.write("\n")
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        except BaseException:
            try:
                os.unlink(temporary)
            except FileNotFoundError:
                pass
            raise

    @staticmethod
    def _family_name(family: socket.AddressFamily) -> str:
        return "ipv6" if family == socket.AF_INET6 else "ipv4"

    @staticmethod
    def _peer_family(peer: tuple[Any, ...]) -> str:
        return "ipv6" if ":" in str(peer[0]) else "ipv4"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run a bounded RFC 5389 STUN Binding-only E2E fixture."
    )
    parser.add_argument("--listen-host", required=True)
    parser.add_argument("--listen-port", type=int, required=True)
    parser.add_argument("--event-log", type=Path, required=True)
    parser.add_argument("--ready-file", type=Path)
    parser.add_argument("--max-bindings", type=int, default=0)
    args = parser.parse_args()
    if not 0 <= args.listen_port <= 65535:
        parser.error("--listen-port must be in 0..65535")
    if args.max_bindings < 0:
        parser.error("--max-bindings must be non-negative")
    try:
        ipaddress.ip_address(args.listen_host)
    except ValueError as exc:
        parser.error(f"--listen-host must be a literal IP address: {exc}")
    return args


def main() -> int:
    args = parse_args()
    server = BindingServer(
        args.listen_host,
        args.listen_port,
        args.event_log,
        args.ready_file,
        args.max_bindings,
    )
    signal.signal(signal.SIGINT, server.stop)
    signal.signal(signal.SIGTERM, server.stop)
    try:
        server.run()
    except OSError as exc:
        print(f"remoteapp STUN binding server failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
