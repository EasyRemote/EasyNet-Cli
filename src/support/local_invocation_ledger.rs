// EasyNet CLI — Local invocation ledger reader
// ============================================
//
// File: src/support/local_invocation_ledger.rs
// Description: Side-effect-free reads from the daemon-owned invocation ledger.
//
// This module is deliberately not an Ability client. CLI support code uses it
// only after a local daemon Invoke has already returned a request_id and needs
// to observe the ledger projection for that same request. Calling
// `invocation.history.get` for that read would create a second invocation and
// corrupt the audit trail the caller is trying to inspect.

use std::path::PathBuf;

use anyhow::Context;
use serde_json::Value;

use crate::persistence::daemon_config::{default_config_path, default_ledger_dir, DaemonConfig};

/// Read-only local daemon invocation ledger view.
///
/// Invariant 1: every method is side-effect-free; this type never invokes an
/// ability and never writes the ledger.
/// Invariant 2: lookup keys are Axon ledger keys, not CLI display aliases.
/// Invariant 3: absence of the ledger file means "no record yet", not an
/// operator error, because the daemon sink may persist asynchronously after the
/// unary response returns.
#[derive(Debug, Clone)]
pub(crate) struct LocalInvocationLedgerReader {
    path: PathBuf,
}

impl LocalInvocationLedgerReader {
    pub(crate) fn from_default_config() -> Self {
        Self {
            path: ledger_path_from_config(),
        }
    }

    pub(crate) fn record_by_request_id(&self, request_id: &str) -> anyhow::Result<Option<Value>> {
        let request_id = request_id.trim();
        if request_id.is_empty() {
            anyhow::bail!("request_id must not be empty");
        }
        if !self.path.exists() {
            return Ok(None);
        }
        let query = easynet_axon::invocation::InvocationLedgerQuery::new()
            .key(
                easynet_axon::invocation::InvocationLedgerFetchKey::RequestId(
                    request_id.to_string(),
                ),
            )
            .limit(1);
        let ledger = easynet_axon::invocation::InvocationLedger::open(&self.path)
            .with_context(|| format!("open invocation ledger at {}", self.path.display()))?;
        let Some(record) = ledger.fetch_one(query)? else {
            return Ok(None);
        };
        serde_json::to_value(record)
            .map(Some)
            .context("serialize invocation ledger record")
    }
}

fn ledger_path_from_config() -> PathBuf {
    DaemonConfig::load(&default_config_path())
        .map(|cfg| cfg.ledger_dir().join("invocations.redb"))
        .unwrap_or_else(|_| default_ledger_dir().join("invocations.redb"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_ledger_is_empty_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let reader = LocalInvocationLedgerReader {
            path: dir.path().join("missing.redb"),
        };

        let record = reader
            .record_by_request_id("req-missing")
            .expect("missing ledger is not an error");
        assert!(record.is_none());
    }

    #[test]
    fn empty_request_id_is_rejected_before_disk_read() {
        let reader = LocalInvocationLedgerReader {
            path: PathBuf::from("/path/that/does/not/matter"),
        };

        let err = reader.record_by_request_id(" ").unwrap_err();
        assert!(err.to_string().contains("request_id"));
    }
}
