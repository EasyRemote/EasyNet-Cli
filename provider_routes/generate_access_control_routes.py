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

MANIFEST = ROOT / "provider_routes/runtime-access-control-routes.v1.json"
GO_OUTPUT = ROOT / "sdk/go/access_control_routes_gen.go"
PY_OUTPUT = ROOT / "sdk/python/easynet_sdk/_access_control_routes.py"
DAEMON_RUST_OUTPUT = ROOT / "src/daemon/ability/access_control_routes_gen.rs"

ALLOWED_ABILITY_PREFIXES = ("authority.binding.", "policy.request.")
ALLOWED_EXACT_ABILITIES = {"admission.explain"}


def is_allowed_ability(ability: str) -> bool:
    return ability in ALLOWED_EXACT_ABILITIES or ability.startswith(ALLOWED_ABILITY_PREFIXES)


def access_control_manifest() -> dict[str, object]:
    return load_manifest(
        MANIFEST,
        expected_provider="runtime",
        expected_capability="access_control",
        route_const_keys={"go_const", "python_const", "daemon_const"},
        route_label="access-control",
        ability_allowed=is_allowed_ability,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    manifest = access_control_manifest()
    digest = manifest_sha(MANIFEST)
    changed = [
        write_if_changed(
            GO_OUTPUT,
            go_source(
                script_name=Path(__file__).name,
                manifest=manifest,
                digest=digest,
                profile_const="accessControlProfile",
                digest_const="accessControlRouteManifestSHA256",
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
                digest_const="_ACCESS_CONTROL_ROUTE_MANIFEST_SHA256",
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
                profile_const="ACCESS_CONTROL_PROFILE",
                digest_const="ACCESS_CONTROL_ROUTE_MANIFEST_SHA256",
                route_const_key="daemon_const",
            ),
            check=args.check,
        ),
    ]
    if not args.check:
        print(f"access-control route bindings generated: {sum(changed)} file(s) updated")


if __name__ == "__main__":
    main()
