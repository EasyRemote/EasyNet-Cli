// EasyNet CLI — ShellGuard: destructive command list
// ====================================================
//
// File: src/support/shellguard/destructive.rs
// Description: The normative list of base command names that
//              `process.exec` and `shell.run` refuse to invoke
//              without `destructive_acknowledged: true` opt-in.
//
// Why this is its own module
// ---------------------------
// AXIOM Tier 2.5 §"shell.run / 8-stage pipeline / Stage 7"
// names "destructive command warning" as a stage every
// implementation MUST run. The list of names IS the contract;
// any caller writing rules against `process.exec` or
// `shell.run` reasons about whether their planned command
// matches one of these names.
//
// Putting the list here (not inside one ability handler)
// guarantees:
//
//   1. Both `process.exec` and `shell.run` see exactly the same
//      list. A rename of `rm` → `remove` would otherwise need
//      to land in two places.
//   2. The list can be tested in isolation. Adding `chsh` is
//      a one-line change with one new test.
//   3. Cross-implementation conformance vectors (Rust vs.
//      future Go / Python ports) reference this exact set;
//      drift between two ports would cause one to refuse a
//      call the other accepts, an immediately observable
//      protocol break.
//
// Categories included (AXIOM Tier 2.5 spec):
//   - delete:   rm, rmdir, shred, trash, srm
//   - overwrite raw: dd
//   - format:   mkfs and mkfs.* family
//   - partition: fdisk, gdisk, parted, sfdisk, cfdisk
//   - bulk find+delete: find with -delete, xargs rm (caught
//                      pipeline-side by `shell.run` stage 4,
//                      not here — this list is base-command
//                      only)
//
// Categories EXCLUDED (deliberately, with rationale):
//   - mv: not destructive in the "data loss" sense — moving a
//     file preserves it. A caller that wants to refuse mv as
//     destructive can compose their own deny rule;
//     base-command list stays focused on irreversible-loss.
//   - cp -f: same reasoning as mv.
//   - tee: not destructive on its own; `tee file` is just a
//     write. The redirection to /etc/hosts type danger is
//     caught by `pathconstraints`, not here.
//   - kill / killall / pkill: process-state mutation, not
//     filesystem destruction. Lives in a future
//     `process_destructive` list when needed.
//   - reboot / shutdown / poweroff: catastrophic but distinct
//     category. Future `system_destructive` list.
//
// The list is intentionally narrow. Its job is to catch the
// "I asked for `rm -rf` and didn't realise" class of mistake,
// not to be a complete sandboxing answer.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

/// The normative AXIOM Tier 2.5 destructive base-command list.
///
/// Sorted alphabetically inside each category for grep-ability.
/// Each entry MUST be a bare base command name (no path, no
/// args). Path stripping happens in [`is_destructive`] so a
/// caller can pass either `"rm"` or `"/usr/bin/rm"` and get
/// the same answer.
const DESTRUCTIVE: &[&str] = &[
    // delete
    "rm",
    "rmdir",
    "shred",
    "srm",
    "trash",
    // overwrite raw
    "dd",
    // partition
    "cfdisk",
    "fdisk",
    "gdisk",
    "parted",
    "sfdisk",
];

/// Prefix-matched destructive families. Entry `"mkfs"` flags
/// every `mkfs*` (e.g. `mkfs.ext4`, `mkfs.btrfs`,
/// `mkfs.exfat`). Distinct from the bare list because there
/// are dozens of mkfs variants and listing them all by name
/// would be brittle.
const DESTRUCTIVE_PREFIXES: &[&str] = &[
    // every mkfs.* variant on Linux/BSD
    "mkfs",
    // every wipefs.* — niche, but same hazard class as mkfs
    "wipefs",
];

/// Return `true` if `command` is on the AXIOM Tier 2.5
/// destructive list. The argument may be a bare command name
/// (e.g. `"rm"`) or a full path (e.g. `"/usr/bin/rm"`); the
/// fn strips to the basename before matching.
///
/// Pathological inputs (empty string, path that resolves to
/// the empty basename) return `false` — they are not the
/// destructive list's problem, the caller must reject them
/// upstream as schema-empty.
pub fn is_destructive(command: &str) -> bool {
    let base = basename(command);
    if base.is_empty() {
        return false;
    }
    if DESTRUCTIVE.iter().any(|&c| c == base) {
        return true;
    }
    DESTRUCTIVE_PREFIXES.iter().any(|&prefix| {
        // Match `prefix` exactly OR `prefix.` followed by
        // anything (mkfs.ext4, wipefs.btrfs). Bare `prefix`
        // catches the no-suffix case (Linux historically
        // ships `mkfs` as a wrapper around the family).
        base == prefix
            || (base.len() > prefix.len()
                && base.starts_with(prefix)
                && base.as_bytes()[prefix.len()] == b'.')
    })
}

/// Strip a command argument to its basename for list lookup.
///
/// Splits on BOTH `/` and `\` regardless of host platform.
/// `Path::file_name` would only recognise the host's native
/// separator (no backslash on Linux, no forward slash inside
/// drive specs on Windows), but a destructive-list lookup
/// must classify correctly even when a Linux receiver gets a
/// Windows-style command string from a remote caller (or
/// vice versa). The split is manual for that reason.
///
/// `.exe` (case-insensitive) is stripped on every platform
/// for the same reason — a remote caller emitting Windows
/// paths shouldn't bypass the destructive list because their
/// command name carries a Windows extension.
pub fn basename(command: &str) -> &str {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return "";
    }
    let last_slash = trimmed.rfind('/');
    let last_backslash = trimmed.rfind('\\');
    let split_at = match (last_slash, last_backslash) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let raw = match split_at {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    };
    if raw.len() > 4 {
        let tail = &raw[raw.len() - 4..];
        if tail.eq_ignore_ascii_case(".exe") {
            return &raw[..raw.len() - 4];
        }
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rm_is_destructive() {
        assert!(is_destructive("rm"));
    }

    #[test]
    fn rm_with_absolute_path_is_destructive() {
        assert!(is_destructive("/bin/rm"));
        assert!(is_destructive("/usr/bin/rm"));
    }

    #[test]
    fn windows_rm_exe_matches_unix_rm_entry() {
        // Cross-platform: a Windows path string fed to a Linux
        // receiver (or via a remote invocation from a Windows
        // operator) still classifies correctly.
        assert!(is_destructive(r"C:\Windows\System32\rm.exe"));
        assert!(is_destructive("rm.EXE"));
    }

    #[test]
    fn dd_is_destructive() {
        assert!(is_destructive("dd"));
        assert!(is_destructive("/usr/bin/dd"));
    }

    #[test]
    fn mkfs_family_is_destructive_via_prefix() {
        assert!(is_destructive("mkfs"));
        assert!(is_destructive("mkfs.ext4"));
        assert!(is_destructive("mkfs.btrfs"));
        assert!(is_destructive("/sbin/mkfs.exfat"));
    }

    #[test]
    fn wipefs_is_destructive() {
        assert!(is_destructive("wipefs"));
        assert!(is_destructive("wipefs.btrfs"));
    }

    #[test]
    fn fdisk_family_is_destructive() {
        for name in ["fdisk", "gdisk", "sfdisk", "cfdisk", "parted"] {
            assert!(is_destructive(name), "{name} should be destructive");
        }
    }

    #[test]
    fn mv_is_not_on_destructive_list() {
        // Documented exclusion: moving preserves data.
        assert!(!is_destructive("mv"));
    }

    #[test]
    fn cp_is_not_on_destructive_list() {
        // Same reasoning as mv. A caller can compose their own
        // deny rule for `cp -f` if they want strictness; the
        // base-command list does not assume.
        assert!(!is_destructive("cp"));
    }

    #[test]
    fn kill_is_not_on_destructive_list() {
        // Documented exclusion: process-state, not filesystem.
        assert!(!is_destructive("kill"));
        assert!(!is_destructive("killall"));
    }

    #[test]
    fn benign_commands_are_not_destructive() {
        for name in ["ls", "cat", "grep", "find", "echo", "true"] {
            assert!(
                !is_destructive(name),
                "{name} should NOT be destructive",
            );
        }
    }

    #[test]
    fn empty_input_returns_false() {
        assert!(!is_destructive(""));
        assert!(!is_destructive("   "));
        assert!(!is_destructive("/"));
    }

    #[test]
    fn near_miss_does_not_match_prefix_family() {
        // `mkfsanything` (without the dot) MUST NOT match the
        // mkfs prefix family. Otherwise a benign tool named
        // `mkfsbackup` would be wrongly flagged.
        assert!(!is_destructive("mkfsanything"));
        assert!(!is_destructive("mkfsbackup"));
        assert!(!is_destructive("rmdirsync")); // not rmdir
    }

    #[test]
    fn basename_strips_path() {
        assert_eq!(basename("rm"), "rm");
        assert_eq!(basename("/bin/rm"), "rm");
        assert_eq!(basename("/usr/local/bin/dd"), "dd");
        assert_eq!(basename(""), "");
        assert_eq!(basename("   "), "");
    }

    #[test]
    fn basename_strips_exe_suffix() {
        assert_eq!(basename("rm.exe"), "rm");
        assert_eq!(basename("rm.EXE"), "rm");
        assert_eq!(basename(r"C:\bin\rm.exe"), "rm");
    }

    #[test]
    fn basename_does_not_strip_other_extensions() {
        // .py / .sh / .bat are not stripped — those are real
        // command names, not Windows executable suffixes.
        assert_eq!(basename("script.py"), "script.py");
        assert_eq!(basename("install.sh"), "install.sh");
        assert_eq!(basename("setup.bat"), "setup.bat");
    }

    #[test]
    fn no_duplicates_in_list() {
        // Sanity: alphabetical ordering inside categories is a
        // grep convention, not enforced by sort_dedup. Catch a
        // future copy-paste duplicate at test time.
        let mut sorted = DESTRUCTIVE.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), DESTRUCTIVE.len(), "duplicate in DESTRUCTIVE");

        let mut p = DESTRUCTIVE_PREFIXES.to_vec();
        p.sort();
        p.dedup();
        assert_eq!(
            p.len(),
            DESTRUCTIVE_PREFIXES.len(),
            "duplicate in DESTRUCTIVE_PREFIXES"
        );
    }
}
