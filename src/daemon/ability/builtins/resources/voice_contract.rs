//! EasyNet voice product contract.
//!
//! Axon owns generic invocation and capability semantics. Voice signaling is
//! an EasyNet product aggregate, so its states and telemetry validation live
//! beside the daemon handler that owns the call lifecycle.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VoiceCallState {
    Ringing,
    Active,
    Ended,
}

impl VoiceCallState {
    pub(super) fn wire_name(self) -> &'static str {
        match self {
            Self::Ringing => "VOICE_CALL_STATE_RINGING",
            Self::Active => "VOICE_CALL_STATE_ACTIVE",
            Self::Ended => "VOICE_CALL_STATE_ENDED",
        }
    }

    pub(super) fn to_wire_i32(self) -> i32 {
        match self {
            Self::Ringing => 1,
            Self::Active => 2,
            Self::Ended => 5,
        }
    }

    pub(super) fn is_terminal(self) -> bool {
        matches!(self, Self::Ended)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VoiceEndReason {
    Unspecified,
    CallerHangup,
    Timeout,
    MediaFailure,
    PolicyDenied,
    AdminForced,
}

impl VoiceEndReason {
    pub(super) fn wire_name(self) -> &'static str {
        match self {
            Self::Unspecified => "VOICE_END_UNSPECIFIED",
            Self::CallerHangup => "VOICE_END_CALLER_HANGUP",
            Self::Timeout => "VOICE_END_TIMEOUT",
            Self::MediaFailure => "VOICE_END_MEDIA_FAILURE",
            Self::PolicyDenied => "VOICE_END_POLICY_DENIED",
            Self::AdminForced => "VOICE_END_ADMIN_FORCED",
        }
    }

    pub(super) fn to_wire_i32(self) -> i32 {
        match self {
            Self::Unspecified => 0,
            Self::CallerHangup => 1,
            Self::Timeout => 2,
            Self::MediaFailure => 3,
            Self::PolicyDenied => 4,
            Self::AdminForced => 5,
        }
    }

    pub(super) fn from_wire(raw: i64) -> anyhow::Result<Self> {
        match raw {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::CallerHangup),
            2 => Ok(Self::Timeout),
            3 => Ok(Self::MediaFailure),
            4 => Ok(Self::PolicyDenied),
            5 => Ok(Self::AdminForced),
            _ => anyhow::bail!("unknown VoiceEndReason wire value {raw}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VoiceEventType {
    ParticipantJoin,
    ParticipantLeave,
    MetricsReported,
    CallEnded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VoiceCallEvent {
    sequence: u64,
    command_id: String,
    event_type: VoiceEventType,
    event: Value,
    at_ms: u64,
}

impl VoiceCallEvent {
    fn to_json(&self) -> Value {
        let mut event = self.event.clone();
        if let Some(object) = event.as_object_mut() {
            object.insert("sequence".into(), json!(self.sequence));
            object.insert("command_id".into(), json!(self.command_id));
            object.insert("event_type".into(), json!(self.event_type.wire_name()));
            object.insert("at_ms".into(), json!(self.at_ms));
        }
        event
    }
}

impl VoiceEventType {
    pub(super) fn wire_name(self) -> &'static str {
        match self {
            Self::ParticipantJoin => "VOICE_EVENT_PARTICIPANT_JOIN",
            Self::ParticipantLeave => "VOICE_EVENT_PARTICIPANT_LEAVE",
            Self::MetricsReported => "VOICE_EVENT_METRICS_REPORTED",
            Self::CallEnded => "VOICE_EVENT_CALL_ENDED",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct VoiceNetworkMetrics {
    rtt_ms: f64,
    jitter_ms: f64,
    packet_loss_ratio: f64,
    concealed_samples: u32,
    audio_level_dbov: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VoiceParticipant {
    participant_id: String,
    state: VoiceParticipantState,
    sdp_offer: Option<String>,
    #[serde(default)]
    ice_candidates: Vec<Value>,
    last_metrics: Option<VoiceNetworkMetrics>,
    joined_at_ms: u64,
    left_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VoiceParticipantState {
    Joined,
    Left,
}

impl VoiceParticipant {
    fn new(participant_id: String, sdp_offer: Option<String>, joined_at_ms: u64) -> Self {
        Self {
            participant_id,
            state: VoiceParticipantState::Joined,
            sdp_offer,
            ice_candidates: Vec::new(),
            last_metrics: None,
            joined_at_ms,
            left_at_ms: None,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "participant_id": self.participant_id,
            "state": self.state,
            "sdp_offer": self.sdp_offer,
            "ice_candidates": self.ice_candidates,
            "last_metrics": self.last_metrics.as_ref().map(VoiceNetworkMetrics::to_json),
            "joined_at_ms": self.joined_at_ms,
            "left_at_ms": self.left_at_ms,
        })
    }
}

/// Durable realm Authority aggregate for one signaling call.
///
/// Every lifecycle mutation is expressed through this type. Persistence owns
/// transaction boundaries; handlers never mutate a detached process-local
/// projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceCallAggregate {
    authority_ura: String,
    call_id: String,
    state: VoiceCallState,
    created_at_ms: u64,
    ended_at_ms: Option<u64>,
    end_reason: Option<VoiceEndReason>,
    #[serde(default)]
    participants: BTreeMap<String, VoiceParticipant>,
    #[serde(default)]
    events: Vec<VoiceCallEvent>,
    revision: u64,
}

/// Realm-shared persistence port for the Authority-owned voice aggregate.
///
/// Implementations must make compare-and-swap atomic across every realm Authority replica
/// serving the realm. A process-local cache or per-replica file does not
/// satisfy this contract. `compare_and_swap` must reject a replacement unless
/// `replacement.revision() == expected_revision + 1`; equal, skipped, and
/// overflowing revisions fail closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCallRepositoryQualification {
    provider_id: String,
    durable: bool,
    realm_scoped: bool,
    linearizable_cas: bool,
    idempotent_commands: bool,
}

impl VoiceCallRepositoryQualification {
    pub(crate) fn production(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            durable: true,
            realm_scoped: true,
            linearizable_cas: true,
            idempotent_commands: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn unqualified(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            durable: false,
            realm_scoped: false,
            linearizable_cas: false,
            idempotent_commands: false,
        }
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn is_durable(&self) -> bool {
        self.durable
    }

    pub fn is_realm_scoped(&self) -> bool {
        self.realm_scoped
    }

    pub fn has_linearizable_cas(&self) -> bool {
        self.linearizable_cas
    }

    pub fn has_idempotent_commands(&self) -> bool {
        self.idempotent_commands
    }

    pub fn validate_production(&self) -> anyhow::Result<()> {
        if self.provider_id.trim().is_empty()
            || !self.durable
            || !self.realm_scoped
            || !self.linearizable_cas
            || !self.idempotent_commands
        {
            anyhow::bail!(
                "voice repository {:?} is not qualified for durable realm authority",
                self.provider_id
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VoiceCallCasOutcome {
    Committed(VoiceCallAggregate),
    Current(VoiceCallAggregate),
    Ambiguous,
}

pub trait VoiceCallRepository: Debug + Send + Sync {
    fn qualification(&self) -> VoiceCallRepositoryQualification;

    fn insert_if_absent(&self, aggregate: VoiceCallAggregate) -> anyhow::Result<bool>;

    fn load(
        &self,
        authority_ura: &str,
        call_id: &str,
    ) -> anyhow::Result<Option<VoiceCallAggregate>>;

    fn list(&self, authority_ura: &str) -> anyhow::Result<Vec<VoiceCallRepositoryEntry>>;

    fn compare_and_swap(
        &self,
        authority_ura: &str,
        call_id: &str,
        expected_revision: u64,
        replacement: VoiceCallAggregate,
    ) -> anyhow::Result<VoiceCallCasOutcome>;
}

/// Production-qualified Voice provider assembly.
///
/// Live route registration accepts this value instead of a raw repository so
/// unqualified providers cannot reach the Authority-owned call state machine.
#[derive(Clone, Debug)]
pub struct VoiceCallProviderAssembly {
    repository: Arc<dyn VoiceCallRepository>,
    qualification: VoiceCallRepositoryQualification,
}

impl VoiceCallProviderAssembly {
    pub fn try_new(repository: Arc<dyn VoiceCallRepository>) -> anyhow::Result<Self> {
        let qualification = repository.qualification();
        qualification.validate_production()?;
        Ok(Self {
            repository,
            qualification,
        })
    }

    pub fn repository(&self) -> Arc<dyn VoiceCallRepository> {
        Arc::clone(&self.repository)
    }

    pub fn qualification(&self) -> &VoiceCallRepositoryQualification {
        &self.qualification
    }
}

/// One repository-indexed row returned by a realm list operation.
///
/// The explicit storage key lets the service verify that a provider did not
/// substitute a different aggregate while preserving a valid payload.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceCallRepositoryEntry {
    authority_ura: String,
    call_id: String,
    aggregate: VoiceCallAggregate,
}

impl VoiceCallRepositoryEntry {
    pub fn new(authority_ura: String, call_id: String, aggregate: VoiceCallAggregate) -> Self {
        Self {
            authority_ura,
            call_id,
            aggregate,
        }
    }

    pub fn validate_and_into_aggregate(
        self,
        requested_authority_ura: &str,
    ) -> anyhow::Result<VoiceCallAggregate> {
        if self.authority_ura != requested_authority_ura {
            anyhow::bail!(
                "voice repository list key mismatch: requested authority {requested_authority_ura:?}, returned authority {:?}",
                self.authority_ura
            );
        }
        self.aggregate
            .validate_repository_key(&self.authority_ura, &self.call_id)?;
        Ok(self.aggregate)
    }
}

/// Shared deterministic repository used only by product state-machine tests.
/// It deliberately has no production visibility: realm deployments must
/// inject a provider whose compare-and-swap spans every realm Authority replica.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TestVoiceCallRepository {
    calls: std::sync::Arc<std::sync::Mutex<BTreeMap<(String, String), VoiceCallAggregate>>>,
}

#[cfg(test)]
impl VoiceCallRepository for TestVoiceCallRepository {
    fn qualification(&self) -> VoiceCallRepositoryQualification {
        VoiceCallRepositoryQualification::unqualified("test-in-memory")
    }

    fn insert_if_absent(&self, aggregate: VoiceCallAggregate) -> anyhow::Result<bool> {
        aggregate.validate_recovered()?;
        let key = (
            aggregate.authority_ura().to_string(),
            aggregate.call_id().to_string(),
        );
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if calls.contains_key(&key) {
            return Ok(false);
        }
        calls.insert(key, aggregate);
        Ok(true)
    }

    fn load(
        &self,
        authority_ura: &str,
        call_id: &str,
    ) -> anyhow::Result<Option<VoiceCallAggregate>> {
        let calls = self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(calls
            .get(&(authority_ura.to_string(), call_id.to_string()))
            .cloned())
    }

    fn list(&self, authority_ura: &str) -> anyhow::Result<Vec<VoiceCallRepositoryEntry>> {
        let calls = self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(calls
            .iter()
            .filter(|((authority, _), _)| authority == authority_ura)
            .map(|((authority, call_id), aggregate)| {
                VoiceCallRepositoryEntry::new(authority.clone(), call_id.clone(), aggregate.clone())
            })
            .collect())
    }

    fn compare_and_swap(
        &self,
        authority_ura: &str,
        call_id: &str,
        expected_revision: u64,
        replacement: VoiceCallAggregate,
    ) -> anyhow::Result<VoiceCallCasOutcome> {
        replacement.validate_cas_replacement(expected_revision)?;
        if replacement.authority_ura() != authority_ura || replacement.call_id() != call_id {
            anyhow::bail!("test voice replacement changed its aggregate key");
        }
        let key = (authority_ura.to_string(), call_id.to_string());
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(current) = calls.get_mut(&key) else {
            anyhow::bail!("voice CAS target does not exist");
        };
        if current.revision() != expected_revision {
            return Ok(VoiceCallCasOutcome::Current(current.clone()));
        }
        *current = replacement;
        Ok(VoiceCallCasOutcome::Committed(current.clone()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VoiceJoinOutcome {
    pub(crate) state: VoiceCallState,
    pub(crate) participant_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VoiceEndOutcome {
    pub(crate) state: VoiceCallState,
    pub(crate) end_reason: VoiceEndReason,
    pub(crate) already_ended: bool,
}

impl VoiceCallAggregate {
    pub(crate) fn new(
        authority_ura: String,
        call_id: String,
        creator_participant_id: Option<String>,
        created_at_ms: u64,
    ) -> Self {
        let mut participants = BTreeMap::new();
        if let Some(participant_id) = creator_participant_id {
            participants.insert(
                participant_id.clone(),
                VoiceParticipant::new(participant_id, None, created_at_ms),
            );
        }
        Self {
            authority_ura,
            call_id,
            state: VoiceCallState::Ringing,
            created_at_ms,
            ended_at_ms: None,
            end_reason: None,
            participants,
            events: Vec::new(),
            revision: 1,
        }
    }

    pub fn authority_ura(&self) -> &str {
        &self.authority_ura
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn bump_revision(&mut self) -> anyhow::Result<()> {
        let next = self.revision.checked_add(1).ok_or_else(|| {
            anyhow::anyhow!("voice aggregate {:?} revision overflow", self.call_id)
        })?;
        if next == u64::MAX {
            anyhow::bail!(
                "voice aggregate {:?} revision space is exhausted",
                self.call_id
            );
        }
        self.revision = next;
        Ok(())
    }

    pub(crate) fn join(
        &mut self,
        command_id: &str,
        participant_id: String,
        sdp_offer: Option<String>,
        at_ms: u64,
    ) -> anyhow::Result<VoiceJoinOutcome> {
        self.ensure_open("voice.join_call")?;
        if let Some(participant) = self.participants.get(&participant_id) {
            match participant.state {
                VoiceParticipantState::Joined => anyhow::bail!(
                    "voice.join_call: participant {participant_id:?} is already joined to call {:?}",
                    self.call_id
                ),
                VoiceParticipantState::Left => anyhow::bail!(
                    "voice.join_call: participant {participant_id:?} has already left call {:?} and cannot rejoin",
                    self.call_id
                ),
            }
        }
        self.participants.insert(
            participant_id.clone(),
            VoiceParticipant::new(participant_id.clone(), sdp_offer, at_ms),
        );
        let active_participant_count = self.active_participant_count();
        if active_participant_count >= 2 {
            self.state = VoiceCallState::Active;
        }
        self.append_event(
            command_id,
            VoiceEventType::ParticipantJoin,
            at_ms,
            json!({
                "type": "joined",
                "participant_id": participant_id,
                "state": self.state.wire_name(),
                "state_code": self.state.to_wire_i32(),
            }),
        )?;
        Ok(VoiceJoinOutcome {
            state: self.state,
            participant_count: active_participant_count,
        })
    }

    pub(crate) fn leave(
        &mut self,
        command_id: &str,
        participant_id: &str,
        reason: String,
        at_ms: u64,
    ) -> anyhow::Result<()> {
        self.ensure_open("voice.leave_call")?;
        let participant = self.participants.get_mut(participant_id).ok_or_else(|| {
            anyhow::anyhow!(
                "voice.leave_call: participant {participant_id:?} not in call {:?}",
                self.call_id
            )
        })?;
        match participant.state {
            VoiceParticipantState::Joined => {
                participant.state = VoiceParticipantState::Left;
                participant.left_at_ms = Some(at_ms);
            }
            VoiceParticipantState::Left => anyhow::bail!(
                "voice.leave_call: participant {participant_id:?} has already left call {:?}",
                self.call_id
            ),
        }
        if self.active_participant_count() < 2 {
            self.state = VoiceCallState::Ringing;
        }
        self.append_event(
            command_id,
            VoiceEventType::ParticipantLeave,
            at_ms,
            json!({
                "type": "left",
                "participant_id": participant_id,
                "reason": reason,
                "state": self.state.wire_name(),
                "state_code": self.state.to_wire_i32(),
            }),
        )?;
        Ok(())
    }

    pub(crate) fn end(
        &mut self,
        command_id: &str,
        end_reason: VoiceEndReason,
        at_ms: u64,
    ) -> anyhow::Result<VoiceEndOutcome> {
        if self.state.is_terminal() {
            return Ok(VoiceEndOutcome {
                state: self.state,
                end_reason: self.end_reason.unwrap_or(end_reason),
                already_ended: true,
            });
        }
        self.state = VoiceCallState::Ended;
        self.ended_at_ms = Some(at_ms);
        self.end_reason = Some(end_reason);
        self.append_event(
            command_id,
            VoiceEventType::CallEnded,
            at_ms,
            json!({
                "type": "ended",
                "end_reason": end_reason.wire_name(),
                "end_reason_code": end_reason.to_wire_i32(),
                "state": self.state.wire_name(),
                "state_code": self.state.to_wire_i32(),
            }),
        )?;
        Ok(VoiceEndOutcome {
            state: self.state,
            end_reason,
            already_ended: false,
        })
    }

    pub(crate) fn report_metrics(
        &mut self,
        command_id: &str,
        participant_id: &str,
        metrics: VoiceNetworkMetrics,
        at_ms: u64,
    ) -> anyhow::Result<()> {
        self.ensure_open("voice.report_metrics")?;
        let participant = self.participants.get_mut(participant_id).ok_or_else(|| {
            anyhow::anyhow!(
                "voice.report_metrics: participant {participant_id:?} not in call {:?}",
                self.call_id
            )
        })?;
        if participant.state == VoiceParticipantState::Left {
            anyhow::bail!(
                "voice.report_metrics: participant {participant_id:?} has already left call {:?}",
                self.call_id
            );
        }
        participant.last_metrics = Some(metrics.clone());
        self.append_event(
            command_id,
            VoiceEventType::MetricsReported,
            at_ms,
            json!({
                "type": "metrics",
                "participant_id": participant_id,
                "metrics": metrics.to_json(),
                "state": self.state.wire_name(),
                "state_code": self.state.to_wire_i32(),
            }),
        )?;
        Ok(())
    }

    fn append_event(
        &mut self,
        command_id: &str,
        event_type: VoiceEventType,
        at_ms: u64,
        event: Value,
    ) -> anyhow::Result<()> {
        if command_id.trim().is_empty() {
            anyhow::bail!("voice mutation command_id must not be empty");
        }
        if self.has_command(command_id) {
            anyhow::bail!("voice mutation command {command_id:?} is already applied");
        }
        let sequence = u64::try_from(self.events.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| anyhow::anyhow!("voice event sequence overflow"))?;
        self.events.push(VoiceCallEvent {
            sequence,
            command_id: command_id.to_string(),
            event_type,
            event,
            at_ms,
        });
        Ok(())
    }

    pub(crate) fn has_command(&self, command_id: &str) -> bool {
        self.events
            .iter()
            .any(|event| event.command_id == command_id)
    }

    pub(crate) fn command_matches(&self, command_id: &str, proposed: &VoiceCallAggregate) -> bool {
        let event = self
            .events
            .iter()
            .find(|event| event.command_id == command_id);
        let proposed_event = proposed
            .events
            .iter()
            .find(|event| event.command_id == command_id);
        event.is_some()
            && event == proposed_event
            && self.revision >= proposed.revision
            && self.authority_ura == proposed.authority_ura
            && self.call_id == proposed.call_id
            && self.created_at_ms == proposed.created_at_ms
            && self.events.len() >= proposed.events.len()
            && self.events[..proposed.events.len()] == proposed.events[..]
    }

    pub(crate) fn events_json(&self) -> Vec<Value> {
        self.events.iter().map(VoiceCallEvent::to_json).collect()
    }

    pub(crate) fn to_json(&self) -> Value {
        let participants = self
            .participants
            .values()
            .map(VoiceParticipant::to_json)
            .collect::<Vec<_>>();
        json!({
            "call_id": self.call_id,
            "state": self.state.wire_name(),
            "state_code": self.state.to_wire_i32(),
            "created_at_ms": self.created_at_ms,
            "ended_at_ms": self.ended_at_ms,
            "end_reason": self.end_reason.map(VoiceEndReason::wire_name),
            "end_reason_code": self.end_reason.map(VoiceEndReason::to_wire_i32),
            "participants": participants,
        })
    }

    pub fn validate_recovered(&self) -> anyhow::Result<()> {
        let authority = crate::core::ura::parse_ura(&self.authority_ura).map_err(|error| {
            anyhow::anyhow!("invalid voice authority {:?}: {error}", self.authority_ura)
        })?;
        if authority.kind != crate::core::ura::URAKind::Authority {
            anyhow::bail!(
                "voice aggregate authority must be Authority, got {:?}",
                self.authority_ura
            );
        }
        if self.call_id.trim().is_empty() {
            anyhow::bail!("voice aggregate call_id must not be empty");
        }
        if self.revision == 0 || self.revision == u64::MAX {
            anyhow::bail!(
                "voice aggregate {:?} has invalid revision {}",
                self.call_id,
                self.revision
            );
        }
        match self.state {
            VoiceCallState::Ended if self.ended_at_ms.is_none() || self.end_reason.is_none() => {
                anyhow::bail!(
                    "ended voice aggregate {:?} lacks terminal facts",
                    self.call_id
                )
            }
            VoiceCallState::Ringing | VoiceCallState::Active
                if self.ended_at_ms.is_some() || self.end_reason.is_some() =>
            {
                anyhow::bail!(
                    "open voice aggregate {:?} carries terminal facts",
                    self.call_id
                )
            }
            _ => {}
        }
        let active = self.active_participant_count();
        match self.state {
            VoiceCallState::Ringing if active >= 2 => anyhow::bail!(
                "ringing voice aggregate {:?} has {active} active participants",
                self.call_id
            ),
            VoiceCallState::Active if active < 2 => anyhow::bail!(
                "active voice aggregate {:?} has only {active} active participants",
                self.call_id
            ),
            _ => {}
        }
        if self
            .ended_at_ms
            .is_some_and(|ended| ended < self.created_at_ms)
        {
            anyhow::bail!("voice aggregate {:?} ended before creation", self.call_id);
        }
        if self
            .participants
            .iter()
            .any(|(key, participant)| key != &participant.participant_id)
        {
            anyhow::bail!(
                "voice aggregate {:?} has a participant key mismatch",
                self.call_id
            );
        }
        if self.participants.values().any(|participant| {
            matches!(
                (participant.state, participant.left_at_ms),
                (VoiceParticipantState::Joined, Some(_)) | (VoiceParticipantState::Left, None)
            )
        }) {
            anyhow::bail!(
                "voice aggregate {:?} has inconsistent participant lifecycle facts",
                self.call_id
            );
        }
        for participant in self.participants.values() {
            if participant.joined_at_ms < self.created_at_ms
                || participant
                    .left_at_ms
                    .is_some_and(|left| left < participant.joined_at_ms)
            {
                anyhow::bail!(
                    "voice aggregate {:?} has invalid participant time ordering",
                    self.call_id
                );
            }
        }
        let expected_revision = u64::try_from(self.events.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| anyhow::anyhow!("voice aggregate event count overflow"))?;
        if self.revision != expected_revision {
            anyhow::bail!(
                "voice aggregate {:?} revision {} does not match event sequence {}",
                self.call_id,
                self.revision,
                expected_revision
            );
        }
        let mut command_ids = std::collections::BTreeSet::new();
        let joined_by_event = self
            .events
            .iter()
            .filter(|event| event.event_type == VoiceEventType::ParticipantJoin)
            .filter_map(|event| event.event.get("participant_id").and_then(Value::as_str))
            .collect::<std::collections::BTreeSet<_>>();
        let creator_ids = self
            .participants
            .keys()
            .filter(|participant_id| !joined_by_event.contains(participant_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if creator_ids.len() > 1 {
            anyhow::bail!(
                "voice aggregate {:?} has more than one implicit creator participant",
                self.call_id
            );
        }
        let mut replayed_participants = BTreeMap::new();
        if let Some(creator_id) = creator_ids.first() {
            let creator = self
                .participants
                .get(creator_id)
                .expect("creator key came from participants");
            if creator.joined_at_ms != self.created_at_ms {
                anyhow::bail!("voice creator participant must join at call creation");
            }
            replayed_participants.insert(creator_id.clone(), VoiceParticipantState::Joined);
        }
        let mut replayed_metrics = BTreeMap::new();
        let mut terminal_seen = false;
        let mut previous_event_at = self.created_at_ms;
        let mut terminal_events = 0usize;
        for (index, event) in self.events.iter().enumerate() {
            let expected_sequence = u64::try_from(index).unwrap_or(u64::MAX) + 1;
            if event.sequence != expected_sequence
                || event.command_id.trim().is_empty()
                || !command_ids.insert(event.command_id.as_str())
                || event.at_ms < self.created_at_ms
                || event.at_ms < previous_event_at
                || self.ended_at_ms.is_some_and(|ended| event.at_ms > ended)
            {
                anyhow::bail!(
                    "voice aggregate {:?} has invalid event sequence, identity, or time ordering",
                    self.call_id
                );
            }
            if terminal_seen {
                anyhow::bail!("voice aggregate carries a mutation after its terminal event");
            }
            previous_event_at = event.at_ms;
            match event.event_type {
                VoiceEventType::ParticipantJoin => {
                    let participant_id = event
                        .event
                        .get("participant_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            anyhow::anyhow!("voice participant join event has no participant key")
                        })?;
                    let participant = self.participants.get(participant_id).ok_or_else(|| {
                        anyhow::anyhow!("voice participant join event references an unknown key")
                    })?;
                    if participant.joined_at_ms != event.at_ms {
                        anyhow::bail!("voice participant join event time disagrees with state");
                    }
                    if replayed_participants
                        .insert(participant_id.to_string(), VoiceParticipantState::Joined)
                        .is_some()
                    {
                        anyhow::bail!("voice participant was joined more than once");
                    }
                }
                VoiceEventType::ParticipantLeave => {
                    let participant_id = event
                        .event
                        .get("participant_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            anyhow::anyhow!("voice participant leave event has no participant key")
                        })?;
                    let participant = self.participants.get(participant_id).ok_or_else(|| {
                        anyhow::anyhow!("voice participant leave event references an unknown key")
                    })?;
                    if participant.state != VoiceParticipantState::Left
                        || participant.left_at_ms != Some(event.at_ms)
                    {
                        anyhow::bail!("voice participant leave event disagrees with state");
                    }
                    match replayed_participants.get_mut(participant_id) {
                        Some(state @ VoiceParticipantState::Joined) => {
                            *state = VoiceParticipantState::Left;
                        }
                        Some(VoiceParticipantState::Left) => {
                            anyhow::bail!("voice participant was left more than once")
                        }
                        None => anyhow::bail!("voice participant left before joining"),
                    }
                }
                VoiceEventType::MetricsReported => {
                    let participant_id = event
                        .event
                        .get("participant_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            anyhow::anyhow!("voice metrics event has no participant key")
                        })?;
                    if replayed_participants.get(participant_id)
                        != Some(&VoiceParticipantState::Joined)
                    {
                        anyhow::bail!(
                            "voice metrics event requires a currently joined participant"
                        );
                    }
                    let metrics =
                        VoiceNetworkMetrics::from_json(event.event.get("metrics").ok_or_else(
                            || anyhow::anyhow!("voice metrics event has no metrics"),
                        )?)?;
                    replayed_metrics.insert(participant_id.to_string(), metrics);
                }
                VoiceEventType::CallEnded => {
                    terminal_events += 1;
                    terminal_seen = true;
                    if self.ended_at_ms != Some(event.at_ms) || index + 1 != self.events.len() {
                        anyhow::bail!("voice terminal event disagrees with terminal state");
                    }
                    let end_reason = self.end_reason.expect("ended aggregate checked above");
                    if event.event.get("end_reason").and_then(Value::as_str)
                        != Some(end_reason.wire_name())
                        || event.event.get("end_reason_code").and_then(Value::as_u64)
                            != Some(u64::from(end_reason.to_wire_i32() as u32))
                    {
                        anyhow::bail!("voice terminal event disagrees with end reason");
                    }
                }
            }
            let replayed_state = if terminal_seen {
                VoiceCallState::Ended
            } else if replayed_participants
                .values()
                .filter(|state| **state == VoiceParticipantState::Joined)
                .count()
                >= 2
            {
                VoiceCallState::Active
            } else {
                VoiceCallState::Ringing
            };
            if event.event.get("state").and_then(Value::as_str) != Some(replayed_state.wire_name())
                || event.event.get("state_code").and_then(Value::as_u64)
                    != Some(u64::from(replayed_state.to_wire_i32() as u32))
            {
                anyhow::bail!("voice event state disagrees with replayed aggregate state");
            }
        }
        match self.state {
            VoiceCallState::Ended if terminal_events != 1 => anyhow::bail!(
                "ended voice aggregate {:?} must have exactly one terminal event",
                self.call_id
            ),
            VoiceCallState::Ringing | VoiceCallState::Active if terminal_events != 0 => {
                anyhow::bail!(
                    "open voice aggregate {:?} has a terminal event",
                    self.call_id
                )
            }
            _ => {}
        }
        if replayed_participants.len() != self.participants.len()
            || self
                .participants
                .iter()
                .any(|(participant_id, participant)| {
                    replayed_participants.get(participant_id) != Some(&participant.state)
                        || participant.last_metrics.as_ref() != replayed_metrics.get(participant_id)
                })
        {
            anyhow::bail!("voice participant snapshot disagrees with replayed event history");
        }
        Ok(())
    }

    /// Validate a provider result against the exact repository key requested
    /// by the service. Aggregate self-validation alone cannot detect a
    /// provider returning a valid call from another realm or call id.
    pub fn validate_repository_key(
        &self,
        authority_ura: &str,
        call_id: &str,
    ) -> anyhow::Result<()> {
        self.validate_recovered()?;
        if self.authority_ura != authority_ura || self.call_id != call_id {
            anyhow::bail!(
                "voice repository key mismatch: requested ({authority_ura:?}, {call_id:?}), returned ({:?}, {:?})",
                self.authority_ura,
                self.call_id
            );
        }
        Ok(())
    }

    /// Validate the one-step revision relation required by repository CAS.
    pub fn validate_cas_replacement(&self, expected_revision: u64) -> anyhow::Result<()> {
        self.validate_recovered()?;
        let required_revision = expected_revision.checked_add(1).ok_or_else(|| {
            anyhow::anyhow!(
                "voice aggregate {:?} CAS expected revision overflows",
                self.call_id
            )
        })?;
        if required_revision == u64::MAX || self.revision != required_revision {
            anyhow::bail!(
                "voice aggregate {:?} CAS replacement revision must be exactly expected+1 (expected {}, replacement {})",
                self.call_id,
                expected_revision,
                self.revision
            );
        }
        Ok(())
    }

    fn active_participant_count(&self) -> usize {
        self.participants
            .values()
            .filter(|participant| participant.state == VoiceParticipantState::Joined)
            .count()
    }

    fn ensure_open(&self, ability: &str) -> anyhow::Result<()> {
        if self.state.is_terminal() {
            anyhow::bail!("{ability}: call {:?} has already ended", self.call_id);
        }
        Ok(())
    }
}

impl VoiceNetworkMetrics {
    pub(super) fn from_json(value: &Value) -> anyhow::Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("voice metrics must be a JSON object"))?;
        let concealed_samples = object
            .get("concealed_samples")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .try_into()
            .map_err(|_| anyhow::anyhow!("voice metrics concealed_samples is outside u32 range"))?;
        let metrics = Self {
            rtt_ms: number(object, "rtt_ms")?,
            jitter_ms: number(object, "jitter_ms")?,
            packet_loss_ratio: number(object, "packet_loss_ratio")?,
            concealed_samples,
            audio_level_dbov: number(object, "audio_level_dbov")?,
        };
        if !(0.0..=1.0).contains(&metrics.packet_loss_ratio) {
            anyhow::bail!("voice metrics packet_loss_ratio must be in [0.0, 1.0]");
        }
        Ok(metrics)
    }

    pub(super) fn to_json(&self) -> Value {
        json!({
            "rtt_ms": self.rtt_ms,
            "jitter_ms": self.jitter_ms,
            "packet_loss_ratio": self.packet_loss_ratio,
            "concealed_samples": self.concealed_samples,
            "audio_level_dbov": self.audio_level_dbov,
        })
    }
}

fn number(object: &serde_json::Map<String, Value>, key: &'static str) -> anyhow::Result<f64> {
    match object.get(key) {
        None => Ok(0.0),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("voice metrics field `{key}` must be numeric")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_wire_values_remain_stable_after_axon_extraction() {
        assert_eq!(VoiceCallState::Ringing.to_wire_i32(), 1);
        assert_eq!(VoiceCallState::Active.to_wire_i32(), 2);
        assert_eq!(VoiceCallState::Ended.to_wire_i32(), 5);
        assert_eq!(VoiceEndReason::PolicyDenied.to_wire_i32(), 4);
        assert_eq!(
            VoiceEventType::ParticipantJoin.wire_name(),
            "VOICE_EVENT_PARTICIPANT_JOIN"
        );
    }

    #[test]
    fn metrics_reject_invalid_product_payloads() {
        assert!(VoiceNetworkMetrics::from_json(&json!({"packet_loss_ratio": 1.1})).is_err());
        assert!(VoiceNetworkMetrics::from_json(&json!({"rtt_ms": "fast"})).is_err());
    }

    #[test]
    fn provider_assembly_rejects_unqualified_repository() {
        let repository = Arc::new(TestVoiceCallRepository::default());
        let error = VoiceCallProviderAssembly::try_new(repository)
            .expect_err("test repository must not qualify for production assembly");
        assert!(error
            .to_string()
            .contains("not qualified for durable realm authority"));
    }

    #[test]
    fn repository_compare_and_swap_rejects_a_stale_revision() {
        const AUTHORITY: &str = "easynet:///r/voice-cas/authority";
        let repository = TestVoiceCallRepository::default();
        repository
            .insert_if_absent(VoiceCallAggregate::new(
                AUTHORITY.to_string(),
                "call-1".to_string(),
                Some("alice".to_string()),
                10,
            ))
            .expect("insert aggregate");
        let stale = repository
            .load(AUTHORITY, "call-1")
            .expect("load aggregate")
            .expect("aggregate exists");

        let mut winner = stale.clone();
        winner
            .join("join-bob", "bob".to_string(), None, 20)
            .expect("winner transition");
        winner.bump_revision().expect("winner revision");
        assert!(matches!(
            repository
                .compare_and_swap(AUTHORITY, "call-1", 1, winner)
                .expect("winner compare-and-swap"),
            VoiceCallCasOutcome::Committed(_)
        ));

        let mut loser = stale;
        loser
            .join("join-carol", "carol".to_string(), None, 30)
            .expect("stale transition");
        loser.bump_revision().expect("loser revision");
        assert!(matches!(
            repository
                .compare_and_swap(AUTHORITY, "call-1", 1, loser)
                .expect("stale compare-and-swap"),
            VoiceCallCasOutcome::Current(_)
        ));
        assert_eq!(
            repository
                .load(AUTHORITY, "call-1")
                .expect("reload aggregate")
                .expect("aggregate exists")
                .revision(),
            2
        );
    }

    #[test]
    fn repository_rejects_a_non_advancing_cas_replacement() {
        const AUTHORITY: &str = "easynet:///r/voice-cas/authority";
        let repository = TestVoiceCallRepository::default();
        let aggregate = VoiceCallAggregate::new(
            AUTHORITY.to_string(),
            "call-same-revision".to_string(),
            None,
            10,
        );
        repository
            .insert_if_absent(aggregate.clone())
            .expect("insert aggregate");
        let error = repository
            .compare_and_swap(AUTHORITY, "call-same-revision", 1, aggregate)
            .expect_err("CAS replacement must advance exactly one revision");
        assert!(error.to_string().contains("expected+1"));
    }

    #[test]
    fn maximum_revision_fails_recovery_and_mutation_closed() {
        const AUTHORITY: &str = "easynet:///r/voice-revision/authority";
        let aggregate =
            VoiceCallAggregate::new(AUTHORITY.to_string(), "call-max".to_string(), None, 10);
        let mut encoded = serde_json::to_value(aggregate).expect("serialize aggregate");
        encoded["revision"] = json!(u64::MAX);
        let mut recovered: VoiceCallAggregate =
            serde_json::from_value(encoded).expect("decode max revision fixture");

        assert!(recovered.validate_recovered().is_err());
        assert!(recovered.bump_revision().is_err());

        let mut near_max = serde_json::to_value(VoiceCallAggregate::new(
            AUTHORITY.to_string(),
            "call-near-max".to_string(),
            None,
            10,
        ))
        .expect("serialize aggregate");
        near_max["revision"] = json!(u64::MAX - 1);
        let mut near_max: VoiceCallAggregate =
            serde_json::from_value(near_max).expect("decode near-max revision fixture");
        assert!(near_max.bump_revision().is_err());
        assert_eq!(near_max.revision(), u64::MAX - 1);
    }

    #[test]
    fn recovery_rejects_cardinality_time_and_event_sequence_corruption() {
        const AUTHORITY: &str = "easynet:///r/voice-recovery/authority";
        let base = VoiceCallAggregate::new(
            AUTHORITY.to_string(),
            "call-corrupt".to_string(),
            Some("alice".to_string()),
            10,
        );

        let mut active = serde_json::to_value(&base).unwrap();
        active["state"] = json!("active");
        let active: VoiceCallAggregate = serde_json::from_value(active).unwrap();
        assert!(active
            .validate_recovered()
            .unwrap_err()
            .to_string()
            .contains("active participants"));

        let mut bad_time = serde_json::to_value(&base).unwrap();
        bad_time["participants"]["alice"]["joined_at_ms"] = json!(9);
        let bad_time: VoiceCallAggregate = serde_json::from_value(bad_time).unwrap();
        assert!(bad_time
            .validate_recovered()
            .unwrap_err()
            .to_string()
            .contains("time ordering"));

        let mut bad_sequence = base;
        bad_sequence
            .join("join-bob", "bob".to_string(), None, 20)
            .unwrap();
        bad_sequence.bump_revision().unwrap();
        let mut encoded = serde_json::to_value(bad_sequence).unwrap();
        encoded["events"][0]["sequence"] = json!(2);
        let bad_sequence: VoiceCallAggregate = serde_json::from_value(encoded).unwrap();
        assert!(bad_sequence
            .validate_recovered()
            .unwrap_err()
            .to_string()
            .contains("event sequence"));
    }
}
