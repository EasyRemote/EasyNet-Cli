// EasyNet CLI — ShellGuard: dangerous-pattern detectors
// =====================================================
//
// File: src/support/shellguard/security/mod.rs
// Description: Stage 4 of the AXIOM Tier 2.5 shell.run 8-stage
//              pipeline. Runs post-AST argv-level checks that
//              the parser cannot express:
//
//                * argv[0] is an eval-like / zsh-module builtin
//                  that escapes the argv abstraction
//                  (eval/source/./trap/coproc/zmodload/...).
//                * argv carries an interpreter `-c` flag with
//                  inline code (bash -c, sh -c, python -c,
//                  perl -e, ruby -e, node -e, …) — the inline
//                  code is opaque to permission rules.
//                * argv content matches a known-dangerous
//                  pattern (`/proc/self/environ`, jq
//                  `system(...)`, dd of=/dev/{sd,nvme}…) —
//                  patterns the parser cannot assert from
//                  argv structure alone.
//
// Why three sibling modules
// -------------------------
// Each detector category answers a distinct question and is
// independently extensible:
//
//   builtins.rs — "is argv[0] one of a fixed set of names?"
//   flags.rs    — "does argv carry one of a fixed set of
//                 interpreter flags whose next argument is
//                 inline code?"
//   patterns.rs — "does any element of argv match one of a
//                 fixed set of regex / substring patterns?"
//
// New entries land in exactly one file. The orchestrator here
// runs the three in spec order and short-circuits on the first
// violation, so test cases can pin a verdict to the offending
// detector without coupling the test to detector ordering by
// accident.
//
// Public surface
// --------------
// `SecurityVerdict { Ok | Reject(SecurityViolation) }` is the
// single result the shell.run dispatcher consumes. `evaluate`
// runs the pipeline; submodules expose their per-detector
// public functions for unit-testing in isolation.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

pub mod builtins;
pub mod flags;
pub mod patterns;

use super::ast::SimpleCommand;

/// Result of running the stage-4 security pipeline against a
/// flat list of `SimpleCommand`s.
///
/// `Ok` advances to the next stage. `Reject` carries the first
/// violation found; the dispatcher MUST refuse the call and
/// surface the violation in the receipt's `denied_reason` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityVerdict {
    Ok,
    Reject(SecurityViolation),
}

/// Reason a `SimpleCommand` failed the stage-4 pipeline.
///
/// `category` selects the detector module; `name` is the
/// detector entry that fired (e.g. `"eval"`, `"bash -c"`,
/// `"proc-environ-access"`); `argv_index` is the zero-based
/// index of the offending command in the input slice;
/// `detail` carries any extra string the detector wants to
/// include in the receipt (the inline code body, the env path,
/// etc.) — kept short, no full argv echo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityViolation {
    pub category: SecurityCategory,
    pub name: String,
    pub argv_index: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityCategory {
    Builtin,
    Flag,
    Pattern,
}

impl std::fmt::Display for SecurityCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => f.write_str("builtin"),
            Self::Flag => f.write_str("flag"),
            Self::Pattern => f.write_str("pattern"),
        }
    }
}

/// Run the stage-4 pipeline against `commands`. Returns on the
/// first violation (short-circuit) so the receipt names exactly
/// one offending detector — chaining multiple violations would
/// confuse the operator about which one to fix first.
///
/// Order: builtins → flags → patterns. Spec-ordered: builtins
/// answer "is the entire argv shape illegal" (most categorical),
/// flags refine to "is a specific flag carrying inline code"
/// (more specific), patterns sweep for the residual cases that
/// fall through both. A command that hits two categories
/// surfaces the most categorical one.
pub fn evaluate(commands: &[SimpleCommand]) -> SecurityVerdict {
    for (idx, cmd) in commands.iter().enumerate() {
        if let Some(violation) = builtins::check(cmd) {
            return SecurityVerdict::Reject(SecurityViolation {
                category: SecurityCategory::Builtin,
                name: violation.name,
                argv_index: idx,
                detail: violation.detail,
            });
        }
        if let Some(violation) = flags::check(cmd) {
            return SecurityVerdict::Reject(SecurityViolation {
                category: SecurityCategory::Flag,
                name: violation.name,
                argv_index: idx,
                detail: violation.detail,
            });
        }
        if let Some(violation) = patterns::check(cmd) {
            return SecurityVerdict::Reject(SecurityViolation {
                category: SecurityCategory::Pattern,
                name: violation.name,
                argv_index: idx,
                detail: violation.detail,
            });
        }
    }
    SecurityVerdict::Ok
}

/// Per-detector hit. Each submodule's `check` returns `None` for
/// "not flagged" and `Some(DetectorHit { name, detail })` for a
/// violation. The orchestrator wraps the hit in a
/// `SecurityViolation` with the appropriate category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorHit {
    pub name: String,
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::shellguard::ast::SimpleCommand;

    fn cmd(argv: &[&str]) -> SimpleCommand {
        SimpleCommand {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env_vars: vec![],
            redirects: vec![],
            text: argv.join(" "),
        }
    }

    #[test]
    fn empty_input_is_ok() {
        assert_eq!(evaluate(&[]), SecurityVerdict::Ok);
    }

    #[test]
    fn benign_command_is_ok() {
        assert_eq!(evaluate(&[cmd(&["ls", "-la"])]), SecurityVerdict::Ok);
    }

    #[test]
    fn eval_builtin_rejects_with_builtin_category() {
        match evaluate(&[cmd(&["eval", "rm -rf /"])]) {
            SecurityVerdict::Reject(v) => {
                assert_eq!(v.category, SecurityCategory::Builtin);
                assert_eq!(v.name, "eval");
                assert_eq!(v.argv_index, 0);
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn first_violation_short_circuits() {
        // Both commands violate; the first wins.
        let v = evaluate(&[cmd(&["eval", "x"]), cmd(&["bash", "-c", "y"])]);
        match v {
            SecurityVerdict::Reject(violation) => {
                assert_eq!(violation.argv_index, 0);
                assert_eq!(violation.category, SecurityCategory::Builtin);
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn category_fmt_renders_lowercase() {
        assert_eq!(SecurityCategory::Builtin.to_string(), "builtin");
        assert_eq!(SecurityCategory::Flag.to_string(), "flag");
        assert_eq!(SecurityCategory::Pattern.to_string(), "pattern");
    }
}
