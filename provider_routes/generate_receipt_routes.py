#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

from route_generator import (
    ROOT,
    go_source,
    load_manifest,
    manifest_sha,
    python_source,
    rust_source,
    write_if_changed,
)

MANIFEST = ROOT / "provider_routes/easynet-receipt-routes.v1.json"
GO_OUTPUT = ROOT / "sdk/go/receipt_routes_gen.go"
PY_OUTPUT = ROOT / "sdk/python/easynet_sdk/_receipt_routes.py"
DAEMON_RUST_OUTPUT = ROOT / "src/daemon/ability/receipt_routes_gen.rs"


def receipt_manifest() -> dict[str, object]:
    return load_manifest(
        MANIFEST,
        expected_provider="easynet",
        expected_capability="receipt",
        route_const_keys={"go_const", "python_const", "daemon_const"},
        route_label="receipt",
        ability_allowed=lambda ability: ability.startswith("invocation.history.")
        or ability.startswith("invocation.trace."),
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    manifest = receipt_manifest()
    digest = manifest_sha(MANIFEST)
    changed = [
        write_if_changed(
            GO_OUTPUT,
            go_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                profile_const="receiptProfile",
                digest_const="receiptRouteManifestSHA256",
                route_const_key="go_const",
            ),
            check=args.check,
        ),
        write_if_changed(
            PY_OUTPUT,
            python_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                digest_const="_RECEIPT_ROUTE_MANIFEST_SHA256",
                route_const_key="python_const",
            ),
            check=args.check,
        ),
        write_if_changed(
            DAEMON_RUST_OUTPUT,
            rust_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                profile_const="RECEIPT_PROFILE",
                digest_const="RECEIPT_ROUTE_MANIFEST_SHA256",
                route_const_key="daemon_const",
            ),
            check=args.check,
        ),
    ]
    if not args.check:
        print(f"receipt route bindings generated: {sum(changed)} file(s) updated")


if __name__ == "__main__":
    main()
