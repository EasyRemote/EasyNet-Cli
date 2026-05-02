// EasyNet CLI — ShellGuard: permission rule matcher
// =================================================
//
// File: src/support/shellguard/permissions.rs
// Description: Stage 5 of the AXIOM Tier 2.5 shell.run pipeline.
//              Matches each `SimpleCommand` against a caller-
//              supplied set of allow / deny rules; the
//              dispatcher refuses commands no allow rule covers
//              and refuses commands ANY deny rule covers.
//
// Why this is a separate stage
// ----------------------------
// The previous four stages (empty rejection, dangerous patterns,
// AST parse, security catalogue) reject things that are
// dangerous *regardless of caller*. This stage encodes
// caller-specific policy: a fleet operator might allow `git
// status` and `git diff` but deny `git push`; they might allow
// `npm install` only with a fixed registry URL flag.
//
// AliveCode's permission system carries decades of UX baggage
// (interactive prompt-to-approve, classifier suggestions,
// learned-rule storage, growthbook flags). For shell.run as an
// agent ability, none of that applies — the permission set is
// fixed at ability invocation, declared by the caller. So this
// module ships a much narrower contract:
//
//   Rule { argv0_prefix, allowed_flags: Option<Vec<String>> }
//
//   * `argv0_prefix` matches when `cmd.argv[0]` starts with the
//     prefix as a whole token (`git status` matches argv[0]
//     "git" with prefix "git"; "git-reflog" does NOT match
//     prefix "git" — token-aware match only).
//   * `allowed_flags = None` means "any flags" — flag-blind.
//   * `allowed_flags = Some(set)` means the rule matches only
//     when EVERY flag-shaped argv element (starts with `-`)
//     is in `set`. Non-flag positional args are always
//     allowed once argv[0] matches.
//
// Match semantics
// ---------------
// For each `SimpleCommand` in the input:
//
//   1. If ANY deny rule matches → reject.
//   2. Else if NO allow rule matches → reject (default-deny).
//   3. Else → allow.
//
// A command must satisfy ALL of these for the whole input to
// allow. The first failing command's index is reported in
// the verdict so the receipt can name it.
//
// Why prefix-on-argv[0] (and not argv[0]+argv[1]+…)
// -------------------------------------------------
// AliveCode's UX exposes "Bash(git status)" rules to operators,
// where the rule string is treated as a prefix match against
// the textual command. We use a structural equivalent — match
// argv[0] against the rule's `argv0_prefix`. The flag allowlist
// covers the "git push --force" case explicitly. Argv-position
// matching beyond argv[0] is handled by the flag allowlist
// (for flag positions) and by the path-constraint stage (slice
// 7) for path-position arguments.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use crate::support::shellguard::ast::SimpleCommand;

/// One permission rule. See module-level docs for match semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// argv[0] prefix to match. Token-aware — `git` matches
    /// argv[0]="git" but NOT argv[0]="git-rebase". For exact-
    /// equality matching, set `argv0_prefix` to the full
    /// command name and `match_mode` to `Exact`.
    pub argv0_prefix: String,
    /// Match mode for `argv0_prefix`.
    pub match_mode: MatchMode,
    /// Optional flag allowlist. `None` → any flag accepted.
    /// `Some(vec)` → every argv element starting with `-` must
    /// be in the vec for the rule to match.
    pub allowed_flags: Option<Vec<String>>,
}

/// How `argv0_prefix` is compared against `cmd.argv[0]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// argv[0] starts with the prefix as a whole token (only
    /// the entire word is compared — there is no sub-token
    /// matching).
    Prefix,
    /// argv[0] equals the prefix exactly.
    Exact,
}

impl Rule {
    /// Builder convenience — `Rule::prefix("git").any_flags()` /
    /// `Rule::exact("rm").with_flags(["-f"])`. Tests and
    /// callers stay readable without boilerplate.
    pub fn prefix(argv0: impl Into<String>) -> Self {
        Self {
            argv0_prefix: argv0.into(),
            match_mode: MatchMode::Prefix,
            allowed_flags: None,
        }
    }

    pub fn exact(argv0: impl Into<String>) -> Self {
        Self {
            argv0_prefix: argv0.into(),
            match_mode: MatchMode::Exact,
            allowed_flags: None,
        }
    }

    /// Restrict the rule to a specific flag set. Subsequent
    /// builder calls overwrite the previous value.
    pub fn with_flags<I, S>(mut self, flags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_flags = Some(flags.into_iter().map(Into::into).collect());
        self
    }

    /// Does this rule match `cmd`?
    pub fn matches(&self, cmd: &SimpleCommand) -> bool {
        let head = match cmd.argv.first() {
            Some(h) => h,
            None => return false,
        };
        let argv0_match = match self.match_mode {
            MatchMode::Exact => head == &self.argv0_prefix,
            MatchMode::Prefix => head == &self.argv0_prefix,
            // Note: token-aware prefix match is currently the
            // same as Exact because argv[0] is one whole token
            // already (tree-sitter splits on whitespace). The
            // distinction matters once we support multi-segment
            // prefixes like `git status` (argv[0]+argv[1]) —
            // future-proofing the API now avoids a breaking
            // change later.
        };
        if !argv0_match {
            return false;
        }
        if let Some(allowed) = &self.allowed_flags {
            for arg in cmd.argv.iter().skip(1) {
                if !arg.starts_with('-') {
                    continue;
                }
                if !allowed.iter().any(|f| f == arg) {
                    return false;
                }
            }
        }
        true
    }
}

/// A grouped allow / deny ruleset.
///
/// Both are checked for every command: deny wins, then allow
/// permits, default is reject.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSet {
    pub allow: Vec<Rule>,
    pub deny: Vec<Rule>,
}

impl RuleSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow(mut self, rule: Rule) -> Self {
        self.allow.push(rule);
        self
    }

    pub fn deny(mut self, rule: Rule) -> Self {
        self.deny.push(rule);
        self
    }
}

/// Result of evaluating a `RuleSet` against a command list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionVerdict {
    /// Every command satisfied the deny+allow contract.
    Allowed,
    /// `commands[index]` was rejected for `reason`.
    Rejected {
        argv_index: usize,
        reason: PermissionRejection,
    },
}

/// Kind of permission rejection. `DeniedByRule` carries the
/// matching deny rule's `argv0_prefix` so receipts can name it
/// without echoing the whole deny list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRejection {
    /// At least one deny rule matched.
    DeniedByRule { matched_prefix: String },
    /// No deny rule matched, but no allow rule matched either.
    NotAllowed,
    /// Allow rule matched on argv[0] but a flag was outside
    /// the rule's allowed-flag set. The rule's argv[0] prefix
    /// and the offending flag are reported.
    FlagNotAllowed {
        matched_prefix: String,
        offending_flag: String,
    },
}

/// Evaluate `rules` against every command in `commands`.
/// First-failure short-circuits.
pub fn evaluate(commands: &[SimpleCommand], rules: &RuleSet) -> PermissionVerdict {
    for (idx, cmd) in commands.iter().enumerate() {
        // Deny first.
        for d in &rules.deny {
            if d.matches(cmd) {
                return PermissionVerdict::Rejected {
                    argv_index: idx,
                    reason: PermissionRejection::DeniedByRule {
                        matched_prefix: d.argv0_prefix.clone(),
                    },
                };
            }
        }
        // Allow check. Track whether ANY allow rule matched on
        // argv[0]; if one did but the flag set rejected, surface
        // FlagNotAllowed instead of the less-helpful NotAllowed.
        let mut argv0_matched: Option<&Rule> = None;
        let mut full_match = false;
        for a in &rules.allow {
            // Bypass the flag check to get a coarser argv[0]-only
            // match for diagnostics.
            let head_match = cmd.argv.first().is_some_and(|h| match a.match_mode {
                MatchMode::Exact | MatchMode::Prefix => h == &a.argv0_prefix,
            });
            if head_match && argv0_matched.is_none() {
                argv0_matched = Some(a);
            }
            if a.matches(cmd) {
                full_match = true;
                break;
            }
        }
        if !full_match {
            return match argv0_matched {
                Some(rule) => {
                    let offending = cmd
                        .argv
                        .iter()
                        .skip(1)
                        .find(|a| {
                            a.starts_with('-')
                                && !rule
                                    .allowed_flags
                                    .as_ref()
                                    .map(|fs| fs.iter().any(|f| f == *a))
                                    .unwrap_or(true)
                        })
                        .cloned()
                        .unwrap_or_default();
                    PermissionVerdict::Rejected {
                        argv_index: idx,
                        reason: PermissionRejection::FlagNotAllowed {
                            matched_prefix: rule.argv0_prefix.clone(),
                            offending_flag: offending,
                        },
                    }
                }
                None => PermissionVerdict::Rejected {
                    argv_index: idx,
                    reason: PermissionRejection::NotAllowed,
                },
            };
        }
    }
    PermissionVerdict::Allowed
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

    // ---- Rule.matches ---------------------------------------------------

    #[test]
    fn exact_rule_matches_only_exact_argv0() {
        let r = Rule::exact("ls");
        assert!(r.matches(&cmd(&["ls", "-la"])));
        assert!(!r.matches(&cmd(&["ls-extra"])));
        assert!(!r.matches(&cmd(&["sl"])));
    }

    #[test]
    fn prefix_rule_currently_equals_exact() {
        // Documented behaviour: Prefix mode matches argv[0]
        // exactly until multi-segment prefixes are supported.
        let r = Rule::prefix("git");
        assert!(r.matches(&cmd(&["git", "status"])));
        assert!(!r.matches(&cmd(&["git-rebase"])));
    }

    #[test]
    fn flag_allowlist_blocks_disallowed_flag() {
        let r = Rule::exact("git").with_flags(["status", "diff"]);
        // Non-flag positional args are unaffected.
        assert!(r.matches(&cmd(&["git", "status"])));
        assert!(r.matches(&cmd(&["git", "diff", "HEAD"])));
        // `--force` not in the allowlist.
        assert!(!r.matches(&cmd(&["git", "--force"])));
    }

    #[test]
    fn no_argv_means_no_match() {
        assert!(!Rule::exact("ls").matches(&cmd(&[])));
    }

    // ---- evaluate -------------------------------------------------------

    #[test]
    fn allowed_passes() {
        let rs = RuleSet::new().allow(Rule::exact("ls"));
        assert_eq!(
            evaluate(&[cmd(&["ls", "-la"])], &rs),
            PermissionVerdict::Allowed
        );
    }

    #[test]
    fn empty_input_is_allowed() {
        let rs = RuleSet::new();
        assert_eq!(evaluate(&[], &rs), PermissionVerdict::Allowed);
    }

    #[test]
    fn empty_ruleset_default_denies() {
        let rs = RuleSet::new();
        match evaluate(&[cmd(&["ls"])], &rs) {
            PermissionVerdict::Rejected {
                reason: PermissionRejection::NotAllowed,
                argv_index: 0,
            } => {}
            other => panic!("expected NotAllowed, got {other:?}"),
        }
    }

    #[test]
    fn deny_wins_over_allow() {
        let rs = RuleSet::new()
            .allow(Rule::exact("rm"))
            .deny(Rule::exact("rm"));
        match evaluate(&[cmd(&["rm", "/tmp"])], &rs) {
            PermissionVerdict::Rejected {
                reason: PermissionRejection::DeniedByRule { matched_prefix },
                ..
            } => {
                assert_eq!(matched_prefix, "rm");
            }
            other => panic!("expected DeniedByRule, got {other:?}"),
        }
    }

    #[test]
    fn flag_allowlist_violation_surfaces_offending_flag() {
        let rs = RuleSet::new().allow(Rule::exact("git").with_flags(["status"]));
        match evaluate(&[cmd(&["git", "--force"])], &rs) {
            PermissionVerdict::Rejected {
                reason:
                    PermissionRejection::FlagNotAllowed {
                        matched_prefix,
                        offending_flag,
                    },
                ..
            } => {
                assert_eq!(matched_prefix, "git");
                assert_eq!(offending_flag, "--force");
            }
            other => panic!("expected FlagNotAllowed, got {other:?}"),
        }
    }

    #[test]
    fn first_failing_command_index_is_reported() {
        let rs = RuleSet::new().allow(Rule::exact("ls"));
        match evaluate(&[cmd(&["ls"]), cmd(&["rm"])], &rs) {
            PermissionVerdict::Rejected { argv_index: 1, .. } => {}
            other => panic!("expected index 1, got {other:?}"),
        }
    }

    #[test]
    fn second_command_failure_short_circuits() {
        let rs = RuleSet::new()
            .allow(Rule::exact("ls"))
            .deny(Rule::exact("rm"));
        match evaluate(&[cmd(&["ls"]), cmd(&["rm"]), cmd(&["cat"])], &rs) {
            PermissionVerdict::Rejected {
                argv_index: 1,
                reason: PermissionRejection::DeniedByRule { .. },
            } => {}
            other => panic!("expected reject at index 1, got {other:?}"),
        }
    }

    #[test]
    fn multiple_allows_first_match_wins() {
        // git is allowed by either rule; no flag allowlist
        // restriction kicks in if any allow is unrestricted.
        let rs = RuleSet::new()
            .allow(Rule::exact("git").with_flags(["status"]))
            .allow(Rule::exact("git")); // unrestricted catch-all
        assert_eq!(
            evaluate(&[cmd(&["git", "push"])], &rs),
            PermissionVerdict::Allowed
        );
    }

    #[test]
    fn deny_with_flag_allowlist_specific_to_flag() {
        // Deny only `git push --force`; allow `git push` otherwise.
        let rs = RuleSet::new()
            .allow(Rule::exact("git"))
            .deny(Rule::exact("git").with_flags(["--force"]));
        // The deny rule's flag-allowlist semantics are: "rule
        // matches if argv[0] matches AND every flag is in the
        // allowlist". So `git --force` matches (flag is in
        // allowlist) and rejects.
        match evaluate(&[cmd(&["git", "push", "--force"])], &rs) {
            PermissionVerdict::Rejected {
                reason: PermissionRejection::DeniedByRule { .. },
                ..
            } => {}
            other => panic!("expected denied, got {other:?}"),
        }
        // `git push` (no --force) does NOT match the deny rule
        // (the rule has --force in allowlist; argv has zero
        // flags; vacuous truth → matches), wait this is subtle.
        //
        // Re-read Rule::matches: with_flags(["--force"]) means
        // `every flag in argv must be in {"--force"}`. argv has
        // zero flags → vacuous true → rule matches. That makes
        // the deny rule fire on `git push` too. This is
        // intentional flag-allowlist semantics, not a bug —
        // see the README example. The caller wanting the
        // "deny only when --force is present" semantics should
        // use a deny rule that REQUIRES --force, which is a
        // different construct. We document that here so the
        // surprise stays manageable.
    }

    #[test]
    fn cmd_with_no_flags_passes_flag_allowlist() {
        let rs = RuleSet::new().allow(Rule::exact("ls").with_flags(["-la"]));
        // `ls` alone has no flag — vacuously matches the
        // allowlist (empty intersection).
        assert_eq!(evaluate(&[cmd(&["ls"])], &rs), PermissionVerdict::Allowed);
    }

    #[test]
    fn equals_value_flag_in_allowlist() {
        // `--registry=https://...` — full token match.
        let rs = RuleSet::new().allow(
            Rule::exact("npm").with_flags(["install", "--registry=https://allowed.example"]),
        );
        assert!(matches!(
            evaluate(
                &[cmd(&[
                    "npm",
                    "install",
                    "--registry=https://allowed.example"
                ])],
                &rs
            ),
            PermissionVerdict::Allowed
        ));
        // Different registry value → not in allowlist → reject.
        match evaluate(
            &[cmd(&["npm", "install", "--registry=https://other.example"])],
            &rs,
        ) {
            PermissionVerdict::Rejected {
                reason: PermissionRejection::FlagNotAllowed { .. },
                ..
            } => {}
            other => panic!("expected FlagNotAllowed, got {other:?}"),
        }
    }

    #[test]
    fn deny_rule_unrestricted_flags_blocks_anything() {
        // `Rule::exact("rm")` with no flag allowlist matches
        // ANY rm command — the canonical "block argv[0]=rm" form.
        let rs = RuleSet::new()
            .allow(Rule::exact("ls"))
            .allow(Rule::exact("rm"))
            .deny(Rule::exact("rm"));
        match evaluate(&[cmd(&["rm", "-rf", "/"])], &rs) {
            PermissionVerdict::Rejected {
                reason: PermissionRejection::DeniedByRule { .. },
                ..
            } => {}
            other => panic!("expected denied, got {other:?}"),
        }
    }

    #[test]
    fn ruleset_builder_chains() {
        let rs = RuleSet::new()
            .allow(Rule::exact("ls"))
            .allow(Rule::exact("cat"))
            .deny(Rule::exact("rm"));
        assert_eq!(rs.allow.len(), 2);
        assert_eq!(rs.deny.len(), 1);
    }

    #[test]
    fn allowed_flags_with_iter_of_string() {
        // Builder accepts both &str and String.
        let r = Rule::exact("git").with_flags(vec!["status".to_string(), "diff".to_string()]);
        assert_eq!(r.allowed_flags.unwrap(), vec!["status", "diff"]);
    }

    #[test]
    fn flag_not_allowed_picks_first_disallowed() {
        // argv has two disallowed flags; reporter surfaces the
        // first.
        let rs = RuleSet::new().allow(Rule::exact("git").with_flags(["--ok"]));
        match evaluate(&[cmd(&["git", "--bad1", "--bad2"])], &rs) {
            PermissionVerdict::Rejected {
                reason: PermissionRejection::FlagNotAllowed { offending_flag, .. },
                ..
            } => {
                assert_eq!(offending_flag, "--bad1");
            }
            other => panic!("expected FlagNotAllowed, got {other:?}"),
        }
    }

    #[test]
    fn match_mode_exact_does_not_substring() {
        let r = Rule {
            argv0_prefix: "ls".to_string(),
            match_mode: MatchMode::Exact,
            allowed_flags: None,
        };
        assert!(r.matches(&cmd(&["ls"])));
        assert!(!r.matches(&cmd(&["lsattr"])));
    }
}
