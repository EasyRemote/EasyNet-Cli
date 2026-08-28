#!/usr/bin/env python3
"""Verify same-session Hub relay lease refresh and transport replacement evidence."""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import tempfile
from typing import Any


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


def reject_sensitive_fields(value: Any, path: str = "$") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            lowered = key.lower()
            if any(marker in lowered for marker in ("credential", "password", "secret", "token")):
                raise ValueError(f"{path}.{key} exposes forbidden relay authorization material")
            reject_sensitive_fields(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_sensitive_fields(child, f"{path}[{index}]")


def matching_step(browser: dict[str, Any], name: str, session_id: str) -> dict[str, Any]:
    for raw in browser.get("steps", []):
        if isinstance(raw, dict) and raw.get("name") == name and raw.get("session_id") == session_id:
            return raw
    raise ValueError(f"Browser evidence has no {name} step for the selected session")


def matching_snapshot(
    snapshots: list[Any],
    *,
    lease_id: str,
    ability: str,
    session_id: str,
    subject_ura: str,
    after_observed_at_ms: int = 0,
    expected_observed_at_ms: int | None = None,
) -> dict[str, Any]:
    for raw in snapshots:
        if not isinstance(raw, dict):
            continue
        if (
            raw.get("lease_id") == lease_id
            and raw.get("source_ability") == ability
            and raw.get("session_id") == session_id
            and raw.get("resource_ura") == subject_ura
            and isinstance(raw.get("observed_at_ms"), int)
            and raw["observed_at_ms"] > after_observed_at_ms
            and (
                expected_observed_at_ms is None
                or raw["observed_at_ms"] == expected_observed_at_ms
            )
        ):
            return raw
    raise ValueError(f"relay lease snapshot {lease_id} from {ability} is missing")


def verify(browser: dict[str, Any], expected_origin: str) -> dict[str, Any]:
    if browser.get("status") != "passed" or browser.get("evidence_origin") != expected_origin:
        raise ValueError(f"Browser evidence must be a passed {expected_origin} artifact")
    if browser.get("component_mock") is not False or browser.get("real_backend_runtime") is not True:
        raise ValueError("relay refresh evidence must use the real Browser/backend/runtime path")

    session_id = string_value(browser.get("session_id"), "session_id")
    subject_ura = string_value(browser.get("selected_resource_ura"), "selected_resource_ura")
    resume = object_value(browser.get("transport_resume"), "transport_resume")
    refresh = object_value(resume.get("relay_lease_refresh"), "relay_lease_refresh")
    if resume.get("same_public_session") is not True or resume.get("session_id") != session_id:
        raise ValueError("transport resume must preserve the same public RemoteApp session")
    if resume.get("subject_ura") != subject_ura:
        raise ValueError("transport resume subject must match the selected Resource URA")
    prior_epoch = positive_int(resume.get("prior_transport_epoch"), "prior_transport_epoch")
    resumed_epoch = positive_int(resume.get("transport_epoch"), "transport_epoch")
    if resumed_epoch <= prior_epoch or resume.get("transport_epoch_increased") is not True:
        raise ValueError("same-session reconnect must use a strictly newer transport epoch")
    if resume.get("new_peer_connection") is not True or resume.get("watch_events_reestablished") is not True:
        raise ValueError("same-session reconnect must replace the PeerConnection and event watch")
    if positive_int(resume.get("frames_presented_after_resume"), "frames_presented_after_resume") <= 0:
        raise ValueError("same-session reconnect must render media on the replacement transport")

    if refresh.get("provider") != "easynet_relay":
        raise ValueError("relay refresh provider must be easynet_relay")
    if refresh.get("session_id") != session_id or refresh.get("subject_ura") != subject_ura:
        raise ValueError("relay refresh identity must match the RemoteApp session and Resource")
    initial_lease_id = string_value(refresh.get("initial_lease_id"), "initial_lease_id")
    refreshed_lease_id = string_value(refresh.get("refreshed_lease_id"), "refreshed_lease_id")
    if initial_lease_id == refreshed_lease_id or refresh.get("lease_id_changed") is not True:
        raise ValueError("Hub relay refresh must advance to a distinct lease ID")
    resumed_lease_id = string_value(refresh.get("resumed_lease_id"), "resumed_lease_id")
    if (
        refresh.get("refresh_observed_before_disconnect") is not True
        or refresh.get("resumed_lease_reuses_or_advances_refresh") is not True
    ):
        raise ValueError("relay refresh must be observed before disconnect and reused after restart")
    if positive_int(refresh.get("resumed_transport_epoch"), "resumed_transport_epoch") != resumed_epoch:
        raise ValueError("refreshed lease evidence must bind the replacement transport epoch")

    initial_issued_at = positive_int(refresh.get("initial_issued_at_ms"), "initial_issued_at_ms")
    refreshed_issued_at = positive_int(refresh.get("refreshed_issued_at_ms"), "refreshed_issued_at_ms")
    initial_expires_at = positive_int(refresh.get("initial_expires_at_ms"), "initial_expires_at_ms")
    refreshed_expires_at = positive_int(refresh.get("refreshed_expires_at_ms"), "refreshed_expires_at_ms")
    initial_observed_at = positive_int(refresh.get("initial_observed_at_ms"), "initial_observed_at_ms")
    refreshed_observed_at = positive_int(refresh.get("refreshed_observed_at_ms"), "refreshed_observed_at_ms")
    resumed_observed_at = positive_int(refresh.get("resumed_observed_at_ms"), "resumed_observed_at_ms")
    resumed_issued_at = positive_int(refresh.get("resumed_issued_at_ms"), "resumed_issued_at_ms")
    resumed_expires_at = positive_int(refresh.get("resumed_expires_at_ms"), "resumed_expires_at_ms")
    if refreshed_issued_at <= initial_issued_at or refreshed_expires_at <= initial_expires_at:
        raise ValueError("refreshed Hub lease must advance issue and expiry times")
    if refreshed_observed_at <= initial_observed_at:
        raise ValueError("refreshed Hub lease must be observed after the initial lease")
    if resumed_observed_at <= refreshed_observed_at:
        raise ValueError("replacement transport must observe the current Hub lease after daemon restart")
    if resumed_issued_at < refreshed_issued_at or resumed_expires_at < refreshed_expires_at:
        raise ValueError("daemon restart must reuse or advance the pre-disconnect refreshed lease")
    if resumed_lease_id != refreshed_lease_id and resumed_issued_at == refreshed_issued_at:
        raise ValueError("a newer resumed lease ID must carry a later issue time")

    snapshots = browser.get("relay_lease_snapshots")
    if not isinstance(snapshots, list) or not snapshots:
        raise ValueError("relay_lease_snapshots must be a non-empty list")
    reject_sensitive_fields(snapshots, "$.relay_lease_snapshots")
    initial = matching_snapshot(
        snapshots,
        lease_id=initial_lease_id,
        ability="remote_desktop.create_session",
        session_id=session_id,
        subject_ura=subject_ura,
        expected_observed_at_ms=initial_observed_at,
    )
    refreshed = matching_snapshot(
        snapshots,
        lease_id=refreshed_lease_id,
        ability="remote_desktop.show_session",
        session_id=session_id,
        subject_ura=subject_ura,
        expected_observed_at_ms=refreshed_observed_at,
    )
    resumed = matching_snapshot(
        snapshots,
        lease_id=resumed_lease_id,
        ability="remote_desktop.show_session",
        session_id=session_id,
        subject_ura=subject_ura,
        after_observed_at_ms=refreshed_observed_at,
        expected_observed_at_ms=resumed_observed_at,
    )
    for label, snapshot in (("initial", initial), ("refreshed", refreshed)):
        if (
            snapshot.get("provider") != "easynet_relay"
            or snapshot.get("state") != "active"
            or snapshot.get("ephemeral_auth_configured") is not True
            or positive_int(snapshot.get("url_count"), f"{label}.url_count") <= 0
        ):
            raise ValueError(f"{label} relay snapshot is not an active redacted Hub lease")
    if initial.get("issued_at_ms") != initial_issued_at or initial.get("expires_at_ms") != initial_expires_at:
        raise ValueError("initial relay refresh summary does not match its source snapshot")
    if refreshed.get("issued_at_ms") != refreshed_issued_at or refreshed.get("expires_at_ms") != refreshed_expires_at:
        raise ValueError("refreshed relay summary does not match its source snapshot")
    if resumed.get("issued_at_ms") != resumed_issued_at or resumed.get("expires_at_ms") != resumed_expires_at:
        raise ValueError("resumed relay summary does not match its post-restart source snapshot")

    pre_disconnect_refresh_step = matching_step(
        browser,
        "relay_lease_refreshed_before_daemon_disconnect",
        session_id,
    )
    disconnect_step = matching_step(browser, "transport_disconnected", session_id)
    reconnect_step = matching_step(browser, "transport_reconnected", session_id)
    transport_refresh_step = matching_step(
        browser,
        "relay_lease_bound_to_replacement_transport",
        session_id,
    )
    terminal_step = matching_step(browser, "terminal_receipt_visible", session_id)
    if pre_disconnect_refresh_step.get("refreshed_lease_id") != refreshed_lease_id:
        raise ValueError("pre-disconnect refresh step does not bind the refreshed lease ID")
    if transport_refresh_step.get("resumed_lease_id") != resumed_lease_id:
        raise ValueError("replacement transport step does not bind the resumed lease ID")
    if transport_refresh_step.get("transport_epoch") != resumed_epoch:
        raise ValueError("replacement transport step does not bind the newer transport epoch")
    pre_refresh_observed = positive_int(
        pre_disconnect_refresh_step.get("observed_at_ms"),
        "pre-disconnect refresh observed_at_ms",
    )
    disconnect_observed = positive_int(disconnect_step.get("observed_at_ms"), "disconnect observed_at_ms")
    reconnect_observed = positive_int(reconnect_step.get("observed_at_ms"), "reconnect observed_at_ms")
    transport_refresh_observed = positive_int(
        transport_refresh_step.get("observed_at_ms"),
        "replacement transport relay observed_at_ms",
    )
    if not pre_refresh_observed < disconnect_observed < reconnect_observed < transport_refresh_observed:
        raise ValueError("relay refresh/disconnect/reconnect/replacement evidence is not causally ordered")
    if positive_int(terminal_step.get("observed_at_ms"), "terminal observed_at_ms") <= positive_int(
        transport_refresh_step.get("observed_at_ms"), "replacement transport relay observed_at_ms"
    ):
        raise ValueError("terminal receipt must follow relay refresh and reconnect evidence")

    return {
        "status": "passed",
        "evidence_origin": expected_origin,
        "proof_mode": "real_hub_relay_lease_refresh",
        "component_mock": False,
        "real_backend_runtime": True,
        "product_complete_claim": False,
        "provider": "easynet_relay",
        "session_id": session_id,
        "selected_resource_ura": subject_ura,
        "same_public_session": True,
        "initial_lease_id": initial_lease_id,
        "refreshed_lease_id": refreshed_lease_id,
        "resumed_lease_id": resumed_lease_id,
        "lease_id_changed": True,
        "refresh_observed_before_disconnect": True,
        "resumed_lease_reuses_or_advances_refresh": True,
        "initial_issued_at_ms": initial_issued_at,
        "refreshed_issued_at_ms": refreshed_issued_at,
        "initial_expires_at_ms": initial_expires_at,
        "refreshed_expires_at_ms": refreshed_expires_at,
        "resumed_issued_at_ms": resumed_issued_at,
        "resumed_expires_at_ms": resumed_expires_at,
        "resumed_observed_at_ms": resumed_observed_at,
        "prior_transport_epoch": prior_epoch,
        "transport_epoch": resumed_epoch,
        "new_peer_connection": True,
        "watch_events_reestablished": True,
        "frames_presented_after_resume": resume["frames_presented_after_resume"],
        "terminal_receipt_visible": True,
        "credentials_redacted": True,
    }


def self_test_fixture() -> dict[str, Any]:
    session_id = "rd-relay-refresh-self-test"
    subject = "easynet:///r/localhost/resource/device.provider/streams/window.42"
    initial = {
        "observed_at_ms": 100,
        "source_ability": "remote_desktop.create_session",
        "provider": "easynet_relay",
        "state": "active",
        "lease_id": "lease-initial",
        "session_id": session_id,
        "device_ura": "easynet:///r/localhost/device/provider",
        "resource_ura": subject,
        "url_count": 1,
        "ephemeral_auth_configured": True,
        "issued_at_ms": 10,
        "expires_at_ms": 110,
        "refresh_after_ms": 60,
    }
    refreshed = {
        **initial,
        "observed_at_ms": 200,
        "source_ability": "remote_desktop.show_session",
        "lease_id": "lease-refreshed",
        "issued_at_ms": 70,
        "expires_at_ms": 170,
        "refresh_after_ms": 120,
    }
    resumed = {
        **refreshed,
        "observed_at_ms": 260,
    }
    return {
        "status": "passed",
        "evidence_origin": "contract_self_test",
        "component_mock": False,
        "real_backend_runtime": True,
        "session_id": session_id,
        "selected_resource_ura": subject,
        "transport_resume": {
            "session_id": session_id,
            "subject_ura": subject,
            "same_public_session": True,
            "old_peer_retired": True,
            "new_peer_connection": True,
            "prior_transport_epoch": 1,
            "transport_epoch": 2,
            "transport_epoch_increased": True,
            "watch_events_reestablished": True,
            "frames_presented_after_resume": 2,
            "relay_lease_refresh": {
                "provider": "easynet_relay",
                "session_id": session_id,
                "subject_ura": subject,
                "initial_lease_id": "lease-initial",
                "refreshed_lease_id": "lease-refreshed",
                "resumed_lease_id": "lease-refreshed",
                "lease_id_changed": True,
                "refresh_observed_before_disconnect": True,
                "resumed_lease_reuses_or_advances_refresh": True,
                "initial_observed_at_ms": 100,
                "refreshed_observed_at_ms": 200,
                "initial_issued_at_ms": 10,
                "refreshed_issued_at_ms": 70,
                "initial_expires_at_ms": 110,
                "refreshed_expires_at_ms": 170,
                "resumed_observed_at_ms": 260,
                "resumed_issued_at_ms": 70,
                "resumed_expires_at_ms": 170,
                "resumed_transport_epoch": 2,
            },
        },
        "relay_lease_snapshots": [initial, refreshed, resumed],
        "steps": [
            {
                "name": "relay_lease_refreshed_before_daemon_disconnect",
                "session_id": session_id,
                "refreshed_lease_id": "lease-refreshed",
                "observed_at_ms": 270,
            },
            {
                "name": "transport_disconnected",
                "session_id": session_id,
                "observed_at_ms": 300,
            },
            {
                "name": "transport_reconnected",
                "session_id": session_id,
                "observed_at_ms": 350,
            },
            {
                "name": "relay_lease_bound_to_replacement_transport",
                "session_id": session_id,
                "refreshed_lease_id": "lease-refreshed",
                "resumed_lease_id": "lease-refreshed",
                "transport_epoch": 2,
                "observed_at_ms": 360,
            },
            {
                "name": "terminal_receipt_visible",
                "session_id": session_id,
                "terminal": True,
                "observed_at_ms": 400,
            },
        ],
    }


def run_self_test() -> None:
    fixture = self_test_fixture()
    verify(fixture, "contract_self_test")
    mutations = []
    same_id = copy.deepcopy(fixture)
    same_id["transport_resume"]["relay_lease_refresh"]["refreshed_lease_id"] = "lease-initial"
    mutations.append(same_id)
    leaked = copy.deepcopy(fixture)
    leaked["relay_lease_snapshots"][1]["credential"] = "forbidden"
    mutations.append(leaked)
    wrong_subject = copy.deepcopy(fixture)
    wrong_subject["relay_lease_snapshots"][1]["resource_ura"] = "easynet:///r/localhost/resource/display.other"
    mutations.append(wrong_subject)
    stale_epoch = copy.deepcopy(fixture)
    stale_epoch["transport_resume"]["transport_epoch"] = 1
    mutations.append(stale_epoch)
    restart_only = copy.deepcopy(fixture)
    restart_only["transport_resume"]["relay_lease_refresh"]["refresh_observed_before_disconnect"] = False
    mutations.append(restart_only)
    out_of_order = copy.deepcopy(fixture)
    for step in out_of_order["steps"]:
        if step["name"] == "relay_lease_refreshed_before_daemon_disconnect":
            step["observed_at_ms"] = 310
    mutations.append(out_of_order)
    for index, mutation in enumerate(mutations):
        try:
            verify(mutation, "contract_self_test")
        except ValueError:
            continue
        raise AssertionError(f"negative relay refresh mutation {index} was accepted")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser-evidence", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        print("verify-remoteapp-relay-refresh self-test ok")
        return
    if args.browser_evidence is None or args.output is None:
        parser.error("--browser-evidence and --output are required without --self-test")
    browser = object_value(
        json.loads(args.browser_evidence.read_text(encoding="utf-8")),
        "Browser evidence",
    )
    result = verify(browser, "live_runner")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
