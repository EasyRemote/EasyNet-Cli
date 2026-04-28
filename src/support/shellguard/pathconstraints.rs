// EasyNet CLI — ShellGuard: write-redirect path constraints
// =========================================================
//
// File: src/support/shellguard/pathconstraints.rs
// Description: Stage 6 of the shell.run pipeline. Caller
//              declares one or more "write-allowed roots";
//              every write redirect whose normalised target
//              path is not under at least one allowed root
//              rejects.
//
// Why this is a distinct stage
// ----------------------------
// The permission stage (slice 6) gates argv[0] and flags. It
// has no opinion about the redirect targets attached to a
// `redirected_statement` because those targets aren't argv
// elements — they're the right-hand side of `>` / `>>` / etc.
// A `git log > /etc/passwd` permission rule that only checks
// argv[0]="git" with allowed-flag "log" would let the
// redirect destroy /etc/passwd while the policy was nominally
// satisfied.
//
// This stage closes that gap. For each `Redirect` whose op
// writes data (`>`, `>>`, `>|`, `&>`, `&>>`):
//
//   1. Compute the absolute normalised target path. The cwd
//      that shell.run would run under is the reference for
//      relative targets.
//   2. Compute the canonical path WITHOUT touching the
//      filesystem (we operate on the string form because the
//      target might not exist yet — `> new-file.log` is the
//      common case). `..` segments are folded; symbolic links
//      are NOT resolved here. (The caller's ALLOWED_ROOTS list
//      should also be canonical to avoid the symlink-evasion
//      class of bypass; the receiver-side check refuses to
//      trust filesystem state because the allowed roots come
//      from policy declared at agent-spawn time, not from the
//      live filesystem.)
//   3. Match the target path against each allowed root by
//      prefix. The match is path-component-aware: `/tmp/x` is
//      under root `/tmp`, but `/tmp-other` is NOT under root
//      `/tmp`. Trailing-slash quirks normalised away.
//
// If no allowed root contains the target → reject with the
// offending target path.
//
// Read redirects (`<`, `<<`, `<<<`) are NOT gated here — the
// path constraint stage only constrains writes, since reads
// are handled by the permission stage's flag allowlist (or
// the read-only classifier in slice 7b for the read-only
// mode). A future "read-allowed-roots" extension could land
// in this module if a use case arises.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::path::{Component, Path, PathBuf};

use crate::support::shellguard::ast::{Redirect, SimpleCommand};

/// Caller-declared write-allowed roots. Empty list means "no
/// writes allowed at all" — every write redirect rejects.
/// `None` would mean "no constraint at all"; use
/// `evaluate_or_skip` instead of building an empty `Constraints`
/// for that case so the API doesn't conflate "unconstrained"
/// with "deny-all".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Constraints {
    /// Each entry is treated as an absolute path. Relative
    /// entries are normalised against `cwd` at evaluate time.
    pub write_allowed_roots: Vec<PathBuf>,
    /// Working directory the command will run under. Used to
    /// resolve relative redirect targets.
    pub cwd: PathBuf,
}

/// Outcome of evaluating one command list against constraints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathVerdict {
    Ok,
    Rejected {
        argv_index: usize,
        target: PathBuf,
        op: String,
    },
}

/// Run path constraints. Reads `cmd.redirects` for each
/// command, normalises each write-target against `cwd`, and
/// rejects on the first target outside every allowed root.
pub fn evaluate(commands: &[SimpleCommand], c: &Constraints) -> PathVerdict {
    for (idx, cmd) in commands.iter().enumerate() {
        for r in &cmd.redirects {
            if !is_write_op(&r.op) {
                continue;
            }
            let target_abs = normalise_target(&r.target, &c.cwd);
            if !is_under_any_root(&target_abs, &c.write_allowed_roots) {
                return PathVerdict::Rejected {
                    argv_index: idx,
                    target: target_abs,
                    op: r.op.clone(),
                };
            }
        }
    }
    PathVerdict::Ok
}

/// Convenience: skip the stage when no write-allowed roots
/// were declared. Returns Ok if `c` is `None`.
pub fn evaluate_or_skip(commands: &[SimpleCommand], c: Option<&Constraints>) -> PathVerdict {
    match c {
        Some(c) => evaluate(commands, c),
        None => PathVerdict::Ok,
    }
}

/// Is the redirect op a write?
fn is_write_op(op: &str) -> bool {
    matches!(op, ">" | ">>" | ">|" | "&>" | "&>>")
}

/// Normalise `target` to an absolute path, folding `..`
/// segments. No filesystem touched — purely string-level.
/// `target` is interpreted relative to `cwd` if not absolute.
///
/// `..` folding stops at root: an absolute path like `/a/../..`
/// normalises to `/`; a relative path joined to cwd is
/// canonicalised the same way.
fn normalise_target(target: &str, cwd: &Path) -> PathBuf {
    let raw = Path::new(target);
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    };
    fold_dot_dots(&absolute)
}

/// Fold `..` and `.` components in a path. Standard library
/// `canonicalize` would touch the filesystem (and fail for
/// non-existent paths, which is the usual case for write
/// targets). This is a string-only equivalent.
fn fold_dot_dots(p: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {} // drop `.`
            Component::ParentDir => {
                // Pop unless the only thing on the stack is the root.
                match out.last() {
                    Some(Component::RootDir) | Some(Component::Prefix(_)) | None => {
                        // At root or on Windows root prefix — `..` is
                        // a no-op (you can't escape the root).
                        if out.is_empty() {
                            out.push(c);
                        }
                    }
                    Some(Component::ParentDir) => out.push(c),
                    _ => {
                        out.pop();
                    }
                }
            }
            other => out.push(other),
        }
    }
    let mut buf = PathBuf::new();
    for c in out {
        buf.push(c);
    }
    if buf.as_os_str().is_empty() {
        // Pure `.` / empty input → cwd would lose absoluteness.
        // Caller's responsibility to give us an absolute input;
        // we return `/` as a safe fallback if somehow we got
        // here.
        buf.push("/");
    }
    buf
}

/// Is `target` under at least one of the `roots`? Uses
/// component-aware prefix matching: `/tmp/x` is under `/tmp`
/// but `/tmp-other` is NOT.
fn is_under_any_root(target: &Path, roots: &[PathBuf]) -> bool {
    for root in roots {
        if path_starts_with_components(target, root) {
            return true;
        }
    }
    false
}

/// Component-aware prefix match. `Path::starts_with` already
/// does this (yes, it really does — `/tmp-other`.starts_with(/tmp)
/// returns false). We expose the function name for clarity in
/// the call site.
fn path_starts_with_components(target: &Path, root: &Path) -> bool {
    target.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn cmd_no_redirect(argv: &[&str]) -> SimpleCommand {
        SimpleCommand {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env_vars: vec![],
            redirects: vec![],
            text: argv.join(" "),
        }
    }

    fn constraints(roots: &[&str], cwd: &str) -> Constraints {
        Constraints {
            write_allowed_roots: roots.iter().map(PathBuf::from).collect(),
            cwd: PathBuf::from(cwd),
        }
    }

    // ---- happy path -----------------------------------------------------

    #[test]
    fn write_inside_allowed_root_passes() {
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [cmd_with_redirect(&["echo", "hi"], ">", "/tmp/log")];
        assert_eq!(evaluate(&cmds, &c), PathVerdict::Ok);
    }

    #[test]
    fn no_redirect_at_all_passes() {
        let c = constraints(&["/tmp"], "/tmp");
        assert_eq!(evaluate(&[cmd_no_redirect(&["ls"])], &c), PathVerdict::Ok);
    }

    #[test]
    fn read_redirect_is_ignored_by_pathconstraints() {
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [cmd_with_redirect(&["cat"], "<", "/etc/passwd")];
        assert_eq!(evaluate(&cmds, &c), PathVerdict::Ok);
    }

    #[test]
    fn append_op_inside_root_passes() {
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [cmd_with_redirect(&["echo"], ">>", "/tmp/log")];
        assert_eq!(evaluate(&cmds, &c), PathVerdict::Ok);
    }

    #[test]
    fn force_overwrite_inside_root_passes() {
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [cmd_with_redirect(&["echo"], ">|", "/tmp/log")];
        assert_eq!(evaluate(&cmds, &c), PathVerdict::Ok);
    }

    // ---- rejections -----------------------------------------------------

    #[test]
    fn write_outside_root_rejects() {
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [cmd_with_redirect(&["echo"], ">", "/etc/passwd")];
        match evaluate(&cmds, &c) {
            PathVerdict::Rejected { target, op, .. } => {
                assert_eq!(target, PathBuf::from("/etc/passwd"));
                assert_eq!(op, ">");
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn dot_dot_escape_attempt_rejects() {
        // `> /tmp/../etc/passwd` — `..` folds to `/etc/passwd`
        // which is outside /tmp.
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [cmd_with_redirect(&["echo"], ">", "/tmp/../etc/passwd")];
        match evaluate(&cmds, &c) {
            PathVerdict::Rejected { target, .. } => {
                assert_eq!(target, PathBuf::from("/etc/passwd"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn relative_target_resolved_against_cwd() {
        let c = constraints(&["/tmp/work"], "/tmp/work");
        let cmds = [cmd_with_redirect(&["echo"], ">", "out.log")];
        assert_eq!(evaluate(&cmds, &c), PathVerdict::Ok);
    }

    #[test]
    fn relative_target_outside_cwd_rejects() {
        let c = constraints(&["/tmp/work"], "/tmp/work");
        let cmds = [cmd_with_redirect(&["echo"], ">", "../out.log")];
        match evaluate(&cmds, &c) {
            PathVerdict::Rejected { target, .. } => {
                assert_eq!(target, PathBuf::from("/tmp/out.log"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn lookalike_root_rejects() {
        // /tmp-other is NOT under /tmp.
        let c = constraints(&["/tmp"], "/");
        let cmds = [cmd_with_redirect(&["echo"], ">", "/tmp-other/x")];
        match evaluate(&cmds, &c) {
            PathVerdict::Rejected { .. } => {}
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn empty_roots_means_deny_all_writes() {
        let c = constraints(&[], "/tmp");
        let cmds = [cmd_with_redirect(&["echo"], ">", "/tmp/x")];
        match evaluate(&cmds, &c) {
            PathVerdict::Rejected { .. } => {}
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    // ---- multiple roots / multiple commands ------------------------------

    #[test]
    fn multiple_roots_any_match_passes() {
        let c = constraints(&["/tmp", "/var/log"], "/");
        let cmds = [
            cmd_with_redirect(&["echo"], ">", "/tmp/a"),
            cmd_with_redirect(&["echo"], ">", "/var/log/b"),
        ];
        assert_eq!(evaluate(&cmds, &c), PathVerdict::Ok);
    }

    #[test]
    fn multiple_commands_first_failure_index_reported() {
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [
            cmd_with_redirect(&["echo"], ">", "/tmp/ok"),
            cmd_with_redirect(&["echo"], ">", "/etc/bad"),
        ];
        match evaluate(&cmds, &c) {
            PathVerdict::Rejected { argv_index: 1, .. } => {}
            other => panic!("expected reject at idx 1, got {other:?}"),
        }
    }

    // ---- skip path ------------------------------------------------------

    #[test]
    fn evaluate_or_skip_with_none_returns_ok() {
        let cmds = [cmd_with_redirect(&["echo"], ">", "/etc/x")];
        assert_eq!(evaluate_or_skip(&cmds, None), PathVerdict::Ok);
    }

    #[test]
    fn evaluate_or_skip_with_some_evaluates_normally() {
        let c = constraints(&["/tmp"], "/tmp");
        let cmds = [cmd_with_redirect(&["echo"], ">", "/etc/x")];
        match evaluate_or_skip(&cmds, Some(&c)) {
            PathVerdict::Rejected { .. } => {}
            other => panic!("expected reject, got {other:?}"),
        }
    }

    // ---- helpers --------------------------------------------------------

    #[test]
    fn fold_dot_dots_handles_root_anchored() {
        assert_eq!(fold_dot_dots(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        assert_eq!(fold_dot_dots(Path::new("/a/./b")), PathBuf::from("/a/b"));
        assert_eq!(fold_dot_dots(Path::new("/a/../..")), PathBuf::from("/"));
    }

    #[test]
    fn is_write_op_table() {
        for op in [">", ">>", ">|", "&>", "&>>"] {
            assert!(is_write_op(op), "{op} should be write");
        }
        for op in ["<", "<<", "<<<", "<&", ">&"] {
            assert!(!is_write_op(op), "{op} should NOT be write");
        }
    }
}
