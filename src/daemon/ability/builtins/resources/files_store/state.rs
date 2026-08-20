// EasyNet CLI — Files: storage root resolution
// =============================================
//
// Content-addressed blob storage rooted at:
//   $EASYNET_FILES_ROOT (default ~/.easynet/files)
//
// Per-blob paths:
//   <root>/<sha256-hex>
//   <root>/<sha256-hex>.metadata.json
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};

/// Default subpath under HOME when EASYNET_FILES_ROOT is unset.
const DEFAULT_REL: &str = ".easynet/files";

/// Resolve the absolute root directory where content-addressed
/// blobs are stored. Boot-time env-read; lift the result into the
/// `FilesConfig` so handlers can run parallel-safe tests without
/// touching `EASYNET_FILES_ROOT` mid-process.
pub fn root_from_env() -> std::io::Result<PathBuf> {
    let p = match std::env::var("EASYNET_FILES_ROOT") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => {
            let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "HOME unset and EASYNET_FILES_ROOT not provided",
                )
            })?;
            home.join(DEFAULT_REL)
        }
    };
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

/// Ensure a caller-supplied root exists; convenience for tests.
pub fn ensure_root(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)
}

/// Path of a blob inside `root` given its sha256-hex.
pub fn blob_path(root: &Path, sha256_hex: &str) -> std::io::Result<PathBuf> {
    if sha256_hex.len() != 64 || !sha256_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sha256 must be 64 hex chars",
        ));
    }
    Ok(root.join(sha256_hex))
}

/// Path of the producer-supplied immutable metadata for a blob.
pub fn metadata_path(root: &Path, sha256_hex: &str) -> std::io::Result<PathBuf> {
    if sha256_hex.len() != 64 || !sha256_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sha256 must be 64 hex chars",
        ));
    }
    Ok(root.join(format!("{sha256_hex}.metadata.json")))
}

/// v4.1.5 URA for a blob owned by `user`.
pub fn blob_ura(realm: &str, user: &str, sha256_hex: &str) -> String {
    crate::core::ura::resource_dot_ura(realm, &format!("{user}.files"), sha256_hex)
}
