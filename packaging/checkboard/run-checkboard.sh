#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
manifest="$script_dir/tests.manifest.tsv"
run_id="$(date +%Y%m%d-%H%M%S)"
out_root="$repo_root/target/e2e/checkboard/$run_id"
filter=""
include_docker=0
include_external=0
include_manual=0
run_all=0
stop_on_fail=0
dry_run=0

usage() {
    cat <<'USAGE'
Usage:
  packaging/checkboard/run-checkboard.sh [options]

Options:
  --manifest PATH        TSV manifest. Default: packaging/checkboard/tests.manifest.tsv.
  --output DIR           Output directory. Default: target/e2e/checkboard/<timestamp>.
  --filter TEXT          Run rows whose id, path, tags, or description contains TEXT.
  --include-docker       Include rows tagged docker.
  --include-external     Include rows tagged external.
  --include-manual       Include rows tagged manual.
  --all                  Include docker, external, and manual rows.
  --stop-on-fail         Stop after the first failed selected row.
  --dry-run              Print selected commands without executing them.
  -h, --help             Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --manifest)
            manifest="${2:?--manifest requires a value}"
            shift 2
            ;;
        --output)
            out_root="${2:?--output requires a value}"
            shift 2
            ;;
        --filter)
            filter="${2:?--filter requires a value}"
            shift 2
            ;;
        --include-docker)
            include_docker=1
            shift
            ;;
        --include-external)
            include_external=1
            shift
            ;;
        --include-manual)
            include_manual=1
            shift
            ;;
        --all)
            run_all=1
            include_docker=1
            include_external=1
            include_manual=1
            shift
            ;;
        --stop-on-fail)
            stop_on_fail=1
            shift
            ;;
        --dry-run)
            dry_run=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 64
            ;;
    esac
done

has_tag() {
    local tags="$1"
    local wanted="$2"
    [[ ",$tags," == *",$wanted,"* ]]
}

matches_filter() {
    local haystack="$1"
    [[ -z "$filter" || "$haystack" == *"$filter"* ]]
}

should_skip() {
    local tags="$1"
    if [[ "$run_all" == "1" ]]; then
        return 1
    fi
    if has_tag "$tags" "docker" && [[ "$include_docker" != "1" ]]; then
        echo "tagged docker; pass --include-docker or --all"
        return 0
    fi
    if has_tag "$tags" "external" && [[ "$include_external" != "1" ]]; then
        echo "tagged external; pass --include-external or --all"
        return 0
    fi
    if has_tag "$tags" "manual" && [[ "$include_manual" != "1" ]]; then
        echo "tagged manual; pass --include-manual or --all"
        return 0
    fi
    return 1
}

now_ms() {
    case "$(uname -s)" in
        Darwin)
            python3 -c 'import time; print(int(time.time() * 1000))'
            ;;
        *)
            date +%s%3N
            ;;
    esac
}

safe_id() {
    printf '%s' "$1" | tr -c 'A-Za-z0-9_.-' '_'
}

tsv_field() {
    printf '%s' "$1" | tr '\t\r\n' '   '
}

write_error_report() {
    local case_dir="$1"
    local id="$2"
    local path="$3"
    local description="$4"
    local exit_code="$5"
    local duration="$6"
    local stdout_log="$7"
    local stderr_log="$8"
    {
        echo "# E2E Failure: $id"
        echo
        echo "- path: \`$path\`"
        echo "- description: $description"
        echo "- exit_code: \`$exit_code\`"
        echo "- duration_ms: \`$duration\`"
        echo "- stdout_log: \`$stdout_log\`"
        echo "- stderr_log: \`$stderr_log\`"
        echo
        echo "## Command"
        echo
        echo '```bash'
        cat "$case_dir/command.sh"
        echo '```'
        echo
        echo "## stderr tail"
        echo
        echo '```text'
        tail -120 "$case_dir/stderr.log" || true
        echo '```'
        echo
        echo "## stdout tail"
        echo
        echo '```text'
        tail -80 "$case_dir/stdout.log" || true
        echo '```'
    } >"$case_dir/error.md"
}

write_case_evidence_report() {
    local case_dir="$1"
    local stdout_log="$2"
    local nested_report
    nested_report="$(awk '/^==> report: / { sub(/^==> report: /, ""); print; exit }' "$stdout_log" || true)"
    [[ -n "$nested_report" ]] || return 1
    [[ -f "$nested_report" ]] || return 1

    local nested_dir
    local nested_json
    nested_dir="$(cd "$(dirname "$nested_report")" && pwd)"
    nested_json="$nested_dir/report.json"
    local evidence="$case_dir/evidence.md"

    {
        echo "# Architecture Evidence"
        echo
        echo "- nested_report: \`$nested_report\`"
        if [[ -f "$nested_json" ]]; then
            echo "- nested_report_json: \`$nested_json\`"
        fi
        echo
        if [[ -f "$nested_json" ]] && command -v jq >/dev/null 2>&1; then
            echo "## Evidence Scope"
            echo
            echo "This case probes daemon-local custom agent/ability/skill materialization"
            echo "against Hub/frontend-equivalent auth projection. It does not claim"
            echo "branch-wide SDK, receipt, LocalRuntime, or provider convergence."
            echo
            echo "## Projection Facts"
            echo
            echo '```json'
            jq '{
              baseline_counts,
              device_online_projection,
              projection_assertions
            }' "$nested_json"
            echo '```'
            echo
            echo "## Evidence Interpretation"
            echo
            jq -r '
              def all_true($object): ([($object // {})[]] | all(. == true));
              def any_false($object): ([($object // {})[]] | any(. == false));
              .projection_assertions as $projection |
              [
                "- local_daemon_complete: \(all_true($projection.local_daemon))",
                "- frontend_auth_projection_complete: \(all_true($projection.frontend_auth_projection))",
                "- frontend_projection_gap_observed: \(any_false($projection.frontend_auth_projection))",
                "- device_online_projection: \(.device_online_projection // "unknown")"
              ] | .[]
            ' "$nested_json"
            echo
            echo "## Load Summary"
            echo
            echo '```json'
            jq '.load' "$nested_json"
            echo '```'
            echo
            echo "## Load Failures"
            echo
            echo '```json'
            jq '(.load // {})
              | to_entries
              | map(select((.value.fail // 0) > 0)
                  | {
                      channel: .key,
                      fail: .value.fail,
                      count: .value.count,
                      p95_ms: .value.p95_ms,
                      max_ms: .value.max_ms
                    })' "$nested_json"
            echo '```'
            echo
            echo "## Audit Mapping"
            echo
            echo "- A21/C08: exact daemon routes and projection paths still need LocalRuntime convergence."
            echo "- C07/A71: receipt/projection trust remains outside this evidence gate."
            echo "- A36: child policy integration remains open."
            echo "- SDK/product-provider separation remains open; this case only covers product projection evidence."
        fi
    } >"$evidence"

    printf '%s\n' "$evidence"
}

mkdir -p "$out_root"
summary="$out_root/summary.tsv"
report="$out_root/report.md"

cat >"$out_root/run.env" <<EOF
repo_root=$repo_root
manifest=$manifest
run_id=$run_id
started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
filter=$filter
include_docker=$include_docker
include_external=$include_external
include_manual=$include_manual
run_all=$run_all
dry_run=$dry_run
EOF

printf 'status\tid\tkind\tpath\ttags\tdescription\texit_code\tduration_ms\tcase_dir\tcommand\tstdout_log\tstderr_log\terror_report\tevidence_report\tdetail\n' >"$summary"

selected=0
passed=0
failed=0
skipped=0
dry_runs=0

{
    read -r _header
    while IFS=$'\t' read -r id kind path tags command description; do
        [[ -z "${id:-}" ]] && continue
        haystack="$id $kind $path $tags $description"
        if ! matches_filter "$haystack"; then
            continue
        fi

        selected=$((selected + 1))
        case_name="$(safe_id "$id")"
        case_dir="$out_root/$case_name"
        stdout_log="$case_dir/stdout.log"
        stderr_log="$case_dir/stderr.log"
        error_report="$case_dir/error.md"
        evidence_report="-"
        mkdir -p "$case_dir"
        printf '%s\n' "$command" >"$case_dir/command.sh"
        chmod +x "$case_dir/command.sh"
        cat >"$case_dir/metadata.env" <<EOF
id=$id
kind=$kind
path=$path
tags=$tags
description=$description
EOF

        if reason="$(should_skip "$tags")"; then
            skipped=$((skipped + 1))
            printf 'SKIP\t%s\t%s\t%s\t%s\t%s\t-\t0\t%s\t%s\t-\t-\t-\t-\t%s\n' \
                "$(tsv_field "$id")" "$(tsv_field "$kind")" "$(tsv_field "$path")" \
                "$(tsv_field "$tags")" "$(tsv_field "$description")" "$case_dir" \
                "$(tsv_field "$command")" "$(tsv_field "$reason")" >>"$summary"
            continue
        fi

        if [[ "$dry_run" == "1" ]]; then
            dry_runs=$((dry_runs + 1))
            printf 'DRY-RUN\t%s\t%s\t%s\t%s\t%s\t-\t0\t%s\t%s\t-\t-\t-\t-\tdry-run\n' \
                "$(tsv_field "$id")" "$(tsv_field "$kind")" "$(tsv_field "$path")" \
                "$(tsv_field "$tags")" "$(tsv_field "$description")" "$case_dir" \
                "$(tsv_field "$command")" >>"$summary"
            continue
        fi

        start_ms="$(now_ms)"
        set +e
        (
            cd "$repo_root"
            bash -lc "$command"
        ) </dev/null >"$stdout_log" 2>"$stderr_log"
        exit_code="$?"
        set -e
        end_ms="$(now_ms)"
        duration_ms="$((end_ms - start_ms))"
        printf '%s\n' "$exit_code" >"$case_dir/exit_code.txt"
        printf '%s\n' "$duration_ms" >"$case_dir/duration_ms.txt"
        evidence_report="$(write_case_evidence_report "$case_dir" "$stdout_log" || printf '-')"

        if [[ "$exit_code" == "0" ]]; then
            passed=$((passed + 1))
            printf 'PASS\t%s\t%s\t%s\t%s\t%s\t0\t%s\t%s\t%s\t%s\t%s\t-\t%s\tok\n' \
                "$(tsv_field "$id")" "$(tsv_field "$kind")" "$(tsv_field "$path")" \
                "$(tsv_field "$tags")" "$(tsv_field "$description")" "$duration_ms" \
                "$case_dir" "$(tsv_field "$command")" "$stdout_log" "$stderr_log" \
                "$evidence_report" >>"$summary"
        else
            failed=$((failed + 1))
            write_error_report "$case_dir" "$id" "$path" "$description" "$exit_code" "$duration_ms" "$stdout_log" "$stderr_log"
            printf 'FAIL\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
                "$(tsv_field "$id")" "$(tsv_field "$kind")" "$(tsv_field "$path")" \
                "$(tsv_field "$tags")" "$(tsv_field "$description")" "$exit_code" \
                "$duration_ms" "$case_dir" "$(tsv_field "$command")" "$stdout_log" \
                "$stderr_log" "$error_report" "$evidence_report" "$error_report" >>"$summary"
            if [[ "$stop_on_fail" == "1" ]]; then
                break
            fi
        fi
    done
} <"$manifest"

{
    echo "# EasyNet Checkboard Report"
    echo
    echo "- run_id: \`$run_id\`"
    echo "- output: \`$out_root\`"
    echo "- manifest: \`$manifest\`"
    echo "- selected: $selected"
    echo "- pass: $passed"
    echo "- fail: $failed"
    echo "- skip: $skipped"
    echo "- dry_run: $dry_runs"
    echo
    echo "## Executed Tests"
    echo
    if [[ "$passed" == "0" && "$failed" == "0" ]]; then
        echo "- none"
    else
        awk -F '\t' 'NR > 1 && ($1 == "PASS" || $1 == "FAIL") {
            printf "- `%s` [%s] `%s` (%sms)\n", $2, $1, $4, $8
            printf "  - description: %s\n", $6
            printf "  - command: `%s`\n", $10
            printf "  - stdout: `%s`\n", $11
            printf "  - stderr: `%s`\n", $12
            if ($14 != "-") {
                printf "  - evidence: `%s`\n", $14
            }
            if ($1 == "FAIL") {
                printf "  - error: `%s`\n", $13
            }
        }' "$summary"
    fi
    echo
    echo "## Architecture Evidence"
    echo
    if ! awk -F '\t' 'NR > 1 && ($1 == "PASS" || $1 == "FAIL") && $14 != "-" { found=1 } END { exit(found ? 0 : 1) }' "$summary"; then
        echo "- none"
    else
        awk -F '\t' 'NR > 1 && ($1 == "PASS" || $1 == "FAIL") && $14 != "-" {
            printf "- `%s`: `%s`\n", $2, $14
        }' "$summary"
    fi
    echo
    echo "## Architecture Evidence Details"
    echo
    evidence_found=0
    {
        read -r _summary_header
        while IFS=$'\t' read -r row_status row_id _row_kind _row_path _row_tags _row_description _row_exit _row_duration _row_case_dir _row_command _row_stdout _row_stderr _row_error row_evidence _row_detail; do
            [[ "$row_status" == "PASS" || "$row_status" == "FAIL" ]] || continue
            [[ "$row_evidence" != "-" && -f "$row_evidence" ]] || continue
            evidence_found=1
            echo "### $row_id"
            echo
            sed 's/^#/###/' "$row_evidence"
            echo
        done
    } <"$summary"
    if [[ "$evidence_found" == "0" ]]; then
        echo "- none"
    fi
    echo
    echo "## Failures"
    echo
    if [[ "$failed" == "0" ]]; then
        echo "- none"
    else
        awk -F '\t' 'NR > 1 && $1 == "FAIL" { printf "- `%s` (`%s`): %s\n", $2, $4, $13 }' "$summary"
    fi
    echo
    echo "## Skips"
    echo
    if [[ "$skipped" == "0" ]]; then
        echo "- none"
    else
        awk -F '\t' 'NR > 1 && $1 == "SKIP" { printf "- `%s` (`%s`): %s\n", $2, $4, $15 }' "$summary"
    fi
    echo
    echo "## Dry Runs"
    echo
    if [[ "$dry_runs" == "0" ]]; then
        echo "- none"
    else
        awk -F '\t' 'NR > 1 && $1 == "DRY-RUN" {
            printf "- `%s` (`%s`): command planned but not executed\n", $2, $4
            printf "  - description: %s\n", $6
            printf "  - command: `%s`\n", $10
        }' "$summary"
    fi
    echo
    echo "## Summary TSV"
    echo
    echo "\`$summary\`"
} >"$report"

echo "$report"

if [[ "$failed" != "0" ]]; then
    exit 1
fi
