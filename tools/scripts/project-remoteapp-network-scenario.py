#!/usr/bin/env python3
"""Project one real Browser RemoteApp run into canonical network evidence.

The projector deliberately keeps transport topology separate from Invocation
identity. It consumes redacted Browser RTCStats and, for relay routes, a
server-side allocation log. Raw candidate addresses and relay credentials are
never copied into the evidence artifact.
"""

from __future__ import annotations

import argparse
from datetime import datetime
import json
from pathlib import Path
import re
from typing import Any


ALLOCATION_MARKER = "Global turn allocation count incremented"
STUN_BINDING_MARKER = "incoming packet BINDING processed, success"
LOG_TIMESTAMP = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?[+-]\d{4})")
ROUTE_CONSTRAINTS = {
    "direct": ({"direct"}, {"relay"}),
    "stun_srflx": ({"stun_srflx"}, {"direct"}),
    "turn_relay": ({"relay"}, {"direct", "stun_srflx"}),
    "easynet_relay": ({"relay"}, {"direct", "stun_srflx"}),
}


def object_value(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be an object")
    return value


def string_value(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{label} must be a non-empty string")
    return value


def positive_int(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise ValueError(f"{label} must be a positive integer")
    return value


def observed_marker_times(path: Path, marker: str, missing_message: str) -> list[int]:
    observed: list[int] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if marker not in line:
            continue
        match = LOG_TIMESTAMP.match(line)
        if not match:
            raise ValueError("TURN allocation log entry has no parseable timestamp")
        timestamp = datetime.strptime(match.group(1), "%Y-%m-%dT%H:%M:%S.%f%z")
        observed.append(int(timestamp.timestamp() * 1000))
    if not observed:
        raise ValueError(missing_message)
    return observed


def parse_allocation_log(path: Path, provider_kind: str) -> dict[str, Any]:
    observed = observed_marker_times(
        path,
        ALLOCATION_MARKER,
        "TURN server did not report a relay allocation",
    )
    return {
        "provider_kind": provider_kind,
        "allocation_observed": True,
        "allocation_count": len(observed),
        "observed_at_ms": min(observed),
    }


def parse_stun_binding_log(path: Path) -> dict[str, Any]:
    observed: list[int] = []
    observer_kinds: set[str] = set()
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if STUN_BINDING_MARKER in line:
            match = LOG_TIMESTAMP.match(line)
            if not match:
                raise ValueError("STUN binding log entry has no parseable timestamp")
            timestamp = datetime.strptime(match.group(1), "%Y-%m-%dT%H:%M:%S.%f%z")
            observed.append(int(timestamp.timestamp() * 1000))
            observer_kinds.add("coturn")
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(event, dict) or event.get("event") != "stun_binding_succeeded":
            continue
        if event.get("schema") != "easynet.remoteapp.stun-binding-event.v1":
            raise ValueError("STUN binding event has an unsupported schema")
        observed.append(positive_int(event.get("observed_at_ms"), "STUN binding time"))
        observer_kinds.add("native_rfc5389_fixture")
    if not observed:
        raise ValueError("STUN server did not report a binding transaction")
    return {
        "provider_kind": "stun",
        "binding_observed": True,
        "binding_count": len(observed),
        "observed_at_ms": min(observed),
        "observer_kinds": sorted(observer_kinds),
    }


def terminal_step(steps: list[Any], session_id: str) -> dict[str, Any]:
    for step in steps:
        if not isinstance(step, dict) or step.get("name") != "terminal_receipt_visible":
            continue
        if step.get("terminal") is True and step.get("session_id") == session_id:
            return step
    raise ValueError("Browser evidence has no terminal receipt for the selected session")


def project(args: argparse.Namespace) -> dict[str, Any]:
    browser = object_value(
        json.loads(args.browser_evidence.read_text(encoding="utf-8")),
        "Browser evidence",
    )
    if browser.get("status") != "passed" or browser.get("evidence_origin") != "live_runner":
        raise ValueError("Browser evidence must be a passed live_runner artifact")
    network = object_value(browser.get("network_transport"), "network_transport")
    pair = object_value(network.get("selected_candidate_pair"), "selected_candidate_pair")
    media = object_value(network.get("media"), "network media")
    route_kind = args.route_kind
    if args.relay_refresh is not None and route_kind != "easynet_relay":
        raise ValueError("--relay-refresh is valid only for the easynet_relay route")
    relay_release: dict[str, Any] | None = None
    relay_refresh: dict[str, Any] | None = None
    expected_route_class = "relay" if route_kind in {"turn_relay", "easynet_relay"} else route_kind
    if pair.get("selected_route_class") != expected_route_class:
        raise ValueError(
            f"selected route class must be {expected_route_class} for {route_kind}"
        )

    subject_ura = string_value(network.get("selected_resource_ura"), "selected_resource_ura")
    session_id = string_value(network.get("session_id"), "session_id")
    pair_observed_at_ms = positive_int(
        pair.get("selected_pair_observed_at_ms"), "selected_pair_observed_at_ms"
    )
    constraints_applied_at_ms = positive_int(
        args.constraints_applied_at_ms, "constraints_applied_at_ms"
    )
    if constraints_applied_at_ms >= pair_observed_at_ms:
        raise ValueError("network constraints must be applied before candidate-pair selection")

    fixture: dict[str, Any] = {
        "fixture_kind": "deployment",
        "route_constraints_applied": True,
        "expected_route_kind": route_kind,
        "allowed_route_classes": sorted(ROUTE_CONSTRAINTS[route_kind][0]),
        "blocked_route_classes": sorted(ROUTE_CONSTRAINTS[route_kind][1]),
        "constraints_applied_at_ms": constraints_applied_at_ms,
    }
    if route_kind == "direct":
        if network.get("client_ice_url_count") != 0:
            raise ValueError("direct projection requires zero daemon-projected ICE server URLs")
        local_description_types = {
            str(value).lower()
            for value in pair.get("local_description_candidate_types", [])
            if isinstance(value, str)
        }
        remote_description_types = {
            str(value).lower()
            for value in pair.get("remote_description_candidate_types", [])
            if isinstance(value, str)
        }
        if local_description_types != {"host"} or remote_description_types != {"host"}:
            raise ValueError("direct projection requires host-only local and remote SDP")
        fixture["constraint_method"] = "daemon_zero_ice_servers_plus_host_only_sdp"
    elif route_kind == "stun_srflx":
        if args.binding_log is None:
            raise ValueError("stun_srflx projection requires --binding-log")
        if not isinstance(network.get("client_ice_url_count"), int) or network["client_ice_url_count"] <= 0:
            raise ValueError("STUN projection requires a positive daemon-projected ICE URL count")
        schemes = {
            str(value).lower()
            for value in network.get("client_ice_url_schemes", [])
            if isinstance(value, str)
        }
        if not schemes or not schemes.issubset({"stun", "stuns"}):
            raise ValueError("STUN projection requires only redacted stun/stuns URL schemes")
        admission = object_value(network.get("candidate_admission"), "candidate_admission")
        expected_directional_types = {
            "outbound": {"prflx", "srflx"},
            "inbound": {"host", "prflx", "srflx"},
        }
        for direction, expected_types in expected_directional_types.items():
            counters = object_value(admission.get(direction), f"candidate_admission.{direction}")
            allowed_types = {
                str(value).lower()
                for value in counters.get("allowed_types", [])
                if isinstance(value, str)
            }
            if allowed_types != expected_types:
                raise ValueError(
                    f"STUN projection requires {direction} candidate admission types "
                    f"{sorted(expected_types)}"
                )
            if not isinstance(counters.get("accepted"), int) or counters["accepted"] <= 0:
                raise ValueError(f"STUN projection requires accepted {direction} candidates")
            if not isinstance(counters.get("rejected"), int) or counters["rejected"] < 0:
                raise ValueError(f"STUN projection requires bounded {direction} rejection counters")
        if admission["outbound"]["rejected"] <= 0:
            raise ValueError("STUN projection requires rejected outbound Browser host candidates")
        local_candidate_type = str(pair.get("local_candidate_type", "")).lower()
        remote_candidate_type = str(pair.get("remote_candidate_type", "")).lower()
        if local_candidate_type not in {"prflx", "srflx"}:
            raise ValueError(
                "STUN projection requires the selected Browser-local candidate to be reflexive"
            )
        if remote_candidate_type not in {"host", "prflx", "srflx"}:
            raise ValueError(
                "STUN projection requires the selected provider candidate to be host or reflexive"
            )
        local_description_types = {
            str(value).lower()
            for value in pair.get("local_description_candidate_types", [])
            if isinstance(value, str)
        }
        if not local_description_types or not local_description_types.issubset({"prflx", "srflx"}):
            raise ValueError(
                "STUN projection requires Browser local SDP to contain only reflexive candidates"
            )
        binding = parse_stun_binding_log(args.binding_log)
        observed_at_ms = positive_int(binding["observed_at_ms"], "STUN binding time")
        if not constraints_applied_at_ms < observed_at_ms <= pair_observed_at_ms:
            raise ValueError(
                "STUN binding must be observed after constraints and before pair selection"
            )
        fixture["constraint_method"] = (
            "browser_reflexive_outbound_plus_provider_host_return_and_server_binding"
        )
        fixture["stun_binding"] = binding
    elif route_kind in {"turn_relay", "easynet_relay"}:
        if args.allocation_log is None:
            raise ValueError(f"{route_kind} projection requires --allocation-log")
        provider_kind = "turn" if route_kind == "turn_relay" else "easynet_relay"
        allocation = parse_allocation_log(args.allocation_log, provider_kind)
        observed_at_ms = positive_int(allocation["observed_at_ms"], "relay allocation time")
        if not constraints_applied_at_ms < observed_at_ms <= pair_observed_at_ms:
            raise ValueError(
                "TURN allocation must be observed after constraints and before pair selection"
            )
        fixture["constraint_method"] = (
            "browser_relay_only_policy_plus_coturn_allocation"
            if route_kind == "turn_relay"
            else "hub_lease_plus_browser_relay_only_policy_plus_coturn_allocation"
        )
        fixture["relay_allocation"] = allocation

        if not isinstance(network.get("client_ice_url_count"), int) or network["client_ice_url_count"] <= 0:
            raise ValueError(f"{route_kind} projection requires Hub-projected ICE server URLs")
        schemes = {
            str(value).lower()
            for value in network.get("client_ice_url_schemes", [])
            if isinstance(value, str)
        }
        if not schemes or not schemes.issubset({"turn", "turns"}):
            raise ValueError(f"{route_kind} projection requires only redacted turn/turns URL schemes")

        if route_kind == "easynet_relay":
            if args.release_probe is None:
                raise ValueError("easynet_relay projection requires --release-probe")
            relay = object_value(network.get("easynet_relay"), "easynet_relay")
            forbidden = {"username", "credential", "password", "secret"}
            if forbidden.intersection(relay):
                raise ValueError("EasyNet relay evidence contains raw credential fields")
            if relay.get("provider") != "easynet_relay" or relay.get("state") != "active":
                raise ValueError("EasyNet relay evidence must identify one active Hub lease")
            if relay.get("ephemeral_auth_configured") is not True:
                raise ValueError("EasyNet relay evidence must confirm ephemeral credential configuration")
            if relay.get("session_id") != session_id or relay.get("resource_ura") != subject_ura:
                raise ValueError("EasyNet relay lease identity must match the RemoteApp session")
            if not isinstance(relay.get("url_count"), int) or relay["url_count"] <= 0:
                raise ValueError("EasyNet relay lease must expose a positive redacted URL count")
            relay_session_id = string_value(relay.get("lease_id"), "easynet_relay.lease_id")
            relay_release = object_value(
                json.loads(args.release_probe.read_text(encoding="utf-8")),
                "relay release probe",
            )
            if (
                relay_release.get("status_code") != 409
                or relay_release.get("terminal_reacquire_rejected") is not True
            ):
                raise ValueError("EasyNet relay release probe must prove terminal reacquire rejection")
            if args.relay_refresh is not None:
                relay_refresh = object_value(
                    json.loads(args.relay_refresh.read_text(encoding="utf-8")),
                    "relay refresh evidence",
                )
                if (
                    relay_refresh.get("status") != "passed"
                    or relay_refresh.get("evidence_origin") != "live_runner"
                    or relay_refresh.get("proof_mode") != "real_hub_relay_lease_refresh"
                    or relay_refresh.get("component_mock") is not False
                    or relay_refresh.get("real_backend_runtime") is not True
                    or relay_refresh.get("product_complete_claim") is not False
                ):
                    raise ValueError("relay refresh evidence must be a passed live Hub/runtime proof")
                if (
                    relay_refresh.get("session_id") != session_id
                    or relay_refresh.get("selected_resource_ura") != subject_ura
                ):
                    raise ValueError("relay refresh evidence must bind the projected session and Resource")
                if relay_refresh.get("initial_lease_id") != relay_session_id:
                    raise ValueError("relay refresh evidence must start from the projected Browser lease")
                if (
                    relay_refresh.get("initial_lease_id") == relay_refresh.get("refreshed_lease_id")
                    or relay_refresh.get("lease_id_changed") is not True
                ):
                    raise ValueError("relay refresh evidence must advance the Hub lease ID")
                string_value(relay_refresh.get("resumed_lease_id"), "relay refresh resumed_lease_id")
                if (
                    relay_refresh.get("refresh_observed_before_disconnect") is not True
                    or relay_refresh.get("resumed_lease_reuses_or_advances_refresh") is not True
                ):
                    raise ValueError("relay refresh must precede disconnect and survive daemon restart")
                if (
                    positive_int(relay_refresh.get("transport_epoch"), "relay refresh transport_epoch")
                    <= positive_int(
                        relay_refresh.get("prior_transport_epoch"),
                        "relay refresh prior_transport_epoch",
                    )
                ):
                    raise ValueError("relay refresh evidence must bind a newer transport epoch")
                if (
                    relay_refresh.get("same_public_session") is not True
                    or relay_refresh.get("new_peer_connection") is not True
                    or relay_refresh.get("watch_events_reestablished") is not True
                    or relay_refresh.get("terminal_receipt_visible") is not True
                    or relay_refresh.get("credentials_redacted") is not True
                    or positive_int(
                        relay_refresh.get("frames_presented_after_resume"),
                        "relay refresh frames_presented_after_resume",
                    )
                    <= 0
                ):
                    raise ValueError("relay refresh evidence does not prove resumed media lifecycle")

    abilities = []
    for raw in network.get("abilities", []):
        ability = object_value(raw, "network ability")
        projected = dict(ability)
        projected["name"] = projected.pop("ability", None)
        abilities.append(projected)
    terminal = terminal_step(browser.get("steps", []), session_id)
    first_frame_at_ms = positive_int(
        media.get("first_rendered_frame_at_ms"), "first_rendered_frame_at_ms"
    )
    ended_at_ms = positive_int(terminal.get("observed_at_ms"), "terminal observed_at_ms")
    if relay_release is not None:
        release_observed_at_ms = positive_int(
            relay_release.get("observed_at_ms"), "relay release observed_at_ms"
        )
        if release_observed_at_ms < ended_at_ms:
            raise ValueError("EasyNet relay release must be observed after the terminal receipt")

    scenario: dict[str, Any] = {
        "name": f"host-browser-{route_kind}",
        "route_kind": route_kind,
        "status": "passed",
        "credentials_redacted": True,
        "caller_ura": string_value(network.get("caller_ura"), "caller_ura"),
        "callee_ura": string_value(network.get("callee_ura"), "callee_ura"),
        "provider_device_ura": string_value(
            network.get("provider_device_ura"), "provider_device_ura"
        ),
        "client_endpoint_id": string_value(
            network.get("client_endpoint_id"), "client_endpoint_id"
        ),
        "selected_resource_ura": subject_ura,
        "session_id": session_id,
        "network_fixture": fixture,
        "abilities": abilities,
        "webrtc": {
            **network,
            "route_kind": route_kind,
        },
        "media": {
            "selected_resource_ura": subject_ura,
            "session_id": session_id,
            "route_kind": route_kind,
            "candidate_pair_id": string_value(
                media.get("candidate_pair_id"), "media candidate_pair_id"
            ),
            "frames_rendered": positive_int(media.get("frames_rendered"), "frames_rendered"),
            "duration_ms": max(1, ended_at_ms - first_frame_at_ms),
            "rendered_after_selected_pair": media.get("rendered_after_selected_pair") is True,
            "first_rendered_frame_at_ms": first_frame_at_ms,
        },
        "terminal_receipt": {
            "terminal": True,
            "session_id": session_id,
            "reason_code": terminal.get("reason_code"),
            "observed_at_ms": ended_at_ms,
        },
    }
    if route_kind == "turn_relay":
        scenario["turn_relay_uri_redacted"] = True
    elif route_kind == "easynet_relay":
        scenario["route_provider"] = "easynet_relay"
        scenario["relay_reachability"] = True
        scenario["relay_session_id"] = relay_session_id
        scenario["relay_release"] = relay_release
        if relay_refresh is not None:
            scenario["relay_refresh"] = relay_refresh

    return {
        "status": "passed",
        "evidence_origin": args.evidence_origin,
        "proof_mode": "real_network_fallback_matrix",
        "runner_kind": "deployment",
        "component_mock": False,
        "real_backend_runtime": True,
        "product_complete_claim": False,
        "scenarios": [scenario],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser-evidence", type=Path, required=True)
    parser.add_argument("--route-kind", choices=sorted(ROUTE_CONSTRAINTS), required=True)
    parser.add_argument("--constraints-applied-at-ms", type=int, required=True)
    parser.add_argument("--allocation-log", type=Path)
    parser.add_argument("--binding-log", type=Path)
    parser.add_argument("--release-probe", type=Path)
    parser.add_argument("--relay-refresh", type=Path)
    parser.add_argument(
        "--evidence-origin",
        choices=("live_runner", "contract_self_test"),
        default="live_runner",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    evidence = project(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
