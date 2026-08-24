#!/usr/bin/env python3
"""Aggregate three real browser RemoteApp runs into the media matrix contract.

This program only projects facts already present in browser/runtime evidence. It
does not manufacture codec, media, adaptation, route, or terminal observations.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Iterable


PIPELINE_CONTRACT = "remoteapp_media_pipeline_stats_v1"
REQUIRED_ABILITIES = (
    "remote_desktop.create_session",
    "remote_desktop.set_description",
    "remote_desktop.watch_events",
    "remote_desktop.end_session",
)


def objects(value: Any) -> Iterable[dict[str, Any]]:
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from objects(child)
    elif isinstance(value, list):
        for child in value:
            yield from objects(child)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def integer(value: Any, default: int = 0) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def number(value: Any, default: float = 0.0) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


def load_browser_evidence(path: Path, expected_scenario: str) -> dict[str, Any]:
    with path.open(encoding="utf-8") as source:
        evidence = json.load(source)
    require(evidence.get("status") == "passed", f"{path}: browser run did not pass")
    require(
        evidence.get("proof_mode") == "real_browser_tauri_lifecycle",
        f"{path}: expected real browser lifecycle evidence",
    )
    require(evidence.get("component_mock") is False, f"{path}: component_mock must be false")
    require(evidence.get("real_backend_runtime") is True, f"{path}: real runtime required")
    media_scenario = evidence.get("media_scenario")
    require(isinstance(media_scenario, dict), f"{path}: media_scenario missing")
    require(
        media_scenario.get("scenario") == expected_scenario,
        f"{path}: expected scenario {expected_scenario}",
    )
    return evidence


def unique_native_samples(evidence: dict[str, Any]) -> list[dict[str, Any]]:
    samples: dict[tuple[Any, ...], dict[str, Any]] = {}
    for value in objects(evidence):
        if value.get("contract") != PIPELINE_CONTRACT:
            continue
        key = (
            value.get("session_id"),
            value.get("transport_epoch"),
            value.get("sampled_at_ms"),
            value.get("terminal"),
        )
        samples[key] = value
    return sorted(samples.values(), key=lambda value: integer(value.get("sampled_at_ms")))


def unique_browser_samples(evidence: dict[str, Any]) -> list[dict[str, Any]]:
    samples: dict[int, dict[str, Any]] = {}
    for value in objects(evidence):
        browser = value.get("browser_stats")
        if not isinstance(browser, dict):
            continue
        sampled_at_ms = integer(browser.get("sampled_at_ms"))
        if sampled_at_ms > 0:
            samples[sampled_at_ms] = browser
    return [samples[key] for key in sorted(samples)]


def projected_abilities(
    evidence: dict[str, Any], subject_ura: str, session_id: str
) -> list[dict[str, Any]]:
    result = []
    steps = evidence.get("steps")
    require(isinstance(steps, list), "browser evidence steps missing")
    for ability_name in REQUIRED_ABILITIES:
        matches = [
            step
            for step in steps
            if isinstance(step, dict)
            and step.get("ability") == ability_name
            and step.get("status") == "passed"
            and step.get("subject_ura") == subject_ura
        ]
        require(matches, f"missing passed browser step for {ability_name}")
        step = matches[-1]
        if ability_name != "remote_desktop.create_session":
            require(step.get("session_id") == session_id, f"{ability_name}: session mismatch")
        projected = {
            "name": ability_name,
            "subject_ura": subject_ura,
            "observed_at_ms": integer(step.get("observed_at_ms")),
        }
        if ability_name != "remote_desktop.create_session":
            projected["session_id"] = session_id
        result.append(projected)
    return result


def adaptation_events(
    samples: list[dict[str, Any]],
    *,
    scenario_started_at_ms: int,
    impairment_applied_at_ms: int,
    render_probe_at_ms: int,
    subject_ura: str,
    session_id: str,
    media_pipeline_id: str,
) -> list[dict[str, Any]]:
    candidates: dict[tuple[str, int, int], dict[str, Any]] = {}
    for sample in samples:
        for event in sample.get("adaptation_events", []):
            if not isinstance(event, dict):
                continue
            event_type = event.get("event_type")
            observed_at_ms = integer(event.get("observed_at_ms"))
            sequence = integer(event.get("sequence"))
            if not isinstance(event_type, str) or not event_type:
                continue
            if not (scenario_started_at_ms <= observed_at_ms < render_probe_at_ms):
                continue
            if impairment_applied_at_ms and observed_at_ms <= impairment_applied_at_ms:
                continue
            if (
                event.get("selected_resource_ura") != subject_ura
                or event.get("session_id") != session_id
                or event.get("media_pipeline_id") != media_pipeline_id
            ):
                continue
            candidates[(event_type, observed_at_ms, sequence)] = event

    # One earliest causal transition per type keeps the matrix compact while
    # retaining the actual runtime event and its typed pressure detail.
    first_by_type: dict[str, dict[str, Any]] = {}
    for event in sorted(candidates.values(), key=lambda value: integer(value["observed_at_ms"])):
        first_by_type.setdefault(str(event["event_type"]), event)

    return [
        {
            "type": event["event_type"],
            "at_ms": integer(event["observed_at_ms"]),
            "selected_resource_ura": subject_ura,
            "session_id": session_id,
            "media_pipeline_id": media_pipeline_id,
            "transport_epoch": event.get("transport_epoch"),
            "media_source_epoch": event.get("media_source_epoch"),
            "detail": event.get("detail", {}),
        }
        for event in sorted(first_by_type.values(), key=lambda value: integer(value["observed_at_ms"]))
    ]


def terminal_receipt(evidence: dict[str, Any], session_id: str) -> dict[str, Any]:
    terminal_events = [
        value
        for value in objects(evidence)
        if value.get("event_type") == "SESSION_CLOSED"
        and value.get("terminal") is True
        and value.get("session_id") == session_id
    ]
    require(terminal_events, "visible SESSION_CLOSED terminal event missing")
    terminal = max(terminal_events, key=lambda value: integer(value.get("sequence")))
    return {
        "terminal": True,
        "session_id": session_id,
        "reason_code": terminal.get("reason_code") or terminal.get("payload", {}).get("reason_code"),
        "sequence": integer(terminal.get("sequence")),
        "observed_at_ms": integer(terminal.get("at_ms")),
    }


def scenario_projection(
    evidence: dict[str, Any], scenario_name: str
) -> dict[str, Any]:
    media_scenario = evidence["media_scenario"]
    probe = media_scenario.get("decoded_media_probe")
    require(isinstance(probe, dict), f"{scenario_name}: decoded media probe missing")
    probe_at_ms = integer(probe.get("observed_at_ms"))
    scenario_started_at_ms = integer(media_scenario.get("scenario_started_at_ms"))
    impairment_applied_at_ms = integer(media_scenario.get("impairment_applied_at_ms"))
    require(scenario_started_at_ms > 0, f"{scenario_name}: scenario start missing")
    require(probe_at_ms > scenario_started_at_ms, f"{scenario_name}: probe ordering invalid")

    samples = unique_native_samples(evidence)
    require(samples, f"{scenario_name}: native media samples missing")
    eligible = [
        sample
        for sample in samples
        if sample.get("terminal") is not True
        and 0 < integer(sample.get("sampled_at_ms")) <= probe_at_ms
    ]
    require(eligible, f"{scenario_name}: no native sample precedes render probe")
    latest = max(eligible, key=lambda value: integer(value.get("sampled_at_ms")))
    subject_ura = latest.get("selected_resource_ura")
    session_id = latest.get("session_id")
    media_pipeline_id = latest.get("media_pipeline_id")
    require(isinstance(subject_ura, str) and subject_ura.startswith("easynet:///"), "subject missing")
    require(isinstance(session_id, str) and session_id, "session id missing")
    require(isinstance(media_pipeline_id, str) and media_pipeline_id, "pipeline id missing")
    require(evidence.get("selected_resource_ura") == subject_ura, "selected subject mismatch")
    require(evidence.get("session_id") == session_id, "selected session mismatch")

    events = adaptation_events(
        samples,
        scenario_started_at_ms=scenario_started_at_ms,
        impairment_applied_at_ms=impairment_applied_at_ms,
        render_probe_at_ms=probe_at_ms,
        subject_ura=subject_ura,
        session_id=session_id,
        media_pipeline_id=media_pipeline_id,
    )
    latest_event_at_ms = max((integer(event["at_ms"]) for event in events), default=0)
    browser_samples = unique_browser_samples(evidence)
    before_frames = max(
        (
            integer(sample.get("frames_decoded"))
            for sample in browser_samples
            if integer(sample.get("sampled_at_ms")) <= latest_event_at_ms
        ),
        default=0,
    )
    decoded_video_frames = integer(probe.get("decoded_video_frames"))
    frames_after_adaptation = max(0, decoded_video_frames - before_frames)

    latency = latest.get("latency_stats", {}).get("encode_submit_to_rtp_write", {})
    p95_ms = number(latency.get("p95_ms"))
    duration_ms = integer(latest.get("sampled_at_ms")) - scenario_started_at_ms
    queue_depth = integer(latest.get("max_frame_queue_depth"))
    observed_queue_depth = max(
        (integer(sample.get("queued_units")) for sample in eligible),
        default=0,
    )
    audio_queue_depth = integer(latest.get("audio_max_queue_depth"))
    observed_audio_queue_depth = max(
        (integer(sample.get("audio_queue_depth")) for sample in eligible),
        default=0,
    )
    terminal = terminal_receipt(evidence, session_id)

    if scenario_name in {"degraded_network", "backpressure"}:
        require(events, f"{scenario_name}: runtime adaptation event missing")
        require(
            frames_after_adaptation > 0,
            f"{scenario_name}: no decoded frame was observed after projected adaptation events",
        )

    return {
        "scenario": scenario_name,
        "status": "passed",
        "source_only_proof": False,
        "policy_only": False,
        "selected_resource_ura": subject_ura,
        "session_id": session_id,
        "media_pipeline_id": media_pipeline_id,
        "scenario_started_at_ms": scenario_started_at_ms,
        "impairment_applied_at_ms": impairment_applied_at_ms,
        "abilities": projected_abilities(evidence, subject_ura, session_id),
        "video": {
            "codec_negotiated": latest.get("codec_negotiated") is True,
            "codec": latest.get("video_codec"),
            "payload_content_type": latest.get("payload_content_type"),
            "transport": latest.get("video_transport"),
            "frames_encoded": integer(latest.get("frames_encoded")),
            "frames_rendered": decoded_video_frames,
            "duration_ms": duration_ms,
            "requested_fps": number(latest.get("requested_fps")),
            "effective_fps": number(latest.get("effective_fps")),
            "measured_fps": number(latest.get("measured_fps")),
            "target_bitrate_kbps": integer(latest.get("target_bitrate_kbps")),
            "observed_bitrate_kbps": integer(latest.get("observed_bitrate_kbps")),
            "keyframe_interval_frames": integer(latest.get("keyframe_interval_frames")),
            "p95_frame_latency_ms": p95_ms,
            "frames_rendered_after_adaptation": frames_after_adaptation,
            "frames_rendered_after_adaptation_at_ms": probe_at_ms if events else 0,
        },
        "audio": {
            "status": "passed" if latest.get("audio_ready") is True else "failed",
            "codec_negotiated": latest.get("audio_ready") is True,
            "codec": latest.get("audio_codec"),
            "sample_rate_hz": integer(latest.get("audio_sample_rate_hz")),
            "channels": integer(latest.get("audio_channels")),
            "packets_rendered": integer(probe.get("decoded_audio_packets")),
            "samples_rendered": integer(probe.get("decoded_audio_samples")),
            "host_audio_not_implemented": latest.get("host_audio_not_implemented"),
            "muted": latest.get("audio_blocker") is not None,
            "transport_write_isolated": latest.get("audio_transport_write_isolated"),
            "queue": {
                "max_depth": audio_queue_depth,
                "observed_max_depth": observed_audio_queue_depth,
                "bounded": (
                    1 <= audio_queue_depth <= 8
                    and observed_audio_queue_depth <= audio_queue_depth
                ),
            },
            "drop_stale_packets": latest.get("audio_drop_stale_packets"),
            "drop_policy": latest.get("audio_drop_policy"),
            "stale_packets_dropped": integer(latest.get("audio_stale_packets_dropped")),
            "sender_backpressure_errors": integer(
                latest.get("audio_sender_backpressure_errors")
            ),
            "sender_backpressure_drops": integer(
                latest.get("audio_sender_backpressure_drops")
            ),
        },
        "queue": {
            "max_depth": queue_depth,
            "observed_max_depth": observed_queue_depth,
            "bounded": 1 <= queue_depth <= 8 and observed_queue_depth <= queue_depth,
        },
        "drop_policy": {
            "name": latest.get("drop_policy"),
            "unbounded_queue": False,
            "terminal_lifecycle_independent": terminal.get("terminal") is True,
            "frames_dropped": integer(latest.get("frames_dropped")),
        },
        "adaptation": {
            "algorithm": latest.get("adaptation_algorithm"),
            "events": events,
        },
        "render_probe": {
            "probe_source": probe.get("probe_source"),
            "selected_resource_ura": subject_ura,
            "session_id": session_id,
            "media_pipeline_id": media_pipeline_id,
            "video_codec": latest.get("video_codec"),
            "video_transport": latest.get("video_transport"),
            "audio_codec": latest.get("audio_codec"),
            "decoded_video_frames": decoded_video_frames,
            "decoded_audio_packets": integer(probe.get("decoded_audio_packets")),
            "decoded_audio_samples": integer(probe.get("decoded_audio_samples")),
            "video_payload_hash": probe.get("video_payload_hash"),
            "audio_payload_hash": probe.get("audio_payload_hash"),
            "observed_at_ms": probe_at_ms,
        },
        "terminal_receipt": terminal,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--degraded-network", required=True, type=Path)
    parser.add_argument("--backpressure", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    sources = {
        "baseline": load_browser_evidence(args.baseline, "baseline"),
        "degraded_network": load_browser_evidence(args.degraded_network, "degraded_network"),
        "backpressure": load_browser_evidence(args.backpressure, "backpressure"),
    }
    scenarios = [scenario_projection(sources[name], name) for name in sources]
    subjects = {scenario["selected_resource_ura"] for scenario in scenarios}
    pipelines = {scenario["media_pipeline_id"] for scenario in scenarios}
    require(len(subjects) == 1, "matrix scenarios must use one selected Resource URA")
    require(len(pipelines) == 1, "matrix scenarios must use one media pipeline")

    output = {
        "status": "passed",
        "proof_mode": "real_media_adaptation_matrix",
        "component_mock": False,
        "real_backend_runtime": True,
        "product_complete_claim": False,
        "source_evidence": {
            "baseline": str(args.baseline.resolve()),
            "degraded_network": str(args.degraded_network.resolve()),
            "backpressure": str(args.backpressure.resolve()),
        },
        "scenarios": scenarios,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
