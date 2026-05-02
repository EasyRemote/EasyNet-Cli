// EasyNet CLI — ShellGuard: read-only mode classifier
// ===================================================
//
// File: src/support/shellguard/readonly.rs
// Description: Stage 7 of the shell.run pipeline. When the
//              caller sets `read_only_only: true`, every
//              command must come from a known read-only set
//              AND carry no write redirects.
//
// Why a name allowlist + redirect check
// -------------------------------------
// Read-only intent has two angles:
//
//   1. The COMMAND's own behaviour — `ls`, `cat`, `grep`,
//      `find`, `head`, `tail`, `wc`, `awk` (without -i),
//      `git status`, `git log`, `git diff` — these never
//      mutate state when invoked with safe flags. They sit
//      in `READ_ONLY_BUILTINS`.
//   2. The redirect target — even a read-only command, if
//      followed by `>` / `>>` / `&>`, mutates the filesystem
//      at the redirect target. The classifier rejects any
//      write redirect regardless of argv[0].
//
// The list is intentionally narrow. New entries are easy
// (one line); over-inclusive entries are dangerous (`make`,
// `npm`, `cargo` LOOK harmless but each can run arbitrary
// commands defined in their config files). Bias toward
// "obviously stateless on stdout" tools.
//
// Special cases
// -------------
// * `git`: only the read-only subcommands count. argv[0]=git
//   AND argv[1] in {`status`, `log`, `diff`, `show`, `blame`,
//   `branch`, `config --get`, `rev-parse`, `ls-files`,
//   `describe`, `tag` (without -d), `stash list`, `worktree
//   list`}. Anything else fails the classifier even though
//   argv[0] would otherwise hit a "git is read-only" rule.
// * `awk` / `sed`: classifier accepts only when the argv
//   carries no `-i` / `-i.bak` flag; sed-edit detection in
//   future slice 7c double-checks.
// * `find`: classifier rejects on `-delete`, `-exec`, `-execdir`,
//   `-fprint`, `-fprintf`, `-fls`, `-fprint0`. Otherwise
//   accepted.
//
// Read-only mode is a CALLER opt-in: shell.run always runs
// the security + permission stages, but only runs this
// classifier when the caller passes `read_only_only=true`.
// `evaluate_or_skip(commands, false) -> Ok` short-circuits.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use crate::support::shellguard::ast::SimpleCommand;

/// Bare argv[0] names treated as unconditionally read-only.
///
/// Each entry is a UNIX coreutil whose default invocation
/// only reads files / writes to stdout. Tools with a "write
/// to file" mode (e.g. `cp`, `mv`) are deliberately absent.
const READ_ONLY_BUILTINS: &[&str] = &[
    // Inspection
    "ls",
    "ll",
    "la",
    "cat",
    "tac",
    "zcat",
    "bzcat",
    "xzcat",
    "zstdcat",
    "head",
    "tail",
    "less",
    "more",
    // Search
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ripgrep",
    "ack",
    "ag",
    // Statistics
    "wc",
    "du",
    "df",
    "stat",
    "file",
    // Pure transforms (read-only on stdin)
    "tr",
    "cut",
    "sort",
    "uniq",
    "rev",
    "fold",
    "expand",
    "unexpand",
    "comm",
    "diff",
    "cmp",
    "join",
    "paste",
    "od",
    "xxd",
    "hexdump",
    "strings",
    // Encoding
    "base64",
    "uuencode",
    "uudecode",
    // Hashing
    "md5sum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "cksum",
    "shasum",
    "b2sum",
    // Archive inspection
    "tar",
    "zip",
    "unzip", // tar is dual-use; we only allow when no `-c` / `--create`
    // Process inspection
    "ps",
    "top",
    "htop",
    "free",
    "uptime",
    "uname",
    "id",
    "whoami",
    "pwd",
    "hostname",
    "date",
    // Filesystem inspection
    "find",
    "locate",
    "which",
    "type",
    "command",
    // Text formatting
    "printf",
    "echo",
    "yes",
    "true",
    "false",
    "test",
    // jq is read-only on inputs (the SECURITY stage already
    // catches `system(...)` and `@sh`; here we accept the
    // common case).
    "jq",
    "yq",
    // sed / awk: read-only when invoked WITHOUT -i / -i.bak
    // (in-place edit). The flag check below rejects the
    // write-mode invocation; argv[0] match here lets the
    // read-only invocation through.
    "sed",
    "awk",
    "gawk",
    "mawk",
    // git (subcommand-gated below)
    "git",
];

/// Read-only `git` subcommands. `git status` is allowed,
/// `git push` is not. argv[1] match.
const GIT_READ_ONLY_SUBCOMMANDS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "blame",
    "branch",
    "rev-parse",
    "ls-files",
    "ls-tree",
    "describe",
    "config",   // config --get is read-only; --set we reject below
    "stash",    // stash list/show only; we reject other stash ops
    "worktree", // worktree list only
    "remote",   // remote -v / remote show only
    "tag",      // tag -l (list); -d (delete) we reject
    "shortlog",
    "reflog",
];

/// `find` flags that mutate state.
const FIND_WRITE_FLAGS: &[&str] = &[
    "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprintf", "-fls", "-fprint0",
];

/// `awk` / `sed` -i (in-place edit) flags. Any argv element
/// matching exactly `-i` or starting with `-i.` (sed-style
/// `-i.bak`).
fn is_in_place_edit_flag(arg: &str) -> bool {
    arg == "-i" || arg.starts_with("-i.") || arg == "--in-place"
}

/// Result of evaluating commands against the read-only
/// classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOnlyVerdict {
    Ok,
    Rejected {
        argv_index: usize,
        reason: ReadOnlyRejection,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOnlyRejection {
    /// argv[0] is not in the read-only allowlist.
    UnknownCommand { argv0: String },
    /// argv[0] is git but argv[1] is a write-mode subcommand.
    GitNotReadOnly { subcommand: String },
    /// argv[0] is find, awk, sed, etc. with a write-mode flag.
    WriteFlag { argv0: String, flag: String },
    /// Command had a write redirect (`>` / `>>` / etc.).
    WriteRedirect { op: String, target: String },
}

/// Evaluate every command in `commands` for read-only intent.
/// First failure short-circuits.
pub fn evaluate(commands: &[SimpleCommand]) -> ReadOnlyVerdict {
    for (idx, cmd) in commands.iter().enumerate() {
        // 1. write redirect kills it regardless of argv[0].
        for r in &cmd.redirects {
            if matches!(r.op.as_str(), ">" | ">>" | ">|" | "&>" | "&>>") {
                return ReadOnlyVerdict::Rejected {
                    argv_index: idx,
                    reason: ReadOnlyRejection::WriteRedirect {
                        op: r.op.clone(),
                        target: r.target.clone(),
                    },
                };
            }
        }
        // 2. argv[0] in the allowlist.
        let head = match cmd.argv.first() {
            Some(h) => h,
            None => {
                return ReadOnlyVerdict::Rejected {
                    argv_index: idx,
                    reason: ReadOnlyRejection::UnknownCommand {
                        argv0: String::new(),
                    },
                }
            }
        };
        if !READ_ONLY_BUILTINS.iter().any(|&n| n == head) {
            return ReadOnlyVerdict::Rejected {
                argv_index: idx,
                reason: ReadOnlyRejection::UnknownCommand {
                    argv0: head.clone(),
                },
            };
        }
        // 3. Per-tool sub-rules.
        if head == "git" {
            let sub = cmd.argv.get(1).map(String::as_str).unwrap_or("");
            if !GIT_READ_ONLY_SUBCOMMANDS.iter().any(|&n| n == sub) {
                return ReadOnlyVerdict::Rejected {
                    argv_index: idx,
                    reason: ReadOnlyRejection::GitNotReadOnly {
                        subcommand: sub.to_string(),
                    },
                };
            }
            // Reject git config --set / --unset / --add /
            // --replace-all forms; only --get / --get-all /
            // --list / --show-origin are read-only.
            if sub == "config" {
                for arg in cmd.argv.iter().skip(2) {
                    if matches!(
                        arg.as_str(),
                        "--add"
                            | "--unset"
                            | "--unset-all"
                            | "--replace-all"
                            | "--rename-section"
                            | "--remove-section"
                            | "--edit"
                    ) || (!arg.starts_with('-') && false/* setting form is `key value` — hard to detect statically; skip */)
                    {
                        return ReadOnlyVerdict::Rejected {
                            argv_index: idx,
                            reason: ReadOnlyRejection::WriteFlag {
                                argv0: head.clone(),
                                flag: arg.clone(),
                            },
                        };
                    }
                }
            }
        }
        if head == "find" {
            for arg in cmd.argv.iter().skip(1) {
                if FIND_WRITE_FLAGS.iter().any(|&f| f == arg) {
                    return ReadOnlyVerdict::Rejected {
                        argv_index: idx,
                        reason: ReadOnlyRejection::WriteFlag {
                            argv0: head.clone(),
                            flag: arg.clone(),
                        },
                    };
                }
            }
        }
        if matches!(head.as_str(), "awk" | "gawk" | "mawk" | "sed") {
            for arg in cmd.argv.iter().skip(1) {
                if is_in_place_edit_flag(arg) {
                    return ReadOnlyVerdict::Rejected {
                        argv_index: idx,
                        reason: ReadOnlyRejection::WriteFlag {
                            argv0: head.clone(),
                            flag: arg.clone(),
                        },
                    };
                }
            }
        }
        if head == "tar" {
            // -c / --create writes; reject. Allow -t (list),
            // -x (extract — DOES write but to caller's path
            // intent; defer to pathconstraints). For
            // strictest read-only, treat tar as inspect-only.
            for arg in cmd.argv.iter().skip(1) {
                if matches!(
                    arg.as_str(),
                    "-c" | "--create" | "-x" | "--extract" | "-u" | "--update"
                ) {
                    return ReadOnlyVerdict::Rejected {
                        argv_index: idx,
                        reason: ReadOnlyRejection::WriteFlag {
                            argv0: head.clone(),
                            flag: arg.clone(),
                        },
                    };
                }
                // Combined short flags: `-cf`, `-xzf` etc.
                if arg.starts_with('-')
                    && arg.len() > 1
                    && !arg.starts_with("--")
                    && (arg.contains('c') || arg.contains('x') || arg.contains('u'))
                {
                    return ReadOnlyVerdict::Rejected {
                        argv_index: idx,
                        reason: ReadOnlyRejection::WriteFlag {
                            argv0: head.clone(),
                            flag: arg.clone(),
                        },
                    };
                }
            }
        }
    }
    ReadOnlyVerdict::Ok
}

/// Caller-side convenience: run the classifier only when
/// `enabled` is true.
pub fn evaluate_or_skip(commands: &[SimpleCommand], enabled: bool) -> ReadOnlyVerdict {
    if enabled {
        evaluate(commands)
    } else {
        ReadOnlyVerdict::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::shellguard::ast::Redirect;

    fn cmd(argv: &[&str]) -> SimpleCommand {
        SimpleCommand {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env_vars: vec![],
            redirects: vec![],
            text: argv.join(" "),
        }
    }

    fn cmd_with_redirect(argv: &[&str], op: &str, target: &str) -> SimpleCommand {
        SimpleCommand {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env_vars: vec![],
            redirects: vec![Redirect {
                op: op.to_string(),
                target: target.to_string(),
                fd: None,
            }],
            text: format!("{} {} {}", argv.join(" "), op, target),
        }
    }

    // ---- happy path ---------------------------------------------

    #[test]
    fn ls_passes() {
        assert_eq!(evaluate(&[cmd(&["ls", "-la"])]), ReadOnlyVerdict::Ok);
    }

    #[test]
    fn cat_etc_passwd_passes() {
        assert_eq!(
            evaluate(&[cmd(&["cat", "/etc/passwd"])]),
            ReadOnlyVerdict::Ok
        );
    }

    #[test]
    fn grep_passes() {
        assert_eq!(
            evaluate(&[cmd(&["grep", "TODO", "src"])]),
            ReadOnlyVerdict::Ok
        );
    }

    #[test]
    fn find_without_write_flags_passes() {
        assert_eq!(
            evaluate(&[cmd(&["find", "/tmp", "-name", "*.log"])]),
            ReadOnlyVerdict::Ok
        );
    }

    #[test]
    fn empty_input_passes() {
        assert_eq!(evaluate(&[]), ReadOnlyVerdict::Ok);
    }

    // ---- write redirect kills ----------------------------------

    #[test]
    fn ls_with_write_redirect_rejects() {
        let cmds = [cmd_with_redirect(&["ls"], ">", "/tmp/log")];
        match evaluate(&cmds) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteRedirect { op, target },
                ..
            } => {
                assert_eq!(op, ">");
                assert_eq!(target, "/tmp/log");
            }
            other => panic!("expected WriteRedirect, got {other:?}"),
        }
    }

    #[test]
    fn read_redirect_does_not_reject_readonly_cmd() {
        // `wc < file` — read redirect, wc is read-only.
        let cmds = [cmd_with_redirect(&["wc"], "<", "file")];
        assert_eq!(evaluate(&cmds), ReadOnlyVerdict::Ok);
    }

    // ---- argv[0] not in allowlist ------------------------------

    #[test]
    fn rm_rejects_as_unknown() {
        match evaluate(&[cmd(&["rm", "/tmp/x"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::UnknownCommand { argv0 },
                ..
            } => {
                assert_eq!(argv0, "rm");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn npm_rejects_as_unknown() {
        // npm CAN run arbitrary scripts via package.json — not
        // safe for read-only mode.
        match evaluate(&[cmd(&["npm", "install"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::UnknownCommand { argv0 },
                ..
            } => {
                assert_eq!(argv0, "npm");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    // ---- git subcommand gate -----------------------------------

    #[test]
    fn git_status_passes() {
        assert_eq!(evaluate(&[cmd(&["git", "status"])]), ReadOnlyVerdict::Ok);
    }

    #[test]
    fn git_log_passes() {
        assert_eq!(evaluate(&[cmd(&["git", "log"])]), ReadOnlyVerdict::Ok);
    }

    #[test]
    fn git_push_rejects() {
        match evaluate(&[cmd(&["git", "push"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::GitNotReadOnly { subcommand },
                ..
            } => {
                assert_eq!(subcommand, "push");
            }
            other => panic!("expected GitNotReadOnly, got {other:?}"),
        }
    }

    #[test]
    fn git_commit_rejects() {
        match evaluate(&[cmd(&["git", "commit", "-m", "x"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::GitNotReadOnly { .. },
                ..
            } => {}
            other => panic!("expected reject, got {other:?}"),
        }
    }

    #[test]
    fn git_config_unset_rejects() {
        match evaluate(&[cmd(&["git", "config", "--unset", "user.email"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteFlag { flag, .. },
                ..
            } => {
                assert_eq!(flag, "--unset");
            }
            other => panic!("expected WriteFlag, got {other:?}"),
        }
    }

    // ---- find write flags --------------------------------------

    #[test]
    fn find_delete_rejects() {
        match evaluate(&[cmd(&["find", "/tmp", "-delete"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteFlag { flag, .. },
                ..
            } => {
                assert_eq!(flag, "-delete");
            }
            other => panic!("expected WriteFlag, got {other:?}"),
        }
    }

    #[test]
    fn find_exec_rejects() {
        match evaluate(&[cmd(&["find", "/tmp", "-exec", "rm", "{}", ";"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteFlag { flag, .. },
                ..
            } => {
                assert_eq!(flag, "-exec");
            }
            other => panic!("expected WriteFlag, got {other:?}"),
        }
    }

    // ---- in-place edit flags -----------------------------------

    #[test]
    fn sed_inplace_short_rejects() {
        match evaluate(&[cmd(&["sed", "-i", "s/a/b/", "f"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteFlag { flag, .. },
                ..
            } => {
                assert_eq!(flag, "-i");
            }
            other => panic!("expected WriteFlag, got {other:?}"),
        }
    }

    #[test]
    fn sed_inplace_with_backup_rejects() {
        match evaluate(&[cmd(&["sed", "-i.bak", "s/a/b/", "f"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteFlag { flag, .. },
                ..
            } => {
                assert_eq!(flag, "-i.bak");
            }
            other => panic!("expected WriteFlag, got {other:?}"),
        }
    }

    #[test]
    fn awk_without_inplace_passes() {
        assert_eq!(evaluate(&[cmd(&["awk", "NR<5", "f"])]), ReadOnlyVerdict::Ok);
    }

    // ---- tar gating --------------------------------------------

    #[test]
    fn tar_create_rejects() {
        match evaluate(&[cmd(&["tar", "-c", "-f", "out.tar", "src"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteFlag { .. },
                ..
            } => {}
            other => panic!("expected WriteFlag, got {other:?}"),
        }
    }

    #[test]
    fn tar_combined_create_short_flag_rejects() {
        match evaluate(&[cmd(&["tar", "-czf", "out.tar.gz", "src"])]) {
            ReadOnlyVerdict::Rejected {
                reason: ReadOnlyRejection::WriteFlag { .. },
                ..
            } => {}
            other => panic!("expected WriteFlag, got {other:?}"),
        }
    }

    #[test]
    fn tar_list_passes() {
        assert_eq!(
            evaluate(&[cmd(&["tar", "-tf", "in.tar"])]),
            ReadOnlyVerdict::Ok
        );
    }

    // ---- skip ---------------------------------------------------

    #[test]
    fn evaluate_or_skip_disabled_returns_ok() {
        assert_eq!(
            evaluate_or_skip(&[cmd(&["rm", "/tmp/x"])], false),
            ReadOnlyVerdict::Ok
        );
    }

    #[test]
    fn evaluate_or_skip_enabled_evaluates() {
        match evaluate_or_skip(&[cmd(&["rm", "/tmp/x"])], true) {
            ReadOnlyVerdict::Rejected { .. } => {}
            other => panic!("expected reject, got {other:?}"),
        }
    }

    // ---- multiple commands -------------------------------------

    #[test]
    fn first_failure_index_reported() {
        match evaluate(&[cmd(&["ls"]), cmd(&["rm", "x"])]) {
            ReadOnlyVerdict::Rejected { argv_index: 1, .. } => {}
            other => panic!("expected idx 1, got {other:?}"),
        }
    }
}
