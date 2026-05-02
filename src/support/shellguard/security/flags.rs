// EasyNet CLI — ShellGuard: dangerous-flag detector
// =================================================
//
// File: src/support/shellguard/security/flags.rs
// Description: Detects interpreter `-c` (and the `-e` /
//              `--command` / `-Command` siblings) that carry
//              inline code as a positional argument. Mirrors
//              AliveCode's interpreter-c-flag rule set.
//
// Why this is a separate detector
// -------------------------------
// `bash -c "rm -rf /"` parses cleanly through the AST stage:
// argv is `["bash", "-c", "rm -rf /"]`, all tokens are `word`
// nodes, no expansion. Permission rules that match argv[0]
// would see `bash` (likely allowlisted) and the `-c` body
// stays opaque.
//
// The same pattern repeats across every interpreter:
// `python -c`, `perl -e`, `ruby -e`, `node -e`, `awk PROGRAM`,
// `sed -e`. Each one packages inline code as a positional
// argument that bypasses the argv-shape contract permission
// rules rely on.
//
// Detection strategy
// ------------------
// For each interpreter known to take inline code:
//
//   1. Match argv[0] against the interpreter table.
//   2. Look ahead in argv for one of the entry's flag names.
//   3. If the flag is followed by another argv element (the
//      code body), the command rejects.
//
// Pure positional `-c` / `-e` use without a body (`bash -c`
// alone, no following arg) is NOT flagged here — those would
// reach the actual interpreter at runtime and error; the
// danger is only inline code.
//
// Each entry surfaces its name as `<interpreter> <flag>` for
// the receipt's `name` field (`bash -c`, `python -c`,
// `perl -e`, …) so the operator immediately recognises which
// interpreter triggered the rejection.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use crate::support::shellguard::ast::SimpleCommand;

use super::DetectorHit;

/// Each interpreter known to take inline code, with the
/// flag(s) that introduce that code. argv[0] match is exact;
/// flag match is exact (no glob, no fuzzy).
///
/// Entries split per-flag (one row per `(interpreter, flag)`)
/// so the receipt names the exact pair that fired. `bash -c`
/// and `bash --command` would be two rows.
struct InterpreterFlag {
    interpreter: &'static str,
    flag: &'static str,
}

const INTERPRETER_FLAGS: &[InterpreterFlag] = &[
    // Shells
    InterpreterFlag {
        interpreter: "bash",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "sh",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "zsh",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "ksh",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "dash",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "fish",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "ash",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "busybox",
        flag: "sh",
    }, // `busybox sh -c X` would match the next interpreter row;
    // Generic interpreters
    InterpreterFlag {
        interpreter: "python",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "python2",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "python3",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "perl",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "perl",
        flag: "-E",
    },
    InterpreterFlag {
        interpreter: "ruby",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "node",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "node",
        flag: "--eval",
    },
    InterpreterFlag {
        interpreter: "node",
        flag: "-p",
    },
    InterpreterFlag {
        interpreter: "node",
        flag: "--print",
    },
    InterpreterFlag {
        interpreter: "deno",
        flag: "eval",
    }, // `deno eval CODE` (subcommand, not flag)
    InterpreterFlag {
        interpreter: "lua",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "luajit",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "tcl",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "tclsh",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "php",
        flag: "-r",
    },
    InterpreterFlag {
        interpreter: "Rscript",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "R",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "ghc",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "runghc",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "stack",
        flag: "exec",
    }, // `stack exec -- X`
    // PowerShell — Windows interop, but a Linux receiver could still
    // see `pwsh -Command X` from a remote caller.
    InterpreterFlag {
        interpreter: "pwsh",
        flag: "-Command",
    },
    InterpreterFlag {
        interpreter: "pwsh",
        flag: "-c",
    },
    InterpreterFlag {
        interpreter: "powershell",
        flag: "-Command",
    },
    InterpreterFlag {
        interpreter: "powershell",
        flag: "-c",
    },
    // sed / awk — `-e PROGRAM` is the inline-code form
    InterpreterFlag {
        interpreter: "sed",
        flag: "-e",
    },
    InterpreterFlag {
        interpreter: "awk",
        flag: "-v",
    }, // `-v var=$(rm)` — value is shell-evaluated by awk runtime
    InterpreterFlag {
        interpreter: "gawk",
        flag: "-v",
    },
    InterpreterFlag {
        interpreter: "mawk",
        flag: "-v",
    },
    // env -S splits + executes — `env -S "rm -rf /"` runs the embedded
    // shell-like syntax through env's GNU extension.
    InterpreterFlag {
        interpreter: "env",
        flag: "-S",
    },
];

/// Public entry. Returns `Some(DetectorHit)` if argv[0] matches
/// an interpreter table entry AND the matching flag is followed
/// by an additional argv element (the code body).
pub fn check(cmd: &SimpleCommand) -> Option<DetectorHit> {
    let head = cmd.argv.first()?;
    for entry in INTERPRETER_FLAGS {
        if entry.interpreter != head {
            continue;
        }
        // Scan argv tail for the flag.
        for (i, tok) in cmd.argv.iter().enumerate().skip(1) {
            if tok == entry.flag && cmd.argv.get(i + 1).is_some() {
                let body = &cmd.argv[i + 1];
                let detail = if body.len() > 80 {
                    format!("{}…", &body[..80])
                } else {
                    body.clone()
                };
                return Some(DetectorHit {
                    name: format!("{} {}", entry.interpreter, entry.flag),
                    detail,
                });
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

    #[test]
    fn benign_command_is_none() {
        assert!(check(&cmd(&["ls", "-la"])).is_none());
    }

    #[test]
    fn bash_dash_c_with_body_rejects() {
        let h = check(&cmd(&["bash", "-c", "rm -rf /"])).unwrap();
        assert_eq!(h.name, "bash -c");
        assert_eq!(h.detail, "rm -rf /");
    }

    #[test]
    fn bash_dash_c_without_body_does_not_reject() {
        // `bash -c` with no following arg would error at runtime
        // — not our problem, no inline code body to flag.
        assert!(check(&cmd(&["bash", "-c"])).is_none());
    }

    #[test]
    fn sh_dash_c_rejects() {
        assert_eq!(check(&cmd(&["sh", "-c", "rm /tmp"])).unwrap().name, "sh -c");
    }

    #[test]
    fn zsh_dash_c_rejects() {
        assert_eq!(check(&cmd(&["zsh", "-c", "x"])).unwrap().name, "zsh -c");
    }

    #[test]
    fn python_dash_c_rejects() {
        assert_eq!(
            check(&cmd(&["python", "-c", "import os"])).unwrap().name,
            "python -c"
        );
    }

    #[test]
    fn python3_dash_c_rejects() {
        assert_eq!(
            check(&cmd(&["python3", "-c", "x"])).unwrap().name,
            "python3 -c"
        );
    }

    #[test]
    fn perl_dash_e_rejects() {
        assert_eq!(
            check(&cmd(&["perl", "-e", "exit"])).unwrap().name,
            "perl -e"
        );
    }

    #[test]
    fn perl_dash_capital_e_rejects() {
        assert_eq!(
            check(&cmd(&["perl", "-E", "say 1"])).unwrap().name,
            "perl -E"
        );
    }

    #[test]
    fn ruby_dash_e_rejects() {
        assert_eq!(
            check(&cmd(&["ruby", "-e", "puts 1"])).unwrap().name,
            "ruby -e"
        );
    }

    #[test]
    fn node_dash_e_rejects() {
        assert_eq!(check(&cmd(&["node", "-e", "1"])).unwrap().name, "node -e");
    }

    #[test]
    fn node_long_eval_flag_rejects() {
        assert_eq!(
            check(&cmd(&["node", "--eval", "1"])).unwrap().name,
            "node --eval"
        );
    }

    #[test]
    fn pwsh_command_rejects() {
        assert_eq!(
            check(&cmd(&["pwsh", "-Command", "Get-Process"]))
                .unwrap()
                .name,
            "pwsh -Command"
        );
    }

    #[test]
    fn sed_dash_e_rejects() {
        assert_eq!(
            check(&cmd(&["sed", "-e", "s/a/b/", "f"])).unwrap().name,
            "sed -e"
        );
    }

    #[test]
    fn env_dash_s_rejects() {
        assert_eq!(
            check(&cmd(&["env", "-S", "rm -rf /"])).unwrap().name,
            "env -S"
        );
    }

    #[test]
    fn flag_must_be_followed_by_body() {
        // `-c` is the LAST argv element → no body → no match.
        assert!(check(&cmd(&["bash", "script.sh", "-c"])).is_none());
    }

    #[test]
    fn flag_in_middle_with_body_after_rejects() {
        // `bash -i -c CODE` — `-c` is mid-argv, body is after.
        let h = check(&cmd(&["bash", "-i", "-c", "rm /tmp"])).unwrap();
        assert_eq!(h.name, "bash -c");
        assert_eq!(h.detail, "rm /tmp");
    }

    #[test]
    fn body_truncates_at_80_chars() {
        let body = "x".repeat(200);
        let h = check(&cmd(&["bash", "-c", &body])).unwrap();
        // 80 ASCII chars + 3-byte UTF-8 `…` = 83 bytes max.
        assert!(h.detail.len() <= 83);
        assert!(h.detail.ends_with('…'));
    }

    #[test]
    fn unknown_interpreter_does_not_match() {
        // `myscript -c X` — `myscript` is not an interpreter we
        // know about.
        assert!(check(&cmd(&["myscript", "-c", "x"])).is_none());
    }

    #[test]
    fn flag_for_different_interpreter_does_not_match() {
        // `bash -e CODE` — `-e` is perl/ruby, not bash. bash
        // -e means "exit on error" with no argument required —
        // not the inline-code form.
        assert!(check(&cmd(&["bash", "-e", "set"])).is_none());
    }
}
