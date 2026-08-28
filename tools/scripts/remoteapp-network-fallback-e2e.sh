#!/usr/bin/env bash
# RemoteApp network fallback E2E evidence verifier.
#
# Boundary:
# - This harness verifies evidence produced by a real two-device,
#   network-namespace, or deployment runner for RemoteApp direct/STUN/TURN/
#   EasyNet relay paths.
# - It does not provision networking infrastructure and does not simulate
#   WebRTC. A live pass requires either --evidence-json from an external runner
#   or --runner-cmd that writes the evidence JSON path provided through
#   EASYNET_REMOTEAPP_NETWORK_FALLBACK_EVIDENCE_JSON.
# - Self-test validates the evidence contract only; it is not product evidence.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
PROVENANCE_HELPER="$SELF_DIR/remoteapp-evidence-provenance.py"

MODE=skip
SELF_TEST=0
OUT_DIR="${EASYNET_REMOTEAPP_NETWORK_FALLBACK_OUT_DIR:-$REPO_ROOT/target/e2e/remoteapp-network-fallback/$(date -u +%Y%m%d-%H%M%S)-$$}"
RUNNER_CMD="${EASYNET_REMOTEAPP_NETWORK_FALLBACK_RUNNER_CMD:-}"
EVIDENCE_INPUT="${EASYNET_REMOTEAPP_NETWORK_FALLBACK_EVIDENCE_JSON:-}"
REQUIRED_ROUTES_CSV="direct,stun_srflx,turn_relay,easynet_relay"

usage() {
  cat <<'USAGE'
Usage:
  remoteapp-network-fallback-e2e.sh --run --evidence-json PATH
  remoteapp-network-fallback-e2e.sh --run --runner-cmd CMD
  remoteapp-network-fallback-e2e.sh --self-test

Options:
  --run                 Verify real RemoteApp network fallback evidence.
  --self-test           Validate the harness against synthetic positive evidence.
  --runner-cmd CMD      Command that drives real network scenarios and writes
                        evidence to EASYNET_REMOTEAPP_NETWORK_FALLBACK_EVIDENCE_JSON.
  --evidence-json PATH  Existing evidence JSON emitted by a real network runner.
  --required-routes CSV
                        Require a route subset for a focused child proof.
                        Product completion must use the default four routes.
  --out-dir DIR         Report directory.
  -h, --help            Show this help.

Environment:
  EASYNET_REMOTEAPP_NETWORK_FALLBACK_E2E=1
                        Equivalent to --run.

Evidence contract:
  The evidence JSON must prove a real network fallback matrix, not route-model
  source checks:
  direct, stun_srflx, turn_relay, and easynet_relay scenarios, each with
  connected WebRTC candidate-pair evidence, nominated/selected/succeeded ICE
  pair state, selected route-class evidence, WebRTC/session binding to the
  selected Resource URA, admitted caller, Remote Desktop SystemAgent callee,
  provider Device execution host, and Browser/network client endpoint; applied
  network fixture constraints; rendered media bound to the selected pair after
  selected-pair observation; production RemoteApp WebRTC abilities; selected
  Resource URA subject binding; session end; and a visible terminal receipt.

Non-claims:
  A skipped report or self-test does not prove network product readiness.
  This harness verifies one network fallback artifact; OS capture, input
  injection, codec soak, frontend Browser/Tauri lifecycle, and cross-device
  product behavior still require their own evidence.
USAGE
}

if [[ "${EASYNET_REMOTEAPP_NETWORK_FALLBACK_E2E:-0}" == "1" ]]; then
  MODE=run
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) MODE=run; shift ;;
    --self-test) SELF_TEST=1; MODE=self-test; shift ;;
    --runner-cmd) RUNNER_CMD="${2:?missing value for --runner-cmd}"; shift 2 ;;
    --evidence-json) EVIDENCE_INPUT="${2:?missing value for --evidence-json}"; shift 2 ;;
    --required-routes) REQUIRED_ROUTES_CSV="${2:?missing value for --required-routes}"; shift 2 ;;
    --out-dir) OUT_DIR="${2:?missing value for --out-dir}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

IFS=',' read -r -a required_route_values <<<"$REQUIRED_ROUTES_CSV"
[[ "${#required_route_values[@]}" -gt 0 ]] || {
  echo "--required-routes must not be empty" >&2
  exit 64
}
for route in "${required_route_values[@]}"; do
  case "$route" in
    direct|stun_srflx|turn_relay|easynet_relay) ;;
    *) echo "unknown required route: $route" >&2; exit 64 ;;
  esac
done

mkdir -p "$OUT_DIR"
EVIDENCE_JSON="$OUT_DIR/evidence.json"
REPORT_JSON="$OUT_DIR/report.json"
REPORT_MD="$OUT_DIR/report.md"
RUNNER_STDOUT="$OUT_DIR/runner.stdout.txt"
RUNNER_STDERR="$OUT_DIR/runner.stderr.txt"

write_report() {
  local status="$1"
  local reason="$2"
  python3 - "$REPORT_JSON" "$REPORT_MD" "$status" "$reason" "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

report_path, md_path, status, reason, evidence_path = sys.argv[1:6]
report = {
    "script": "tools/scripts/remoteapp-network-fallback-e2e.sh",
    "status": status,
    "reason": reason,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
    "coverage": {
        "direct": False,
        "stun_srflx": False,
        "turn_relay": False,
        "easynet_relay": False,
    },
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
pathlib.Path(md_path).write_text(
    "# RemoteApp Network Fallback E2E\n\n"
    f"- Status: `{status}`\n"
    f"- Reason: `{reason}`\n"
    f"- Evidence: `{evidence_path}`\n",
    encoding="utf-8",
)
PY
}

validate_evidence() {
  python3 "$PROVENANCE_HELPER" verify --mode "$MODE" --evidence "$EVIDENCE_JSON"
  python3 - "$EVIDENCE_JSON" "$REPORT_JSON" "$REPORT_MD" "$REQUIRED_ROUTES_CSV" <<'PY'
import json
import pathlib
import sys

evidence_path, report_path, md_path, required_routes_csv = sys.argv[1:5]
with open(evidence_path, encoding="utf-8") as f:
    evidence = json.load(f)

errors = []

def require(condition, message):
    if not condition:
        errors.append(message)

def is_ura(value):
    return isinstance(value, str) and value.startswith("easynet:///")

def has_ura_kind(value, kind):
    return is_ura(value) and (f"/{kind}/" in value or value.endswith(f"/{kind}"))

def lower(value):
    return value.lower() if isinstance(value, str) else value

def expected_selected_route_class(route_kind):
    if route_kind == "direct":
        return "direct"
    if route_kind == "stun_srflx":
        return "stun_srflx"
    if route_kind in {"turn_relay", "easynet_relay"}:
        return "relay"
    return None

def expected_allowed_route_classes(route_kind):
    if route_kind == "direct":
        return {"direct"}
    if route_kind == "stun_srflx":
        return {"stun_srflx"}
    if route_kind in {"turn_relay", "easynet_relay"}:
        return {"relay"}
    return set()

def expected_blocked_route_classes(route_kind):
    if route_kind == "direct":
        return {"relay"}
    if route_kind == "stun_srflx":
        return {"direct"}
    if route_kind in {"turn_relay", "easynet_relay"}:
        return {"direct", "stun_srflx"}
    return set()

def sensitive_key_errors(value, path="$"):
    if isinstance(value, dict):
        for key, child in value.items():
            lowered = key.lower()
            allowed = lowered in {
                "credentials_redacted",
                "credential_policy",
                "redacted_credentials",
            }
            sensitive = any(
                marker in lowered
                for marker in ("credential", "password", "secret", "token", "private_key", "access_key")
            )
            if sensitive and not allowed:
                errors.append(f"{path}.{key}: raw credential/secret fields are forbidden")
            sensitive_key_errors(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            sensitive_key_errors(child, f"{path}[{index}]")

allowed_routes = {"direct", "stun_srflx", "turn_relay", "easynet_relay"}
required_routes = set(required_routes_csv.split(","))
allowed_runner_kinds = {"two_device", "network_namespace", "deployment"}
terminal_reasons = {"caller_ended", "user_cancelled", "network_fallback_e2e_cleanup"}

require(evidence.get("status") == "passed", "evidence.status must be passed")
require(evidence.get("proof_mode") == "real_network_fallback_matrix",
        "proof_mode must be real_network_fallback_matrix")
require(evidence.get("runner_kind") in allowed_runner_kinds,
        "runner_kind must be two_device, network_namespace, or deployment")
require(evidence.get("component_mock") is False,
        "component_mock must be false")
require(evidence.get("real_backend_runtime") is True,
        "real_backend_runtime must be true")
require(evidence.get("product_complete_claim") is False,
        "product_complete_claim must remain false")
sensitive_key_errors(evidence)

scenarios = evidence.get("scenarios")
require(isinstance(scenarios, list) and scenarios, "scenarios must be a non-empty list")
seen_routes = set()
scenario_reports = []
relay_refresh_covered = False

if isinstance(scenarios, list):
    for index, scenario in enumerate(scenarios):
        if not isinstance(scenario, dict):
            errors.append(f"scenarios[{index}] must be an object")
            continue
        route_kind = scenario.get("route_kind")
        name = scenario.get("name") or f"scenario[{index}]"
        seen_routes.add(route_kind)
        prefix = f"{name}/{route_kind}"
        require(route_kind in allowed_routes,
                f"{prefix}: route_kind must be one of {sorted(allowed_routes)}")
        require(scenario.get("status") == "passed",
                f"{prefix}: status must be passed")
        require(scenario.get("credentials_redacted") is True,
                f"{prefix}: credentials_redacted must be true")

        caller_ura = scenario.get("caller_ura")
        callee_ura = scenario.get("callee_ura")
        provider_device_ura = scenario.get("provider_device_ura")
        client_endpoint_id = scenario.get("client_endpoint_id")
        subject_ura = scenario.get("selected_resource_ura")
        session_id = scenario.get("session_id")
        require(any(has_ura_kind(caller_ura, kind) for kind in ("user", "agent", "authority")),
                f"{prefix}: caller_ura must identify an admitted User, Agent, or Authority")
        require(has_ura_kind(callee_ura, "agent"),
                f"{prefix}: callee_ura must identify the Remote Desktop SystemAgent")
        require(has_ura_kind(provider_device_ura, "device"),
                f"{prefix}: provider_device_ura must identify the execution host Device")
        require(isinstance(client_endpoint_id, str) and client_endpoint_id,
                f"{prefix}: client_endpoint_id must identify the Browser/network peer")
        require(is_ura(subject_ura), f"{prefix}: selected_resource_ura must be canonical")
        require(isinstance(session_id, str) and session_id, f"{prefix}: session_id must be recorded")

        network_fixture = scenario.get("network_fixture")
        require(isinstance(network_fixture, dict),
                f"{prefix}: network_fixture evidence must be an object")
        if not isinstance(network_fixture, dict):
            network_fixture = {}
        fixture_kind = network_fixture.get("fixture_kind")
        require(fixture_kind in allowed_runner_kinds,
                f"{prefix}: network_fixture.fixture_kind must be two_device, network_namespace, or deployment")
        require(network_fixture.get("route_constraints_applied") is True,
                f"{prefix}: network_fixture.route_constraints_applied must be true")
        require(network_fixture.get("expected_route_kind") == route_kind,
                f"{prefix}: network_fixture.expected_route_kind must match route_kind")
        allowed_route_classes = {
            lower(item)
            for item in network_fixture.get("allowed_route_classes", [])
            if isinstance(item, str)
        }
        blocked_route_classes = {
            lower(item)
            for item in network_fixture.get("blocked_route_classes", [])
            if isinstance(item, str)
        }
        require(expected_allowed_route_classes(route_kind).issubset(allowed_route_classes),
                f"{prefix}: network_fixture.allowed_route_classes must include expected route class")
        require(expected_blocked_route_classes(route_kind).issubset(blocked_route_classes),
                f"{prefix}: network_fixture.blocked_route_classes must include forbidden fallback classes")
        require(int(network_fixture.get("constraints_applied_at_ms", 0)) > 0,
                f"{prefix}: network_fixture.constraints_applied_at_ms must be positive")
        relay_allocation = network_fixture.get("relay_allocation")
        stun_binding = network_fixture.get("stun_binding")
        if route_kind == "stun_srflx":
            require(isinstance(stun_binding, dict),
                    f"{prefix}: STUN route must include server-observed stun_binding evidence")
        if not isinstance(stun_binding, dict):
            stun_binding = {}
        if route_kind in {"turn_relay", "easynet_relay"}:
            require(isinstance(relay_allocation, dict),
                    f"{prefix}: relay route must include server-observed relay_allocation evidence")
        if not isinstance(relay_allocation, dict):
            relay_allocation = {}

        abilities = scenario.get("abilities")
        require(isinstance(abilities, list) and abilities, f"{prefix}: abilities must be non-empty")
        ability_by_name = {}
        if isinstance(abilities, list):
            for ability in abilities:
                if isinstance(ability, dict) and isinstance(ability.get("name"), str):
                    ability_by_name[ability["name"]] = ability
        for ability_name in (
            "remote_desktop.create_session",
            "remote_desktop.set_description",
            "remote_desktop.watch_events",
            "remote_desktop.report_client_state",
            "remote_desktop.end_session",
        ):
            ability = ability_by_name.get(ability_name)
            require(isinstance(ability, dict), f"{prefix}: missing ability {ability_name}")
            if isinstance(ability, dict):
                require(ability.get("caller_ura") == caller_ura,
                        f"{prefix}: {ability_name} must bind caller_ura")
                require(ability.get("callee_ura") == callee_ura,
                        f"{prefix}: {ability_name} must bind callee_ura")
                require(ability.get("provider_device_ura") == provider_device_ura,
                        f"{prefix}: {ability_name} must bind provider_device_ura")
                require(ability.get("subject_ura") == subject_ura,
                        f"{prefix}: {ability_name} must bind selected Resource URA")
                if ability_name != "remote_desktop.create_session":
                    require(ability.get("session_id") == session_id,
                            f"{prefix}: {ability_name} must bind session_id")

        webrtc = scenario.get("webrtc")
        require(isinstance(webrtc, dict), f"{prefix}: webrtc evidence must be an object")
        if not isinstance(webrtc, dict):
            webrtc = {}
        require(webrtc.get("selected_resource_ura") == subject_ura,
                f"{prefix}: webrtc selected_resource_ura must bind selected Resource URA")
        require(webrtc.get("session_id") == session_id,
                f"{prefix}: webrtc session_id must bind session_id")
        require(webrtc.get("caller_ura") == caller_ura,
                f"{prefix}: webrtc caller_ura must bind the admitted caller")
        require(webrtc.get("callee_ura") == callee_ura,
                f"{prefix}: webrtc callee_ura must bind the SystemAgent callee")
        require(webrtc.get("provider_device_ura") == provider_device_ura,
                f"{prefix}: webrtc provider_device_ura must bind the execution host")
        require(webrtc.get("client_endpoint_id") == client_endpoint_id,
                f"{prefix}: webrtc client_endpoint_id must bind the Browser/network peer")
        require(webrtc.get("route_kind") == route_kind,
                f"{prefix}: webrtc route_kind must match scenario")
        require(webrtc.get("ice_connection_state") in {"connected", "completed"},
                f"{prefix}: ice_connection_state must be connected or completed")
        pair = webrtc.get("selected_candidate_pair")
        require(isinstance(pair, dict), f"{prefix}: selected_candidate_pair must be present")
        if not isinstance(pair, dict):
            pair = {}
        local_type = lower(pair.get("local_candidate_type"))
        remote_type = lower(pair.get("remote_candidate_type"))
        selected_route_class = lower(pair.get("selected_route_class"))
        expected_route_class = expected_selected_route_class(route_kind)
        candidate_types = {local_type, remote_type}
        local_description_candidate_types = {
            lower(item)
            for item in pair.get("local_description_candidate_types", [])
            if isinstance(item, str)
        }
        relay_only_sdp_proof = (
            pair.get("relay_path_proof") == "relay_only_policy_and_local_sdp"
            and lower(pair.get("ice_transport_policy")) == "relay"
            and local_description_candidate_types == {"relay"}
        )
        relay_path_proven = "relay" in candidate_types or relay_only_sdp_proof
        require(pair.get("selected") is True,
                f"{prefix}: selected_candidate_pair.selected must be true")
        require(pair.get("nominated") is True,
                f"{prefix}: selected_candidate_pair.nominated must be true")
        require(lower(pair.get("state")) == "succeeded",
                f"{prefix}: selected_candidate_pair.state must be succeeded")
        require(isinstance(pair.get("candidate_pair_id"), str) and pair.get("candidate_pair_id"),
                f"{prefix}: selected_candidate_pair.candidate_pair_id must be recorded")
        require(isinstance(pair.get("local_candidate_id"), str) and pair.get("local_candidate_id"),
                f"{prefix}: selected_candidate_pair.local_candidate_id must be recorded")
        require(isinstance(pair.get("remote_candidate_id"), str) and pair.get("remote_candidate_id"),
                f"{prefix}: selected_candidate_pair.remote_candidate_id must be recorded")
        require(selected_route_class in {"direct", "stun_srflx", "relay"},
                f"{prefix}: selected_route_class must be direct, stun_srflx, or relay")
        require(expected_route_class is None or selected_route_class == expected_route_class,
                f"{prefix}: selected_route_class must be {expected_route_class}")
        require(pair.get("protocol") in {"udp", "tcp", "UDP", "TCP"},
                f"{prefix}: candidate pair protocol must be udp or tcp")
        require(float(pair.get("current_round_trip_time_ms", 0)) >= 0,
                f"{prefix}: candidate pair RTT must be non-negative")
        require(int(pair.get("selected_pair_observed_at_ms", 0)) > int(network_fixture.get("constraints_applied_at_ms", 0)) > 0,
                f"{prefix}: selected_pair_observed_at_ms must be after network constraints")
        require(selected_route_class in allowed_route_classes,
                f"{prefix}: selected_route_class must be allowed by the network fixture")
        require(selected_route_class not in blocked_route_classes,
                f"{prefix}: selected_route_class must not be blocked by the network fixture")
        require(int(webrtc.get("bytes_sent", 0)) > 0 or int(webrtc.get("bytes_received", 0)) > 0,
                f"{prefix}: WebRTC bytes_sent or bytes_received must be positive")

        if route_kind == "direct":
            require("host" in candidate_types,
                    f"{prefix}: direct route must use host candidates")
            require("relay" not in candidate_types,
                    f"{prefix}: direct route must not use relay candidates")
        elif route_kind == "stun_srflx":
            require(local_type in {"srflx", "prflx"},
                    f"{prefix}: STUN route Browser-local candidate must be srflx/prflx")
            require(remote_type in {"host", "srflx", "prflx"},
                    f"{prefix}: STUN route provider candidate must be host/srflx/prflx")
            require(bool(local_description_candidate_types)
                    and local_description_candidate_types.issubset({"srflx", "prflx"}),
                    f"{prefix}: STUN Browser local SDP must contain only reflexive candidates")
            candidate_admission = webrtc.get("candidate_admission")
            require(isinstance(candidate_admission, dict),
                    f"{prefix}: STUN route must include directional candidate admission evidence")
            if not isinstance(candidate_admission, dict):
                candidate_admission = {}
            expected_directional_types = {
                "outbound": {"prflx", "srflx"},
                "inbound": {"host", "prflx", "srflx"},
            }
            for direction, expected_types in expected_directional_types.items():
                counters = candidate_admission.get(direction)
                require(isinstance(counters, dict),
                        f"{prefix}: STUN candidate admission must record {direction} counters")
                if not isinstance(counters, dict):
                    counters = {}
                admitted_types = {
                    lower(item)
                    for item in counters.get("allowed_types", [])
                    if isinstance(item, str)
                }
                require(admitted_types == expected_types,
                        f"{prefix}: STUN {direction} candidate admission types must be {sorted(expected_types)}")
                require(int(counters.get("accepted", 0)) > 0,
                        f"{prefix}: STUN candidate admission must accept {direction} candidates")
                require(int(counters.get("rejected", -1)) >= 0,
                        f"{prefix}: STUN candidate admission must record bounded {direction} rejections")
            require(int(candidate_admission.get("outbound", {}).get("rejected", 0)) > 0,
                    f"{prefix}: STUN candidate admission must reject outbound Browser host candidates")
            require(stun_binding.get("provider_kind") == "stun",
                    f"{prefix}: STUN binding provider_kind must be stun")
            require(stun_binding.get("binding_observed") is True,
                    f"{prefix}: STUN server must observe a binding transaction")
            require(int(stun_binding.get("binding_count", 0)) > 0,
                    f"{prefix}: STUN binding_count must be positive")
            require(
                int(network_fixture.get("constraints_applied_at_ms", 0))
                < int(stun_binding.get("observed_at_ms", 0))
                <= int(pair.get("selected_pair_observed_at_ms", 0)),
                f"{prefix}: STUN binding must be observed after fixture apply and before selected pair",
            )
        elif route_kind == "turn_relay":
            require(relay_path_proven,
                    f"{prefix}: TURN route must include relay candidate or relay-only SDP evidence")
            require(relay_allocation.get("provider_kind") == "turn",
                    f"{prefix}: TURN allocation provider_kind must be turn")
            require(relay_allocation.get("allocation_observed") is True,
                    f"{prefix}: TURN server must observe a relay allocation")
            require(int(relay_allocation.get("allocation_count", 0)) > 0,
                    f"{prefix}: TURN allocation_count must be positive")
            require(
                int(network_fixture.get("constraints_applied_at_ms", 0))
                < int(relay_allocation.get("observed_at_ms", 0))
                <= int(pair.get("selected_pair_observed_at_ms", 0)),
                f"{prefix}: TURN allocation must be observed after fixture apply and before selected pair",
            )
            require(scenario.get("turn_relay_uri_redacted") is True,
                    f"{prefix}: TURN relay URI/credentials must be redacted")
        elif route_kind == "easynet_relay":
            require(relay_path_proven,
                    f"{prefix}: EasyNet relay must include relay candidate or relay-only SDP evidence")
            require(relay_allocation.get("provider_kind") == "easynet_relay",
                    f"{prefix}: EasyNet relay allocation provider_kind must be easynet_relay")
            require(relay_allocation.get("allocation_observed") is True,
                    f"{prefix}: EasyNet relay provider must observe an allocation")
            require(int(relay_allocation.get("allocation_count", 0)) > 0,
                    f"{prefix}: EasyNet relay allocation_count must be positive")
            require(
                int(network_fixture.get("constraints_applied_at_ms", 0))
                < int(relay_allocation.get("observed_at_ms", 0))
                <= int(pair.get("selected_pair_observed_at_ms", 0)),
                f"{prefix}: EasyNet relay allocation must be observed after fixture apply and before selected pair",
            )
            require(scenario.get("route_provider") == "easynet_relay",
                    f"{prefix}: EasyNet relay must set route_provider=easynet_relay")
            require(scenario.get("relay_reachability") is True,
                    f"{prefix}: EasyNet relay reachability must be true")
            require(isinstance(scenario.get("relay_session_id"), str) and scenario.get("relay_session_id"),
                    f"{prefix}: EasyNet relay must include relay_session_id")
            relay_release = scenario.get("relay_release")
            require(isinstance(relay_release, dict),
                    f"{prefix}: EasyNet relay must include terminal release evidence")
            if not isinstance(relay_release, dict):
                relay_release = {}
            require(relay_release.get("status_code") == 409,
                    f"{prefix}: EasyNet relay terminal reacquire must return conflict")
            require(relay_release.get("terminal_reacquire_rejected") is True,
                    f"{prefix}: EasyNet relay terminal release tombstone must reject reacquire")
            require(
                int(relay_release.get("observed_at_ms", 0))
                >= int(scenario.get("terminal_receipt", {}).get("observed_at_ms", 0)),
                f"{prefix}: EasyNet relay release proof must follow the terminal receipt",
            )
            relay_refresh = scenario.get("relay_refresh")
            if relay_refresh is not None:
                require(isinstance(relay_refresh, dict),
                        f"{prefix}: relay_refresh must be a verified evidence object")
                if not isinstance(relay_refresh, dict):
                    relay_refresh = {}
                require(relay_refresh.get("status") == "passed",
                        f"{prefix}: relay refresh status must be passed")
                require(relay_refresh.get("proof_mode") == "real_hub_relay_lease_refresh",
                        f"{prefix}: relay refresh proof_mode must be real_hub_relay_lease_refresh")
                require(relay_refresh.get("component_mock") is False,
                        f"{prefix}: relay refresh must not use a component mock")
                require(relay_refresh.get("real_backend_runtime") is True,
                        f"{prefix}: relay refresh must use the real Hub/backend/runtime path")
                require(relay_refresh.get("product_complete_claim") is False,
                        f"{prefix}: relay refresh must not claim product completion")
                require(relay_refresh.get("session_id") == session_id,
                        f"{prefix}: relay refresh must bind session_id")
                require(relay_refresh.get("selected_resource_ura") == subject_ura,
                        f"{prefix}: relay refresh must bind selected Resource URA")
                require(relay_refresh.get("initial_lease_id") == scenario.get("relay_session_id"),
                        f"{prefix}: relay refresh must start from the projected Browser lease")
                require(relay_refresh.get("initial_lease_id") != relay_refresh.get("refreshed_lease_id"),
                        f"{prefix}: relay refresh must advance the lease ID")
                require(relay_refresh.get("lease_id_changed") is True,
                        f"{prefix}: relay refresh must report a changed lease ID")
                require(isinstance(relay_refresh.get("resumed_lease_id"), str)
                        and relay_refresh.get("resumed_lease_id"),
                        f"{prefix}: relay refresh must expose the post-restart lease ID")
                require(relay_refresh.get("refresh_observed_before_disconnect") is True,
                        f"{prefix}: relay refresh must be observed before daemon disconnect")
                require(relay_refresh.get("resumed_lease_reuses_or_advances_refresh") is True,
                        f"{prefix}: daemon restart must reuse or advance the refreshed lease")
                require(int(relay_refresh.get("transport_epoch", 0))
                        > int(relay_refresh.get("prior_transport_epoch", 0)),
                        f"{prefix}: relay refresh must bind a newer transport epoch")
                require(relay_refresh.get("same_public_session") is True,
                        f"{prefix}: relay refresh must preserve the public session")
                require(relay_refresh.get("new_peer_connection") is True,
                        f"{prefix}: relay refresh must replace the PeerConnection")
                require(relay_refresh.get("watch_events_reestablished") is True,
                        f"{prefix}: relay refresh must restore watch_events")
                require(int(relay_refresh.get("frames_presented_after_resume", 0)) > 0,
                        f"{prefix}: relay refresh must render media after resume")
                require(relay_refresh.get("terminal_receipt_visible") is True,
                        f"{prefix}: relay refresh must retain a visible terminal receipt")
                require(relay_refresh.get("credentials_redacted") is True,
                        f"{prefix}: relay refresh credentials must remain redacted")
                relay_refresh_covered = not errors

        media = scenario.get("media")
        require(isinstance(media, dict), f"{prefix}: media evidence must be an object")
        if not isinstance(media, dict):
            media = {}
        require(media.get("selected_resource_ura") == subject_ura,
                f"{prefix}: media selected_resource_ura must bind selected Resource URA")
        require(media.get("session_id") == session_id,
                f"{prefix}: media session_id must bind session_id")
        require(media.get("route_kind") == route_kind,
                f"{prefix}: media route_kind must match scenario")
        require(media.get("candidate_pair_id") == pair.get("candidate_pair_id"),
                f"{prefix}: media candidate_pair_id must match selected_candidate_pair")
        require(int(media.get("frames_rendered", 0)) > 0,
                f"{prefix}: media.frames_rendered must be positive")
        require(int(media.get("duration_ms", 0)) > 0,
                f"{prefix}: media.duration_ms must be positive")
        require(media.get("rendered_after_selected_pair") is True,
                f"{prefix}: media.rendered_after_selected_pair must be true")
        require(int(media.get("first_rendered_frame_at_ms", 0)) > int(pair.get("selected_pair_observed_at_ms", 0)),
                f"{prefix}: media.first_rendered_frame_at_ms must be after selected pair observation")

        terminal = scenario.get("terminal_receipt")
        require(isinstance(terminal, dict), f"{prefix}: terminal_receipt must be visible")
        if not isinstance(terminal, dict):
            terminal = {}
        require(terminal.get("terminal") is True,
                f"{prefix}: terminal_receipt.terminal must be true")
        require(terminal.get("session_id") == session_id,
                f"{prefix}: terminal_receipt must bind session_id")
        require(terminal.get("reason_code") in terminal_reasons,
                f"{prefix}: terminal_receipt.reason_code must be a known cleanup/end reason")

        scenario_reports.append({
            "name": name,
            "route_kind": route_kind,
            "ice_connection_state": webrtc.get("ice_connection_state"),
            "selected_route_class": selected_route_class,
            "candidate_pair_id": pair.get("candidate_pair_id"),
            "candidate_types": sorted(str(item) for item in candidate_types if item),
            "allowed_route_classes": sorted(allowed_route_classes),
            "blocked_route_classes": sorted(blocked_route_classes),
            "frames_rendered": media.get("frames_rendered"),
            "session_id": session_id,
            "relay_lease_refresh_resume": (
                route_kind == "easynet_relay" and isinstance(scenario.get("relay_refresh"), dict)
            ),
        })

missing_routes = sorted(required_routes - seen_routes)
require(not missing_routes, "missing route scenarios: " + ", ".join(missing_routes))

coverage = {route: route in seen_routes for route in sorted(allowed_routes)}
report = {
    "script": "tools/scripts/remoteapp-network-fallback-e2e.sh",
    "status": "failed" if errors else "passed",
    "errors": errors,
    "coverage": coverage,
    "scenario_count": len(scenario_reports),
    "scenarios": scenario_reports,
    "relay_lease_refresh_resume": relay_refresh_covered,
    "evidence_json": evidence_path,
    "product_complete_claim": False,
}
pathlib.Path(report_path).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
with open(md_path, "w", encoding="utf-8") as f:
    f.write("# RemoteApp Network Fallback E2E\n\n")
    f.write(f"- Status: `{report['status']}`\n")
    f.write(f"- Evidence: `{evidence_path}`\n")
    for route, covered in coverage.items():
        f.write(f"- {route}: `{str(covered).lower()}`\n")
    if errors:
        f.write("\n## Errors\n")
        for error in errors:
            f.write(f"- {error}\n")
if errors:
    for error in errors:
        print(error, file=sys.stderr)
    raise SystemExit(1)
PY
  python3 "$PROVENANCE_HELPER" project-report --mode "$MODE" \
    --evidence "$EVIDENCE_JSON" --report "$REPORT_JSON"
}

if [[ "$SELF_TEST" -eq 1 ]]; then
  python3 - "$EVIDENCE_JSON" <<'PY'
import json
import pathlib
import sys

subject = "easynet:///r/localhost/resource/device.receiver/streams/display.primary"
caller = "easynet:///r/localhost/user/alice"
callee = "easynet:///r/localhost/agent/device.receiver.remote-desktop"
provider_device = "easynet:///r/localhost/device/receiver"
client_endpoint = "browser-peer-self-test"
routes = [
    ("direct", "host", "host", "direct", ["direct"], ["relay"], {}),
    ("stun_srflx", "srflx", "host", "stun_srflx", ["stun_srflx"], ["direct"], {}),
    ("turn_relay", "relay", "host", "relay", ["relay"], ["direct", "stun_srflx"], {"turn_relay_uri_redacted": True}),
    ("easynet_relay", "relay", "relay", "relay", ["relay"], ["direct", "stun_srflx"], {
        "route_provider": "easynet_relay",
        "relay_reachability": True,
        "relay_session_id": "relay-self-test-1",
        "relay_release": {
            "status_code": 409,
            "terminal_reacquire_rejected": True,
            "observed_at_ms": 2100,
        },
    }),
]
scenarios = []
for route_kind, local_type, remote_type, selected_route_class, allowed, blocked, extra in routes:
    session_id = f"rd-network-{route_kind}-self-test"
    scenario = {
        "name": f"{route_kind}-self-test",
        "route_kind": route_kind,
        "status": "passed",
        "credentials_redacted": True,
        "caller_ura": caller,
        "callee_ura": callee,
        "provider_device_ura": provider_device,
        "client_endpoint_id": client_endpoint,
        "selected_resource_ura": subject,
        "session_id": session_id,
        "network_fixture": {
            "fixture_kind": "network_namespace",
            "route_constraints_applied": True,
            "expected_route_kind": route_kind,
            "allowed_route_classes": allowed,
            "blocked_route_classes": blocked,
            "constraints_applied_at_ms": 1000,
            **({
                "stun_binding": {
                    "provider_kind": "stun",
                    "binding_observed": True,
                    "binding_count": 2,
                    "observed_at_ms": 1250,
                },
            } if route_kind == "stun_srflx" else {}),
            **({
                "relay_allocation": {
                    "provider_kind": "turn" if route_kind == "turn_relay" else "easynet_relay",
                    "allocation_observed": True,
                    "allocation_count": 2,
                    "observed_at_ms": 1250,
                },
            } if route_kind in {"turn_relay", "easynet_relay"} else {}),
        },
        "abilities": [
            {
                "name": name,
                "caller_ura": caller,
                "callee_ura": callee,
                "provider_device_ura": provider_device,
                "subject_ura": subject,
                **({} if name == "remote_desktop.create_session" else {"session_id": session_id}),
            }
            for name in (
                "remote_desktop.create_session",
                "remote_desktop.set_description",
                "remote_desktop.watch_events",
                "remote_desktop.report_client_state",
                "remote_desktop.end_session",
            )
        ],
        "webrtc": {
            "selected_resource_ura": subject,
            "session_id": session_id,
            "caller_ura": caller,
            "callee_ura": callee,
            "provider_device_ura": provider_device,
            "client_endpoint_id": client_endpoint,
            "route_kind": route_kind,
            "ice_connection_state": "connected",
            "bytes_sent": 4096,
            "bytes_received": 8192,
            **({
                "candidate_admission": {
                    "outbound": {
                        "allowed_types": ["prflx", "srflx"],
                        "accepted": 1,
                        "rejected": 1,
                    },
                    "inbound": {
                        "allowed_types": ["host", "prflx", "srflx"],
                        "accepted": 1,
                        "rejected": 0,
                    },
                },
            } if route_kind == "stun_srflx" else {}),
            "selected_candidate_pair": {
                "candidate_pair_id": f"pair-{route_kind}",
                "local_candidate_type": local_type,
                "remote_candidate_type": remote_type,
                "selected_route_class": selected_route_class,
                "selected": True,
                "nominated": True,
                "state": "succeeded",
                "local_candidate_id": f"local-{route_kind}",
                "remote_candidate_id": f"remote-{route_kind}",
                "protocol": "udp",
                "current_round_trip_time_ms": 12.5,
                "selected_pair_observed_at_ms": 1500,
                **({
                    "local_description_candidate_types": ["srflx"],
                    "remote_description_candidate_types": ["host"],
                } if route_kind == "stun_srflx" else {}),
            },
        },
        "media": {
            "selected_resource_ura": subject,
            "session_id": session_id,
            "route_kind": route_kind,
            "candidate_pair_id": f"pair-{route_kind}",
            "frames_rendered": 5,
            "duration_ms": 1000,
            "rendered_after_selected_pair": True,
            "first_rendered_frame_at_ms": 1800,
        },
        "terminal_receipt": {
            "terminal": True,
            "session_id": session_id,
            "reason_code": "network_fallback_e2e_cleanup",
            "observed_at_ms": 2000,
        },
    }
    scenario.update(extra)
    if route_kind == "easynet_relay":
        scenario["relay_refresh"] = {
            "status": "passed",
            "evidence_origin": "contract_self_test",
            "proof_mode": "real_hub_relay_lease_refresh",
            "component_mock": False,
            "real_backend_runtime": True,
            "product_complete_claim": False,
            "provider": "easynet_relay",
            "session_id": session_id,
            "selected_resource_ura": subject,
            "same_public_session": True,
            "initial_lease_id": "relay-self-test-1",
            "refreshed_lease_id": "relay-self-test-2",
            "resumed_lease_id": "relay-self-test-2",
            "lease_id_changed": True,
            "refresh_observed_before_disconnect": True,
            "resumed_lease_reuses_or_advances_refresh": True,
            "prior_transport_epoch": 1,
            "transport_epoch": 2,
            "new_peer_connection": True,
            "watch_events_reestablished": True,
            "frames_presented_after_resume": 2,
            "terminal_receipt_visible": True,
            "credentials_redacted": True,
        }
    scenarios.append(scenario)

evidence = {
    "status": "passed",
    "evidence_origin": "contract_self_test",
    "proof_mode": "real_network_fallback_matrix",
    "runner_kind": "network_namespace",
    "component_mock": False,
    "real_backend_runtime": True,
    "product_complete_claim": False,
    "scenarios": scenarios,
}
pathlib.Path(sys.argv[1]).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
  validate_evidence
  echo "remoteapp-network-fallback-e2e self-test ok"
  exit 0
fi

if [[ "$MODE" != "run" ]]; then
  write_report "skipped" "set EASYNET_REMOTEAPP_NETWORK_FALLBACK_E2E=1 or pass --run"
  echo "[remoteapp-network-fallback-e2e] skipped: $REPORT_MD"
  exit 0
fi

if [[ -n "$EVIDENCE_INPUT" ]]; then
  [[ -f "$EVIDENCE_INPUT" ]] || {
    write_report "failed" "evidence json does not exist: $EVIDENCE_INPUT"
    echo "[remoteapp-network-fallback-e2e] missing evidence json: $EVIDENCE_INPUT" >&2
    exit 1
  }
  cp "$EVIDENCE_INPUT" "$EVIDENCE_JSON"
elif [[ -n "$RUNNER_CMD" ]]; then
  export EASYNET_REMOTEAPP_NETWORK_FALLBACK_EVIDENCE_JSON="$EVIDENCE_JSON"
  if ! bash -lc "$RUNNER_CMD" >"$RUNNER_STDOUT" 2>"$RUNNER_STDERR"; then
    write_report "failed" "runner command failed"
    echo "[remoteapp-network-fallback-e2e] runner command failed" >&2
    cat "$RUNNER_STDERR" >&2 || true
    exit 1
  fi
  [[ -f "$EVIDENCE_JSON" ]] || {
    write_report "failed" "runner did not write evidence json"
    echo "[remoteapp-network-fallback-e2e] runner did not write $EVIDENCE_JSON" >&2
    exit 1
  }
else
  write_report "failed" "--run requires --evidence-json or --runner-cmd"
  echo "[remoteapp-network-fallback-e2e] --run requires --evidence-json or --runner-cmd" >&2
  exit 1
fi

validate_evidence
echo "[remoteapp-network-fallback-e2e] PASS: $REPORT_MD"
