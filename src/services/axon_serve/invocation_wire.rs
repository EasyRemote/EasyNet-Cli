// EasyNet CLI — Axon invocation wire builders
// ===========================================
//
// File: src/services/axon_serve/invocation_wire.rs
// Description: Typed construction boundary for proto InvokeRequest
//              and Envelope values emitted by the CLI/daemon.
//
// Protocol Responsibility
// -----------------------
// This module owns the outbound proto envelope shape. Callers supply
// domain URAs and JSON bytes; this builder validates URA grammar,
// installs the default URA profile, and generates a replay nonce for
// complete envelopes.
//
// Implementation Approach
// -----------------------
// Keep the API deliberately small:
//   * `caller_only` for genesis/prelude calls that are admitted by a
//     special path before the full AXIOM tuple is available.
//   * `loopback` for local daemon/hub self-calls where caller,
//     callee, and subject are the same URA.
//   * `targeted` for normal caller → callee with explicit subject.
//
// Usage Contract
// --------------
// Production call sites should not hand-build `Envelope` /
// `InvokeRequest` struct literals. Tests may still construct raw
// proto fixtures when they intentionally exercise malformed shapes.
//
// Architectural Position
// ----------------------
// This is the wire-facade counterpart to `crate::ura` (canonical URA
// construction/parsing) and `runtime::invocation` (domain invocation
// records). It does not perform admission or signing.

use rand::RngCore;

use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest, SubjectIdentity};

pub const DEFAULT_URA_PROFILE: &str = "easynet-strict-v2";

#[derive(Debug, Clone)]
pub struct ProtoEnvelope {
    inner: Envelope,
}

impl ProtoEnvelope {
    pub fn caller_only(caller_ura: impl Into<String>) -> anyhow::Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        Ok(Self {
            inner: Envelope {
                caller: Some(agent_identity(caller_ura)),
                ..Envelope::default()
            },
        })
    }

    pub fn loopback(ura: impl Into<String>) -> anyhow::Result<Self> {
        let ura = checked_ura(ura.into(), "loopback_ura")?;
        Self::targeted(ura.clone(), ura.clone(), ura)
    }

    pub fn targeted(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        subject_ura: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        let callee_ura = checked_ura(callee_ura.into(), "callee_ura")?;
        let subject_ura = checked_ura(subject_ura.into(), "subject_ura")?;
        Ok(Self {
            inner: Envelope {
                caller: Some(agent_identity(caller_ura)),
                callee: Some(agent_identity(callee_ura)),
                subject: Some(subject_identity(subject_ura)),
                invocation_nonce: fresh_invocation_nonce().to_vec(),
                ..Envelope::default()
            },
        })
    }

    #[must_use]
    pub fn into_inner(self) -> Envelope {
        self.inner
    }

    pub fn invoke_request(
        self,
        function_name: impl Into<String>,
        arguments: Vec<u8>,
    ) -> anyhow::Result<InvokeRequest> {
        let function_name = function_name.into();
        if function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        Ok(InvokeRequest {
            envelope: Some(self.into_inner()),
            function_name,
            arguments,
            ..InvokeRequest::default()
        })
    }
}

fn checked_ura(ura: String, field: &str) -> anyhow::Result<String> {
    let ura = ura.trim().to_string();
    if ura.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    crate::ura::parse_ura(&ura).map_err(|e| anyhow::anyhow!("{field} is not a valid URA: {e}"))?;
    Ok(ura)
}

fn agent_identity(ura: String) -> AgentIdentity {
    AgentIdentity {
        ura,
        profile: DEFAULT_URA_PROFILE.to_string(),
    }
}

fn subject_identity(ura: String) -> SubjectIdentity {
    SubjectIdentity {
        ura,
        profile: DEFAULT_URA_PROFILE.to_string(),
    }
}

fn fresh_invocation_nonce() -> [u8; 16] {
    let mut nonce = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targeted_envelope_has_full_tuple_and_nonce() {
        let env = ProtoEnvelope::targeted(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/hub",
            "easynet:///r/acme/user/alice",
        )
        .unwrap()
        .into_inner();
        assert_eq!(env.caller.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(env.callee.unwrap().ura, "easynet:///r/acme/hub");
        assert_eq!(env.subject.unwrap().ura, "easynet:///r/acme/user/alice");
        assert_eq!(env.invocation_nonce.len(), 16);
    }

    #[test]
    fn loopback_sets_caller_callee_subject_to_same_ura() {
        let req = ProtoEnvelope::loopback("easynet:///r/acme/device/dev-a")
            .unwrap()
            .invoke_request("federation.discover", b"{}".to_vec())
            .unwrap();
        let env = req.envelope.unwrap();
        assert_eq!(env.caller.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(env.callee.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(env.subject.unwrap().ura, "easynet:///r/acme/device/dev-a");
        assert_eq!(req.function_name, "federation.discover");
    }

    #[test]
    fn caller_only_keeps_tuple_incomplete_for_genesis_preludes() {
        let env = ProtoEnvelope::caller_only("easynet:///r/acme/device/dev-a")
            .unwrap()
            .into_inner();
        assert!(env.caller.is_some());
        assert!(env.callee.is_none());
        assert!(env.subject.is_none());
    }

    #[test]
    fn invalid_ura_is_rejected_before_wire_send() {
        let err = ProtoEnvelope::loopback("agent://self").unwrap_err();
        assert!(format!("{err}").contains("valid URA"));
    }
}
