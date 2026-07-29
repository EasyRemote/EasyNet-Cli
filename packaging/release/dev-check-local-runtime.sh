#!/usr/bin/env bash
# dev-check-local-runtime.sh - audit whether the local EasyNet device is
# truly product-online after a dev install / join / runtime start cycle.
#
# This is a developer verification harness, not a replacement for daemon
# state machines. It reads public CLI JSON surfaces and local state files,
# then explains why the local device is not online.
#
# Usage:
#   packaging/release/dev-check-local-runtime.sh
#   packaging/release/dev-check-local-runtime.sh --install-local --restart --wait-online 30
#   packaging/release/dev-check-local-runtime.sh --check-hub-pairing-contract --no-fail
#   packaging/release/dev-check-local-runtime.sh --json --no-fail
#
# Exit codes:
#   0 - true product-online, or --no-fail was set
#   1 - audit completed and the local device is not true product-online
#   2 - usage / prerequisite error
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cli_root="$(cd "$script_dir/../.." && pwd)"

easynet_bin="${EASYNET_BIN:-easynet}"
install_local=0
install_profile="release"
start_runtime=0
restart_runtime=0
wait_online=0
json_output=0
no_fail=0
check_hub_pairing_contract=0

usage() {
    sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --install-local)
            install_local=1
            shift
            ;;
        --debug)
            install_profile="debug"
            shift
            ;;
        --release)
            install_profile="release"
            shift
            ;;
        --start)
            start_runtime=1
            shift
            ;;
        --restart)
            restart_runtime=1
            shift
            ;;
        --wait-online)
            if [ $# -lt 2 ]; then
                echo "dev-check-local-runtime.sh: --wait-online requires seconds" >&2
                exit 2
            fi
            wait_online="$2"
            shift 2
            ;;
        --json)
            json_output=1
            shift
            ;;
        --check-hub-pairing-contract)
            check_hub_pairing_contract=1
            shift
            ;;
        --no-fail)
            no_fail=1
            shift
            ;;
        --easynet-bin)
            if [ $# -lt 2 ]; then
                echo "dev-check-local-runtime.sh: --easynet-bin requires a path" >&2
                exit 2
            fi
            easynet_bin="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "dev-check-local-runtime.sh: unknown arg: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$wait_online" in
    ''|*[!0-9]*)
        echo "dev-check-local-runtime.sh: --wait-online must be a non-negative integer" >&2
        exit 2
        ;;
esac

if [ "$install_local" -eq 1 ]; then
    install_args=("--$install_profile")
    "$script_dir/dev-install-local.sh" "${install_args[@]}"
    easynet_bin="/usr/local/bin/easynet"
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "dev-check-local-runtime.sh: python3 is required" >&2
    exit 2
fi

if [ "$restart_runtime" -eq 1 ]; then
    "$easynet_bin" runtime stop >/dev/null 2>&1 || true
    "$easynet_bin" runtime start
elif [ "$start_runtime" -eq 1 ]; then
    "$easynet_bin" runtime start
fi

export EASYNET_AUDIT_BIN="$easynet_bin"
export EASYNET_AUDIT_JSON="$json_output"
export EASYNET_AUDIT_WAIT_SECONDS="$wait_online"
export EASYNET_AUDIT_NO_FAIL="$no_fail"
export EASYNET_AUDIT_REPO="$cli_root"
export EASYNET_AUDIT_CHECK_HUB_PAIRING_CONTRACT="$check_hub_pairing_contract"

python3 - <<'PY'
import json
import os
import pathlib
import shutil
import socket
import subprocess
import sys
import time
import urllib.parse


EASYNET = os.environ["EASYNET_AUDIT_BIN"]
JSON_OUTPUT = os.environ["EASYNET_AUDIT_JSON"] == "1"
WAIT_SECONDS = int(os.environ["EASYNET_AUDIT_WAIT_SECONDS"])
NO_FAIL = os.environ["EASYNET_AUDIT_NO_FAIL"] == "1"
CHECK_HUB_PAIRING_CONTRACT = (
    os.environ["EASYNET_AUDIT_CHECK_HUB_PAIRING_CONTRACT"] == "1"
)
STATE_DIR = pathlib.Path.home() / ".easynet"


def run(args, timeout=10, env=None):
    try:
        completed = subprocess.run(
            args,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            env=env,
        )
        return {
            "ok": completed.returncode == 0,
            "returncode": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
    except FileNotFoundError as exc:
        return {
            "ok": False,
            "returncode": 127,
            "stdout": "",
            "stderr": str(exc),
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "ok": False,
            "returncode": 124,
            "stdout": exc.stdout or "",
            "stderr": exc.stderr or f"timed out after {timeout}s",
        }


def run_json(label, args, timeout=10):
    result = run(args, timeout=timeout)
    data = None
    parse_error = None
    if result["stdout"].strip():
        try:
            data = json.loads(result["stdout"])
        except json.JSONDecodeError as exc:
            parse_error = str(exc)
    result.update({"label": label, "data": data, "parse_error": parse_error})
    return result


def read_json(path):
    try:
        return json.loads(path.read_text())
    except FileNotFoundError:
        return None
    except Exception as exc:
        return {"_error": str(exc)}


def auth_session():
    raw = read_json(STATE_DIR / "auth.json")
    if not isinstance(raw, dict):
        return raw
    out = dict(raw)
    for key in list(out):
        lowered = key.lower()
        if "token" in lowered or "secret" in lowered:
            out[key] = "<redacted>"
    return out


def sanitized_credentials():
    raw = read_json(STATE_DIR / "credentials.json")
    if not isinstance(raw, dict):
        return raw
    out = dict(raw)
    for key in list(out):
        lowered = key.lower()
        if "token" in lowered or "signature" in lowered or "secret" in lowered:
            out[key] = "<redacted>"
        if key == "hub_tls_ca_pem_b64":
            out[key] = "<redacted>"
    realm = out.get("realm")
    user_id = out.get("user_id")
    node_id = out.get("node_id")
    if realm and user_id:
        out["user_ura"] = f"easynet:///r/{realm}/user/{user_id}"
    if realm and node_id:
        out["device_ura"] = f"easynet:///r/{realm}/device/{node_id}"
    return out


def command_path(program):
    if os.path.sep in program:
        return str(pathlib.Path(program).expanduser())
    found = shutil.which(program)
    return found or program


def ps_for_pid(pid):
    if not pid:
        return None
    result = run(["ps", "-p", str(pid), "-o", "pid=,ppid=,command="], timeout=3)
    return result["stdout"].strip() if result["ok"] else None


def hub_listener(endpoint):
    if not endpoint or shutil.which("lsof") is None:
        return None
    parsed = urllib.parse.urlparse(endpoint)
    port = parsed.port
    if not port:
        return None
    result = run(["lsof", "-nP", f"-iTCP:{port}", "-sTCP:LISTEN"], timeout=5)
    lines = [line for line in result["stdout"].splitlines() if line.strip()]
    return {
        "endpoint": endpoint,
        "port": port,
        "ok": result["ok"],
        "lines": lines,
    }


def current_boot_log_lines(path):
    lines = path.read_text(errors="replace").splitlines()
    for index in range(len(lines) - 1, -1, -1):
        line = lines[index]
        if "[boot] kind=stage_started stage=kernel" in line:
            return lines[index:]
    return lines[-500:]


def active_session_log_lines(lines):
    for index in range(len(lines) - 1, -1, -1):
        line = lines[index]
        if (
            "[session] kind=connection_state_projected" in line
            and "transition_id=T10_ADMIT_PRESENCE" in line
        ):
            return lines[index + 1 :]
    return lines


def recent_session_errors():
    path = STATE_DIR / "logs" / "easynet-daemon.log"
    try:
        lines = current_boot_log_lines(path)
    except FileNotFoundError:
        return []
    lines = active_session_log_lines(lines)
    needles = (
        "bidi_error_reconnecting",
        "initial_admission_failed",
        "user_trust_sync_publish_failed",
        "hub_trust_sync_resolve_failed",
        "advertise_abilities_prelude_failed",
    )
    return [line for line in lines if any(needle in line for needle in needles)][-8:]


def extract_joined_node_id(output):
    for line in output.splitlines():
        stripped = line.strip()
        if stripped.startswith("node_id"):
            parts = stripped.split(":", 1)
            if len(parts) == 2:
                node_id = parts[1].strip()
                if node_id:
                    return node_id
    return None


def joined_node_id_from_probe_home(temp_home):
    credentials = read_json(temp_home / ".easynet" / "credentials.json")
    if isinstance(credentials, dict):
        node_id = str(credentials.get("node_id") or "").strip()
        if node_id:
            return node_id
    return None


def cleanup_contract_probe_device(node_id):
    if not node_id:
        return {
            "attempted": False,
            "ok": None,
            "reason": "join did not expose node_id",
        }
    result = run([EASYNET, "auth", "device-remove", node_id, "--yes"], timeout=15)
    return {
        "attempted": True,
        "ok": result["ok"],
        "node_id": node_id,
        "returncode": result["returncode"],
        "stdout": result["stdout"],
        "stderr": result["stderr"],
    }


def check_live_hub_pairing_contract():
    if not CHECK_HUB_PAIRING_CONTRACT:
        return {"enabled": False}

    session = read_json(STATE_DIR / "auth.json")
    if not isinstance(session, dict):
        return {
            "enabled": True,
            "ok": None,
            "skipped": True,
            "reason": "auth session missing or unreadable",
        }
    hub_url = str(session.get("hub_url") or "").strip()
    if not hub_url:
        return {
            "enabled": True,
            "ok": None,
            "skipped": True,
            "reason": "auth session missing hub_url",
        }

    pair = run([EASYNET, "auth", "pair", "--quiet"], timeout=20)
    if not pair["ok"]:
        return {
            "enabled": True,
            "ok": False,
            "stage": "mint_pairing_token",
            "returncode": pair["returncode"],
            "stdout": pair["stdout"],
            "stderr": pair["stderr"],
        }
    token = pair["stdout"].strip()
    if not token:
        return {
            "enabled": True,
            "ok": False,
            "stage": "mint_pairing_token",
            "returncode": pair["returncode"],
            "stdout": pair["stdout"],
            "stderr": pair["stderr"] or "auth pair returned an empty token",
        }

    # macOS Unix-domain socket paths are capped by SUN_LEN. The daemon/keyring
    # place sockets under HOME, so this probe must prefer a short HOME root
    # instead of the long per-user $TMPDIR path.
    temp_root = pathlib.Path("/tmp")
    if not temp_root.is_dir():
        temp_root = pathlib.Path(os.environ.get("TMPDIR", "/tmp"))
    temp_home = temp_root / f"en-probe-{os.getpid()}-{int(time.time() * 1000)}"
    temp_home.mkdir(parents=True, exist_ok=False)
    probe_env = dict(os.environ)
    probe_env["HOME"] = str(temp_home)
    join = run(
        [EASYNET, "join", token, "--hub", hub_url, "--boot", "no", "--yes"],
        timeout=45,
        env=probe_env,
    )
    node_id = None
    if join["ok"]:
        node_id = joined_node_id_from_probe_home(temp_home)
        if not node_id:
            node_id = extract_joined_node_id(join["stdout"] + "\n" + join["stderr"])
    cleanup = cleanup_contract_probe_device(node_id) if join["ok"] else {"attempted": False}
    temp_home_cleanup = {"removed": False, "path": str(temp_home)}
    if join["ok"]:
        try:
            shutil.rmtree(temp_home)
            temp_home_cleanup["removed"] = True
        except Exception as exc:
            temp_home_cleanup["error"] = str(exc)
    return {
        "enabled": True,
        "ok": join["ok"],
        "hub_url": hub_url,
        "stage": "join_validate_pairing_contract",
        "returncode": join["returncode"],
        "stdout": join["stdout"],
        "stderr": join["stderr"],
        "joined_node_id": node_id,
        "cleanup": cleanup,
        "temp_home": temp_home_cleanup,
    }


def collect_once():
    version = run([EASYNET, "--version"], timeout=5)
    runtime = run_json("runtime_status", [EASYNET, "runtime", "status", "--json"], timeout=10)
    doctor = run_json("doctor", [EASYNET, "doctor", "--json"], timeout=10)
    devices = run_json(
        "device_list",
        [EASYNET, "device", "list", "--state", "all", "--format", "json"],
        timeout=10,
    )
    peers = run_json("federation_peers", [EASYNET, "federation", "peers", "--json"], timeout=10)
    discover = run_json(
        "federation_discover",
        [EASYNET, "federation", "discover", "--json"],
        timeout=10,
    )
    credentials = sanitized_credentials()
    runtime_data = runtime.get("data") if isinstance(runtime.get("data"), dict) else {}
    runtime_projection = runtime_data.get("runtime") if isinstance(runtime_data, dict) else None
    daemon = runtime_data.get("daemon") if isinstance(runtime_data, dict) else None
    connection = runtime_data.get("connection") if isinstance(runtime_data, dict) else None
    presence = runtime_data.get("product_presence") if isinstance(runtime_data, dict) else None
    hub_endpoint = None
    if isinstance(connection, dict):
        hub_endpoint = connection.get("hub_endpoint")
    if not hub_endpoint and isinstance(runtime_projection, dict):
        hub_endpoint = runtime_projection.get("hub")
    if not hub_endpoint and isinstance(credentials, dict):
        hub_endpoint = credentials.get("hub_endpoint")

    pid = None
    if isinstance(runtime_projection, dict):
        pid = runtime_projection.get("pid")
    if not pid and isinstance(daemon, dict):
        pid = daemon.get("pid")

    facts = {
        "cli": {
            "requested": EASYNET,
            "path": command_path(EASYNET),
            "version": version["stdout"].strip() if version["ok"] else None,
            "version_error": version["stderr"].strip() if not version["ok"] else None,
        },
        "state_dir": str(STATE_DIR),
        "credentials": credentials,
        "auth_session": auth_session(),
        "hub_pairing_contract": check_live_hub_pairing_contract(),
        "runtime_status": runtime,
        "doctor": doctor,
        "device_list": devices,
        "federation_peers": peers,
        "federation_discover": discover,
        "process": {
            "pid": pid,
            "ps": ps_for_pid(pid),
        },
        "hub_listener": hub_listener(hub_endpoint),
        "recent_session_errors": recent_session_errors(),
    }
    facts["diagnosis"] = diagnose(facts)
    return facts


def self_device_record(facts):
    creds = facts.get("credentials")
    node_id = creds.get("node_id") if isinstance(creds, dict) else None
    data = facts["device_list"].get("data")
    nodes = data.get("nodes") if isinstance(data, dict) else None
    if not isinstance(nodes, list):
        return None
    for node in nodes:
        if not isinstance(node, dict):
            continue
        if node.get("is_self") is True:
            return node
    if node_id:
        for node in nodes:
            if isinstance(node, dict) and node.get("node_id") == node_id:
                return node
    return None


def self_discover_record(facts):
    creds = facts.get("credentials")
    device_ura = creds.get("device_ura") if isinstance(creds, dict) else None
    data = facts["federation_discover"].get("data")
    entries = data.get("entries") if isinstance(data, dict) else None
    if not isinstance(entries, list) or not device_ura:
        return None
    for entry in entries:
        if isinstance(entry, dict) and entry.get("agent_ura") == device_ura:
            return entry
    return None


def doctor_check(facts, name):
    data = facts["doctor"].get("data")
    checks = data.get("checks") if isinstance(data, dict) else None
    if not isinstance(checks, list):
        return None
    for check in checks:
        if isinstance(check, dict) and check.get("name") == name:
            return check
    return None


def add_issue(issues, severity, code, message, hint=None):
    issues.append({
        "severity": severity,
        "code": code,
        "message": message,
        "hint": hint,
    })


def diagnose(facts):
    issues = []
    runtime_data = facts["runtime_status"].get("data")
    credentials = facts.get("credentials")
    joined = isinstance(credentials, dict) and bool(credentials.get("node_id"))
    runtime_state = runtime_data.get("runtime_status") if isinstance(runtime_data, dict) else None
    connection = runtime_data.get("connection") if isinstance(runtime_data, dict) else None
    daemon = runtime_data.get("daemon") if isinstance(runtime_data, dict) else None
    presence = runtime_data.get("product_presence") if isinstance(runtime_data, dict) else None
    session_admitted = presence.get("session_admitted") if isinstance(presence, dict) else None
    directory_status = presence.get("directory_status") if isinstance(presence, dict) else None
    connection_state = connection.get("state") if isinstance(connection, dict) else None
    state_code = connection.get("state_code") if isinstance(connection, dict) else None
    transition = connection.get("transition_id") if isinstance(connection, dict) else None
    source = connection.get("source") if isinstance(connection, dict) else None

    if not joined:
        add_issue(
            issues,
            "fail",
            "NOT_JOINED",
            "No local device credentials were found.",
            "Run `easynet device join <token>` before runtime start.",
        )
        if state_code == "F520" or connection_state == "START_FAILED_CREDENTIAL_VERIFY":
            add_issue(
                issues,
                "fail",
                "CREDENTIAL_REJECTED_BY_HUB",
                "The last runtime start failed during Hub credential verification.",
                "The Hub returned credential rejection or the device was removed; create a new pairing token and run `easynet device join <token>`.",
            )

    if not facts["runtime_status"].get("ok"):
        add_issue(
            issues,
            "fail",
            "RUNTIME_STATUS_UNAVAILABLE",
            "Could not read `easynet runtime status --json`.",
            facts["runtime_status"].get("stderr", "").strip() or None,
        )
    elif runtime_state != "running":
        add_issue(
            issues,
            "fail",
            "RUNTIME_NOT_RUNNING",
            f"Runtime status is `{runtime_state}`.",
            "Run `easynet runtime start`.",
        )

    if runtime_state == "running":
        if not isinstance(daemon, dict):
            add_issue(
                issues,
                "fail",
                "DAEMON_DISCOVERY_MISSING",
                "Runtime projection exists but daemon discovery is missing.",
                "Restart with `easynet runtime stop && easynet runtime start`.",
            )
        else:
            if daemon.get("control_accepting") is not True:
                add_issue(issues, "fail", "CONTROL_NOT_ACCEPTING", "Daemon control socket is not accepting.")
            if daemon.get("invocation_accepting") is not True:
                add_issue(issues, "fail", "INVOCATION_NOT_ACCEPTING", "Daemon invocation socket is not accepting.")

    if session_admitted is not True:
        details = ", ".join(
            part for part in [
                f"state={connection_state}" if connection_state else None,
                f"code={state_code}" if state_code else None,
                f"transition={transition}" if transition else None,
                f"source={source}" if source else None,
                f"directory_status={directory_status}" if directory_status else None,
            ] if part
        )
        add_issue(
            issues,
            "fail",
            "SESSION_NOT_ADMITTED",
            f"Product session is not admitted ({details or 'no connection snapshot'}).",
            "Inspect recent session errors below; a live daemon socket alone is not online.",
        )
    elif connection_state != "FRONTEND_CONNECTED":
        add_issue(
            issues,
            "warn",
            "CONNECTION_STATE_NOT_FRONTEND_CONNECTED",
            f"Session is admitted but connection state is `{connection_state}`.",
        )

    doctor_connection = doctor_check(facts, "connection state")
    if (
        isinstance(doctor_connection, dict)
        and doctor_connection.get("status") == "ok"
        and session_admitted is not True
    ):
        add_issue(
            issues,
            "warn",
            "DOCTOR_MASKED_DEGRADED",
            "`easynet doctor` reports connection state ok, but product session is not admitted.",
            "Use this script or `runtime status --json` until doctor is upgraded everywhere.",
        )

    user_key_check = doctor_check(facts, "user signing key")
    if isinstance(user_key_check, dict) and user_key_check.get("status") != "ok":
        add_issue(
            issues,
            "warn",
            "USER_SIGNING_KEY_NOT_OK",
            user_key_check.get("detail") or "User signing key check is not ok.",
            user_key_check.get("hint"),
        )

    self_node = self_device_record(facts)
    if isinstance(self_node, dict) and session_admitted is not True:
        online = self_node.get("online")
        state = self_node.get("state")
        if online is True or state in {"HEALTHY", "ONLINE", "active"}:
            add_issue(
                issues,
                "warn",
                "DEVICE_LIST_WEAKER_THAN_SESSION",
                f"`device list` reports self as online/state={state}, but session_admitted=false.",
                "Treat device-list/federation rows as inventory, not the true online gate.",
            )

    self_entry = self_discover_record(facts)
    if isinstance(self_entry, dict) and session_admitted is not True and self_entry.get("status") == "active":
        add_issue(
            issues,
            "warn",
            "FEDERATION_ENTRY_ACTIVE_WHILE_SESSION_DOWN",
            "Federation discover has an active self entry while product session is not admitted.",
            "The read model can be stale or weaker than the session admission gate.",
        )

    listener = facts.get("hub_listener")
    pairing_contract = facts.get("hub_pairing_contract")
    listener_lines = listener.get("lines") if isinstance(listener, dict) else []
    if listener_lines and any("com.docke" in line or "docker" in line.lower() for line in listener_lines):
        add_issue(
            issues,
            "info",
            "HUB_LISTENER_DOCKER",
            "Hub endpoint is currently served by a Docker-owned process.",
            "If admission fails, rebuild/restart the Hub container with the current EasyNet code.",
        )

    if isinstance(pairing_contract, dict) and pairing_contract.get("enabled"):
        if pairing_contract.get("skipped"):
            add_issue(
                issues,
                "info",
                "HUB_PAIRING_CONTRACT_CHECK_SKIPPED",
                f"Live Hub pairing contract probe skipped: {pairing_contract.get('reason')}.",
                "Run `easynet auth login <email> --hub <url>` and retry with --check-hub-pairing-contract.",
            )
        elif pairing_contract.get("ok") is not True:
            detail = (
                pairing_contract.get("stderr", "").strip()
                or pairing_contract.get("stdout", "").strip()
                or "no error detail"
            )
            add_issue(
                issues,
                "fail",
                "HUB_PAIRING_CONTRACT_CHECK_FAILED",
                f"Live Hub pairing contract probe failed at {pairing_contract.get('stage')}: {detail}",
                "Rebuild/restart the Hub container from the current EasyNet backend source, then retry.",
            )
        else:
            cleanup = pairing_contract.get("cleanup")
            if isinstance(cleanup, dict) and cleanup.get("attempted") and cleanup.get("ok") is not True:
                add_issue(
                    issues,
                    "warn",
                    "HUB_PAIRING_CONTRACT_PROBE_CLEANUP_FAILED",
                    f"Temporary probe device {cleanup.get('node_id')} was paired but not removed.",
                    cleanup.get("stderr", "").strip() or cleanup.get("stdout", "").strip() or None,
                )

    errors = facts.get("recent_session_errors") or []
    if errors:
        last_error = errors[-1]
        hint = None
        if "cannot author `user` trust row" in last_error:
            hint = "Hub admission is rejecting paired-user trust bootstrap; update/restart the Hub side."
        elif "sign descriptor-bound invocation" in last_error:
            hint = "Prelude signing/material is incomplete; check descriptor-bound signing setup on the Hub path."
        add_issue(
            issues,
            "info",
            "RECENT_SESSION_ERROR",
            last_error,
            hint,
        )

    connected = (
        joined
        and runtime_state == "running"
        and isinstance(daemon, dict)
        and daemon.get("control_accepting") is True
        and daemon.get("invocation_accepting") is True
        and session_admitted is True
        and connection_state == "FRONTEND_CONNECTED"
    )
    return {
        "true_online": connected,
        "joined": joined,
        "runtime_status": runtime_state,
        "session_admitted": session_admitted,
        "directory_status": directory_status,
        "connection_state": connection_state,
        "state_code": state_code,
        "transition_id": transition,
        "source": source,
        "self_device": self_node,
        "self_federation_entry": self_entry,
        "issues": issues,
    }


def collect_with_wait():
    deadline = time.time() + WAIT_SECONDS
    facts = collect_once()
    while WAIT_SECONDS > 0 and not facts["diagnosis"]["true_online"] and time.time() < deadline:
        time.sleep(1)
        facts = collect_once()
    return facts


def print_human(facts):
    diagnosis = facts["diagnosis"]
    credentials = facts.get("credentials")
    runtime_data = facts["runtime_status"].get("data")
    runtime = runtime_data.get("runtime") if isinstance(runtime_data, dict) else None
    daemon = runtime_data.get("daemon") if isinstance(runtime_data, dict) else None
    listener = facts.get("hub_listener")
    peers = facts["federation_peers"].get("data")
    pairing_contract = facts.get("hub_pairing_contract")

    print("EasyNet local runtime audit")
    print("")
    print(f"CLI: {facts['cli']['path']} {facts['cli'].get('version') or ''}".rstrip())
    print(f"State dir: {facts['state_dir']}")
    print("")
    print("Join")
    if isinstance(credentials, dict):
        print(f"  joined: yes")
        print(f"  realm: {credentials.get('realm') or '-'}")
        print(f"  device: {credentials.get('device_ura') or credentials.get('node_id') or '-'}")
        print(f"  user: {credentials.get('user_ura') or credentials.get('username') or '-'}")
        print(f"  hub: {credentials.get('hub_endpoint') or '-'}")
    else:
        print("  joined: no")
    print("")
    print("Runtime")
    print(f"  runtime_status: {diagnosis.get('runtime_status') or '-'}")
    print(f"  true_online: {'yes' if diagnosis['true_online'] else 'no'}")
    print(f"  session_admitted: {diagnosis.get('session_admitted')}")
    print(
        "  connection: "
        f"{diagnosis.get('connection_state') or '-'} "
        f"[{diagnosis.get('state_code') or '-'}] "
        f"transition={diagnosis.get('transition_id') or '-'} "
        f"source={diagnosis.get('source') or '-'}"
    )
    if isinstance(runtime, dict):
        print(f"  runtime_pid: {runtime.get('pid')}")
    if isinstance(daemon, dict):
        print(
            "  daemon: "
            f"pid={daemon.get('pid')} "
            f"control={daemon.get('control_accepting')} "
            f"invocation={daemon.get('invocation_accepting')}"
        )
    if facts["process"].get("ps"):
        print(f"  process: {facts['process']['ps']}")
    print("")
    print("Inventory")
    self_node = diagnosis.get("self_device")
    self_entry = diagnosis.get("self_federation_entry")
    print(f"  device_list_self: {json.dumps(self_node, ensure_ascii=False) if self_node else '-'}")
    print(f"  federation_self: {json.dumps(self_entry, ensure_ascii=False) if self_entry else '-'}")
    if isinstance(peers, dict):
        print(f"  federated_peers: {json.dumps(peers.get('federated_peers'), ensure_ascii=False)}")
        print(f"  trusted_hubs: {len(peers.get('trusted_hubs') or [])}")
    if isinstance(listener, dict):
        print("  hub_listener:")
        for line in listener.get("lines") or []:
            print(f"    {line}")
    print("")
    print("Hub pairing contract")
    if not isinstance(pairing_contract, dict) or not pairing_contract.get("enabled"):
        print("  checked: no")
    elif pairing_contract.get("skipped"):
        print("  checked: skipped")
        print(f"  reason: {pairing_contract.get('reason') or '-'}")
    else:
        print("  checked: yes")
        print(f"  ok: {pairing_contract.get('ok')}")
        print(f"  hub: {pairing_contract.get('hub_url') or '-'}")
        print(f"  temporary_node: {pairing_contract.get('joined_node_id') or '-'}")
        temp_home = pairing_contract.get("temp_home")
        if isinstance(temp_home, dict) and not temp_home.get("removed"):
            print(f"  temp_home: {temp_home.get('path') or '-'}")
        cleanup = pairing_contract.get("cleanup")
        if isinstance(cleanup, dict):
            print(f"  cleanup: {cleanup.get('ok') if cleanup.get('attempted') else 'not-attempted'}")
    print("")
    print("Issues")
    issues = diagnosis.get("issues") or []
    if not issues:
        print("  none")
    else:
        for issue in issues:
            print(f"  [{issue['severity'].upper()}] {issue['code']}: {issue['message']}")
            if issue.get("hint"):
                print(f"         hint: {issue['hint']}")
    errors = facts.get("recent_session_errors") or []
    if errors:
        print("")
        print("Recent session errors")
        for line in errors[-5:]:
            print(f"  {line}")


facts = collect_with_wait()
if JSON_OUTPUT:
    print(json.dumps(facts, indent=2, sort_keys=True, ensure_ascii=False))
else:
    print_human(facts)

if facts["diagnosis"]["true_online"] or NO_FAIL:
    sys.exit(0)
sys.exit(1)
PY
