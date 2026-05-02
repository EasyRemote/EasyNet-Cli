// EasyNet CLI — ShellGuard: dangerous-builtin argv[0] detector
// ============================================================
//
// File: src/support/shellguard/security/builtins.rs
// Description: argv[0]-name table of bash / zsh builtins that
//              escape the argv abstraction or load native code.
//              Mirrors AliveCode's `EVAL_LIKE_BUILTINS` and
//              `ZSH_DANGEROUS_BUILTINS`.
//
// Why argv[0] is the right key
// ----------------------------
// These builtins are not external programs on PATH — they're
// shell internals. tree-sitter-bash parses them as ordinary
// `command` nodes with the builtin name as `argv[0]`, no
// distinguishing syntax. The AST stage cannot reject them
// without a name table; the permission stage's argv[0]-prefix
// rules can be subverted by symlinks or PATH games. So we
// table-drive the rejection here, after the AST stage has
// produced argv but before any permission allowlist match.
//
// Two distinct categories live in this module:
//
//   EVAL_LIKE — argv content is inline code or escapes argv
//               structure. Examples: `eval "rm -rf /"`,
//               `source script`, `trap 'cmd' EXIT`,
//               `coproc rm -rf /`, `noglob cmd`.
//
//   ZSH_DANGEROUS — zsh-specific module loaders / FFI / IPC
//                   builtins. The shell.run dispatcher MAY run
//                   under the operator's default shell which
//                   could be zsh; these names are dangerous
//                   even when the surrounding tree-sitter
//                   parse is bash-clean.
//
// Categories are reported through one return type so the
// caller doesn't need to fan out, but the underlying tables
// stay separate to make adding entries trivial. AliveCode
// found four categories of bypass over 18 months and the
// "single-table" anti-pattern would have made each fix
// touch two files.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use crate::support::shellguard::ast::SimpleCommand;

use super::DetectorHit;

/// Bash / POSIX builtins that take inline code or escape the
/// argv abstraction. argv[0] match is exact (no path strip,
/// no case-fold). All entries are bare builtin names — for the
/// `.` (dot) builtin specifically, the entry is the literal
/// single-char string `"."`.
///
/// The entries here are NOT every bash builtin — most are
/// inert (`echo`, `printf`, `pwd`, …) and require no
/// table-level rejection. This list contains exactly the
/// builtins whose argv shape lies to permission-rule
/// matchers.
const EVAL_LIKE_BUILTINS: &[&str] = &[
    "eval",      // executes argv tail as shell code
    "source",    // reads + executes argv[1] as script
    ".",         // POSIX synonym for `source`
    "exec",      // replaces shell with argv tail (process tree mutation)
    "command",   // bypasses function/alias lookup; permission rule can't see real argv[0]
    "builtin",   // bypasses function lookup; same problem as `command`
    "fc", // history → re-execute previous command — permission rule sees `fc`, not the replayed cmd
    "coproc", // `coproc rm -rf /` — argv[0] is `coproc`, real command is argv[1]
    "noglob", // zsh precommand: `noglob cmd` runs cmd with globbing off
    "nocorrect", // zsh precommand: same shape as noglob
    "trap", // `trap 'cmd' EXIT` — code-string runs at end of every shell.run invocation
    "enable", // `enable -f /path/lib.so name` — dlopen native lib
    "mapfile", // `mapfile -C cmd` — callback runs as code every N input lines
    "readarray", // alias for mapfile
    "hash", // `hash -p /path cmd` — poisons command lookup; subsequent cmd resolves to /path
    "bind", // `bind -x '"key":cmd'` — interactive callback as code
    "complete", // `complete -C cmd` — completion callback as code
    "compgen", // `compgen -C cmd` — IMMEDIATELY runs cmd to generate completions
    "alias", // `alias name='cmd'` — aliases expand under shopt -s expand_aliases
    "let", // `let 'x=a[$(id)]'` — arithmetic eval expands $() in subscript
];

/// Zsh module / FFI / IPC builtins. Loaded via `zmodload` or
/// shipped as part of `zsh/files`, `zsh/net/socket`,
/// `zsh/system`. None of these are external programs; argv[0]
/// match is the only way to detect them.
const ZSH_DANGEROUS_BUILTINS: &[&str] = &[
    "zmodload", // load arbitrary zsh module
    "emulate",  // switch zsh to bash/sh/csh emulation mode
    "sysopen",  // open file descriptor (zsh/system)
    "sysread",  // read from fd (zsh/system)
    "syswrite", // write to fd (zsh/system)
    "sysseek",  // seek fd (zsh/system)
    "zpty",     // open pseudo-TTY (zsh/zpty)
    "ztcp",     // open TCP socket (zsh/net/tcp)
    "zsocket",  // open Unix socket (zsh/net/socket)
    "zf_rm",    // zsh/files functions — same hazard as the GNU coreutils
    "zf_mv", "zf_ln", "zf_chmod", "zf_chown", "zf_mkdir", "zf_rmdir", "zf_chgrp",
];

/// Public entry. Returns `Some(DetectorHit)` if `cmd.argv[0]`
/// matches an entry in either table; `None` for benign commands.
///
/// Empty argv (which the AST stage rejects, but defensive code
/// is cheap) returns `None` — empty argv has no argv[0] to
/// match, downstream stages handle the empty case.
pub fn check(cmd: &SimpleCommand) -> Option<DetectorHit> {
    let head = cmd.argv.first()?;
    if EVAL_LIKE_BUILTINS.iter().any(|&n| n == head) {
        return Some(DetectorHit {
            name: head.clone(),
            detail: format!(
                "eval-like builtin (argv tail is inline code: {})",
                truncate_argv_summary(&cmd.argv)
            ),
        });
    }
    if ZSH_DANGEROUS_BUILTINS.iter().any(|&n| n == head) {
        return Some(DetectorHit {
            name: head.clone(),
            detail: "zsh-module builtin (argv[0] is a shell internal)".to_string(),
        });
    }
    None
}

/// Short summary of argv tail for the receipt's `detail` field.
/// Caps at 64 chars total to keep receipts compact; longer
/// argv get a `…` ellipsis. The summary is for operator
/// debugging only — permission rules and audit fingerprints
/// use the full argv hash from the runner, not this.
fn truncate_argv_summary(argv: &[String]) -> String {
    let mut joined = String::new();
    for (i, arg) in argv.iter().enumerate().skip(1) {
        if i > 1 {
            joined.push(' ');
        }
        joined.push_str(arg);
        if joined.len() > 60 {
            joined.truncate(60);
            joined.push('…');
            break;
        }
    }
    joined
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

    #[test]
    fn benign_command_returns_none() {
        assert!(check(&cmd(&["ls", "-la"])).is_none());
        assert!(check(&cmd(&["echo", "hi"])).is_none());
        assert!(check(&cmd(&["git", "status"])).is_none());
    }

    #[test]
    fn empty_argv_returns_none() {
        assert!(check(&cmd(&[])).is_none());
    }

    // ---- EVAL_LIKE_BUILTINS ----

    #[test]
    fn eval_is_flagged() {
        let h = check(&cmd(&["eval", "rm -rf /"])).unwrap();
        assert_eq!(h.name, "eval");
        assert!(h.detail.starts_with("eval-like builtin"));
    }

    #[test]
    fn source_is_flagged() {
        assert_eq!(
            check(&cmd(&["source", "script.sh"])).unwrap().name,
            "source"
        );
    }

    #[test]
    fn dot_builtin_is_flagged() {
        assert_eq!(check(&cmd(&[".", "script.sh"])).unwrap().name, ".");
    }

    #[test]
    fn exec_is_flagged() {
        assert_eq!(
            check(&cmd(&["exec", "rm", "-rf", "/"])).unwrap().name,
            "exec"
        );
    }

    #[test]
    fn coproc_is_flagged() {
        // `coproc rm -rf /` — argv[0]='coproc' hides the real command.
        assert_eq!(
            check(&cmd(&["coproc", "rm", "-rf", "/"])).unwrap().name,
            "coproc"
        );
    }

    #[test]
    fn trap_is_flagged() {
        assert_eq!(
            check(&cmd(&["trap", "rm /tmp/lock", "EXIT"])).unwrap().name,
            "trap"
        );
    }

    #[test]
    fn enable_is_flagged() {
        assert_eq!(
            check(&cmd(&["enable", "-f", "/tmp/x.so", "name"]))
                .unwrap()
                .name,
            "enable"
        );
    }

    #[test]
    fn hash_is_flagged() {
        // Poisons command lookup.
        assert_eq!(
            check(&cmd(&["hash", "-p", "/evil/rm", "rm"])).unwrap().name,
            "hash"
        );
    }

    #[test]
    fn alias_is_flagged() {
        assert_eq!(check(&cmd(&["alias", "ls=rm"])).unwrap().name, "alias");
    }

    #[test]
    fn let_is_flagged() {
        // `let 'x=a[$(id)]'` — arithmetic eval expands $() in subscript.
        assert_eq!(check(&cmd(&["let", "x=a[$(id)]"])).unwrap().name, "let");
    }

    #[test]
    fn mapfile_is_flagged() {
        assert_eq!(
            check(&cmd(&["mapfile", "-C", "rm /tmp", "-c", "1", "arr"]))
                .unwrap()
                .name,
            "mapfile"
        );
    }

    #[test]
    fn readarray_is_flagged() {
        assert_eq!(
            check(&cmd(&["readarray", "-C", "rm /tmp"])).unwrap().name,
            "readarray"
        );
    }

    #[test]
    fn noglob_precommand_is_flagged() {
        // Zsh precommand modifier — argv[0]='noglob' hides the real command.
        assert_eq!(
            check(&cmd(&["noglob", "rm", "-rf", "/"])).unwrap().name,
            "noglob"
        );
    }

    #[test]
    fn compgen_is_flagged() {
        // `compgen -C cmd` immediately runs cmd, even non-interactively.
        assert_eq!(
            check(&cmd(&["compgen", "-C", "id"])).unwrap().name,
            "compgen"
        );
    }

    #[test]
    fn fc_is_flagged() {
        assert_eq!(check(&cmd(&["fc", "-s"])).unwrap().name, "fc");
    }

    // ---- ZSH_DANGEROUS_BUILTINS ----

    #[test]
    fn zmodload_is_flagged() {
        let h = check(&cmd(&["zmodload", "zsh/system"])).unwrap();
        assert_eq!(h.name, "zmodload");
        assert!(h.detail.contains("zsh-module"));
    }

    #[test]
    fn emulate_is_flagged() {
        assert_eq!(check(&cmd(&["emulate", "sh"])).unwrap().name, "emulate");
    }

    #[test]
    fn zpty_is_flagged() {
        assert_eq!(check(&cmd(&["zpty"])).unwrap().name, "zpty");
    }

    #[test]
    fn ztcp_is_flagged() {
        assert_eq!(check(&cmd(&["ztcp", "-l", "8080"])).unwrap().name, "ztcp");
    }

    #[test]
    fn zf_rm_is_flagged() {
        // zsh/files version of rm.
        assert_eq!(check(&cmd(&["zf_rm", "-rf", "/"])).unwrap().name, "zf_rm");
    }

    // ---- discrimination tests ----

    #[test]
    fn similar_but_safe_names_not_flagged() {
        // `evaluate`, `sourcing`, `traps`, `coprocess` etc must NOT match
        // — exact-name only.
        assert!(check(&cmd(&["evaluate"])).is_none());
        assert!(check(&cmd(&["sourcing"])).is_none());
        assert!(check(&cmd(&["traps"])).is_none());
        assert!(check(&cmd(&["coprocess"])).is_none());
    }

    #[test]
    fn detail_summary_truncates_long_argv() {
        let long = "x".repeat(200);
        let h = check(&cmd(&["eval", &long])).unwrap();
        // Cap is the 60-char argv summary plus the descriptive
        // wrapper (`eval-like builtin (argv tail is inline code: …)`).
        // Total stays below 120 — well short of receipts' soft
        // budget of one screen line. The exact value isn't a
        // contract, just an upper bound.
        assert!(h.detail.len() < 120, "detail too long: {}", h.detail.len());
        assert!(h.detail.contains('…'));
    }
}
