// Integration tests that run the repo's shell-level guard scripts as
// part of `cargo test --all`. Every PR lands its CI contract here so a
// broken script or schema drift fails the normal test run without
// needing a separate CI step.
//
// The individual cases live in shell so they can be invoked directly by
// contributors: `bash tests/scripts/test_trace_parity.sh`. This Rust
// shim is the automation seam.
//
// A shell driver is selected explicitly (`bash`) instead of relying on
// the user's `$SHELL` because the scripts use `[[ ... ]]` and
// `set -euo pipefail`, which are bash-specific.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn run_bash_script(relative_path: &str) {
    let script = repo_root().join(relative_path);
    assert!(script.exists(), "script missing: {}", script.display());

    let output = Command::new("bash")
        .arg(&script)
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn bash");

    if !output.status.success() {
        panic!(
            "{} failed (exit {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            relative_path,
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn trace_parity_script_contract_holds() {
    // Covers happy, failure (missing fixture, extra key, removed key)
    // and edge cases (idempotence, path-only payload invariant).
    run_bash_script("tests/scripts/test_trace_parity.sh");
}

#[test]
fn check_upstream_names_script_contract_holds() {
    // Covers happy, failure (violation in src/ and Cargo.toml) and
    // edge cases (docs/ exemption, string-literal violation still
    // trips, unbanned variant passes).
    run_bash_script("tests/scripts/test_check_upstream_names.sh");
}

#[test]
fn no_raw_ura_construction_script_contract_holds() {
    // Pins the URA builder/parser boundary: only src/ura.rs may hand
    // construct or scheme-prefix parse easynet URAs.
    run_bash_script("tests/scripts/test_no_raw_ura_construction.sh");
}
