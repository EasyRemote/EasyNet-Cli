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

MANIFEST = ROOT / "provider_routes/easynet-runtime-admin-routes.v1.json"
GO_OUTPUT = ROOT / "sdk/go/runtime_admin_routes_gen.go"
PY_OUTPUT = ROOT / "sdk/python/easynet_sdk/_runtime_admin_routes.py"
DAEMON_RUST_OUTPUT = ROOT / "src/daemon/ability/runtime_admin_routes_gen.rs"

ALLOWED_ABILITIES = {"session.list", "federation.revoke"}


def runtime_admin_manifest() -> dict[str, object]:
    return load_manifest(
        MANIFEST,
        expected_provider="easynet",
        expected_capability="runtime_admin",
        route_const_keys={"go_const", "python_const", "daemon_const"},
        route_label="runtime-admin",
        ability_allowed=lambda ability: ability in ALLOWED_ABILITIES,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    manifest = runtime_admin_manifest()
    digest = manifest_sha(MANIFEST)
    changed = [
        write_if_changed(
            GO_OUTPUT,
            go_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                profile_const="runtimeAdminProfile",
                digest_const="runtimeAdminRouteManifestSHA256",
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
                digest_const="_RUNTIME_ADMIN_ROUTE_MANIFEST_SHA256",
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
                profile_const="RUNTIME_ADMIN_PROFILE",
                digest_const="RUNTIME_ADMIN_ROUTE_MANIFEST_SHA256",
                route_const_key="daemon_const",
            ),
            check=args.check,
        ),
    ]
    if not args.check:
        print(f"runtime-admin route bindings generated: {sum(changed)} file(s) updated")


if __name__ == "__main__":
    main()
