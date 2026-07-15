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

MANIFEST = ROOT / "provider_routes/easynet-principal-lifecycle-routes.v1.json"
GO_OUTPUT = ROOT / "sdk/go/principal_routes_gen.go"
PY_OUTPUT = ROOT / "sdk/python/easynet_sdk/_principal_routes.py"
RUST_OUTPUT = ROOT / "src/cli/commands/groups/principal_routes_gen.rs"
DAEMON_RUST_OUTPUT = ROOT / "src/daemon/ability/principal_routes_gen.rs"


def principal_manifest() -> dict[str, object]:
    return load_manifest(
        MANIFEST,
        expected_provider="easynet",
        expected_capability="principal_lifecycle",
        route_const_keys={"go_const", "python_const", "rust_const", "daemon_const"},
        route_label="principal",
        ability_allowed=lambda ability: ability.startswith("principal.lifecycle."),
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    manifest = principal_manifest()
    digest = manifest_sha(MANIFEST)
    changed = [
        write_if_changed(
            GO_OUTPUT,
            go_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                profile_const="principalLifecycleProfile",
                digest_const="principalLifecycleRouteManifestSHA256",
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
                digest_const="_PRINCIPAL_ROUTE_MANIFEST_SHA256",
                route_const_key="python_const",
            ),
            check=args.check,
        ),
        write_if_changed(
            RUST_OUTPUT,
            rust_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                profile_const="PRINCIPAL_LIFECYCLE_PROFILE",
                digest_const="PRINCIPAL_ROUTE_MANIFEST_SHA256",
                route_const_key="rust_const",
            ),
            check=args.check,
        ),
        write_if_changed(
            DAEMON_RUST_OUTPUT,
            rust_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                profile_const="PRINCIPAL_LIFECYCLE_PROFILE",
                digest_const="PRINCIPAL_ROUTE_MANIFEST_SHA256",
                route_const_key="daemon_const",
            ),
            check=args.check,
        ),
    ]
    if not args.check:
        print(f"principal route bindings generated: {sum(changed)} file(s) updated")


if __name__ == "__main__":
    main()
