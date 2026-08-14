// EasyNet CLI — remote desktop session creation workflow
// =======================================================
//
// File: plugins/remote-desktop/src/session_creation.rs
// Description: Pre-row creation workflow for targeted remote desktop sessions.
//
// Architectural Boundary:
// - This workflow owns pre-session lifecycle states.
// - The active session aggregate is constructed only after consent and target
//   binding have both succeeded.
// - No unresolved or pre-binding workflow state is inserted into
//   RemoteDesktopSessionStore.

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::persistence::resources::ResourceEntry;
use crate::daemon::plugins::remote_desktop::consent_registry::{
    RemoteDesktopConsentRegistry, CONSENT_INTENT,
};
use crate::daemon::plugins::remote_desktop::constants::ABILITY_CREATE_SESSION;
use crate::daemon::plugins::remote_desktop::request::{
    mint_session_id, mint_session_token, parse_input_policy, parse_lease_ttl_ms, parse_mode,
    parse_optional_session_id, parse_transport_preferences, parse_video_constraints, require_str,
    RemoteDesktopInputPolicy, RemoteDesktopVideoConstraints,
};
use crate::daemon::plugins::remote_desktop::resource::resolve_screen_resource_from_envelope;
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSessionInit;
use crate::daemon::plugins::remote_desktop::session_consent::RemoteDesktopConsentGrant;
use crate::daemon::plugins::remote_desktop::target::{
    verify_target_binding_for_session, RemoteAppTargetBinding, RemoteAppTargetError,
    RemoteAppTargetResolver, ResolvedCaptureTargetProof, ResourceEntryTargetResolver,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteDesktopSessionCreationState {
    ValidatingSubject,
    AwaitingConsent,
    ResolvingTarget,
    ReadyToInsert,
}

/// Pre-row workflow for `remote_desktop.create_session`.
///
/// The workflow deliberately owns the values needed to build
/// `RemoteDesktopSessionInit`; the handler can only insert a session after
/// `into_session_init` succeeds.
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSessionCreationWorkflow {
    state: RemoteDesktopSessionCreationState,
    caller_ura: String,
    entry: ResourceEntry,
    session_id: String,
    session_token: String,
    mode: String,
    lease_ttl_ms: u64,
    transport_preferences: Vec<String>,
    video: RemoteDesktopVideoConstraints,
    input_policy: RemoteDesktopInputPolicy,
    consent_ticket: String,
    consent: Option<RemoteDesktopConsentGrant>,
    target_binding: Option<RemoteAppTargetBinding>,
}

pub(in crate::daemon::plugins::remote_desktop) trait RemoteAppTargetBindingVerifier {
    fn verify_for_session(
        &self,
        ability: &'static str,
        binding: &RemoteAppTargetBinding,
    ) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError>;
}

pub(in crate::daemon::plugins::remote_desktop) struct PlatformRemoteAppTargetBindingVerifier;

impl RemoteAppTargetBindingVerifier for PlatformRemoteAppTargetBindingVerifier {
    fn verify_for_session(
        &self,
        ability: &'static str,
        binding: &RemoteAppTargetBinding,
    ) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
        verify_target_binding_for_session(ability, binding)
    }
}

impl RemoteDesktopSessionCreationWorkflow {
    pub(in crate::daemon::plugins::remote_desktop) fn start(
        env: &EnvelopeContext,
        args: &serde_json::Value,
    ) -> anyhow::Result<Self> {
        let state = RemoteDesktopSessionCreationState::ValidatingSubject;
        let entry = resolve_screen_resource_from_envelope(ABILITY_CREATE_SESSION, env, args)?;
        let mode = parse_mode(args)?;
        let lease_ttl_ms = parse_lease_ttl_ms(args)?;
        let transport_preferences = parse_transport_preferences(args)?;
        let video = parse_video_constraints(args)?;
        let input_policy = parse_input_policy(args, &mode)?;
        let session_id = parse_optional_session_id(args)?.unwrap_or_else(mint_session_id);
        let session_token = mint_session_token();
        let consent_ticket =
            require_str(args, "consent_ticket", ABILITY_CREATE_SESSION)?.to_string();
        let mut workflow = Self {
            state,
            caller_ura: env.caller().to_string(),
            entry,
            session_id,
            session_token,
            mode,
            lease_ttl_ms,
            transport_preferences,
            video,
            input_policy,
            consent_ticket,
            consent: None,
            target_binding: None,
        };
        workflow.state = RemoteDesktopSessionCreationState::AwaitingConsent;
        Ok(workflow)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn consume_consent(
        mut self,
        registry: &RemoteDesktopConsentRegistry,
        env: &EnvelopeContext,
    ) -> anyhow::Result<Self> {
        self.assert_state(RemoteDesktopSessionCreationState::AwaitingConsent);
        let authorization = registry.consume(
            &self.consent_ticket,
            &self.caller_ura,
            &self.entry.resource_ura,
            CONSENT_INTENT,
        )?;
        self.consent = Some(RemoteDesktopConsentGrant::required_from_envelope(
            ABILITY_CREATE_SESSION,
            &self.session_id,
            env,
            authorization,
        )?);
        self.state = RemoteDesktopSessionCreationState::ResolvingTarget;
        Ok(self)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn resolve_target(self) -> anyhow::Result<Self> {
        self.resolve_target_with_verifier(&PlatformRemoteAppTargetBindingVerifier)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn resolve_target_with_verifier(
        mut self,
        verifier: &impl RemoteAppTargetBindingVerifier,
    ) -> anyhow::Result<Self> {
        self.assert_state(RemoteDesktopSessionCreationState::ResolvingTarget);
        let mut target_binding = ResourceEntryTargetResolver.resolve_for_session(
            ABILITY_CREATE_SESSION,
            &self.entry,
            &self.mode,
            1,
        )?;
        let capture_proof = verifier.verify_for_session(ABILITY_CREATE_SESSION, &target_binding)?;
        target_binding.commit_capture_proof(ABILITY_CREATE_SESSION, capture_proof)?;
        self.input_policy = self.input_policy.constrained_for_binding(&target_binding);
        self.target_binding = Some(target_binding);
        self.state = RemoteDesktopSessionCreationState::ReadyToInsert;
        Ok(self)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn into_session_init(
        self,
    ) -> RemoteDesktopSessionInit {
        self.assert_state(RemoteDesktopSessionCreationState::ReadyToInsert);
        RemoteDesktopSessionInit {
            session_id: self.session_id,
            session_token: self.session_token,
            creator_caller_ura: self.caller_ura,
            consent: self
                .consent
                .expect("ReadyToInsert workflow must contain consent"),
            target_binding: self
                .target_binding
                .expect("ReadyToInsert workflow must contain target binding"),
            mode: self.mode,
            lease_ttl_ms: self.lease_ttl_ms,
            transport_preferences: self.transport_preferences,
            video: self.video,
            input_policy: self.input_policy,
        }
    }

    fn assert_state(&self, expected: RemoteDesktopSessionCreationState) {
        assert_eq!(
            self.state, expected,
            "remote_desktop.create_session workflow state mismatch"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::persistence::{
        resources,
        resources::{ResourceType, ResourcesFile},
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, seed_display, test_lock, test_plugin, with_consent_ticket,
    };

    #[test]
    fn creation_workflow_builds_insertable_session_only_after_target_binding() {
        let _lock = test_lock();
        let plugin = test_plugin();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-workflow-display");
        resources::save(&file).unwrap();
        let env = env_for(&ura);
        let args = with_consent_ticket(
            &plugin,
            &env,
            json!({"session_id": "rd-workflow", "mode": "view_only"}),
        );

        let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args)
            .expect("subject and request parse")
            .consume_consent(&plugin.consent_registry(), &env)
            .expect("consent consumed")
            .resolve_target()
            .expect("target resolved");
        assert_eq!(workflow.session_id(), "rd-workflow");

        let init = workflow.into_session_init();
        assert_eq!(init.session_id, "rd-workflow");
        assert_eq!(init.target_binding.subject_ura(), ura);
        assert_eq!(
            init.target_binding.target_kind().resource_type(),
            ResourceType::Display
        );
    }
}
