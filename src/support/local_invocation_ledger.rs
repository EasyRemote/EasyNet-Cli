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
use std::sync::{Arc, RwLock};

use anyhow::Context;
use once_cell::sync::Lazy;
use serde_json::Value;

use crate::persistence::daemon_config::{default_config_path, default_ledger_dir, DaemonConfig};

static PROCESS_LEDGER: Lazy<RwLock<Option<Arc<easynet_axon::invocation::InvocationLedger>>>> =
    Lazy::new(|| RwLock::new(None));

pub(crate) fn register_process_ledger(ledger: Arc<easynet_axon::invocation::InvocationLedger>) {
    if let Ok(mut slot) = PROCESS_LEDGER.write() {
        *slot = Some(ledger);
    }
}

fn process_ledger() -> Option<Arc<easynet_axon::invocation::InvocationLedger>> {
    PROCESS_LEDGER.read().ok().and_then(|slot| slot.clone())
}

#[derive(Clone)]
enum LocalInvocationLedgerSource {
    Shared(Arc<easynet_axon::invocation::InvocationLedger>),
    Path(PathBuf),
}

/// Read-only local daemon invocation ledger view.
///
/// Invariant 1: every method is side-effect-free; this type never invokes an
/// ability and never writes the ledger.
/// Invariant 2: lookup keys are Axon ledger keys, not CLI display aliases.
/// Invariant 3: absence of the ledger file means "no record yet", not an
/// operator error, because the daemon sink may persist asynchronously after the
/// unary response returns.
#[derive(Clone)]
pub(crate) struct LocalInvocationLedgerReader {
    source: LocalInvocationLedgerSource,
}

impl LocalInvocationLedgerReader {
    pub(crate) fn from_default_config() -> Self {
        if let Some(ledger) = process_ledger() {
            return Self {
                source: LocalInvocationLedgerSource::Shared(ledger),
            };
        }
        Self {
            source: LocalInvocationLedgerSource::Path(ledger_path_from_config()),
        }
    }

    #[cfg(test)]
    fn from_path(path: PathBuf) -> Self {
        Self {
            source: LocalInvocationLedgerSource::Path(path),
        }
    }

    pub(crate) fn record_by_request_id(&self, request_id: &str) -> anyhow::Result<Option<Value>> {
        let request_id = request_id.trim();
        if request_id.is_empty() {
            anyhow::bail!("request_id must not be empty");
        }
        let query = easynet_axon::invocation::InvocationLedgerQuery::new()
            .key(
                easynet_axon::invocation::InvocationLedgerFetchKey::RequestId(
                    request_id.to_string(),
                ),
            )
            .limit(1);
        let record = match &self.source {
            LocalInvocationLedgerSource::Shared(ledger) => ledger.fetch_one(query)?,
            LocalInvocationLedgerSource::Path(path) => {
                if !path.exists() {
                    return Ok(None);
                }
                let ledger = easynet_axon::invocation::InvocationLedger::open(path)
                    .with_context(|| format!("open invocation ledger at {}", path.display()))?;
                ledger.fetch_one(query)?
            }
        };
        let Some(record) = record else {
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
        let reader = LocalInvocationLedgerReader::from_path(dir.path().join("missing.redb"));

        let record = reader
            .record_by_request_id("req-missing")
            .expect("missing ledger is not an error");
        assert!(record.is_none());
    }

    #[test]
    fn empty_request_id_is_rejected_before_disk_read() {
        let reader =
            LocalInvocationLedgerReader::from_path(PathBuf::from("/path/that/does/not/matter"));

        let err = reader.record_by_request_id(" ").unwrap_err();
        assert!(err.to_string().contains("request_id"));
    }
}
