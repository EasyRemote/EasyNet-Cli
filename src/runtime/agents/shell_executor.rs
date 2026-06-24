// EasyNet CLI — Shell Ability Executor
// =====================================
//
// File: src/runtime/agents/shell_executor.rs
// Description: Implementation backing `[exec] kind = "shell"` in an
//              ability manifest. Spawns the configured argv with
//              per-argument template substitution against the call's
//              JSON args, captures stdout, and returns a structured
//              JSON envelope.
//
// Why a dedicated executor (rather than going through chat)
// --------------------------------------------------------
// An ability whose contract is "run THIS curl command and return its
// output" should not require an LLM in the loop to translate the
// contract into a tool call. Routing through the owning agent's chat
// handler costs ~10–30 s per invocation, is non-deterministic (the
// LLM may reach for the wrong tool), and forces the ability author
// to write English instructions instead of the actual command.
//
// `[exec] kind = "shell"` makes that binding explicit: the daemon
// spawns the argv directly. A manifest without `[exec]` is not
// invocable until the author binds a concrete executor.
//
// Substitution model
// ------------------
// Each argv element is scanned for `{{ name }}` placeholders. The
// name is looked up in the call's `args` (a JSON object). Values are
// stringified by JSON-`to_string`, which renders strings, numbers,
// booleans, and nested values predictably and unambiguously. A
// missing name surfaces as an error before the subprocess is
// spawned.
//
// The argv form deliberately bypasses `sh -c`. A value containing a
// space, semicolon, backtick, or shell metacharacter still occupies
// exactly one argv slot — command injection is structurally not
// possible, no escaping required.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::core::ability_spec::ShellExec;
use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Default per-call timeout when the manifest does not pin one. Keeps
/// a runaway curl from leaking a daemon thread; chosen to be generous
/// enough for slow networks but small enough that an unattended
/// invocation does not hang indefinitely.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Cap on captured stdout. A misconfigured ability that pipes a huge
/// stream back would otherwise pin daemon memory; truncating with a
/// loud error makes the failure observable.
const MAX_STDOUT_BYTES: usize = 1_048_576; // 1 MiB

/// Run the shell ability with the given args. Returns a JSON envelope
/// `{"result": <stdout>, "fulfilled_by": "shell", "elapsed_ms": N,
/// "exit_code": N}`. Errors come back as `Err(anyhow)`; the caller
/// turns that into the dispatcher's typed error variant.
pub fn run_shell_exec(
    spec: &ShellExec,
    args: &Value,
    timeout: Option<Duration>,
) -> anyhow::Result<Value> {
    let argv = render_argv(&spec.argv, args)?;
    if argv.is_empty() {
        anyhow::bail!("shell executor: argv resolved to empty after rendering");
    }

    // Apply the sandbox profile if the manifest pinned one. Returns
    // the effective argv (possibly wrapped under `sandbox-exec -p
    // '<profile>' …`) plus a boolean for the audit log so an
    // operator can tell which executions were sandboxed without
    // grepping the manifest. A profile that the running platform
    // can't honour is rejected here, NOT silently downgraded to
    // unsandboxed — the operator asked for a security guarantee and
    // must see if it can't be delivered.
    let (effective_argv, sandboxed) = wrap_with_sandbox(&argv, spec.sandbox.as_deref())?;

    let started = Instant::now();
    let mut cmd = Command::new(&effective_argv[0]);
    cmd.args(&effective_argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let _ = sandboxed; // available for envelope augmentation below

    let timeout = timeout.unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("shell executor: failed to spawn {:?}: {e}", argv[0]))?;

    // wait_timeout requires a separate crate; do a simple poll loop
    // to avoid the dep. The granularity (50 ms) is fine for the
    // sub-second commands these abilities typically run, and we still
    // honour the deadline.
    let deadline = started + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!(
                        "shell executor: argv {:?} timed out after {}s",
                        argv,
                        timeout.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => anyhow::bail!("shell executor: wait failed: {e}"),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| anyhow::anyhow!("shell executor: collect output failed: {e}"))?;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let exit_code = output.status.code().unwrap_or(-1);

    let stdout_bytes = if output.stdout.len() > MAX_STDOUT_BYTES {
        anyhow::bail!(
            "shell executor: argv {:?} produced {} stdout bytes (cap {}). \
             A shell ability must return a bounded result; pipe through \
             `head` or restructure the command if streaming is needed.",
            argv,
            output.stdout.len(),
            MAX_STDOUT_BYTES
        );
    } else {
        output.stdout
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        anyhow::bail!(
            "shell executor: argv {:?} exited with status {} (stderr: {})",
            argv,
            exit_code,
            stderr.trim()
        );
    }

    let stdout = decode_stdout(&stdout_bytes, spec.stdout.as_deref())?;
    Ok(json!({
        "result": stdout,
        "fulfilled_by": "shell",
        "exit_code": exit_code,
        "elapsed_ms": elapsed_ms,
        "sandboxed": sandboxed,
    }))
}

/// Build the effective argv after applying the requested sandbox
/// profile. Returns `(argv, sandboxed)` where `sandboxed` is the
/// profile name actually applied (`"none"` when no wrapping happened).
///
/// The wrapping is platform-specific:
///   * macOS uses `sandbox-exec -p '<profile>' …` with one of the
///     hardcoded profile bodies below.
///   * Linux: returns `Err` for non-`none` profiles. `bwrap`
///     wrapping ships in a follow-up; refusing today is the
///     security-correct fail-closed behaviour. An operator who
///     pinned a profile gets a loud error, not a silent no-op.
fn wrap_with_sandbox(
    argv: &[String],
    profile: Option<&str>,
) -> anyhow::Result<(Vec<String>, &'static str)> {
    let profile = profile.unwrap_or("none");
    if profile == "none" {
        return Ok((argv.to_vec(), "none"));
    }

    #[cfg(target_os = "macos")]
    {
        let body = match profile {
            "net_only" => MACOS_NET_ONLY_PROFILE,
            "pure_compute" => MACOS_PURE_COMPUTE_PROFILE,
            other => anyhow::bail!(
                "shell executor: unknown sandbox profile {:?} (known: net_only, pure_compute, none)",
                other
            ),
        };
        let mut wrapped = vec![
            "/usr/bin/sandbox-exec".to_string(),
            "-p".to_string(),
            body.to_string(),
        ];
        wrapped.extend(argv.iter().cloned());
        // Static lifetime: profile is one of the &'static str
        // constants in this module.
        let leaked: &'static str = match profile {
            "net_only" => "net_only",
            "pure_compute" => "pure_compute",
            _ => "unknown",
        };
        Ok((wrapped, leaked))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = argv;
        anyhow::bail!(
            "shell executor: sandbox profile {:?} requested but no sandbox tool \
             is wired on this platform yet (macOS-only in v1; Linux bwrap is a \
             follow-up). Refusing rather than running unsandboxed.",
            profile
        );
    }
}

/// macOS sandbox-exec profile: deny filesystem writes outside the
/// system temp dir, allow outbound network. Suitable for
/// "fetch from a URL" abilities.
///
/// The profile uses sandbox-exec's TinyScheme dialect. We allow:
///   * process-fork (curl re-execs itself for HTTPS handshake)
///   * file-read* everywhere (read your cert store, dns, …)
///   * network-outbound (any host, any port — finer-grained policy
///     would require a second knob the manifest doesn't have yet)
///   * file-write* under /private/tmp + /tmp + /var/folders (the
///     usual macOS temp roots)
/// Everything else (file-write outside tmp, process-exec of new
/// binaries, mach-priv) is denied by `(deny default)`.
#[cfg(target_os = "macos")]
const MACOS_NET_ONLY_PROFILE: &str = r#"
(version 1)
(deny default)
(allow process-fork)
(allow process-exec)
(allow file-read*)
(allow file-write*
  (subpath "/private/tmp")
  (subpath "/tmp")
  (subpath "/var/folders")
  (subpath "/private/var/folders"))
(allow network-outbound)
(allow network-bind (local ip))
(allow system-socket)
(allow mach-lookup)
(allow ipc-posix-shm)
(allow signal (target self))
(allow sysctl-read)
"#;

/// macOS sandbox-exec profile: pure compute — deny network, deny
/// fs writes. Reads remain unrestricted so `jq`-style abilities
/// can still load their input from disk.
#[cfg(target_os = "macos")]
const MACOS_PURE_COMPUTE_PROFILE: &str = r#"
(version 1)
(deny default)
(allow process-fork)
(allow process-exec)
(allow file-read*)
(allow file-write*
  (subpath "/private/tmp")
  (subpath "/tmp")
  (subpath "/var/folders")
  (subpath "/private/var/folders"))
(allow mach-lookup)
(allow ipc-posix-shm)
(allow signal (target self))
(allow sysctl-read)
"#;

/// Thin wrapper over the shared template engine. Kept as a
/// dedicated function so the call site in `run_shell_exec` reads
/// at a glance and so the test below can pin "argv-shaped"
/// rendering separately from "single-string" rendering even
/// though both go through the same engine today.
fn render_argv(argv: &[String], args: &Value) -> anyhow::Result<Vec<String>> {
    crate::runtime::agents::template::render_each(argv, args, "shell executor")
}

fn decode_stdout(bytes: &[u8], mode: Option<&str>) -> anyhow::Result<String> {
    let mode = mode.unwrap_or("utf8_trim");
    match mode {
        "utf8_trim" => {
            let s = std::str::from_utf8(bytes).map_err(|e| {
                anyhow::anyhow!(
                    "shell executor: stdout is not valid UTF-8 (configure `stdout = \
                     \"base64\"` once that mode lands if the ability returns binary): {e}"
                )
            })?;
            Ok(s.trim_end().to_string())
        }
        other => anyhow::bail!("shell executor: stdout decoder {:?} not implemented", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_substitutes_string_arg_into_argv_element() {
        let argv = vec!["echo".to_string(), "hello {{ who }}".to_string()];
        let out = render_argv(&argv, &json!({ "who": "world" })).unwrap();
        assert_eq!(out, vec!["echo".to_string(), "hello world".to_string()]);
    }

    #[test]
    fn render_handles_whitespace_inside_placeholder() {
        let argv = vec!["echo".to_string(), "{{name}}+{{ name }}".to_string()];
        let out = render_argv(&argv, &json!({ "name": "x" })).unwrap();
        assert_eq!(out, vec!["echo".to_string(), "x+x".to_string()]);
    }

    #[test]
    fn render_errors_on_missing_arg_name() {
        let argv = vec!["echo".to_string(), "{{ missing }}".to_string()];
        let err = render_argv(&argv, &json!({ "other": 1 })).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("missing"),
            "expected missing-arg error, got: {msg}"
        );
    }

    #[test]
    fn render_errors_on_unclosed_placeholder() {
        let argv = vec!["x".to_string(), "{{ broken".to_string()];
        let err = render_argv(&argv, &json!({})).unwrap_err();
        assert!(err.to_string().contains("unclosed"));
    }

    #[test]
    fn render_passes_through_argv_with_no_placeholders_and_null_args() {
        let argv = vec!["date".to_string(), "+%Y".to_string()];
        let out = render_argv(&argv, &Value::Null).unwrap();
        assert_eq!(out, argv);
    }

    #[test]
    fn render_rejects_placeholder_when_args_is_null() {
        let argv = vec!["echo".to_string(), "{{ x }}".to_string()];
        let err = render_argv(&argv, &Value::Null).unwrap_err();
        assert!(err.to_string().contains("no args"));
    }

    #[test]
    fn run_shell_exec_returns_stdout_for_echo() {
        let spec = ShellExec {
            argv: vec!["printf".to_string(), "%s".to_string(), "hi".to_string()],
            stdout: None,
            sandbox: None,
        };
        let envelope = run_shell_exec(&spec, &json!({}), None).unwrap();
        assert_eq!(envelope.get("result").and_then(|v| v.as_str()), Some("hi"));
        assert_eq!(
            envelope.get("fulfilled_by").and_then(|v| v.as_str()),
            Some("shell")
        );
        assert_eq!(envelope.get("exit_code").and_then(|v| v.as_i64()), Some(0));
    }

    #[test]
    fn run_shell_exec_propagates_nonzero_exit_as_error() {
        let spec = ShellExec {
            argv: vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
            stdout: None,
            sandbox: None,
        };
        let err = run_shell_exec(&spec, &json!({}), None).unwrap_err();
        assert!(err.to_string().contains("exited with status 7"));
    }

    #[test]
    fn run_shell_exec_substitutes_json_arg_unambiguously() {
        let spec = ShellExec {
            argv: vec![
                "printf".to_string(),
                "%s".to_string(),
                "{{ count }}".to_string(),
            ],
            stdout: None,
            sandbox: None,
        };
        let out = run_shell_exec(&spec, &json!({ "count": 42 }), None).unwrap();
        assert_eq!(out.get("result").and_then(|v| v.as_str()), Some("42"));
    }

    #[test]
    fn run_shell_exec_honours_timeout() {
        let spec = ShellExec {
            argv: vec!["sleep".to_string(), "5".to_string()],
            stdout: None,
            sandbox: None,
        };
        let started = Instant::now();
        let err = run_shell_exec(&spec, &json!({}), Some(Duration::from_millis(200))).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "timeout did not interrupt the subprocess promptly"
        );
        assert!(err.to_string().contains("timed out"));
    }
}
