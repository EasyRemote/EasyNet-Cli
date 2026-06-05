use std::collections::HashMap;

use super::{DaemonError, Result};

/// Complete unary Invocation submitted through `DaemonClient`.
///
/// What this type is: an inspectable SDK record for the full Axon
/// Invocation tuple plus transport metadata. It can generate unary,
/// server-stream, and bidi frame-0 requests.
///
/// What this type is not: it is not a CLI-owned canonical Invocation
/// model. Canonical bytes, admission, signatures, and receipts remain
/// owned by Axon and the daemon Invocation transport.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct DaemonInvocation {
    caller_ura: String,
    callee_ura: String,
    ability: String,
    subject_ura: String,
    nonce: [u8; 16],
    causal_context: easynet_axon::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
    metadata: HashMap<String, String>,
    caller_signature: Option<easynet_axon::pb::axon::v1::CallerSignature>,
}

#[cfg(feature = "axon-pb")]
impl DaemonInvocation {
    /// Start building a complete Invocation. A fresh nonce is
    /// generated immediately so callers can inspect it before
    /// dispatch.
    pub fn builder(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        ability: impl Into<String>,
        subject_ura: impl Into<String>,
    ) -> Result<DaemonInvocationBuilder> {
        DaemonInvocationBuilder::new(caller_ura, callee_ura, ability, subject_ura)
    }

    /// Caller URA.
    pub fn caller_ura(&self) -> &str {
        &self.caller_ura
    }

    /// Callee URA.
    pub fn callee_ura(&self) -> &str {
        &self.callee_ura
    }

    /// Ability/function name.
    pub fn ability(&self) -> &str {
        &self.ability
    }

    /// Subject URA.
    pub fn subject_ura(&self) -> &str {
        &self.subject_ura
    }

    /// Invocation nonce.
    pub fn nonce(&self) -> [u8; 16] {
        self.nonce
    }

    /// Causal context carried in the request envelope.
    pub fn causal_context(&self) -> &easynet_axon::pb::axon::v1::CausalContext {
        &self.causal_context
    }

    /// Raw ability arguments.
    pub fn args(&self) -> &[u8] {
        &self.args
    }

    /// Request content type.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Non-axiom request metadata.
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    /// Optional caller signature carried on the envelope.
    pub fn caller_signature(&self) -> Option<&easynet_axon::pb::axon::v1::CallerSignature> {
        self.caller_signature.as_ref()
    }

    fn envelope(&self) -> easynet_axon::pb::axon::v1::Envelope {
        use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope, SubjectIdentity};
        Envelope {
            caller: Some(AgentIdentity {
                ura: self.caller_ura.clone(),
                profile: crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: self.callee_ura.clone(),
                profile: crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string(),
            }),
            subject: Some(SubjectIdentity {
                ura: self.subject_ura.clone(),
                profile: crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string(),
            }),
            invocation_nonce: self.nonce.to_vec(),
            causal_context: Some(self.causal_context.clone()),
            caller_signature: self.caller_signature.clone(),
            ..Envelope::default()
        }
    }

    fn content_envelope(&self) -> easynet_axon::pb::axon::v1::ContentEnvelope {
        easynet_axon::pb::axon::v1::ContentEnvelope {
            content_type: self.content_type.clone(),
            encoding: "identity".to_string(),
            ..easynet_axon::pb::axon::v1::ContentEnvelope::default()
        }
    }

    pub(crate) fn into_request(self) -> Result<easynet_axon::pb::axon::v1::InvokeRequest> {
        use easynet_axon::pb::axon::v1::InvokeRequest;
        let envelope = self.envelope();
        let content_envelope = self.content_envelope();
        Ok(InvokeRequest {
            envelope: Some(envelope),
            function_name: self.ability,
            arguments: self.args,
            content_type: self.content_type,
            metadata: self.metadata,
            content_envelope: Some(content_envelope),
            ..InvokeRequest::default()
        })
    }

    pub(crate) fn into_server_stream_request(
        self,
    ) -> Result<easynet_axon::pb::axon::v1::InvokeServerStreamRequest> {
        use easynet_axon::pb::axon::v1::InvokeServerStreamRequest;
        let envelope = self.envelope();
        let content_envelope = self.content_envelope();
        Ok(InvokeServerStreamRequest {
            envelope: Some(envelope),
            function_name: self.ability,
            arguments: self.args,
            content_type: self.content_type,
            metadata: self.metadata,
            content_envelope: Some(content_envelope),
            ..InvokeServerStreamRequest::default()
        })
    }

    pub(crate) fn into_bidi_open_frame(
        self,
        streams: Vec<easynet_axon::pb::axon::v1::StreamDescriptor>,
    ) -> Result<easynet_axon::pb::axon::v1::InvokeBidiUp> {
        use easynet_axon::pb::axon::v1::{
            invoke_bidi_up, EnvelopeOpen, InvocationTarget, InvokeBidiUp,
        };
        if streams.is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "bidi streams must not be empty".to_string(),
            ));
        }
        validate_bidi_streams(&streams)?;
        let envelope = self.envelope();
        let content_envelope = self.content_envelope();
        let mac = envelope
            .caller_signature
            .as_ref()
            .map(|sig| sig.signature.clone())
            .unwrap_or_default();
        Ok(InvokeBidiUp {
            sequence: 0,
            mac,
            payload: Some(invoke_bidi_up::Payload::EnvelopeOpen(EnvelopeOpen {
                envelope: Some(envelope),
                target: Some(InvocationTarget {
                    ability_name: self.ability,
                    ..InvocationTarget::default()
                }),
                initial_args: self.args,
                args_content_type: self.content_type.clone(),
                streams,
                metadata: self.metadata,
                content_envelope: Some(content_envelope),
            })),
        })
    }
}

/// Builder for `DaemonInvocation`.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct DaemonInvocationBuilder {
    caller_ura: String,
    callee_ura: String,
    ability: String,
    subject_ura: String,
    nonce: [u8; 16],
    causal_context: easynet_axon::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
    metadata: HashMap<String, String>,
    caller_signature: Option<easynet_axon::pb::axon::v1::CallerSignature>,
}

#[cfg(feature = "axon-pb")]
impl DaemonInvocationBuilder {
    fn new(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        ability: impl Into<String>,
        subject_ura: impl Into<String>,
    ) -> Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        let callee_ura = checked_ura(callee_ura.into(), "callee_ura")?;
        let subject_ura = checked_ura(subject_ura.into(), "subject_ura")?;
        let ability = ability.into();
        if ability.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "ability must not be empty".to_string(),
            ));
        }
        Ok(Self {
            caller_ura,
            callee_ura,
            ability: ability.trim().to_string(),
            subject_ura,
            nonce: easynet_axon::invocation::fresh_nonce(),
            causal_context: empty_causal_context(),
            args: Vec::new(),
            content_type: "application/json".to_string(),
            metadata: HashMap::new(),
            caller_signature: None,
        })
    }

    /// Override the generated nonce. Primarily for deterministic
    /// tests and receipt-chain replay fixtures.
    pub fn nonce(mut self, nonce: [u8; 16]) -> Self {
        self.nonce = nonce;
        self
    }

    /// Override the default root causal context.
    pub fn causal_context(
        mut self,
        causal_context: easynet_axon::pb::axon::v1::CausalContext,
    ) -> Self {
        self.causal_context = causal_context;
        self
    }

    /// Supply raw argument bytes and content type.
    pub fn args_bytes(
        mut self,
        args: impl Into<Vec<u8>>,
        content_type: impl Into<String>,
    ) -> Result<Self> {
        let content_type = content_type.into();
        if content_type.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "content_type must not be empty".to_string(),
            ));
        }
        self.args = args.into();
        self.content_type = content_type.trim().to_string();
        Ok(self)
    }

    /// Supply JSON arguments.
    pub fn args_json(mut self, value: &serde_json::Value) -> Result<Self> {
        self.args = serde_json::to_vec(value).map_err(DaemonError::EncodeArguments)?;
        self.content_type = "application/json".to_string();
        Ok(self)
    }

    /// Replace non-axiom request metadata. Metadata is transported
    /// on unary/server-stream requests and on bidi frame-0
    /// `EnvelopeOpen`; it is deliberately not part of canonical
    /// invocation bytes.
    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Attach a caller signature to the envelope. The daemon
    /// admission gate verifies this against Axon canonical bytes;
    /// this SDK surface only carries the already-produced signature.
    pub fn caller_signature(
        mut self,
        caller_signature: easynet_axon::pb::axon::v1::CallerSignature,
    ) -> Self {
        self.caller_signature = Some(caller_signature);
        self
    }

    /// Finish building the Invocation.
    pub fn build(self) -> DaemonInvocation {
        DaemonInvocation {
            caller_ura: self.caller_ura,
            callee_ura: self.callee_ura,
            ability: self.ability,
            subject_ura: self.subject_ura,
            nonce: self.nonce,
            causal_context: self.causal_context,
            args: self.args,
            content_type: self.content_type,
            metadata: self.metadata,
            caller_signature: self.caller_signature,
        }
    }
}

#[cfg(feature = "axon-pb")]
fn validate_bidi_streams(streams: &[easynet_axon::pb::axon::v1::StreamDescriptor]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    if streams.len() > 1 && streams.iter().any(|stream| stream.stream_id == 0) {
        return Err(DaemonError::InvalidInvocation(
            "bidi stream_id 0 is legal only for a single stream".to_string(),
        ));
    }
    for stream in streams {
        if stream.content_type.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "bidi stream content_type must not be empty".to_string(),
            ));
        }
        if stream.ordering.trim().is_empty() || stream.ordering != "STRICT" {
            return Err(DaemonError::InvalidInvocation(
                "bidi stream ordering must be STRICT".to_string(),
            ));
        }
        if !seen.insert(stream.stream_id) {
            return Err(DaemonError::InvalidInvocation(format!(
                "duplicate bidi stream_id {}",
                stream.stream_id
            )));
        }
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
fn checked_ura(value: String, field: &str) -> Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(DaemonError::InvalidInvocation(format!(
            "{field} must not be empty"
        )));
    }
    crate::ura::parse_ura(&value).map_err(|err| {
        DaemonError::InvalidInvocation(format!("{field} is not a valid URA: {err}"))
    })?;
    Ok(value)
}

#[cfg(feature = "axon-pb")]
fn empty_causal_context() -> easynet_axon::pb::axon::v1::CausalContext {
    use easynet_axon::pb::axon::v1::{causal_context, CausalContext, Empty};
    CausalContext {
        form: Some(causal_context::Form::None(Empty {})),
    }
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    #[test]
    fn invocation_builder_keeps_complete_tuple_inspectable() {
        let invocation = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/hub",
            "device.observe.health",
            "easynet:///r/acme/device/dev-a",
        )
        .unwrap()
        .nonce([0x42; 16])
        .args_json(&serde_json::json!({"ok": true}))
        .unwrap()
        .build();

        assert_eq!(invocation.caller_ura(), "easynet:///r/acme/device/dev-a");
        assert_eq!(invocation.callee_ura(), "easynet:///r/acme/hub");
        assert_eq!(invocation.ability(), "device.observe.health");
        assert_eq!(invocation.subject_ura(), "easynet:///r/acme/device/dev-a");
        assert_eq!(invocation.nonce(), [0x42; 16]);
        assert_eq!(invocation.content_type(), "application/json");
        assert!(!invocation.args().is_empty());
    }

    #[test]
    fn invocation_builder_emits_complete_stream_request() {
        let request = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/hub",
            "device.watch.health",
            "easynet:///r/acme/device/dev-a",
        )
        .unwrap()
        .nonce([0x24; 16])
        .args_json(&serde_json::json!({"interval_ms": 1000}))
        .unwrap()
        .build()
        .into_server_stream_request()
        .unwrap();

        let envelope = request
            .envelope
            .expect("stream request must carry envelope");
        assert_eq!(request.function_name, "device.watch.health");
        assert_eq!(request.content_type, "application/json");
        assert_eq!(request.arguments, br#"{"interval_ms":1000}"#);
        assert_eq!(envelope.invocation_nonce, vec![0x24; 16]);
        assert_eq!(
            envelope.caller.expect("caller required").ura,
            "easynet:///r/acme/device/dev-a"
        );
        assert_eq!(
            envelope.callee.expect("callee required").ura,
            "easynet:///r/acme/hub"
        );
        assert_eq!(
            envelope.subject.expect("subject required").ura,
            "easynet:///r/acme/device/dev-a"
        );
        assert!(
            envelope.causal_context.is_some(),
            "stream request must carry causal context"
        );
    }

    #[test]
    fn invocation_builder_emits_complete_bidi_frame0() {
        use easynet_axon::pb::axon::v1::{invoke_bidi_up, CallerSignature, StreamDescriptor};
        let mut metadata = HashMap::new();
        metadata.insert("x-easynet-delegation".to_string(), "producer".to_string());

        let frame = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/hub",
            "device.pty.attach",
            "easynet:///r/acme/device/dev-a",
        )
        .unwrap()
        .nonce([0x33; 16])
        .args_json(&serde_json::json!({"session_id": "pty-1"}))
        .unwrap()
        .metadata(metadata)
        .caller_signature(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: vec![7; 64],
            key_id_hint: "caller-key".to_string(),
        })
        .build()
        .into_bidi_open_frame(vec![StreamDescriptor {
            stream_id: 1,
            content_type: "text/pty".to_string(),
            codec_params: "raw".to_string(),
            ordering: "STRICT".to_string(),
        }])
        .unwrap();

        assert_eq!(frame.sequence, 0);
        assert_eq!(frame.mac, vec![7; 64]);
        let invoke_bidi_up::Payload::EnvelopeOpen(open) =
            frame.payload.expect("frame0 must be EnvelopeOpen")
        else {
            panic!("frame0 must carry EnvelopeOpen");
        };
        let envelope = open.envelope.expect("EnvelopeOpen must carry envelope");
        assert_eq!(
            envelope.caller.expect("caller required").ura,
            "easynet:///r/acme/device/dev-a"
        );
        assert_eq!(
            open.target.expect("target required").ability_name,
            "device.pty.attach"
        );
        assert_eq!(open.initial_args, br#"{"session_id":"pty-1"}"#);
        assert_eq!(open.args_content_type, "application/json");
        assert_eq!(open.metadata["x-easynet-delegation"], "producer");
        assert_eq!(open.streams.len(), 1);
        assert_eq!(open.streams[0].stream_id, 1);
        assert_eq!(
            open.content_envelope
                .expect("content envelope required")
                .encoding,
            "identity"
        );
    }

    #[test]
    fn invocation_builder_rejects_ambiguous_bidi_stream_zero() {
        use easynet_axon::pb::axon::v1::StreamDescriptor;
        let err = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/hub",
            "device.pty.attach",
            "easynet:///r/acme/device/dev-a",
        )
        .unwrap()
        .build()
        .into_bidi_open_frame(vec![
            StreamDescriptor {
                stream_id: 0,
                content_type: "text/pty".to_string(),
                ordering: "STRICT".to_string(),
                ..StreamDescriptor::default()
            },
            StreamDescriptor {
                stream_id: 2,
                content_type: "application/json".to_string(),
                ordering: "STRICT".to_string(),
                ..StreamDescriptor::default()
            },
        ])
        .unwrap_err();

        assert!(format!("{err}").contains("stream_id 0"));
    }

    #[test]
    fn invocation_builder_rejects_invalid_ura() {
        let err = DaemonInvocation::builder(
            "not-a-ura",
            "easynet:///r/acme/hub",
            "x",
            "easynet:///r/acme/hub",
        )
        .unwrap_err();
        assert!(format!("{err}").contains("caller_ura"));
    }
}
