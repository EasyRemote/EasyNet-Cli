// EasyNet CLI — remote desktop session identity
// =============================================
//
// File: src/plugins/builtin/remote_desktop/session_identity.rs
// Description: Immutable identity and caller-visible policy captured at session creation.

use crate::persistence::resources::ResourceType;
use crate::plugins::remote_desktop::request::{
    RemoteDesktopInputPolicy, RemoteDesktopVideoConstraints,
};
use crate::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;

/// Construction payload for a remote desktop session row.
///
/// This keeps the session's required identity, lease, media policy, and
/// event-channel fields together so callers cannot create half-initialized
/// session state. What this is NOT: live transport or signaling state; those
/// are established only after the session row exists.
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopSessionInit {
    pub(in crate::plugins::builtin::remote_desktop) session_id: String,
    pub(in crate::plugins::builtin::remote_desktop) session_token: String,
    pub(in crate::plugins::builtin::remote_desktop) creator_caller_ura: Option<String>,
    pub(in crate::plugins::builtin::remote_desktop) consent: RemoteDesktopConsentGrant,
    pub(in crate::plugins::builtin::remote_desktop) subject_ura: String,
    pub(in crate::plugins::builtin::remote_desktop) subject_type: ResourceType,
    pub(in crate::plugins::builtin::remote_desktop) subject_display_name: String,
    pub(in crate::plugins::builtin::remote_desktop) mode: String,
    pub(in crate::plugins::builtin::remote_desktop) lease_ttl_ms: u64,
    pub(in crate::plugins::builtin::remote_desktop) transport_preferences: Vec<String>,
    pub(in crate::plugins::builtin::remote_desktop) video: RemoteDesktopVideoConstraints,
    pub(in crate::plugins::builtin::remote_desktop) input_policy: RemoteDesktopInputPolicy,
}

/// Immutable session profile captured before transport negotiation starts.
///
/// Invariant 1: every field in this profile is creation-time metadata and must
/// not be mutated by signaling, media, input, or lease operations.
/// Invariant 2: the bearer token is never exposed through a general accessor;
/// callers can only compare it or fetch it for the create-session response.
/// Invariant 3: when a caller URA is present, subsequent control and
/// data-plane calls must present the same caller; the token alone is not a
/// reusable authorization object.
/// Invariant 4: the local-user consent grant is captured once at creation and
/// cannot be rewritten by signaling or media paths.
#[derive(Debug, Clone)]
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopSessionProfile {
    session_id: String,
    session_token: String,
    creator_caller_ura: Option<String>,
    consent: RemoteDesktopConsentGrant,
    subject_ura: String,
    subject_type: ResourceType,
    subject_display_name: String,
    mode: String,
    transport_preferences: Vec<String>,
    video: RemoteDesktopVideoConstraints,
    input_policy: RemoteDesktopInputPolicy,
}

impl RemoteDesktopSessionProfile {
    /// Build the immutable profile and return the requested initial lease TTL.
    pub(in crate::plugins::builtin::remote_desktop) fn from_init(
        init: RemoteDesktopSessionInit,
    ) -> (Self, u64) {
        let profile = Self {
            session_id: init.session_id,
            session_token: init.session_token,
            creator_caller_ura: init.creator_caller_ura,
            consent: init.consent,
            subject_ura: init.subject_ura,
            subject_type: init.subject_type,
            subject_display_name: init.subject_display_name,
            mode: init.mode,
            transport_preferences: init.transport_preferences,
            video: init.video,
            input_policy: init.input_policy,
        };
        (profile, init.lease_ttl_ms)
    }

    /// Stable opaque identifier for this remote desktop session.
    pub(in crate::plugins::builtin::remote_desktop) fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Return whether the caller supplied the session's bearer token.
    pub(in crate::plugins::builtin::remote_desktop) fn matches_session_token(
        &self,
        token: &str,
    ) -> bool {
        self.session_token == token
    }

    /// Opaque token returned only by create-session responses.
    pub(in crate::plugins::builtin::remote_desktop) fn session_token_for_create_response(
        &self,
    ) -> &str {
        &self.session_token
    }

    /// Authenticated creator caller captured from the Axon envelope, when
    /// the invocation path supplied one.
    pub(in crate::plugins::builtin::remote_desktop) fn creator_caller_ura(&self) -> Option<&str> {
        self.creator_caller_ura.as_deref()
    }

    /// Immutable local-user consent grant captured at session creation.
    pub(in crate::plugins::builtin::remote_desktop) fn consent(
        &self,
    ) -> &RemoteDesktopConsentGrant {
        &self.consent
    }

    /// Canonical resource URA that this session is allowed to operate on.
    pub(in crate::plugins::builtin::remote_desktop) fn subject_ura(&self) -> &str {
        &self.subject_ura
    }

    /// Resource type captured at session creation.
    pub(in crate::plugins::builtin::remote_desktop) fn subject_type(&self) -> ResourceType {
        self.subject_type
    }

    /// Human-facing display name for the acted-on resource.
    pub(in crate::plugins::builtin::remote_desktop) fn subject_display_name(&self) -> &str {
        &self.subject_display_name
    }

    /// Requested session mode.
    pub(in crate::plugins::builtin::remote_desktop) fn mode(&self) -> &str {
        &self.mode
    }

    /// Ordered transport preference list captured at session creation.
    pub(in crate::plugins::builtin::remote_desktop) fn transport_preferences(&self) -> &[String] {
        &self.transport_preferences
    }

    /// Video constraints captured at session creation.
    pub(in crate::plugins::builtin::remote_desktop) fn video(
        &self,
    ) -> &RemoteDesktopVideoConstraints {
        &self.video
    }

    /// Input policy captured at session creation.
    pub(in crate::plugins::builtin::remote_desktop) fn input_policy(
        &self,
    ) -> &RemoteDesktopInputPolicy {
        &self.input_policy
    }
}
