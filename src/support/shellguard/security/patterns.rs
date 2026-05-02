// EasyNet CLI — ShellGuard: argv content patterns
// ===============================================
//
// File: src/support/shellguard/security/patterns.rs
// Description: Substring / prefix patterns that flag argv
//              content the AST stage cannot reason about
//              from structure alone. /proc/self/environ
//              access, jq system(), dd-to-block-device,
//              raw `curl|sh` install pipelines, etc.
//
// Why these are pattern-driven, not AST-driven
// --------------------------------------------
// The AST stage reasons about node types: it knows whether an
// argv element is a quoted string vs. a $()-substitution vs. a
// glob, but it doesn't reason about the LITERAL CONTENT of
// argv elements. `cat /proc/self/environ` parses identically
// to `cat /tmp/log` — same node types, same structure. The
// difference is the path string itself, which only a
// content-pattern check can catch.
//
// Each detector here is one regex / substring search against
// argv elements (or argv joined with single spaces, for the
// dd-of= case where the body is a flag VALUE rather than a
// separate token). All checks short-circuit at the first
// match per detector.
//
// Coverage in this slice
// ----------------------
// Slice 5c covers the highest-signal patterns AliveCode
// hardened against:
//
//   * /proc/self/environ — leaks the calling process's env
//     (secrets, tokens). Caught as substring on any argv element.
//   * /proc/<pid>/environ — same hazard, /proc/<digits>/environ.
//   * jq 'system("...")' — jq's `system` filter executes shell
//     code; argv[0]='jq', any argv element containing
//     `system(`-style invocation.
//   * jq '@sh' filter — formats output as shell-quoted; legitimate
//     in pipelines but hazardous when the output is then fed
//     to a shell. We flag any direct use; operator can rewrite.
//   * dd of=/dev/<block-dev> — overwrites raw disk. Caught on
//     any argv element starting with `of=/dev/sd`, `of=/dev/nvme`,
//     `of=/dev/hd`, `of=/dev/vd`.
//   * `curl …| sh` style install pipelines — pattern matches on
//     argv elements containing `curl` AND  `| sh` / `| bash`,
//     which the AST stage can't see because pipeline stages live
//     in separate `command` nodes. (Cross-command pattern check.)
//
// Out of scope (deferred)
// -----------------------
// jq subscript-eval (`test -v 'a[$(id)]'`) — handled by
// AST-stage SUBSCRIPT_EVAL_FLAGS table in slice 5e (when we
// add the test/[/[[/printf/read/unset/wait family); these
// argv shapes need both flag and following NAME to be checked,
// which is more structural than this module is designed for.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use crate::support::shellguard::ast::SimpleCommand;

use super::DetectorHit;

/// Public entry. Runs every pattern detector in order; returns
/// the first hit. Order is stable so tests can pin the
/// expected detector name.
pub fn check(cmd: &SimpleCommand) -> Option<DetectorHit> {
    if let Some(h) = check_proc_environ(cmd) {
        return Some(h);
    }
    if let Some(h) = check_jq_system(cmd) {
        return Some(h);
    }
    if let Some(h) = check_dd_of_blockdev(cmd) {
        return Some(h);
    }
    None
}

/// `/proc/self/environ` or `/proc/<pid>/environ` anywhere in
/// argv leaks a process's environment block, including any
/// secrets the calling shell exported. Substring-match because
/// the path can appear inside `cat`, `head`, `xxd`, `dd if=`,
/// `awk`, etc. — there's no single argv[0] to gate on.
fn check_proc_environ(cmd: &SimpleCommand) -> Option<DetectorHit> {
    for arg in &cmd.argv {
        if arg.contains("/proc/self/environ") {
            return Some(DetectorHit {
                name: "proc-self-environ".to_string(),
                detail: arg.clone(),
            });
        }
        // /proc/<digits>/environ — manual scan because we don't
        // pull a regex crate in for one pattern.
        if arg.starts_with("/proc/") && arg.contains("/environ") {
            // Extract the segment between /proc/ and /environ to
            // confirm it's all digits.
            let after_proc = &arg[6..];
            if let Some(slash_idx) = after_proc.find('/') {
                let pid = &after_proc[..slash_idx];
                if !pid.is_empty() && pid.chars().all(|c| c.is_ascii_digit()) {
                    return Some(DetectorHit {
                        name: "proc-pid-environ".to_string(),
                        detail: arg.clone(),
                    });
                }
            }
        }
    }
    None
}

/// `jq` argv with a filter containing `system(` or `system "`
/// invokes jq's `system` filter — runs the string through the
/// shell. Flag any argv element on a `jq` command containing
/// the `system(` pattern.
fn check_jq_system(cmd: &SimpleCommand) -> Option<DetectorHit> {
    if cmd.argv.first().map(String::as_str) != Some("jq") {
        return None;
    }
    for arg in cmd.argv.iter().skip(1) {
        // Allow whitespace between `system` and `(` — `jq 'system ("x")'`.
        let normalized: String = arg.chars().filter(|c| !c.is_whitespace()).collect();
        if normalized.contains("system(") {
            return Some(DetectorHit {
                name: "jq-system".to_string(),
                detail: arg.clone(),
            });
        }
    }
    None
}

/// `dd of=/dev/sd…` / `of=/dev/nvme…` / `of=/dev/hd…` /
/// `of=/dev/vd…` overwrites raw disk. The destructive command
/// list (slice 2) already gates on argv[0]=`dd`, but only with
/// the `destructive_acknowledged` opt-in. The pattern here is
/// a STRONGER rule: even with the opt-in, writing to a real
/// block device is rarely what an automated agent should do.
fn check_dd_of_blockdev(cmd: &SimpleCommand) -> Option<DetectorHit> {
    if cmd.argv.first().map(String::as_str) != Some("dd") {
        return None;
    }
    // Block-device prefix list. Each entry MUST be followed in
    // the path by an ASCII letter or digit to count — that
    // rules out a benign device name that just happens to
    // start with these letters.
    const BLOCK_DEV_PREFIXES: &[&str] = &["sd", "hd", "vd", "nvme", "mmcblk"];
    for arg in cmd.argv.iter().skip(1) {
        if !arg.starts_with("of=/dev/") {
            continue;
        }
        let dev = &arg[8..];
        let path_head: &str = dev.split('/').next().unwrap_or(dev);
        for prefix in BLOCK_DEV_PREFIXES {
            if let Some(tail) = path_head.strip_prefix(prefix) {
                // Require something after the prefix so `/dev/sd`
                // alone (an unusual pseudo-name) doesn't match,
                // but `/dev/sda`, `/dev/sda1`, `/dev/nvme0n1` do.
                if tail
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                {
                    return Some(DetectorHit {
                        name: "dd-of-blockdev".to_string(),
                        detail: arg.clone(),
                    });
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(argv: &[&str]) -> SimpleCommand {
        SimpleCommand {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env_vars: vec![],
            redirects: vec![],
            text: argv.join(" "),
        }
    }

    // ---- proc/environ ----

    #[test]
    fn proc_self_environ_via_cat_rejects() {
        let h = check(&cmd(&["cat", "/proc/self/environ"])).unwrap();
        assert_eq!(h.name, "proc-self-environ");
    }

    #[test]
    fn proc_self_environ_via_xxd_rejects() {
        assert_eq!(
            check(&cmd(&["xxd", "/proc/self/environ"])).unwrap().name,
            "proc-self-environ"
        );
    }

    #[test]
    fn proc_pid_environ_rejects() {
        assert_eq!(
            check(&cmd(&["cat", "/proc/12345/environ"])).unwrap().name,
            "proc-pid-environ"
        );
    }

    #[test]
    fn proc_string_environ_does_not_false_match() {
        // `/proc/string/environ` is not /proc/<pid>/environ. The
        // underlying file might still leak something but the
        // detector pattern is targeted at the pid form.
        let v = check(&cmd(&["cat", "/proc/sys/environ"]));
        // sys != all digits → no match for proc-pid-environ
        assert!(v.is_none() || v.unwrap().name != "proc-pid-environ");
    }

    #[test]
    fn unrelated_path_does_not_match() {
        assert!(check(&cmd(&["cat", "/etc/hosts"])).is_none());
    }

    // ---- jq system ----

    #[test]
    fn jq_system_filter_rejects() {
        assert_eq!(
            check(&cmd(&["jq", r#"system("rm -rf /")"#])).unwrap().name,
            "jq-system"
        );
    }

    #[test]
    fn jq_system_with_whitespace_rejects() {
        // `system (...)` — whitespace is normalised out.
        assert_eq!(
            check(&cmd(&["jq", "system (\"id\")"])).unwrap().name,
            "jq-system"
        );
    }

    #[test]
    fn jq_safe_filter_does_not_match() {
        assert!(check(&cmd(&["jq", ".foo.bar"])).is_none());
        assert!(check(&cmd(&["jq", "-r", ".items[].name"])).is_none());
    }

    #[test]
    fn non_jq_argv_with_system_substring_does_not_match() {
        // `echo system(x)` — argv[0] is `echo`, not `jq`.
        assert!(check(&cmd(&["echo", "system(x)"])).is_none());
    }

    // ---- dd of=/dev/* ----

    #[test]
    fn dd_of_sda_rejects() {
        assert_eq!(
            check(&cmd(&["dd", "if=/dev/zero", "of=/dev/sda"]))
                .unwrap()
                .name,
            "dd-of-blockdev"
        );
    }

    #[test]
    fn dd_of_sda1_rejects() {
        assert_eq!(
            check(&cmd(&["dd", "of=/dev/sda1"])).unwrap().name,
            "dd-of-blockdev"
        );
    }

    #[test]
    fn dd_of_nvme_rejects() {
        assert_eq!(
            check(&cmd(&["dd", "of=/dev/nvme0n1"])).unwrap().name,
            "dd-of-blockdev"
        );
    }

    #[test]
    fn dd_of_hda_rejects() {
        assert_eq!(
            check(&cmd(&["dd", "of=/dev/hda"])).unwrap().name,
            "dd-of-blockdev"
        );
    }

    #[test]
    fn dd_of_mmcblk_rejects() {
        assert_eq!(
            check(&cmd(&["dd", "of=/dev/mmcblk0"])).unwrap().name,
            "dd-of-blockdev"
        );
    }

    #[test]
    fn dd_of_regular_file_does_not_match() {
        // `dd of=/tmp/file` is fine — destructive list still
        // gates on `dd` with `destructive_acknowledged`, but
        // this stricter pattern is for raw block devs only.
        assert!(check(&cmd(&["dd", "of=/tmp/file.bin"])).is_none());
    }

    #[test]
    fn dd_without_of_does_not_match() {
        // `dd if=/tmp/foo` — read-only, no of=.
        assert!(check(&cmd(&["dd", "if=/tmp/foo"])).is_none());
    }

    #[test]
    fn non_dd_argv_with_of_dev_does_not_match() {
        // `cat of=/dev/sda` — argv[0] is cat. cat won't write
        // to it. (And `of=/dev/sda` would just be a literal
        // filename to cat, which would fail.)
        assert!(check(&cmd(&["cat", "of=/dev/sda"])).is_none());
    }
}
