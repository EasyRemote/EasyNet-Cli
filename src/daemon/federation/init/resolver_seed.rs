// EasyNet CLI — Shard resolver seed loader
// ===========================================
//
// File: src/daemon/federation/init/resolver_seed.rs
//
// Loads `~/.config/easynet/shards.json` (operator-curated realm
// → shard endpoint map) into a `ResolverSeed` value the daemon
// passes to axon-runtime via `install_shard_resolver`. Strictly
// optional — when the file is absent, federation runs in single-
// shard mode (every realm is local from this hub's perspective),
// which is the right default for v1 deployments.
//
// File schema is intentionally minimal — just `realms`, mapping
// realm FQDN to a list of remote shard endpoint URLs:
//
// ```json
// {
//   "realms": {
//     "alice.easynet": ["http://alice-shard.example:7700"],
//     "acme.com":      ["http://acme-east.example:7700",
//                       "http://acme-west.example:7700"]
//   }
// }
// ```
//
// The local shard's own realms are NOT listed here — they're
// derived from the daemon's own credentials.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const SHARDS_FILE_NAME: &str = "shards.json";

/// On-disk schema for `~/.config/easynet/shards.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ShardsFile {
    /// `realm_fqdn → [endpoint, ...]`. Endpoint priority is the
    /// order given here — the dispatcher tries the first reachable.
    #[serde(default)]
    pub realms: HashMap<String, Vec<String>>,
}

/// Result of loading the shards file. The seed is what the
/// daemon hands to axon-runtime. Loading errors are surfaced as
/// typed variants so the caller (daemon boot) can decide
/// whether to fail-soft (default) or fail-hard (operator
/// override via env).
#[derive(Debug, Clone, PartialEq)]
pub enum ResolverSeed {
    /// File absent — single-shard mode is the safe default.
    Absent { path: PathBuf },
    /// File loaded; n realms mapped.
    Loaded {
        path: PathBuf,
        realm_count: usize,
        seed: ShardsFile,
    },
    /// File present but malformed. Operator must fix.
    Malformed { path: PathBuf, reason: String },
}

impl ResolverSeed {
    /// True when this seed contributes routes (Loaded with at
    /// least one realm).
    pub fn has_routes(&self) -> bool {
        matches!(self, Self::Loaded { realm_count, .. } if *realm_count > 0)
    }

    pub fn realms(&self) -> &HashMap<String, Vec<String>> {
        static EMPTY: std::sync::OnceLock<HashMap<String, Vec<String>>> =
            std::sync::OnceLock::new();
        match self {
            Self::Loaded { seed, .. } => &seed.realms,
            _ => EMPTY.get_or_init(HashMap::new),
        }
    }
}

/// Load from the given path. Returns `Absent` when the file does
/// not exist; `Malformed` on parse error; `Loaded` on success.
pub fn load_from(path: impl AsRef<Path>) -> ResolverSeed {
    let path = path.as_ref().to_path_buf();
    if !path.exists() {
        return ResolverSeed::Absent { path };
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            return ResolverSeed::Malformed {
                path: path.clone(),
                reason: format!("read {}: {e}", path.display()),
            };
        }
    };
    let parsed: ShardsFile = match serde_json::from_slice(&bytes) {
        Ok(f) => f,
        Err(e) => {
            return ResolverSeed::Malformed {
                path,
                reason: format!("parse: {e}"),
            };
        }
    };
    let realm_count = parsed.realms.len();
    ResolverSeed::Loaded {
        path,
        realm_count,
        seed: parsed,
    }
}

/// Default loader: locates the file at `$XDG_CONFIG_HOME/easynet/shards.json`
/// (or platform fallback). Returns `Absent` with the canonical
/// path when no config dir exists either.
///
/// Cold path by design (F-050 classification): a one-shot boot-time
/// config read, not a catalog query — the snapshot-read rule does not
/// apply, direct disk read stays.
pub fn load_default() -> ResolverSeed {
    let path = match dirs::config_dir() {
        Some(d) => d.join("easynet").join(SHARDS_FILE_NAME),
        None => {
            return ResolverSeed::Absent {
                path: PathBuf::from(SHARDS_FILE_NAME),
            };
        }
    };
    load_from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn absent_when_path_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("does-not-exist.json");
        let s = load_from(&p);
        assert!(matches!(s, ResolverSeed::Absent { .. }));
        assert!(!s.has_routes());
        assert!(s.realms().is_empty());
    }

    #[test]
    fn loaded_round_trips_minimal_schema() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(SHARDS_FILE_NAME);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(
            br#"{
                "realms": {
                  "alice.easynet": ["http://alice-shard.example:7700"],
                  "acme.com":      ["http://acme-east.example:7700",
                                    "http://acme-west.example:7700"]
                }
              }"#,
        )
        .unwrap();
        let s = load_from(&p);
        match s {
            ResolverSeed::Loaded {
                realm_count, seed, ..
            } => {
                assert_eq!(realm_count, 2);
                assert_eq!(
                    seed.realms.get("alice.easynet").unwrap(),
                    &vec!["http://alice-shard.example:7700".to_string()]
                );
                assert_eq!(seed.realms.get("acme.com").unwrap().len(), 2);
            }
            other => panic!("expected Loaded, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(SHARDS_FILE_NAME);
        std::fs::write(&p, b"{ this is not json").unwrap();
        let s = load_from(&p);
        match s {
            ResolverSeed::Malformed { reason, .. } => {
                assert!(reason.contains("parse"), "{reason}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn empty_realms_is_loaded_but_has_no_routes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(SHARDS_FILE_NAME);
        std::fs::write(&p, br#"{"realms":{}}"#).unwrap();
        let s = load_from(&p);
        assert!(matches!(s, ResolverSeed::Loaded { realm_count: 0, .. }));
        assert!(!s.has_routes());
    }

    #[test]
    fn has_routes_only_for_loaded_with_entries() {
        let with = ResolverSeed::Loaded {
            path: PathBuf::from("/x"),
            realm_count: 3,
            seed: ShardsFile {
                realms: HashMap::from([("a".into(), vec!["b".into()])]),
            },
        };
        assert!(with.has_routes());
        let absent = ResolverSeed::Absent {
            path: PathBuf::from("/x"),
        };
        assert!(!absent.has_routes());
    }
}
