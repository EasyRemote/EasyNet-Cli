// EasyNet CLI — Pages reference system: kernel-enforced read sandbox
// ==================================================================
//
// File: src/runtime/agents/pages/sandbox.rs
// Description: opens a file inside the published folder root using
//              a kernel contract that path resolution cannot escape.
//
//              Linux: openat2(2) with RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS
//              | RESOLVE_NO_MAGICLINKS. The kernel itself refuses to
//              resolve any path that would leave the dirfd, regardless
//              of `..` components, regardless of symlinks, regardless
//              of filesystem mutations between calls.
//
//              macOS: realpath() + prefix-check + O_NOFOLLOW. Weaker
//              (TOCTOU race possible during the gap between realpath
//              and open); production targets Linux. Documented in
//              RFC-006-B v0.6 §5.
//
// Conformance: RFC-006-B v0.6 INV-3 (Deterministic Projection)
//              + the read-sandbox semantic obligation (§4.4).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs::File;
use std::path::Path;

#[cfg(unix)]
pub type PublishedFolderHandle = std::os::fd::OwnedFd;
#[cfg(not(unix))]
pub type PublishedFolderHandle = std::path::PathBuf;

/// Result of sandboxed open: a regular-file `File` handle that the
/// kernel guarantees lives inside the dirfd's subtree.
///
/// On Linux this is enforced by `openat2(2)` with
/// `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`
/// — the kernel itself refuses to resolve a path that would leave
/// the dirfd. On macOS / other unixes the contract is approximated
/// by `realpath()` + prefix-check + `O_NOFOLLOW`. Production
/// targets Linux; macOS support is dev-only.
pub fn open_beneath(
    folder_handle: &PublishedFolderHandle,
    canonical_root: &Path,
    rel_path: &str,
) -> anyhow::Result<File> {
    // Reject obvious abuse before we even talk to the kernel. The
    // kernel will reject too, but failing fast makes log lines
    // and receipts cleaner.
    if rel_path.is_empty() {
        anyhow::bail!("path is empty");
    }
    let normalized = rel_path.trim_start_matches('/');
    if normalized.is_empty() {
        anyhow::bail!("path resolves to empty after trim");
    }

    // Default-deny dotfiles. Walks every path segment because
    // `assets/.git/HEAD` is a dotfile probe even if the leading
    // segment looks innocuous.
    for seg in normalized.split('/') {
        if seg.starts_with('.') {
            anyhow::bail!("dotfile path component refused: {seg}");
        }
    }

    open_inner(folder_handle, canonical_root, normalized)
}

#[cfg(target_os = "linux")]
fn open_inner(
    folder_handle: &PublishedFolderHandle,
    _canonical_root: &Path,
    normalized: &str,
) -> anyhow::Result<File> {
    use rustix::fs::{openat2, Mode, OFlags, ResolveFlags};
    use std::os::fd::AsFd;

    // Pin the resolution to the dirfd's subtree at the kernel level.
    let resolve = ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS;
    let oflags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW;

    let fd = openat2(
        folder_handle.as_fd(),
        normalized,
        oflags,
        Mode::empty(),
        resolve,
    )
    .map_err(|errno| match errno {
        rustix::io::Errno::XDEV => anyhow::anyhow!("path escapes published root"),
        rustix::io::Errno::LOOP => anyhow::anyhow!("path traverses a symlink"),
        rustix::io::Errno::NOENT => anyhow::anyhow!("file not found: {normalized}"),
        other => anyhow::anyhow!("openat2 failed: {other}"),
    })?;

    Ok(File::from(fd))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn open_inner(
    _folder_handle: &PublishedFolderHandle,
    canonical_root: &Path,
    normalized: &str,
) -> anyhow::Result<File> {
    // macOS / other unixes: realpath + prefix check + O_NOFOLLOW.
    //
    // We use the canonical_root path that publish stored at
    // open-folder time, NOT a `/dev/fd/<n>` reverse-lookup —
    // macOS does not expose a stable readable path for an open
    // dirfd. Acknowledged TOCTOU window: a symlink swap between
    // the realpath() check and the open() call could in principle
    // route the open at a target outside the root. Production
    // targets Linux; this fallback exists so dev on macOS works.
    use std::os::fd::FromRawFd;

    let candidate = canonical_root.join(normalized);
    let resolved =
        std::fs::canonicalize(&candidate).map_err(|e| anyhow::anyhow!("file not found: {e}"))?;
    if !resolved.starts_with(canonical_root) {
        anyhow::bail!("path escapes published root");
    }

    // Reject symlinks beneath the root. starts_with passed even if
    // `resolved` is the resolved target of a symlink whose link
    // body happens to live inside the root; symlink_metadata on
    // the original candidate detects the link.
    if std::fs::symlink_metadata(&candidate)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        anyhow::bail!("path traverses a symlink");
    }

    let cstr = std::ffi::CString::new(resolved.as_os_str().as_encoded_bytes())
        .map_err(|_| anyhow::anyhow!("path contains nul byte"))?;
    let raw = unsafe {
        libc::open(
            cstr.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if raw < 0 {
        let errno = std::io::Error::last_os_error();
        anyhow::bail!("open failed: {errno}");
    }
    Ok(unsafe { File::from_raw_fd(raw) })
}

#[cfg(not(unix))]
fn open_inner(
    _folder_handle: &PublishedFolderHandle,
    canonical_root: &Path,
    normalized: &str,
) -> anyhow::Result<File> {
    let candidate = canonical_root.join(normalized);
    let resolved =
        std::fs::canonicalize(&candidate).map_err(|e| anyhow::anyhow!("file not found: {e}"))?;
    if !resolved.starts_with(canonical_root) {
        anyhow::bail!("path escapes published root");
    }
    if std::fs::symlink_metadata(&candidate)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        anyhow::bail!("path traverses a symlink");
    }
    File::open(&resolved).map_err(|e| anyhow::anyhow!("open failed: {e}"))
}

/// Stat-then-validate the opened file: must be a regular file (not
/// a FIFO / socket / device / directory) and within `size_cap`.
pub fn validate_regular(file: &File, size_cap: u64) -> anyhow::Result<u64> {
    let meta = file
        .metadata()
        .map_err(|e| anyhow::anyhow!("stat failed: {e}"))?;
    if !meta.is_file() {
        anyhow::bail!("not a regular file");
    }
    let size = meta.len();
    if size > size_cap {
        anyhow::bail!("file size {size} exceeds cap {size_cap}");
    }
    Ok(size)
}

/// Open a directory by absolute path, returning an owned fd suitable
/// for storing inside a `ProjectHandle`. Used at publish time.
#[cfg(unix)]
pub fn open_directory(path: &Path) -> anyhow::Result<PublishedFolderHandle> {
    use std::ffi::CString;
    let cstr = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| anyhow::anyhow!("folder path contains nul byte"))?;
    let raw = unsafe {
        libc::open(
            cstr.as_ptr(),
            libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        let errno = std::io::Error::last_os_error();
        anyhow::bail!("open(O_DIRECTORY) failed for {}: {}", path.display(), errno);
    }
    use std::os::fd::FromRawFd;
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) })
}

#[cfg(not(unix))]
pub fn open_directory(path: &Path) -> anyhow::Result<PublishedFolderHandle> {
    Ok(path.to_path_buf())
}
