//! EasyNet CLI — canonical remote invocation adapter
//! ===================================================
//!
//! File: src/daemon/invocation/routing/remote_invoke.rs
//! Description: Projects CLI/A2A/EAL remote targets into complete,
//! descriptor-bound unary, stream, and bidi requests.
//!
//! Protocol Responsibility:
//! - Preserve the seven-field Invocation tuple and descriptor binding.
//! - Require authority evaluation before constructing a remote wire carrier.
//!
//! Implementation Approach:
//! - Transition raw requests through `RemoteInvocationAuthorityBinder` into
//!   `AuthorityBoundRemoteInvocation`.
//! - Share that typestate boundary across unary, stream, and bidi transports.
//! - Sign descriptor-bound User authority and the Invocation with the same
//!   owner-bound User signer while keeping the Agent/SystemAgent/Service/Authority
//!   descriptor owner as callee.
//!
//! Usage Contract:
//! - Callers provide explicit caller, callee, ability, subject, nonce, causal
//!   context, arguments, descriptor ref, and timeout.
//! - Explicit delegation or SessionAuthority is preserved and never combined
//!   with synthesized authority.
//!
//! Architectural Position:
//! - EasyNet daemon invocation policy and transport adapter; Axon remains the
//!   canonical authority payload, signature, admission, and receipt owner.
//
//! Wire shape
//! ----------
//! 1. Caller passes `(ability_ura, args, route_target)`. The route target is
//!    either a Device placement locator or the exact Agent/SystemAgent/Service/Authority
//!    callee selected from the catalogue; non-canonical inputs surface as a
//!    typed error before any IPC.
//! 2. Authority policy transitions the request into an authority-bound state.
//! 3. The selected carrier dials the local daemon's canonical Invocation
//!    endpoint and sends the complete signed request.
//
//! Feature gating
//! --------------
//! `axon-pb` feature gates the entire module. Production builds run with it;
//! minimal builds can omit the daemon transport.
//!
//! Author: Silan.Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::core::ura::URAKind;
use crate::daemon::ability::CallMode;
use crate::daemon::invocation::routing::target::InvocationCausalContext;
use crate::daemon::invocation::{
    InvocationDerivationPolicy, ProtoEnvelope, RootInvocationDerivationIssuer,
};
use crate::daemon::persistence::daemon_config;
use axon_sdk::invocation::CausalContext;
use axon_sdk::pb::axon::v1::{InvocationState as WireInvocationState, InvokeResponse};

const FEDERATION_REVOKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type RemoteInvocationCallerSigner =
    Arc<dyn crate::daemon::identity::self_identity::CanonicalSigner>;

#[derive(Debug)]
pub(crate) enum RemoteInvocationFailure {
    RequestBuild(String),
    Transport(String),
    DaemonRejected {
        target_ura: String,
        execution_target_ura: String,
        code: tonic::Code,
        message: String,
    },
    InvocationRejected {
        state: String,
        code: String,
        message: String,
    },
    ProtocolViolation(String),
    ResultDecode(String),
}

impl std::fmt::Display for RemoteInvocationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestBuild(message) => write!(f, "{message}"),
            Self::Transport(message) => write!(f, "{message}"),
            Self::DaemonRejected {
                target_ura,
                execution_target_ura,
                code,
                message,
            } => write!(
                f,
                "daemon rejected canonical remote invocation `{target_ura}` for target \
                 `{execution_target_ura}` (code={code:?}): {message}"
            ),
            Self::InvocationRejected {
                state,
                code,
                message,
            } => write!(
                f,
                "remote Invoke did not complete: state={state} code={code} message={message}"
            ),
            Self::ProtocolViolation(message) => write!(f, "{message}"),
            Self::ResultDecode(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for RemoteInvocationFailure {}

/// Complete caller-side target for a canonical remote invocation.
///
/// This value object is the root fix for the execution-target/callee-owner
/// split. `execution_target_ura` is the node or hub the local daemon sends the
/// frame to. `callee_ura` is the Agent/SystemAgent/Service/Authority identity that
/// advertises the AbilityDescriptor. For device-native abilities the callee is
/// a device-sponsored SystemAgent and the execution target remains the Device.
///
/// What this is not: it is not a route answer. The remote daemon/authority remains
/// responsible for proving that a hosted Agent is actually runnable at the
/// execution target. This object only prevents the CLI from corrupting the
/// signed Invocation tuple before the resolver gets a chance to decide.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoteAbilityInvocationTarget {
    execution_target_ura: String,
    ability_ura: String,
    callee_ura: String,
    public_ability: String,
    descriptor_ref: String,
}

impl RemoteAbilityInvocationTarget {
    /// Project a target-owned system ability selector into a full remote
    /// invocation target with descriptor binding deferred to that target.
    ///
    /// Use this for command adapters whose product contract is "call this
    /// device-hosted SystemAgent or authority-owned system ability on `--node`".
    /// It is deliberately named target-owned so it is not reused for arbitrary
    /// Ability URAs.
    pub(crate) fn for_target_owned_selector(
        execution_target_ura: &str,
        selector: &str,
    ) -> anyhow::Result<Self> {
        Self::for_target_owned_selector_for_mode(execution_target_ura, selector, CallMode::Rpc)
    }

    pub(crate) fn for_target_owned_selector_for_mode(
        execution_target_ura: &str,
        selector: &str,
        call_mode: CallMode,
    ) -> anyhow::Result<Self> {
        validate_remote_target_ura(execution_target_ura)?;
        let public_ability =
            crate::core::ura::owner_local_ability_name(execution_target_ura, selector);
        RemoteRootAbilityAdmission::evaluate(&public_ability).require(&public_ability)?;
        let owner_ura = remote_target_owned_selector_owner_ura(execution_target_ura, selector)?;
        let public_ability = crate::core::ura::descriptor_public_ability_name(&owner_ura, selector);
        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, &public_ability)
            .ok_or_else(|| anyhow!("derive ability URA for {owner_ura} {public_ability}"))?;
        Self::from_ability_ura_for_mode(execution_target_ura, &ability_ura, call_mode)
    }

    /// Project a remote runtime catalogue read target without routing it
    /// through the target-owned system action selector.
    pub(crate) fn for_catalogue_read(execution_target_ura: &str) -> anyhow::Result<Self> {
        validate_remote_target_ura(execution_target_ura)?;
        let catalogue_owner_ura =
            runtime_introspection_owner_for_execution_target(execution_target_ura)?;
        let ability_ura = crate::core::ura::owner_ability_ura(
            &catalogue_owner_ura,
            crate::daemon::ability::builtins::governance::meta::ABILITY_LIST_ABILITIES,
        )
        .ok_or_else(|| anyhow!("derive catalogue Ability URA for {catalogue_owner_ura}"))?;
        Self::from_ability_ura(execution_target_ura, &ability_ura)
    }

    /// Accept an already-canonical Ability URA and bind it to an execution
    /// target without rewriting the Ability owner.
    pub(crate) fn from_ability_ura(
        execution_target_ura: &str,
        ability_ura: &str,
    ) -> anyhow::Result<Self> {
        Self::from_ability_ura_for_mode(execution_target_ura, ability_ura, CallMode::Rpc)
    }

    /// Accept an already-canonical Ability URA for the requested call mode and
    /// bind it to an execution target without rewriting the Ability owner.
    pub(crate) fn from_ability_ura_for_mode(
        execution_target_ura: &str,
        ability_ura: &str,
        call_mode: CallMode,
    ) -> anyhow::Result<Self> {
        validate_remote_target_ura(execution_target_ura)?;
        let trimmed = ability_ura.trim();
        let selector = crate::core::ura::AbilitySelector::parse(trimmed)?;
        validate_remote_execution_target(execution_target_ura, &selector)?;
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
                selector.owner_ura(),
                selector.public_name(),
                call_mode,
            )
            .map_err(|err| {
                anyhow!(
                    "ability URA `{trimmed}` is not descriptor-bound and no local system catalog \
                     descriptor can prove it: {err}. Pass an explicit descriptor-bound Ability ref."
                )
            })?;
        Ok(Self::from_validated_selector(
            execution_target_ura,
            trimmed,
            &selector,
            descriptor_ref,
        ))
    }

    /// Accept a descriptor-bound Ability ref and preserve the version for
    /// origin-caller proof generation.
    pub(crate) fn from_descriptor_ref(
        execution_target_ura: &str,
        descriptor_ref: &str,
    ) -> anyhow::Result<Self> {
        let canonical = axon_sdk::invocation::canonical_ability_descriptor_ref(descriptor_ref)
            .map_err(|err| anyhow!("invalid descriptor-bound Ability ref: {err}"))?;
        let ability_ura =
            crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(&canonical)
                .map_err(|err| {
                    anyhow!("descriptor-bound Ability ref is missing ability URA: {err}")
                })?;
        validate_remote_target_ura(execution_target_ura)?;
        let selector = crate::core::ura::AbilitySelector::parse(&ability_ura)?;
        validate_remote_execution_target(execution_target_ura, &selector)?;
        Ok(Self::from_validated_selector(
            execution_target_ura,
            &ability_ura,
            &selector,
            canonical,
        ))
    }

    fn from_validated_selector(
        execution_target_ura: &str,
        ability_ura: &str,
        selector: &crate::core::ura::AbilitySelector,
        descriptor_ref: String,
    ) -> Self {
        Self {
            execution_target_ura: execution_target_ura.trim().to_string(),
            ability_ura: ability_ura.trim().to_string(),
            callee_ura: selector.owner_ura().to_string(),
            public_ability: selector.public_name().to_string(),
            descriptor_ref,
        }
    }

    /// Borrow the canonical URA for the transport helper.
    pub(crate) fn as_str(&self) -> &str {
        &self.ability_ura
    }

    /// Node or hub that receives the forwarding request.
    pub(crate) fn execution_target_ura(&self) -> &str {
        &self.execution_target_ura
    }

    pub(crate) fn descriptor_ref(&self) -> &str {
        &self.descriptor_ref
    }

    /// Public route function sent to namespace.resolve.
    ///
    /// The signed descriptor ref remains the canonical ability proof. The
    /// route function is only the owner-local executable name (`er.add`,
    /// `fs.read`, ...), matching the resolver and LocalRuntime dispatch
    /// contract used by unary, stream, and bidi ingress.
    pub(crate) fn route_function_name(&self) -> &str {
        &self.public_ability
    }

    pub(crate) fn public_ability(&self) -> &str {
        &self.public_ability
    }

    pub(crate) fn callee_ura(&self) -> &str {
        &self.callee_ura
    }
}

/// Admission state for a selector that claims to be a target-owned daemon
/// system ability.
///
/// Receipt/history abilities are governance reads whose caller, subject,
/// authority, and filter scope are selected by the canonical history read
/// model. Treating them as target-owned system calls recreates the legacy
/// "callee as subject" path and produces AUTHORITY_SUBJECT_MISMATCH at
/// admission. This state object keeps the rejection at the factory/issuer
/// boundary where the tuple policy is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteRootAbilityAdmission {
    Accepted,
    ReceiptHistoryRead,
}

impl RemoteRootAbilityAdmission {
    fn evaluate(public_ability: &str) -> Self {
        if crate::daemon::ability::names::governance::is_invocation_history_read(public_ability) {
            Self::ReceiptHistoryRead
        } else {
            Self::Accepted
        }
    }

    fn require(self, public_ability: &str) -> anyhow::Result<()> {
        match self {
            Self::Accepted => Ok(()),
            Self::ReceiptHistoryRead => anyhow::bail!(
                "receipt history ability `{}` is not a target-owned remote system ability; \
                 use the canonical invocation history read path",
                public_ability.trim()
            ),
        }
    }
}

/// Admission state for public remote descriptor-bound ingress.
///
/// Public remote invoke/stream/bidi routes are caller-declared action
/// invocations. Receipt/history abilities are governance read-model routes
/// with their own caller, subject, authority, and filter semantics. Admitting
/// them here lets product callers bypass the canonical history read issuer and
/// mint noncanonical session subjects outside the history read authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemotePublicAbilityAdmission {
    Accepted,
    ReceiptHistoryRead,
}

impl RemotePublicAbilityAdmission {
    fn evaluate(public_ability: &str) -> Self {
        if crate::daemon::ability::names::governance::is_invocation_history_read(public_ability) {
            Self::ReceiptHistoryRead
        } else {
            Self::Accepted
        }
    }

    fn require(self, public_ability: &str) -> anyhow::Result<()> {
        match self {
            Self::Accepted => Ok(()),
            Self::ReceiptHistoryRead => anyhow::bail!(
                "receipt history ability `{}` is not a public remote action; \
                 use the canonical invocation history read path",
                public_ability.trim()
            ),
        }
    }
}

/// Complete caller-owned facts for one descriptor-bound remote invocation.
///
/// The selected target supplies only the descriptor-bound `ability` and
/// `callee`. Every remaining semantic field is mandatory here so the transport
/// cannot recover identities, invent causal placement, or mint freshness on
/// behalf of a public/product caller.
pub(crate) struct RemoteInvocationRequest<'a> {
    target: &'a RemoteAbilityInvocationTarget,
    caller_ura: String,
    subject_ura: String,
    invocation_nonce: [u8; 16],
    causal_context: CausalContext,
    args: Value,
    request_metadata: HashMap<String, String>,
    timeout: Duration,
}

/// Typestate proving that remote authority policy has been evaluated before
/// any carrier constructs its wire envelope.
///
/// The state does not claim that every Invocation needs SessionAuthority.
/// System callers and non-descriptor subjects retain their explicit authority
/// policy; the invariant is that the decision is complete and transport code
/// cannot silently invent or patch it.
struct AuthorityBoundRemoteInvocation<'a> {
    request: RemoteInvocationRequest<'a>,
}

impl<'a> AuthorityBoundRemoteInvocation<'a> {
    fn into_request(self) -> RemoteInvocationRequest<'a> {
        self.request
    }
}

/// EasyNet daemon policy object that transitions a complete remote request
/// into the authority-bound state consumed by unary, stream, and bidi.
struct RemoteInvocationAuthorityBinder;

impl RemoteInvocationAuthorityBinder {
    async fn bind<'a>(
        mut request: RemoteInvocationRequest<'a>,
        signer: &dyn crate::daemon::identity::self_identity::CanonicalSigner,
    ) -> Result<AuthorityBoundRemoteInvocation<'a>, RemoteInvocationFailure> {
        let request_metadata = std::mem::take(&mut request.request_metadata);
        request.request_metadata = issue_user_resource_authority_if_required(
            request.target,
            &request.caller_ura,
            &request.subject_ura,
            request.invocation_nonce,
            request_metadata,
            signer,
        )
        .await?;
        Ok(AuthorityBoundRemoteInvocation { request })
    }
}

/// Subject provenance for a remote invocation tuple.
///
/// Public ingress uses [`RemoteInvocationSubject::CallerDeclared`]. Daemon
/// system/root issuers may use [`RemoteInvocationSubject::DaemonTargetOwned`]
/// only after they have selected a target-owned subject explicitly. There is
/// no public subject omission, callee substitution, or descriptor substitution
/// policy in this state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteInvocationSubject {
    CallerDeclared(String),
    RuntimeReadProjection(String),
    DaemonTargetOwned(String),
}

impl RemoteInvocationSubject {
    fn resolve(&self) -> anyhow::Result<String> {
        let (value, field) = match self {
            Self::CallerDeclared(value) => (value, "caller-declared subject"),
            Self::RuntimeReadProjection(value) => (value, "runtime read projection subject"),
            Self::DaemonTargetOwned(value) => (value, "daemon target-owned subject"),
        };
        checked_remote_invocation_ura(value.clone(), field)
    }

    #[cfg(test)]
    fn policy_name(&self) -> &'static str {
        match self {
            Self::CallerDeclared(_) => "CallerDeclared",
            Self::RuntimeReadProjection(_) => "RuntimeReadProjection",
            Self::DaemonTargetOwned(_) => "DaemonTargetOwned",
        }
    }
}

/// Explicit nonce selected before remote transport dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteInvocationNonce {
    Explicit([u8; 16]),
}

impl RemoteInvocationNonce {
    fn derive(self) -> [u8; 16] {
        match self {
            Self::Explicit(nonce) => nonce,
        }
    }
}

/// Inspectable seven-field tuple plan for remote descriptor-bound dispatch.
///
/// This object is CLI policy, not Axon canonical wire state. It makes the
/// subject, nonce, and causal derivation policy explicit before lowering into
/// `RemoteInvocationRequest`, which then enters the existing descriptor-bound
/// runtime path.
#[derive(Debug)]
pub(crate) struct RemoteInvocationTuplePlan<'a> {
    target: &'a RemoteAbilityInvocationTarget,
    caller_ura: String,
    subject: RemoteInvocationSubject,
    nonce: RemoteInvocationNonce,
    causal_context: InvocationCausalContext,
    args: Value,
    timeout: Duration,
}

impl<'a> RemoteInvocationTuplePlan<'a> {
    pub(crate) fn public_explicit(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject_ura: impl Into<String>,
        invocation_nonce: [u8; 16],
        causal_context: CausalContext,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        RemotePublicAbilityAdmission::evaluate(target.public_ability())
            .require(target.public_ability())?;
        Self::new(
            target,
            caller_ura,
            RemoteInvocationSubject::CallerDeclared(subject_ura.into()),
            RemoteInvocationNonce::Explicit(invocation_nonce),
            InvocationCausalContext::explicit(causal_context),
            args,
            timeout,
        )
    }

    fn with_explicit_nonce(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject: RemoteInvocationSubject,
        invocation_nonce: [u8; 16],
        causal_context: CausalContext,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        Self::new(
            target,
            caller_ura,
            subject,
            RemoteInvocationNonce::Explicit(invocation_nonce),
            InvocationCausalContext::explicit(causal_context),
            args,
            timeout,
        )
    }

    fn system_root_with_explicit_nonce(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject: RemoteInvocationSubject,
        invocation_nonce: [u8; 16],
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        Self::new(
            target,
            caller_ura,
            subject,
            RemoteInvocationNonce::Explicit(invocation_nonce),
            InvocationCausalContext::daemon_system_root(),
            args,
            timeout,
        )
    }

    fn new(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject: RemoteInvocationSubject,
        nonce: RemoteInvocationNonce,
        causal_context: InvocationCausalContext,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let caller_ura = checked_remote_invocation_ura(caller_ura.into(), "caller")?;
        if timeout.is_zero() {
            bail!("remote invocation timeout must be greater than zero");
        }
        subject.resolve()?;
        Ok(Self {
            target,
            caller_ura,
            subject,
            nonce,
            causal_context,
            args,
            timeout,
        })
    }

    pub(crate) fn into_request(self) -> anyhow::Result<RemoteInvocationRequest<'a>> {
        RemoteInvocationRequest::new(
            self.target,
            self.caller_ura,
            self.subject.resolve()?,
            self.nonce.derive(),
            self.causal_context.as_axon(),
            self.args,
            self.timeout,
        )
    }
}

/// Issuer for product operations whose tuple remains accountable to a User
/// while the CLI facade owns a named fresh-root policy (deploy, uninstall,
/// and target Resource staging). The facade supplies every semantic identity;
/// this issuer contributes only root freshness and causal placement.
pub(crate) struct RemoteUserActionInvocationIssuer;

impl RemoteUserActionInvocationIssuer {
    pub(crate) fn caller_declared_root_plan<'a>(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject_ura: impl Into<String>,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<RemoteInvocationTuplePlan<'a>> {
        RemotePublicAbilityAdmission::evaluate(target.public_ability())
            .require(target.public_ability())?;
        RemoteInvocationTuplePlan::with_explicit_nonce(
            target,
            caller_ura,
            RemoteInvocationSubject::CallerDeclared(subject_ura.into()),
            axon_sdk::invocation::fresh_nonce(),
            CausalContext::None,
            args,
            timeout,
        )
    }
}

/// Issuer for daemon-owned remote root calls.
///
/// System callers select caller, subject, target, and timeout; only this issuer
/// mints the fresh nonce used for the named daemon-system root policy.
pub(crate) struct RemoteSystemInvocationIssuer;

impl RemoteSystemInvocationIssuer {
    pub(crate) fn target_owned_root_plan<'a>(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<RemoteInvocationTuplePlan<'a>> {
        let subject = target_owned_remote_system_subject(target)?;
        RemoteInvocationTuplePlan::system_root_with_explicit_nonce(
            target,
            caller_ura,
            subject,
            axon_sdk::invocation::fresh_nonce(),
            args,
            timeout,
        )
    }
}

/// Issuer for daemon-owned remote runtime catalogue reads.
///
/// Catalogue reads share the descriptor-bound remote transport with system
/// actions, but the tuple policy is not a product action: the subject is the
/// runtime owner being read, not an action-specific target-owned surrogate.
pub(crate) struct RemoteCatalogueReadIssuer;

impl RemoteCatalogueReadIssuer {
    pub(crate) fn catalogue_read_plan<'a>(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<RemoteInvocationTuplePlan<'a>> {
        if target.public_ability()
            != crate::daemon::ability::builtins::governance::meta::ABILITY_LIST_ABILITIES
        {
            anyhow::bail!(
                "remote catalogue read issuer requires `{}`, got `{}`",
                crate::daemon::ability::builtins::governance::meta::ABILITY_LIST_ABILITIES,
                target.public_ability()
            );
        }
        RemoteInvocationTuplePlan::system_root_with_explicit_nonce(
            target,
            caller_ura,
            RemoteInvocationSubject::RuntimeReadProjection(
                target.execution_target_ura().to_string(),
            ),
            axon_sdk::invocation::fresh_nonce(),
            args,
            timeout,
        )
    }
}

fn runtime_introspection_owner_for_execution_target(
    execution_target_ura: &str,
) -> anyhow::Result<String> {
    crate::daemon::ability::catalog::ownership::execution_target_owner_ura_for_public_ability(
        execution_target_ura,
        crate::daemon::ability::names::governance::META_LIST_ABILITIES,
    )
    .map_err(|error| anyhow!("remote catalogue owner projection failed: {error}"))
}

fn remote_target_owned_selector_owner_ura(
    execution_target_ura: &str,
    selector: &str,
) -> anyhow::Result<String> {
    let public_ability = crate::core::ura::owner_local_ability_name(execution_target_ura, selector);
    crate::daemon::ability::catalog::ownership::execution_target_owner_ura_for_public_ability(
        execution_target_ura,
        &public_ability,
    )
    .map_err(|error| anyhow!("remote target-owned ability owner projection failed: {error}"))
}

/// Issuer for descriptor-bound remote session follow-up roots.
///
/// Session authority metadata carries the continuation capability, while this
/// issuer owns the complete invocation tuple policy: caller-declared session
/// subject, fresh nonce, explicit root placement, arguments, and timeout.
/// Product facades must not mint or patch those fields independently.
pub(crate) struct RemoteSessionInvocationIssuer;

impl RemoteSessionInvocationIssuer {
    pub(crate) fn followup_root_plan<'a>(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject_ura: impl Into<String>,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<RemoteInvocationTuplePlan<'a>> {
        RemotePublicAbilityAdmission::evaluate(target.public_ability())
            .require(target.public_ability())?;
        RemoteInvocationTuplePlan::with_explicit_nonce(
            target,
            caller_ura,
            RemoteInvocationSubject::CallerDeclared(subject_ura.into()),
            axon_sdk::invocation::fresh_nonce(),
            CausalContext::None,
            args,
            timeout,
        )
    }
}

fn target_owned_remote_system_subject(
    target: &RemoteAbilityInvocationTarget,
) -> anyhow::Result<RemoteInvocationSubject> {
    RemoteRootAbilityAdmission::evaluate(target.public_ability())
        .require(target.public_ability())?;
    let callee = crate::core::ura::parse_ura(target.callee_ura())
        .map_err(|error| anyhow!("remote system callee URA is invalid: {error}"))?;
    let subject_ura = match callee.kind {
        crate::core::ura::URAKind::Agent if callee.device_agent_ids().is_some() => {
            target.execution_target_ura().to_string()
        }
        crate::core::ura::URAKind::Authority => target.as_str().to_string(),
        other => anyhow::bail!(
            "target-owned remote system ability requires SystemAgent or Authority callee, got {other}"
        ),
    };
    Ok(RemoteInvocationSubject::DaemonTargetOwned(subject_ura))
}

/// Issuer for child/continuation remote calls spawned from admitted runtime
/// context. The parent causal context is explicit; freshness is centralized
/// here rather than minted by product bridge code.
pub(crate) struct RemoteChildInvocationIssuer;

impl RemoteChildInvocationIssuer {
    pub(crate) fn child_plan<'a>(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject: RemoteInvocationSubject,
        causal_context: CausalContext,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<RemoteInvocationTuplePlan<'a>> {
        RemoteInvocationTuplePlan::with_explicit_nonce(
            target,
            caller_ura,
            subject,
            axon_sdk::invocation::fresh_nonce(),
            causal_context,
            args,
            timeout,
        )
    }
}

impl<'a> RemoteInvocationRequest<'a> {
    fn new(
        target: &'a RemoteAbilityInvocationTarget,
        caller_ura: impl Into<String>,
        subject_ura: impl Into<String>,
        invocation_nonce: [u8; 16],
        causal_context: CausalContext,
        args: Value,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let caller_ura = checked_remote_invocation_ura(caller_ura.into(), "caller")?;
        let subject_ura = checked_remote_invocation_ura(subject_ura.into(), "subject")?;
        if invocation_nonce == [0; 16] {
            bail!("remote invocation nonce must not be all-zero");
        }
        if timeout.is_zero() {
            bail!("remote invocation timeout must be greater than zero");
        }
        Ok(Self {
            target,
            caller_ura,
            subject_ura,
            invocation_nonce,
            causal_context,
            args,
            request_metadata: HashMap::new(),
            timeout,
        })
    }

    pub(crate) fn with_authority_metadata(
        mut self,
        authority_metadata: crate::daemon::invocation::admission::authority_metadata::IssuedAuthorityMetadata,
    ) -> Self {
        self.request_metadata = authority_metadata.into_map();
        self
    }
}

pub(crate) fn invoke_remote_target(request: RemoteInvocationRequest<'_>) -> anyhow::Result<Value> {
    let signer = load_remote_invocation_caller_signer(request.caller_ura.as_str())?;
    invoke_remote_target_with_signer(request, signer)
}

pub(crate) fn invoke_remote_target_with_signer(
    request: RemoteInvocationRequest<'_>,
    signer: RemoteInvocationCallerSigner,
) -> anyhow::Result<Value> {
    let socket_path = daemon_config::resolved_local_uds_path_with_env_override();
    invoke_remote_target_with_signer_at_endpoint(request, signer, socket_path)
}

/// Submit a signed remote invocation through one explicitly attached daemon
/// endpoint. Native SDK handles use this entry so caller identity and routing
/// remain canonical without silently switching to the process-default daemon.
pub(crate) fn invoke_remote_target_with_signer_at_endpoint(
    request: RemoteInvocationRequest<'_>,
    signer: RemoteInvocationCallerSigner,
    socket_path: std::path::PathBuf,
) -> anyhow::Result<Value> {
    if signer.owner_ura() != request.caller_ura {
        anyhow::bail!(
            "remote invocation signer owner `{}` does not match request caller `{}`",
            signer.owner_ura(),
            request.caller_ura
        );
    }
    ensure_remote_invocation_daemon_accepting(&socket_path)?;
    invoke_remote_target_on_ready_socket(request, signer, socket_path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteInvocationCarrier {
    Unary,
    Stream,
    Bidi,
}

impl RemoteInvocationCarrier {
    fn signer_error_label(self) -> &'static str {
        match self {
            Self::Unary => "remote invocation",
            Self::Stream => "remote stream invocation",
            Self::Bidi => "remote bidi invocation",
        }
    }
}

pub(crate) fn load_remote_invocation_caller_signer(
    caller_ura: &str,
) -> anyhow::Result<RemoteInvocationCallerSigner> {
    load_remote_invocation_caller_signer_for_carrier(caller_ura, RemoteInvocationCarrier::Unary)
}

pub(crate) fn load_remote_invocation_caller_signer_at_endpoint(
    caller_ura: &str,
    daemon_endpoint: &std::path::Path,
) -> anyhow::Result<RemoteInvocationCallerSigner> {
    let keyring_socket = daemon_endpoint
        .parent()
        .map(|parent| parent.join("keyring.sock"));
    let Some(keyring_socket) = keyring_socket else {
        anyhow::bail!(
            "remote invocation requires a daemon endpoint with a state-root parent, got {}",
            daemon_endpoint.display()
        );
    };
    crate::daemon::identity::self_identity::load_runtime_caller_signer_at_keyring_socket(
        caller_ura.to_string(),
        keyring_socket,
    )
    .map_err(|_err| caller_signer_unavailable_error("remote invocation", caller_ura))
}

fn load_remote_invocation_caller_signer_for_carrier(
    caller_ura: &str,
    carrier: RemoteInvocationCarrier,
) -> anyhow::Result<RemoteInvocationCallerSigner> {
    let caller_ura = caller_ura.to_string();
    let label = carrier.signer_error_label();
    crate::daemon::identity::self_identity::load_runtime_caller_signer(caller_ura.clone())
        .map_err(|_err| caller_signer_unavailable_error(label, &caller_ura))
}

fn caller_signer_unavailable_error(label: &str, caller_ura: &str) -> anyhow::Error {
    anyhow!(
        "{label} requires a caller signer for `{caller_ura}`; \
         load or provision that identity in the local key service"
    )
}

fn ensure_remote_invocation_daemon_accepting(socket_path: &std::path::Path) -> anyhow::Result<()> {
    if !crate::support::platform::local_daemon_grpc::probe_accepting(socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }
    Ok(())
}

fn invoke_remote_target_on_ready_socket(
    request: RemoteInvocationRequest<'_>,
    signer: RemoteInvocationCallerSigner,
    socket_path: std::path::PathBuf,
) -> anyhow::Result<Value> {
    invoke_remote_target_on_ready_socket_typed(request, signer, socket_path)
        .map_err(anyhow::Error::new)
}

fn invoke_remote_target_on_ready_socket_typed(
    request: RemoteInvocationRequest<'_>,
    signer: RemoteInvocationCallerSigner,
    socket_path: std::path::PathBuf,
) -> Result<Value, RemoteInvocationFailure> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            RemoteInvocationFailure::Transport(format!(
                "build tokio runtime for canonical remote invoke: {error}"
            ))
        })?;

    runtime.block_on(async move {
        let bound = RemoteInvocationAuthorityBinder::bind(request, signer.as_ref()).await?;
        let RemoteInvocationRequest {
            target,
            caller_ura,
            subject_ura,
            invocation_nonce,
            causal_context,
            args,
            request_metadata,
            timeout,
        } = bound.into_request();
        let arguments = serde_json::to_vec(&args).map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "serialise remote invocation arguments: {error}"
            ))
        })?;
        let timeout_seconds = i32::try_from(timeout.as_secs().max(1)).unwrap_or(i32::MAX);
        let mut request = ProtoEnvelope::from_target(
            caller_ura.clone(),
            target.callee_ura.clone(),
            subject_ura,
            InvocationDerivationPolicy::Explicit {
                invocation_nonce,
                causal_context,
            },
        )
        .map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "build remote invocation envelope: {error}"
            ))
        })?
        .signed_descriptor_ref_invoke_request_with_signer(
            target.route_function_name(),
            target.descriptor_ref(),
            arguments,
            signer.as_ref(),
        )
        .await
        .map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "build signed descriptor-bound remote request: {error}"
            ))
        })?;
        request.content_type = "application/json".to_string();
        request.content_envelope = Some(axon_sdk::pb::axon::v1::ContentEnvelope {
            content_type: "application/json".to_string(),
            encoding: "identity".to_string(),
            ..axon_sdk::pb::axon::v1::ContentEnvelope::default()
        });
        request.timeout_seconds = timeout_seconds;
        request.metadata = request_metadata;
        let channel = crate::support::platform::local_daemon_grpc::connect_channel(
            socket_path.clone(),
            timeout,
            Duration::from_secs(10),
        )
        .await
        .map_err(|error| {
            RemoteInvocationFailure::Transport(format!(
                "connect to local daemon gRPC endpoint at {}: {error:#}",
                socket_path.display()
            ))
        })?;

        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
        let response = tokio::time::timeout(timeout, client.invoke(request))
            .await
            .map_err(|_| {
                RemoteInvocationFailure::Transport(format!(
                    "canonical remote invocation `{}` for target `{}` timed out after {} ms \
                     waiting for the local daemon Invoke reply",
                    target.as_str(),
                    target.execution_target_ura(),
                    timeout.as_millis()
                ))
            })?
            .map_err(|status| RemoteInvocationFailure::DaemonRejected {
                target_ura: target.as_str().to_string(),
                execution_target_ura: target.execution_target_ura().to_string(),
                code: status.code(),
                message: status.message().to_string(),
            })?;
        let body = response.into_inner();
        // The local daemon owns remote dispatch and has already verified the
        // forwarded admission/terminal receipt chain against its live
        // DeviceTrustSync-backed key resolver before returning this response.
        // Re-verifying here with a fresh static realm-trust.toml snapshot would
        // reintroduce a second, stale receipt authority.
        ensure_completed_invoke_response_typed(&body)?;
        decode_invoke_result_bytes(&body.result)
            .map_err(|error| RemoteInvocationFailure::ResultDecode(error.to_string()))
    })
}

/// Public User actions use either a descriptor-bound invocation resource, an
/// existing session resource, or an exact resource subject. These
/// require exact User-signed SessionAuthority because the subject differs from
/// the User caller and the remote runtime needs an explicit authority bridge.
/// Descriptor-bound calls use invocation freshness as their authority session;
/// lifecycle calls use the existing session id encoded by the canonical
/// subject. The distinction is made once at the authority boundary and is
/// shared by unary, stream, and bidi carriers.
async fn issue_user_resource_authority_if_required(
    target: &RemoteAbilityInvocationTarget,
    caller_ura: &str,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    request_metadata: HashMap<String, String>,
    signer: &dyn crate::daemon::identity::self_identity::CanonicalSigner,
) -> Result<HashMap<String, String>, RemoteInvocationFailure> {
    use crate::daemon::invocation::admission::authority_metadata::{
        authority_subject_kind, canonical_user_session_subject_identity, AuthoritySubjectKind,
        CanonicalSessionAuthorityIssuer, SessionAuthorityRequest, DELEGATION_METADATA_KEY,
        SESSION_AUTHORITY_METADATA_KEY,
    };

    if request_metadata.contains_key(SESSION_AUTHORITY_METADATA_KEY)
        || request_metadata.contains_key(DELEGATION_METADATA_KEY)
    {
        return Ok(request_metadata);
    }

    let subject_kind = authority_subject_kind(subject_ura);
    if subject_kind == AuthoritySubjectKind::Agent {
        let caller = crate::core::ura::parse_ura(caller_ura).map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "parse remote User caller for Agent delegation: {error}"
            ))
        })?;
        if caller.kind != URAKind::User {
            return Err(RemoteInvocationFailure::RequestBuild(format!(
                "Agent subject `{subject_ura}` requires explicit delegation for non-User caller `{caller_ura}`"
            )));
        }
        let caller_user_id = caller.user_id().ok_or_else(|| {
            RemoteInvocationFailure::RequestBuild(
                "remote User caller has no canonical User id".to_string(),
            )
        })?;
        let subject = crate::core::ura::parse_ura(subject_ura).map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "parse remote Agent subject for delegation: {error}"
            ))
        })?;
        let Some((subject_owner_user_id, _)) = subject.agent_ids() else {
            return Err(RemoteInvocationFailure::RequestBuild(format!(
                "Agent subject `{subject_ura}` is not a canonical user-owned Agent"
            )));
        };
        if subject_owner_user_id != caller_user_id {
            return Err(RemoteInvocationFailure::RequestBuild(format!(
                "remote User caller `{caller_ura}` cannot delegate Agent subject owned by User `{subject_owner_user_id}`"
            )));
        }
        let issued_at_ms: i64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| {
                RemoteInvocationFailure::RequestBuild(format!(
                    "read clock for remote User Agent delegation: {error}"
                ))
            })?
            .as_millis()
            .try_into()
            .map_err(|_| {
                RemoteInvocationFailure::RequestBuild(
                    "remote User Agent delegation timestamp exceeds i64".to_string(),
                )
            })?;
        let expires_at_ms = issued_at_ms.checked_add(5 * 60 * 1_000).ok_or_else(|| {
            RemoteInvocationFailure::RequestBuild(
                "remote User Agent delegation expiry overflow".to_string(),
            )
        })?;
        let claims = crate::daemon::ability::DelegationAuthorityClaims::new(
            caller_ura,
            subject_ura,
            caller_ura,
            target.callee_ura(),
            [target.public_ability()],
            issued_at_ms,
            expires_at_ms,
        )
        .map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "prepare remote User Agent delegation: {error}"
            ))
        })?;
        let delegation = claims
            .signed_metadata_value(signer)
            .await
            .map_err(|error| {
                RemoteInvocationFailure::RequestBuild(format!(
                    "sign remote User Agent delegation: {error}"
                ))
            })?;
        let mut metadata = request_metadata;
        metadata.insert(DELEGATION_METADATA_KEY.to_string(), delegation);
        return Ok(metadata);
    }
    if !matches!(
        subject_kind,
        AuthoritySubjectKind::DescriptorBound
            | AuthoritySubjectKind::Resource
            | AuthoritySubjectKind::Session
    ) {
        return Ok(request_metadata);
    }

    let caller = crate::core::ura::parse_ura(caller_ura).map_err(|error| {
        RemoteInvocationFailure::RequestBuild(format!(
            "parse remote User caller for session authority: {error}"
        ))
    })?;
    if caller.kind != URAKind::User {
        return Err(RemoteInvocationFailure::RequestBuild(format!(
            "User resource subject `{subject_ura}` requires explicit authority for non-User caller `{caller_ura}`"
        )));
    }
    let caller_user_id = caller.user_id().ok_or_else(|| {
        RemoteInvocationFailure::RequestBuild(
            "remote User caller has no canonical User id".to_string(),
        )
    })?;
    let (subject_owner_user_id, authority_session_id) = match subject_kind {
        AuthoritySubjectKind::DescriptorBound => {
            let subject = crate::core::ura::parse_ura(subject_ura).map_err(|error| {
                RemoteInvocationFailure::RequestBuild(format!(
                    "parse descriptor-bound remote subject for session authority: {error}"
                ))
            })?;
            let owner = subject
                .resource_owner_id()
                .and_then(|owner| owner.strip_prefix("user."))
                .ok_or_else(|| {
                    RemoteInvocationFailure::RequestBuild(
                        "descriptor-bound remote subject has no canonical User owner".to_string(),
                    )
                })?
                .to_string();
            (owner, format!("invoke-{}", hex::encode(invocation_nonce)))
        }
        AuthoritySubjectKind::Session => canonical_user_session_subject_identity(subject_ura)
            .ok_or_else(|| {
                RemoteInvocationFailure::RequestBuild(
                    "session-bound remote subject has no canonical User/session identity"
                        .to_string(),
                )
            })?,
        AuthoritySubjectKind::Resource => (
            caller_user_id.to_string(),
            format!("invoke-{}", hex::encode(invocation_nonce)),
        ),
        _ => unreachable!("non-User resource subjects returned before authority issuance"),
    };
    if subject_owner_user_id != caller_user_id {
        return Err(RemoteInvocationFailure::RequestBuild(format!(
            "remote User caller `{caller_ura}` cannot authorize subject owned by User `{subject_owner_user_id}`"
        )));
    }

    let action =
        axon_sdk::invocation::admission_action_from_descriptor_ref(target.descriptor_ref())
            .map_err(|error| {
                RemoteInvocationFailure::RequestBuild(format!(
                    "derive admission action for remote User authority: {error}"
                ))
            })?;
    let issued_at_ms: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "read clock for remote User authority: {error}"
            ))
        })?
        .as_millis()
        .try_into()
        .map_err(|_| {
            RemoteInvocationFailure::RequestBuild(
                "remote User authority timestamp exceeds i64".to_string(),
            )
        })?;
    let expires_at_ms = issued_at_ms.checked_add(5 * 60 * 1_000).ok_or_else(|| {
        RemoteInvocationFailure::RequestBuild("remote User authority expiry overflow".to_string())
    })?;
    let public_ability = target.public_ability().to_string();
    let prepared = CanonicalSessionAuthorityIssuer::prepare(
        SessionAuthorityRequest {
            issuer_ura: caller_ura.to_string(),
            session_id: authority_session_id,
            session_owner_user_id: caller_user_id.to_string(),
            creator_principal_id: caller_ura.to_string(),
            callee_ura: target.callee_ura().to_string(),
            subject_ura: subject_ura.to_string(),
            audience: target.callee_ura().to_string(),
            scopes: vec![public_ability.clone()],
            allowed_actions: vec![action.to_string()],
            allowed_followup_abilities: vec![public_ability],
            issued_at_ms,
            expires_at_ms,
        },
        signer.owner_ura(),
    )
    .map_err(|error| {
        RemoteInvocationFailure::RequestBuild(format!(
            "prepare remote User resource authority: {error}"
        ))
    })?;
    let signature = signer
        .sign_canonical(prepared.canonical_payload())
        .await
        .map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "sign remote User resource authority: {error}"
            ))
        })?;
    let authority = prepared
        .seal(signature.to_bytes().to_vec())
        .map_err(|error| {
            RemoteInvocationFailure::RequestBuild(format!(
                "seal remote User resource authority: {error}"
            ))
        })?;
    let mut metadata = request_metadata;
    metadata.insert(authority.key().to_string(), authority.value().to_string());
    Ok(metadata)
}

pub(crate) fn invoke_remote_target_stream(
    request: RemoteInvocationRequest<'_>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    if max_frames == Some(0) {
        bail!("--max-frames must be greater than 0 when provided");
    }
    let caller_ura = request.caller_ura.clone();
    let signer = load_remote_invocation_caller_signer_for_carrier(
        &caller_ura,
        RemoteInvocationCarrier::Stream,
    )?;
    let socket_path = daemon_config::resolved_local_uds_path_with_env_override();
    if !crate::support::platform::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for remote InvokeStream")?;

    runtime.block_on(async move {
        let bound = RemoteInvocationAuthorityBinder::bind(request, signer.as_ref())
            .await
            .map_err(anyhow::Error::new)?;
        let RemoteInvocationRequest {
            target,
            caller_ura,
            subject_ura,
            invocation_nonce,
            causal_context,
            args,
            request_metadata,
            timeout,
        } = bound.into_request();
        let arguments = serde_json::to_vec(&args).context("serialise remote stream arguments")?;
        let timeout_seconds = i32::try_from(timeout.as_secs()).unwrap_or(i32::MAX);
        let mut stream_request = ProtoEnvelope::from_target(
            caller_ura.clone(),
            target.callee_ura.clone(),
            subject_ura,
            InvocationDerivationPolicy::Explicit {
                invocation_nonce,
                causal_context,
            },
        )?
        .signed_descriptor_ref_stream_request_with_signer(
            target.route_function_name(),
            target.descriptor_ref(),
            arguments,
            signer.as_ref(),
        )
        .await?;
        stream_request.content_type = "application/json".to_string();
        stream_request.content_envelope = Some(axon_sdk::pb::axon::v1::ContentEnvelope {
            content_type: "application/json".to_string(),
            encoding: "identity".to_string(),
            ..axon_sdk::pb::axon::v1::ContentEnvelope::default()
        });
        stream_request.timeout_seconds = timeout_seconds;
        stream_request.metadata = request_metadata;

        let channel = crate::support::platform::local_daemon_grpc::connect_channel(
            socket_path.clone(),
            timeout,
            Duration::from_secs(10),
        )
        .await
        .context("connect to local daemon gRPC endpoint")?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
        let mut stream = client
            .invoke_stream(stream_request)
            .await
            .map_err(|status| {
                anyhow!(
                    "daemon rejected canonical remote stream invocation `{}` for target `{}` \
                     (code={:?}): {}",
                    target.as_str(),
                    target.execution_target_ura(),
                    status.code(),
                    status.message(),
                )
            })?
            .into_inner();

        let mut frames = Vec::new();
        while let Some(chunk) = stream.message().await.map_err(|status| {
            anyhow!(
                "remote stream invocation `{}` for target `{}` failed while reading \
                 daemon stream (code={:?}): {}",
                target.as_str(),
                target.execution_target_ura(),
                status.code(),
                status.message(),
            )
        })? {
            let payload = if chunk.payload.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&chunk.payload)
                    .with_context(|| format!("decode {} stream frame JSON", target.as_str()))?
            };
            let terminal = chunk.terminal;
            frames.push(crate::support::platform::local_invoke::LocalStreamFrame {
                sequence: chunk.sequence,
                content_type: chunk.content_type,
                terminal,
                payload,
            });
            if terminal {
                break;
            }
            if max_frames.is_some_and(|limit| frames.len() >= limit) {
                break;
            }
        }
        Ok::<_, anyhow::Error>(frames)
    })
}

pub(crate) fn invoke_remote_target_bidi_json_frames(
    request: RemoteInvocationRequest<'_>,
    input_frames: Vec<Value>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    invoke_remote_target_bidi_frames(
        request,
        input_frames
            .into_iter()
            .map(RemoteBidiInputFrame::Json)
            .collect(),
        max_frames,
    )
}

/// Canonical caller-side input frame for a remote bidi invocation.
///
/// The frame keeps bytes, structured JSON, and transport EOF distinct until
/// the daemon wire codec consumes them.  In particular, FileTransfer bytes
/// must not be pre-wrapped as JSON before crossing the Hub, or the execution
/// host would base64-encode an already encoded business frame a second time.
pub(crate) enum RemoteBidiInputFrame {
    Binary(Vec<u8>),
    Json(Value),
    Eof,
}

pub(crate) fn invoke_remote_target_bidi_frames(
    request: RemoteInvocationRequest<'_>,
    input_frames: Vec<RemoteBidiInputFrame>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    use axon_sdk::pb::axon::v1::{
        bidi_control, invoke_bidi_up::Payload as UpPayload, BidiControl, BinaryChunk, InvokeBidiUp,
    };

    if max_frames == Some(0) {
        bail!("--max-frames must be greater than 0 when provided");
    }
    let caller_ura = request.caller_ura.clone();
    let signer = load_remote_invocation_caller_signer_for_carrier(
        &caller_ura,
        RemoteInvocationCarrier::Bidi,
    )?;
    let socket_path = daemon_config::resolved_local_uds_path_with_env_override();
    if !crate::support::platform::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for remote InvokeBidi")?;

    runtime.block_on(async move {
        let bound = RemoteInvocationAuthorityBinder::bind(request, signer.as_ref())
            .await
            .map_err(anyhow::Error::new)?;
        let RemoteInvocationRequest {
            target,
            caller_ura,
            subject_ura,
            invocation_nonce,
            causal_context,
            args,
            request_metadata,
            timeout,
        } = bound.into_request();
        let arguments = serde_json::to_vec(&args).context("serialise remote bidi arguments")?;
        let mut envelope_open = ProtoEnvelope::from_target(
            caller_ura.clone(),
            target.callee_ura.clone(),
            subject_ura,
            InvocationDerivationPolicy::Explicit {
                invocation_nonce,
                causal_context,
            },
        )?
        .signed_descriptor_ref_bidi_open_with_signer(
            target.route_function_name(),
            target.descriptor_ref(),
            arguments,
            signer.as_ref(),
        )
        .await?;
        envelope_open.metadata = request_metadata;
        let mac = remote_bidi_frame_chain_mac(&envelope_open)?;

        let channel = crate::support::platform::local_daemon_grpc::connect_channel(
            socket_path.clone(),
            timeout,
            Duration::from_secs(10),
        )
        .await
        .context("connect to local daemon gRPC endpoint")?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);

        let mut up_frames = vec![InvokeBidiUp {
            sequence: 0,
            mac,
            payload: Some(UpPayload::EnvelopeOpen(envelope_open)),
        }];

        let mut next_sequence = 1_u64;
        for input in input_frames {
            let payload = match input {
                RemoteBidiInputFrame::Binary(data) => UpPayload::BinaryChunk(BinaryChunk {
                    stream_id: 1,
                    data,
                    ..BinaryChunk::default()
                }),
                RemoteBidiInputFrame::Json(value) => {
                    let data = serde_json::to_vec(&value).with_context(|| {
                        format!("encode {} bidi input JSON frame", target.as_str())
                    })?;
                    UpPayload::BinaryChunk(BinaryChunk {
                        stream_id: 1,
                        data,
                        ..BinaryChunk::default()
                    })
                }
                RemoteBidiInputFrame::Eof => UpPayload::Control(BidiControl {
                    control: Some(bidi_control::Control::Eof(true)),
                }),
            };
            up_frames.push(InvokeBidiUp {
                sequence: next_sequence,
                mac: Vec::new(),
                payload: Some(payload),
            });
            next_sequence = next_sequence.saturating_add(1);
        }

        let mut down = client
            .invoke_bidi(tonic::Request::new(tokio_stream::iter(up_frames)))
            .await
            .map_err(|status| {
                anyhow!(
                    "daemon rejected canonical remote bidi invocation `{}` for target `{}` \
                     (code={:?}): {}",
                    target.as_str(),
                    target.execution_target_ura(),
                    status.code(),
                    status.message(),
                )
            })?
            .into_inner();

        let mut frames = Vec::new();
        while let Some(frame) = down.message().await.map_err(|status| {
            anyhow!(
                "remote bidi invocation `{}` for target `{}` failed while reading \
                 daemon stream (code={:?}): {}",
                target.as_str(),
                target.execution_target_ura(),
                status.code(),
                status.message(),
            )
        })? {
            let Some(projected) =
                crate::support::platform::local_invoke::project_invoke_bidi_down_frame(frame)?
            else {
                continue;
            };
            let terminal = projected.terminal;
            frames.push(projected);
            if terminal {
                break;
            }
            if max_frames.is_some_and(|limit| frames.len() >= limit) {
                break;
            }
        }
        Ok::<_, anyhow::Error>(frames)
    })
}

fn remote_bidi_frame_chain_mac(
    envelope_open: &axon_sdk::pb::axon::v1::EnvelopeOpen,
) -> anyhow::Result<Vec<u8>> {
    let envelope = envelope_open
        .envelope
        .as_ref()
        .ok_or_else(|| anyhow!("remote bidi builder omitted envelope"))?;
    let signature = envelope
        .caller_signature
        .as_ref()
        .ok_or_else(|| anyhow!("remote bidi builder omitted caller signature"))?;
    if signature.signature.is_empty() {
        bail!("remote bidi builder produced empty caller signature");
    }
    Ok(signature.signature.clone())
}

fn checked_remote_invocation_ura(value: String, field: &str) -> anyhow::Result<String> {
    crate::core::identity::RuntimeIdentityUra::parse(value)
        .map(crate::core::identity::RuntimeIdentityUra::into_string)
        .map_err(|error| anyhow!("remote invocation {field} URA {error}"))
}

pub(crate) fn ensure_completed_invoke_response(
    surface: &str,
    body: &InvokeResponse,
) -> anyhow::Result<()> {
    let completed = axon_sdk::invocation::InvocationState::Completed.to_wire_i32();
    if body.state == completed {
        return Ok(());
    }

    let state = remote_invoke_response_state_name(body.state)?;
    let error = body.error.as_ref();
    let (code, message) = error
        .map(|error| {
            (
                error.code.trim().to_string(),
                error.message.trim().to_string(),
            )
        })
        .unwrap_or_else(|| {
            (
                "INVOKE_NOT_COMPLETED".to_string(),
                "InvokeResponse did not carry a structured error".to_string(),
            )
        });
    bail!(
        "{surface} did not complete: state={state} code={} message={}",
        if code.is_empty() {
            "INVOKE_NOT_COMPLETED"
        } else {
            code.as_str()
        },
        if message.is_empty() {
            "InvokeResponse did not carry an error message"
        } else {
            message.as_str()
        },
    )
}

fn ensure_completed_invoke_response_typed(
    body: &InvokeResponse,
) -> Result<(), RemoteInvocationFailure> {
    let completed = axon_sdk::invocation::InvocationState::Completed.to_wire_i32();
    if body.state == completed {
        return Ok(());
    }

    let state = remote_invoke_response_state_name_typed(body.state)?;
    let error = body.error.as_ref();
    let (code, message) = error
        .map(|error| {
            (
                error.code.trim().to_string(),
                error.message.trim().to_string(),
            )
        })
        .unwrap_or_else(|| {
            (
                "INVOKE_NOT_COMPLETED".to_string(),
                "InvokeResponse did not carry a structured error".to_string(),
            )
        });
    Err(RemoteInvocationFailure::InvocationRejected {
        state,
        code: if code.is_empty() {
            "INVOKE_NOT_COMPLETED".to_string()
        } else {
            code
        },
        message: if message.is_empty() {
            "InvokeResponse did not carry an error message".to_string()
        } else {
            message
        },
    })
}

fn remote_invoke_response_state_name(state: i32) -> anyhow::Result<String> {
    remote_invoke_response_state_name_typed(state).map_err(anyhow::Error::from)
}

fn remote_invoke_response_state_name_typed(state: i32) -> Result<String, RemoteInvocationFailure> {
    WireInvocationState::try_from(state)
        .map(|state| state.as_str_name().to_string())
        .map_err(|_| {
            RemoteInvocationFailure::ProtocolViolation(format!(
                "remote InvokeResponse carried unknown InvocationState wire value `{state}`"
            ))
        })
}

fn decode_invoke_result_bytes(result_bytes: &[u8]) -> anyhow::Result<Value> {
    if result_bytes.is_empty() {
        return Ok(Value::Null);
    }
    match serde_json::from_slice::<Value>(result_bytes) {
        Ok(v) => Ok(v),
        Err(_) => Ok(serde_json::json!({
            "result_bytes_len": result_bytes.len(),
            "result_bytes_hex": result_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>(),
        })),
    }
}

fn validate_remote_execution_target(
    execution_target_ura: &str,
    selector: &crate::core::ura::AbilitySelector,
) -> anyhow::Result<()> {
    let target = crate::core::identity::RuntimeIdentityUra::parse(execution_target_ura)
        .map_err(|err| anyhow!("invalid target URA `{execution_target_ura}`: {err}"))?;
    let owner =
        crate::core::identity::RuntimeIdentityUra::parse(selector.owner_ura()).map_err(|err| {
            anyhow!(
                "invalid ability owner URA `{}`: {err}",
                selector.owner_ura()
            )
        })?;
    if owner.realm() != target.realm() {
        bail!(
            "ability URA `{}` belongs to realm `{}`, but execution target `{}` belongs to realm `{}`",
            selector.ability_ura(),
            owner.realm(),
            execution_target_ura,
            target.realm()
        );
    }

    match (target.kind(), selector.owner_kind()) {
        (URAKind::Agent, "agent" | "system-agent") => {
            if selector.owner_ura() == target.as_str() {
                Ok(())
            } else {
                bail!(
                    "exact Agent/SystemAgent target `{}` does not own ability `{}`; owner is `{}`",
                    execution_target_ura,
                    selector.ability_ura(),
                    selector.owner_ura()
                );
            }
        }
        (URAKind::Device, "agent") => Ok(()),
        (URAKind::Device, "service") => Ok(()),
        (URAKind::Device, "system-agent") => {
            let owner = crate::core::ura::parse_ura(selector.owner_ura()).map_err(|error| {
                anyhow!(
                    "system-agent ability owner URA `{}` is invalid: {error}",
                    selector.owner_ura()
                )
            })?;
            let target = crate::core::ura::parse_ura(execution_target_ura)
                .map_err(|error| anyhow!("Device execution target URA is invalid: {error}"))?;
            let Some((owner_device_id, _agent_id)) = owner.device_agent_ids() else {
                bail!(
                    "system-agent ability URA `{}` has non-system-agent owner `{}`",
                    selector.ability_ura(),
                    selector.owner_ura()
                );
            };
            let Some(target_device_id) = target.device_id() else {
                bail!(
                    "system-agent ability URA `{}` requires a Device execution target with device id, got `{}`",
                    selector.ability_ura(),
                    execution_target_ura
                );
            };
            if owner_device_id == target_device_id {
                Ok(())
            } else {
                bail!(
                    "system-agent ability URA `{}` must execute on sponsoring Device `{}`, not `{}`",
                    selector.ability_ura(),
                    crate::core::ura::device_ura(&owner.realm, owner_device_id),
                    execution_target_ura
                );
            }
        }
        (URAKind::Device, "device") => bail!(
            "direct Device-owned ability URA `{}` is migration-only and cannot be used as a normal remote invocation callee; use the device-sponsored SystemAgent owner for this ability",
            selector.ability_ura()
        ),
        (URAKind::Authority, "authority") => {
            if selector.owner_ura() == target.as_str() {
                Ok(())
            } else {
                bail!(
                    "authority-owned ability URA `{}` must execute on its owning Authority `{}`, not `{}`",
                    selector.ability_ura(),
                    selector.owner_ura(),
                    execution_target_ura
                );
            }
        }
        (URAKind::Authority, "agent") => Ok(()),
        (URAKind::Service, "service") => {
            if selector.owner_ura() == target.as_str() {
                Ok(())
            } else {
                bail!(
                    "service-owned ability URA `{}` must execute through its owning Service `{}`, not `{}`",
                    selector.ability_ura(),
                    selector.owner_ura(),
                    execution_target_ura
                );
            }
        }
        (URAKind::Device, "authority") => bail!(
            "authority-owned ability URA `{}` requires an Authority execution target, not device `{}`",
            selector.ability_ura(),
            execution_target_ura
        ),
        (URAKind::Authority, "device") => bail!(
            "device-owned ability URA `{}` requires its owning Device execution target `{}`, not Authority `{}`",
            selector.ability_ura(),
            selector.owner_ura(),
            execution_target_ura
        ),
        (target_kind, owner_kind) => bail!(
            "ability URA `{}` cannot execute on target `{}`: unsupported owner/target pair owner_kind={} target_kind={}",
            selector.ability_ura(),
            execution_target_ura,
            owner_kind,
            target_kind
        ),
    }
}

fn validate_remote_target_ura(target_ura: &str) -> anyhow::Result<()> {
    let identity = crate::core::identity::RuntimeIdentityUra::parse(target_ura)
        .map_err(|err| anyhow::anyhow!("invalid target URA `{target_ura}`: {err}"))?;
    match identity.kind() {
        URAKind::Device => Ok(()),
        URAKind::Agent => Ok(()),
        URAKind::Service => Ok(()),
        URAKind::Authority => Ok(()),
        other => {
            bail!(
                "target URA `{target_ura}` must identify a Device placement or exact \
                 Agent/SystemAgent/Service/Authority callee, got kind={other}"
            )
        }
    }
}

/// Operator/audit cross-realm directory query against the local daemon's
/// `federation.discover` ability. This intentionally performs an unfiltered
/// directory read and must not be used by product surfaces that act on behalf
/// of a paired user.
pub fn invoke_federation_discover_for_operator_audit(
    agent_ura_filter: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let local_daemon_ura = crate::daemon::identity::local_invocation::local_daemon_ura()?;
    let scope = FederationDiscoverScope::operator_audit(&local_daemon_ura)?;
    invoke_federation_discover_with_scope(agent_ura_filter, scope)
}

/// User-scoped cross-realm directory query. Product surfaces use this path so
/// the daemon can enforce the PR-N4 user-binding privacy filter. A missing or
/// empty user id is unresolved caller state, not permission to fall back to the
/// operator/audit directory.
pub fn invoke_federation_discover_for_user(
    agent_ura_filter: Option<&str>,
    local_user_id_filter: &str,
) -> anyhow::Result<Vec<Value>> {
    let local_user_id_filter = validate_federation_discover_local_user_id(local_user_id_filter)?;
    let local_daemon_ura = crate::daemon::identity::local_invocation::local_daemon_ura()?;
    let scope = FederationDiscoverScope::user(&local_daemon_ura, local_user_id_filter)?;
    invoke_federation_discover_with_scope(agent_ura_filter, scope)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FederationDiscoverScope {
    query_target_ura: String,
    caller_ura: String,
    subject_ura: String,
    local_user_id_filter: Option<String>,
}

impl FederationDiscoverScope {
    fn operator_audit(local_daemon_ura: &str) -> anyhow::Result<Self> {
        if local_daemon_ura.trim().is_empty() {
            anyhow::bail!("federation.discover operator/audit scope requires local daemon URA");
        }
        Ok(Self {
            query_target_ura: local_daemon_ura.to_string(),
            caller_ura: local_daemon_ura.to_string(),
            subject_ura: federation_discover_authority_subject_ura(local_daemon_ura)?,
            local_user_id_filter: None,
        })
    }

    fn user(local_daemon_ura: &str, local_user_id_filter: &str) -> anyhow::Result<Self> {
        let local_user_id_filter =
            validate_federation_discover_local_user_id(local_user_id_filter)?;
        let parsed_daemon = crate::core::ura::parse_ura(local_daemon_ura)
            .map_err(|err| anyhow!("parse local daemon URA for federation.discover: {err}"))?;
        let caller_ura = crate::core::ura::user_ura(&parsed_daemon.realm, local_user_id_filter);
        Ok(Self {
            query_target_ura: crate::core::ura::authority_ura(&parsed_daemon.realm),
            caller_ura,
            subject_ura: crate::core::ura::resource_dot_ura(
                &parsed_daemon.realm,
                &format!("user.{local_user_id_filter}"),
                "directory/devices",
            ),
            local_user_id_filter: Some(local_user_id_filter.to_string()),
        })
    }

    fn query_target_ura(&self) -> &str {
        &self.query_target_ura
    }

    fn caller_ura(&self) -> &str {
        &self.caller_ura
    }

    fn subject_ura(&self) -> &str {
        &self.subject_ura
    }

    fn write_request_args(&self, req_args: &mut Value) {
        if let Some(user) = &self.local_user_id_filter {
            req_args["local_user_id"] = Value::String(user.clone());
        }
    }
}

fn validate_federation_discover_local_user_id(local_user_id: &str) -> anyhow::Result<&str> {
    let local_user_id = local_user_id.trim();
    if local_user_id.is_empty() {
        anyhow::bail!("federation.discover user filter requires a non-empty local_user_id");
    }
    if crate::core::identity::is_all_zero_principal_id(local_user_id) {
        anyhow::bail!("federation.discover user filter rejects the all-zero principal placeholder");
    }
    Ok(local_user_id)
}

fn invoke_federation_discover_with_scope(
    agent_ura_filter: Option<&str>,
    scope: FederationDiscoverScope,
) -> anyhow::Result<Vec<Value>> {
    let mut req_args = json!({});
    if let Some(ura) = agent_ura_filter {
        req_args["agent_ura"] = Value::String(ura.to_string());
    }
    scope.write_request_args(&mut req_args);
    let arg_bytes = serde_json::to_vec(&req_args).context("encode discover args")?;

    let target = RemoteAbilityInvocationTarget::for_target_owned_selector(
        scope.query_target_ura(),
        "federation.discover",
    )?;
    let signer = load_federation_caller_signer(scope.caller_ura(), "federation.discover")?;
    let socket_path = daemon_config::resolved_local_uds_path_with_env_override();
    if !crate::support::platform::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }
    let request_envelope = ProtoEnvelope::from_target(
        scope.caller_ura(),
        target.callee_ura(),
        scope.subject_ura(),
        RootInvocationDerivationIssuer::fresh_root(),
    )?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation.discover")?;

    let response: axon_sdk::pb::axon::v1::InvokeResponse = {
        runtime.block_on(async move {
            let request = request_envelope
                .signed_descriptor_ref_invoke_request_with_signer(
                    target.route_function_name(),
                    target.descriptor_ref(),
                    arg_bytes,
                    signer.as_ref(),
                )
                .await
                .context("build descriptor-bound federation.discover request")?;
            let channel = crate::support::platform::local_daemon_grpc::connect_channel(
                socket_path.clone(),
                Duration::from_secs(10),
                Duration::from_secs(5),
            )
            .await
            .context("connect to local daemon gRPC endpoint")?;
            let mut client = crate::daemon::invocation::transport::invocation_client(channel);
            let resp = client.invoke(request).await.map_err(|status| {
                anyhow!(
                    "daemon rejected federation.discover: code={:?} message={}",
                    status.code(),
                    status.message()
                )
            })?;
            Ok::<_, anyhow::Error>(resp.into_inner())
        })?
    };

    ensure_completed_invoke_response("federation.discover", &response)?;

    let body: Value =
        serde_json::from_slice(&response.result).context("decode discover response body")?;
    Ok(body
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn federation_discover_authority_subject_ura(callee_ura: &str) -> anyhow::Result<String> {
    let parsed = crate::core::ura::parse_ura(callee_ura)?;
    if parsed.kind == crate::core::ura::URAKind::Authority {
        return crate::core::ura::owner_ability_ura(callee_ura, "federation.discover").ok_or_else(
            || anyhow!("cannot derive federation.discover subject for hub callee `{callee_ura}`"),
        );
    }
    Ok(callee_ura.to_string())
}

fn load_federation_caller_signer(
    caller_ura: &str,
    ability: &str,
) -> anyhow::Result<RemoteInvocationCallerSigner> {
    crate::daemon::identity::self_identity::load_runtime_caller_signer(caller_ura.to_string())
        .map_err(|_err| caller_signer_unavailable_error(ability, caller_ura))
}

/// `federation.revoke` against the local daemon's gRPC
/// InvocationServer. Removes the named Agent's directory entry on
/// the hub. CLI lifecycle surfaces (`easynet device remove`,
/// `easynet device reset --force`) call this helper instead of
/// relying on local-only `node.remove` acknowledgements.
///
/// Args:
///   * `agent_ura` — canonical URA of the Agent to revoke (typically
///     a device URA `easynet:///r/<realm>/device/<id>`).
///   * `reason` — operator-supplied label, written through to the
///     receipt for audit. `"deregister"` / `"reset"` are common.
///   * `caller_ura` — explicit local daemon caller selected by the product
///     facade before transport entry. The helper validates it against the
///     active control-discovery identity instead of silently reselecting an
///     ambient caller.
/// Returns `Ok(())` on a successful ack from the daemon. Best-effort
/// by contract on the hub side, but this helper still surfaces
/// transport / parse errors so callers can log them honestly.
pub fn invoke_federation_revoke(
    agent_ura: &str,
    reason: &str,
    caller_ura: &str,
) -> anyhow::Result<()> {
    let req_args = json!({
        "agent_ura": agent_ura,
        "reason": reason,
    });
    let arg_bytes = serde_json::to_vec(&req_args).context("encode revoke args")?;
    invoke_federation_revoke_with_arguments(agent_ura, arg_bytes, caller_ura).map(|_| ())
}

/// Submit an already-encoded `federation.revoke` payload through the local
/// canonical Invocation service. Runtime lifecycle callers use this entry for
/// the durable purge fields that are not part of the compact CLI command.
/// Keeping both callers on this boundary guarantees that product revocation is
/// carried as a signed `ReverseDispatchCall`, never as a JSON session-control
/// request.
pub(crate) fn invoke_federation_revoke_with_arguments(
    agent_ura: &str,
    arg_bytes: Vec<u8>,
    caller_ura: &str,
) -> anyhow::Result<crate::daemon::invocation::dispatch::federation_wrappers::RevokeResponse> {
    let command: crate::daemon::invocation::dispatch::federation_wrappers::RevokeRequest =
        serde_json::from_slice(&arg_bytes).context("decode federation.revoke command proof")?;
    if command.agent_ura.trim() != agent_ura {
        bail!("federation.revoke command target does not match the invocation subject");
    }
    let socket_path = daemon_config::resolved_local_uds_path_with_env_override();
    if !crate::support::platform::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    let local_daemon_ura = crate::daemon::identity::local_invocation::local_daemon_ura()?;
    let caller_ura = canonical_federation_revoke_caller(caller_ura, &local_daemon_ura)?;
    let target = federation_revoke_authority_target(&local_daemon_ura)?;
    let signer = load_federation_caller_signer(&caller_ura, "federation.revoke")?;
    let request_envelope = ProtoEnvelope::from_target(
        caller_ura.as_str(),
        target.callee_ura(),
        agent_ura,
        RootInvocationDerivationIssuer::fresh_root(),
    )?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation.revoke")?;

    runtime.block_on(async move {
        let mut request = request_envelope
            .signed_descriptor_ref_invoke_request_with_signer(
                target.route_function_name(),
                target.descriptor_ref(),
                arg_bytes,
                signer.as_ref(),
            )
            .await
            .context("build descriptor-bound federation.revoke request")?;
        request.timeout_seconds = i32::try_from(FEDERATION_REVOKE_TIMEOUT.as_secs())
            .unwrap_or(i32::MAX)
            .max(1);
        let channel = crate::support::platform::local_daemon_grpc::connect_channel(
            socket_path.clone(),
            FEDERATION_REVOKE_TIMEOUT,
            Duration::from_secs(5),
        )
        .await
        .context("connect to local daemon gRPC endpoint")?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
        let response = tokio::time::timeout(FEDERATION_REVOKE_TIMEOUT, client.invoke(request))
            .await
            .map_err(|_| anyhow!("daemon federation.revoke timed out after 10s"))?
            .map_err(|status| {
                anyhow!(
                    "daemon rejected federation.revoke: code={:?} message={}",
                    status.code(),
                    status.message()
                )
            })?;
        let response = response.into_inner();
        ensure_completed_invoke_response("federation.revoke", &response)?;
        decode_and_verify_federation_revoke_response(&command, &response.result)
    })
}

fn decode_and_verify_federation_revoke_response(
    command: &crate::daemon::invocation::dispatch::federation_wrappers::RevokeRequest,
    result: &[u8],
) -> anyhow::Result<crate::daemon::invocation::dispatch::federation_wrappers::RevokeResponse> {
    use crate::daemon::persistence::federation_revoke::FederationRevokeDisposition;

    let receipt: crate::daemon::invocation::dispatch::federation_wrappers::RevokeResponse =
        serde_json::from_slice(result).context("decode federation.revoke semantic receipt")?;
    if !receipt.ack {
        bail!("federation.revoke semantic receipt rejected the command");
    }
    match command.purge_transaction_id.as_deref() {
        Some(expected_transaction_id) => {
            if receipt.purge_transaction_id.as_deref() != Some(expected_transaction_id) {
                bail!("federation.revoke receipt transaction does not match the durable command");
            }
            match receipt.disposition {
                Some(FederationRevokeDisposition::Retired)
                | Some(FederationRevokeDisposition::AlreadyRetired) => {}
                Some(FederationRevokeDisposition::SupersededByNewIncarnation) => {
                    bail!("federation.revoke did not retire the requested Hub incarnation")
                }
                None => bail!("federation.revoke purge receipt has no durable disposition"),
            }
        }
        None => {
            if receipt.purge_transaction_id.is_some() || receipt.disposition.is_some() {
                bail!(
                    "federation.revoke immediate receipt unexpectedly claims a purge transaction"
                );
            }
        }
    }
    Ok(receipt)
}

fn canonical_federation_revoke_caller(
    caller_ura: &str,
    local_daemon_ura: &str,
) -> anyhow::Result<String> {
    let caller = checked_remote_invocation_ura(caller_ura.to_string(), "federation.revoke caller")?;
    let local = checked_remote_invocation_ura(
        local_daemon_ura.to_string(),
        "federation.revoke local daemon",
    )?;
    if caller != local {
        bail!("federation.revoke caller `{caller}` does not match active local daemon `{local}`");
    }
    let parsed = crate::core::ura::parse_ura(&caller)
        .map_err(|error| anyhow!("federation.revoke caller is not canonical: {error}"))?;
    if !matches!(
        parsed.kind,
        crate::core::ura::URAKind::Device | crate::core::ura::URAKind::Authority
    ) {
        bail!(
            "federation.revoke caller must be a Device or Authority URA, got {}",
            parsed.kind
        );
    }
    Ok(caller)
}

fn federation_revoke_authority_target(
    local_daemon_ura: &str,
) -> anyhow::Result<RemoteAbilityInvocationTarget> {
    let parsed = crate::core::ura::parse_ura(local_daemon_ura)
        .map_err(|error| anyhow!("federation.revoke local daemon is not canonical: {error}"))?;
    let authority_ura = crate::core::ura::authority_ura(&parsed.realm);
    RemoteAbilityInvocationTarget::for_target_owned_selector(&authority_ura, "federation.revoke")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::identity::self_identity::TestCanonicalSigner;
    use crate::daemon::invocation::admission::authority_metadata::{
        decode_delegation_authority_wire, decode_session_authority_wire, DELEGATION_METADATA_KEY,
        SESSION_AUTHORITY_METADATA_KEY,
    };

    fn device_system_agent_owner_ura(
        realm: &str,
        device_id: &str,
        system_agent_id: &str,
    ) -> String {
        crate::core::ura::device_agent_ura(realm, device_id, system_agent_id)
    }

    fn descriptor_ref_for_device_system_agent(
        realm: &str,
        device_id: &str,
        system_agent_id: &str,
        public_ability: &str,
    ) -> String {
        let owner_ura = device_system_agent_owner_ura(realm, device_id, system_agent_id);
        let ability_ura = crate::core::ura::owner_ability_ura(&owner_ura, public_ability)
            .expect("SystemAgent ability URA");
        format!(
            "{ability_ura}@2.4.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!invoke"
        )
    }

    fn health_descriptor_ref(realm: &str) -> String {
        descriptor_ref_for_device_system_agent(
            realm,
            "node-a",
            crate::daemon::ability::names::governance::RUNTIME_HEALTH_SYSTEM_AGENT_ID,
            crate::daemon::ability::names::governance::OBSERVE_HEALTH,
        )
    }

    fn remote_request<'a>(
        target: &'a RemoteAbilityInvocationTarget,
        caller: &str,
        subject: &str,
        nonce: [u8; 16],
        metadata: HashMap<String, String>,
    ) -> RemoteInvocationRequest<'a> {
        let mut request = RemoteInvocationRequest::new(
            target,
            caller,
            subject,
            nonce,
            CausalContext::None,
            json!({}),
            Duration::from_secs(30),
        )
        .expect("complete User action request");
        request.request_metadata = metadata;
        request
    }

    #[tokio::test]
    async fn authority_binder_issues_exact_descriptor_action_for_all_actions() {
        let descriptor_prefix = health_descriptor_ref("realm")
            .rsplit_once('!')
            .expect("descriptor action")
            .0
            .to_string();
        let caller = "easynet:///r/realm/user/alice";
        let subject = "easynet:///r/realm/resource/user.alice/invoke/observe.health";
        let signer = TestCanonicalSigner::new(caller, [0x42; 32]);

        for (index, action) in ["read", "invoke", "manage"].into_iter().enumerate() {
            let descriptor = format!("{descriptor_prefix}!{action}");
            let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
                "easynet:///r/realm/device/node-a",
                &descriptor,
            )
            .expect("target");
            let nonce = [0x11_u8.saturating_add(index as u8); 16];
            let bound = RemoteInvocationAuthorityBinder::bind(
                remote_request(&target, caller, subject, nonce, HashMap::new()),
                &signer,
            )
            .await
            .expect("bind exact User authority");
            let request = bound.into_request();
            let wire = decode_session_authority_wire(
                request
                    .request_metadata
                    .get(SESSION_AUTHORITY_METADATA_KEY)
                    .expect("session authority metadata"),
            )
            .expect("decode session authority");

            assert_eq!(wire.payload.issuer_ura, caller);
            assert_eq!(wire.payload.creator_principal_id, caller);
            assert_eq!(wire.payload.session_owner_user_id, "alice");
            assert_eq!(wire.payload.callee_ura, target.callee_ura());
            assert_eq!(wire.payload.audience, target.callee_ura());
            assert_eq!(wire.payload.subject_ura, subject);
            assert_eq!(wire.payload.scopes, ["observe.health"]);
            assert_eq!(wire.payload.allowed_followup_abilities, ["observe.health"]);
            assert_eq!(wire.payload.allowed_actions, [action]);
            assert_eq!(
                wire.payload.session_id,
                format!("invoke-{}", hex::encode(nonce))
            );
        }
    }

    #[tokio::test]
    async fn authority_binder_issues_user_delegation_for_agent_subject() {
        let descriptor = descriptor_ref_for_device_system_agent(
            "realm",
            "node-a",
            crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
            crate::daemon::ability::names::automation::MISSION_RUN,
        );
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("mission.run target");
        let caller = "easynet:///r/realm/user/alice";
        let subject = "easynet:///r/realm/agent/alice.worker";
        let signer = TestCanonicalSigner::new(caller, [0x62; 32]);

        let bound = RemoteInvocationAuthorityBinder::bind(
            remote_request(&target, caller, subject, [0x62; 16], HashMap::new()),
            &signer,
        )
        .await
        .expect("bind Agent subject delegation");
        let request = bound.into_request();

        assert!(
            !request
                .request_metadata
                .contains_key(SESSION_AUTHORITY_METADATA_KEY),
            "Agent subjects must not use session authority"
        );
        let wire = decode_delegation_authority_wire(
            request
                .request_metadata
                .get(DELEGATION_METADATA_KEY)
                .expect("delegation metadata"),
        )
        .expect("decode delegation authority");
        assert_eq!(wire.payload.issuer_ura(), caller);
        assert_eq!(wire.payload.caller_ura(), caller);
        assert_eq!(wire.payload.subject_ura(), subject);
        assert_eq!(wire.payload.audience(), target.callee_ura());
        assert_eq!(wire.payload.scopes(), ["mission.run"]);
    }

    #[tokio::test]
    async fn authority_binder_issues_session_authority_for_device_resource_subject() {
        let descriptor = descriptor_ref_for_device_system_agent(
            "realm",
            "node-a",
            crate::daemon::ability::names::device_control::LOCOMOTION_SYSTEM_AGENT_ID,
            crate::daemon::ability::names::device_control::FS_TRANSFER,
        )
        .rsplit_once('!')
        .map(|(prefix, _)| format!("{prefix}!stream"))
        .expect("fs.transfer descriptor action");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("fs.transfer target");
        let caller = "easynet:///r/realm/user/alice";
        let subject =
            "easynet:///r/realm/resource/device.node-a/fs/tmp/easynet-ability-deploy/bundle.tar.gz";
        let nonce = [0x63; 16];
        let signer = TestCanonicalSigner::new(caller, [0x63; 32]);

        let bound = RemoteInvocationAuthorityBinder::bind(
            remote_request(&target, caller, subject, nonce, HashMap::new()),
            &signer,
        )
        .await
        .expect("bind Device Resource authority");
        let request = bound.into_request();
        let wire = decode_session_authority_wire(
            request
                .request_metadata
                .get(SESSION_AUTHORITY_METADATA_KEY)
                .expect("session authority metadata"),
        )
        .expect("decode session authority");

        assert_eq!(wire.payload.issuer_ura, caller);
        assert_eq!(wire.payload.creator_principal_id, caller);
        assert_eq!(wire.payload.session_owner_user_id, "alice");
        assert_eq!(wire.payload.callee_ura, target.callee_ura());
        assert_eq!(wire.payload.audience, target.callee_ura());
        assert_eq!(wire.payload.subject_ura, subject);
        assert_eq!(wire.payload.scopes, ["fs.transfer"]);
        assert_eq!(wire.payload.allowed_followup_abilities, ["fs.transfer"]);
        assert_eq!(wire.payload.allowed_actions, ["stream"]);
        assert_eq!(
            wire.payload.session_id,
            format!("invoke-{}", hex::encode(nonce))
        );
    }

    #[tokio::test]
    async fn authority_binder_issues_session_authority_for_generic_resource_subject() {
        let descriptor = descriptor_ref_for_device_system_agent(
            "realm",
            "node-a",
            crate::daemon::ability::names::integrations::PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID,
            "user_plugin.echo",
        );
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("user_plugin.echo target");
        let caller = "easynet:///r/realm/user/alice";
        let subject = "easynet:///r/realm/resource/e2e/user-plugin/echo";
        let signer = TestCanonicalSigner::new(caller, [0x64; 32]);

        let bound = RemoteInvocationAuthorityBinder::bind(
            remote_request(&target, caller, subject, [0x64; 16], HashMap::new()),
            &signer,
        )
        .await
        .expect("generic Resource gets exact User session authority");
        let request = bound.into_request();
        let wire = decode_session_authority_wire(
            request
                .request_metadata
                .get(SESSION_AUTHORITY_METADATA_KEY)
                .expect("session authority metadata"),
        )
        .expect("decode session authority");

        assert_eq!(wire.payload.issuer_ura, caller);
        assert_eq!(wire.payload.creator_principal_id, caller);
        assert_eq!(wire.payload.session_owner_user_id, "alice");
        assert_eq!(wire.payload.callee_ura, target.callee_ura());
        assert_eq!(wire.payload.audience, target.callee_ura());
        assert_eq!(wire.payload.subject_ura, subject);
        assert_eq!(wire.payload.scopes, ["user_plugin.echo"]);
        assert_eq!(
            wire.payload.allowed_followup_abilities,
            ["user_plugin.echo"]
        );
        assert_eq!(wire.payload.allowed_actions, ["invoke"]);
    }

    #[tokio::test]
    async fn authority_binder_uses_existing_session_id_for_session_resource() {
        let descriptor = descriptor_ref_for_device_system_agent(
            "realm",
            "node-a",
            crate::daemon::ability::names::device_control::TERMINAL_SYSTEM_AGENT_ID,
            crate::daemon::ability::names::device_control::TERMINAL_CLOSE,
        )
        .rsplit_once('!')
        .map(|(prefix, _)| format!("{prefix}!manage"))
        .expect("terminal.close descriptor action");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("terminal.close target");
        let caller = "easynet:///r/realm/user/alice";
        let subject = "easynet:///r/realm/resource/user.alice/session/pty-1";
        let signer = TestCanonicalSigner::new(caller, [0x52; 32]);

        let bound = RemoteInvocationAuthorityBinder::bind(
            remote_request(&target, caller, subject, [0x33; 16], HashMap::new()),
            &signer,
        )
        .await
        .expect("bind session lifecycle authority");
        let request = bound.into_request();
        let wire = decode_session_authority_wire(
            request
                .request_metadata
                .get(SESSION_AUTHORITY_METADATA_KEY)
                .expect("session authority metadata"),
        )
        .expect("decode session authority");

        assert_eq!(wire.payload.session_id, "pty-1");
        assert_eq!(wire.payload.subject_ura, subject);
        assert_eq!(wire.payload.session_owner_user_id, "alice");
        assert_eq!(wire.payload.scopes, ["terminal.close"]);
        assert_eq!(wire.payload.allowed_followup_abilities, ["terminal.close"]);
        assert_eq!(wire.payload.allowed_actions, ["manage"]);
    }

    #[tokio::test]
    async fn descriptor_bound_user_action_rejects_different_owner() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let caller = "easynet:///r/realm/user/alice";
        let signer = TestCanonicalSigner::new(caller, [0x43; 32]);
        let error = match RemoteInvocationAuthorityBinder::bind(
            remote_request(
                &target,
                caller,
                "easynet:///r/realm/resource/user.mallory/invoke/observe.health",
                [0x12; 16],
                HashMap::new(),
            ),
            &signer,
        )
        .await
        {
            Ok(_) => panic!("caller must not authorize another User subject"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("cannot authorize subject owned"));
    }

    #[tokio::test]
    async fn authority_binder_rejects_subject_bound_to_different_ability() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let caller = "easynet:///r/realm/user/alice";
        let signer = TestCanonicalSigner::new(caller, [0x46; 32]);
        let error = match RemoteInvocationAuthorityBinder::bind(
            remote_request(
                &target,
                caller,
                "easynet:///r/realm/resource/user.alice/invoke/fs.read",
                [0x15; 16],
                HashMap::new(),
            ),
            &signer,
        )
        .await
        {
            Ok(_) => panic!("subject ability must match the selected descriptor"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(
                "descriptor-bound subject ability must be an exact allowed follow-up ability"
            ),
            "wrong error: {error}"
        );
    }

    #[tokio::test]
    async fn explicit_authority_metadata_is_never_double_issued() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let caller = "easynet:///r/realm/user/alice";
        let subject = "easynet:///r/realm/resource/user.alice/invoke/observe.health";
        let signer = TestCanonicalSigner::new(caller, [0x44; 32]);

        for key in [SESSION_AUTHORITY_METADATA_KEY, DELEGATION_METADATA_KEY] {
            let existing = HashMap::from([(key.to_string(), "explicit-authority".to_string())]);
            let bound = RemoteInvocationAuthorityBinder::bind(
                remote_request(&target, caller, subject, [0x13; 16], existing.clone()),
                &signer,
            )
            .await
            .expect("preserve explicit authority");
            assert_eq!(bound.into_request().request_metadata, existing);
        }
    }

    #[tokio::test]
    async fn authority_binder_does_not_synthesize_user_authority_for_other_subject_kinds() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let caller = "easynet:///r/realm/user/alice";
        let signer = TestCanonicalSigner::new(caller, [0x45; 32]);
        let bound = RemoteInvocationAuthorityBinder::bind(
            remote_request(
                &target,
                caller,
                "easynet:///r/realm/device/node-a",
                [0x14; 16],
                HashMap::new(),
            ),
            &signer,
        )
        .await
        .expect("evaluate non-descriptor subject policy");
        assert!(bound.into_request().request_metadata.is_empty());
    }

    #[tokio::test]
    async fn authority_binder_rejects_non_user_caller_without_explicit_authority() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let caller = "easynet:///r/realm/device/node-b";
        let signer = TestCanonicalSigner::new(caller, [0x47; 32]);
        let error = match RemoteInvocationAuthorityBinder::bind(
            remote_request(
                &target,
                caller,
                "easynet:///r/realm/resource/user.alice/invoke/observe.health",
                [0x16; 16],
                HashMap::new(),
            ),
            &signer,
        )
        .await
        {
            Ok(_) => panic!("non-User caller must provide explicit authority"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("requires explicit authority"),
            "wrong error: {error}"
        );
    }

    fn receipt_history_descriptor_ref() -> String {
        let owner_ura = device_system_agent_owner_ura(
            "realm",
            "node-a",
            crate::daemon::ability::names::governance::RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID,
        );
        let ability_ura = crate::core::ura::owner_ability_ura(
            &owner_ura,
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
        )
        .expect("runtime-governance receipt history Ability URA");
        format!(
            "{ability_ura}@1.0.0#aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!read"
        )
    }

    #[test]
    fn plain_remote_system_ability_is_bound_to_catalog_descriptor() {
        let owner_ura = device_system_agent_owner_ura(
            "realm",
            "node-a",
            crate::daemon::ability::names::device_control::LOCOMOTION_SYSTEM_AGENT_ID,
        );
        let ability_ura =
            crate::core::ura::owner_ability_ura(&owner_ura, "fs.read").expect("ability URA");
        let target = RemoteAbilityInvocationTarget::from_ability_ura(
            "easynet:///r/realm/device/node-a",
            &ability_ura,
        )
        .expect("target");
        let expected =
            crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
                &owner_ura,
                "fs.read",
                crate::daemon::ability::CallMode::Rpc,
            )
            .expect("descriptor");
        assert_eq!(target.descriptor_ref(), expected);
        assert_eq!(target.route_function_name(), "fs.read");
        assert_ne!(target.route_function_name(), target.as_str());
    }

    #[test]
    fn federation_system_abilities_are_bound_to_hub_catalog_descriptors() {
        let hub = "easynet:///r/realm/authority";
        for ability in ["federation.discover", "federation.revoke"] {
            let target = RemoteAbilityInvocationTarget::for_target_owned_selector(hub, ability)
                .expect("target");
            let expected =
                crate::daemon::axon_bridge::descriptor_ref::system_protocol_descriptor_ref_for_wire(
                    hub,
                    ability,
                    crate::daemon::ability::CallMode::Rpc,
                )
                .expect("descriptor");

            assert_eq!(target.callee_ura(), hub);
            assert_eq!(target.descriptor_ref(), expected);
            assert_eq!(
                target.route_function_name(),
                ability,
                "federation system ability dispatch must carry the public route function, not the Ability URA"
            );
            assert_ne!(
                target.route_function_name(),
                target.as_str(),
                "route table function_name must not be the canonical Ability URA"
            );
            assert_ne!(target.descriptor_ref(), ability);
            assert!(
                target.descriptor_ref().contains('@'),
                "descriptor ref must include descriptor version"
            );
        }
    }

    #[test]
    fn federation_revoke_targets_realm_authority_not_local_device_registry() {
        let target = federation_revoke_authority_target("easynet:///r/realm/device/local-device")
            .expect("target");

        assert_eq!(target.callee_ura(), "easynet:///r/realm/authority");
        assert_eq!(target.route_function_name(), "federation.revoke");
        assert!(
            target
                .descriptor_ref()
                .contains("easynet:///r/realm/ability/authority.federation.revoke"),
            "descriptor ref must bind authority-owned federation.revoke, got {}",
            target.descriptor_ref()
        );
    }

    #[test]
    fn explicit_descriptor_version_is_preserved() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        assert_eq!(target.descriptor_ref(), descriptor.as_str());
        assert_eq!(target.route_function_name(), "observe.health");
    }

    #[test]
    fn remote_execution_validation_accepts_canonical_authority_owner_kind() {
        let selector = crate::core::ura::AbilitySelector::parse(
            "easynet:///r/realm/ability/authority.federation.status",
        )
        .expect("authority selector");

        validate_remote_execution_target("easynet:///r/realm/authority", &selector)
            .expect("authority-owned ability should execute on its Authority owner");
    }

    #[test]
    fn remote_execution_validation_accepts_canonical_service_owner_kind() {
        let service_owner = crate::core::ura::service_ura("realm", "user-a", "pages");
        let ability = crate::core::ura::owner_ability_ura(&service_owner, "project_list")
            .expect("Service ability URA");
        let selector =
            crate::core::ura::AbilitySelector::parse(&ability).expect("Service ability selector");

        validate_remote_execution_target(&service_owner, &selector)
            .expect("service-owned ability should execute through its Service callee");
        validate_remote_execution_target("easynet:///r/realm/device/node-a", &selector)
            .expect("service-owned ability may use a Device placement before route resolution");
    }

    #[test]
    fn remote_execution_validation_rejects_authority_owner_on_device_target() {
        let selector = crate::core::ura::AbilitySelector::parse(
            "easynet:///r/realm/ability/authority.federation.status",
        )
        .expect("authority selector");

        let error = validate_remote_execution_target("easynet:///r/realm/device/node-a", &selector)
            .expect_err("authority-owned ability must not execute on a Device target");

        let message = error.to_string();
        assert!(message.contains("authority-owned ability URA"), "{message}");
        assert!(
            message.contains("requires an Authority execution target"),
            "{message}"
        );
    }

    #[test]
    fn remote_execution_validation_rejects_direct_device_owned_ability_on_device_target() {
        let selector = crate::core::ura::AbilitySelector::parse(
            "easynet:///r/realm/ability/device.node-a.fs.read",
        )
        .expect("legacy device-owned selector");

        let error = validate_remote_execution_target("easynet:///r/realm/device/node-a", &selector)
            .expect_err("direct Device-owned ability must not be a remote callee");

        let message = error.to_string();
        assert!(
            message.contains("direct Device-owned ability URA")
                && message.contains("migration-only")
                && message.contains("device-sponsored SystemAgent"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn target_owned_selector_projects_device_target_to_system_agent_callee() {
        let target = RemoteAbilityInvocationTarget::for_target_owned_selector(
            "easynet:///r/realm/device/node-a",
            "node.describe",
        )
        .expect("target-owned SystemAgent target");

        assert_eq!(
            target.execution_target_ura(),
            "easynet:///r/realm/device/node-a"
        );
        assert_eq!(
            target.callee_ura(),
            "easynet:///r/realm/agent/device.node-a.node-management"
        );
        assert_eq!(
            target.as_str(),
            "easynet:///r/realm/ability/system-agent.node-a.node-management.node.describe"
        );

        let subject =
            target_owned_remote_system_subject(&target).expect("SystemAgent target-owned subject");
        assert_eq!(subject.policy_name(), "DaemonTargetOwned");
        assert_eq!(
            subject.resolve().expect("subject URA"),
            "easynet:///r/realm/device/node-a"
        );
    }

    #[test]
    fn target_owned_terminal_selector_preserves_owner_ability_boundary() {
        let target = RemoteAbilityInvocationTarget::for_target_owned_selector(
            "easynet:///r/realm/device/node-a",
            "terminal.create",
        )
        .expect("terminal SystemAgent target");

        assert_eq!(
            target.callee_ura(),
            "easynet:///r/realm/agent/device.node-a.terminal"
        );
        assert_eq!(
            target.as_str(),
            "easynet:///r/realm/ability/system-agent.node-a.terminal.terminal.create"
        );
    }

    #[test]
    fn target_owned_file_transfer_binds_bidi_descriptor() {
        let target = RemoteAbilityInvocationTarget::for_target_owned_selector_for_mode(
            "easynet:///r/realm/device/node-a",
            crate::daemon::ability::names::device_control::FS_TRANSFER,
            CallMode::Bidi,
        )
        .expect("file-transfer SystemAgent bidi target");

        assert_eq!(target.public_ability(), "fs.transfer");
        assert!(target.descriptor_ref().ends_with("!stream"));
    }

    #[test]
    fn remote_request_preserves_explicit_tuple_facts() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let request = RemoteInvocationRequest::new(
            &target,
            "easynet:///r/realm/device/caller",
            "easynet:///r/realm/resource/device.node-a/probe/alive",
            [0x42; 16],
            CausalContext::None,
            json!({"probe": true}),
            Duration::from_secs(7),
        )
        .expect("complete request");

        assert_eq!(request.caller_ura, "easynet:///r/realm/device/caller");
        assert_eq!(
            request.subject_ura,
            "easynet:///r/realm/resource/device.node-a/probe/alive"
        );
        assert_eq!(request.invocation_nonce, [0x42; 16]);
        assert_eq!(request.causal_context, CausalContext::None);
        assert_eq!(request.timeout, Duration::from_secs(7));
    }

    #[test]
    fn public_tuple_plan_preserves_explicit_tuple_facts() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let causal_context = CausalContext::None;
        let plan = RemoteInvocationTuplePlan::public_explicit(
            &target,
            "easynet:///r/realm/device/caller",
            "easynet:///r/realm/resource/user.alice/invoke/echo",
            [0x41; 16],
            causal_context.clone(),
            json!({"probe": true}),
            Duration::from_secs(7),
        )
        .expect("tuple plan");

        assert_eq!(plan.subject.policy_name(), "CallerDeclared");
        assert_eq!(plan.nonce, RemoteInvocationNonce::Explicit([0x41; 16]));
        assert_eq!(
            plan.causal_context,
            InvocationCausalContext::explicit(causal_context)
        );

        let request = plan.into_request().expect("request");
        assert_eq!(request.caller_ura, "easynet:///r/realm/device/caller");
        assert_eq!(
            request.subject_ura,
            "easynet:///r/realm/resource/user.alice/invoke/echo"
        );
        assert_eq!(request.invocation_nonce, [0x41; 16]);
        assert_eq!(request.causal_context, CausalContext::None);
    }

    #[test]
    fn public_tuple_plan_rejects_receipt_history_before_request_construction() {
        let descriptor = receipt_history_descriptor_ref();
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("descriptor-bound history target");

        let error = RemoteInvocationTuplePlan::public_explicit(
            &target,
            "easynet:///r/realm/device/caller",
            "easynet:///r/realm/resource/user.alice/runtime-state/read",
            [0x41; 16],
            CausalContext::None,
            json!({"limit": 25}),
            Duration::from_secs(7),
        )
        .expect_err("receipt history must use the canonical history read path");

        let message = error.to_string();
        assert!(
            message.contains("receipt history ability `invocation.history.list`")
                && message.contains("not a public remote action")
                && message.contains("canonical invocation history read path"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn remote_invoke_response_rejects_unknown_wire_state_without_fallback_label() {
        let body = InvokeResponse {
            state: 440,
            ..InvokeResponse::default()
        };

        let error = ensure_completed_invoke_response("remote.test", &body)
            .expect_err("unknown wire state must fail as a protocol violation");
        let message = error.to_string();
        assert!(
            message.contains("unknown InvocationState wire value `440`"),
            "unexpected error: {message}"
        );
        assert!(
            !message.contains("UNKNOWN_STATE_"),
            "unknown wire state must not be projected as a product state: {message}"
        );
    }

    #[test]
    fn typed_remote_invoke_response_rejects_unknown_wire_state_without_fallback_label() {
        let body = InvokeResponse {
            state: 440,
            ..InvokeResponse::default()
        };

        let error = ensure_completed_invoke_response_typed(&body)
            .expect_err("unknown wire state must fail as a protocol violation");
        assert!(
            matches!(error, RemoteInvocationFailure::ProtocolViolation(_)),
            "unexpected typed failure: {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("unknown InvocationState wire value `440`"),
            "unexpected error: {message}"
        );
        assert!(
            !message.contains("UNKNOWN_STATE_"),
            "unknown wire state must not be projected as a product state: {message}"
        );
    }

    #[test]
    fn public_tuple_plan_preserves_explicit_causal_context() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let causal_context = CausalContext::Merkle {
            root: [0x71; 32],
            proof_ura: "easynet:///r/realm/resource/user.alice/proof/causal".to_string(),
        };
        let request = RemoteInvocationTuplePlan::public_explicit(
            &target,
            "easynet:///r/realm/device/caller",
            "easynet:///r/realm/resource/user.alice/task/child",
            [0x51; 16],
            causal_context.clone(),
            Value::Null,
            Duration::from_secs(7),
        )
        .expect("tuple plan")
        .into_request()
        .expect("request");

        assert_eq!(request.causal_context, causal_context);
    }

    #[test]
    fn remote_system_issuer_names_system_root_derivation() {
        let target = RemoteAbilityInvocationTarget::for_target_owned_selector(
            "easynet:///r/realm/authority",
            "federation.resolve",
        )
        .expect("target");
        let plan = RemoteSystemInvocationIssuer::target_owned_root_plan(
            &target,
            "easynet:///r/realm/device/caller",
            Value::Null,
            Duration::from_secs(30),
        )
        .expect("tuple plan");

        assert_eq!(plan.subject.policy_name(), "DaemonTargetOwned");
        assert_eq!(
            plan.causal_context,
            InvocationCausalContext::daemon_system_root()
        );
        assert_ne!(plan.nonce.derive(), [0; 16]);

        let request = plan.into_request().expect("request");
        assert_eq!(request.subject_ura, target.as_str());
        assert_ne!(request.invocation_nonce, [0; 16]);
        assert_eq!(request.causal_context, CausalContext::None);
    }

    #[test]
    fn remote_user_action_issuer_preserves_user_caller_and_declared_subject() {
        let target = RemoteAbilityInvocationTarget::for_target_owned_selector(
            "easynet:///r/realm/device/node-a",
            crate::daemon::ability::names::federation::ABILITY_DEPLOY,
        )
        .expect("ability-management target");
        let caller = "easynet:///r/realm/user/alice";
        let subject = "easynet:///r/realm/resource/user.alice/staged/ability-bundle";
        let plan = RemoteUserActionInvocationIssuer::caller_declared_root_plan(
            &target,
            caller,
            subject,
            json!({}),
            Duration::from_secs(30),
        )
        .expect("User-action tuple plan");

        assert_eq!(plan.subject.policy_name(), "CallerDeclared");
        assert_ne!(plan.nonce.derive(), [0; 16]);
        let request = plan.into_request().expect("request");
        assert_eq!(request.caller_ura, caller);
        assert_eq!(request.subject_ura, subject);
        assert_eq!(request.causal_context, CausalContext::None);
        assert_eq!(
            target.callee_ura(),
            "easynet:///r/realm/agent/device.node-a.ability-management"
        );
    }

    #[test]
    fn remote_catalogue_read_issuer_uses_read_projection_subject() {
        let target =
            RemoteAbilityInvocationTarget::for_catalogue_read("easynet:///r/realm/device/node-a")
                .expect("catalogue target");
        let plan = RemoteCatalogueReadIssuer::catalogue_read_plan(
            &target,
            "easynet:///r/realm/device/caller",
            json!({}),
            Duration::from_secs(30),
        )
        .expect("catalogue read tuple plan");

        assert_eq!(plan.subject.policy_name(), "RuntimeReadProjection");
        assert_eq!(
            plan.causal_context,
            InvocationCausalContext::daemon_system_root()
        );
        assert_ne!(plan.nonce.derive(), [0; 16]);

        let request = plan.into_request().expect("request");
        assert_eq!(request.subject_ura, target.execution_target_ura());
        assert_eq!(
            target.callee_ura(),
            "easynet:///r/realm/agent/device.node-a.runtime-introspection"
        );
        assert_ne!(request.invocation_nonce, [0; 16]);
        assert_eq!(request.causal_context, CausalContext::None);
    }

    #[test]
    fn remote_catalogue_read_issuer_rejects_non_catalogue_target() {
        let target = RemoteAbilityInvocationTarget::for_target_owned_selector(
            "easynet:///r/realm/device/node-a",
            "node.describe",
        )
        .expect("system target");

        let error = RemoteCatalogueReadIssuer::catalogue_read_plan(
            &target,
            "easynet:///r/realm/device/caller",
            json!({}),
            Duration::from_secs(30),
        )
        .expect_err("non-catalogue target must not enter catalogue read issuer");

        assert!(
            error
                .to_string()
                .contains("remote catalogue read issuer requires"),
            "wrong error: {error}"
        );
    }

    #[test]
    fn target_owned_selector_rejects_receipt_history_before_tuple_build() {
        let error = RemoteAbilityInvocationTarget::for_target_owned_selector(
            "easynet:///r/realm/device/node-a",
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
        )
        .expect_err("receipt history must not construct a target-owned selector");

        let message = error.to_string();
        assert!(
            message.contains("receipt history ability `invocation.history.list`")
                && message.contains("canonical invocation history read path"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn remote_system_issuer_rejects_receipt_history_as_target_owned() {
        let owner_ura = device_system_agent_owner_ura(
            "realm",
            "node-a",
            crate::daemon::ability::names::governance::RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID,
        );
        let ability_ura = crate::core::ura::owner_ability_ura(
            &owner_ura,
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
        )
        .expect("runtime-governance receipt history Ability URA");
        let target = RemoteAbilityInvocationTarget::from_ability_ura(
            "easynet:///r/realm/device/node-a",
            &ability_ura,
        )
        .expect("explicit target");

        let error = RemoteSystemInvocationIssuer::target_owned_root_plan(
            &target,
            "easynet:///r/realm/device/caller",
            json!({"limit": 5}),
            Duration::from_secs(30),
        )
        .expect_err("receipt history must not use target-owned remote system dispatch");

        let message = error.to_string();
        assert!(
            message.contains("receipt history ability `invocation.history.list`")
                && message.contains("canonical invocation history read path"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn federation_revoke_caller_must_match_active_local_daemon() {
        let caller = canonical_federation_revoke_caller(
            "easynet:///r/realm/device/local",
            "easynet:///r/realm/device/local",
        )
        .expect("matching caller");

        assert_eq!(caller, "easynet:///r/realm/device/local");

        let error = canonical_federation_revoke_caller(
            "easynet:///r/realm/device/stale",
            "easynet:///r/realm/device/local",
        )
        .expect_err("stale caller must not be repaired from ambient daemon state");

        let message = error.to_string();
        assert!(
            message.contains("does not match active local daemon"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn federation_revoke_caller_rejects_non_runtime_owner() {
        let error = canonical_federation_revoke_caller(
            "easynet:///r/realm/resource/user.alice/runtime-state/read",
            "easynet:///r/realm/resource/user.alice/runtime-state/read",
        )
        .expect_err("revoke caller must be a daemon owner identity");

        let message = error.to_string();
        assert!(
            message.contains("must be a Device or Authority URA"),
            "wrong error: {message}"
        );
    }

    #[test]
    fn tuple_plan_rejects_hidden_invalid_defaults() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");

        let bad_subject = RemoteInvocationTuplePlan::public_explicit(
            &target,
            "easynet:///r/realm/device/caller",
            "",
            [0x52; 16],
            CausalContext::None,
            Value::Null,
            Duration::from_secs(7),
        );
        assert!(bad_subject.is_err());

        let zero_timeout = RemoteInvocationTuplePlan::public_explicit(
            &target,
            "easynet:///r/realm/device/caller",
            target.callee_ura().to_string(),
            [0x53; 16],
            CausalContext::None,
            Value::Null,
            Duration::ZERO,
        );
        assert!(zero_timeout.is_err());
    }

    #[test]
    fn remote_tuple_rejects_all_zero_principals_before_signer_or_transport() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");
        let placeholder = "00000000-0000-0000-0000-000000000000";

        for (field, caller, subject) in [
            (
                "caller",
                crate::core::ura::user_ura("realm", placeholder),
                target.callee_ura().to_string(),
            ),
            (
                "caller-declared subject",
                "easynet:///r/realm/device/caller".to_string(),
                crate::core::ura::resource_dot_ura(
                    "realm",
                    &format!("user.{placeholder}"),
                    "task/read",
                ),
            ),
        ] {
            let error = RemoteInvocationTuplePlan::public_explicit(
                &target,
                caller,
                subject,
                [0x54; 16],
                CausalContext::None,
                Value::Null,
                Duration::from_secs(7),
            )
            .expect_err("all-zero principal must fail at remote tuple construction");
            let message = error.to_string();
            assert!(
                message.contains(field) && message.contains("all-zero principal placeholder"),
                "wrong {field} error: {message}"
            );
        }
    }

    #[test]
    fn federation_discover_rejects_all_zero_user_filter_before_daemon_io() {
        let error =
            invoke_federation_discover_for_user(None, crate::core::identity::ALL_ZERO_PRINCIPAL_ID)
                .expect_err("all-zero user filter must reject before local daemon transport");
        assert!(error.to_string().contains("all-zero principal"));
    }

    #[test]
    fn federation_discover_user_scope_binds_user_caller_before_daemon_io() {
        let local_daemon_ura = crate::core::ura::device_ura("acme", "device-a");
        let scope = FederationDiscoverScope::user(&local_daemon_ura, "user-a").expect("user scope");

        assert_eq!(
            scope.query_target_ura(),
            crate::core::ura::authority_ura("acme"),
            "user-scoped directory reads must target the realm Authority"
        );
        assert_eq!(
            scope.caller_ura(),
            crate::core::ura::user_ura("acme", "user-a")
        );
        assert_eq!(
            scope.subject_ura(),
            crate::core::ura::resource_dot_ura("acme", "user.user-a", "directory/devices"),
            "user-scoped discovery must act on the caller-owned directory projection"
        );
        assert_eq!(
            scope.local_user_id_filter.as_deref(),
            Some("user-a"),
            "user-scoped discover must carry the same local user filter"
        );
        assert_ne!(
            scope.caller_ura(),
            local_daemon_ura,
            "user-scoped discover must not sign as the daemon/device owner"
        );
    }

    #[test]
    fn federation_discover_operator_scope_binds_daemon_caller_without_user_filter() {
        let local_daemon_ura = crate::core::ura::hub_ura("acme");
        let scope = FederationDiscoverScope::operator_audit(&local_daemon_ura)
            .expect("operator/audit scope");

        assert_eq!(scope.caller_ura(), local_daemon_ura);
        assert_eq!(
            scope.subject_ura(),
            crate::core::ura::owner_ability_ura(&local_daemon_ura, "federation.discover")
                .expect("Authority discover subject")
        );
        assert_eq!(
            scope.query_target_ura(),
            local_daemon_ura,
            "operator/audit reads remain local to their explicitly selected Authority"
        );
        assert_eq!(scope.local_user_id_filter, None);
    }

    #[test]
    fn remote_request_rejects_incomplete_tuple_facts() {
        let descriptor = health_descriptor_ref("realm");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/realm/device/node-a",
            &descriptor,
        )
        .expect("target");

        let missing_caller = RemoteInvocationRequest::new(
            &target,
            "",
            target.callee_ura(),
            [0x11; 16],
            CausalContext::None,
            Value::Null,
            Duration::from_secs(1),
        );
        assert!(missing_caller.is_err());

        let zero_nonce = RemoteInvocationRequest::new(
            &target,
            "easynet:///r/realm/device/caller",
            target.callee_ura(),
            [0; 16],
            CausalContext::None,
            Value::Null,
            Duration::from_secs(1),
        );
        assert!(zero_nonce.is_err());
    }

    fn signer_first_test_request<'a>(
        target: &'a RemoteAbilityInvocationTarget,
    ) -> RemoteInvocationRequest<'a> {
        RemoteInvocationRequest::new(
            target,
            "easynet:///r/signer-first-test/device/missing-caller",
            "easynet:///r/signer-first-test/resource/device.node-a/probe/signer-first",
            [0x61; 16],
            CausalContext::None,
            json!({"probe": true}),
            Duration::from_secs(1),
        )
        .expect("signer-first request")
    }

    fn assert_signer_first_error(error: anyhow::Error, label: &str) {
        let message = error.to_string();
        assert!(
            message.contains(label),
            "remote carrier must name signer custody stage: {message}"
        );
        assert!(
            message.contains("requires a caller signer"),
            "remote carrier must fail at caller signer readiness: {message}"
        );
        assert!(
            !message.contains("daemon not running"),
            "caller signer readiness must run before daemon socket probe: {message}"
        );
        assert!(
            !message.contains("keyring entry not found")
                && !message.contains("self-identity:")
                && !message.contains("keyring rejected request"),
            "caller signer readiness must not expose keyring implementation details: {message}"
        );
    }

    #[test]
    fn remote_unary_loads_caller_signer_before_daemon_socket_probe() {
        let descriptor = health_descriptor_ref("signer-first-test");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/signer-first-test/device/node-a",
            &descriptor,
        )
        .expect("target");
        let error = invoke_remote_target(signer_first_test_request(&target))
            .expect_err("missing signer must fail before daemon socket readiness");

        assert_signer_first_error(error, "remote invocation");
    }

    #[test]
    fn remote_stream_loads_caller_signer_before_daemon_socket_probe() {
        let descriptor = health_descriptor_ref("signer-first-test");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/signer-first-test/device/node-a",
            &descriptor,
        )
        .expect("target");
        let error = invoke_remote_target_stream(signer_first_test_request(&target), None)
            .expect_err("missing signer must fail before stream daemon socket readiness");

        assert_signer_first_error(error, "remote stream invocation");
    }

    #[test]
    fn remote_bidi_loads_caller_signer_before_daemon_socket_probe() {
        let descriptor = health_descriptor_ref("signer-first-test");
        let target = RemoteAbilityInvocationTarget::from_descriptor_ref(
            "easynet:///r/signer-first-test/device/node-a",
            &descriptor,
        )
        .expect("target");
        let error = invoke_remote_target_bidi_json_frames(
            signer_first_test_request(&target),
            Vec::new(),
            None,
        )
        .expect_err("missing signer must fail before bidi daemon socket readiness");

        assert_signer_first_error(error, "remote bidi invocation");
    }

    #[test]
    fn remote_bidi_frame_chain_mac_rejects_missing_open_envelope() {
        let error = remote_bidi_frame_chain_mac(&axon_sdk::pb::axon::v1::EnvelopeOpen::default())
            .expect_err("remote bidi open must contain an envelope before daemon IO");

        assert!(
            error
                .to_string()
                .contains("remote bidi builder omitted envelope"),
            "wrong error: {error:#}"
        );
    }

    #[test]
    fn remote_bidi_frame_chain_mac_rejects_missing_caller_signature() {
        let open = axon_sdk::pb::axon::v1::EnvelopeOpen {
            envelope: Some(axon_sdk::pb::axon::v1::Envelope::default()),
            ..axon_sdk::pb::axon::v1::EnvelopeOpen::default()
        };

        let error = remote_bidi_frame_chain_mac(&open)
            .expect_err("remote bidi open must contain caller signature bytes");

        assert!(
            error
                .to_string()
                .contains("remote bidi builder omitted caller signature"),
            "wrong error: {error:#}"
        );
    }

    #[test]
    fn remote_bidi_frame_chain_mac_rejects_empty_caller_signature() {
        let open = axon_sdk::pb::axon::v1::EnvelopeOpen {
            envelope: Some(axon_sdk::pb::axon::v1::Envelope {
                caller_signature: Some(axon_sdk::pb::axon::v1::CallerSignature {
                    algorithm: "ed25519".to_string(),
                    signature: Vec::new(),
                    key_id_hint: "test-signer".to_string(),
                }),
                ..axon_sdk::pb::axon::v1::Envelope::default()
            }),
            ..axon_sdk::pb::axon::v1::EnvelopeOpen::default()
        };

        let error = remote_bidi_frame_chain_mac(&open)
            .expect_err("remote bidi open must not use an empty MAC seed");

        assert!(
            error
                .to_string()
                .contains("remote bidi builder produced empty caller signature"),
            "wrong error: {error:#}"
        );
    }

    #[test]
    fn remote_bidi_frame_chain_mac_uses_caller_signature_bytes() {
        let signature = vec![0x5a; 64];
        let open = axon_sdk::pb::axon::v1::EnvelopeOpen {
            envelope: Some(axon_sdk::pb::axon::v1::Envelope {
                caller_signature: Some(axon_sdk::pb::axon::v1::CallerSignature {
                    algorithm: "ed25519".to_string(),
                    signature: signature.clone(),
                    key_id_hint: "test-signer".to_string(),
                }),
                ..axon_sdk::pb::axon::v1::Envelope::default()
            }),
            ..axon_sdk::pb::axon::v1::EnvelopeOpen::default()
        };

        assert_eq!(remote_bidi_frame_chain_mac(&open).unwrap(), signature);
    }

    #[test]
    fn federation_discover_authority_subject_uses_ability_subject_and_device_self_subject() {
        let hub = crate::core::ura::hub_ura("acme");
        assert_eq!(
            federation_discover_authority_subject_ura(&hub).expect("hub subject"),
            crate::core::ura::owner_ability_ura(&hub, "federation.discover")
                .expect("realm Authority ability subject")
        );

        let device = crate::core::ura::device_ura("acme", "device-a");
        assert_eq!(
            federation_discover_authority_subject_ura(&device).expect("device subject"),
            device
        );
    }

    fn purge_revoke_command(
    ) -> crate::daemon::invocation::dispatch::federation_wrappers::RevokeRequest {
        crate::daemon::invocation::dispatch::federation_wrappers::RevokeRequest {
            agent_ura: "easynet:///r/acme/agent/user.worker".to_string(),
            purge_transaction_id: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            generation: Some(7),
            reason: Some("agent.stop".to_string()),
            authority_ura: Some("easynet:///r/acme/device/device-a".to_string()),
            protocol_version: Some(
                crate::daemon::persistence::federation_revoke::REVOKE_PROTOCOL_VERSION,
            ),
            delivery_fence: Some(3),
        }
    }

    fn revoke_receipt_bytes(
        ack: bool,
        transaction_id: Option<&str>,
        disposition: Option<
            crate::daemon::persistence::federation_revoke::FederationRevokeDisposition,
        >,
    ) -> Vec<u8> {
        serde_json::to_vec(
            &crate::daemon::invocation::dispatch::federation_wrappers::RevokeResponse {
                ack,
                was_active: true,
                purge_transaction_id: transaction_id.map(str::to_string),
                replayed: false,
                disposition,
            },
        )
        .unwrap()
    }

    #[test]
    fn federation_revoke_receipt_proves_exact_transaction_and_retirement() {
        use crate::daemon::persistence::federation_revoke::FederationRevokeDisposition;

        let command = purge_revoke_command();
        for disposition in [
            FederationRevokeDisposition::Retired,
            FederationRevokeDisposition::AlreadyRetired,
        ] {
            let receipt = decode_and_verify_federation_revoke_response(
                &command,
                &revoke_receipt_bytes(
                    true,
                    command.purge_transaction_id.as_deref(),
                    Some(disposition),
                ),
            )
            .expect("exact durable retirement receipt");
            assert_eq!(receipt.disposition, Some(disposition));
        }
    }

    #[test]
    fn federation_revoke_receipt_rejects_unproven_or_superseded_retirement() {
        use crate::daemon::persistence::federation_revoke::FederationRevokeDisposition;

        let command = purge_revoke_command();
        let cases = [
            revoke_receipt_bytes(
                false,
                command.purge_transaction_id.as_deref(),
                Some(FederationRevokeDisposition::Retired),
            ),
            revoke_receipt_bytes(
                true,
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                Some(FederationRevokeDisposition::Retired),
            ),
            revoke_receipt_bytes(
                true,
                command.purge_transaction_id.as_deref(),
                Some(FederationRevokeDisposition::SupersededByNewIncarnation),
            ),
            revoke_receipt_bytes(true, command.purge_transaction_id.as_deref(), None),
        ];
        for bytes in cases {
            assert!(decode_and_verify_federation_revoke_response(&command, &bytes).is_err());
        }
    }

    #[test]
    fn federation_revoke_receipt_rejects_unknown_fields() {
        let command = purge_revoke_command();
        let bytes = serde_json::to_vec(&json!({
            "ack": true,
            "was_active": true,
            "purge_transaction_id": command.purge_transaction_id,
            "disposition": "retired",
            "unbound_generation": 7,
        }))
        .unwrap();

        assert!(decode_and_verify_federation_revoke_response(&command, &bytes).is_err());
    }
}
